#![cfg_attr(not(test), windows_subsystem = "windows")]

mod capture;
mod clipboard;
mod config;
mod encode;
mod files;
mod glow;
mod hook;
mod ocr;
mod overlay;
mod popup;
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

pub const WM_APP_SNIP: u32 = WM_APP + 1;
pub const WM_APP_CANCEL: u32 = WM_APP + 2;
pub const WM_APP_TRAY: u32 = WM_APP + 3;
/// From the OCR worker: lParam is a `Box<(String, RECT)>` for the popup.
pub const WM_APP_OCR: u32 = WM_APP + 4;

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
        tray::init(hwnd, hinst);
        hook::install();
        trim();

        let mut msg = zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            if !popup::pre_translate(&msg) {
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
            WM_APP_SNIP => overlay::start(),
            WM_APP_CANCEL => overlay::cancel(),
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
    clipboard::set(px, w, h, &png, file.as_deref());

    drop(png);
    drop(shot);
    files::sweep_temp();
    trim();
}

/// The Shift-release worker: OCR, text on the clipboard, then the popup on the main thread.
pub fn deliver_text(shot: capture::Shot, sel: RECT) {
    let text = ocr::recognize(shot.pixels(), shot.w as u32, shot.h as u32);
    drop(shot);
    match text {
        Some(text) => {
            clipboard::set_text(&text);
            let msg = Box::into_raw(Box::new((text, sel)));
            let main = MAIN_WINDOW.load(Relaxed);
            if unsafe { PostMessageW(main, WM_APP_OCR, 0, msg as LPARAM) } == 0 {
                drop(unsafe { Box::from_raw(msg) });
            }
        }
        None => unsafe {
            MessageBeep(MB_ICONWARNING);
        },
    }
    trim();
}

pub fn trim() {
    if config::TRIM_AFTER_SNIP {
        unsafe { SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX) };
    }
}
