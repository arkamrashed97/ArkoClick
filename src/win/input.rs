//! Click injection: PostMessage (background) and SendInput (foreground).

use super::screen::virtual_screen;
use super::SafeHwnd;
use crate::config::{ClickKind, MouseButton};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
    MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
    MOUSEEVENTF_VIRTUALDESK, MOUSEINPUT, MOUSE_EVENT_FLAGS,
};

use windows::Win32::UI::WindowsAndMessaging::{
    PostMessageW, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDBLCLK, WM_MBUTTONDOWN, WM_MBUTTONUP,
    WM_MOUSEMOVE, WM_RBUTTONDBLCLK, WM_RBUTTONDOWN, WM_RBUTTONUP,
};

// MK_* modifier flags carried in wParam of mouse messages (not exported by the windows crate).
const MK_LBUTTON: u32 = 0x0001;
const MK_RBUTTON: u32 = 0x0002;
const MK_MBUTTON: u32 = 0x0010;

fn make_lparam(x: i32, y: i32) -> LPARAM {
    LPARAM((((y as u32) & 0xFFFF) << 16 | ((x as u32) & 0xFFFF)) as isize)
}

/// Post a full click (move + down + up, plus dblclk + up for double) to a window's
/// client coordinates. Never touches the real cursor.
///
/// `WM_MOUSEMOVE` first matters: many controls (and all of WPF/WinUI) track hover state
/// and ignore a button-down that arrives without a preceding move.
pub fn post_click(h: SafeHwnd, x: i32, y: i32, button: MouseButton, kind: ClickKind) -> bool {
    let (down, up, dbl, mk) = match button {
        MouseButton::Left => (WM_LBUTTONDOWN, WM_LBUTTONUP, WM_LBUTTONDBLCLK, MK_LBUTTON),
        MouseButton::Right => (WM_RBUTTONDOWN, WM_RBUTTONUP, WM_RBUTTONDBLCLK, MK_RBUTTON),
        MouseButton::Middle => (WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MBUTTONDBLCLK, MK_MBUTTON),
    };
    let lp = make_lparam(x, y);
    let hwnd = h.hwnd();
    unsafe {
        let _ = PostMessageW(hwnd, WM_MOUSEMOVE, WPARAM(0), lp);
        let a = PostMessageW(hwnd, down, WPARAM(mk as usize), lp).is_ok();
        let b = PostMessageW(hwnd, up, WPARAM(0), lp).is_ok();
        if kind == ClickKind::Double {
            let _ = PostMessageW(hwnd, dbl, WPARAM(mk as usize), lp);
            let _ = PostMessageW(hwnd, up, WPARAM(0), lp);
        }
        a && b
    }
}

fn mouse_input(dx: i32, dy: i32, flags: MOUSE_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT { dx, dy, mouseData: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 },
        },
    }
}

/// Inject a real click at absolute screen coordinates (multi-monitor aware). The cursor
/// jumps to the point; this is what games that read raw input need.
pub fn send_input_click(x: i32, y: i32, button: MouseButton, kind: ClickKind) -> bool {
    let (vx, vy, vw, vh) = virtual_screen();
    if vw <= 1 || vh <= 1 {
        return false;
    }
    let dx = (((x - vx) as i64 * 65535) / (vw as i64 - 1)) as i32;
    let dy = (((y - vy) as i64 * 65535) / (vh as i64 - 1)) as i32;
    let (down, up) = match button {
        MouseButton::Left => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
        MouseButton::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
        MouseButton::Middle => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
    };
    let base = MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK;
    let mut inputs = vec![mouse_input(dx, dy, base | down), mouse_input(dx, dy, base | up)];
    if kind == ClickKind::Double {
        inputs.push(mouse_input(dx, dy, base | down));
        inputs.push(mouse_input(dx, dy, base | up));
    }
    let n = inputs.len() as u32;
    unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) == n }
}

/// Inject a real click at the mouse's current physical position on screen without moving it.
/// Windows sends down and up events wherever the cursor is located at this instant.
pub fn send_mouse_click_here(button: MouseButton, kind: ClickKind) -> bool {
    let (down, up) = match button {
        MouseButton::Left => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
        MouseButton::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
        MouseButton::Middle => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
    };
    let mut inputs = vec![mouse_input(0, 0, down), mouse_input(0, 0, up)];
    if kind == ClickKind::Double {
        inputs.push(mouse_input(0, 0, down));
        inputs.push(mouse_input(0, 0, up));
    }
    let n = inputs.len() as u32;
    unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) == n }
}


/// Post one key press (down + up) to a window. The target's own message loop runs
/// `TranslateMessage`, so printable keys also produce `WM_CHAR` there.
/// Modifier state cannot be faked this way (the target reads its own keyboard state),
/// which is why key steps take a single key.
pub fn post_key(h: SafeHwnd, vk: u32) -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{MapVirtualKeyW, MAPVK_VK_TO_VSC};
    use windows::Win32::UI::WindowsAndMessaging::{WM_KEYDOWN, WM_KEYUP};
    let scan = unsafe { MapVirtualKeyW(vk, MAPVK_VK_TO_VSC) } & 0xFF;
    // Navigation keys live on the extended part of the keyboard.
    let extended = matches!(vk, 0x21..=0x28 | 0x2D | 0x2E);
    let base: u32 = 1 | (scan << 16) | if extended { 1 << 24 } else { 0 };
    let down = LPARAM(base as isize);
    let up = LPARAM((base | 1 << 30 | 1 << 31) as isize);
    unsafe {
        let a = PostMessageW(h.hwnd(), WM_KEYDOWN, WPARAM(vk as usize), down).is_ok();
        let b = PostMessageW(h.hwnd(), WM_KEYUP, WPARAM(vk as usize), up).is_ok();
        a && b
    }
}

/// Type a string by posting `WM_CHAR` per UTF-16 unit (surrogate pairs included), so any
/// language works regardless of the keyboard layout.
pub fn post_text(h: SafeHwnd, text: &str) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::WM_CHAR;
    let mut ok = true;
    // Native edit boxes give "\r\n"; a single carriage return is what Enter sends.
    let text = text.replace("\r\n", "\r").replace('\n', "\r");
    for unit in text.encode_utf16() {
        ok &= unsafe { PostMessageW(h.hwnd(), WM_CHAR, WPARAM(unit as usize), LPARAM(1)).is_ok() };
    }
    ok
}
