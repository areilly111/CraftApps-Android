//! Type → Type on a Path → Type on a Path Options… (#429): the dialog shows the selected type's
//! options (Effect, Flip, Align to Path, Spacing) and OK leaves its brackets where they are.

use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind, TextKind};
use vectorcraft_engine::Session;

use crate::{VectorcraftApp, dialogs, menus};

/// The start and end brackets of type on a path `id`.
fn brackets(app: &VectorcraftApp, id: NodeId) -> (f64, Option<f64>) {
    match app.session.doc().unwrap().doc.node(id).map(|n| &n.kind) {
        Some(NodeKind::Text(t)) => match t.kind {
            TextKind::OnPath { start, end, .. } => (start, end),
            ref k => panic!("not type on a path: {k:?}"),
        },
        k => panic!("not text: {k:?}"),
    }
}

/// Open Type on a Path Options… from its menu item.
fn open_options(app: &mut VectorcraftApp) {
    let entry = menus::menu_entries(app).into_iter().find(|e| e.label == "Type on a Path Options…").expect("menu item");
    assert_eq!(entry.path, ["Type", "Type on a Path"]);
    assert!(entry.enabled);
    let (cmd, params) = menus::click_target(&entry.label, entry.command.as_deref().unwrap(), &entry.params);
    menus::invoke(app, &cmd, params);
}

#[test]
fn type_on_a_path_options_keep_the_type_where_it_is() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 600, "height": 400})).unwrap();
    let path = app.run("path.create", json!({"d": "M100 200 L500 200"})).unwrap()["id"].as_u64().unwrap();
    let r = app.run("text.createInPath", json!({"path": path, "mode": "onPath", "text": "Along", "at": [300, 210]})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    app.run("select.set", json!({"ids": [id.0]})).unwrap();
    let (start, _) = brackets(&app, id);
    assert!((start - 0.5).abs() < 1e-3, "{start}");

    open_options(&mut app);
    let d = app.ui.dialog.as_ref().expect("a dialog opens");
    assert_eq!((d.str("__command"), d.str("__label")), ("type.pathOptions".to_string(), "Type on a Path Options".to_string()));
    for (k, v) in [("effect", json!("rainbow")), ("flip", json!(false)), ("alignToPath", json!("baseline")), ("spacing", json!(0.0))] {
        assert_eq!(d.fields.get(k), Some(&v), "{k}");
    }
    assert!(!d.fields.contains_key("start") && !d.fields.contains_key("end"), "the brackets aren't the dialog's");
    // It draws headlessly: the choices by name.
    let text = crate::tests_labels::painted_text(&mut app, |app, ui| dialogs::show(app, ui.ctx()));
    for label in ["Type on a Path Options", "Effect:", "Rainbow", "Flip:", "Align to Path:", "Baseline", "Spacing:"] {
        assert!(text.contains(label), "{label} in {text}");
    }
    // OK as it opened: nothing moves.
    dialogs::confirm(&mut app).unwrap();
    assert_eq!(brackets(&app, id), (start, None));

    // Flip and Align to Path: the type turns to the other side of the same stretch of its path.
    open_options(&mut app);
    let d = app.ui.dialog.as_mut().unwrap();
    d.fields.insert("flip".into(), Value::Bool(true));
    d.fields.insert("alignToPath".into(), json!("center"));
    dialogs::confirm(&mut app).unwrap();
    let (s, e) = brackets(&app, id);
    assert!(s.abs() < 1e-9 && e.is_some_and(|e| (e - (1.0 - start)).abs() < 1e-9), "{s} {e:?}");
    // The dialog shows the new alignment, and Flip is unchecked again (it flips once more).
    open_options(&mut app);
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.fields.get("alignToPath"), d.fields.get("flip")), (Some(&json!("center")), Some(&json!(false))));
}
