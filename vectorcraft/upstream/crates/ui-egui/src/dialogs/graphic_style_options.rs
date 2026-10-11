//! Graphic Style Options: the style's name. Opened on a style it renames it (`graphicStyle.rename`,
//! linked objects stay linked); opened for New Graphic Style it names the style made from the
//! selection (`graphicStyle.new`); opened for Merge Graphic Styles it names the style merged from
//! the selected ones (`graphicStyle.merge`).
//!
//! Fields: `name`, `__style` (the style being renamed; empty for a new style) and `__merge` (the
//! styles to merge, if merging).

use serde_json::{Value, json};

use super::swatch_options::{grid, label};
use super::{DialogSpec, form};
use crate::VectorcraftApp;
use crate::state::Dialog;

/// The dialog kind of Graphic Style Options.
pub const KIND: &str = "graphicStyleOptions";

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Graphic Style Options").into(), body, confirm, min_width: 320.0, ..DialogSpec::FORM };

/// Open Graphic Style Options for style `name`, or (`None`) to name a new style from the selection.
pub fn open(app: &mut VectorcraftApp, name: Option<&str>) -> Result<Value, String> {
    let d = &app.session.active().ok_or("no document open")?.doc;
    let name = match name {
        Some(n) => d.graphic_style(n).ok_or_else(|| format!("no graphic style `{n}`"))?.name.clone(),
        None => d.new_graphic_style_name(),
    };
    let style = if d.graphic_style(&name).is_some() { name.clone() } else { String::new() };
    app.ui.dialog = Some(Dialog::new(KIND, json!({ "name": name, "__style": style })));
    Ok(Value::Null)
}

/// Open Graphic Style Options to name the style Merge Graphic Styles makes of `names`.
pub fn open_merge(app: &mut VectorcraftApp, names: Vec<String>) -> Result<Value, String> {
    let d = &app.session.active().ok_or("no document open")?.doc;
    if names.len() < 2 {
        return Err("select two or more graphic styles to merge".into());
    }
    if let Some(n) = names.iter().find(|n| d.graphic_style(n).is_none()) {
        return Err(format!("no graphic style `{n}`"));
    }
    app.ui.dialog = Some(Dialog::new(KIND, json!({ "name": d.new_graphic_style_name(), "__style": "", "__merge": names })));
    Ok(Value::Null)
}

fn body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    grid(ui, |ui| {
        label(ui, tl!("Style Name:"));
        form::text(ui, d, "name", 190.0);
        ui.end_row();
    });
    false
}

/// OK: name the new or merged style, or rename the style. A name another style has keeps the
/// dialog open.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let (name, style) = (d.str("name"), d.str("__style"));
    let merge = d.fields.get("__merge").filter(|m| m.as_array().is_some_and(|a| !a.is_empty()));
    let r = match style.as_str() {
        "" if merge.is_some() => app.run("graphicStyle.merge", json!({ "names": merge, "name": name })),
        "" => app.run("graphicStyle.new", json!({ "name": name })),
        s if s == name => Ok(Value::Null),
        s => app.run("graphicStyle.rename", json!({ "name": s, "to": name })),
    };
    if r.is_ok() {
        app.ui.dialog = None;
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    fn app() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap();
        app
    }

    fn set(app: &mut VectorcraftApp, v: &str) {
        app.ui.dialog.as_mut().unwrap().fields.insert("name".into(), json!(v));
    }

    fn names(app: &VectorcraftApp) -> Vec<String> {
        app.session.active().unwrap().doc.graphic_styles.iter().map(|g| g.name.clone()).collect()
    }

    #[test]
    fn names_a_new_style_then_renames_it() {
        let mut app = app();
        app.run("ui.graphicStyleOptions", json!({})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("name"), "Graphic Style 5");
        // Drawn headlessly in the shared frame.
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(Default::default(), |ui| super::super::show(&mut app, ui.ctx()));
        out.textures_delta.clear();
        set(&mut app, "Glow");
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none() && names(&app).last().unwrap() == "Glow");

        app.run("ui.graphicStyleOptions", json!({"name": "Glow"})).unwrap();
        set(&mut app, "Sunshine");
        assert!(super::super::confirm(&mut app).is_err(), "names are unique");
        assert!(app.ui.dialog.is_some(), "a taken name keeps the dialog open");
        set(&mut app, "Halo");
        super::super::confirm(&mut app).unwrap();
        assert_eq!(names(&app).last().unwrap(), "Halo");
        assert!(app.run("ui.graphicStyleOptions", json!({"name": "nope"})).is_err());
    }
}
