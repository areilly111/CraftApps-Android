//! The Print dialog's Advanced and Color Management options and the Print Tiling tool's placement
//! in it: they draw, label, save with the document and move with the preview.

use serde_json::{Value, json};
use vectorcraft_engine::Session;

use super::print::KIND;
use crate::state::Dialog;
use crate::{VectorcraftApp, theme};

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 120, "height": 90})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    app
}

fn frame(app: &mut VectorcraftApp) {
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| super::show(app, ui.ctx()));
    out.textures_delta.clear();
}

fn set(app: &mut VectorcraftApp, field: &str, value: Value) {
    app.ui.dialog.as_mut().expect("a dialog is open").fields.insert(field.into(), value);
}

#[test]
fn advanced_options_draw_label_and_save_with_the_document() {
    let mut app = app();
    app.run("print.tiling.set", json!({"origin": [-20, -10]})).unwrap();
    app.run("file.print", json!({})).unwrap();
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["tileOrigin"]["placed"], true, "the tool's placement comes in");
    for section in ["General", "Color Management", "Advanced", "Summary"] {
        for mode in ["composite", "separations"] {
            set(&mut app, "__section", json!(section));
            set(&mut app, "output", json!({"mode": mode}));
            set(&mut app, "advanced", json!({"printAsBitmap": true, "overprints": "simulate", "flattenerPreset": "High Resolution"}));
            set(&mut app, "color", json!({"profile": vectorcraft_color::cms::GENERIC_CMYK}));
            frame(&mut app);
            assert!(app.ui.dialog.is_some(), "{section} ({mode}) closed the dialog");
        }
    }
    let mut changed = vec![json!({"option": "advanced.overprints", "value": "simulate"})];
    super::print::labelled(&mut changed);
    assert_eq!(changed[0]["value"], "Simulate");
    assert_eq!(super::print::option_label("advanced.printAsBitmap"), "Advanced › Print As Bitmap");
    // Done keeps them with the document.
    set(&mut app, "output", json!({"mode": "composite"}));
    set(&mut app, "discard", json!(true));
    super::confirm(&mut app).unwrap();
    let saved = app.session.execute("print.setup", &json!({})).unwrap()["settings"].clone();
    assert_eq!(saved["advanced"], json!({"printAsBitmap": true, "overprints": "simulate", "flattenerPreset": "High Resolution"}));
    assert_eq!(saved["color"]["profile"], vectorcraft_color::cms::GENERIC_CMYK);
    assert_eq!(saved["tileOrigin"]["placed"], true);
}

#[test]
fn dragging_the_preview_moves_the_print_tiling_tools_pages() {
    let app = &mut app();
    for settings in [json!({}), json!({"transverse": true, "scaling": "custom", "scale": {"width": 50, "height": 200}})] {
        let mut settings = settings;
        settings["tileOrigin"] = json!({"placed": true, "x": -20, "y": -10});
        let before = app.session.execute("print.preview", &json!({ "settings": settings })).unwrap();
        let mut d = Dialog::new(KIND, settings.clone());
        super::print::move_placement(&mut d, &before["sheets"][0], (10.0, 5.0));
        let moved = super::print::settings(&d);
        assert_eq!(moved["placement"], Value::Null, "the placement stays");
        let after = app.session.execute("print.preview", &json!({ "settings": moved })).unwrap();
        let (t0, t1) = (&before["sheets"][0]["trim"], &after["sheets"][0]["trim"]);
        let delta = (t1[0].as_f64().unwrap() - t0[0].as_f64().unwrap(), t1[1].as_f64().unwrap() - t0[1].as_f64().unwrap());
        assert!((delta.0 - 10.0).abs() < 0.05 && (delta.1 - 5.0).abs() < 0.05, "{settings}: moved {delta:?}");
    }
}
