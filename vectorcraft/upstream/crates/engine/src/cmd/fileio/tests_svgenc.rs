//! SVG encodings, profiles and embedded fonts through the engine: the options reach the file,
//! files in each encoding open again, and `document.serialize` reads them as text.

use serde_json::json;

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
    s.execute("text.create", &json!({"x": 10, "y": 40, "text": "Grüße €"})).unwrap();
    s
}

fn serialize(s: &mut Session, svg: serde_json::Value) -> serde_json::Value {
    s.execute("document.serialize", &json!({"format": "svg", "svg": svg})).unwrap()
}

#[test]
fn each_encoding_reopens_and_serializes_as_text() {
    let mut s = session();
    for (encoding, name, bom) in [("utf8", "UTF-8", false), ("utf16", "UTF-16", true), ("latin1", "ISO-8859-1", false)] {
        let r = serialize(&mut s, json!({"encoding": encoding}));
        let text = r["text"].as_str().unwrap();
        assert!(text.contains("Grüße") && text.contains(&format!("encoding=\"{name}\"")), "{text}");
        let bytes = match r.get("dataBase64") {
            Some(b) => vectorcraft_format::base64_decode(b.as_str().unwrap()).unwrap(),
            None => text.as_bytes().to_vec(),
        };
        assert_eq!(bom, bytes.starts_with(&[0xfe, 0xff]), "{encoding}");
        assert_eq!(r.get("dataBase64").is_some(), encoding != "utf8", "{encoding}");
        for (file, data) in [("t.svg", bytes.clone()), ("t.svgz", vectorcraft_svg::compress_bytes(&bytes))] {
            let mut o = Session::new();
            o.execute("document.open", &json!({"name": file, "dataBase64": vectorcraft_format::base64_encode(&data)})).unwrap();
            let plain = o.execute("document.inspect", &json!({})).unwrap().to_string();
            assert!(plain.contains("Grüße €"), "{encoding} {file}: {plain}");
        }
    }
}

#[test]
fn profile_and_embedded_fonts_reach_the_writer() {
    let mut s = session();
    let tiny = serialize(&mut s, json!({"profile": "tiny12", "styling": "css"}));
    let tiny = tiny["text"].as_str().unwrap();
    assert!(tiny.contains("baseProfile=\"tiny\"") && !tiny.contains("<style>"), "{tiny}");
    let fonts = serialize(&mut s, json!({"embedFonts": true}));
    assert!(fonts["text"].as_str().unwrap().contains("@font-face{"));
    assert!(s.execute("document.serialize", &json!({"format": "svg", "svg": {"profile": "svg2"}})).is_err(), "unknown profiles are refused");
    assert!(s.execute("document.serialize", &json!({"format": "svg", "encoding": "ebcdic"})).is_err(), "unknown encodings are refused");
}

/// #550: an SVG of chosen artboards holds the art over each, not every artboard's art; with no
/// artboard named (as Save writes it) the whole document's art is there.
#[test]
fn a_chosen_artboard_s_svg_holds_only_its_own_art() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100, "artboards": 3})).unwrap();
    let x = |s: &Session, b: usize| s.doc().unwrap().doc.artboards[b].rect.x0;
    let (first, third) = (x(&s, 0), x(&s, 2));
    s.execute("shape.rectangle", &json!({"x": first + 10.0, "y": 10, "width": 30, "height": 30})).unwrap();
    s.execute("shape.ellipse", &json!({"x": third + 10.0, "y": 10, "width": 30, "height": 30})).unwrap();
    // A rectangle has straight sides only; the ellipse is curves.
    let shapes = |svg: &str| (svg.matches("<path").count() + svg.matches("<rect").count(), svg.contains(" C") || svg.contains("<ellipse"));
    let text = |v: &serde_json::Value| v["text"].as_str().unwrap().to_string();
    let third_only = text(&s.execute("document.serialize", &json!({"format": "svg", "useArtboards": true, "range": "3"})).unwrap());
    assert_eq!(shapes(&third_only), (1, true), "{third_only}");
    let all = s.execute("document.serialize", &json!({"format": "svg", "useArtboards": true, "range": "all"})).unwrap();
    let files: Vec<String> = all["files"].as_array().unwrap().iter().map(text).collect();
    assert_eq!(files.iter().map(|f| shapes(f)).collect::<Vec<_>>(), [(1, false), (0, false), (1, true)]);
    // Export for Screens names one artboard at a time the same way.
    let screen = text(&s.execute("document.serialize", &json!({"format": "svg", "artboard": 0})).unwrap());
    assert_eq!(shapes(&screen), (1, false));
    let whole = text(&s.execute("document.serialize", &json!({"format": "svg"})).unwrap());
    assert_eq!(shapes(&whole).0, 2, "nothing named: the whole document's art");
}
