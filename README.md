# RSnap

> Win+Shift+S without the frozen screen. A 520 KB snipping tool that sits at 0% CPU until you press the shortcut.

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-2024-orange.svg)](https://www.rust-lang.org/)
[![Platform](https://img.shields.io/badge/platform-Windows%2010%20%2F%2011-lightgrey.svg)](#setup)
[![Binary size](https://img.shields.io/badge/binary-520_KB-blue.svg)](#numbers)
[![Idle RAM](https://img.shields.io/badge/idle_RAM-~1_MB-brightgreen.svg)](#numbers)

## The problem

The Windows Snipping Tool freezes and dims the whole screen every time I press Win+Shift+S, and it
takes a moment to even show up. ShareX is fast and reliable, but its settings page is a kilometer long
and I don't need any of it.

Then there's Gemini. Paste one screenshot, fine. Paste a second one and it refuses, because every
pasted screenshot is called `image.png` and that one is "already in chat".

## The solution

Same shortcut, nothing else.

```
Win+Shift+S ──▸ keyboard hook ──▸ crosshair over the live screen ──▸ drag ──▸ clipboard
```

The screen never freezes or dims. Video keeps playing while you drag, a glowing frame shows what you
picked, and on release the pixels under it go to the clipboard as a randomly named file **and** as an
image. Apps that take files get `rsnap_k3x9q2ma.png`; Paint gets pixels.

Hold **Ctrl** when you let go and it's saved to `Pictures\RSnap` as well.

Hold **Shift** instead and you get the text in the snip. It's read by the OCR engine built into
Windows, so nothing leaves your machine, put on the clipboard, and shown in a small box where you can
fix a misread before you paste.

When it isn't snipping, it isn't doing anything: no polling, no timers, one thread asleep in the
message loop. That constraint is the whole point of the project.

---

## Using it

| Do this | Get this |
|---|---|
| **Win+Shift+S** | Crosshair. The screen stays live. |
| **Drag, release** | Snip on the clipboard |
| **Ctrl + release** | Same, plus saved to `Pictures\RSnap` |
| **Shift + release** | The text in the snip on the clipboard, plus a popup to fix it or copy part of it |
| **Win+Shift+T** | Crosshair in text mode: release always gives text |
| **Esc** or **right-click** | Cancel |
| **Tray icon, left-click** | Start a snip |
| **Tray icon, right-click** | Settings… · Open folder · Start with Windows · Exit |

Both hotkeys, which release key saves and which reads text, and the rest are in
[Settings](#settings).

The crosshair never takes focus, so the app you were in is still focused and ready for Ctrl+V.

### What lands on the clipboard

| Format | Who reads it |
|---|---|
| A file, `rsnap_<8 random chars>.png` | Anything that accepts pasted files: Explorer, chat apps, upload boxes. The name comes along. |
| `PNG` | Browsers, Office, most modern apps |
| `CF_DIB` | Everything else, Paint included |

The file lives in `%TEMP%\RSnap` and is deleted after 24 hours. With Ctrl+release it's written to
`Pictures\RSnap` instead and the clipboard points there.

### Text snips

Shift + release reads the snip with Windows' own OCR, in your Windows display language (Czech
reads English too). Small screen text is enlarged 2× first, because Windows OCR skips it otherwise.
Only Unicode text goes on the clipboard, no image. If nothing readable is found you hear a warning
beep and the clipboard is left alone.

Windows OCR reads codes as if they were words, so in serial numbers 0 comes back as O and 1 as I.
With **Serial numbers: read O as 0 and I as 1** (on by default), tokens that look like codes get
those digits back; ordinary words and numbers are untouched. On 2160 rendered serials that took exact
reads from 62% to about 78%. Turn it off if your codes really contain the letters O or I.

The popup under the snip already has focus:

| Key | Does |
|---|---|
| **Enter** | Copies your selection, or everything if nothing is selected (with your fixes), and closes |
| **Shift+Enter** | New line |
| **Ctrl+A** / **Ctrl+C** | Select all / copy the selection, popup stays |
| **Esc**, or click anywhere else | Closes |

### Clipboard modes

Apps generally take the first clipboard format they understand, so the order matters. Pick it in
[Settings](#settings).

| Mode | Order | Pick it when |
|---|---|---|
| `FileFirst` (default) | File, PNG, DIB | You paste into chat and web apps. Anything that takes the file gets a new name every time, so a second snip never looks like a duplicate of the first. |
| `ImageFirst` | PNG, DIB, file | You mostly paste into Word, Outlook or other desktop apps that would turn a file into an attachment instead of an inline picture. |
| `FileOnly` | File | An app keeps grabbing the image (and calling it `image.png`) even though the file is there. Image-only apps like Paint can't paste this. |

---

## Numbers

Measured on my machine: Windows 11, four monitors, 8320×1440 desktop.

| What | Measured |
|---|---|
| Win+Shift+S → crosshair | ~3.3 ms |
| Release → clipboard ready (500×350 snip) | ~24 ms |
| Shift-release → text on clipboard (500×350 snip) | ~30 ms |
| Idle CPU | 0. CPU time didn't move over 45 s idle. |
| Idle RAM | ~1 MB working set, ~2.4 MB private |
| Binary | 520 KB, one exe, no runtime |
| Accuracy | Pixel-exact against a full-screen capture |

## How it works

- **Hotkey.** Explorer owns Win+Shift+S, so `RegisterHotKey` can't have it. A low-level keyboard hook
  eats the S before Windows sees the combo, then taps an unassigned key so letting go of Win doesn't
  open the Start menu (the AutoHotkey trick; it also stops Alt from opening a menu bar). Per keystroke
  the hook compares the key with the two hotkeys and returns. The same hook records new combinations
  in the settings window, which is why Win combos can be recorded at all.
- **Crosshair.** One window over every monitor, created with `WS_EX_NOREDIRECTIONBITMAP`. It has no
  surface at all, so it draws nothing and can't tint anything, but it still catches the mouse.
- **Glow.** Eight per-pixel-alpha windows, four edges and four corners. The gradients are drawn once
  per snip and dragging only moves the windows, so nothing bigger than a thin strip is ever redrawn.
- **Capture.** `BitBlt` straight from the screen. The crosshair and glow are excluded from capture
  (`WDA_EXCLUDEFROMCAPTURE`), so they never end up in a snip, or in OBS.
- **Output.** A short-lived worker thread encodes the PNG, writes the file and fills the clipboard,
  so the keyboard hook is never kept waiting. Then RSnap hands its unused memory back to Windows.
- **Text.** Shift-release sends the snip through `Windows.Media.Ocr`, the engine Windows already
  ships, so no models are bundled. The snip is converted to grayscale and enlarged 2× first, which
  takes 12 px screen text from unreadable to exact. The engine is created per snip and released;
  Windows keeps the OCR library mapped after the first text snip, and the trim after each snip pages
  it out of the working set. The popup is a plain Windows edit box.
- **Settings.** A dialog template in the exe, created when you open it and destroyed when you close
  it. Its modern controls come from an activation context used only around it, and the colour and
  folder pickers load only when clicked, so none of that is loaded until you open the window. Values
  live in `HKCU\Software\RSnap`; anything missing falls back to the defaults in `config.rs`.
- **DPI.** Per-Monitor V2 awareness, declared in the embedded manifest. Every coordinate is a physical
  pixel, so a 150% laptop screen next to a 100% monitor captures at native resolution on both, with
  no scaling blur and no offset selection.

## Known limits

| Issue | Details |
|---|---|
| **Admin windows** | While an elevated window is focused, Windows doesn't pass its keystrokes to a non-elevated hook, so Win+Shift+S opens the stock Snipping Tool instead. Run RSnap as administrator if that bothers you. |
| **Hover menus and tooltips** | The screen is live, so anything that only exists while you hover can close before you finish dragging. |
| **Word and Outlook** | They may paste the file as an attachment rather than an inline picture. Set the clipboard mode to "Image, then file" if you mostly paste into Office. |
| **Exclusive-fullscreen games** | Nothing can draw over them. Borderless windowed is fine. |
| **Recording a demo** | The glow is hidden from screen capture by design, so OBS and ShareX won't see it. |
| **HDR** | Snips come out tone-mapped to SDR. |
| **Unsigned** | SmartScreen may warn the first time you run the installer. |
| **Text: pasting straight away** | The popup has focus, so Ctrl+V right after a text snip pastes into the popup. Click where you want to paste first; that also closes it. |
| **Text: languages** | Windows only reads scripts it has an OCR pack for. Latin script works with the Czech or English pack; Chinese, Cyrillic and others need their pack installed. |
| **Text: serial numbers** | Even with the O/I fix, about 1 in 5 serials comes back with a mistake: a dropped hyphen, 5 read as S, U as 1J. Small decimal points can be dropped or, with the Czech recognizer, turned into commas (`51.2` → `512`, `14.3` → `14,3`). Check codes in the popup; fix and press Enter. |
| **Text: very wide snips** | The engine takes at most 10,000 px a side, so snips wider than 5,000 px are enlarged less than 2× and small text in them may be missed. |

---

## Setup

Requires Windows 10 or 11.

Grab `RSnap-x.y.z-setup.exe` from the releases page and run it. It installs for your user only (no
admin prompt) into `%LOCALAPPDATA%\Programs\RSnap` and adds a Start menu shortcut. The last page has
**Start with Windows** ticked; you can flip it later from the tray menu. Uninstall from Apps & features; snips in `Pictures\RSnap` are left
alone.

### Building from source

Needs Rust (stable, MSVC toolchain) and, for the installer, [NSIS](https://nsis.sourceforge.io/)
(`winget install NSIS.NSIS`). If your default Rust toolchain is GNU, run
`rustup override set stable-x86_64-pc-windows-msvc` in the repo first.

```powershell
git clone https://github.com/EmperorHeyman/RSnap.git
cd RSnap
cargo build --release                 # target\release\rsnap.exe
makensis installer\rsnap.nsi          # dist\RSnap-<version>-setup.exe
```

The exe on its own is fully portable: run it from anywhere and use the tray menu's
**Start with Windows**. If you move it, start it once from the new place and the autostart entry
follows.

## Settings

Right-click the tray icon → **Settings…**. OK applies immediately; Cancel or Esc discards.

| Setting | Default | Notes |
|---|---|---|
| Snip hotkey | Win+Shift+S | Click the box and press any combination. Needs Win, Ctrl or Alt, except Print Screen, Pause, Scroll Lock and F13–F24. Moving it off Win+Shift+S gives that back to the stock Snipping Tool. |
| Text hotkey | Win+Shift+T | Opens the crosshair in text mode. Backspace turns it off. |
| On release | Ctrl saves, Shift reads text | Or the other way round |
| Glow colour | Blue | Any colour, or your Windows accent colour |
| Thickness | Normal | Thin, Normal or Thick |
| Clipboard | File, then image | See [clipboard modes](#clipboard-modes) |
| Save folder | `Pictures\RSnap` | Where Ctrl-release saves |
| OCR language | Windows language | Any installed OCR recognizer |
| Serial numbers: read O as 0 and I as 1 | On | See [text snips](#text-snips) |
| Start with Windows | Set by the installer | Same as the tray menu item |

Settings are stored in `HKCU\Software\RSnap`; the uninstaller removes them.

### Build-time constants

The rest are constants in [`src/config.rs`](src/config.rs): change one, rebuild. The defaults for
the settings above live there too.

| Constant | Default | Description |
|---|---|---|
| `GLOW_CORE_ALPHA` / `GLOW_FADE_ALPHA` | `235` / `130` | Opacity of the line, and where the falloff starts |
| `MIN_DRAG` | `4` | Smaller drags count as a click and cancel |
| `FILE_PREFIX` | `rsnap_` | File name prefix |
| `DIR_NAME` | `RSnap` | Folder under Pictures and `%TEMP%` |
| `TEMP_RETENTION_SECS` | 24 h | How long clipboard files are kept |
| `TRIM_AFTER_SNIP` | `true` | Give memory back to Windows after each snip |
| `MASK_KEY` | `0xE8` | Unassigned key that keeps the Start menu shut |
| `OCR_SCALE` | `2.0` | How much text snips are enlarged before OCR |
| `POPUP_FONT_PT` | `10` | Popup font size, in points |
| `POPUP_MIN_W` / `POPUP_MAX_W` | `240` / `640` | Popup width limits, in px at 100% scaling |
| `POPUP_MAX_LINES` | `12` | Lines shown before the popup scrolls |

## Project structure

```
RSnap/
├── src/
│   ├── main.rs        # Entry, message loop, the worker that delivers a snip
│   ├── hook.rs        # Low-level keyboard hook: the hotkeys, Esc, recording combinations
│   ├── hotkey.rs      # Key combinations: matching, rules, labels
│   ├── overlay.rs     # Invisible full-screen window: crosshair and drag
│   ├── glow.rs        # The glow frame
│   ├── capture.rs     # BitBlt from the screen
│   ├── encode.rs      # PNG and DIB
│   ├── ocr.rs         # Text from pixels with the Windows OCR engine
│   ├── popup.rs       # The editable popup for OCR'd text
│   ├── clipboard.rs   # File + PNG + DIB, or text, on the clipboard
│   ├── files.rs       # Random names, folders, temp cleanup
│   ├── tray.rs        # Tray icon, menu, autostart
│   ├── settings.rs    # Settings in the registry
│   ├── settings_window.rs # The settings dialog
│   ├── ids.rs         # Dialog control IDs, shared with build.rs
│   └── config.rs      # Defaults and build-time constants
├── assets/            # Icon and manifests (per-monitor DPI; modern controls for the dialog)
├── installer/
│   └── rsnap.nsi      # NSIS installer
└── build.rs           # Embeds icon, manifests, settings dialog and version info
```

## Tech

Rust, raw Win32 through `windows-sys`, the `windows` crate for the WinRT OCR engine and the folder picker, and the `png`
crate. No GUI framework, no async runtime, no
tray-icon crate. The CRT is linked statically, so it's a single exe with nothing to install
alongside it.

## License

MIT

Made by RAPL Group, s.r.o.
