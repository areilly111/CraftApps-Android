//! Glyph grids for the Glyphs panel: a font's characters drawn into square cells.

use designcraft_fonts::ScopedFonts;
use vello_cpu::{RenderContext, Resources, kurbo, peniko};

use crate::Rendered;

/// Draw `chars` of `family`/`style` (as `db`, a document's fonts, has it) into a grid of `cols`
/// columns of `cell`-pixel cells, in `color` (straight RGBA) on transparency. Glyphs are scaled to
/// the font's em and centred.
pub fn glyph_grid(db: &ScopedFonts<'_>, family: &str, style: &str, chars: &[char], cols: u32, cell: u32, color: [u8; 4]) -> Rendered {
    let face = db.face(family, style);
    let cols = cols.max(1);
    let rows = (chars.len() as u32).div_ceil(cols).max(1);
    let (w, h) = ((cols * cell).clamp(1, u16::MAX as u32) as u16, (rows * cell).clamp(1, u16::MAX as u32) as u16);
    let mut ctx = RenderContext::new(w, h);
    ctx.set_paint(peniko::Color::from_rgba8(color[0], color[1], color[2], color[3]));
    let upem = face.units_per_em().max(1.0);
    let (asc, desc) = face.vertical_metrics();
    let k = cell as f64 * 0.62 / upem;
    for (i, c) in chars.iter().enumerate() {
        let gid = face.glyph_for(*c);
        if gid == 0 {
            continue;
        }
        let outline = db.outline(&face, gid);
        if outline.elements().is_empty() {
            continue;
        }
        let (col, row) = (i as u32 % cols, i as u32 / cols);
        let adv = face.advance(gid);
        // Horizontally centred by advance; the baseline puts the em box in the middle.
        let x = (col * cell) as f64 + (cell as f64 - adv * k) / 2.0;
        let mid = (asc - desc.abs()) / 2.0;
        let y = (row * cell) as f64 + cell as f64 / 2.0 + mid * k;
        ctx.set_transform(kurbo::Affine::translate((x, y)) * kurbo::Affine::scale(k));
        ctx.fill_path(&outline);
    }
    ctx.flush();
    let mut pixels = vec![0u8; w as usize * h as usize * 4];
    let mut res = Resources::new();
    if let Some(pm) = vello_cpu::PixmapMut::new(w, h, &mut pixels) {
        ctx.render(pm, &mut res);
    }
    Rendered { width: w as u32, height: h as u32, pixels }
}

/// `text` set in `family`/`style` (as `db`, a document's fonts, has it) on one line, `height`
/// pixels tall (the em fits the height), in `color` on transparency — for font menus that preview
/// each family.
pub fn text_line(db: &ScopedFonts<'_>, family: &str, style: &str, text: &str, height: u32, color: [u8; 4]) -> Rendered {
    let face = db.face(family, style);
    let upem = face.units_per_em().max(1.0);
    let k = height as f64 * 0.72 / upem;
    let glyphs = designcraft_fonts::shape(&face, text, &[], |c| c);
    let width: f64 = glyphs.iter().map(|g| g.x_advance as f64 * k).sum::<f64>().ceil() + 2.0;
    let (w, h) = ((width as u32).clamp(1, 2048) as u16, height.clamp(1, 512) as u16);
    let mut ctx = RenderContext::new(w, h);
    ctx.set_paint(peniko::Color::from_rgba8(color[0], color[1], color[2], color[3]));
    let (asc, desc) = face.vertical_metrics();
    let baseline = h as f64 / 2.0 + (asc - desc.abs()) / 2.0 * k;
    let mut x = 1.0;
    // A preview shows what the font draws: not the box DesignCraft draws for a missing glyph.
    let boxed = glyphs.iter().any(|g| g.gid == 0) && db.missing_box(&face).is_some();
    for g in &glyphs {
        let outline = db.outline(&face, g.gid);
        if !(outline.elements().is_empty() || (boxed && g.gid == 0)) {
            ctx.set_transform(kurbo::Affine::translate((x + g.x_offset as f64 * k, baseline - g.y_offset as f64 * k)) * kurbo::Affine::scale(k));
            ctx.fill_path(&outline);
        }
        x += g.x_advance as f64 * k;
    }
    ctx.flush();
    let mut pixels = vec![0u8; w as usize * h as usize * 4];
    let mut res = Resources::new();
    if let Some(pm) = vello_cpu::PixmapMut::new(w, h, &mut pixels) {
        ctx.render(pm, &mut res);
    }
    Rendered { width: w as u32, height: h as u32, pixels }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_draws_each_cell() {
        let db = designcraft_fonts::FontDb::global().scoped(0);
        let img = glyph_grid(&db, designcraft_fonts::DEFAULT_FAMILY, "Regular", &['A', 'B', 'C', ' '], 2, 40, [0, 0, 0, 255]);
        assert_eq!((img.width, img.height), (80, 80));
        let ink = |cx: u32, cy: u32| {
            (cx * 40..cx * 40 + 40).flat_map(|x| (cy * 40..cy * 40 + 40).map(move |y| (x, y))).filter(|&(x, y)| img.pixel(x, y)[3] > 128).count()
        };
        assert!(ink(0, 0) > 30 && ink(1, 0) > 30 && ink(0, 1) > 30, "letters drawn");
        assert_eq!(ink(1, 1), 0, "a space is blank");
    }

    #[test]
    fn text_line_draws_the_name() {
        let db = designcraft_fonts::FontDb::global().scoped(0);
        let img = text_line(&db, designcraft_fonts::DEFAULT_FAMILY, "Regular", "Serif", 24, [0, 0, 0, 255]);
        assert_eq!(img.height, 24);
        assert!(img.width > 30);
        let ink = (0..img.width).flat_map(|x| (0..img.height).map(move |y| (x, y))).filter(|&(x, y)| img.pixel(x, y)[3] > 128).count();
        assert!(ink > 40, "{ink}");
    }

    #[test]
    fn text_line_draws_upright_and_whole() {
        let db = designcraft_fonts::FontDb::global().scoped(0);
        let img = text_line(&db, designcraft_fonts::DEFAULT_FAMILY, "Regular", "L", 40, [0, 0, 0, 255]);
        let inked = |x: u32, y: u32| img.pixel(x, y)[3] > 128;
        let row_ink = |y: u32| (0..img.width).filter(|&x| inked(x, y)).count();
        let rows: Vec<u32> = (0..img.height).filter(|&y| row_ink(y) > 0).collect();
        let (Some(&top), Some(&bottom)) = (rows.first(), rows.last()) else { panic!("no ink") };
        // Nothing touches the edges: the letter isn't cut off.
        assert!(top > 0 && bottom < img.height - 1, "clipped: ink rows {top}..={bottom} of {}", img.height);
        assert!((0..img.height).all(|y| !inked(0, y) && !inked(img.width - 1, y)), "clipped at the sides");
        // An upright L has its bar at the bottom: its widest ink row is in the lower half.
        let widest = rows.iter().copied().max_by_key(|&y| row_ink(y)).unwrap_or(top);
        assert!(widest > (top + bottom) / 2, "upside down: widest row {widest} in ink rows {top}..={bottom}");
    }
}
