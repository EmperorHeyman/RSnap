use std::mem::{size_of, zeroed};
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::Graphics::Gdi::*;

/// Top-down BGRA pixels in a DIB section, freed on drop.
pub struct Shot {
    bmp: HBITMAP,
    bits: *const u8,
    pub w: i32,
    pub h: i32,
}

// DIB section memory is process-wide: any thread may read it and delete the handle.
unsafe impl Send for Shot {}

impl Shot {
    pub fn pixels(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.bits, (self.w * self.h * 4) as usize) }
    }
}

impl Drop for Shot {
    fn drop(&mut self) {
        unsafe { DeleteObject(self.bmp) };
    }
}

pub fn grab(r: RECT) -> Option<Shot> {
    let (w, h) = (r.right - r.left, r.bottom - r.top);
    unsafe {
        let screen = GetDC(null_mut());
        let mem = CreateCompatibleDC(screen);
        let mut bmi: BITMAPINFO = zeroed();
        bmi.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = w;
        bmi.bmiHeader.biHeight = -h;
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB;
        let mut bits = null_mut();
        let bmp = CreateDIBSection(screen, &bmi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        let mut ok = false;
        if !bmp.is_null() {
            let old = SelectObject(mem, bmp);
            // CAPTUREBLT includes layered windows (tooltips, menus) under the selection.
            ok = BitBlt(mem, 0, 0, w, h, screen, r.left, r.top, SRCCOPY | CAPTUREBLT) != 0;
            SelectObject(mem, old);
            GdiFlush();
        }
        DeleteDC(mem);
        ReleaseDC(null_mut(), screen);
        if ok {
            Some(Shot {
                bmp,
                bits: bits as *const u8,
                w,
                h,
            })
        } else {
            if !bmp.is_null() {
                DeleteObject(bmp);
            }
            None
        }
    }
}
