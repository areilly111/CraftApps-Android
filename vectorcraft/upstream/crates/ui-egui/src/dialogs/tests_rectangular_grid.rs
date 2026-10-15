//! Rectangular Grid click-to-size flow, from canvas dispatch through the registered dialog.
//! All geometry here is original, synthetic test data.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::{Document, NodeId, NodeKind};
use vectorcraft_engine::{Session, ViewInfo};
use vectorcraft_geom::{PathData, Point, Rect};
use vectorcraft_tools::distort::perspective::{PerspectiveGrid, Plane};
use vectorcraft_tools::{PointerEvent, PointerKind};

use crate::{VectorcraftApp, theme};

const KIND: &str = "rectangularGrid";

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    app.select_tool(KIND);
    // Pointer-up persists tool settings even for a click. Seed them so opening/cancelling can
    // be compared with the complete pre-click preferences, including an unchanged store.
    saved_counts(&mut app, 5, 5);
    app
}

fn saved_counts(app: &mut VectorcraftApp, rows: u32, columns: u32) {
    let values = json!({"rows": rows, "columns": columns});
    app.session.set_tool_options(Some(KIND), values.as_object().unwrap());
}

fn view() -> ViewInfo {
    ViewInfo { smart_guides: false, snap_to_point: false, snap_to_grid: false, snap_to_pixel: false, ..Default::default() }
}

fn pointer(app: &mut VectorcraftApp, kind: PointerKind, p: Point) {
    crate::canvas::dispatch(app, &PointerEvent::new(kind, p.x, p.y), view());
}

fn click(app: &mut VectorcraftApp, at: Point) {
    app.select_tool(KIND);
    pointer(app, PointerKind::Down, at);
    assert!(app.ui.dialog.is_none(), "the dialog opens on release");
    pointer(app, PointerKind::Up, at);
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(KIND));
    assert!(!app.session.tool_busy());
    assert!(!app.session.in_interaction());
}

fn field(app: &VectorcraftApp, key: &str) -> Value {
    app.ui.dialog.as_ref().unwrap().fields.get(key).cloned().unwrap_or_default()
}

fn set(app: &mut VectorcraftApp, key: &str, value: Value) {
    app.ui.dialog.as_mut().unwrap().fields.insert(key.into(), value);
}

fn values(app: &mut VectorcraftApp, width: Value, height: Value, rows: Value, columns: Value) {
    for (key, value) in [("width", width), ("height", height), ("rows", rows), ("columns", columns)] {
        set(app, key, value);
    }
}

fn confirm(app: &mut VectorcraftApp) -> NodeId {
    let result = super::confirm(app).unwrap();
    assert!(app.ui.dialog.is_none());
    NodeId(result["id"].as_u64().expect("confirmation returns the created group"))
}

fn paths(app: &VectorcraftApp, id: NodeId) -> Vec<PathData> {
    let doc = &app.session.doc().unwrap().doc;
    let group = doc.node(id).unwrap();
    assert!(matches!(group.kind, NodeKind::Group { .. }));
    group.children().unwrap().iter().map(|n| n.path_data().expect("every grid child is an editable path").clone()).collect()
}

fn assert_path(path: &PathData, expected: &[Point], closed: bool) {
    assert_eq!(path.subpaths.len(), 1);
    let subpath = &path.subpaths[0];
    assert_eq!(subpath.closed, closed);
    assert_eq!(subpath.anchors.len(), expected.len());
    for (anchor, expected) in subpath.anchors.iter().zip(expected) {
        assert!(anchor.p.distance(*expected) < 1e-7, "{:?} != {expected:?}", anchor.p);
        assert!(anchor.h_in.distance(anchor.p) < 1e-7 && anchor.h_out.distance(anchor.p) < 1e-7, "grid edges stay straight");
    }
}

/// Independent coordinate oracle: counts mean interior dividers, plus one closed outer frame.
fn assert_grid(app: &VectorcraftApp, id: NodeId, bounds: Rect, rows: u32, columns: u32) {
    let paths = paths(app, id);
    assert_eq!(paths.len(), (rows + columns + 1) as usize);
    let mut remaining = paths.iter();
    for i in 1..=rows {
        let y = bounds.y0 + bounds.height() * f64::from(i) / f64::from(rows + 1);
        assert_path(remaining.next().unwrap(), &[Point::new(bounds.x0, y), Point::new(bounds.x1, y)], false);
    }
    for i in 1..=columns {
        let x = bounds.x0 + bounds.width() * f64::from(i) / f64::from(columns + 1);
        assert_path(remaining.next().unwrap(), &[Point::new(x, bounds.y0), Point::new(x, bounds.y1)], false);
    }
    assert_path(
        remaining.next().unwrap(),
        &[Point::new(bounds.x0, bounds.y0), Point::new(bounds.x1, bounds.y0), Point::new(bounds.x1, bounds.y1), Point::new(bounds.x0, bounds.y1)],
        true,
    );
    assert!(remaining.next().is_none());
}

struct Snapshot {
    doc: Arc<Document>,
    selected: Vec<NodeId>,
    prefs: Value,
    options: Value,
    undo: usize,
    redo: usize,
    journal: vectorcraft_engine::Journal,
    dirty: bool,
}

impl Snapshot {
    fn take(app: &VectorcraftApp) -> Self {
        let state = app.session.doc().unwrap();
        Self {
            doc: state.doc.clone(),
            selected: state.selection.objects.clone(),
            prefs: app.session.prefs.to_json(),
            options: app.session.tool_options_of(KIND),
            undo: state.history.undo.len(),
            redo: state.history.redo.len(),
            journal: app.session.journal.clone(),
            dirty: state.is_dirty(),
        }
    }

    fn assert_unchanged(&self, app: &VectorcraftApp) {
        let state = app.session.doc().unwrap();
        assert_eq!(state.doc, self.doc, "the document, including allocated ids, stays unchanged");
        assert_eq!(state.selection.objects, self.selected);
        assert_eq!(app.session.prefs.to_json(), self.prefs);
        assert_eq!(app.session.tool_options_of(KIND), self.options);
        assert_eq!((state.history.undo.len(), state.history.redo.len()), (self.undo, self.redo));
        assert_eq!(app.session.journal, self.journal);
        assert_eq!(state.is_dirty(), self.dirty);
        assert!(!app.session.in_interaction());
    }
}

fn key_frame(app: &mut VectorcraftApp, key: egui::Key) {
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    theme::apply(&ctx, Default::default());
    let event = egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
    let input = egui::RawInput { events: vec![event], ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| {
        crate::shortcuts::handle(app, ui.ctx());
        super::show(app, ui.ctx());
    });
    out.textures_delta.clear();
}

#[test]
fn rectangular_grid_canvas_click_opens_registered_dialog_with_saved_counts_and_point_defaults() {
    let mut app = app();
    saved_counts(&mut app, 7, 3);
    app.run("document.setUnits", json!({"units": "mm"})).unwrap();
    let before = Snapshot::take(&app);
    click(&mut app, Point::new(137.25, 86.5));
    before.assert_unchanged(&app);
    assert!(super::DialogKind::of(KIND).is_some(), "must not fall through to the no-op form spec");
    assert_eq!((field(&app, "x"), field(&app, "y")), (json!(137.25), json!(86.5)));
    assert_eq!((field(&app, "width"), field(&app, "height")), (json!("100 pt"), json!("100 pt")));
    assert_eq!((field(&app, "rows"), field(&app, "columns")), (json!(7), json!(3)));
    let text = crate::tests_labels::painted_text(&mut app, |app, ui| super::show(app, ui.ctx()));
    let labels = ["Width:", "Height:", "Horizontal Dividers:", "Vertical Dividers:"];
    let positions: Vec<_> = labels.iter().map(|label| text.find(label).expect("a labelled field")).collect();
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]), "{text}");
    let id = confirm(&mut app);
    assert_grid(&app, id, Rect::new(137.25, 86.5, 237.25, 186.5), 7, 3);
}

#[test]
fn rectangular_grid_creates_exact_dividers_as_one_undo_redo_step() {
    let mut app = app();
    app.run("shape.rectangle", json!({"x": 12, "y": 18, "width": 9, "height": 11})).unwrap();
    let before = Snapshot::take(&app);
    click(&mut app, Point::new(125.0, 75.0));
    values(&mut app, json!(120), json!(60), json!(2), json!(3));
    let id = confirm(&mut app);
    assert_grid(&app, id, Rect::new(125.0, 75.0, 245.0, 135.0), 2, 3);
    assert_eq!(app.session.doc().unwrap().selection.objects, vec![id]);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), before.undo + 1);
    assert_eq!(app.session.journal.last().unwrap().0, "shape.rectangularGrid");
    assert_eq!(app.session.journal.len(), before.journal.len() + 1);
    let created = app.session.doc().unwrap().doc.clone();
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc, before.doc);
    assert_eq!(app.session.doc().unwrap().selection.objects, before.selected);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), before.undo);
    assert_eq!(app.session.tool_options_of(KIND), json!({"rows": 2, "columns": 3}), "undo does not undo the saved tool settings");
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc, created);
    assert_grid(&app, id, Rect::new(125.0, 75.0, 245.0, 135.0), 2, 3);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), before.undo + 1);
}

#[test]
fn rectangular_grid_uses_shared_length_parsing_and_numeric_points() {
    for (unit, width, height, expected_width, expected_height) in [
        ("mm", json!("25.4 mm"), json!("12.7 mm"), 72.0, 36.0),
        ("mm", json!(72), json!(36), 72.0, 36.0),
        ("inches", json!("2 cm"), json!("18 pt"), 144.0 / 2.54, 18.0),
        ("mm", json!("1 in + 18 pt"), json!("2p6"), 90.0, 30.0),
    ] {
        let mut app = app();
        app.run("document.setUnits", json!({"units": unit})).unwrap();
        click(&mut app, Point::new(-13.5, 29.25));
        values(&mut app, width, height, json!("2"), json!("3"));
        let id = confirm(&mut app);
        assert_grid(&app, id, Rect::new(-13.5, 29.25, -13.5 + expected_width, 29.25 + expected_height), 2, 3);
    }
}

#[test]
fn rectangular_grid_cancel_and_escape_discard_edits_and_reopen_from_saved_values() {
    for escape in [false, true] {
        let mut app = app();
        saved_counts(&mut app, 2, 8);
        // A pending redo must survive opening, editing and cancelling the size dialog.
        app.run("shape.rectangle", json!({"x": 5, "y": 6, "width": 7, "height": 8})).unwrap();
        app.run("edit.undo", json!({})).unwrap();
        let before = Snapshot::take(&app);
        click(&mut app, Point::new(111.0, 147.0));
        values(&mut app, json!("18 mm"), json!("nonsense"), json!(9), json!(1));
        if escape {
            key_frame(&mut app, egui::Key::Escape);
        } else {
            super::cancel(&mut app);
        }
        assert!(app.ui.dialog.is_none());
        before.assert_unchanged(&app);
        click(&mut app, Point::new(209.0, 173.0));
        assert_eq!((field(&app, "x"), field(&app, "y")), (json!(209.0), json!(173.0)));
        assert_eq!((field(&app, "width"), field(&app, "height")), (json!("100 pt"), json!("100 pt")));
        assert_eq!((field(&app, "rows"), field(&app, "columns")), (json!(2), json!(8)));
        super::cancel(&mut app);
        before.assert_unchanged(&app);
    }
}

#[test]
fn rectangular_grid_enter_confirms_through_the_shared_modal_frame() {
    let mut app = app();
    click(&mut app, Point::new(121.0, 143.0));
    values(&mut app, json!("84 pt"), json!("56 pt"), json!(2), json!(3));
    key_frame(&mut app, egui::Key::Enter);
    assert!(app.ui.dialog.is_none());
    let selected = &app.session.doc().unwrap().selection.objects;
    assert_eq!(selected.len(), 1);
    assert_grid(&app, selected[0], Rect::new(121.0, 143.0, 205.0, 199.0), 2, 3);
}

#[test]
fn rectangular_grid_perspective_click_projects_each_anchor_and_is_one_undo_step() {
    let mut app = app();
    app.run("perspective.grid.preset", json!({"kind": 2})).unwrap();
    app.run("perspective.grid.set", json!({"snap": false})).unwrap();
    app.run("perspective.plane.set", json!({"plane": "right"})).unwrap();
    let grid = PerspectiveGrid::effective(&app.session.doc().unwrap().doc);
    let at = grid.to_page(Plane::Right, Point::new(65.0, 85.0)).unwrap();
    let map = grid.size_homography(Plane::Right, at, false).unwrap();
    let before = Snapshot::take(&app);
    click(&mut app, at);
    values(&mut app, json!(96), json!(54), json!(1), json!(2));
    let id = confirm(&mut app);
    let expected = [
        vec![Point::new(at.x, at.y + 27.0), Point::new(at.x + 96.0, at.y + 27.0)],
        vec![Point::new(at.x + 32.0, at.y), Point::new(at.x + 32.0, at.y + 54.0)],
        vec![Point::new(at.x + 64.0, at.y), Point::new(at.x + 64.0, at.y + 54.0)],
        vec![at, Point::new(at.x + 96.0, at.y), Point::new(at.x + 96.0, at.y + 54.0), Point::new(at.x, at.y + 54.0)],
    ];
    let actual = paths(&app, id);
    assert_eq!(actual.len(), expected.len());
    for (index, (path, points)) in actual.iter().zip(expected).enumerate() {
        let projected: Vec<Point> = points.into_iter().map(|p| map.apply(p).unwrap()).collect();
        assert_path(path, &projected, index == 3);
    }
    let state = app.session.doc().unwrap();
    assert_eq!(PerspectiveGrid::effective(&state.doc).attachment_of(&state.doc, id), Some((Plane::Right, 0.0)));
    assert_eq!(state.history.undo.len(), before.undo + 1);
    let (command, params) = app.session.journal.last().unwrap();
    assert_eq!(command, "perspective.draw");
    assert_eq!(params["command"], json!("shape.rectangularGrid"));
    assert_eq!(app.session.journal.len(), before.journal.len() + 1);
    let created = state.doc.clone();
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc, before.doc);
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc, created);
}

#[test]
fn rectangular_grid_failed_creation_does_not_save_counts() {
    let mut app = app();
    saved_counts(&mut app, 2, 3);
    let layer = app.session.doc().unwrap().doc.layers[0].id;
    app.run("layer.setProps", json!({"ids": [layer.0], "locked": true})).unwrap();
    let before = Snapshot::take(&app);
    click(&mut app, Point::new(30.0, 40.0));
    values(&mut app, json!(120), json!(60), json!(7), json!(8));
    assert!(super::confirm(&mut app).is_err());
    assert_eq!(app.session.tool_options_of(KIND), before.options);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), before.undo);
}
