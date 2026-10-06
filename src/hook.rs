//! Low-level keyboard hook: takes Win+Shift+S from Windows, and eats Esc while snipping or while
//! the text popup is open.

use std::mem::{size_of, zeroed};
use std::ptr::null;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};

use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::{MAIN_WINDOW, WM_APP_CANCEL, WM_APP_SNIP, config, popup};

/// True while the crosshair is up.
pub static ACTIVE: AtomicBool = AtomicBool::new(false);
/// We ate the S key-down, so eat its key-up too.
static EAT_S_UP: AtomicBool = AtomicBool::new(false);
/// Tags our own injected keys so the hook lets them through.
const MARKER: usize = 0x5253_4E50;
const VK_S: u32 = b'S' as u32;

pub fn install() {
    unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(proc), GetModuleHandleW(null()), 0) };
}

fn held(vk: VIRTUAL_KEY) -> bool {
    unsafe { GetAsyncKeyState(vk as i32) < 0 }
}

/// Esc cancels the crosshair or closes the text popup. Taking it here rather than in the popup
/// means it works even when Windows refused the popup focus.
fn eats_esc(snipping: bool, popup_open: bool) -> bool {
    snipping || popup_open
}

unsafe extern "system" fn proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if code == HC_ACTION as i32 {
            let k = &*(lp as *const KBDLLHOOKSTRUCT);
            if k.dwExtraInfo != MARKER {
                let down = wp == WM_KEYDOWN as usize || wp == WM_SYSKEYDOWN as usize;
                let main = MAIN_WINDOW.load(Relaxed);
                if k.vkCode == VK_S {
                    if down
                        && (held(VK_LWIN) || held(VK_RWIN))
                        && held(VK_SHIFT)
                        && !held(VK_CONTROL)
                        && !held(VK_MENU)
                    {
                        EAT_S_UP.store(true, Relaxed);
                        // Auto-repeat while S is held must not retrigger.
                        if !ACTIVE.load(Relaxed) {
                            mask_start_menu();
                            PostMessageW(main, WM_APP_SNIP, 0, 0);
                        }
                        return 1;
                    }
                    if !down && EAT_S_UP.swap(false, Relaxed) {
                        return 1;
                    }
                } else if k.vkCode == VK_ESCAPE as u32
                    && eats_esc(ACTIVE.load(Relaxed), popup::is_open())
                {
                    if down {
                        PostMessageW(main, WM_APP_CANCEL, 0, 0);
                    }
                    return 1;
                }
            }
        }
        CallNextHookEx(std::ptr::null_mut(), code, wp, lp)
    }
}

/// Windows opens Start when Win goes down and up with no other key in between, and we ate the S.
/// Tapping an unassigned key breaks that sequence (the AutoHotkey trick).
unsafe fn mask_start_menu() {
    unsafe {
        let mut inputs: [INPUT; 2] = zeroed();
        for (input, flags) in inputs.iter_mut().zip([0, KEYEVENTF_KEYUP]) {
            input.r#type = INPUT_KEYBOARD;
            input.Anonymous.ki = KEYBDINPUT {
                wVk: config::MASK_KEY,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: MARKER,
            };
        }
        SendInput(2, inputs.as_ptr(), size_of::<INPUT>() as i32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn esc_belongs_to_rsnap_while_crosshair_or_popup_is_up() {
        assert!(eats_esc(true, false));
        // The popup may not have focus if Windows refused it; Esc must still close it.
        assert!(eats_esc(false, true));
        assert!(!eats_esc(false, false));
    }
}
