//! Navigator panel: a thumbnail of the document (rendered with `Renderer::render_region`, cached by
//! document revision) with the red view box you drag to pan, zoom out/in buttons, a zoom slider
//! and a zoom field.

use egui::{Color32, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use serde_json::json;
use vectorcraft_geom::{Point, Rect as DRect};

use super::{pstate, set_pstate};
use crate::VectorcraftApp;
use crate::theme::Tokens;
use crate::widgets::{self, menu_item};

#[derive(Default)]
struct Cache {
    renderer: Option<vectorcraft_render::Renderer>,
    key: Option<(usize, u64, u32, u32, bool)>,
    tex: Option<egui::TextureHandle>,
}

thread_local! {
    static CACHE: crate::graphics::TexCache<Cache> = crate::graphics::TexCache::default();
}

/// Map a document point into the thumbnail rect (region → rect, aspect preserved).
pub fn doc_to_thumb(region: DRect, thumb: Rect, p: Point) -> egui::Pos2 {
    let s = (thumb.width() as f64 / region.width()).min(thumb.height() as f64 / region.height());
    let ox = thumb.left() as f64 + (thumb.width() as f64 - region.width() * s) / 2.0;
    let oy = thumb.top() as f64 + (thumb.height() as f64 - region.height() * s) / 2.0;
    pos2((ox + (p.x - region.x0) * s) as f32, (oy + (p.y - region.y0) * s) as f32)
}

/// Inverse of [`doc_to_thumb`].
pub fn thumb_to_doc(region: DRect, thumb: Rect, p: egui::Pos2) -> Point {
    let s = (thumb.width() as f64 / region.width()).min(thumb.height() as f64 / region.height());
    let ox = thumb.left() as f64 + (thumb.width() as f64 - region.width() * s) / 2.0;
    let oy = thumb.top() as f64 + (thumb.height() as f64 - region.height() * s) / 2.0;
    Point::new(region.x0 + (p.x as f64 - ox) / s, region.y0 + (p.y as f64 - oy) / s)
}

/// Slider position (0..1) ↔ zoom percent on Illustrator's log range 3.13 %–64 000 %.
pub fn zoom_to_slider(z: f64) -> f64 {
    ((z.clamp(3.13, 64000.0)).ln() - 3.13f64.ln()) / (64000f64.ln() - 3.13f64.ln())
}
pub fn slider_to_zoom(t: f64) -> f64 {
    (3.13f64.ln() + t.clamp(0.0, 1.0) * (64000f64.ln() - 3.13f64.ln())).exp()
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(st) = app.session.active() else {
        super::empty_state(ui, "map", tl!("No document"), tl!("Open a document to navigate it."));
        return;
    };
    let artboard_only: bool = pstate(ui.ctx(), "nav-ab-only");
    let doc = st.doc.clone();
    let rev = st.revision;
    let doc_idx = app.session.documents().iter().position(|d| std::sync::Arc::ptr_eq(&d.doc, &doc)).unwrap_or(0);
    let mut region = doc.artboards.iter().map(|a| a.rect).reduce(|a, b| a.union(b)).unwrap_or(DRect::new(0.0, 0.0, 612.0, 792.0));
    if !artboard_only && let Some(b) = doc.art_bounds() {
        region = region.union(b);
    }
    let region = region.inflate(region.width() * 0.04 + 4.0, region.height() * 0.04 + 4.0);
    let w = ui.available_width();
    let h = (w * 0.72).clamp(100.0, 200.0);
    let (thumb, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click_and_drag());
    ui.painter().rect_filled(thumb, 0.0, t.pasteboard);
    let ppp = ui.ctx().pixels_per_point();
    let (pw, ph) = ((w * ppp) as u32, (h * ppp) as u32);
    let key = (doc_idx, rev, pw, ph, artboard_only);
    let tex = CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.key != Some(key) || c.tex.is_none() {
            let s = (pw as f64 / region.width()).min(ph as f64 / region.height());
            let r = c.renderer.get_or_insert_with(vectorcraft_render::Renderer::new).render_region(&doc, region, s, false);
            let img = egui::ColorImage::from_rgba_premultiplied([r.width as usize, r.height as usize], &r.pixels);
            match &mut c.tex {
                Some(tx) => tx.set(img, egui::TextureOptions::LINEAR),
                None => c.tex = Some(ui.ctx().load_texture("navigator", img, egui::TextureOptions::LINEAR)),
            }
            c.key = Some(key);
        }
        c.tex.clone()
    });
    // Artboards (white) under the art thumbnail, like the canvas.
    for a in &doc.artboards {
        let r = Rect::from_min_max(doc_to_thumb(region, thumb, a.rect.origin()), doc_to_thumb(region, thumb, Point::new(a.rect.x1, a.rect.y1)));
        ui.painter().rect_filled(r, 0.0, Color32::WHITE);
    }
    if let Some(tex) = tex {
        let tl = doc_to_thumb(region, thumb, region.origin());
        let br = doc_to_thumb(region, thumb, Point::new(region.x1, region.y1));
        ui.painter().image(tex.id(), Rect::from_min_max(tl, br), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }
    // The view box.
    let view = app.view().copied();
    if let (Some(v), Some(cr)) = (view, app.canvas_rect) {
        let hw = cr.width() as f64 / 2.0 / v.zoom;
        let hh = cr.height() as f64 / 2.0 / v.zoom;
        let a = doc_to_thumb(region, thumb, Point::new(v.center.x - hw, v.center.y - hh));
        let b = doc_to_thumb(region, thumb, Point::new(v.center.x + hw, v.center.y + hh));
        let vr = Rect::from_min_max(a, b).intersect(thumb.expand(1.0));
        ui.painter().with_clip_rect(thumb).rect_stroke(vr, 0.0, Stroke::new(1.5, Color32::from_rgb(0xff, 0x30, 0x30)), StrokeKind::Middle);
        if (resp.dragged() || resp.clicked())
            && let Some(p) = resp.interact_pointer_pos()
        {
            let c = thumb_to_doc(region, thumb, p);
            app.run("view.setZoom", json!({"zoom": v.zoom * 100.0, "center": [c.x, c.y]})).ok();
        }
    }
    ui.painter().rect_stroke(thumb, 0.0, Stroke::new(1.0, t.border), StrokeKind::Outside);
    // Zoom controls.
    let zoom = view.map(|v| v.zoom * 100.0).unwrap_or(100.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let mut z = None;
        if let Some(v) = widgets::plain_field(ui, "nav-zoom", zoom, "%", 2, 64.0) {
            z = Some(v);
        }
        if widgets::icon_button(ui, "dc-zoom-small", tl!("Zoom Out"), false, 22.0).clicked() {
            app.run("view.zoomOut", json!({})).ok();
        }
        let sw = (ui.available_width() - 30.0).max(40.0);
        let track = |_: f32| t.input_border;
        if let (Some(v), phase) = widgets::color_slider(ui, "nav-slider", zoom_to_slider(zoom) as f32, sw, &track)
            && phase != widgets::Live::Idle
        {
            z = Some(slider_to_zoom(v as f64));
        }
        if widgets::icon_button(ui, "dc-zoom-large", tl!("Zoom In"), false, 22.0).clicked() {
            app.run("view.zoomIn", json!({})).ok();
        }
        if let Some(z) = z {
            app.run("view.setZoom", json!({"zoom": z.clamp(3.13, 64000.0)})).ok();
        }
    });
}

pub fn menu(_app: &mut VectorcraftApp, ui: &mut Ui) {
    let ab: bool = pstate(ui.ctx(), "nav-ab-only");
    if menu_item(ui, tl!("View Artboard Only"), true, ab) {
        set_pstate(ui.ctx(), "nav-ab-only", !ab);
    }
    menu_item(ui, tl!("Panel Options…"), false, false);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumb_mapping_roundtrip() {
        let region = DRect::new(0.0, 0.0, 200.0, 100.0);
        let thumb = Rect::from_min_size(pos2(10.0, 10.0), vec2(100.0, 100.0));
        let p = doc_to_thumb(region, thumb, Point::new(100.0, 50.0));
        assert!((p.x - 60.0).abs() < 1e-3 && (p.y - 60.0).abs() < 1e-3);
        let q = thumb_to_doc(region, thumb, p);
        assert!((q.x - 100.0).abs() < 1e-6 && (q.y - 50.0).abs() < 1e-6);
    }

    #[test]
    fn zoom_slider_roundtrip() {
        assert!((zoom_to_slider(3.13)).abs() < 1e-9);
        assert!((zoom_to_slider(64000.0) - 1.0).abs() < 1e-9);
        assert!((slider_to_zoom(zoom_to_slider(100.0)) - 100.0).abs() < 1e-6);
    }
}
