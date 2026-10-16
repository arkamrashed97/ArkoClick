//! System-wide window event watcher (WinEvent hook, event-driven, zero polling):
//!
//! - When the user minimizes a window that a running clicker targets (and the clicker has
//!   "keep running when minimized" on), turn the minimize into an invisible-but-running
//!   state, because many apps (all Chromium browsers) stop handling input while minimized.
//! - When a hidden window gets activated (taskbar click, Alt+Tab), make it visible again
//!   so it behaves like a normal restore.

use super::{ghost, window, SafeHwnd};
use crate::engine::Engine;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, TranslateMessage, EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MINIMIZESTART, MSG,
    WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
};

static ENGINE: OnceLock<Arc<Engine>> = OnceLock::new();
const OBJID_WINDOW: i32 = 0;

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
        }
        _ => {}
    }
}

pub fn spawn(engine: Arc<Engine>) {
    let _ = ENGINE.set(engine);
    std::thread::Builder::new()
        .name("window-watch".into())
        .spawn(|| unsafe {
            let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
            let a = SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, Some(on_event), 0, 0, flags);
            let b = SetWinEventHook(EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MINIMIZESTART, None, Some(on_event), 0, 0, flags);
            if a.is_invalid() || b.is_invalid() {
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
