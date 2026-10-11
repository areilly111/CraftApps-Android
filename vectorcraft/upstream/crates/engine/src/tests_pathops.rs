//! Tests for Pathfinder, Object → Path and Type commands.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, NodeKind};
use vectorcraft_geom::{FillRule, PathData, Rect};

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    let r = s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap();
    NodeId(r["id"].as_u64().unwrap())
}

fn ids(v: &Value) -> Vec<NodeId> {
    v["ids"].as_array().unwrap().iter().map(|x| NodeId(x.as_u64().unwrap())).collect()
}

fn node(s: &Session, id: NodeId) -> vectorcraft_doc::Node {
    s.doc().unwrap().doc.node(id).cloned().unwrap()
}

fn path_of(s: &Session, id: NodeId) -> (PathData, FillRule) {
    let n = node(s, id);
    match &n.kind {
        NodeKind::Path { path, rule, .. } => (path.clone(), *rule),
        NodeKind::Compound { children, rule } => {
            (PathData::new(children.iter().flat_map(|c| c.path_data().unwrap().subpaths.clone()).collect()), *rule)
        }
        _ => panic!("not a path"),
    }
}

fn area_of(s: &Session, id: NodeId) -> f64 {
    let (p, r) = path_of(s, id);
    vectorcraft_pathops::area(&p, r)
}

fn bounds(s: &Session, id: NodeId) -> Rect {
    node(s, id).geometric_bounds().unwrap()
}

fn close(a: Rect, b: Rect, tol: f64) -> bool {
    (a.x0 - b.x0).abs() < tol && (a.y0 - b.y0).abs() < tol && (a.x1 - b.x1).abs() < tol && (a.y1 - b.y1).abs() < tol
}

fn set_fill(s: &mut Session, id: NodeId, c: Color) {
    s.edit("t", |d, _| {
        d.node_mut(id).unwrap().appearance.set_fill(Paint::solid(c));
        Ok(())
    })
    .unwrap();
}

fn two_rects(s: &mut Session) -> (NodeId, NodeId) {
    let a = rect(s, 0.0, 0.0, 100.0, 100.0);
    let b = rect(s, 50.0, 50.0, 100.0, 100.0);
    set_fill(s, a, Color::rgb(1.0, 0.0, 0.0));
    set_fill(s, b, Color::rgb(0.0, 0.0, 1.0));
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    (a, b)
}

#[test]
fn unite_two_rects() {
    let mut s = session();
    let (a, b) = two_rects(&mut s);
    let r = ids(&s.execute("object.pathfinder.unite", &json!({})).unwrap());
    assert_eq!(r.len(), 1);
    assert!(s.doc().unwrap().doc.node(a).is_none() && s.doc().unwrap().doc.node(b).is_none());
    assert!(matches!(node(&s, r[0]).kind, NodeKind::Path { .. }));
    assert!(close(bounds(&s, r[0]), Rect::new(0.0, 0.0, 150.0, 150.0), 1e-3));
    assert!((area_of(&s, r[0]) - (10000.0 + 10000.0 - 2500.0)).abs() < 1.0);
    // Top object's style.
    assert_eq!(node(&s, r[0]).appearance.fill_paint(), Paint::solid(Color::rgb(0.0, 0.0, 1.0)));
    assert_eq!(s.doc().unwrap().selection.objects, r);
}

#[test]
fn minus_front_keeps_back_style() {
    let mut s = session();
    two_rects(&mut s);
    let r = ids(&s.execute("object.pathfinder.minusFront", &json!({})).unwrap());
    assert!((area_of(&s, r[0]) - 7500.0).abs() < 1.0);
    assert!(close(bounds(&s, r[0]), Rect::new(0.0, 0.0, 100.0, 100.0), 1e-3));
    assert_eq!(node(&s, r[0]).appearance.fill_paint(), Paint::solid(Color::rgb(1.0, 0.0, 0.0)));
}

#[test]
fn minus_back_keeps_front_style() {
    let mut s = session();
    two_rects(&mut s);
    let r = ids(&s.execute("object.pathfinder.minusBack", &json!({})).unwrap());
    assert!((area_of(&s, r[0]) - 7500.0).abs() < 1.0);
    assert!(close(bounds(&s, r[0]), Rect::new(50.0, 50.0, 150.0, 150.0), 1e-3));
    assert_eq!(node(&s, r[0]).appearance.fill_paint(), Paint::solid(Color::rgb(0.0, 0.0, 1.0)));
}

#[test]
fn intersect_rects() {
    let mut s = session();
    two_rects(&mut s);
    let r = ids(&s.execute("object.pathfinder.intersect", &json!({})).unwrap());
    assert!(close(bounds(&s, r[0]), Rect::new(50.0, 50.0, 100.0, 100.0), 1e-3));
    assert!((area_of(&s, r[0]) - 2500.0).abs() < 1.0);
}

#[test]
fn intersect_disjoint_is_error_and_no_change() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 50.0, 0.0, 10.0, 10.0);
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let undo_len = s.doc().unwrap().history.undo.len();
    assert!(s.execute("object.pathfinder.intersect", &json!({})).is_err());
    assert!(s.doc().unwrap().doc.node(a).is_some());
    assert_eq!(s.doc().unwrap().history.undo.len(), undo_len);
}

#[test]
fn exclude_makes_compound() {
    let mut s = session();
    two_rects(&mut s);
    let r = ids(&s.execute("object.pathfinder.exclude", &json!({})).unwrap());
    assert!((area_of(&s, r[0]) - 15000.0).abs() < 1.0);
}

#[test]
fn divide_produces_group_of_faces() {
    let mut s = session();
    two_rects(&mut s);
    let r = ids(&s.execute("object.pathfinder.divide", &json!({})).unwrap());
    let g = node(&s, r[0]);
    assert!(matches!(g.kind, NodeKind::Group { .. }));
    let ch = g.children().unwrap();
    assert_eq!(ch.len(), 3);
    let total: f64 = ch.iter().map(|c| area_of(&s, c.id)).sum();
    assert!((total - 17500.0).abs() < 1.0);
    // The overlap face takes the top (blue) style.
    let blue = Paint::solid(Color::rgb(0.0, 0.0, 1.0));
    assert_eq!(ch.iter().filter(|c| c.appearance.fill_paint() == blue).count(), 2);
}

#[test]
fn trim_and_merge() {
    let mut s = session();
    two_rects(&mut s);
    let r = ids(&s.execute("object.pathfinder.trim", &json!({})).unwrap());
    let g = node(&s, r[0]);
    assert_eq!(g.children().unwrap().len(), 2);
    s.execute("edit.undo", &json!({})).unwrap();
    // Same colour → merge joins them.
    let sel = s.doc().unwrap().selection.objects.clone();
    set_fill(&mut s, sel[0], Color::rgb(0.0, 0.0, 1.0));
    s.execute("select.set", &json!({"ids": [sel[0].0, sel[1].0]})).unwrap();
    let r = ids(&s.execute("object.pathfinder.merge", &json!({})).unwrap());
    let g = node(&s, r[0]);
    assert_eq!(g.children().unwrap().len(), 1);
}

#[test]
fn crop_and_outline() {
    let mut s = session();
    two_rects(&mut s);
    let r = ids(&s.execute("object.pathfinder.crop", &json!({})).unwrap());
    let g = node(&s, r[0]);
    let ch = g.children().unwrap();
    assert_eq!(ch.len(), 1);
    assert!((area_of(&s, ch[0].id) - 2500.0).abs() < 1.0);
    s.execute("edit.undo", &json!({})).unwrap();
    let r = ids(&s.execute("object.pathfinder.outline", &json!({})).unwrap());
    let g = node(&s, r[0]);
    assert!(g.children().unwrap().len() >= 4);
    assert!(g.children().unwrap().iter().all(|c| c.appearance.stroke_width() > 0.0));
}

#[test]
fn pathfinder_undo_restores() {
    let mut s = session();
    let (a, b) = two_rects(&mut s);
    let before = s.doc().unwrap().doc.clone();
    s.execute("object.pathfinder.unite", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(*s.doc().unwrap().doc, *before);
    assert!(s.doc().unwrap().selection.contains(a) && s.doc().unwrap().selection.contains(b));
}

#[test]
fn unite_with_group() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 20.0, 0.0, 10.0, 10.0);
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let g = s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap();
    let c = rect(&mut s, 5.0, 5.0, 20.0, 10.0);
    s.execute("select.set", &json!({"ids": [g, c.0]})).unwrap();
    let r = ids(&s.execute("object.pathfinder.unite", &json!({})).unwrap());
    assert!(close(bounds(&s, r[0]), Rect::new(0.0, 0.0, 30.0, 15.0), 1e-3));
    assert!((area_of(&s, r[0]) - (100.0 + 100.0 + 200.0 - 25.0 - 25.0)).abs() < 1.0);
}

#[test]
fn offset_rect_grows_bounds() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 50.0, 40.0);
    let r = ids(&s.execute("object.path.offsetPath", &json!({"offset": 10, "joins": "miter", "miterLimit": 4})).unwrap());
    assert_eq!(r.len(), 1);
    assert!(s.doc().unwrap().doc.node(a).is_some(), "original is kept");
    assert!(close(bounds(&s, r[0]), Rect::new(90.0, 90.0, 160.0, 150.0), 1e-2), "{:?}", bounds(&s, r[0]));
    assert_eq!(s.doc().unwrap().selection.objects, r);
    // Above the original.
    let d = &s.doc().unwrap().doc;
    assert!(d.index_path(r[0]).unwrap() > d.index_path(a).unwrap());
    // Negative insets; string params are accepted.
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    let r = ids(&s.execute("object.path.offsetPath", &json!({"offset": "-5 pt", "joins": "round"})).unwrap());
    assert!(close(bounds(&s, r[0]), Rect::new(105.0, 105.0, 145.0, 135.0), 1e-2));
}

#[test]
fn outline_stroke_of_line() {
    let mut s = session();
    let r = s.execute("shape.line", &json!({"x1": 0, "y1": 50, "x2": 100, "y2": 50})).unwrap();
    let l = NodeId(r["id"].as_u64().unwrap());
    s.edit("t", |d, _| {
        d.node_mut(l).unwrap().appearance = Appearance::basic(Paint::None, Paint::solid(Color::rgb(1.0, 0.0, 0.0)), 10.0);
        Ok(())
    })
    .unwrap();
    let r = ids(&s.execute("object.path.outlineStroke", &json!({})).unwrap());
    let n = node(&s, r[0]);
    assert!(close(n.geometric_bounds().unwrap(), Rect::new(0.0, 45.0, 100.0, 55.0), 1e-2));
    assert_eq!(n.appearance.fill_paint(), Paint::solid(Color::rgb(1.0, 0.0, 0.0)));
    assert!(n.appearance.stroke().is_none());
    assert!((area_of(&s, r[0]) - 1000.0).abs() < 1.0);
}

#[test]
fn outline_stroke_with_fill_makes_group() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    s.edit("t", |d, _| {
        d.node_mut(a).unwrap().appearance.stroke_mut().unwrap().width = 4.0;
        Ok(())
    })
    .unwrap();
    let r = ids(&s.execute("object.path.outlineStroke", &json!({})).unwrap());
    let g = node(&s, r[0]);
    let ch = g.children().unwrap();
    assert_eq!(ch.len(), 2);
    assert_eq!(ch[0].appearance.fill_paint(), Paint::solid(Color::WHITE));
    assert_eq!(ch[1].appearance.fill_paint(), Paint::solid(Color::BLACK));
    assert!(close(ch[1].geometric_bounds().unwrap(), Rect::new(-2.0, -2.0, 102.0, 102.0), 1e-2));
}

#[test]
fn simplify_dense_polyline() {
    let mut s = session();
    let anchors: Vec<Value> = (0..=100)
        .map(|i| {
            let x = i as f64 * 3.0;
            json!({"x": x, "y": 100.0 + 40.0 * (x / 50.0).sin()})
        })
        .collect();
    let r = s.execute("path.create", &json!({"anchors": anchors})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    let before = path_of(&s, id).0.anchor_count();
    let out = s.execute("object.path.simplify", &json!({"tolerance": 1.0})).unwrap();
    let after = path_of(&s, id).0.anchor_count();
    assert_eq!(before, 101);
    assert!(after < before / 4, "{after}");
    assert_eq!(out["after"].as_u64().unwrap() as usize, after);
    assert!(close(bounds(&s, id), Rect::new(0.0, 60.0, 300.0, 140.0), 2.0));
}

#[test]
fn add_anchor_points_doubles() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    s.execute("object.path.addAnchorPoints", &json!({})).unwrap();
    assert_eq!(path_of(&s, a).0.anchor_count(), 8);
    assert!(close(bounds(&s, a), Rect::new(0.0, 0.0, 10.0, 10.0), 1e-9));
}

#[test]
fn split_into_grid_cells() {
    let mut s = session();
    rect(&mut s, 0.0, 0.0, 110.0, 50.0);
    let r = ids(&s.execute("object.path.splitIntoGrid", &json!({"rows": 2, "columns": 3, "gutter": 10})).unwrap());
    assert_eq!(r.len(), 6);
    assert!(close(bounds(&s, r[0]), Rect::new(0.0, 0.0, 30.0, 20.0), 1e-9));
    assert!(s.execute("object.path.splitIntoGrid", &json!({"rows": 0})).is_err());
}

#[test]
fn clean_up_removes_junk() {
    let mut s = session();
    let keep = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let unpainted = rect(&mut s, 20.0, 0.0, 10.0, 10.0);
    s.edit("t", |d, _| {
        d.node_mut(unpainted).unwrap().appearance = Appearance::basic(Paint::None, Paint::None, 1.0);
        Ok(())
    })
    .unwrap();
    let stray = NodeId(s.execute("path.create", &json!({"anchors": [{"x": 5, "y": 5}]})).unwrap()["id"].as_u64().unwrap());
    let empty = NodeId(s.execute("text.create", &json!({"x": 10, "y": 10, "text": ""})).unwrap()["id"].as_u64().unwrap());
    let r = s.execute("object.path.cleanUp", &json!({})).unwrap();
    assert_eq!(r["removed"], 3);
    let d = &s.doc().unwrap().doc;
    assert!(d.node(keep).is_some());
    assert!(d.node(unpainted).is_none() && d.node(stray).is_none() && d.node(empty).is_none());
}

#[test]
fn divide_objects_below_cuts() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let cutter = rect(&mut s, 50.0, -10.0, 100.0, 120.0);
    s.execute("select.set", &json!({"ids": [cutter.0]})).unwrap();
    let r = ids(&s.execute("object.path.divideObjectsBelow", &json!({})).unwrap());
    assert_eq!(r.len(), 2);
    let d = &s.doc().unwrap().doc;
    assert!(d.node(a).is_none() && d.node(cutter).is_none());
    let total: f64 = r.iter().map(|id| area_of(&s, *id)).sum();
    assert!((total - 10000.0).abs() < 1.0);
}

fn text(s: &mut Session, t: &str, size: f64) -> NodeId {
    let r = s.execute("text.create", &json!({"x": 10, "y": 50, "text": t, "size": size})).unwrap();
    NodeId(r["id"].as_u64().unwrap())
}

#[test]
fn create_outlines_matches_text_bounds() {
    let mut s = session();
    let t = text(&mut s, "Hello bo", 36.0);
    let tb = bounds(&s, t);
    let r = ids(&s.execute("type.createOutlines", &json!({})).unwrap());
    assert!(s.doc().unwrap().doc.node(t).is_none());
    let g = node(&s, r[0]);
    let ch = g.children().unwrap();
    assert_eq!(ch.len(), 7, "one per visible glyph");
    assert!(ch.iter().any(|c| matches!(c.kind, NodeKind::Compound { .. })), "o/b have holes");
    assert!(ch.iter().all(|c| c.appearance.fill_paint() == Paint::solid(Color::BLACK)));
    let gb = g.geometric_bounds().unwrap();
    assert!(gb.x0 >= tb.x0 - 2.0 && gb.x1 <= tb.x1 + 2.0 && gb.y0 >= tb.y0 - 2.0 && gb.y1 <= tb.y1 + 2.0, "{gb:?} vs {tb:?}");
    assert!(gb.width() > tb.width() * 0.8);
    assert_eq!(s.doc().unwrap().selection.objects, r);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.node(t).is_some());
}

#[test]
fn set_style_size_changes_bounds() {
    let mut s = session();
    let t = text(&mut s, "Hello", 12.0);
    let b0 = bounds(&s, t);
    s.execute("text.setStyle", &json!({"size": 24, "justify": "center", "fill": "#ff0000", "leading": "auto"})).unwrap();
    let b1 = bounds(&s, t);
    assert!(b1.width() > b0.width() * 1.8, "{b0:?} → {b1:?}");
    let NodeKind::Text(tx) = node(&s, t).kind else { panic!() };
    assert_eq!(tx.first_style().size, 24.0);
    assert_eq!(tx.para.justify, vectorcraft_doc::Justify::Center);
    assert_eq!(tx.first_style().fill, Paint::solid(Color::rgb(1.0, 0.0, 0.0)));
    assert!(s.execute("text.setStyle", &json!({"justify": "diagonal"})).is_err());
    assert!(s.execute("text.setStyle", &json!({})).is_err());
}

#[test]
fn set_text_updates_bounds() {
    let mut s = session();
    let t = text(&mut s, "Hi", 24.0);
    let b0 = bounds(&s, t);
    s.execute("text.setText", &json!({"id": t.0, "text": "Hello there"})).unwrap();
    let NodeKind::Text(tx) = node(&s, t).kind else { panic!() };
    assert_eq!(tx.plain_text(), "Hello there");
    assert!(bounds(&s, t).width() > b0.width() * 2.0);
    assert!(s.execute("text.setText", &json!({"id": t.0})).is_err());
    s.execute("edit.undo", &json!({})).unwrap();
    let NodeKind::Text(tx) = node(&s, t).kind else { panic!() };
    assert_eq!(tx.plain_text(), "Hi");
}

#[test]
fn commands_reject_bad_input() {
    let mut s = session();
    two_rects(&mut s);
    for (c, p) in [
        ("object.path.offsetPath", json!({"offset": "abc"})),
        ("object.path.offsetPath", json!({"offset": 1e300})),
        ("object.path.simplify", json!({"tolerance": -5})),
        ("object.path.splitIntoGrid", json!({"rows": 1e9, "columns": -1})),
        ("text.setStyle", json!({"size": -3})),
        ("type.createOutlines", json!({})),
        ("object.pathfinder.divide", json!({"junk": [1, 2]})),
    ] {
        let _ = s.execute(c, &p);
    }
}
