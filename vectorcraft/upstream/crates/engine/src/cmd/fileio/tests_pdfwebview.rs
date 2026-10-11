//! `document.exportPdf {thumbnails, fastWebView}`: each page's thumbnail is the page drawn small
//! (its bleed and marks too, without the layers the page leaves out), and the file is linearised.

use std::io::Read;

use serde_json::{Value, json};

use super::*;

fn export(s: &mut Session, p: Value) -> (Vec<u8>, Vec<Value>) {
    let v = s.execute("document.exportPdf", &p).unwrap_or_else(|e| panic!("{p}: {e}"));
    let bytes = vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap();
    (bytes, v["warnings"].as_array().unwrap().clone())
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    hay.get(from..)?.windows(needle.len()).position(|w| w == needle).map(|i| i + from)
}

/// The page thumbnails of `pdf`, in file order: (width, height, RGB pixels).
fn thumbnails(pdf: &[u8]) -> Vec<(u32, u32, Vec<u8>)> {
    let mut out = vec![];
    let mut from = 0;
    while let Some(at) = find(pdf, b"<</Width ", from) {
        from = at + 1;
        let head = String::from_utf8_lossy(&pdf[at..find(pdf, b">>", at).unwrap()]).into_owned();
        if !head.contains("/ColorSpace/DeviceRGB/BitsPerComponent 8/Filter/FlateDecode/Length ") {
            continue;
        }
        let num = |key: &str| -> usize {
            let rest = &head[head.find(key).unwrap() + key.len()..];
            rest[..rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len())].parse().unwrap()
        };
        let data = find(pdf, b"stream\n", at).unwrap() + 7;
        let mut rgb = vec![];
        flate2::read::ZlibDecoder::new(&pdf[data..data + num("/Length ")]).read_to_end(&mut rgb).unwrap();
        out.push((num("/Width ") as u32, num("/Height ") as u32, rgb));
    }
    out
}

#[test]
fn thumbnails_show_the_pages() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 100, "artboards": 2})).unwrap();
    let second = s.doc().unwrap().doc.artboards[1].rect;
    // A red first page, a green second one.
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 200, "height": 100})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    s.execute("shape.rectangle", &json!({"x": second.x0, "y": second.y0, "width": 200, "height": 100})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#00ff00"})).unwrap();
    // A non-printing layer covering the first page in blue.
    let notes = s.execute("layer.new", &json!({})).unwrap()["id"].clone();
    s.execute("layer.setProps", &json!({"id": notes, "printable": false})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#0000ff"})).unwrap();

    let (pdf, warnings) = export(&mut s, json!({"thumbnails": true}));
    assert_eq!(warnings, Vec::<Value>::new());
    let thumbs = thumbnails(&pdf);
    assert_eq!(thumbs.len(), 2, "one a page");
    for ((w, h, rgb), colour) in thumbs.iter().zip([[255, 0, 0], [0, 255, 0]]) {
        assert_eq!((*w, *h), (vectorcraft_pdf::THUMBNAIL_SIZE, 53), "the page's shape, 106 px long");
        // The middle of the left half: the page's colour (the non-printing layer is left out).
        let at = ((*h / 2 * w + w / 4) * 3) as usize;
        assert_eq!(rgb[at..at + 3], colour);
    }
    // Non-printing layers included: the thumbnail shows them too.
    let (pdf, _) = export(&mut s, json!({"thumbnails": true, "includeNonPrinting": true}));
    let (w, h, rgb) = &thumbnails(&pdf)[0];
    let at = ((*h / 2 * w + w / 4) * 3) as usize;
    assert_eq!(rgb[at..at + 3], [0, 0, 255]);
    // With the bleed the page grows, and so does its thumbnail.
    let (pdf, _) = export(&mut s, json!({"thumbnails": true, "bleed": {"top": 50, "bottom": 50, "left": 0, "right": 0}}));
    assert_eq!(thumbnails(&pdf).iter().map(|t| (t.0, t.1)).collect::<Vec<_>>(), [(106, 106), (106, 106)]);
    // Off: none.
    assert!(thumbnails(&export(&mut s, json!({})).0).is_empty());
}

#[test]
fn fast_web_view_through_every_pdf_path() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 80, "artboards": 3})).unwrap();
    s.execute("shape.ellipse", &json!({"x": 10, "y": 10, "width": 40, "height": 30})).unwrap();
    let linearised = |pdf: &[u8]| String::from_utf8_lossy(&pdf[..200.min(pdf.len())]).contains("/Linearized 1");
    let (pdf, warnings) = export(&mut s, json!({"fastWebView": true, "thumbnails": true}));
    assert!(linearised(&pdf) && warnings.is_empty(), "{warnings:?}");
    // Reopened: the pages, and the document the editing data carries.
    s.execute("document.close", &json!({"force": true})).ok();
    let v = s.execute("document.open", &json!({"dataBase64": vectorcraft_format::base64_encode(&pdf), "name": "web.pdf"})).unwrap();
    assert!(v.is_object());
    assert_eq!(s.doc().unwrap().doc.artboards.len(), 3);
    assert!(s.doc().unwrap().doc.layers[0].children().unwrap().iter().any(|n| n.path_data().is_some()), "the ellipse is back");
    // document.export takes the same options; without them the file isn't linearised.
    let v = s.execute("document.export", &json!({"format": "pdf", "fastWebView": true, "range": "2"})).unwrap();
    assert!(linearised(&vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap()));
    assert!(!linearised(&export(&mut s, json!({})).0));
}
