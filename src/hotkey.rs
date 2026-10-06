//! A key combination: any of Win, Ctrl, Alt and Shift plus one key.

use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyNameTextW, MAPVK_VK_TO_VSC, MapVirtualKeyW, VIRTUAL_KEY, VK_CONTROL,
    VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
};

pub const WIN: u8 = 1;
pub const CTRL: u8 = 2;
pub const ALT: u8 = 4;
pub const SHIFT: u8 = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hotkey {
    pub mods: u8,
    pub vk: u16,
}

impl Hotkey {
    pub const fn new(mods: u8, vk: u16) -> Hotkey {
        Hotkey { mods, vk }
    }

    /// One number, for the registry and the hook's atomics. 0 is "no hotkey".
    pub const fn pack(self) -> u32 {
        (self.mods as u32) << 16 | self.vk as u32
    }

    pub const fn unpack(v: u32) -> Option<Hotkey> {
        let vk = (v & 0xFFFF) as u16;
        if vk == 0 {
            None
        } else {
            Some(Hotkey::new((v >> 16) as u8 & (WIN | CTRL | ALT | SHIFT), vk))
        }
    }

    /// Exactly these modifiers: Win+Shift+S must not fire on Win+Ctrl+Shift+S.
    pub fn matches(self, vk: u32, mods: u8) -> bool {
        self.vk as u32 == vk && self.mods == mods
    }

    /// Why this can't be a hotkey, if it can't.
    pub fn check(self) -> Result<(), &'static str> {
        const TAB: u16 = 0x09;
        const ENTER: u16 = 0x0D;
        const ESC: u16 = 0x1B;
        const BACKSPACE: u16 = 0x08;
        if is_modifier(self.vk) || matches!(self.vk, TAB | ENTER | ESC | BACKSPACE) {
            return Err("That key can't be a hotkey.");
        }
        if self.mods & (WIN | CTRL | ALT) == 0 && !works_alone(self.vk) {
            return Err(
                "Add Win, Ctrl or Alt, or it would fire while you type. \
                 Print Screen, Pause, Scroll Lock and F13-F24 also work on their own.",
            );
        }
        Ok(())
    }

    pub fn label(self) -> String {
        let mut s = String::new();
        for (bit, name) in [(WIN, "Win"), (CTRL, "Ctrl"), (ALT, "Alt"), (SHIFT, "Shift")] {
            if self.mods & bit != 0 {
                s.push_str(name);
                s.push('+');
            }
        }
        s.push_str(&key_name(self.vk));
        s
    }
}

/// The modifiers held right now, as the hook sees them.
pub fn held() -> u8 {
    let down = |vk: VIRTUAL_KEY| unsafe { GetAsyncKeyState(vk as i32) } < 0;
    let mut mods = 0;
    if down(VK_LWIN) || down(VK_RWIN) {
        mods |= WIN;
    }
    if down(VK_CONTROL) {
        mods |= CTRL;
    }
    if down(VK_MENU) {
        mods |= ALT;
    }
    if down(VK_SHIFT) {
        mods |= SHIFT;
    }
    mods
}

/// Shift, Ctrl, Alt and Win in all their left/right/generic forms.
pub fn is_modifier(vk: u16) -> bool {
    matches!(vk, 0x10..=0x12 | 0x5B | 0x5C | 0xA0..=0xA5)
}

/// Print Screen, Pause, Scroll Lock and F13-F24: nobody types with them.
fn works_alone(vk: u16) -> bool {
    matches!(vk, 0x2C | 0x13 | 0x91 | 0x7C..=0x87)
}

fn key_name(vk: u16) -> String {
    match vk {
        0x30..=0x39 | 0x41..=0x5A => (vk as u8 as char).to_string(),
        0x70..=0x87 => format!("F{}", vk - 0x6F),
        0x60..=0x69 => format!("Num {}", vk - 0x60),
        0x2C => "Print Screen".into(),
        0x13 => "Pause".into(),
        0x91 => "Scroll Lock".into(),
        0x20 => "Space".into(),
        0x2D => "Insert".into(),
        0x2E => "Delete".into(),
        0x24 => "Home".into(),
        0x23 => "End".into(),
        0x21 => "Page Up".into(),
        0x22 => "Page Down".into(),
        0x25 => "Left".into(),
        0x26 => "Up".into(),
        0x27 => "Right".into(),
        0x28 => "Down".into(),
        // Punctuation differs per keyboard layout, so ask Windows.
        _ => layout_name(vk).unwrap_or_else(|| format!("Key {vk:#04X}")),
    }
}

fn layout_name(vk: u16) -> Option<String> {
    let mut buf = [0u16; 32];
    let n = unsafe {
        let scan = MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC);
        GetKeyNameTextW((scan << 16) as i32, buf.as_mut_ptr(), buf.len() as i32)
    };
    (n > 0).then(|| String::from_utf16_lossy(&buf[..n as usize]))
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: u16 = b'S' as u16;

    #[test]
    fn labels_read_like_windows_shortcuts() {
        assert_eq!(Hotkey::new(WIN | SHIFT, S).label(), "Win+Shift+S");
        assert_eq!(Hotkey::new(CTRL | ALT | SHIFT | WIN, S).label(), "Win+Ctrl+Alt+Shift+S");
        assert_eq!(Hotkey::new(0, 0x2C).label(), "Print Screen");
        assert_eq!(Hotkey::new(CTRL, 0x7B).label(), "Ctrl+F12");
        assert_eq!(Hotkey::new(ALT, b'7' as u16).label(), "Alt+7");
        assert_eq!(Hotkey::new(CTRL, 0x22).label(), "Ctrl+Page Down");
    }

    #[test]
    fn packs_into_one_number_and_back() {
        let k = Hotkey::new(WIN | SHIFT, S);
        assert_eq!(Hotkey::unpack(k.pack()), Some(k));
        // 0 means "no hotkey".
        assert_eq!(Hotkey::unpack(0), None);
    }

    #[test]
    fn typing_keys_need_win_ctrl_or_alt() {
        assert!(Hotkey::new(WIN | SHIFT, S).check().is_ok());
        assert!(Hotkey::new(CTRL, S).check().is_ok());
        // Plain S or Shift+S would fire while typing.
        assert!(Hotkey::new(0, S).check().is_err());
        assert!(Hotkey::new(SHIFT, S).check().is_err());
        assert!(Hotkey::new(0, 0x74).check().is_err()); // F5 alone
    }

    #[test]
    fn keys_nothing_types_with_work_alone() {
        assert!(Hotkey::new(0, 0x2C).check().is_ok()); // Print Screen
        assert!(Hotkey::new(SHIFT, 0x2C).check().is_ok());
        assert!(Hotkey::new(0, 0x13).check().is_ok()); // Pause
        assert!(Hotkey::new(0, 0x7C).check().is_ok()); // F13
    }

    #[test]
    fn modifiers_and_dialog_keys_are_not_hotkeys() {
        assert!(Hotkey::new(CTRL, 0x10).check().is_err()); // Shift as the key
        assert!(Hotkey::new(WIN, 0x5B).check().is_err()); // Left Win as the key
        assert!(Hotkey::new(CTRL, 0x1B).check().is_err()); // Esc
        assert!(Hotkey::new(CTRL, 0x09).check().is_err()); // Tab
        assert!(Hotkey::new(CTRL, 0x0D).check().is_err()); // Enter
    }

    #[test]
    fn matches_only_the_exact_modifiers() {
        let k = Hotkey::new(WIN | SHIFT, S);
        assert!(k.matches(S as u32, WIN | SHIFT));
        assert!(!k.matches(S as u32, WIN | SHIFT | CTRL));
        assert!(!k.matches(S as u32, WIN));
        assert!(!k.matches(b'T' as u32, WIN | SHIFT));
    }
}
