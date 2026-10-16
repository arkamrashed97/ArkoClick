//! Full-screen target picker, implemented as a plain Win32 window painted with GDI.
//!
//! A topmost popup covers the whole virtual desktop and shows a frozen screenshot with a
//! crosshair, a magnifier and the cursor coordinates. Left click picks the spot, Esc or
//! right click cancels. On pick the overlay hides itself first so `WindowFromPoint` sees
//! the real window underneath.
//!
//! It runs on its own thread with its own message loop and reports back over a channel,
//! so the egui side only has to poll `Picker::poll` once per frame.

use super::{screen, to_wide, window, SafeHwnd};
use crate::config::{ClickPoint, TargetInfo, APP_TITLE};
use std::sync::mpsc;
use std::sync::Mutex;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW, CreatePen, CreateSolidBrush, DeleteDC,
    DeleteObject, EndPaint, FillRect, InvalidateRect, LineTo, MoveToEx, SelectObject, SetBkMode, SetStretchBltMode,
    SetTextColor, StretchBlt, TextOutW, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, COLORONCOLOR, DEFAULT_CHARSET,
    DEFAULT_PITCH, FW_NORMAL, HBITMAP, HDC, HGDIOBJ, OUT_DEFAULT_PRECIS, PAINTSTRUCT, PS_SOLID, SRCCOPY, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW, LoadCursorW,
    PostQuitMessage, RegisterClassW, SetCursor, SetForegroundWindow, ShowWindow, TranslateMessage, IDC_CROSS, MSG,
    SW_HIDE, SW_SHOW, WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN, WM_MOUSEMOVE, WM_PAINT,
    WM_RBUTTONDOWN, WM_SETCURSOR, WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

pub const PICKER_TITLE: &str = "ArkoClick Picker";
const CLASS_NAME: &str = "ArkoClickPickerOverlay";
const VK_ESCAPE: usize = 0x1B;
const MAG_SRC: i32 = 70;
const MAG_VIEW: i32 = 210;

#[derive(Debug, Clone)]
pub struct PickResult {
    pub target: TargetInfo,
    pub point: ClickPoint,
    /// THUMB_SIZE x THUMB_SIZE RGBA crop of the screen around the picked point.
    pub thumb: Option<Vec<u8>>,
}

#[derive(Debug)]
pub enum PickOutcome {
    Picked(PickResult),
    Cancelled,
    Failed(String),
}

/// Handle to an open overlay. Dropping it does not close the overlay (the user does).
pub struct Picker {
    rx: mpsc::Receiver<PickOutcome>,
}

impl Picker {
    /// `hint` is shown at the bottom of the overlay (already translated); `rtl` draws it
    /// right-to-left.
    pub fn open(hint: String, rtl: bool) -> Self {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("picker-overlay".into())
            .spawn(move || run_overlay(tx, hint, rtl))
            .expect("spawn picker thread");
        Self { rx }
    }

    /// Non-blocking. Returns the outcome once the user has clicked or cancelled.
    pub fn poll(&self) -> Option<PickOutcome> {
        self.rx.try_recv().ok()
    }
}

fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | (g as u32) << 8 | (b as u32) << 16)
}

struct State {
    tx: mpsc::Sender<PickOutcome>,
    /// Memory DC holding the screenshot (selected bitmap kept alive in `shot_bmp`).
    shot_dc: isize,
    shot_bmp: isize,
    vx: i32,
    vy: i32,
    vw: i32,
    vh: i32,
    finished: bool,
    hint: Vec<u16>,
    rtl: bool,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn run_overlay(tx: mpsc::Sender<PickOutcome>, hint: String, rtl: bool) {
    let (vx, vy, vw, vh) = screen::virtual_screen();
    let (shot_dc, shot_bmp) = match screen::capture_bitmap(vx, vy, vw, vh) {
        Some(v) => v,
        None => {
            let _ = tx.send(PickOutcome::Failed("could not capture the screen".into()));
            return;
        }
    };
    *STATE.lock().unwrap() =
        Some(State { tx: tx.clone(), shot_dc: shot_dc.0 as isize, shot_bmp: shot_bmp.0 as isize, vx, vy, vw, vh, finished: false, hint: hint.encode_utf16().collect(), rtl });

    unsafe {
        let class = to_wide(CLASS_NAME);
        let hinst = GetModuleHandleW(None).unwrap_or_default();
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinst.into(),
            lpszClassName: PCWSTR(class.as_ptr()),
            hCursor: LoadCursorW(None, IDC_CROSS).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassW(&wc); // fails harmlessly when already registered
        let title = to_wide(PICKER_TITLE);
        let hwnd = match CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            PCWSTR(class.as_ptr()),
            PCWSTR(title.as_ptr()),
            WS_POPUP,
            vx,
            vy,
            vw,
            vh,
            None,
            None,
            hinst,
            None,
        ) {
            Ok(h) => h,
            Err(e) => {
                let _ = tx.send(PickOutcome::Failed(format!("overlay window failed: {e}")));
                cleanup();
                return;
            }
        };
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    // If the loop ended without a verdict (window destroyed externally), report cancel.
    let unfinished = STATE.lock().unwrap().as_ref().map_or(false, |s| !s.finished);
    if unfinished {
        let _ = tx.send(PickOutcome::Cancelled);
    }
    cleanup();
}

fn cleanup() {
    if let Some(s) = STATE.lock().unwrap().take() {
        unsafe {
            let _ = DeleteObject(HGDIOBJ(s.shot_bmp as *mut _));
            let _ = DeleteDC(HDC(s.shot_dc as *mut _));
        }
    }
}

fn finish(hwnd: HWND, outcome: PickOutcome) {
    if let Some(s) = STATE.lock().unwrap().as_mut() {
        if s.finished {
            return;
        }
        s.finished = true;
        let _ = s.tx.send(outcome);
    }
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_MOUSEMOVE => {
            let _ = InvalidateRect(hwnd, None, false);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_SETCURSOR => {
            SetCursor(LoadCursorW(None, IDC_CROSS).unwrap_or_default());
            LRESULT(1)
        }
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            // Hide synchronously so WindowFromPoint looks through to the real window.
            // Crop the zoom thumbnail from the frozen screenshot (the live screen is covered
            // by this overlay right now).
            let thumb = STATE.lock().unwrap().as_ref().and_then(|s| {
                let half = crate::config::THUMB_SIZE as i32 / 2;
                let size = crate::config::THUMB_SIZE;
                let x = (pt.x - s.vx - half).clamp(0, (s.vw - size as i32).max(0));
                let y = (pt.y - s.vy - half).clamp(0, (s.vh - size as i32).max(0));
                screen::copy_from_dc(HDC(s.shot_dc as *mut _), x, y, size, size)
            });
            let _ = ShowWindow(hwnd, SW_HIDE);
            let outcome = match resolve(pt.x, pt.y) {
                PickOutcome::Picked(mut r) => {
                    r.thumb = thumb;
                    PickOutcome::Picked(r)
                }
                other => other,
            };
            finish(hwnd, outcome);
            LRESULT(0)
        }
        WM_RBUTTONDOWN => {
            finish(hwnd, PickOutcome::Cancelled);
            LRESULT(0)
        }
        WM_KEYDOWN if wparam.0 == VK_ESCAPE => {
            finish(hwnd, PickOutcome::Cancelled);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn paint(hwnd: HWND) {
    let (shot_dc, vx, vy, vw, vh, hint, rtl) = {
        let g = STATE.lock().unwrap();
        let Some(s) = g.as_ref() else { return };
        (HDC(s.shot_dc as *mut _), s.vx, s.vy, s.vw, s.vh, s.hint.clone(), s.rtl)
    };
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);

    // Double buffer: compose everything in memory, blit once.
    let mem = CreateCompatibleDC(hdc);
    let back = CreateCompatibleBitmap(hdc, vw, vh);
    let old_bmp = SelectObject(mem, back);
    let _ = BitBlt(mem, 0, 0, vw, vh, shot_dc, 0, 0, SRCCOPY);

    let mut cur = POINT::default();
    let _ = GetCursorPos(&mut cur);
    let cx = cur.x - vx;
    let cy = cur.y - vy;

    // Crosshair
    let pen = CreatePen(PS_SOLID, 1, rgb(255, 80, 80));
    let old_pen = SelectObject(mem, pen);
    let _ = MoveToEx(mem, 0, cy, None);
    let _ = LineTo(mem, vw, cy);
    let _ = MoveToEx(mem, cx, 0, None);
    let _ = LineTo(mem, cx, vh);
    SelectObject(mem, old_pen);
    let _ = DeleteObject(pen);

    // Magnifier
    let half = MAG_SRC / 2;
    let sx = (cx - half).clamp(0, (vw - MAG_SRC).max(0));
    let sy = (cy - half).clamp(0, (vh - MAG_SRC).max(0));
    let mut mx = cx + 18;
    let mut my = cy + 18;
    if mx + MAG_VIEW > vw {
        mx = cx - 18 - MAG_VIEW;
    }
    if my + MAG_VIEW + 24 > vh {
        my = cy - 18 - MAG_VIEW - 24;
    }
    mx = mx.max(0);
    my = my.max(0);
    SetStretchBltMode(mem, COLORONCOLOR);
    let _ = StretchBlt(mem, mx, my, MAG_VIEW, MAG_VIEW, shot_dc, sx, sy, MAG_SRC, MAG_SRC, SRCCOPY);
    // Magnifier frame + centre cross
    let frame_pen = CreatePen(PS_SOLID, 2, rgb(255, 120, 80));
    let old_pen = SelectObject(mem, frame_pen);
    let _ = MoveToEx(mem, mx, my, None);
    let _ = LineTo(mem, mx + MAG_VIEW, my);
    let _ = LineTo(mem, mx + MAG_VIEW, my + MAG_VIEW);
    let _ = LineTo(mem, mx, my + MAG_VIEW);
    let _ = LineTo(mem, mx, my);
    SelectObject(mem, old_pen);
    let _ = DeleteObject(frame_pen);
    let cross_pen = CreatePen(PS_SOLID, 1, rgb(255, 0, 0));
    let old_pen = SelectObject(mem, cross_pen);
    let c = (mx + MAG_VIEW / 2, my + MAG_VIEW / 2);
    let _ = MoveToEx(mem, c.0 - 10, c.1, None);
    let _ = LineTo(mem, c.0 + 10, c.1);
    let _ = MoveToEx(mem, c.0, c.1 - 10, None);
    let _ = LineTo(mem, c.0, c.1 + 10);
    SelectObject(mem, old_pen);
    let _ = DeleteObject(cross_pen);

    // Text: coordinates under the magnifier, hint at the bottom of the screen.
    let font = CreateFontW(
        -20,
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
    let old_font = SelectObject(mem, font);
    SetBkMode(mem, TRANSPARENT);
    SetTextColor(mem, rgb(255, 255, 255));
    let dark = CreateSolidBrush(rgb(20, 22, 28));

    let coords = to_wide(&format!("({}, {})", cur.x, cur.y));
    let coord_rect = RECT { left: mx, top: my + MAG_VIEW, right: mx + MAG_VIEW, bottom: my + MAG_VIEW + 24 };
    FillRect(mem, &coord_rect, dark);
    let _ = TextOutW(mem, mx + 6, my + MAG_VIEW + 3, &coords[..coords.len() - 1]);

    // Hint: measured and drawn by DrawText, which shapes Arabic and lays it out RTL.
    {
        use windows::Win32::Graphics::Gdi::{DrawTextW, DT_CALCRECT, DT_CENTER, DT_RTLREADING, DT_SINGLELINE, DT_VCENTER};
        let mut text = hint.clone();
        let mut calc = RECT { left: 0, top: 0, right: vw, bottom: 40 };
        let rtl_flag = if rtl { DT_RTLREADING } else { Default::default() };
        DrawTextW(mem, &mut text, &mut calc, DT_CALCRECT | DT_SINGLELINE | rtl_flag);
        let hw = (calc.right - calc.left) + 40;
        let hx = (vw - hw) / 2;
        let hy = vh - 80;
        let mut hint_rect = RECT { left: hx, top: hy, right: hx + hw, bottom: hy + 40 };
        FillRect(mem, &hint_rect, dark);
        let mut text = hint.clone();
        DrawTextW(mem, &mut text, &mut hint_rect, DT_CENTER | DT_VCENTER | DT_SINGLELINE | rtl_flag);
    }

    SelectObject(mem, old_font);
    let _ = DeleteObject(font);
    let _ = DeleteObject(dark);

    let _ = BitBlt(hdc, 0, 0, vw, vh, mem, 0, 0, SRCCOPY);
    SelectObject(mem, old_bmp);
    let _ = DeleteObject(back);
    let _ = DeleteDC(mem);
    let _ = EndPaint(hwnd, &ps);
}

/// Look up the top-level window under a screen point and build the target + point.
fn resolve(sx: i32, sy: i32) -> PickOutcome {
    let Some(root) = window::root_at(sx, sy) else {
        return PickOutcome::Failed("nothing under the cursor".into());
    };
    let info = window::info_of(root);
    if info.title == APP_TITLE || info.title == PICKER_TITLE {
        return PickOutcome::Failed(format!("that is {APP_TITLE} itself; pick another window"));
    }
    let Some((cx, cy)) = window::screen_to_client(root, sx, sy) else {
        return PickOutcome::Failed("cannot map the point into the window".into());
    };
    let (w, h) = window::client_size(root).unwrap_or((0, 0));
    if cx < 0 || cy < 0 || (w > 0 && cx > w) || (h > 0 && cy > h) {
        return PickOutcome::Failed("point is on the window frame, not inside its content".into());
    }
    PickOutcome::Picked(PickResult {
        target: TargetInfo { title: info.title, class: info.class, process: info.process, hwnd: Some(root.0) },
        point: ClickPoint { x: cx, y: cy, client_w: w, client_h: h, relative: true },
        thumb: None,
    })
}

#[allow(dead_code)]
fn _unused(_: SafeHwnd, _: HBITMAP) {}
