//! Global hotkeys on a dedicated thread. `RegisterHotKey` with a NULL window delivers
//! `WM_HOTKEY` to the registering thread's queue, so no hidden window is needed.
//! The UI publishes the wanted bindings (and a "suspended" flag while it records a new
//! key) into a shared slot and wakes the thread with a thread message.

use crate::config::Hotkey;
use crate::engine::Engine;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, PostThreadMessageW, TranslateMessage, MSG, WM_APP, WM_HOTKEY,
};

const ID_TOGGLE: i32 = 1;
const ID_STOP: i32 = 2;
const ID_CLICKER_BASE: i32 = 100;
const WM_REBIND: u32 = WM_APP + 7;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Bindings {
    pub toggle_all: Hotkey,
    pub stop_all: Hotkey,
    /// (clicker id, hotkey, clicker name for error messages)
    pub clickers: Vec<(u64, Hotkey, String)>,
}

#[derive(Default)]
struct Wanted {
    bindings: Bindings,
    suspended: bool,
}

static THREAD_ID: AtomicU32 = AtomicU32::new(0);
static WANTED: Mutex<Option<Wanted>> = Mutex::new(None);

fn wake() {
    let tid = THREAD_ID.load(Ordering::Acquire);
    if tid != 0 {
        unsafe {
            let _ = PostThreadMessageW(tid, WM_REBIND, WPARAM(0), LPARAM(0));
        }
    }
}

/// Publish new bindings. Safe from any thread.
pub fn rebind(b: Bindings) {
    WANTED.lock().unwrap().get_or_insert_with(Default::default).bindings = b;
    wake();
}

/// Temporarily release every hotkey (so the UI can record a key that is currently bound).
pub fn set_suspended(s: bool) {
    WANTED.lock().unwrap().get_or_insert_with(Default::default).suspended = s;
    wake();
}

fn mods(h: &Hotkey) -> HOT_KEY_MODIFIERS {
    let mut m = MOD_NOREPEAT;
    if h.ctrl {
        m |= MOD_CONTROL;
    }
    if h.alt {
        m |= MOD_ALT;
    }
    if h.shift {
        m |= MOD_SHIFT;
    }
    m
}

/// Unregister what we hold, then register what is wanted. Returns (id -> clicker map, failures).
fn apply(held: &mut Vec<i32>, w: &Wanted) -> (Vec<(i32, u64)>, Vec<String>) {
    unsafe {
        for id in held.drain(..) {
            let _ = UnregisterHotKey(None, id);
        }
    }
    let mut map = Vec::new();
    let mut failures = Vec::new();
    if w.suspended {
        return (map, failures);
    }
    let mut seen: Vec<Hotkey> = Vec::new();
    let mut reg = |id: i32, h: &Hotkey, what: &str, failures: &mut Vec<String>, held: &mut Vec<i32>| {
        if !h.is_set() {
            return false;
        }
        if seen.contains(h) {
            failures.push(format!("{} is used twice ({what})", h.label()));
            return false;
        }
        seen.push(*h);
        if unsafe { RegisterHotKey(None, id, mods(h), h.vk) }.is_ok() {
            held.push(id);
            true
        } else {
            failures.push(format!("{} ({what}) is taken by another app", h.label()));
            false
        }
    };
    reg(ID_TOGGLE, &w.bindings.toggle_all, "start/stop all", &mut failures, held);
    reg(ID_STOP, &w.bindings.stop_all, "emergency stop", &mut failures, held);
    for (i, (cid, h, name)) in w.bindings.clickers.iter().enumerate() {
        let id = ID_CLICKER_BASE + i as i32;
        if reg(id, h, name, &mut failures, held) {
            map.push((id, *cid));
        }
    }
    (map, failures)
}

/// Spawn the hotkey thread. Actions run directly against the engine, so they keep
/// working while the main window is hidden in the tray. `on_change` is called after
/// every action so the UI can repaint.
pub fn spawn(initial: Bindings, engine: Arc<Engine>, on_change: impl Fn() + Send + 'static) {
    WANTED.lock().unwrap().get_or_insert_with(Default::default).bindings = initial;
    std::thread::Builder::new()
        .name("hotkeys".into())
        .spawn(move || {
            THREAD_ID.store(unsafe { GetCurrentThreadId() }, Ordering::Release);
            let mut held: Vec<i32> = Vec::new();
            let mut map: Vec<(i32, u64)> = Vec::new();
            let refresh = |held: &mut Vec<i32>, map: &mut Vec<(i32, u64)>| {
                let g = WANTED.lock().unwrap();
                let Some(w) = g.as_ref() else { return };
                let (m, fails) = apply(held, w);
                *map = m;
                engine.set_notice(fails.join("; "));
                for f in fails {
                    log::warn!("hotkey: {f}");
                }
            };
            refresh(&mut held, &mut map);
            let mut msg = MSG::default();
            loop {
                let got = unsafe { GetMessageW(&mut msg, None, 0, 0) };
                if !got.as_bool() {
                    break;
                }
                match msg.message {
                    WM_HOTKEY => {
                        match msg.wParam.0 as i32 {
                            ID_TOGGLE => engine.toggle_all(),
                            ID_STOP => engine.stop_all(),
                            id => {
                                if let Some((_, cid)) = map.iter().find(|(i, _)| *i == id) {
                                    engine.toggle(*cid);
                                }
                            }
                        }
                        on_change();
                    }
                    WM_REBIND => {
                        refresh(&mut held, &mut map);
                        on_change();
                    }
                    _ => unsafe {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    },
                }
            }
        })
        .expect("spawn hotkey thread");
}
