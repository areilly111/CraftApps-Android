//! TIFF, BMP and Targa Options through Export As, drawn headlessly.

use serde_json::json;

use super::tests_export::{app, frame, kind, set};
use super::*;

#[test]
fn tiff_options_pick_the_model_byte_order_and_compression() {
    let (mut app, written) = app(1);
    app.run("file.exportAs", json!({"format": "tiff"})).unwrap();
    confirm(&mut app).unwrap();
    assert_eq!(kind(&app), Some("tiffOptions"));
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((spec(&d.kind).heading)(d), "TIFF Options");
    assert_eq!((d.str("colorModel"), d.str("byteOrder"), d.bool("lzw"), d.bool("embedIcc")), ("rgb".into(), "little".into(), true, true));
    frame(&mut app);
    set(&mut app, "byteOrder", json!("big"));
    set(&mut app, "colorModel", json!("cmyk"));
    set(&mut app, "lzw", json!(false));
    frame(&mut app);
    confirm(&mut app).unwrap();
    let (path, tiff) = written.borrow()[0].clone();
    assert_eq!(path, "/out/Untitled-1.tif");
    assert_eq!(&tiff[..4], b"MM\0*");
    assert!(app.ui.dialog.is_none());

    // A CMYK document exports CMYK by default.
    app.run("object.convertDocumentColorMode", json!({"mode": "cmyk"})).unwrap();
    app.run("file.exportAs", json!({"format": "tiff"})).unwrap();
    confirm(&mut app).unwrap();
    assert_eq!(app.ui.dialog.as_ref().unwrap().str("colorModel"), "cmyk");
}

#[test]
fn bmp_options_keep_their_choices_writable() {
    let (mut app, written) = app(1);
    app.run("file.exportAs", json!({"format": "bmp"})).unwrap();
    confirm(&mut app).unwrap();
    assert_eq!(kind(&app), Some("bmpOptions"));
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!(
        (d.str("colorModel"), d.str("fileFormat"), d.f64("depth", 0.0), d.bool("rle"), d.bool("flipRows")),
        ("rgb".into(), "windows".into(), 24.0, false, false)
    );
    frame(&mut app);
    // OS/2 has no 32 bits: the depth falls back to 24.
    set(&mut app, "fileFormat", json!("os2"));
    set(&mut app, "depth", json!(32));
    frame(&mut app);
    assert_eq!(app.ui.dialog.as_ref().unwrap().f64("depth", 0.0), 24.0);
    // Compressed rows are bottom-up: RLE turns flipped rows off.
    set(&mut app, "fileFormat", json!("windows"));
    set(&mut app, "depth", json!(8));
    set(&mut app, "flipRows", json!(true));
    set(&mut app, "rle", json!(true));
    frame(&mut app);
    assert!(!app.ui.dialog.as_ref().unwrap().bool("flipRows"));
    confirm(&mut app).unwrap();
    let (path, bmp) = written.borrow()[0].clone();
    assert_eq!(path, "/out/Untitled-1.bmp");
    assert_eq!((&bmp[..2], bmp[28], bmp[30]), (&b"BM"[..], 8, 1), "8-bit RLE8");
}

#[test]
fn targa_options_pick_the_depth() {
    let (mut app, written) = app(1);
    app.run("file.exportAs", json!({"format": "tga"})).unwrap();
    confirm(&mut app).unwrap();
    assert_eq!(kind(&app), Some("tgaOptions"));
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!(((spec(&d.kind).heading)(d), d.f64("depth", 0.0)), ("Targa Options".into(), 24.0));
    assert!(!tiff_bmp_tga::keeps_alpha("tga", d), "24 bits are flattened on white");
    frame(&mut app);
    set(&mut app, "depth", json!(32));
    frame(&mut app);
    assert!(tiff_bmp_tga::keeps_alpha("tga", app.ui.dialog.as_ref().unwrap()));
    confirm(&mut app).unwrap();
    let (path, tga) = written.borrow()[0].clone();
    assert_eq!((path.as_str(), tga[16], tga[17]), ("/out/Untitled-1.tga", 32, 8));
}
