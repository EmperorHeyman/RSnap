#![cfg_attr(not(test), windows_subsystem = "windows")]

mod capture;
mod clipboard;
mod config;
mod encode;
mod files;
mod glow;
mod hook;
mod hotkey;
mod ocr;
mod overlay;
mod popup;
mod ids;
mod settings;
mod settings_window;
mod tray;

use std::ffi::c_void;
use std::mem::zeroed;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicPtr, Ordering::Relaxed};

use windows_sys::Win32::Foundation::{
    ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, RECT, WPARAM,
};
use windows_sys::Win32::System::Diagnostics::Debug::MessageBeep;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{
    CreateMutexW, GetCurrentProcess, SetProcessWorkingSetSize,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;
use windows_sys::w;

/// wParam 1 opens the crosshair in text mode.
pub const WM_APP_SNIP: u32 = WM_APP + 1;
pub const WM_APP_CANCEL: u32 = WM_APP + 2;
pub const WM_APP_TRAY: u32 = WM_APP + 3;
/// From the OCR worker: lParam is a `Box<(String, RECT)>` for the popup.
pub const WM_APP_OCR: u32 = WM_APP + 4;
/// From the keyboard hook to the settings window: wParam is a packed `Hotkey`, 0 to clear.
pub const WM_APP_KEYREC: u32 = WM_APP + 5;

/// The hidden main window. The keyboard hook and the OCR worker post to it.
pub static MAIN_WINDOW: AtomicPtr<c_void> = AtomicPtr::new(null_mut());

fn main() {
    unsafe {
        // Single instance.
        CreateMutexW(null(), 0, w!("Local\\RSnap.SingleInstance"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return;
        }

        let hinst = GetModuleHandleW(null());
        let class = WNDCLASSW {
            lpfnWndProc: Some(main_proc),
            hInstance: hinst,
            lpszClassName: w!("RSnapMain"),
            ..zeroed()
        };
        RegisterClassW(&class);
        overlay::register(hinst);
        glow::register(hinst);
        popup::register(hinst);

        // Hidden top-level window, not message-only: it has to hear the TaskbarCreated broadcast.
        let hwnd = CreateWindowExW(
            0,
            w!("RSnapMain"),
            w!("RSnap"),
            0,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            hinst,
            null(),
        );
        MAIN_WINDOW.store(hwnd, Relaxed);
        settings::load();
        tray::init(hwnd, hinst);
        hook::install();
        trim();

        let mut msg = zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            if !popup::pre_translate(&msg) && !settings_window::pre_translate(&msg) {
                // The popup's edit box needs WM_CHAR to take typing.
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        tray::remove();
    }
}

unsafe extern "system" fn main_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_APP_SNIP => overlay::start(wp == 1),
            // Esc: whichever of the crosshair and the text popup is up.
            WM_APP_CANCEL => {
                overlay::cancel();
                popup::close();
            }
            WM_APP_TRAY => tray::on_event(hwnd, lp),
            WM_APP_OCR => popup::on_text(lp),
            WM_DESTROY => PostQuitMessage(0),
            _ if tray::is_taskbar_created(msg) => tray::add(),
            _ => return DefWindowProcW(hwnd, msg, wp, lp),
        }
        0
    }
}

/// Runs on a short-lived worker thread so encoding and clipboard retries never stall the keyboard hook.
pub fn deliver(shot: capture::Shot, save: bool) {
    let (w, h) = (shot.w as u32, shot.h as u32);
    let px = shot.pixels();
    let Some(png) = encode::png(px, w, h) else {
        return;
    };

    let name = files::random_name();
    let file = save
        .then(files::save_dir)
        .flatten()
        .and_then(|dir| files::write(&dir, &name, &png))
        .or_else(|| files::write(&files::temp_dir(), &name, &png));
    clipboard::set(px, w, h, &png, file.as_deref(), settings::current().clipboard);

    drop(png);
    drop(shot);
    files::sweep_temp();
    trim();
}

/// The Shift-release worker: OCR, text on the clipboard, then the popup on the main thread.
pub fn deliver_text(shot: capture::Shot, sel: RECT) {
    let s = settings::current();
    let text = ocr::recognize(shot.pixels(), shot.w as u32, shot.h as u32, s.ocr_language.as_deref())
        .map(|t| if s.fix_codes { ocr::fix_codes(&t) } else { t });
    drop(shot);
    match text {
        Some(text) => {
            copy_text(&text);
            let msg = Box::into_raw(Box::new((text, sel)));
            let main = MAIN_WINDOW.load(Relaxed);
            if unsafe { PostMessageW(main, WM_APP_OCR, 0, msg as LPARAM) } == 0 {
                drop(unsafe { Box::from_raw(msg) });
            }
        }
        None => warn(),
    }
    trim();
}

/// Text on the clipboard, or a warning beep if another app kept it locked through every retry,
/// so a paste never silently brings back the previous clipboard. Worker threads only.
pub fn copy_text(text: &str) -> bool {
    let ok = clipboard::set_text(text);
    if !ok {
        warn();
    }
    ok
}

fn warn() {
    unsafe { MessageBeep(MB_ICONWARNING) };
}

pub fn trim() {
    if config::TRIM_AFTER_SNIP {
        unsafe { SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::System::DataExchange::{CloseClipboard, OpenClipboard};

    /// Another app holds the clipboard past every retry. Needs the real clipboard, alone:
    /// `cargo test -- --ignored --test-threads=1`.
    #[test]
    #[ignore]
    fn copy_text_reports_a_locked_clipboard() {
        let (opened, wait) = std::sync::mpsc::channel();
        let holder = std::thread::spawn(move || unsafe {
            assert!(OpenClipboard(null_mut()) != 0);
            opened.send(()).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(600));
            CloseClipboard();
        });
        wait.recv().unwrap();
        assert!(!copy_text("never lands"));
        holder.join().unwrap();
    }
}
