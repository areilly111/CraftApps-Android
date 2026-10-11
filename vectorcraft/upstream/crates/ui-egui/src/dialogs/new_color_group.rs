//! New Color Group: a name and what the group is made from, the swatches selected in the Swatches
//! panel or the colours of the selected artwork (whose process colours can become global swatches
//! the art links to, with swatches for tints too). OK runs `swatch.newGroup`.
//!
//! Fields: `name`, `fromArtwork`, `toGlobal`, `includeTints`, `swatches` (the selected swatch
//! names) and `__art` (artwork is selected).

use serde_json::{Value, json};

use super::swatch_options::{grid, label};
use super::{DialogSpec, form, run_and_close};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of New Color Group.
pub const KIND: &str = "newColorGroup";

pub(super) const SPEC: DialogSpec = DialogSpec { heading: |_| tl!("New Color Group").into(), body, confirm, min_width: 320.0, ..DialogSpec::FORM };

/// Open New Color Group for the swatches selected in the panel. It starts from the artwork when
/// art is selected and no swatches are.
pub fn open(app: &mut VectorcraftApp, swatches: Vec<String>) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document open")?;
    let art = !st.selection.is_empty();
    let fields = json!({
        "name": st.doc.free_swatch_name("Color Group"),
        "fromArtwork": art && swatches.is_empty(),
        "toGlobal": true,
        "includeTints": false,
        "swatches": swatches,
        "__art": art,
    });
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(Value::Null)
}

fn body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let art = d.bool("fromArtwork");
    let set = |d: &mut Dialog, key: &str, v: bool| {
        d.fields.insert(key.into(), json!(v));
    };
    grid(ui, |ui| {
        label(ui, tl!("Name:"));
        form::text(ui, d, "name", 190.0);
        ui.end_row();
        label(ui, tl!("Create From:"));
        if widgets::radio(ui, tl!("Selected Swatches"), !art, true) {
            set(d, "fromArtwork", false);
        }
        ui.end_row();
        ui.label("");
        if widgets::radio(ui, tl!("Selected Artwork"), art, d.bool("__art")) {
            set(d, "fromArtwork", true);
        }
        ui.end_row();
        for (key, text) in [("toGlobal", tl!("Convert Process to Global")), ("includeTints", tl!("Include Swatches for Tints"))] {
            ui.label("");
            ui.horizontal(|ui| {
                ui.add_space(18.0);
                let on = d.bool(key);
                if widgets::check(ui, text, on, art) {
                    set(d, key, !on);
                }
            });
            ui.end_row();
        }
    });
    false
}

/// `swatch.newGroup` parameters from the fields.
fn params(d: &Dialog) -> Value {
    if d.bool("fromArtwork") {
        json!({"name": d.str("name"), "fromArtwork": true, "toGlobal": d.bool("toGlobal"), "includeTints": d.bool("includeTints")})
    } else {
        json!({"name": d.str("name"), "swatches": d.fields.get("swatches").cloned().unwrap_or_else(|| json!([]))})
    }
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    run_and_close(app, "swatch.newGroup", params(d))
}
