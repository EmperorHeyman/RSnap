use std::ffi::OsString;
use std::hash::{BuildHasher, Hash, Hasher, RandomState};
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::ptr::null_mut;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::{Duration, Instant, SystemTime};

use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::UI::Shell::{FOLDERID_Pictures, SHGetKnownFolderPath};

use crate::config::{DIR_NAME, FILE_PREFIX, TEMP_RETENTION_SECS};

/// `RandomState` is seeded from the OS RNG.
pub fn random_name() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut hasher = RandomState::new().build_hasher();
    COUNTER.fetch_add(1, Relaxed).hash(&mut hasher);
    Instant::now().hash(&mut hasher);
    let mut n = hasher.finish();
    let mut name = String::from(FILE_PREFIX);
    for _ in 0..8 {
        name.push(b"0123456789abcdefghijklmnopqrstuvwxyz"[(n % 36) as usize] as char);
        n /= 36;
    }
    name.push_str(".png");
    name
}

pub fn temp_dir() -> PathBuf {
    std::env::temp_dir().join(DIR_NAME)
}

/// Follows the Pictures folder if it was moved (e.g. to OneDrive).
pub fn save_dir() -> Option<PathBuf> {
    unsafe {
        let mut p = null_mut();
        let hr = SHGetKnownFolderPath(&FOLDERID_Pictures, 0, null_mut(), &mut p);
        let dir = (hr >= 0 && !p.is_null()).then(|| {
            let len = (0..).take_while(|&i| *p.add(i) != 0).count();
            PathBuf::from(OsString::from_wide(std::slice::from_raw_parts(p, len))).join(DIR_NAME)
        });
        CoTaskMemFree(p as *const _);
        dir
    }
}

pub fn write(dir: &Path, name: &str, data: &[u8]) -> Option<PathBuf> {
    std::fs::create_dir_all(dir).ok()?;
    let path = dir.join(name);
    std::fs::write(&path, data).ok()?;
    Some(path)
}

/// Kept for a while because some apps read a pasted file late.
pub fn sweep_temp() {
    let Ok(entries) = std::fs::read_dir(temp_dir()) else {
        return;
    };
    let max_age = Duration::from_secs(TEMP_RETENTION_SECS);
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with(FILE_PREFIX) || !name.ends_with(".png") {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_some_and(|age| age > max_age);
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}
