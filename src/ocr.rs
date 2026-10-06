//! Text from pixels, using the OCR engine built into Windows. Nothing is kept between snips.

use std::ptr::null;

use windows::Globalization::Language;
use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::core::HSTRING;
use windows::Media::Ocr::OcrEngine;
use windows::Security::Cryptography::CryptographicBuffer;
use windows_sys::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};

use crate::config::OCR_SCALE;

/// `px` is top-down BGRA, as captured. `lang` is a recognizer's language tag; `None` or one that
/// isn't installed means the Windows display language. `None` back when nothing readable was found.
pub fn recognize(px: &[u8], w: u32, h: u32, lang: Option<&str>) -> Option<String> {
    with_com(|| run(px, w, h, lang).ok().flatten())
}

/// Installed recognizers as (language tag, display name), for the settings window.
pub fn languages() -> Vec<(String, String)> {
    let list = || -> windows::core::Result<Vec<(String, String)>> {
        let mut out = Vec::new();
        for lang in OcrEngine::AvailableRecognizerLanguages()? {
            out.push((lang.LanguageTag()?.to_string_lossy(), lang.DisplayName()?.to_string_lossy()));
        }
        Ok(out)
    };
    with_com(|| list().unwrap_or_default())
}

fn with_com<T>(f: impl FnOnce() -> T) -> T {
    unsafe {
        let hr = CoInitializeEx(null(), COINIT_MULTITHREADED as u32);
        let out = f();
        if hr >= 0 {
            CoUninitialize();
        }
        out
    }
}

fn run(px: &[u8], w: u32, h: u32, lang: Option<&str>) -> windows::core::Result<Option<String>> {
    let (sw, sh) = scaled_size(w, h, OCR_SCALE, OcrEngine::MaxImageDimension()?);
    let gray = gray_scaled(px, w, h, sw, sh);
    let buffer = CryptographicBuffer::CreateFromByteArray(&gray)?;
    drop(gray);
    let bitmap =
        SoftwareBitmap::CreateCopyFromBuffer(&buffer, BitmapPixelFormat::Gray8, sw as i32, sh as i32)?;
    drop(buffer);
    let result = engine(lang)?.RecognizeAsync(&bitmap)?.join()?;
    let lines = result.Lines()?.into_iter().map(|line| {
        line.Text()
            .map(|t| t.to_string_lossy())
            .unwrap_or_default()
    });
    Ok(join_lines(lines))
}

/// The chosen recognizer, else the one for the Windows display language, else any installed one.
fn engine(lang: Option<&str>) -> windows::core::Result<OcrEngine> {
    let chosen = || -> windows::core::Result<OcrEngine> {
        let tag = lang.ok_or_else(windows::core::Error::empty)?;
        OcrEngine::TryCreateFromLanguage(&Language::CreateLanguage(&HSTRING::from(tag))?)
    };
    chosen()
        .or_else(|_| OcrEngine::TryCreateFromUserProfileLanguages())
        .or_else(|_| {
            let lang = OcrEngine::AvailableRecognizerLanguages()?.GetAt(0)?;
            OcrEngine::TryCreateFromLanguage(&lang)
        })
}

/// `scale` times bigger, shrunk if needed so neither side passes the engine's limit.
fn scaled_size(w: u32, h: u32, scale: f32, max_dim: u32) -> (u32, u32) {
    let s = scale.min(max_dim as f32 / w.max(h) as f32);
    let fit = |n: u32| ((n as f32 * s).round() as u32).clamp(1, max_dim);
    (fit(w), fit(h))
}

/// Luma of top-down BGRA, resized to `sw` x `sh` with bilinear filtering.
/// One byte a pixel keeps an enlarged full-screen snip small; the engine reads grayscale anyway.
fn gray_scaled(px: &[u8], w: u32, h: u32, sw: u32, sh: u32) -> Vec<u8> {
    let (w, h, sw, sh) = (w as usize, h as usize, sw as usize, sh as usize);
    let luma: Vec<u8> = px
        .chunks_exact(4)
        .map(|p| ((p[2] as u32 * 77 + p[1] as u32 * 150 + p[0] as u32 * 29) >> 8) as u8)
        .collect();
    // For each output pixel along one axis: the two source pixels and the weight of the second.
    let taps = |n: usize, sn: usize| -> Vec<(usize, usize, f32)> {
        (0..sn)
            .map(|i| {
                let f = ((i as f32 + 0.5) * n as f32 / sn as f32 - 0.5).max(0.0);
                let i0 = (f as usize).min(n - 1);
                (i0, (i0 + 1).min(n - 1), f - i0 as f32)
            })
            .collect()
    };
    let (cols, rows) = (taps(w, sw), taps(h, sh));
    let mut out = vec![0u8; sw * sh];
    for (row, &(y0, y1, ty)) in out.chunks_exact_mut(sw).zip(&rows) {
        let (r0, r1) = (&luma[y0 * w..][..w], &luma[y1 * w..][..w]);
        for (o, &(x0, x1, tx)) in row.iter_mut().zip(&cols) {
            let top = r0[x0] as f32 + (r0[x1] as f32 - r0[x0] as f32) * tx;
            let bot = r1[x0] as f32 + (r1[x1] as f32 - r1[x0] as f32) * tx;
            *o = (top + (bot - top) * ty + 0.5) as u8;
        }
    }
    out
}

/// Serial numbers and codes. The engine reads them as words, so 0 comes back as O and 1 as I most
/// of all (2160 rendered serials without the letters I and O: 62% exact without this, about 78% with
/// it; serials that do use those letters lose about 7 points). In tokens that look
/// like codes those letters become digits again; ordinary words and numbers are left alone.
/// It's wrong for codes that really contain the letters O or I, so it can be turned off.
pub fn fix_codes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut token = String::new();
    for c in text.chars() {
        if c.is_whitespace() {
            push_token(&mut out, &token);
            token.clear();
            out.push(c);
        } else {
            token.push(c);
        }
    }
    push_token(&mut out, &token);
    out
}

fn push_token(out: &mut String, token: &str) {
    if is_code(token) {
        out.extend(token.chars().map(|c| match c {
            'O' | 'Ô' | 'Ó' | 'Ö' | 'Ő' | 'ô' | 'ó' | 'ö' | 'ő' => '0',
            'I' | 'Í' | 'í' | 'l' | 'Ł' | 'ł' | '|' | '!' => '1',
            c => c,
        }));
    } else {
        out.push_str(token);
    }
}

/// At least four letters or digits, at least one digit, and no lowercase word in it. "CO2", "51.2"
/// and "Model2024" aren't codes; "A123-LFP-88213" is.
fn is_code(token: &str) -> bool {
    let alnum = token.chars().filter(|c| c.is_alphanumeric()).count();
    let digit = token.chars().any(|c| c.is_ascii_digit());
    let lowercase_run = token
        .chars()
        .fold((0, 0), |(run, longest), c| {
            let run = if c.is_lowercase() { run + 1 } else { 0 };
            (run, longest.max(run))
        })
        .1;
    alnum >= 4 && digit && lowercase_run < 3
}

/// One line of text per OCR line, CRLF between them as the clipboard expects. Blank lines dropped.
fn join_lines(lines: impl IntoIterator<Item = String>) -> Option<String> {
    let text = lines
        .into_iter()
        .map(|l| l.trim().to_owned())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\r\n");
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{size_of, zeroed};
    use std::ptr::null_mut;
    use windows_sys::Win32::Graphics::Gdi::*;
    use windows_sys::w;

    /// Black Segoe UI text on white, top-down BGRA with the alpha byte at 0, as BitBlt leaves it.
    fn render(lines: &[&str], px: i32, w: i32, h: i32) -> Vec<u8> {
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            let mut bmi: BITMAPINFO = zeroed();
            bmi.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
            bmi.bmiHeader.biWidth = w;
            bmi.bmiHeader.biHeight = -h;
            bmi.bmiHeader.biPlanes = 1;
            bmi.bmiHeader.biBitCount = 32;
            bmi.bmiHeader.biCompression = BI_RGB;
            let mut bits = null_mut();
            let bmp = CreateDIBSection(dc, &bmi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            let old = SelectObject(dc, bmp);
            let body = std::slice::from_raw_parts_mut(bits as *mut u8, (w * h * 4) as usize);
            for p in body.chunks_exact_mut(4) {
                p.copy_from_slice(&[255, 255, 255, 0]);
            }
            let font = CreateFontW(
                -px,
                0,
                0,
                0,
                FW_NORMAL as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                0,
                0,
                CLEARTYPE_QUALITY as u32,
                0,
                w!("Segoe UI"),
            );
            let old_font = SelectObject(dc, font);
            SetBkMode(dc, TRANSPARENT as i32);
            for (i, line) in lines.iter().enumerate() {
                let t: Vec<u16> = line.encode_utf16().collect();
                TextOutW(dc, 4, 4 + i as i32 * px * 2, t.as_ptr(), t.len() as i32);
            }
            GdiFlush();
            let out = body.to_vec();
            SelectObject(dc, old_font);
            DeleteObject(font);
            SelectObject(dc, old);
            DeleteObject(bmp);
            DeleteDC(dc);
            out
        }
    }

    #[test]
    fn reads_small_screen_text() {
        // 12 px is Segoe UI 9 pt at 100% scaling. At 1x the engine returns nothing for it.
        let (w, h) = (300, 32);
        let px = render(&["RSnap 0123456789 ABC-7X4K29Q1"], 12, w, h);
        let text = recognize(&px, w as u32, h as u32, None).expect("text");
        assert!(text.contains("0123456789"), "{text:?}");
        assert!(text.contains("7X4K29Q1"), "{text:?}");
    }

    #[test]
    fn keeps_line_breaks() {
        let (w, h) = (300, 80);
        let px = render(&["First line here", "Second line 42"], 16, w, h);
        assert_eq!(
            recognize(&px, w as u32, h as u32, None).as_deref(),
            Some("First line here\r\nSecond line 42")
        );
    }

    #[test]
    fn reads_czech() {
        let (w, h) = (400, 40);
        let px = render(&["Žluťoučký kůň úpěl ďábelské ódy 2024"], 12, w, h);
        assert_eq!(
            recognize(&px, w as u32, h as u32, None).as_deref(),
            Some("Žluťoučký kůň úpěl ďábelské ódy 2024")
        );
    }

    #[test]
    fn reads_a_snip_too_wide_to_double() {
        // A whole 8320 px desktop only fits 1.2x under the engine limit. The last character of a
        // serial is where 1, l and I get confused, so it is left out of the check.
        let (w, h) = (8320, 60);
        let px = render(&["Serial BAT-7X4K29Q1 at the far left"], 14, w, h);
        let text = recognize(&px, w as u32, h as u32, None).expect("text");
        assert!(text.contains("BAT-7X4K29Q"), "{text:?}");
    }

    #[test]
    fn reads_with_a_chosen_language() {
        let (w, h) = (300, 32);
        let px = render(&["RSnap 0123456789 ABC-7X4K29Q1"], 12, w, h);
        let text = recognize(&px, w as u32, h as u32, Some("en-US")).expect("text");
        assert!(text.contains("0123456789"), "{text:?}");
    }

    #[test]
    fn unknown_language_falls_back_to_the_windows_one() {
        let (w, h) = (300, 32);
        let px = render(&["RSnap 0123456789"], 12, w, h);
        let text = recognize(&px, w as u32, h as u32, Some("xx-NOPE")).expect("text");
        assert!(text.contains("0123456789"), "{text:?}");
    }

    #[test]
    fn lists_installed_languages() {
        // This machine has the Czech and English OCR packs.
        let tags: Vec<String> = languages().into_iter().map(|(tag, _)| tag).collect();
        assert!(tags.iter().any(|t| t == "cs"), "{tags:?}");
        assert!(tags.iter().any(|t| t == "en-US"), "{tags:?}");
    }

    #[test]
    fn blank_is_none() {
        let px = vec![255u8; 200 * 50 * 4];
        assert_eq!(recognize(&px, 200, 50, None), None);
    }

    #[test]
    fn scales_up_within_the_engine_limit() {
        assert_eq!(scaled_size(500, 350, 2.0, 10000), (1000, 700));
        // An 8320 px wide desktop only fits 1.2x.
        assert_eq!(scaled_size(8320, 1440, 2.0, 10000), (10000, 1731));
        // Wider than the limit even at 1x: shrink to fit.
        assert_eq!(scaled_size(12000, 100, 2.0, 10000), (10000, 83));
        assert_eq!(scaled_size(1, 1, 2.0, 10000), (2, 2));
    }

    #[test]
    fn gray_uses_luma_weights() {
        // BGRA: white, black, pure red, pure green.
        let px = [255, 255, 255, 0, 0, 0, 0, 0, 0, 0, 255, 0, 0, 255, 0, 0];
        assert_eq!(gray_scaled(&px, 4, 1, 4, 1), [255, 0, 76, 149]);
    }

    #[test]
    fn gray_upscale_is_bilinear() {
        // Black then white, doubled: the middle pixels blend.
        let px = [0, 0, 0, 0, 255, 255, 255, 0];
        assert_eq!(gray_scaled(&px, 2, 1, 4, 2), [0, 64, 191, 255, 0, 64, 191, 255]);
    }

    #[test]
    fn code_fix_turns_letters_in_codes_into_digits() {
        // The engine's most common misreads in serials: 0 as O and 1 as I, plus Czech accents.
        assert_eq!(fix_codes("BAT-7X4K29QI"), "BAT-7X4K29Q1");
        assert_eq!(fix_codes("MHLV-OMLO-7CSD"), "MHLV-0ML0-7CSD");
        assert_eq!(fix_codes("SN: A123-LFP-882l3"), "SN: A123-LFP-88213");
        assert_eq!(fix_codes("DAKô-1E8M"), "DAK0-1E8M");
        assert_eq!(fix_codes("Z06M-FC2J-ŁO1O"), "Z06M-FC2J-1010");
    }

    #[test]
    fn code_fix_leaves_ordinary_text_alone() {
        for t in [
            "Battery pack nominal voltage 51.2 V",
            "Hradec Králové 2024",
            "Model LFP280 cell lot 07",
            "ISO 9001 certified",
            "Store between 10 and 35 °C",
            "CO2 and H2O",
            "Win11 Model2024",
        ] {
            assert_eq!(fix_codes(t), t);
        }
    }

    #[test]
    fn code_fix_keeps_line_breaks() {
        assert_eq!(fix_codes("SN BAT-IOO2\r\nLot 0O71"), "SN BAT-1002\r\nLot 0071");
    }

    #[test]
    fn join_lines_trims_and_drops_blanks() {
        let lines = ["  SN 123 ", "", "  ", "Made in CZ"].map(String::from);
        assert_eq!(join_lines(lines).as_deref(), Some("SN 123\r\nMade in CZ"));
        assert_eq!(join_lines(["", " "].map(String::from)), None);
    }
}
