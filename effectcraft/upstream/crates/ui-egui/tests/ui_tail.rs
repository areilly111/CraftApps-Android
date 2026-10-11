//! Headless checks for the M13.5 long-tail UI: horizontal scrolling of the Timeline outline and
//! Project panel columns (egui_kittest, UI logic only).

use effectcraft_engine::Session;
use effectcraft_ui_egui::EffectcraftApp;
use egui::{Event, Pos2, Rect, pos2, vec2};
use egui_kittest::Harness;
use serde_json::json;

fn harness(w: f32) -> Harness<'static, EffectcraftApp> {
    let mut s = Session::default();
    s.execute("comp.new", json!({"name": "Tail", "width": 640, "height": 360, "duration": 4})).unwrap();
    s.execute("layer.newSolid", json!({"name": "Plate", "color": "#406080", "width": 640, "height": 360})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(w, 900.0)).build_eframe(|_| EffectcraftApp::new(s));
    h.run_steps(3);
    h
}

fn rect(h: &Harness<'_, EffectcraftApp>, id: &str) -> Rect {
    let e = h.state().auto.find(id).unwrap_or_else(|| panic!("no {id}"));
    Rect::from_min_size(pos2(e.rect[0], e.rect[1]), vec2(e.rect[2], e.rect[3]))
}

fn drag(h: &mut Harness<'_, EffectcraftApp>, from: Pos2, to: Pos2) {
    h.input_mut().events.push(Event::PointerMoved(from));
    h.input_mut().events.push(Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.step();
    for i in 1..=8 {
        h.input_mut().events.push(Event::PointerMoved(from + (to - from) * (i as f32 / 8.0)));
        h.step();
    }
    h.input_mut().events.push(Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(2);
}

#[test]
fn timeline_outline_columns_scroll_horizontally() {
    let mut h = harness(1100.0);
    // Every column on: wider than the outline pane.
    let ctx = h.ctx.clone();
    for c in ["keys", "comment", "modes", "parent", "in", "out", "duration", "stretch"] {
        effectcraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "timeline.column", json!({"column": c, "visible": true})).unwrap();
    }
    h.run_steps(3);
    let bar = rect(&h, "timeline.outlineScroll");
    let name0 = rect(&h, "timeline.header.name").min.x;
    assert_eq!(h.state().ui.timeline.outline_scroll, 0.0);
    drag(&mut h, pos2(bar.min.x + 8.0, bar.center().y), pos2(bar.max.x + 50.0, bar.center().y));
    h.run_steps(2);
    let s = h.state().ui.timeline.outline_scroll;
    assert!(s > 50.0, "{s}");
    let name1 = rect(&h, "timeline.header.name").min.x;
    assert!((name0 - name1 - s).abs() < 1.0, "the columns moved by the scroll: {name0} → {name1} ({s})");
    // The scroll is clamped to the overflow and resets when the columns fit again.
    for c in ["keys", "comment", "modes", "parent", "in", "out", "duration", "stretch"] {
        effectcraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "timeline.column", json!({"column": c, "visible": false})).unwrap();
    }
    h.run_steps(3);
    assert_eq!(h.state().ui.timeline.outline_scroll, 0.0);
    assert!(h.state().auto.find("timeline.outlineScroll").is_none());
}

fn styles(h: &mut Harness<'_, EffectcraftApp>) -> Vec<String> {
    let v = h.state_mut().session.execute("layer.style.list", json!({})).unwrap();
    v["styles"].as_array().unwrap().iter().map(|s| s["style"].as_str().unwrap().to_string()).collect()
}

#[test]
fn layer_style_dialog_adds_previews_and_cancels() {
    let mut h = harness(1400.0);
    let lid = h.state().session.active_comp().unwrap().layers[0].id.0;
    h.state_mut().session.execute("layer.select", json!({"layers": [lid]})).unwrap();
    let ctx = h.ctx.clone();
    // Layer ▸ Layer Styles ▸ Drop Shadow (from the menu): adds the style and opens the dialog on it.
    effectcraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "layer.style.dropShadow", json!({})).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().dialog, Some(effectcraft_ui_egui::Dialog::LayerStyles));
    assert_eq!(styles(&mut h), ["dropShadow"]);
    assert!(h.state().auto.find("dialog.layerStyle.prop.dropShadow/distance").is_some(), "the style's controls are shown");
    // Live preview: the controls edit the real property (here through the same command).
    h.state_mut().session.execute("prop.set", json!({"layer": lid, "path": "layerStyles/dropShadow/distance", "value": 25, "merge": "x"})).unwrap();
    // Pick Stroke in the list (its page: not applied), then Cancel: everything done in the
    // dialog is rolled back. (Dialog buttons are driven through the dialog's functions:
    // kittest pointer events don't reach Foreground areas here.)
    effectcraft_ui_egui::panels::layer_styles_dialog::select(h.state_mut(), "stroke");
    h.run_steps(2);
    assert_eq!(h.state().auto.find("dialog.layerStyle.page").unwrap().label, "stroke");
    assert!(h.state().auto.find("dialog.layerStyle.prop.dropShadow/distance").is_none());
    assert!(h.state().auto.find("dialog.layerStyle.cancel").is_some());
    effectcraft_ui_egui::panels::layer_styles_dialog::finish(h.state_mut(), false);
    h.run_steps(2);
    assert!(h.state().dialog.is_none());
    assert!(styles(&mut h).is_empty(), "Cancel removes the style added from the menu");
    // Layer Style Options… on Stroke, add it (the list check box runs layer.style.add), OK keeps it.
    effectcraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "layer.style.options", json!({"style": "stroke"})).unwrap();
    h.run_steps(3);
    assert!(h.state().auto.find("dialog.layerStyle.prop.stroke/size").is_none(), "not applied yet");
    h.state_mut().session.execute("layer.style.add", json!({"style": "stroke"})).unwrap();
    h.run_steps(3);
    assert!(h.state().auto.find("dialog.layerStyle.prop.stroke/size").is_some(), "its controls appear");
    effectcraft_ui_egui::panels::layer_styles_dialog::finish(h.state_mut(), true);
    assert!(h.state().dialog.is_none());
    assert_eq!(styles(&mut h), ["stroke"]);
}

#[test]
fn project_panel_columns_scroll_horizontally() {
    let mut h = harness(1400.0);
    for c in ["type", "size", "duration", "fps", "path", "comment"] {
        effectcraft_ui_egui::panels::project::set_column(&mut h.state_mut().ui.project_columns, c, true);
    }
    h.run_steps(3);
    let bar = rect(&h, "project.hscroll");
    let x0 = rect(&h, "project.sort.comment").min.x;
    let name0 = rect(&h, "project.sort.name").min.x;
    drag(&mut h, pos2(bar.min.x + 4.0, bar.center().y), pos2(bar.max.x + 40.0, bar.center().y));
    h.run_steps(2);
    let s = h.state().ui.project_hscroll;
    assert!(s > 20.0, "{s}");
    assert!(rect(&h, "project.sort.comment").min.x < x0 - 20.0);
    assert_eq!(rect(&h, "project.sort.name").min.x, name0, "Name stays frozen");
}
