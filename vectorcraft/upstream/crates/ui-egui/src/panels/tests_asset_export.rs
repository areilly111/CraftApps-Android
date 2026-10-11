//! The Asset Export panel drawn headless: its empty state, art dropped in (one asset per object,
//! or one with Alt), selecting and renaming assets, the shared export settings rows and Export.

use std::cell::RefCell;
use std::rc::Rc;

use egui::{Pos2, Rect, pos2, vec2};
use serde_json::{Value, json};
use vectorcraft_doc::NodeId;
use vectorcraft_engine::Session;

use super::asset_export::{ID, menu, show};
use crate::widgets::PanelDrag;
use crate::{Services, VectorcraftApp};

type Log = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

/// An app with two unstroked squares; `web` gives it a download service (recorded in the log).
fn app(web: bool) -> (VectorcraftApp, Log, [u64; 2]) {
    let log = Log::default();
    let l = log.clone();
    let mut services = Services::default();
    if web {
        services.download = Some(Box::new(move |name: &str, bytes: &[u8]| l.borrow_mut().push((name.to_string(), bytes.to_vec()))));
    }
    let mut app = VectorcraftApp::new(Session::new(), services);
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.session.paint.stroke = vectorcraft_color::Paint::None;
    let mut rect = |x: f64| app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 30, "height": 20})).unwrap()["id"].as_u64().unwrap();
    let ids = [rect(10.0), rect(100.0)];
    (app, log, ids)
}

/// One frame of the panel, 236 pt wide as in the dock → its texts and where they are.
fn frame(ctx: &egui::Context, app: &mut VectorcraftApp, events: Vec<egui::Event>, alt: bool) -> Vec<(String, Rect)> {
    fn texts(s: &egui::Shape, out: &mut Vec<(String, Rect)>) {
        match s {
            egui::Shape::Text(t) => out.push((t.galley.text().to_string(), t.visual_bounding_rect())),
            egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
            _ => {}
        }
    }
    let mut all = vec![egui::Event::ModifiersChanged(egui::Modifiers { alt, ..Default::default() })];
    all.extend(events);
    let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(236.0, 700.0))), events: all, ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| show(app, ui));
    out.textures_delta.clear();
    let mut v = vec![];
    out.shapes.iter().for_each(|c| texts(&c.shape, &mut v));
    v
}

fn click(at: Pos2) -> Vec<egui::Event> {
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    vec![egui::Event::PointerMoved(at), button(true), button(false)]
}

fn release(at: Pos2) -> Vec<egui::Event> {
    vec![
        egui::Event::PointerMoved(at),
        egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() },
    ]
}

/// Where text `label` was drawn.
fn at(texts: &[(String, Rect)], label: &str) -> Pos2 {
    texts
        .iter()
        .find(|(t, _)| t == label)
        .unwrap_or_else(|| panic!("no `{label}` in {:?}", texts.iter().map(|t| &t.0).collect::<Vec<_>>()))
        .1
        .center()
}

fn assets(app: &mut VectorcraftApp) -> Vec<Value> {
    app.run("assets.list", json!({})).unwrap()["assets"].as_array().unwrap().clone()
}

fn has(texts: &[(String, Rect)], label: &str) -> bool {
    texts.iter().any(|(t, _)| t == label)
}

#[test]
fn an_empty_panel_says_how_to_add_assets_and_shows_the_export_settings() {
    let (mut app, _, _) = app(false);
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    frame(&ctx, &mut app, vec![], false);
    let texts = frame(&ctx, &mut app, vec![], false);
    for label in ["No assets", "Export Settings", "Scale", "Suffix", "Format", "+ Add Scale", "Export"] {
        assert!(has(&texts, label), "{label}");
    }
    // The menu draws.
    let menu = crate::tests_labels::painted_text(&mut app, menu);
    for item in ["Add Selected Artwork as One Asset", "Remove Selected Assets", "Export for Screens…"] {
        assert!(menu.contains(item), "{item}: {menu}");
    }
    assert!(crate::state::ICON_PANELS.iter().any(|(id, label, _)| *id == ID && *label == "Asset Export"));
}

#[test]
fn art_dropped_on_the_panel_becomes_assets() {
    let (mut app, _, [a, b]) = app(false);
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    frame(&ctx, &mut app, vec![], false);
    egui::DragAndDrop::set_payload(&ctx, PanelDrag::Art(vec![NodeId(a), NodeId(b)]));
    frame(&ctx, &mut app, release(pos2(100.0, 50.0)), false);
    let list = assets(&mut app);
    assert_eq!(list.len(), 2, "one asset per object");
    // With Alt, one asset of them all.
    egui::DragAndDrop::set_payload(&ctx, PanelDrag::Art(vec![NodeId(a), NodeId(b)]));
    frame(&ctx, &mut app, release(pos2(100.0, 50.0)), true);
    let list = assets(&mut app);
    assert_eq!(list.len(), 3);
    assert_eq!(list[2]["nodes"], json!([a, b]));
    let texts = frame(&ctx, &mut app, vec![], false);
    assert!(has(&texts, "Asset 1") && has(&texts, "Asset 3"));
    // The new asset is the panel's selection.
    assert!(has(&texts, "Export 1 Selected"));
}

#[test]
fn the_plus_button_collects_the_selection_and_the_bin_removes_selected_assets() {
    let (mut app, _, [a, b]) = app(false);
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    frame(&ctx, &mut app, vec![], false);
    let texts = frame(&ctx, &mut app, vec![], false);
    // The + is the bottom bar's second icon from the right.
    let bar_y = at(&texts, "Export").y + 34.0;
    frame(&ctx, &mut app, click(pos2(236.0 - 40.0, bar_y)), false);
    assert_eq!(assets(&mut app).len(), 2, "+ adds one asset per object");
    let texts = frame(&ctx, &mut app, vec![], false);
    // Click one: it alone is selected; the bin removes it (the art stays).
    frame(&ctx, &mut app, click(at(&texts, "Asset 2")), false);
    let texts = frame(&ctx, &mut app, vec![], false);
    // The bin is the bottom bar's last icon.
    frame(&ctx, &mut app, click(pos2(236.0 - 13.0, at(&texts, "Export 1 Selected").y + 34.0)), false);
    let names: Vec<Value> = assets(&mut app).iter().map(|x| x["name"].clone()).collect();
    assert_eq!(names, [json!("Asset 1")]);
    assert!(app.session.active().unwrap().doc.node(NodeId(b)).is_some());
}

#[test]
fn renaming_an_asset_runs_assets_rename() {
    let (mut app, _, [a, _]) = app(false);
    let id = app.run("assets.add", json!({"ids": [a]})).unwrap()["assets"][0].as_u64().unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    frame(&ctx, &mut app, vec![], false);
    // Double-click starts editing the name (as typed here); Enter commits it.
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("asset-export-rename"), (id, "Logo".to_string())));
    frame(&ctx, &mut app, vec![], false);
    let enter = egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
    frame(&ctx, &mut app, vec![enter], false);
    assert_eq!(assets(&mut app)[0]["name"], "Logo");
}

#[test]
fn the_rows_are_the_shared_export_settings() {
    let (mut app, log, [a, b]) = app(true);
    app.run("assets.add", json!({"ids": [a, b]})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    frame(&ctx, &mut app, vec![], false);
    let texts = frame(&ctx, &mut app, vec![], false);
    // + Add Scale adds a 2x row to the document's export settings (Export for Screens' too).
    frame(&ctx, &mut app, click(at(&texts, "+ Add Scale")), false);
    let settings = app.run("document.exportSettings", json!({})).unwrap()["settings"].clone();
    assert_eq!(settings["formats"].as_array().unwrap().len(), 2);
    assert_eq!(settings["formats"][1]["scale"], "2x");
    app.run("file.exportForScreens", json!({})).unwrap();
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["formats"][1]["scale"], "2x");
    crate::dialogs::cancel(&mut app);
    // A preset set by an agent shows as its rows.
    app.run("assets.settings.set", json!({"preset": "mobile"})).unwrap();
    let texts = frame(&ctx, &mut app, vec![], false);
    assert!(has(&texts, "Mobile 1x/2x/3x") && has(&texts, "@3x"));
    // Export (nothing selected: every asset) downloads one zip on the web.
    let texts = frame(&ctx, &mut app, vec![], false);
    frame(&ctx, &mut app, click(at(&texts, "Export")), false);
    let log = log.borrow();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].0, "Untitled-1.zip");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn export_writes_into_the_picked_folder() {
    let dir = std::env::temp_dir().join(format!("vc-asset-panel-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let folder = dir.to_string_lossy().replace('\\', "/");
    let (mut app, _, [a, _]) = app(false);
    let picked = folder.clone();
    app.services.pick_folder = Some(Box::new(move || Some(picked.clone())));
    app.run("assets.add", json!({"ids": [a], "name": "Badge"})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    frame(&ctx, &mut app, vec![], false);
    let texts = frame(&ctx, &mut app, vec![], false);
    frame(&ctx, &mut app, click(at(&texts, "Export")), false);
    assert!(dir.join("Badge.png").is_file(), "{:?}", std::fs::read_dir(&dir).map(|d| d.count()));
    let _ = std::fs::remove_dir_all(&dir);
}
