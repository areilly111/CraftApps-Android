//! Object → Expand: what to expand (Object: live shapes, type and effects; Fill: gradient fills;
//! Stroke: strokes into filled outlines) and what gradient fills become (a gradient mesh, or a
//! number of objects). Options the selection has nothing for are disabled. OK runs
//! `object.expand` as one undo step.
//!
//! Fields: `object`, `fill`, `stroke`, `gradient` (`objects` | `mesh`), `steps` (1..1000), and
//! `__object`, `__fill`, `__stroke` (that option has something to expand: `object.expand.info`).

use serde_json::{Value, json};
use vectorcraft_engine::cmd::expand::{DEFAULT_STEPS, MAX_STEPS};

use super::{DialogSpec, run_and_close};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Expand.
pub const KIND: &str = "expand";

const CMD: &str = "object.expand";

/// The options and their checkbox labels.
const OPTIONS: [(&str, &str); 3] = [("object", "Object"), ("fill", "Fill"), ("stroke", "Stroke")];

pub(super) const SPEC: DialogSpec = DialogSpec { heading: |_| tl!("Expand").into(), body, confirm, min_width: 260.0, ..DialogSpec::FORM };

/// Open Expand for the selection, every option on (as `object.expand {}` does).
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let info = app.session.execute("object.expand.info", &json!({})).map_err(|e| e.to_string())?;
    let mut fields = json!({"object": true, "fill": true, "stroke": true, "gradient": "objects", "steps": DEFAULT_STEPS});
    for (key, _) in OPTIONS {
        fields[format!("__{key}")] = info[key].clone();
    }
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(Value::Null)
}

fn body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let set = |d: &mut Dialog, key: &str, v: Value| {
        d.fields.insert(key.into(), v);
    };
    widgets::subheader(ui, tl!("Expand"));
    for (key, label) in OPTIONS {
        let on = d.bool(key);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            if widgets::check(ui, label, on, d.bool(&format!("__{key}"))) {
                set(d, key, json!(!on));
            }
        });
    }
    ui.add_space(10.0);
    widgets::subheader(ui, tl!("Expand Gradient To"));
    // Only gradient fills that Fill expands care.
    let enabled = d.bool("__fill") && d.bool("fill");
    let objects = d.str("gradient") != "mesh";
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        if widgets::radio(ui, tl!("Gradient Mesh"), !objects, enabled) {
            set(d, "gradient", json!("mesh"));
        }
    });
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        if widgets::radio(ui, tl!("Specify:"), objects, enabled) {
            set(d, "gradient", json!("objects"));
        }
        ui.add_enabled_ui(enabled && objects, |ui| {
            if let Some(n) = widgets::plain_field(ui, "expand-steps", d.f64("steps", DEFAULT_STEPS as f64), "", 0, 52.0) {
                set(d, "steps", json!(n.round().clamp(1.0, MAX_STEPS as f64)));
            }
            widgets::dim_label(ui, tl!("Objects"));
        });
    });
    false
}

/// `object.expand` parameters from the fields.
fn params(d: &Dialog) -> Value {
    json!({
        "object": d.bool("object"),
        "fill": d.bool("fill"),
        "stroke": d.bool("stroke"),
        "gradient": d.str("gradient"),
        "steps": d.f64("steps", DEFAULT_STEPS as f64),
    })
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    run_and_close(app, CMD, params(d))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;
    use vectorcraft_engine::doc::NodeKind;

    fn frame(app: &mut VectorcraftApp) -> String {
        crate::tests_labels::painted_text(app, |app, ui| super::super::show(app, ui.ctx()))
    }

    #[test]
    fn opens_with_what_applies_and_ok_expands_the_gradient_to_a_mesh() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 100, "height": 60})).unwrap();
        app.run("paint.setFill", json!({"gradient": {"kind": "linear"}})).unwrap();
        app.run("paint.setStroke", json!({"none": true})).unwrap();
        // The menu item (and `object.expand` without params) opens the dialog.
        crate::menus::invoke(&mut app, "object.expand", json!({}));
        let d = app.ui.dialog.as_ref().expect("the Expand dialog");
        assert_eq!(d.kind, KIND);
        assert_eq!((d.bool("__object"), d.bool("__fill"), d.bool("__stroke")), (true, true, false), "no stroke to expand");
        assert_eq!(params(d), json!({"object": true, "fill": true, "stroke": true, "gradient": "objects", "steps": 255.0}));
        let text = frame(&mut app);
        for label in ["Expand", "Object", "Fill", "Stroke", "Expand Gradient To", "Gradient Mesh", "Specify:", "Objects", "OK", "Cancel"] {
            assert!(text.contains(label), "{label} in {text}");
        }
        let undo = app.session.doc().unwrap().history.undo.len();
        app.ui.dialog.as_mut().unwrap().fields.insert("gradient".into(), json!("mesh"));
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let st = app.session.doc().unwrap();
        assert_eq!(st.history.undo.len(), undo + 1);
        let g = st.doc.node(st.selection.objects[0]).unwrap();
        let NodeKind::Group { children, clip: true } = &g.kind else { panic!("a clip group: {:?}", g.kind) };
        assert!(matches!(children[1].kind, NodeKind::Mesh(_)));
    }

    #[test]
    fn the_command_palette_opens_it_and_ui_expand_dialog_too() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 100, "height": 60})).unwrap();
        app.run("ui.expandDialog", json!({})).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.bool("__object"), d.bool("__fill"), d.bool("__stroke")), (true, false, true));
        frame(&mut app);
        // Explicit params still run the command directly.
        app.ui.dialog = None;
        crate::menus::invoke(&mut app, "object.expand", json!({"stroke": false}));
        assert!(app.ui.dialog.is_none());
        let st = app.session.doc().unwrap();
        assert!(matches!(st.doc.node(st.selection.objects[0]).unwrap().kind, NodeKind::Path { live: None, .. }));
    }
}
