//! The W/H link (Constrain Width and Height Proportions) in the Transform panel, the Properties
//! panel's Transform section and the Control bar: one preference, passed on as `proportional`.

use egui::{Event, Key, Modifiers, PointerButton};
use serde_json::json;

use super::tests_appearance::{frame_raw, run};
use super::*;

/// What draws a set of size fields.
type Draw = fn(&mut VectorcraftApp, &mut Ui);

/// A selected 100 × 50 pt rectangle.
fn rect_app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    run(&mut app, "file.new", json!({"width": 400, "height": 400}));
    let id = run(&mut app, "shape.rectangle", json!({"x": 10, "y": 10, "width": 100, "height": 50}))["id"].clone();
    run(&mut app, "select.set", json!({ "ids": [id] }));
    app
}

fn size(app: &VectorcraftApp) -> (f64, f64) {
    let st = app.session.active().unwrap();
    let b = st.doc.bounds_of(&st.selection.objects, false).unwrap();
    (b.width().round(), b.height().round())
}

/// Type `value` into the W field (the one showing "100 pt") of what `draw` shows, and press Enter.
fn set_width(app: &mut VectorcraftApp, draw: Draw, value: &str) {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut time = 0.0;
    let mut frame = |app: &mut VectorcraftApp, events: Vec<Event>| {
        time += 1.0;
        frame_raw(&ctx, app, egui::RawInput { events, time: Some(time), ..Default::default() }, draw)
    };
    let shown = frame(app, vec![]);
    let at = shown.iter().find(|(s, _)| s == "100 pt").unwrap_or_else(|| panic!("no W field in {shown:?}")).1.center();
    let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    frame(app, vec![Event::PointerMoved(at), button(true), button(false)]);
    // Focus selected the whole field: typing replaces it.
    frame(app, vec![]);
    frame(app, vec![Event::Text(value.into())]);
    let enter = Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
    frame(app, vec![enter]);
}

#[test]
fn the_link_keeps_proportions_in_every_size_field() {
    let surfaces: [(&str, Draw); 3] =
        [("Transform panel", transform::show), ("Properties", properties::transform_section), ("Control bar", crate::chrome::control_bar)];
    for (name, draw) in surfaces {
        // Off: W alone.
        let mut app = rect_app();
        set_width(&mut app, draw, "200");
        assert_eq!(size(&app), (200.0, 50.0), "{name}, unlinked");
        // On: H follows.
        let mut app = rect_app();
        run(&mut app, "prefs.set", json!({"key": "constrainProportions", "value": true}));
        set_width(&mut app, draw, "200");
        assert_eq!(size(&app), (200.0, 100.0), "{name}, linked");
    }
}

#[test]
fn the_link_is_a_preference_agents_can_read_and_set() {
    let mut app = rect_app();
    assert_eq!(run(&mut app, "prefs.get", json!({"key": "constrainProportions"})), json!(false));
    run(&mut app, "prefs.set", json!({"key": "constrainProportions", "value": true}));
    assert!(app.session.prefs.constrain_proportions);
    // What the fields send with the link on.
    run(&mut app, "object.setBounds", json!({"height": 100, "reference": 4, "proportional": true}));
    assert_eq!(size(&app), (200.0, 100.0));
}
