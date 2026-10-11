//! The native save options of the file itself: Include Linked Files, embedded ICC profiles and the
//! PDF a PDF-compatible file carries, each round-tripping (and ignored by readers that don't
//! need them).
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use serde_json::Value;
use vectorcraft_doc::links::LinkInfo;
use vectorcraft_doc::{Document, ImageBlob, ImageObject, Node, NodeKind};
use vectorcraft_format::{ICC_MIME, SaveOptions, load, load_file, pdf_content, save_with};
use vectorcraft_geom::Affine;

fn json_of(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).unwrap()
}

/// A document showing image `k` linked to a file; the blob holds the file's bytes and a preview.
fn linked_doc() -> Document {
    let mut d = Document::new(100.0, 100.0);
    let layer = d.layers[0].id;
    let id = d.alloc_id();
    let im = ImageObject {
        key: "k".into(),
        width: 2,
        height: 2,
        xf: Affine::IDENTITY,
        link: Some(LinkInfo::new("/art/photo.png")),
        placement: Default::default(),
    };
    d.insert(Some(layer), 0, Node::new(id, NodeKind::Image(im))).unwrap();
    let mut blob = ImageBlob::new("image/png", vec![1, 2, 3, 4, 5, 6]);
    blob.proxy = Some(Arc::new(vec![9, 9]));
    d.images.insert("k".into(), blob);
    d
}

#[test]
fn include_linked_files_keeps_the_files_bytes() {
    let d = linked_doc();
    let plain = save_with(&d, &SaveOptions::default()).unwrap();
    let image = &json_of(&plain)["images"]["k"];
    assert_eq!(image["proxy"], true, "only the preview by default");
    let full = save_with(&d, &SaveOptions { include_linked: true, ..SaveOptions::default() }).unwrap();
    let image = &json_of(&full)["images"]["k"];
    assert!(image.get("proxy").is_none());
    let back = load(&full).unwrap();
    assert_eq!(back.images["k"].bytes.as_slice(), &[1, 2, 3, 4, 5, 6]);
    assert!(!back.images["k"].is_proxy());
    // Only the preview loaded (the file was never found): the preview is all there is to write.
    let mut missing = linked_doc();
    let preview = Arc::new(vec![9, 9]);
    missing.images.insert("k".into(), ImageBlob { mime: "image/png".into(), bytes: preview.clone(), proxy: Some(preview) });
    let out = save_with(&missing, &SaveOptions { include_linked: true, ..SaveOptions::default() }).unwrap();
    assert_eq!(json_of(&out)["images"]["k"]["proxy"], true);
}

#[test]
fn embedded_profiles_and_pdf_round_trip() {
    let mut d = Document::new(100.0, 100.0);
    d.color_profiles.rgb = Some("Studio RGB".into());
    let o = SaveOptions {
        profiles: [("Studio RGB".to_string(), vec![0, 1, 2, 3, 250])].into(),
        pdf: Some(b"%PDF-1.7 fake".to_vec()),
        compress: true,
        ..SaveOptions::default()
    };
    let bytes = save_with(&d, &o).unwrap();
    let f = load_file(&bytes).unwrap();
    assert_eq!(f.profiles["Studio RGB"], vec![0, 1, 2, 3, 250]);
    assert_eq!(f.doc.color_profiles.rgb.as_deref(), Some("Studio RGB"));
    assert_eq!(pdf_content(&bytes).unwrap(), b"%PDF-1.7 fake");
    // The plain file says what they are; a file without them has neither member.
    let plain = save_with(&d, &SaveOptions { compress: false, ..o }).unwrap();
    let v = json_of(&plain);
    assert_eq!((v["profiles"]["Studio RGB"]["mime"].as_str(), v["pdf"]["mime"].as_str()), (Some(ICC_MIME), Some("application/pdf")));
    let none = save_with(&d, &SaveOptions::default()).unwrap();
    assert!(json_of(&none).get("profiles").is_none() && pdf_content(&none).is_none());
    assert!(load_file(&none).unwrap().profiles.is_empty());
    // Damaged profile data is left out rather than failing the open.
    let damaged = String::from_utf8(plain).unwrap().replacen("AAECA/o=", "not base64!", 1);
    assert!(load_file(damaged.as_bytes()).unwrap().profiles.is_empty());
}
