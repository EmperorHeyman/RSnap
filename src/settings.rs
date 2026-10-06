//! What the settings window changes, kept in HKCU\Software\RSnap. Read once at startup, written on OK.

use std::path::PathBuf;
use std::ptr::null_mut;
use std::sync::RwLock;

use windows_sys::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_DWORD, REG_SZ, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegDeleteKeyValueW,
    RegDeleteTreeW, RegGetValueW, RegSetKeyValueW,
};

use crate::config::{self, ClipboardMode};
use crate::hotkey::Hotkey;

pub const KEY: &str = "Software\\RSnap";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Thickness {
    Thin,
    Normal,
    Thick,
}

impl Thickness {
    pub const ALL: [Thickness; 3] = [Thickness::Thin, Thickness::Normal, Thickness::Thick];

    /// Glow size and solid core, in px at 100% scaling.
    pub fn glow(self) -> (f32, f32) {
        match self {
            Thickness::Thin => (6.0, 1.5),
            Thickness::Normal => (config::GLOW_SIZE, config::GLOW_CORE),
            Thickness::Thick => (16.0, 3.0),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub snip_key: Hotkey,
    pub text_key: Option<Hotkey>,
    /// Ctrl saves and Shift reads text; false swaps them.
    pub shift_reads_text: bool,
    /// 0x00BBGGRR, or `None` for the Windows accent colour.
    pub glow_color: Option<u32>,
    pub thickness: Thickness,
    pub clipboard: ClipboardMode,
    /// `None` is Pictures\RSnap.
    pub save_dir: Option<PathBuf>,
    /// Language tag of an installed OCR recognizer; `None` is the Windows display language.
    pub ocr_language: Option<String>,
}

impl Settings {
    pub const DEFAULT: Settings = Settings {
        snip_key: config::SNIP_HOTKEY,
        text_key: config::TEXT_HOTKEY,
        shift_reads_text: true,
        glow_color: config::GLOW_COLOR,
        thickness: Thickness::Normal,
        clipboard: config::CLIPBOARD,
        save_dir: None,
        ocr_language: None,
    };
}

impl Default for Settings {
    fn default() -> Settings {
        Settings::DEFAULT
    }
}

static CURRENT: RwLock<Settings> = RwLock::new(Settings::DEFAULT);

/// A copy of the settings in force. Snips take one when they start or finish.
pub fn current() -> Settings {
    CURRENT.read().map(|s| s.clone()).unwrap_or_default()
}

/// At startup.
pub fn load() {
    apply(load_from(KEY));
}

/// From the settings window's OK.
pub fn save(s: Settings) {
    save_to(KEY, &s);
    apply(s);
}

fn apply(s: Settings) {
    crate::hook::set_hotkeys(s.snip_key, s.text_key);
    if let Ok(mut current) = CURRENT.write() {
        *current = s;
    }
}

const CLIPBOARD_MODES: [ClipboardMode; 3] = [
    ClipboardMode::FileFirst,
    ClipboardMode::ImageFirst,
    ClipboardMode::FileOnly,
];
/// Stored instead of a colour when the glow follows the Windows accent colour.
const ACCENT: u32 = u32::MAX;

/// Anything missing or unusable falls back to the default, so a fresh install behaves as before.
pub fn load_from(key: &str) -> Settings {
    let d = Settings::DEFAULT;
    let k = wide(key);
    let dword = |name: &str| read_dword(&k, name);
    let string = |name: &str| read_string(&k, name).filter(|s| !s.is_empty());
    let snip_key = dword("SnipHotkey")
        .and_then(Hotkey::unpack)
        .filter(|h| h.check().is_ok())
        .unwrap_or(d.snip_key);
    let text_key = match dword("TextHotkey") {
        Some(0) => None,
        Some(v) => Hotkey::unpack(v)
            .filter(|h| h.check().is_ok() && *h != snip_key)
            .or(d.text_key),
        None => d.text_key,
    };
    let glow_color = match dword("GlowColor") {
        Some(ACCENT) => None,
        Some(c) => Some(c & 0x00FF_FFFF),
        None => d.glow_color,
    };
    Settings {
        snip_key,
        text_key,
        shift_reads_text: dword("ShiftReadsText").map_or(d.shift_reads_text, |v| v != 0),
        glow_color,
        thickness: dword("GlowThickness")
            .and_then(|i| Thickness::ALL.get(i as usize).copied())
            .unwrap_or(d.thickness),
        clipboard: dword("Clipboard")
            .and_then(|i| CLIPBOARD_MODES.get(i as usize).copied())
            .unwrap_or(d.clipboard),
        save_dir: string("SaveFolder").map(PathBuf::from),
        ocr_language: string("OcrLanguage"),
    }
}

pub fn save_to(key: &str, s: &Settings) {
    let k = wide(key);
    let index = |list: &[ClipboardMode]| list.iter().position(|m| *m == s.clipboard).unwrap_or(0);
    write_dword(&k, "SnipHotkey", s.snip_key.pack());
    write_dword(&k, "TextHotkey", s.text_key.map_or(0, Hotkey::pack));
    write_dword(&k, "ShiftReadsText", s.shift_reads_text as u32);
    write_dword(&k, "GlowColor", s.glow_color.unwrap_or(ACCENT));
    write_dword(&k, "GlowThickness", Thickness::ALL.iter().position(|t| *t == s.thickness).unwrap_or(1) as u32);
    write_dword(&k, "Clipboard", index(&CLIPBOARD_MODES) as u32);
    write_string(&k, "SaveFolder", s.save_dir.as_ref().map(|p| p.to_string_lossy().into_owned()));
    write_string(&k, "OcrLanguage", s.ocr_language.clone());
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

fn read_dword(key: &[u16], name: &str) -> Option<u32> {
    let name = wide(name);
    let mut v = 0u32;
    let mut len = 4u32;
    let ok = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            null_mut(),
            &mut v as *mut u32 as *mut _,
            &mut len,
        )
    } == 0;
    ok.then_some(v)
}

fn read_string(key: &[u16], name: &str) -> Option<String> {
    let name = wide(name);
    let mut buf = vec![0u16; 1024];
    let mut len = (buf.len() * 2) as u32;
    let ok = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            null_mut(),
            buf.as_mut_ptr() as *mut _,
            &mut len,
        )
    } == 0;
    ok.then(|| {
        let n = (len as usize / 2).saturating_sub(1);
        String::from_utf16_lossy(&buf[..n])
    })
}

fn write_dword(key: &[u16], name: &str, v: u32) {
    let name = wide(name);
    unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            REG_DWORD,
            &v as *const u32 as *const _,
            4,
        )
    };
}

/// `None` deletes the value, so the default applies again.
fn write_string(key: &[u16], name: &str, v: Option<String>) {
    let name = wide(name);
    unsafe {
        match v {
            Some(v) => {
                let data = wide(&v);
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    key.as_ptr(),
                    name.as_ptr(),
                    REG_SZ,
                    data.as_ptr() as *const _,
                    (data.len() * 2) as u32,
                );
            }
            None => {
                RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr());
            }
        }
    }
}

/// The uninstaller does the same; also used by tests.
#[allow(dead_code)]
pub fn delete_key(key: &str) {
    let k = wide(key);
    unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, k.as_ptr()) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkey::{ALT, CTRL, SHIFT, WIN};

    /// A throwaway key per test, deleted on drop.
    struct TestKey(String);

    impl TestKey {
        fn new(name: &str) -> TestKey {
            let key = TestKey(format!("Software\\RSnap-test-{name}-{}", std::process::id()));
            key.delete();
            key
        }

        fn delete(&self) {
            let wide = wide(&self.0);
            unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, wide.as_ptr()) };
        }
    }

    impl Drop for TestKey {
        fn drop(&mut self) {
            self.delete();
        }
    }

    #[test]
    fn defaults_are_todays_behaviour() {
        let d = Settings::default();
        assert_eq!(d.snip_key, Hotkey::new(WIN | SHIFT, b'S' as u16));
        assert_eq!(d.text_key, Some(Hotkey::new(WIN | SHIFT, b'T' as u16)));
        assert!(d.shift_reads_text);
        assert_eq!(d.glow_color, crate::config::GLOW_COLOR);
        assert_eq!(d.thickness, Thickness::Normal);
        assert_eq!(d.clipboard, crate::config::CLIPBOARD);
        assert_eq!(d.save_dir, None);
        assert_eq!(d.ocr_language, None);
    }

    #[test]
    fn missing_key_loads_defaults() {
        let key = TestKey::new("missing");
        assert_eq!(load_from(&key.0), Settings::default());
    }

    #[test]
    fn round_trips_through_the_registry() {
        let key = TestKey::new("round-trip");
        let s = Settings {
            snip_key: Hotkey::new(CTRL | ALT, b'S' as u16),
            text_key: None,
            shift_reads_text: false,
            glow_color: None,
            thickness: Thickness::Thick,
            clipboard: ClipboardMode::ImageFirst,
            save_dir: Some(PathBuf::from(r"D:\Snips\Batérie")),
            ocr_language: Some("en-US".into()),
        };
        save_to(&key.0, &s);
        assert_eq!(load_from(&key.0), s);

        // Back to defaults clears the optional values instead of leaving stale ones.
        save_to(&key.0, &Settings::default());
        assert_eq!(load_from(&key.0), Settings::default());
    }

    #[test]
    fn bad_values_fall_back_to_defaults() {
        let key = TestKey::new("bad");
        let k = wide(&key.0);
        let put = |name: &str, v: u32| unsafe {
            let n = wide(name);
            RegSetKeyValueW(HKEY_CURRENT_USER, k.as_ptr(), n.as_ptr(), REG_DWORD, &v as *const u32 as *const _, 4);
        };
        put("SnipHotkey", Hotkey::new(0, b'S' as u16).pack()); // fires while typing
        put("Clipboard", 7);
        put("GlowThickness", 9);
        let s = load_from(&key.0);
        assert_eq!(s.snip_key, Settings::default().snip_key);
        assert_eq!(s.clipboard, Settings::default().clipboard);
        assert_eq!(s.thickness, Thickness::Normal);
    }

    #[test]
    fn thickness_presets_scale_the_glow() {
        assert_eq!(Thickness::Normal.glow(), (crate::config::GLOW_SIZE, crate::config::GLOW_CORE));
        assert!(Thickness::Thin.glow().0 < Thickness::Normal.glow().0);
        assert!(Thickness::Thick.glow().0 > Thickness::Normal.glow().0);
    }
}
