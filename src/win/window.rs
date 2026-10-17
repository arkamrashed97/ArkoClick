//! Window lookup, geometry and coordinate mapping.

use super::SafeHwnd;
use windows::Win32::Foundation::{BOOL, CloseHandle, HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::Graphics::Gdi::{ClientToScreen, MapWindowPoints, ScreenToClient};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetAncestor, GetClassNameW, GetClientRect, GetWindowLongPtrW,
    GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible, SetForegroundWindow,
    SetWindowPos, ShowWindow, WindowFromPoint,     GA_ROOT, GWL_EXSTYLE, HWND_TOPMOST, SWP_NOACTIVATE, SW_HIDE, SW_RESTORE, SW_SHOW, WS_EX_TOOLWINDOW,
};
use windows::core::PWSTR;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInfo {
    pub hwnd: SafeHwnd,
    pub title: String,
    pub class: String,
    pub pid: u32,
    pub process: String,
}

impl WindowInfo {
    pub fn display(&self) -> String {
        if self.title.is_empty() {
            format!("[{}] ({})", self.class, self.process)
        } else {
            format!("{} ({})", self.title, self.process)
        }
    }
}

pub fn is_window(h: SafeHwnd) -> bool {
    !h.is_null() && unsafe { IsWindow(h.hwnd()).as_bool() }
}

pub fn is_iconic(h: SafeHwnd) -> bool {
    unsafe { IsIconic(h.hwnd()).as_bool() }
}

fn is_cloaked(h: HWND) -> bool {
    let mut cloaked: u32 = 0;
    unsafe {
        DwmGetWindowAttribute(
            h,
            DWMWA_CLOAKED,
            &mut cloaked as *mut u32 as *mut core::ffi::c_void,
            std::mem::size_of::<u32>() as u32,
        )
        .is_ok()
            && cloaked != 0
    }
}

pub fn title_of(h: SafeHwnd) -> String {
    let mut buf = [0u16; 512];
    let n = unsafe { GetWindowTextW(h.hwnd(), &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

pub fn class_of(h: SafeHwnd) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetClassNameW(h.hwnd(), &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

pub fn pid_of(h: SafeHwnd) -> u32 {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(h.hwnd(), Some(&mut pid)) };
    pid
}

/// Executable file name (e.g. `chrome.exe`) of a process, or empty when access is denied.
pub fn process_name(pid: u32) -> String {
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return String::new();
        };
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
        let _ = CloseHandle(handle);
        if ok.is_err() {
            return String::new();
        }
        let full = String::from_utf16_lossy(&buf[..len as usize]);
        full.rsplit(['\\', '/']).next().unwrap_or("").to_string()
    }
}

pub fn info_of(h: SafeHwnd) -> WindowInfo {
    let pid = pid_of(h);
    WindowInfo { hwnd: h, title: title_of(h), class: class_of(h), pid, process: process_name(pid) }
}

/// True when the mouse cursor is over one of ArkoClick's own windows. The classic clicker
/// never clicks there, so it can't press its own Stop / Start buttons.
pub fn cursor_over_own_process() -> bool {
    let (x, y) = super::screen::cursor_pos();
    root_at(x, y).map_or(false, |h| pid_of(h) == std::process::id())
}

/// Top-level ancestor of the window at a screen point (None when nothing is there).
pub fn root_at(x: i32, y: i32) -> Option<SafeHwnd> {
    unsafe {
        let h = WindowFromPoint(POINT { x, y });
        if h.0.is_null() {
            return None;
        }
        let root = GetAncestor(h, GA_ROOT);
        Some(SafeHwnd::from_hwnd(if root.0.is_null() { h } else { root }))
    }
}

pub fn screen_to_client(h: SafeHwnd, x: i32, y: i32) -> Option<(i32, i32)> {
    let mut pt = POINT { x, y };
    unsafe { ScreenToClient(h.hwnd(), &mut pt).as_bool() }.then_some((pt.x, pt.y))
}

pub fn client_to_screen(h: SafeHwnd, x: i32, y: i32) -> Option<(i32, i32)> {
    let mut pt = POINT { x, y };
    unsafe { ClientToScreen(h.hwnd(), &mut pt).as_bool() }.then_some((pt.x, pt.y))
}

/// Client-area size. Minimized windows report 0x0, callers must keep a cached size.
pub fn client_size(h: SafeHwnd) -> Option<(i32, i32)> {
    let mut rc = RECT::default();
    unsafe { GetClientRect(h.hwnd(), &mut rc).ok()? };
    Some((rc.right - rc.left, rc.bottom - rc.top))
}

/// Walk down from `root` to the deepest visible, enabled child under a client point.
/// Returns the child and the point converted to that child's client coordinates.
/// Classic Win32 apps route mouse messages per control, so posting to the root alone
/// would never reach a button.
pub fn deepest_child_at(root: SafeHwnd, x: i32, y: i32) -> (SafeHwnd, i32, i32) {
    let mut cur = root.hwnd();
    let mut pt = POINT { x, y };
    for _ in 0..32 {
        let Some(child) = child_at(cur, pt) else { break };
        let mut pts = [pt];
        unsafe { MapWindowPoints(cur, child, &mut pts) };
        pt = pts[0];
        cur = child;
    }
    (SafeHwnd::from_hwnd(cur), pt.x, pt.y)
}

/// Topmost visible, enabled direct child of `parent` containing a point in `parent`'s
/// client coordinates. Pure geometry over the z-ordered child list: unlike
/// `ChildWindowFromPointEx` it keeps working when the parent is layered/click-through
/// (our "hidden" mode) or minimized.
fn child_at(parent: HWND, pt: POINT) -> Option<HWND> {
    use windows::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled;
    use windows::Win32::UI::WindowsAndMessaging::{GetWindow, GW_CHILD, GW_HWNDNEXT};
    unsafe {
        let mut c = GetWindow(parent, GW_CHILD).ok()?;
        for _ in 0..4096 {
            if IsWindowVisible(c).as_bool() && IsWindowEnabled(c).as_bool() {
                let mut rc = RECT::default();
                if windows::Win32::UI::WindowsAndMessaging::GetWindowRect(c, &mut rc).is_ok() {
                    let mut corners = [POINT { x: rc.left, y: rc.top }, POINT { x: rc.right, y: rc.bottom }];
                    MapWindowPoints(None, parent, &mut corners);
                    let (l, t, r, b) = (corners[0].x, corners[0].y, corners[1].x, corners[1].y);
                    if pt.x >= l && pt.x < r && pt.y >= t && pt.y < b {
                        return Some(c);
                    }
                }
            }
            c = match GetWindow(c, GW_HWNDNEXT) {
                Ok(n) if !n.0.is_null() => n,
                _ => return None,
            };
        }
        None
    }
}

/// Offset of `child`'s client origin inside `root`'s client area.
pub fn child_offset(root: SafeHwnd, child: SafeHwnd) -> (i32, i32) {
    let mut pts = [POINT { x: 0, y: 0 }];
    unsafe { MapWindowPoints(child.hwnd(), root.hwnd(), &mut pts) };
    (pts[0].x, pts[0].y)
}

/// All visible, non-tool, non-cloaked top-level windows with a title.
pub fn enumerate() -> Vec<WindowInfo> {
    unsafe extern "system" fn cb(h: HWND, lparam: LPARAM) -> BOOL {
        let out = &mut *(lparam.0 as *mut Vec<WindowInfo>);
        if !IsWindowVisible(h).as_bool() || is_cloaked(h) {
            return BOOL(1);
        }
        let ex = GetWindowLongPtrW(h, GWL_EXSTYLE) as u32;
        if ex & WS_EX_TOOLWINDOW.0 != 0 {
            return BOOL(1);
        }
        let sh = SafeHwnd::from_hwnd(h);
        let title = title_of(sh);
        if title.is_empty() {
            return BOOL(1);
        }
        let pid = pid_of(sh);
        out.push(WindowInfo { hwnd: sh, title, class: class_of(sh), pid, process: process_name(pid) });
        BOOL(1)
    }
    let mut out: Vec<WindowInfo> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut out as *mut _ as isize));
    }
    out
}

/// Find a window matching saved target info. Class + process must match; the title is a
/// tie-breaker (titles change constantly in browsers, so we never require it).
pub fn find_matching(class: &str, process: &str, title: &str) -> Option<SafeHwnd> {
    let all = enumerate_all_toplevel();
    let mut best: Option<(u8, SafeHwnd)> = None;
    for w in all {
        if !class.is_empty() && w.class != class {
            continue;
        }
        if !process.is_empty() && !w.process.eq_ignore_ascii_case(process) {
            continue;
        }
        let score = if !title.is_empty() && w.title == title {
            3
        } else if !title.is_empty() && (w.title.contains(title) || title.contains(&w.title)) && !w.title.is_empty() {
            2
        } else {
            1
        };
        if best.map_or(true, |(s, _)| score > s) {
            best = Some((score, w.hwnd));
        }
    }
    best.map(|(_, h)| h)
}

/// Like `enumerate` but includes untitled and hidden windows (a minimized/hidden target
/// still has to be found when re-binding).
fn enumerate_all_toplevel() -> Vec<WindowInfo> {
    unsafe extern "system" fn cb(h: HWND, lparam: LPARAM) -> BOOL {
        let out = &mut *(lparam.0 as *mut Vec<WindowInfo>);
        let sh = SafeHwnd::from_hwnd(h);
        let pid = pid_of(sh);
        out.push(WindowInfo { hwnd: sh, title: title_of(sh), class: class_of(sh), pid, process: process_name(pid) });
        BOOL(1)
    }
    let mut out: Vec<WindowInfo> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut out as *mut _ as isize));
    }
    out
}

/// Our own top-level window, found by exact title (eframe sets it from the viewport builder).
pub fn find_by_title(title: &str) -> Option<SafeHwnd> {
    enumerate_all_toplevel().into_iter().find(|w| w.title == title).map(|w| w.hwnd)
}

pub fn show_and_focus(h: SafeHwnd) {
    unsafe {
        if IsIconic(h.hwnd()).as_bool() {
            let _ = ShowWindow(h.hwnd(), SW_RESTORE);
        } else {
            let _ = ShowWindow(h.hwnd(), SW_SHOW);
        }
        let _ = SetForegroundWindow(h.hwnd());
    }
}

pub fn hide(h: SafeHwnd) {
    unsafe {
        let _ = ShowWindow(h.hwnd(), SW_HIDE);
    }
}

/// Pin a window to an exact physical-pixel rectangle, topmost, without activating it.
/// Used to stretch the picker overlay across the whole virtual desktop regardless of
/// per-monitor DPI scaling.
pub fn pin_rect(h: SafeHwnd, x: i32, y: i32, w: i32, h_: i32) {
    unsafe {
        let _ = SetWindowPos(h.hwnd(), HWND_TOPMOST, x, y, w, h_, SWP_NOACTIVATE);
    }
}

pub fn window_rect(h: SafeHwnd) -> Option<(i32, i32, i32, i32)> {
    let mut rc = RECT::default();
    unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowRect(h.hwnd(), &mut rc).ok()? };
    Some((rc.left, rc.top, rc.right - rc.left, rc.bottom - rc.top))
}

/// The control that has keyboard focus inside `root`'s GUI thread, if it belongs to
/// `root`. Each thread keeps its own focus even while its window is in the background,
/// so this is where a real keystroke would land once the window is activated.
pub fn focused_child(root: SafeHwnd) -> Option<SafeHwnd> {
    use windows::Win32::UI::WindowsAndMessaging::{GetGUIThreadInfo, IsChild, GUITHREADINFO};
    unsafe {
        let tid = GetWindowThreadProcessId(root.hwnd(), None);
        if tid == 0 {
            return None;
        }
        let mut info = GUITHREADINFO { cbSize: std::mem::size_of::<GUITHREADINFO>() as u32, ..Default::default() };
        GetGUIThreadInfo(tid, &mut info).ok()?;
        let f = info.hwndFocus;
        if f.0.is_null() {
            return None;
        }
        (f == root.hwnd() || IsChild(root.hwnd(), f).as_bool()).then_some(SafeHwnd::from_hwnd(f))
    }
}
