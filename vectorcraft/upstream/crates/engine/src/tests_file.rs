use serde_json::json;

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s
}

#[test]
fn batch_is_one_undo_step() {
    let mut s = session();
    let r = s
        .execute(
            "command.batch",
            &json!({"label": "Logo", "commands": [
                {"command": "paint.setFill", "params": {"color": "#ff0000"}},
                {"command": "shape.rectangle", "params": {"x": 0, "y": 0, "width": 50, "height": 50}},
                {"command": "shape.ellipse", "params": {"x": 60, "y": 0, "width": 50, "height": 50}},
            ]}),
        )
        .unwrap();
    assert_eq!(r["results"].as_array().unwrap().len(), 3);
    let st = s.doc().unwrap();
    assert_eq!(st.history.undo.len(), 1);
    assert_eq!(st.history.undo[0].label, "Logo");
    assert_eq!(st.doc.layers[0].children().unwrap().len(), 2);
    assert_eq!(s.journal.iter().filter(|(c, _)| c == "command.batch").count(), 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc.layers[0].children().unwrap().len(), 0);
}

#[test]
fn batch_rolls_back_on_error() {
    let mut s = session();
    let r = s.execute(
        "command.batch",
        &json!({"commands": [
            {"command": "shape.rectangle", "params": {"x": 0, "y": 0, "width": 50, "height": 50}},
            {"command": "no.such.command", "params": {}},
        ]}),
    );
    assert!(r.is_err());
    assert_eq!(s.doc().unwrap().doc.layers[0].children().unwrap().len(), 0);
    assert!(s.doc().unwrap().history.undo.is_empty());
    assert!(!s.in_interaction());
}

#[test]
fn serialize_and_reopen() {
    let mut s = session();
    s.execute("shape.ellipse", &json!({"x": 10, "y": 10, "width": 80, "height": 40})).unwrap();
    let b64 = s.execute("document.serialize", &json!({"format": "vectorcraft"})).unwrap()["dataBase64"].as_str().unwrap().to_string();
    let svg = s.execute("document.serialize", &json!({"format": "svg"})).unwrap()["text"].as_str().unwrap().to_string();
    assert!(svg.contains("<svg"));
    s.execute("document.open", &json!({"name": "copy.vectorcraft", "dataBase64": b64})).unwrap();
    assert_eq!(s.documents().len(), 2);
    assert_eq!(s.doc().unwrap().doc.node_count(), 2);
    let png = s.execute("document.serialize", &json!({"format": "png", "scale": 0.5})).unwrap()["dataBase64"].as_str().unwrap().to_string();
    assert!(png.len() > 100);
}

#[test]
fn save_and_open_path() {
    let mut s = session();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 80, "height": 40})).unwrap();
    let dir = std::env::temp_dir().join(format!("dc-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.vectorcraft");
    s.execute("document.save", &json!({"path": path.to_string_lossy()})).unwrap();
    assert!(!s.doc().unwrap().is_dirty());
    s.execute("document.open", &json!({"path": path.to_string_lossy()})).unwrap();
    assert_eq!(s.doc().unwrap().path.as_deref(), Some(path.to_string_lossy().as_ref()));
    let svg = dir.join("t.svg");
    s.execute("document.export", &json!({"path": svg.to_string_lossy()})).unwrap_or_default();
    s.execute("document.export", &json!({"format": "svg", "path": svg.to_string_lossy()})).unwrap();
    assert!(std::fs::read_to_string(&svg).unwrap().contains("<svg"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn pdf_roundtrip_through_engine() {
    let mut s = session();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 80, "height": 40})).unwrap();
    let b64 = s.execute("document.serialize", &json!({"format": "pdf"})).unwrap()["dataBase64"].as_str().unwrap().to_string();
    s.execute("document.open", &json!({"name": "x.pdf", "dataBase64": b64})).unwrap();
    assert!(s.doc().unwrap().doc.node_count() >= 2);
}

#[test]
fn draw_behind_and_inside() {
    let mut s = session();
    let a = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    s.execute("view.drawMode", &json!({"mode": "behind"})).unwrap();
    let b = s.execute("shape.ellipse", &json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
    let order: Vec<u64> = s.doc().unwrap().doc.layers[0].children().unwrap().iter().map(|n| n.id.0).collect();
    assert_eq!(order, vec![b, a]);
    s.execute("select.set", &json!({"ids": [a]})).unwrap();
    s.execute("view.drawMode", &json!({"mode": "inside"})).unwrap();
    let c = s.execute("shape.ellipse", &json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    let d = &s.doc().unwrap().doc;
    let g = d.parent_of(NodeId(c)).unwrap();
    assert_eq!(d.node(g).unwrap().kind_label(), "Clip Group");
    assert_eq!(d.parent_of(NodeId(a)), Some(g));
    // A second shape goes into the same clip group.
    s.execute("shape.ellipse", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    assert_eq!(s.doc().unwrap().doc.node(g).unwrap().children().unwrap().len(), 4);
    s.execute("view.drawMode", &json!({"mode": "normal"})).unwrap();
    assert!(s.draw_inside.is_none());
}

#[test]
fn export_for_screens_writes_every_combination() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 80, "artboards": 2})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    let dir = std::env::temp_dir().join(format!("dc-efs-{}", std::process::id()));
    let r = s
        .execute("document.exportForScreens", &json!({"folder": dir.to_string_lossy(), "formats": [{"format": "png", "scale": 1}, {"format": "png", "scale": 2}, {"format": "jpg"}, {"format": "svg"}, {"format": "webp"}]}))
        .unwrap();
    let files = r["files"].as_array().unwrap();
    assert_eq!(files.len(), 10);
    for f in files {
        assert!(std::fs::metadata(f.as_str().unwrap()).unwrap().len() > 50, "{f}");
    }
    assert!(files.iter().any(|f| f.as_str().unwrap().ends_with("Artboard-2@2x.png")));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn recolor_maps_fills_strokes_gradients() {
    let mut s = session();
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    let a = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    s.execute(
        "paint.setFill",
        &json!({"gradient": {"kind": "linear", "stops": [{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}]}}),
    )
    .unwrap();
    s.execute("select.all", &json!({})).unwrap();
    let c = s.execute("recolor.colors", &json!({})).unwrap();
    let hexes: Vec<&str> = c["colors"].as_array().unwrap().iter().map(|v| v["hex"].as_str().unwrap()).collect();
    assert!(hexes.contains(&"#ff0000") && hexes.contains(&"#0000ff") && hexes.contains(&"#000000"));
    s.execute("recolor.apply", &json!({"map": {"#FF0000": "#00ff00"}})).unwrap();
    let n = s.doc().unwrap().doc.node(NodeId(a)).unwrap().clone();
    match n.appearance.fill_paint() {
        vectorcraft_color::Paint::Gradient(g) => assert_eq!(g.gradient.stops[0].color.to_hex(), "#00ff00"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn export_selection_crops_to_the_selection() {
    let mut s = session();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 200, "y": 100, "width": 80, "height": 40})).unwrap();
    let v = s.execute("document.exportSelection", &json!({"format": "png", "scale": 2})).unwrap();
    let b = v["bounds"].as_array().unwrap();
    // Only the selected (last) rectangle, with its stroke.
    let w = b[2].as_f64().unwrap();
    assert!((81.0..90.0).contains(&w) && b[0].as_f64().unwrap() > 190.0, "{v}");
    let png = vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap();
    assert_eq!(&png[..4], b"\x89PNG");
    assert_eq!(u32::from_be_bytes([png[16], png[17], png[18], png[19]]), (w * 2.0).round() as u32);
    let svg = s.execute("document.exportSelection", &json!({"format": "svg"})).unwrap();
    let text = String::from_utf8(vectorcraft_format::base64_decode(svg["dataBase64"].as_str().unwrap()).unwrap()).unwrap();
    assert_eq!(text.matches("<path").count(), 1, "{text}");
}

/// A selection export reports the encoder's warnings like a whole-document export, and an empty
/// list when nothing was approximated.
#[test]
fn export_selection_reports_the_encoders_warnings() {
    let mut s = session();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    s.execute("transparency.set", &json!({"blend": "Multiply"})).unwrap();
    let warnings = |v: &serde_json::Value| v["warnings"].as_array().cloned().unwrap_or_else(|| panic!("no warnings key: {v}"));
    // Control: nothing approximated.
    let plain = s.execute("document.exportSelection", &json!({"format": "svg"})).unwrap();
    assert_eq!(plain["format"], "svg");
    assert!(warnings(&plain).is_empty(), "{plain}");
    for (format, options, says) in [
        ("webp", json!({"lossless": false}), "lossless"),
        ("svg", json!({"profile": "tiny12"}), "blend"),
        ("pdf", json!({"createLayers": true, "compatibility": "1.4"}), "layers"),
    ] {
        let mut p = options.clone();
        p["format"] = json!(format);
        let sel = s.execute("document.exportSelection", &p).unwrap();
        let doc = s.execute("document.export", &p).unwrap();
        let w = warnings(&sel);
        assert!(w.iter().any(|w| w.as_str().unwrap_or("").to_lowercase().contains(says)), "{format}: {sel}");
        assert_eq!(w, warnings(&doc), "{format}: the same warnings as a whole-document export");
    }
}

#[test]
fn template_opens_as_untitled() {
    let mut s = session();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap();
    let dir = std::env::temp_dir().join(format!("vc-tpl-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("letterhead.vectorcraft");
    s.execute("file.saveAsTemplate", &json!({"path": path.to_str().unwrap()})).unwrap();
    s.execute("document.open", &json!({"path": path.to_str().unwrap()})).unwrap();
    let st = s.doc().unwrap();
    assert!(st.path.is_none() && !st.doc.template);
    assert!(st.doc.title.starts_with("Untitled-"), "{}", st.doc.title);
    assert_eq!(st.doc.layers[0].children().unwrap().len(), 1);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn raster_export_too_large_is_an_error_not_a_crash() {
    // The largest artboard (16383 pt) at 300 ppi is ~68 000 px a side: more than the rasteriser can
    // address. The export used to panic inside the renderer (crashing the app or the MCP server).
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 16383, "height": 16383})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 100, "height": 100})).unwrap();
    let dir = std::env::temp_dir().join(format!("vc-raster-limit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for format in ["png", "jpg", "webp"] {
        let path = dir.join(format!("big.{format}"));
        let e = s.execute("document.export", &json!({"path": path.to_string_lossy(), "scale": 300.0 / 72.0})).unwrap_err().to_string();
        assert!(e.contains("pixels"), "{format}: {e}");
        assert!(!path.exists(), "{format}");
        assert!(s.execute("document.serialize", &json!({"format": format, "scale": 300.0 / 72.0})).is_err(), "{format}");
    }
    // A size within the limits still exports.
    let path = dir.join("ok.png");
    s.execute("document.export", &json!({"path": path.to_string_lossy(), "scale": 0.25})).unwrap();
    assert!(std::fs::read(&path).unwrap().starts_with(b"\x89PNG"));
    let _ = std::fs::remove_dir_all(dir);
}

/// Japanese file and folder names save, open, export and title documents as typed: the native
/// format, SVG, PNG and PDF, in a folder with a Japanese name too.
#[test]
fn japanese_file_names_save_open_and_export() {
    let mut s = session();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 80, "height": 40})).unwrap();
    s.execute("text.create", &json!({"x": 10, "y": 80, "text": "日本語のテキスト"})).unwrap();
    let dir = std::env::temp_dir().join(format!("vc-日本語フォルダ-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("運動会カード（最終版）・2024.vectorcraft");
    s.execute("document.save", &json!({"path": path.to_string_lossy()})).unwrap();
    assert!(path.exists());
    assert_eq!(s.doc().unwrap().title(), "運動会カード（最終版）・2024.vectorcraft", "the tab names the file as typed");
    s.execute("document.open", &json!({"path": path.to_string_lossy()})).unwrap();
    let d = s.doc().unwrap();
    assert_eq!(d.path.as_deref(), Some(path.to_string_lossy().as_ref()));
    let mut found = false;
    d.doc.walk(|n| found |= matches!(&n.kind, vectorcraft_doc::NodeKind::Text(t) if t.plain_text() == "日本語のテキスト"));
    assert!(found, "the Japanese text comes back");
    for (format, name) in [("svg", "書き出し.svg"), ("png", "書き出し.png"), ("pdf", "書き出し.pdf")] {
        let out = dir.join(name);
        s.execute("document.export", &json!({"format": format, "path": out.to_string_lossy()})).unwrap();
        assert!(std::fs::metadata(&out).is_ok_and(|m| m.len() > 0), "{name}");
    }
    let _ = std::fs::remove_dir_all(dir);
}
