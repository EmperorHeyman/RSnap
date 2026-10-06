//! The settings window: a dialog from the exe's resources, opened from the tray menu. It exists only
//! while open, and loads the modern controls, the colour picker and the folder picker only then.

use std::cell::{Cell, RefCell};
use std::mem::{size_of, transmute, zeroed};
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{
    FreeLibrary, HANDLE, HWND, INVALID_HANDLE_VALUE, LPARAM, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{CreateSolidBrush, DeleteObject, HBRUSH, InvalidateRect};
use windows_sys::Win32::System::ApplicationInstallationAndServicing::{
    ACTCTXW, ActivateActCtx, CreateActCtxW, DeactivateActCtx,
};
use windows_sys::Win32::System::Diagnostics::Debug::MessageBeep;
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows_sys::Win32::UI::Controls::Dialogs::{CC_FULLOPEN, CC_RGBINIT, CHOOSECOLORW};
use windows_sys::Win32::UI::Controls::{
    BST_CHECKED, BST_UNCHECKED, CheckDlgButton, CheckRadioButton, IsDlgButtonChecked,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};
use windows_sys::Win32::UI::WindowsAndMessaging::*;
use windows_sys::core::{BOOL, PCWSTR};
use windows_sys::{s, w};

use crate::config::ClipboardMode;
use crate::hotkey::Hotkey;
use crate::ids::*;
use crate::settings::{self, Settings, Thickness, wide};
use crate::{WM_APP_KEYREC, files, glow, hook, ocr, tray};

const HINT: &str = "Click a box and press the keys. Backspace turns the text hotkey off.";
const THICKNESS: [&str; 3] = ["Thin", "Normal", "Thick"];
const CLIPBOARD: [(ClipboardMode, &str); 3] = [
    (ClipboardMode::FileFirst, "File, then image (chat and web apps)"),
    (ClipboardMode::ImageFirst, "Image, then file (Word, Outlook)"),
    (ClipboardMode::FileOnly, "File only"),
];
// ACTCTX_FLAG_RESOURCE_NAME_VALID | ACTCTX_FLAG_HMODULE_VALID
const ACTCTX_FROM_MODULE_RESOURCE: u32 = 0x08 | 0x80;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Snip,
    Text,
}

/// The settings being edited. Nothing applies until OK.
struct Edit {
    s: Settings,
    autostart: bool,
    /// Language tags in list order, after "Windows language".
    langs: Vec<String>,
    /// The hotkey box that has focus, if any: the hook records into it.
    recording: Option<Field>,
    /// The colour to go back to when "Windows accent colour" is unticked.
    color: u32,
}

thread_local! {
    static DIALOG: Cell<HWND> = const { Cell::new(null_mut()) };
    static EDIT: RefCell<Option<Edit>> = const { RefCell::new(None) };
    static SWATCH: Cell<HBRUSH> = const { Cell::new(null_mut()) };
    /// Activation context for the modern controls. Made on first open, kept after.
    static CONTROLS: Cell<HANDLE> = const { Cell::new(null_mut()) };
    static CUSTOM_COLORS: Cell<[u32; 16]> = const { Cell::new([0x00FF_FFFF; 16]) };
}

/// From the tray menu. Brings an open window to the front instead of opening a second one.
pub fn open() {
    let open = DIALOG.get();
    unsafe {
        if !open.is_null() {
            ShowWindow(open, SW_RESTORE);
            SetForegroundWindow(open);
            return;
        }
        let dlg = create();
        if !dlg.is_null() {
            ShowWindow(dlg, SW_SHOW);
            SetForegroundWindow(dlg);
        }
    }
}

/// Tab, Enter and Esc for the window; checked in the main loop before TranslateMessage.
pub fn pre_translate(msg: &MSG) -> bool {
    let dlg = DIALOG.get();
    !dlg.is_null() && unsafe { IsDialogMessageW(dlg, msg) } != 0
}

fn create() -> HWND {
    let dlg = with_modern_controls(|| unsafe {
        CreateDialogParamW(
            GetModuleHandleW(null()),
            IDD_SETTINGS as usize as PCWSTR,
            null_mut(),
            Some(proc),
            0,
        )
    });
    DIALOG.set(dlg);
    dlg
}

/// Controls (and the colour and folder pickers) made inside this get the modern look. The rest of
/// RSnap never activates it, so comctl32 is only loaded once the window has been opened.
fn with_modern_controls<T>(f: impl FnOnce() -> T) -> T {
    unsafe {
        let mut ctx = CONTROLS.get();
        if ctx.is_null() {
            let mut a: ACTCTXW = zeroed();
            a.cbSize = size_of::<ACTCTXW>() as u32;
            a.dwFlags = ACTCTX_FROM_MODULE_RESOURCE;
            a.hModule = GetModuleHandleW(null());
            a.lpResourceName = MANIFEST_MODERN_CONTROLS as usize as PCWSTR;
            ctx = CreateActCtxW(&a);
            CONTROLS.set(ctx);
        }
        let mut cookie = 0;
        let on = ctx != INVALID_HANDLE_VALUE && ActivateActCtx(ctx, &mut cookie) != 0;
        let out = f();
        if on {
            DeactivateActCtx(0, cookie);
        }
        out
    }
}

unsafe extern "system" fn proc(dlg: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> isize {
    unsafe {
        match msg {
            WM_INITDIALOG => {
                init(dlg);
                // Focus starts on OK, not on a hotkey box that would start recording.
                return 0;
            }
            WM_COMMAND => command(dlg, (wp & 0xFFFF) as u16, ((wp >> 16) & 0xFFFF) as u32),
            WM_APP_KEYREC => recorded(dlg, wp as u32),
            WM_CTLCOLORSTATIC if lp as HWND == GetDlgItem(dlg, IDC_SWATCH as i32) => {
                return SWATCH.get() as isize;
            }
            WM_CLOSE => close(),
            WM_DESTROY => destroyed(),
            _ => return 0,
        }
        1
    }
}

fn init(dlg: HWND) {
    let s = settings::current();
    // Off the UI thread: listing recognizers joins a COM apartment.
    let langs = std::thread::spawn(ocr::languages).join().unwrap_or_default();
    let color = s.glow_color.unwrap_or_else(glow::accent_color);
    unsafe {
        let icon = LoadImageW(GetModuleHandleW(null()), 1 as PCWSTR, IMAGE_ICON, 0, 0, LR_DEFAULTSIZE | LR_SHARED);
        SendMessageW(dlg, WM_SETICON, ICON_BIG as WPARAM, icon as LPARAM);
        SendMessageW(dlg, WM_SETICON, ICON_SMALL as WPARAM, icon as LPARAM);
        set_text(dlg, IDC_HINT, HINT);
        set_text(dlg, IDC_SNIP, &s.snip_key.label());
        set_text(dlg, IDC_TEXT, &text_label(s.text_key));
        let release = if s.shift_reads_text { IDC_SHIFT_TEXT } else { IDC_CTRL_TEXT };
        CheckRadioButton(dlg, IDC_SHIFT_TEXT as i32, IDC_CTRL_TEXT as i32, release as i32);
        let accent = if s.glow_color.is_none() { BST_CHECKED } else { BST_UNCHECKED };
        CheckDlgButton(dlg, IDC_ACCENT as i32, accent);
        EnableWindow(GetDlgItem(dlg, IDC_PICK as i32), s.glow_color.is_some() as BOOL);
        let thickness = Thickness::ALL.iter().position(|t| *t == s.thickness).unwrap_or(1);
        fill(dlg, IDC_THICKNESS, THICKNESS.iter().map(|t| t.to_string()), thickness);
        let clipboard = CLIPBOARD.iter().position(|(m, _)| *m == s.clipboard).unwrap_or(0);
        fill(dlg, IDC_CLIPBOARD, CLIPBOARD.iter().map(|(_, t)| t.to_string()), clipboard);
        set_text(dlg, IDC_FOLDER, &folder_text(s.save_dir.as_deref()));
        let tags: Vec<String> = langs.iter().map(|(tag, _)| tag.clone()).collect();
        let names = std::iter::once("Windows language".to_string()).chain(langs.into_iter().map(|(_, n)| n));
        fill(dlg, IDC_LANGUAGE, names, language_index(&tags, s.ocr_language.as_deref()));
        let autostart = tray::autostart_enabled();
        CheckDlgButton(dlg, IDC_AUTOSTART as i32, if autostart { BST_CHECKED } else { BST_UNCHECKED });
        SetFocus(GetDlgItem(dlg, IDOK));
        set_swatch(dlg, color);
        EDIT.set(Some(Edit {
            s,
            autostart,
            langs: tags,
            recording: None,
            color,
        }));
    }
}

/// Runs `f` on the edit state. Never hold it across a modal dialog: its message loop calls back in.
fn with_edit<T>(f: impl FnOnce(&mut Edit) -> T) -> Option<T> {
    EDIT.with(|e| e.try_borrow_mut().ok().and_then(|mut e| e.as_mut().map(f)))
}

fn command(dlg: HWND, id: u16, code: u32) {
    let checked = |id: u16| unsafe { IsDlgButtonChecked(dlg, id as i32) } == BST_CHECKED;
    let selected = |id: u16| unsafe { SendDlgItemMessageW(dlg, id as i32, CB_GETCURSEL, 0, 0) };
    match (id, code) {
        (1, _) => ok(),     // IDOK
        (2, _) => close(),  // IDCANCEL
        (IDC_SNIP | IDC_TEXT, EN_SETFOCUS) => {
            let field = if id == IDC_SNIP { Field::Snip } else { Field::Text };
            with_edit(|e| e.recording = Some(field));
            hook::record(dlg);
        }
        (IDC_SNIP | IDC_TEXT, EN_KILLFOCUS) => {
            with_edit(|e| e.recording = None);
            hook::record(null_mut());
        }
        (IDC_SHIFT_TEXT | IDC_CTRL_TEXT, BN_CLICKED) => {
            with_edit(|e| e.s.shift_reads_text = checked(IDC_SHIFT_TEXT));
        }
        (IDC_ACCENT, BN_CLICKED) => {
            let accent = checked(IDC_ACCENT);
            let color = with_edit(|e| {
                e.s.glow_color = (!accent).then_some(e.color);
                e.color
            });
            unsafe { EnableWindow(GetDlgItem(dlg, IDC_PICK as i32), (!accent) as BOOL) };
            set_swatch(dlg, if accent { glow::accent_color() } else { color.unwrap_or(0) });
        }
        (IDC_PICK, BN_CLICKED) => {
            let current = with_edit(|e| e.color).unwrap_or(0);
            if let Some(c) = pick_color(dlg, current) {
                with_edit(|e| {
                    e.color = c;
                    e.s.glow_color = Some(c);
                });
                set_swatch(dlg, c);
            }
        }
        (IDC_THICKNESS, CBN_SELCHANGE) => {
            if let Some(&t) = Thickness::ALL.get(selected(IDC_THICKNESS) as usize) {
                with_edit(|e| e.s.thickness = t);
            }
        }
        (IDC_CLIPBOARD, CBN_SELCHANGE) => {
            if let Some(&(m, _)) = CLIPBOARD.get(selected(IDC_CLIPBOARD) as usize) {
                with_edit(|e| e.s.clipboard = m);
            }
        }
        (IDC_LANGUAGE, CBN_SELCHANGE) => {
            let i = selected(IDC_LANGUAGE);
            with_edit(|e| e.s.ocr_language = (i > 0).then(|| e.langs.get(i as usize - 1).cloned()).flatten());
        }
        (IDC_BROWSE, BN_CLICKED) => {
            let start = with_edit(|e| e.s.save_dir.clone()).flatten().or_else(files::default_save_dir);
            if let Some(dir) = pick_folder(dlg, start.as_deref()) {
                set_text(dlg, IDC_FOLDER, &folder_text(Some(&dir)));
                with_edit(|e| e.s.save_dir = Some(dir));
            }
        }
        (IDC_RESET, BN_CLICKED) => {
            with_edit(|e| e.s.save_dir = None);
            set_text(dlg, IDC_FOLDER, &folder_text(None));
        }
        (IDC_AUTOSTART, BN_CLICKED) => {
            with_edit(|e| e.autostart = checked(IDC_AUTOSTART));
        }
        _ => {}
    }
}

/// A combination from the hook, for the hotkey box that has focus. 0 means Backspace or Delete.
fn recorded(dlg: HWND, packed: u32) {
    let Some((Some(field), other)) = with_edit(|e| {
        let other = match e.recording {
            Some(Field::Snip) => e.s.text_key,
            _ => Some(e.s.snip_key),
        };
        (e.recording, other)
    }) else {
        return;
    };
    let key = Hotkey::unpack(packed);
    if let Some(why) = problem(field, key, other) {
        set_text(dlg, IDC_HINT, why);
        unsafe { MessageBeep(MB_ICONWARNING) };
        return;
    }
    set_text(dlg, IDC_HINT, HINT);
    match field {
        Field::Snip => {
            if let Some(k) = key {
                with_edit(|e| e.s.snip_key = k);
                set_text(dlg, IDC_SNIP, &k.label());
            }
        }
        Field::Text => {
            with_edit(|e| e.s.text_key = key);
            set_text(dlg, IDC_TEXT, &text_label(key));
        }
    }
}

/// Why this combination can't go in this box, if it can't.
fn problem(field: Field, key: Option<Hotkey>, other: Option<Hotkey>) -> Option<&'static str> {
    match key {
        None if field == Field::Snip => Some("The snip hotkey can't be turned off."),
        None => None,
        Some(k) => match k.check() {
            Err(why) => Some(why),
            Ok(()) if Some(k) == other => Some("That's already the other hotkey."),
            Ok(()) => None,
        },
    }
}

/// 0 is "Windows language"; a tag that's no longer installed shows as that too.
fn language_index(tags: &[String], chosen: Option<&str>) -> usize {
    chosen
        .and_then(|c| tags.iter().position(|t| t == c))
        .map_or(0, |i| i + 1)
}

fn ok() {
    if let Some(Some(edit)) = EDIT.with(|e| e.try_borrow_mut().ok().map(|mut e| e.take())) {
        settings::save(edit.s);
        if edit.autostart != tray::autostart_enabled() {
            tray::set_autostart(edit.autostart);
        }
    }
    close();
}

fn close() {
    let dlg = DIALOG.get();
    if !dlg.is_null() {
        unsafe { DestroyWindow(dlg) };
    }
}

fn destroyed() {
    hook::record(null_mut());
    EDIT.with(|e| {
        if let Ok(mut e) = e.try_borrow_mut() {
            *e = None;
        }
    });
    let brush = SWATCH.replace(null_mut());
    if !brush.is_null() {
        unsafe { DeleteObject(brush) };
    }
    DIALOG.set(null_mut());
    crate::trim();
}

fn text_label(key: Option<Hotkey>) -> String {
    key.map_or_else(|| "Off".to_string(), Hotkey::label)
}

fn folder_text(dir: Option<&Path>) -> String {
    dir.map(Path::to_path_buf)
        .or_else(files::default_save_dir)
        .map(|d| d.display().to_string())
        .unwrap_or_default()
}

fn set_text(dlg: HWND, id: u16, text: &str) {
    let t = wide(text);
    unsafe { SetDlgItemTextW(dlg, id as i32, t.as_ptr()) };
}

fn fill(dlg: HWND, id: u16, items: impl Iterator<Item = String>, selected: usize) {
    unsafe {
        SendDlgItemMessageW(dlg, id as i32, CB_RESETCONTENT, 0, 0);
        for item in items {
            let t = wide(&item);
            SendDlgItemMessageW(dlg, id as i32, CB_ADDSTRING, 0, t.as_ptr() as LPARAM);
        }
        SendDlgItemMessageW(dlg, id as i32, CB_SETCURSEL, selected, 0);
    }
}

fn set_swatch(dlg: HWND, color: u32) {
    unsafe {
        let old = SWATCH.replace(CreateSolidBrush(color));
        if !old.is_null() {
            DeleteObject(old);
        }
        InvalidateRect(GetDlgItem(dlg, IDC_SWATCH as i32), null(), 1);
    }
}

/// The standard colour dialog. comdlg32 is loaded for it and let go after, so RSnap never imports it.
fn pick_color(owner: HWND, initial: u32) -> Option<u32> {
    type ChooseColor = unsafe extern "system" fn(*mut CHOOSECOLORW) -> BOOL;
    unsafe {
        let lib = LoadLibraryW(w!("comdlg32.dll"));
        if lib.is_null() {
            return None;
        }
        let picked = GetProcAddress(lib, s!("ChooseColorW")).and_then(|f| {
            let choose: ChooseColor = transmute(f);
            let mut custom = CUSTOM_COLORS.get();
            let mut cc: CHOOSECOLORW = zeroed();
            cc.lStructSize = size_of::<CHOOSECOLORW>() as u32;
            cc.hwndOwner = owner;
            cc.rgbResult = initial;
            cc.lpCustColors = custom.as_mut_ptr();
            cc.Flags = CC_RGBINIT | CC_FULLOPEN;
            let ok = with_modern_controls(|| choose(&mut cc)) != 0;
            CUSTOM_COLORS.set(custom);
            ok.then_some(cc.rgbResult & 0x00FF_FFFF)
        });
        FreeLibrary(lib);
        picked
    }
}

/// The Explorer folder picker.
fn pick_folder(owner: HWND, start: Option<&Path>) -> Option<PathBuf> {
    use windows::Win32::Foundation::HWND as WinHwnd;
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoCreateInstance,
        CoInitializeEx, CoTaskMemFree, CoUninitialize,
    };
    use windows::Win32::UI::Shell::{
        FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog, IShellItem,
        SHCreateItemFromParsingName, SIGDN_FILESYSPATH,
    };
    use windows::core::HSTRING;

    let pick = || -> windows::core::Result<PathBuf> {
        unsafe {
            let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
            dialog.SetOptions(dialog.GetOptions()? | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM)?;
            if let Some(start) = start {
                let start = HSTRING::from(start.to_string_lossy().as_ref());
                if let Ok(folder) = SHCreateItemFromParsingName::<_, _, IShellItem>(&start, None) {
                    let _ = dialog.SetFolder(&folder);
                }
            }
            dialog.Show(Some(WinHwnd(owner)))?;
            let name = dialog.GetResult()?.GetDisplayName(SIGDN_FILESYSPATH)?;
            let path = name.to_string().map(PathBuf::from);
            CoTaskMemFree(Some(name.0 as *const _));
            path.map_err(|_| windows::core::Error::empty())
        }
    };
    unsafe {
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        let picked = with_modern_controls(pick).ok();
        if com.is_ok() {
            CoUninitialize();
        }
        picked
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkey::{CTRL, SHIFT, WIN};

    #[test]
    fn language_list_starts_with_the_windows_language() {
        let tags = vec!["cs".to_string(), "en-US".to_string()];
        assert_eq!(language_index(&tags, None), 0);
        assert_eq!(language_index(&tags, Some("en-US")), 2);
        // Uninstalled since it was chosen: show the default rather than a wrong one.
        assert_eq!(language_index(&tags, Some("de-DE")), 0);
    }

    /// Opens the real window off-screen and saves a picture of it to target\settings-window.png.
    /// Run on request: `cargo test renders -- --ignored`.
    #[test]
    #[ignore]
    fn renders() {
        use windows_sys::Win32::Foundation::RECT;
        use windows_sys::Win32::Graphics::Gdi::*;
        use windows_sys::Win32::Storage::Xps::PrintWindow;
        unsafe {
            let dlg = create();
            assert!(!dlg.is_null(), "dialog template missing from the test exe");
            SetWindowPos(dlg, null_mut(), -20000, -20000, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
            ShowWindow(dlg, SW_SHOWNOACTIVATE);
            let until = std::time::Instant::now() + std::time::Duration::from_millis(400);
            let mut msg: MSG = zeroed();
            while std::time::Instant::now() < until {
                while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
                    DispatchMessageW(&msg);
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            let mut r: RECT = zeroed();
            GetWindowRect(dlg, &mut r);
            let (w, h) = (r.right - r.left, r.bottom - r.top);
            let mem = CreateCompatibleDC(null_mut());
            let mut bmi: BITMAPINFO = zeroed();
            bmi.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
            bmi.bmiHeader.biWidth = w;
            bmi.bmiHeader.biHeight = -h;
            bmi.bmiHeader.biPlanes = 1;
            bmi.bmiHeader.biBitCount = 32;
            let mut bits = null_mut();
            let bmp = CreateDIBSection(mem, &bmi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            let old = SelectObject(mem, bmp);
            assert!(PrintWindow(dlg, mem, PW_RENDERFULLCONTENT) != 0);
            GdiFlush();
            let px = std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize);
            let png = crate::encode::png(px, w as u32, h as u32).unwrap();
            std::fs::write(concat!(env!("CARGO_MANIFEST_DIR"), "/target/settings-window.png"), png).unwrap();
            SelectObject(mem, old);
            DeleteObject(bmp);
            DeleteDC(mem);
            DestroyWindow(dlg);
        }
    }

    #[test]
    fn recorded_hotkeys_are_checked() {
        let s = Some(Hotkey::new(WIN | SHIFT, b'S' as u16));
        let t = Some(Hotkey::new(WIN | SHIFT, b'T' as u16));
        assert_eq!(problem(Field::Snip, s, t), None);
        assert_eq!(problem(Field::Text, None, s), None); // text hotkey off is fine
        assert!(problem(Field::Snip, None, t).is_some()); // the snip hotkey can't be off
        assert!(problem(Field::Text, s, s).is_some()); // same as the other one
        assert!(problem(Field::Snip, Some(Hotkey::new(0, b'S' as u16)), t).is_some()); // plain S
        assert_eq!(problem(Field::Text, Some(Hotkey::new(CTRL, b'K' as u16)), s), None);
    }
}
