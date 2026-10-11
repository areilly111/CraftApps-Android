//! GIF export: a palette image ([`super::quantize`]) as a one-frame GIF89a with its transparent
//! entry, optionally interlaced (rows in four passes, so the image builds up while it loads).

use std::borrow::Cow;

use super::quantize::Indexed;

/// Encode `img` as a GIF file.
pub fn encode(img: &Indexed, interlaced: bool) -> Result<Vec<u8>, String> {
    let fail = |e: gif::EncodingError| format!("GIF encoding failed: {e}");
    let (Ok(w), Ok(h)) = (u16::try_from(img.width), u16::try_from(img.height)) else {
        return Err(format!("{} × {} pixels is too large for GIF (at most 65535 pixels a side): lower the resolution", img.width, img.height));
    };
    if w == 0 || h == 0 || img.indices.len() != w as usize * h as usize || img.palette.is_empty() || img.palette.len() > 256 {
        return Err("GIF encoding failed: the palette image is malformed".into());
    }
    let buffer = if interlaced { Cow::Owned(interlace(&img.indices, w as usize)) } else { Cow::Borrowed(img.indices.as_slice()) };
    let mut enc = gif::Encoder::new(Vec::new(), w, h, &img.palette.concat()).map_err(fail)?;
    let frame = gif::Frame { width: w, height: h, buffer, transparent: img.transparent, interlaced, ..gif::Frame::default() };
    enc.write_frame(&frame).map_err(fail)?;
    enc.into_inner().map_err(fail)
}

/// The rows of a `width`-wide image in GIF's interlaced order: every 8th row from 0, every 8th
/// from 4, every 4th from 2, then every 2nd from 1.
fn interlace(px: &[u8], width: usize) -> Vec<u8> {
    let rows: Vec<&[u8]> = px.chunks(width.max(1)).collect();
    [(0, 8), (4, 8), (2, 4), (1, 2)]
        .into_iter()
        .flat_map(|(start, step)| rows.iter().skip(start).step_by(step))
        .flat_map(|r| r.iter().copied())
        .collect()
}
