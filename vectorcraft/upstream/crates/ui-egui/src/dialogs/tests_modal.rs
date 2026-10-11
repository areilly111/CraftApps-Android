//! #423: modal dialogs move by their heading (and stay on screen and where they were moved), and
//! the form dialogs' choice fields are dropdowns (Offset Path's Joins).

use egui::{Event, Pos2, Rect, Vec2, vec2};
use serde_json::json;
use vectorcraft_engine::Session;

use super::*;
use crate::theme;

const SCREEN: Vec2 = vec2(1280.0, 900.0);

/// A context drawing the dialog layer in a 1280 × 900 window.
fn ctx() -> egui::Context {
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    theme::apply(&ctx, Default::default());
    ctx
}

/// One frame of the dialog layer with `events`: every text drawn, with its screen rect.
fn frame(ctx: &egui::Context, app: &mut VectorcraftApp, events: Vec<Event>) -> Vec<(String, Rect)> {
    let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, SCREEN)), events, ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| show(app, ui.ctx()));
    out.textures_delta.clear();
    fn collect(s: &egui::Shape, out: &mut Vec<(String, Rect)>) {
        match s {
            egui::Shape::Text(t) => out.push((t.galley.text().to_string(), t.visual_bounding_rect())),
            egui::Shape::Vec(v) => v.iter().for_each(|s| collect(s, out)),
            _ => {}
        }
    }
    let mut texts = vec![];
    out.shapes.iter().for_each(|c| collect(&c.shape, &mut texts));
    texts
}

fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }
}

/// Drag with the mouse from `from` by `by`, in steps, then let go.
fn drag(ctx: &egui::Context, app: &mut VectorcraftApp, from: Pos2, by: Vec2) {
    frame(ctx, app, vec![Event::PointerMoved(from), button(from, true)]);
    for i in 1..=6 {
        frame(ctx, app, vec![Event::PointerMoved(from + by * (i as f32 / 6.0))]);
    }
    frame(ctx, app, vec![button(from + by, false)]);
    frame(ctx, app, vec![]);
}

fn click(ctx: &egui::Context, app: &mut VectorcraftApp, pos: Pos2) -> Vec<(String, Rect)> {
    frame(ctx, app, vec![Event::PointerMoved(pos), button(pos, true)]);
    frame(ctx, app, vec![button(pos, false)]);
    frame(ctx, app, vec![])
}

/// Frames until a window that just opened has measured and placed itself.
fn settle(ctx: &egui::Context, app: &mut VectorcraftApp) {
    for _ in 0..6 {
        frame(ctx, app, vec![]);
    }
}

fn rect_of(ctx: &egui::Context, id: egui::Id) -> Rect {
    ctx.memory(|m| m.area_rect(id)).expect("the dialog window was shown")
}

fn app_with_selection() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 80, "height": 60})).unwrap();
    app
}

fn close_to(a: Vec2, b: Vec2) -> bool {
    (a - b).length() < 1.5
}

/// Opens a dialog.
type Open = fn(&mut VectorcraftApp);

/// Every kind of modal window: the shared frame (Offset Path), and the ones drawing their own
/// window (New Document without a heading, Preferences, Keyboard Shortcuts).
#[test]
fn dialogs_move_by_their_heading_stay_on_screen_and_where_they_were_moved() {
    let screen = Rect::from_min_size(Pos2::ZERO, SCREEN);
    let opens: [(&str, Open, egui::Id); 4] = [
        ("Offset Path", |app| crate::menus::invoke(app, "object.path.offsetPath", json!({})), egui::Id::new(("dialog", "offsetPath"))),
        ("New Document", open_new_document, egui::Id::new(("dialog", "newDocument"))),
        ("Preferences", |app| crate::prefs_dialog::open(app, None), egui::Id::new("dialog-preferences")),
        ("Keyboard Shortcuts", crate::shortcut_editor::open, egui::Id::new("dialog-shortcuts")),
    ];
    for (name, open, id) in opens {
        let (ctx, mut app) = (ctx(), app_with_selection());
        open(&mut app);
        settle(&ctx, &mut app);
        let start = rect_of(&ctx, id);
        // Dragging the heading (the band from the top edge down to it) moves the window.
        drag(&ctx, &mut app, start.left_top() + vec2(40.0, 10.0), vec2(-90.0, 50.0));
        let moved = rect_of(&ctx, id);
        assert!(close_to(moved.min - start.min, vec2(-90.0, 50.0)), "{name}: {start:?} → {moved:?}");
        // A drag that starts below the heading (in the bottom margin) doesn't.
        drag(&ctx, &mut app, moved.left_bottom() + vec2(6.0, -4.0), vec2(80.0, -80.0));
        assert_eq!(rect_of(&ctx, id), moved, "{name}: dragging the body moved it");
        // Dragged far past the window's edge, it stops at the edge.
        drag(&ctx, &mut app, moved.left_top() + vec2(40.0, 10.0), vec2(5000.0, 5000.0));
        let edge = rect_of(&ctx, id);
        assert!(screen.contains_rect(edge.shrink(0.5)), "{name}: off screen at {edge:?}");
        assert!((edge.right() - screen.right()).abs() < 1.5 && (edge.bottom() - screen.bottom()).abs() < 1.5, "{name}: {edge:?}");
        // Closed and opened again (this session): where it was left.
        app.ui.dialog = None;
        frame(&ctx, &mut app, vec![]);
        open(&mut app);
        settle(&ctx, &mut app);
        assert!(close_to(rect_of(&ctx, id).min - edge.min, Vec2::ZERO), "{name}: reopened at {:?}, left at {edge:?}", rect_of(&ctx, id));
    }
}

/// Offset Path's Joins is a dropdown of Miter, Round and Bevel (it was a text field), listed after
/// Offset; choosing Round runs the command with round joins.
#[test]
fn offset_path_joins_is_a_dropdown() {
    let (ctx, mut app) = (ctx(), app_with_selection());
    app.run("select.all", json!({})).unwrap();
    crate::menus::invoke(&mut app, "object.path.offsetPath", json!({}));
    frame(&ctx, &mut app, vec![]);
    let texts = frame(&ctx, &mut app, vec![]);
    let at = |texts: &[(String, Rect)], s: &str| texts.iter().find(|(t, _)| t == s).map(|(_, r)| *r);
    let (offset, joins, miter) = (at(&texts, "Offset:").unwrap(), at(&texts, "Joins:").unwrap(), at(&texts, "Miter Limit:").unwrap());
    assert!(offset.top() < joins.top() && joins.top() < miter.top(), "Offset, Joins, Miter Limit: {texts:?}");
    let current = at(&texts, "Miter").expect("Joins shows its choice");
    assert!(at(&texts, "miter").is_none(), "Joins is a text field: {texts:?}");
    // The list: Miter, Round, Bevel.
    let open = click(&ctx, &mut app, current.center());
    for choice in ["Round", "Bevel"] {
        assert!(at(&open, choice).is_some(), "{choice} isn't listed: {open:?}");
    }
    let round = at(&open, "Round").unwrap();
    click(&ctx, &mut app, round.center());
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["joins"], json!("round"));
    // OK: the same result as the command with round joins (not miter).
    confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    let made = |app: &VectorcraftApp| {
        let st = app.session.doc().unwrap();
        format!("{:?}", st.doc.node(st.selection.objects[0]).unwrap().kind)
    };
    let direct = |joins: &str| {
        let mut app = app_with_selection();
        app.run("select.all", json!({})).unwrap();
        app.run("object.path.offsetPath", json!({"offset": 10, "joins": joins, "miterLimit": 4})).unwrap();
        made(&app)
    };
    assert_eq!(made(&app), direct("round"));
    assert_ne!(made(&app), direct("miter"));
}

/// The other form dialogs' choices are dropdowns too: Average's and Shear's axis.
#[test]
fn form_dialog_axes_are_dropdowns() {
    for (id, shown) in [("path.average", "Both"), ("object.shear", "Horizontal"), ("object.reflect", "Vertical")] {
        let (ctx, mut app) = (ctx(), app_with_selection());
        app.run("select.all", json!({})).unwrap();
        crate::menus::invoke(&mut app, id, json!({}));
        frame(&ctx, &mut app, vec![]);
        let texts = frame(&ctx, &mut app, vec![]);
        assert!(texts.iter().any(|(t, _)| t == shown), "{id}: {texts:?}");
        assert!(!texts.iter().any(|(t, _)| *t == shown.to_lowercase()), "{id} has a text field: {texts:?}");
    }
}

/// The axis choices are translated in the "axis" context: in Japanese the plain Horizontal and
/// Vertical are the type orientations (horizontal and vertical writing).
#[test]
fn axis_choices_read_as_axes_in_every_complete_language() {
    use crate::i18n::{LANGUAGES, Lang, tr, tr_ctx};
    let ja = Lang::from_code("ja").unwrap();
    assert_eq!(tr_ctx(ja, "axis", "Horizontal"), "水平方向");
    assert_ne!(tr_ctx(ja, "axis", "Horizontal"), tr(ja, "Horizontal"));
    for l in LANGUAGES.iter().filter(|l| l.complete_menus) {
        for s in ["Horizontal", "Vertical", "Both"] {
            assert!(l.source.lines().any(|line| line.starts_with(&format!("axis\t{s}\t"))), "{}: no axis row for {s}", l.code);
        }
    }
}

/// Offset Path previews on the canvas; Cancel rolls the preview back.
#[test]
fn offset_path_previews() {
    let (ctx, mut app) = (ctx(), app_with_selection());
    app.run("select.all", json!({})).unwrap();
    let count = |app: &VectorcraftApp| app.session.doc().unwrap().doc.layers.iter().map(|l| l.children().map_or(0, |c| c.len())).sum::<usize>();
    crate::menus::invoke(&mut app, "object.path.offsetPath", json!({}));
    let texts = frame(&ctx, &mut app, vec![]);
    let texts = if texts.is_empty() { frame(&ctx, &mut app, vec![]) } else { texts };
    let preview = texts.iter().find(|(t, _)| t == "Preview").map(|(_, r)| *r).expect("a Preview checkbox");
    click(&ctx, &mut app, preview.center());
    frame(&ctx, &mut app, vec![]);
    assert_eq!(count(&app), 2, "previewed");
    cancel(&mut app);
    assert_eq!(count(&app), 1, "rolled back");
}
