//! Headless checks for on-canvas text editing in the viewer and the Character / Paragraph panels
//! acting on the selection (egui_kittest, UI logic only: no GPU needed).

use effectcraft_engine::Session;
use effectcraft_engine::commands::text_edit::layer_doc;
use effectcraft_engine::project::LayerId;
use effectcraft_ui_egui::EffectcraftApp;
use effectcraft_ui_egui::dock::PanelKind;
use egui::{Event, Key, Modifiers, PointerButton, Pos2};
use egui_kittest::Harness;
use serde_json::json;

fn app() -> EffectcraftApp {
    let mut s = Session::default();
    s.execute("comp.new", json!({"name": "Type", "width": 640, "height": 360, "duration": 2})).unwrap();
    EffectcraftApp::new(s)
}

fn rect(h: &Harness<'_, EffectcraftApp>, id: &str) -> egui::Rect {
    let e = h.state().auto.find(id).unwrap_or_else(|| panic!("no {id}"));
    egui::Rect::from_min_size(egui::pos2(e.rect[0], e.rect[1]), egui::vec2(e.rect[2], e.rect[3]))
}

fn click(h: &mut Harness<'_, EffectcraftApp>, p: Pos2, modifiers: Modifiers) {
    h.input_mut().events.push(Event::PointerMoved(p));
    h.input_mut().events.push(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers });
    h.run_steps(1);
    h.input_mut().events.push(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers });
    h.run_steps(2);
}

fn drag(h: &mut Harness<'_, EffectcraftApp>, from: Pos2, to: Pos2) {
    h.input_mut().events.push(Event::PointerMoved(from));
    h.input_mut().events.push(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    for i in 1..=8 {
        h.input_mut().events.push(Event::PointerMoved(from + (to - from) * (i as f32 / 8.0)));
        h.run_steps(1);
    }
    h.input_mut().events.push(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

fn key(h: &mut Harness<'_, EffectcraftApp>, key: Key, modifiers: Modifiers) {
    h.input_mut().events.push(Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
    h.input_mut().events.push(Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers });
    h.run_steps(2);
}

fn type_text(h: &mut Harness<'_, EffectcraftApp>, t: &str) {
    h.input_mut().events.push(Event::Text(t.into()));
    h.run_steps(2);
}

fn edited(h: &Harness<'_, EffectcraftApp>) -> (u64, String, usize, usize) {
    let s = &h.state().session;
    let e = s.state.text_edit.clone().expect("editing");
    (e.layer.0, layer_doc(s, e.layer).unwrap().text, e.anchor, e.caret)
}

#[test]
fn type_tool_skips_hidden_and_unsoloed_text_layers() {
    for switch in ["video", "solo"] {
        let mut s = Session::default();
        s.execute("comp.new", json!({"name": "Type", "width": 640, "height": 360, "duration": 2})).unwrap();
        let mut ids = vec![];
        for _ in 0..2 {
            let lid = s.execute("layer.newText", json!({"text": "alpha beta", "size": 80, "position": [100, 200], "justify": "left"})).unwrap()["layer"]
                .as_u64()
                .unwrap();
            ids.push(lid);
        }
        let (target, value) = if switch == "video" { (ids[1], false) } else { (ids[0], true) };
        s.execute("layer.setSwitch", json!({"layers": [target], "switch": switch, "value": value})).unwrap();
        s.execute("edit.deselectAll", json!({})).unwrap();
        let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).build_eframe(move |_| EffectcraftApp::new(s));
        h.state_mut().ui.tool = effectcraft_ui_egui::state::Tool::Type;
        h.run_steps(3);
        let comp = rect(&h, "viewer.comp");
        let k = comp.width() / 640.0;
        click(&mut h, comp.min + egui::vec2(330.0 * k, 180.0 * k), Modifiers::NONE);
        assert_eq!(edited(&h).0, ids[0], "{switch}");
    }
}

#[test]
fn modal_blocks_background_text_events() {
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).build_eframe(|_| app());
    h.run_steps(3);
    h.state_mut().ui.tool = effectcraft_ui_egui::state::Tool::Type;
    h.run_steps(1);
    let p = rect(&h, "viewer.comp").center();
    click(&mut h, p, Modifiers::NONE);
    type_text(&mut h, "Preserve");
    let before = edited(&h).1;
    h.state_mut().dialog = Some(effectcraft_ui_egui::Dialog::About);
    h.run_steps(3);
    h.input_mut().events.push(Event::Text("changed".into()));
    h.input_mut().events.push(Event::Paste("pasted".into()));
    key(&mut h, Key::Backspace, Modifiers::NONE);
    assert_eq!(edited(&h).1, before);
}

#[test]
fn type_tool_click_type_edit_and_commit() {
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).with_step_dt(1.0 / 60.0).build_eframe(|_| app());
    h.state_mut().show_panel(PanelKind::Composition);
    h.run_steps(3);
    h.state_mut().ui.tool = effectcraft_ui_egui::state::Tool::Type;
    h.run_steps(1);
    let comp = rect(&h, "viewer.comp");
    // Click: a point text layer in edit mode, caret ready.
    click(&mut h, comp.center(), Modifiers::NONE);
    assert_eq!(h.state().session.active_comp().unwrap().layers.len(), 1);
    type_text(&mut h, "Hello");
    type_text(&mut h, " world");
    let (lid, text, a, c) = edited(&h);
    assert_eq!((text.as_str(), a, c), ("Hello world", 11, 11));
    assert!(h.state().auto.find("viewer.textEdit").is_some(), "edit region registered");
    // Arrow keys, Shift-extend, Backspace.
    key(&mut h, Key::ArrowLeft, Modifiers::NONE);
    key(&mut h, Key::ArrowLeft, Modifiers::SHIFT);
    assert_eq!((edited(&h).2, edited(&h).3), (10, 9));
    key(&mut h, Key::Backspace, Modifiers::NONE);
    assert_eq!(edited(&h).1, "Hello word");
    // Word jump and Home / End.
    key(&mut h, Key::Home, Modifiers::NONE);
    assert_eq!(edited(&h).3, 0);
    key(&mut h, Key::ArrowRight, Modifiers::COMMAND);
    assert_eq!(edited(&h).3, 5);
    key(&mut h, Key::End, Modifiers::NONE);
    assert_eq!(edited(&h).3, 10);
    // Enter starts a new paragraph.
    key(&mut h, Key::Enter, Modifiers::NONE);
    type_text(&mut h, "two");
    assert_eq!(edited(&h).1, "Hello word\ntwo");
    // Select all, then the Character panel styles the selection only.
    key(&mut h, Key::ArrowUp, Modifiers::NONE);
    assert!(edited(&h).3 <= 10, "up to the first line");
    key(&mut h, Key::Home, Modifiers::NONE);
    key(&mut h, Key::End, Modifiers::SHIFT);
    let (_, _, a, c) = edited(&h);
    assert_eq!((a, c), (0, 10));
    h.state_mut().show_panel(PanelKind::Character);
    h.run_steps(3);
    let bold = rect(&h, "character.fauxBold");
    click(&mut h, bold.center(), Modifiers::NONE);
    let doc = layer_doc(&h.state().session, LayerId(lid)).unwrap();
    assert!(doc.style_at(2).faux_bold, "selection is bold");
    assert!(!doc.style_at(12).faux_bold, "second paragraph untouched");
    assert_eq!(doc.runs().len(), 2);
    // Undo the style; Escape commits.
    h.state_mut().session.execute("edit.undo", json!({})).unwrap();
    assert!(layer_doc(&h.state().session, LayerId(lid)).unwrap().is_uniform());
    h.state_mut().show_panel(PanelKind::Composition);
    h.run_steps(3);
    h.state_mut().session.execute("text.setSelection", json!({"start": 0})).unwrap();
    h.run_steps(2);
    key(&mut h, Key::Escape, Modifiers::NONE);
    assert!(h.state().session.state.text_edit.is_none());
}

#[test]
fn switching_from_type_to_selection_commits_and_moves_text_layer() {
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).with_step_dt(1.0 / 60.0).build_eframe(|_| app());
    h.state_mut().show_panel(PanelKind::Composition);
    h.run_steps(3);
    h.state_mut().ui.tool = effectcraft_ui_egui::state::Tool::Type;
    h.run_steps(1);
    let comp = rect(&h, "viewer.comp");
    let k = comp.width() / 640.0;
    click(&mut h, comp.min + egui::vec2(320.0 * k, 180.0 * k), Modifiers::NONE);
    type_text(&mut h, "Move me");
    let layer_id = h.state().session.state.text_edit.as_ref().unwrap().layer;
    let layer_position = |h: &Harness<'_, EffectcraftApp>| {
        h.state().session.active_comp().unwrap().layer(layer_id).unwrap().props.prop("transform/position").unwrap().value.as_vec3()
    };
    let original_text = layer_doc(&h.state().session, layer_id).unwrap().text;
    let before = layer_position(&h);

    h.state_mut().ui.tool = effectcraft_ui_egui::state::Tool::Selection;
    h.run_steps(2);
    assert!(h.state().session.state.text_edit.is_none(), "leaving the Type tool commits text editing");
    type_text(&mut h, " ignored");
    assert_eq!(layer_doc(&h.state().session, layer_id).unwrap().text, original_text, "Selection does not type into the layer");
    let from = comp.min + egui::vec2(340.0 * k, 150.0 * k);
    let to = from + egui::vec2(60.0 * k, 50.0 * k);
    drag(&mut h, from, to);
    let after = layer_position(&h);
    assert!(after[0] > before[0] && after[1] > before[1], "Selection dragging moves the text layer: {before:?} -> {after:?}");
}

#[test]
fn double_click_enters_editing_and_selects_words() {
    let mut s = Session::default();
    s.execute("comp.new", json!({"name": "Type", "width": 640, "height": 360, "duration": 2})).unwrap();
    let lid =
        s.execute("layer.newText", json!({"text": "alpha beta", "size": 80, "position": [100, 200], "justify": "left"})).unwrap()["layer"].as_u64().unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).with_step_dt(1.0 / 60.0).build_eframe(move |_| EffectcraftApp::new(s));
    h.state_mut().show_panel(PanelKind::Composition);
    h.run_steps(3);
    // A point inside "beta" (comp space → window).
    let comp = rect(&h, "viewer.comp");
    let k = comp.width() / 640.0;
    let p = comp.min + egui::vec2(330.0 * k, 180.0 * k);
    click(&mut h, p, Modifiers::NONE);
    click(&mut h, p, Modifiers::NONE);
    let e = h.state().session.state.text_edit.clone().expect("double-click edits text");
    assert_eq!(e.layer.0, lid);
    assert_eq!((e.anchor.min(e.caret), e.anchor.max(e.caret)), (0, 10), "everything selected");
    // Double-clicking inside the edited text selects a word.
    h.run_steps(40);
    click(&mut h, p, Modifiers::NONE);
    click(&mut h, p, Modifiers::NONE);
    let e = h.state().session.state.text_edit.clone().unwrap();
    assert_eq!((e.anchor, e.caret), (6, 10));
    // Typing replaces it; Cmd+Z undoes while editing.
    type_text(&mut h, "gamma");
    assert_eq!(edited(&h).1, "alpha gamma");
    key(&mut h, Key::Z, Modifiers::COMMAND);
    assert_eq!(edited(&h).1, "alpha beta");
    // Paragraph panel: alignment applies to the edited paragraph (the South Asian and Middle
    // Eastern text engine shows the direction popup too).
    h.state_mut().session.prefs.type_.text_engine = "southAsian".into();
    h.state_mut().show_panel(PanelKind::Paragraph);
    h.run_steps(3);
    let r = rect(&h, "paragraph.right");
    click(&mut h, r.center(), Modifiers::NONE);
    let doc = layer_doc(&h.state().session, LayerId(lid)).unwrap();
    assert_eq!(doc.justify, effectcraft_engine::keyframe::Justify::Right);
    for id in
        ["paragraph.indentLeft", "paragraph.spaceBefore", "paragraph.direction", "paragraph.composer", "paragraph.hangingPunctuation", "paragraph.justifyAll"]
    {
        assert!(h.state().auto.find(id).is_some(), "missing {id}");
    }
}

#[test]
fn type_tool_drag_makes_paragraph_text_with_box_handles() {
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).with_step_dt(1.0 / 60.0).build_eframe(|_| app());
    h.state_mut().show_panel(PanelKind::Composition);
    h.run_steps(3);
    h.state_mut().ui.tool = effectcraft_ui_egui::state::Tool::Type;
    h.run_steps(1);
    let comp = rect(&h, "viewer.comp");
    let a = comp.min + egui::vec2(comp.width() * 0.2, comp.height() * 0.2);
    let b = comp.min + egui::vec2(comp.width() * 0.7, comp.height() * 0.6);
    h.input_mut().events.push(Event::PointerMoved(a));
    h.input_mut().events.push(Event::PointerButton { pos: a, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    for i in 1..=8 {
        h.input_mut().events.push(Event::PointerMoved(a + (b - a) * (i as f32 / 8.0)));
        h.run_steps(1);
    }
    h.input_mut().events.push(Event::PointerButton { pos: b, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    let (lid, _, _, _) = edited(&h);
    let doc = layer_doc(&h.state().session, LayerId(lid)).unwrap();
    let bs = doc.box_size.expect("paragraph text");
    assert!((bs[0] - 320.0).abs() < 4.0 && (bs[1] - 144.0).abs() < 4.0, "{bs:?}");
    type_text(&mut h, "Some words that wrap inside the box when there are enough of them");
    h.run_steps(2);
    assert!(h.state().auto.find("viewer.textBox.handle.4").is_some());
    // Dragging the bottom-right handle widens the box.
    let hr = rect(&h, "viewer.textBox.handle.4").center();
    let to = hr + egui::vec2(60.0, 0.0);
    h.input_mut().events.push(Event::PointerMoved(hr));
    h.input_mut().events.push(Event::PointerButton { pos: hr, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    for i in 1..=6 {
        h.input_mut().events.push(Event::PointerMoved(hr + (to - hr) * (i as f32 / 6.0)));
        h.run_steps(1);
    }
    h.input_mut().events.push(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    let doc2 = layer_doc(&h.state().session, LayerId(lid)).unwrap();
    assert!(doc2.box_size.unwrap()[0] > bs[0] + 20.0, "{:?}", doc2.box_size);
    assert!(h.state().session.state.text_edit.is_some(), "still editing after resizing");
}
