//! Thin, documented wrappers over the Win32 calls we need. All `unsafe` lives here.

pub mod autostart;
pub mod ghost;
pub mod hotkey;
pub mod input;
pub mod picker;
pub mod prompt;
pub mod screen;
pub mod textdlg;
pub mod timer;
pub mod tray;
pub mod watch;
pub mod window;

use windows::Win32::Foundation::HWND;

/// `HWND` is a raw pointer and therefore `!Send`. The handle value itself is just an
/// opaque integer that is safe to move between threads, so we carry it as `isize`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SafeHwnd(pub isize);

impl SafeHwnd {
    pub fn from_hwnd(h: HWND) -> Self {
        Self(h.0 as isize)
    }
    pub fn hwnd(self) -> HWND {
        HWND(self.0 as *mut core::ffi::c_void)
    }
    pub fn is_null(self) -> bool {
        self.0 == 0
    }
}

pub fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
