use std::cell::Cell;
use std::mem::{size_of, zeroed};
use std::os::windows::ffi::OsStrExt;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};

use windows_sys::Win32::Foundation::{HINSTANCE, HWND, LPARAM, POINT};
use windows_sys::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;
use windows_sys::core::PCWSTR;
use windows_sys::w;

use crate::{WM_APP_SNIP, WM_APP_TRAY, files};

const ID_OPEN: usize = 1;
const ID_AUTOSTART: usize = 2;
const ID_EXIT: usize = 3;
const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const RUN_VALUE: PCWSTR = w!("RSnap");

static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(u32::MAX);

thread_local! {
    static OWNER: Cell<HWND> = const { Cell::new(null_mut()) };
    static INSTANCE: Cell<HINSTANCE> = const { Cell::new(null_mut()) };
}

pub fn init(hwnd: HWND, hinst: HINSTANCE) {
    OWNER.set(hwnd);
    INSTANCE.set(hinst);
    TASKBAR_CREATED.store(
        unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) },
        Relaxed,
    );
    // Follow the exe if it was moved.
    if autostart_enabled() {
        set_autostart(true);
    }
    add();
}

pub fn is_taskbar_created(msg: u32) -> bool {
    msg == TASKBAR_CREATED.load(Relaxed)
}

fn data() -> NOTIFYICONDATAW {
    let mut nid: NOTIFYICONDATAW = unsafe { zeroed() };
    nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = OWNER.get();
    nid.uID = 1;
    nid
}

/// Also called when Explorer restarts.
pub fn add() {
    unsafe {
        let mut nid = data();
        nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        nid.uCallbackMessage = WM_APP_TRAY;
        nid.hIcon = LoadImageW(
            INSTANCE.get(),
            1 as PCWSTR,
            IMAGE_ICON,
            GetSystemMetrics(SM_CXSMICON),
            GetSystemMetrics(SM_CYSMICON),
            LR_DEFAULTCOLOR,
        ) as HICON;
        let tip: Vec<u16> = "RSnap \u{2014} Win+Shift+S".encode_utf16().collect();
        nid.szTip[..tip.len()].copy_from_slice(&tip);
        Shell_NotifyIconW(NIM_ADD, &nid);
    }
}

pub fn remove() {
    unsafe { Shell_NotifyIconW(NIM_DELETE, &data()) };
}

pub fn on_event(hwnd: HWND, lp: LPARAM) {
    match lp as u32 {
        WM_LBUTTONUP => unsafe {
            PostMessageW(hwnd, WM_APP_SNIP, 0, 0);
        },
        WM_RBUTTONUP | WM_CONTEXTMENU => menu(hwnd),
        _ => {}
    }
}

fn menu(hwnd: HWND) {
    unsafe {
        let m = CreatePopupMenu();
        let checked = if autostart_enabled() { MF_CHECKED } else { 0 };
        AppendMenuW(m, MF_STRING, ID_OPEN, w!("Open folder"));
        AppendMenuW(
            m,
            MF_STRING | checked,
            ID_AUTOSTART,
            w!("Start with Windows"),
        );
        AppendMenuW(m, MF_SEPARATOR, 0, null());
        AppendMenuW(m, MF_STRING, ID_EXIT, w!("Exit"));

        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        // Without this the menu doesn't close when you click elsewhere.
        SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(
            m,
            TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY,
            pt.x,
            pt.y,
            0,
            hwnd,
            null(),
        );
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(m);

        match cmd as usize {
            ID_OPEN => open_folder(),
            ID_AUTOSTART => set_autostart(checked == 0),
            ID_EXIT => {
                DestroyWindow(hwnd);
            }
            _ => {}
        }
    }
}

/// Spawn Explorer: ShellExecute would load half the shell into our process for good.
fn open_folder() {
    let dir = files::save_dir().unwrap_or_else(files::temp_dir);
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::process::Command::new("explorer.exe").arg(&dir).spawn();
}

fn autostart_enabled() -> bool {
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            RUN_VALUE,
            RRF_RT_REG_SZ,
            null_mut(),
            null_mut(),
            null_mut(),
        ) == 0
    }
}

fn set_autostart(on: bool) {
    unsafe {
        if on {
            let Ok(exe) = std::env::current_exe() else {
                return;
            };
            let value: Vec<u16> = std::ffi::OsStr::new("\"")
                .encode_wide()
                .chain(exe.as_os_str().encode_wide())
                .chain("\"\0".encode_utf16())
                .collect();
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                RUN_KEY,
                RUN_VALUE,
                REG_SZ,
                value.as_ptr() as *const _,
                (value.len() * 2) as u32,
            );
        } else {
            RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE);
        }
    }
}
