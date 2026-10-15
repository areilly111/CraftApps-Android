//! #1212: real ruler gestures and position fields share the document's zero point.

use egui::{Event, Key, PointerButton, Pos2, Rect, Ui, pos2, vec2};
use serde_json::json;
use vectorcraft_doc::{NodeId, Unit};
use vectorcraft_engine::Session;
use vectorcraft_geom::Point;

use crate::{VectorcraftApp, canvas, chrome, panels};

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.ui.view.rulers = true;
    app.ui.control_bar = true;
    app
}

fn context() -> egui::Context {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    ctx
}

fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>, draw: fn(&mut VectorcraftApp, &mut Ui)) -> Vec<(String, Rect)> {
    let raw = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 900.0))),
        time: Some(ctx.input(|i| i.time) + 0.05),
        events,
        ..Default::default()
    };
    let mut out = ctx.run_ui(raw, |ui| draw(app, ui));
    out.textures_delta.clear();
    fn walk(s: &egui::Shape, v: &mut Vec<(String, Rect)>) {
        match s {
            egui::Shape::Text(t) => v.push((t.galley.text().to_string(), Rect::from_min_size(t.pos, t.galley.size()))),
            egui::Shape::Vec(s) => s.iter().for_each(|s| walk(s, v)),
            _ => {}
        }
    }
    let mut texts = vec![];
    out.shapes.iter().for_each(|c| walk(&c.shape, &mut texts));
    texts
}

fn press(at: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() }
}

fn key(key: Key) -> Event {
    Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }
}

fn start_drag(app: &mut VectorcraftApp, ctx: &egui::Context, to: Pos2) {
    let at = pos2(8.0, 8.0);
    frame(app, ctx, vec![Event::PointerMoved(at)], canvas::show);
    frame(app, ctx, vec![press(at, true)], canvas::show);
    frame(app, ctx, vec![Event::PointerMoved(to)], canvas::show);
}

#[test]
fn ruler_origin_drag_uses_the_rotated_zoomed_view_and_commits_only_on_release() {
    let mut app = app();
    app.select_tool("rectangle");
    let ctx = context();
    frame(&mut app, &ctx, vec![], canvas::show);
    let v = app.view_mut().unwrap();
    v.center = Point::new(220.0, 130.0);
    v.zoom = 2.3;
    v.rotation = 35.0;
    frame(&mut app, &ctx, vec![], canvas::show);
    let before = app.session.doc().unwrap().doc.clone();
    let undo = app.session.doc().unwrap().history.undo.len();
    let goal = Point::new(125.25, 90.5);
    let to = canvas::Xf::new(app.canvas_rect.unwrap(), app.view().unwrap()).to_screen(goal);
    start_drag(&mut app, &ctx, to);
    assert_eq!(&*app.session.doc().unwrap().doc, &*before, "preview does not edit the document");
    frame(&mut app, &ctx, vec![press(to, false)], canvas::show);
    let st = app.session.doc().unwrap();
    assert!(st.doc.ruler_origin.distance(goal) < 1e-4, "{:?}", st.doc.ruler_origin);
    assert_eq!(st.history.undo.len(), undo + 1);
    assert_eq!(st.doc.art_bounds(), None, "the drawing tool never receives the corner drag");
    assert!(st.doc.guides.is_empty());
    let origin = st.doc.ruler_origin;
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(&*app.session.doc().unwrap().doc, &*before);
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(app.ruler_origin(), origin);
}

#[test]
fn ruler_origin_double_click_resets_to_the_active_artboard() {
    let mut app = app();
    app.run("artboard.new", json!({"x": 700, "y": -250, "width": 200, "height": 150})).unwrap();
    app.run("document.setRulerOrigin", json!({"x": 125, "y": 90})).unwrap();
    let ctx = context();
    frame(&mut app, &ctx, vec![], canvas::show);
    app.view_mut().unwrap().artboard = 1;
    let at = pos2(8.0, 8.0);
    let undo = app.session.doc().unwrap().history.undo.len();
    for _ in 0..2 {
        frame(&mut app, &ctx, vec![Event::PointerMoved(at)], canvas::show);
        frame(&mut app, &ctx, vec![press(at, true)], canvas::show);
        frame(&mut app, &ctx, vec![press(at, false)], canvas::show);
    }
    assert_eq!(app.ruler_origin(), Point::new(700.0, -250.0));
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.ruler_origin(), Point::new(125.0, 90.0));
}

#[test]
fn ruler_origin_cancelled_drags_do_not_modify_either_document() {
    #[derive(Debug, PartialEq)]
    enum Cancel {
        Outside,
        Escape,
        Focus,
        Document,
    }
    for cancel in [Cancel::Outside, Cancel::Escape, Cancel::Focus, Cancel::Document] {
        let mut app = app();
        app.run("document.setRulerOrigin", json!({"x": 25, "y": 30})).unwrap();
        let ctx = context();
        frame(&mut app, &ctx, vec![], canvas::show);
        let to = app.canvas_rect.unwrap().center();
        let before = app.session.doc().unwrap().doc.clone();
        let undo = app.session.doc().unwrap().history.undo.len();
        start_drag(&mut app, &ctx, to);
        let release = match cancel {
            Cancel::Outside => pos2(4.0, 4.0),
            Cancel::Escape => {
                frame(&mut app, &ctx, vec![key(Key::Escape)], canvas::show);
                to
            }
            Cancel::Focus => {
                let mut out = ctx.run_ui(egui::RawInput { focused: false, ..Default::default() }, |ui| canvas::show(&mut app, ui));
                out.textures_delta.clear();
                to
            }
            Cancel::Document => {
                app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
                to
            }
        };
        frame(&mut app, &ctx, vec![Event::PointerMoved(release), press(release, false)], canvas::show);
        if cancel == Cancel::Document {
            assert_eq!(app.ruler_origin(), Point::ZERO);
            app.run("document.activate", json!({"index": 0})).unwrap();
            assert_eq!(&*app.session.doc().unwrap().doc, &*before);
        } else {
            assert_eq!(&*app.session.doc().unwrap().doc, &*before, "{cancel:?}");
            assert_eq!(app.session.doc().unwrap().history.undo.len(), undo, "{cancel:?}");
        }
    }
}

#[test]
fn ruler_origin_position_fields_show_relative_values_and_edit_absolute_positions() {
    for draw in [panels::properties::show as fn(&mut VectorcraftApp, &mut Ui), panels::transform::show, chrome::control_bar] {
        for (label, arrow) in [("X:", (1.0, 0.0)), ("Y:", (0.0, 1.0))] {
            let mut app = app();
            let id = NodeId(app.run("shape.rectangle", json!({"x": 100, "y": 50, "width": 80, "height": 40})).unwrap()["id"].as_u64().unwrap());
            app.run("document.setRulerOrigin", json!({"x": 120, "y": 50})).unwrap();
            let ctx = context();
            frame(&mut app, &ctx, vec![], draw);
            let texts = frame(&mut app, &ctx, vec![], draw);
            assert!(texts.iter().filter(|(t, _)| t == &Unit::Points.format(20.0)).count() >= 2, "{texts:?}");
            let label = texts.iter().find(|(t, _)| t == label).unwrap().1;
            let at = pos2(label.right() + 40.0, label.center().y);
            frame(&mut app, &ctx, vec![Event::PointerMoved(at), press(at, true)], draw);
            frame(&mut app, &ctx, vec![press(at, false)], draw);
            frame(&mut app, &ctx, vec![key(Key::ArrowUp)], draw);
            let bounds = app.session.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap();
            assert_eq!((bounds.x0, bounds.y0), (100.0 + arrow.0, 50.0 + arrow.1));
            assert_eq!((bounds.width(), bounds.height()), (80.0, 40.0));
            app.run("edit.undo", json!({})).unwrap();
            assert_eq!(app.session.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap().x0, 100.0);
        }
    }
}

#[test]
fn ruler_origin_artboard_fields_preserve_the_absolute_reference_point() {
    for draw in [panels::properties::show as fn(&mut VectorcraftApp, &mut Ui), panels::transform::show] {
        for label in ["X:", "Y:"] {
            let mut app = app();
            app.run("document.setRulerOrigin", json!({"x": 120, "y": 50})).unwrap();
            app.select_tool("artboard");
            let ctx = context();
            frame(&mut app, &ctx, vec![], draw);
            let texts = frame(&mut app, &ctx, vec![], draw);
            let label_rect = texts.iter().find(|(t, _)| t == label).unwrap().1;
            let at = pos2(label_rect.right() + 40.0, label_rect.center().y);
            frame(&mut app, &ctx, vec![Event::PointerMoved(at), press(at, true)], draw);
            frame(&mut app, &ctx, vec![press(at, false)], draw);
            frame(&mut app, &ctx, vec![key(Key::ArrowUp)], draw);
            let ab = app.session.doc().unwrap().doc.artboards[0].rect;
            assert_eq!((ab.x0, ab.y0), if label == "X:" { (1.0, 0.0) } else { (0.0, 1.0) });
            assert_eq!((ab.width(), ab.height()), (400.0, 300.0));
        }
    }
}
