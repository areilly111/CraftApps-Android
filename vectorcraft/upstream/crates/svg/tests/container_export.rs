//! The appearance of groups and layers in SVG export: their own fills and strokes are baked into
//! paths painting the members (below or above them as the contents slot says), their geometry
//! effects into the members, and their raster effects filter the whole group.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use serde_json::json;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, Document, Effect, FillLayer, Node};
use vectorcraft_geom::{Rect, shapes};
use vectorcraft_svg::{ExportOptions, export};

/// A group of a red and a blue square with a green fill of its own (`below`: under the members).
fn doc(below: bool, effects: Vec<Effect>) -> Document {
    let mut d = Document::new(200.0, 200.0);
    let sq = |d: &mut Document, x: f64, hex: &str| {
        let fill = Paint::solid(Color::from_hex(hex).unwrap());
        Arc::new(Node::path(d.alloc_id(), shapes::rectangle(Rect::new(x, 20.0, x + 40.0, 60.0)), Appearance::basic(fill, Paint::None, 0.0)))
    };
    let (a, b) = (sq(&mut d, 20.0, "#ff0000"), sq(&mut d, 100.0, "#0000ff"));
    let mut g = Node::group(d.alloc_id(), vec![a, b]);
    g.appearance.items.push(AppearanceItem::Fill(FillLayer::new(Paint::solid(Color::from_hex("#00ff00").unwrap()))));
    if below {
        g.appearance.set_contents_at(1);
    }
    g.appearance.effects = effects;
    let l = d.layers[0].id;
    d.insert(Some(l), 0, g).unwrap();
    d
}

#[test]
fn a_group_fill_exports_as_paths_over_or_under_the_members() {
    let svg = export(&doc(false, vec![]), &ExportOptions::default());
    // Two green paths, one per member, after both members.
    assert_eq!(svg.matches("fill=\"#00ff00\"").count(), 2, "{svg}");
    let green = svg.find("#00ff00").unwrap();
    assert!(svg.find("#ff0000").unwrap() < green && svg.find("#0000ff").unwrap() < green, "{svg}");
    // Below the contents: before them.
    let svg = export(&doc(true, vec![]), &ExportOptions::default());
    let green = svg.rfind("#00ff00").unwrap();
    assert!(green < svg.find("#ff0000").unwrap(), "{svg}");
}

#[test]
fn group_effects_bake_into_the_members_and_filter_the_group() {
    let fx = |id: &str, params| Effect { id: id.into(), params, visible: true };
    let svg =
        export(&doc(false, vec![fx("distort.transform", json!({"moveV": 100})), fx("stylize.dropShadow", json!({}))]), &ExportOptions::default());
    // One filter around the whole group (one combined shadow).
    assert_eq!(svg.matches("<filter ").count(), 1, "{svg}");
    let open = svg.find("<g filter=\"url(#").expect("a filter group");
    assert!(svg[open..].contains("#ff0000") && svg[open..].contains("#0000ff") && svg[open..].contains("#00ff00"), "{svg}");
    // The members moved down by 100 pt.
    let doc = vectorcraft_effects::bake_document(&doc(false, vec![fx("distort.transform", json!({"moveV": 100}))])).unwrap();
    let g = &doc.layers[0].children().unwrap()[0];
    let b = g.geometric_bounds().unwrap();
    assert!((b.y0 - 120.0).abs() < 1e-6, "{b:?}");
    // Every baked piece has an id of its own and the group has no fills or effects left.
    let mut ids = std::collections::HashSet::new();
    g.walk(&mut |n| assert!(ids.insert(n.id), "duplicate id {}", n.id));
    assert!(g.appearance.items.is_empty() && g.appearance.effects.is_empty());
}
