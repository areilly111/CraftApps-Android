//! Object → Expand Appearance: fills and strokes as objects, baked geometry effects, raster
//! effects as images, brushes as art, and the enable rule.

use serde_json::{Value, json};
use vectorcraft_color::BlendMode;
use vectorcraft_doc::{Node, NodeKind};

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).unwrap().clone()
}

/// A selected 100×100 square at (50, 50) with a red fill and a 4 pt blue stroke.
fn square(s: &mut Session) -> NodeId {
    let id = NodeId(run(s, "shape.rectangle", json!({"x": 50, "y": 50, "width": 100, "height": 100}))["id"].as_u64().unwrap());
    run(s, "paint.setFill", json!({"color": "#ff0000"}));
    run(s, "paint.setStroke", json!({"color": "#0000ff"}));
    run(s, "stroke.set", json!({"weight": 4}));
    id
}

fn render(s: &Session) -> vectorcraft_render::Rendered {
    let doc = s.doc().unwrap().doc.clone();
    let opts = vectorcraft_render::RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    vectorcraft_render::Renderer::new().render(&doc, 200, 200, vectorcraft_geom::Affine::IDENTITY, &opts)
}

/// Pixels (of 200×200) whose colour differs by more than `tol` in any channel.
fn differing(a: &vectorcraft_render::Rendered, b: &vectorcraft_render::Rendered, tol: u8) -> usize {
    a.pixels
        .as_chunks::<4>()
        .0
        .iter()
        .zip(b.pixels.as_chunks::<4>().0)
        .filter(|(p, q)| p.iter().zip(q.iter()).any(|(x, y)| x.abs_diff(*y) > tol))
        .count()
}

fn expandable(s: &Session) -> bool {
    s.commands().iter().find(|c| c.id == "effect.expandAppearance").unwrap().enabled
}

#[test]
fn two_fills_and_an_offset_stroke_become_three_paths_that_look_the_same() {
    let mut s = session();
    let id = square(&mut s);
    assert!(!expandable(&s), "a basic appearance has nothing to expand");
    // A second fill on top (half-transparent green) and an outset stroke.
    run(&mut s, "appearance.addFill", json!({}));
    run(&mut s, "appearance.setItem", json!({"index": 2, "color": "#00ff00", "opacity": 50}));
    run(&mut s, "effect.apply", json!({"effect": "path.offsetPath", "params": {"offset": 10}, "item": 1}));
    assert!(expandable(&s));
    let before = render(&s);
    let r = run(&mut s, "effect.expandAppearance", json!({}));
    assert_eq!(r["ids"], json!([id.0]));
    let n = node(&s, id);
    let ch = n.children().expect("a group of the fills and the stroke");
    assert_eq!(ch.len(), 3);
    assert!(ch.iter().all(|c| matches!(c.kind, NodeKind::Path { .. } | NodeKind::Compound { .. }) && c.appearance.items.len() == 1));
    // In paint order: red fill, the outlined stroke (a filled ring 10 pt out), the green fill.
    assert!(ch.iter().all(|c| c.appearance.items[0].is_fill() && c.appearance.items[0].effects().is_empty()));
    let ring = ch[1].geometric_bounds().unwrap();
    assert!((ring.width() - 124.0).abs() < 0.5, "{ring:?}");
    assert!((ch[2].opacity - 0.5).abs() < 1e-6, "the fill's opacity is the path's own");
    assert!(ch[2].appearance.is_basic());
    let after = render(&s);
    assert!(differing(&before, &after, 8) < 200, "{} pixels differ", differing(&before, &after, 8));
    assert!(!expandable(&s));
    // One undo step.
    run(&mut s, "edit.undo", json!({}));
    assert!(matches!(node(&s, id).kind, NodeKind::Path { .. }));
    assert_eq!(node(&s, id).appearance.items.len(), 3);
}

#[test]
fn a_shadow_becomes_an_image_under_the_art_and_transparency_stays_on_the_group() {
    let mut s = session();
    let id = square(&mut s);
    run(&mut s, "effect.apply", json!({"effect": "stylize.dropShadow", "params": {"x": 8, "y": 8, "blur": 2, "opacity": 100}}));
    run(&mut s, "transparency.set", json!({"opacity": 60, "blend": "multiply"}));
    let before = render(&s);
    run(&mut s, "effect.expandAppearance", json!({}));
    let n = node(&s, id);
    assert!((n.opacity - 0.6).abs() < 1e-6 && n.blend == BlendMode::Multiply);
    assert!(n.appearance.effects.is_empty() && n.appearance.items.is_empty());
    let ch = n.children().unwrap();
    assert_eq!(ch.len(), 3, "the shadow image, the fill, the outlined stroke");
    let NodeKind::Image(im) = &ch[0].kind else { panic!("the shadow is an image: {}", ch[0].kind_label()) };
    assert!(s.doc().unwrap().doc.images.contains_key(&im.key));
    assert!(ch[1..].iter().all(|c| c.opacity == 1.0 && c.appearance.effects.is_empty()));
    let after = render(&s);
    assert!(differing(&before, &after, 24) < 400, "{} pixels differ", differing(&before, &after, 24));
    // The shadow is still there, below and right of the square.
    let p = after.pixel(155, 155);
    assert!(p[0] < 200, "{p:?}");
}

#[test]
fn a_blur_turns_the_object_into_an_image() {
    let mut s = session();
    let id = square(&mut s);
    run(&mut s, "effect.apply", json!({"effect": "blur.gaussian", "params": {"radius": 3}}));
    run(&mut s, "effect.expandAppearance", json!({}));
    let n = node(&s, id);
    assert!(matches!(n.kind, NodeKind::Image(_)), "{}", n.kind_label());
    assert!(n.appearance.effects.is_empty());
}

#[test]
fn an_art_brush_becomes_a_group_of_its_art() {
    let mut s = session();
    let id = NodeId(run(&mut s, "shape.line", json!({"x1": 20, "y1": 100, "x2": 180, "y2": 100}))["id"].as_u64().unwrap());
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    run(&mut s, "paint.setFill", json!({"none": true}));
    run(&mut s, "brush.apply", json!({"name": "Charcoal"}));
    assert!(expandable(&s));
    let before = render(&s);
    run(&mut s, "effect.expandAppearance", json!({}));
    let n = node(&s, id);
    assert!(matches!(n.kind, NodeKind::Group { .. }), "{}", n.kind_label());
    let mut brushed = false;
    n.walk(&mut |c| brushed |= vectorcraft_brush::has_brush(c));
    assert!(!brushed, "no brush strokes are left");
    let after = render(&s);
    assert!(differing(&before, &after, 24) < 200, "{} pixels differ", differing(&before, &after, 24));
    run(&mut s, "edit.undo", json!({}));
    assert!(vectorcraft_brush::has_brush(&node(&s, id)));
}

#[test]
fn a_targeted_layer_with_a_fill_expands_into_art_among_its_members() {
    let mut s = session();
    let a = square(&mut s);
    let layer = s.doc().unwrap().doc.layers[0].id;
    run(&mut s, "layer.target", json!({"id": layer.0}));
    run(&mut s, "appearance.addStroke", json!({}));
    run(&mut s, "stroke.set", json!({"item": 0, "weight": 6}));
    assert!(expandable(&s));
    let before = render(&s);
    run(&mut s, "effect.expandAppearance", json!({}));
    let l = node(&s, layer);
    assert!(l.appearance.items.is_empty());
    let ch = l.children().unwrap();
    assert_eq!(ch.len(), 2, "the square, then the layer's stroke above it");
    assert_eq!(ch[0].id, a);
    // The layer's stroke is outlined: a filled path.
    assert!(ch[1].appearance.items.iter().all(|i| i.is_fill()));
    let after = render(&s);
    assert!(differing(&before, &after, 8) < 200, "{} pixels differ", differing(&before, &after, 8));
}

#[test]
fn type_with_a_shadow_stays_type_and_type_with_its_own_stroke_is_outlined() {
    let mut s = session();
    let t = NodeId(run(&mut s, "text.create", json!({"x": 20, "y": 80, "text": "Hi", "size": 48}))["id"].as_u64().unwrap());
    run(&mut s, "select.set", json!({"ids": [t.0]}));
    run(&mut s, "effect.apply", json!({"effect": "stylize.dropShadow"}));
    run(&mut s, "effect.expandAppearance", json!({}));
    let n = node(&s, t);
    let ch = n.children().unwrap();
    assert!(matches!(ch[0].kind, NodeKind::Image(_)) && matches!(ch[1].kind, NodeKind::Text(_)));
    // A stroke of its own: outlined glyphs.
    let u = NodeId(run(&mut s, "text.create", json!({"x": 20, "y": 160, "text": "Yo", "size": 48}))["id"].as_u64().unwrap());
    run(&mut s, "select.set", json!({"ids": [u.0]}));
    run(&mut s, "appearance.addStroke", json!({}));
    run(&mut s, "effect.expandAppearance", json!({}));
    let mut text = false;
    node(&s, u).walk(&mut |c| text |= matches!(c.kind, NodeKind::Text(_)));
    assert!(!text, "the type is outlined");
    assert!(!expandable(&s));
}

#[test]
fn a_blurred_layer_keeps_its_image_as_its_only_member() {
    let mut s = session();
    square(&mut s);
    let layer = s.doc().unwrap().doc.layers[0].id;
    run(&mut s, "layer.clippingMask.toggle", json!({"id": layer.0}));
    run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 200, "height": 200}));
    run(&mut s, "layer.target", json!({"id": layer.0}));
    run(&mut s, "effect.apply", json!({"effect": "blur.gaussian"}));
    run(&mut s, "effect.expandAppearance", json!({}));
    let l = node(&s, layer);
    assert!(l.is_layer() && !l.clips() && l.appearance.effects.is_empty());
    let ch = l.children().unwrap();
    assert!(ch.len() == 1 && matches!(ch[0].kind, NodeKind::Image(_)));
}
