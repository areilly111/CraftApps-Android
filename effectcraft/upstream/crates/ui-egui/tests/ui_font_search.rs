//! Real keyboard/pointer coverage for the font menus in both text panels.
use effectcraft_engine::{Session, commands::text_edit::layer_doc, project::LayerId};
use effectcraft_ui_egui::{EffectcraftApp, dock::PanelKind};
use egui::{Event, Key, Modifiers, PointerButton};
use egui_kittest::Harness;
use serde_json::json;

fn click(h: &mut Harness<'_, EffectcraftApp>, id: &str) {
    let e = h.state().auto.find(id).unwrap_or_else(|| panic!("missing {id}")).clone();
    let pos = egui::pos2(e.rect[0] + e.rect[2] / 2.0, e.rect[1] + e.rect[3] / 2.0);
    h.input_mut().events.push(Event::PointerMoved(pos));
    for pressed in [true, false] {
        h.input_mut().events.push(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(2);
    }
}

fn key(h: &mut Harness<'_, EffectcraftApp>, key: Key) {
    for pressed in [true, false] {
        h.input_mut().events.push(Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE });
    }
    h.run_steps(3);
}

fn type_text(h: &mut Harness<'_, EffectcraftApp>, text: &str) {
    h.input_mut().events.push(Event::Text(text.into()));
    h.run_steps(3);
}

#[test]
fn font_search_filters_selects_and_undoes_in_both_panels() {
    for (panel, prefix) in [(PanelKind::Character, "character.font"), (PanelKind::Properties, "properties.text.font")] {
        let mut s = Session::default();
        s.execute("comp.new", json!({"width": 320, "height": 180})).unwrap();
        let lid = s.execute("layer.newText", json!({"text": "Keep this text", "font": "Inter"})).unwrap()["layer"].as_u64().unwrap();
        let builder = Harness::builder().with_size(egui::vec2(1600.0, 1000.0));
        let mut h = if std::env::var_os("EC_SNAPSHOT_DIR").is_some() {
            builder.wgpu().build_eframe(move |_| EffectcraftApp::new(s))
        } else {
            builder.build_eframe(move |_| EffectcraftApp::new(s))
        };
        h.state_mut().show_panel(panel);
        h.run_steps(3);
        click(&mut h, prefix);
        assert!(h.state().auto.find(&format!("{prefix}.search")).is_some());
        assert!(effectcraft_ui_egui::widgets::any_menu_open(&h.ctx), "font search must hold app shortcuts like other popup lists");
        let field = h.state().auto.find(prefix).unwrap();
        let search = h.state().auto.find(&format!("{prefix}.search")).unwrap();
        assert!(
            (field.rect[1] + field.rect[3] / 2.0 - search.rect[1] - search.rect[3] / 2.0).abs() < 1.0,
            "font selection and search must occupy the same row"
        );
        // Search gets focus on open. Mixed case and surrounding whitespace are accepted.
        type_text(&mut h, "  jEtBrAiNs  ");
        let rows = h.state().auto.query(&format!("{prefix}.option."));
        assert!(!rows.is_empty());
        assert!(rows.iter().all(|r| r.label.to_lowercase().contains("jetbrains")));
        if let Ok(dir) = std::env::var("EC_SNAPSHOT_DIR") {
            h.render().unwrap().save(format!("{dir}/{prefix}-search.png")).unwrap();
        }
        assert_eq!(layer_doc(&h.state().session, LayerId(lid)).unwrap().font, "Inter", "typing only filters");
        key(&mut h, Key::Enter);
        assert_eq!(layer_doc(&h.state().session, LayerId(lid)).unwrap().font, "JetBrains Mono");
        assert_eq!(layer_doc(&h.state().session, LayerId(lid)).unwrap().text, "Keep this text");
        h.state_mut().session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(layer_doc(&h.state().session, LayerId(lid)).unwrap().font, "Inter");
        click(&mut h, prefix);
        type_text(&mut h, "no_such_font_123456789");
        assert!(h.state().auto.query(&format!("{prefix}.option.")).is_empty());
        assert!(h.state().auto.find(&format!("{prefix}.empty")).is_some());
        key(&mut h, Key::Enter);
        assert!(h.state().auto.find(&format!("{prefix}.search")).is_some(), "no result keeps menu open");
        key(&mut h, Key::Escape);
        assert_eq!(layer_doc(&h.state().session, LayerId(lid)).unwrap().font, "Inter");
        click(&mut h, prefix);
        // Reopening clears the previous query, and mouse selection still works.
        h.run_steps(2);
        if let Ok(dir) = std::env::var("EC_SNAPSHOT_DIR") {
            h.render().unwrap().save(format!("{dir}/{prefix}-reopened.png")).unwrap();
        }
        assert!(h.state().auto.query(&format!("{prefix}.option.")).len() > 1);
        type_text(&mut h, "JetBrains");
        click(&mut h, &format!("{prefix}.option.JetBrains Mono"));
        assert_eq!(layer_doc(&h.state().session, LayerId(lid)).unwrap().font, "JetBrains Mono");
    }
}

#[test]
fn font_search_preserves_edited_text_and_applies_only_to_selected_characters() {
    let mut s = Session::default();
    s.execute("comp.new", json!({"width": 320, "height": 180})).unwrap();
    let lid = s.execute("layer.newText", json!({"text": "First second", "font": "Inter"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("text.edit", json!({"layer": lid})).unwrap();
    s.execute("text.setSelection", json!({"start": 0, "end": 5})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).build_eframe(move |_| EffectcraftApp::new(s));
    h.state_mut().show_panel(PanelKind::Character);
    h.run_steps(3);
    click(&mut h, "character.font");
    type_text(&mut h, "noto serif");
    key(&mut h, Key::ArrowDown);
    key(&mut h, Key::ArrowUp);
    key(&mut h, Key::Enter);
    let doc = layer_doc(&h.state().session, LayerId(lid)).unwrap();
    assert_eq!(doc.text, "First second");
    assert_eq!(doc.style_at(0).font, "Noto Serif");
    assert_eq!(doc.style_at(8).font, "Inter");
}
