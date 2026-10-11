//! Recolor Artwork (M3.78) and Edit or Apply Color Group (M3.79), drawn headlessly.

use egui::{Event, Modifiers, PointerButton, Pos2, Rect, vec2};
use serde_json::{Value, json};
use vectorcraft_color::Color;
use vectorcraft_color::recolor::ColorKey;
use vectorcraft_engine::Session;

use crate::{VectorcraftApp, dialogs, panels};

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    app
}

fn context() -> egui::Context {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    ctx
}

fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>, time: f64, draw: fn(&mut VectorcraftApp, &mut egui::Ui)) {
    let screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(900.0, 800.0)));
    let input = egui::RawInput { events, time: Some(time), screen_rect, ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| draw(app, ui));
    out.textures_delta.clear();
}

fn dialog_frame(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let time = ctx.input(|i| i.time) + 1.0;
    frame(app, ctx, vec![], time, |app, ui| dialogs::show(app, ui.ctx()));
}

fn press(at: Pos2, clicks: usize) -> Vec<Event> {
    let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    std::iter::once(Event::PointerMoved(at)).chain((0..clicks).flat_map(|_| [button(true), button(false)])).collect()
}

/// Rectangles filled with `colors`, all selected.
fn art(app: &mut VectorcraftApp, colors: &[&str]) {
    for (i, c) in colors.iter().enumerate() {
        let id = app.run("shape.rectangle", json!({"x": 10 + 30 * i, "y": 10, "width": 20, "height": 20})).unwrap()["id"].clone();
        app.run("paint.setFill", json!({"ids": [id], "color": c})).unwrap();
        app.run("paint.setStroke", json!({"ids": [id], "none": true})).unwrap();
    }
    app.run("select.all", json!({})).unwrap();
}

fn colors(app: &mut VectorcraftApp) -> usize {
    app.session.execute("recolor.colors", &json!({})).unwrap()["colors"].as_array().unwrap().len()
}

fn group_keys(app: &VectorcraftApp, name: &str) -> Vec<Value> {
    let d = &app.session.doc().unwrap().doc;
    let g = d.swatch_groups.iter().find(|g| g.name == name).unwrap();
    g.swatches.iter().filter_map(|w| w.paint.color()).map(|c| json!(ColorKey::of(&c).to_string())).collect()
}

fn field(app: &VectorcraftApp, k: &str) -> Value {
    app.ui.dialog.as_ref().and_then(|d| d.fields.get(k).cloned()).unwrap_or(Value::Null)
}

fn set(app: &mut VectorcraftApp, k: &str, v: Value) {
    app.ui.dialog.as_mut().unwrap().fields.insert(k.into(), v);
}

#[test]
fn the_dialog_reduces_previews_and_applies_as_one_undo_step() {
    let mut app = app();
    let ctx = context();
    art(&mut app, &["#ff0000", "#e01010", "#0000ff", "#1010e0", "#ff3030"]);
    app.run("ui.recolorDialog", json!({})).unwrap();
    dialog_frame(&mut app, &ctx);
    assert_eq!(field(&app, "rows").as_array().unwrap().len(), 5, "Auto: a row per colour");
    assert!(app.session.in_interaction(), "the dialog previews");
    // Two colours, Exact; OK without another frame still reduces first.
    set(&mut app, "colors", json!(2));
    set(&mut app, "method", json!("exact"));
    let undo = app.session.doc().unwrap().history.undo.len();
    dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(colors(&mut app), 2);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(colors(&mut app), 5);
    // Cancel rolls the preview back.
    app.run("ui.recolorDialog", json!({"colors": 1})).unwrap();
    set(&mut app, "method", json!("exact"));
    dialog_frame(&mut app, &ctx);
    assert_eq!(colors(&mut app), 1, "previewed");
    dialogs::cancel(&mut app);
    assert_eq!(colors(&mut app), 5);
}

#[test]
fn presets_open_colour_jobs_and_libraries_and_both_tabs_draw() {
    let mut app = app();
    let ctx = context();
    art(&mut app, &["#ff0000", "#00ff00", "#0000ff"]);
    let entries = crate::menus::menu_entries(&app);
    let presets: Vec<(&str, &Value)> =
        entries.iter().filter(|e| e.path.last().is_some_and(|p| p == "Recolor with Preset")).map(|e| (e.label.as_str(), &e.params)).collect();
    assert_eq!(presets.iter().map(|p| p.0).collect::<Vec<_>>(), ["1 Color Job…", "2 Color Job…", "3 Color Job…", "Color Library…"]);
    app.run("ui.recolorDialog", presets[1].1.clone()).unwrap();
    dialog_frame(&mut app, &ctx);
    assert_eq!((field(&app, "colors"), field(&app, "method")), (json!(2), json!("scaleTints")));
    assert_eq!(field(&app, "rows").as_array().unwrap().len(), 2);
    set(&mut app, "tab", json!("edit"));
    dialog_frame(&mut app, &ctx);
    dialogs::cancel(&mut app);
    app.run("ui.recolorDialog", presets[3].1.clone()).unwrap();
    assert_eq!(field(&app, "limitTo"), "web-safe-216", "Color Library starts on the first library");
    dialog_frame(&mut app, &ctx);
    let web = |k: &Value| vectorcraft_engine::cmd::color_value(k).unwrap().to_rgb().iter().all(|v| ((v * 5.0).round() - v * 5.0).abs() < 1e-3);
    assert!(field(&app, "rows").as_array().unwrap().iter().all(|r| web(&r["to"])));
    dialogs::cancel(&mut app);
    // The Color Guide's colours become the new colours.
    app.run("ui.recolorDialog", json!({"colors": ["#123456", "#abcdef"]})).unwrap();
    dialog_frame(&mut app, &ctx);
    let tos: Vec<Value> = field(&app, "rows").as_array().unwrap().iter().map(|r| r["to"].clone()).collect();
    assert_eq!(tos, [json!("rgb 18 52 86"), json!("rgb 171 205 239")]);
    dialogs::cancel(&mut app);
}

#[test]
fn double_clicking_a_colour_group_opens_it_and_ok_rewrites_it() {
    let mut app = app();
    let ctx = context();
    let show: fn(&mut VectorcraftApp, &mut egui::Ui) = panels::swatches::show;
    frame(&mut app, &ctx, vec![], 0.0, show);
    let folder = ctx.read_response(egui::Id::new(("swatch-tile", "Brights"))).expect("the Brights folder").rect.center();
    frame(&mut app, &ctx, press(folder, 2), 1.0, show);
    let d = app.ui.dialog.as_ref().expect("a dialog opened");
    assert_eq!((d.kind.as_str(), d.str("group")), (dialogs::recolor::KIND, "Brights".to_string()));
    let keys = group_keys(&app, "Brights");
    let from: Vec<Value> = field(&app, "rows").as_array().unwrap().iter().map(|r| r["from"][0].clone()).collect();
    assert_eq!(from, keys, "a row per group colour, in order");
    dialog_frame(&mut app, &ctx);
    // A new first colour and a new name; nothing is selected, so only the group changes.
    let mut rows = field(&app, "rows");
    rows[0]["to"] = json!("cmyk 100 0 0 0");
    set(&mut app, "rows", rows);
    set(&mut app, "groupName", json!("Brights 2026"));
    let undo = app.session.doc().unwrap().history.undo.len();
    dialogs::confirm(&mut app).unwrap();
    let mut want = keys.clone();
    want[0] = json!("cmyk 100 0 0 0");
    assert_eq!(group_keys(&app, "Brights 2026"), want);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
}

#[test]
fn the_options_button_edits_a_selected_group_and_applies_it_to_the_art() {
    let mut app = app();
    let ctx = context();
    art(&mut app, &["#ff0000", "#00ff00", "#0000ff", "#ffff00", "#ff00ff"]);
    let show: fn(&mut VectorcraftApp, &mut egui::Ui) = panels::swatches::show;
    frame(&mut app, &ctx, vec![], 0.0, show);
    let folder = ctx.read_response(egui::Id::new(("swatch-tile", "Brights"))).unwrap().rect.center();
    frame(&mut app, &ctx, press(folder, 1), 1.0, show);
    frame(&mut app, &ctx, vec![], 2.0, show);
    // The bottom bar's third button: Swatch Options, or Edit or Apply Color Group for a group.
    let mut bar: Vec<Rect> = ctx.viewport(|vp| {
        vp.prev_pass.widgets.layers().flat_map(|(_, w)| w.iter()).filter(|w| w.rect.size() == vec2(24.0, 24.0)).map(|w| w.rect).collect()
    });
    let bottom = bar.iter().map(|r| r.top()).fold(f32::MIN, f32::max);
    bar.retain(|r| r.top() == bottom);
    bar.sort_by(|a, b| a.left().total_cmp(&b.left()));
    frame(&mut app, &ctx, press(bar[2].center(), 1), 3.0, show);
    assert_eq!(field(&app, "group"), "Brights");
    dialog_frame(&mut app, &ctx);
    let group = group_keys(&app, "Brights");
    let tos: Vec<Value> = field(&app, "rows").as_array().unwrap().iter().map(|r| r["to"].clone()).collect();
    assert_eq!(tos, group, "the art's colours in as many rows as the group has, taking its colours");
    dialogs::confirm(&mut app).unwrap();
    let used: Vec<String> = app.session.execute("recolor.colors", &json!({})).unwrap()["colors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["key"].as_str().unwrap().to_string())
        .collect();
    assert!(used.iter().all(|k| group.contains(&json!(k))), "the art uses the group's colours: {used:?}");
    assert!(matches!(ColorKey::parse(&used[0]).unwrap().color(), Color::Rgb { .. }));
}

#[test]
fn a_group_keeps_the_colours_no_art_row_took() {
    let mut app = app();
    let ctx = context();
    art(&mut app, &["#ff0000", "#0000ff"]);
    let before = group_keys(&app, "Brights");
    app.run("ui.recolorDialog", json!({"group": "Brights"})).unwrap();
    dialog_frame(&mut app, &ctx);
    assert_eq!(field(&app, "rows").as_array().unwrap().len(), 2, "two art colours");
    let mut rows = field(&app, "rows");
    rows[1]["to"] = json!("rgb 1 2 3");
    set(&mut app, "rows", rows);
    dialogs::confirm(&mut app).unwrap();
    let after = group_keys(&app, "Brights");
    assert_eq!(after.len(), before.len());
    assert_eq!((&after[0], &after[1]), (&before[0], &json!("rgb 1 2 3")));
    assert_eq!(after[2..], before[2..]);
}
