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
mod tray;

use std::mem::zeroed;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{
    ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, WPARAM,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{
    CreateMutexW, GetCurrentProcess, SetProcessWorkingSetSize,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;
use windows_sys::w;

pub const WM_APP_SNIP: u32 = WM_APP + 1;
pub const WM_APP_CANCEL: u32 = WM_APP + 2;
pub const WM_APP_TRAY: u32 = WM_APP + 3;

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
        tray::init(hwnd, hinst);
        hook::install(hwnd);
        trim();

        let mut msg = zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            DispatchMessageW(&msg);
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

fn trim() {
    if config::TRIM_AFTER_SNIP {
        unsafe { SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX) };
    }
}
