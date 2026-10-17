//! System-wide window event watcher (WinEvent hook, event-driven, zero polling):
//!
//! - When the user minimizes a window that a running clicker targets (and the clicker has
//!   "keep running when minimized" on), turn the minimize into an invisible-but-running
//!   state, because many apps (all Chromium browsers) stop handling input while minimized.
//! - When a hidden window gets activated (taskbar click, Alt+Tab), make it visible again
//!   so it behaves like a normal restore.
//! - When a program / browser page that saved tasks are linked to comes to the front (or the
//!   front window's title changes to it, e.g. a browser tab switch), offer to start them.

use super::{ghost, prompt, window, SafeHwnd};
use crate::engine::Engine;
use crate::triggers::{Cooldowns, Rule};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, TranslateMessage, EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MINIMIZESTART, MSG,
    WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
};

static ENGINE: OnceLock<Arc<Engine>> = OnceLock::new();
static ON_CHANGE: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();
const OBJID_WINDOW: i32 = 0;
const CHILDID_SELF: i32 = 0;
const EVENT_OBJECT_NAMECHANGE: u32 = 0x800C;

/// Linked-task state shared with the UI thread.
#[derive(Default)]
struct Links {
    rules: Vec<Rule>,
    arabic: bool,
    dark: bool,
    cooldowns: Cooldowns,
    /// Tasks the user answered "don't ask again" for; the UI turns their link off.
    disabled: Vec<u64>,
    /// Process names by pid (title changes are frequent; avoid reopening the process).
    names: HashMap<u32, String>,
}

static LINKS: Mutex<Option<Links>> = Mutex::new(None);

fn with_links<R>(f: impl FnOnce(&mut Links) -> R) -> R {
    let mut g = LINKS.lock().unwrap();
    f(g.get_or_insert_with(Links::default))
}

/// Publish the current rules and prompt look. Called by the UI when they change.
pub fn set_links(rules: Vec<Rule>, arabic: bool, dark: bool) {
    with_links(|l| {
        l.rules = rules;
        l.arabic = arabic;
        l.dark = dark;
    });
}

/// Tasks whose link the user turned off from the prompt since the last call.
pub fn take_disabled() -> Vec<u64> {
    with_links(|l| std::mem::take(&mut l.disabled))
}

/// A window came to the front or the front window's title changed: ask about linked tasks.
fn check_links(h: SafeHwnd) {
    let Some(engine) = ENGINE.get() else { return };
    if prompt::is_open() {
        return;
    }
    let pid = window::pid_of(h);
    let found = with_links(|l| {
        if l.rules.is_empty() {
            return None;
        }
        let process = l.names.entry(pid).or_insert_with(|| window::process_name(pid)).clone();
        let title = window::title_of(h);
        let now = Instant::now();
        let rule = l.rules.iter().find(|r| r.matches(&process, &title))?.clone();
        // Nothing to offer if every linked task is already running.
        let ids: Vec<u64> = rule.ids.iter().copied().filter(|id| !engine.is_active(*id)).collect();
        if ids.is_empty() || !l.cooldowns.should_ask(h.0, &rule, now) {
            return None;
        }
        l.cooldowns.mark(h.0, &rule, now);
        if l.names.len() > 256 {
            l.names.clear();
        }
        Some((rule, ids, l.arabic, l.dark))
    });
    let Some((rule, ids, arabic, dark)) = found else { return };
    let names: Vec<String> =
        rule.ids.iter().zip(&rule.names).filter(|(id, _)| ids.contains(id)).map(|(_, n)| n.clone()).collect();
    let message = match (arabic, names.len()) {
        (true, 1) => format!("«{}» مفتوح الآن. هل تريد تشغيل المهمة المحفوظة «{}»؟", rule.label, names[0]),
        (true, n) => format!("«{}» مفتوح الآن. هل تريد تشغيل {n} مهام محفوظة ({})؟", rule.label, names.join("، ")),
        (false, 1) => format!("\"{}\" is open. Start the saved task \"{}\"?", rule.label, names[0]),
        (false, n) => format!("\"{}\" is open. Start {n} saved tasks ({})?", rule.label, names.join(", ")),
    };
    let t = |en: &str, ar: &str| if arabic { ar.to_string() } else { en.to_string() };
    let spec = prompt::PromptSpec {
        title: "ArkoClick".into(),
        message,
        start: t("Start", "تشغيل"),
        later: t("Not now", "ليس الآن"),
        never: t("Don't ask again", "لا تسألني مرة أخرى"),
        rtl: arabic,
        dark,
    };
    let engine = Arc::clone(engine);
    prompt::ask(spec, move |choice| {
        match choice {
            prompt::Choice::Start => {
                for id in &ids {
                    engine.start_on(*id, h);
                }
            }
            prompt::Choice::Never => with_links(|l| {
                l.disabled.extend(&rule.ids);
                l.rules.retain(|r| r.key() != rule.key());
            }),
            prompt::Choice::NotNow => {}
        }
        if let Some(f) = ON_CHANGE.get() {
            f();
        }
    });
}

unsafe extern "system" fn on_event(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    _id_child: i32,
    _thread: u32,
    _time: u32,
) {
    if id_object != OBJID_WINDOW || hwnd.0.is_null() {
        return;
    }
    let h = SafeHwnd::from_hwnd(hwnd);
    if event == EVENT_OBJECT_NAMECHANGE {
        // Title of a top-level window changed: only interesting for the front window
        // (browser tab switch / navigation). Ignore controls and background windows.
        if _id_child != CHILDID_SELF || windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() != hwnd {
            return;
        }
        check_links(h);
        return;
    }
    match event {
        EVENT_SYSTEM_MINIMIZESTART => {
            let Some(engine) = ENGINE.get() else { return };
            if !engine.wants_auto_hide(h.0) || ghost::is_ghosted(h) {
                return;
            }
            // The event fires as the minimize *starts*; let it finish, then restore the
            // window invisibly. Off the hook thread so the hook never blocks.
            let engine = Arc::clone(engine);
            std::thread::spawn(move || {
                for _ in 0..20 {
                    std::thread::sleep(Duration::from_millis(50));
                    if window::is_iconic(h) {
                        break;
                    }
                }
                if engine.wants_auto_hide(h.0) && !ghost::is_ghosted(h) {
                    if let Err(e) = ghost::hide_auto(h) {
                        log::warn!("auto-hide failed: {e}");
                    }
                }
            });
        }
        EVENT_SYSTEM_FOREGROUND => {
            log::debug!("watch: foreground 0x{:X} ghosted={}", h.0, ghost::is_ghosted(h));
            if ghost::is_ghosted(h) {
                ghost::show(h);
            }
            check_links(h);
        }
        _ => {}
    }
}

pub fn spawn(engine: Arc<Engine>, on_change: impl Fn() + Send + Sync + 'static) {
    let _ = ENGINE.set(engine);
    let _ = ON_CHANGE.set(Box::new(on_change));
    std::thread::Builder::new()
        .name("window-watch".into())
        .spawn(|| unsafe {
            let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
            let a = SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, Some(on_event), 0, 0, flags);
            let b = SetWinEventHook(EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MINIMIZESTART, None, Some(on_event), 0, 0, flags);
            let c = SetWinEventHook(EVENT_OBJECT_NAMECHANGE, EVENT_OBJECT_NAMECHANGE, None, Some(on_event), 0, 0, flags);
            if a.is_invalid() || b.is_invalid() || c.is_invalid() {
                log::warn!("window-watch: SetWinEventHook failed");
            }
            // Out-of-context hooks are delivered through this thread's message queue.
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        })
        .expect("spawn window-watch thread");
}
