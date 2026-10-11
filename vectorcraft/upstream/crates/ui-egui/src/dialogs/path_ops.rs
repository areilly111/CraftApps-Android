//! Path dialogs: Average, Offset Path (with Preview), Simplify and Split Into Grid.

use serde_json::{Value, json};

use super::{DialogSpec, form, run_and_close};
use crate::VectorcraftApp;
use crate::state::Dialog;

pub(super) const SPEC: DialogSpec = DialogSpec { heading: |d| title(&d.kind).into(), body, confirm, preview: true, ..DialogSpec::FORM };

fn title(kind: &str) -> &'static str {
    match kind {
        "average" => tl!("Average"),
        "offsetPath" => tl!("Offset Path"),
        "simplify" => tl!("Simplify"),
        _ => tl!("Split Into Grid"),
    }
}

/// The command the dialog runs and its parameters.
fn command(d: &Dialog) -> (&'static str, Value) {
    match d.kind.as_str() {
        "average" => ("path.average", json!({"axis": d.str("axis")})),
        "offsetPath" => {
            ("object.path.offsetPath", json!({"offset": d.f64("offset", 10.0), "joins": d.str("joins"), "miterLimit": d.f64("miterLimit", 4.0)}))
        }
        "simplify" => ("object.path.simplify", json!({"tolerance": d.f64("tolerance", 1.0)})),
        _ => ("object.path.splitIntoGrid", json!({"rows": d.f64("rows", 2.0), "columns": d.f64("columns", 2.0), "gutter": d.f64("gutter", 12.0)})),
    }
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    form::grid(ui, d, app.session.general_unit());
    if d.kind == "offsetPath" {
        let (id, params) = command(d);
        form::preview(app, ui, d, "Offset Path", id, params);
    }
    false
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let (id, params) = command(d);
    if d.kind == "offsetPath" { form::commit_preview(app, id, params) } else { run_and_close(app, id, params) }
}
