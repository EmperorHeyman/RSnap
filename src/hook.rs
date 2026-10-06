//! Low-level keyboard hook: takes the snip and text hotkeys from Windows, eats Esc while snipping
//! or while the text popup is open, and records combinations for the settings window.

use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, Ordering::Relaxed};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::hotkey::{self, ALT, Hotkey, WIN};
use crate::{MAIN_WINDOW, WM_APP_CANCEL, WM_APP_KEYREC, WM_APP_SNIP, config, popup};

/// True while the crosshair is up.
pub static ACTIVE: AtomicBool = AtomicBool::new(false);
/// The hotkeys, packed (see `Hotkey::pack`); 0 is "none". Set from the settings.
static SNIP_KEY: AtomicU32 = AtomicU32::new(config::SNIP_HOTKEY.pack());
static TEXT_KEY: AtomicU32 = AtomicU32::new(match config::TEXT_HOTKEY {
    Some(k) => k.pack(),
    None => 0,
});
/// The settings window while one of its hotkey boxes has focus: the next combination goes there.
static RECORD: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
/// We ate this key's key-down, so eat its key-up too.
static EAT_UP: AtomicU32 = AtomicU32::new(0);
/// Tags our own injected keys so the hook lets them through.
const MARKER: usize = 0x5253_4E50;
const ESC: u32 = VK_ESCAPE as u32;

pub fn install() {
    unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(proc), GetModuleHandleW(null()), 0) };
}

pub fn set_hotkeys(snip: Hotkey, text: Option<Hotkey>) {
    SNIP_KEY.store(snip.pack(), Relaxed);
    TEXT_KEY.store(text.map_or(0, Hotkey::pack), Relaxed);
}

/// Send the next combination to `target` as `WM_APP_KEYREC`; null stops recording.
pub fn record(target: HWND) {
    RECORD.store(target, Relaxed);
}

/// Everything a key-down decision depends on.
struct State {
    snip: Option<Hotkey>,
    text: Option<Hotkey>,
    recording: bool,
    snipping: bool,
    popup: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum Action {
    Pass,
    Snip { text: bool },
    /// Esc for the crosshair or the popup.
    Cancel,
    /// A packed combination for the settings window; 0 clears the box.
    Record(u32),
}

fn decide(vk: u32, mods: u8, s: &State) -> Action {
    const TAB: u32 = 0x09;
    const ENTER: u32 = 0x0D;
    const BACKSPACE: u32 = 0x08;
    const DELETE: u32 = 0x2E;
    if s.recording {
        return match vk {
            _ if hotkey::is_modifier(vk as u16) => Action::Pass,
            // The dialog keeps Tab, Enter (OK) and Esc (Cancel).
            TAB | ENTER | ESC => Action::Pass,
            BACKSPACE | DELETE if mods == 0 => Action::Record(0),
            _ => Action::Record(Hotkey::new(mods, vk as u16).pack()),
        };
    }
    if vk == ESC && (s.snipping || s.popup) {
        return Action::Cancel;
    }
    if s.snip.is_some_and(|k| k.matches(vk, mods)) {
        return Action::Snip { text: false };
    }
    if s.text.is_some_and(|k| k.matches(vk, mods)) {
        return Action::Snip { text: true };
    }
    Action::Pass
}

unsafe extern "system" fn proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if code == HC_ACTION as i32 {
            let k = &*(lp as *const KBDLLHOOKSTRUCT);
            if k.dwExtraInfo != MARKER && act(k.vkCode, wp) {
                return 1;
            }
        }
        CallNextHookEx(null_mut(), code, wp, lp)
    }
}

/// True to eat the key.
unsafe fn act(vk: u32, wp: WPARAM) -> bool {
    let down = wp == WM_KEYDOWN as usize || wp == WM_SYSKEYDOWN as usize;
    if !down {
        return EAT_UP.compare_exchange(vk, 0, Relaxed, Relaxed).is_ok();
    }
    let (snip, text) = (SNIP_KEY.load(Relaxed), TEXT_KEY.load(Relaxed));
    let recorder = RECORD.load(Relaxed);
    // Most keystrokes stop here, before any key state is read.
    let key_of = |packed: u32| packed & 0xFFFF;
    if recorder.is_null() && vk != ESC && vk != key_of(snip) && vk != key_of(text) {
        return false;
    }
    let mods = hotkey::held();
    let state = State {
        snip: Hotkey::unpack(snip),
        text: Hotkey::unpack(text),
        recording: !recorder.is_null(),
        snipping: ACTIVE.load(Relaxed),
        popup: popup::is_open(),
    };
    unsafe {
        match decide(vk, mods, &state) {
            Action::Pass => return false,
            Action::Cancel => {
                PostMessageW(MAIN_WINDOW.load(Relaxed), WM_APP_CANCEL, 0, 0);
            }
            // Auto-repeat while the key is held must not retrigger.
            Action::Snip { .. } if state.snipping => {}
            Action::Snip { text } => {
                mask_menus(mods);
                PostMessageW(MAIN_WINDOW.load(Relaxed), WM_APP_SNIP, text as WPARAM, 0);
            }
            Action::Record(packed) => {
                mask_menus(mods);
                PostMessageW(recorder, WM_APP_KEYREC, packed as WPARAM, 0);
            }
        }
    }
    EAT_UP.store(vk, Relaxed);
    true
}

/// We ate a key pressed with Win or Alt held. Letting go of Win alone opens Start, and Alt alone
/// opens the focused app's menu bar.
unsafe fn mask_menus(mods: u8) {
    if mods & (WIN | ALT) != 0 {
        unsafe { mask_start_menu() };
    }
}

/// Windows opens Start (or a menu bar, for Alt) when the modifier goes down and up with no other key
/// in between, and we ate the key. Tapping an unassigned key breaks that sequence (the AutoHotkey trick).
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
    use crate::hotkey::{ALT, CTRL, SHIFT, WIN};

    const S: u32 = b'S' as u32;
    const T: u32 = b'T' as u32;
    const ESC: u32 = 0x1B;

    fn state() -> State {
        State {
            snip: Some(Hotkey::new(WIN | SHIFT, S as u16)),
            text: Some(Hotkey::new(WIN | SHIFT, T as u16)),
            recording: false,
            snipping: false,
            popup: false,
        }
    }

    #[test]
    fn hotkeys_start_a_snip() {
        assert_eq!(decide(S, WIN | SHIFT, &state()), Action::Snip { text: false });
        assert_eq!(decide(T, WIN | SHIFT, &state()), Action::Snip { text: true });
        assert_eq!(decide(S, WIN | SHIFT | CTRL, &state()), Action::Pass);
        assert_eq!(decide(S, 0, &state()), Action::Pass);
        let no_text = State { text: None, ..state() };
        assert_eq!(decide(T, WIN | SHIFT, &no_text), Action::Pass);
    }

    #[test]
    fn esc_belongs_to_rsnap_while_crosshair_or_popup_is_up() {
        assert_eq!(decide(ESC, 0, &State { snipping: true, ..state() }), Action::Cancel);
        // The popup may not have focus if Windows refused it; Esc must still close it.
        assert_eq!(decide(ESC, 0, &State { popup: true, ..state() }), Action::Cancel);
        assert_eq!(decide(ESC, 0, &state()), Action::Pass);
    }

    #[test]
    fn recording_takes_the_next_combination() {
        let rec = State { recording: true, ..state() };
        // Even the current snip hotkey is recorded rather than starting a snip.
        assert_eq!(decide(S, WIN | SHIFT, &rec), Action::Record(Hotkey::new(WIN | SHIFT, S as u16).pack()));
        assert_eq!(decide(b'K' as u32, CTRL | ALT, &rec), Action::Record(Hotkey::new(CTRL | ALT, b'K' as u16).pack()));
        // Backspace or Delete alone clears the box.
        assert_eq!(decide(0x08, 0, &rec), Action::Record(0));
        assert_eq!(decide(0x2E, 0, &rec), Action::Record(0));
    }

    #[test]
    fn recording_leaves_modifiers_and_dialog_keys_alone() {
        let rec = State { recording: true, ..state() };
        assert_eq!(decide(0xA0, SHIFT, &rec), Action::Pass); // Left Shift going down
        assert_eq!(decide(0x5B, WIN, &rec), Action::Pass); // Left Win going down
        assert_eq!(decide(0x09, 0, &rec), Action::Pass); // Tab moves to the next field
        assert_eq!(decide(0x09, SHIFT, &rec), Action::Pass);
        assert_eq!(decide(0x0D, 0, &rec), Action::Pass); // Enter is OK
        assert_eq!(decide(ESC, 0, &rec), Action::Pass); // Esc is Cancel
    }
}
