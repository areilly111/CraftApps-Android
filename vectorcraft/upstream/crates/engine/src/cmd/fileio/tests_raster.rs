//! Raster export options: resolution (stored in the file), background, anti-aliasing and
//! interlacing, also on Export for Screens and Rasterize.

use serde_json::{Value, json};

use super::*;

fn session(width: f64, height: f64, artboards: usize) -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": width, "height": height, "artboards": artboards})).unwrap();
    s
}

fn b64(v: &Value) -> Vec<u8> {
    vectorcraft_format::base64_decode(v["dataBase64"].as_str().expect("dataBase64")).unwrap()
}

fn rgba(bytes: &[u8]) -> image::RgbaImage {
    image::load_from_memory(bytes).unwrap().to_rgba8()
}

/// A chunk's data by type (first match) in a PNG file.
fn png_chunk<'a>(file: &'a [u8], ty: &[u8; 4]) -> Option<&'a [u8]> {
    let mut i = 8;
    while i + 8 <= file.len() {
        let len = u32::from_be_bytes(file[i..i + 4].try_into().unwrap()) as usize;
        if &file[i + 4..i + 8] == ty {
            return Some(&file[i + 8..i + 8 + len]);
        }
        i += 12 + len;
    }
    None
}

#[test]
fn png_options_resolution_background_interlace_and_anti_aliasing() {
    let mut s = session(40.0, 30.0, 1);
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    s.execute("shape.ellipse", &json!({"x": 5.3, "y": 4.7, "width": 25.8, "height": 21.5})).unwrap();
    let png = |s: &mut Session, p: Value| b64(&s.execute("document.export", &p).unwrap());

    let hi = png(&mut s, json!({"format": "png", "ppi": 300}));
    let ppm = (300.0f64 / 0.0254).round() as u32;
    assert_eq!(&png_chunk(&hi, b"pHYs").unwrap()[..4], ppm.to_be_bytes(), "pHYs = ppi / 0.0254");
    assert_eq!(rgba(&png(&mut s, json!({"format": "png", "ppi": 144}))).dimensions(), (80, 60), "144 ppi doubles the size");
    // scale alone stores 72 × scale.
    let scaled = png(&mut s, json!({"format": "png", "scale": 2}));
    assert_eq!(&png_chunk(&scaled, b"pHYs").unwrap()[..4], ((144.0f64 / 0.0254).round() as u32).to_be_bytes());

    let white = rgba(&png(&mut s, json!({"format": "png", "background": "white"})));
    assert!(white.pixels().all(|p| p[3] == 255), "a white background has no alpha below 255");
    assert_eq!(white.get_pixel(0, 0).0, [255, 255, 255, 255]);
    assert_eq!(rgba(&png(&mut s, json!({"format": "png", "background": "black"}))).get_pixel(0, 0).0, [0, 0, 0, 255]);
    assert_eq!(rgba(&png(&mut s, json!({"format": "png", "background": "#3366cc"}))).get_pixel(0, 0).0, [0x33, 0x66, 0xcc, 255]);
    assert_eq!(rgba(&png(&mut s, json!({"format": "png"}))).get_pixel(0, 0)[3], 0, "transparent by default");
    assert_eq!(rgba(&png(&mut s, json!({"format": "jpg", "background": "black"}))).get_pixel(0, 0)[0], 0);

    let laced = png(&mut s, json!({"format": "png", "interlaced": true}));
    assert_eq!(laced[28], 1, "IHDR interlace method: Adam7");
    assert_eq!(rgba(&laced), rgba(&png(&mut s, json!({"format": "png"}))), "the interlaced file decodes to the same image");

    let hard = rgba(&png(&mut s, json!({"format": "png", "antiAlias": "none"})));
    assert!(hard.pixels().all(|p| p[3] == 0 || p[3] == 255), "no anti-aliasing: alpha is 0 or 255");
    let soft = rgba(&png(&mut s, json!({"format": "png", "antiAlias": "art"})));
    assert!(soft.pixels().any(|p| p[3] != 0 && p[3] != 255));
    for bad in [
        json!({"antiAlias": "smooth"}),
        json!({"background": "plaid"}),
        json!({"interlaced": "yes"}),
        json!({"ppi": "high"}),
        json!({"ppi": 0}),
        json!({"ppi": -72}),
    ] {
        let mut p = bad.clone();
        p["format"] = json!("png");
        assert!(s.execute("document.export", &p).is_err(), "{bad}");
    }
}

#[test]
fn screens_and_rasterize_take_anti_aliasing() {
    let mut s = session(40.0, 30.0, 1);
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    let id = s.execute("shape.ellipse", &json!({"x": 5.3, "y": 4.7, "width": 25.8, "height": 21.5})).unwrap()["id"].as_u64().unwrap();
    let hard = |img: &image::RgbaImage| img.pixels().all(|p| p[3] == 0 || p[3] == 255);
    let r = s
        .execute(
            "document.exportForScreens",
            &json!({"antiAlias": "none", "formats": [{"format": "png"}, {"format": "png", "scale": 2, "antiAlias": "art"}]}),
        )
        .unwrap();
    assert!(hard(&rgba(&b64(&r["files"][0]))), "the top-level mode applies to rows without their own");
    assert!(!hard(&rgba(&b64(&r["files"][1]))), "a row's own mode wins");

    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    s.execute("object.rasterize", &json!({"ppi": 72, "antiAlias": "none", "background": "transparent"})).unwrap();
    let doc = &s.doc().unwrap().doc;
    let vectorcraft_doc::NodeKind::Image(im) = &doc.layers[0].children().unwrap()[0].kind else { panic!("rasterized to an image") };
    assert!(hard(&rgba(&doc.images[&im.key].bytes)));
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    s.execute("object.rasterize", &json!({"ppi": 72, "background": "black"})).unwrap();
    let doc = &s.doc().unwrap().doc;
    let vectorcraft_doc::NodeKind::Image(im) = &doc.layers[0].children().unwrap()[0].kind else { panic!("rasterized to an image") };
    assert_eq!(rgba(&doc.images[&im.key].bytes).get_pixel(0, 0).0, [0, 0, 0, 255]);
    assert!(s.execute("object.rasterize", &json!({"antiAlias": "fuzzy"})).is_err());
}

#[test]
fn formats_list_the_raster_options() {
    let mut s = Session::new();
    let r = s.execute("document.formats", &json!({})).unwrap();
    let opts = |id: &str| r["formats"].as_array().unwrap().iter().find(|f| f["id"] == id).unwrap()["options"].clone();
    for k in ["ppi", "scale", "background", "antiAlias", "interlaced"] {
        assert!(opts("png").get(k).is_some(), "png lists {k}");
    }
    assert_eq!(opts("png")["antiAlias"]["default"], "art");
    assert_eq!(opts("png")["ppi"]["default"], 72);
    assert!(opts("jpg").get("quality").is_some() && opts("jpg").get("interlaced").is_none());
    assert!(opts("webp").get("antiAlias").is_some() && opts("svg").get("antiAlias").is_none());
}

#[test]
fn screens_rows_with_a_resolution_name_their_scale() {
    let mut s = session(40.0, 30.0, 1);
    let r = s.execute("document.exportForScreens", &json!({"formats": [{"format": "png", "ppi": 144}, {"format": "svg", "ppi": 144}]})).unwrap();
    assert_eq!((r["files"][0]["name"].as_str(), r["files"][1]["name"].as_str()), (Some("Artboard-1@2x.png"), Some("Artboard-1.svg")), "{r}");
    assert_eq!(rgba(&b64(&r["files"][0])).dimensions(), (80, 60));
}

#[test]
fn a_300_ppi_export_places_back_at_its_physical_size() {
    let mut s = session(72.0, 36.0, 1);
    let png = b64(&s.execute("document.export", &json!({"format": "png", "ppi": 300, "background": "white"})).unwrap());
    assert_eq!(rgba(&png).dimensions(), (300, 150));
    assert_eq!(super::ppi::resolution(&png), Some((300.0, 300.0)), "the place reader reads the writer's pHYs");
    let r = s.execute("file.place", &json!({"name": "art.png", "dataBase64": vectorcraft_format::base64_encode(&png)})).unwrap();
    assert!((r["width"].as_f64().unwrap() - 72.0).abs() < 1e-6 && (r["height"].as_f64().unwrap() - 36.0).abs() < 1e-6, "100%: {r}");
}

#[test]
fn the_document_background_is_the_default_export_background() {
    let mut s = session(40.0, 30.0, 1);
    s.execute("document.setup", &json!({"backgroundContents": "white"})).unwrap();
    let png = |s: &mut Session, p: Value| rgba(&b64(&s.execute("document.export", &p).unwrap()));
    assert_eq!(png(&mut s, json!({"format": "png"})).get_pixel(0, 0).0, [255, 255, 255, 255], "a white document exports opaque");
    assert_eq!(png(&mut s, json!({"format": "png", "background": "transparent"})).get_pixel(0, 0)[3], 0, "the option wins");
    assert_eq!(png(&mut s, json!({"format": "webp", "background": "black"})).get_pixel(0, 0).0, [0, 0, 0, 255]);
}

#[test]
fn an_image_opens_at_the_size_its_resolution_declares() {
    let mut s = session(72.0, 36.0, 1);
    let hi = b64(&s.execute("document.export", &json!({"format": "png", "ppi": 300, "background": "white"})).unwrap());
    let lo = b64(&s.execute("document.export", &json!({"format": "png", "background": "white"})).unwrap());
    let lo = super::ppi::with_png_resolution(&lo, (72.0, 72.0));
    for (png, w, h, what) in [(hi, 72.0, 36.0, "300 ppi: 300 x 150 px is 1 x 0.5 in"), (lo, 72.0, 36.0, "72 ppi: 1 px = 1 pt")] {
        s.execute("document.open", &json!({"name": "art.png", "dataBase64": vectorcraft_format::base64_encode(&png)})).unwrap();
        let doc = &s.doc().unwrap().doc;
        let ab = doc.artboards[0].rect;
        assert!((ab.width() - w).abs() < 1e-6 && (ab.height() - h).abs() < 1e-6, "{what}: artboard {ab:?}");
        let art = doc.art_bounds().expect("the image");
        assert!((art.width() - w).abs() < 1e-6 && (art.height() - h).abs() < 1e-6, "{what}: the image fills it, {art:?}");
    }
}
