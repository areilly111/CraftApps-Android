//! WebAssembly plug-ins (#121): object filters through `plugin.run` (one undo step,
//! transactional), live-effect plug-ins in appearance stacks, and the `plugin.*` management
//! commands.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_geom::{PathData, Point, SubPath};
use vectorcraft_plugins::wat as tpl;

use super::*;

/// The example Desaturate plug-in (built from `crates/plugins/example`).
const DESATURATE: &[u8] = include_bytes!("../../plugins/tests/fixtures/desaturate.wasm");
const DESATURATE_ID: &str = "org.vectorcraft.example.desaturate";

fn run(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

fn install(s: &mut Session, wasm: &[u8]) -> Value {
    run(s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(wasm)}))
}

fn install_wat(s: &mut Session, src: &str) -> Value {
    install(s, &wat::parse_str(src).unwrap())
}

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 200})).unwrap();
    s
}

/// A 10 × 10 rectangle at `x` filled with `fill` and no stroke.
fn rect(s: &mut Session, x: f64, fill: &str) -> NodeId {
    let id = NodeId(run(s, "shape.rectangle", json!({"x": x, "y": 10, "width": 10, "height": 10}))["id"].as_u64().unwrap());
    run(s, "paint.setFill", json!({"ids": [id.0], "color": fill}));
    run(s, "paint.setStroke", json!({"ids": [id.0], "none": true}));
    id
}

fn node(s: &Session, id: NodeId) -> vectorcraft_doc::Node {
    s.doc().unwrap().doc.node(id).unwrap().clone()
}

fn triangle() -> PathData {
    PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(30.0, 0.0), Point::new(15.0, 20.0)], true))
}

#[test]
fn object_filter_recolours_the_selection_as_one_undo_step() {
    let mut s = session();
    let info = install(&mut s, DESATURATE);
    assert_eq!((info["id"].as_str(), info["kind"].as_str()), (Some(DESATURATE_ID), Some("filter")));
    let a = rect(&mut s, 10.0, "#ff0000");
    let b = rect(&mut s, 40.0, "#0000ff");
    run(&mut s, "select.all", json!({}));
    let undo = s.doc().unwrap().history.undo.len();
    let r = run(&mut s, "plugin.run", json!({"id": DESATURATE_ID, "params": {"amount": 100}}));
    assert_eq!(r["ids"], json!([a.0, b.0]));
    for id in [a, b] {
        let [r, g, b] = node(&s, id).appearance.fill_paint().color().unwrap().to_rgb_uncalibrated();
        assert!((r - g).abs() < 1e-6 && (g - b).abs() < 1e-6, "grey: {r} {g} {b}");
    }
    let st = s.doc().unwrap();
    assert_eq!(st.history.undo.len(), undo + 1);
    assert_eq!(st.history.undo.last().unwrap().label, "Desaturate");
    assert_eq!(st.selection.objects, [a, b]);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(node(&s, a).appearance.fill_paint(), Paint::solid(Color::rgb(1.0, 0.0, 0.0)));
    // Plug-in parameters may also be top-level keys; wrong types are refused before it runs.
    run(&mut s, "plugin.run", json!({"id": DESATURATE_ID, "amount": 0}));
    assert_eq!(node(&s, a).appearance.fill_paint(), Paint::solid(Color::rgb(1.0, 0.0, 0.0)));
    assert!(matches!(s.execute("plugin.run", &json!({"id": DESATURATE_ID, "amount": "all"})), Err(EngineError::BadParams { .. })));
}

#[test]
fn object_filters_add_replace_and_generate_objects() {
    let mut s = session();
    let a = rect(&mut s, 10.0, "#ff0000");
    let b = rect(&mut s, 40.0, "#00ff00");
    run(&mut s, "select.all", json!({}));
    // Keeps A, drops B and adds a triangle above A.
    let out =
        json!({"objects": [{"id": a.0}, {"path": triangle(), "fills": [{"type": "solid", "color": {"model": "gray", "k": 1}}], "name": "Tri"}]});
    install_wat(&mut s, &tpl::returning(&tpl::manifest("test.engine.replace", "filter", "{}"), &out.to_string()));
    let r = run(&mut s, "plugin.run", json!({"id": "test.engine.replace"}));
    let ids: Vec<NodeId> = r["ids"].as_array().unwrap().iter().map(|v| NodeId(v.as_u64().unwrap())).collect();
    assert_eq!(ids[0], a);
    let st = s.doc().unwrap();
    assert!(st.doc.node(b).is_none());
    let layer: Vec<NodeId> = st.doc.layers[0].children().unwrap().iter().map(|n| n.id).collect();
    assert_eq!(layer, [a, ids[1]]);
    assert_eq!(st.doc.node(ids[1]).unwrap().name.as_deref(), Some("Tri"));
    assert_eq!(st.selection.objects, ids);
    // A generator with nothing selected adds to the current layer.
    let generated = json!({"objects": [{"path": triangle()}]});
    install_wat(&mut s, &tpl::returning(&tpl::manifest("test.engine.generate", "filter", "{}"), &generated.to_string()));
    run(&mut s, "select.none", json!({}));
    let r = run(&mut s, "plugin.run", json!({"id": "test.engine.generate"}));
    let made = NodeId(r["ids"][0].as_u64().unwrap());
    assert_eq!(s.doc().unwrap().doc.layers[0].children().unwrap().last().unwrap().id, made);
}

#[test]
fn failing_plugins_leave_the_document_and_history_alone() {
    let mut s = session();
    rect(&mut s, 10.0, "#ff0000");
    run(&mut s, "select.all", json!({}));
    let man = |id: &str| tpl::manifest(id, "filter", "{}");
    install_wat(&mut s, &tpl::module(&man("test.engine.trap"), "unreachable", "", 1));
    install_wat(&mut s, &tpl::returning(&man("test.engine.far"), r#"{"objects":[{"path":{"subpaths":[{"anchors":[{"p":[0,0]},{"p":[1e9,0]}]}]}}]}"#));
    install_wat(&mut s, &tpl::returning(&man("test.engine.declines"), r#"{"error":"Select two paths."}"#));
    let before = s.doc().unwrap().doc.clone();
    let undo = s.doc().unwrap().history.undo.len();
    for id in ["test.engine.trap", "test.engine.far", "test.engine.declines", "test.engine.missing"] {
        let e = s.execute("plugin.run", &json!({"id": id})).unwrap_err();
        if id == "test.engine.declines" {
            assert!(e.to_string().contains("Select two paths."), "{e}");
        }
        assert_eq!(s.doc().unwrap().doc, before, "{id}");
        assert_eq!(s.doc().unwrap().history.undo.len(), undo, "{id}");
    }
}

#[test]
fn live_effect_plugins_reshape_editable_art_and_expand() {
    let mut s = session();
    let a = rect(&mut s, 10.0, "#ff0000");
    // An effect that draws a triangle, and one that hands the geometry back.
    let tri = json!({"objects": [{"path": triangle()}]});
    install_wat(
        &mut s,
        &tpl::returning(&tpl::manifest("test.engine.tri", "effect", r#"{"size":{"type":"number","min":0,"max":10,"default":3}}"#), &tri.to_string()),
    );
    install_wat(&mut s, &tpl::module(&tpl::manifest("test.engine.echo", "effect", "{}"), tpl::ECHO, "", 1));
    let catalog = run(&mut s, "effect.list", json!({}))["catalog"].clone();
    let entry = catalog.as_array().unwrap().iter().find(|e| e["id"] == "plugin.test.engine.tri").expect("listed");
    assert_eq!((entry["label"].as_str(), entry["menu"].clone()), (Some("Test test.engine.tri…"), json!(["Effect", "Plug-ins"])));
    assert_eq!(entry["defaults"], json!({"size": 3.0}));
    // A filter can't be applied as an effect, nor an effect run as a filter.
    install(&mut s, DESATURATE);
    assert!(s.execute("effect.apply", &json!({"ids": [a.0], "effect": format!("plugin.{DESATURATE_ID}")})).is_err());
    assert!(s.execute("plugin.run", &json!({"id": "test.engine.tri"})).is_err());

    let effected = |s: &Session| {
        let n = node(s, a);
        let path = n.path_data().unwrap().clone();
        let b = path.bounds().unwrap();
        vectorcraft_render::effects::apply_geometry(&n.appearance.effects, &path, b)
    };
    run(&mut s, "effect.apply", json!({"ids": [a.0], "effect": "plugin.test.engine.echo"}));
    assert_eq!(effected(&s), *node(&s, a).path_data().unwrap(), "identity");
    // Re-evaluated when the object changes (non-destructive).
    run(&mut s, "select.all", json!({}));
    run(&mut s, "object.move", json!({"dx": 50, "dy": 0}));
    assert_eq!(effected(&s), *node(&s, a).path_data().unwrap());
    run(&mut s, "effect.apply", json!({"ids": [a.0], "effect": "plugin.test.engine.tri", "params": {"size": 4}}));
    assert_eq!(effected(&s), triangle());
    assert_eq!(node(&s, a).appearance.effects[1].params, json!({"size": 4}));
    // Uninstalled: the geometry is drawn as it is.
    run(&mut s, "plugin.remove", json!({"id": "test.engine.tri"}));
    assert_eq!(effected(&s), *node(&s, a).path_data().unwrap());
    install_wat(&mut s, &tpl::returning(&tpl::manifest("test.engine.tri", "effect", "{}"), &tri.to_string()));
    // Expand Appearance bakes it.
    run(&mut s, "select.all", json!({}));
    run(&mut s, "effect.expandAppearance", json!({}));
    let mut baked = None;
    s.doc().unwrap().doc.walk(|n| {
        if let Some(p) = n.path_data() {
            baked = Some(p.clone());
        }
    });
    assert_eq!(baked.unwrap().bounds(), triangle().bounds());
}

#[test]
fn plugin_management_commands() {
    let mut s = session();
    install(&mut s, DESATURATE);
    let list = run(&mut s, "plugin.list", json!({}));
    assert!(list["plugins"].as_array().unwrap().iter().any(|p| p["id"] == DESATURATE_ID));
    let info = run(&mut s, "plugin.info", json!({"id": DESATURATE_ID}));
    assert_eq!(info["defaults"], json!({"amount": 100.0}));
    assert_eq!(info["params"], "{amount: 0..100 (100)}");
    assert_eq!(info["manifest"]["kind"], "filter");
    // Refused installs.
    for bad in [
        json!({}),
        json!({"dataBase64": "!!"}),
        json!({"dataBase64": "AGFzbQ=="}),
        json!({"dataBase64": vectorcraft_format::base64_encode(DESATURATE), "replace": false}),
    ] {
        assert!(s.execute("plugin.install", &bad).is_err(), "{bad}");
    }
    assert!(s.execute("plugin.info", &json!({"id": "test.engine.nothing"})).is_err());
    assert!(s.execute("plugin.remove", &json!({"id": "test.engine.nothing"})).is_err());
    assert!(s.execute("plugin.reload", &json!({})).is_err(), "no folder set");

    // The Additional Plug-ins Folder preference installs what's in it.
    let dir = std::env::temp_dir().join(format!("vectorcraft-engine-plugins-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = tpl::returning(&tpl::manifest("test.engine.folder", "filter", "{}"), r#"{"objects":[]}"#);
    std::fs::write(dir.join("folder.wasm"), wat::parse_str(src).unwrap()).unwrap();
    std::fs::write(dir.join("broken.wasm"), b"\0asm?").unwrap();
    run(&mut s, "prefs.set", json!({"key": "pluginsFolder", "value": dir.display().to_string()}));
    assert!(vectorcraft_plugins::registry::get("test.engine.folder").is_some());
    run(&mut s, "plugin.remove", json!({"id": "test.engine.folder"}));
    let r = run(&mut s, "plugin.reload", json!({}));
    assert_eq!(r["loaded"], json!(["test.engine.folder"]));
    assert_eq!(r["failed"].as_array().unwrap().len(), 1);
    assert_eq!(run(&mut s, "plugin.list", json!({}))["folder"]["loaded"], json!(["test.engine.folder"]));
    std::fs::remove_dir_all(&dir).ok();
}
