//! OpenType panel: per-character OpenType features (ligatures, alternates, swashes, small caps,
//! ordinals, fractions, figure style) for the selected text or the Type tool's selected characters.

use egui::Ui;
use serde_json::json;
use vectorcraft_text::OtFeatures;

use super::character::{text_editing, text_style};
use crate::VectorcraftApp;
use crate::widgets::{self, menu_item};

/// A checkbox row: label and the feature flag it toggles.
type Toggle = (&'static str, fn(&mut OtFeatures) -> &mut bool);

const FIGURES: [&str; 4] = ["Default Figure", "Tabular Lining", "Proportional Oldstyle", "Tabular Oldstyle"];

/// Apply `f` to the selection's features (the Type tool range, or whole objects).
fn set(app: &mut VectorcraftApp, f: OtFeatures) {
    let tags = f.to_tags();
    let r = match text_editing(app) {
        Some((id, a, b)) if b > a => app.run("text.setRangeStyle", json!({ "id": id.0, "start": a, "end": b, "features": tags })),
        Some((id, _, _)) => app.run("text.setRangeStyle", json!({ "id": id.0, "features": tags })),
        None => app.run("text.setStyle", json!({ "features": tags })),
    };
    if let Err(e) = r {
        app.ui.status = e;
    }
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some((st, _)) = text_style(app) else {
        widgets::dim_label(ui, tl!("Select text to set its OpenType features."));
        return;
    };
    let cur = OtFeatures::from_tags(st.features.iter().map(String::as_str));
    let mut next = cur;
    ui.horizontal(|ui| {
        widgets::dim_label(ui, tl!("Figure:"));
        let fig = match (cur.tabular_figures, cur.oldstyle_figures) {
            (false, false) => 0,
            (true, false) => 1,
            (false, true) => 2,
            (true, true) => 3,
        };
        if let Some(i) = widgets::dropdown(ui, "ot-figure", FIGURES[fig], &FIGURES, 170.0) {
            next.tabular_figures = i == 1 || i == 3;
            next.oldstyle_figures = i >= 2;
        }
    });
    widgets::divider(ui);
    let rows: [Toggle; 7] = [
        (tl!("Standard Ligatures"), |f| &mut f.ligatures),
        (tl!("Contextual Alternates"), |f| &mut f.contextual),
        (tl!("Discretionary Ligatures"), |f| &mut f.discretionary_ligatures),
        (tl!("Swash"), |f| &mut f.swash),
        (tl!("Small Caps"), |f| &mut f.small_caps),
        (tl!("Ordinals"), |f| &mut f.ordinals),
        (tl!("Fractions"), |f| &mut f.fractions),
    ];
    for (label, field) in rows {
        let on = *field(&mut next.clone());
        if widgets::check(ui, label, on, true) {
            *field(&mut next) = !on;
        }
    }
    if cur.ligatures && vectorcraft_text::ligatures_suppressed_by(st.tracking) && !vectorcraft_text::explicit_ligatures(&st.features) {
        widgets::dim_label(ui, tl!("Ligatures are off while the text is tracked."));
    }
    if next != cur {
        set(app, next);
    }
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    if menu_item(ui, tl!("Reset OpenType Features"), text_style(app).is_some(), false) {
        set(app, OtFeatures::default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggles_reach_the_text() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        let id = app.session.execute("text.create", &json!({"x": 10, "y": 40, "text": "fi 1/2"})).unwrap()["id"].clone();
        app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            show(&mut app, ui);
            menu(&mut app, ui);
        });
        out.textures_delta.clear();
        set(&mut app, OtFeatures { ligatures: false, fractions: true, ..Default::default() });
        assert_eq!(text_style(&app).unwrap().0.features, ["-liga", "frac"]);
    }
}
