# OCR snips: design

Date: 2026-10-06 · Status: approved; updated with spike results

## Why

Two everyday jobs need text out of pixels, and today both go through extra tools:

- Battery serial numbers arrive as WhatsApp photos. Getting one into a form means screenshotting,
  uploading to Google Lens, and copying from there.
- A colleague translating screen text has to OCR it with the stock Snipping Tool first.

RSnap is already the snipping tool. It should hand back text as easily as it hands back pixels,
without giving up what makes it worth using: ~3 ms to the crosshair, 0% CPU and ~1 MB RAM when idle,
a single small exe.

## What the user gets

| Do this | Get this |
|---|---|
| Drag, hold **Shift**, release | Recognized text on the clipboard, plus a small editable popup with the same text |
| Shift + Ctrl, release | Same as Shift (text wins) |

The popup is optional to use: the text is already on the clipboard before it appears. It exists to
fix a misread (0/O, 1/l) or copy only part of the text.

If no text is recognized: a warning beep, no popup, clipboard untouched.

### Success criteria

- A serial number snipped from a photo pastes correctly with Shift-release, Ctrl+V.
- A Latin-script paragraph snipped from the screen pastes as text with its line breaks.
- Idle CPU stays at 0, and idle RAM after an OCR snip (popup closed) is within ~1 MB of today's figure.
- Exe size growth is measured and reported; anything above +300 KB is raised before continuing.
- Shift-release → text on clipboard is measured and reported (expected well under 200 ms on a
  typical snip).

## Out of scope

- Non-Latin scripts. The Windows profile language's recognizer (Czech here, which also reads English)
  is enough for the stated use. Other scripts need extra OCR packs and recognizer selection.
- Runtime settings (rebinding the main hotkey, a direct OCR hotkey, swapping the Ctrl/Shift roles,
  glow colour, clipboard mode). That is the next design. The only accommodation here is that the
  image/text decision travels as a value (`Output`), so a future hotkey can choose text up front.
- Translation, image + text on the clipboard at once, OCR history.

## Engine

`Windows.Media.Ocr`, the OCR engine built into Windows 10/11 (the one PowerToys Text Extractor uses).

- Ships with Windows: no model files in the exe. Language data comes from installed OCR packs; this
  machine has `cs` and `en-US`.
- Offline: snips never leave the machine.
- Loaded only while an OCR snip is being processed, on the existing worker thread. Nothing persists
  between snips.

Rejected: Tesseract (10+ MB of language data per language, slower), cloud OCR (network round trip,
sends work data out), bundled ONNX models (runtime + model size).

New dependency: the `windows` crate, for the WinRT calls only, with the narrowest feature set that
compiles. Everything else stays on `windows-sys`.

## Flow

```
WM_LBUTTONUP (overlay.rs)
  Shift held?  ── no ──▸ existing image path, unchanged
     │ yes
     ▼
capture::grab(sel)                       same BitBlt, overlay and glow still excluded
     ▼
worker thread: deliver_text(shot, sel)   main.rs
  ├─ ocr::recognize(pixels, w, h)        new src/ocr.rs
  ├─ none ─▸ MessageBeep, stop
  ├─ clipboard::set_text(&text)          CF_UNICODETEXT only
  └─ PostMessage(main, WM_APP_OCR, Box<(text, sel)>)
     ▼
main thread: popup::show(text, sel)      new src/popup.rs
```

### Trigger (overlay.rs)

On `WM_LBUTTONUP`, Shift is read the same way Ctrl is today: `MK_SHIFT` in `wParam` or
`GetAsyncKeyState(VK_SHIFT)`. `finish` takes an `Output` value instead of `save: bool`:

```rust
pub enum Output { Image { save: bool }, Text }
```

`overlay::start` also closes an open popup, so starting a new snip always dismisses the previous one.

Known risk: holding Shift from Win+Shift+S through a very quick drag gives text instead of an image.
Shipped as is; Shift would only need a fresh press if this turns out to happen in practice.

### Recognition (src/ocr.rs)

`pub fn recognize(px: &[u8], w: u32, h: u32) -> Option<String>`. Input is the top-down BGRA from
`Shot::pixels`.

1. Join the COM apartment as MTA for the duration of the call (balanced uninitialize on return).
2. Convert to grayscale and enlarge ×`OCR_SCALE` (2.0) with bilinear filtering, in Rust. If that
   would pass `OcrEngine::MaxImageDimension` (10000 px) on either side, use the largest scale that
   fits instead (an 8320 px wide desktop gets 1.2×).
3. Wrap the result in a `SoftwareBitmap` as `Gray8`. Grayscale is a quarter of the memory of BGRA,
   which matters once a big snip is doubled, and sidesteps BitBlt's zero alpha byte.
4. Engine: `OcrEngine::TryCreateFromUserProfileLanguages()`, falling back to the first entry of
   `AvailableRecognizerLanguages`. No engine → `None`.

Spike results behind steps 2–3 (Segoe UI rendered with GDI, this machine's Czech recognizer):

| Text size | ×1 | ×2 GDI halftone, BGRA | ×2 bilinear, gray | ×3 / ×4 bilinear |
|---|---|---|---|---|
| 11 px | nothing | digits dropped | perfect | perfect / perfect |
| 12–13 px | nothing | perfect | perfect | perfect |
| 10 px | not tried | 2 of 3 words lost | one misread | more misreads than ×2 |

Czech diacritics read perfectly at 12 px. OCR itself takes 5–15 ms on a typical snip; a full
2560×1440 snip at ×2 takes ~105 ms including scaling.
5. `RecognizeAsync(..).get()`: blocking is fine on the worker thread.
6. Text = each `OcrLine::Text()` joined with `\r\n`, trimmed. Empty → `None`.

The engine, bitmap and result are dropped before returning. The worker then trims the working set
as the image path does.

### Clipboard (clipboard.rs)

`pub fn set_text(text: &str) -> bool` puts UTF-16, NUL-terminated `CF_UNICODETEXT` on the clipboard,
reusing the existing owner window, `open` retry loop, `put` and `global`. The owner-window setup is
shared between `set` and `set_text` rather than copied.

`set_text` can sleep in the retry loop, so it is never called on the main thread: the keyboard hook
runs there, and Windows drops a low-level hook that stalls. Calls from the popup go through a
short-lived thread, the same way `deliver` does.

### Hand-off to the main thread

The worker posts `WM_APP_OCR` (`WM_APP + 4`) with `lParam = Box::into_raw(Box::new((text, sel)))`.
`main_proc` reclaims it with `Box::from_raw` and calls `popup::show`. If `PostMessageW` fails, the
worker frees the box itself.

## Popup (src/popup.rs)

### Window

- Top-level `WS_POPUP | WS_BORDER`, `WS_EX_TOPMOST | WS_EX_TOOLWINDOW` (no taskbar button), one
  child `EDIT` (`ES_MULTILINE | ES_AUTOVSCROLL`; `WS_VSCROLL` only when the text exceeds the height
  cap). The stock user32 edit box: no comctl32 load.
- Font: Segoe UI, `POPUP_FONT_PT` (starting value 10) scaled to the DPI of the snip's monitor,
  with a small inner margin (`EM_SETMARGINS`). The font is deleted with the window.
- Text: the recognized text, caret at the start, nothing selected (so a long text that scrolls
  shows its first lines).

### Size and position

- Width: the snip's width, clamped to [`POPUP_MIN_W`, `POPUP_MAX_W`] (starting values 240 and 640,
  in px at 100% scaling, DPI-scaled).
- Height: the measured text height (`DrawTextW` with `DT_CALCRECT` at that width) plus margins,
  capped at `POPUP_MAX_LINES` (starting value 12) lines; beyond that the edit box scrolls.
- Position (a pure function, unit-tested): left-aligned with the snip, a small gap below it. If it
  would leave the monitor's work area at the bottom, it goes above the snip instead; if neither
  fits, it sits at the bottom of the work area. X is clamped into the work area. The monitor is the
  one nearest the snip (`MonitorFromRect`).

### Focus

The popup is shown and then given focus (`SetForegroundWindow`, `SetFocus` on the edit box). RSnap
just received the mouse click, so Windows should permit this; it is verified during implementation.
If Windows refuses, the popup still shows and one click on it gives it focus.

### Keys

Handled in the main message loop before dispatch, only for messages to the popup's edit box:

| Key | Action |
|---|---|
| Enter | Copy the selection, or all text if nothing is selected (including edits), then close |
| Shift+Enter | New line (the edit box's normal behaviour) |
| Ctrl+C | Copy the selection (the edit box's normal behaviour); popup stays open |
| Ctrl+A | Select all (the stock edit box doesn't do this by itself) |
| Esc | Close; clipboard keeps whatever was last copied |

The keyboard hook only swallows Esc while the crosshair is up, so Esc reaches the popup.

### Closing

The popup closes on Enter, Esc, losing activation (clicking anywhere else, Alt+Tab), or a new
snip. Closing destroys the window and font, then trims the working set. Only one popup exists at a
time.

Gotcha, documented in the README: the popup has focus, so Ctrl+V straight after the snip pastes
into the popup. In both target workflows the user clicks into a different app to paste anyway,
which closes the popup.

### Message loop (main.rs)

The loop gains `TranslateMessage`, which the edit box needs to receive typed characters, plus a
`popup::pre_translate(&msg)` check for the keys above. With no popup open, both cost nothing beyond
a null check per message.

## Configuration (config.rs)

New constants, each with a comment like the existing ones: `OCR_SCALE` (2.0), `POPUP_FONT_PT`
(10), `POPUP_MIN_W` (240), `POPUP_MAX_W` (640), `POPUP_MAX_LINES` (12).

## Testing

- **OCR, against the real engine:** render known text (`RSnap 0123456789 ABC-7X4K29Q1`) into a DIB
  with GDI, run `ocr::recognize`, assert the digits and serial come back. Also a blank image returns
  `None`.
- **Pure helpers, unit tests:** line joining, scale selection against `MaxImageDimension`, popup
  placement (below, flipped above, pinned to the bottom, clamped left/right, monitor to the left of
  primary with negative coordinates).
- **Manual checklist:** Shift-release on screen text and on a WhatsApp photo; Ctrl+Shift gives
  text; no-text beep; popup focus; every key in the table; click-away; a new snip while the popup is
  open; popup on each monitor including a 150% one; plain release and Ctrl-release unchanged.
- **Numbers, reported against the README:** exe size, Shift-release → clipboard time, idle CPU over
  45 s, idle working set and private bytes after an OCR snip with the popup closed.

## Docs

README: usage table row, "What lands on the clipboard" note for text snips, a "How it works" bullet
for OCR and the popup, the new config constants, `ocr.rs` and `popup.rs` in the project structure,
the `windows` crate in Tech, updated Numbers, and in Known limits: the Ctrl+V gotcha, Latin-only
recognition, 1/l/I confusion in serials, and that snips wider than 5000 px get less enlargement.
