//! Update notifications: background check (at startup, then daily), banner, Windows
//! notification, one-click download + install, and the Settings card.

use super::*;
use crate::update::{self, Release, Version};
use std::sync::atomic::AtomicBool;
use std::sync::{mpsc, Mutex};

/// First automatic check this long after start (don't slow down startup / boot).
const FIRST_CHECK_DELAY: Duration = Duration::from_secs(8);
const CHECK_EVERY: Duration = Duration::from_secs(24 * 3600);

pub(crate) enum UpdateMsg {
    Checked { result: Result<Release, String>, manual: bool },
    Downloaded(Result<std::path::PathBuf, String>),
}

#[derive(Default)]
pub(crate) enum UpdateState {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    Downloading(Release),
    Failed { message: String, release: Option<Release> },
}

/// What the background thread needs to announce an update on its own. The window may be
/// hidden in the tray, where the UI does not run, so the thread sends the notification.
#[derive(Clone, Default)]
pub(crate) struct NotifyPrefs {
    pub arabic: bool,
    pub skipped: Option<String>,
}

pub(crate) struct Updater {
    pub state: UpdateState,
    tx: mpsc::Sender<UpdateMsg>,
    rx: mpsc::Receiver<UpdateMsg>,
    /// Banner closed with × for this session.
    pub dismissed: bool,
    /// Mirrors the "check automatically" setting for the background thread.
    auto: Arc<AtomicBool>,
    prefs: Arc<Mutex<NotifyPrefs>>,
}

/// Windows notification for a newer release, unless the user skipped that version.
fn announce(rel: &Release, prefs: &NotifyPrefs, notified: &mut Option<Version>) {
    if prefs.skipped.as_deref() == Some(rel.tag.as_str()) || *notified == Some(rel.version) {
        return;
    }
    *notified = Some(rel.version);
    let (title, body) = if prefs.arabic {
        (format!("يتوفر إصدار جديد من ArkoClick ({})", rel.version_text()), "انقر لفتح ArkoClick والتحديث.")
    } else {
        (format!("ArkoClick {} is available", rel.version_text()), "Click to open ArkoClick and update.")
    };
    tray::notify(&title, body);
}

impl Updater {
    pub fn new(ctx: &Context, auto_enabled: bool, prefs: NotifyPrefs) -> Self {
        let (tx, rx) = mpsc::channel();
        let auto = Arc::new(AtomicBool::new(auto_enabled));
        let prefs = Arc::new(Mutex::new(prefs));
        let (t, a, c, p) = (tx.clone(), Arc::clone(&auto), ctx.clone(), Arc::clone(&prefs));
        std::thread::Builder::new()
            .name("update-check".into())
            .spawn(move || {
                let mut notified: Option<Version> = None;
                std::thread::sleep(FIRST_CHECK_DELAY);
                loop {
                    if a.load(Ordering::Relaxed) {
                        let result = update::fetch_latest();
                        if let Ok(rel) = &result {
                            if rel.version > update::current_version() {
                                let prefs = p.lock().unwrap().clone();
                                announce(rel, &prefs, &mut notified);
                            }
                        }
                        if t.send(UpdateMsg::Checked { result, manual: false }).is_err() {
                            return;
                        }
                        c.request_repaint();
                    }
                    std::thread::sleep(CHECK_EVERY);
                }
            })
            .expect("spawn update thread");
        Self { state: UpdateState::Idle, tx, rx, dismissed: false, auto, prefs }
    }

    /// Keep the thread's copy of language / skipped version current.
    pub fn set_prefs(&self, prefs: NotifyPrefs) {
        *self.prefs.lock().unwrap() = prefs;
    }

    pub fn set_auto(&self, on: bool) {
        self.auto.store(on, Ordering::Relaxed);
    }

    pub fn check_now(&mut self, ctx: &Context) {
        if matches!(self.state, UpdateState::Checking | UpdateState::Downloading(_)) {
            return;
        }
        self.state = UpdateState::Checking;
        self.dismissed = false;
        let (t, c) = (self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let result = update::fetch_latest();
            let _ = t.send(UpdateMsg::Checked { result, manual: true });
            c.request_repaint();
        });
    }

    pub fn download(&mut self, rel: Release, ctx: &Context) {
        self.state = UpdateState::Downloading(rel.clone());
        let (t, c) = (self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let r = update::download_setup(&rel);
            let _ = t.send(UpdateMsg::Downloaded(r));
            c.request_repaint();
        });
    }
}

impl App {
    /// Handle results from the update threads. Called once per frame.
    pub(crate) fn poll_updates(&mut self, ctx: &Context) {
        self.updater.set_prefs(NotifyPrefs { arabic: self.tr().ar, skipped: self.cfg.skipped_version.clone() });
        while let Ok(msg) = self.updater.rx.try_recv() {
            match msg {
                UpdateMsg::Checked { result: Ok(rel), manual } => {
                    if rel.version > update::current_version() {
                        // (The Windows notification was already sent by the check thread.)
                        if manual {
                            self.updater.dismissed = false;
                        }
                        self.updater.state = UpdateState::Available(rel);
                    } else {
                        self.updater.state = UpdateState::UpToDate;
                    }
                }
                UpdateMsg::Checked { result: Err(e), manual } => {
                    log::info!("update check failed: {e}");
                    if manual {
                        self.updater.state = UpdateState::Failed { message: e, release: None };
                    }
                }
                UpdateMsg::Downloaded(Ok(path)) => {
                    let release = match std::mem::take(&mut self.updater.state) {
                        UpdateState::Downloading(r) => Some(r),
                        _ => None,
                    };
                    match update::launch_installer(&path, self.tr().ar) {
                        // The installer waits for us to exit, installs, and reopens ArkoClick.
                        Ok(()) => self.quit(ctx),
                        Err(e) => self.updater.state = UpdateState::Failed { message: e, release },
                    }
                }
                UpdateMsg::Downloaded(Err(e)) => {
                    let release = match std::mem::take(&mut self.updater.state) {
                        UpdateState::Downloading(r) => Some(r),
                        _ => None,
                    };
                    self.updater.state = UpdateState::Failed { message: e, release };
                }
            }
        }
        if matches!(self.updater.state, UpdateState::Checking | UpdateState::Downloading(_)) {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
    }

    fn start_update(&mut self, rel: Release, ctx: &Context) {
        if update::is_installed_copy() && rel.setup_url.is_some() {
            self.updater.download(rel, ctx);
        } else {
            // Portable copy: there is no installed program to replace, so open the page.
            ctx.open_url(egui::OpenUrl::new_tab(rel.page.clone()));
        }
    }

    /// Does the banner have something to show this frame?
    pub(crate) fn update_banner_visible(&self) -> bool {
        match &self.updater.state {
            UpdateState::Available(r) => {
                !self.updater.dismissed && self.cfg.skipped_version.as_deref() != Some(r.tag.as_str())
            }
            UpdateState::Downloading(_) => true,
            UpdateState::Failed { release: Some(_), .. } => !self.updater.dismissed,
            _ => false,
        }
    }

    /// Strip under the header announcing a new version.
    pub(crate) fn update_banner(&mut self, ui: &mut egui::Ui, ctx: &Context) {
        let tr = self.tr();
        let current = update::fmt_version(update::current_version());
        let installed = update::is_installed_copy();
        let mut action: Option<&'static str> = None;
        match &self.updater.state {
            UpdateState::Available(rel) => {
                tr.row_wrap(ui, |ui| {
                    let msg = if tr.ar {
                        format!("يتوفر إصدار جديد {} (لديك {current})", rel.version_text())
                    } else {
                        format!("A new version, {}, is available (you have {current}).", rel.version_text())
                    };
                    ui.label(semibold(tr.v(&msg)));
                    let primary = if installed && rel.setup_url.is_some() {
                        tr.t("⟳  Download and install", "⟳  تنزيل وتثبيت")
                    } else {
                        tr.t("Open download page", "فتح صفحة التنزيل")
                    };
                    if ui.add(egui::Button::new(primary.color(Color32::WHITE)).fill(ACCENT_STRONG)).clicked() {
                        action = Some("install");
                    }
                    if ui.button(tr.t("What's new", "ما الجديد")).clicked() {
                        action = Some("notes");
                    }
                    if ui.button(tr.t("Skip this version", "تخطّي هذا الإصدار")).clicked() {
                        action = Some("skip");
                    }
                    if ui.small_button("×").on_hover_text(tr.v(tr.s("Later", "لاحقًا"))).clicked() {
                        action = Some("dismiss");
                    }
                });
            }
            UpdateState::Downloading(rel) => {
                tr.row(ui, |ui| {
                    ui.add(egui::Spinner::new());
                    let msg = if tr.ar {
                        format!("جارٍ تنزيل الإصدار {}… سيُغلق ArkoClick ويُفتح من جديد بعد التثبيت.", rel.version_text())
                    } else {
                        format!("Downloading version {}… ArkoClick will close and reopen once it's installed.", rel.version_text())
                    };
                    ui.label(tr.d(msg));
                });
            }
            UpdateState::Failed { message, release: Some(_) } => {
                tr.row_wrap(ui, |ui| {
                    ui.label(tr.t("The update couldn't be downloaded.", "تعذّر تنزيل التحديث.").color(DANGER));
                    ui.label(tr.muted_d(ui, message).small());
                    if ui.button(tr.t("Try again", "حاول مرة أخرى")).clicked() {
                        action = Some("retry");
                    }
                    if ui.button(tr.t("Open download page", "فتح صفحة التنزيل")).clicked() {
                        action = Some("page");
                    }
                    if ui.small_button("×").clicked() {
                        action = Some("dismiss");
                    }
                });
            }
            _ => {}
        }
        let rel = match &self.updater.state {
            UpdateState::Available(r) => Some(r.clone()),
            UpdateState::Failed { release, .. } => release.clone(),
            _ => None,
        };
        match (action, rel) {
            (Some("install") | Some("retry"), Some(r)) => self.start_update(r, ctx),
            (Some("notes") | Some("page"), Some(r)) => ctx.open_url(egui::OpenUrl::new_tab(r.page)),
            (Some("skip"), Some(r)) => {
                self.cfg.skipped_version = Some(r.tag);
                self.mark_dirty();
            }
            (Some("dismiss"), _) => self.updater.dismissed = true,
            _ => {}
        }
    }

    /// "Updates" card in Settings.
    pub(crate) fn updates_card(&mut self, ui: &mut egui::Ui, ctx: &Context) {
        let tr = self.tr();
        let mut check = false;
        let mut changed = false;
        let mut open_page: Option<String> = None;
        let current = update::fmt_version(update::current_version());
        card(ui, tr, None, tr.t("Updates", "التحديثات"), |ui| {
            if switch(
                ui,
                tr,
                &mut self.cfg.check_updates,
                tr.t("Check for new versions automatically", "تحقّق من الإصدارات الجديدة تلقائيًا"),
            ) {
                changed = true;
            }
            tr.row_wrap(ui, |ui| {
                let busy = matches!(self.updater.state, UpdateState::Checking | UpdateState::Downloading(_));
                if ui.add_enabled(!busy, egui::Button::new(tr.t("Check now", "تحقّق الآن"))).clicked() {
                    check = true;
                }
                let you_have = if tr.ar { format!("الإصدار الحالي {current}") } else { format!("You have {current}") };
                ui.label(tr.muted_d(ui, you_have));
            });
            match &self.updater.state {
                UpdateState::Checking => {
                    tr.row(ui, |ui| {
                        ui.add(egui::Spinner::new());
                        ui.label(tr.t("Checking…", "جارٍ التحقق…"));
                    });
                }
                UpdateState::UpToDate => {
                    ui.label(tr.t("✓ You have the latest version.", "✓ لديك أحدث إصدار.").color(ACCENT));
                }
                UpdateState::Available(r) => {
                    tr.row_wrap(ui, |ui| {
                        let msg = if tr.ar { format!("يتوفر الإصدار {}", r.version_text()) } else { format!("Version {} is available", r.version_text()) };
                        ui.label(tr.d(msg).color(INFO));
                        if ui.small_button(tr.t("What's new", "ما الجديد")).clicked() {
                            open_page = Some(r.page.clone());
                        }
                    });
                }
                UpdateState::Failed { message, .. } => {
                    ui.label(tr.t("Couldn't check for updates. Is the internet connected?", "تعذّر التحقق من التحديثات. هل الإنترنت متصل؟").color(WARN));
                    ui.label(tr.muted_d(ui, message).small());
                }
                _ => {}
            }
        });
        if changed {
            self.updater.set_auto(self.cfg.check_updates);
            self.mark_dirty();
        }
        if check {
            // A manual check also brings back a version that was skipped before.
            self.cfg.skipped_version = None;
            self.mark_dirty();
            self.updater.check_now(ctx);
        }
        if let Some(url) = open_page {
            ctx.open_url(egui::OpenUrl::new_tab(url));
        }
    }
}
