//! `document.exportPdf {createLayers}` writes PDF layers that reopen as layers, and
//! `advanced.overprint` keeps overprinting in the file or drops it.

use serde_json::{Value, json};

use super::*;

fn b64(v: &Value) -> Vec<u8> {
    vectorcraft_format::base64_decode(v["dataBase64"].as_str().expect("dataBase64")).unwrap()
}

fn warnings(v: &Value) -> Vec<String> {
    v["warnings"].as_array().expect("warnings").iter().map(|w| w.as_str().unwrap().to_string()).collect()
}

/// A document with a rectangle on "Layer 1" and one on a hidden, non-printing layer "Notes".
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 80})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    let notes = s.execute("layer.new", &json!({"name": "Notes"})).unwrap()["id"].as_u64().unwrap();
    s.execute("shape.rectangle", &json!({"x": 50, "y": 10, "width": 20, "height": 20})).unwrap();
    s.execute("layer.setProps", &json!({"id": notes, "visible": false, "printable": false})).unwrap();
    s
}

#[test]
fn export_pdf_create_layers_writes_layers_that_reopen_as_layers() {
    let mut s = session();
    let v = s.execute("document.exportPdf", &json!({"createLayers": true, "preserveEditing": false})).unwrap();
    assert!(warnings(&v).is_empty(), "{:?}", warnings(&v));
    let back = vectorcraft_pdf::import(&b64(&v)).unwrap();
    let layers: Vec<(String, bool, bool)> = back
        .layers
        .iter()
        .map(|l| (l.name.clone().unwrap_or_default(), l.visible, matches!(l.kind, vectorcraft_doc::NodeKind::Layer { printable: true, .. })))
        .collect();
    assert_eq!(layers, [("Layer 1".to_string(), true, true), ("Notes".to_string(), false, false)]);
    // The default preset carries the editing data too.
    let v = s.execute("document.exportPdf", &json!({"createLayers": true})).unwrap();
    assert!(warnings(&v).is_empty(), "{:?}", warnings(&v));
    assert!(vectorcraft_pdf::editing(&b64(&v)).is_some_and(|e| e.intact));
    // At PDF 1.4 there are no PDF layers.
    let v = s.execute("document.exportPdf", &json!({"createLayers": true, "compatibility": "1.4"})).unwrap();
    assert!(warnings(&v).iter().any(|w| w.contains("PDF 1.5")), "{:?}", warnings(&v));
}

#[test]
fn export_pdf_overprint_is_preserved_or_discarded() {
    let mut s = session();
    s.execute("select.all", &json!({})).unwrap();
    // The rectangles' strokes are black (their white fills would knock out: discardWhiteOverprint).
    s.execute("object.setOverprint", &json!({"stroke": true})).unwrap();
    let overprints = |s: &mut Session, p: Value| {
        let v = s.execute("document.exportPdf", &p).unwrap();
        assert!(warnings(&v).iter().all(|w| !w.contains("overprint")), "{:?}", warnings(&v));
        String::from_utf8_lossy(&b64(&v)).matches("/OP true/op true/OPM 1").count()
    };
    assert_eq!(overprints(&mut s, json!({"compression": {"compressText": false}})), 1);
    assert_eq!(overprints(&mut s, json!({"advanced": {"overprint": "discard"}})), 0);
}

/// A `.ai` file's PDF part, which apps that don't read the native document open, has the layers
/// and sublayers as PDF layers, the hidden ones with their art (#372).
#[test]
fn ai_files_write_their_layers_and_sublayers_as_pdf_layers() {
    let mut s = session();
    let layer1 = s.execute("document.inspect", &json!({})).unwrap()["layers"][1]["id"].as_u64().unwrap();
    let sub = s.execute("layer.newSublayer", &json!({"parent": layer1, "name": "Sketch"})).unwrap()["id"].as_u64().unwrap();
    s.execute("layer.setCurrent", &json!({"id": sub})).unwrap();
    s.execute("shape.ellipse", &json!({"x": 10, "y": 40, "width": 20, "height": 20})).unwrap();
    s.execute("layer.setProps", &json!({"id": sub, "visible": false})).unwrap();
    // Name, shown and what it holds (sublayers by name), top-level layers in paint order.
    fn rows(layers: &[std::sync::Arc<vectorcraft_doc::Node>]) -> Vec<(String, bool, Vec<String>)> {
        layers
            .iter()
            .map(|l| {
                let held =
                    l.children().into_iter().flatten().map(|c| c.name.clone().filter(|_| c.is_layer()).unwrap_or_else(|| "art".into())).collect();
                (l.name.clone().unwrap_or_default(), l.visible, held)
            })
            .collect()
    }
    let v = s.execute("document.export", &json!({"format": "ai"})).unwrap();
    let back = vectorcraft_pdf::import(&b64(&v)).unwrap();
    let art = || "art".to_string();
    assert_eq!(rows(&back.layers), [("Layer 1".to_string(), true, vec![art(), "Sketch".into()]), ("Notes".to_string(), false, vec![art()])]);
    let sketch = back.layers.first().and_then(|l| l.children()?.last().cloned()).unwrap();
    assert_eq!(rows(&[sketch]), [("Sketch".to_string(), false, vec![art()])]);
    // Asked not to, it writes none.
    let v = s.execute("document.export", &json!({"format": "ai", "createLayers": false})).unwrap();
    assert_eq!(vectorcraft_pdf::import(&b64(&v)).unwrap().layers.len(), 1);
}
