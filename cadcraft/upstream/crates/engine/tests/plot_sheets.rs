//! PLOT / EXPORTPDF with several sheets (issue #560): `layouts` names the sheets of one PDF
//! (any case, or "all"); the engine checks them and reports the pages.

use cadcraft_engine::Session;
use cadcraft_engine::cmd::file::{IoHooks, base64_decode, set_io};
use serde_json::{Value, json};

/// A plotter that echoes the sheets it was asked for.
fn fake_plot(_: &cadcraft_doc::Drawing, space: &cadcraft_doc::Space, opts: &Value) -> Result<Vec<u8>, String> {
    Ok(format!("%PDF-1.4 {space:?} {}", opts.get("layouts").map(Value::to_string).unwrap_or_default()).into_bytes())
}

fn session() -> Session {
    set_io(IoHooks { read: |_, _| Err("no reader in this test".into()), write: |_, _| Err("no writer in this test".into()), plot: Some(fake_plot) });
    let mut s = Session::new();
    s.execute("line", &json!({"points": [[0, 0], [100, 0]]})).unwrap();
    s.execute("layout.new", &json!({"name": "Sheet B"})).unwrap();
    s
}

fn plotted(r: &Value) -> String {
    String::from_utf8(base64_decode(r["data"].as_str().unwrap()).unwrap()).unwrap()
}

#[test]
fn layouts_name_the_sheets_of_one_pdf() {
    let mut s = session();
    for cmd in ["plot", "exportpdf"] {
        let r = s.execute(cmd, &json!({"layouts": ["model", "sheet b"]})).unwrap();
        assert_eq!(r["pages"], 2);
        assert_eq!(r["layout"], "Model");
        assert!(plotted(&r).ends_with(r#"["model","sheet b"]"#), "the plotter gets the list: {}", plotted(&r));
    }
    // "all": model (it has objects), then the layouts in tab order.
    let r = s.execute("exportpdf", &json!({"layouts": "all"})).unwrap();
    assert_eq!((r["pages"].as_u64(), r["layout"].as_str()), (Some(4), Some("Model")));
    // One sheet without `layouts` as before.
    assert_eq!(s.execute("plot", &json!({})).unwrap()["pages"], 1);
}

#[test]
fn bad_sheet_lists_are_errors() {
    let mut s = session();
    let many = vec!["Model"; 257];
    for p in [json!({"layouts": []}), json!({"layouts": ["Nope"]}), json!({"layouts": "Model"}), json!({"layouts": [7]}), json!({ "layouts": many })]
    {
        assert!(s.execute("exportpdf", &p).is_err(), "{p}");
    }
}
