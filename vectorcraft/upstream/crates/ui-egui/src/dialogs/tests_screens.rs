//! Export for Screens, drawn headlessly: the fields it opens on (defaults, then the document's last
//! settings), the artboard choice, Format Settings, desktop folders and web downloads.

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::{Value, json};
use vectorcraft_engine::Session;
use vectorcraft_engine::cmd::fileio;

use super::tests_export::{frame, kind, set};
use super::*;
use crate::Services;

type Log = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

/// An app with `artboards` 60×40 pt artboards and an ellipse on the first; `web` gives it a
/// download service (recorded in the log) instead of a file manager (whose reveals are recorded).
fn app(artboards: usize, web: bool) -> (VectorcraftApp, Log) {
    let log = Log::default();
    let l = log.clone();
    let mut services = Services::default();
    if web {
        services.download = Some(Box::new(move |name: &str, bytes: &[u8]| l.borrow_mut().push((name.to_string(), bytes.to_vec()))));
    } else {
        services.reveal = Some(Box::new(move |path: &str| {
            l.borrow_mut().push((path.to_string(), vec![]));
            Ok(())
        }));
        services.pick_folder = Some(Box::new(|| None));
    }
    let mut app = VectorcraftApp::new(Session::new(), services);
    app.run("file.new", json!({"width": 60, "height": 40, "artboards": artboards})).unwrap();
    app.run("shape.ellipse", json!({"x": 5, "y": 4, "width": 30, "height": 20})).unwrap();
    (app, log)
}

fn field(app: &VectorcraftApp, k: &str) -> Value {
    app.ui.dialog.as_ref().expect("a dialog is open").fields.get(k).cloned().unwrap_or(Value::Null)
}

fn open(app: &mut VectorcraftApp) {
    app.run("file.exportForScreens", json!({})).unwrap();
    assert_eq!(kind(app), Some(export_for_screens::KIND));
}

fn temp_folder(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("vc-efs-{tag}-{}", std::process::id()))
}

#[test]
fn opens_on_defaults_and_draws_every_view() {
    let (mut app, _) = app(3, false);
    open(&mut app);
    assert_eq!((field(&app, "select"), field(&app, "range"), field(&app, "boards")), (json!("all"), json!("1-3"), json!([true, true, true])));
    assert_eq!(field(&app, "formats"), json!([{"format": "png", "scale": "1x", "suffix": ""}]));
    assert_eq!(field(&app, "folder"), json!(fileio::export_folder().unwrap_or_default()), "the Desktop, else home");
    assert_eq!((field(&app, "openLocation"), field(&app, "subfolders"), field(&app, "preset")), (json!(true), json!(false), json!("")));
    let settings = field(&app, "settings");
    for f in ["png", "png8", "jpg", "webp", "gif", "svg", "pdf"] {
        assert!(settings[f].is_object(), "{f} has settings");
    }
    assert!(settings["jpg"].get("quality").is_none() && settings["jpg"].get("imageMap").is_none(), "rows choose quality; no maps");
    assert!(settings["svg"].get("images").is_none(), "no linked images");
    assert_eq!(settings["pdf"]["preset"], json!(fileio::pdf::DEFAULT_PRESET));
    frame(&mut app);
    // Assets, Full Document, each format's Format Settings: all draw.
    set(&mut app, "tab", json!("assets"));
    frame(&mut app);
    set(&mut app, "tab", json!("artboards"));
    set(&mut app, "select", json!("full"));
    for f in ["png", "png8", "jpg", "webp", "gif", "svg", "pdf"] {
        set(&mut app, "__settings", json!(f));
        frame(&mut app);
        assert_eq!(field(&app, "__settings"), json!(f));
    }
    assert_eq!(kind(&app), Some(export_for_screens::KIND));
    assert_eq!((spec(export_for_screens::KIND).ok_label.unwrap())(&app), "Export Artboard");
}

#[test]
fn checked_artboards_read_as_a_range() {
    let (mut app, _) = app(5, false);
    open(&mut app);
    set(&mut app, "boards", json!([true, false, true, true, false]));
    frame(&mut app);
    // Nothing clicked: the fields stay as set.
    assert_eq!(field(&app, "boards"), json!([true, false, true, true, false]));
    for (boards, text) in [
        (vec![true, true, true], "1-3"),
        (vec![false, true, false, true], "2, 4"),
        (vec![true, false, true, true, false, true], "1, 3-4, 6"),
        (vec![false], ""),
    ] {
        assert_eq!(export_for_screens::range_text(&boards), text);
    }
}

#[test]
fn exports_into_the_folder_and_shows_it() {
    let (mut app, log) = app(2, false);
    let dir = temp_folder("desk");
    let folder = dir.to_string_lossy().replace('\\', "/");
    open(&mut app);
    set(&mut app, "folder", json!(folder));
    set(&mut app, "subfolders", json!(true));
    set(
        &mut app,
        "formats",
        json!([{"format": "png", "scale": "1x", "suffix": ""}, {"format": "png", "scale": "120w", "suffix": "@120w"}, {"format": "svg", "scale": "1x", "suffix": ""}]),
    );
    // A bad range keeps the dialog open; so does no folder.
    set(&mut app, "select", json!("range"));
    set(&mut app, "range", json!("7"));
    assert!(confirm(&mut app).is_err());
    assert_eq!(kind(&app), Some(export_for_screens::KIND));
    set(&mut app, "range", json!("2"));
    set(&mut app, "folder", json!(" "));
    assert!(confirm(&mut app).is_err());
    set(&mut app, "folder", json!(folder));
    let r = confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    let files: Vec<&str> = r["files"].as_array().unwrap().iter().map(|f| f.as_str().unwrap()).collect();
    assert_eq!(files, [format!("{folder}/1x/Artboard-2.png"), format!("{folder}/120w/Artboard-2@120w.png"), format!("{folder}/SVG/Artboard-2.svg")]);
    let png = image::load_from_memory(&std::fs::read(dir.join("120w").join("Artboard-2@120w.png")).unwrap()).unwrap();
    assert_eq!((png.width(), png.height()), (120, 80));
    assert_eq!(log.borrow().iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>(), [files[0]], "Open Location shows the first file");
    // It reopens on those settings.
    open(&mut app);
    assert_eq!((field(&app, "select"), field(&app, "range"), field(&app, "boards")), (json!("range"), json!("2"), json!([false, true])));
    assert_eq!(field(&app, "formats")[1]["scale"], json!("120w"));
    assert_eq!((field(&app, "subfolders"), field(&app, "folder")), (json!(true), json!(folder)));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_web_downloads_one_zip_or_the_file() {
    let (mut app, log) = app(2, true);
    assert_eq!((spec(export_for_screens::KIND).ok_label.unwrap())(&app), "Download");
    open(&mut app);
    frame(&mut app);
    set(
        &mut app,
        "formats",
        json!([{"format": "png", "scale": "1x", "suffix": ""}, {"format": "jpg", "quality": 80, "scale": "2x", "suffix": "@2x"}]),
    );
    set(&mut app, "settings", json!({"png": {"background": "black"}}));
    let r = confirm(&mut app).unwrap();
    assert!(r.get("dataBase64").is_some(), "four files: one zip");
    {
        let log = log.borrow();
        let [(name, zip)] = log.as_slice() else { panic!("one download: {:?}", log.iter().map(|(n, _)| n).collect::<Vec<_>>()) };
        assert_eq!(name, "Untitled-1.zip");
        assert_eq!(&vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap(), zip);
    }
    assert_eq!(r["files"], json!(["Artboard-1.png", "Artboard-1@2x.jpg", "Artboard-2.png", "Artboard-2@2x.jpg"]));
    // One file downloads as itself, with the format's settings.
    log.borrow_mut().clear();
    open(&mut app);
    assert_eq!(field(&app, "settings")["png"]["background"], json!("black"), "remembered");
    set(&mut app, "select", json!("range"));
    set(&mut app, "range", json!("1"));
    set(&mut app, "formats", json!([{"format": "png", "scale": "1x", "suffix": ""}]));
    confirm(&mut app).unwrap();
    let log = log.borrow();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].0, "Artboard-1.png");
    assert_eq!(image::load_from_memory(&log[0].1).unwrap().to_rgba8().get_pixel(1, 1).0, [0, 0, 0, 255], "the PNG settings' background");
}

#[test]
fn a_preset_export_reopens_on_its_rows() {
    let (mut app, _) = app(1, true);
    app.run("file.exportForScreens", json!({"preset": "density", "fullDocument": true})).unwrap();
    open(&mut app);
    assert_eq!(field(&app, "preset"), json!("density"));
    assert_eq!(field(&app, "select"), json!("full"));
    assert_eq!(field(&app, "subfolders"), json!(true));
    let rows = field(&app, "formats");
    assert_eq!(rows.as_array().unwrap().len(), 6);
    assert_eq!((rows[0]["scale"].clone(), rows[0]["folder"].clone()), (json!("0.75x"), json!("ldpi")));
    frame(&mut app);
    // Agents' numeric sizes show as text.
    app.run(
        "file.exportForScreens",
        json!({"formats": [{"format": "png", "width": 64}, {"format": "png", "scale": 3}, {"format": "png", "ppi": 144}]}),
    )
    .unwrap();
    open(&mut app);
    let scales: Vec<Value> = field(&app, "formats").as_array().unwrap().iter().map(|r| r["scale"].clone()).collect();
    assert_eq!(scales, [json!("64w"), json!("3x"), json!("144ppi")]);
}
