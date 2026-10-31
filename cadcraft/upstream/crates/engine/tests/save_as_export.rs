//! Saving to an image or PDF path exports the drawing; it isn't a save of the drawing (#327). The
//! drawing keeps its name and its unsaved changes, so closing still asks and QSAVE doesn't write
//! the image again.

use cadcraft_engine::Session;
use cadcraft_engine::cmd::file::{IoHooks, set_io};
use serde_json::json;

#[test]
fn saveas_to_an_image_keeps_the_drawing_unsaved_under_its_name() {
    set_io(IoHooks { read: |_, _| Err("no reader in this test".into()), write: |_, _| Ok(b"0\nEOF\n".to_vec()), plot: None });
    let dir = std::env::temp_dir().join(format!("cadcraft-saveas-export-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::new();
    s.cmdline("circle 0,0 5").unwrap();
    let title = s.state().unwrap().title.clone();

    for ext in ["png", "svg", "pdf"] {
        let out = dir.join(format!("plate.{ext}"));
        s.execute("saveas", &json!({ "path": out.to_string_lossy() })).unwrap();
        assert!(out.exists(), "{ext}: the export is written");
        let st = s.state().unwrap();
        assert!(st.is_dirty(), "{ext}: an export must not clear the unsaved changes");
        assert_eq!((st.path.as_deref(), st.title.as_str()), (None, title.as_str()), "{ext}: the drawing keeps its name");
    }
    // QSAVE with a path is the same export.
    s.execute("qsave", &json!({ "path": dir.join("again.png").to_string_lossy() })).unwrap();
    assert!(s.state().unwrap().is_dirty());
    // Still no file name: a plain QSAVE asks for one instead of exporting the image again.
    assert!(s.execute("qsave", &json!({})).is_err());

    // A drawing file is a save: name, title and saved state follow it.
    let dxf = dir.join("plate.dxf");
    s.execute("saveas", &json!({ "path": dxf.to_string_lossy() })).unwrap();
    let st = s.state().unwrap();
    assert!(!st.is_dirty());
    assert_eq!((st.path.as_deref(), st.title.as_str()), (Some(dxf.to_string_lossy().as_ref()), "plate.dxf"));
    // Exporting afterwards keeps that name, so QSAVE writes the DXF again.
    s.execute("saveas", &json!({ "path": dir.join("plate.png").to_string_lossy() })).unwrap();
    let r = s.execute("qsave", &json!({})).unwrap();
    assert_eq!(r["path"], json!(dxf.to_string_lossy()));
    let _ = std::fs::remove_dir_all(&dir);
}
