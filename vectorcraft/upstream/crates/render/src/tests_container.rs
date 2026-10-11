//! The appearance of groups, layers and type: a container's own fills and strokes paint its
//! members' geometry below or above them (the contents slot), its effects apply to the composite
//! (one combined shadow) and reshape its members as one piece; type's own fills paint under or
//! over its characters.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::text::{CharStyle, TextObject};
use vectorcraft_doc::{Appearance, AppearanceItem, Effect, FillLayer, Knockout, Node, NodeKind, StrokeLayer};
use vectorcraft_geom::{Point, shapes};

use super::*;

fn fx(id: &str, params: Value) -> Effect {
    Effect { id: id.into(), params, visible: true }
}

fn solid(r: f32, g: f32, b: f32) -> Paint {
    Paint::solid(Color::rgb(r, g, b))
}

/// A rectangle filled with `paint` and no stroke.
fn rect(d: &mut Document, r: Rect, paint: Paint) -> Arc<Node> {
    Arc::new(Node::path(d.alloc_id(), shapes::rectangle(r), Appearance::basic(paint, Paint::None, 0.0)))
}

/// A 100×100 document holding a group of a red square (10..40) and a blue one (50..80), on a row
/// from y 10 to 40, given its own appearance by `style`.
fn group_doc(style: impl FnOnce(&mut Node)) -> Document {
    let mut d = Document::new(100.0, 100.0);
    let a = rect(&mut d, Rect::new(10.0, 10.0, 40.0, 40.0), solid(1.0, 0.0, 0.0));
    let b = rect(&mut d, Rect::new(50.0, 10.0, 80.0, 40.0), solid(0.0, 0.0, 1.0));
    let mut g = Node::group(d.alloc_id(), vec![a, b]);
    style(&mut g);
    let l = d.layers[0].id;
    d.insert(Some(l), 0, g).unwrap();
    d
}

fn render_threads(d: &Document, threads: u16) -> Rendered {
    let mut r = Renderer::new();
    r.threads = threads;
    r.render(d, 100, 100, Affine::IDENTITY, &RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() })
}

fn render(d: &Document) -> Rendered {
    render_threads(d, 0)
}

const GREEN: [u8; 4] = [0, 255, 0, 255];
const RED: [u8; 4] = [255, 0, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];

fn green_fill(g: &mut Node) {
    g.appearance.items.push(AppearanceItem::Fill(FillLayer::new(solid(0.0, 1.0, 0.0))));
}

#[test]
fn a_group_fill_paints_every_member_above_or_below_them() {
    // Above the contents (the default): both members turn green; the gap between them stays white.
    let img = render(&group_doc(green_fill));
    assert_eq!((img.pixel(25, 25), img.pixel(65, 25), img.pixel(45, 25)), (GREEN, GREEN, WHITE));
    // Below the contents the members' own fills cover it.
    let img = render(&group_doc(|g| {
        green_fill(g);
        g.appearance.set_contents_at(1);
    }));
    assert_eq!((img.pixel(25, 25), img.pixel(65, 25)), (RED, BLUE));
    // A group stroke strokes each member's outline.
    let img = render(&group_doc(|g| {
        g.appearance.items.push(AppearanceItem::Stroke(StrokeLayer::new(solid(0.0, 1.0, 0.0), 4.0)));
    }));
    assert_eq!((img.pixel(10, 25), img.pixel(80, 25), img.pixel(25, 25)), (GREEN, GREEN, RED));
    // Hidden fills paint nothing.
    let img = render(&group_doc(|g| {
        green_fill(g);
        if let AppearanceItem::Fill(f) = &mut g.appearance.items[0] {
            f.visible = false;
        }
    }));
    assert_eq!(img.pixel(25, 25), RED);
}

#[test]
fn a_group_fill_with_opacity_composites_as_one_layer() {
    // Two overlapping members under a 50 % group fill: the overlap is as light as the rest.
    let mut d = Document::new(100.0, 100.0);
    let a = rect(&mut d, Rect::new(10.0, 10.0, 60.0, 40.0), Paint::None);
    let b = rect(&mut d, Rect::new(40.0, 10.0, 90.0, 40.0), Paint::None);
    let mut g = Node::group(d.alloc_id(), vec![a, b]);
    let mut f = FillLayer::new(Paint::solid(Color::BLACK));
    f.opacity = 0.5;
    g.appearance.items.push(AppearanceItem::Fill(f));
    let l = d.layers[0].id;
    d.insert(Some(l), 0, g).unwrap();
    let img = render(&d);
    let (single, both) = (img.pixel(20, 25), img.pixel(50, 25));
    assert!(single[0].abs_diff(128) <= 2 && single == both, "{single:?} vs {both:?}");
}

#[test]
fn a_group_shadow_is_one_combined_shadow() {
    // The members' shadows (offset 30 pt right) overlap at x 65 but not at x 80; one combined
    // shadow is equally dark at both, two separate ones would double up at 65.
    let mut d = Document::new(100.0, 100.0);
    let a = rect(&mut d, Rect::new(10.0, 10.0, 40.0, 40.0), Paint::solid(Color::BLACK));
    let b = rect(&mut d, Rect::new(30.0, 10.0, 60.0, 40.0), Paint::solid(Color::BLACK));
    let mut g = Node::group(d.alloc_id(), vec![a, b]);
    g.appearance.effects.push(fx("stylize.dropShadow", json!({"x": 30, "y": 0, "blur": 0, "opacity": 50, "mode": "normal"})));
    let l = d.layers[0].id;
    d.insert(Some(l), 0, g).unwrap();
    for threads in [0, 3] {
        let img = render_threads(&d, threads);
        let (overlap, single) = (img.pixel(65, 25), img.pixel(80, 25));
        assert!(single[0] < 160 && single[0] > 100, "{single:?}");
        assert!(overlap[0].abs_diff(single[0]) <= 2, "one shadow, not two: {overlap:?} vs {single:?}");
        assert_eq!(img.pixel(95, 25), WHITE);
    }
}

#[test]
fn group_geometry_effects_move_the_members_as_one_piece() {
    let img = render(&group_doc(|g| g.appearance.effects.push(fx("distort.transform", json!({"moveV": 50})))));
    assert_eq!((img.pixel(25, 25), img.pixel(25, 75), img.pixel(65, 75)), (WHITE, RED, BLUE));
    // The group's own fill paints the moved members, and the shadow follows them.
    let img = render(&group_doc(|g| {
        g.appearance.effects.push(fx("distort.transform", json!({"moveV": 50})));
        green_fill(g);
    }));
    assert_eq!((img.pixel(25, 75), img.pixel(65, 75)), (GREEN, GREEN));
}

#[test]
fn a_layer_paints_its_own_fill_and_a_knockout_group_keeps_knocking_out() {
    let mut d = group_doc(|_| {});
    let l = Arc::make_mut(&mut d.layers[0]);
    l.appearance.items.push(AppearanceItem::Fill(FillLayer::new(solid(0.0, 1.0, 0.0))));
    let img = render(&d);
    assert_eq!((img.pixel(25, 25), img.pixel(65, 25)), (GREEN, GREEN));

    // A knockout group with a drop shadow: its half-transparent members still knock each other
    // out (the overlap shows the top one over the backdrop), and it casts its shadow.
    let mut d = Document::new(100.0, 100.0);
    let half = |d: &mut Document, r: Rect, p: Paint| {
        let mut n = Node::path(d.alloc_id(), shapes::rectangle(r), Appearance::basic(p, Paint::None, 0.0));
        n.opacity = 0.5;
        Arc::new(n)
    };
    let a = half(&mut d, Rect::new(10.0, 10.0, 50.0, 40.0), solid(1.0, 0.0, 0.0));
    let b = half(&mut d, Rect::new(30.0, 10.0, 70.0, 40.0), solid(0.0, 0.0, 1.0));
    let mut g = Node::group(d.alloc_id(), vec![a, b]);
    g.knockout = Knockout::On;
    g.appearance.effects.push(fx("stylize.dropShadow", json!({"x": 0, "y": 40, "blur": 0, "opacity": 100, "mode": "normal"})));
    let l = d.layers[0].id;
    d.insert(Some(l), 0, g).unwrap();
    let img = render(&d);
    let overlap = img.pixel(40, 25);
    assert!(overlap[0] > 120 && overlap[2] > 250, "the blue knocks the red out: {overlap:?}");
    // The shadow of the composite: as opaque as the knocked-out art (half), the overlap too.
    assert!(img.pixel(40, 65)[0].abs_diff(127) <= 2 && img.pixel(20, 65) == img.pixel(40, 65), "the shadow: {:?}", img.pixel(40, 65));
}

/// "IIII" in black at 40 pt with its own red fill; `below`: under the characters.
fn type_doc(below: bool) -> Document {
    let mut d = Document::new(100.0, 100.0);
    let style = CharStyle { size: 40.0, fill: Paint::solid(Color::BLACK), ..Default::default() };
    let mut n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(Point::new(5.0, 60.0), "IIII", style))));
    n.appearance.items.push(AppearanceItem::Fill(FillLayer::new(solid(1.0, 0.0, 0.0))));
    if below {
        n.appearance.set_contents_at(1);
    }
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    d
}

fn red_pixels(img: &Rendered) -> usize {
    (0..100).flat_map(|y| (0..100).map(move |x| (x, y))).filter(|&(x, y)| matches!(img.pixel(x, y), [r, g, _, _] if r > 200 && g < 60)).count()
}

#[test]
fn a_fill_below_the_characters_is_hidden_by_the_glyphs() {
    assert!(red_pixels(&render(&type_doc(false))) > 50, "above: the red fill covers the glyphs");
    assert_eq!(red_pixels(&render(&type_doc(true))), 0, "below: the black glyphs cover it");
}
