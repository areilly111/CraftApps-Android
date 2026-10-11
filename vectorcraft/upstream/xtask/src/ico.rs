//! `cargo xtask ico <out.ico> <png>...`: pack square PNGs (each at most 256 px) into a Windows `.ico`.
//!
//! Every entry is stored as PNG data, which Windows Vista and later read at every size. Used by
//! `packaging/icons.sh`.

use std::path::Path;

/// Width and height from a PNG's IHDR chunk.
fn png_size(png: &[u8]) -> Result<(u32, u32), String> {
    if png.len() < 24 || &png[..8] != b"\x89PNG\r\n\x1a\n" || &png[12..16] != b"IHDR" {
        return Err("not a PNG".into());
    }
    let be = |i: usize| u32::from_be_bytes([png[i], png[i + 1], png[i + 2], png[i + 3]]);
    Ok((be(16), be(20)))
}

/// Build the `.ico` bytes from PNG images.
pub fn pack(pngs: &[Vec<u8>]) -> Result<Vec<u8>, String> {
    let count = u16::try_from(pngs.len()).map_err(|_| "too many images")?;
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]);
    out.extend_from_slice(&count.to_le_bytes());
    let mut offset = 6 + 16 * pngs.len() as u32;
    for png in pngs {
        let (w, h) = png_size(png)?;
        if w > 256 || h > 256 {
            return Err(format!("{w}x{h} is larger than 256 px"));
        }
        // 256 is stored as 0.
        out.push((w % 256) as u8);
        out.push((h % 256) as u8);
        out.extend_from_slice(&[0, 0]); // palette size, reserved
        out.extend_from_slice(&1u16.to_le_bytes()); // colour planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        out.extend_from_slice(&(png.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for png in pngs {
        out.extend_from_slice(png);
    }
    Ok(out)
}

pub fn run(args: &[&str]) -> Result<(), String> {
    let [out, inputs @ ..] = args else {
        return Err("usage: cargo xtask ico <out.ico> <png>...".into());
    };
    if inputs.is_empty() {
        return Err("usage: cargo xtask ico <out.ico> <png>...".into());
    }
    let pngs = inputs.iter().map(|p| std::fs::read(p).map_err(|e| format!("{p}: {e}"))).collect::<Result<Vec<_>, _>>()?;
    std::fs::write(Path::new(out), pack(&pngs)?).map_err(|e| format!("{out}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_png() {
        assert!(pack(&[b"nope".to_vec()]).is_err());
    }
}
