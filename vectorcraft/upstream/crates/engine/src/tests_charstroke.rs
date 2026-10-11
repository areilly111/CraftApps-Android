//! Stroke on type: the Stroke panel's options on the characters' stroke (every run, or a range
//! through `text.setRangeStyle`), and the character stroke kept by the Eyedropper, outlines and
//! the native format.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{CharStyle, Dash, LineCap, LineJoin, NodeId, NodeKind, TextObject};

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// Selected point type "Stroke" with a 1 pt red character stroke.
fn stroked_text(s: &mut Session) -> NodeId {
    let id = NodeId(run(s, "text.create", json!({"x": 20, "y": 100, "text": "Stroke", "size": 48}))["id"].as_u64().unwrap());
    run(s, "select.set", json!({ "ids": [id.0] }));
    run(s, "paint.setStroke", json!({"color": "#ff0000"}));
    id
}

fn text(s: &Session, id: NodeId) -> TextObject {
    match &s.doc().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Text(t) => (**t).clone(),
        _ => panic!("not type"),
    }
}

#[test]
fn stroke_options_on_type_style_every_run_and_add_no_object_stroke() {
    let mut s = session();
    let id = stroked_text(&mut s);
    run(&mut s, "text.setRangeStyle", json!({"id": id.0, "start": 0, "end": 2, "size": 60}));
    assert_eq!(text(&s, id).runs.len(), 2);
    let undo = s.doc().unwrap().history.undo.len();
    run(&mut s, "stroke.set", json!({"weight": 3, "join": "round", "cap": "round", "miterLimit": 4, "dash": [6, 2]}));
    let t = text(&s, id);
    for r in &t.runs {
        let st = &r.style;
        assert_eq!((st.stroke_width, st.stroke_join, st.stroke_cap, st.stroke_miter_limit), (3.0, LineJoin::Round, LineCap::Round, 4.0));
        assert_eq!(st.stroke_dash.as_ref().map(|d| d.pattern.clone()), Some(vec![6.0, 2.0]));
        assert_eq!(st.stroke, Paint::solid(Color::rgb8(255, 0, 0)), "the stroke colour stays");
    }
    assert!(s.doc().unwrap().doc.node(id).unwrap().appearance.items.is_empty(), "no object-level stroke is added");
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
    // The shown stroke (Stroke panel, Control bar) is the first run's.
    let shown = s.shown_stroke().unwrap();
    assert_eq!((shown.width, shown.join), (3.0, LineJoin::Round));
    // Dash options keep working on the character dash; null turns it solid.
    run(&mut s, "stroke.set", json!({"alignDashes": true, "dashOffset": 1}));
    assert_eq!(text(&s, id).runs[0].style.stroke_dash, Some(Dash { pattern: vec![6.0, 2.0], offset: 1.0, align_corners: true }));
    run(&mut s, "stroke.set", json!({"dash": null}));
    assert!(text(&s, id).runs.iter().all(|r| r.style.stroke_dash.is_none()));
    assert!(s.execute("stroke.set", &json!({"join": "pointy"})).is_err(), "unknown joins are rejected");
}

#[test]
fn an_object_stroke_on_type_is_still_reached_through_its_item() {
    let mut s = session();
    let id = stroked_text(&mut s);
    run(&mut s, "appearance.addStroke", json!({}));
    let item = s.doc().unwrap().doc.node(id).unwrap().appearance.items.len() - 1;
    run(&mut s, "stroke.set", json!({"item": item, "weight": 5, "dash": [3, 3]}));
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let st = n.appearance.stroke_at(Some(item)).unwrap();
    assert_eq!((st.width, st.dash.as_ref().map(|d| d.pattern.clone())), (5.0, Some(vec![3.0, 3.0])));
    assert_eq!(text(&s, id).runs[0].style.stroke_width, 1.0, "the characters keep their stroke");
}

#[test]
fn range_stroke_options_split_runs() {
    let mut s = session();
    let id = stroked_text(&mut s);
    run(
        &mut s,
        "text.setRangeStyle",
        json!({"id": id.0, "start": 0, "end": 3, "strokeWidth": 2, "strokeOptions": {"join": "bevel", "dash": [1, 1]}}),
    );
    let t = text(&s, id);
    assert_eq!(t.runs.len(), 2);
    assert_eq!((t.runs[0].style.stroke_width, t.runs[0].style.stroke_join), (2.0, LineJoin::Bevel));
    assert!(t.runs[0].style.stroke_dash.is_some());
    assert_eq!((t.runs[1].style.stroke_width, t.runs[1].style.stroke_join), (1.0, LineJoin::Miter));
    assert!(s.execute("text.setRangeStyle", &json!({"id": id.0, "strokeOptions": 3})).is_err());
}

#[test]
fn eyedropper_and_outlines_carry_the_character_stroke() {
    let mut s = session();
    let id = stroked_text(&mut s);
    run(&mut s, "stroke.set", json!({"weight": 2, "join": "round", "dash": [4, 2]}));
    // Type → path: the path takes the characters' stroke options.
    let rect = run(&mut s, "shape.rectangle", json!({"x": 10, "y": 150, "width": 50, "height": 50}))["id"].as_u64().unwrap();
    run(&mut s, "appearance.copyFrom", json!({"source": id.0, "ids": [rect]}));
    let r = s.doc().unwrap().doc.node(NodeId(rect)).unwrap().appearance.stroke().unwrap().clone();
    assert_eq!((r.width, r.join, r.dash.map(|d| d.pattern)), (2.0, LineJoin::Round, Some(vec![4.0, 2.0])));
    // Path → type.
    run(&mut s, "stroke.set", json!({"ids": [rect], "cap": "square", "dash": null}));
    run(&mut s, "appearance.copyFrom", json!({"source": rect, "ids": [id.0]}));
    let st = text(&s, id).runs[0].style.clone();
    assert_eq!((st.stroke_cap, st.stroke_join, st.stroke_dash), (LineCap::Square, LineJoin::Round, None));
    // Create Outlines keeps them on every glyph.
    run(&mut s, "select.set", json!({ "ids": [id.0] }));
    let g = run(&mut s, "type.createOutlines", json!({}))["ids"][0].as_u64().unwrap();
    let d = &s.doc().unwrap().doc;
    let glyphs = d.node(NodeId(g)).unwrap().children().unwrap().to_vec();
    assert!(!glyphs.is_empty());
    assert!(glyphs.iter().all(|c| c.appearance.stroke().is_some_and(|st| st.cap == LineCap::Square && st.join == LineJoin::Round)));
}

#[test]
fn character_stroke_options_round_trip_natively_and_old_files_load() {
    let mut s = session();
    let id = stroked_text(&mut s);
    run(&mut s, "stroke.set", json!({"cap": "round", "join": "bevel", "miterLimit": 3, "dash": [5, 1]}));
    let d = &s.doc().unwrap().doc;
    let back = vectorcraft_format::load(&vectorcraft_format::save(d, false)).unwrap();
    let NodeKind::Text(t) = &back.node(id).unwrap().kind else { panic!("not type") };
    assert_eq!(t.runs, text(&s, id).runs);
    // A character style written before stroke options existed takes the defaults.
    let mut old = serde_json::to_value(CharStyle::default()).unwrap();
    for k in ["stroke_cap", "stroke_join", "stroke_miter_limit", "stroke_dash"] {
        old.as_object_mut().unwrap().remove(k);
    }
    let st: CharStyle = serde_json::from_value(old).unwrap();
    assert_eq!((st.stroke_cap, st.stroke_join, st.stroke_miter_limit, st.stroke_dash), (LineCap::Butt, LineJoin::Miter, 10.0, None));
    assert!(serde_json::to_value(CharStyle::default()).unwrap().get("stroke_miter_limit").is_none(), "defaults aren't written");
}
