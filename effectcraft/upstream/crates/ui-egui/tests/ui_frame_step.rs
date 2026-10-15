//! Stepping the current time from the keyboard: Page Down / Page Up and Cmd+Right / Cmd+Left
//! Arrow move one frame, with Shift ten.

use effectcraft_engine::Session;
use effectcraft_ui_egui::EffectcraftApp;
use egui::{Event, Key, Modifiers};
use egui_kittest::Harness;
use serde_json::json;

fn key(h: &mut Harness<'_, EffectcraftApp>, key: Key, modifiers: Modifiers) {
    h.event(Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
    h.step();
    h.event(Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers });
    h.step();
}

#[test]
fn cmd_arrows_step_the_current_frame() {
    let mut session = Session::default();
    session.execute("comp.new", json!({"name": "Main", "width": 320, "height": 180, "frameRate": 30.0, "duration": 4.0})).unwrap();
    let cid = session.state.active_comp.unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(|_| EffectcraftApp::new(session));
    h.run_steps(2);
    let frame = |h: &Harness<'_, EffectcraftApp>| {
        let app = h.state();
        app.session.project.comp(cid).unwrap().frame_rate.frame_at(app.session.time())
    };
    assert_eq!(frame(&h), 0);
    let cmd = Modifiers::COMMAND;
    let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
    key(&mut h, Key::ArrowRight, cmd);
    assert_eq!(frame(&h), 1);
    key(&mut h, Key::PageDown, Modifiers::NONE);
    assert_eq!(frame(&h), 2);
    key(&mut h, Key::ArrowRight, cmd_shift);
    assert_eq!(frame(&h), 12);
    key(&mut h, Key::ArrowLeft, cmd);
    assert_eq!(frame(&h), 11);
    key(&mut h, Key::ArrowLeft, cmd_shift);
    assert_eq!(frame(&h), 1);
    // The plain arrows still nudge layers, not the time.
    key(&mut h, Key::ArrowRight, Modifiers::NONE);
    assert_eq!(frame(&h), 1);
}
