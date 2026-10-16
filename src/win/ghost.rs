//! "Ghost" a window: make it invisible and click-through *without minimizing it*.
//!
//! Many apps stop processing input or pause their logic while minimized. Chromium is the
//! worst case: a minimized Chrome/Edge window drops most posted clicks. A ghosted window is
//! still "shown" as far as the app is concerned (not minimized, not occluded because we
//! also keep it topmost), so it renders and handles every click, yet the user cannot see
//! it or accidentally click it.
//!
//! Every change is recorded in `%APPDATA%\ArkoClick\ghosted.json` so that if ArkoClick
//! crashes, the next launch restores the windows instead of leaving them invisible.

use super::SafeHwnd;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use windows::Win32::Foundation::COLORREF;
use windows::Win32::UI::WindowsAndMessaging::{
    GetLayeredWindowAttributes, GetWindowLongPtrW, IsIconic, IsWindow, SetLayeredWindowAttributes, SetWindowLongPtrW,
    SetWindowPos, ShowWindow, GWL_EXSTYLE, HWND_NOTOPMOST, HWND_TOPMOST, LAYERED_WINDOW_ATTRIBUTES_FLAGS, LWA_ALPHA,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_SHOWNOACTIVATE, WS_EX_LAYERED, WS_EX_TOPMOST, WS_EX_TRANSPARENT,
};

/// Alpha 1/255 rather than 0: fully transparent layered windows are treated specially by
/// some compositors and apps; 1 is invisible to the eye but still "drawn".
const GHOST_ALPHA: u8 = 1;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct Saved {
    hwnd: isize,
    ex_style: u32,
    /// The app's own layered attributes if it was already layered: (key, alpha, flags).
    layered: Option<(u32, u8, u32)>,
    /// Hidden automatically because the user minimized it while a clicker was running;
    /// goes back to minimized when the clicker stops.
    #[serde(default)]
    auto: bool,
}

static GHOSTS: Mutex<Option<HashMap<isize, Saved>>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut HashMap<isize, Saved>) -> R) -> R {
    let mut g = GHOSTS.lock().unwrap();
    f(g.get_or_insert_with(HashMap::new))
}

fn store_path() -> PathBuf {
    let mut p = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    p.push(crate::config::APP_TITLE);
    p.push("ghosted.json");
    p
}

fn persist(map: &HashMap<isize, Saved>) {
    let path = store_path();
    if map.is_empty() {
        let _ = std::fs::remove_file(&path);
        return;
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let list: Vec<&Saved> = map.values().collect();
    if let Ok(text) = serde_json::to_string(&list) {
        let _ = std::fs::write(path, text);
    }
}

pub fn is_ghosted(h: SafeHwnd) -> bool {
    with(|m| m.contains_key(&h.0))
}

pub fn count() -> usize {
    with(|m| m.len())
}

/// Make `h` invisible + click-through, restore it if minimized, and keep it topmost so no
/// other window "occludes" it (Chromium throttles occluded windows too).
pub fn hide(h: SafeHwnd) -> Result<(), String> {
    hide_impl(h, false)
}

/// Same as `hide`, but marked as automatic (see `release_auto`).
pub fn hide_auto(h: SafeHwnd) -> Result<(), String> {
    hide_impl(h, true)
}

pub fn is_auto(h: SafeHwnd) -> bool {
    with(|m| m.get(&h.0).map_or(false, |s| s.auto))
}

/// Undo an automatic hide: restore the window's styles and put it back to minimized,
/// which is how the user left it. Manual hides are left alone.
pub fn release_auto(h: SafeHwnd) {
    let saved = with(|m| {
        if m.get(&h.0).map_or(false, |s| s.auto) {
            let s = m.remove(&h.0);
            persist(m);
            s
        } else {
            None
        }
    });
    if let Some(s) = saved {
        restore(&s);
        unsafe {
            let _ = ShowWindow(h.hwnd(), windows::Win32::UI::WindowsAndMessaging::SW_SHOWMINNOACTIVE);
        }
    }
}

fn hide_impl(h: SafeHwnd, auto: bool) -> Result<(), String> {
    if is_ghosted(h) {
        return Ok(());
    }
    let hwnd = h.hwnd();
    unsafe {
        if !IsWindow(hwnd).as_bool() {
            return Err("window no longer exists".into());
        }
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let layered = if ex & WS_EX_LAYERED.0 != 0 {
            let (mut key, mut alpha, mut flags) = (COLORREF(0), 0u8, LAYERED_WINDOW_ATTRIBUTES_FLAGS(0));
            GetLayeredWindowAttributes(hwnd, Some(&mut key), Some(&mut alpha), Some(&mut flags))
                .ok()
                .map(|_| (key.0, alpha, flags.0))
        } else {
            None
        };
        let saved = Saved { hwnd: h.0, ex_style: ex, layered, auto };

        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, (ex | WS_EX_LAYERED.0 | WS_EX_TRANSPARENT.0) as isize);
        SetLayeredWindowAttributes(hwnd, COLORREF(0), GHOST_ALPHA, LWA_ALPHA).map_err(|e| {
            // Undo the style change if the window refused layering.
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex as isize);
            format!("window refused transparency: {e}")
        })?;
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
        let _ = SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        with(|m| {
            m.insert(h.0, saved);
            persist(m);
        });
        log::debug!("ghost: hid 0x{:X} (auto={auto})", h.0);
    }
    Ok(())
}

fn restore(s: &Saved) {
    let hwnd = SafeHwnd(s.hwnd).hwnd();
    unsafe {
        if !IsWindow(hwnd).as_bool() {
            return;
        }
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, s.ex_style as isize);
        if let Some((key, alpha, flags)) = s.layered {
            let _ = SetLayeredWindowAttributes(hwnd, COLORREF(key), alpha, LAYERED_WINDOW_ATTRIBUTES_FLAGS(flags));
        }
        let z = if s.ex_style & WS_EX_TOPMOST.0 != 0 { HWND_TOPMOST } else { HWND_NOTOPMOST };
        let _ = SetWindowPos(hwnd, z, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
}

pub fn show(h: SafeHwnd) {
    log::debug!("ghost: show 0x{:X}", h.0);
    let saved = with(|m| {
        let s = m.remove(&h.0);
        persist(m);
        s
    });
    if let Some(s) = saved {
        restore(&s);
    }
}

pub fn show_all() {
    let all: Vec<Saved> = with(|m| {
        let v = m.values().copied().collect();
        m.clear();
        persist(m);
        v
    });
    for s in &all {
        restore(s);
    }
}

/// Called at startup: undo ghosting left behind by a crashed previous run.
pub fn recover_after_crash() {
    let Ok(text) = std::fs::read_to_string(store_path()) else { return };
    let list: Vec<Saved> = serde_json::from_str(&text).unwrap_or_default();
    for s in &list {
        // Only touch the window if it still carries exactly our ghost styles; the handle
        // may have been recycled by an unrelated window.
        let ex = unsafe { GetWindowLongPtrW(SafeHwnd(s.hwnd).hwnd(), GWL_EXSTYLE) } as u32;
        let ours = WS_EX_LAYERED.0 | WS_EX_TRANSPARENT.0;
        if ex & ours == ours {
            log::warn!("restoring window 0x{:X} left hidden by a previous run", s.hwnd);
            restore(s);
        }
    }
    let _ = std::fs::remove_file(store_path());
}
