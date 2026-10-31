//! Opening a file that isn't a drawing (SVG, read into entities): the drawing gets no file name,
//! so QSAVE asks for one instead of writing CADCraft's SVG export over the original.

use cadcraft_engine::Session;
use cadcraft_engine::cmd::file::{IoHooks, set_io};
use cadcraft_engine::doc::Drawing;
use serde_json::json;

#[test]
fn an_opened_svg_is_not_saved_over() {
    set_io(IoHooks { read: |_, _| Ok(Drawing::new_metric()), write: |_, _| Ok(b"exported".to_vec()), plot: None });
    let dir = std::env::temp_dir().join(format!("cadcraft-open-svg-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let svg = dir.join("Logo.svg");
    std::fs::write(&svg, b"<svg/>").unwrap();
    let mut s = Session::empty();
    s.execute("open", &json!({"path": svg.to_string_lossy()})).unwrap();
    let st = s.state().unwrap();
    assert_eq!(st.title, "Logo.svg");
    assert_eq!(st.path, None);
    assert!(s.execute("qsave", &json!({})).is_err(), "QSAVE asks for a file name");
    assert_eq!(std::fs::read(&svg).unwrap(), b"<svg/>", "the original is untouched");

    // A drawing file keeps its path.
    let dxf = dir.join("Plan.dxf");
    std::fs::write(&dxf, b"0\nEOF\n").unwrap();
    s.execute("open", &json!({"path": dxf.to_string_lossy()})).unwrap();
    assert_eq!(s.state().unwrap().path.as_deref(), Some(dxf.to_string_lossy().as_ref()));
    let _ = std::fs::remove_dir_all(&dir);
}
