//! Strokes on type in SVG: character stroke options on live text (and back), runs that differ
//! from the first, and the object's own strokes drawn over the glyph outlines.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{AppearanceItem, CharStyle, Dash, Document, LineCap, LineJoin, Node, NodeKind, StrokeLayer, TextObject, TextRun};
use vectorcraft_geom::Point;
use vectorcraft_svg::{ExportOptions, export, import};

fn text_doc(t: TextObject, items: Vec<AppearanceItem>) -> Document {
    let mut d = Document::new(300.0, 200.0);
    let mut n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(t)));
    n.appearance.items = items;
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    d
}

fn stroked() -> CharStyle {
    CharStyle {
        size: 40.0,
        stroke: Paint::solid(Color::BLACK),
        stroke_width: 2.0,
        stroke_cap: LineCap::Round,
        stroke_join: LineJoin::Round,
        stroke_dash: Some(Dash { pattern: vec![3.0, 1.0], ..Default::default() }),
        ..Default::default()
    }
}

#[test]
fn an_object_stroke_on_type_is_written_with_its_dashes() {
    let mut st = StrokeLayer::new(Paint::solid(Color::rgb8(255, 0, 0)), 3.0);
    st.dash = Some(Dash { pattern: vec![4.0, 2.0], ..Default::default() });
    st.opacity = 0.5;
    let t = TextObject::point(Point::new(20.0, 100.0), "Dashed", CharStyle { size: 40.0, ..Default::default() });
    for outline_text in [false, true] {
        let svg = export(&text_doc(t.clone(), vec![AppearanceItem::Stroke(st.clone())]), &ExportOptions { outline_text, ..Default::default() });
        assert!(svg.contains("stroke-dasharray=\"4 2\""), "{svg}");
        assert!(svg.contains("stroke-opacity=\"0.5\""), "{svg}");
        assert_eq!(svg.contains("<text"), !outline_text, "{svg}");
    }
}

#[test]
fn character_stroke_options_round_trip_through_live_text() {
    let svg = export(&text_doc(TextObject::point(Point::new(20.0, 100.0), "Round", stroked()), vec![]), &ExportOptions::default());
    for attr in ["stroke-linecap=\"round\"", "stroke-linejoin=\"round\"", "stroke-dasharray=\"3 1\""] {
        assert!(svg.contains(attr), "{attr}: {svg}");
    }
    let back = import(&svg).unwrap();
    let mut found = None;
    back.walk(|n| {
        if let NodeKind::Text(t) = &n.kind {
            found = Some(t.runs[0].style.clone());
        }
    });
    let st = found.expect("live text comes back");
    assert_eq!((st.stroke_cap, st.stroke_join), (LineCap::Round, LineJoin::Round));
    assert_eq!(st.stroke_dash.map(|d| d.pattern), Some(vec![3.0, 1.0]));
}

#[test]
fn a_run_without_the_first_runs_stroke_resets_it() {
    let mut t = TextObject::point(Point::new(20.0, 100.0), "Ab", stroked());
    t.runs.push(TextRun { text: "cd".into(), style: CharStyle { size: 40.0, ..Default::default() }, inline: None });
    let svg = export(&text_doc(t, vec![]), &ExportOptions::default());
    let second = svg.split("<tspan").nth(2).expect("a tspan per run");
    assert!(
        second.contains("stroke=\"none\"") && second.contains("stroke-linejoin=\"miter\"") && second.contains("stroke-dasharray=\"none\""),
        "{svg}"
    );
    // Outlined, each run's glyphs carry their own stroke options.
    let svg = export(
        &text_doc(TextObject::point(Point::new(20.0, 100.0), "Ab", stroked()), vec![]),
        &ExportOptions { outline_text: true, ..Default::default() },
    );
    assert!(svg.contains("stroke-linejoin=\"round\"") && svg.contains("stroke-dasharray=\"3 1\""), "{svg}");
}
