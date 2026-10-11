//! The Home icon shows the Home screen over open documents; Cmd+N opens the New Document dialog.

use serde_json::json;

use crate::{VectorcraftApp, canvas};

fn app_with_doc() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({})).unwrap();
    app
}

/// Draw one canvas frame; whether it drew the document (else the Home screen).
fn shows_document(app: &mut VectorcraftApp) -> bool {
    app.canvas_rect = None;
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))), ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| canvas::show(app, ui));
    out.textures_delta.clear();
    app.canvas_rect.is_some()
}

#[test]
fn home_shows_over_open_documents_until_a_document_is_chosen() {
    let mut app = app_with_doc();
    assert!(shows_document(&mut app));
    app.run("app.home", json!({})).unwrap();
    assert!(!shows_document(&mut app), "Home replaces the canvas");
    assert_eq!(app.session.documents().len(), 1, "the document stays open");
    assert!(app.ui.dialog.is_none(), "Home is not the New Document dialog");
    // Choosing the document's tab returns to it.
    app.ui.home = None;
    assert!(shows_document(&mut app));
    // A new document (from Home's presets, New… or Open) replaces Home too.
    app.run("app.home", json!({})).unwrap();
    app.run("file.new", json!({})).unwrap();
    assert!(shows_document(&mut app));
    assert!(app.ui.home.is_none());
}

/// Draw one canvas frame; whether it drew any text (the Home screen's labels; an empty window
/// draws none).
fn draws_text(app: &mut VectorcraftApp) -> bool {
    fn has_text(s: &egui::Shape) -> bool {
        match s {
            egui::Shape::Text(_) => true,
            egui::Shape::Vec(v) => v.iter().any(has_text),
            _ => false,
        }
    }
    app.canvas_rect = None;
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))), ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| canvas::show(app, ui));
    out.textures_delta.clear();
    out.shapes.iter().any(|c| has_text(&c.shape))
}

/// General › Show The Home Screen When No Documents Are Open (#394): off, an app with no document
/// shows an empty window instead of the Home screen, and the Home button still opens it.
#[test]
fn home_screen_without_documents_follows_its_preference() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    assert!(app.session.active().is_none());
    assert!(draws_text(&mut app), "Home with no document open (the default)");
    assert!(crate::menus::home_showing(&app));
    app.session.execute("prefs.set", &json!({"key": "showHomeScreen", "value": false})).unwrap();
    assert!(!draws_text(&mut app), "off: an empty window");
    assert!(!shows_document(&mut app));
    assert!(!crate::menus::home_showing(&app), "the Home button isn't lit");
    app.run("app.home", json!({})).unwrap();
    assert!(draws_text(&mut app), "the Home button still shows the Home screen");
    app.run("file.new", json!({})).unwrap();
    assert!(shows_document(&mut app));
    assert!(app.ui.home.is_none());
    app.run("file.close", json!({})).unwrap();
    assert!(app.session.active().is_none());
    assert!(!draws_text(&mut app), "closing the last document leaves the window empty");
    app.session.execute("prefs.set", &json!({"key": "showHomeScreen", "value": true})).unwrap();
    assert!(draws_text(&mut app), "on again: Home");
}

#[test]
fn cmd_n_opens_the_new_document_dialog() {
    assert_eq!(crate::shortcut_editor::command_for_key("Cmd+N"), Some("file.newDialog"));
    let mut app = app_with_doc();
    app.run("file.newDialog", json!({})).unwrap();
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some("newDocument"));
    assert_eq!(app.session.documents().len(), 1, "no document is made until the dialog's OK");
}

/// One Home screen frame (no document open) with `events`: the texts drawn and where.
fn home_texts(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<(String, egui::Rect)> {
    fn walk(s: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
        match s {
            egui::Shape::Text(t) => out.push((t.galley.text().to_string(), t.visual_bounding_rect())),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let raw =
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 900.0))), events, ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| canvas::show(app, ui));
    out.textures_delta.clear();
    let mut texts = vec![];
    out.shapes.iter().for_each(|c| walk(&c.shape, &mut texts));
    texts
}

/// The Home screen lists the first recent files (#663), each by name and folder; a click opens it,
/// and one that can't be opened says so. With none, there is no Recent Files section.
#[test]
fn home_lists_recent_files_and_a_click_opens_one() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let has = |texts: &[(String, egui::Rect)], s: &str| texts.iter().any(|(t, _)| t == s);
    assert!(!has(&home_texts(&mut app, &ctx, vec![]), "Recent Files"), "none: no section");
    let dir = std::env::temp_dir().join("vectorcraft-home-recent");
    let missing = dir.join("Gone.svg").to_string_lossy().into_owned();
    app.ui.recent_files = (0..9).map(|i| dir.join(format!("Drawing {i}.svg")).to_string_lossy().into_owned()).collect();
    app.ui.recent_files.insert(0, missing);
    let texts = home_texts(&mut app, &ctx, vec![]);
    assert!(has(&texts, "Recent Files") && has(&texts, "Gone.svg") && has(&texts, "Drawing 4.svg"), "{texts:?}");
    assert!(!has(&texts, "Drawing 5.svg"), "the first six only");
    assert!(has(&texts, &dir.to_string_lossy()), "with its folder");
    let at = texts.iter().find(|(t, _)| t == "Gone.svg").map(|(_, r)| r.center()).unwrap();
    let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    home_texts(&mut app, &ctx, vec![egui::Event::PointerMoved(at), press(true)]);
    home_texts(&mut app, &ctx, vec![press(false)]);
    assert!(app.session.documents().is_empty());
    assert!(!app.ui.status.is_empty(), "a file that can't be opened says so");
}
