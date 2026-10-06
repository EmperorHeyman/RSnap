//! Invisible full-screen window that shows the crosshair and tracks the drag.
//! It has no surface at all, so it draws nothing, and it never takes focus.

use std::cell::Cell;
use std::mem::zeroed;
use std::ptr::{null, null_mut};
use std::sync::atomic::Ordering::Relaxed;

use windows_sys::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SetCapture, VK_CONTROL, VK_SHIFT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;
use windows_sys::w;

use crate::{capture, config, glow, hook};

const MK_SHIFT: usize = 0x0004;
const MK_CONTROL: usize = 0x0008;

/// What a finished drag produces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    /// PNG, DIB and a named file; `save` also writes it to Pictures.
    Image { save: bool },
    /// OCR'd text and the popup.
    Text,
}

thread_local! {
    static OVERLAY: Cell<HWND> = const { Cell::new(null_mut()) };
    /// Virtual screen. The origin is negative when a monitor sits left of or above the primary.
    static BOUNDS: Cell<RECT> = const { Cell::new(RECT { left: 0, top: 0, right: 0, bottom: 0 }) };
    static ANCHOR: Cell<Option<POINT>> = const { Cell::new(None) };
}

pub fn register(hinst: HINSTANCE) {
    unsafe {
        let class = WNDCLASSW {
            lpfnWndProc: Some(proc),
            hInstance: hinst,
            hCursor: LoadCursorW(null_mut(), IDC_CROSS),
            lpszClassName: w!("RSnapOverlay"),
            ..zeroed()
        };
        RegisterClassW(&class);
    }
}

pub fn start() {
    if hook::ACTIVE.swap(true, Relaxed) {
        return;
    }
    unsafe {
        let b = RECT {
            left: GetSystemMetrics(SM_XVIRTUALSCREEN),
            top: GetSystemMetrics(SM_YVIRTUALSCREEN),
            right: GetSystemMetrics(SM_XVIRTUALSCREEN) + GetSystemMetrics(SM_CXVIRTUALSCREEN),
            bottom: GetSystemMetrics(SM_YVIRTUALSCREEN) + GetSystemMetrics(SM_CYVIRTUALSCREEN),
        };
        let hwnd = CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            w!("RSnapOverlay"),
            null(),
            WS_POPUP,
            b.left,
            b.top,
            b.right - b.left,
            b.bottom - b.top,
            null_mut(),
            null_mut(),
            GetModuleHandleW(null()),
            null(),
        );
        if hwnd.is_null() {
            hook::ACTIVE.store(false, Relaxed);
            return;
        }
        SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE);
        OVERLAY.set(hwnd);
        BOUNDS.set(b);
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        SetCursor(LoadCursorW(null_mut(), IDC_CROSS));
        // Glow windows are created after the overlay so they sit above it.
        glow::create(GetDpiForWindow(hwnd), b.right - b.left, b.bottom - b.top);
    }
}

pub fn cancel() {
    ANCHOR.set(None);
    let hwnd = OVERLAY.replace(null_mut());
    glow::destroy();
    if !hwnd.is_null() {
        unsafe { DestroyWindow(hwnd) };
    }
    hook::ACTIVE.store(false, Relaxed);
}

/// Shift asks for text and wins over Ctrl.
fn output(ctrl: bool, shift: bool) -> Output {
    if shift {
        Output::Text
    } else {
        Output::Image { save: ctrl }
    }
}

fn finish(sel: RECT, out: Output) {
    let (w, h) = (sel.right - sel.left, sel.bottom - sel.top);
    if w < config::MIN_DRAG && h < config::MIN_DRAG {
        cancel();
        return;
    }
    // Overlay and glow are excluded from capture, so grab before tearing them down.
    let shot = capture::grab(sel);
    cancel();
    if let Some(shot) = shot {
        // Stack is only reserved, not committed; the OCR path calls into WinRT code we don't control.
        let stack = match out {
            Output::Image { .. } => 256 * 1024,
            Output::Text => 1024 * 1024,
        };
        let _ = std::thread::Builder::new()
            .stack_size(stack)
            .spawn(move || match out {
                Output::Image { save } => crate::deliver(shot, save),
                Output::Text => crate::deliver_text(shot, sel),
            });
    }
}

/// Client to screen coordinates, clamped. Client ones go negative while the mouse is captured.
fn to_screen(lp: LPARAM) -> POINT {
    let b = BOUNDS.get();
    let x = (lp & 0xFFFF) as u16 as i16 as i32 + b.left;
    let y = ((lp >> 16) & 0xFFFF) as u16 as i16 as i32 + b.top;
    POINT {
        x: x.clamp(b.left, b.right - 1),
        y: y.clamp(b.top, b.bottom - 1),
    }
}

/// Both corner pixels are included.
fn selection(a: POINT, b: POINT) -> RECT {
    RECT {
        left: a.x.min(b.x),
        top: a.y.min(b.y),
        right: a.x.max(b.x) + 1,
        bottom: a.y.max(b.y) + 1,
    }
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_MOUSEACTIVATE => return MA_NOACTIVATE as LRESULT,
            WM_LBUTTONDOWN => {
                let p = to_screen(lp);
                ANCHOR.set(Some(p));
                SetCapture(hwnd);
                glow::update(selection(p, p));
            }
            WM_MOUSEMOVE => {
                if let Some(a) = ANCHOR.get() {
                    glow::update(selection(a, to_screen(lp)));
                }
            }
            WM_LBUTTONUP => {
                if let Some(a) = ANCHOR.take() {
                    let ctrl = wp & MK_CONTROL != 0 || GetAsyncKeyState(VK_CONTROL as i32) < 0;
                    let shift = wp & MK_SHIFT != 0 || GetAsyncKeyState(VK_SHIFT as i32) < 0;
                    finish(selection(a, to_screen(lp)), output(ctrl, shift));
                }
            }
            WM_RBUTTONDOWN => cancel(),
            // Something stole the mouse mid-drag: bail out rather than get stuck.
            WM_CAPTURECHANGED => {
                if ANCHOR.get().is_some() {
                    cancel();
                }
            }
            _ => return DefWindowProcW(hwnd, msg, wp, lp),
        }
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_keys_pick_the_output() {
        assert_eq!(output(false, false), Output::Image { save: false });
        assert_eq!(output(true, false), Output::Image { save: true });
        assert_eq!(output(false, true), Output::Text);
        assert_eq!(output(true, true), Output::Text);
    }
}
