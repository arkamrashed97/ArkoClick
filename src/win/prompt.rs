//! Small native "start saved task?" box shown near the clock when a linked program or page
//! comes to the front. Native Win32 so it works while ArkoClick is hidden in the tray, and
//! "no-activate" so it never steals focus from the window the user just opened.
//! Closes by itself after a while ("not now").

use super::to_wide;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateSolidBrush, DeleteObject, EndPaint, FillRect, GetDC, GetDeviceCaps, ReleaseDC,
    SetBkColor, SetTextColor, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH, FW_NORMAL,
    FW_SEMIBOLD, HBRUSH, HDC, HFONT, HGDIOBJ, LOGPIXELSY, OUT_DEFAULT_PRECIS, PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::SetWindowTheme;
use windows::Win32::UI::WindowsAndMessaging::*;

const ID_START: i32 = 11;
const ID_LATER: i32 = 12;
const ID_NEVER: i32 = 13;
const TIMER_CLOSE: usize = 1;
const AUTO_CLOSE_MS: u32 = 30_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Start,
    NotNow,
    Never,
}

pub struct PromptSpec {
    pub title: String,
    pub message: String,
    pub start: String,
    pub later: String,
    pub never: String,
    pub rtl: bool,
    pub dark: bool,
}

/// Only one box at a time; a second trigger while one is open is ignored.
static OPEN: AtomicBool = AtomicBool::new(false);

pub fn is_open() -> bool {
    OPEN.load(Ordering::Acquire)
}

struct Ctx {
    choice: Choice,
    bg: isize,
    fg: u32,
    bg_col: u32,
    title_font: isize,
    title: Vec<u16>,
    icon: isize,
    scale: i32,
    rtl: bool,
}

static CTX: Mutex<Option<Ctx>> = Mutex::new(None);

fn rgb(r: u8, g: u8, b: u8) -> u32 {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}

/// Show the box on its own thread and call `on_choice` with the answer when it closes.
pub fn ask(spec: PromptSpec, on_choice: impl FnOnce(Choice) + Send + 'static) {
    if OPEN.swap(true, Ordering::AcqRel) {
        return;
    }
    std::thread::Builder::new()
        .name("prompt".into())
        .spawn(move || {
            let choice = run(spec);
            OPEN.store(false, Ordering::Release);
            on_choice(choice);
        })
        .expect("spawn prompt thread");
}

fn finish(hwnd: HWND, choice: Choice) {
    if let Some(c) = CTX.lock().unwrap().as_mut() {
        c.choice = choice;
    }
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_COMMAND => {
            match (wparam.0 & 0xFFFF) as i32 {
                ID_START => finish(hwnd, Choice::Start),
                ID_LATER => finish(hwnd, Choice::NotNow),
                ID_NEVER => finish(hwnd, Choice::Never),
                _ => {}
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_CLOSE => {
            finish(hwnd, Choice::NotNow);
            LRESULT(0)
        }
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_CTLCOLORSTATIC => {
            let g = CTX.lock().unwrap();
            let Some(c) = g.as_ref() else { return DefWindowProcW(hwnd, msg, wparam, lparam) };
            let hdc = HDC(wparam.0 as *mut _);
            SetTextColor(hdc, COLORREF(c.fg));
            SetBkColor(hdc, COLORREF(c.bg_col));
            LRESULT(c.bg)
        }
        WM_ERASEBKGND => {
            let bg = CTX.lock().unwrap().as_ref().map(|c| c.bg).unwrap_or(0);
            let mut rc = RECT::default();
            let _ = GetClientRect(hwnd, &mut rc);
            FillRect(HDC(wparam.0 as *mut _), &rc, HBRUSH(bg as *mut _));
            LRESULT(1)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            if let Some(c) = CTX.lock().unwrap().as_ref() {
                let s = |v: i32| v * c.scale / 96;
                // Icon + "ArkoClick" title line. (With RTL layout the DC is mirrored, so the
                // same coordinates land on the right.)
                let _ = DrawIconEx(hdc, s(16), s(14), HICON(c.icon as *mut _), s(24), s(24), 0, None, DI_NORMAL);
                let old = windows::Win32::Graphics::Gdi::SelectObject(hdc, HGDIOBJ(c.title_font as *mut _));
                SetTextColor(hdc, COLORREF(c.fg));
                windows::Win32::Graphics::Gdi::SetBkMode(hdc, windows::Win32::Graphics::Gdi::TRANSPARENT);
                let mut rc = RECT { left: s(48), top: s(14), right: s(400), bottom: s(40) };
                let mut t = c.title.clone();
                let flags = windows::Win32::Graphics::Gdi::DT_SINGLELINE | windows::Win32::Graphics::Gdi::DT_VCENTER;
                let flags = if c.rtl { flags | windows::Win32::Graphics::Gdi::DT_RTLREADING } else { flags };
                windows::Win32::Graphics::Gdi::DrawTextW(hdc, &mut t, &mut rc, flags);
                windows::Win32::Graphics::Gdi::SelectObject(hdc, old);
            }
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn run(spec: PromptSpec) -> Choice {
    let (fg, bg_col) = if spec.dark { (rgb(0xE6, 0xEA, 0xF0), rgb(0x1D, 0x21, 0x2B)) } else { (rgb(0x1A, 0x20, 0x2A), rgb(0xFF, 0xFF, 0xFF)) };
    unsafe {
        let hinst = GetModuleHandleW(None).unwrap_or_default();
        let dc = GetDC(None);
        let scale = GetDeviceCaps(dc, LOGPIXELSY);
        ReleaseDC(None, dc);
        let s = |v: i32| v * scale / 96;

        let make_font = |size: i32, weight: i32| -> HFONT {
            CreateFontW(
                -s(size),
                0,
                0,
                0,
                weight,
                0,
                0,
                0,
                DEFAULT_CHARSET.0 as u32,
                OUT_DEFAULT_PRECIS.0 as u32,
                CLIP_DEFAULT_PRECIS.0 as u32,
                CLEARTYPE_QUALITY.0 as u32,
                DEFAULT_PITCH.0 as u32,
                windows::core::w!("Segoe UI"),
            )
        };
        let font = make_font(15, FW_NORMAL.0 as i32);
        let title_font = make_font(16, FW_SEMIBOLD.0 as i32);
        let bg = CreateSolidBrush(COLORREF(bg_col));
        let icon = LoadImageW(hinst, PCWSTR(1 as *const u16), IMAGE_ICON, s(24), s(24), LR_DEFAULTCOLOR)
            .map(|h| h.0 as isize)
            .unwrap_or(0);
        *CTX.lock().unwrap() = Some(Ctx {
            choice: Choice::NotNow,
            bg: bg.0 as isize,
            fg,
            bg_col,
            title_font: title_font.0 as isize,
            title: spec.title.encode_utf16().collect(),
            icon,
            scale,
            rtl: spec.rtl,
        });

        let class = to_wide("ArkoClickPrompt");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinst.into(),
            lpszClassName: PCWSTR(class.as_ptr()),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassW(&wc);

        // Bottom-right of the work area (above the taskbar).
        let mut work = RECT::default();
        let _ = SystemParametersInfoW(SPI_GETWORKAREA, 0, Some(&mut work as *mut RECT as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
        let (w, h) = (s(440), s(170));
        let (x, y) = (work.right - w - s(16), work.bottom - h - s(16));
        let mut ex = WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
        if spec.rtl {
            ex |= WS_EX_LAYOUTRTL;
        }
        let Ok(hwnd) = CreateWindowExW(ex, PCWSTR(class.as_ptr()), PCWSTR::null(), WS_POPUP | WS_BORDER, x, y, w, h, None, None, hinst, None)
        else {
            cleanup(font, title_font, bg);
            return Choice::NotNow;
        };

        let add = |class: &str, text: &str, style: WINDOW_STYLE, x: i32, y: i32, cw: i32, ch: i32, id: i32| {
            let c = to_wide(class);
            let t = to_wide(text);
            let ex = if spec.rtl { WS_EX_RTLREADING } else { WINDOW_EX_STYLE(0) };
            let ctl = CreateWindowExW(
                ex,
                PCWSTR(c.as_ptr()),
                PCWSTR(t.as_ptr()),
                WS_CHILD | WS_VISIBLE | style,
                x,
                y,
                cw,
                ch,
                hwnd,
                HMENU(id as isize as *mut _),
                hinst,
                None,
            )
            .unwrap_or_default();
            SendMessageW(ctl, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
            ctl
        };
        add("STATIC", &spec.message, WINDOW_STYLE(0), s(16), s(48), w - s(32), s(56), -1);
        let bw = (w - s(32) - s(16)) / 3;
        let by = h - s(16) - s(34);
        let b1 = add("BUTTON", &spec.start, WINDOW_STYLE(BS_DEFPUSHBUTTON as u32), s(16), by, bw, s(34), ID_START);
        let b2 = add("BUTTON", &spec.later, WINDOW_STYLE(0), s(16) + bw + s(8), by, bw, s(34), ID_LATER);
        let b3 = add("BUTTON", &spec.never, WINDOW_STYLE(0), s(16) + 2 * (bw + s(8)), by, bw, s(34), ID_NEVER);
        if spec.dark {
            for b in [b1, b2, b3] {
                let _ = SetWindowTheme(b, windows::core::w!("DarkMode_Explorer"), PCWSTR::null());
            }
        }

        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        SetTimer(hwnd, TIMER_CLOSE, AUTO_CLOSE_MS, None);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let choice = CTX.lock().unwrap().take().map(|c| c.choice).unwrap_or(Choice::NotNow);
        if icon != 0 {
            let _ = DestroyIcon(HICON(icon as *mut _));
        }
        cleanup(font, title_font, bg);
        choice
    }
}

unsafe fn cleanup(font: HFONT, title_font: HFONT, bg: HBRUSH) {
    let _ = DeleteObject(HGDIOBJ(font.0));
    let _ = DeleteObject(HGDIOBJ(title_font.0));
    let _ = DeleteObject(HGDIOBJ(bg.0));
}
