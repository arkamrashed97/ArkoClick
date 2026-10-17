//! System tray icon with a small context menu. Runs on its own thread with a hidden
//! message window. Start/stop act on the engine directly; "Show" restores the main
//! window through Win32 so it works even while the egui thread is idle.

use super::{to_wide, window, SafeHwnd};
use crate::config::APP_TITLE;
use crate::engine::Engine;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NIN_BALLOONUSERCLICK, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow, DispatchMessageW,
    GetCursorPos, GetMessageW, LoadIconW, PostMessageW, RegisterClassW, SetForegroundWindow, TrackPopupMenu,
    TranslateMessage, HICON, IDI_APPLICATION, MF_GRAYED, MF_SEPARATOR, MF_STRING, MSG, TPM_BOTTOMALIGN,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_COMMAND, WM_DESTROY, WM_LBUTTONUP,
    WM_RBUTTONUP, WNDCLASSW,
};

const TRAY_ID: u32 = 1;
const CALLBACK_MSG: u32 = WM_APP + 1;
const WM_UPDATE_TIP: u32 = WM_APP + 2;
const WM_SHOW_NOTIFICATION: u32 = WM_APP + 3;

const ID_SHOW: usize = 1;
const ID_START_ALL: usize = 2;
const ID_STOP_ALL: usize = 3;
const ID_SHOW_GHOSTS: usize = 4;
const ID_QUIT: usize = 9;

/// Raised by the tray's Quit item. The UI thread polls it; a fallback timer force-exits
/// if the UI never wakes (hidden egui windows receive no redraws).
pub static QUIT_REQUESTED: AtomicBool = AtomicBool::new(false);
/// Menu language, kept in sync by the UI.
pub static ARABIC: AtomicBool = AtomicBool::new(false);

fn t<'a>(en: &'a str, ar: &'a str) -> &'a str {
    if ARABIC.load(Ordering::Relaxed) { ar } else { en }
}

struct Shared {
    engine: Arc<Engine>,
    on_change: Box<dyn Fn() + Send + Sync>,
}

static SHARED: OnceLock<Shared> = OnceLock::new();
static TRAY_HWND: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

pub fn show_main_window() {
    if let Some(h) = window::find_by_title(APP_TITLE) {
        window::show_and_focus(h);
    }
    if let Some(s) = SHARED.get() {
        (s.on_change)();
    }
}

/// Update the tooltip (e.g. "ArkoClick - 2 running"). Safe from any thread.
/// Show a Windows notification from the tray icon (e.g. "update available"). Clicking it
/// opens the main window. Safe from any thread.
pub fn notify(title: &str, body: &str) {
    let h = TRAY_HWND.load(Ordering::Acquire);
    if h == 0 {
        return;
    }
    let boxed = Box::new((title.to_string(), body.to_string()));
    unsafe {
        let _ = PostMessageW(HWND(h as *mut _), WM_SHOW_NOTIFICATION, WPARAM(0), LPARAM(Box::into_raw(boxed) as isize));
    }
}

pub fn set_tooltip(text: &str) {
    let h = TRAY_HWND.load(Ordering::Acquire);
    if h == 0 {
        return;
    }
    // Pass ownership of a boxed string through the message; the tray thread frees it.
    let boxed = Box::new(text.to_string());
    unsafe {
        let _ = PostMessageW(HWND(h as *mut _), WM_UPDATE_TIP, WPARAM(0), LPARAM(Box::into_raw(boxed) as isize));
    }
}

/// Our own icon (resource ID 1 embedded by build.rs) at the small-icon size, falling back
/// to the stock application icon when the exe was built without resources.
unsafe fn app_icon() -> HICON {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, LoadImageW, IMAGE_ICON, LR_DEFAULTCOLOR, SM_CXSMICON, SM_CYSMICON,
    };
    let hinst = GetModuleHandleW(None).unwrap_or_default();
    let small = LoadImageW(
        hinst,
        PCWSTR(1 as *const u16),
        IMAGE_ICON,
        GetSystemMetrics(SM_CXSMICON),
        GetSystemMetrics(SM_CYSMICON),
        LR_DEFAULTCOLOR,
    );
    match small {
        Ok(h) if !h.is_invalid() => HICON(h.0),
        _ => LoadIconW(None, IDI_APPLICATION).unwrap_or_default(),
    }
}

unsafe fn notify_data(hwnd: HWND, icon: HICON, tip: &str) -> NOTIFYICONDATAW {
    let mut nid = NOTIFYICONDATAW::default();
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = TRAY_ID;
    nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    nid.uCallbackMessage = CALLBACK_MSG;
    nid.hIcon = icon;
    for (i, u) in tip.encode_utf16().take(127).enumerate() {
        nid.szTip[i] = u;
    }
    nid
}

unsafe fn show_menu(hwnd: HWND) {
    let Some(s) = SHARED.get() else { return };
    let running = s.engine.running_count() > 0;
    let menu = CreatePopupMenu().unwrap_or_default();
    let add = |flags, id: usize, text: &str| {
        let w = to_wide(text);
        let _ = AppendMenuW(menu, flags, id, PCWSTR(w.as_ptr()));
    };
    add(MF_STRING, ID_SHOW, t("Open ArkoClick", "فتح ArkoClick"));
    add(MF_SEPARATOR, 0, "");
    add(if running { MF_STRING | MF_GRAYED } else { MF_STRING }, ID_START_ALL, t("Start all", "تشغيل الكل"));
    add(if running { MF_STRING } else { MF_STRING | MF_GRAYED }, ID_STOP_ALL, t("Stop all", "إيقاف الكل"));
    let ghosts = super::ghost::count();
    add(
        if ghosts > 0 { MF_STRING } else { MF_STRING | MF_GRAYED },
        ID_SHOW_GHOSTS,
        &format!("{} ({ghosts})", t("Show hidden windows", "إظهار النوافذ المخفية")),
    );
    add(MF_SEPARATOR, 0, "");
    add(MF_STRING, ID_QUIT, t("Exit", "خروج"));

    let mut pt = POINT::default();
    let _ = GetCursorPos(&mut pt);
    // Required so the menu closes when the user clicks elsewhere.
    let _ = SetForegroundWindow(hwnd);
    let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, pt.x, pt.y, 0, hwnd, None);
    let _ = DestroyMenu(menu);
    match cmd.0 as usize {
        ID_SHOW => show_main_window(),
        ID_START_ALL => {
            s.engine.start_all();
            (s.on_change)();
        }
        ID_STOP_ALL => {
            s.engine.stop_all();
            (s.on_change)();
        }
        ID_SHOW_GHOSTS => {
            super::ghost::show_all();
            (s.on_change)();
        }
        ID_QUIT => {
            s.engine.stop_all();
            QUIT_REQUESTED.store(true, Ordering::Release);
            (s.on_change)();
            // Give the UI thread a moment to save and exit cleanly; otherwise exit here.
            std::thread::spawn(|| {
                std::thread::sleep(std::time::Duration::from_millis(600));
                super::ghost::show_all();
                remove_icon();
                std::process::exit(0);
            });
        }
        _ => {}
    }
}

fn remove_icon() {
    let h = TRAY_HWND.load(Ordering::Acquire);
    if h == 0 {
        return;
    }
    unsafe {
        let mut nid = NOTIFYICONDATAW::default();
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = HWND(h as *mut _);
        nid.uID = TRAY_ID;
        let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        CALLBACK_MSG if wparam.0 as u32 == TRAY_ID => {
            match lparam.0 as u32 {
                WM_LBUTTONUP => show_main_window(),
                WM_RBUTTONUP => show_menu(hwnd),
                NIN_BALLOONUSERCLICK => show_main_window(),
                _ => {}
            }
            LRESULT(0)
        }
        WM_SHOW_NOTIFICATION => {
            let msg: Box<(String, String)> = Box::from_raw(lparam.0 as *mut (String, String));
            let mut nid = NOTIFYICONDATAW::default();
            nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = hwnd;
            nid.uID = TRAY_ID;
            nid.uFlags = NIF_INFO;
            nid.dwInfoFlags = NIIF_INFO;
            for (i, u) in msg.0.encode_utf16().take(63).enumerate() {
                nid.szInfoTitle[i] = u;
            }
            for (i, u) in msg.1.encode_utf16().take(255).enumerate() {
                nid.szInfo[i] = u;
            }
            let ok = Shell_NotifyIconW(NIM_MODIFY, &nid).as_bool();
            log::info!("tray notification shown={ok}: {}", msg.0);
            LRESULT(0)
        }
        WM_UPDATE_TIP => {
            let text: Box<String> = Box::from_raw(lparam.0 as *mut String);
            let nid = notify_data(hwnd, app_icon(), &text);
            let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
            LRESULT(0)
        }
        WM_COMMAND => LRESULT(0),
        WM_DESTROY => LRESULT(0),
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

pub fn spawn(engine: Arc<Engine>, on_change: impl Fn() + Send + Sync + 'static) {
    let _ = SHARED.set(Shared { engine, on_change: Box::new(on_change) });
    std::thread::Builder::new()
        .name("tray".into())
        .spawn(move || unsafe {
            let class = to_wide("ArkoClickTrayWnd");
            let hinst = GetModuleHandleW(None).unwrap_or_default();
            let wc = WNDCLASSW {
                lpfnWndProc: Some(wnd_proc),
                hInstance: hinst.into(),
                lpszClassName: PCWSTR(class.as_ptr()),
                ..Default::default()
            };
            if RegisterClassW(&wc) == 0 {
                log::error!("tray: RegisterClassW failed");
                return;
            }
            let hwnd = match CreateWindowExW(
                WINDOW_EX_STYLE(0),
                PCWSTR(class.as_ptr()),
                PCWSTR::null(),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                None,
                None,
                hinst,
                None,
            ) {
                Ok(h) => h,
                Err(e) => {
                    log::error!("tray: CreateWindowExW failed: {e}");
                    return;
                }
            };
            TRAY_HWND.store(hwnd.0 as isize, Ordering::Release);

            let nid = notify_data(hwnd, app_icon(), APP_TITLE);
            let _ = Shell_NotifyIconW(NIM_ADD, &nid);

            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
            let _ = DestroyWindow(hwnd);
        })
        .expect("spawn tray thread");
}

/// Remove the icon on clean exit (Windows does not clean up after a process dies).
pub fn shutdown() {
    remove_icon();
}

#[allow(dead_code)]
pub fn tray_hwnd() -> SafeHwnd {
    SafeHwnd(TRAY_HWND.load(Ordering::Acquire))
}
