//! Eyedropper Options (double-click the Eyedropper tool): the raster sample size and the Picks Up /
//! Applies trees of attributes. OK runs `eyedropper.setOptions` (the options live in the
//! preferences).
//!
//! Fields: `sampleSize` (1, 3 or 5), `pickUp` and `apply` (`{appearance: {transparency, fill:
//! {color, transparency, overprint}, stroke: {color, transparency, overprint, weight, cap, join,
//! miter, dash}}, character, paragraph}`, each leaf a bool).

use serde_json::{Value, json};

use super::DialogSpec;
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Eyedropper Options.
pub const KIND: &str = "eyedropperOptions";

pub(super) const SPEC: DialogSpec = DialogSpec { heading: |_| tl!("Eyedropper Options").into(), body, confirm, min_width: 460.0, ..DialogSpec::FORM };

/// The raster sample sizes: (pixels square, label).
const SAMPLE_SIZES: [(u64, &str); 3] = [(1, "Point Sample"), (3, "3 x 3 Average"), (5, "5 x 5 Average")];

/// The rows of a Picks Up / Applies tree: (JSON pointer into the tree, label, depth).
const TREE: &[(&str, &str, u8)] = &[
    ("/appearance", "Appearance", 0),
    ("/appearance/transparency", "Transparency", 1),
    ("/appearance/fill", "Focal Fill", 1),
    ("/appearance/fill/color", "Color", 2),
    ("/appearance/fill/transparency", "Transparency", 2),
    ("/appearance/fill/overprint", "Overprint", 2),
    ("/appearance/stroke", "Focal Stroke", 1),
    ("/appearance/stroke/color", "Color", 2),
    ("/appearance/stroke/transparency", "Transparency", 2),
    ("/appearance/stroke/overprint", "Overprint", 2),
    ("/appearance/stroke/weight", "Weight", 2),
    ("/appearance/stroke/cap", "Cap", 2),
    ("/appearance/stroke/join", "Join", 2),
    ("/appearance/stroke/miter", "Miter Limit", 2),
    ("/appearance/stroke/dash", "Dash Pattern", 2),
    ("/character", "Character Style", 0),
    ("/paragraph", "Paragraph Style", 0),
];

/// Open the dialog with the current options.
pub fn open(app: &mut VectorcraftApp) {
    app.ui.dialog = Some(Dialog::new(KIND, json!(app.session.prefs.eyedropper)));
}

/// A flag or a branch of flags: `Some(on)` when every flag under it is `on`, `None` when mixed.
fn state(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::Object(o) => {
            let mut s = o.values().map(state);
            let first = s.next()??;
            s.all(|x| x == Some(first)).then_some(first)
        }
        _ => Some(false),
    }
}

/// Set every flag under `v` to `on`.
fn set_all(v: &mut Value, on: bool) {
    match v {
        Value::Object(o) => o.values_mut().for_each(|c| set_all(c, on)),
        _ => *v = Value::Bool(on),
    }
}

/// One tree (`key`: `pickUp` or `apply`) of checkboxes; a branch's box sets everything under it.
fn tree(ui: &mut egui::Ui, d: &mut Dialog, key: &str, title: &str) {
    ui.vertical(|ui| {
        ui.label(egui::RichText::new(tl!(title)).strong());
        ui.add_space(4.0);
        let Some(t) = d.fields.get_mut(key) else { return };
        for (path, label, depth) in TREE {
            let Some(node) = t.pointer_mut(path) else { continue };
            let s = state(node);
            ui.horizontal(|ui| {
                ui.add_space(*depth as f32 * 18.0);
                if widgets::check3(ui, label, s, true) {
                    set_all(node, s != Some(true));
                }
            });
        }
    });
}

fn body(_app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let size = d.fields.get("sampleSize").and_then(Value::as_u64).unwrap_or(1);
    let current = SAMPLE_SIZES.iter().find(|(n, _)| *n == size).map_or(SAMPLE_SIZES[0].1, |s| s.1);
    ui.horizontal(|ui| {
        widgets::dim_label(ui, tl!("Raster Sample Size:"));
        let labels = SAMPLE_SIZES.map(|s| s.1);
        if let Some(i) = widgets::dropdown(ui, "eyedropper-sample", current, &labels, 150.0) {
            d.fields.insert("sampleSize".into(), json!(SAMPLE_SIZES[i].0));
        }
    });
    ui.add_space(10.0);
    ui.horizontal_top(|ui| {
        tree(ui, d, "pickUp", tl!("Eyedropper Picks Up:"));
        ui.add_space(24.0);
        tree(ui, d, "apply", tl!("Eyedropper Applies:"));
    });
    false
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    super::run_and_close(app, "eyedropper.setOptions", Value::Object(d.fields.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    #[test]
    fn the_trees_set_the_options() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("tool.options", json!({"tool": "eyedropper"})).unwrap();
        let text = crate::tests_labels::painted_text(&mut app, |app, ui| super::super::show(app, ui.ctx()));
        for label in [
            "Eyedropper Options",
            "Raster Sample Size:",
            "Point Sample",
            "Eyedropper Picks Up:",
            "Eyedropper Applies:",
            "Focal Stroke",
            "Dash Pattern",
            "Paragraph Style",
        ] {
            assert!(text.contains(label), "{label} in {text}");
        }
        // A branch's box sets the flags under it; a mixed branch shows a dash.
        let d = app.ui.dialog.as_mut().unwrap();
        let t = d.fields.get_mut("apply").unwrap();
        set_all(t.pointer_mut("/appearance/stroke").unwrap(), false);
        assert_eq!(state(&t["appearance"]["stroke"]), Some(false));
        assert_eq!(state(&t["appearance"]), None);
        d.fields.insert("sampleSize".into(), json!(5));
        super::super::confirm(&mut app).unwrap();
        let o = app.session.prefs.eyedropper;
        assert_eq!(
            (o.sample_size, o.apply.appearance.stroke.dash, o.apply.appearance.fill.color, o.pick_up.appearance.stroke.dash),
            (5, false, true, true)
        );
        assert!(app.ui.dialog.is_none());
    }
}
