use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr::{copy_nonoverlapping, null, null_mut};
use std::time::Duration;

use windows_sys::Win32::Foundation::{GlobalFree, HGLOBAL, HWND, POINT};
use windows_sys::Win32::System::DataExchange::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Memory::{
    GMEM_MOVEABLE, GMEM_ZEROINIT, GlobalAlloc, GlobalLock, GlobalUnlock,
};
use windows_sys::Win32::System::Ole::{CF_DIB, CF_HDROP, CF_UNICODETEXT, DROPEFFECT_COPY};
use windows_sys::Win32::UI::Shell::DROPFILES;
use windows_sys::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, HWND_MESSAGE};
use windows_sys::w;

use crate::config::ClipboardMode;
use crate::encode;

pub fn set(px: &[u8], w: u32, h: u32, png: &[u8], file: Option<&Path>, mode: ClipboardMode) -> bool {
    with_clipboard(|| unsafe {
        let put_file = |f: &Path| {
            put(CF_HDROP as u32, hdrop(f));
            put(
                RegisterClipboardFormatW(w!("Preferred DropEffect")),
                global(&DROPEFFECT_COPY.to_ne_bytes()),
            );
        };
        let put_image = || {
            put(RegisterClipboardFormatW(w!("PNG")), global(png));
            put(CF_DIB as u32, encode::dib(px, w, h));
        };
        match (mode, file) {
            (_, None) => put_image(),
            (ClipboardMode::FileFirst, Some(f)) => {
                put_file(f);
                put_image();
            }
            (ClipboardMode::ImageFirst, Some(f)) => {
                put_image();
                put_file(f);
            }
            (ClipboardMode::FileOnly, Some(f)) => put_file(f),
        }
    })
}

/// Text only, as CF_UNICODETEXT. Windows synthesizes the ANSI and OEM text formats from it.
pub fn set_text(text: &str) -> bool {
    let wide: Vec<u16> = text.encode_utf16().chain([0]).collect();
    let bytes = unsafe { std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2) };
    with_clipboard(|| unsafe { put(CF_UNICODETEXT as u32, global(bytes)) })
}

/// Empties the clipboard, lets `fill` put formats on it, and closes it again.
/// Can sleep while another app holds the clipboard, so keep it off the main thread.
fn with_clipboard(fill: impl FnOnce()) -> bool {
    unsafe {
        // SetClipboardData needs an owner window on this thread.
        let owner = CreateWindowExW(
            0,
            w!("STATIC"),
            null(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            null_mut(),
            GetModuleHandleW(null()),
            null(),
        );
        let ok = !owner.is_null() && open(owner);
        if ok {
            EmptyClipboard();
            fill();
            CloseClipboard();
        }
        if !owner.is_null() {
            DestroyWindow(owner);
        }
        ok
    }
}

/// Win+V history and other watchers briefly lock the clipboard after every change.
fn open(owner: HWND) -> bool {
    for wait_ms in [0, 10, 20, 40, 80, 160] {
        std::thread::sleep(Duration::from_millis(wait_ms));
        if unsafe { OpenClipboard(owner) } != 0 {
            return true;
        }
    }
    false
}

/// The clipboard owns the memory on success; free it on failure.
unsafe fn put(format: u32, mem: Option<HGLOBAL>) {
    unsafe {
        if let Some(mem) = mem
            && SetClipboardData(format, mem).is_null()
        {
            GlobalFree(mem);
        }
    }
}

fn global(bytes: &[u8]) -> Option<HGLOBAL> {
    unsafe {
        let mem = GlobalAlloc(GMEM_MOVEABLE, bytes.len());
        if mem.is_null() {
            return None;
        }
        copy_nonoverlapping(bytes.as_ptr(), GlobalLock(mem) as *mut u8, bytes.len());
        GlobalUnlock(mem);
        Some(mem)
    }
}

fn hdrop(path: &Path) -> Option<HGLOBAL> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0, 0]).collect();
    let head = size_of::<DROPFILES>();
    unsafe {
        let mem = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, head + wide.len() * 2);
        if mem.is_null() {
            return None;
        }
        let base = GlobalLock(mem) as *mut u8;
        let df = DROPFILES {
            pFiles: head as u32,
            pt: POINT { x: 0, y: 0 },
            fNC: 0,
            fWide: 1,
        };
        copy_nonoverlapping(&df as *const _ as *const u8, base, head);
        copy_nonoverlapping(wide.as_ptr() as *const u8, base.add(head), wide.len() * 2);
        GlobalUnlock(mem);
        Some(mem)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // These overwrite your clipboard, so they only run on request, one at a time because they all
    // share it: `cargo test -- --ignored --test-threads=1`.

    fn read_text() -> String {
        unsafe {
            assert!(open(null_mut()));
            let mem = GetClipboardData(CF_UNICODETEXT as u32);
            assert!(!mem.is_null());
            let p = GlobalLock(mem) as *const u16;
            let len = (0..).take_while(|&i| *p.add(i) != 0).count();
            let text = String::from_utf16_lossy(std::slice::from_raw_parts(p, len));
            GlobalUnlock(mem);
            CloseClipboard();
            text
        }
    }

    #[test]
    #[ignore]
    fn set_text_round_trips() {
        assert!(set_text("SN BAT-7X4K29Q1\r\nŽluťoučký kůň"));
        assert_eq!(read_text(), "SN BAT-7X4K29Q1\r\nŽluťoučký kůň");
    }

    /// Win+V history and other watchers hold the clipboard briefly after every change.
    #[test]
    #[ignore]
    fn set_text_waits_for_a_busy_clipboard() {
        let (opened, wait) = std::sync::mpsc::channel();
        let holder = std::thread::spawn(move || unsafe {
            assert!(OpenClipboard(null_mut()) != 0);
            opened.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(100));
            CloseClipboard();
        });
        wait.recv().unwrap();
        assert!(set_text("still lands"));
        holder.join().unwrap();
        assert_eq!(read_text(), "still lands");
    }
}
