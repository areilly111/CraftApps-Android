//! File → Place (`file.place`, `file.place.info`, `image.info`) and the place cursor
//! (`file.place.queue` and the `place` tool).

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::Paint;
use vectorcraft_doc::{AppearanceItem, Node, NodeKind};
use vectorcraft_geom::Rect;
use vectorcraft_tools::{Mods, PointerEvent, PointerKind, ToolKey};

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s
}

/// A `w`×`h` PNG of one colour (`rgba`), declaring `ppi` when given.
fn png(w: u32, h: u32, rgba: [u8; 4], ppi: Option<f64>) -> Vec<u8> {
    let mut out = vec![];
    image::RgbaImage::from_pixel(w, h, image::Rgba(rgba)).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    match ppi {
        Some(r) => cmd::fileio::ppi::with_png_resolution(&out, (r, r)),
        None => out,
    }
}

fn file(name: &str, bytes: &[u8]) -> Value {
    json!({ "name": name, "dataBase64": vectorcraft_format::base64_encode(bytes) })
}

/// `file.place` of `bytes` named `name`, with `extra` params.
fn place(s: &mut Session, name: &str, bytes: &[u8], extra: Value) -> Value {
    let mut p = file(name, bytes);
    if let (Some(o), Value::Object(e)) = (p.as_object_mut(), extra) {
        o.extend(e);
    }
    s.execute("file.place", &p).unwrap()
}

fn node(s: &Session, id: &Value) -> Node {
    s.doc().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap().clone()
}

fn top_children(s: &Session) -> Vec<Arc<Node>> {
    s.doc().unwrap().doc.layers.last().unwrap().children().unwrap().clone()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

fn serialize(s: &mut Session, format: &str) -> Vec<u8> {
    let v = s.execute("document.serialize", &json!({"format": format})).unwrap();
    vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap()
}

#[test]
fn malformed_place_coordinates_are_rejected_instead_of_silently_changed() {
    let mut s = session();
    let png = png(4, 2, [20, 40, 60, 255], None);
    let original = s.doc().unwrap().doc.layers.clone();
    // Previously filter_map dropped the string from a five-element rectangle and
    // treated the remaining four values as a different, valid placement.
    for rect in [json!([0, "ignored", 10, 40, 50]), json!([10, 20, 40, null, 50]), json!([10, 20, 40, 50, 60])] {
        let mut params = file("tiny.png", &png);
        params["rect"] = rect;
        assert!(s.execute("file.place", &params).is_err(), "{params}");
        assert_eq!(s.doc().unwrap().doc.layers, original, "failed Place must not add art");
    }
    // An explicitly supplied invalid position formerly behaved as though no
    // position were supplied and centred the image on the artboard.
    for at in [json!(["not-a-number", 20]), json!([10, null]), json!([10, 20, 30]), json!("invalid")] {
        let mut params = file("tiny.png", &png);
        params["at"] = at;
        assert!(s.execute("file.place", &params).is_err(), "{params}");
        assert_eq!(s.doc().unwrap().doc.layers, original);
    }
    let mut params = file("tiny.png", &png);
    params["rect"] = json!([10, 20, 80, 60]);
    let placed = s.execute("file.place", &params).unwrap();
    assert_eq!(placed["ids"].as_array().unwrap().len(), 1);
    // A null position is no position, as for the other optional params.
    let mut params = file("tiny.png", &png);
    params["at"] = Value::Null;
    assert_eq!(s.execute("file.place", &params).unwrap()["ids"].as_array().unwrap().len(), 1);
}

#[test]
fn a_300_px_png_at_300_ppi_places_as_72_pt_centred_selected_and_undoable() {
    let mut s = session();
    let r = place(&mut s, "photo.png", &png(300, 150, [255, 0, 0, 255], Some(300.0)), json!({}));
    assert_eq!((r["format"].as_str(), r["linked"].as_bool()), (Some("png"), Some(false)), "bytes have no path to link");
    assert!(close(r["width"].as_f64().unwrap(), 72.0) && close(r["height"].as_f64().unwrap(), 36.0), "{r}");
    let id = &r["ids"][0];
    let n = node(&s, id);
    assert_eq!(n.name.as_deref(), Some("photo.png"));
    let b = n.geometric_bounds().unwrap();
    assert!(close(b.center().x, 200.0) && close(b.center().y, 150.0), "centred on the artboard: {b:?}");
    let st = s.doc().unwrap();
    assert_eq!(st.selection.objects, [NodeId(id.as_u64().unwrap())]);
    assert_eq!((st.history.undo.len(), st.history.undo[0].label.as_str()), (1, "Place"));
    // No resolution: 72 ppi, one point per pixel; `at` centres it there.
    let r = place(&mut s, "plain.png", &png(30, 20, [0, 0, 255, 255], None), json!({"at": [50, 60]}));
    let b = node(&s, &r["ids"][0]).geometric_bounds().unwrap();
    assert!(close(b.width(), 30.0) && close(b.center().x, 50.0) && close(b.center().y, 60.0), "{b:?}");
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(top_children(&s).is_empty(), "each Place is one undo step");
}

#[test]
fn rect_fits_the_art_inside_keeping_its_aspect() {
    let mut s = session();
    let r = place(&mut s, "wide.png", &png(40, 20, [0, 0, 0, 255], None), json!({"rect": [10, 10, 100, 100]}));
    let b = node(&s, &r["ids"][0]).geometric_bounds().unwrap();
    assert!(close(b.width(), 100.0) && close(b.height(), 50.0) && close(b.center().y, 60.0), "{b:?}");
}

#[test]
fn svg_with_a_data_png_renders_and_leaves_the_clipboard_alone() {
    let mut s = session();
    let rect = s.execute("shape.rectangle", &json!({"x": 300, "y": 10, "width": 20, "height": 20})).unwrap()["id"].clone();
    s.execute("select.set", &json!({"ids": [rect]})).unwrap();
    s.execute("edit.copy", &json!({})).unwrap();
    let clipboard = s.clipboard.clone();
    let red = vectorcraft_format::base64_encode(&png(4, 2, [255, 0, 0, 255], None));
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><image width="40" height="20" href="data:image/png;base64,{red}"/><rect width="10" height="10" fill="#0000ff"/></svg>"##
    );
    let r = place(&mut s, "art.svg", svg.as_bytes(), json!({"at": [100, 100]}));
    assert_eq!(r["format"], "svg");
    let g = node(&s, &r["ids"][0]);
    let NodeKind::Group { children, clip: false } = &g.kind else { panic!("one group: {:?}", g.kind) };
    assert_eq!(children.len(), 2);
    assert_eq!(g.name.as_deref(), Some("art.svg"));
    assert_eq!(s.clipboard, clipboard, "Place never touches the clipboard");
    let doc = &s.doc().unwrap().doc;
    assert!(doc.images.len() == 1, "the embedded image joined the document");
    // The group spans 80..120 × 90..110: blue square top left, red image elsewhere.
    let img = vectorcraft_testkit::raster::render_region(doc, Rect::new(80.0, 90.0, 120.0, 110.0), 1.0);
    assert_eq!(img.over_white(30, 15), [255, 0, 0]);
    assert_eq!(img.over_white(4, 4), [0, 0, 255]);
}

#[test]
fn replace_keeps_the_transform_and_the_stacking_place() {
    let mut s = session();
    let first = place(&mut s, "a.png", &png(100, 50, [255, 0, 0, 255], None), json!({"at": [150, 150]}))["ids"][0].clone();
    s.execute("object.rotate", &json!({"angle": 30})).unwrap();
    s.execute("object.scale", &json!({"sx": 50})).unwrap();
    let centre = node(&s, &first).geometric_bounds().unwrap().center();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    s.execute("select.set", &json!({"ids": [first]})).unwrap();
    assert!(s.execute("file.place", &json!({"replace": true, "at": [0, 0], "name": "b.png", "dataBase64": ""})).is_err());
    let r = place(&mut s, "b.png", &png(20, 20, [0, 255, 0, 255], Some(144.0)), json!({"replace": true}));
    let id = &r["ids"][0];
    assert_eq!(top_children(&s)[0].id.0, id.as_u64().unwrap(), "same place in the stacking order");
    assert_eq!(top_children(&s).len(), 2);
    let c = node(&s, id).geometric_bounds().unwrap().center();
    assert!(close(c.x, centre.x) && close(c.y, centre.y), "{c:?} vs {centre:?}");
    // 144 ppi at the old image's 50%: 288 ppi, rotated as it was.
    let info = s.execute("image.info", &json!({})).unwrap();
    assert!(close(info["ppi"][0].as_f64().unwrap(), 288.0) && close(info["ppi"][1].as_f64().unwrap(), 288.0), "{info}");
    let NodeKind::Image(im) = &node(&s, id).kind else { panic!() };
    let [a, b, ..] = im.xf.as_coeffs();
    assert!(close(b.atan2(a).to_degrees(), -30.0), "{:?}", im.xf);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(top_children(&s)[0].id.0, first.as_u64().unwrap(), "one undo step brings the old image back");
}

#[test]
fn replace_needs_exactly_one_selected_object() {
    let mut s = session();
    let r = s
        .execute("file.place", &json!({"replace": true, "name": "b.png", "dataBase64": vectorcraft_format::base64_encode(&png(2, 2, [0; 4], None))}));
    assert!(r.unwrap_err().to_string().contains("select the one object"));
}

#[test]
fn template_puts_the_art_on_a_locked_template_layer_below_the_current_one() {
    let mut s = session();
    s.execute("layer.new", &json!({})).unwrap();
    let current = s.doc().unwrap().current_layer().unwrap();
    let r = place(&mut s, "sketch.png", &png(10, 10, [0, 0, 0, 255], None), json!({"template": true}));
    let st = s.doc().unwrap();
    let layers = &st.doc.layers;
    assert_eq!(layers.len(), 3);
    let t = layers.iter().position(|l| l.name.as_deref() == Some("Template sketch.png")).unwrap();
    assert_eq!(layers[t + 1].id, current, "right below the current layer");
    assert!(layers[t].locked && matches!(layers[t].kind, NodeKind::Layer { template: true, .. }));
    assert_eq!(st.doc.layer_of(NodeId(r["ids"][0].as_u64().unwrap())), Some(layers[t].id));
    assert!(st.selection.is_empty(), "locked template art isn't selected");
    assert_eq!(st.history.undo.len(), 2, "layer.new, then Place as one step");
}

#[test]
fn pdf_pages_and_native_artboards_place_as_clipped_groups() {
    let mut src = Session::new();
    src.execute("file.new", &json!({"width": 100, "height": 50, "artboards": 2})).unwrap();
    src.execute("artboard.setProps", &json!({"index": 1, "width": 40})).unwrap();
    let b1 = src.doc().unwrap().doc.artboards[1].rect;
    src.execute("shape.rectangle", &json!({"x": b1.x0 + 5.0, "y": 5, "width": 20, "height": 10})).unwrap();
    let pdf = serialize(&mut src, "pdf");
    let native = serialize(&mut src, "vectorcraft");
    let mut s = session();
    for (name, bytes) in [("two.pdf", &pdf), ("two.vectorcraft", &native)] {
        let r = place(&mut s, name, bytes, json!({"page": 2}));
        assert!(close(r["width"].as_f64().unwrap(), 40.0) && close(r["height"].as_f64().unwrap(), 50.0), "{name}: {r}");
        let g = node(&s, &r["ids"][0]);
        assert!(matches!(g.kind, NodeKind::Group { clip: true, .. }), "{name}: clipped to the page");
        let r = place(&mut s, name, bytes, json!({"page": 2, "crop": "bounding"}));
        assert!((r["width"].as_f64().unwrap() - 20.0).abs() < 1.5, "{name}: the art's bounds: {r}");
        let err = s.execute("file.place", &json!({"name": name, "dataBase64": vectorcraft_format::base64_encode(bytes), "page": 3})).unwrap_err();
        assert!(err.to_string().contains("has 2"), "{name}: {err}");
    }
}

#[test]
fn placed_art_brings_its_symbols_and_swatches_renaming_on_conflict() {
    // The file: a global "Brand" red rectangle and a "Star" symbol instance.
    let mut src = session();
    src.execute("swatch.new", &json!({"name": "Brand", "color": "#ff0000", "global": true})).unwrap();
    src.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    src.execute("paint.setFill", &json!({"swatch": "Brand"})).unwrap();
    src.execute("shape.ellipse", &json!({"x": 50, "y": 10, "width": 20, "height": 20})).unwrap();
    src.execute("symbol.new", &json!({"name": "Star"})).unwrap();
    let bytes = serialize(&mut src, "vectorcraft");
    // The document: its own, different "Brand" and "Star".
    let mut s = session();
    s.execute("swatch.new", &json!({"name": "Brand", "color": "#00ff00", "global": true})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 5, "height": 5})).unwrap();
    s.execute("symbol.new", &json!({"name": "Star"})).unwrap();
    let r = place(&mut s, "logo.vectorcraft", &bytes, json!({"crop": "bounding"}));
    let g = node(&s, &r["ids"][0]);
    let (mut fill, mut symbol) = (None, None);
    g.walk(&mut |n| match &n.kind {
        NodeKind::SymbolInstance { symbol: name, .. } => symbol = Some(name.clone()),
        NodeKind::Path { .. } => {
            fill = n.appearance.items.iter().find_map(|i| if let AppearanceItem::Fill(f) = i { Some(f.paint.clone()) } else { None })
        }
        _ => {}
    });
    assert_eq!(symbol.as_deref(), Some("Star 2"));
    let doc = &s.doc().unwrap().doc;
    assert!(doc.symbols.iter().any(|x| x.name == "Star 2") && doc.symbols.iter().any(|x| x.name == "Star"));
    let Some(Paint::Solid { color, swatch: None, .. }) = fill else { panic!("unlinked: {fill:?}") };
    assert_eq!(color.to_hex(), "#ff0000", "keeps its look");
    assert_eq!(doc.swatch("Brand").and_then(|w| w.paint.color()).map(|c| c.to_hex()).as_deref(), Some("#00ff00"));
    // A document without them gets them as they are.
    let mut fresh = session();
    let r = place(&mut fresh, "logo.vectorcraft", &bytes, json!({}));
    let doc = &fresh.doc().unwrap().doc;
    assert!(doc.symbols.iter().any(|x| x.name == "Star") && doc.swatch("Brand").is_some());
    let mut linked = false;
    node(&fresh, &r["ids"][0]).walk(&mut |n| {
        linked |= n
            .appearance
            .items
            .iter()
            .any(|i| matches!(i, AppearanceItem::Fill(f) if matches!(&f.paint, Paint::Solid { swatch: Some(w), .. } if w == "Brand")));
    });
    assert!(linked, "still linked to the swatch it brought");
}

#[test]
fn info_reports_size_resolution_colour_mode_and_link() {
    let mut s = session();
    let mut gray = vec![];
    image::GrayImage::from_pixel(300, 150, image::Luma([128])).write_to(&mut std::io::Cursor::new(&mut gray), image::ImageFormat::Png).unwrap();
    let gray = cmd::fileio::ppi::with_png_resolution(&gray, (300.0, 300.0));
    let mut p = file("scan.png", &gray);
    p["thumbnail"] = json!(16);
    let info = s.execute("file.place.info", &p).unwrap();
    assert_eq!((info["colorMode"].as_str(), info["pixelWidth"].as_u64()), (Some("Grayscale"), Some(300)));
    assert_eq!(info["ppi"], json!([300.0, 300.0]));
    assert!(close(info["width"].as_f64().unwrap(), 72.0));
    let thumb = image::load_from_memory(&vectorcraft_format::base64_decode(info["thumbnailBase64"].as_str().unwrap()).unwrap()).unwrap();
    assert_eq!((thumb.width(), thumb.height()), (16, 8));
    assert!(top_children(&s).is_empty(), "info places nothing");
    // A path is linked by default.
    let path = std::env::temp_dir().join(format!("vectorcraft-place-{}.png", std::process::id()));
    std::fs::write(&path, &gray).unwrap();
    let path = path.to_string_lossy().to_string();
    let r = s.execute("file.place", &json!({"path": path})).unwrap();
    assert_eq!(r["linked"], true);
    let im = s.execute("image.info", &json!({})).unwrap();
    assert_eq!((im["linked"].as_bool(), im["link"].as_str(), im["colorMode"].as_str()), (Some(true), Some(path.as_str()), Some("Grayscale")));
    assert!(close(im["ppi"][0].as_f64().unwrap(), 300.0), "{im}");
    let r = s.execute("file.place", &json!({"path": path, "link": false})).unwrap();
    assert_eq!(r["linked"], false);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_queue_of_3_with_click_drag_and_esc_places_2() {
    let mut s = session();
    let view = ViewInfo::default();
    let files: Vec<Value> = ["a.png", "b.png", "c.png"].iter().map(|n| file(n, &png(40, 20, [9, 9, 9, 255], None))).collect();
    let r = s.execute("file.place.queue", &json!({"files": files, "thumbnail": 8})).unwrap();
    assert_eq!(r["count"], 3);
    assert!(r["files"][0]["thumbnailBase64"].is_string());
    assert_eq!(s.tool_id(), "place");
    assert_eq!(s.tool_options()["name"], "a.png");
    let ev = |kind, x, y| PointerEvent::new(kind, x, y);
    // Click: 100% with its top-left corner at the click.
    s.pointer(&ev(PointerKind::Down, 10.0, 10.0), view).unwrap();
    s.pointer(&ev(PointerKind::Up, 10.0, 10.0), view).unwrap();
    let a = s.doc().unwrap().selection.objects[0];
    assert_eq!(s.doc().unwrap().doc.node(a).unwrap().geometric_bounds(), Some(Rect::new(10.0, 10.0, 50.0, 30.0)));
    // Drag: the dragged size, aspect kept.
    s.pointer(&ev(PointerKind::Down, 100.0, 100.0), view).unwrap();
    s.pointer(&ev(PointerKind::Drag, 180.0, 110.0), view).unwrap();
    s.pointer(&ev(PointerKind::Up, 180.0, 110.0), view).unwrap();
    let b = s.doc().unwrap().selection.objects[0];
    assert_eq!(s.doc().unwrap().doc.node(b).unwrap().name.as_deref(), Some("b.png"));
    assert_eq!(s.doc().unwrap().doc.node(b).unwrap().geometric_bounds(), Some(Rect::new(100.0, 100.0, 180.0, 140.0)));
    // Esc discards the last one and asks for the previous tool back.
    assert!(s.tool_claims_key(ToolKey::Escape, view));
    let reqs = s.tool_key(ToolKey::Escape, Mods::default(), view).unwrap();
    assert_eq!(reqs, vec![UiRequest::SwitchTool("selection".into())]);
    assert_eq!(top_children(&s).len(), 2);
    assert_eq!(s.doc().unwrap().history.undo.len(), 2, "one undo step per placement");
}

#[test]
fn bad_place_params_are_errors() {
    let mut s = session();
    let ok = vectorcraft_format::base64_encode(&png(2, 2, [0; 4], None));
    for p in [
        json!({}),
        json!({"name": "x.png", "dataBase64": "%%%"}),
        json!({"name": "notes.xyz", "dataBase64": vectorcraft_format::base64_encode(b"hello")}),
        json!({"name": "x.png", "dataBase64": ok, "rect": [0, 0, 0, 10]}),
        json!({"name": "x.png", "dataBase64": ok, "at": [1e12, 0]}),
        json!({"name": "x.png", "dataBase64": ok, "page": 0}),
        json!({"name": "x.png", "dataBase64": ok, "crop": "trim"}),
        json!({"name": "x.png", "dataBase64": ok, "template": true, "replace": true}),
    ] {
        assert!(s.execute("file.place", &p).is_err(), "{p}");
    }
    assert!(s.execute("file.place.queue", &json!({})).is_err());
    assert!(s.execute("file.place.queue", &json!({"files": [{"name": "notes.txt", "dataBase64": ""}]})).is_err());
    assert_eq!(s.tool_id(), "selection", "a failed queue leaves the tool alone");
    assert!(top_children(&s).is_empty());
}
