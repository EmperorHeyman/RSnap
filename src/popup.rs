//! The box that shows OCR'd text under the snip. Unlike the crosshair it takes focus, so a misread
//! can be fixed before copying. Enter copies and closes; Esc or clicking elsewhere just closes.

use std::cell::Cell;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use std::sync::atomic::Ordering::Relaxed;

use windows_sys::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::{EM_GETSEL, EM_SETMARGINS, EM_SETSEL};
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, SetFocus, VIRTUAL_KEY, VK_A, VK_CONTROL, VK_ESCAPE, VK_RETURN, VK_SHIFT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;
use windows_sys::w;

use crate::config::{POPUP_FONT_PT, POPUP_MAX_LINES, POPUP_MAX_W, POPUP_MIN_W};
use crate::{clipboard, hook};

/// Space between the snip and the popup, and around the text inside it, in px at 100% scaling.
const GAP: i32 = 8;
const PAD: i32 = 6;

thread_local! {
    static POPUP: Cell<HWND> = const { Cell::new(null_mut()) };
    static EDIT: Cell<HWND> = const { Cell::new(null_mut()) };
    static FONT: Cell<HFONT> = const { Cell::new(null_mut()) };
}

pub fn register(hinst: HINSTANCE) {
    unsafe {
        let class = WNDCLASSW {
            lpfnWndProc: Some(proc),
            hInstance: hinst,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: (COLOR_WINDOW + 1) as usize as HBRUSH,
            lpszClassName: w!("RSnapPopup"),
            ..zeroed()
        };
        RegisterClassW(&class);
    }
}

/// Takes ownership of the `Box<(String, RECT)>` the OCR worker posted with `WM_APP_OCR`.
pub fn on_text(lp: LPARAM) {
    let (text, sel) = *unsafe { Box::from_raw(lp as *mut (String, RECT)) };
    // A new snip started while OCR ran. The text is on the clipboard already; don't cover the crosshair.
    if !hook::ACTIVE.load(Relaxed) {
        show(&text, sel);
    }
}

fn show(text: &str, sel: RECT) {
    close();
    unsafe {
        let mon = MonitorFromRect(&sel, MONITOR_DEFAULTTONEAREST);
        let mut info: MONITORINFO = zeroed();
        info.cbSize = size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(mon, &mut info);
        let work = info.rcWork;
        let (mut dpi, mut dpi_y) = (96, 96);
        GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dpi, &mut dpi_y);
        let px = |v: i32| v * dpi as i32 / 96;

        let font = CreateFontW(
            -(POPUP_FONT_PT * dpi as i32 / 72),
            0,
            0,
            0,
            FW_NORMAL as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            0,
            0,
            CLEARTYPE_QUALITY as u32,
            0,
            w!("Segoe UI"),
        );
        let wide: Vec<u16> = text.encode_utf16().chain([0]).collect();

        // 1 px border on each side, then the padding, then the text.
        let pad = px(PAD);
        let width = (sel.right - sel.left)
            .clamp(px(POPUP_MIN_W), px(POPUP_MAX_W))
            .min(work.right - work.left);
        let text_w = width - 2 - 2 * pad;
        let (line, text_h) = measure(font, &wide, text_w);
        let max_h = line * POPUP_MAX_LINES;
        let scroll = text_h > max_h;
        let edit_h = text_h.clamp(line, max_h);
        let height = edit_h + 2 + 2 * pad;
        let pos = place(sel, width, height, work, px(GAP));

        let hinst = GetModuleHandleW(null());
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            w!("RSnapPopup"),
            null(),
            WS_POPUP | WS_BORDER,
            pos.x,
            pos.y,
            width,
            height,
            null_mut(),
            null_mut(),
            hinst,
            null(),
        );
        if hwnd.is_null() {
            DeleteObject(font);
            return;
        }
        let style = WS_CHILD
            | WS_VISIBLE
            | (ES_MULTILINE | ES_AUTOVSCROLL) as u32
            | if scroll { WS_VSCROLL } else { 0 };
        let edit = CreateWindowExW(
            0,
            w!("EDIT"),
            wide.as_ptr(),
            style,
            pad,
            pad,
            text_w,
            edit_h,
            hwnd,
            null_mut(),
            hinst,
            null(),
        );
        SendMessageW(edit, WM_SETFONT, font as WPARAM, 0);
        // The padding comes from the popup, so the text wraps exactly where `measure` said.
        SendMessageW(edit, EM_SETMARGINS, (EC_LEFTMARGIN | EC_RIGHTMARGIN) as WPARAM, 0);
        // Caret at the start, so a long text shows its first lines.
        SendMessageW(edit, EM_SETSEL, 0, 0);
        POPUP.set(hwnd);
        EDIT.set(edit);
        FONT.set(font);

        ShowWindow(hwnd, SW_SHOW);
        // Allowed because RSnap just received the mouse click that ended the snip.
        SetForegroundWindow(hwnd);
        SetFocus(edit);
    }
}

/// Line height and wrapped text height for `wide` (NUL-terminated) in `font` at `width` px.
fn measure(font: HFONT, wide: &[u16], width: i32) -> (i32, i32) {
    unsafe {
        let dc = GetDC(null_mut());
        let old = SelectObject(dc, font);
        let mut tm: TEXTMETRICW = zeroed();
        GetTextMetricsW(dc, &mut tm);
        let mut rc = RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: 0,
        };
        DrawTextW(
            dc,
            wide.as_ptr(),
            -1,
            &mut rc,
            DT_CALCRECT | DT_WORDBREAK | DT_EDITCONTROL | DT_NOPREFIX,
        );
        SelectObject(dc, old);
        ReleaseDC(null_mut(), dc);
        (tm.tmHeight, rc.bottom)
    }
}

/// Under the snip, else above it, else at the bottom of the work area; never off the sides.
fn place(sel: RECT, w: i32, h: i32, work: RECT, gap: i32) -> POINT {
    let x = sel.left.min(work.right - w).max(work.left);
    let y = if sel.bottom + gap + h <= work.bottom {
        sel.bottom + gap
    } else if sel.top - gap - h >= work.top {
        sel.top - gap - h
    } else {
        work.bottom - h
    };
    POINT {
        x,
        y: y.max(work.top),
    }
}

/// Enter, Esc and Ctrl+A for the edit box, checked before TranslateMessage. True when handled.
pub fn pre_translate(msg: &MSG) -> bool {
    let edit = EDIT.get();
    if edit.is_null() || msg.hwnd != edit || msg.message != WM_KEYDOWN {
        return false;
    }
    let held = |vk: VIRTUAL_KEY| unsafe { GetKeyState(vk as i32) } < 0;
    match msg.wParam as VIRTUAL_KEY {
        VK_RETURN if !held(VK_SHIFT) => copy_and_close(edit),
        VK_ESCAPE => close(),
        // The stock edit box has no select-all shortcut.
        VK_A if held(VK_CONTROL) => unsafe {
            SendMessageW(edit, EM_SETSEL, 0, -1);
        },
        _ => return false,
    }
    true
}

fn copy_and_close(edit: HWND) {
    let text = chosen_text(edit);
    close();
    // set_text can wait on a busy clipboard, and the keyboard hook shares this thread.
    let _ = std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(move || clipboard::set_text(&text));
}

/// The selection, or everything when nothing is selected. Includes the user's edits.
fn chosen_text(edit: HWND) -> String {
    unsafe {
        let mut buf = vec![0u16; GetWindowTextLengthW(edit) as usize + 1];
        let n = GetWindowTextW(edit, buf.as_mut_ptr(), buf.len() as i32);
        buf.truncate(n.max(0) as usize);
        let (mut start, mut end) = (0u32, 0u32);
        SendMessageW(
            edit,
            EM_GETSEL,
            &mut start as *mut u32 as WPARAM,
            &mut end as *mut u32 as LPARAM,
        );
        String::from_utf16_lossy(selection_or_all(&buf, start as usize, end as usize))
    }
}

fn selection_or_all(text: &[u16], start: usize, end: usize) -> &[u16] {
    let end = end.min(text.len());
    if start < end { &text[start..end] } else { text }
}

/// Also called when a new snip starts. Does nothing if no popup is open.
pub fn close() {
    let hwnd = POPUP.replace(null_mut());
    if hwnd.is_null() {
        return;
    }
    EDIT.set(null_mut());
    unsafe {
        DestroyWindow(hwnd);
        DeleteObject(FONT.replace(null_mut()));
    }
    crate::trim();
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            // Clicked elsewhere or Alt+Tabbed away. Close once the activation change is done.
            WM_ACTIVATE if (wp & 0xFFFF) as u32 == WA_INACTIVE => {
                PostMessageW(hwnd, WM_CLOSE, 0, 0);
            }
            // DefWindowProc would focus the popup itself rather than the edit box.
            WM_ACTIVATE => {
                SetFocus(EDIT.get());
            }
            WM_CLOSE => close(),
            _ => return DefWindowProcW(hwnd, msg, wp, lp),
        }
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: RECT = RECT {
        left: 0,
        top: 0,
        right: 1920,
        bottom: 1040,
    };

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT {
            left,
            top,
            right,
            bottom,
        }
    }

    fn at(p: POINT) -> (i32, i32) {
        (p.x, p.y)
    }

    #[test]
    fn goes_under_the_snip() {
        assert_eq!(at(place(rect(100, 100, 400, 200), 300, 80, WORK, 8)), (100, 208));
    }

    #[test]
    fn flips_above_when_there_is_no_room_below() {
        assert_eq!(at(place(rect(100, 900, 400, 1000), 300, 80, WORK, 8)), (100, 812));
    }

    #[test]
    fn pins_to_the_bottom_when_neither_fits() {
        // The snip covers nearly the whole height.
        assert_eq!(at(place(rect(100, 20, 400, 1030), 300, 80, WORK, 8)), (100, 960));
    }

    #[test]
    fn stays_inside_left_and_right() {
        assert_eq!(at(place(rect(1800, 100, 1900, 200), 300, 80, WORK, 8)), (1620, 208));
        // Monitor left of the primary: negative coordinates.
        let left = rect(-1920, 0, 0, 1040);
        assert_eq!(at(place(rect(-1925, 100, -1800, 200), 300, 80, left, 8)), (-1920, 208));
    }

    #[test]
    fn copies_selection_or_everything() {
        let text: Vec<u16> = "SN 123".encode_utf16().collect();
        assert_eq!(String::from_utf16_lossy(selection_or_all(&text, 3, 6)), "123");
        assert_eq!(String::from_utf16_lossy(selection_or_all(&text, 2, 2)), "SN 123");
        assert_eq!(String::from_utf16_lossy(selection_or_all(&text, 3, 99)), "123");
    }
}
