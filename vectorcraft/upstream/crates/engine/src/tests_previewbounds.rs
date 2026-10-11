//! Use Preview Bounds: the Transform panel values, the bounding box, Align and Distribute measure
//! visual bounds (strokes included) when it is on.

use serde_json::{Value, json};
use vectorcraft_doc::NodeId;
use vectorcraft_geom::Rect;
use vectorcraft_tools::{PointerEvent, PointerKind};

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// A rectangle with a `weight` pt stroke (0: no stroke).
fn rect(s: &mut Session, x: f64, w: f64, weight: f64) -> NodeId {
    let id = NodeId(run(s, "shape.rectangle", json!({"x": x, "y": 100, "width": w, "height": 50}))["id"].as_u64().unwrap());
    if weight > 0.0 {
        run(s, "stroke.set", json!({"weight": weight}));
    } else {
        run(s, "paint.setStroke", json!({"none": true}));
    }
    id
}

fn geometric(s: &Session, id: NodeId) -> Rect {
    s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap()
}

fn preview(s: &mut Session, on: bool) {
    run(s, "prefs.set", json!({"key": "usePreviewBounds", "value": on}));
}

/// Two rectangles: one with a 10 pt stroke at x = 100, an unstroked one at x = 20; align left.
fn aligned_left(on: bool, p: Value) -> f64 {
    let mut s = session();
    let a = rect(&mut s, 100.0, 50.0, 10.0);
    rect(&mut s, 20.0, 50.0, 0.0);
    preview(&mut s, on);
    run(&mut s, "select.all", json!({}));
    let mut p = p;
    p["horizontal"] = json!("left");
    run(&mut s, "object.align", p);
    geometric(&s, a).x0
}

#[test]
fn align_left_differs_by_half_the_stroke() {
    assert_eq!(aligned_left(false, json!({})), 20.0);
    // The 10 pt stroke's outer edge lines up with the other object: the path sits 5 pt in.
    assert_eq!(aligned_left(true, json!({})), 25.0);
    // `bounds` overrides the preference either way.
    assert_eq!(aligned_left(true, json!({"bounds": "geometric"})), 20.0);
    assert_eq!(aligned_left(false, json!({"bounds": "preview"})), 25.0);
    let mut s = session();
    rect(&mut s, 0.0, 10.0, 0.0);
    assert!(s.execute("object.align", &json!({"horizontal": "left", "bounds": "visual"})).is_err());
}

#[test]
fn distribute_spacing_measures_preview_bounds() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 50.0, 10.0);
    let b = rect(&mut s, 200.0, 50.0, 10.0);
    preview(&mut s, true);
    run(&mut s, "select.set", json!({"ids": [a.0, b.0]}));
    run(&mut s, "object.distributeSpacing", json!({"axis": "horizontal", "spacing": 0}));
    // The strokes touch: the paths are a stroke weight apart.
    assert_eq!(geometric(&s, b).x0 - geometric(&s, a).x1, 10.0);
}

#[test]
fn the_w_field_sets_the_visual_width() {
    let mut s = session();
    let id = rect(&mut s, 100.0, 100.0, 10.0);
    preview(&mut s, true);
    assert_eq!(s.transform_bounds(&[id]).unwrap().width(), 110.0);
    // Strokes scale: the whole visual box doubles, from its left edge.
    run(&mut s, "object.setBounds", json!({"width": 220, "reference": 0, "proportional": true, "strokes": true}));
    let v = s.transform_bounds(&[id]).unwrap();
    assert!((v.width() - 220.0).abs() < 1e-9 && (v.x0 - 95.0).abs() < 1e-9, "{v:?}");
    assert!((geometric(&s, id).width() - 200.0).abs() < 1e-9);
    run(&mut s, "edit.undo", json!({}));
    // Strokes kept: the path takes the difference.
    run(&mut s, "object.setBounds", json!({"width": 220, "reference": 0, "strokes": false}));
    let v = s.transform_bounds(&[id]).unwrap();
    assert!((v.width() - 220.0).abs() < 1e-9 && (v.x0 - 95.0).abs() < 1e-9, "{v:?}");
    assert!((geometric(&s, id).width() - 210.0).abs() < 1e-9);
    // X places the visual box's reference point.
    run(&mut s, "object.setBounds", json!({"x": 0, "reference": 0}));
    assert!((s.transform_bounds(&[id]).unwrap().x0).abs() < 1e-9);
    assert!((geometric(&s, id).x0 - 5.0).abs() < 1e-9);
}

#[test]
fn the_bounding_box_handles_sit_on_the_preview_bounds() {
    // Drag from the visual right edge (10 pt outside the path) 100 pt to the right.
    let drag = |on: bool| {
        let mut s = session();
        let id = rect(&mut s, 100.0, 100.0, 20.0);
        preview(&mut s, on);
        let v = ViewInfo::default();
        s.select_tool("selection", v).unwrap();
        for (k, x) in [(PointerKind::Down, 210.0), (PointerKind::Drag, 310.0), (PointerKind::Up, 310.0)] {
            s.pointer(&PointerEvent::new(k, x, 125.0), v).unwrap();
        }
        geometric(&s, id)
    };
    assert!(drag(true).width() > 150.0, "a scale from the visual edge");
    assert_eq!(drag(false).width(), 100.0, "off, the point is no handle");
}
