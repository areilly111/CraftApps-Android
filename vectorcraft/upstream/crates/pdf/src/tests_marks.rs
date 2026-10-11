//! Marks and Bleeds: page boxes, the art kept in the bleed, non-printing layers and printer's marks.

use std::sync::Arc;

use serde_json::json;
use vectorcraft_color::{Color, Paint, Swatch};
use vectorcraft_doc::marks::PrinterMarks;
use vectorcraft_doc::{Appearance, Document, LayerColor, Node, NodeKind};
use vectorcraft_geom::{Rect, shapes};

use crate::*;

/// A 200 × 100 document with a red square reaching 5 pt left of its artboard.
fn doc() -> Document {
    let mut d = Document::new(200.0, 100.0);
    let layer = d.default_layer().unwrap();
    let id = d.alloc_id();
    let red = Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0);
    d.insert(Some(layer), 0, Node::path(id, shapes::rectangle(Rect::new(-5.0, 10.0, 40.0, 50.0)), red)).unwrap();
    d
}

fn settings(v: serde_json::Value) -> PdfSettings {
    serde_json::from_value(v).unwrap()
}

/// The PDF of `d` with settings `v`; marks and bleed raise no warnings.
fn pdf(d: &Document, v: serde_json::Value) -> Vec<u8> {
    let opts = PdfOptions { settings: settings(v), created: Some(1_791_200_000), ..PdfOptions::uncompressed() };
    let r = export_with_report(d, &opts).unwrap();
    assert!(r.warnings.iter().all(|w| !w.contains("bleed") && !w.contains("marks")), "{:?}", r.warnings);
    r.bytes
}

/// Page 1's box `which` (PDF space: y up).
fn page_box(bytes: &[u8], which: CropTo) -> Rect {
    info(bytes, None).unwrap().pages[0].boxes.iter().find(|(c, _)| *c == which).unwrap().1
}

/// The bounds of every path of `d` painted with a paint `pick` accepts.
fn painted(d: &Document, pick: impl Fn(&Paint) -> bool) -> Vec<Rect> {
    let mut out = vec![];
    d.walk(|n| {
        let paints = n.appearance.fill().map(|f| &f.paint).into_iter().chain(n.appearance.stroke().map(|s| &s.paint));
        if paints.into_iter().any(&pick)
            && let Some(b) = n.path_data().and_then(|p| p.bounds())
        {
            out.push(b);
        }
    });
    out
}

fn rgb(p: &Paint, rgb: [u8; 3]) -> bool {
    p.color().is_some_and(|c| c.to_rgba8(1.0)[..3] == rgb)
}

#[test]
fn bleed_grows_the_media_box_around_the_trim_box() {
    let d = doc();
    // No bleed and no marks: the page is the artboard.
    let plain = pdf(&d, json!({}));
    assert_eq!(page_box(&plain, CropTo::Media), Rect::new(0.0, 0.0, 200.0, 100.0));
    assert_eq!(page_box(&plain, CropTo::Trim), Rect::new(0.0, 0.0, 200.0, 100.0));
    // A 9 pt bleed, custom or the document's.
    let mut nine = d.clone();
    nine.setup.bleed = [9.0; 4];
    for v in [json!({"bleed": {"top": 9, "bottom": 9, "left": 9, "right": 9}}), json!({"bleed": {"useDocument": true, "top": 1}})] {
        let bytes = pdf(&nine, v.clone());
        let media = page_box(&bytes, CropTo::Media);
        assert_eq!((media.width(), media.height()), (218.0, 118.0), "{v}: MediaBox = artboard + 2 × 9 pt");
        assert_eq!(page_box(&bytes, CropTo::Trim), Rect::new(9.0, 9.0, 209.0, 109.0), "{v}: TrimBox = artboard");
        assert_eq!(page_box(&bytes, CropTo::Bleed), media, "{v}: BleedBox = artboard + bleed");
    }
    // A document bleed read from a file is kept in range.
    nine.setup.bleed = [f64::NAN, -3.0, 1e9, 2.0];
    let b = pdf(&nine, json!({"bleed": {"useDocument": true}}));
    assert_eq!(page_box(&b, CropTo::Media), Rect::new(0.0, 0.0, 274.0, 100.0));
    // Uneven bleed (PDF space is y up: the bottom bleed is below the trim box).
    let bytes = pdf(&d, json!({"bleed": {"top": 4, "bottom": 2, "left": 3, "right": 1}}));
    assert_eq!(page_box(&bytes, CropTo::Media), Rect::new(0.0, 0.0, 204.0, 106.0));
    assert_eq!(page_box(&bytes, CropTo::Trim), Rect::new(3.0, 2.0, 203.0, 102.0));
}

#[test]
fn art_in_the_bleed_is_kept_and_clipped_to_the_bleed_box() {
    let d = doc();
    let back = import(&pdf(&d, json!({"bleed": {"left": 9, "top": 9, "bottom": 9, "right": 9}}))).unwrap();
    let red = painted(&back, |p| rgb(p, [255, 0, 0]));
    assert_eq!(red.len(), 1);
    assert!((red[0].x0 - 4.0).abs() < 0.01, "the square reaches 5 pt into the 9 pt bleed: {red:?}");
    // With marks the sheet is larger than the bleed box: the art is clipped to it.
    let bytes = pdf(&d, json!({"bleed": {"left": 3}, "marks": {"trim": true}}));
    let (trim, bleed) = (page_box(&bytes, CropTo::Trim), page_box(&bytes, CropTo::Bleed));
    assert!((trim.x0 - bleed.x0 - 3.0).abs() < 1e-3 && bleed.x0 > 0.0, "{trim:?} {bleed:?}");
    let back = import(&bytes).unwrap();
    let mut clips = vec![];
    back.walk(|n| {
        if let NodeKind::Group { children, clip: true } = &n.kind {
            clips.extend(children.first().and_then(|c| c.path_data()).and_then(|p| p.bounds()));
        }
    });
    let media = page_box(&bytes, CropTo::Media);
    let bleed_doc = Rect::new(bleed.x0, media.y1 - bleed.y1, bleed.x1, media.y1 - bleed.y0);
    assert!(clips.iter().any(|c| (c.x0 - bleed_doc.x0).abs() < 0.01 && (c.x1 - bleed_doc.x1).abs() < 0.01), "{clips:?} vs {bleed_doc:?}");
}

#[test]
fn non_printing_layers_are_left_out_unless_asked_for() {
    let mut d = doc();
    let mut layer = Node::layer(d.alloc_id(), "Notes", LayerColor::Preset(1));
    let blue = Appearance::basic(Paint::solid(Color::rgb(0.0, 0.0, 1.0)), Paint::None, 0.0);
    let note = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(100.0, 20.0, 150.0, 60.0)), blue);
    if let NodeKind::Layer { children, printable, .. } = &mut layer.kind {
        *printable = false;
        children.push(Arc::new(note));
    }
    d.layers.push(Arc::new(layer));
    let blues = |v| painted(&import(&pdf(&d, v)).unwrap(), |p| rgb(p, [0, 0, 255])).len();
    assert_eq!(blues(json!({})), 0, "a non-printing layer is absent");
    assert_eq!(blues(json!({"includeNonPrinting": true})), 1);
    assert_eq!(blues(json!({"createLayers": true})), 1, "PDF layers keep every layer");
}

#[test]
fn marks_are_drawn_in_registration_on_every_plate_inside_the_media_box() {
    // Wide enough for the spot ink's colour bar patch after the process ones.
    let mut d = doc();
    let w = 300.0;
    d.artboards[0].rect = Rect::new(0.0, 0.0, w, 100.0);
    d.swatches.push(Swatch { name: "Gold".into(), paint: Paint::solid(Color::cmyk(0.0, 0.2, 0.8, 0.1)), global: true, spot: true });
    let all = json!({"marks": {"trim": true, "registration": true, "colorBars": true, "pageInfo": true}});
    let bytes = pdf(&d, all.clone());
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("/Separation/All"), "Registration is the /All separation");
    assert!(text.contains("/Separation/Gold"), "the spot ink gets a colour bar patch");
    // MediaBox fits the marks: [top, bottom, left, right] around the trim box.
    let [top, bottom, left, right] = settings(all).marks.printer_marks().reach([0.0; 4]);
    let (media, trim) = (page_box(&bytes, CropTo::Media), page_box(&bytes, CropTo::Trim));
    let near = |a: f64, b: f64| (a - b).abs() < 1e-3;
    assert!(near(media.width(), w + left + right) && near(media.height(), 100.0 + top + bottom), "{media:?}");
    assert!(near(trim.x0, left) && near(trim.y0, bottom), "{trim:?}");
    // Read back (y down): every Registration mark lies outside the trim box and inside the sheet.
    let back = import(&bytes).unwrap();
    let sheet = Rect::new(0.0, 0.0, media.width(), media.height()).inflate(0.5, 0.5);
    let trim_doc = Rect::new(left, top, left + w, top + 100.0).inflate(-0.5, -0.5);
    let marks = painted(&back, Paint::is_registration);
    // 8 trim marks, 4 targets (2 rings and 2 lines each) and the page information's glyphs.
    assert!(marks.len() > 8 + 16, "{}", marks.len());
    for b in &marks {
        assert!(sheet.contains_rect(*b) && b.intersect(trim_doc).area() <= 0.0, "{b:?}");
    }
    let gold = painted(&back, |p| matches!(p, Paint::Solid { swatch: Some(s), .. } if s == "Gold"));
    assert_eq!(gold.len(), 1);
    assert!(gold[0].y1 < trim_doc.y0, "colour bars sit above the trim box: {gold:?}");
}

#[test]
fn page_information_names_the_file_artboard_and_date() {
    let d = doc();
    assert_eq!(crate::marks::page_info(&d, "Poster", 0, Some(1_791_200_000)), "Poster  ·  Artboard 1 (1 of 1)  ·  2026-10-05 11:33 UTC");
    assert_eq!(crate::marks::page_info(&d, " ", 0, None), "Untitled  ·  Artboard 1 (1 of 1)");
    // Its glyphs are filled in Registration under the band of bottom marks.
    let bytes = pdf(&d, json!({"marks": {"pageInfo": true}}));
    let [top, ..] = settings(json!({"marks": {"pageInfo": true}})).marks.printer_marks().reach([0.0; 4]);
    let back = import(&bytes).unwrap();
    let glyphs = painted(&back, Paint::is_registration);
    assert!(!glyphs.is_empty());
    let band_end = top + 100.0 + 6.0 + PrinterMarks::LENGTH;
    assert!(glyphs.iter().all(|g| g.y0 > band_end && g.x0 >= top - 0.5), "{glyphs:?}");
}
