//! View → New View… / Edit Views…: saved views stored in the document; per-document view
//! toggles (Show Transparency Grid).

use serde_json::{Value, json};
use vectorcraft_doc::SavedView;
use vectorcraft_geom::Point;

use super::*;

/// Illustrator keeps up to 25 views per document.
pub const MAX_VIEWS: usize = 25;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "view.saved.new",
            "New View…",
            ["View"],
            None,
            "{name?, center: [x, y], zoom: (1 = 100%), rotation?: deg} save a view (the app fills in the current one) → {name, index}",
            has_doc,
            new_view
        ),
        cmd!(
            "view.saved.edit",
            "Edit Views…",
            ["View"],
            None,
            "{name, newName?, delete?: bool} rename or delete a saved view; no params → {views}",
            has_doc,
            edit_views
        ),
        cmd!(query "view.saved.list", "Saved Views", [], None, "{} → {views: [{name, center, zoom, rotation}]}", has_doc, list_views),
        cmd!(
            query "view.transparencyGrid",
            "Show Transparency Grid",
            ["View"],
            Some("Cmd+Shift+D"),
            "{on?: bool (default: toggle)} show the transparency grid behind the active document's artboards (each document has its own setting) → {on}",
            has_doc,
            transparency_grid
        ),
    ]
}

fn transparency_grid(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.doc_mut()?;
    st.transparency_grid = p.get("on").and_then(Value::as_bool).unwrap_or(!st.transparency_grid);
    Ok(json!({ "on": st.transparency_grid }))
}

fn list_views(s: &mut Session, _: &Value) -> Result<Value> {
    Ok(json!({ "views": s.doc()?.doc.views }))
}

fn new_view(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "view.saved.new";
    let [x, y] = p.get("center").and_then(finite_numbers::<2>).ok_or_else(|| bad(C, "center must be [x, y]"))?;
    let zoom = f64_req(p, "zoom", C)?;
    if !(zoom.is_finite() && zoom > 0.0) {
        return Err(bad(C, "zoom must be positive"));
    }
    // Given, the rotation is a number: nothing malformed falls back to 0°.
    let rotation = match p.get("rotation") {
        Some(r) => r.as_f64().filter(|r| r.is_finite()).ok_or_else(|| bad(C, "rotation must be a number of degrees"))?,
        None => 0.0,
    };
    let st = s.doc()?;
    if st.doc.views.len() >= MAX_VIEWS {
        return Err(bad(C, format!("a document keeps at most {MAX_VIEWS} views")));
    }
    let name = match str_param(p, "name").map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) => n.to_string(),
        None => (1..).map(|i| format!("View {i}")).find(|n| !st.doc.views.iter().any(|v| &v.name == n)).unwrap_or_default(),
    };
    if st.doc.views.iter().any(|v| v.name == name) {
        return Err(bad(C, format!("a view named `{name}` exists")));
    }
    let view = SavedView { name: name.clone(), center: Point::new(x, y), zoom, rotation };
    let index = s.edit("New View", |d, _| {
        d.views.push(view);
        Ok(d.views.len() - 1)
    })?;
    Ok(json!({ "name": name, "index": index }))
}

fn edit_views(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "view.saved.edit";
    let Some(name) = str_param(p, "name") else { return list_views(s, p) };
    let i = s.doc()?.doc.views.iter().position(|v| v.name == name).ok_or_else(|| bad(C, format!("no view named `{name}`")))?;
    if bool_or(p, "delete", false) {
        s.edit("Delete View", |d, _| {
            d.views.remove(i);
            Ok(())
        })?;
        return Ok(json!({ "deleted": name }));
    }
    let new = str_param(p, "newName").map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad(C, "give newName or delete"))?.to_string();
    if s.doc()?.doc.views.iter().any(|v| v.name == new) {
        return Err(bad(C, format!("a view named `{new}` exists")));
    }
    s.edit("Rename View", |d, _| {
        d.views[i].name = new.clone();
        Ok(())
    })?;
    Ok(json!({ "name": new }))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::Session;

    /// Malformed dimensions are rejected rather than saving an arbitrary or partial viewport.
    #[test]
    fn saved_views_require_exact_finite_coordinates_and_rotation() {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let before = s.doc().unwrap().history.undo.len();
        for params in [
            json!({"center": [10], "zoom": 2}),
            json!({"center": [10, 20, 30], "zoom": 2}),
            json!({"center": [10, "bad"], "zoom": 2}),
            json!({"center": [10, 20], "zoom": 2, "rotation": "bad"}),
            json!({"center": [10, 20], "zoom": 2, "rotation": null}),
        ] {
            assert!(s.execute("view.saved.new", &params).is_err(), "accepted {params}");
            assert!(s.doc().unwrap().doc.views.is_empty());
            assert_eq!(s.doc().unwrap().history.undo.len(), before);
        }
        s.execute("view.saved.new", &json!({"center": [10, 20], "zoom": 2, "rotation": -15})).unwrap();
        let views = &s.doc().unwrap().doc.views;
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].rotation, -15.0);
    }

    #[test]
    fn save_rename_delete_views() {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let v = s.execute("view.saved.new", &json!({"center": [100, 200], "zoom": 2.0})).unwrap();
        assert_eq!(v["name"], json!("View 1"));
        s.execute("view.saved.new", &json!({"name": "Logo", "center": [10, 20], "zoom": 4.0, "rotation": 15})).unwrap();
        assert!(s.execute("view.saved.new", &json!({"name": "Logo", "center": [0, 0], "zoom": 1.0})).is_err());
        assert!(s.execute("view.saved.new", &json!({"center": [0, 0], "zoom": 0})).is_err());
        s.execute("view.saved.edit", &json!({"name": "View 1", "newName": "Overview"})).unwrap();
        let l = s.execute("view.saved.list", &json!({})).unwrap();
        assert_eq!(l["views"][0]["name"], json!("Overview"));
        assert_eq!(l["views"][1]["rotation"], json!(15.0));
        s.execute("view.saved.edit", &json!({"name": "Logo", "delete": true})).unwrap();
        assert_eq!(s.doc().unwrap().doc.views.len(), 1);
        // Views travel with the file.
        let bytes = vectorcraft_format::save_file(&s.doc().unwrap().doc);
        assert_eq!(vectorcraft_format::load(&bytes).unwrap().views[0].name, "Overview");
    }
}
