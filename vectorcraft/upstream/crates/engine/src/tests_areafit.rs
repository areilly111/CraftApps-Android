//! Area type fitting: Auto Size (the frame's height follows the text), Shrink Text to Fit, and
//! overflow reporting.

use serde_json::{Value, json};
use vectorcraft_doc::{AreaFit, NodeKind, TextKind, TextObject};
use vectorcraft_geom::Rect;
use vectorcraft_tools::{Mods, PointerEvent, PointerKind, ToolKey};

use super::*;

const STORY: &str = "The quick brown fox jumps over the lazy dog. A second sentence follows the first one here. Then a third one ends it.";

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn area(s: &mut Session, w: f64, h: f64, text: &str, extra: Value) -> NodeId {
    let mut a = json!({"width": w, "height": h});
    if let (Some(a), Some(e)) = (a.as_object_mut(), extra.as_object()) {
        a.extend(e.clone());
    }
    let id = s.execute("text.create", &json!({"x": 40, "y": 40, "size": 12, "area": a, "text": text})).unwrap()["id"].as_u64().unwrap();
    NodeId(id)
}

fn text(s: &Session, id: NodeId) -> TextObject {
    match &s.doc().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Text(t) => (**t).clone(),
        k => panic!("not text: {k:?}"),
    }
}

fn frame(s: &Session, id: NodeId) -> Rect {
    match text(s, id).kind {
        TextKind::Area { frame } => frame.bounds().unwrap(),
        k => panic!("not area type: {k:?}"),
    }
}

fn overflows(s: &Session, id: NodeId) -> bool {
    vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &text(s, id)).overflow
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

#[test]
fn area_options_set_and_report_the_fit() {
    let mut s = session();
    let id = area(&mut s, 120.0, 30.0, STORY, json!({}));
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    let q = s.execute("text.areaOptions", &json!({})).unwrap();
    assert_eq!((q["fit"].as_str(), q["fitMinPercent"].as_f64()), (Some("none"), Some(50.0)));
    assert_eq!((q["overflow"].as_bool(), q["fitScale"].as_f64()), (Some(true), Some(1.0)));
    let n = undo_len(&s);
    let r = s.execute("text.areaOptions", &json!({"fit": "shrinkText", "fitMinPercent": 20})).unwrap();
    assert_eq!(undo_len(&s), n + 1);
    assert_eq!((r["fit"].as_str(), r["fitMinPercent"].as_f64()), (Some("shrinkText"), Some(20.0)));
    assert_eq!(r["overflow"], false, "{r}");
    let f = r["fitScale"].as_f64().unwrap();
    assert!((0.2..1.0).contains(&f), "{f}");
    assert_eq!(text(&s, id).area.fit, AreaFit::ShrinkText { min_percent: 20.0 });
    assert_eq!(text(&s, id).runs[0].style.size, 12.0, "the stored size stays");
    // The minimum alone, clamped; the object form; and bad values.
    s.execute("text.areaOptions", &json!({"fitMinPercent": 3})).unwrap();
    assert_eq!(text(&s, id).area.fit, AreaFit::ShrinkText { min_percent: 10.0 });
    s.execute("text.areaOptions", &json!({"fit": {"shrinkText": {"minPercent": 95}}})).unwrap();
    assert_eq!(text(&s, id).area.fit, AreaFit::ShrinkText { min_percent: 95.0 });
    assert_eq!(s.execute("text.areaOptions", &json!({})).unwrap()["overflow"], true, "95 % is not enough");
    assert!(s.execute("text.areaOptions", &json!({"fit": "squash"})).is_err());
    assert!(s.execute("text.areaOptions", &json!({"fit": 7})).is_err());
    // The reply's own keys go back in unchanged (the dialog sends everything).
    let q = s.execute("text.areaOptions", &json!({})).unwrap();
    let n = undo_len(&s);
    let again = s.execute("text.areaOptions", &q).unwrap();
    assert_eq!(again, q);
    assert_eq!(undo_len(&s), n + 1);
    // Undo back to no fit.
    while text(&s, id).area.fit != AreaFit::None {
        s.execute("edit.undo", &json!({})).unwrap();
    }
    assert!(overflows(&s, id));
}

#[test]
fn inspect_reports_overflow_and_fit_scale() {
    let mut s = session();
    let id = area(&mut s, 120.0, 30.0, STORY, json!({"fit": "shrinkText", "fitMinPercent": 10}));
    s.execute("text.create", &json!({"x": 10, "y": 500, "text": "point"})).unwrap();
    let doc = s.execute("document.inspect", &json!({})).unwrap();
    let kids = doc["layers"][0]["children"].as_array().unwrap();
    let node = kids.iter().find(|k| k["id"] == id.0).unwrap();
    assert_eq!(node["overflow"], false);
    assert_eq!(node["fit"], "shrinkText");
    assert!(node["fitScale"].as_f64().unwrap() < 1.0);
    let point = kids.iter().find(|k| k["id"] != id.0).unwrap();
    assert!(point.get("overflow").is_none() && point.get("fitScale").is_none(), "point type never overflows");
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("text.areaOptions", &json!({"fit": "none"})).unwrap();
    let doc = s.execute("document.inspect", &json!({})).unwrap();
    let node = doc["layers"][0]["children"].as_array().unwrap().iter().find(|k| k["id"] == id.0).unwrap().clone();
    assert_eq!((node["overflow"].as_bool(), node["fitScale"].as_f64(), node["fit"].as_str()), (Some(true), Some(1.0), Some("none")));
}

#[test]
fn auto_height_follows_edits_in_the_same_undo_step() {
    let mut s = session();
    let id = area(&mut s, 200.0, 300.0, "One line", json!({"fit": "autoHeight"}));
    let one = frame(&s, id);
    assert!(one.height() < 30.0, "the frame fits one line: {one:?}");
    assert_eq!((one.x0, one.y0, one.width()), (0.0, 0.0, 200.0), "top and width stay");
    assert!(!overflows(&s, id));
    let n = undo_len(&s);
    s.execute("text.editRange", &json!({"id": id.0, "start": 8, "insert": format!(" {STORY}")})).unwrap();
    assert_eq!(undo_len(&s), n + 1);
    let grown = frame(&s, id);
    assert!(grown.height() > one.height() * 2.0, "{grown:?}");
    assert!(!overflows(&s, id), "the grown frame holds the text");
    // Tight: a point less overflows.
    let mut t = text(&s, id);
    t.area.fit = AreaFit::None;
    t.kind = TextKind::Area {
        frame: vectorcraft_geom::PathData::from_bezpath(&vectorcraft_geom::Shape::to_path(&Rect::new(0.0, 0.0, 200.0, grown.height() - 1.0), 0.1)),
    };
    assert!(vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t).overflow);
    // Character changes resize it too.
    s.execute("text.setRangeStyle", &json!({"id": id.0, "start": 0, "end": 3, "size": 40})).unwrap();
    assert!(frame(&s, id).height() > grown.height());
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(frame(&s, id), grown);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(frame(&s, id), one, "one undo restores the text and the frame");
    assert_eq!(text(&s, id).plain_text(), "One line");
}

#[test]
fn auto_height_follows_typing_with_the_type_tool() {
    let mut s = session();
    s.prefs.auto_size_area_type = true;
    // An empty frame (no placeholder text), so its first height is one line.
    s.prefs.placeholder_text = false;
    let v = ViewInfo::default();
    s.select_tool("type", v).unwrap();
    for (kind, (x, y)) in [(PointerKind::Down, (100.0, 100.0)), (PointerKind::Drag, (300.0, 400.0)), (PointerKind::Up, (300.0, 400.0))] {
        s.pointer(&PointerEvent::new(kind, x, y), v).unwrap();
    }
    let id = s.doc().unwrap().selection.objects[0];
    assert_eq!(text(&s, id).area.fit, AreaFit::AutoHeight, "the preference gives new area type Auto Size");
    let empty = frame(&s, id);
    assert!(empty.height() < 30.0, "{empty:?}");
    let n = undo_len(&s);
    for word in STORY.split_inclusive(' ') {
        s.tool_text(word, v).unwrap();
    }
    s.tool_key(ToolKey::Escape, Mods::default(), v).unwrap();
    assert_eq!(text(&s, id).plain_text(), STORY);
    assert_eq!(undo_len(&s), n + 1, "one typing step");
    let typed = frame(&s, id);
    assert!(typed.height() > empty.height() * 2.0, "{typed:?}");
    assert!(!overflows(&s, id));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(frame(&s, id), empty);
    assert_eq!(text(&s, id).plain_text(), "");
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(frame(&s, id), typed);
}

#[test]
fn auto_height_in_columns_and_by_hand() {
    let mut s = session();
    let id = area(&mut s, 300.0, 400.0, &STORY.repeat(3), json!({"fit": "autoHeight"}));
    let one = frame(&s, id).height();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    // Height is ignored while Auto Size is on.
    let r = s.execute("text.areaOptions", &json!({"columns": 2, "gutter": 12, "height": 500})).unwrap();
    let two = r["height"].as_f64().unwrap();
    assert!(two < 500.0 && two > one * 0.5, "{one} → {two}");
    assert_eq!(r["overflow"], false);
    assert_eq!(r["fit"], "autoHeight");
    // The least height the two columns fit: a point less overflows, and both columns are used.
    let mut t = text(&s, id);
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
    assert!(lay.glyphs.iter().any(|g| g.origin.x > 160.0), "text flows into the second column");
    t.area.fit = AreaFit::None;
    t.kind = TextKind::Area {
        frame: vectorcraft_geom::PathData::from_bezpath(&vectorcraft_geom::Shape::to_path(&Rect::new(0.0, 0.0, 300.0, two - 1.0), 0.1)),
    };
    assert!(vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t).overflow);
    // Dragging the bottom edge sets the height by hand: Auto Size goes off.
    let b = frame(&s, id);
    s.execute("text.reshapeArea", &json!({"id": id.0, "anchors": [[0, 2], [0, 3]], "dx": 0, "dy": 50})).unwrap();
    assert_eq!(text(&s, id).area.fit, AreaFit::None);
    assert!((frame(&s, id).height() - (b.height() + 50.0)).abs() < 1e-6);
    s.execute("edit.undo", &json!({})).unwrap();
    // A new width keeps it: the height follows the reflow.
    s.execute("text.reshapeArea", &json!({"id": id.0, "anchors": [[0, 1], [0, 2]], "dx": 100, "dy": 0})).unwrap();
    assert_eq!(text(&s, id).area.fit, AreaFit::AutoHeight);
    assert!(frame(&s, id).height() < b.height());
    // Point type has no frame to size.
    let p = s.execute("text.create", &json!({"x": 10, "y": 10, "text": "x"})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [p]})).unwrap();
    assert!(s.execute("text.areaOptions", &json!({"fit": "autoHeight"})).is_err(), "point type has no area options");
}

#[test]
fn shrink_text_round_trips_through_the_native_format() {
    let mut s = session();
    let id = area(&mut s, 120.0, 30.0, STORY, json!({"fit": "shrinkText", "fitMinPercent": 35}));
    let doc = s.doc().unwrap().doc.clone();
    let bytes = vectorcraft_format::save(&doc, false);
    let back = vectorcraft_format::load(&bytes).unwrap();
    let NodeKind::Text(t) = &back.node(id).unwrap().kind else { panic!() };
    assert_eq!(t.area.fit, AreaFit::ShrinkText { min_percent: 35.0 });
}

#[test]
fn auto_height_follows_the_leading_model_and_character_alignment() {
    let mut s = session();
    let id = area(&mut s, 200.0, 300.0, STORY, json!({"fit": "autoHeight"}));
    let roman = frame(&s, id);
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    // Top-to-Top leading: the first line moves up to touch the frame, the frame follows.
    s.execute("text.setFormat", &json!({"leadingModel": "emBoxTop"})).unwrap();
    let em = frame(&s, id);
    assert_ne!(em.height(), roman.height());
    assert!(!overflows(&s, id), "the frame holds the text under emBoxTop");
    // Mixed sizes aligned on the em box centre still fit exactly.
    s.execute("text.setRangeStyle", &json!({"id": id.0, "start": 0, "end": 3, "size": 30})).unwrap();
    s.execute("text.setFormat", &json!({"charAlign": "emBoxCenter"})).unwrap();
    let mixed = frame(&s, id);
    assert!(mixed.height() > em.height());
    assert!(!overflows(&s, id));
    let mut t = text(&s, id);
    t.area.fit = AreaFit::None;
    t.kind = TextKind::Area {
        frame: vectorcraft_geom::PathData::from_bezpath(&vectorcraft_geom::Shape::to_path(&Rect::new(0.0, 0.0, 200.0, mixed.height() - 1.0), 0.1)),
    };
    assert!(vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t).overflow, "tight");
}
