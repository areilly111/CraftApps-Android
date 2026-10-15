//! Headless tests of the Color Grading flyout (issue #737): the 3-Way / Shadows / Midtones /
//! Highlights tabs, the equal-size wheels, and each wheel's eye that hides its effect for viewing.

use std::time::Duration;

use serde_json::json;

use crate::headless::Headless;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);
const SETTLE: Duration = Duration::from_secs(120);

fn demo() -> Headless {
    let services = Services { png: None, ..Default::default() };
    let app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), services);
    let mut h = Headless::new(app, [1400.0, 2400.0], 1.0);
    let r = h.request("ui.set", json!({"view": "detail", "right": "edit"}), T);
    assert_eq!(r["ok"], true, "{r}");
    if !h.app.ui.section_open("color") {
        h.app.ui.toggle_section("color");
    }
    if !h.app.ui.flyout_open("grading") {
        h.app.ui.toggle_flyout("grading");
    }
    h.settle(SETTLE);
    h
}

fn widget(h: &Headless, id: &str) -> Option<egui::Rect> {
    h.app.widgets.iter().find(|(w, _)| w == id).map(|(_, r)| *r)
}

fn click(h: &mut Headless, id: &str) {
    let r = h.request("ui.clickWidget", json!({"id": id}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.step();
    h.step();
}

#[test]
fn three_way_is_the_default_with_three_equal_wheels() {
    let h = demo();
    assert_eq!(h.app.ui.grading_mode, "3way");
    let sizes: Vec<f32> = ["shadows", "midtones", "highlights"].iter().map(|k| widget(&h, &format!("wheel:{k}")).unwrap().width()).collect();
    assert!(sizes.iter().all(|w| (w - sizes[0]).abs() < 0.5), "{sizes:?}");
    for tab in ["3way", "shadows", "midtones", "highlights"] {
        assert!(widget(&h, &format!("icon:grading-{tab}")).is_some(), "tab {tab}");
    }
}

#[test]
fn a_range_tab_shows_one_bigger_wheel_and_its_hue_and_saturation() {
    let mut h = demo();
    let small = widget(&h, "wheel:shadows").unwrap().width();
    click(&mut h, "icon:grading-shadows");
    assert_eq!(h.app.ui.grading_mode, "shadows");
    assert!(widget(&h, "wheel:shadows").unwrap().width() > small);
    assert!(widget(&h, "wheel:midtones").is_none() && widget(&h, "wheel:highlights").is_none());
    for c in ["grading.shadows.hue", "grading.shadows.sat", "grading.shadows.lum", "grading.blending", "grading.balance"] {
        assert!(h.app.widgets.iter().any(|(w, _)| w.contains(c)), "slider {c}");
    }
    assert!(!h.app.widgets.iter().any(|(w, _)| w.contains("grading.midtones.lum")));
}

#[test]
fn the_eye_hides_a_wheel_for_viewing_only_and_resets_on_another_photo() {
    let mut h = demo();
    let id = h.app.session.active().unwrap();
    h.app.session.execute("develop.set", &json!({"values": {"grading.midtones.sat": 50}})).unwrap();
    h.step();
    let r = h.request("ui.hoverWidget", json!({"id": "wheel:midtones"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.step();
    h.step();
    click(&mut h, "wheelEye:midtones");
    assert_eq!(h.app.ui.grading_hidden_for(id), [false, true, false]);
    // not an edit
    assert_eq!(h.app.session.catalog.photo(id).unwrap().develop.grading.midtones.sat, 50.0);
    // the eye stays on screen while the wheel is hidden, and shows it again
    h.request("ui.move", json!({"x": 5.0, "y": 5.0}), T);
    h.step();
    assert!(widget(&h, "wheelEye:midtones").is_some());
    click(&mut h, "wheelEye:midtones");
    assert_eq!(h.app.ui.grading_hidden_for(id), [false; 3]);
    // hidden state belongs to one photo
    h.app.ui.set_grading_hidden(id, 0, true);
    let other = lightcraft_catalog::PhotoId(id.0 + 1);
    assert_eq!(h.app.ui.grading_hidden_for(other), [false; 3]);
}

#[test]
fn hovering_a_three_way_wheel_shows_its_hue_and_saturation() {
    let mut h = demo();
    h.app.session.execute("develop.set", &json!({"values": {"grading.shadows.hue": 220, "grading.shadows.sat": 45}})).unwrap();
    h.step();
    let readout = "H 220°  S 45";
    assert!(!h.painted_text().iter().any(|t| t == readout), "hidden until hovered");
    let r = h.request("ui.hoverWidget", json!({"id": "wheel:shadows"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.step();
    h.step();
    assert!(h.painted_text().iter().any(|t| t == readout), "{:?}", h.painted_text());
}

#[test]
fn shift_drag_keeps_the_hue_and_changes_only_saturation() {
    use crate::panels::edit::wheel_pick;
    let close = |a: f32, b: f32| (a - b).abs() < 0.01;
    // free: hue from the angle (screen y points down), saturation from the distance
    let (h, s) = wheel_pick(egui::vec2(0.0, -20.0), 40.0, None);
    assert!(close(h, 90.0) && close(s, 50.0), "{h} {s}");
    // locked at 0°: a rough drag up and right keeps the hue; only the part along the spoke counts
    let (h, s) = wheel_pick(egui::vec2(30.0, -12.0), 40.0, Some(0.0));
    assert!(close(h, 0.0) && close(s, 75.0), "{h} {s}");
    // past the centre or the rim: clamped to 0..100
    assert_eq!(wheel_pick(egui::vec2(-10.0, 0.0), 40.0, Some(0.0)).1, 0.0);
    assert_eq!(wheel_pick(egui::vec2(90.0, 0.0), 40.0, Some(0.0)).1, 100.0);
    // a degenerate radius doesn't divide by zero
    assert!(wheel_pick(egui::vec2(1.0, 1.0), 0.0, None).1.is_finite());
}
