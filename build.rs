use std::env;
use std::path::PathBuf;

#[allow(dead_code)]
mod ids {
    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/ids.rs"));
}
use ids::*;

const COMPANY: &str = "RAPL Group, s.r.o.";

fn main() {
    let version = env::var("CARGO_PKG_VERSION").unwrap();
    let nums: Vec<u16> = version.split('.').map(|p| p.parse().unwrap_or(0)).collect();
    let (major, minor, patch) = (nums[0], nums[1], nums[2]);

    let assets = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("assets");
    let asset = |name: &str| {
        assets
            .join(name)
            .display()
            .to_string()
            .replace('\\', "\\\\")
    };

    let rc = format!(
        r#"#include <windows.h>

1 ICON "{icon}"
1 24 "{manifest}"
{MANIFEST_MODERN_CONTROLS} 24 "{controls}"

{IDD_SETTINGS} DIALOGEX 0, 0, 300, 276
STYLE DS_SETFONT | DS_MODALFRAME | DS_CENTER | WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX
EXSTYLE WS_EX_APPWINDOW
CAPTION "RSnap settings"
FONT 9, "Segoe UI", 400, 0, 1
BEGIN
  GROUPBOX "Hotkeys", -1, 7, 5, 286, 92
  LTEXT "Snip", -1, 16, 20, 58, 8
  EDITTEXT {IDC_SNIP}, 78, 18, 206, 13, ES_READONLY | ES_AUTOHSCROLL
  LTEXT "Text", -1, 16, 37, 58, 8
  EDITTEXT {IDC_TEXT}, 78, 35, 206, 13, ES_READONLY | ES_AUTOHSCROLL
  LTEXT "", {IDC_HINT}, 78, 51, 206, 16
  LTEXT "On release", -1, 16, 70, 58, 8
  AUTORADIOBUTTON "Ctrl saves, Shift reads text", {IDC_SHIFT_TEXT}, 78, 69, 206, 10, WS_GROUP | WS_TABSTOP
  AUTORADIOBUTTON "Shift saves, Ctrl reads text", {IDC_CTRL_TEXT}, 78, 81, 206, 10
  GROUPBOX "Look", -1, 7, 101, 286, 48
  LTEXT "Glow colour", -1, 16, 117, 58, 8
  LTEXT "", {IDC_SWATCH}, 78, 114, 22, 14, WS_BORDER
  PUSHBUTTON "Pick...", {IDC_PICK}, 104, 114, 48, 14, WS_GROUP
  AUTOCHECKBOX "Windows accent colour", {IDC_ACCENT}, 160, 116, 124, 10
  LTEXT "Thickness", -1, 16, 134, 58, 8
  COMBOBOX {IDC_THICKNESS}, 78, 132, 90, 60, CBS_DROPDOWNLIST | WS_VSCROLL | WS_TABSTOP
  GROUPBOX "Output", -1, 7, 153, 286, 84
  LTEXT "Clipboard", -1, 16, 169, 58, 8
  COMBOBOX {IDC_CLIPBOARD}, 78, 167, 206, 60, CBS_DROPDOWNLIST | WS_VSCROLL | WS_TABSTOP
  LTEXT "Save folder", -1, 16, 187, 58, 8
  EDITTEXT {IDC_FOLDER}, 78, 185, 110, 13, ES_READONLY | ES_AUTOHSCROLL
  PUSHBUTTON "Browse...", {IDC_BROWSE}, 192, 184, 46, 14
  PUSHBUTTON "Reset", {IDC_RESET}, 242, 184, 42, 14
  LTEXT "OCR language", -1, 16, 205, 58, 8
  COMBOBOX {IDC_LANGUAGE}, 78, 203, 206, 80, CBS_DROPDOWNLIST | WS_VSCROLL | WS_TABSTOP
  AUTOCHECKBOX "Serial numbers: read O as 0 and I as 1", {IDC_FIX_CODES}, 78, 221, 206, 10
  AUTOCHECKBOX "Start with Windows", {IDC_AUTOSTART}, 10, 243, 140, 10
  DEFPUSHBUTTON "OK", IDOK, 186, 256, 50, 14
  PUSHBUTTON "Cancel", IDCANCEL, 243, 256, 50, 14
END

1 VERSIONINFO
FILEVERSION {major},{minor},{patch},0
PRODUCTVERSION {major},{minor},{patch},0
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "CompanyName", "{COMPANY}"
      VALUE "FileDescription", "RSnap"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "rsnap"
      VALUE "LegalCopyright", "Copyright (c) 2026 {COMPANY}"
      VALUE "OriginalFilename", "rsnap.exe"
      VALUE "ProductName", "RSnap"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#,
        icon = asset("rsnap.ico"),
        manifest = asset("rsnap.manifest"),
        controls = asset("controls.manifest"),
    );

    let rc_path = PathBuf::from(env::var("OUT_DIR").unwrap()).join("rsnap.rc");
    std::fs::write(&rc_path, rc).unwrap();
    println!("cargo:rerun-if-changed=assets");
    println!("cargo:rerun-if-changed=src/ids.rs");
    // Tests too: they open the settings dialog from its template.
    embed_resource::compile_for_everything(&rc_path, embed_resource::NONE)
        .manifest_required()
        .unwrap();
}
