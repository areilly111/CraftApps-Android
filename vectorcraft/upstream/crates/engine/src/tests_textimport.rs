//! File → Place of a text file: Text Import Options, and the text set as area type.

use serde_json::{Value, json};
use vectorcraft_doc::{NodeKind, TextKind};
use vectorcraft_geom::Rect;

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s
}

fn place_text(s: &mut Session, text: &[u8], extra: Value) -> Value {
    let mut p = json!({"name": "notes.txt", "dataBase64": vectorcraft_format::base64_encode(text)});
    if let (Some(o), Value::Object(e)) = (p.as_object_mut(), extra) {
        o.extend(e);
    }
    s.execute("file.place", &p).unwrap()
}

/// The placed type's text and frame bounds.
fn placed(s: &Session, r: &Value) -> (String, Rect) {
    let n = s.doc().unwrap().doc.node(NodeId(r["ids"][0].as_u64().unwrap())).unwrap().clone();
    let NodeKind::Text(t) = &n.kind else { panic!("not type: {:?}", n.kind) };
    assert!(matches!(t.kind, TextKind::Area { .. }), "area type");
    (t.plain_text(), n.geometric_bounds().unwrap())
}

fn close(a: Rect, b: Rect) -> bool {
    [a.x0 - b.x0, a.y0 - b.y0, a.x1 - b.x1, a.y1 - b.y1].iter().all(|d| d.abs() < 1e-6)
}

#[test]
fn crlf_with_returns_removed_places_one_paragraph_per_block_as_area_type() {
    let mut s = session();
    let text = b"Dear reader,\r\nthis is the first\r\nparagraph.\r\n\r\nAnd   here\r\nthe second.\r\n";
    let opts = json!({"text": {"removeLineReturns": true, "removeParagraphReturns": true, "replaceSpaces": 3}});
    let r = place_text(&mut s, text, opts);
    assert_eq!((r["format"].as_str(), r["name"].as_str(), r["linked"].as_bool()), (Some("txt"), Some("notes.txt"), Some(false)));
    let (body, frame) = placed(&s, &r);
    assert_eq!(body, "Dear reader, this is the first paragraph.\nAnd\there the second.");
    assert!(close(frame, Rect::new(36.0, 36.0, 364.0, 264.0)), "the artboard less a 36 pt margin: {frame:?}");
    let st = s.doc().unwrap();
    assert_eq!((st.history.undo.len(), st.selection.objects.len()), (1, 1), "one undo step, selected");
    // Without options each line is a paragraph; `rect` sets the frame.
    let r = place_text(&mut s, text, json!({"rect": [10, 20, 100, 50]}));
    let (body, frame) = placed(&s, &r);
    assert_eq!(body, "Dear reader,\nthis is the first\nparagraph.\n\nAnd   here\nthe second.");
    assert!(close(frame, Rect::new(10.0, 20.0, 110.0, 70.0)), "{frame:?}");
}

#[test]
fn text_files_report_and_queue_with_their_options() {
    let mut s = session();
    let file = json!({"name": "a.txt", "dataBase64": vectorcraft_format::base64_encode(b"caf\xe9")});
    let info = s.execute("file.place.info", &file).unwrap();
    assert_eq!((info["format"].as_str(), info["width"].as_f64(), info["height"].as_f64()), (Some("txt"), Some(468.0), Some(648.0)));
    // Not UTF-8: read as Windows-1252, or Mac Roman for a Mac file.
    let r = place_text(&mut s, b"caf\x8e", json!({"text": {"characterSet": "ansi", "platform": "mac"}}));
    assert_eq!(placed(&s, &r).0, "café");
    let r = s.execute("file.place.queue", &json!({"files": [file], "text": {"characterSet": "ansi"}})).unwrap();
    assert_eq!((r["count"].as_u64(), r["files"][0]["format"].as_str()), (Some(1), Some("txt")));
    assert_eq!(s.tool_options()["name"], "a.txt");
    let bad = json!({"name": "a.txt", "dataBase64": "YQ==", "text": {"platform": "beos"}});
    assert!(s.execute("file.place", &bad).is_err());
    let blank = json!({"name": "blank.txt", "dataBase64": vectorcraft_format::base64_encode(b" \r\n\t\r\n")});
    assert!(s.execute("file.place", &blank).is_err(), "no text to place");
    // Replace: the frame takes the replaced object's bounds.
    s.execute("shape.rectangle", &json!({"x": 50, "y": 60, "width": 80, "height": 40})).unwrap();
    let r = place_text(&mut s, b"Swap", json!({"replace": true}));
    let (body, frame) = placed(&s, &r);
    assert_eq!(body, "Swap");
    assert!(close(frame, Rect::new(50.0, 60.0, 130.0, 100.0)), "{frame:?}");
}

#[test]
fn place_reads_what_open_reads_and_text_files() {
    use cmd::fileio::{OPEN_EXTS, PLACE_EXTS, TEXT_EXTS};
    assert_eq!(PLACE_EXTS, [OPEN_EXTS, TEXT_EXTS].concat());
    let filters: Vec<_> = cmd::fileio::place_filters().collect();
    assert_eq!((filters.first(), filters.last()), (Some(&("All placeable files", PLACE_EXTS)), Some(&("Text", TEXT_EXTS))));
    assert!(!cmd::fileio::open_filters().any(|(_, exts)| exts.contains(&"txt")), "Open doesn't read text files");
}
