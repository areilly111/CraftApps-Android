//! The Links panel, its flyout, Link Info, Edit Original / Show in Folder, the Placement Options
//! dialog and the Properties panel's image section, drawn headless.

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::json;
use vectorcraft_engine::Session;

use crate::VectorcraftApp;
use crate::tests_labels::painted_text;

/// A fresh folder for one test (removed when dropped).
struct Folder(std::path::PathBuf);

impl Folder {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("vectorcraft-ui-links-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
    fn file(&self, name: &str, bytes: &[u8]) -> String {
        let p = self.0.join(name);
        std::fs::write(&p, bytes).unwrap();
        p.to_string_lossy().into_owned()
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn png(w: u32, h: u32) -> Vec<u8> {
    let mut out = vec![];
    image::RgbaImage::from_pixel(w, h, image::Rgba([200, 40, 40, 255]))
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

/// A document with a linked `linked.png` and an embedded `embedded.png` (the selection).
fn app(dir: &Folder) -> (VectorcraftApp, u64, u64) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let a = dir.file("linked.png", &png(300, 200));
    let b = dir.file("embedded.png", &png(40, 20));
    let linked = app.run("file.place", json!({"path": a, "at": [100, 100]})).unwrap()["ids"][0].as_u64().unwrap();
    let embedded = app.run("file.place", json!({"path": b, "at": [300, 200], "link": false})).unwrap()["ids"][0].as_u64().unwrap();
    (app, linked, embedded)
}

fn panel(app: &mut VectorcraftApp) -> String {
    painted_text(app, super::links::show)
}

#[test]
fn the_panel_lists_the_images_with_link_info_for_the_selected_one() {
    let dir = Folder::new("panel");
    let (mut app, linked, _) = app(&dir);
    let text = panel(&mut app);
    assert!(text.contains("linked.png") && text.contains("embedded.png"), "{text}");
    // The selection is the embedded image.
    assert!(text.contains("Embedded") && text.contains("Effective PPI"), "{text}");
    app.run("select.set", json!({"ids": [linked]})).unwrap();
    let text = panel(&mut app);
    assert!(text.contains("Linked") && text.contains("300 × 200 px") && text.contains("Location"), "{text}");
    // The flyout draws with every item.
    let menu = painted_text(&mut app, super::links::menu);
    for item in ["Relink to Folder…", "Embed Image(s)", "Placement Options…", "Show Missing", "Sort by Status", "Large Thumbnails", "Show in Folder"]
    {
        assert!(menu.contains(item), "{item}: {menu}");
    }
}

#[test]
fn an_empty_document_shows_how_to_add_images() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    assert!(panel(&mut app).contains("No images"));
}

#[test]
fn edit_original_and_show_in_folder_hand_the_file_to_the_system() {
    let dir = Folder::new("open");
    let (mut app, linked, embedded) = app(&dir);
    let opened: Rc<RefCell<Vec<(String, bool)>>> = Rc::default();
    let (o, r) = (opened.clone(), opened.clone());
    app.services.open_file = Some(Box::new(move |p: &str| {
        o.borrow_mut().push((p.to_string(), false));
        Ok(())
    }));
    app.services.reveal = Some(Box::new(move |p: &str| {
        r.borrow_mut().push((p.to_string(), true));
        Ok(())
    }));
    assert!(!crate::menus::enabled(&app, "links.editOriginal"), "the embedded image is selected");
    assert!(app.run("links.editOriginal", json!({})).is_err());
    app.run("select.set", json!({"ids": [linked, embedded]})).unwrap();
    assert!(crate::menus::enabled(&app, "links.editOriginal"));
    let r = app.run("links.editOriginal", json!({})).unwrap();
    let path = r["path"].as_str().unwrap().to_string();
    assert!(path.ends_with("linked.png"));
    app.run("links.reveal", json!({"id": linked})).unwrap();
    assert_eq!(*opened.borrow(), [(path.clone(), false), (path, true)]);
    // Without the services (the web): an error, nothing opened.
    (app.services.open_file, app.services.reveal) = (None, None);
    assert!(app.run("links.reveal", json!({"id": linked})).is_err());
    assert!(app.run("links.editOriginal", json!({"id": linked})).is_err());
}

#[test]
fn placement_options_dialog_sets_the_options() {
    let dir = Folder::new("dialog");
    let (mut app, linked, _) = app(&dir);
    app.run("select.set", json!({"ids": [linked]})).unwrap();
    assert!(crate::menus::enabled(&app, "ui.placementOptionsDialog"));
    app.run("ui.placementOptionsDialog", json!({})).unwrap();
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.kind.as_str(), d.str("preserve"), d.str("align")), ("placementOptions", "bounds".into(), "center".into()));
    let text = painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
    assert!(text.contains("Clip to Bounding Box") && text.contains("Preserve"), "{text}");
    let fields = &mut app.ui.dialog.as_mut().unwrap().fields;
    fields.insert("preserve".into(), json!("fit"));
    fields.insert("align".into(), json!("bottomRight"));
    crate::dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    let r = app.run("links.placementOptions", json!({"ids": [linked]})).unwrap();
    assert_eq!(r["placement"], json!({"preserve": "fit", "align": "bottomRight", "clip": false}));
}

#[test]
fn properties_shows_the_selected_image_and_its_link_actions() {
    let dir = Folder::new("props");
    let (mut app, linked, _) = app(&dir);
    let text = painted_text(&mut app, super::properties::show);
    assert!(text.contains("Embedded Image") && text.contains("embedded.png") && text.contains("Unembed…"), "{text}");
    app.run("select.set", json!({"ids": [linked]})).unwrap();
    let text = painted_text(&mut app, super::properties::show);
    assert!(text.contains("Linked File") && text.contains("Edit Original") && text.contains("Embed"), "{text}");
}

#[test]
fn the_links_menu_items_run_commands() {
    let tree = crate::menus::menu_tree();
    let ids = |label: &str| {
        let (_, items) = tree.iter().find(|(t, _)| *t == label).unwrap();
        format!("{items:?}")
    };
    assert!(ids("Edit").contains("links.editOriginal"));
    assert!(ids("Window").contains(super::links::ID));
}
