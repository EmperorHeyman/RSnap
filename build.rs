use std::env;
use std::path::PathBuf;

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
        r#"1 ICON "{icon}"
1 24 "{manifest}"

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
    );

    let rc_path = PathBuf::from(env::var("OUT_DIR").unwrap()).join("rsnap.rc");
    std::fs::write(&rc_path, rc).unwrap();
    println!("cargo:rerun-if-changed=assets");
    embed_resource::compile(&rc_path, embed_resource::NONE)
        .manifest_required()
        .unwrap();
}
