//! The Layers panel New Layer button: a click makes a layer at once, ⌥-click opens the New Layer
//! dialog first, as in Photoshop.

use egui::Modifiers;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::PhotocraftApp;

fn panel() -> egui_kittest::Harness<'static, PhotocraftApp> {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(s, crate::Services::default())
    });
    h.run_steps(4);
    h
}

fn layers(h: &egui_kittest::Harness<'static, PhotocraftApp>) -> usize {
    h.state().session.active().unwrap().doc.layers.len()
}

#[test]
fn click_makes_a_layer_without_asking() {
    let mut h = panel();
    h.get_by_label("Create a new layer").click();
    h.run_steps(4);
    assert_eq!(layers(&h), 2);
    assert!(!h.state().ui.dialogs.iter().any(|d| crate::new_layer_ui::owns(&d.fields)));
}

#[test]
fn alt_click_opens_the_new_layer_dialog() {
    let mut h = panel();
    h.get_by_label("Create a new layer").click_modifiers(Modifiers::ALT);
    h.run_steps(4);
    assert_eq!(layers(&h), 1, "nothing made until OK");
    let id = h.state().ui.dialogs.iter().find(|d| crate::new_layer_ui::owns(&d.fields)).map(|d| d.id).expect("the New Layer dialog");
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    assert_eq!(layers(&h), 2);
}
