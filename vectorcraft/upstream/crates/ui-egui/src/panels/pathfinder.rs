//! Pathfinder panel: Shape Modes (+ Expand) and Pathfinders, with icons drawn for VectorCraft.

use egui::Ui;
use serde_json::json;

use super::{pstate, selection_len, set_pstate};
use crate::VectorcraftApp;
use crate::widgets::{self, menu_item};

pub const SHAPE_MODES: [(&str, &str, &str); 4] = [
    ("dc-pf-unite", "Unite", "unite"),
    ("dc-pf-minus-front", "Minus Front", "minusFront"),
    ("dc-pf-intersect", "Intersect", "intersect"),
    ("dc-pf-exclude", "Exclude", "exclude"),
];

pub const PATHFINDERS: [(&str, &str, &str); 6] = [
    ("dc-pf-divide", "Divide", "divide"),
    ("dc-pf-trim", "Trim", "trim"),
    ("dc-pf-merge", "Merge", "merge"),
    ("dc-pf-crop", "Crop", "crop"),
    ("dc-pf-outline", "Outline", "outline"),
    ("dc-pf-minus-back", "Minus Back", "minusBack"),
];

fn run(app: &mut VectorcraftApp, ui: &Ui, op: &str, label: &str) {
    if app.run(&format!("object.pathfinder.{op}"), json!({})).is_ok() {
        set_pstate(ui.ctx(), "pf-last", Some((op.to_string(), label.to_string())));
    }
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let n = selection_len(app);
    let t = crate::theme::Tokens::get(ui.ctx());
    widgets::subheader(ui, tl!("Shape Modes:"));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        for (icon, tip, op) in SHAPE_MODES {
            if widgets::icon_button_enabled(ui, icon, tip, false, n >= 2, 34.0).clicked() {
                run(app, ui, op, tip);
            }
        }
        ui.add_enabled_ui(false, |ui| {
            let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width().min(90.0), 26.0), egui::Sense::hover());
            ui.painter().rect_filled(r, 2, t.hover.gamma_multiply(0.5));
            ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, tl!("Expand"), egui::FontId::proportional(12.5), t.text_disabled);
        })
        .response
        .on_disabled_hover_text(tl!("Expand applies to compound shapes (Alt-click a shape mode) — on the roadmap"));
    });
    widgets::subheader(ui, tl!("Pathfinders:"));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        for (icon, tip, op) in PATHFINDERS {
            let min = if op == "outline" || op == "divide" { 1 } else { 2 };
            if widgets::icon_button_enabled(ui, icon, tip, false, n >= min, 34.0).clicked() {
                run(app, ui, op, tip);
            }
        }
    });
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    menu_item(ui, tl!("Trap…"), false, false);
    let last: Option<(String, String)> = pstate(ui.ctx(), "pf-last");
    let label =
        last.as_ref().map(|(_, l)| crate::i18n::fmt(tl!("Repeat {name}"), &[("name", tl!(l))])).unwrap_or_else(|| tl!("Repeat Pathfinder").into());
    if menu_item(ui, &label, last.is_some() && selection_len(app) >= 1, false)
        && let Some((op, l)) = last
    {
        run(app, ui, &op, &l);
    }
    menu_item(ui, tl!("Pathfinder Options…"), false, false);
    ui.separator();
    menu_item(ui, tl!("Make Compound Shape"), false, false);
    menu_item(ui, tl!("Release Compound Shape"), false, false);
    menu_item(ui, tl!("Expand Compound Shape"), false, false);
}
