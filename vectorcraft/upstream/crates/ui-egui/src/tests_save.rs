//! The File menu around saving, in the app: Revert's confirmation, recent files, Show in Folder,
//! the single Document Color Mode entry and documents reopening at their saved view.

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_geom::Point;

use crate::{Services, VectorcraftApp, dialogs, io, menus, palette, theme};

/// An app that reads and writes real files.
fn app() -> VectorcraftApp {
    let services = Services {
        read: Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string()))),
        write: Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string()))),
        ..Default::default()
    };
    VectorcraftApp::new(Session::new(), services)
}

/// A fresh folder for one test.
fn dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("vc-ui-save-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn rect(app: &mut VectorcraftApp) {
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
}

fn objects(app: &VectorcraftApp) -> usize {
    app.session.doc().unwrap().doc.node_count()
}

/// One headless frame of the dialog layer with Enter pressed (OK).
fn press_ok(app: &mut VectorcraftApp) {
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    let key = egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
    let mut out = ctx.run_ui(egui::RawInput { events: vec![key], ..Default::default() }, |ui| dialogs::show(app, ui.ctx()));
    out.textures_delta.clear();
}

#[test]
fn revert_asks_then_reloads_in_the_same_tab_keeping_every_zoom() {
    let d = dir("revert");
    let path = d.join("a.vectorcraft").to_string_lossy().to_string();
    let mut app = app();
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    rect(&mut app);
    app.run("file.saveAs", json!({"path": path})).unwrap();
    assert!(!menus::enabled(&app, "file.revert"), "nothing changed since the save");
    app.run("file.new", json!({"width": 50, "height": 50})).unwrap();
    app.views[0].zoom = 2.0;
    app.views[1].zoom = 4.0;
    app.session.set_active(0);
    rect(&mut app);
    assert!(menus::enabled(&app, "file.revert"));
    let r = app.run("file.revert", json!({})).unwrap();
    assert_eq!(r["pending"], dialogs::confirm::KIND);
    assert_eq!(objects(&app), 3, "nothing happens before OK");
    press_ok(&mut app);
    assert!(app.ui.dialog.is_none());
    assert_eq!((objects(&app), app.session.active_index(), app.session.documents().len()), (2, Some(0), 2));
    assert!(!app.session.active().unwrap().is_dirty());
    assert_eq!((app.views[0].zoom, app.views[1].zoom), (2.0, 4.0));
    // Cancel leaves the changes.
    rect(&mut app);
    app.run("file.revert", json!({})).unwrap();
    app.ui.dialog = None;
    assert_eq!(objects(&app), 3);
    // An untitled document can't revert.
    app.session.set_active(1);
    rect(&mut app);
    assert!(app.run("file.revert", json!({})).is_err() && app.ui.dialog.is_none());
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn new_from_template_takes_bytes_from_agents() {
    let mut app = app();
    app.run("file.new", json!({})).unwrap();
    let bytes = app.session.execute("document.serialize", &json!({"format": "template"})).unwrap();
    let r = app.run("file.newFromTemplate", json!({"name": "flyer.vctemplate", "dataBase64": bytes["dataBase64"]})).unwrap();
    assert!(r["title"].as_str().is_some_and(|t| t.starts_with("Untitled-")), "{r}");
    assert_eq!((app.session.documents().len(), app.views.len()), (2, 2));
    assert_eq!(app.session.active().unwrap().path, None);
}

#[test]
fn recent_files_show_as_many_as_the_preference_keeps() {
    let mut app = app();
    for i in 0..35 {
        io::note_recent(&mut app, &format!("/art/{i}.svg"));
    }
    let listed = |app: &VectorcraftApp| {
        menus::menu_entries(app).iter().filter(|e| e.command.as_deref().is_some_and(|c| c.starts_with("file.openRecent"))).count()
    };
    assert_eq!((io::recent_files(&app).len(), listed(&app)), (20, 20), "the default");
    assert_eq!(menus::dynamic_label(&app, "file.openRecent1", ""), "34.svg");
    app.session.prefs.recent_files_count = 30;
    assert_eq!((io::recent_files(&app).len(), listed(&app)), (30, 30));
    assert!(menus::enabled(&app, "file.openRecent30"));
    app.session.prefs.recent_files_count = 0;
    assert_eq!((io::recent_files(&app).len(), listed(&app)), (0, 0), "0 hides the list");
    assert!(menus::shown_state(&app, "file.openRecent1", &json!(null)).is_none(), "the slots leave the menus");
    assert!(app.run("file.openRecent1", json!({})).is_err());
    app.session.prefs.recent_files_count = 5;
    assert_eq!(io::recent_files(&app)[0], "/art/34.svg", "hiding them didn't forget them");
}

#[test]
fn show_in_folder_needs_a_saved_document_and_a_file_manager() {
    let shown = Rc::new(RefCell::new(vec![]));
    let s = shown.clone();
    let mut app = app();
    app.run("file.new", json!({})).unwrap();
    assert!(!menus::enabled(&app, "file.reveal"));
    app.session.doc_mut().unwrap().path = Some("/art/a.vectorcraft".into());
    assert!(!menus::enabled(&app, "file.reveal"), "no file manager (web)");
    app.services.reveal = Some(Box::new(move |p: &str| {
        s.borrow_mut().push(p.to_string());
        Ok(())
    }));
    assert!(menus::enabled(&app, "file.reveal"));
    assert_eq!(app.run("file.reveal", json!({})).unwrap()["path"], "/art/a.vectorcraft");
    assert_eq!(*shown.borrow(), ["/art/a.vectorcraft"]);
}

#[test]
fn the_palette_lists_document_color_mode_once() {
    let items = palette::items();
    let ids: Vec<&str> = items.iter().map(|(_, id, _)| id.as_str()).filter(|id| id.to_ascii_lowercase().contains("documentcolormode")).collect();
    assert_eq!(ids, ["file.documentColorMode"]);
    let mut app = app();
    app.run("file.new", json!({})).unwrap();
    let entries = menus::menu_entries(&app);
    let modes: Vec<&str> =
        entries.iter().filter(|e| e.path.last().is_some_and(|p| p == "Document Color Mode")).filter_map(|e| e.command.as_deref()).collect();
    assert_eq!(modes, ["file.documentColorMode", "file.documentColorMode"], "CMYK and RGB run the one command");
}

#[test]
fn a_document_reopens_at_its_saved_view() {
    let d = dir("view");
    let path = d.join("v.vectorcraft").to_string_lossy().to_string();
    let mut app = app();
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    {
        let v = app.view_mut().unwrap();
        (v.zoom, v.center, v.rotation, v.fitted) = (3.0, Point::new(70.0, 40.0), 30.0, true);
    }
    app.run("file.saveAs", json!({"path": path})).unwrap();
    // Moving around afterwards isn't an edit.
    app.view_mut().unwrap().zoom = 0.5;
    assert!(!app.session.active().unwrap().is_dirty());
    io::open_path(&mut app, &path).unwrap();
    let v = *app.view().unwrap();
    assert_eq!((v.zoom, v.center, v.rotation, v.fitted), (3.0, Point::new(70.0, 40.0), 30.0, true));
    // A document without a saved view is fitted on first display.
    app.run("file.new", json!({})).unwrap();
    assert!(!app.view().unwrap().fitted);
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn replacing_a_file_that_opening_left_things_out_of_asks_first() {
    let d = dir("lossy");
    let path = d.join("a.vectorcraft").to_string_lossy().to_string();
    let mut app = app();
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    rect(&mut app);
    app.run("file.saveAs", json!({"path": path})).unwrap();
    // As if reading the file had left out text it can't read yet.
    let st = app.session.doc_mut().unwrap();
    st.imported_from = Some(path.clone());
    st.import_losses = vec!["hidden type".into()];
    let saved = std::fs::read(&path).unwrap();
    rect(&mut app);
    let r = app.run("file.save", json!({})).unwrap();
    assert_eq!(r["pending"], dialogs::confirm::KIND);
    let dialog = app.ui.dialog.as_ref().unwrap();
    assert!(dialog.str("detail").contains("“hidden type”"), "{:?}", dialog.fields);
    assert_eq!(std::fs::read(&path).unwrap(), saved, "nothing is written before OK");
    // Export As over it asks too; Cancel leaves the file.
    app.ui.dialog = None;
    assert_eq!(app.run("file.exportAs", json!({"path": path, "format": "svg"})).unwrap()["pending"], dialogs::confirm::KIND);
    app.ui.dialog = None;
    assert_eq!(std::fs::read(&path).unwrap(), saved);
    // Another name doesn't ask.
    let copy = d.join("b.svg").to_string_lossy().to_string();
    assert!(app.run("file.exportAs", json!({"path": copy, "format": "svg"})).unwrap().get("pending").is_none());
    crate::background::wait_all(&mut app);
    assert!(app.ui.dialog.is_none());
    // OK replaces it.
    app.run("file.save", json!({})).unwrap();
    press_ok(&mut app);
    crate::background::wait_all(&mut app);
    assert!(app.ui.dialog.is_none());
    assert_ne!(std::fs::read(&path).unwrap(), saved);
    assert!(!app.session.active().unwrap().is_dirty());
    let _ = std::fs::remove_dir_all(d);
}
