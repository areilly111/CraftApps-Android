//! `document.open` keeps a PDF's CMYK, Gray and spot colours, and `colorMode` picks the mode.

use serde_json::json;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::ColorMode;

use super::*;

/// A CMYK document with a CMYK fill, a Gray fill and a 40 % tint of spot ink "Gold", as PDF.
fn cmyk_pdf() -> String {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 50, "colorMode": "cmyk"})).unwrap();
    s.execute("swatch.new", &json!({"name": "Gold", "color": {"model": "cmyk", "c": 0.0, "m": 0.2, "y": 0.8, "k": 0.1}, "spot": true})).unwrap();
    for (x, fill) in [
        (0, json!({"color": {"model": "cmyk", "c": 0.6, "m": 0.1, "y": 0.0, "k": 0.2}})),
        (20, json!({"color": {"model": "gray", "k": 0.3}})),
        (40, json!({"swatch": "Gold", "tint": 40})),
    ] {
        s.execute("shape.rectangle", &json!({"x": x, "y": 0, "width": 10, "height": 10})).unwrap();
        s.execute("paint.setFill", &fill).unwrap();
    }
    let v = s.execute("document.serialize", &json!({"format": "pdf"})).unwrap();
    v["dataBase64"].as_str().unwrap().to_string()
}

fn fills(s: &Session) -> Vec<Paint> {
    let d = &s.active().unwrap().doc;
    d.layers[0].children().unwrap().iter().filter_map(|n| n.appearance.fill().map(|f| f.paint.clone())).collect()
}

#[test]
fn a_cmyk_pdf_opens_as_a_cmyk_document_with_its_spot_swatch() {
    let mut s = Session::new();
    s.execute("document.open", &json!({"name": "press.pdf", "dataBase64": cmyk_pdf()})).unwrap();
    assert_eq!(s.execute("file.info", &json!({})).unwrap()["colorMode"], "cmyk");
    let f = fills(&s);
    assert!(matches!(f[0].color(), Some(Color::Cmyk { c, .. }) if (c - 0.6).abs() < 0.004), "{:?}", f[0]);
    assert!(matches!(f[1].color(), Some(Color::Gray { k }) if (k - 0.3).abs() < 0.004), "{:?}", f[1]);
    assert!(matches!(&f[2], Paint::Solid { swatch: Some(n), tint, .. } if n == "Gold" && (tint - 0.4).abs() < 0.004), "{:?}", f[2]);
    let d = &s.active().unwrap().doc;
    let gold = d.swatch("Gold").unwrap();
    assert!(gold.spot && gold.global);
    assert!(matches!(gold.paint.color(), Some(Color::Cmyk { m, y, .. }) if (m - 0.2).abs() < 0.004 && (y - 0.8).abs() < 0.004));
}

#[test]
fn color_mode_opens_the_file_in_the_mode_asked_for() {
    let mut s = Session::new();
    s.execute("document.open", &json!({"name": "press.pdf", "dataBase64": cmyk_pdf(), "colorMode": "RGB"})).unwrap();
    assert_eq!(s.active().unwrap().doc.color_mode, ColorMode::Rgb);
    let f = fills(&s);
    // Converted as Document Color Mode does: no CMYK left, greys stay grey.
    assert!(f.iter().all(|p| matches!(p.color(), Some(Color::Rgb { .. } | Color::Gray { .. }))), "{f:?}");
    assert!(matches!(f[0].color(), Some(Color::Rgb { .. })) && matches!(f[1].color(), Some(Color::Gray { .. })), "{f:?}");
    // Any format opens in the mode asked for.
    let mut svg = Session::new();
    svg.execute("file.new", &json!({"width": 20, "height": 20})).unwrap();
    svg.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    let text = svg.execute("document.serialize", &json!({"format": "svg"})).unwrap()["text"].as_str().unwrap().to_string();
    s.execute("document.open", &json!({"name": "a.svg", "dataBase64": vectorcraft_format::base64_encode(text.as_bytes()), "colorMode": "cmyk"}))
        .unwrap();
    assert_eq!(s.active().unwrap().doc.color_mode, ColorMode::Cmyk);
    assert!(fills(&s).iter().all(|p| !matches!(p.color(), Some(Color::Rgb { .. }))));
    let e = s.execute("document.open", &json!({"name": "press.pdf", "dataBase64": cmyk_pdf(), "colorMode": "lab"})).unwrap_err().to_string();
    assert!(e.contains("rgb or cmyk"), "{e}");
}
