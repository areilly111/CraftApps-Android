//! The paste chords (egui reports every Cmd+V chord as a paste, not as its key) and the default
//! shortcuts (no chord does two things).

use std::collections::BTreeMap;

use egui::{Event, Key, Modifiers};
use serde_json::json;
use vectorcraft_engine::Session;

use crate::{VectorcraftApp, shortcut_editor, shortcuts};

/// One headless frame of the shortcut handler (and the app's logic, which publishes copies) with
/// `modifiers` held and `events`.
fn frame(app: &mut VectorcraftApp, modifiers: Modifiers, events: Vec<Event>) -> egui::FullOutput {
    let events = std::iter::once(Event::ModifiersChanged(modifiers)).chain(events).collect();
    let mut out = egui::Context::default().run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
        shortcuts::handle(app, ui.ctx());
        app.logic(ui.ctx());
    });
    out.textures_delta.clear();
    out
}

fn last_object_bounds(app: &VectorcraftApp) -> (f64, f64) {
    let st = app.session.active().unwrap();
    let id = st.doc.layers[0].children().unwrap().last().unwrap().id;
    let b = st.doc.node(id).unwrap().geometric_bounds().unwrap();
    (b.x0, b.y0)
}

#[test]
fn cmd_shift_v_pastes_in_place() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 10, "y": 20, "width": 30, "height": 30})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    let out = frame(&mut app, Modifiers::COMMAND, vec![Event::Copy]);
    let svg = out
        .platform_output
        .commands
        .iter()
        .find_map(|c| match c {
            egui::OutputCommand::CopyText(t) => Some(t.clone()),
            _ => None,
        })
        .unwrap();
    // The text egui read from the clipboard comes with the chord still held.
    frame(&mut app, Modifiers::COMMAND | Modifiers::SHIFT, vec![Event::Paste(svg.clone())]);
    assert_eq!(app.session.active().unwrap().doc.layers[0].children().unwrap().len(), 2);
    assert_eq!(last_object_bounds(&app), (10.0, 20.0), "Cmd+Shift+V pastes in place");
    frame(&mut app, Modifiers::COMMAND, vec![Event::Paste(svg)]);
    assert_eq!(app.session.active().unwrap().doc.layers[0].children().unwrap().len(), 3);
    assert_ne!(last_object_bounds(&app), (10.0, 20.0), "Cmd+V pastes offset or centred");
}

#[test]
fn a_paste_takes_the_command_of_the_v_chord_held() {
    let cmd = Modifiers::COMMAND;
    assert_eq!(shortcuts::paste_command(cmd), "edit.paste");
    assert_eq!(shortcuts::paste_command(cmd | Modifiers::SHIFT), "edit.pasteInPlace");
    assert_eq!(shortcuts::paste_command(cmd | Modifiers::ALT), "edit.pasteWithoutFormatting");
    assert_eq!(shortcuts::paste_command(cmd | Modifiers::ALT | Modifiers::SHIFT), "edit.pasteOnAllArtboards");
    // The Paste key, and Shift+Insert, paste.
    assert_eq!(shortcuts::paste_command(Modifiers::NONE), "edit.paste");
    assert_eq!(shortcuts::paste_command(Modifiers::SHIFT), "edit.paste");
    // What isn't text: only the key's release is reported, with the chord's modifiers.
    let mut chord = shortcuts::PasteChord::default();
    let release = Event::Key { key: Key::V, physical_key: None, pressed: false, repeat: false, modifiers: cmd | Modifiers::SHIFT };
    assert_eq!(chord.textless_paste(&[release]), Some(cmd | Modifiers::SHIFT));
}

#[test]
fn no_two_commands_tools_or_panels_share_a_default_shortcut() {
    assert_eq!(shortcut_editor::all_conflicts(&BTreeMap::new()), vec![]);
}
