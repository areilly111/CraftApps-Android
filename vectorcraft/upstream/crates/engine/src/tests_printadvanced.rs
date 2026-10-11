//! File → Print › Advanced: simulated overprints print what Overprint Preview shows, Print as
//! Bitmap prints images of the art, and a flattener preset flattens transparency first.

use serde_json::{Value, json};
use vectorcraft_doc::{Document, NodeKind};
use vectorcraft_geom::Rect;
use vectorcraft_pdf::PrintSettings;
use vectorcraft_render::{RenderOptions, Renderer};

use super::*;

/// A 200 × 100 CMYK document: a cyan square under a yellow one whose fill overprints, and a
/// half-transparent magenta one; their ids.
fn session() -> (Session, [u64; 3]) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 100, "colorMode": "cmyk"})).unwrap();
    let mut ids = [0; 3];
    for (i, (x, color)) in [(10, "#00ffff"), (40, "#ffff00"), (120, "#ff00ff")].into_iter().enumerate() {
        let r = s.execute("shape.rectangle", &json!({"x": x, "y": 10, "width": 50, "height": 50})).unwrap();
        ids[i] = r["id"].as_u64().unwrap();
        s.execute("paint.setFill", &json!({"ids": [ids[i]], "color": color})).unwrap();
        s.execute("paint.setStroke", &json!({"ids": [ids[i]], "none": true})).unwrap();
    }
    s.execute("object.setOverprint", &json!({"ids": [ids[1]], "fill": true})).unwrap();
    s.execute("transparency.set", &json!({"ids": [ids[2]], "opacity": 50, "item": null})).unwrap();
    (s, ids)
}

fn settings(v: Value) -> PrintSettings {
    serde_json::from_value(v).unwrap()
}

fn doc(s: &Session) -> &Document {
    &s.doc().unwrap().doc
}

/// `doc` rendered over its artboard at 2 px/pt on white.
fn pixels(doc: &Document, opts: RenderOptions) -> Vec<u8> {
    let opts = RenderOptions { background: Some([255; 4]), skip_templates: true, ..opts };
    Renderer::new().render_region_with(doc, Rect::new(0.0, 0.0, 200.0, 100.0), 2.0, &opts).pixels
}

/// The pixel of the page `pdf` prints at document point `(x, y)` of the first sheet.
fn printed_pixel(s: &mut Session, settings: Value, (x, y): (f64, f64)) -> [u8; 4] {
    let v = s.execute("file.print", &json!({ "settings": settings })).unwrap();
    let bytes = vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap();
    let pv = s.execute("print.preview", &json!({ "settings": settings })).unwrap();
    let t: Vec<f64> = pv["sheets"][0]["transform"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
    let back = vectorcraft_pdf::import(&bytes).unwrap();
    let page = back.artboards[0].rect;
    let (px, py) = (page.x0 + t[0] * x + t[2] * y + t[4], page.y0 + t[1] * x + t[3] * y + t[5]);
    let img = Renderer::new().render_region(&back, Rect::new(px - 0.5, py - 0.5, px + 0.5, py + 0.5), 1.0, true);
    img.pixels[..4].try_into().unwrap()
}

fn near(a: [u8; 4], b: [u8; 4]) -> bool {
    a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 4)
}

#[test]
fn simulated_overprints_print_what_overprint_preview_shows() {
    let (mut s, _) = session();
    let preview = pixels(doc(&s), RenderOptions { overprint_preview: true, ..Default::default() });
    let simulated = vectorcraft_pdf::printed_document(doc(&s), &settings(json!({"advanced": {"overprints": "simulate"}})));
    assert_eq!(pixels(&simulated, RenderOptions::default()), preview, "the same pixels as Overprint Preview");
    assert_ne!(pixels(doc(&s), RenderOptions::default()), preview, "overprinting shows");
    // On the printed page too: where yellow overprints cyan it prints green; discarded, yellow.
    let at = (55.0, 30.0);
    let i = ((30.0 * 2.0) as usize * 400 + (55.0 * 2.0) as usize) * 4;
    let shown: [u8; 4] = preview[i..i + 4].try_into().unwrap();
    let sim = printed_pixel(&mut s, json!({"advanced": {"overprints": "simulate"}}), at);
    assert!(near(sim, shown), "{sim:?} vs {shown:?}");
    let discarded = printed_pixel(&mut s, json!({"advanced": {"overprints": "discard"}}), at);
    assert!(!near(discarded, shown) && discarded[2] < 64 && discarded[0] > 192, "yellow knocks out: {discarded:?}");
}

#[test]
fn a_flattener_preset_flattens_transparency_before_printing() {
    let (mut s, _) = session();
    let saved = s.prefs.flattener_presets.clone();
    let transparent = |d: &Document| {
        let mut any = false;
        d.walk(|n| any |= n.opacity < 1.0);
        any
    };
    assert!(transparent(doc(&s)));
    let none = super::cmd::printadvanced::prepare("file.print", doc(&s), &settings(json!({})), &saved).unwrap();
    assert!(none.is_none(), "transparency prints live without a preset");
    for name in ["High Resolution", "low"] {
        let set = settings(json!({"advanced": {"flattenerPreset": name}}));
        let flat = super::cmd::printadvanced::prepare("file.print", doc(&s), &set, &saved).unwrap().unwrap();
        assert!(!transparent(&flat), "{name}: flattened");
    }
    // A saved preset by name; an unknown one is refused by preview and print alike.
    s.execute("flattener.presets.save", &json!({"name": "Proofs", "preset": "low"})).unwrap();
    let ok = json!({"settings": {"advanced": {"flattenerPreset": "Proofs"}}});
    assert!(s.execute("print.preview", &ok).is_ok() && s.execute("file.print", &ok).is_ok());
    let bad = json!({"settings": {"advanced": {"flattenerPreset": "Nope"}}});
    assert!(s.execute("print.preview", &bad).is_err() && s.execute("file.print", &bad).is_err());
}

#[test]
fn print_as_bitmap_prints_an_image_of_each_artboard() {
    let (mut s, _) = session();
    s.execute("artboard.new", &json!({"x": 300, "y": 0, "width": 200, "height": 100})).unwrap();
    let saved = s.prefs.flattener_presets.clone();
    let set = settings(json!({"advanced": {"printAsBitmap": true}}));
    let bitmap = super::cmd::printadvanced::prepare("file.print", doc(&s), &set, &saved).unwrap().unwrap();
    let mut kinds = vec![];
    bitmap.walk(|n| kinds.push(matches!(n.kind, NodeKind::Image(_))));
    assert_eq!(kinds.iter().filter(|k| **k).count(), 1, "one image: the empty artboard gets none");
    // The image looks like the art, at the raster effects resolution.
    assert_eq!(pixels(&bitmap, RenderOptions::default()).len(), pixels(doc(&s), RenderOptions::default()).len());
    let v = s.execute("file.print", &json!({"settings": {"advanced": {"printAsBitmap": true}, "skipBlank": true}})).unwrap();
    assert_eq!(v["pages"], 1, "the blank artboard is skipped");
    let bytes = vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap();
    let back = vectorcraft_pdf::import(&bytes).unwrap();
    let (mut images, mut paths) = (0, 0);
    back.walk(|n| match n.kind {
        NodeKind::Image(_) => images += 1,
        NodeKind::Path { .. } => paths += 1,
        _ => {}
    });
    assert_eq!((images, paths), (1, 0), "no vector art is left");
    // Separations print the art as it is.
    let seps = json!({"settings": {"advanced": {"printAsBitmap": true}, "output": {"mode": "separations"}}});
    let pv = s.execute("print.preview", &seps).unwrap();
    assert!(pv["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("Print as Bitmap")));
    let set = settings(seps["settings"].clone());
    assert!(super::cmd::printadvanced::prepare("file.print", doc(&s), &set, &saved).unwrap().is_none());
}
