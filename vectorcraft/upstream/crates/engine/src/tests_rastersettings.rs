//! Effect → Document Raster Effects Settings (`document.rasterEffectsSettings`): the settings, the
//! raster effect images they make (PDF export, Expand Appearance) and the Rasterize defaults.

use serde_json::{Value, json};
use vectorcraft_doc::{Background, Document, NodeId, NodeKind, RasterColorModel, RasterEffectsSettings};

use super::*;

const C: &str = "document.rasterEffectsSettings";

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 300})).unwrap();
    s
}

fn id(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

/// A red rectangle with a drop shadow.
fn shadowed(s: &mut Session) -> NodeId {
    let r = id(&s.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap());
    s.execute("paint.setFill", &json!({"color": "#ff0000", "ids": [r.0]})).unwrap();
    s.execute("effect.apply", &json!({"effect": "stylize.dropShadow", "ids": [r.0]})).unwrap();
    r
}

/// The decoded pixels of the first raster effect image of `doc` flattened for output.
fn effect_image(doc: &Document) -> image::RgbaImage {
    let flat = crate::flatten_raster_effects(doc).unwrap();
    let kids = flat.layers[0].children().unwrap();
    let NodeKind::Group { children, .. } = &kids[0].kind else { panic!("a shadow becomes a group") };
    let NodeKind::Image(im) = &children[0].kind else { panic!("the image goes under the object") };
    image::load_from_memory(&flat.images[&im.key].bytes).unwrap().to_rgba8()
}

#[test]
fn settings_report_change_in_one_step_and_reject_junk() {
    let mut s = session();
    let r = s.execute(C, &json!({})).unwrap();
    assert_eq!(
        r,
        json!({"resolution": 72.0, "colorModel": "rgb", "background": "transparent", "antiAlias": true, "clippingMask": false, "addAround": 0.0, "preserveSpotColors": true})
    );
    let r = s
        .execute(C, &json!({"resolution": "high", "colorModel": "grayscale", "background": "white", "antiAlias": false, "clippingMask": true, "addAround": 36}))
        .unwrap();
    assert_eq!((r["resolution"].as_f64(), r["colorModel"].as_str(), r["addAround"].as_f64()), (Some(300.0), Some("grayscale"), Some(36.0)));
    assert_eq!(s.doc().unwrap().history.undo.len(), 1);
    // The report fed back is no change.
    s.execute(C, &r).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), 1);
    for bad in [
        json!({"colorModel": "cmyk"}),
        json!({"colorModel": "lab"}),
        json!({"background": "pink"}),
        json!({"antiAlias": "yes"}),
        json!({"addAround": -1}),
        json!({"addAround": 5000}),
        json!({"resolution": 0}),
        json!({"dpi": 300}),
    ] {
        assert!(s.execute(C, &bad).is_err(), "{bad}");
    }
    s.execute("edit.undo", &json!({})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert_eq!((d.raster_effects_ppi, &d.raster_effects), (72.0, &RasterEffectsSettings::default()));
    // A CMYK document names its own mode.
    let mut s = Session::new();
    s.execute("file.new", &json!({"colorMode": "cmyk"})).unwrap();
    assert_eq!(s.execute(C, &json!({"colorModel": "cmyk"})).unwrap()["colorModel"], "cmyk");
}

#[test]
fn a_white_background_makes_effect_images_opaque() {
    let mut s = session();
    shadowed(&mut s);
    let clear = effect_image(&s.doc().unwrap().doc);
    assert_eq!(clear.get_pixel(0, 0)[3], 0, "transparent by default");
    s.execute(C, &json!({"background": "white"})).unwrap();
    let white = effect_image(&s.doc().unwrap().doc);
    assert!(white.pixels().all(|p| p[3] == 255), "every pixel is opaque");
    assert_eq!(white.get_pixel(0, 0).0, [255, 255, 255, 255]);
    // With a clipping mask, the white stays under the art only.
    s.execute(C, &json!({"clippingMask": true})).unwrap();
    assert_eq!(effect_image(&s.doc().unwrap().doc).get_pixel(0, 0)[3], 0);
}

#[test]
fn add_around_grows_the_effect_image() {
    let mut s = session();
    shadowed(&mut s);
    let before = effect_image(&s.doc().unwrap().doc);
    s.execute(C, &json!({"addAround": 36})).unwrap();
    let after = effect_image(&s.doc().unwrap().doc);
    // 36 pt on every side at 72 ppi.
    assert_eq!((after.width() - before.width(), after.height() - before.height()), (72, 72));
}

#[test]
fn the_color_model_and_anti_alias_reach_the_pixels() {
    let mut s = session();
    shadowed(&mut s);
    s.execute(C, &json!({"colorModel": "bitmap", "antiAlias": false})).unwrap();
    let img = effect_image(&s.doc().unwrap().doc);
    assert!(img.pixels().filter(|p| p[3] > 0).all(|p| p[0] == p[1] && p[1] == p[2] && (p[0] == 0 || p[0] == 255)), "black and white");
    // Anti-alias off: Rasterize leaves no partly covered pixel on a circle's edge.
    let e = id(&s.execute("shape.ellipse", &json!({"x": 180, "y": 180, "width": 50, "height": 50})).unwrap());
    s.execute("select.set", &json!({"ids": [e.0]})).unwrap();
    let out = s.execute("object.rasterize", &json!({"colorModel": "rgb"})).unwrap();
    let d = &s.doc().unwrap().doc;
    let NodeKind::Image(im) = &d.node(id(&out)).unwrap().kind else { panic!("an image") };
    let img = image::load_from_memory(&d.images[&im.key].bytes).unwrap().to_rgba8();
    assert!(img.pixels().all(|p| p[3] == 0 || p[3] == 255), "hard edges");
    assert!(img.pixels().any(|p| p[3] == 0) && img.pixels().any(|p| p[3] == 255));
}

#[test]
fn rasterize_takes_the_document_settings_as_defaults() {
    let mut s = session();
    s.execute(C, &json!({"background": "white", "colorModel": "grayscale", "addAround": 10})).unwrap();
    let r = id(&s.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 40, "height": 40})).unwrap());
    s.execute("paint.setFill", &json!({"color": "#ff0000", "ids": [r.0]})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true, "ids": [r.0]})).unwrap();
    s.execute("select.set", &json!({"ids": [r.0]})).unwrap();
    let out = s.execute("object.rasterize", &json!({"ppi": 72})).unwrap();
    assert_eq!((out["width"].as_u64(), out["height"].as_u64()), (Some(60), Some(60)), "10 pt around");
    let d = &s.doc().unwrap().doc;
    let NodeKind::Image(im) = &d.node(id(&out)).unwrap().kind else { panic!("an image") };
    let img = image::load_from_memory(&d.images[&im.key].bytes).unwrap().to_rgba8();
    assert_eq!(img.get_pixel(0, 0).0, [255, 255, 255, 255], "white around the art");
    let p = img.get_pixel(30, 30).0;
    assert!(p[0] == p[1] && p[1] == p[2] && p[0] < 128, "red in grayscale: {p:?}");
    // Params win; a clipping mask makes a clip group with the art's outline.
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": [r.0]})).unwrap();
    let out = s.execute("object.rasterize", &json!({"ppi": 72, "padding": 0, "clippingMask": true, "background": "transparent"})).unwrap();
    let d = &s.doc().unwrap().doc;
    let NodeKind::Group { children, clip: true } = &d.node(id(&out)).unwrap().kind else { panic!("a clip group") };
    let clip = children[0].geometric_bounds().unwrap();
    assert!((clip.width() - 40.0).abs() < 1e-6 && matches!(children[1].kind, NodeKind::Image(_)));
    assert_eq!(s.doc().unwrap().selection.objects.to_vec(), [id(&out)]);
    assert!(s.execute("object.rasterize", &json!({"colorModel": "lab"})).is_err());
}

#[test]
fn raster_settings_round_trip_and_old_files_load() {
    let mut d = Document::new(10.0, 10.0);
    d.raster_effects =
        RasterEffectsSettings { color_model: RasterColorModel::Bitmap, background: Background::White, add_around: 12.0, ..Default::default() };
    let back = vectorcraft_format::load(&vectorcraft_format::save_file(&d)).unwrap();
    assert_eq!(back.raster_effects, d.raster_effects);
    // Saved before the settings existed: only the resolution.
    let old = Document::new(10.0, 10.0);
    let bytes = vectorcraft_format::save_file(&old);
    assert!(!String::from_utf8_lossy(&bytes).contains("raster_effects\""), "default settings aren't written");
    let back = vectorcraft_format::load(&bytes).unwrap();
    assert_eq!((back.raster_effects_ppi, back.raster_effects), (72.0, RasterEffectsSettings::default()));
}
