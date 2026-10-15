//! Path dialogs: Average, Offset Path and Simplify (with Preview), and Split Into Grid.

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
    if matches!(d.kind.as_str(), "offsetPath" | "simplify") {
        let (id, params) = command(d);
        if d.fields.get(form::PREVIEWED) != Some(&params) || !d.bool("preview") {
            d.fields.remove("__counts");
        }
        let label = if d.kind == "simplify" { "Simplify" } else { "Offset Path" };
        if let Some(r) = form::preview_result(app, ui, d, label, id, params)
            && d.kind == "simplify"
        {
            d.fields.insert("__counts".into(), json!({"before": r["before"], "after": r["after"]}));
        }
        if d.kind == "simplify"
            && d.bool("preview")
            && let Some(counts) = d.fields.get("__counts")
            && let (Some(before), Some(after)) = (counts["before"].as_u64(), counts["after"].as_u64())
        {
            ui.label(format!("{}: {before} → {after}", tl!("Anchor Points")));
        }
    }
    false
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let (id, params) = command(d);
    if !matches!(d.kind.as_str(), "offsetPath" | "simplify") {
        return run_and_close(app, id, params);
    }
    let r = form::commit_preview(app, id, params)?;
    // The fields as the dialog held them, so they show as typed when it opens again.
    if d.kind == "offsetPath" {
        let kept = ["offset", "joins", "miterLimit"].into_iter().filter_map(|k| Some((k.to_string(), d.fields.get(k)?.clone())));
        app.ui.offset_path = Value::Object(kept.collect());
    }
    Ok(r)
}
