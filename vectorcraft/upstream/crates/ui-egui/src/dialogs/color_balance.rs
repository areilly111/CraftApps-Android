//! Edit → Edit Colors → Adjust Color Balance: the colour mode (Gray, RGB, CMYK or Global), a slider
//! per channel (−100..100 %; Global has one, Tint, for the tints of global and spot colours),
//! Convert, Fill and Stroke, previewed live on the canvas; OK keeps the preview as one undo step
//! (`edit.colors.adjustBalance`).
//!
//! Fields: `mode` (`gray`, `rgb`, `cmyk` or `global`), the channels `r`, `g`, `b`, `c`, `m`, `y`,
//! `k`, `gray`, `tint`, then `convert`, `fill`, `stroke` and `preview`.

use serde_json::{Value, json};
use vectorcraft_color::Color;

use super::{DialogSpec, form};
use crate::VectorcraftApp;
use crate::panels::c32;
use crate::state::Dialog;
use crate::widgets;

/// The dialog kind of Adjust Color Balance.
pub const KIND: &str = "colorBalance";

const CMD: &str = "edit.colors.adjustBalance";

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Adjust Colors").into(), body, confirm, preview: true, min_width: 340.0, ..DialogSpec::FORM };

/// A colour mode: its `mode` id, label and channels as (field, label).
struct Mode {
    id: &'static str,
    label: &'static str,
    channels: &'static [(&'static str, &'static str)],
}

const MODES: [Mode; 4] = [
    Mode { id: "gray", label: "Grayscale", channels: &[("gray", "Black")] },
    Mode { id: "rgb", label: "RGB", channels: &[("r", "Red"), ("g", "Green"), ("b", "Blue")] },
    Mode { id: "cmyk", label: "CMYK", channels: &[("c", "Cyan"), ("m", "Magenta"), ("y", "Yellow"), ("k", "Black")] },
    Mode { id: "global", label: "Global", channels: &[("tint", "Tint")] },
];

/// Global mode: it shifts the tints of linked colours, which keep their model (no Convert).
const GLOBAL: &str = "global";

/// Open the dialog with every channel at 0, adjusting fills and strokes.
pub fn open(app: &mut VectorcraftApp) {
    let fields = json!({
        "mode": "rgb", "r": 0, "g": 0, "b": 0, "c": 0, "m": 0, "y": 0, "k": 0, "gray": 0, "tint": 0,
        "convert": false, "fill": true, "stroke": true, "preview": true,
    });
    app.ui.dialog = Some(Dialog::new(KIND, fields));
}

fn mode(d: &Dialog) -> &'static Mode {
    let id = d.str("mode");
    MODES.iter().find(|m| m.id == id).unwrap_or(&MODES[1])
}

/// `edit.colors.adjustBalance` parameters: the mode, its channels and the options.
fn params(d: &Dialog) -> Value {
    let m = mode(d);
    let mut p = json!({"mode": m.id, "convert": d.bool("convert"), "fill": d.bool("fill"), "stroke": d.bool("stroke")});
    for (k, _) in m.channels {
        p[*k] = json!(d.f64(k, 0.0));
    }
    p
}

/// The rail of channel `key`: the channel from −100 % to +100 % over a mid colour.
fn track(key: &str, x: f32) -> egui::Color32 {
    let v = x.clamp(0.0, 1.0);
    let c = match key {
        "r" => Color::rgb(v, 0.5, 0.5),
        "g" => Color::rgb(0.5, v, 0.5),
        "b" => Color::rgb(0.5, 0.5, v),
        "c" => Color::cmyk(v, 0.0, 0.0, 0.0),
        "m" => Color::cmyk(0.0, v, 0.0, 0.0),
        "y" => Color::cmyk(0.0, 0.0, v, 0.0),
        "k" => Color::cmyk(0.0, 0.0, 0.0, v),
        _ => Color::gray(v),
    };
    c32(&c)
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let m = mode(d);
    ui.horizontal(|ui| {
        widgets::dim_label(ui, tl!("Color Mode:"));
        let labels: Vec<&str> = MODES.iter().map(|m| m.label).collect();
        if let Some(i) = widgets::dropdown(ui, "balance-mode", m.label, &labels, 120.0) {
            d.fields.insert("mode".into(), json!(MODES[i].id));
        }
        ui.add_space(12.0);
        if widgets::check(ui, tl!("Convert"), d.bool("convert"), m.id != GLOBAL) {
            d.fields.insert("convert".into(), json!(!d.bool("convert")));
        }
    });
    ui.add_space(8.0);
    let m = mode(d);
    if m.id == GLOBAL {
        widgets::dim_label(ui, tl!("Shifts the tints of global and spot colors; other colors stay."));
    }
    for (k, label) in m.channels {
        form::slider(ui, d, k, label, -100.0..=100.0, "%", &|x| track(k, x));
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        widgets::dim_label(ui, tl!("Adjust Options:"));
        for (k, label) in [("fill", tl!("Fill")), ("stroke", tl!("Stroke"))] {
            if widgets::check(ui, label, d.bool(k), true) {
                d.fields.insert(k.into(), json!(!d.bool(k)));
            }
        }
    });
    let p = params(d);
    form::preview(app, ui, d, "Adjust Colors", CMD, p);
    false
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    form::commit_preview(app, CMD, params(d))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    fn fill_hex(app: &VectorcraftApp, id: u64) -> String {
        let n = app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().clone();
        n.appearance.fill_paint().color().unwrap().to_hex()
    }

    fn frame(app: &mut VectorcraftApp) {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| super::super::show(app, ui.ctx()));
        out.textures_delta.clear();
    }

    #[test]
    fn previews_live_and_commits_one_undo_step() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
        app.run("paint.setFill", json!({"color": "#808080"})).unwrap();
        let undo = app.session.doc().unwrap().history.undo.len();
        app.run("ui.colorBalanceDialog", json!({})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(KIND));
        app.ui.dialog.as_mut().unwrap().fields.insert("r".into(), json!(20));
        frame(&mut app);
        assert!(app.session.in_interaction(), "the preview runs");
        assert_eq!(fill_hex(&app, id), "#b38080", "previewed on the canvas");
        // A field set from outside (`ui.dialog.set`) previews on the next frame.
        app.ui.dialog.as_mut().unwrap().fields.insert("r".into(), json!(40));
        frame(&mut app);
        assert_eq!(fill_hex(&app, id), "#e68080");
        // Cancel rolls the preview back.
        super::super::cancel(&mut app);
        assert_eq!(fill_hex(&app, id), "#808080");
        // OK keeps it as one undo step.
        app.run("ui.colorBalanceDialog", json!({})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("mode".into(), json!("cmyk"));
        d.fields.insert("k".into(), json!(-50));
        frame(&mut app);
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none() && !app.session.in_interaction());
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1);
        let lighter = vectorcraft_color::Color::from_hex(&fill_hex(&app, id)).unwrap().to_rgb()[0];
        assert!(lighter > 0.6, "{lighter}");
    }

    #[test]
    fn global_mode_shifts_the_tints_of_global_colours() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app.run("swatch.new", json!({"name": "Ink", "color": {"c": 0, "m": 1, "y": 0, "k": 0}, "spot": true})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
        app.run("paint.setFill", json!({"swatch": "Ink", "tint": 50})).unwrap();
        app.run("ui.colorBalanceDialog", json!({})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("mode".into(), json!("global"));
        d.fields.insert("tint".into(), json!(-20));
        frame(&mut app);
        assert_eq!(params(app.ui.dialog.as_ref().unwrap())["tint"], json!(-20.0));
        super::super::confirm(&mut app).unwrap();
        let n = app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().clone();
        let vectorcraft_color::Paint::Solid { swatch, tint, .. } = n.appearance.fill_paint() else { panic!() };
        assert_eq!(swatch.as_deref(), Some("Ink"), "the colour stays linked");
        assert!((tint - 0.3).abs() < 1e-6, "{tint}");
    }
}
