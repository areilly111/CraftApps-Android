//! Effect → Document Raster Effects Settings: resolution, colour model, background, and the
//! anti-alias, clipping mask, room around the art and spot colour options.
//!
//! Fields: what `document.rasterEffectsSettings` reports (and takes back). OK runs it with them:
//! one undo step. On an error the dialog stays open.

use serde_json::{Value, json};

use super::DialogSpec;
use super::document_setup::{LABEL, check, choice};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

pub(super) const KIND: &str = "rasterEffectsSettings";
const CMD: &str = "document.rasterEffectsSettings";
/// The resolution presets: (ppi, label).
const RESOLUTIONS: [(f64, &str); 3] = [(72.0, "Screen (72 ppi)"), (150.0, "Medium (150 ppi)"), (300.0, "High (300 ppi)")];

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Document Raster Effects Settings").into(),
    body,
    confirm,
    min_width: 440.0,
    max_width: Some(440.0),
    ..DialogSpec::FORM
};

/// Open the dialog on the active document's settings.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let fields = app.session.execute(CMD, &json!({})).map_err(|e| e.to_string())?;
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(Value::Null)
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let r = app.run(CMD, Value::Object(d.fields.clone()));
    if r.is_ok() {
        app.ui.dialog = None;
    }
    r
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    // Resolution: a preset, or Other with its own value.
    let ppi = d.f64("resolution", 72.0);
    let preset = RESOLUTIONS.iter().position(|(p, _)| (*p - ppi).abs() < 1e-9);
    let mut labels: Vec<&str> = RESOLUTIONS.iter().map(|(_, l)| tl!(l)).collect();
    labels.push(tl!("Other"));
    widgets::label_row(ui, tl!("Resolution:"), LABEL, |ui| {
        if let Some(i) = widgets::dropdown(ui, "raster-resolution", labels[preset.unwrap_or(RESOLUTIONS.len())], &labels, 170.0) {
            // Other keeps the current value to edit.
            if let Some((p, _)) = RESOLUTIONS.get(i) {
                d.fields.insert("resolution".into(), json!(p));
            } else if preset.is_some() {
                d.fields.insert("resolution".into(), json!(ppi.round() + 1.0));
            }
        }
        if preset.is_none()
            && let Some(v) = widgets::plain_field(ui, "raster-ppi", ppi, " ppi", 0, 80.0)
        {
            d.fields.insert("resolution".into(), json!(v));
        }
    });
    ui.add_space(4.0);
    let own =
        if app.session.active().is_some_and(|s| s.doc.color_mode == vectorcraft_doc::ColorMode::Cmyk) { ("cmyk", "CMYK") } else { ("rgb", "RGB") };
    choice(ui, d, "colorModel", tl!("Color Model:"), &[own, ("grayscale", tl!("Grayscale")), ("bitmap", tl!("Bitmap"))]);
    ui.add_space(6.0);
    widgets::label_row(ui, tl!("Background:"), LABEL, |ui| {
        let cur = d.str("background");
        for (id, label) in [("white", tl!("White")), ("transparent", tl!("Transparent"))] {
            if widgets::radio(ui, label, cur == id, true) {
                d.fields.insert("background".into(), json!(id));
            }
            ui.add_space(12.0);
        }
    });
    ui.add_space(10.0);
    widgets::subheader(ui, tl!("Options"));
    check(ui, d, "antiAlias", tl!("Anti-alias"));
    check(ui, d, "clippingMask", tl!("Create Clipping Mask"));
    let unit = app.session.general_unit();
    let t = crate::theme::Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("Add")).color(t.text));
        if let Some(v) = widgets::num_field(ui, "raster-add-around", Some(d.f64("addAround", 0.0)), unit, 80.0) {
            d.fields.insert("addAround".into(), json!(v));
        }
        ui.label(egui::RichText::new(tl!("Around Object")).color(t.text));
    });
    check(ui, d, "preserveSpotColors", tl!("Preserve Spot Colors"));
    ui.add_space(8.0);
    super::form::caption(ui, tl!("Changing these settings changes how raster effects look when exported, flattened or expanded."));
    false
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vectorcraft_engine::Session;

    use crate::VectorcraftApp;

    fn frame(app: &mut VectorcraftApp) {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        crate::theme::apply(&ctx, Default::default());
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 900.0))), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| crate::dialogs::show(app, ui.ctx()));
        out.textures_delta.clear();
    }

    #[test]
    fn raster_effects_settings_draw_and_ok_is_one_undo_step() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        crate::menus::invoke(&mut app, "document.rasterEffectsSettings", json!({}));
        let d = app.ui.dialog.as_mut().unwrap();
        assert_eq!((d.kind.as_str(), d.fields["colorModel"].as_str()), (super::KIND, Some("rgb")));
        for (k, v) in [("resolution", json!(300)), ("background", json!("white")), ("addAround", json!(36)), ("colorModel", json!("grayscale"))] {
            d.fields.insert(k.into(), v);
        }
        frame(&mut app);
        // Other resolutions show their own field.
        app.ui.dialog.as_mut().unwrap().fields.insert("resolution".into(), json!(200));
        frame(&mut app);
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let st = app.session.active().unwrap();
        let r = &st.doc.raster_effects;
        assert_eq!((st.doc.raster_effects_ppi, r.add_around, r.background), (200.0, 36.0, vectorcraft_doc::Background::White));
        assert_eq!(r.color_model, vectorcraft_doc::RasterColorModel::Grayscale);
        assert_eq!(st.history.undo.len(), 1);
        // A bad value keeps the dialog open.
        app.run("ui.rasterEffectsSettingsDialog", json!({})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("addAround".into(), json!(-5));
        assert!(crate::dialogs::confirm(&mut app).is_err());
        assert!(app.ui.dialog.is_some());
        crate::dialogs::cancel(&mut app);
        // Rasterize… starts from these settings.
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
        crate::menus::invoke(&mut app, "object.rasterize", json!({}));
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!(
            (d.fields["ppi"].as_f64(), d.fields["background"].as_str(), d.fields["addAround"].as_f64()),
            (Some(200.0), Some("white"), Some(36.0))
        );
        crate::dialogs::confirm(&mut app).unwrap();
        let st = app.session.active().unwrap();
        assert!(st.selection.objects.iter().any(|id| matches!(st.doc.node(*id).map(|n| &n.kind), Some(vectorcraft_doc::NodeKind::Image(_)))));
    }
}
