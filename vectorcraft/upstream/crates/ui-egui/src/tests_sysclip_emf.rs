//! Windows metafiles another app copied paste as vector art (M4.95, through the system clipboard
//! service of M4.57).

use serde_json::json;
use vectorcraft_doc::NodeKind;
use vectorcraft_engine::Session;
use vectorcraft_engine::cmd::clipboard::{EMF, PNG};

use crate::sysclip::import_command;
use crate::tests_sysclip::{app, blue_png, copy_elsewhere, pasted, run};

#[test]
fn an_emf_from_another_app_pastes_as_vectors_ahead_of_its_bitmap() {
    let (mut app, board) = app();
    let mut other = Session::new();
    other.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    other.execute("shape.ellipse", &json!({"x": 10, "y": 10, "width": 30, "height": 30})).unwrap();
    let emf = other.execute("document.export", &json!({"format": "emf"})).unwrap();
    let emf = vectorcraft_format::base64_decode(emf["dataBase64"].as_str().unwrap()).unwrap();
    let flavour = vectorcraft_engine::cmd::clipboard::Flavour { mime: EMF, data: emf.clone() };
    assert_eq!(import_command(&flavour, None).0, "clipboard.importEmf");
    copy_elsewhere(&board, vec![(PNG, blue_png()), (EMF, emf)]);
    run(&mut app, "edit.paste", json!({}));
    let p = pasted(&app);
    assert!(!p.is_empty() && p.iter().all(|n| !matches!(n.kind, NodeKind::Image(_))), "{p:?}");
    let b = p[0].geometric_bounds().unwrap();
    assert!((b.width() - 30.0).abs() < 0.05, "{b:?}");
}
