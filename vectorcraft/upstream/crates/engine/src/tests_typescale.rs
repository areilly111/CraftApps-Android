//! #1034: Character sizes and horizontal scales account for the object's affine transform.

use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind, TextObject};
use vectorcraft_geom::Point;

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn create(s: &mut Session, size: f64) -> NodeId {
    NodeId(s.execute("text.create", &json!({"x": 40, "y": 60, "text": "Axis label", "size": size})).unwrap()["id"].as_u64().unwrap())
}

fn text(s: &Session, id: NodeId) -> TextObject {
    match &s.doc().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Text(t) => (**t).clone(),
        _ => panic!("not text"),
    }
}

fn em(t: &TextObject, run: usize) -> (f64, f64) {
    let st = &t.runs[run].style;
    let origin = t.xf * Point::ZERO;
    ((t.xf * Point::new(st.size * st.h_scale / 100.0, 0.0) - origin).hypot(), (t.xf * Point::new(0.0, st.size * st.v_scale / 100.0) - origin).hypot())
}

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-8, "{actual} != {expected}");
}

#[test]
fn scaled_font_size_setter_sets_document_points_and_undoes() {
    let mut s = session();
    let id = create(&mut s, 12.0);
    s.execute("object.scale", &json!({"sx": 200})).unwrap();
    s.execute("object.scale", &json!({"sx": 150})).unwrap();
    let before = text(&s, id);
    near(em(&before, 0).1, 36.0);
    s.execute("text.setStyle", &json!({"size": 10})).unwrap();
    let after = text(&s, id);
    near(em(&after, 0).1, 10.0);
    assert_eq!(after.xf, before.xf, "font editing preserves placement and rotation");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(text(&s, id), before);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(text(&s, id), after);
}

#[test]
fn group_font_size_setter_handles_different_accumulated_scales() {
    let mut s = session();
    let a = create(&mut s, 12.0);
    s.execute("object.scale", &json!({"sx": 200})).unwrap();
    let b = create(&mut s, 18.0);
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    s.execute("object.group", &json!({})).unwrap();
    s.execute("object.group", &json!({})).unwrap();
    s.execute("object.scale", &json!({"sx": 200})).unwrap();
    near(em(&text(&s, a), 0).1, 48.0);
    near(em(&text(&s, b), 0).1, 36.0);
    s.execute("text.setStyle", &json!({"size": 10})).unwrap();
    near(em(&text(&s, a), 0).1, 10.0);
    near(em(&text(&s, b), 0).1, 10.0);
}

#[test]
fn scaled_range_size_and_keyboard_steps_use_document_points() {
    let mut s = session();
    let id = create(&mut s, 12.0);
    s.execute("object.transform", &json!({"matrix": [2, 0, 0, 2, 0, 0]})).unwrap();
    s.execute("text.setRangeStyle", &json!({"id": id.0, "start": 0, "end": 4, "size": 10})).unwrap();
    near(em(&text(&s, id), 0).1, 10.0);
    near(em(&text(&s, id), 1).1, 24.0);
    s.prefs.type_size_increment = 2.0;
    s.execute("type.step", &json!({"attribute": "size", "id": id.0, "start": 0, "end": 4})).unwrap();
    near(em(&text(&s, id), 0).1, 12.0);
    near(em(&text(&s, id), 1).1, 24.0);
}

#[test]
fn horizontal_scale_reset_removes_object_stretch_including_groups() {
    for grouped in [false, true] {
        let mut s = session();
        let id = create(&mut s, 12.0);
        if grouped {
            s.execute("object.group", &json!({})).unwrap();
        }
        s.execute("object.scale", &json!({"sx": 200, "sy": 100})).unwrap();
        let before = text(&s, id);
        s.execute("text.setFormat", &json!({"hScale": 100, "vScale": 100})).unwrap();
        let after = text(&s, id);
        near(em(&after, 0).0, 12.0);
        near(em(&after, 0).1, 12.0);
        assert_eq!(after.xf, before.xf);
    }
}

#[test]
fn rotated_reflected_nonuniform_range_edits_preserve_the_affine() {
    let mut s = session();
    let id = create(&mut s, 12.0);
    // A quarter turn, reflected in the baseline axis: horizontal ×2, vertical ×3.
    s.execute("object.transform", &json!({"matrix": [0, -2, -3, 0, 0, 0]})).unwrap();
    let before = text(&s, id);
    s.execute("text.setRangeStyle", &json!({"id": id.0, "size": 10, "hScale": 100, "vScale": 100})).unwrap();
    let after = text(&s, id);
    near(em(&after, 0).0, 10.0);
    near(em(&after, 0).1, 10.0);
    assert_eq!(after.xf, before.xf);
    // The native document representation retains both the compensation and the affine.
    let bytes = vectorcraft_format::save_file(&s.doc().unwrap().doc);
    let reopened = vectorcraft_format::load(&bytes).unwrap();
    let NodeKind::Text(t) = &reopened.node(id).unwrap().kind else { panic!("not text") };
    near(em(t, 0).0, 10.0);
    near(em(t, 0).1, 10.0);
}

#[test]
fn effective_range_query_leaves_raw_runs_available_for_clipboard() {
    let mut s = session();
    let id = create(&mut s, 12.0);
    s.execute("object.scale", &json!({"sx": 300, "sy": 200})).unwrap();
    let query = |s: &mut Session, effective| -> Value {
        s.execute("text.getRange", &json!({"id": id.0, "effective": effective})).unwrap()["runs"][0]["style"].clone()
    };
    let effective = query(&mut s, true);
    assert_eq!(effective["size"], 24.0);
    assert_eq!(effective["h_scale"], 150.0);
    let raw = query(&mut s, false);
    assert_eq!(raw["size"], 12.0);
    assert_eq!(raw["h_scale"], 100.0);
}

#[test]
fn scaled_point_units_and_small_sizes_are_converted_before_local_storage() {
    let mut s = session();
    let id = create(&mut s, 12.0);
    s.execute("object.scale", &json!({"sx": 200})).unwrap();
    s.execute("text.setStyle", &json!({"size": 0.1, "leading": 14})).unwrap();
    let t = text(&s, id);
    near(em(&t, 0).1, 0.1);
    near(t.runs[0].style.leading.unwrap() * 2.0, 14.0);
    s.execute("text.setRangeStyle", &json!({"id": id.0, "size": 12, "baselineShift": 4, "leading": 16})).unwrap();
    let t = text(&s, id);
    near(t.runs[0].style.baseline_shift * 2.0, 4.0);
    near(t.runs[0].style.leading.unwrap() * 2.0, 16.0);
}

#[test]
fn singular_text_size_edit_returns_an_error_and_rolls_back_the_group() {
    let mut s = session();
    let a = create(&mut s, 12.0);
    let b = create(&mut s, 18.0);
    s.execute("object.transform", &json!({"matrix": [0, 0, 0, 0, 0, 0]})).unwrap();
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    s.execute("object.group", &json!({})).unwrap();
    let before = s.doc().unwrap().doc.clone();
    let history = s.doc().unwrap().history.undo.len();
    assert!(s.execute("text.setStyle", &json!({"size": 10})).is_err());
    assert_eq!(s.doc().unwrap().doc, before);
    assert_eq!(s.doc().unwrap().history.undo.len(), history);
    assert!(s.execute("text.setRangeStyle", &json!({"id": b.0, "size": 10})).is_err());
    assert!(s.execute("text.setFormat", &json!({"id": b.0, "hScale": 100})).is_err());
    // Non-dimensional edits remain possible, including on imported collapsed text.
    s.execute("text.setStyle", &json!({"fill": "#123456"})).unwrap();
}

#[test]
fn sheared_text_keeps_its_affine_and_accepts_exact_em_dimensions() {
    let mut s = session();
    let id = create(&mut s, 12.0);
    s.execute("object.transform", &json!({"matrix": [2, 0, 1, 2, 0, 0]})).unwrap();
    let before = text(&s, id);
    s.execute("text.setRangeStyle", &json!({"id": id.0, "size": 10, "hScale": 100})).unwrap();
    let after = text(&s, id);
    near(em(&after, 0).0, 10.0);
    near(em(&after, 0).1, 10.0);
    assert_eq!(after.xf, before.xf);
}

#[test]
fn area_frame_resize_reflows_at_same_size_and_object_scaling_reports_scaled_size() {
    let mut s = session();
    let id = NodeId(
        s.execute("text.create", &json!({"x": 40, "y": 60, "text": "Area label", "size": 12, "area": {"width": 120, "height": 80}})).unwrap()["id"]
            .as_u64()
            .unwrap(),
    );
    s.execute("object.transform", &json!({"matrix": [2, 0, 0, 2, 0, 0], "typeAreas": true})).unwrap();
    near(em(&text(&s, id), 0).1, 12.0);
    s.execute("object.scale", &json!({"sx": 200})).unwrap();
    near(em(&text(&s, id), 0).1, 24.0);
    s.execute("text.setStyle", &json!({"size": 10})).unwrap();
    near(em(&text(&s, id), 0).1, 10.0);
}

#[test]
fn point_based_keyboard_increments_and_format_fields_follow_document_units() {
    let mut s = session();
    let id = create(&mut s, 12.0);
    s.execute("object.scale", &json!({"sx": 200})).unwrap();
    s.execute("text.setStyle", &json!({"leading": 14})).unwrap();
    s.execute("text.setFormat", &json!({"baselineShift": 4})).unwrap();
    s.prefs.type_size_increment = 2.0;
    s.prefs.baseline_shift_increment = 3.0;
    s.execute("type.step", &json!({"attribute": "leading"})).unwrap();
    s.execute("type.step", &json!({"attribute": "baselineShift"})).unwrap();
    let t = text(&s, id);
    near(t.runs[0].style.leading.unwrap() * 2.0, 16.0);
    near(t.runs[0].style.baseline_shift * 2.0, 7.0);
}

#[test]
fn tiny_finite_transform_rejects_unsafe_compensation_without_history_changes() {
    let mut s = session();
    let id = create(&mut s, 12.0);
    s.execute("object.transform", &json!({"matrix": [1e-306, 0, 0, 1e-306, 0, 0]})).unwrap();
    let before = text(&s, id);
    let history = s.doc().unwrap().history.undo.len();
    let error = s.execute("text.setStyle", &json!({"size": 160})).unwrap_err();
    assert!(matches!(error, EngineError::BadParams { .. }), "reject before attempting unsafe layout: {error}");
    assert_eq!(text(&s, id), before);
    assert_eq!(s.doc().unwrap().history.undo.len(), history);
}
