//! Perspective plane options: double-clicking a plane widget of the perspective grid (or
//! `ui.perspectivePlane {plane}`) moves that plane to an exact location, alone, with the objects on
//! it, or with copies of them. OK runs `perspective.plane.move`.
//!
//! Fields: `plane` (`left`, `right` or `ground`), `location` (points along the plane's normal; 0
//! is its place in the grid's definition), `objects` (`none`, `move` or `copy`).

use serde_json::{Value, json};
use vectorcraft_tools::distort::perspective::{PerspectiveGrid, Plane};

use super::swatch_options::{grid, label};
use super::{DialogSpec, run_and_close};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of the plane options.
pub const KIND: &str = "perspectivePlane";

pub(super) const SPEC: DialogSpec = DialogSpec { heading, body, confirm, min_width: 300.0, ..DialogSpec::FORM };

/// What becomes of the objects on the plane: (`objects` value, label).
const OBJECTS: [(&str, &str); 3] = [("none", "Do Not Move"), ("move", "Move All Objects"), ("copy", "Copy All Objects")];

fn heading(d: &Dialog) -> String {
    match Plane::parse(&d.str("plane")) {
        Some(Plane::Right) => tl!("Right Plane"),
        Some(Plane::Ground) => tl!("Horizontal Plane"),
        _ => tl!("Left Plane"),
    }
    .into()
}

/// Open the options of `plane` (`{plane}`; default the active plane), filled in with its location.
pub fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let doc = &app.session.active().ok_or("no document")?.doc;
    let g = PerspectiveGrid::current(doc);
    let plane = match p.get("plane").and_then(Value::as_str) {
        Some(s) => Plane::parse(s).ok_or("plane must be left, right or ground")?,
        None => g.plane,
    };
    if plane == Plane::None {
        return Err("no active perspective plane".into());
    }
    let fields = json!({"plane": plane.id(), "location": g.offset(plane), "objects": "none"});
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(Value::Null)
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let unit = app.session.general_unit();
    grid(ui, |ui| {
        label(ui, tl!("Location:"));
        if let Some(v) = widgets::num_field(ui, "pp-location", Some(d.f64("location", 0.0)), unit, 120.0) {
            d.fields.insert("location".into(), json!(v));
        }
        ui.end_row();
    });
    ui.add_space(6.0);
    let cur = d.str("objects");
    for (value, text) in OBJECTS {
        if widgets::radio(ui, text, cur == value || (cur.is_empty() && value == "none"), true) {
            d.fields.insert("objects".into(), json!(value));
        }
    }
    false
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let objects = Some(d.str("objects")).filter(|o| OBJECTS.iter().any(|(v, _)| v == o)).unwrap_or_else(|| "none".into());
    let p = json!({"plane": d.str("plane"), "offset": d.f64("location", 0.0), "objects": objects});
    run_and_close(app, "perspective.plane.move", p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;
    use vectorcraft_tools::{PointerEvent, PointerKind};

    fn shown(app: &mut VectorcraftApp) -> Vec<String> {
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
    fn double_clicking_a_plane_widget_opens_its_options_and_ok_moves_it() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
        app.run("perspective.grid.preset", json!({"kind": 2})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 450, "y": 380, "width": 60, "height": 60})).unwrap()["id"].clone();
        app.run("perspective.attach", json!({"ids": [id], "plane": "right"})).unwrap();
        let g = PerspectiveGrid::current(&app.session.active().unwrap().doc);
        let (_, w) = g.plane_widgets().into_iter().find(|(p, _)| *p == Plane::Right).unwrap();
        app.select_tool("perspectiveGrid");
        let view = app.view_info();
        crate::canvas::dispatch(&mut app, &PointerEvent::new(PointerKind::DoubleClick, w.x, w.y), view);
        let d = app.ui.dialog.clone().expect("the plane's options opened");
        assert_eq!((d.kind.as_str(), d.str("plane").as_str(), d.f64("location", 1.0)), (KIND, "right", 0.0));
        let texts = shown(&mut app);
        for s in ["Right Plane", "Location:", "Do Not Move", "Move All Objects", "Copy All Objects"] {
            assert!(texts.iter().any(|t| t == s), "{s} in {texts:?}");
        }
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("location".into(), json!(40));
        d.fields.insert("objects".into(), json!("copy"));
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let doc = &app.session.active().unwrap().doc;
        assert_eq!(PerspectiveGrid::current(doc).right_offset, 40.0);
        assert_eq!(doc.layers[0].children().unwrap().len(), 2, "copied");
        // The UI command opens it too; unknown planes are refused.
        app.run("ui.perspectivePlane", json!({"plane": "ground"})).unwrap();
        assert_eq!(heading(app.ui.dialog.as_ref().unwrap()), "Horizontal Plane");
        assert!(app.run("ui.perspectivePlane", json!({"plane": "up"})).is_err());
    }

    #[test]
    fn a_click_to_size_rectangle_goes_on_the_active_plane() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
        app.run("perspective.grid.preset", json!({"kind": 2})).unwrap();
        app.run("perspective.plane.set", json!({"plane": "right"})).unwrap();
        app.select_tool("rectangle");
        let view = app.view_info();
        for kind in [PointerKind::Down, PointerKind::Up] {
            crate::canvas::dispatch(&mut app, &PointerEvent::new(kind, 470.0, 300.0), view);
        }
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some("rectangle"));
        super::super::confirm(&mut app).unwrap();
        assert_eq!(app.session.journal.last().unwrap().0, "perspective.draw");
        let st = app.session.active().unwrap();
        let id = *st.selection.objects.first().unwrap();
        assert_eq!(vectorcraft_tools::distort::perspective::attachment(st.doc.node(id).unwrap()).map(|a| a.0), Some(Plane::Right));
    }
}
