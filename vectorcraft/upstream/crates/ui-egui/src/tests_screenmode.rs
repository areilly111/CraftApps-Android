//! Screen modes and Presentation Mode (#472): Escape leaves Presentation Mode, F cycles the three
//! screen modes, and no screen mode is saved with the preferences.

use serde_json::json;
use vectorcraft_engine::Session;

use crate::VectorcraftApp;

/// One headless frame of the whole window with `events`.
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) {
    let raw =
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))), events, ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| {
        app.logic(ui.ctx());
        app.ui(ui);
    });
    out.textures_delta.clear();
}

fn escape() -> Vec<egui::Event> {
    [true, false]
        .map(|pressed| egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed, repeat: false, modifiers: Default::default() })
        .to_vec()
}

#[test]
fn escape_leaves_presentation_mode() {
    let mut app = VectorcraftApp::new(Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.run("view.presentation", json!({})).unwrap(), json!(3));
    frame(&mut app, &ctx, vec![]);
    assert_eq!(crate::control::inspect(&app, &ctx)["screenMode"], json!(3), "agents see the screen mode");
    frame(&mut app, &ctx, escape());
    assert_eq!(app.ui.screen_mode, 0, "Escape is back in Normal Screen Mode");
    // Escape in Normal Screen Mode leaves it so.
    frame(&mut app, &ctx, escape());
    assert_eq!(app.ui.screen_mode, 0);
}

#[test]
fn f_cycles_the_screen_modes_and_leaves_presentation_mode() {
    let mut app = VectorcraftApp::new(Session::new(), crate::Services::default());
    let modes: Vec<u64> = (0..4).map(|_| app.run("view.screenMode", json!({})).unwrap().as_u64().unwrap()).collect();
    assert_eq!(modes, [1, 2, 0, 1]);
    app.run("view.presentation", json!({})).unwrap();
    assert_eq!(app.run("view.screenMode", json!({})).unwrap(), json!(0), "from Presentation Mode, F goes back to Normal");
}

#[test]
fn the_screen_mode_is_not_saved() {
    let mut app = VectorcraftApp::new(Session::new(), crate::Services::default());
    app.run("view.presentation", json!({})).unwrap();
    let saved = serde_json::to_value(&app.ui).unwrap();
    assert!(saved.get("screen_mode").is_none(), "{saved}");
    // Preferences an older version saved in Presentation Mode start in Normal Screen Mode.
    let old: crate::state::UiState = serde_json::from_value(json!({ "screen_mode": 3 })).unwrap();
    assert_eq!(old.screen_mode, 0);
}
