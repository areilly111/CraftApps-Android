//! Settings, keyboard shortcut presets, auto-save and crash recovery.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::json;

use crate::config::{ConfigStore, DirConfig, MemoryConfig, atomic_write, atomic_write_with, temp_path};
use crate::prefs::{PREFS_FILE, Prefs};
use crate::shortcuts::{DEFAULT_PRESET, Keymaps, normalize};
use crate::{Session, autosave};

fn tmp(name: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let d = std::env::temp_dir().join(format!("ec-settings-{}-{}-{name}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn session_with_comp() -> Session {
    let mut s = Session::default();
    s.execute("comp.new", json!({"width": 320, "height": 180, "duration": 10})).unwrap();
    s
}

// ---------------------------------------------------------------- prefs model

#[test]
fn prefs_round_trip_through_json() {
    let mut p = Prefs::default();
    p.set("general.undoLevels", json!(12)).unwrap();
    p.set("labels.2.name", json!("Teal")).unwrap();
    p.set("autoSave.location", json!("custom")).unwrap();
    p.set("appearance.brightness", json!(0.25)).unwrap();
    p.push_recent("/a/b.ecproj");
    let back = Prefs::from_json(&p.to_json());
    assert_eq!(back, p);
    assert_eq!(Prefs::from_json(&Prefs::default().to_json()), Prefs::default());
}

#[test]
fn prefs_migrate_old_layout_and_keep_unknown_keys() {
    let old = r#"{
        "undoLevels": 50, "autoSaveMinutes": 7, "autoSaveVersions": 3, "theme": "light",
        "labelNames": ["Hot", "Sun"],
        "futureSetting": {"x": 1},
        "general": {"showToolTips": false, "notYetKnown": true},
        "memory": {"layerCacheMb": "not a number"}
    }"#;
    let p = Prefs::from_json(old);
    assert_eq!(p.version, crate::prefs::PREFS_VERSION);
    assert_eq!(p.general.undo_levels, 50);
    assert!(!p.general.show_tool_tips);
    assert_eq!(p.auto_save.interval_minutes, 7);
    assert_eq!(p.auto_save.max_versions, 3);
    assert_eq!(p.appearance.theme, "light");
    assert_eq!(p.labels[0].name, "Hot");
    assert_eq!(p.labels[1].name, "Sun");
    assert_eq!(p.labels[2].name, "Aqua");
    assert_eq!(p.labels.len(), 16);
    // A bad value falls back to that page's defaults; other pages survive.
    assert_eq!(p.memory.layer_cache_mb, 1024);
    // Unknown keys (a newer version's) are kept and written back.
    let again: serde_json::Value = serde_json::from_str(&p.to_json()).unwrap();
    assert_eq!(again["futureSetting"], json!({"x": 1}));
    assert_eq!(again["general"]["notYetKnown"], json!(true));
    assert!(again.get("undoLevels").is_none(), "migrated keys leave the top level");
    // Garbage is defaults.
    assert_eq!(Prefs::from_json("not json"), Prefs::default());
}

#[test]
fn prefs_set_validates_keys_types_and_ranges() {
    let mut p = Prefs::default();
    assert!(p.set("general.nope", json!(1)).is_err());
    assert!(p.set("general.showToolTips", json!([1])).is_err());
    p.set("general.undoLevels", json!(500)).unwrap();
    assert_eq!(p.general.undo_levels, 99);
    p.set("general.undoLevels", json!("7")).unwrap();
    assert_eq!(p.general.undo_levels, 7);
    p.set("labels.0.color", json!("zzz")).unwrap();
    assert_eq!(p.labels[0].color, Prefs::default().labels[0].color);
    p.reset(Some("general")).unwrap();
    assert_eq!(p.general.undo_levels, 32);
    assert!(p.reset(Some("bogus")).is_err());
}

/// Settings ▸ Appearance ▸ UI Scale (#284): a percentage, 100 by default, kept within 75–200;
/// the dialog's choices set it as a number.
#[test]
fn ui_scale_is_a_percentage_from_75_to_200() {
    let mut p = Prefs::default();
    assert_eq!(p.appearance.ui_scale, 100);
    p.set("appearance.uiScale", json!("150")).unwrap();
    assert_eq!(p.appearance.ui_scale, 150);
    assert_eq!(p.get("appearance.uiScale"), Some(json!(150)));
    p.set("appearance.uiScale", json!(500)).unwrap();
    assert_eq!(p.appearance.ui_scale, 200);
    p.set("appearance.uiScale", json!(10)).unwrap();
    assert_eq!(p.appearance.ui_scale, 75);
    let page = crate::prefs::pages().into_iter().find(|pg| pg.id == "appearance").unwrap();
    let listed = page.items.iter().any(|i| {
        matches!(i, crate::prefs::Item::Setting { key: "appearance.uiScale", kind: crate::prefs::Kind::Choice(c), live: true, .. } if c.contains(&("200%", "200")))
    });
    assert!(listed, "Settings ▸ Appearance offers it");
    // Older settings files without it get the default.
    assert_eq!(Prefs::from_json(r#"{"version": 2, "appearance": {"theme": "light"}}"#).appearance.ui_scale, 100);
}

/// Settings ▸ Appearance: new installs stay Dark (Sync with System is opt-in), and settings saved
/// with a single theme load as a fixed mode showing that theme.
#[test]
fn appearance_defaults_and_legacy_theme_migrate() {
    let p = Prefs::default();
    assert_eq!((p.appearance.appearance_mode.as_str(), p.appearance.dark_theme.as_str(), p.appearance.light_theme.as_str()), ("dark", "dark", "light"));
    assert_eq!(p.appearance.resolved_theme(Some(true)), "dark");
    for (saved, mode, dark, light) in [("darker", "dark", "darker", "light"), ("light", "light", "dark", "light"), ("dark", "dark", "dark", "light")] {
        let p = Prefs::from_json(&json!({"version": 2, "appearance": {"theme": saved}}).to_string());
        assert_eq!(p.appearance.appearance_mode, mode, "{saved}");
        assert_eq!(p.appearance.dark_theme, dark, "{saved}");
        assert_eq!(p.appearance.light_theme, light, "{saved}");
        assert_eq!(p.appearance.theme, saved);
    }
    // The flat layout of the earliest builds migrates too.
    assert_eq!(Prefs::from_json(r#"{"theme": "light"}"#).appearance.appearance_mode, "light");
    // A file that already has a mode keeps it; garbage falls back to the defaults.
    let p = Prefs::from_json(r#"{"version": 2, "appearance": {"theme": "dark", "appearanceMode": "auto", "darkTheme": "darker"}}"#);
    assert_eq!((p.appearance.appearance_mode.as_str(), p.appearance.dark_theme.as_str()), ("auto", "darker"));
    let p = Prefs::from_json(r#"{"version": 2, "appearance": {"appearanceMode": "sepia", "darkTheme": "light", "lightTheme": 7}}"#);
    assert_eq!((p.appearance.appearance_mode.as_str(), p.appearance.dark_theme.as_str(), p.appearance.light_theme.as_str()), ("dark", "dark", "light"));
}

/// The appearance settings take only their choices, survive a restart, and old clients that set
/// the single theme still pick a visible theme without losing the other family's choice.
#[test]
fn appearance_choices_validate_and_legacy_theme_still_selects() {
    let mut s = Session::default();
    s.execute("prefs.set", json!({"values": {"appearance.appearanceMode": "auto", "appearance.darkTheme": "darker", "appearance.lightTheme": "light"}}))
        .unwrap();
    assert_eq!(s.prefs.appearance.appearance_mode, "auto");
    assert_eq!(s.prefs.appearance.dark_theme, "darker");
    assert_eq!(s.prefs.appearance.resolved_theme(Some(true)), "light");
    assert_eq!(s.prefs.appearance.resolved_theme(Some(false)), "darker");
    assert_eq!(s.prefs.appearance.resolved_theme(None), "darker", "no system answer: dark");
    assert_eq!(Prefs::from_json(&s.prefs.to_json()).appearance, s.prefs.appearance);
    for (key, bad) in
        [("appearance.darkTheme", "light"), ("appearance.lightTheme", "darker"), ("appearance.appearanceMode", "sepia"), ("appearance.theme", "neon")]
    {
        assert!(s.execute("prefs.set", json!({"key": key, "value": bad})).is_err(), "{key} = {bad}");
    }
    s.execute("prefs.set", json!({"key": "appearance.theme", "value": "light"})).unwrap();
    assert_eq!(s.prefs.appearance.appearance_mode, "light");
    assert_eq!(s.prefs.appearance.dark_theme, "darker");
    s.execute("prefs.set", json!({"key": "appearance", "value": {"theme": "dark"}})).unwrap();
    assert_eq!(s.prefs.appearance.appearance_mode, "dark");
    assert_eq!(s.prefs.appearance.dark_theme, "dark");
    // A fixed mode mirrors its theme in the single-theme key for old readers.
    s.execute("prefs.set", json!({"key": "appearance.appearanceMode", "value": "light"})).unwrap();
    assert_eq!(s.prefs.get("appearance.theme"), Some(json!("light")));
    s.prefs.reset(Some("appearance")).unwrap();
    assert_eq!(s.prefs.appearance, crate::prefs::Appearance::default());
}

#[test]
fn every_schema_key_exists_and_docs_list_the_todo_settings() {
    let p = Prefs::default();
    let mut ids = vec![];
    for page in crate::prefs::pages() {
        assert!(crate::prefs::page_id(page.id).is_some(), "page {}", page.id);
        ids.push(page.id);
        for item in page.items {
            if let crate::prefs::Item::Setting { key, .. } | crate::prefs::Item::AudioDevices { key } = item {
                assert!(p.get(key).is_some(), "schema key `{key}` is not a setting");
            }
        }
    }
    // The Settings menu and the dialog list the same pages.
    for (path, e) in crate::menus::entries() {
        if e.command == "app.settings" {
            let page = e.params["page"].as_str().unwrap();
            assert!(ids.contains(&page), "{path:?} opens unknown page {page}");
        }
    }
    let docs = include_str!("../../../docs/preferences.md");
    for k in crate::prefs::todo_keys() {
        assert!(docs.contains(&format!("`{k}`")), "docs/preferences.md must list the not-yet-wired setting `{k}`");
    }
}

// ---------------------------------------------------------------- prefs effects

#[test]
fn prefs_commands_get_set_reset_and_persist() {
    let store = Arc::new(MemoryConfig::default());
    let mut s = Session { config: Some(store.clone()), ..Default::default() };
    s.execute_checked("prefs.set", json!({"key": "general.recentItems", "value": 4})).unwrap();
    assert_eq!(s.execute("prefs.get", json!({"key": "general.recentItems"})).unwrap(), json!(4));
    s.execute("prefs.set", json!({"values": {"autoSave.maxVersions": 9, "grids.gridSpacing": 50}})).unwrap();
    assert!(s.execute("prefs.set", json!({"key": "nope.nope", "value": 1})).is_err());
    let saved = store.read(PREFS_FILE).unwrap();
    let mut s2 = Session { config: Some(store.clone()), ..Default::default() };
    s2.load_settings();
    assert_eq!(s2.prefs.general.recent_items, 4);
    assert_eq!(s2.prefs.auto_save.max_versions, 9);
    assert!(saved.contains("\"maxVersions\": 9"));
    s2.execute("prefs.reset", json!({"page": "project"})).unwrap();
    assert_eq!(s2.prefs.auto_save.max_versions, 5);
    assert!(s2.execute("prefs.pages", json!({})).unwrap().as_array().unwrap().len() >= 17);
    s2.execute("prefs.open", json!({"page": "Auto-Save"})).unwrap();
    assert!(
        s2.drain_events().iter().any(|e| matches!(e, crate::Event::Frontend { command, params } if command == "app.settings" && params["page"] == "project"))
    );
}

#[test]
fn undo_levels_are_honoured() {
    let mut s = session_with_comp();
    s.execute("prefs.set", json!({"key": "general.undoLevels", "value": 3})).unwrap();
    for i in 0..6 {
        s.execute("layer.newNull", json!({"name": format!("N{i}")})).unwrap();
    }
    assert_eq!(s.history.undo.len(), 3);
    for _ in 0..3 {
        assert!(s.undo());
    }
    assert!(!s.undo());
    assert_eq!(s.active_comp().unwrap().layers.len(), 3);
    // Lowering the setting trims the history right away.
    s.execute("prefs.set", json!({"key": "general.undoLevels", "value": 1})).unwrap();
    assert!(s.history.undo.len() <= 1);
}

#[test]
fn renamed_labels_show_in_the_label_menu_and_apply_by_name() {
    let mut s = session_with_comp();
    s.execute("layer.newNull", json!({})).unwrap();
    s.execute("prefs.set", json!({"key": "labels.0.name", "value": "Hot Sauce"})).unwrap();
    s.execute("prefs.set", json!({"key": "labels.0.color", "value": "#102030"})).unwrap();
    let red = crate::menus::entries().into_iter().find(|(_, e)| e.command == "edit.label" && e.params["label"] == "Red").unwrap().1;
    assert_eq!(crate::menus::entry_label(&s, red), "Hot Sauce");
    assert_eq!(s.prefs.label_rgb(effectcraft_color::Label::Red), [0x10, 0x20, 0x30]);
    s.execute("edit.label", json!({"label": "Yellow"})).unwrap();
    s.execute("edit.label", json!({"label": "hot sauce"})).unwrap();
    let l = &s.active_comp().unwrap().layers[0];
    assert_eq!(l.label, effectcraft_color::Label::Red);
}

#[test]
fn cache_budgets_are_applied() {
    let mut s = Session::default();
    s.execute("prefs.set", json!({"key": "memory.layerCacheMb", "value": 128})).unwrap();
    assert_eq!(s.layer_cache.budget(), 128 << 20);
    struct Src(std::sync::Mutex<usize>);
    impl effectcraft_render::FootageSource for Src {
        fn frame(&self, _: effectcraft_project::ItemId, _: &effectcraft_project::Footage, _: effectcraft_time::Tick) -> Option<Arc<effectcraft_raster::Image>> {
            None
        }
        fn set_cache_budget(&self, b: usize) {
            *self.0.lock().unwrap() = b;
        }
    }
    let src = Arc::new(Src(Default::default()));
    s.footage = src.clone();
    s.execute("prefs.set", json!({"key": "memory.mediaCacheMb", "value": 256})).unwrap();
    assert_eq!(*src.0.lock().unwrap(), 256 << 20);
}

#[test]
fn new_layers_start_at_the_current_time_when_asked() {
    let mut s = session_with_comp();
    let nested = s.execute("comp.new", json!({"name": "Nested", "open": false})).unwrap()["comp"].as_u64().unwrap();
    s.set_time(effectcraft_time::Tick::from_seconds_f64(2.0));
    s.execute("layer.addItem", json!({"item": nested})).unwrap();
    assert_eq!(s.active_comp().unwrap().layers[0].start_time, effectcraft_time::Tick::ZERO);
    s.execute("prefs.set", json!({"key": "general.createLayersAtCompStart", "value": false})).unwrap();
    s.execute("layer.addItem", json!({"item": nested})).unwrap();
    assert_eq!(s.active_comp().unwrap().layers[0].start_time.seconds().round(), 2.0);
}

#[test]
fn default_spatial_interpolation_linear() {
    let mut s = session_with_comp();
    s.execute("layer.newNull", json!({})).unwrap();
    s.execute("prop.set", json!({"path": "transform/position", "value": [0, 0], "time": 0})).unwrap();
    s.execute("prop.toggleAnimation", json!({"path": "transform/position", "value": true})).ok();
    s.execute("prop.addKey", json!({"path": "transform/position", "time": 0})).unwrap();
    let auto =
        |s: &Session| s.active_comp().unwrap().layers[0].props.prop("transform/position").unwrap().keys.iter().map(|k| k.spatial_auto).collect::<Vec<_>>();
    assert!(auto(&s).iter().all(|a| *a));
    s.execute("prefs.set", json!({"key": "general.defaultSpatialLinear", "value": true})).unwrap();
    s.execute("prop.addKey", json!({"path": "transform/position", "time": 1, "value": [100, 50]})).unwrap();
    assert_eq!(auto(&s), vec![true, false]);
}

#[test]
fn new_project_template_and_default_renderer() {
    let d = tmp("template");
    let tpl = d.join("tpl.ecproj");
    let mut a = session_with_comp();
    a.execute("file.saveAs", json!({"path": tpl.to_string_lossy()})).unwrap();
    let mut s = Session::default();
    s.execute("prefs.set", json!({"values": {"project.useTemplate": true, "project.templatePath": tpl.to_string_lossy()}})).unwrap();
    s.execute("file.newProject", json!({})).unwrap();
    assert_eq!(s.project.comps().count(), 1);
    assert!(s.path.is_none(), "a template opens untitled");
    s.execute("prefs.set", json!({"key": "threeD.defaultRenderer", "value": "advanced"})).unwrap();
    s.execute("comp.new", json!({})).unwrap();
    assert_eq!(s.active_comp().unwrap().renderer, effectcraft_project::Renderer::Advanced3D);
    let _ = std::fs::remove_dir_all(d);
}

// ---------------------------------------------------------------- shortcuts

#[test]
fn shortcut_text_normalizes() {
    assert_eq!(normalize("shift+cmd+k").as_deref(), Some("Cmd+Shift+K"));
    assert_eq!(normalize("Opt+Ctrl+f5").as_deref(), Some("Ctrl+Alt+F5"));
    assert_eq!(normalize("⌘⇧S").as_deref(), Some("Cmd+Shift+S"));
    assert_eq!(normalize("Cmd++").as_deref(), Some("Cmd++"));
    assert_eq!(normalize("space").as_deref(), Some("Space"));
    assert_eq!(normalize("Cmd+").as_deref(), None);
}

#[test]
fn defaults_have_no_conflicts_and_match_the_menus() {
    let s = Session::default();
    let t = s.shortcuts();
    assert!(t.all_conflicts().is_empty(), "{:?}", t.all_conflicts());
    assert_eq!(t.shortcut_of("file.save", &json!(null)), Some("Cmd+S"));
    assert_eq!(t.shortcut_of("app.settings", &json!({"page": "general"})), Some("Cmd+Alt+;"));
    assert!(t.bindables.len() > 500);
}

#[test]
fn shortcut_edits_detect_conflicts_and_create_a_custom_preset() {
    let store = Arc::new(MemoryConfig::default());
    let mut s = Session { config: Some(store.clone()), ..Default::default() };
    let r = s.execute_checked("shortcuts.set", json!({"command": "layer.newNull", "keys": "cmd+s"})).unwrap();
    assert_eq!(r["keys"], json!(["Cmd+S"]));
    assert_eq!(r["created"], json!("Custom"));
    assert!(r["conflicts"].as_array().unwrap().iter().any(|c| c["key"] == "file.save"), "{r}");
    assert_eq!(s.keymaps.active, "Custom");
    assert_eq!(s.shortcuts().shortcut_of("layer.newNull", &json!(null)), Some("Cmd+S"));
    assert!(!s.execute("shortcuts.conflicts", json!({})).unwrap().as_array().unwrap().is_empty());
    // The default preset is untouched.
    s.execute("shortcuts.preset", json!({"op": "select", "name": DEFAULT_PRESET})).unwrap();
    assert_eq!(s.shortcuts().shortcut_of("layer.newNull", &json!(null)), Some("Cmd+Alt+Shift+Y"));
    // Remove a shortcut; reset one.
    s.execute("shortcuts.preset", json!({"op": "select", "name": "Custom"})).unwrap();
    s.execute("shortcuts.set", json!({"command": "file.save", "keys": null})).unwrap();
    assert_eq!(s.shortcuts().shortcut_of("file.save", &json!(null)), None);
    s.execute("shortcuts.reset", json!({"command": "file.save"})).unwrap();
    assert_eq!(s.shortcuts().shortcut_of("file.save", &json!(null)), Some("Cmd+S"));
    // Bound-parameter entries are bindable too.
    s.execute("shortcuts.set", json!({"command": "app.settings", "params": {"page": "labels"}, "keys": ["F9"]})).unwrap();
    assert_eq!(s.shortcuts().shortcut_of("app.settings", &json!({"page": "labels"})), Some("F9"));
    // Persisted.
    let k = Keymaps::from_json(&store.read(crate::shortcuts::SHORTCUTS_FILE).unwrap());
    assert_eq!(k.active, "Custom");
    assert!(s.execute("shortcuts.set", json!({"command": "no.such", "keys": "F1"})).is_err());
    assert!(s.execute("shortcuts.set", json!({"command": "file.save", "keys": "Cmd+"})).is_err());
    let list = s.execute("shortcuts.list", json!({"keys": "F9"})).unwrap();
    assert!(list["commands"].as_array().unwrap().iter().any(|c| c["command"] == "app.settings" && c["params"]["page"] == "labels"), "{list}");
}

#[test]
fn shortcut_presets_duplicate_rename_delete_export_import() {
    let d = tmp("presets");
    let mut s = Session::default();
    s.execute("shortcuts.preset", json!({"op": "new", "name": "Mine"})).unwrap();
    s.execute("shortcuts.set", json!({"command": "edit.undo", "keys": ["Cmd+Z", "F2"]})).unwrap();
    s.execute("shortcuts.preset", json!({"op": "duplicate", "name": "Mine 2"})).unwrap();
    assert_eq!(s.keymaps.names(), vec![DEFAULT_PRESET, "Mine", "Mine 2"]);
    s.execute("shortcuts.preset", json!({"op": "rename", "name": "Mine 2", "newName": "Other"})).unwrap();
    assert!(s.execute("shortcuts.preset", json!({"op": "delete", "name": DEFAULT_PRESET})).is_err());
    let path = d.join("mine.json");
    s.execute("shortcuts.export", json!({"preset": "Mine", "path": path.to_string_lossy()})).unwrap();
    s.execute("shortcuts.preset", json!({"op": "delete", "name": "Mine"})).unwrap();
    s.execute("shortcuts.preset", json!({"op": "delete", "name": "Other"})).unwrap();
    assert_eq!(s.keymaps.active, DEFAULT_PRESET);
    let r = s.execute("shortcuts.import", json!({"path": path.to_string_lossy()})).unwrap();
    assert_eq!(r["preset"], json!("Mine"));
    assert_eq!(s.shortcuts().keys["edit.undo"], vec!["Cmd+Z".to_string(), "F2".into()]);
    s.execute("shortcuts.reset", json!({})).unwrap();
    assert_eq!(s.shortcuts().keys["edit.undo"], vec!["Cmd+Z".to_string()]);
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn frontend_commands_are_bindable_with_panel_scope() {
    let mut s = Session::default();
    s.set_ui_commands(vec![
        crate::shortcuts::UiCommand { id: "timeline.zoomIn".into(), label: "Zoom In Time".into(), shortcut: Some("=".into()) },
        crate::shortcuts::UiCommand { id: "tool.hand".into(), label: "Hand Tool".into(), shortcut: Some("H".into()) },
    ]);
    let b = s.shortcuts().find("timeline.zoomIn").unwrap().clone();
    assert_eq!(b.scope, "Timeline");
    assert_eq!(s.shortcuts().shortcut_of("tool.hand", &json!(null)), Some("H"));
    // A panel shortcut conflicts with an application shortcut on the same keys.
    let c = s.shortcuts().conflicts(&b, "H");
    assert!(c.iter().any(|c| c.key == "tool.hand"));
}

// ---------------------------------------------------------------- auto-save / recovery

#[test]
fn atomic_write_survives_a_crash_mid_write() {
    let d = tmp("atomic");
    let f = d.join("p.ecproj");
    atomic_write(&f, b"version 1").unwrap();
    // The writer dies halfway through the temporary file.
    let r = atomic_write_with(&f, b"version 2 is much longer", |tmp, data| {
        std::fs::write(tmp, &data[..5])?;
        Err(std::io::Error::other("power cut"))
    });
    assert!(r.is_err());
    assert_eq!(std::fs::read(&f).unwrap(), b"version 1");
    assert!(!temp_path(&f).exists(), "the partial temp file is cleaned up");
    // A temp file left by a hard kill doesn't count as an auto-save.
    std::fs::write(d.join("p auto-save 1.ecproj.tmp"), b"junk").unwrap();
    assert!(autosave::existing(&d, "p").is_empty());
    atomic_write(&f, b"version 3").unwrap();
    assert_eq!(std::fs::read(&f).unwrap(), b"version 3");
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn autosave_rotates_through_max_versions() {
    let d = tmp("rotate");
    let proj = d.join("Intro.ecproj");
    let mut s = session_with_comp();
    s.execute("file.saveAs", json!({"path": proj.to_string_lossy()})).unwrap();
    s.execute("prefs.set", json!({"key": "autoSave.maxVersions", "value": 3})).unwrap();
    let folder = d.join(autosave::AUTOSAVE_FOLDER);
    let mut written = vec![];
    for i in 0..5 {
        s.execute("layer.newNull", json!({"name": format!("save{i}")})).unwrap();
        written.push(s.autosave_now().unwrap());
    }
    let names: Vec<String> = written.iter().map(|p| PathBuf::from(p).file_name().unwrap().to_string_lossy().to_string()).collect();
    assert_eq!(
        names,
        ["Intro auto-save 1.ecproj", "Intro auto-save 2.ecproj", "Intro auto-save 3.ecproj", "Intro auto-save 1.ecproj", "Intro auto-save 2.ecproj"]
    );
    let have: Vec<u32> = autosave::existing(&folder, "Intro").iter().map(|e| e.0).collect();
    assert_eq!(have, [1, 2, 3]);
    let slot2 = std::fs::read_to_string(folder.join("Intro auto-save 2.ecproj")).unwrap();
    assert!(slot2.contains("save4"), "slot 2 holds the newest save");
    // The project file itself is untouched and still dirty.
    assert!(s.is_dirty());
    assert!(!std::fs::read_to_string(&proj).unwrap().contains("save0"));
    // Lowering the maximum drops the extra slots.
    s.execute("prefs.set", json!({"key": "autoSave.maxVersions", "value": 2})).unwrap();
    s.autosave_now().unwrap();
    let have: Vec<u32> = autosave::existing(&folder, "Intro").iter().map(|e| e.0).collect();
    assert_eq!(have, [1, 2]);
    // A fresh session continues after the newest file on disk.
    let next = autosave::next_slot(&autosave::existing(&folder, "Intro"), None, 2);
    let newest = autosave::existing(&folder, "Intro").into_iter().max_by_key(|e| e.2).unwrap().0;
    assert_eq!(next, newest % 2 + 1);
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn autosave_tick_waits_for_the_interval_and_changes() {
    let d = tmp("tick");
    let mut s = session_with_comp();
    s.execute("file.saveAs", json!({"path": d.join("T.ecproj").to_string_lossy()})).unwrap();
    s.execute(
        "prefs.set",
        json!({"values": {"autoSave.intervalMinutes": 1, "autoSave.location": "custom", "autoSave.folder": d.join("custom").to_string_lossy()}}),
    )
    .unwrap();
    assert!(s.autosave_tick(1000.0).is_none(), "the first tick starts the clock");
    s.execute("layer.newNull", json!({})).unwrap();
    assert!(s.autosave_tick(1030.0).is_none(), "interval not reached");
    let p = s.autosave_tick(1061.0).unwrap().unwrap();
    assert!(p.contains("custom") && p.ends_with("T auto-save 1.ecproj"), "{p}");
    assert!(s.autosave_tick(1200.0).is_none(), "nothing changed since");
    s.execute("prefs.set", json!({"key": "autoSave.enabled", "value": false})).unwrap();
    s.execute("layer.newNull", json!({})).unwrap();
    assert!(s.autosave_tick(5000.0).is_none(), "disabled");
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn crash_recovery_detects_an_unclean_exit() {
    let d = tmp("recovery");
    let store: Arc<dyn ConfigStore> = Arc::new(DirConfig::new(d.join("config")));
    // First run: open a project, auto-save, then "crash" (no end_recovery).
    let mut s = session_with_comp();
    s.config = Some(store.clone());
    assert!(s.begin_recovery().is_none(), "first launch is clean");
    let proj = d.join("Crash.ecproj");
    s.execute("file.saveAs", json!({"path": proj.to_string_lossy()})).unwrap();
    s.execute("layer.newNull", json!({"name": "unsaved work"})).unwrap();
    let auto = s.autosave_now().unwrap();
    drop(s);
    // Second run finds the sentinel and offers the auto-save.
    let mut s2 = Session { config: Some(store.clone()), ..Default::default() };
    let r = s2.begin_recovery().expect("unclean exit detected");
    assert_eq!(r.project.as_deref(), Some(proj.to_string_lossy().as_ref()));
    assert_eq!(r.autosave.as_deref(), Some(auto.as_str()));
    s2.execute("file.open", json!({"path": r.autosave.unwrap()})).unwrap();
    assert!(s2.active_comp().unwrap().layers.iter().any(|l| l.name == "unsaved work"));
    // Clean exit: nothing to recover next time.
    s2.end_recovery();
    let mut s3 = Session { config: Some(store.clone()), ..Default::default() };
    assert!(s3.begin_recovery().is_none());
    // The setting can turn the offer off.
    let mut s4 = Session { config: Some(store), ..Default::default() };
    s4.prefs.startup.offer_crash_recovery = false;
    assert!(s4.begin_recovery().is_none());
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn open_recent_increment_and_revert() {
    let d = tmp("recent");
    let a = d.join("A.ecproj");
    let mut s = session_with_comp();
    s.execute("file.saveAs", json!({"path": a.to_string_lossy()})).unwrap();
    let r = s.execute("file.incrementAndSave", json!({})).unwrap();
    assert!(r["path"].as_str().unwrap().ends_with("A 2.ecproj"));
    let r = s.execute("file.incrementAndSave", json!({})).unwrap();
    assert!(r["path"].as_str().unwrap().ends_with("A 3.ecproj"));
    assert!(s.prefs.recent_projects[0].ends_with("A 3.ecproj"));
    assert!(s.prefs.recent_projects[2].ends_with("A.ecproj"));
    s.execute("layer.newNull", json!({})).unwrap();
    s.execute("file.revert", json!({})).unwrap();
    assert_eq!(s.active_comp().unwrap().layers.len(), 0);
    s.execute("file.openRecent", json!({"index": 2})).unwrap();
    assert!(s.path.as_deref().unwrap().ends_with("A.ecproj"));
    assert!(s.prefs.recent_projects[0].ends_with("A.ecproj"));
    s.execute("file.clearRecent", json!({})).unwrap();
    assert!(!s.is_enabled("file.openRecent"));
    assert_eq!(std::path::PathBuf::from(autosave::increment_path("/x/Intro 9.ecproj", |_| false)), std::path::PathBuf::from("/x/Intro 10.ecproj"));
    assert_eq!(std::path::PathBuf::from(autosave::increment_path("/x/v1.ecproj", |p| p.ends_with("v1 2.ecproj"))), std::path::PathBuf::from("/x/v1 3.ecproj"));
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn memory_tick_never_waits_for_the_system() {
    let mut s = Session::default();
    let t0 = std::time::Instant::now();
    s.memory_tick();
    s.memory_tick();
    // Reading the system's memory starts PowerShell on Windows (about a second): the UI thread
    // only takes the last reading.
    assert!(t0.elapsed() < std::time::Duration::from_millis(200), "{:?}", t0.elapsed());
    if crate::sysinfo::memory().is_some() {
        let mut got = None;
        for _ in 0..200 {
            got = s.memory_watch.latest();
            if got.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(got.is_some_and(|m| m.total > 0), "the reading arrives in the background");
    }
}

/// Settings ▸ Startup & Repair ▸ Window Graphics takes `auto` or `gl` (#243).
#[test]
fn window_graphics_is_validated() {
    let mut s = Session::default();
    assert_eq!(s.prefs.startup.window_graphics, "auto");
    s.execute("prefs.set", json!({"key": "startup.windowGraphics", "value": "gl"})).unwrap();
    for bad in [json!("vulkan"), json!(true)] {
        assert!(s.execute("prefs.set", json!({"key": "startup.windowGraphics", "value": bad})).is_err());
    }
    assert_eq!(s.prefs.startup.window_graphics, "gl");
    assert_eq!(Prefs::from_json(r#"{"startup":{"windowGraphics":"dx9"}}"#).startup.window_graphics, "auto");
}

#[test]
fn interface_language_is_validated_persisted_and_backward_compatible() {
    let store = Arc::new(MemoryConfig::default());
    let mut s = Session { config: Some(store.clone()), ..Default::default() };
    assert_eq!(s.prefs.general.language, "system", "Match System by default (#229)");
    s.execute("prefs.set", json!({"key": "general.language", "value": "en"})).unwrap();
    s.execute("prefs.set", json!({"key": "general.language", "value": "ja"})).unwrap();
    assert_eq!(s.execute("prefs.get", json!({"key": "general.language"})).unwrap(), json!("ja"));
    // pt-br is registered alongside en and ja, and round-trips through the file like any other.
    s.execute("prefs.set", json!({"key": "general.language", "value": "pt-br"})).unwrap();
    assert_eq!(s.execute("prefs.get", json!({"key": "general.language"})).unwrap(), json!("pt-br"));
    s.execute("prefs.set", json!({"key": "general.language", "value": "ja"})).unwrap();
    let saved = store.read(PREFS_FILE).unwrap();
    for bad in [json!("fr"), json!(""), json!(17), json!(null)] {
        assert!(s.execute("prefs.set", json!({"key": "general.language", "value": bad})).is_err());
        assert_eq!(s.prefs.general.language, "ja");
        assert_eq!(store.read(PREFS_FILE).unwrap(), saved);
    }
    // zh-hans is registered alongside en and ja: it is accepted, persisted and read back.
    s.execute("prefs.set", json!({"key": "general.language", "value": "zh-hans"})).unwrap();
    let mut zh = Session { config: Some(store.clone()), ..Default::default() };
    zh.load_settings();
    assert_eq!(zh.prefs.general.language, "zh-hans");
    // zh-hant is the same: another language of the registered set, not an error.
    s.execute("prefs.set", json!({"key": "general.language", "value": "zh-hant"})).unwrap();
    let mut zh_hant = Session { config: Some(store.clone()), ..Default::default() };
    zh_hant.load_settings();
    assert_eq!(zh_hant.prefs.general.language, "zh-hant");
    s.execute("prefs.set", json!({"key": "general.language", "value": "uk"})).unwrap();
    assert_eq!(s.execute("prefs.get", json!({"key": "general.language"})).unwrap(), json!("uk"));
    let mut uk = Session { config: Some(store.clone()), ..Default::default() };
    uk.load_settings();
    assert_eq!(uk.prefs.general.language, "uk");
    assert_eq!(Prefs::from_json(r#"{"general":{"language":"uk"}}"#).general.language, "uk");
    // es is registered the same way.
    s.execute("prefs.set", json!({"key": "general.language", "value": "es"})).unwrap();
    let mut es = Session { config: Some(store.clone()), ..Default::default() };
    es.load_settings();
    assert_eq!(es.prefs.general.language, "es");
    s.execute("prefs.set", json!({"key": "general.language", "value": "ja"})).unwrap();
    let mut reloaded = Session { config: Some(store), ..Default::default() };
    reloaded.load_settings();
    assert_eq!(reloaded.prefs.general.language, "ja");
    reloaded.execute("prefs.reset", json!({"page": "general"})).unwrap();
    assert_eq!(reloaded.prefs.general.language, "system");
    assert_eq!(Prefs::from_json(r#"{"general":{"undoLevels":17}}"#).general.language, "system");
    assert_eq!(Prefs::from_json(r#"{"general":{"language":"unknown"}}"#).general.language, "system");
    assert_eq!(Prefs::from_json(r#"{"general":{"language":"en"}}"#).general.language, "en", "a chosen language stays");
}

#[test]
fn unicode_label_colors_fall_back_without_panicking() {
    use effectcraft_color::Label;
    let mut s = Session::default();
    let default = s.prefs.label_rgb(Label::Red);
    for color in ["#€abc", "#a€bc", "#abc€"] {
        s.execute("prefs.set", json!({"key": "labels.0.color", "value": color})).unwrap();
        assert_eq!(s.prefs.label_rgb(Label::Red), default);
    }
    s.execute("prefs.set", json!({"key": "labels.0.color", "value": "#aBcD09"})).unwrap();
    assert_eq!(s.prefs.label_rgb(Label::Red), [0xab, 0xcd, 9]);
}

/// `render.backend` says why it renders on the CPU (#262): the Software Only renderer, or the
/// host's reason there is no GPU compositor.
#[test]
fn render_backend_says_why_it_renders_on_the_cpu() {
    let mut s = Session::default();
    let why = |s: &mut Session| s.execute("render.backend", json!({})).unwrap()["why"].clone();
    assert_eq!(why(&mut s), json!("no GPU compositor is attached"));
    s.accel_note = Some("headless: renders on the CPU unless started with --gpu".into());
    assert_eq!(why(&mut s), json!("headless: renders on the CPU unless started with --gpu"));
    assert_eq!(s.execute("render.backend", json!({"backend": "cpu"})).unwrap()["why"], json!("the project's renderer is Mercury Software Only"));
}
