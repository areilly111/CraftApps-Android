//! Strokes on type keep every stroke option in PDF: the characters' cap, join, miter limit and
//! dash array, and the alignment of the object's own strokes.

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{AppearanceItem, CharStyle, Dash, Document, LineCap, LineJoin, Node, NodeKind, StrokeAlign, StrokeLayer, TextObject};
use vectorcraft_geom::Point;

use crate::*;

#[test]
fn text_strokes_keep_cap_join_miter_dashes_and_alignment() {
    let style = CharStyle {
        size: 48.0,
        stroke: Paint::solid(Color::BLACK),
        stroke_width: 1.5,
        stroke_cap: LineCap::Round,
        stroke_join: LineJoin::Miter,
        stroke_miter_limit: 7.0,
        stroke_dash: Some(Dash { pattern: vec![4.0, 2.0], ..Default::default() }),
        ..Default::default()
    };
    let mut d = Document::new(240.0, 100.0);
    let mut n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(Point::new(10.0, 70.0), "Wave", style))));
    let mut own = StrokeLayer::new(Paint::solid(Color::rgb8(0, 0, 255)), 3.0);
    own.align = StrokeAlign::Inside;
    n.appearance.items.push(AppearanceItem::Stroke(own));
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    let pdf = export(&d, &PdfOptions::uncompressed()).unwrap();
    assert!(String::from_utf8_lossy(&pdf).contains("7 M 1 J[4 2]0 d"), "the characters' miter limit, cap and dash array");
    let back = import(&pdf).unwrap();
    let mut strokes = vec![];
    back.walk(|n| strokes.extend(n.appearance.items.iter().filter_map(|i| if let AppearanceItem::Stroke(s) = i { Some(s.clone()) } else { None })));
    let chars = strokes.iter().find(|s| s.dash.is_some()).expect("the characters' stroke");
    assert_eq!((chars.cap, chars.join), (LineCap::Round, LineJoin::Miter));
    assert!((chars.miter_limit - 7.0).abs() < 1e-3, "miter limit {}", chars.miter_limit);
    // An inside stroke is drawn twice as wide, clipped to the glyphs.
    assert!(strokes.iter().any(|s| s.dash.is_none() && (s.width - 6.0).abs() < 1e-3), "{strokes:?}");
}
