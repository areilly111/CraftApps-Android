//! Delete and Backspace with the Width and Puppet Warp tools, through the shortcut routing: the
//! selected width point or pin goes, and with nothing of the tool's selected the keys clear the
//! selected art as usual.

use egui::{Key, Modifiers};
use serde_json::{Value, json};
use vectorcraft_engine::Session;
use vectorcraft_tools::{PointerEvent, PointerKind};

use crate::VectorcraftApp;

/// A 300×300 document with a selected 10 pt line from (100, 200) to (300, 200) → its id.
fn app_with_line() -> (VectorcraftApp, u64) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    let id = app.run("shape.line", json!({"x1": 100, "y1": 200, "x2": 300, "y2": 200})).unwrap()["id"].as_u64().unwrap();
    app.run("stroke.set", json!({"weight": 10})).unwrap();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    (app, id)
}

/// One frame pressing `key` through the shortcut handler.
fn press(app: &mut VectorcraftApp, key: Key) {
    let ctx = egui::Context::default();
    let ev = egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
    let mut out = ctx.run_ui(egui::RawInput { events: vec![ev], ..Default::default() }, |ui| crate::shortcuts::handle(app, ui.ctx()));
    out.textures_delta.clear();
}

fn click(app: &mut VectorcraftApp, x: f64, y: f64) {
    let v = app.view_info();
    for kind in [PointerKind::Down, PointerKind::Up] {
        app.session.pointer(&PointerEvent::new(kind, x, y), v).unwrap();
    }
}

fn exists(app: &VectorcraftApp, id: u64) -> bool {
    app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).is_some()
}

fn width_points(app: &VectorcraftApp, id: u64) -> Value {
    let n = app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).cloned().unwrap();
    json!(n.appearance.stroke().unwrap().profile.as_ref().map(|p| p.points.len()))
}

#[test]
fn delete_and_backspace_remove_the_selected_width_point_then_clear() {
    for key in [Key::Delete, Key::Backspace] {
        let (mut app, id) = app_with_line();
        app.run("stroke.widthPoint.set", json!({"id": id, "t": 0.5, "left": 12, "right": 12})).unwrap();
        assert_eq!(width_points(&app, id), json!(3));
        app.select_tool("width");
        // A click on the width point's centre selects it; the key removes it, not the line.
        click(&mut app, 200.0, 200.0);
        press(&mut app, key);
        assert!(exists(&app, id), "{key:?} kept the line");
        assert_eq!(width_points(&app, id), json!(2), "{key:?} removed the width point");
        // Nothing of the tool's is selected now: the key clears the selected line.
        press(&mut app, key);
        assert!(!exists(&app, id), "{key:?} with no width point selected clears the selection");
    }
}

#[test]
fn delete_and_backspace_remove_the_selected_pin_then_clear() {
    for key in [Key::Delete, Key::Backspace] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 200, "height": 100})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [id]})).unwrap();
        app.select_tool("puppetWarp");
        let pins = |app: &VectorcraftApp| app.session.tool_options()["pins"].as_array().map_or(0, Vec::len);
        // A click on the art away from the automatic pins adds a pin and selects it.
        click(&mut app, 112.0, 188.0);
        let n = pins(&app);
        assert!(n > 1, "{n} pins");
        press(&mut app, key);
        assert!(exists(&app, id), "{key:?} kept the art");
        assert_eq!(pins(&app), n - 1, "{key:?} removed the pin");
        press(&mut app, key);
        assert!(!exists(&app, id), "{key:?} with no pin selected clears the selection");
    }
}
