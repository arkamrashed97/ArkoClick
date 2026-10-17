//! Main window. Four tabs (classic, simple, advanced, keyboard), each with its own
//! list of tasks and a step-by-step editor. Everything is translated (English / Arabic,
//! mirrored layout in Arabic) and follows the light/dark/system theme.
//! Repaints only while something is running.

mod advanced;
mod classic;
mod keyboard;
mod keys;
pub mod rtl;
mod settings;
mod simple;
pub mod theme;
mod updates;
mod widgets;

use crate::config::{
    is_function_key, Action, AppConfig, ClickPoint, ClickerConfig, ClickerKind, ClickMode, Hotkey, Lang, Profile,
    Step, TargetInfo, APP_TITLE, APP_VERSION, DEFAULT_WAIT_MS, DEVELOPER, DEVELOPER_EMAIL, DEVELOPER_PHONE,
    THUMB_SIZE,
};
use crate::engine::{ClickerStats, Engine, Status};
use crate::win::picker::{self, PickOutcome, PickResult, Picker};
use crate::win::textdlg::{TextDialog, TextDialogSpec};
use crate::win::{ghost, hotkey, tray, window, SafeHwnd};
use base64::Engine as _;
use egui::{Color32, Context, RichText, TextureHandle, ViewportCommand};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};
use theme::{muted_color, semibold, ACCENT, ACCENT_STRONG, DANGER, INFO, WARN};
use widgets::{big_button, card, chip, spot_picture, status_pill, switch, Tr};

const SAVE_DEBOUNCE: Duration = Duration::from_millis(800);
/// Wait after the "click the text box first" step of a keyboard task.
const FOCUS_WAIT_MS: u64 = 150;
/// Countdown before a classic clicker started from the window clicks for the first time.
const CLASSIC_START_DELAY: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PickPurpose {
    /// Set the task's window and its first click spot.
    Target,
    /// Set the spot of one click step (advanced).
    Step(usize),
    /// Keyboard task: the window and the text box to click before typing.
    KeyboardField,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Capture {
    ToggleAll,
    StopAll,
    Clicker(u64),
    StepKey(u64, usize),
}

impl Capture {
    fn wants_hotkey(self) -> bool {
        !matches!(self, Capture::StepKey(..))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TextTarget {
    ClickerName(u64),
    StepText(u64, usize),
    NewProfile,
    ProfileName(usize),
    TriggerTitle(u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Confirm {
    DeleteClicker(u64),
    DeleteProfile(usize),
}

/// Things the editors ask for while drawing; applied after the frame's borrows end.
pub(crate) enum Act {
    Pick(u64, PickPurpose),
    Test(u64, usize),
    Toggle(u64),
    EditText(TextTarget, String, bool),
    Capture(Capture),
    Ghost(u64),
    Confirm(Confirm),
    Select(ClickerKind, u64),
    New(ClickerKind),
    Toast(&'static str, &'static str, bool),
}

pub struct App {
    pub(crate) cfg: AppConfig,
    pub(crate) engine: Arc<Engine>,
    pub(crate) tab: ClickerKind,
    pub(crate) selected: HashMap<ClickerKind, u64>,
    picker: Option<Picker>,
    pick_for: Option<(u64, PickPurpose)>,
    text_dialog: Option<(TextDialog, TextTarget)>,
    pub(crate) windows_cache: Vec<window::WindowInfo>,
    windows_cache_at: Option<Instant>,
    dirty_since: Option<Instant>,
    pub(crate) show_settings: bool,
    toast: Option<(String, Instant, bool)>,
    pub(crate) stats: HashMap<u64, ClickerStats>,
    last_tooltip: String,
    pub(crate) capture: Option<Capture>,
    last_bindings: Option<hotkey::Bindings>,
    confirm: Option<Confirm>,
    thumbs: HashMap<u64, TextureHandle>,
    pub(crate) expanded: HashSet<u64>,
    dark: bool,
    frame_counter: u64,
    pub(crate) updater: updates::Updater,
    last_links: Option<(Vec<crate::triggers::Rule>, bool, bool)>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, cfg: AppConfig, engine: Arc<Engine>, start_hidden: bool) -> Self {
        theme::install_fonts(&cc.egui_ctx);
        theme::apply_style(&cc.egui_ctx, cfg.theme, cfg.font_size, cfg.lang() == Lang::Ar);
        tray::ARABIC.store(cfg.lang() == Lang::Ar, Ordering::Relaxed);
        engine.sync(cfg.clickers());
        for c in cfg.clickers().iter().filter(|c| c.auto_run && c.target.is_set()) {
            engine.start(c.id);
        }
        if start_hidden {
            cc.egui_ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        }
        // Open on the tab of the first existing task (classic when empty).
        let tab = cfg.clickers().first().map(|c| c.kind()).unwrap_or(ClickerKind::Classic);
        let updater = updates::Updater::new(
            &cc.egui_ctx,
            cfg.check_updates,
            updates::NotifyPrefs { arabic: cfg.lang() == Lang::Ar, skipped: cfg.skipped_version.clone() },
        );
        Self {
            cfg,
            engine,
            tab,
            selected: HashMap::new(),
            picker: None,
            pick_for: None,
            text_dialog: None,
            windows_cache: Vec::new(),
            windows_cache_at: None,
            dirty_since: None,
            show_settings: false,
            toast: None,
            stats: HashMap::new(),
            last_tooltip: String::new(),
            capture: None,
            last_bindings: None,
            confirm: None,
            thumbs: HashMap::new(),
            expanded: HashSet::new(),
            dark: true,
            frame_counter: 0,
            updater,
            last_links: None,
        }
    }

    // ------------------------------------------------------------------ small helpers

    pub(crate) fn tr(&self) -> Tr {
        Tr { ar: self.cfg.lang() == Lang::Ar }
    }

    pub(crate) fn mark_dirty(&mut self) {
        self.dirty_since = Some(Instant::now());
        self.engine.sync(self.cfg.clickers());
    }

    fn flush_save(&mut self) {
        if self.dirty_since.take().is_some() {
            self.cfg.save();
        }
    }

    pub(crate) fn toast(&mut self, en: &str, ar: &str, is_error: bool) {
        let msg = self.tr().s(en, ar).to_string();
        self.toast = Some((msg, Instant::now(), is_error));
    }

    pub(crate) fn refresh_windows(&mut self, force: bool) {
        let stale = self.windows_cache_at.map_or(true, |t| t.elapsed() > Duration::from_secs(3));
        if force || stale {
            let mut list = window::enumerate();
            list.retain(|w| w.title != APP_TITLE && w.title != picker::PICKER_TITLE);
            self.windows_cache = list;
            self.windows_cache_at = Some(Instant::now());
        }
    }

    pub(crate) fn index_of(&self, id: u64) -> Option<usize> {
        self.cfg.clickers().iter().position(|c| c.id == id)
    }

    fn clicker_mut(&mut self, id: u64) -> Option<&mut ClickerConfig> {
        self.cfg.clickers_mut().iter_mut().find(|c| c.id == id)
    }

    /// The live window a task is bound to: the engine's (after re-binds), else the
    /// picked handle, else a fresh lookup by class/process.
    pub(crate) fn live_root(&self, c: &ClickerConfig) -> Option<SafeHwnd> {
        if let Some(h) = self.stats.get(&c.id).and_then(|s| s.root) {
            return Some(SafeHwnd(h));
        }
        if let Some(h) = c.target.hwnd.map(SafeHwnd).filter(|h| window::is_window(*h)) {
            return Some(h);
        }
        if c.target.is_set() {
            return window::find_matching(&c.target.class, &c.target.process, &c.target.title);
        }
        None
    }

    /// Texture for a stored spot thumbnail (cached by content).
    pub(crate) fn thumb_tex(&mut self, ctx: &Context, b64: Option<&str>) -> Option<TextureHandle> {
        use std::hash::{Hash, Hasher};
        let b64 = b64?;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        b64.hash(&mut h);
        let key = h.finish();
        if let Some(t) = self.thumbs.get(&key) {
            return Some(t.clone());
        }
        let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
        let n = THUMB_SIZE as usize;
        if bytes.len() != n * n * 4 {
            return None;
        }
        let img = egui::ColorImage::from_rgba_unmultiplied([n, n], &bytes);
        let tex = ctx.load_texture(format!("thumb-{key}"), img, egui::TextureOptions::NEAREST);
        self.thumbs.insert(key, tex.clone());
        Some(tex)
    }

    fn hide_to_tray(&mut self, ctx: &Context) {
        self.flush_save();
        ctx.send_viewport_cmd(ViewportCommand::Visible(false));
    }

    pub(crate) fn quit(&mut self, ctx: &Context) {
        self.engine.stop_all();
        ghost::show_all();
        self.flush_save();
        tray::shutdown();
        ctx.send_viewport_cmd(ViewportCommand::Close);
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(300));
            std::process::exit(0);
        });
    }

    pub(crate) fn switch_profile(&mut self, idx: usize) {
        if idx >= self.cfg.profiles.len() {
            return;
        }
        self.engine.stop_all();
        self.cfg.active_profile = idx;
        self.selected.clear();
        self.mark_dirty();
    }

    pub(crate) fn set_language(&mut self, ctx: &Context, lang: Lang) {
        self.cfg.lang = Some(lang);
        tray::ARABIC.store(lang == Lang::Ar, Ordering::Relaxed);
        theme::apply_style(ctx, self.cfg.theme, self.cfg.font_size, lang == Lang::Ar);
        self.mark_dirty();
    }

    fn current_bindings(&self) -> hotkey::Bindings {
        hotkey::Bindings {
            toggle_all: self.cfg.hotkey_toggle_all,
            stop_all: self.cfg.hotkey_stop_all,
            clickers: self
                .cfg
                .clickers()
                .iter()
                .filter(|c| c.hotkey.is_set())
                .map(|c| (c.id, c.hotkey, c.name.clone()))
                .collect(),
        }
    }

    /// Selected task of a tab (falls back to the tab's first task).
    pub(crate) fn selected_in(&self, kind: ClickerKind) -> Option<u64> {
        let in_tab = |id: u64| self.cfg.clickers().iter().any(|c| c.id == id && c.kind() == kind);
        self.selected
            .get(&kind)
            .copied()
            .filter(|id| in_tab(*id))
            .or_else(|| self.cfg.clickers().iter().find(|c| c.kind() == kind).map(|c| c.id))
    }

    fn new_clicker(&mut self, kind: ClickerKind) {
        let id = self.cfg.alloc_id();
        let n = self.cfg.clickers().iter().filter(|c| c.kind() == kind).count() + 1;
        let mut c = ClickerConfig::new_of_kind(id, kind, n, self.cfg.lang());
        if kind == ClickerKind::Classic {
            let mut used: Vec<Hotkey> = self.cfg.clickers().iter().map(|c| c.hotkey).collect();
            used.push(self.cfg.hotkey_toggle_all);
            used.push(self.cfg.hotkey_stop_all);
            c.hotkey = crate::config::find_available_classic_hotkey(&used);
        }
        self.cfg.clickers_mut().push(c);
        self.selected.insert(kind, id);
        self.tab = kind;
        self.mark_dirty();
    }

    // ------------------------------------------------------------------ key recording

    fn begin_capture(&mut self, c: Capture) {
        self.capture = Some(c);
        hotkey::set_suspended(true);
    }

    fn end_capture(&mut self) {
        self.capture = None;
        hotkey::set_suspended(false);
    }

    /// While recording, swallow key events before any widget sees them.
    fn handle_capture(&mut self, ctx: &Context) {
        let Some(cap) = self.capture else { return };
        let events = ctx.input_mut(|i| std::mem::take(&mut i.events));
        for e in events {
            let egui::Event::Key { key, pressed: true, modifiers, .. } = e else { continue };
            if key == egui::Key::Escape && !modifiers.any() {
                self.end_capture();
                return;
            }
            let Some(vk) = keys::egui_to_vk(key) else { continue };
            if cap.wants_hotkey() {
                if (key == egui::Key::Backspace || key == egui::Key::Delete) && !modifiers.any() {
                    self.apply_capture(cap, Hotkey::NONE);
                    return;
                }
                let hk = Hotkey { vk, ctrl: modifiers.ctrl, alt: modifiers.alt, shift: modifiers.shift };
                if !is_function_key(vk) && !hk.has_modifier() {
                    self.toast(
                        "Use F1-F12 alone, or hold Ctrl / Alt / Shift with another key",
                        "استخدم F1 إلى F12 وحدها، أو اضغط Ctrl أو Alt أو Shift مع زر آخر",
                        true,
                    );
                    continue;
                }
                self.apply_capture(cap, hk);
            } else {
                self.apply_capture(cap, Hotkey::key(vk));
            }
            return;
        }
    }

    fn apply_capture(&mut self, cap: Capture, hk: Hotkey) {
        match cap {
            Capture::ToggleAll => self.cfg.hotkey_toggle_all = hk,
            Capture::StopAll => self.cfg.hotkey_stop_all = hk,
            Capture::Clicker(id) => {
                if let Some(c) = self.clicker_mut(id) {
                    c.hotkey = hk;
                }
            }
            Capture::StepKey(id, i) => {
                if let Some(step) = self.clicker_mut(id).and_then(|c| c.steps.get_mut(i)) {
                    step.action = Action::Key { vk: hk.vk };
                }
            }
        }
        self.end_capture();
        self.mark_dirty();
    }

    // ------------------------------------------------------------------ dialogs (picker / text)

    fn open_picker(&mut self, id: u64, purpose: PickPurpose) {
        let tr = self.tr();
        let hint = match purpose {
            PickPurpose::KeyboardField => tr.s(
                "Click the box where ArkoClick should type      ·      Esc = cancel",
                "انقر على المكان الذي تريد أن يكتب فيه ArkoClick\u{200F}      ·      \u{200F}Esc للإلغاء",
            ),
            _ => tr.s(
                "Click the exact spot ArkoClick should click      ·      Esc = cancel",
                "انقر على المكان الذي تريد أن يضغط عليه ArkoClick\u{200F}      ·      \u{200F}Esc للإلغاء",
            ),
        };
        self.pick_for = Some((id, purpose));
        self.picker = Some(Picker::open(hint.to_string(), tr.ar));
    }

    fn open_text(&mut self, target: TextTarget, initial: String, multiline: bool) {
        let tr = self.tr();
        let (title, prompt) = match target {
            TextTarget::ClickerName(_) => (tr.s("Rename", "تغيير الاسم"), tr.s("Name for this task:", "اسم هذه المهمة:")),
            TextTarget::StepText(..) => {
                (tr.s("Text to type", "النص المطلوب كتابته"), tr.s("ArkoClick will type this text:", "سيكتب ArkoClick هذا النص:"))
            }
            TextTarget::NewProfile => (tr.s("New profile", "ملف جديد"), tr.s("Profile name:", "اسم الملف:")),
            TextTarget::ProfileName(_) => (tr.s("Rename profile", "تغيير اسم الملف"), tr.s("Profile name:", "اسم الملف:")),
            TextTarget::TriggerTitle(_) => (
                tr.s("Which window?", "أي نافذة؟"),
                tr.s("Ask only when the window title contains (leave empty for any window):", "اسأل فقط عندما يحتوي عنوان النافذة على (اتركه فارغًا لأي نافذة):"),
            ),
        };
        let spec = TextDialogSpec {
            title: title.into(),
            prompt: prompt.into(),
            initial,
            ok: tr.s("OK", "موافق").into(),
            cancel: tr.s("Cancel", "إلغاء").into(),
            multiline,
            rtl: tr.ar,
            dark: self.dark,
        };
        self.text_dialog = Some((TextDialog::open(spec), target));
    }

    fn poll_text_dialog(&mut self, ctx: &Context) {
        let Some((dlg, target)) = self.text_dialog.as_ref() else { return };
        let target = *target;
        let Some(result) = dlg.poll() else {
            ctx.request_repaint_after(Duration::from_millis(100));
            return;
        };
        self.text_dialog = None;
        let Some(text) = result else { return };
        match target {
            TextTarget::ClickerName(id) => {
                let name = text.trim().to_string();
                if !name.is_empty() {
                    if let Some(c) = self.clicker_mut(id) {
                        c.name = name;
                    }
                }
            }
            TextTarget::StepText(id, i) => {
                if let Some(step) = self.clicker_mut(id).and_then(|c| c.steps.get_mut(i)) {
                    step.action = Action::Text { text: text.replace("\r\n", "\n") };
                }
            }
            TextTarget::NewProfile => {
                let name = if text.trim().is_empty() { format!("Profile {}", self.cfg.profiles.len() + 1) } else { text.trim().into() };
                let id = self.cfg.alloc_id();
                let c = ClickerConfig::new_of_kind(id, ClickerKind::Simple, 1, self.cfg.lang());
                self.cfg.profiles.push(Profile { name, clickers: vec![c] });
                self.switch_profile(self.cfg.profiles.len() - 1);
            }
            TextTarget::ProfileName(i) => {
                if !text.trim().is_empty() && i < self.cfg.profiles.len() {
                    self.cfg.profiles[i].name = text.trim().into();
                }
            }
            TextTarget::TriggerTitle(id) => {
                if let Some(c) = self.clicker_mut(id) {
                    c.trigger.title_contains = text.trim().to_string();
                }
            }
        }
        self.mark_dirty();
    }

    fn poll_picker(&mut self, ctx: &Context) {
        let Some(p) = self.picker.as_ref() else { return };
        let Some(outcome) = p.poll() else {
            ctx.request_repaint_after(Duration::from_millis(100));
            return;
        };
        self.picker = None;
        let pick_for = self.pick_for.take();
        match outcome {
            PickOutcome::Picked(r) => {
                if let Some((id, purpose)) = pick_for {
                    self.apply_pick(id, purpose, r);
                }
            }
            PickOutcome::Cancelled => {}
            PickOutcome::Failed(e) => {
                log::info!("pick failed: {e}");
                if e.contains("itself") {
                    self.toast(
                        "That is ArkoClick itself. Pick a spot in another program.",
                        "هذا ArkoClick نفسه. اختر مكانًا في برنامج آخر.",
                        true,
                    );
                } else {
                    self.toast(
                        "That spot can't be used. Click inside the program's content, not its title bar.",
                        "لا يمكن استخدام هذا المكان. انقر داخل محتوى البرنامج وليس على شريط العنوان.",
                        true,
                    );
                }
            }
        }
        if let Some(h) = window::find_by_title(APP_TITLE) {
            window::show_and_focus(h);
        }
    }

    fn apply_pick(&mut self, id: u64, purpose: PickPurpose, r: PickResult) {
        let thumb = r.thumb.as_ref().map(|b| base64::engine::general_purpose::STANDARD.encode(b));
        let live = self.cfg.clickers().iter().find(|c| c.id == id).and_then(|c| self.live_root(c));
        let Some(c) = self.clicker_mut(id) else { return };
        let set_click = |step: &mut Step, point: ClickPoint| {
            if let Action::Click { point: p, .. } = &mut step.action {
                *p = ClickPoint { relative: p.relative, ..point };
            } else {
                step.action = Action::Click { point, button: Default::default(), kind: Default::default() };
            }
            step.thumb = thumb.clone();
        };
        match purpose {
            PickPurpose::Target => {
                c.target = r.target;
                c.post_to_root = false;
                match c.steps.iter().position(|s| matches!(s.action, Action::Click { .. })) {
                    Some(i) => set_click(&mut c.steps[i], r.point),
                    None => {
                        let mut s = Step::click(r.point, DEFAULT_WAIT_MS);
                        s.thumb = thumb.clone();
                        c.steps.insert(0, s);
                    }
                }
                self.toast("Spot saved. Press Start when you're ready.", "تم حفظ المكان. اضغط تشغيل عندما تكون جاهزًا.", false);
            }
            PickPurpose::KeyboardField => {
                c.target = r.target;
                c.post_to_root = false;
                if c.steps.len() >= 2 && matches!(c.steps[0].action, Action::Click { .. }) {
                    set_click(&mut c.steps[0], r.point);
                    c.steps[0].enabled = true;
                } else {
                    let mut s = Step::click(r.point, FOCUS_WAIT_MS);
                    s.thumb = thumb.clone();
                    c.steps.insert(0, s);
                }
                self.toast("Typing spot saved. Press Start when you're ready.", "تم حفظ مكان الكتابة. اضغط تشغيل عندما تكون جاهزًا.", false);
            }
            PickPurpose::Step(i) => {
                let same_window = !c.target.is_set()
                    || live.map_or(false, |h| Some(h.0) == r.target.hwnd)
                    || (c.target.class == r.target.class && c.target.process == r.target.process);
                if !same_window {
                    self.toast(
                        "That spot is in a different program. Each task works with one window; add another task for other programs.",
                        "هذا المكان في برنامج مختلف. كل مهمة تعمل مع نافذة واحدة، أضف مهمة أخرى للبرامج الأخرى.",
                        true,
                    );
                    return;
                }
                if !c.target.is_set() {
                    c.target = r.target.clone();
                }
                if let Some(step) = c.steps.get_mut(i) {
                    set_click(step, r.point);
                }
                self.toast("Spot saved.", "تم حفظ المكان.", false);
            }
        }
        self.mark_dirty();
    }

    // ------------------------------------------------------------------ deferred actions

    pub(crate) fn apply_acts(&mut self, acts: Vec<Act>) {
        for a in acts {
            match a {
                Act::Pick(id, p) => self.open_picker(id, p),
                Act::Test(id, step) => match self.engine.test_step(id, step) {
                    Ok(()) => self.toast("Sent once.", "تم الإرسال مرة واحدة.", false),
                    Err(e) => {
                        log::info!("test failed: {e}");
                        self.toast(
                            "Couldn't reach the window. Is it still open?",
                            "تعذر الوصول إلى النافذة. هل ما زالت مفتوحة؟",
                            true,
                        )
                    }
                },
                Act::Toggle(id) => {
                    let classic = self.cfg.clickers().iter().any(|c| c.id == id && c.kind() == ClickerKind::Classic);
                    let active = self.stats.get(&id).map_or(false, |s| s.status.is_active());
                    if classic && !active {
                        // Started from the window: give the user time to move the mouse.
                        self.engine.start_after(id, CLASSIC_START_DELAY);
                    } else {
                        self.engine.toggle(id);
                    }
                }
                Act::EditText(t, initial, multiline) => self.open_text(t, initial, multiline),
                Act::Capture(c) => {
                    if self.capture == Some(c) {
                        self.end_capture();
                    } else {
                        self.begin_capture(c);
                    }
                }
                Act::Ghost(id) => {
                    let Some(c) = self.cfg.clickers().iter().find(|c| c.id == id).cloned() else { continue };
                    if let Some(h) = self.live_root(&c) {
                        if ghost::is_ghosted(h) {
                            ghost::show(h);
                        } else if let Err(e) = ghost::hide(h) {
                            log::warn!("hide failed: {e}");
                            self.toast("This window can't be hidden.", "لا يمكن إخفاء هذه النافذة.", true);
                        } else {
                            self.toast(
                                "Window hidden and still working. Click its taskbar button to bring it back.",
                                "تم إخفاء النافذة وما زالت تعمل. انقر على أيقونتها في شريط المهام لإظهارها.",
                                false,
                            );
                        }
                    }
                }
                Act::Confirm(c) => self.confirm = Some(c),
                Act::Select(kind, id) => {
                    self.selected.insert(kind, id);
                }
                Act::New(kind) => self.new_clicker(kind),
                Act::Toast(en, ar, err) => self.toast(en, ar, err),
            }
        }
    }

    // ------------------------------------------------------------------ header / tabs / footer

    fn header(&mut self, ui: &mut egui::Ui, ctx: &Context) {
        let tr = self.tr();
        let size = self.cfg.font_size;
        let running = self.stats.values().filter(|s| s.status.is_active()).count();
        let mut toggle_lang = false;
        let mut toggle_theme = false;
        tr.row(ui, |ui| {
            theme::paint_logo(ui, 34.0);
            ui.label(semibold(APP_TITLE).size(size + 8.0));
            ui.label(RichText::new(format!("v{APP_VERSION}")).small().color(muted_color(ui)));
            ui.add_space(10.0);
            if running > 0 {
                let text = if tr.ar { format!("{running} يعمل الآن") } else { format!("{running} running") };
                status_pill(ui, tr.d(text), ACCENT);
            }
            tr.far(ui, |ui| {
                if ui.add(egui::Button::new(tr.t("⚙  Settings", "⚙  الإعدادات")).min_size(egui::vec2(0.0, 34.0))).clicked() {
                    self.show_settings = !self.show_settings;
                }
                let dark = ui.visuals().dark_mode;
                let (icon, tip) = if dark {
                    ("☀", tr.s("Switch to day mode", "التبديل إلى الوضع النهاري"))
                } else {
                    ("☾", tr.s("Switch to night mode", "التبديل إلى الوضع الليلي"))
                };
                if ui
                    .add(egui::Button::new(RichText::new(icon).size(size + 4.0)).min_size(egui::vec2(40.0, 34.0)))
                    .on_hover_text(tr.v(tip))
                    .clicked()
                {
                    toggle_theme = true;
                }
                let other = if tr.ar { "English" } else { "العربية" };
                if ui
                    .add(egui::Button::new(tr.d(format!("🌐  {other}"))).min_size(egui::vec2(0.0, 34.0)))
                    .on_hover_text(tr.v(tr.s("Change language", "تغيير اللغة")))
                    .clicked()
                {
                    toggle_lang = true;
                }
            });
        });
        if toggle_lang {
            let next = if tr.ar { Lang::En } else { Lang::Ar };
            self.set_language(ctx, next);
        }
        if toggle_theme {
            self.cfg.theme = if ui.visuals().dark_mode { crate::config::ThemePref::Light } else { crate::config::ThemePref::Dark };
            theme::apply_style(ctx, self.cfg.theme, self.cfg.font_size, tr.ar);
            self.mark_dirty();
        }
    }

    fn tabs(&mut self, ui: &mut egui::Ui) {
        let tr = self.tr();
        let size = self.cfg.font_size;
        let tabs = [
            (ClickerKind::Classic, "🎯", "Classic clicker", "النقر الكلاسيكي"),
            (ClickerKind::Simple, "🖱", "Simple clicker", "النقر البسيط"),
            (ClickerKind::Advanced, "☰", "Advanced clicker", "النقر المتقدم"),
            (ClickerKind::Keyboard, "⌨", "Keyboard", "لوحة المفاتيح"),
        ];
        let mut switch_to = None;
        let mut profile_to = None;
        tr.row(ui, |ui| {
            for (kind, icon, en, ar) in tabs {
                let selected = self.tab == kind;
                let count = self.cfg.clickers().iter().filter(|c| c.kind() == kind).count();
                let running = self
                    .cfg
                    .clickers()
                    .iter()
                    .filter(|c| c.kind() == kind)
                    .any(|c| self.stats.get(&c.id).map_or(false, |s| s.status.is_active()));
                let mut label = format!("{icon}  {}", tr.s(en, ar));
                if count > 0 {
                    label.push_str(&format!("  ({count})"));
                }
                let text = semibold(tr.v(&label)).size(size + 1.0);
                let (fill, color) = if selected {
                    (ACCENT_STRONG, Color32::WHITE)
                } else {
                    (Color32::TRANSPARENT, ui.visuals().text_color())
                };
                let resp = ui.add(egui::Button::new(text.color(color)).fill(fill).min_size(egui::vec2(0.0, 40.0)).corner_radius(10.0));
                if running {
                    let c = if tr.ar { resp.rect.left_top() + egui::vec2(8.0, 8.0) } else { resp.rect.right_top() + egui::vec2(-8.0, 8.0) };
                    ui.painter().circle_filled(c, 4.5, if selected { Color32::WHITE } else { ACCENT });
                }
                if resp.clicked() {
                    switch_to = Some(kind);
                }
            }
            if self.cfg.profiles.len() > 1 {
                tr.far(ui, |ui| {
                    egui::ComboBox::from_id_salt("profile")
                        .selected_text(tr.d(&self.cfg.profile().name))
                        .width(150.0)
                        .show_ui(ui, |ui| {
                            for (i, p) in self.cfg.profiles.iter().enumerate() {
                                if ui.selectable_label(i == self.cfg.active_profile, tr.d(&p.name)).clicked() {
                                    profile_to = Some(i);
                                }
                            }
                        });
                    ui.label(tr.muted(ui, "Profile", "الملف"));
                });
            }
        });
        if let Some(k) = switch_to {
            self.tab = k;
        }
        if let Some(i) = profile_to.filter(|i| *i != self.cfg.active_profile) {
            self.switch_profile(i);
        }
    }

    fn footer(&mut self, ui: &mut egui::Ui) {
        // Kept exactly as designed: left-to-right, with the developer's contact details.
        let tr = self.tr();
        ui.horizontal_wrapped(|ui| {
            let muted = muted_color(ui);
            ui.label(RichText::new(format!("© 2026 {DEVELOPER}")).color(muted));
            ui.separator();
            ui.label(RichText::new("Email").color(muted));
            ui.hyperlink_to(DEVELOPER_EMAIL, format!("mailto:{DEVELOPER_EMAIL}"));
            ui.separator();
            ui.label(RichText::new("Phone / WhatsApp").color(muted));
            ui.hyperlink_to(DEVELOPER_PHONE, format!("https://wa.me/{}", DEVELOPER_PHONE.trim_start_matches('+')));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mut hint = Vec::new();
                if self.cfg.hotkey_toggle_all.is_set() {
                    hint.push(format!("{} {}", self.cfg.hotkey_toggle_all.label(), tr.s("start/stop all", "تشغيل/إيقاف الكل")));
                }
                if self.cfg.hotkey_stop_all.is_set() {
                    hint.push(format!("{} {}", self.cfg.hotkey_stop_all.label(), tr.s("stop", "إيقاف")));
                }
                ui.label(tr.d(hint.join("  ·  ")).small().color(muted));
            });
        });
    }

    // ------------------------------------------------------------------ task list

    fn task_list(&mut self, ui: &mut egui::Ui) {
        let tr = self.tr();
        let kind = self.tab;
        let selected = self.selected_in(kind);
        let mut acts = Vec::new();
        let new_label = match kind {
            ClickerKind::Classic => tr.s("+  New classic clicker", "+  مهمة نقر كلاسيكي جديدة"),
            ClickerKind::Simple => tr.s("+  New clicker", "+  مهمة نقر جديدة"),
            ClickerKind::Advanced => tr.s("+  New advanced task", "+  مهمة متقدمة جديدة"),
            ClickerKind::Keyboard => tr.s("+  New typing task", "+  مهمة كتابة جديدة"),
        };
        ui.add_space(8.0);
        tr.column(ui, |ui| {
            if big_button(ui, tr.d(new_label), ACCENT_STRONG, ui.available_width()).clicked() {
                acts.push(Act::New(kind));
            }
        });
        ui.add_space(8.0);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
            .show(ui, |ui| {
                for c in self.cfg.clickers().iter().filter(|c| c.kind() == kind) {
                    let st = self.stats.get(&c.id);
                    let active = st.map_or(false, |s| s.status.is_active());
                    let is_sel = selected == Some(c.id);
                    let stroke = if is_sel { ACCENT } else { ui.visuals().widgets.noninteractive.bg_stroke.color };
                    let frame = egui::Frame::new()
                        .fill(theme::card_fill(ui))
                        .stroke(egui::Stroke::new(if is_sel { 2.0_f32 } else { 1.0_f32 }, stroke))
                        .corner_radius(10.0)
                        .inner_margin(egui::Margin::same(10));
                    let resp = frame
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            tr.row(ui, |ui| {
                                let (text, color) = status_text(tr, st.map(|s| &s.status), ui);
                                let (r, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                                ui.painter().circle_filled(r.center(), 6.0, color);
                                ui.label(semibold(tr.v(&c.name)));
                                tr.far(ui, |ui| {
                                    let (icon, col) = if active { ("■", DANGER) } else { ("▶", ACCENT) };
                                    if ui
                                        .add(egui::Button::new(RichText::new(icon).color(col)).min_size(egui::vec2(34.0, 30.0)))
                                        .on_hover_text(tr.v(if active { tr.s("Stop", "إيقاف") } else { tr.s("Start", "تشغيل") }))
                                        .clicked()
                                    {
                                        if c.kind() == ClickerKind::Classic || c.target.is_set() {
                                            acts.push(Act::Toggle(c.id));
                                        } else {
                                            acts.push(Act::Select(kind, c.id));
                                            acts.push(Act::Toast(
                                                "Choose a spot first.",
                                                "اختر المكان أولًا.",
                                                true,
                                            ));
                                        }
                                    }
                                    let _ = text;
                                });
                            });
                            let sub = if c.kind() == ClickerKind::Classic {
                                if c.hotkey.is_set() {
                                    format!("{}  ·  {}", c.hotkey.label(), tr.s("Follows cursor", "يتبع المؤشر"))
                                } else {
                                    tr.s("Follows mouse cursor", "يتبع مؤشر الماوس").to_string()
                                }
                            } else if c.target.is_set() {
                                if c.trigger.enabled { format!("🔔  {}", app_name(&c.target)) } else { app_name(&c.target) }
                            } else {
                                tr.s("No spot chosen yet", "لم يتم اختيار المكان بعد").to_string()
                            };
                            tr.row(ui, |ui| {
                                ui.label(tr.muted_d(ui, sub).small());
                                if let Some(s) = st.filter(|s| s.count > 0) {
                                    let n = if c.kind() == ClickerKind::Keyboard { s.rounds } else { s.count };
                                    tr.far(ui, |ui| ui.label(tr.muted_d(ui, format!("{n}")).small()));
                                }
                            });
                        })
                        .response;
                    let click = ui.interact(resp.rect, egui::Id::new(("task-card", c.id)), egui::Sense::click());
                    if click.clicked() {
                        acts.push(Act::Select(kind, c.id));
                    }
                    ui.add_space(6.0);
                }
            });
        self.apply_acts(acts);
    }

    fn empty_state(&mut self, ui: &mut egui::Ui) {
        let tr = self.tr();
        let kind = self.tab;
        let (title, body) = match kind {
            ClickerKind::Classic => (
                tr.s("Classic mouse autoclicker", "الأوتوكليكر الكلاسيكي للماوس"),
                tr.s(
                    "Clicks automatically at your mouse cursor's current position as you move it around. Press the shortcut key anywhere to start or stop clicking instantly.",
                    "ينقر تلقائيًا في المكان الحالي لمؤشر الماوس أثناء تحريكه بحرية. اضغط زر الاختصار من أي مكان لتشغيل النقر أو إيقافه فورًا.",
                ),
            ),
            ClickerKind::Simple => (
                tr.s("Click automatically on any spot", "انقر تلقائيًا على أي مكان"),
                tr.s(
                    "Choose a spot in any program, choose how fast, and press Start. ArkoClick keeps clicking even if the window is behind others or minimized, and your mouse stays free.",
                    "اختر مكانًا في أي برنامج، واختر السرعة، ثم اضغط تشغيل. يستمر ArkoClick في النقر حتى لو كانت النافذة خلف نوافذ أخرى أو مصغّرة، ويبقى الماوس حرًا لك.",
                ),
            ),
            ClickerKind::Advanced => (
                tr.s("Build a sequence of actions", "أنشئ سلسلة من الخطوات"),
                tr.s(
                    "Combine several clicks, key presses and text in order, with a wait after each one. For experienced users.",
                    "اجمع عدة نقرات وضغطات أزرار ونصوص بالترتيب، مع وقت انتظار بعد كل خطوة. مناسب للمستخدمين المتمرسين.",
                ),
            ),
            ClickerKind::Keyboard => (
                tr.s("Press keys or type text automatically", "اضغط أزرارًا أو اكتب نصًا تلقائيًا"),
                tr.s(
                    "Choose where to type, then pick a key to press or write a text. ArkoClick repeats it as often as you like, even in a hidden window.",
                    "اختر مكان الكتابة، ثم اختر زرًا للضغط عليه أو اكتب نصًا. يكرره ArkoClick كما تريد، حتى في نافذة مخفية.",
                ),
            ),
        };
        let new_label = match kind {
            ClickerKind::Classic => tr.s("+  Create a classic clicker", "+  أنشئ مهمة نقر كلاسيكي"),
            ClickerKind::Simple => tr.s("+  Create my first clicker", "+  أنشئ أول مهمة نقر"),
            ClickerKind::Advanced => tr.s("+  Create an advanced task", "+  أنشئ مهمة متقدمة"),
            ClickerKind::Keyboard => tr.s("+  Create a typing task", "+  أنشئ مهمة كتابة"),
        };
        ui.add_space(40.0);
        ui.vertical_centered(|ui| {
            ui.set_max_width(560.0);
            card(ui, tr, None, tr.d(title), |ui| {
                tr.para(ui, body, Some(muted_color(ui)), false);
                ui.add_space(12.0);
                if big_button(ui, tr.d(new_label), ACCENT_STRONG, 260.0).clicked() {
                    self.new_clicker(kind);
                }
            });
        });
    }

    // ------------------------------------------------------------------ shared editor parts

    /// Name, status and the big Start/Stop button at the top of every editor.
    pub(crate) fn editor_header(&mut self, ui: &mut egui::Ui, id: u64, acts: &mut Vec<Act>) {
        let tr = self.tr();
        let Some(idx) = self.index_of(id) else { return };
        let c = &self.cfg.clickers()[idx];
        let st = self.stats.get(&id).cloned();
        let active = st.as_ref().map_or(false, |s| s.status.is_active());
        let size = self.cfg.font_size;
        tr.row(ui, |ui| {
            ui.label(semibold(tr.v(&c.name)).size(size + 6.0));
            if ui.small_button("✎").on_hover_text(tr.v(tr.s("Rename", "تغيير الاسم"))).clicked() {
                acts.push(Act::EditText(TextTarget::ClickerName(id), c.name.clone(), false));
            }
            tr.far(ui, |ui| {
                let (label, fill) =
                    if active { (tr.s("■  Stop", "■  إيقاف"), DANGER) } else { (tr.s("▶  Start", "▶  تشغيل"), ACCENT_STRONG) };
                let ready = c.kind() == ClickerKind::Classic || c.target.is_set();
                let resp = ui.add_enabled(ready, |ui: &mut egui::Ui| big_button(ui, tr.d(label), fill, 150.0));
                if resp.clicked() {
                    acts.push(Act::Toggle(id));
                }
                if !ready {
                    resp.on_disabled_hover_text(tr.v(tr.s("Choose a spot first (step 1).", "اختر المكان أولًا (الخطوة 1).")));
                }
            });
        });
        tr.row(ui, |ui| {
            let (text, color) = status_text(tr, st.as_ref().map(|s| &s.status), ui);
            status_pill(ui, text, color);
            if let Some(s) = &st {
                let n = if c.kind() == ClickerKind::Keyboard { s.rounds } else { s.count };
                let unit = match c.kind() {
                    ClickerKind::Keyboard => tr.s("times", "مرة"),
                    ClickerKind::Classic | ClickerKind::Simple => tr.s("clicks", "نقرة"),
                    ClickerKind::Advanced => tr.s("actions", "خطوة"),
                };
                ui.label(tr.muted_d(ui, format!("·  {n} {unit}  ·  {}", fmt_duration(s.elapsed, tr))));
            }
            if c.hotkey.is_set() && !active {
                tr.far(ui, |ui| {
                    let hint = if tr.ar {
                        format!("أو اضغط {} للتشغيل/الإيقاف", c.hotkey.label())
                    } else {
                        format!("or press {} to start/stop", c.hotkey.label())
                    };
                    ui.label(tr.muted_d(ui, hint).small());
                });
            } else if self.cfg.hotkey_toggle_all.is_set() && !active {
                tr.far(ui, |ui| {
                    let hint = if tr.ar {
                        format!("أو اضغط {} لتشغيل/إيقاف الكل", self.cfg.hotkey_toggle_all.label())
                    } else {
                        format!("or press {} to start/stop everything", self.cfg.hotkey_toggle_all.label())
                    };
                    ui.label(tr.muted_d(ui, hint).small());
                });
            }
        });
        if let Some(d) = st.as_ref().and_then(|s| s.starts_in) {
            let secs = d.as_secs() + 1;
            let msg = if tr.ar {
                format!("يبدأ خلال {secs} ث، حرّك الماوس إلى المكان الذي تريد النقر عليه")
            } else {
                format!("Starting in {secs} s. Move the mouse to where you want to click.")
            };
            ui.label(tr.d(msg).color(WARN).size(self.cfg.font_size + 1.0));
        }
        if let Some(Status::Waiting(_)) = st.as_ref().map(|s| &s.status) {
            ui.label(tr.t(
                "The window is closed. Open the program again and ArkoClick will continue on its own.",
                "النافذة مغلقة. افتح البرنامج مرة أخرى وسيكمل ArkoClick تلقائيًا.",
            )
            .color(WARN));
        }
        ui.add_space(8.0);
    }

    /// The "When should it stop?" card, shared by every tab.
    pub(crate) fn stop_card(&mut self, ui: &mut egui::Ui, idx: usize, number: usize, unit_en: &str, unit_ar: &str) -> bool {
        let tr = self.tr();
        let mut changed = false;
        let c = &mut self.cfg.clickers_mut()[idx];
        card(ui, tr, Some(number), tr.t("When should it stop?", "متى يتوقف؟"), |ui| {
            #[derive(PartialEq, Clone, Copy)]
            enum Mode {
                Never,
                Count,
                Time,
            }
            let mode = if c.max_rounds > 0 {
                Mode::Count
            } else if c.max_seconds > 0 {
                Mode::Time
            } else {
                Mode::Never
            };
            let mut new_mode = mode;
            tr.row_wrap(ui, |ui| {
                if chip(ui, mode == Mode::Never, tr.t("When I press Stop", "عندما أضغط إيقاف")).clicked() {
                    new_mode = Mode::Never;
                }
                let count_label = if tr.ar { format!("بعد عدد من المرات") } else { "After a number of times".to_string() };
                if chip(ui, mode == Mode::Count, tr.d(count_label)).clicked() {
                    new_mode = Mode::Count;
                }
                if chip(ui, mode == Mode::Time, tr.t("After some time", "بعد مدة من الوقت")).clicked() {
                    new_mode = Mode::Time;
                }
            });
            if new_mode != mode {
                changed = true;
                match new_mode {
                    Mode::Never => {
                        c.max_rounds = 0;
                        c.max_seconds = 0;
                    }
                    Mode::Count => {
                        c.max_rounds = 100;
                        c.max_seconds = 0;
                    }
                    Mode::Time => {
                        c.max_rounds = 0;
                        c.max_seconds = 300;
                    }
                }
            }
            match new_mode {
                Mode::Count => {
                    tr.row(ui, |ui| {
                        ui.label(tr.t("Stop after", "توقف بعد"));
                        changed |= ui.add(egui::DragValue::new(&mut c.max_rounds).range(1..=100_000_000).speed(5)).changed();
                        ui.label(tr.t(unit_en, unit_ar));
                    });
                }
                Mode::Time => {
                    let mut minutes = c.max_seconds as f64 / 60.0;
                    tr.row(ui, |ui| {
                        ui.label(tr.t("Stop after", "توقف بعد"));
                        if ui.add(egui::DragValue::new(&mut minutes).range(0.1..=1440.0).speed(0.5).max_decimals(1)).changed() {
                            c.max_seconds = (minutes * 60.0).round().max(1.0) as u64;
                            changed = true;
                        }
                        ui.label(tr.t("minutes", "دقيقة"));
                    });
                }
                Mode::Never => {
                    ui.label(tr.muted(ui, "It keeps going until you press Stop.", "يستمر حتى تضغط إيقاف."));
                }
            }
        });
        changed
    }

    /// Collapsible "More options" shared by every tab.
    pub(crate) fn more_options(&mut self, ui: &mut egui::Ui, id: u64, acts: &mut Vec<Act>) -> bool {
        let tr = self.tr();
        let Some(idx) = self.index_of(id) else { return false };
        let open = self.expanded.contains(&id);
        let mut changed = false;
        let mut toggle = false;
        tr.row(ui, |ui| {
            let label = if open { tr.s("▾  Fewer options", "▾  خيارات أقل") } else { tr.s("▸  More options", "▸  خيارات أكثر") };
            if ui.add(egui::Button::new(tr.d(label)).frame(false)).clicked() {
                toggle = true;
            }
        });
        if toggle {
            if open {
                self.expanded.remove(&id);
            } else {
                self.expanded.insert(id);
            }
        }
        if !open {
            return false;
        }
        let capture = self.capture;
        let live = self.live_root(&self.cfg.clickers()[idx]);
        let ghosted = live.map_or(false, ghost::is_ghosted);
        let c = &mut self.cfg.clickers_mut()[idx];
        let kind = c.kind();
        card(ui, tr, None, tr.t("More options", "خيارات إضافية"), |ui| {
            tr.row(ui, |ui| {
                ui.label(tr.t("Shortcut key for this task:", "زر اختصار لهذه المهمة:"));
                let recording = capture == Some(Capture::Clicker(id));
                let text = if recording {
                    tr.t("Press the keys…  (Esc cancels)", "اضغط الأزرار…  (Esc للإلغاء)").color(WARN)
                } else if c.hotkey.is_set() {
                    RichText::new(c.hotkey.label())
                } else {
                    tr.t("None", "لا يوجد")
                };
                if ui.add(egui::Button::new(text).min_size(egui::vec2(120.0, 30.0))).clicked() {
                    acts.push(Act::Capture(Capture::Clicker(id)));
                }
            });
            changed |= switch(ui, tr, &mut c.auto_run, tr.t("Start automatically when ArkoClick opens", "ابدأ تلقائيًا عند فتح ArkoClick"));
            changed |= switch(
                ui,
                tr,
                &mut c.hide_when_minimized,
                tr.t("Keep working when the window is minimized", "استمر في العمل عند تصغير النافذة"),
            );
            if kind != ClickerKind::Keyboard && kind != ClickerKind::Classic {
                let mut game = c.mode == ClickMode::Foreground;
                if switch(
                    ui,
                    tr,
                    &mut game,
                    tr.t("Game mode (moves the real mouse; only if clicks don't work)", "وضع الألعاب (يحرك الماوس الحقيقي؛ فقط إذا لم تنجح النقرات)"),
                ) {
                    c.mode = if game { ClickMode::Foreground } else { ClickMode::Background };
                    changed = true;
                }
                tr.row(ui, |ui| {
                    ui.label(tr.t("Random variation in timing ±", "تغيير عشوائي في التوقيت ±"));
                    changed |= ui.add(egui::DragValue::new(&mut c.jitter_ms).range(0..=60_000)).changed();
                    ui.label(tr.t("ms", "ملي ثانية"));
                });
            }
            tr.row_wrap(ui, |ui| {
                let hide_label =
                    if ghosted { tr.t("Show the window", "إظهار النافذة") } else { tr.t("Hide the window (keeps working)", "إخفاء النافذة (تستمر في العمل)") };
                if ui.add_enabled(live.is_some(), egui::Button::new(hide_label)).clicked() {
                    acts.push(Act::Ghost(id));
                }
                if ui.button(tr.t("🗑  Delete this task", "🗑  حذف هذه المهمة").color(DANGER)).clicked() {
                    acts.push(Act::Confirm(Confirm::DeleteClicker(id)));
                }
            });
        });
        changed
    }

    /// "Where" card content once a spot is picked: picture, window name, actions.
    pub(crate) fn spot_summary(
        ui: &mut egui::Ui,
        tr: Tr,
        tex: Option<&TextureHandle>,
        target: &TargetInfo,
        found: bool,
        ghosted: bool,
        add_buttons: impl FnOnce(&mut egui::Ui),
    ) {
        tr.row(ui, |ui| {
            spot_picture(ui, tex, 104.0);
            ui.add_space(6.0);
            tr.column(ui, |ui| {
                ui.label(semibold(tr.v(&app_name(target))));
                ui.label(tr.muted_d(ui, truncate(&target.title, 48)).small());
                if ghosted {
                    ui.label(tr.t("Hidden, still working", "مخفية وتعمل").color(INFO));
                } else if found {
                    ui.label(tr.t("✓ Window is open", "✓ النافذة مفتوحة").color(ACCENT));
                } else {
                    ui.label(tr.t("Window is closed right now", "النافذة مغلقة الآن").color(WARN));
                }
                tr.row_wrap(ui, add_buttons);
            });
        });
    }

    /// Keep the window watcher's linked-task rules in step with the config, and apply
    /// "don't ask again" answers given in the prompt.
    fn sync_links(&mut self, ctx: &Context) {
        let disabled = crate::win::watch::take_disabled();
        if !disabled.is_empty() {
            for c in self.cfg.clickers_mut().iter_mut().filter(|c| disabled.contains(&c.id)) {
                c.trigger.enabled = false;
            }
            self.mark_dirty();
        }
        let rules = crate::triggers::build_rules(self.cfg.clickers());
        let state = (rules, self.tr().ar, ctx.style().visuals.dark_mode);
        if self.last_links.as_ref() != Some(&state) {
            crate::win::watch::set_links(state.0.clone(), state.1, state.2);
            self.last_links = Some(state);
        }
    }

    /// "Ask me to start this task when … opens" switch + title text, shown in the
    /// "where" card once a window is chosen. Edits `trig`; returns true when it changed.
    pub(crate) fn trigger_ui(
        ui: &mut egui::Ui,
        tr: Tr,
        id: u64,
        trig: &mut crate::config::Trigger,
        target: &TargetInfo,
        acts: &mut Vec<Act>,
    ) -> bool {
        let mut changed = false;
        ui.add_space(6.0);
        let label = if target.is_browser() {
            tr.t("🔔  Ask me to start this task when this page opens", "🔔  اسألني لتشغيل هذه المهمة عند فتح هذه الصفحة")
        } else if tr.ar {
            tr.d(format!("🔔  اسألني لتشغيل هذه المهمة عند فتح {}", app_name(target)))
        } else {
            tr.d(format!("🔔  Ask me to start this task when {} opens", app_name(target)))
        };
        let mut on = trig.enabled;
        if switch(ui, tr, &mut on, label) {
            trig.enabled = on;
            if on && trig.process.is_empty() {
                trig.process = target.process.clone();
            }
            if on && trig.title_contains.is_empty() {
                trig.title_contains = crate::config::suggested_title_hint(target);
            }
            changed = true;
        }
        if trig.enabled {
            tr.row_wrap(ui, |ui| {
                ui.label(tr.muted(ui, "Window title contains:", "عنوان النافذة يحتوي على:"));
                let shown = if trig.title_contains.trim().is_empty() {
                    tr.t("(any window of this program)", "(أي نافذة لهذا البرنامج)")
                } else {
                    semibold(tr.v(&trig.title_contains))
                };
                ui.label(shown);
                if ui.small_button(tr.t("✎  Change", "✎  تغيير")).clicked() {
                    acts.push(Act::EditText(TextTarget::TriggerTitle(id), trig.title_contains.clone(), false));
                }
            });
            // Wrapped by `para` (egui would cut a long Arabic line instead of wrapping it).
            let hint = tr.s(
                "When it comes to the front, a small box near the clock asks whether to start. For browser pages, use a word from the page title shown on its tab (e.g. YouTube).",
                "عند ظهورها في المقدمة يظهر مربع صغير بجوار الساعة يسألك هل تريد التشغيل. لصفحات المتصفح استخدم كلمة من عنوان الصفحة الظاهر على التبويب (مثل YouTube).",
            );
            tr.para(ui, hint, Some(muted_color(ui)), true);
        }
        changed
    }

    // ------------------------------------------------------------------ dialogs

    fn confirm_dialog(&mut self, ctx: &Context) {
        let Some(what) = self.confirm else { return };
        let tr = self.tr();
        let (title, body) = match what {
            Confirm::DeleteClicker(id) => {
                let name = self.cfg.clickers().iter().find(|c| c.id == id).map(|c| c.name.clone()).unwrap_or_default();
                (
                    tr.s("Delete task", "حذف المهمة").to_string(),
                    if tr.ar { format!("هل تريد حذف \"{name}\"؟") } else { format!("Delete \"{name}\"?") },
                )
            }
            Confirm::DeleteProfile(i) => {
                let p = &self.cfg.profiles[i.min(self.cfg.profiles.len() - 1)];
                (
                    tr.s("Delete profile", "حذف الملف").to_string(),
                    if tr.ar {
                        format!("هل تريد حذف الملف \"{}\" وكل مهامه ({})؟", p.name, p.clickers.len())
                    } else {
                        format!("Delete profile \"{}\" and its {} tasks?", p.name, p.clickers.len())
                    },
                )
            }
        };
        let mut close = false;
        let mut confirmed = false;
        egui::Window::new(semibold(tr.v(&title)))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                tr.column(ui, |ui| {
                    ui.label(tr.d(&body));
                    ui.add_space(8.0);
                    tr.row(ui, |ui| {
                        if ui.add(egui::Button::new(tr.t("Delete", "حذف").color(Color32::WHITE)).fill(DANGER)).clicked() {
                            confirmed = true;
                        }
                        if ui.button(tr.t("Cancel", "إلغاء")).clicked() {
                            close = true;
                        }
                    });
                });
            });
        if confirmed {
            match what {
                Confirm::DeleteClicker(id) => {
                    self.engine.stop(id);
                    self.cfg.clickers_mut().retain(|c| c.id != id);
                    self.expanded.remove(&id);
                    self.mark_dirty();
                }
                Confirm::DeleteProfile(i) => {
                    if self.cfg.profiles.len() > 1 && i < self.cfg.profiles.len() {
                        self.engine.stop_all();
                        self.cfg.profiles.remove(i);
                        let next = self.cfg.active_profile.min(self.cfg.profiles.len() - 1);
                        self.switch_profile(next);
                    }
                }
            }
            close = true;
        }
        if close {
            self.confirm = None;
        }
    }

    fn toast_ui(&mut self, ctx: &Context) {
        let tr = self.tr();
        if let Some((msg, at, is_err)) = &self.toast {
            if at.elapsed() > Duration::from_secs(5) {
                self.toast = None;
                return;
            }
            let color = if *is_err { DANGER } else { ACCENT };
            egui::Area::new(egui::Id::new("toast"))
                .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -56.0))
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style()).fill(ui.visuals().extreme_bg_color).inner_margin(egui::Margin::symmetric(16, 10)).show(ui, |ui| {
                        ui.label(tr.d(msg).color(color));
                    });
                });
            ctx.request_repaint_after(Duration::from_millis(500));
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        if std::env::var_os("ARKOCLICK_FRAME_DEBUG").is_some() {
            self.frame_counter += 1;
            if self.frame_counter % 100 == 0 {
                log::info!("frame {} causes: {:?}", self.frame_counter, ctx.repaint_causes());
            }
        }
        if tray::QUIT_REQUESTED.load(Ordering::Acquire) {
            self.quit(ctx);
            return;
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            if self.cfg.close_to_tray {
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                self.hide_to_tray(ctx);
            } else {
                self.quit(ctx);
            }
            return;
        }
        self.dark = ctx.style().visuals.dark_mode;

        self.handle_capture(ctx);

        self.stats = self.engine.stats();
        let running = self.stats.values().filter(|s| s.status.is_active()).count();
        let tip = if running > 0 { format!("{APP_TITLE} - {running} running") } else { APP_TITLE.to_string() };
        if tip != self.last_tooltip {
            tray::set_tooltip(&tip);
            self.last_tooltip = tip;
        }
        let bindings = self.current_bindings();
        if self.last_bindings.as_ref() != Some(&bindings) {
            hotkey::rebind(bindings.clone());
            self.last_bindings = Some(bindings);
        }

        self.poll_picker(ctx);
        self.poll_text_dialog(ctx);
        self.poll_updates(ctx);
        self.sync_links(ctx);
        self.refresh_windows(false);

        let rtl = self.tr().ar;
        let strip = if ctx.style().visuals.dark_mode { egui::Color32::from_rgb(0x12, 0x15, 0x1B) } else { egui::Color32::WHITE };
        egui::TopBottomPanel::top("header")
            .frame(egui::Frame::new().fill(strip).inner_margin(egui::Margin::symmetric(16, 10)))
            .show(ctx, |ui| self.header(ui, ctx));
        if self.update_banner_visible() {
            let fill = if ctx.style().visuals.dark_mode { egui::Color32::from_rgb(0x1B, 0x33, 0x3A) } else { egui::Color32::from_rgb(0xE3, 0xF3, 0xF0) };
            egui::TopBottomPanel::top("update-banner")
                .frame(egui::Frame::new().fill(fill).inner_margin(egui::Margin::symmetric(16, 8)))
                .show(ctx, |ui| self.update_banner(ui, ctx));
        }
        egui::TopBottomPanel::top("tabs")
            .frame(egui::Frame::side_top_panel(&ctx.style()).inner_margin(egui::Margin::symmetric(14, 8)))
            .show(ctx, |ui| self.tabs(ui));
        egui::TopBottomPanel::bottom("footer")
            .frame(egui::Frame::new().fill(strip).inner_margin(egui::Margin::symmetric(16, 8)))
            .show(ctx, |ui| self.footer(ui));

        let has_tasks = self.cfg.clickers().iter().any(|c| c.kind() == self.tab);
        if has_tasks {
            let panel = if rtl { egui::SidePanel::right("list") } else { egui::SidePanel::left("list") };
            panel.resizable(false).exact_width(260.0).show(ctx, |ui| self.task_list(ui));
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            if !has_tasks {
                self.empty_state(ui);
                return;
            }
            let Some(id) = self.selected_in(self.tab) else { return };
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                .show(ui, |ui| {
                    ui.add_space(4.0);
                    let tr = self.tr();
                    tr.column(ui, |ui| match self.tab {
                        ClickerKind::Classic => self.classic_editor(ui, id),
                        ClickerKind::Simple => self.simple_editor(ui, id),
                        ClickerKind::Advanced => self.advanced_editor(ui, id),
                        ClickerKind::Keyboard => self.keyboard_editor(ui, id),
                    });
                });
        });
        if self.show_settings {
            self.settings_window(ctx);
        }
        self.confirm_dialog(ctx);
        self.toast_ui(ctx);

        if let Some(t) = self.dirty_since {
            if t.elapsed() > SAVE_DEBOUNCE {
                self.flush_save();
            } else {
                ctx.request_repaint_after(SAVE_DEBOUNCE);
            }
        }
        if running > 0 {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.flush_save();
        self.engine.stop_all();
        ghost::show_all();
        tray::shutdown();
    }
}

// ---------------------------------------------------------------------- helpers

pub(crate) fn status_text(tr: Tr, s: Option<&Status>, ui: &egui::Ui) -> (RichText, Color32) {
    match s {
        Some(Status::Running) => (tr.t("Working", "يعمل الآن"), ACCENT),
        Some(Status::Waiting(_)) => (tr.t("Waiting for the window", "بانتظار النافذة"), WARN),
        Some(Status::Finished) => (tr.t("Finished", "انتهى"), INFO),
        Some(Status::Error(_)) => (tr.t("Needs attention", "يحتاج إلى انتباه"), DANGER),
        _ => (tr.t("Stopped", "متوقف"), muted_color(ui)),
    }
}

/// A friendly program name: "Google Chrome" from "Some page - Google Chrome".
pub(crate) fn app_name(t: &TargetInfo) -> String {
    if let Some(last) = t.title.rsplit(" - ").next().filter(|s| !s.trim().is_empty() && t.title.contains(" - ")) {
        return last.trim().to_string();
    }
    if !t.title.trim().is_empty() {
        return truncate(t.title.trim(), 40);
    }
    let p = t.process.trim_end_matches(".exe").trim_end_matches(".EXE");
    let mut chars = p.chars();
    match chars.next() {
        Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

pub(crate) fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let t: String = s.chars().take(n).collect();
        format!("{t}…")
    }
}

pub(crate) fn fmt_duration(d: Duration, tr: Tr) -> String {
    let s = d.as_secs();
    let (h, m, sec) = (s / 3600, (s % 3600) / 60, s % 60);
    if tr.ar {
        if h > 0 {
            format!("{h} س {m:02} د")
        } else if m > 0 {
            format!("{m} د {sec:02} ث")
        } else {
            format!("{sec} ث")
        }
    } else if h > 0 {
        format!("{h}h {m:02}m")
    } else if m > 0 {
        format!("{m}m {sec:02}s")
    } else {
        format!("{sec}s")
    }
}
