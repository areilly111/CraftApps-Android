//! Width Point Edit: double-clicking a width point with the Width tool (or `ui.widthPointEdit`)
//! edits its side widths. Total Width changes both sides in proportion, the link keeps them equal,
//! and Adjust Adjoining Width Points changes the nearest points either side in proportion too.
//! OK runs `stroke.widthPoint.set`; Delete removes the point (`stroke.widthPoint.remove`).
//!
//! Fields: `id`, `index` (the width point), `t` (its place), `side1`, `side2` (the left and right
//! widths, points), `linked`, `adjustAdjoining`; Delete sets `discard: true` and confirms.

use egui::vec2;
use serde_json::{Value, json};
use vectorcraft_engine::doc::NodeId;

use super::swatch_options::{grid, label};
use super::{DialogSpec, run_and_close};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Width Point Edit.
pub const KIND: &str = "widthPoint";

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Width Point Edit").into(), body, confirm, discard: Some("Delete"), min_width: 300.0, ..DialogSpec::FORM };

/// Width point `index` of `id`: its place and side widths (points).
fn point(app: &VectorcraftApp, id: u64, index: usize) -> Option<(f64, f64, f64)> {
    let st = app.session.active()?.doc.node(NodeId(id))?.appearance.stroke()?;
    let half = st.width / 2.0;
    st.profile.as_ref()?.points.get(index).map(|&(t, l, r)| (t, l * half, r * half))
}

/// Open Width Point Edit on width point `index` of path `id` (`{id, index}`), filled in with its
/// widths.
pub fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let (id, index) = (p.get("id").and_then(Value::as_u64), p.get("index").and_then(Value::as_u64));
    let (Some(id), Some(index)) = (id, index) else { return Err("give the path `id` and the width point `index`".into()) };
    let (t, l, r) = point(app, id, index as usize).ok_or("no such width point")?;
    let fields = json!({"id": id, "index": index, "t": t, "side1": l, "side2": r, "linked": (l - r).abs() < 1e-9, "adjustAdjoining": false});
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(Value::Null)
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let unit = app.session.stroke_unit();
    let (s1, s2, linked) = (d.f64("side1", 0.0), d.f64("side2", 0.0), d.bool("linked"));
    let mut sides = None;
    grid(ui, |ui| {
        label(ui, tl!("Side 1:"));
        ui.horizontal(|ui| {
            if let Some(v) = widgets::num_field(ui, "wp-side1", Some(s1), unit, 90.0) {
                sides = Some((v, if linked { v } else { s2 }));
            }
            if widgets::icon_button(ui, if linked { "link" } else { "link-2-off" }, tl!("Keep both sides the same width"), linked, 22.0).clicked() {
                d.fields.insert("linked".into(), json!(!linked));
                if !linked {
                    sides = Some((s1, s1));
                }
            }
        });
        ui.end_row();
        label(ui, tl!("Side 2:"));
        if let Some(v) = widgets::num_field(ui, "wp-side2", Some(s2), unit, 90.0) {
            sides = Some((if linked { v } else { s1 }, v));
        }
        ui.end_row();
        label(ui, tl!("Total Width:"));
        if let Some(v) = widgets::num_field(ui, "wp-total", Some(s1 + s2), unit, 90.0) {
            // Both sides in proportion (equal halves from nothing).
            let k = if s1 + s2 > 1e-9 { v / (s1 + s2) } else { 0.0 };
            sides = Some(if k > 0.0 { (s1 * k, s2 * k) } else { (v / 2.0, v / 2.0) });
        }
        ui.end_row();
    });
    if let Some((a, b)) = sides {
        d.fields.insert("side1".into(), json!(a.max(0.0)));
        d.fields.insert("side2".into(), json!(b.max(0.0)));
    }
    ui.add_space(4.0);
    ui.allocate_ui(vec2(ui.available_width(), 22.0), |ui| {
        if widgets::check(ui, tl!("Adjust Adjoining Width Points"), d.bool("adjustAdjoining"), true) {
            d.fields.insert("adjustAdjoining".into(), json!(!d.bool("adjustAdjoining")));
        }
    });
    false
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let (id, index) = (d.fields.get("id").cloned().unwrap_or_default(), d.fields.get("index").cloned().unwrap_or_default());
    if d.bool("discard") {
        return run_and_close(app, "stroke.widthPoint.remove", json!({"id": id, "index": index}));
    }
    let (left, right) = (d.f64("side1", 0.0), d.f64("side2", 0.0));
    let p = json!({"id": id, "index": index, "t": d.f64("t", 0.0), "left": left, "right": right, "adjustAdjoining": d.bool("adjustAdjoining")});
    run_and_close(app, "stroke.widthPoint.set", p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;
    use vectorcraft_tools::{PointerEvent, PointerKind};

    /// A 10 pt line from (100, 150) to (300, 150) with a 4 + 4 pt width point in the middle.
    fn app() -> (VectorcraftApp, u64) {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        let id = app.run("shape.line", json!({"x1": 100, "y1": 150, "x2": 300, "y2": 150})).unwrap()["id"].as_u64().unwrap();
        app.run("stroke.set", json!({"ids": [id], "weight": 10})).unwrap();
        app.run("stroke.widthPoint.set", json!({"id": id, "t": 0.5, "left": 4, "right": 4})).unwrap();
        (app, id)
    }

    fn points(app: &VectorcraftApp, id: u64) -> Vec<(f64, f64, f64)> {
        let st = app.session.active().unwrap().doc.node(NodeId(id)).unwrap().appearance.stroke().unwrap().clone();
        let round = |v: f64| (v * 1e9).round() / 1e9;
        st.profile.map(|p| p.points.into_iter().map(|(t, l, r)| (round(t), round(l), round(r))).collect()).unwrap_or_default()
    }

    fn frame(app: &mut VectorcraftApp) -> Vec<String> {
        fn texts(s: &egui::Shape, out: &mut Vec<String>) {
            match s {
                egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
                egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
                _ => {}
            }
        }
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        // A new window lays itself out in its first frame and paints in the next.
        ctx.run_ui(Default::default(), |ui| super::super::show(app, ui.ctx())).textures_delta.clear();
        let mut out = ctx.run_ui(Default::default(), |ui| super::super::show(app, ui.ctx()));
        out.textures_delta.clear();
        let mut v = vec![];
        out.shapes.iter().for_each(|c| texts(&c.shape, &mut v));
        v
    }

    #[test]
    fn double_click_opens_it_and_ok_sets_the_widths() {
        let (mut app, id) = app();
        let view = app.view_info();
        app.select_tool("width");
        let ev = PointerEvent::new(PointerKind::DoubleClick, 200.0, 150.0);
        crate::canvas::dispatch(&mut app, &ev, view);
        let d = app.ui.dialog.clone().expect("Width Point Edit opened");
        assert_eq!((d.kind.as_str(), d.f64("side1", 0.0), d.f64("side2", 0.0), d.bool("linked")), (KIND, 4.0, 4.0, true));
        let shown = frame(&mut app);
        for s in ["Width Point Edit", "Side 1:", "Side 2:", "Total Width:", "Adjust Adjoining Width Points", "Delete"] {
            assert!(shown.iter().any(|t| t == s), "{s} in {shown:?}");
        }
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("side1".into(), json!(6));
        d.fields.insert("side2".into(), json!(2));
        d.fields.insert("adjustAdjoining".into(), json!(true));
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        // Side 1 is 6 pt (factor 1.2 of the 5 pt half weight), and the ends scaled with it.
        assert_eq!(points(&app, id), vec![(0.0, 1.5, 0.5), (0.5, 1.2, 0.4), (1.0, 1.5, 0.5)]);
    }

    #[test]
    fn delete_removes_the_point() {
        let (mut app, id) = app();
        app.run("ui.widthPointEdit", json!({"id": id, "index": 1})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("discard".into(), json!(true));
        super::super::confirm(&mut app).unwrap();
        assert_eq!(points(&app, id).len(), 2);
        assert!(app.run("ui.widthPointEdit", json!({"id": id, "index": 9})).is_err());
    }

    #[test]
    fn it_does_not_stretch_to_the_screen() {
        let height = |screen_h: f32| {
            let (mut app, id) = app();
            app.run("ui.widthPointEdit", json!({"id": id, "index": 1})).unwrap();
            let ctx = egui::Context::default();
            crate::theme::install_fonts(&ctx);
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, screen_h));
            for _ in 0..4 {
                let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
                ctx.run_ui(input, |ui| super::super::show(&mut app, ui.ctx())).textures_delta.clear();
            }
            ctx.memory(|m| m.area_rect(egui::Id::new(("dialog", KIND)))).unwrap().height()
        };
        let (short, tall) = (height(1000.0), height(2000.0));
        assert!((short - tall).abs() < 1.0 && short < 400.0, "{short} vs {tall}");
    }
}
