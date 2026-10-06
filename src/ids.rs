// Resource IDs for the settings dialog. Shared with build.rs (through include!), which writes the
// dialog template, so plain items only: no inner doc comments, no `use`.

pub const IDD_SETTINGS: u16 = 100;
pub const IDC_SNIP: u16 = 101;
pub const IDC_TEXT: u16 = 102;
pub const IDC_HINT: u16 = 103;
pub const IDC_SHIFT_TEXT: u16 = 104;
pub const IDC_CTRL_TEXT: u16 = 105;
pub const IDC_SWATCH: u16 = 106;
pub const IDC_PICK: u16 = 107;
pub const IDC_ACCENT: u16 = 108;
pub const IDC_THICKNESS: u16 = 109;
pub const IDC_CLIPBOARD: u16 = 110;
pub const IDC_FOLDER: u16 = 111;
pub const IDC_BROWSE: u16 = 112;
pub const IDC_RESET: u16 = 113;
pub const IDC_LANGUAGE: u16 = 114;
pub const IDC_AUTOSTART: u16 = 115;
// Manifest that switches the dialog to the modern controls, activated only while it's open.
pub const MANIFEST_MODERN_CONTROLS: u16 = 2;
