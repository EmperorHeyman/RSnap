use std::io::Write;
use std::mem::size_of;
use std::ptr::copy_nonoverlapping;

use windows_sys::Win32::Foundation::HGLOBAL;
use windows_sys::Win32::Graphics::Gdi::{BI_RGB, BITMAPINFOHEADER};
use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};

pub fn png(px: &[u8], w: u32, h: u32) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(px.len() / 6);
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Fast);
        let mut writer = enc.write_header().ok()?;
        let mut stream = writer.stream_writer().ok()?;
        let mut row = vec![0u8; w as usize * 3];
        for src in px.chunks_exact(w as usize * 4) {
            for (d, s) in row.chunks_exact_mut(3).zip(src.chunks_exact(4)) {
                d[0] = s[2];
                d[1] = s[1];
                d[2] = s[0];
            }
            stream.write_all(&row).ok()?;
        }
        stream.finish().ok()?;
        writer.finish().ok()?;
    }
    Some(out)
}

/// Bottom-up 24bpp, the DIB layout every app reads. Windows synthesizes CF_BITMAP and CF_DIBV5 from it.
pub fn dib(px: &[u8], w: u32, h: u32) -> Option<HGLOBAL> {
    let (w, h) = (w as usize, h as usize);
    let stride = (w * 3 + 3) & !3;
    let hdr = size_of::<BITMAPINFOHEADER>();
    unsafe {
        let mem = GlobalAlloc(GMEM_MOVEABLE, hdr + stride * h);
        if mem.is_null() {
            return None;
        }
        let base = GlobalLock(mem) as *mut u8;
        let info = BITMAPINFOHEADER {
            biSize: hdr as u32,
            biWidth: w as i32,
            biHeight: h as i32,
            biPlanes: 1,
            biBitCount: 24,
            biCompression: BI_RGB,
            biSizeImage: (stride * h) as u32,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        };
        copy_nonoverlapping(&info as *const _ as *const u8, base, hdr);
        let body = std::slice::from_raw_parts_mut(base.add(hdr), stride * h);
        for (y, src) in px.chunks_exact(w * 4).enumerate() {
            let dst = &mut body[(h - 1 - y) * stride..][..stride];
            for (d, s) in dst.chunks_exact_mut(3).zip(src.chunks_exact(4)) {
                d.copy_from_slice(&s[..3]);
            }
            dst[w * 3..].fill(0);
        }
        GlobalUnlock(mem);
        Some(mem)
    }
}
