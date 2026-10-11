//! A blend's spine and keys on the canvas (M8.19): spine points moved, added and deleted (the
//! straight spine becomes a path on the first edit), keys picked and edited live, Release keeping
//! the spine as a path.

use serde_json::{Value, json};
use vectorcraft_color::Paint;
use vectorcraft_doc::live::BlendSpec;
use vectorcraft_doc::{Node, NodeKind};
use vectorcraft_geom::{Point, Rect};

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn id_of(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).unwrap().clone()
}

fn spec(s: &Session, id: NodeId) -> BlendSpec {
    match node(s, id).kind {
        NodeKind::Blend { spec, .. } => spec,
        _ => panic!("not a blend"),
    }
}

fn center(n: &Node) -> Point {
    n.geometric_bounds().unwrap().center()
}

/// Two 10 pt squares centred on (5, 5) and (105, 5), blended with one step.
fn blend(s: &mut Session) -> (NodeId, NodeId, NodeId) {
    let a = id_of(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap());
    let b = id_of(&s.execute("shape.rectangle", &json!({"x": 100, "y": 0, "width": 10, "height": 10})).unwrap());
    let g = id_of(&s.execute("object.blend.make", &json!({"ids": [a.0, b.0], "steps": 1})).unwrap());
    (a, b, g)
}

fn steps(s: &Session, g: NodeId) -> Vec<Node> {
    vectorcraft_doc::live::expand_live(&node(s, g))
}

#[test]
fn a_spine_point_added_and_moved_bends_the_blend() {
    let mut s = session();
    let (_, _, g) = blend(&mut s);
    assert!(spec(&s, g).spine.is_none(), "straight until edited");
    let r = s.execute("object.blend.spine.addAnchor", &json!({"x": 55, "y": 8})).unwrap();
    assert_eq!(r["anchor"], json!(1));
    let sp = spec(&s, g);
    assert_eq!(sp.key_anchors, vec![0, 2], "the keys keep their points");
    assert!(sp.spine.unwrap().subpaths[0].anchors[1].p.distance(Point::new(55.0, 5.0)) < 1e-9, "on the spine, nearest the click");
    s.execute("object.blend.spine.moveAnchor", &json!({"anchor": 1, "x": 55, "y": 105})).unwrap();
    let st = steps(&s, g);
    assert!(center(&st[1]).distance(Point::new(55.0, 105.0)) < 1e-6, "the step follows the spine: {:?}", center(&st[1]));
    assert_eq!(center(&st[0]), Point::new(5.0, 5.0), "the keys stay put");
    // One undo step each.
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(center(&steps(&s, g)[1]).distance(Point::new(55.0, 5.0)) < 1e-6);
}

#[test]
fn moving_a_key_point_moves_the_key_and_the_other_way_round() {
    let mut s = session();
    let (a, b, g) = blend(&mut s);
    s.execute("select.set", &json!({"ids": [g.0]})).unwrap();
    s.execute("object.blend.spine.moveAnchor", &json!({"anchor": 0, "x": 5, "y": 105})).unwrap();
    assert_eq!(center(&node(&s, a)), Point::new(5.0, 105.0));
    // A handle drag curves the spine without moving the key.
    s.execute("object.blend.spine.moveAnchor", &json!({"anchor": 0, "handle": "out", "x": 5, "y": 300})).unwrap();
    assert_eq!(center(&node(&s, a)), Point::new(5.0, 105.0));
    assert!(center(&steps(&s, g)[1]).y > 100.0, "the step bows out");
    // Editing a key on the canvas (it is selectable inside the blend) drags its point along.
    s.execute("select.set", &json!({"ids": [b.0]})).unwrap();
    s.execute("object.move", &json!({"dx": 0, "dy": 50})).unwrap();
    let n = node(&s, g);
    let NodeKind::Blend { children, spec } = &n.kind else { panic!() };
    let (path, _) = vectorcraft_doc::live::blend_spine(children, spec).unwrap();
    assert_eq!(path.subpaths[0].anchors[1].p, Point::new(105.0, 55.0));
    assert!(s.execute("object.blend.spine.moveAnchor", &json!({"id": g.0, "anchor": 9, "x": 0, "y": 0})).is_err());
    assert!(s.execute("object.blend.spine.moveAnchor", &json!({"id": g.0, "anchor": 0, "x": f64::MAX, "y": 0})).is_err());
}

#[test]
fn only_points_without_a_key_can_be_deleted() {
    let mut s = session();
    let (_, _, g) = blend(&mut s);
    assert!(s.execute("object.blend.spine.removeAnchor", &json!({"id": g.0, "anchor": 0})).is_err());
    s.execute("object.blend.spine.addAnchor", &json!({"id": g.0, "x": 30, "y": 5})).unwrap();
    s.execute("object.blend.spine.removeAnchor", &json!({"id": g.0, "anchor": 1})).unwrap();
    let sp = spec(&s, g);
    assert_eq!((sp.spine.unwrap().subpaths[0].anchors.len(), sp.key_anchors), (2, vec![0, 1]));
}

#[test]
fn info_lists_the_spine() {
    let mut s = session();
    let (_, _, g) = blend(&mut s);
    let info = s.execute("object.blend.info", &json!({})).unwrap();
    assert_eq!(info["spine"]["explicit"], json!(false));
    assert_eq!(info["spine"]["keyAnchors"], json!([0, 1]));
    assert_eq!(info["spine"]["anchors"][1]["x"], json!(105.0));
    let _ = g;
}

#[test]
fn release_keeps_the_spine_as_a_path() {
    let mut s = session();
    let (a, b, g) = blend(&mut s);
    s.execute("object.blend.spine.addAnchor", &json!({"id": g.0, "x": 55, "y": 5})).unwrap();
    s.execute("object.blend.spine.moveAnchor", &json!({"id": g.0, "anchor": 1, "x": 55, "y": 60})).unwrap();
    let r = s.execute("object.blend.release", &json!({})).unwrap();
    assert_eq!(r["ids"], json!([a.0, b.0]));
    let spine = NodeId(r["spines"][0].as_u64().unwrap());
    let n = node(&s, spine);
    assert!(n.appearance.fill_paint().is_none() && n.appearance.stroke_paint() == Paint::None, "paints nothing");
    let pts: Vec<Point> = n.path_data().unwrap().anchors().map(|(_, _, a)| a.p).collect();
    assert_eq!(pts, vec![Point::new(5.0, 5.0), Point::new(55.0, 60.0), Point::new(105.0, 5.0)]);
    let d = &s.doc().unwrap().doc;
    let layer = d.layers[0].children().unwrap().iter().map(|c| c.id).collect::<Vec<_>>();
    assert_eq!(layer, vec![spine, a, b], "the spine below the keys");
}

#[test]
fn reverse_spine_and_front_to_back_keep_an_edited_spine() {
    let mut s = session();
    let (a, b, g) = blend(&mut s);
    s.execute("object.blend.spine.addAnchor", &json!({"id": g.0, "x": 55, "y": 5})).unwrap();
    s.execute("object.blend.spine.moveAnchor", &json!({"id": g.0, "anchor": 1, "x": 55, "y": 60})).unwrap();
    s.execute("select.set", &json!({"ids": [g.0]})).unwrap();
    s.execute("object.blend.reverseSpine", &json!({})).unwrap();
    assert_eq!((center(&node(&s, a)), center(&node(&s, b))), (Point::new(105.0, 5.0), Point::new(5.0, 5.0)), "the keys swap ends");
    assert!(center(&steps(&s, g)[1]).distance(Point::new(55.0, 60.0)) < 1e-6, "the spine keeps its shape");
    s.execute("object.blend.reverseFrontToBack", &json!({})).unwrap();
    assert_eq!(center(&node(&s, a)), Point::new(105.0, 5.0), "stacking changes, places don't");
    assert!(center(&steps(&s, g)[1]).distance(Point::new(55.0, 60.0)) < 1e-6);
}

#[test]
fn a_blend_with_an_edited_spine_takes_more_keys_along_it() {
    let mut s = session();
    let (a, b, g) = blend(&mut s);
    s.execute("object.blend.spine.addAnchor", &json!({"id": g.0, "x": 55, "y": 5})).unwrap();
    s.execute("object.blend.spine.moveAnchor", &json!({"id": g.0, "anchor": 1, "x": 55, "y": 60})).unwrap();
    let c = id_of(&s.execute("shape.rectangle", &json!({"x": 200, "y": 0, "width": 10, "height": 10})).unwrap());
    let g2 = id_of(&s.execute("object.blend.make", &json!({"ids": [g.0, c.0]})).unwrap());
    let sp = spec(&s, g2);
    assert_eq!(sp.key_anchors, vec![0, 2, 3]);
    let pts: Vec<Point> = sp.spine.unwrap().subpaths[0].anchors.iter().map(|a| a.p).collect();
    assert_eq!(pts[1], Point::new(55.0, 60.0), "the bend is kept");
    assert_eq!(pts[3], Point::new(205.0, 5.0));
    let _ = (a, b);
}

#[test]
fn direct_selection_picks_keys_and_edits_update_the_blend_live() {
    let mut s = session();
    let (a, _, g) = blend(&mut s);
    s.execute("paint.setFill", &json!({"ids": [a.0], "color": "#000000"})).ok();
    let opt = vectorcraft_doc::hit::HitOptions::default();
    let h = vectorcraft_doc::hit::hit_test(&s.doc().unwrap().doc, Point::new(3.0, 3.0), opt).unwrap();
    assert_eq!((h.leaf, h.top_object(None)), (a, g), "the key is hit; the Selection tool still takes the blend");
    let h = vectorcraft_doc::hit::hit_test(&s.doc().unwrap().doc, Point::new(55.0, 5.0), opt).unwrap();
    assert_eq!(h.leaf, g, "a step hits as the blend");
    let mut r = vectorcraft_render::Renderer::new();
    let before = r.render_region(&s.doc().unwrap().doc, Rect::new(0.0, 0.0, 120.0, 20.0), 1.0, true).pixel(55, 5);
    s.execute("paint.setFill", &json!({"ids": [a.0], "color": "#ff0000"})).unwrap();
    let after = r.render_region(&s.doc().unwrap().doc, Rect::new(0.0, 0.0, 120.0, 20.0), 1.0, true).pixel(55, 5);
    assert_ne!(before, after, "the step redraws with the key's new colour");
}

#[test]
fn key_anchors_round_trip() {
    let mut s = session();
    let (_, _, g) = blend(&mut s);
    s.execute("object.blend.spine.addAnchor", &json!({"id": g.0, "x": 55, "y": 5})).unwrap();
    let d = &s.doc().unwrap().doc;
    let back = vectorcraft_format::load(&vectorcraft_format::save(d, false)).unwrap();
    assert_eq!(&back.layers, &d.layers);
    let old: BlendSpec = serde_json::from_value(json!({"spine": {"subpaths": []}})).unwrap();
    assert!(old.key_anchors.is_empty());
}
