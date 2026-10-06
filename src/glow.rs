//! The glow frame: 8 per-pixel-alpha windows (4 edges, 4 corners) around the selection.
//! Gradients are rendered once per snip; a mouse move only re-points the windows at them.

use std::cell::RefCell;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{HINSTANCE, HWND, POINT, SIZE};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows_sys::Win32::UI::WindowsAndMessaging::*;
use windows_sys::w;

use crate::config::{GLOW_CORE_ALPHA, GLOW_FADE_ALPHA};
use crate::settings;

const FALLBACK_COLOR: u32 = 0x00FF_A82F;

/// A memory DC holding a premultiplied BGRA bitmap, as UpdateLayeredWindow wants.
struct Sheet {
    dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
}

impl Sheet {
    fn new(w: i32, h: i32, fill: impl Fn(i32, i32) -> u32) -> Sheet {
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            let mut bmi: BITMAPINFO = zeroed();
            bmi.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
            bmi.bmiHeader.biWidth = w;
            bmi.bmiHeader.biHeight = -h;
            bmi.bmiHeader.biPlanes = 1;
            bmi.bmiHeader.biBitCount = 32;
            bmi.bmiHeader.biCompression = BI_RGB;
            let mut bits = null_mut();
            let bmp = CreateDIBSection(dc, &bmi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            if !bmp.is_null() {
                let px = std::slice::from_raw_parts_mut(bits as *mut u32, (w * h) as usize);
                for (i, p) in px.iter_mut().enumerate() {
                    *p = fill(i as i32 % w, i as i32 / w);
                }
            }
            let old = SelectObject(dc, bmp);
            Sheet { dc, bmp, old }
        }
    }
}

impl Drop for Sheet {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            DeleteObject(self.bmp);
            DeleteDC(self.dc);
        }
    }
}

struct Glow {
    /// Corners TL, TR, BL, BR, then edges top, bottom, left, right.
    wins: [HWND; 8],
    shown: bool,
    /// Glow thickness in physical px.
    m: i32,
    /// Top/bottom edge profiles stacked (max_w x 2m), left/right side by side (2m x max_h),
    /// and the four corners as quadrants (2m x 2m).
    horiz: Sheet,
    vert: Sheet,
    corners: Sheet,
}

thread_local! {
    static GLOW: RefCell<Option<Glow>> = const { RefCell::new(None) };
}

pub fn register(hinst: HINSTANCE) {
    unsafe {
        let class = WNDCLASSW {
            lpfnWndProc: Some(DefWindowProcW),
            hInstance: hinst,
            lpszClassName: w!("RSnapGlow"),
            ..zeroed()
        };
        RegisterClassW(&class);
    }
}

/// Opacity at `d` px outside the selection: solid core line, then a quadratic falloff.
fn alpha(d: f32, core: f32, size: f32) -> f32 {
    let fade = if d >= size {
        0.0
    } else {
        GLOW_FADE_ALPHA * (1.0 - ((d - core).max(0.0) / (size - core))).powi(2)
    };
    let k = (core + 0.5 - d).clamp(0.0, 1.0);
    k * GLOW_CORE_ALPHA + (1.0 - k) * fade
}

fn accent_color() -> u32 {
    let mut v = 0u32;
    let mut len = size_of::<u32>() as u32;
    let ok = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\DWM"),
            w!("AccentColor"),
            RRF_RT_REG_DWORD,
            null_mut(),
            &mut v as *mut u32 as *mut _,
            &mut len,
        )
    } == 0;
    let (r, g, b) = (v & 0xFF, (v >> 8) & 0xFF, (v >> 16) & 0xFF);
    // A near-black accent would make an invisible glow.
    let luma = (r * 299 + g * 587 + b * 114) / 1000;
    if ok && luma >= 80 {
        v & 0x00FF_FFFF
    } else {
        FALLBACK_COLOR
    }
}

pub fn create(dpi: u32, max_w: i32, max_h: i32) {
    let s = settings::current();
    let (glow_size, glow_core) = s.thickness.glow();
    let scale = dpi.max(96) as f32 / 96.0;
    let size = (glow_size * scale).round().max(2.0);
    let core = (glow_core * scale).max(1.0);
    let m = size as i32;

    let c = s.glow_color.unwrap_or_else(accent_color);
    let (r, g, b) = (
        (c & 0xFF) as f32,
        ((c >> 8) & 0xFF) as f32,
        ((c >> 16) & 0xFF) as f32,
    );
    let pixel = |d: f32| -> u32 {
        let a = alpha(d, core, size);
        let pm = |ch: f32| (ch * a / 255.0 + 0.5) as u32;
        ((a + 0.5) as u32) << 24 | pm(r) << 16 | pm(g) << 8 | pm(b)
    };
    // Distance from the selection edge for row/column k of a 2m-wide sheet.
    let dist = |k: i32| {
        if k < m {
            (m - k) as f32 - 0.5
        } else {
            (k - m) as f32 + 0.5
        }
    };
    let profile: Vec<u32> = (0..2 * m).map(|k| pixel(dist(k))).collect();

    let horiz = Sheet::new(max_w, 2 * m, |_, y| profile[y as usize]);
    let vert = Sheet::new(2 * m, max_h, |x, _| profile[x as usize]);
    let corners = Sheet::new(2 * m, 2 * m, |x, y| pixel(dist(x).hypot(dist(y))));

    let mut wins = [null_mut(); 8];
    unsafe {
        let hinst = GetModuleHandleW(null());
        for hwnd in &mut wins {
            *hwnd = CreateWindowExW(
                WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_TOPMOST
                    | WS_EX_TOOLWINDOW
                    | WS_EX_NOACTIVATE,
                w!("RSnapGlow"),
                null(),
                WS_POPUP,
                0,
                0,
                0,
                0,
                null_mut(),
                null_mut(),
                hinst,
                null(),
            );
            SetWindowDisplayAffinity(*hwnd, WDA_EXCLUDEFROMCAPTURE);
        }
    }
    GLOW.set(Some(Glow {
        wins,
        shown: false,
        m,
        horiz,
        vert,
        corners,
    }));
}

pub fn update(sel: windows_sys::Win32::Foundation::RECT) {
    GLOW.with(|cell| {
        let Ok(mut guard) = cell.try_borrow_mut() else {
            return;
        };
        let Some(g) = guard.as_mut() else { return };
        let m = g.m;
        let (l, t, r, b) = (sel.left, sel.top, sel.right, sel.bottom);
        let (w, h) = (r - l, b - t);
        // (x, y, w, h, sheet, src x, src y), in `wins` order.
        let parts: [(i32, i32, i32, i32, &Sheet, i32, i32); 8] = [
            (l - m, t - m, m, m, &g.corners, 0, 0),
            (r, t - m, m, m, &g.corners, m, 0),
            (l - m, b, m, m, &g.corners, 0, m),
            (r, b, m, m, &g.corners, m, m),
            (l, t - m, w, m, &g.horiz, 0, 0),
            (l, b, w, m, &g.horiz, 0, m),
            (l - m, t, m, h, &g.vert, 0, 0),
            (r, t, m, h, &g.vert, m, 0),
        ];
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        unsafe {
            for (&hwnd, (x, y, cx, cy, sheet, sx, sy)) in g.wins.iter().zip(parts) {
                UpdateLayeredWindow(
                    hwnd,
                    null_mut(),
                    &POINT { x, y },
                    &SIZE { cx, cy },
                    sheet.dc,
                    &POINT { x: sx, y: sy },
                    0,
                    &blend,
                    ULW_ALPHA,
                );
            }
            if !g.shown {
                for &hwnd in &g.wins {
                    ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                }
                g.shown = true;
            }
        }
    });
}

pub fn destroy() {
    let glow = GLOW.with(|cell| cell.try_borrow_mut().ok().and_then(|mut g| g.take()));
    if let Some(g) = glow {
        for hwnd in g.wins {
            unsafe { DestroyWindow(hwnd) };
        }
    }
}
