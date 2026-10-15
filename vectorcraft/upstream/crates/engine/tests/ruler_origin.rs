//! #1212: ruler coordinates are saved metadata, independent of the artwork's coordinates.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::{Value, json};
use vectorcraft_engine::Session;
use vectorcraft_format::{SaveOptions, load, save, save_with};

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 100, "y": 50, "width": 80, "height": 40})).unwrap();
    s.execute("guide.add", &json!({"vertical": true, "pos": 125})).unwrap();
    s
}

fn origin(s: &mut Session) -> Value {
    s.execute("document.rulerOrigin", &json!({})).unwrap()
}

#[test]
fn changing_origin_preserves_artwork_and_selection_and_is_one_undo_step() {
    let mut s = session();
    let before = s.doc().unwrap().doc.clone();
    let selection = s.doc().unwrap().selection.clone();
    let undo = s.doc().unwrap().history.undo.len();
    assert_eq!(origin(&mut s), json!({"x": 0.0, "y": 0.0}));
    let p = json!({"x": 125.5, "y": -30.25});
    s.execute("document.setRulerOrigin", &p).unwrap();
    assert_eq!(origin(&mut s), p);
    assert_eq!(s.doc().unwrap().selection, selection);
    let mut changed = s.execute("document.json", &json!({})).unwrap();
    changed.as_object_mut().unwrap().remove("ruler_origin");
    assert_eq!(changed, serde_json::to_value(&*before).unwrap(), "only the origin changed");
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    let revision = s.doc().unwrap().revision;
    s.execute("document.setRulerOrigin", &p).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "same origin is a no-op");
    assert_eq!(s.doc().unwrap().revision, revision);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(&*s.doc().unwrap().doc, &*before);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(origin(&mut s), p);
}

#[test]
fn reset_uses_the_requested_artboards_top_left_and_ignores_noops() {
    let mut s = session();
    s.execute("artboard.new", &json!({"x": 700, "y": -250, "width": 200, "height": 150})).unwrap();
    s.execute("document.setRulerOrigin", &json!({"x": 25, "y": 50})).unwrap();
    let boards = s.doc().unwrap().doc.artboards.clone();
    s.execute("document.resetRulerOrigin", &json!({"artboard": 1})).unwrap();
    assert_eq!(origin(&mut s), json!({"x": 700.0, "y": -250.0}));
    assert_eq!(s.doc().unwrap().doc.artboards, boards);
    let undo = s.doc().unwrap().history.undo.len();
    s.execute("document.resetRulerOrigin", &json!({"artboard": 1})).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
    for p in [json!({"artboard": 9}), json!({"artboard": -1}), json!({"artboard": "first"})] {
        assert!(s.execute("document.resetRulerOrigin", &p).is_err());
        assert_eq!(s.doc().unwrap().history.undo.len(), undo);
    }
    s.execute("document.resetRulerOrigin", &json!({})).unwrap();
    assert_eq!(origin(&mut s), json!({"x": 0.0, "y": 0.0}));
}

#[test]
fn native_versions_and_compression_keep_the_origin_and_old_files_default_to_zero() {
    let mut s = session();
    let legacy = String::from_utf8(save(&s.doc().unwrap().doc, false)).unwrap().replacen("\"format\":\"vectorcraft\"", "\"format\":\"drawcraft\"", 1);
    assert!(serde_json::to_value(load(legacy.as_bytes()).unwrap()).unwrap().get("ruler_origin").is_none());
    let p = json!({"x": -12.5, "y": 27.75});
    s.execute("document.setRulerOrigin", &p).unwrap();
    let doc = &s.doc().unwrap().doc;
    for (version, compress) in [(1, false), (2, false), (3, false), (4, false), (4, true)] {
        let bytes = save_with(doc, &SaveOptions { version, compress, ..Default::default() }).unwrap();
        let back = load(&bytes).unwrap();
        assert_eq!(serde_json::to_value(&back).unwrap()["ruler_origin"], p, "v{version}, compression {compress}");
        assert_eq!(back.layers, doc.layers);
        assert_eq!(back.guides, doc.guides);
    }
}

#[test]
fn origin_rejects_invalid_coordinates_without_changing_absolute_command_coordinates() {
    let mut s = session();
    s.execute("document.setRulerOrigin", &json!({"x": 200, "y": 150})).unwrap();
    let before = s.doc().unwrap().doc.clone();
    let undo = s.doc().unwrap().history.undo.len();
    for p in
        [json!({"x": 4_000_001, "y": 0}), json!({"x": 0, "y": -4_000_001}), json!({"x": "10", "y": 0}), json!({"x": 0}), json!({"x": null, "y": 0})]
    {
        assert!(s.execute("document.setRulerOrigin", &p).is_err(), "{p}");
        assert_eq!(&*s.doc().unwrap().doc, &*before);
        assert_eq!(s.doc().unwrap().history.undo.len(), undo);
    }
    s.execute("object.setBounds", &json!({"x": 80, "y": 40, "reference": 0})).unwrap();
    let bounds = s.doc().unwrap().doc.bounds_of(&s.doc().unwrap().selection.objects, false).unwrap();
    assert_eq!((bounds.x0, bounds.y0), (80.0, 40.0), "engine coordinates stay absolute");
    assert_eq!(origin(&mut s), json!({"x": 200.0, "y": 150.0}));
}
