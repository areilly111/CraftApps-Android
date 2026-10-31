//! WBLOCK writes the drawing with its base point at the origin (INSBASE 0,0,0), for selected
//! objects and for a block, where `base` overrides the block's own base point.

use cadcraft_engine::Session;
use cadcraft_engine::cmd::file::{IoHooks, set_io};
use cadcraft_engine::doc::{Drawing, EntityKind};
use serde_json::json;

/// Writes "INSBASE x,y;line ax,ay" for each line, so the test can read the written geometry.
fn write(d: &Drawing, _path: &str) -> Result<Vec<u8>, String> {
    let ins = d.header.point("INSBASE").ok_or("no INSBASE")?;
    let mut out = format!("{},{}", ins.x, ins.y);
    for e in d.model.iter() {
        if let EntityKind::Line(l) = &e.kind {
            out.push_str(&format!(";{},{}", l.a.x, l.a.y));
        }
    }
    Ok(out.into_bytes())
}

fn written(s: &mut Session, p: serde_json::Value) -> String {
    let path = std::env::temp_dir().join(format!("cadcraft-wblock-base-{}.dxf", std::process::id()));
    let mut p = p;
    p["path"] = json!(path.to_string_lossy());
    s.execute("wblock", &p).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    text
}

#[test]
fn wblock_honours_the_base_point() {
    set_io(IoHooks { read: |_, _| Err("no reader in this test".into()), write, plot: None });
    let mut s = Session::new();
    let h = s.execute("line", &json!({ "points": [[10, 20], [15, 20]] })).unwrap()["handles"][0].as_str().unwrap().to_string();

    // Objects: the base point moves to the origin.
    assert_eq!(written(&mut s, json!({ "handles": [h], "base": [10, 5] })), "0,0;0,15");
    assert_eq!(written(&mut s, json!({ "handles": [h] })), "0,0;10,20");

    // A block: its base point by default, `base` (block coordinates) when given.
    s.execute("block", &json!({ "name": "Part", "base": [4, 4], "handles": [h], "keep": "delete" })).unwrap();
    assert_eq!(written(&mut s, json!({ "name": "Part" })), "0,0;6,16");
    assert_eq!(written(&mut s, json!({ "name": "Part", "base": [10, 20] })), "0,0;0,0");
}
