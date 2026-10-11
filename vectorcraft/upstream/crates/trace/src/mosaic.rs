//! Object → Create Object Mosaic: average colours of a grid of tiles over a raster.

use crate::Raster;

/// Average colour (straight RGBA) of each of `cols` × `rows` tiles covering `img`, row by row.
/// Transparent pixels count by their alpha, so a tile over transparency fades out.
pub fn mosaic(img: &Raster, cols: u32, rows: u32) -> Vec<[u8; 4]> {
    let (cols, rows) = (cols.max(1), rows.max(1));
    let mut out = Vec::with_capacity((cols * rows) as usize);
    for r in 0..rows {
        let y0 = (r as u64 * img.height as u64 / rows as u64) as u32;
        let y1 = (((r + 1) as u64 * img.height as u64 / rows as u64) as u32).max(y0 + 1).min(img.height);
        for c in 0..cols {
            let x0 = (c as u64 * img.width as u64 / cols as u64) as u32;
            let x1 = (((c + 1) as u64 * img.width as u64 / cols as u64) as u32).max(x0 + 1).min(img.width);
            let (mut sr, mut sg, mut sb, mut sa, mut n) = (0u64, 0u64, 0u64, 0u64, 0u64);
            for y in y0..y1.max(y0 + 1).min(img.height) {
                for x in x0..x1.max(x0 + 1).min(img.width) {
                    let [pr, pg, pb, pa] = img.pixel(x, y);
                    let a = pa as u64;
                    sr += pr as u64 * a;
                    sg += pg as u64 * a;
                    sb += pb as u64 * a;
                    sa += a;
                    n += 1;
                }
            }
            out.push(match (sr.checked_div(sa), sg.checked_div(sa), sb.checked_div(sa)) {
                (Some(r), Some(g), Some(b)) => [r as u8, g as u8, b as u8, (sa / n.max(1)) as u8],
                _ => [0, 0, 0, 0],
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn averages_each_tile() {
        // Left half red, right half blue; a 2 × 1 mosaic gives one tile of each.
        let img = Raster::from_fn(10, 4, |x, _| if x < 5 { [255, 0, 0, 255] } else { [0, 0, 255, 255] });
        assert_eq!(mosaic(&img, 2, 1), vec![[255, 0, 0, 255], [0, 0, 255, 255]]);
        // One tile: the average, purple.
        let m = mosaic(&img, 1, 1);
        assert_eq!(m[0][3], 255);
        assert!((m[0][0] as i32 - 127).abs() <= 1 && (m[0][2] as i32 - 127).abs() <= 1);
        // More tiles than pixels still works.
        assert_eq!(mosaic(&img, 20, 8).len(), 160);
        // Transparency.
        let clear = Raster::from_fn(2, 2, |_, _| [9, 9, 9, 0]);
        assert_eq!(mosaic(&clear, 1, 1), vec![[0, 0, 0, 0]]);
    }
}
