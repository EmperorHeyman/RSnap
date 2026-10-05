//! All settings live here. Change a value, rebuild, done.

/// Glow colour as 0x00BBGGRR. `None` = your Windows accent colour (falls back to blue if it's too dark to see).
pub const GLOW_COLOR: Option<u32> = Some(0x00FF_A82F);
/// Total glow thickness outside the selection, in px at 100% scaling.
pub const GLOW_SIZE: f32 = 10.0;
/// Solid line hugging the selection, in px at 100% scaling.
pub const GLOW_CORE: f32 = 2.0;
/// Opacity of the solid line, and where the soft falloff starts (0-255).
pub const GLOW_CORE_ALPHA: f32 = 235.0;
pub const GLOW_FADE_ALPHA: f32 = 130.0;

/// A drag smaller than this in both directions counts as a click and cancels.
pub const MIN_DRAG: i32 = 4;

pub const FILE_PREFIX: &str = "rsnap_";
/// Folder name under Pictures (Ctrl+release saves) and under %TEMP% (clipboard files).
pub const DIR_NAME: &str = "RSnap";
/// Clipboard temp files older than this are deleted.
pub const TEMP_RETENTION_SECS: u64 = 24 * 60 * 60;

/// What goes on the clipboard, in which order. Apps usually take the first format they understand.
pub const CLIPBOARD: ClipboardMode = ClipboardMode::FileFirst;

/// Hand unused memory back to Windows after every snip.
pub const TRIM_AFTER_SNIP: bool = true;

/// Unassigned virtual key tapped so releasing Win doesn't open the Start menu.
pub const MASK_KEY: u16 = 0xE8;

#[derive(Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ClipboardMode {
    /// Named file, then image (PNG + DIB).
    FileFirst,
    /// Image first, then the named file.
    ImageFirst,
    /// Named file only; apps that only take images (Paint) won't paste.
    FileOnly,
}
