//! BMP export: the Windows layout (a version 3 header; version 4 with an alpha mask at 32 bits)
//! or the OS/2 one (a core header); 1, 4 or 8 bits per pixel through a palette ([`quantize`]),
//! optionally run-length encoded (RLE4, RLE8), or 16 (5-5-5), 24 or 32 bits (with alpha); rows
//! bottom-up, or top-down (a negative height).

use super::on_white;
use super::png::{pack, pixels_per_metre};
use super::quantize::{self, PaletteOptions, Reduction};

/// Bits per pixel a BMP can have.
pub const DEPTHS: [u8; 6] = [1, 4, 8, 16, 24, 32];
/// Those of the OS/2 layout.
pub const OS2_DEPTHS: [u8; 4] = [1, 4, 8, 24];

/// The BMP options of a raster export.
#[derive(Clone, Debug, PartialEq)]
pub struct BmpOptions {
    /// The OS/2 layout: 1, 4, 8 or 24 bits, uncompressed, rows bottom-up.
    pub os2: bool,
    /// Bits per pixel ([`DEPTHS`]).
    pub depth: u8,
    /// Run-length encode a 4- or 8-bit image.
    pub rle: bool,
    /// Rows top-down (flipped row order: a negative height) instead of bottom-up.
    pub top_down: bool,
    /// Greys instead of colours.
    pub gray: bool,
}

impl Default for BmpOptions {
    fn default() -> Self {
        Self { os2: false, depth: 24, rle: false, top_down: false, gray: false }
    }
}

impl BmpOptions {
    /// Why these options can't be written together, if they can't.
    pub fn check(&self) -> Result<(), String> {
        let err = |s: &str| Err(s.to_string());
        if !DEPTHS.contains(&self.depth) {
            return Err(format!("BMP depth {}: 1, 4, 8, 16, 24 or 32 bits per pixel", self.depth));
        }
        if self.os2 && !OS2_DEPTHS.contains(&self.depth) {
            return err("OS/2 bitmaps have 1, 4, 8 or 24 bits per pixel");
        }
        if self.rle && (self.os2 || !matches!(self.depth, 4 | 8)) {
            return err("RLE compression is for 4- and 8-bit Windows bitmaps");
        }
        if self.top_down && (self.rle || self.os2) {
            return err("compressed and OS/2 bitmaps store their rows bottom-up: turn off flipped rows");
        }
        Ok(())
    }

    /// The colours of a 1-, 4- or 8-bit image: black and white at 1 bit, greys, or `palette`.
    fn palette(&self, palette: &PaletteOptions) -> PaletteOptions {
        let reduction = match self.depth {
            1 => Reduction::BlackWhite,
            _ if self.gray => Reduction::Gray,
            _ => palette.reduction,
        };
        PaletteOptions { colors: 1 << self.depth, reduction, transparency: false, matte: Some([255; 3]), ..palette.clone() }
    }
}

/// Straight-alpha RGBA pixels as a BMP. At 1, 4 and 8 bits `palette` picks the colours (its
/// reduction and dither); every depth but 32 bits blends partly transparent pixels over white.
pub fn encode(rgba: &[u8], width: u32, height: u32, ppi: f64, o: &BmpOptions, palette: &PaletteOptions) -> Result<Vec<u8>, String> {
    o.check()?;
    let (w, h) = (width as usize, height as usize);
    let side = if o.os2 { u32::from(u16::MAX) } else { i32::MAX as u32 };
    if w == 0 || h == 0 || width > side || height > side || w.checked_mul(h).and_then(|n| n.checked_mul(4)) != Some(rgba.len()) {
        return Err(format!("BMP encoding failed: {width} × {height} pixels doesn't fit the format or the pixel buffer"));
    }
    let px = rgba.as_chunks::<4>().0;
    let gray = |c: [u8; 3]| if o.gray { [quantize::luma(c); 3] } else { c };
    // Bottom-up unless flipped.
    let row_at = |y: usize| if o.top_down { y } else { h - 1 - y };
    // Rows are padded to 4 bytes.
    let mut data = Vec::with_capacity((w * usize::from(o.depth)).div_ceil(32) * 4 * h);
    let mut colors = vec![];
    if o.depth <= 8 {
        let ix = quantize::quantize(rgba, width, height, &o.palette(palette));
        for y in 0..h {
            let row = ix.indices.get(row_at(y) * w..).and_then(|r| r.get(..w)).unwrap_or_default();
            if o.rle {
                rle_row(row, o.depth, &mut data);
                // End of line, or of the bitmap after the last.
                data.extend([0, u8::from(y + 1 == h)]);
            } else {
                data.extend_from_slice(&pack(row, width, 1, o.depth));
                data.resize(data.len().next_multiple_of(4), 0);
            }
        }
        colors = ix.palette;
    } else {
        for y in 0..h {
            for p in px.get(row_at(y) * w..).and_then(|r| r.get(..w)).unwrap_or_default() {
                match o.depth {
                    16 => {
                        let [r, g, b] = gray(on_white(p)).map(|v| u16::from(v >> 3));
                        data.extend((r << 10 | g << 5 | b).to_le_bytes());
                    }
                    24 => {
                        let [r, g, b] = gray(on_white(p));
                        data.extend([b, g, r]);
                    }
                    _ => {
                        let [r, g, b] = gray([p[0], p[1], p[2]]);
                        data.extend([b, g, r, p[3]]);
                    }
                }
            }
            data.resize(data.len().next_multiple_of(4), 0);
        }
    }
    header(&data, width, height, ppi, o, &colors)
}

/// The file: file header, info header, palette, then the pixel data.
fn header(data: &[u8], width: u32, height: u32, ppi: f64, o: &BmpOptions, colors: &[[u8; 3]]) -> Result<Vec<u8>, String> {
    let too_large = || "the image is too large for BMP: lower the resolution".to_string();
    let indexed = o.depth <= 8;
    let (info_len, entry) = match (o.os2, o.depth) {
        (true, _) => (12, 3),
        (false, 32) => (108, 4),
        (false, _) => (40, 4),
    };
    // OS/2 palettes have an entry for every index.
    let entries = match (indexed, o.os2) {
        (false, _) => 0,
        (true, true) => 1 << o.depth,
        (true, false) => colors.len(),
    };
    let offset = 14 + info_len + entries * entry;
    let size = u32::try_from(offset + data.len()).map_err(|_| too_large())?;
    let data_len = u32::try_from(data.len()).map_err(|_| too_large())?;
    let mut out = Vec::with_capacity(offset + data.len());
    out.extend(b"BM");
    out.extend(size.to_le_bytes());
    out.extend([0; 4]);
    out.extend((offset as u32).to_le_bytes());
    if o.os2 {
        out.extend(12u32.to_le_bytes());
        // Sides checked against u16 by `encode`.
        out.extend((width as u16).to_le_bytes());
        out.extend((height as u16).to_le_bytes());
        out.extend(1u16.to_le_bytes());
        out.extend(u16::from(o.depth).to_le_bytes());
    } else {
        let compression: u32 = match (o.rle, o.depth) {
            (true, 8) => 1,
            (true, _) => 2,
            (false, 32) => 3,
            (false, _) => 0,
        };
        let ppm = pixels_per_metre(ppi).min(i32::MAX as u32);
        let h = if o.top_down { -(height as i32) } else { height as i32 };
        out.extend((info_len as u32).to_le_bytes());
        out.extend(width.to_le_bytes());
        out.extend(h.to_le_bytes());
        out.extend(1u16.to_le_bytes());
        out.extend(u16::from(o.depth).to_le_bytes());
        out.extend(compression.to_le_bytes());
        out.extend(data_len.to_le_bytes());
        out.extend(ppm.to_le_bytes());
        out.extend(ppm.to_le_bytes());
        out.extend((entries as u32).to_le_bytes());
        out.extend(0u32.to_le_bytes());
        if info_len == 108 {
            // Red, green, blue and alpha masks, sRGB, then unused end points and gammas.
            for mask in [0x00FF_0000u32, 0x0000_FF00, 0x0000_00FF, 0xFF00_0000, u32::from_be_bytes(*b"sRGB")] {
                out.extend(mask.to_le_bytes());
            }
            out.extend([0; 48]);
        }
    }
    for i in 0..entries {
        let [r, g, b] = colors.get(i).copied().unwrap_or_default();
        out.extend(&[b, g, r, 0][..entry]);
    }
    out.extend_from_slice(data);
    Ok(out)
}

/// One row of palette indices run-length encoded (RLE8 at 8 bits, RLE4 at 4): runs of one index
/// as (count, index), other stretches of three pixels or more in absolute mode (0, count, the
/// pixels packed, padded to a word).
fn rle_row(row: &[u8], bits: u8, out: &mut Vec<u8>) {
    let run_byte = |v: u8| if bits == 4 { (v & 15) << 4 | (v & 15) } else { v };
    let mut i = 0;
    while let Some(&v) = row.get(i) {
        let run = row.get(i..).map_or(1, |r| r.iter().take(255).take_while(|x| **x == v).count());
        if run >= 2 {
            out.extend([run as u8, run_byte(v)]);
            i += run;
            continue;
        }
        // Up to the next pair of equal pixels.
        let mut j = i + 1;
        while j < row.len() && j - i < 255 && row.get(j..j + 2).is_none_or(|p| p[0] != p[1]) {
            j += 1;
        }
        let lit = row.get(i..j).unwrap_or_default();
        if lit.len() < 3 {
            // Absolute mode needs three pixels: runs of one.
            lit.iter().for_each(|x| out.extend([1, run_byte(*x)]));
        } else {
            out.extend([0, lit.len() as u8]);
            let start = out.len();
            match bits {
                4 => out.extend(lit.chunks(2).map(|p| (p[0] & 15) << 4 | p.get(1).map_or(0, |x| x & 15))),
                _ => out.extend_from_slice(lit),
            }
            if (out.len() - start) % 2 == 1 {
                out.push(0);
            }
        }
        i = j;
    }
}
