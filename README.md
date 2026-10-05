# RSnap

> Win+Shift+S without the frozen screen. A 450 KB snipping tool that sits at 0% CPU until you press the shortcut.

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-2024-orange.svg)](https://www.rust-lang.org/)
[![Platform](https://img.shields.io/badge/platform-Windows%2010%20%2F%2011-lightgrey.svg)](#setup)
[![Binary size](https://img.shields.io/badge/binary-450_KB-blue.svg)](#numbers)
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

When it isn't snipping, it isn't doing anything: no polling, no timers, one thread asleep in the
message loop. That constraint is the whole point of the project.

---

## Using it

| Do this | Get this |
|---|---|
| **Win+Shift+S** | Crosshair. The screen stays live. |
| **Drag, release** | Snip on the clipboard |
| **Ctrl + release** | Same, plus saved to `Pictures\RSnap` |
| **Esc** or **right-click** | Cancel |
| **Tray icon, left-click** | Start a snip |
| **Tray icon, right-click** | Open folder · Start with Windows · Exit |

The crosshair never takes focus, so the app you were in is still focused and ready for Ctrl+V.

### What lands on the clipboard

| Format | Who reads it |
|---|---|
| A file, `rsnap_<8 random chars>.png` | Anything that accepts pasted files: Explorer, chat apps, upload boxes. The name comes along. |
| `PNG` | Browsers, Office, most modern apps |
| `CF_DIB` | Everything else, Paint included |

The file lives in `%TEMP%\RSnap` and is deleted after 24 hours. With Ctrl+release it's written to
`Pictures\RSnap` instead and the clipboard points there.

### Clipboard modes

Apps generally take the first clipboard format they understand, so the order matters. Pick it with
`CLIPBOARD` in [`src/config.rs`](src/config.rs).

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
| Idle CPU | 0. CPU time didn't move over 45 s idle. |
| Idle RAM | ~1 MB working set, ~2.4 MB private |
| Binary | 450 KB, one exe, no runtime |
| Accuracy | Pixel-exact against a full-screen capture |

## How it works

- **Hotkey.** Explorer owns Win+Shift+S, so `RegisterHotKey` can't have it. A low-level keyboard hook
  eats the S before Windows sees the combo, then taps an unassigned key so letting go of Win doesn't
  open the Start menu (the AutoHotkey trick). Per keystroke the hook tests a few bits and returns.
- **Crosshair.** One window over every monitor, created with `WS_EX_NOREDIRECTIONBITMAP`. It has no
  surface at all, so it draws nothing and can't tint anything, but it still catches the mouse.
- **Glow.** Eight per-pixel-alpha windows, four edges and four corners. The gradients are drawn once
  per snip and dragging only moves the windows, so nothing bigger than a thin strip is ever redrawn.
- **Capture.** `BitBlt` straight from the screen. The crosshair and glow are excluded from capture
  (`WDA_EXCLUDEFROMCAPTURE`), so they never end up in a snip, or in OBS.
- **Output.** A short-lived worker thread encodes the PNG, writes the file and fills the clipboard,
  so the keyboard hook is never kept waiting. Then RSnap hands its unused memory back to Windows.
- **DPI.** Per-Monitor V2 awareness, declared in the embedded manifest. Every coordinate is a physical
  pixel, so a 150% laptop screen next to a 100% monitor captures at native resolution on both, with
  no scaling blur and no offset selection.

## Known limits

| Issue | Details |
|---|---|
| **Admin windows** | While an elevated window is focused, Windows doesn't pass its keystrokes to a non-elevated hook, so Win+Shift+S opens the stock Snipping Tool instead. Run RSnap as administrator if that bothers you. |
| **Hover menus and tooltips** | The screen is live, so anything that only exists while you hover can close before you finish dragging. |
| **Word and Outlook** | They may paste the file as an attachment rather than an inline picture. Set `CLIPBOARD` to `ImageFirst` if you mostly paste into Office. |
| **Exclusive-fullscreen games** | Nothing can draw over them. Borderless windowed is fine. |
| **Recording a demo** | The glow is hidden from screen capture by design, so OBS and ShareX won't see it. |
| **HDR** | Snips come out tone-mapped to SDR. |
| **Unsigned** | SmartScreen may warn the first time you run the installer. |

---

## Setup

Requires Windows 10 or 11.

Grab `RSnap-x.y.z-setup.exe` from the releases page and run it. It installs for your user only (no
admin prompt) into `%LOCALAPPDATA%\Programs\RSnap`, adds a Start menu shortcut and, if you leave the
box ticked, starts with Windows. Uninstall from Apps & features; snips in `Pictures\RSnap` are left
alone.

### Building from source

Needs Rust (stable, MSVC toolchain) and, for the installer, [NSIS](https://nsis.sourceforge.io/)
(`winget install NSIS.NSIS`).

```powershell
git clone https://github.com/EmperorHeyman/RSnap.git
cd RSnap
cargo build --release                 # target\release\rsnap.exe
makensis installer\rsnap.nsi          # target\RSnap-<version>-setup.exe
```

The exe on its own is fully portable: run it from anywhere and use the tray menu's
**Start with Windows**. If you move it, start it once from the new place and the autostart entry
follows.

## Configuration

There's no settings window. Settings are constants in [`src/config.rs`](src/config.rs): change one,
rebuild.

| Setting | Default | Description |
|---|---|---|
| `GLOW_COLOR` | `Some(0x00FFA82F)` | Glow colour as `0x00BBGGRR`, or `None` for your Windows accent colour |
| `GLOW_SIZE` / `GLOW_CORE` | `10.0` / `2.0` | Glow thickness and the solid inner line, in px at 100% scaling |
| `GLOW_CORE_ALPHA` / `GLOW_FADE_ALPHA` | `235` / `130` | Opacity of the line, and where the falloff starts |
| `MIN_DRAG` | `4` | Smaller drags count as a click and cancel |
| `CLIPBOARD` | `FileFirst` | See [clipboard modes](#clipboard-modes) |
| `FILE_PREFIX` | `rsnap_` | File name prefix |
| `DIR_NAME` | `RSnap` | Folder under Pictures and `%TEMP%` |
| `TEMP_RETENTION_SECS` | 24 h | How long clipboard files are kept |
| `TRIM_AFTER_SNIP` | `true` | Give memory back to Windows after each snip |
| `MASK_KEY` | `0xE8` | Unassigned key that keeps the Start menu shut |

## Project structure

```
RSnap/
├── src/
│   ├── main.rs        # Entry, message loop, the worker that delivers a snip
│   ├── hook.rs        # Low-level keyboard hook: Win+Shift+S and Esc
│   ├── overlay.rs     # Invisible full-screen window: crosshair and drag
│   ├── glow.rs        # The glow frame
│   ├── capture.rs     # BitBlt from the screen
│   ├── encode.rs      # PNG and DIB
│   ├── clipboard.rs   # File + PNG + DIB on the clipboard
│   ├── files.rs       # Random names, folders, temp cleanup
│   ├── tray.rs        # Tray icon, menu, autostart
│   └── config.rs      # Every setting
├── assets/            # Icon and manifest (per-monitor DPI)
├── installer/
│   └── rsnap.nsi      # NSIS installer
└── build.rs           # Embeds icon, manifest and version info
```

## Tech

Rust, raw Win32 through `windows-sys`, and the `png` crate. No GUI framework, no async runtime, no
tray-icon crate. The CRT is linked statically, so it's a single exe with nothing to install
alongside it.

## License

MIT

Made by RAPL Group, s.r.o.
