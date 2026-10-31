//! Plot transparency: the page setup's "Plot transparency" (PAGESETUP `plotTransparency`), the
//! plot's own option, and PLOTTRANSPARENCYOVERRIDE (0 never, 1 the setting, 2 always) deciding
//! what the plotter is told.

use cadcraft_engine::Session;
use cadcraft_engine::cmd::file::{IoHooks, base64_decode, set_io};
use serde_json::{Value, json};

/// A plotter that echoes the `plotTransparency` it was given (`none` when it got none).
fn fake_plot(_: &cadcraft_doc::Drawing, _: &cadcraft_doc::Space, opts: &Value) -> Result<Vec<u8>, String> {
    Ok(opts.get("plotTransparency").map_or("none".to_string(), Value::to_string).into_bytes())
}

fn session() -> Session {
    set_io(IoHooks { read: |_, _| Err("no reader in this test".into()), write: |_, _| Err("no writer in this test".into()), plot: Some(fake_plot) });
    let mut s = Session::new();
    s.execute("line", &json!({"points": [[0, 0], [100, 0]]})).unwrap();
    s
}

/// What the plotter was told for a PLOT with `params`.
fn told(s: &mut Session, params: Value) -> String {
    let r = s.execute("plot", &params).unwrap();
    String::from_utf8(base64_decode(r["data"].as_str().unwrap()).unwrap()).unwrap()
}

#[test]
fn the_override_decides_what_the_plotter_is_told() {
    let mut s = session();
    // 1 (default): the plot's option, else nothing (the plotter reads the page setup).
    assert_eq!(s.settings.plot_transparency_override, 1);
    assert_eq!(told(&mut s, json!({})), "none");
    assert_eq!(told(&mut s, json!({"plotTransparency": true})), "true");
    assert_eq!(told(&mut s, json!({"plotTransparency": false})), "false");
    assert_eq!(told(&mut s, json!({"plotTransparency": "yes"})), "none", "not a boolean: ignored");
    // 0: never.
    s.execute("setvar", &json!({"name": "PLOTTRANSPARENCYOVERRIDE", "value": 0})).unwrap();
    assert_eq!(told(&mut s, json!({"plotTransparency": true})), "false");
    assert_eq!(told(&mut s, json!({})), "false");
    // 2: always.
    s.execute("setvar", &json!({"name": "PLOTTRANSPARENCYOVERRIDE", "value": 2})).unwrap();
    assert_eq!(told(&mut s, json!({"plotTransparency": false})), "true");
    assert_eq!(s.execute("exportpdf", &json!({})).map(|r| r["bytes"].clone()).unwrap(), json!(4), "EXPORTPDF too: `true`");
    let r = s.execute("getvar", &json!({"name": "PLOTTRANSPARENCYOVERRIDE"})).unwrap();
    assert_eq!(r["value"], json!(2));
}

#[test]
fn page_setup_stores_plot_transparency() {
    let mut s = session();
    let r = s.execute("pagesetup", &json!({"layout": "Layout1"})).unwrap();
    assert_eq!(r["page"]["plotTransparency"], json!(false), "off by default");
    let r = s.execute("pagesetup", &json!({"layout": "Layout1", "plotTransparency": true})).unwrap();
    assert_eq!(r["page"]["plotTransparency"], json!(true));
    assert!(s.doc().unwrap().layout("Layout1").unwrap().page.plot_transparency);
    // One undo step, like the other page setup changes.
    s.undo().unwrap();
    assert!(!s.doc().unwrap().layout("Layout1").unwrap().page.plot_transparency);
}

#[test]
fn preview_follows_the_plot_setting() {
    let mut s = session();
    // A red solid hatch at 50 % over the line.
    s.execute("rectang", &json!({"p1": [10, -10], "p2": [90, 10]})).unwrap();
    s.execute("hatch", &json!({"points": [[50, 5]], "pattern": "SOLID"})).unwrap();
    let hatch = s.doc().unwrap().model.iter().find(|e| matches!(e.kind, cadcraft_doc::EntityKind::Hatch(_))).map(|e| e.handle).unwrap();
    s.execute("draworder", &json!({"handles": [hatch.hex()], "order": "front"})).unwrap();
    s.execute("properties.set", &json!({"handles": [hatch.hex()], "color": 1, "transparency": 50})).unwrap();
    let preview = |s: &mut Session, p: Value| s.execute("preview", &p).unwrap()["data"].as_str().unwrap().to_string();
    let off = preview(&mut s, json!({"layout": "Model", "width": 200}));
    let on = preview(&mut s, json!({"layout": "Model", "width": 200, "plotTransparency": true}));
    assert_ne!(off, on, "the preview shows transparency when the plot does");
    s.execute("setvar", &json!({"name": "PLOTTRANSPARENCYOVERRIDE", "value": 0})).unwrap();
    assert_eq!(preview(&mut s, json!({"layout": "Model", "width": 200, "plotTransparency": true})), off);
}
