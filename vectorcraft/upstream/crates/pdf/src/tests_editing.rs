//! Preserve Editing: the native document as an embedded file, found again (also under its legacy
//! name) and checked against the pages it was written with.

use serde_json::json;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Node};
use vectorcraft_geom::{Rect, shapes};

use crate::*;

fn doc() -> Document {
    let mut d = Document::new(100.0, 100.0);
    let layer = d.default_layer().unwrap();
    let id = d.alloc_id();
    let n = Node::path(id, shapes::rectangle(Rect::new(10.0, 10.0, 50.0, 50.0)), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
    d.insert(Some(layer), 0, n).unwrap();
    d
}

/// The document's PDF with `native` embedded (content streams and the payload uncompressed when
/// `compress` is false).
fn pdf_with(native: Option<&[u8]>, compress: bool) -> ExportReport {
    let mut settings: PdfSettings = serde_json::from_value(json!({"preserveEditing": true})).unwrap();
    settings.compression.compress_text = compress;
    export_with_report(&doc(), &PdfOptions { settings, native: native.map(<[u8]>::to_vec), created: Some(0), ..Default::default() }).unwrap()
}

/// `pdf` with `from` replaced by `to` (the same length, so the cross-reference stays valid).
fn patched(pdf: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    assert_eq!(from.len(), to.len());
    let at = pdf.windows(from.len()).position(|w| w == from).unwrap_or_else(|| panic!("no {:?}", String::from_utf8_lossy(from)));
    let mut out = pdf.to_vec();
    out[at..at + to.len()].copy_from_slice(to);
    out
}

#[test]
fn the_native_document_comes_back_intact() {
    for compress in [true, false] {
        let r = pdf_with(Some(b"{\"native\": true}"), compress);
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        assert!(r.bytes.starts_with(b"%PDF-"));
        let e = editing(&r.bytes).unwrap();
        assert_eq!((e.data.as_slice(), e.intact), (&b"{\"native\": true}"[..], true));
        // The import reports it too, and the pages still import as artwork.
        let imported = import_with_report(&r.bytes, &ImportOptions::default()).unwrap();
        assert_eq!(imported.native, Some(e));
        assert_eq!(imported.document.artboards.len(), 1);
    }
}

#[test]
fn changed_pages_are_noticed() {
    let r = pdf_with(Some(b"native"), false);
    let resized = patched(&r.bytes, b"/MediaBox[0 0 100 100]", b"/MediaBox[0 0 100 101]");
    assert!(!editing(&resized).unwrap().intact, "another page size");
    // Drawing changed elsewhere: the rectangle's first corner moves.
    let moved = patched(&r.bytes, b"10 10 m", b"10 11 m");
    assert!(!editing(&moved).unwrap().intact, "other page content");
    // The description (not part of the hash) can change: re-saving keeps the data usable.
    assert!(editing(&r.bytes).unwrap().intact);
}

#[test]
fn the_legacy_name_is_read_and_plain_pdfs_have_no_editing_data() {
    let r = pdf_with(Some(b"legacy"), false);
    let name = format!("({EDITING_FILE})");
    let legacy = format!("({LEGACY_EDITING_FILE}){}", " ".repeat(EDITING_FILE.len() - LEGACY_EDITING_FILE.len()));
    let mut old = r.bytes.clone();
    // Every place the name is written (/F, /UF and the name tree key).
    while let Some(at) = old.windows(name.len()).position(|w| w == name.as_bytes()) {
        old[at..at + legacy.len()].copy_from_slice(legacy.as_bytes());
    }
    let e = editing(&old).unwrap();
    assert_eq!((e.data.as_slice(), e.intact), (&b"legacy"[..], true));
    // Off, or with no native document given: nothing embedded (the latter warns).
    let plain = export(&doc(), &PdfOptions::default()).unwrap();
    assert_eq!(editing(&plain), None);
    assert_eq!(import_with_report(&plain, &ImportOptions::default()).unwrap().native, None);
    let r = pdf_with(None, true);
    assert!(r.warnings.len() == 1 && r.warnings[0].contains("Preserve editing"), "{:?}", r.warnings);
    assert_eq!(editing(&r.bytes), None);
}

#[test]
fn pdf_a_carries_no_editing_data() {
    let s: PdfSettings = serde_json::from_value(json!({"standard": "pdfA2b", "preserveEditing": true})).unwrap();
    assert!(matches!(s.check(), Err(PdfError::BadSetting(m)) if m.contains("editing")));
    assert!(matches!(s.check_values(), Err(PdfError::BadSetting(_))));
}
