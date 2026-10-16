//! A small native Windows "enter text" box.
//!
//! egui cannot join Arabic letters or lay text out right-to-left while you type, so every
//! free-text field in ArkoClick (clicker names, profile names, text to type) is edited in a
//! real Win32 EDIT control instead: perfect Arabic shaping, IME support, mirrored layout
//! in Arabic mode, and dark/light colours to match the app.
//!
//! The box runs on its own thread with its own message loop and reports through a
//! channel, like the picker overlay.

use super::{to_wide, window, SafeHwnd};
use std::sync::mpsc;
use std::sync::Mutex;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
use windows::Win32::Graphics::Gdi::{
    CreateFontW, CreateSolidBrush, DeleteObject, FillRect, GetDC, ReleaseDC, SetBkColor, SetTextColor,
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH, FW_NORMAL, HBRUSH, HDC, HFONT, HGDIOBJ,
    OUT_DEFAULT_PRECIS,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{SetWindowTheme, EM_SETSEL};
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::WindowsAndMessaging::*;

const ID_EDIT: i32 = 100;
const ID_OK: i32 = 1; // IDOK: Enter triggers it through IsDialogMessage
const ID_CANCEL: i32 = 2; // IDCANCEL: Esc triggers it

pub struct TextDialogSpec {
    pub title: String,
    pub prompt: String,
    pub initial: String,
    pub ok: String,
    pub cancel: String,
    pub multiline: bool,
    pub rtl: bool,
    pub dark: bool,
}

/// Handle to an open box; poll once per frame.
pub struct TextDialog {
    rx: mpsc::Receiver<Option<String>>,
}

impl TextDialog {
    pub fn open(spec: TextDialogSpec) -> Self {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("text-dialog".into())
            .spawn(move || run(spec, tx))
            .expect("spawn text dialog thread");
        Self { rx }
    }

    /// `Some(Some(text))` on OK, `Some(None)` on cancel, `None` while still open.
    pub fn poll(&self) -> Option<Option<String>> {
        self.rx.try_recv().ok()
    }
}

struct Ctx {
    tx: mpsc::Sender<Option<String>>,
    edit: isize,
    bg: isize,
    edit_bg: isize,
    fg: u32,
    bg_col: u32,
    edit_col: u32,
    done: bool,
}

static CTX: Mutex<Option<Ctx>> = Mutex::new(None);

fn rgb(r: u8, g: u8, b: u8) -> u32 {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}

fn finish(hwnd: HWND, result: Option<String>) {
    if let Some(c) = CTX.lock().unwrap().as_mut() {
        if !c.done {
            c.done = true;
            let _ = c.tx.send(result);
        }
    }
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
}

fn read_edit(edit: HWND) -> String {
    unsafe {
        let n = GetWindowTextLengthW(edit).max(0) as usize;
        let mut buf = vec![0u16; n + 1];
        let got = GetWindowTextW(edit, &mut buf);
        String::from_utf16_lossy(&buf[..got.max(0) as usize])
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as i32;
            if id == ID_OK {
                let edit = CTX.lock().unwrap().as_ref().map(|c| c.edit).unwrap_or(0);
                let text = read_edit(SafeHwnd(edit).hwnd());
                finish(hwnd, Some(text));
                return LRESULT(0);
            }
            if id == ID_CANCEL {
                finish(hwnd, None);
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC => {
            let g = CTX.lock().unwrap();
            let Some(c) = g.as_ref() else { return DefWindowProcW(hwnd, msg, wparam, lparam) };
            let hdc = HDC(wparam.0 as *mut _);
            SetTextColor(hdc, COLORREF(c.fg));
            if msg == WM_CTLCOLOREDIT {
                SetBkColor(hdc, COLORREF(c.edit_col));
                LRESULT(c.edit_bg)
            } else {
                SetBkColor(hdc, COLORREF(c.bg_col));
                LRESULT(c.bg)
            }
        }
        WM_ERASEBKGND => {
            let bg = CTX.lock().unwrap().as_ref().map(|c| c.bg).unwrap_or(0);
            let mut rc = RECT::default();
            let _ = GetClientRect(hwnd, &mut rc);
            FillRect(HDC(wparam.0 as *mut _), &rc, HBRUSH(bg as *mut _));
            LRESULT(1)
        }
        WM_CLOSE => {
            finish(hwnd, None);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn run(spec: TextDialogSpec, tx: mpsc::Sender<Option<String>>) {
    let (fg, bg_col, edit_col) = if spec.dark {
        (rgb(0xE4, 0xE8, 0xEE), rgb(0x1D, 0x21, 0x2B), rgb(0x10, 0x13, 0x19))
    } else {
        (rgb(0x1C, 0x22, 0x2B), rgb(0xF4, 0xF6, 0xF9), rgb(0xFF, 0xFF, 0xFF))
    };
    unsafe {
        let bg = CreateSolidBrush(COLORREF(bg_col));
        let edit_bg = CreateSolidBrush(COLORREF(edit_col));
        *CTX.lock().unwrap() =
            Some(Ctx { tx: tx.clone(), edit: 0, bg: bg.0 as isize, edit_bg: edit_bg.0 as isize, fg, bg_col, edit_col, done: false });

        let hinst = GetModuleHandleW(None).unwrap_or_default();
        let class = to_wide("ArkoClickTextDialog");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinst.into(),
            lpszClassName: PCWSTR(class.as_ptr()),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassW(&wc);

        // DPI-aware sizing: scale our 96-dpi layout by the system DPI.
        let dc = GetDC(None);
        let dpi = windows::Win32::Graphics::Gdi::GetDeviceCaps(dc, windows::Win32::Graphics::Gdi::LOGPIXELSY);
        ReleaseDC(None, dc);
        let s = |v: i32| v * dpi / 96;

        let (w, h) = (s(480), if spec.multiline { s(300) } else { s(200) });
        let owner = window::find_by_title(crate::config::APP_TITLE);
        // Centre over the ArkoClick window (or the screen).
        let (cx, cy) = owner
            .and_then(window::window_rect)
            .map(|(x, y, ww, hh)| (x + (ww - w) / 2, y + (hh - h) / 2))
            .unwrap_or_else(|| ((GetSystemMetrics(SM_CXSCREEN) - w) / 2, (GetSystemMetrics(SM_CYSCREEN) - h) / 2));

        let mut ex = WS_EX_DLGMODALFRAME | WS_EX_TOPMOST;
        if spec.rtl {
            ex |= WS_EX_LAYOUTRTL;
        }
        let title = to_wide(&spec.title);
        let Ok(hwnd) = CreateWindowExW(
            ex,
            PCWSTR(class.as_ptr()),
            PCWSTR(title.as_ptr()),
            WS_POPUP | WS_CAPTION | WS_SYSMENU,
            cx,
            cy,
            w,
            h,
            owner.map(|o| o.hwnd()).unwrap_or_default(),
            None,
            hinst,
            None,
        ) else {
            let _ = tx.send(None);
            return;
        };
        if spec.dark {
            let on: i32 = 1;
            let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &on as *const i32 as *const _, 4);
        }

        let font: HFONT = CreateFontW(
            -s(17),
            0,
            0,
            0,
            FW_NORMAL.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET.0 as u32,
            OUT_DEFAULT_PRECIS.0 as u32,
            CLIP_DEFAULT_PRECIS.0 as u32,
            CLEARTYPE_QUALITY.0 as u32,
            DEFAULT_PITCH.0 as u32,
            windows::core::w!("Segoe UI"),
        );
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        let cw = client.right - client.left;
        let ch = client.bottom - client.top;
        let pad = s(16);
        let btn_w = s(110);
        let btn_h = s(34);

        let make = |class: &str, text: &str, style: WINDOW_STYLE, ex: WINDOW_EX_STYLE, x: i32, y: i32, ww: i32, hh: i32, id: i32| {
            let c = to_wide(class);
            let t = to_wide(text);
            let h = CreateWindowExW(
                ex,
                PCWSTR(c.as_ptr()),
                PCWSTR(t.as_ptr()),
                WS_CHILD | WS_VISIBLE | style,
                x,
                y,
                ww,
                hh,
                hwnd,
                HMENU(id as isize as *mut _),
                hinst,
                None,
            )
            .unwrap_or_default();
            SendMessageW(h, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
            h
        };

        let rtl_text = if spec.rtl { WS_EX_RTLREADING } else { WINDOW_EX_STYLE(0) };
        make("STATIC", &spec.prompt, WINDOW_STYLE(0), rtl_text, pad, pad, cw - 2 * pad, s(24), -1);
        let edit_style = if spec.multiline {
            WS_TABSTOP | WS_VSCROLL | WINDOW_STYLE((ES_MULTILINE | ES_AUTOVSCROLL | ES_WANTRETURN) as u32)
        } else {
            WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32)
        };
        let edit_top = pad + s(32);
        let edit_h = ch - edit_top - btn_h - 2 * pad;
        let edit = make("EDIT", &spec.initial, edit_style | WS_BORDER, rtl_text, pad, edit_top, cw - 2 * pad, edit_h, ID_EDIT);
        let by = ch - pad - btn_h;
        let ok = make(
            "BUTTON",
            &spec.ok,
            WS_TABSTOP | WINDOW_STYLE(BS_DEFPUSHBUTTON as u32),
            WINDOW_EX_STYLE(0),
            cw - pad - btn_w,
            by,
            btn_w,
            btn_h,
            ID_OK,
        );
        let cancel = make(
            "BUTTON",
            &spec.cancel,
            WS_TABSTOP,
            WINDOW_EX_STYLE(0),
            cw - pad - 2 * btn_w - s(10),
            by,
            btn_w,
            btn_h,
            ID_CANCEL,
        );
        if spec.dark {
            for c in [edit, ok, cancel] {
                let _ = SetWindowTheme(c, windows::core::w!("DarkMode_Explorer"), PCWSTR::null());
            }
        }
        if let Some(c) = CTX.lock().unwrap().as_mut() {
            c.edit = edit.0 as isize;
        }

        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(edit);
        SendMessageW(edit, EM_SETSEL, WPARAM(0), LPARAM(-1));

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            // Tab between controls, Enter = OK, Esc = Cancel (multi-line keeps Enter).
            if IsDialogMessageW(hwnd, &msg).as_bool() {
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        let unfinished = CTX.lock().unwrap().as_ref().map_or(false, |c| !c.done);
        if unfinished {
            let _ = tx.send(None);
        }
        *CTX.lock().unwrap() = None;
        let _ = DeleteObject(HGDIOBJ(font.0));
        let _ = DeleteObject(HGDIOBJ(bg.0));
        let _ = DeleteObject(HGDIOBJ(edit_bg.0));
    }
}
