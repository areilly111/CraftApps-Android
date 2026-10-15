//! View extras: rulers (⌘R), grid (⌘'), guides (⌘;). Guides are dragged out of the rulers and
//! moved with the Move tool (drag one off the canvas to delete it), all through undoable engine
//! commands (`view.newGuide` / `view.moveGuide` / `view.deleteGuide`).

use egui::{Align2, Color32, Pos2, Rect, Sense, Stroke, pos2, vec2};
use photocraft_doc::Document;
use photocraft_engine::prefs::{GuidesGridAndSlices, Unit};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::ViewXform;
use crate::theme::Tokens;

pub const RULER: f32 = 16.0;

/// A `#rrggbb` preference colour.
fn pref_color(s: &str, fallback: Color32) -> Color32 {
    photocraft_engine::prefs::parse_hex(s).map_or(fallback, |c| Color32::from_rgb(c[0], c[1], c[2]))
}

/// Draw a line in a Guides/Grid preference style (lines, dashed lines, dots).
fn styled_line(painter: &egui::Painter, a: Pos2, b: Pos2, stroke: Stroke, style: photocraft_engine::prefs::LineStyle) {
    use photocraft_engine::prefs::LineStyle;
    match style {
        LineStyle::Lines => {
            painter.line_segment([a, b], stroke);
        }
        LineStyle::Dashed => {
            painter.extend(egui::Shape::dashed_line(&[a, b], stroke, 4.0, 3.0));
        }
        LineStyle::Dots => {
            painter.extend(egui::Shape::dotted_line(&[a, b], stroke.color, 3.0, 0.6));
        }
    }
}

/// Ruler step in units so labelled ticks are at least `min` units apart: …0.1, 0.25, 0.5, 1, 2, 5,
/// 10… (whole steps only for pixels).
fn nice_step(min: f64, whole: bool) -> f64 {
    let mut step = if whole { 1.0 } else { 0.001 };
    loop {
        for m in [1.0, 2.0, 2.5, 5.0] {
            if (m != 2.5 || !whole) && step * m >= min {
                return step * m;
            }
        }
        step *= 10.0;
    }
}

/// English display name for a length unit, as shown in Preferences › Units & Rulers.
fn unit_label(u: Unit) -> &'static str {
    match u {
        Unit::Pixels => "Pixels",
        Unit::Inches => "Inches",
        Unit::Centimeters => "Centimeters",
        Unit::Millimeters => "Millimeters",
        Unit::Points => "Points",
        Unit::Picas => "Picas",
        Unit::Percent => "Percent",
    }
}

/// Every length unit, in the Preferences › Units & Rulers order.
const UNITS: [Unit; 7] = [Unit::Pixels, Unit::Inches, Unit::Centimeters, Unit::Millimeters, Unit::Points, Unit::Picas, Unit::Percent];

/// The ruler right-click menu: every length unit with its (translated) label and whether it's the
/// one in effect. Picking one changes the ruler unit exactly as the preference pane does.
fn unit_menu_items(current: Unit) -> Vec<(String, &'static str, bool)> {
    UNITS.iter().map(|&u| (tl!(unit_label(u)).to_string(), u.name(), u == current)).collect()
}

/// The length units menu that right-clicking a ruler (or Free Transform's W and H fields) opens;
/// the chosen unit's name is returned (`None` if the menu is dismissed without a pick).
pub(crate) fn unit_menu(ui: &mut egui::Ui, current: Unit) -> Option<&'static str> {
    crate::widgets::menu_scroll(ui, |ui| {
        ui.set_min_width(150.0);
        let mut chosen = None;
        for (label, name, checked) in unit_menu_items(current) {
            let label = if checked { format!("✓ {label}") } else { label };
            if ui.button(label).clicked() {
                ui.close();
                chosen = Some(name);
            }
        }
        chosen
    })
}

/// A guide being dragged: from a ruler (new) or an existing one (index).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GuideDrag {
    pub vertical: bool,
    pub index: Option<usize>,
    pub pos: f64,
}

/// Canvas area left after the rulers.
pub fn content_rect(app: &PhotocraftApp, rect: Rect) -> Rect {
    if app.ui.extras.rulers { Rect::from_min_max(rect.min + vec2(RULER, RULER), rect.max) } else { rect }
}

/// Tick step (document px) so major ticks are at least ~60 screen px apart: 1, 2, 5, 10, 20, 50…
#[cfg(test)]
fn tick_step(zoom: f32) -> f64 {
    nice_step(60.0 / zoom.max(1e-4) as f64, true)
}

/// Grid spacing in document pixels. Percentages use each axis's own extent.
pub(crate) fn grid_major_px(g: &GuidesGridAndSlices, dpi: f64, size: [f64; 2], ppi: f64) -> [f64; 2] {
    size.map(|extent| g.major_px(dpi, extent, ppi))
}

/// View › Show › Grid, from Preferences › Guides, Grid & Slices (spacing, subdivisions, colour,
/// style). Photoshop's default is a gridline every inch with 4 subdivisions.
pub fn draw_grid(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform, doc: &Document) {
    let g = &app.session.prefs().guides_grid_and_slices;
    let ppi = app.session.prefs().units_and_rulers.point_size.per_inch();
    let (w, h) = (doc.size.width as f64, doc.size.height as f64);
    let major = grid_major_px(g, doc.resolution_dpi.max(1.0) as f64, [w, h], ppi);
    let minor = major.map(|step| step / g.subdivisions.max(1) as f64);
    let base = pref_color(&g.grid_color, Color32::from_gray(140));
    for (steps, alpha) in [(minor, 0.45f32), (major, 1.0)] {
        if alpha < 1.0 && g.subdivisions <= 1 {
            continue;
        }
        let st = Stroke::new(1.0, base.gamma_multiply(alpha));
        let [step_x, step_y] = steps;
        if step_x * (xf.zoom_x() as f64) >= 6.0 {
            let mut x = step_x;
            while x < w {
                styled_line(painter, xf.to_screen(x as f32, 0.0), xf.to_screen(x as f32, h as f32), st, g.grid_style);
                x += step_x;
            }
        }
        if step_y * (xf.zoom as f64) >= 6.0 {
            let mut y = step_y;
            while y < h {
                styled_line(painter, xf.to_screen(0.0, y as f32), xf.to_screen(w as f32, y as f32), st, g.grid_style);
                y += step_y;
            }
        }
    }
}

/// Screen end points of a guide across `clip`: the document line x = `pos` (vertical) or
/// y = `pos`, spanning every document point the clip rect shows. Guides belong to the document,
/// so on a rotated view they turn with the image (#3277); unrotated this is the clip's full
/// height (or width).
fn guide_segment(xf: &ViewXform, clip: Rect, vertical: bool, pos: f64) -> [Pos2; 2] {
    let axis = usize::from(vertical);
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for c in [clip.left_top(), clip.right_top(), clip.right_bottom(), clip.left_bottom()] {
        let d = xf.to_doc(c)[axis];
        lo = lo.min(d);
        hi = hi.max(d);
    }
    let (pos, lo, hi) = (pos as f32, lo as f32, hi as f32);
    if vertical { [xf.to_screen(pos, lo), xf.to_screen(pos, hi)] } else { [xf.to_screen(lo, pos), xf.to_screen(hi, pos)] }
}

pub fn draw_guides(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform, doc: &Document) {
    let clip = painter.clip_rect();
    let drag = app.guide_drag;
    let g = &app.session.prefs().guides_grid_and_slices;
    let preview = crate::guide_layout_ui::preview(app, doc);
    let guides = preview.as_ref().map(|(guides, _)| guides).unwrap_or(&doc.guides);
    let (guide, style) =
        (preview.as_ref().map(|(_, color)| *color).unwrap_or_else(|| pref_color(&g.guide_color, Color32::from_rgb(74, 255, 255))), g.guide_style);
    let line = |vertical: bool, pos: f64, color: Color32| {
        let [a, b] = guide_segment(xf, clip, vertical, pos);
        styled_line(painter, a, b, Stroke::new(1.0, color), style);
    };
    // A guide given a colour in New Guide keeps it; the rest use the preference colour (or, while
    // New Guide Layout previews, the colour it will set).
    let own = |vertical: bool, p: f64| guides.color(vertical, p).map_or(guide, |[r, g, b]| Color32::from_rgb(r, g, b));
    if app.ui.extras.guides || preview.is_some() {
        for (vertical, list) in [(true, &guides.vertical), (false, &guides.horizontal)] {
            for (i, p) in list.iter().enumerate() {
                if drag.is_some_and(|d| d.vertical == vertical && d.index == Some(i)) {
                    continue;
                }
                line(vertical, *p, own(vertical, *p));
            }
        }
    }
    if let Some(d) = drag {
        // A guide being moved shows its own colour; a new one dragged from a ruler the preference's.
        let list = if d.vertical { &doc.guides.vertical } else { &doc.guides.horizontal };
        line(d.vertical, d.pos, d.index.and_then(|i| list.get(i)).map_or(guide, |p| own(d.vertical, *p)));
    }
}

/// Existing guide under a document point (within 4 screen px).
pub fn guide_at(app: &PhotocraftApp, x: f64, y: f64) -> Option<(bool, usize)> {
    // Hidden canvas guides are not interactive: the Move tool must reach the layer below.
    // Keep hit testing consistent with the visibility gate in canvas::draw.
    if !app.ui.extras.guides || !app.ui.view.shows(app.ui.view.show.canvas_guides) || app.ui.extras.lock_guides {
        return None;
    }
    let doc = &app.session.active()?.doc;
    let tol = 4.0 / app.point_zoom().max(0.01) as f64;
    for (i, g) in doc.guides.vertical.iter().enumerate() {
        if (*g - x).abs() <= tol {
            return Some((true, i));
        }
    }
    for (i, g) in doc.guides.horizontal.iter().enumerate() {
        if (*g - y).abs() <= tol {
            return Some((false, i));
        }
    }
    None
}

/// Finish a guide drag: create, move, or delete (dropped outside the canvas).
pub fn finish_drag(app: &mut PhotocraftApp, d: GuideDrag) {
    let Some(size) = app.session.active().map(|s| s.doc.size) else { return };
    let extent = if d.vertical { size.width } else { size.height } as f64;
    let orientation = if d.vertical { "vertical" } else { "horizontal" };
    let inside = d.pos >= 0.0 && d.pos <= extent;
    let pos = (d.pos * 1000.0).round() / 1000.0;
    let _ = match (d.index, inside) {
        (None, true) => app.run("view.newGuide", json!({"orientation": orientation, "position": pos})),
        (Some(i), true) => app.run("view.moveGuide", json!({"orientation": orientation, "index": i, "position": pos})),
        (Some(i), false) => app.run("view.deleteGuide", json!({"orientation": orientation, "index": i})),
        (None, false) => Ok(serde_json::Value::Null),
    };
    if d.index.is_none() && inside {
        app.ui.extras.guides = true;
    }
}

/// The mapping the rulers measure with. Rulers are screen chrome that stays level while
/// View › Rotate View turns the canvas, so they use the view without its rotation: the same zoom,
/// flip and pixel aspect (ticks never stretch), anchored so that 0 sits at the screen position of
/// the document origin as the rotated view draws it, as in Photoshop. Tilted guides then meet the
/// rulers near their numbers. Measuring along the rotated axes instead squeezed the ticks into a
/// few pixels (or none at 90°) and left the rest of the ruler empty (#3233).
fn ruler_xform(xf: &ViewXform) -> ViewXform {
    let level = ViewXform { rotation: 0.0, ..*xf };
    // The level view is an axis-aligned scale, so moving the origin by `shift` on screen moves
    // every point by it: re-centre on the document point that sat `shift` before the centre.
    let shift = xf.to_screen(0.0, 0.0) - level.to_screen(0.0, 0.0);
    let center = level.to_doc(xf.rect.center() - shift);
    if !(center[0].is_finite() && center[1].is_finite()) {
        return level;
    }
    ViewXform { center: [center[0] as f32, center[1] as f32], ..level }
}

/// Visible ruler range in ruler units, normalised to `(min, max)` so the tick loop runs for
/// flipped views too (#1814). `xf` is the canvas mapping; rotation is ignored (see
/// [`ruler_xform`]). Shared by the renderer and its tests.
fn ruler_range(xf: &ViewXform, a: Pos2, b: Pos2, axis: usize, px_per_unit: f64) -> (f64, f64) {
    let rxf = ruler_xform(xf);
    let d0 = rxf.to_doc(a)[axis];
    let d1 = rxf.to_doc(b)[axis];
    let (u0, u1) = (d0 / px_per_unit, d1 / px_per_unit);
    (u0.min(u1), u0.max(u1))
}

/// Screen coordinate along the ruler (x for the top ruler, y for the left one) of the value `u`
/// in ruler units. The inverse of [`ruler_range`]'s mapping.
fn ruler_pos(xf: &ViewXform, vertical: bool, u: f64, px_per_unit: f64) -> f32 {
    let px = (u * px_per_unit) as f32;
    let rxf = ruler_xform(xf);
    if vertical { rxf.to_screen(0.0, px).y } else { rxf.to_screen(px, 0.0).x }
}

/// Document pixels per ruler unit and the labelled tick step (in ruler units) of the top ruler
/// (`y_axis` false) or the left one, from Preferences › Units & Rulers and the zoom. Minor ticks
/// sit at tenths of the step.
fn ruler_scale(app: &PhotocraftApp, xf: &ViewXform, y_axis: bool) -> (f64, f64) {
    let ur = &app.session.prefs().units_and_rulers;
    let (unit, ppi) = (ur.rulers, ur.point_size.per_inch());
    let (dpi, size) =
        app.session.active().map_or((72.0, [1.0, 1.0]), |d| (d.doc.resolution_dpi.max(1.0) as f64, [d.doc.size.width as f64, d.doc.size.height as f64]));
    let px_per_unit = unit.to_px(1.0, dpi, if y_axis { size[1] } else { size[0] }, ppi).max(1e-9);
    // The top ruler runs along document x, which pixel aspect correction stretches.
    let zoom = if y_axis { xf.zoom } else { xf.zoom_x() };
    (px_per_unit, nice_step(60.0 / (zoom as f64 * px_per_unit).max(1e-6), unit == Unit::Pixels))
}

/// `pos` (document px) moved to the nearest tick mark the ruler draws: where a guide lands when
/// Shift is held while dragging it out of a ruler, as in Photoshop (#3770).
fn snap_to_tick(app: &PhotocraftApp, xf: &ViewXform, vertical: bool, pos: f64) -> f64 {
    // A vertical guide sits at an x position, which the top ruler measures.
    let (px_per_unit, step) = ruler_scale(app, xf, !vertical);
    let tick = step / 10.0 * px_per_unit;
    if tick.is_finite() && tick > 0.0 { (pos / tick).round() * tick } else { pos }
}

/// The number a ruler prints at the tick `v` (in ruler units). Like Photoshop, rulers show the
/// distance from the origin, so values left of / above it read without a minus sign.
fn ruler_label(v: f64, step: f64, whole: bool) -> String {
    let v = v.abs();
    if step >= 1.0 || whole { format!("{}", v.round() as i64) } else { crate::widgets::fmt_num2(v) }
}

/// Rulers along the top and left of `full` (the canvas rect before `content_rect`), with the
/// pointer position marked; dragging out of a ruler creates a guide.
pub fn draw_rulers(app: &mut PhotocraftApp, ui: &mut egui::Ui, full: Rect, xf: &ViewXform) {
    let t = Tokens::get(ui.ctx());
    let top = Rect::from_min_max(pos2(full.left() + RULER, full.top()), pos2(full.right(), full.top() + RULER));
    let left = Rect::from_min_max(pos2(full.left(), full.top() + RULER), pos2(full.left() + RULER, full.bottom()));
    let corner = Rect::from_min_size(full.min, vec2(RULER, RULER));
    let p = ui.painter_at(full);
    let bg = t.chrome;
    for r in [top, left, corner] {
        p.rect_filled(r, 0.0, bg);
    }
    p.line_segment([top.left_bottom(), top.right_bottom()], Stroke::new(1.0, t.separator));
    p.line_segment([left.right_top(), left.right_bottom()], Stroke::new(1.0, t.separator));
    let font = egui::FontId::proportional(9.0);
    let tick = Stroke::new(1.0, t.text_faint);
    // Preferences › Units & Rulers: labels in the ruler unit (percent of the document's side).
    let unit = app.session.prefs().units_and_rulers.rulers;
    let whole = unit == Unit::Pixels;
    for vertical in [false, true] {
        let p = p.with_clip_rect(if vertical { left } else { top });
        let (px_per_unit, step) = ruler_scale(app, xf, vertical);
        let (u0, u1) = if vertical {
            ruler_range(xf, left.left_top(), left.left_bottom(), 1, px_per_unit)
        } else {
            ruler_range(xf, top.left_top(), top.right_top(), 0, px_per_unit)
        };
        let mut v = (u0 / step).floor() * step;
        while v <= u1 {
            let at = |u: f64| -> f32 { ruler_pos(xf, vertical, u, px_per_unit) };
            let c = at(v);
            if vertical {
                p.line_segment([pos2(left.right() - RULER, c), pos2(left.right(), c)], tick);
                // Labels stacked, as in Photoshop.
                for (i, ch) in ruler_label(v, step, whole).chars().enumerate() {
                    p.text(pos2(left.left() + 3.0, c + 2.0 + i as f32 * 8.5), Align2::LEFT_TOP, ch, font.clone(), t.text_dim);
                }
            } else {
                p.line_segment([pos2(c, top.bottom() - RULER), pos2(c, top.bottom())], tick);
                p.text(pos2(c + 2.0, top.top() + 1.0), Align2::LEFT_TOP, ruler_label(v, step, whole), font.clone(), t.text_dim);
            }
            for k in 1..10 {
                let m = at(v + step * k as f64 / 10.0);
                let len = if k == 5 { 6.0 } else { 3.0 };
                if vertical {
                    p.line_segment([pos2(left.right() - len, m), pos2(left.right(), m)], tick);
                } else {
                    p.line_segment([pos2(m, top.bottom() - len), pos2(m, top.bottom())], tick);
                }
            }
            v += step;
        }
    }
    // Pointer position markers.
    if let Some(h) = ui.ctx().pointer_hover_pos().filter(|h| full.contains(*h)) {
        let m = Stroke::new(1.0, t.text);
        p.with_clip_rect(top).line_segment([pos2(h.x, top.top()), pos2(h.x, top.bottom())], m);
        p.with_clip_rect(left).line_segment([pos2(left.left(), h.y), pos2(left.right(), h.y)], m);
    }
    // Drag a new guide out of a ruler; right-click to switch the ruler unit.
    let mut chosen_unit: Option<&'static str> = None;
    for (r, vertical, salt) in [(top, false, "ruler-top"), (left, true, "ruler-left"), (corner, false, "ruler-corner")] {
        // Guides drag out of the top/left rulers; all three sense clicks so the right-click unit
        // menu opens anywhere on the ruler (the corner only needs the menu).
        let sense = if salt == "ruler-corner" { Sense::click() } else { Sense::click_and_drag() };
        let resp = ui.interact(r, ui.id().with(salt), sense);
        if resp.hovered() && salt != "ruler-corner" {
            ui.ctx().set_cursor_icon(if vertical { egui::CursorIcon::ResizeHorizontal } else { egui::CursorIcon::ResizeVertical });
        }
        if let Some(pos) = resp.interact_pointer_pos().filter(|_| resp.dragged() || resp.drag_started()) {
            let d = xf.to_doc(pos);
            let along = if vertical { d[0] } else { d[1] };
            let pos = if ui.input(|i| i.modifiers.shift) { snap_to_tick(app, xf, vertical, along) } else { crate::snap_ui::snap_guide(app, vertical, along) };
            app.guide_drag = Some(GuideDrag { vertical, index: None, pos });
        }
        if resp.drag_stopped()
            && let Some(d) = app.guide_drag.take()
        {
            finish_drag(app, d);
        }
        resp.context_menu(|ui| {
            if let Some(name) = unit_menu(ui, unit) {
                chosen_unit = Some(name);
            }
        });
    }
    if let Some(name) = chosen_unit {
        let _ = app.run("prefs.set", json!({"path": "unitsAndRulers.rulers", "value": name}));
    }
    let _ = Pos2::ZERO;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ruler_ticks_labels_and_pointer_stay_out_of_the_origin_corner() {
        let mut failures = Vec::new();
        for theme in crate::theme::ThemeKind::ALL {
            for (zoom, flip, rotation, width) in [(1.0, false, 0.0, 560.0), (1.0, false, 0.0, 800.0), (0.25, true, 0.0, 560.0), (2.0, false, 30.0, 560.0)] {
                let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
                app.run("file.new", json!({"width": 256, "height": 128})).unwrap();
                let full = Rect::from_min_size(pos2(32.0, 32.0), vec2(width - 80.0, 320.0));
                let corner = Rect::from_min_size(full.min, vec2(RULER, RULER));
                let builder = egui_kittest::Harness::builder().with_size(vec2(width, 400.0));
                let mut h = builder.build_ui_state(
                    move |ui, app| {
                        let rect = Rect::from_min_max(full.min + vec2(RULER, RULER), full.max);
                        let xf = ViewXform { rect, zoom, center: [(width - 80.0) / 2.0, 160.0], flip, rotation, aspect: 1.0 };
                        draw_rulers(app, ui, full, &xf);
                    },
                    app,
                );
                PhotocraftApp::setup_context(&h.ctx, theme);
                h.event(egui::Event::PointerMoved(corner.center()));
                h.run_steps(4);
                let mut ticks = 0;
                let mut labels = 0;
                for shape in &h.output().shapes {
                    if matches!(shape.shape, egui::Shape::LineSegment { .. } | egui::Shape::Text(_)) {
                        ticks += usize::from(matches!(shape.shape, egui::Shape::LineSegment { .. }));
                        labels += usize::from(matches!(shape.shape, egui::Shape::Text(_)));
                        let visible = shape.shape.visual_bounding_rect().intersect(shape.clip_rect);
                        if visible.intersect(corner.shrink(0.5)).is_positive() {
                            failures.push(format!("{theme:?}: zoom={zoom}, flip={flip}, rotation={rotation}, {visible:?}"));
                        }
                    }
                }
                assert!(ticks > 2 && labels > 0, "rulers still draw ticks and labels");
            }
        }
        assert!(failures.is_empty(), "ruler drawing overlaps the origin corner: {failures:?}");
    }

    #[test]
    fn percentage_grid_spacing_uses_both_canvas_dimensions() {
        let mut g = GuidesGridAndSlices { grid_unit: Unit::Percent, gridline_every: 20.0, subdivisions: 2, ..Default::default() };
        assert_eq!(grid_major_px(&g, 72.0, [1280.0, 720.0], 72.0), [256.0, 144.0]);
        // Absolute units remain equal on both axes, irrespective of aspect ratio.
        g.grid_unit = Unit::Pixels;
        g.gridline_every = 100.0;
        assert_eq!(grid_major_px(&g, 72.0, [1280.0, 720.0], 72.0), [100.0, 100.0]);
    }

    #[test]
    fn unit_steps_allow_fractions() {
        assert_eq!(nice_step(0.3, false), 0.5);
        assert_eq!(nice_step(0.2, false), 0.2);
        assert_eq!(nice_step(0.22, false), 0.25);
        assert_eq!(nice_step(0.3, true), 1.0);
        assert_eq!(nice_step(23.0, true), 50.0);
    }

    #[test]
    fn ticks_follow_1_2_5_series() {
        assert_eq!(tick_step(1.0), 100.0);
        assert_eq!(tick_step(0.1), 1000.0);
        assert_eq!(tick_step(4.0), 20.0);
        assert_eq!(tick_step(64.0), 1.0);
        assert_eq!(tick_step(0.25), 500.0);
    }

    #[test]
    fn hidden_guides_are_not_hit_tested() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
        app.sync_views();
        app.session.execute("view.newGuide", json!({"orientation": "vertical", "position": 50})).unwrap();
        app.session.execute("view.newGuide", json!({"orientation": "horizontal", "position": 25})).unwrap();
        let positions = app.session.active().unwrap().doc.guides.clone();

        // Visible, unlocked guides remain interactive in either orientation.
        assert_eq!(guide_at(&app, 50.0, 10.0), Some((true, 0)));
        assert_eq!(guide_at(&app, 10.0, 25.0), Some((false, 0)));

        // All ways to hide the canvas guides must also disable hit testing.
        app.ui.extras.guides = false;
        assert_eq!(guide_at(&app, 50.0, 10.0), None);
        assert_eq!(guide_at(&app, 10.0, 25.0), None);
        app.ui.extras.guides = true;

        app.ui.view.show.canvas_guides = false;
        assert_eq!(guide_at(&app, 50.0, 10.0), None);
        assert_eq!(guide_at(&app, 10.0, 25.0), None);
        app.ui.view.show.canvas_guides = true;

        app.ui.view.extras = false;
        assert_eq!(guide_at(&app, 50.0, 10.0), None);
        assert_eq!(guide_at(&app, 10.0, 25.0), None);
        app.ui.view.extras = true;

        // Hiding/showing must not change the stored guide positions.
        assert_eq!(app.session.active().unwrap().doc.guides, positions);
        assert_eq!(guide_at(&app, 50.0, 10.0), Some((true, 0)));
        assert_eq!(guide_at(&app, 10.0, 25.0), Some((false, 0)));

        app.ui.extras.lock_guides = true;
        assert_eq!(guide_at(&app, 50.0, 10.0), None);
        assert_eq!(guide_at(&app, 10.0, 25.0), None);
    }

    #[test]
    fn unit_menu_lists_every_unit_with_the_current_one_checked() {
        let items = unit_menu_items(Unit::Inches);
        assert_eq!(items.len(), Unit::NAMES.len());
        // Every unit is offered, labelled, and in the preference's order.
        assert_eq!(items[0], ("Pixels".to_string(), "pixels", false));
        assert_eq!(items.iter().find(|(_, name, _)| *name == "cm").unwrap().0, "Centimeters");
        // Exactly the current unit is checked.
        let checked: Vec<_> = items.iter().filter(|(_, _, c)| *c).map(|(_, name, _)| *name).collect();
        assert_eq!(checked, vec!["inches"]);
    }

    #[test]
    fn picking_a_ruler_unit_changes_the_preference() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        assert_eq!(app.session.prefs().units_and_rulers.rulers, Unit::Pixels);
        // The context menu applies its pick exactly as the preference pane does.
        app.run("prefs.set", json!({"path": "unitsAndRulers.rulers", "value": "cm"})).unwrap();
        assert_eq!(app.session.prefs().units_and_rulers.rulers, Unit::Centimeters);
    }

    /// View › Rulers is remembered: the next launch, and every document created or opened in it,
    /// shows the rulers as last set, on or off (#2918).
    #[test]
    fn rulers_open_as_last_set_after_a_restart() {
        use std::{cell::RefCell, rc::Rc};
        let saved = Rc::new(RefCell::new(None::<String>));
        let launch = |saved: &Rc<RefCell<Option<String>>>| {
            let (load, store) = (saved.clone(), saved.clone());
            PhotocraftApp::new(
                photocraft_engine::Session::new(),
                crate::Services {
                    load_prefs: Some(Box::new(move || load.borrow().clone())),
                    save_prefs: Some(Box::new(move |text| {
                        *store.borrow_mut() = Some(text.to_string());
                        Ok(())
                    })),
                    ..Default::default()
                },
            )
        };
        for on in [true, false] {
            let mut app = launch(&saved);
            assert_eq!(crate::menus::invoke(&mut app, &egui::Context::default(), "view.rulers", json!({})).unwrap(), json!(on));
            crate::prefs_ui::save_preferences(&mut app).unwrap();
            let mut next = launch(&saved);
            next.session.execute("file.new", json!({"width": 20, "height": 10})).unwrap();
            next.sync_views();
            assert_eq!(next.ui.extras.rulers, on, "a new document after a restart shows the rulers as last set");
        }
    }

    #[test]
    fn guide_drags_create_move_and_delete() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
        app.sync_views();
        finish_drag(&mut app, GuideDrag { vertical: true, index: None, pos: 50.0 });
        finish_drag(&mut app, GuideDrag { vertical: false, index: None, pos: 500.0 }); // off canvas: ignored
        assert_eq!(app.session.active().unwrap().doc.guides.vertical, vec![50.0]);
        assert!(app.session.active().unwrap().doc.guides.horizontal.is_empty());
        assert_eq!(guide_at(&app, 51.0, 10.0), Some((true, 0)));
        finish_drag(&mut app, GuideDrag { vertical: true, index: Some(0), pos: 120.0 });
        assert_eq!(app.session.active().unwrap().doc.guides.vertical, vec![120.0]);
        finish_drag(&mut app, GuideDrag { vertical: true, index: Some(0), pos: -30.0 });
        assert!(app.session.active().unwrap().doc.guides.vertical.is_empty());
        app.ui.extras.lock_guides = true;
        assert_eq!(guide_at(&app, 0.0, 0.0), None);
    }

    /// ⌘ is the Move tool with any tool, so a ⌘-drag over a guide moves the guide, never the layer
    /// (#2690); with Lock Guides a ⌘-drag with a marquee moves the layer as before.
    #[test]
    fn command_drag_over_a_guide_moves_the_guide_with_any_tool() {
        use crate::canvas::{ToolEvent, tool_event};
        use crate::state::Tool;
        for (tool, locked) in [(Tool::Brush, false), (Tool::RectMarquee, false), (Tool::Lasso, false), (Tool::MagicWand, false), (Tool::RectMarquee, true)] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            app.run("file.new", json!({"width": 80, "height": 60, "background": "transparent"})).unwrap();
            app.sync_views();
            app.ui.tool = tool;
            app.ui.extras.lock_guides = locked;
            let layer = app.session.active().unwrap().active_layer.unwrap();
            app.session
                .edit("paint", |doc, _| {
                    doc.layer_mut(layer).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(10, 10, 30, 30), &[1.0, 0.0, 0.0, 1.0]);
                    Ok(())
                })
                .unwrap();
            app.run("view.newGuide", json!({"orientation": "vertical", "position": 50})).unwrap();
            let cmd = egui::Modifiers::COMMAND;
            tool_event(&mut app, ToolEvent::Down { x: 50.0, y: 40.0, pressure: 1.0 }, cmd);
            tool_event(&mut app, ToolEvent::Move { x: 65.0, y: 40.0, pressure: 1.0 }, cmd);
            tool_event(&mut app, ToolEvent::Up { x: 65.0, y: 40.0 }, cmd);
            let doc = &app.session.active().unwrap().doc;
            let moved = doc.layer(layer).unwrap().surface().unwrap().rgba(12, 12)[3] == 0.0;
            if locked {
                assert_eq!((doc.guides.vertical.clone(), moved), (vec![50.0], true), "{tool:?}, guides locked: the layer moves");
            } else {
                assert_eq!((doc.guides.vertical.clone(), moved), (vec![65.0], false), "{tool:?}: the guide moves, the layer stays");
            }
        }
    }

    /// #3233: a rotated view keeps the rulers level and fully ticked at zoom scale. The top ruler
    /// spans the same number of document pixels as the unrotated view, the ticks map back onto
    /// its ends, and both rulers read 0 at the screen position of the (rotated) document origin,
    /// as Photoshop does.
    #[test]
    fn rotated_view_keeps_full_rulers() {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        let (a, b) = (rect.left_top(), rect.right_top());
        for rotation in [0.0f32, 30.0, 45.0, 90.0, -119.0, 180.0] {
            for flip in [false, true] {
                let xf = ViewXform { rect, zoom: 0.5, center: [1000.0, 700.0], flip, rotation, aspect: 1.0 };
                let (u0, u1) = ruler_range(&xf, a, b, 0, 1.0);
                assert!((u1 - u0 - 1600.0).abs() < 1e-2, "rotation {rotation}: top ruler covers {u0}..{u1}");
                let (first, last) = if flip { (u1, u0) } else { (u0, u1) };
                assert!((ruler_pos(&xf, false, first, 1.0) - a.x).abs() < 1e-2, "rotation {rotation}: first tick");
                assert!((ruler_pos(&xf, false, last, 1.0) - b.x).abs() < 1e-2, "rotation {rotation}: last tick");
                // 0 at the projected origin, 500 document px = 500 × zoom screen px further on.
                let origin = xf.to_screen(0.0, 0.0);
                assert!((ruler_pos(&xf, false, 0.0, 1.0) - origin.x).abs() < 1e-2, "rotation {rotation}: top ruler 0");
                assert!((ruler_pos(&xf, true, 0.0, 1.0) - origin.y).abs() < 1e-2, "rotation {rotation}: left ruler 0");
                let dir = if flip { -1.0 } else { 1.0 };
                assert!((ruler_pos(&xf, false, 500.0, 1.0) - origin.x - dir * 250.0).abs() < 1e-2, "rotation {rotation}: top scale");
                assert!((ruler_pos(&xf, true, 500.0, 1.0) - origin.y - 250.0).abs() < 1e-2, "rotation {rotation}: left scale");
            }
        }
        // Distances left of / above the origin read without a minus sign, as in Photoshop.
        let xf = ViewXform { rect, zoom: 0.5, center: [0.0, 0.0], flip: false, rotation: 0.0, aspect: 1.0 };
        let (u0, u1) = ruler_range(&xf, a, b, 0, 1.0);
        assert!(u0 < -500.0 && u1 > 500.0, "the view spans the origin: {u0}..{u1}");
        for (step, whole) in [(100.0, true), (100.0, false), (0.25, false)] {
            let mut v = (u0 / step).floor() * step;
            while v <= u1 {
                let s = ruler_label(v, step, whole);
                assert!(!s.contains('-'), "tick {v} (step {step}) is labelled {s:?}");
                v += step;
            }
        }
        assert_eq!(ruler_label(-500.0, 100.0, true), "500");
        assert_eq!(ruler_label(-0.25, 0.25, false), "0.25");
    }

    /// #3277: guides belong to the document, so a rotated view draws them along the document
    /// line; unrotated they still span the whole clip rect.
    #[test]
    fn guides_turn_with_a_rotated_view() {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        let level = ViewXform { rect, zoom: 0.5, center: [1000.0, 700.0], flip: false, rotation: 0.0, aspect: 1.0 };
        let [a, b] = guide_segment(&level, rect, true, 500.0);
        assert!((a.x - b.x).abs() < 1e-3 && (a.y - rect.top()).abs() < 1e-3 && (b.y - rect.bottom()).abs() < 1e-3, "{a:?} {b:?}");
        for rotation in [30.0f32, 90.0, -119.0] {
            let xf = ViewXform { rotation, ..level };
            for (vertical, pos) in [(true, 500.0), (false, 900.0)] {
                for end in guide_segment(&xf, rect, vertical, pos) {
                    let d = xf.to_doc(end);
                    let on_line = if vertical { d[0] } else { d[1] };
                    assert!((on_line - pos).abs() < 1e-2, "rotation {rotation}: {end:?} maps to {d:?}, off the guide at {pos}");
                }
            }
        }
    }

    #[test]
    fn ruler_range_is_normalised_when_flipped() {
        // #1814: with View > Flip Horizontal, `to_doc` negates x, so the raw
        // (d0, d1) comes out reversed. ruler_range must normalise to (min, max)
        // so the tick loop still runs. Exercises the production helper.
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        let top = Rect::from_min_size(rect.min, vec2(rect.width(), RULER));
        for flip in [false, true] {
            let xf = ViewXform { rect, zoom: 1.0, center: [400.0, 300.0], flip, rotation: 0.0, aspect: 1.0 };
            let (u0, u1) = ruler_range(&xf, top.left_top(), top.right_top(), 0, 1.0);
            assert!(u0 <= u1, "flip={flip}: u0 ({u0}) must be <= u1 ({u1})");
            assert!(u1 - u0 > 0.0, "flip={flip}: range is empty");
        }
    }

    /// #3770: a guide Shift-dragged out of a ruler lands on the nearest tick the ruler draws
    /// (tenths of the labelled step, in the ruler unit); without Shift it lands where it's dropped.
    #[test]
    fn shift_dragged_guides_land_on_ruler_ticks() {
        let full = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0));
        let rect = Rect::from_min_max(full.min + vec2(RULER, RULER), full.max);
        let xf = ViewXform { rect, zoom: 1.0, center: [128.0, 64.0], flip: false, rotation: 0.0, aspect: 1.0 };
        let new_app = || {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            app.run("file.new", json!({"width": 256, "height": 128})).unwrap();
            app
        };
        let drag_guide = |modifiers: egui::Modifiers| {
            let builder = egui_kittest::Harness::builder().with_size(vec2(400.0, 300.0));
            let mut h = builder.build_ui_state(move |ui, app| draw_rulers(app, ui, full, &xf), new_app());
            // Out of the left ruler: a vertical guide, dropped at document x = 37.
            let (from, to) = (pos2(RULER / 2.0, 100.0), xf.to_screen(37.0, 64.0));
            h.event_modifiers(egui::Event::PointerMoved(from), modifiers);
            h.event_modifiers(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers }, modifiers);
            h.run_steps(1);
            h.event_modifiers(egui::Event::PointerMoved(to), modifiers);
            h.run_steps(1);
            h.event_modifiers(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers }, modifiers);
            h.run_steps(2);
            h.state().session.active().unwrap().doc.guides.vertical.clone()
        };
        assert_eq!(drag_guide(egui::Modifiers::NONE), vec![37.0]);
        // Pixels at 100%: labels every 100 px, ticks every 10 px.
        assert_eq!(drag_guide(egui::Modifiers::SHIFT), vec![40.0]);

        let mut app = new_app();
        // At 200% the labels are every 50 px, so the ticks every 5 px.
        assert_eq!(snap_to_tick(&app, &ViewXform { zoom: 2.0, ..xf }, true, 37.0), 35.0);
        // Percent of each side: ticks every 2.5% of 256 px across, every 5% of 128 px down.
        app.run("prefs.set", json!({"path": "unitsAndRulers.rulers", "value": "percent"})).unwrap();
        assert!((snap_to_tick(&app, &xf, true, 37.0) - 38.4).abs() < 1e-9);
        assert!((snap_to_tick(&app, &xf, false, 37.0) - 38.4).abs() < 1e-9);
    }
}
