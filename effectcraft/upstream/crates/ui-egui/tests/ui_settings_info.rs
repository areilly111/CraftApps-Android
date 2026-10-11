//! Settings remain a single transaction while GPU Information is open.
use std::sync::Arc;

use effectcraft_engine::{Session, config::MemoryConfig};
use effectcraft_ui_egui::{Dialog, EffectcraftApp, menus};
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, pos2, vec2};
use egui_kittest::{Harness, kittest::Queryable as _};
use serde_json::json;

fn rect(h: &Harness<'_, EffectcraftApp>, id: &str) -> Rect {
    let e = h.state().auto.find(id).unwrap_or_else(|| panic!("missing {id}"));
    let r = Rect::from_min_size(pos2(e.rect[0], e.rect[1]), vec2(e.rect[2], e.rect[3]));
    assert!(h.ctx.content_rect().contains_rect(r), "visible {id}: {r:?}");
    r
}

fn click(h: &mut Harness<'_, EffectcraftApp>, at: Pos2) {
    h.event(Event::PointerMoved(at));
    h.step();
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
}

fn saved(store: &Arc<MemoryConfig>) -> Session {
    let mut session = Session { config: Some(store.clone()), ..Default::default() };
    session.prefs.disk.disk_cache_enabled = false;
    session.load_settings();
    session
}

fn check_info_transaction(via_escape: bool) {
    let store = Arc::new(MemoryConfig::default());
    let mut session = Session { config: Some(store.clone()), ..Default::default() };
    session.prefs.disk.disk_cache_enabled = false;
    session.save_prefs();
    session.execute("comp.new", json!({"width":32,"height":32,"duration":2})).unwrap();
    let original_brightness = session.prefs.appearance.brightness;
    let mut h = Harness::builder().with_size(vec2(1600.0, 1000.0)).build_eframe(|_| EffectcraftApp::new(session));
    h.run_steps(4);
    let ctx = h.ctx.clone();
    menus::invoke(h.state_mut(), &ctx, "app.settings", json!({"page":"appearance"})).unwrap();
    h.run_steps(3);
    let brightness = rect(&h, "settings.appearance.brightness");
    click(&mut h, brightness.center() + vec2(brightness.width() * 0.25, 0.0));
    let edited_brightness = h.state().session.prefs.appearance.brightness;
    assert!((edited_brightness - original_brightness).abs() > 0.01, "real brightness slider changed");
    assert_eq!(saved(&store).prefs.appearance.brightness, original_brightness, "unconfirmed edit is not saved");
    // Use the upstream percent-valued Scale choice; keyboard zoom is deliberately disabled.
    let at = rect(&h, "settings.appearance.uiScale").center();
    click(&mut h, at);
    let at = h.get_by_label("125%").rect().center();
    click(&mut h, at);
    assert_eq!(h.state().session.prefs.appearance.ui_scale, 125);
    assert_eq!(saved(&store).prefs.appearance.ui_scale, 100, "unconfirmed scale edit is not saved");
    // Let UI Scale settle the centered dialog before reading its next hit target.
    h.run_steps(2);
    let at = rect(&h, "settings.page.previews").center();
    click(&mut h, at);
    let at = rect(&h, "settings.button.GPU Information").center();
    click(&mut h, at);
    assert_eq!(h.state().dialog, Some(Dialog::Info), "real Settings button opened GPU Information");
    assert!(h.query_by_label("GPU Information").is_some(), "rendered GPU Information dialog");
    assert_eq!(h.state().session.prefs.appearance.ui_scale, 125);
    assert_eq!(saved(&store).prefs.appearance.ui_scale, 100, "Info preserves the unconfirmed Settings transaction");
    if via_escape {
        for pressed in [true, false] {
            h.event(Event::Key { key: Key::Escape, physical_key: Some(Key::Escape), pressed, repeat: false, modifiers: Modifiers::NONE });
            h.step();
        }
        h.run_steps(3);
    } else {
        let at = h.get_by_label_contains("OK").rect().center();
        click(&mut h, at);
    }
    eprintln!(
        "after Info dismissal: dialog={:?}, live brightness={}, live scale={}",
        h.state().dialog,
        h.state().session.prefs.appearance.brightness,
        h.state().session.prefs.appearance.ui_scale
    );
    assert_eq!(h.state().dialog, Some(Dialog::Settings), "dismissing GPU Information returns to the pending Settings transaction");
    assert_eq!(h.state().session.prefs.appearance.brightness, edited_brightness);
    if via_escape {
        let at = rect(&h, "settings.ok").center();
        click(&mut h, at);
        let reopened = saved(&store);
        assert_eq!(reopened.prefs.appearance.brightness, edited_brightness, "Settings OK saves the earlier live edit");
        assert_eq!(reopened.prefs.appearance.ui_scale, 125);
    } else {
        let at = rect(&h, "settings.cancel").center();
        click(&mut h, at);
        assert_eq!(h.state().session.prefs.appearance.brightness, original_brightness);
        assert_eq!(h.ctx.zoom_factor(), 1.0);
        assert_eq!(saved(&store).prefs.appearance.brightness, original_brightness);
        assert_eq!(saved(&store).prefs.appearance.ui_scale, 100);
        h.state_mut().set_pref("appearance.uiScale", json!(150)).unwrap();
        h.run_steps(3);
        assert_eq!(saved(&store).prefs.appearance.ui_scale, 150, "preference edits outside Settings persist normally");
    }
    assert!(h.state().dialog.is_none());
}

#[test]
fn gpu_information_ok_returns_to_settings_and_cancel_restores_preferences() {
    check_info_transaction(false);
}

#[test]
fn gpu_information_escape_returns_to_settings_and_ok_saves_preferences() {
    check_info_transaction(true);
}
