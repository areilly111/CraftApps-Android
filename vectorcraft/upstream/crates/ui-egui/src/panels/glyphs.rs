//! Glyphs panel: every character the current font maps (its cmap), drawn from the font's own
//! outlines. Double-click inserts the character at the Type tool's caret (or appends it to the
//! selected text objects).

use std::collections::HashMap;

use egui::{Color32, Sense, Ui, vec2};
use serde_json::json;
use vectorcraft_geom::{Affine, BezPath, PathEl};
use vectorcraft_text::FontDb;

use super::{pstate, set_pstate};
use crate::VectorcraftApp;
use crate::theme::Tokens;
use crate::widgets::{self, menu_item};

const CELL: f32 = 30.0;

/// Character subsets offered by the Show menu.
const SUBSETS: [(&str, u32, u32); 7] = [
    ("Entire Font", 0, 0x10FFFF),
    ("Basic Latin", 0x20, 0x7F),
    ("Latin-1 Supplement", 0xA0, 0xFF),
    ("Latin Extended", 0x100, 0x24F),
    ("Punctuation", 0x2000, 0x206F),
    ("Currency & Symbols", 0x20A0, 0x2BFF),
    ("Greek & Cyrillic", 0x370, 0x52F),
];

thread_local! {
    static TEX: crate::graphics::TexCache<HashMap<(u32, u32, u32), egui::TextureHandle>> = crate::graphics::TexCache::default();
}

/// Antialiased coverage mask (nonzero winding) of `path` in a `w`×`h` pixel grid.
pub(crate) fn rasterize(path: &BezPath, w: usize, h: usize) -> Vec<u8> {
    let mut edges: Vec<(f64, f64, f64, f64)> = vec![];
    let (mut start, mut last) = (None, (0.0, 0.0));
    vectorcraft_geom::kurbo::flatten(path, 0.1, |el| match el {
        PathEl::MoveTo(p) => {
            if let Some(s) = start.replace((p.x, p.y)) {
                edges.push((last.0, last.1, s.0, s.1));
            }
            last = (p.x, p.y);
        }
        PathEl::LineTo(p) => {
            edges.push((last.0, last.1, p.x, p.y));
            last = (p.x, p.y);
        }
        PathEl::ClosePath => {
            if let Some(s) = start.take() {
                edges.push((last.0, last.1, s.0, s.1));
                last = s;
            }
        }
        _ => {}
    });
    if let Some(s) = start {
        edges.push((last.0, last.1, s.0, s.1));
    }
    const SUB: usize = 4;
    let mut acc = vec![0f32; w * h];
    let mut xs: Vec<(f64, i32)> = vec![];
    for row in 0..h {
        for sy in 0..SUB {
            let y = row as f64 + (sy as f64 + 0.5) / SUB as f64;
            xs.clear();
            for &(x0, y0, x1, y1) in &edges {
                if (y0 <= y) != (y1 <= y) {
                    xs.push((x0 + (y - y0) / (y1 - y0) * (x1 - x0), if y1 > y0 { 1 } else { -1 }));
                }
            }
            xs.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut wind = 0;
            for k in 0..xs.len() {
                wind += xs[k].1;
                if wind == 0 || k + 1 >= xs.len() {
                    continue;
                }
                let (a, b) = (xs[k].0.max(0.0), xs[k + 1].0.min(w as f64));
                if b <= a {
                    continue;
                }
                for px in (a.floor() as usize)..(b.ceil() as usize).min(w) {
                    let cov = (b.min(px as f64 + 1.0) - a.max(px as f64)).max(0.0);
                    acc[row * w + px] += (cov / SUB as f64) as f32;
                }
            }
        }
    }
    acc.into_iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8).collect()
}

fn glyph_texture(ctx: &egui::Context, face: &vectorcraft_text::FontFace, gid: u32, px: u32) -> egui::TextureHandle {
    let key = (face.id(), gid, px);
    if let Some(t) = TEX.with(|c| c.borrow().get(&key).cloned()) {
        return t;
    }
    let (asc, desc) = face.vertical_metrics();
    let em = (asc + desc).max(1.0);
    let s = px as f64 * 0.8 / em;
    let adv = face.advance(gid);
    let mut path = (*FontDb::global().outline(face, gid)).clone();
    // Font units (y down, baseline at 0) → pixels, centred on the advance.
    let ox = (px as f64 - adv * s) * 0.5;
    let oy = px as f64 * 0.1 + asc * s;
    path.apply_affine(Affine::translate((ox, oy)) * Affine::scale(s));
    let mask = rasterize(&path, px as usize, px as usize);
    let img = egui::ColorImage::new([px as usize, px as usize], mask.iter().map(|&a| Color32::from_rgba_premultiplied(a, a, a, a)).collect());
    let tex = ctx.load_texture(format!("glyph-{}-{gid}-{px}", face.id()), img, egui::TextureOptions::LINEAR);
    TEX.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() > 4000 {
            c.clear();
        }
        c.insert(key, tex.clone());
    });
    tex
}

/// Insert `ch` at the Type tool's caret, else append it to the selected text.
fn insert(app: &mut VectorcraftApp, ch: char) {
    let s = ch.to_string();
    if app.session.tool_wants_text() {
        let v = app.view_info();
        if let Err(e) = app.session.tool_text(&s, v) {
            app.status(e.to_string());
        }
        app.sync_views();
    } else if super::character::text_style(app).is_some() {
        app.run("type.insert", json!({"text": s})).ok();
    } else {
        app.ui.status = "Select text or click into text with the Type tool to insert glyphs".into();
    }
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let db = FontDb::global();
    let current = super::character::text_style(app).map(|(s, _)| (s.font_family, s.font_style));
    // The panel browses the current text's font unless another font was picked here.
    let picked: Option<(String, String)> = pstate(ui.ctx(), "gl-font");
    let (family, style) = picked.or(current).unwrap_or_else(|| (vectorcraft_text::FALLBACK_FAMILY.to_string(), "Regular".to_string()));
    let Some(face) = db.face(&family, &style) else {
        ui.label(egui::RichText::new(tl!("No fonts are available.")).color(t.text_dim));
        return;
    };
    let subset: usize = pstate(ui.ctx(), "gl-subset");
    let (_, lo, hi) = SUBSETS[subset.min(SUBSETS.len() - 1)];
    let w = ui.available_width();
    let names: Vec<&str> = SUBSETS.iter().map(|s| s.0).collect();
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("Show:")).color(t.text_dim));
        if let Some(i) = widgets::dropdown(ui, "gl-subset", names[subset.min(names.len() - 1)], &names, (w - 50.0).max(80.0)) {
            set_pstate(ui.ctx(), "gl-subset", i);
        }
    });
    let chars: Vec<(char, u32)> =
        face.chars().into_iter().filter(|(c, _)| (lo..=hi).contains(&(*c as u32)) && !c.is_control() && !c.is_whitespace()).collect();
    let cols = ((w / CELL).floor() as usize).max(1);
    let rows = chars.len().div_ceil(cols);
    let px = (CELL * ui.ctx().pixels_per_point()).round() as u32;
    let list_h = (ui.available_height() - 64.0).max(CELL * 3.0);
    let hover_id = egui::Id::new("gl-hover");
    egui::ScrollArea::vertical().max_height(list_h).auto_shrink([false, false]).show_rows(ui, CELL, rows, |ui, range| {
        for r in range {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                for &(c, gid) in chars.iter().skip(r * cols).take(cols) {
                    let (rect, resp) = ui.allocate_exact_size(vec2(CELL, CELL), Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(rect, 2, t.hover);
                        ui.ctx().data_mut(|d| d.insert_temp(hover_id, (c, gid)));
                    }
                    ui.painter().rect_stroke(rect.shrink(0.5), 0, egui::Stroke::new(0.5, t.border), egui::StrokeKind::Inside);
                    let tex = glyph_texture(ui.ctx(), &face, gid, px);
                    ui.painter().image(tex.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), t.text_strong);
                    let resp = resp.on_hover_text(format!("U+{:04X}  {c}  (glyph {gid})\n{}", c as u32, tl!("Double-click to insert")));
                    if resp.double_clicked() {
                        insert(app, c);
                    }
                }
            });
        }
    });
    ui.add_space(4.0);
    let info = ui.ctx().data(|d| d.get_temp::<(char, u32)>(hover_id));
    ui.label(
        egui::RichText::new(match info {
            Some((c, g)) => format!(
                "U+{:04X}  {}  ·  {}",
                c as u32,
                crate::i18n::fmt(tl!("glyph {id}"), &[("id", &g.to_string())]),
                crate::i18n::tn(chars.len() as u64, "{n} glyph", "{n} glyphs")
            ),
            None => crate::i18n::tn(chars.len() as u64, "{n} glyph", "{n} glyphs"),
        })
        .size(11.0)
        .color(t.text_dim),
    );
    ui.horizontal(|ui| {
        // Picks the font the panel browses only: nothing is previewed on the document.
        let sample = crate::font_menu::sample_text(app);
        let look = crate::font_menu::MenuLook::of(app);
        let pick = crate::font_menu::font_menu(ui, "gl-family", &family, (w * 0.6).max(80.0), sample.as_deref(), look);
        if let Some((f, style)) = crate::font_menu::picked(app, pick) {
            let st = style.unwrap_or_else(|| db.face(&f, "Regular").map_or_else(|| "Regular".into(), |face| face.style.clone()));
            set_pstate(ui.ctx(), "gl-font", Some((f, st)));
        }
        let styles = db.styles(&family);
        let snames: Vec<&str> = styles.iter().map(String::as_str).collect();
        if let Some(i) = widgets::dropdown_names(ui, "gl-style", &style, &snames, (w * 0.38 - 8.0).max(60.0)) {
            set_pstate(ui.ctx(), "gl-font", Some((family.clone(), snames[i].to_string())));
        }
    });
}

pub fn menu(_app: &mut VectorcraftApp, ui: &mut Ui) {
    if menu_item(ui, tl!("Use the Selected Text's Font"), true, false) {
        set_pstate::<Option<(String, String)>>(ui.ctx(), "gl-font", None);
    }
    ui.separator();
    for (i, (name, _, _)) in SUBSETS.iter().enumerate() {
        let cur: usize = pstate(ui.ctx(), "gl-subset");
        if menu_item(ui, tl!(name), true, cur == i) {
            set_pstate(ui.ctx(), "gl-subset", i);
        }
    }
}
