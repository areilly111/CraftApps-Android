//! JPEG Options: colour model, coding method, scans, profile and image maps.

use serde_json::{Value, json};

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 60, "height": 40})).unwrap();
    s
}

fn b64(v: &Value) -> Vec<u8> {
    vectorcraft_format::base64_decode(v["dataBase64"].as_str().expect("dataBase64")).unwrap()
}

/// The JPEG's SOF marker and the number of components in its frame header.
fn frame(file: &[u8]) -> (u8, u8) {
    let mut i = 2;
    while !(0xC0..=0xC2).contains(&file[i + 1]) {
        i += 2 + u16::from_be_bytes([file[i + 2], file[i + 3]]) as usize;
    }
    (file[i + 1], file[i + 9])
}

#[test]
fn jpeg_options_reach_the_file() {
    let mut s = session();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
    let jpg = |s: &mut Session, p: Value| b64(&s.execute("document.export", &merge(json!({"format": "jpg"}), p)).unwrap());
    assert_eq!(frame(&jpg(&mut s, json!({}))), (0xC0, 3), "baseline RGB by default");
    assert_eq!(frame(&jpg(&mut s, json!({"colorModel": "cmyk"}))), (0xC0, 4));
    assert_eq!(frame(&jpg(&mut s, json!({"colorModel": "grayscale"}))), (0xC0, 1));
    assert_eq!(frame(&jpg(&mut s, json!({"method": "progressive", "scans": 5}))).0, 0xC2);
    let icc = |f: &[u8]| f.windows(12).any(|w| w == b"ICC_PROFILE\0");
    assert!(icc(&jpg(&mut s, json!({}))) && !icc(&jpg(&mut s, json!({"embedIcc": false}))));
    assert!(jpg(&mut s, json!({"quality": 0})).len() < jpg(&mut s, json!({"quality": 100})).len());
    for bad in [json!({"colorModel": "lab"}), json!({"method": "fast"}), json!({"imageMap": "both"})] {
        assert!(s.execute("document.export", &merge(json!({"format": "jpg"}), bad.clone())).is_err(), "{bad}");
    }
    let r = s.execute("document.formats", &json!({})).unwrap();
    let jpg = r["formats"].as_array().unwrap().iter().find(|f| f["id"] == "jpg").unwrap();
    for k in ["colorModel", "method", "scans", "embedIcc", "imageMap"] {
        assert!(jpg["options"].get(k).is_some(), "{k}");
    }
}

#[test]
fn image_maps_list_the_objects_with_a_url() {
    let mut s = session();
    // Areas cover the visual bounds: no stroke, so they are the shapes' own.
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 5, "width": 20, "height": 10})).unwrap();
    s.execute("attributes.set", &json!({"url": "https://example.com/a?b=1&c=2", "imageMap": "rectangle"})).unwrap();
    s.execute("shape.ellipse", &json!({"x": 35, "y": 20, "width": 20, "height": 10})).unwrap();
    s.execute("attributes.set", &json!({"url": "https://example.com/round", "imageMap": "polygon"})).unwrap();
    // A URL without a map shape makes no area.
    s.execute("shape.rectangle", &json!({"x": 0, "y": 30, "width": 5, "height": 5})).unwrap();
    s.execute("attributes.set", &json!({"url": "https://example.com/none"})).unwrap();

    let r = s.execute("document.export", &json!({"format": "jpg", "imageMap": "client", "ppi": 144})).unwrap();
    let linked = r["linked"].as_array().unwrap();
    assert_eq!(linked.len(), 1);
    assert_eq!(linked[0]["name"], "Untitled-1.html");
    let html = String::from_utf8(b64(&linked[0])).unwrap();
    assert!(html.contains("<img src=\"Untitled-1.jpg\" width=\"120\" height=\"80\" usemap=\"#Untitled-1\""), "{html}");
    assert!(html.contains("<area shape=\"rect\" coords=\"20,10,60,30\" href=\"https://example.com/a?b=1&amp;c=2\""), "{html}");
    let poly = html.lines().find(|l| l.contains("shape=\"poly\"")).expect("a polygon area");
    assert!(poly.contains("coords=\"70,50,") && !poly.contains(",70,50\""), "the ellipse's outline, not closed twice: {poly}");
    assert!(html.find("example.com/round") < html.find("example.com/a?"), "topmost first: {html}");
    assert!(!html.contains("example.com/none"));

    let r = s.execute("document.export", &json!({"format": "jpg", "imageMap": "server"})).unwrap();
    let map = String::from_utf8(b64(&r["linked"][0])).unwrap();
    assert_eq!(r["linked"][0]["name"], "Untitled-1.map");
    assert!(map.lines().any(|l| l == "rect https://example.com/a?b=1&c=2 10,5 30,15"), "{map}");
    assert!(map.lines().any(|l| l.starts_with("poly https://example.com/round ")), "{map}");

    // Written next to the image, named after it.
    let dir = std::env::temp_dir().join(format!("vc-imagemap-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("pic.jpeg");
    let r = s.execute("document.export", &json!({"path": path.to_string_lossy(), "imageMap": "client"})).unwrap();
    assert_eq!(r["linked"][0], dir.join("pic.html").to_string_lossy().as_ref());
    assert!(std::fs::read_to_string(dir.join("pic.html")).unwrap().contains("src=\"pic.jpeg\""));
    let _ = std::fs::remove_dir_all(dir);
    // One file only: Export for Screens can't add the map.
    assert!(s.execute("document.exportForScreens", &json!({"formats": [{"format": "jpg", "imageMap": "client"}]})).is_err());
}
