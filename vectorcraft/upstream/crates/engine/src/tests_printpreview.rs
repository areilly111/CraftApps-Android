//! `print.preview`'s pages say where the art lands (what the Print dialog's preview draws): the
//! transform from the document onto the page, the region each page prints and its trim box.

use serde_json::{Value, json};

use super::*;

/// `print.preview {settings}`'s first page.
fn first_sheet(s: &mut Session, settings: Value) -> Value {
    s.execute("print.preview", &json!({ "settings": settings })).unwrap()["sheets"][0].clone()
}

/// `[a, b, c, d, e, f]` applied to (x, y).
fn apply(t: &Value, (x, y): (f64, f64)) -> (f64, f64) {
    let n = |i: usize| t[i].as_f64().unwrap();
    (n(0) * x + n(2) * y + n(4), n(1) * x + n(3) * y + n(5))
}

fn close(a: (f64, f64), b: (f64, f64)) -> bool {
    (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6
}

#[test]
fn pages_place_the_artboard_on_the_trim_box() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
    s.execute("document.setup", &json!({"bleed": 9})).unwrap();
    // Centred on Letter, turned landscape for the wide artboard.
    let p = first_sheet(&mut s, json!({}));
    assert_eq!((p["width"].clone(), p["height"].clone()), (json!(792.0), json!(612.0)));
    assert_eq!(p["trim"], json!([246.0, 206.0, 546.0, 406.0]));
    assert!(close(apply(&p["transform"], (0.0, 0.0)), (246.0, 206.0)) && close(apply(&p["transform"], (300.0, 200.0)), (546.0, 406.0)));
    assert_eq!(p["area"], json!([-9.0, -9.0, 309.0, 209.0]), "the artboard with the document's bleed");
    // Flipped, the artboard's top-left corner lands on the trim box's bottom-right one.
    let p = first_sheet(
        &mut s,
        json!({"autoRotate": false, "orientation": "landscapeFlipped", "scaling": "custom", "scale": {"width": 50, "height": 50}}),
    );
    let t = &p["trim"];
    assert!(close(apply(&p["transform"], (0.0, 0.0)), (t[2].as_f64().unwrap(), t[3].as_f64().unwrap())), "{p}");
    assert_eq!(p["scale"], json!([50.0, 50.0]));
    // A tile prints its part of the artboard.
    let v = s.execute("print.preview", &json!({"settings": {"scaling": "tileFull", "scale": {"width": 400, "height": 400}}})).unwrap();
    let tiles = v["tiles"][0]["tiles"].as_array().unwrap();
    for (sheet, tile) in v["sheets"].as_array().unwrap().iter().zip(tiles) {
        let (a, t) = (&sheet["area"], tile);
        for i in 0..4 {
            let inside =
                if i < 2 { a[i].as_f64().unwrap() >= t[i].as_f64().unwrap() - 1e-6 } else { a[i].as_f64().unwrap() <= t[i].as_f64().unwrap() + 1e-6 };
            assert!(inside, "{a} within {t}");
        }
    }
}
