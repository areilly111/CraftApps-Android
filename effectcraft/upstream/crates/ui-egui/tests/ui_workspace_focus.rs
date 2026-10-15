//! Workspaces survive a new frontend, and keyboard input follows real widget focus.

use std::sync::Arc;

use effectcraft_engine::Session;
use effectcraft_engine::config::{ConfigStore, DirConfig, MemoryConfig};
use effectcraft_ui_egui::EffectcraftApp;
use effectcraft_ui_egui::dock::{DockNode, PanelKind, SplitSize};
use effectcraft_ui_egui::state::Tool;
use egui::{Event, Id, Key, Modifiers, vec2};
use egui_kittest::Harness;
use serde_json::json;

#[test]
fn named_and_current_workspaces_survive_frontend_restart() {
    let store = Arc::new(MemoryConfig::default());
    let mut app = EffectcraftApp::new(Session { config: Some(store.clone()), ..Default::default() });
    app.ui.dock = DockNode::Split {
        vertical: false,
        size: SplitSize::FixedA(345.0),
        a: Box::new(DockNode::Tabs { panels: vec![PanelKind::Project], active: 0 }),
        b: Box::new(DockNode::Tabs { panels: vec![PanelKind::Composition, PanelKind::Layer], active: 1 }),
    };
    let ctx = egui::Context::default();
    effectcraft_ui_egui::menus::invoke(&mut app, &ctx, "window.saveWorkspaceAs", json!({"name": "Restart layout"})).unwrap();
    let saved = app.ui.dock.clone();
    if let DockNode::Split { size, .. } = &mut app.ui.dock {
        *size = SplitSize::FixedA(390.0);
    }
    app.ui.focused = PanelKind::Layer;
    let current = app.ui.dock.clone();
    eframe::App::on_exit(&mut app);
    assert!(store.read("workspaces.json").is_some(), "workspace data must reach the configured store");
    let mut restarted = EffectcraftApp::new(Session { config: Some(store), ..Default::default() });
    assert_eq!(restarted.ui.workspace, "Restart layout");
    assert_eq!(restarted.ui.dock, current, "restore current unsaved divider changes independently of the named snapshot");
    assert_eq!(restarted.ui.saved_workspaces.get("Restart layout"), Some(&saved));
    assert_eq!(restarted.ui.focused, PanelKind::Layer);
    restarted.set_workspace("Default");
    restarted.set_workspace("Restart layout");
    assert_eq!(restarted.ui.dock, saved, "the named snapshot is separately available after restart");
}

fn key(h: &mut Harness<'_, EffectcraftApp>, key: Key, modifiers: Modifiers) {
    h.event(Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
    h.step();
    h.event(Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers });
    h.step();
    h.run_steps(2);
}

fn scratch_dir(label: &str) -> std::path::PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("effectcraft-workspace-{label}-{}-{id}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    path
}

#[test]
fn native_config_read_is_byte_bounded_and_reports_errors() {
    let dir = scratch_dir("bounded-read");
    let config = DirConfig::new(&dir);
    assert!(config.read_bounded("missing", 4).unwrap().is_none());
    std::fs::write(dir.join("entry"), "é").unwrap();
    assert_eq!(config.read_bounded("entry", 2).unwrap().as_deref(), Some("é"));
    assert_eq!(config.read_bounded("entry", 1).unwrap_err().kind(), std::io::ErrorKind::InvalidData);
    std::fs::write(dir.join("entry"), [0xff, 0xfe]).unwrap();
    assert_eq!(config.read_bounded("entry", 4).unwrap_err().kind(), std::io::ErrorKind::InvalidData);
    std::fs::create_dir(dir.join("folder")).unwrap();
    assert!(config.read_bounded("folder", 4).is_err());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn unreadable_native_workspace_is_preserved_after_later_layout_changes() {
    for bytes in
        [vec![0xff, 0xfe], vec![b' '; 1024 * 1024 + 1], b"{".to_vec(), serde_json::to_vec(&json!({"version":2,"workspaces":{},"floating":{}})).unwrap()]
    {
        let dir = scratch_dir("preserve");
        let path = dir.join("workspaces.json");
        std::fs::write(&path, &bytes).unwrap();
        let mut app = EffectcraftApp::new(Session { config: Some(Arc::new(DirConfig::new(&dir))), ..Default::default() });
        let ctx = egui::Context::default();
        assert!(effectcraft_ui_egui::control::inspect(&app, &ctx)["workspaceStorageError"].is_string());
        app.set_workspace("Minimal");
        app.ui.focused = PanelKind::Project;
        effectcraft_ui_egui::menus::invoke(&mut app, &ctx, "window.saveWorkspaceAs", json!({"name":"Must not overwrite"})).unwrap();
        eframe::App::on_exit(&mut app);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert!(effectcraft_ui_egui::control::inspect(&app, &ctx)["workspaceStorageError"].is_string());
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn legacy_named_workspaces_migrate_once_and_survive_actual_app_restart() {
    let store = Arc::new(MemoryConfig::default());
    let saved = effectcraft_ui_egui::dock::workspace("Minimal");
    store.write("workspaces.json", &json!({"workspaces":{"Legacy":saved},"floating":{}}).to_string()).unwrap();
    let mut app = EffectcraftApp::new(Session { config: Some(store.clone()), ..Default::default() });
    assert_eq!(app.ui.saved_workspaces.get("Legacy"), Some(&saved));
    app.set_workspace("Legacy");
    let ctx = egui::Context::default();
    effectcraft_ui_egui::menus::invoke(&mut app, &ctx, "window.saveWorkspaceAs", json!({"name":"Migrated"})).unwrap();
    let text = store.read("workspaces.json").unwrap();
    assert_eq!(serde_json::from_str::<serde_json::Value>(&text).unwrap()["version"], 1);
    let restarted = EffectcraftApp::new(Session { config: Some(store), ..Default::default() });
    assert_eq!(restarted.ui.saved_workspaces.get("Legacy"), Some(&saved));
    assert_eq!(restarted.ui.saved_workspaces.get("Migrated"), Some(&saved));
    assert_eq!(restarted.ui.workspace, "Migrated");
}

#[test]
fn tab_and_shift_tab_traverse_toolbar_and_space_activates_only_focused_tool() {
    let mut session = Session::default();
    session.execute("comp.new", json!({"width": 160, "height": 90, "duration": 1})).unwrap();
    session.execute("layer.newSolid", json!({"name": "Keep unchanged", "color": "#ff0000"})).unwrap();
    let project = session.project.clone();
    let mut app = EffectcraftApp::new(session);
    app.ui.start_screen = false;
    app.ui.show_menu_bar = false;
    let mut h = Harness::builder().with_size(vec2(1400.0, 900.0)).build_eframe(move |_| app);
    h.run_steps(3);
    for _ in 0..8 {
        key(&mut h, Key::Tab, Modifiers::NONE);
        if h.ctx.memory(|m| m.focused()) == Some(Id::new(("tool-slot", 0usize))) {
            break;
        }
    }
    assert_eq!(h.ctx.memory(|m| m.focused()), Some(Id::new(("tool-slot", 0usize))));
    key(&mut h, Key::Tab, Modifiers::NONE);
    assert_eq!(h.ctx.memory(|m| m.focused()), Some(Id::new(("tool-slot", 1usize))));
    assert_eq!(h.state().ui.tool, Tool::Selection, "focus movement alone must not select a tool");
    key(&mut h, Key::Tab, Modifiers::NONE);
    assert_eq!(h.ctx.memory(|m| m.focused()), Some(Id::new(("tool-slot", 2usize))));
    key(&mut h, Key::Tab, Modifiers::SHIFT);
    assert_eq!(h.ctx.memory(|m| m.focused()), Some(Id::new(("tool-slot", 1usize))));
    key(&mut h, Key::Space, Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::Hand);
    assert!(!h.state().playback.playing, "Space activates the focused widget, not preview playback");
    assert!(Arc::ptr_eq(&h.state().session.project, &project), "keyboard focus and tool selection do not modify the project");
    let response = h.ctx.read_response(Id::new(("tool-slot", 1usize))).unwrap();
    assert!(
        h.output().shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Rect(r) if r.rect == response.rect.shrink(1.0) && r.stroke.color == h.state().tokens.text && r.stroke.width > 0.0)
        }),
        "the custom-painted toolbar must visibly identify keyboard focus"
    );
    let inspected = effectcraft_ui_egui::control::inspect(h.state(), &h.ctx);
    assert_eq!(inspected["keyboardFocus"]["automationId"], "tools.Hand");
    assert_eq!(inspected["keyboardFocus"]["rect"], json!([response.rect.min.x, response.rect.min.y, response.rect.width(), response.rect.height()]));
}

#[test]
fn tab_traversal_activates_an_inactive_dock_tab_without_editing_the_project() {
    let mut session = Session::default();
    session.execute("comp.new", json!({"width": 160, "height": 90, "duration": 1})).unwrap();
    session.execute("layer.newSolid", json!({"name": "Keep unchanged", "color": "#ff0000"})).unwrap();
    let project = session.project.clone();
    let mut app = EffectcraftApp::new(session);
    app.ui.start_screen = false;
    app.ui.show_menu_bar = false;
    // Leave the inactive, layer-named tab visible instead of behind the overflow control.
    if let DockNode::Split { a, .. } = &mut app.ui.dock
        && let DockNode::Split { a, .. } = a.as_mut()
        && let DockNode::Split { size, .. } = a.as_mut()
    {
        *size = SplitSize::FixedA(420.0);
    }
    let mut h = Harness::builder().with_size(vec2(1200.0, 760.0)).build_eframe(move |_| app);
    h.run_steps(3);
    assert!(h.state().auto.find("panel.tab.EffectControls").is_some(), "the inactive tab must be visible in this fixture");
    let before = h.state().ui.dock.clone();
    let target = Id::new(("tab", "aaa".to_string(), 1usize));
    let mut tabs = 0;
    for _ in 0..80 {
        key(&mut h, Key::Tab, Modifiers::NONE);
        tabs += 1;
        if h.ctx.memory(|m| m.focused()) == Some(target) {
            break;
        }
    }
    assert_eq!(h.ctx.memory(|m| m.focused()), Some(target));
    println!("Tab presses from no focus to Effect Controls: {tabs}");
    assert_eq!(h.state().ui.dock, before, "traversal alone must not activate a tab");
    key(&mut h, Key::Space, Modifiers::NONE);
    let mut activated = before;
    assert!(activated.activate(PanelKind::EffectControls));
    assert_eq!(h.state().ui.dock, activated);
    assert_eq!(h.state().ui.focused, PanelKind::EffectControls);
    assert!(!h.state().playback.playing);
    assert!(Arc::ptr_eq(&h.state().session.project, &project));
    let response = h.ctx.read_response(target).unwrap();
    assert!(
        h.output().shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Rect(r) if r.rect == response.rect.shrink(1.0) && r.stroke.color == h.state().tokens.text && r.stroke.width > 0.0)
        }),
        "the active dock tab must visibly retain keyboard focus"
    );
    let inspected = effectcraft_ui_egui::control::inspect(h.state(), &h.ctx);
    assert_eq!(inspected["keyboardFocus"]["automationId"], "panel.tab.EffectControls");
    key(&mut h, Key::Tab, Modifiers::SHIFT);
    assert_eq!(h.ctx.memory(|m| m.focused()), Some(Id::new(("tab", "aaa".to_string(), 0usize))));
    key(&mut h, Key::Enter, Modifiers::NONE);
    assert_eq!(h.state().ui.focused, PanelKind::Project);
    assert!(!h.state().playback.playing);
    assert!(Arc::ptr_eq(&h.state().session.project, &project));
}

struct ControlDriver {
    h: Harness<'static, EffectcraftApp>,
    tx: std::sync::mpsc::Sender<effectcraft_ui_egui::ControlRequest>,
}

impl ControlDriver {
    fn new() -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut app = EffectcraftApp::new(Session::default()).with_control(rx);
        app.ui.start_screen = false;
        app.ui.show_menu_bar = false;
        let mut driver = Self { h: Harness::builder().with_size(vec2(1200.0, 760.0)).build_eframe(move |_| app), tx };
        driver.frames(4, true);
        driver.ok("engine.execute", json!({"command":"comp.new", "params":{"name":"Native Validation", "width":160,"height":90,"duration":1}}), true);
        driver.ok("engine.execute", json!({"command":"layer.newSolid", "params":{"name":"Validation Red","color":"#ff0000"}}), true);
        driver.ok("ui.set", json!({"focused":"Timeline"}), true);
        driver.ok("ui.key", json!({"key":"Z","command":true}), true);
        driver.ok("ui.key", json!({"key":"Z","command":true,"shift":true}), true);
        driver
    }

    fn frames(&mut self, count: usize, native_focused: bool) {
        for _ in 0..count {
            let ctx = self.h.ctx.clone();
            let mut raw = std::mem::take(self.h.input_mut());
            raw.focused = native_focused;
            if let Some(viewport) = raw.viewports.get_mut(&egui::ViewportId::ROOT) {
                viewport.focused = Some(native_focused);
            }
            eframe::App::raw_input_hook(self.h.state_mut(), &ctx, &mut raw);
            *self.h.input_mut() = raw;
            self.h.step();
        }
    }

    fn ok(&mut self, method: &str, params: serde_json::Value, native_focused: bool) -> serde_json::Value {
        let (request, reply) = effectcraft_ui_egui::ControlRequest::new(method, params);
        self.tx.send(request).unwrap();
        for _ in 0..60 {
            self.frames(1, native_focused);
            if let Ok(result) = reply.try_recv() {
                assert_eq!(result["ok"], true, "{result}");
                self.frames(2, native_focused);
                return result["result"].clone();
            }
        }
        panic!("no control reply to {method}");
    }
}

#[test]
fn control_root_divider_drag_after_toolbar_and_dock_tab_keyboard_activation() {
    let mut d = ControlDriver::new();
    let project = d.h.state().session.project.clone();
    let times = d.h.state().session.state.times.clone();
    assert_eq!(d.h.state().ui.focused, PanelKind::Timeline);
    assert!(d.h.state().ui.mini_flowchart.is_none());
    d.ok("ui.click", json!({"id":"tools.Selection"}), true);
    for _ in 0..5 {
        d.ok("ui.key", json!({"key":"Tab"}), true);
        assert!(d.h.state().ui.mini_flowchart.is_none(), "Tab entering or traversing widgets must not also invoke the Timeline mini-flowchart shortcut");
    }
    d.ok("ui.key", json!({"key":"Tab","shift":true}), true);
    d.ok("ui.key", json!({"key":"Space"}), true);
    assert_eq!(d.h.state().ui.tool, Tool::Hand);
    d.ok("ui.key", json!({"key":"Tab","shift":true}), true);
    d.ok("ui.key", json!({"key":"Enter"}), true);
    assert_eq!(d.h.state().ui.tool, Tool::Selection);
    d.ok("ui.drag", json!({"from":{"id":"dock.gutter.aa","fx":0.5,"fy":0.5},"to":{"id":"dock.gutter.aa","fx":10.5,"fy":0.5},"steps":8}), true);
    d.ok("ui.click", json!({"id":"tools.Selection"}), true);
    // The toolbar can grow as upstream adds controls. Traverse to the actual response
    // identity rather than assuming a fixed number of focus stops.
    let target = Id::new(("tab", "aaa".to_string(), 1usize));
    for tabs in 1..=100 {
        d.ok("ui.key", json!({"key":"Tab"}), true);
        assert!(d.h.state().ui.mini_flowchart.is_none(), "dock-tab traversal must not open a foreground popup");
        if d.h.ctx.memory(|memory| memory.focused()) == Some(target) {
            println!("control Tab presses to Effect Controls: {tabs}");
            break;
        }
    }
    assert_eq!(d.h.ctx.memory(|memory| memory.focused()), Some(target), "Effect Controls must remain reachable by control-channel Tab traversal");
    d.ok("ui.key", json!({"key":"Space"}), true);
    assert_eq!(d.h.state().ui.focused, PanelKind::EffectControls);
    d.ok("ui.key", json!({"key":"Tab","shift":true}), true);
    d.ok("ui.key", json!({"key":"Enter"}), true);
    assert_eq!(d.h.state().ui.focused, PanelKind::Project);
    let before = d.h.state().ui.dock.clone();
    let gutter = d.h.state().auto.find("dock.gutter.").unwrap().rect;
    println!("root gutter before control drag: {gutter:?}; mini-flowchart: {:?}", d.h.state().ui.mini_flowchart);
    d.ok("ui.drag", json!({"from":{"id":"dock.gutter.","fx":0.5,"fy":0.5},"to":{"id":"dock.gutter.","fx":5.0,"fy":0.5},"steps":8}), true);
    assert_ne!(d.h.state().ui.dock, before, "the foreground root divider must still accept a drag after dock-tab keyboard activation");
    assert_eq!(d.h.state().session.state.times, times);
    assert!(Arc::ptr_eq(&d.h.state().session.project, &project));
    assert!(!d.h.state().playback.playing);
    for _ in 0..160 {
        d.ok("ui.key", json!({"key":"Tab"}), true);
        assert!(d.h.state().ui.mini_flowchart.is_none(), "wrapping widget traversal must not dispatch the application Tab binding");
    }
}

#[test]
fn unowned_tab_keeps_the_timeline_mini_flowchart_shortcut() {
    let mut session = Session::default();
    session.execute("comp.new", json!({"width":160,"height":90,"duration":1})).unwrap();
    let mut app = EffectcraftApp::new(session);
    app.ui.focused = PanelKind::Timeline;
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        events: vec![egui::Event::Key { key: Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }],
        ..Default::default()
    };
    let mut output = ctx.run_ui(raw, |ui| {
        let ctx = ui.ctx();
        effectcraft_ui_egui::menus::handle_shortcuts(&mut app, ctx);
        assert!(app.ui.mini_flowchart.is_none(), "application Tab waits until widgets can claim focus");
        assert!(!ctx.egui_wants_keyboard_input());
        effectcraft_ui_egui::menus::handle_tab_shortcut(&mut app, ctx);
    });
    output.textures_delta.clear();
    assert!(app.ui.mini_flowchart.is_some(), "an unowned Tab still invokes the application binding");
}
