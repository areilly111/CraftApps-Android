//! Headless frames of the Gradient panel's freeform section and the Color panel on a freeform
//! point.

use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, vec2};
use serde_json::{Value, json};
use vectorcraft_color::{Freeform, FreeformMode, Paint};
use vectorcraft_engine::Session;

use super::*;

/// A selected rectangle with a freeform fill.
fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    run(&mut app, "file.new", json!({"width": 300, "height": 300}));
    run(&mut app, "shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100}));
    run(&mut app, "paint.editGradient", json!({"kind": "freeform"}));
    app
}

fn run(app: &mut VectorcraftApp, id: &str, p: Value) -> Value {
    app.session.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn freeform(app: &VectorcraftApp) -> Freeform {
    match current_paints(app).0 {
        Paint::Gradient(g) => g.freeform.expect("placed points"),
        p => panic!("expected a gradient fill, got {p:?}"),
    }
}

/// One frame of `f` with `events`: the texts drawn and their centres.
fn frame(ctx: &egui::Context, app: &mut VectorcraftApp, events: Vec<Event>, f: fn(&mut VectorcraftApp, &mut egui::Ui)) -> Vec<(String, Pos2)> {
    fn texts(s: &egui::Shape, out: &mut Vec<(String, Pos2)>) {
        match s {
            egui::Shape::Text(t) => out.push((t.galley.text().to_string(), t.pos + t.galley.rect.center().to_vec2())),
            egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
            _ => {}
        }
    }
    let input = egui::RawInput { events, screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(260.0, 600.0))), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| f(app, ui));
    out.textures_delta.clear();
    let mut v = vec![];
    out.shapes.iter().for_each(|c| texts(&c.shape, &mut v));
    v
}

fn at(texts: &[(String, Pos2)], s: &str) -> Pos2 {
    texts.iter().find(|(t, _)| t == s).unwrap_or_else(|| panic!("`{s}` not drawn: {texts:?}")).1
}

fn click(ctx: &egui::Context, app: &mut VectorcraftApp, pos: Pos2, f: fn(&mut VectorcraftApp, &mut egui::Ui)) -> Vec<(String, Pos2)> {
    let b = |pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    frame(ctx, app, vec![Event::PointerMoved(pos), b(true)], f);
    frame(ctx, app, vec![b(false)], f);
    frame(ctx, app, vec![], f)
}

/// Click the field drawn at `pos`, replace its text with `text` and press Enter.
fn type_into(ctx: &egui::Context, app: &mut VectorcraftApp, pos: Pos2, text: &str, f: fn(&mut VectorcraftApp, &mut egui::Ui)) {
    click(ctx, app, pos, f);
    let key = |k, modifiers| Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers };
    frame(ctx, app, vec![key(Key::A, Modifiers::COMMAND), Event::Text(text.into())], f);
    frame(ctx, app, vec![key(Key::Enter, Modifiers::NONE)], f);
}

#[test]
fn the_freeform_section_replaces_the_slider_and_sets_the_draw_mode() {
    let mut app = app();
    let ctx = egui::Context::default();
    let texts = frame(&ctx, &mut app, vec![], gradient::show);
    assert!(texts.iter().any(|(t, _)| t == "Draw:" || t == "Spread:"), "{texts:?}");
    assert!(!texts.iter().any(|(t, _)| t == "Location:"), "no stop fields: {texts:?}");
    // The Draw toggle's Lines button sits right of the Points one.
    let draw = at(&texts, "Draw:");
    click(&ctx, &mut app, draw + vec2(60.0, 0.0), gradient::show);
    assert_eq!(freeform(&app).mode, FreeformMode::Lines);
    click(&ctx, &mut app, draw + vec2(32.0, 0.0), gradient::show);
    assert_eq!(freeform(&app).mode, FreeformMode::Points);
}

#[test]
fn the_selected_points_opacity_spread_and_delete() {
    let mut app = app();
    let n = freeform(&app).points.len();
    run(&mut app, "paint.freeform.selectPoint", json!({"index": 1}));
    let ctx = egui::Context::default();
    let texts = frame(&ctx, &mut app, vec![], gradient::show);
    let hex = freeform(&app).points[1].color.to_hex().to_uppercase();
    assert!(texts.iter().any(|(t, _)| *t == hex), "the point's colour: {texts:?}");
    let spread = at(&texts, "0%");
    type_into(&ctx, &mut app, spread, "40", gradient::show);
    assert!((freeform(&app).points[1].spread - 0.4).abs() < 1e-6, "{:?}", freeform(&app).points[1]);
    let opacity = at(&texts, "100%");
    type_into(&ctx, &mut app, opacity, "50", gradient::show);
    assert!((freeform(&app).points[1].opacity - 0.5).abs() < 1e-6);
    // Delete Point sits right of the Spread field (54 px wide, text left-aligned in it).
    let texts = frame(&ctx, &mut app, vec![], gradient::show);
    let trash = at(&texts, "40%") + vec2(57.0, 0.0);
    click(&ctx, &mut app, trash, gradient::show);
    assert_eq!(freeform(&app).points.len(), n - 1, "Delete Point");
    // Nothing selected: the hint.
    let texts = frame(&ctx, &mut app, vec![], gradient::show);
    assert!(texts.iter().any(|(t, _)| t.starts_with("Click the art")), "{texts:?}");
}

#[test]
fn the_color_panel_recolours_the_selected_point() {
    let mut app = app();
    run(&mut app, "paint.freeform.selectPoint", json!({"index": 2}));
    let ctx = egui::Context::default();
    let texts = frame(&ctx, &mut app, vec![], color::show);
    let hex = freeform(&app).points[2].color.to_hex()[1..].to_uppercase();
    let field = texts.iter().find(|(t, _)| t.eq_ignore_ascii_case(&hex)).unwrap_or_else(|| panic!("{hex} in {texts:?}")).1;
    type_into(&ctx, &mut app, field, "00FF00", color::show);
    assert_eq!(freeform(&app).points[2].color.to_hex(), "#00ff00");
    assert!(matches!(current_paints(&app).0, Paint::Gradient(_)), "still a gradient");
}
