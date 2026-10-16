//! Cursor position, virtual-screen metrics and screen capture (for the picker overlay).

use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, HBITMAP, HDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject, BI_RGB,
    BITMAPINFO, BITMAPINFOHEADER, CAPTUREBLT, DIB_RGB_COLORS, SRCCOPY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

/// Physical-pixel cursor position.
pub fn cursor_pos() -> (i32, i32) {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    (p.x, p.y)
}

/// Bounding rectangle of all monitors: (x, y, w, h). x/y are negative when a monitor
/// sits left of / above the primary one.
pub fn virtual_screen() -> (i32, i32, i32, i32) {
    unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    }
}

/// Capture a screen region as RGBA bytes. `GetDC(NULL)` covers the whole virtual screen,
/// so negative coordinates (secondary monitors) work.
pub fn capture_region(x: i32, y: i32, w: u32, h: u32) -> Option<Vec<u8>> {
    if w == 0 || h == 0 {
        return None;
    }
    unsafe {
        let hdc = GetDC(None);
        if hdc.is_invalid() {
            return None;
        }
        let mem = CreateCompatibleDC(hdc);
        if mem.is_invalid() {
            ReleaseDC(None, hdc);
            return None;
        }
        let mut bmi = BITMAPINFO::default();
        bmi.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w as i32,
            biHeight: -(h as i32), // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let bmp = match CreateDIBSection(mem, &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(b) if !bits.is_null() => b,
            _ => {
                let _ = DeleteDC(mem);
                ReleaseDC(None, hdc);
                return None;
            }
        };
        let old = SelectObject(mem, bmp);
        let _ = BitBlt(mem, 0, 0, w as i32, h as i32, hdc, x, y, SRCCOPY | CAPTUREBLT);

        let n = (w * h * 4) as usize;
        let src = std::slice::from_raw_parts(bits as *const u8, n);
        let mut out = Vec::with_capacity(n);
        for p in src.chunks_exact(4) {
            out.extend_from_slice(&[p[2], p[1], p[0], 255]); // BGRA -> RGBA
        }

        SelectObject(mem, old);
        let _ = DeleteObject(bmp);
        let _ = DeleteDC(mem);
        ReleaseDC(None, hdc);
        Some(out)
    }
}

/// Capture a screen region into a GDI bitmap selected into a memory DC. The caller owns
/// both handles (`DeleteObject` the bitmap, `DeleteDC` the DC).
pub fn capture_bitmap(x: i32, y: i32, w: i32, h: i32) -> Option<(HDC, HBITMAP)> {
    if w <= 0 || h <= 0 {
        return None;
    }
    unsafe {
        let hdc = GetDC(None);
        if hdc.is_invalid() {
            return None;
        }
        let mem = CreateCompatibleDC(hdc);
        let bmp = CreateCompatibleBitmap(hdc, w, h);
        if mem.is_invalid() || bmp.is_invalid() {
            let _ = DeleteObject(bmp);
            let _ = DeleteDC(mem);
            ReleaseDC(None, hdc);
            return None;
        }
        SelectObject(mem, bmp);
        let ok = BitBlt(mem, 0, 0, w, h, hdc, x, y, SRCCOPY | CAPTUREBLT).is_ok();
        ReleaseDC(None, hdc);
        if !ok {
            let _ = DeleteObject(bmp);
            let _ = DeleteDC(mem);
            return None;
        }
        Some((mem, bmp))
    }
}

/// Copy a region of an existing device context (e.g. a frozen screenshot) as RGBA bytes.
pub fn copy_from_dc(src: HDC, x: i32, y: i32, w: u32, h: u32) -> Option<Vec<u8>> {
    if w == 0 || h == 0 {
        return None;
    }
    unsafe {
        let mem = CreateCompatibleDC(src);
        if mem.is_invalid() {
            return None;
        }
        let mut bmi = BITMAPINFO::default();
        bmi.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w as i32,
            biHeight: -(h as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let bmp = match CreateDIBSection(mem, &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(b) if !bits.is_null() => b,
            _ => {
                let _ = DeleteDC(mem);
                return None;
            }
        };
        let old = SelectObject(mem, bmp);
        let _ = BitBlt(mem, 0, 0, w as i32, h as i32, src, x, y, SRCCOPY);
        let n = (w * h * 4) as usize;
        let px = std::slice::from_raw_parts(bits as *const u8, n);
        let mut out = Vec::with_capacity(n);
        for p in px.chunks_exact(4) {
            out.extend_from_slice(&[p[2], p[1], p[0], 255]);
        }
        SelectObject(mem, old);
        let _ = DeleteObject(bmp);
        let _ = DeleteDC(mem);
        Some(out)
    }
}
