//! Swatches panel → Spot Colors…: whether spot colours defined in Lab show and separate from
//! their Lab values or from their CMYK equivalents. OK runs `swatch.spotOptions`.
//!
//! Fields: `useLab` (true: Lab values; false: CMYK equivalents).

use serde_json::{Value, json};

use super::swatch_options::{grid, label};
use super::{DialogSpec, run_and_close};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Spot Colors.
pub const KIND: &str = "spotColors";

pub(super) const SPEC: DialogSpec = DialogSpec { heading: |_| tl!("Spot Colors").into(), body, confirm, min_width: 360.0, ..DialogSpec::FORM };

/// The choices: (`useLab`, label, what it does).
const CHOICES: [(bool, &str, &str); 2] = [
    (true, "Lab Values", "Show and print from the Lab definition; PDF files carry it as the ink's alternate color."),
    (false, "CMYK Equivalents", "Show and separate through the working CMYK space, as in older documents."),
];

/// Open Spot Colors on the active document's setting.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let use_lab = app.session.active().ok_or("no document")?.doc.spot_use_lab;
    app.ui.dialog = Some(Dialog::new(KIND, json!({ "useLab": use_lab })));
    Ok(Value::Null)
}

fn body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let use_lab = d.bool("useLab");
    grid(ui, |ui| {
        for (i, (value, text, help)) in CHOICES.into_iter().enumerate() {
            if i == 0 {
                label(ui, tl!("Lab Spot Colors:"));
            } else {
                ui.label("");
            }
            ui.vertical(|ui| {
                if widgets::radio(ui, tl!(text), use_lab == value, true) {
                    d.fields.insert("useLab".into(), json!(value));
                }
                ui.indent(text, |ui| label(ui, tl!(help)));
            });
            ui.end_row();
        }
    });
    false
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    run_and_close(app, "swatch.spotOptions", json!({ "useLab": d.bool("useLab") }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_color::Color;
    use vectorcraft_engine::Session;

    #[test]
    fn the_dialog_switches_lab_spots_to_cmyk_and_back() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        assert!(app.run("ui.spotColors", json!({})).is_err(), "needs a document");
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app.run("swatch.new", json!({"name": "Ink", "color": {"l": 60, "a": 50, "b": 20}, "spot": true})).unwrap();
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
        app.run("paint.setFill", json!({"swatch": "Ink"})).unwrap();
        let fill = |app: &VectorcraftApp| crate::panels::current_paints(app).0.color().unwrap();
        assert_eq!(fill(&app), Color::lab(60.0, 50.0, 20.0));
        app.run("ui.spotColors", json!({})).unwrap();
        assert!(app.ui.dialog.as_ref().unwrap().bool("useLab"), "opens on the document's setting");
        // Drawn headlessly in the shared frame; picking the CMYK radio goes through the fields.
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(Default::default(), |ui| super::super::show(&mut app, ui.ctx()));
        out.textures_delta.clear();
        app.ui.dialog.as_mut().unwrap().fields.insert("useLab".into(), json!(false));
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert!(!app.session.active().unwrap().doc.spot_use_lab);
        assert!(matches!(fill(&app), Color::Cmyk { .. }), "the linked fill shows the CMYK equivalent");
        app.run("ui.spotColors", json!({})).unwrap();
        assert!(!app.ui.dialog.as_ref().unwrap().bool("useLab"));
        app.ui.dialog.as_mut().unwrap().fields.insert("useLab".into(), json!(true));
        super::super::confirm(&mut app).unwrap();
        assert_eq!(fill(&app), Color::lab(60.0, 50.0, 20.0));
    }
}
