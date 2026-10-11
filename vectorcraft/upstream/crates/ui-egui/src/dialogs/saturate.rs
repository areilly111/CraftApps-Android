//! Edit → Edit Colors → Saturate: an intensity slider (−100..100 %) previewed live on the canvas;
//! OK keeps the preview as one undo step (`edit.colors.saturate`).
//!
//! Fields: `intensity` and `preview`.

use serde_json::{Value, json};
use vectorcraft_color::Color;

use super::{DialogSpec, form};
use crate::VectorcraftApp;
use crate::panels::c32;
use crate::state::Dialog;

/// The dialog kind of Saturate.
pub const KIND: &str = "saturate";

const CMD: &str = "edit.colors.saturate";

pub(super) const SPEC: DialogSpec = DialogSpec { heading: |_| tl!("Saturate").into(), body, confirm, preview: true, ..DialogSpec::FORM };

pub fn open(app: &mut VectorcraftApp) {
    app.ui.dialog = Some(Dialog::new(KIND, json!({"intensity": 0, "preview": true})));
}

fn params(d: &Dialog) -> Value {
    json!({"intensity": d.f64("intensity", 0.0)})
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    // The rail runs from grey to a fully saturated colour.
    form::slider(ui, d, "intensity", tl!("Intensity:"), -100.0..=100.0, "%", &|x| c32(&Color::from_hsb(12.0, x, 0.85)));
    let p = params(d);
    form::preview(app, ui, d, "Saturate", CMD, p);
    false
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    form::commit_preview(app, CMD, params(d))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    #[test]
    fn commits_without_a_drawn_frame_as_one_step() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
        app.run("paint.setFill", json!({"color": "#cc6666"})).unwrap();
        let undo = app.session.doc().unwrap().history.undo.len();
        app.run("ui.saturateDialog", json!({})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("intensity".into(), json!(-100));
        super::super::confirm(&mut app).unwrap();
        let st = app.session.doc().unwrap();
        assert_eq!(st.history.undo.len(), undo + 1);
        let c = st.doc.node(vectorcraft_doc::NodeId(id)).unwrap().appearance.fill_paint().color().unwrap();
        assert!(c.to_hsb()[1] < 1e-3, "fully desaturated");
    }
}
