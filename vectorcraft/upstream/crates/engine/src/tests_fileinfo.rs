//! File → File Info (`file.info`): editing as one undo step, the native round trip, and the PDF,
//! PNG and SVG metadata it writes.

use serde_json::{Value, json};
use vectorcraft_doc::{CopyrightStatus, DocMetadata};

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 80, "title": "Poster"})).unwrap();
    s
}

fn info() -> Value {
    json!({
        "author": "Ada Lovelace",
        "authorTitle": "Designer",
        "description": "A poster\nfor the fair",
        "keywords": "fair, Poster; print, poster",
        "rating": 4,
        "copyrightStatus": "copyrighted",
        "copyrightNotice": "(c) 2026 Ada",
        "copyrightUrl": "https://example.com/rights",
    })
}

fn b64(v: &Value) -> Vec<u8> {
    vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap()
}

#[test]
fn file_info_is_one_undo_step_and_reports_everything() {
    let mut s = session();
    let r = s.execute("file.info", &info()).unwrap();
    assert_eq!(r["keywords"], json!(["fair", "Poster", "print"]), "keywords split, trimmed, each once ignoring case");
    assert_eq!((r["rating"].as_u64(), r["copyrightStatus"].as_str()), (Some(4), Some("copyrighted")));
    assert_eq!(r["description"], "A poster\nfor the fair");
    assert!(r["created"].as_str().is_some_and(|c| c.ends_with('Z')), "a new document has its creation date: {}", r["created"]);
    assert!(r["modified"].is_null(), "never saved");
    assert_eq!(r["title"], "Poster");
    let st = s.doc().unwrap();
    assert_eq!(st.history.undo.len(), 1);
    assert_eq!(st.doc.metadata.author, "Ada Lovelace");
    // The report fed back changes nothing (read-only values are ignored).
    let again = s.execute("file.info", &r).unwrap();
    assert_eq!(again, r);
    assert_eq!(s.doc().unwrap().history.undo.len(), 1);
    s.execute("edit.undo", &json!({})).unwrap();
    let m = &s.doc().unwrap().doc.metadata;
    assert_eq!((m.author.as_str(), m.keywords.len()), ("", 0));
    // Clearing a field with null.
    s.execute("edit.redo", &json!({})).unwrap();
    s.execute("file.info", &json!({"author": null, "keywords": []})).unwrap();
    let m = &s.doc().unwrap().doc.metadata;
    assert_eq!((m.author.as_str(), m.keywords.len(), m.rating), ("", 0, 4));
}

#[test]
fn junk_is_rejected() {
    let mut s = session();
    for bad in [
        json!({"rating": 9}),
        json!({"rating": -1}),
        json!({"rating": "five"}),
        json!({"keywords": 5}),
        json!({"keywords": ["ok", 3]}),
        json!({"copyrightStatus": "maybe"}),
        json!({"copyrightUrl": "not a url"}),
        json!({"author": "two\nlines"}),
        json!({"description": "bell\u{7}"}),
        json!({"title": "  "}),
        json!({"author": 3}),
        json!({"favouriteColour": "blue"}),
        json!([1, 2]),
        json!({"keywords": "x".repeat(300)}),
    ] {
        assert!(s.execute("file.info", &bad).is_err(), "{bad}");
    }
    assert_eq!(s.doc().unwrap().history.undo.len(), 0);
    assert_eq!(s.doc().unwrap().doc.metadata, DocMetadata { created: s.doc().unwrap().doc.metadata.created, ..Default::default() });
}

#[test]
fn file_info_round_trips_and_old_files_load() {
    let mut s = session();
    s.execute("file.info", &info()).unwrap();
    let d = &s.doc().unwrap().doc;
    let back = vectorcraft_format::load(&vectorcraft_format::save_file(d)).unwrap();
    assert_eq!(back.metadata, d.metadata);
    assert_eq!(back.metadata.copyright_status, CopyrightStatus::Copyrighted);
    // A document saved before File Info existed has none.
    let mut old = vectorcraft_doc::Document::new(10.0, 10.0);
    old.metadata = DocMetadata::default();
    let bytes = vectorcraft_format::save_file(&old);
    assert!(!String::from_utf8_lossy(&bytes).contains("\"metadata\""), "default File Info isn't written");
    assert_eq!(vectorcraft_format::load(&bytes).unwrap().metadata, DocMetadata::default());
}

#[test]
fn saving_to_a_file_stamps_the_modified_date() {
    let mut s = session();
    let created = s.doc().unwrap().doc.metadata.created;
    let path = std::env::temp_dir().join(format!("vc-fileinfo-{}.vectorcraft", std::process::id()));
    s.execute("document.save", &json!({"path": path.to_string_lossy()})).unwrap();
    let st = s.doc().unwrap();
    assert!(st.doc.metadata.modified.is_some() && st.doc.metadata.created == created);
    assert!(!st.is_dirty(), "the stamped document is the saved one");
    let back = vectorcraft_format::load(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(back.metadata.modified, st.doc.metadata.modified);
    let _ = std::fs::remove_file(&path);
    assert!(s.execute("file.info", &json!({})).unwrap()["modified"].is_string());
}

#[test]
fn pdf_info_carries_author_subject_and_keywords() {
    let mut s = session();
    s.execute("file.info", &info()).unwrap();
    let pdf = crate::export_pdf(&s.doc().unwrap().doc, &vectorcraft_pdf::PdfOptions::uncompressed()).unwrap();
    let text = String::from_utf8_lossy(&pdf);
    // (The description has a line break, so it is written as UTF-16.)
    for want in ["/Author(Ada Lovelace)", "/Keywords(fair, Poster, print)", "/Title(Poster)", "/Subject<FEFF0041"] {
        assert!(text.contains(want), "PDF Info has {want}");
    }
    // XMP too.
    assert!(text.contains("<dc:creator>") && text.contains("Ada Lovelace"));
}

#[test]
fn png_exports_carry_text_chunks() {
    let mut s = session();
    s.execute("file.info", &info()).unwrap();
    let png = b64(&s.execute("document.export", &json!({"format": "png"})).unwrap());
    let text = crate::cmd::fileio::pngtext::text(&png);
    let get = |k: &str| text.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());
    assert_eq!(get("Title"), Some("Poster"));
    assert_eq!(get("Author"), Some("Ada Lovelace"));
    assert_eq!(get("Keywords"), Some("fair, Poster, print"));
    assert_eq!(get("Copyright"), Some("(c) 2026 Ada"));
    assert!(get("Creation Time").is_some_and(|t| t.ends_with('Z')));
    assert!(image::load_from_memory(&png).is_ok(), "still a valid PNG");
}

#[test]
fn svg_metadata_carries_file_info_as_dublin_core() {
    let mut s = session();
    s.execute("file.info", &info()).unwrap();
    let svg = String::from_utf8(b64(&s.execute("document.export", &json!({"format": "svg", "metadata": true})).unwrap())).unwrap();
    for want in [
        "<dc:title>Poster</dc:title>",
        "<dc:creator>Ada Lovelace</dc:creator>",
        "<dc:subject>fair</dc:subject>",
        "<dc:subject>print</dc:subject>",
        "<dc:rights>(c) 2026 Ada</dc:rights>",
        "<dc:rights>https://example.com/rights</dc:rights>",
        "<desc>A poster\nfor the fair</desc>",
    ] {
        assert!(svg.contains(want), "SVG has {want}");
    }
    // Without the option: no <metadata>, but the title and description stay.
    let plain = String::from_utf8(b64(&s.execute("document.export", &json!({"format": "svg"})).unwrap())).unwrap();
    assert!(!plain.contains("<metadata>") && plain.contains("<desc>"));
}
