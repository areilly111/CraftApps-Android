//! Headless checks for the M13.6 panels: each opens from the Window menu as a real panel and
//! registers its automation ids; the Footage panel's buttons edit into the comp; the Progress
//! panel lists and cancels a job; Lumetri Scopes switch scope through their dropdown.

use effectcraft_engine::Session;
use effectcraft_ui_egui::EffectcraftApp;
use effectcraft_ui_egui::dock::PanelKind;
use egui::{Event, Pos2, pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

fn harness() -> Harness<'static, EffectcraftApp> {
    let mut s = Session::default();
    s.execute("comp.new", json!({"name": "Panels", "width": 320, "height": 180, "duration": 4})).unwrap();
    s.execute("layer.newSolid", json!({"name": "Plate", "color": "#406080"})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).build_eframe(|_| EffectcraftApp::new(s));
    h.run_steps(3);
    h
}

fn open(h: &mut Harness<'_, EffectcraftApp>, panel: &str) {
    let ctx = h.ctx.clone();
    effectcraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "window.panel", json!({"panel": panel})).unwrap();
    h.run_steps(3);
}

fn click(h: &mut Harness<'_, EffectcraftApp>, id: &str) {
    let e = h.state().auto.find(id).unwrap_or_else(|| panic!("no {id}")).clone();
    let p: Pos2 = pos2(e.rect[0] + e.rect[2] / 2.0, e.rect[1] + e.rect[3] / 2.0);
    h.input_mut().events.push(Event::PointerMoved(p));
    h.input_mut().events.push(Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.step();
    h.input_mut().events.push(Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(2);
}

/// #581: the header's Discord pill fits its label with even padding (it was a fixed 104 px with
/// the text at its left).
#[test]
fn discord_pill_fits_its_label() {
    let h = harness();
    let e = h.state().auto.find("header.discord").expect("the Discord pill").clone();
    let text = h.ctx.fonts_mut(|f| f.layout_no_wrap("Discord".into(), effectcraft_ui_egui::theme::Tokens::semibold(12.0), egui::Color32::WHITE).size().x);
    // Padding 10, the 14 px icon, a 6 px gap, the text, padding 12.
    assert!((e.rect[2] - (text + 42.0)).abs() < 0.5, "{} for a {text} px label", e.rect[2]);
}

#[test]
fn window_menu_opens_the_new_panels() {
    let mut h = harness();
    for (name, kind, auto) in [
        ("lumetriScopes", PanelKind::LumetriScopes, "scopes.kind"),
        ("footage", PanelKind::Footage, "footage.empty"),
        ("mediaBrowser", PanelKind::MediaBrowser, "mediaBrowser.path"),
        ("metadata", PanelKind::Metadata, "metadata.projectComment"),
        ("progress", PanelKind::Progress, ""),
        ("contentAwareFill", PanelKind::ContentAwareFill, "contentFill.method"),
    ] {
        open(&mut h, name);
        assert!(h.state().ui.dock.contains(kind) || h.state().ui.floating.iter().any(|f| f.panels.contains(&kind)), "{name} shown");
        if !auto.is_empty() {
            assert!(h.state().auto.find(auto).is_some(), "{name}: {auto}");
        }
    }
}

#[test]
fn scopes_switch_kind() {
    let mut h = harness();
    open(&mut h, "lumetriScopes");
    assert!(h.state().auto.find("scopes.plot").is_some());
    h.state_mut().ui.scopes.scope = "vectorscopeYuv".into();
    h.run_steps(2);
    assert_eq!(h.state().auto.find("scopes.plot").unwrap().label, "Vectorscope YUV");
    h.state_mut().ui.scopes.scope = "histogram".into();
    h.run_steps(2);
    assert_eq!(h.state().auto.find("scopes.kind").unwrap().label, "Histogram");
}

#[test]
fn footage_panel_buttons_edit_into_the_comp() {
    let mut h = harness();
    let item = h.state().session.project.items.values().find(|i| matches!(i.kind, effectcraft_engine::project::ItemKind::Solid(_))).unwrap().id.0;
    h.state_mut().session.execute("footage.open", json!({"item": item})).unwrap();
    h.run_steps(4);
    assert!(h.state().ui.dock.contains(PanelKind::Footage) || h.state().ui.floating.iter().any(|f| f.panels.contains(&PanelKind::Footage)));
    assert!(h.state().auto.find("footage.overlayEdit").is_some());
    let before = h.state().session.active_comp().unwrap().layers.len();
    click(&mut h, "footage.overlayEdit");
    assert_eq!(h.state().session.active_comp().unwrap().layers.len(), before + 1);
    click(&mut h, "footage.rippleInsertEdit");
    assert!(h.state().session.active_comp().unwrap().layers.len() >= before + 2);
}

#[test]
fn progress_panel_cancels_a_job() {
    let mut h = harness();
    h.state_mut()
        .session
        .spawn_task("test", "Long analysis", false, |ctl| {
            while ctl.progress(1, 2) {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            Err("cancelled".into())
        })
        .unwrap();
    open(&mut h, "progress");
    let id = h.state().session.jobs()[0].id.clone();
    assert!(h.state().auto.find(&format!("progress.job.{id}")).is_some());
    click(&mut h, &format!("progress.cancel.{id}"));
    h.state_mut().session.wait_jobs();
    h.run_steps(2);
    assert!(h.state().session.jobs().is_empty());
    assert_eq!(h.state().session.job_log.last().unwrap().status, "cancelled");
}

fn click_at(h: &mut Harness<'_, EffectcraftApp>, p: Pos2) {
    h.input_mut().events.push(Event::PointerMoved(p));
    h.input_mut().events.push(Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.step();
    h.input_mut().events.push(Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(2);
}

fn hover(h: &mut Harness<'_, EffectcraftApp>, p: Pos2) {
    h.input_mut().events.push(Event::PointerMoved(p));
    h.run_steps(3);
}

/// Issue #320: saved workspaces last between sessions (they were only kept in memory): a new
/// app on the same settings store lists them and brings their layout back; renamed and deleted
/// ones stay that way.
#[test]
fn saved_workspaces_persist_across_sessions() {
    use effectcraft_engine::config::{ConfigStore, MemoryConfig};
    use effectcraft_ui_egui::menus::invoke;
    let store = std::sync::Arc::new(MemoryConfig::default());
    let launch = || {
        let s = Session { config: Some(store.clone()), ..Default::default() };
        let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).build_eframe(|_| EffectcraftApp::new(s));
        h.run_steps(2);
        h
    };
    let mut h = launch();
    let ctx = h.ctx.clone();
    invoke(h.state_mut(), &ctx, "window.workspace", json!({"name": "Minimal"})).unwrap();
    invoke(h.state_mut(), &ctx, "window.saveWorkspaceAs", json!({"name": "My Layout"})).unwrap();
    invoke(h.state_mut(), &ctx, "window.saveWorkspaceAs", json!({"name": "Scratch"})).unwrap();
    let saved = h.state().ui.dock.clone();
    drop(h);
    let mut h = launch();
    assert_eq!(h.state().saved_workspace_names(), ["My Layout", "Scratch"]);
    assert_eq!(h.state().ui.workspace, "Default");
    let ctx = h.ctx.clone();
    invoke(h.state_mut(), &ctx, "window.workspace", json!({"name": "My Layout"})).unwrap();
    assert_eq!(h.state().ui.dock, saved);
    invoke(h.state_mut(), &ctx, "window.editWorkspaces", json!({"name": "Scratch", "delete": true})).unwrap();
    invoke(h.state_mut(), &ctx, "window.editWorkspaces", json!({"name": "My Layout", "rename": "Edit Bay"})).unwrap();
    drop(h);
    assert_eq!(launch().state().saved_workspace_names(), ["Edit Bay"]);
    // A damaged file is ignored.
    store.write(effectcraft_ui_egui::WORKSPACES_FILE, "{not json").unwrap();
    assert!(launch().state().saved_workspace_names().is_empty());
}

/// Issue #191: a workspace saved with Save as New Workspace is listed in Window ▸ Workspace
/// (below the built-ins, where the submenu used to be cut off) and choosing it there brings its
/// layout back.
#[test]
fn saved_workspace_is_listed_in_the_workspace_menu() {
    let mut h = harness();
    let ctx = h.ctx.clone();
    effectcraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "window.workspace", json!({"name": "Minimal"})).unwrap();
    effectcraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "window.saveWorkspaceAs", json!({"name": "My Layout"})).unwrap();
    h.run_steps(3);
    let saved = h.state().ui.dock.clone();
    assert_eq!(h.state().saved_workspace_names(), ["My Layout"]);
    let cx = effectcraft_engine::menus::DynCtx { workspace: Some("My Layout"), saved_workspaces: &["My Layout".to_string()] };
    let (entries, _) = effectcraft_engine::menus::dynamic(&h.state().session, "savedWorkspaces", &cx);
    assert_eq!(entries.len(), 1);
    assert_eq!((entries[0].label.as_str(), entries[0].command.as_str(), &entries[0].params), ("My Layout", "window.workspace", &json!({"name": "My Layout"})));
    // Leave it, then pick it from the in-window menu bar.
    effectcraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "window.workspace", json!({"name": "Default"})).unwrap();
    h.run_steps(3);
    assert_ne!(h.state().ui.dock, saved);
    click(&mut h, "menu.Window");
    let ws = h.query_by_label(" Workspace ⏵").expect("Window ▸ Workspace").rect();
    hover(&mut h, ws.center());
    let entry = h.state().auto.find("menu.savedWorkspaces.0").expect("the saved workspace is listed").clone();
    assert_eq!(entry.label, "My Layout");
    let at = h.query_by_label(" My Layout").expect("the saved workspace entry").rect().center();
    // Into the submenu, then down it (as a pointer moves).
    hover(&mut h, pos2(at.x, ws.center().y));
    for k in 1..=10 {
        hover(&mut h, pos2(at.x, ws.center().y + (at.y - ws.center().y) * k as f32 / 10.0));
    }
    click_at(&mut h, at);
    assert_eq!(h.state().ui.workspace, "My Layout");
    assert_eq!(h.state().ui.dock, saved, "its layout came back");
}

/// Edit ▸ Label shows each label's colour before its name (#290), and choosing one labels the
/// selected layer.
#[test]
fn edit_label_menu_shows_the_label_colours() {
    let mut h = harness();
    click(&mut h, "menu.Edit");
    let label = h.query_by_label(" Label ⏵").expect("Edit ▸ Label").rect();
    hover(&mut h, label.center());
    // (Not Dark Green; the entry's label starts with its check-mark column.)
    let green = h.query_by_label(" Green").expect("the Label submenu is open").rect();
    let col = h.state().tokens.label(effectcraft_engine::color::Label::Green);
    let swatch = |s: &egui::Shape| matches!(s, egui::Shape::Rect(r) if r.fill == col && r.rect.width() < 16.0 && green.contains(r.rect.center()));
    let shapes: Vec<egui::Shape> = h.output().shapes.iter().map(|c| c.shape.clone()).collect();
    assert!(shapes.iter().any(|s| swatch(s) || matches!(s, egui::Shape::Vec(v) if v.iter().any(swatch))), "no Green swatch");
    // Into the submenu, then down to Green.
    hover(&mut h, pos2(green.center().x, label.center().y));
    for k in 1..=10 {
        hover(&mut h, pos2(green.center().x, label.center().y + (green.center().y - label.center().y) * k as f32 / 10.0));
    }
    click_at(&mut h, green.center());
    assert_eq!(h.state().session.active_comp().unwrap().layers[0].label, effectcraft_engine::color::Label::Green);
}

/// Learn opens the Home screen's tutorials; clicking another workspace tab closes them again,
/// however quickly the tabs are clicked (they stayed over every workspace, #272).
#[test]
fn leaving_the_learn_workspace_closes_its_tutorials() {
    let mut h = harness();
    for name in ["Learn", "Default", "Learn", "Review", "Learn", "Learn", "Small Screen", "Standard", "Learn", "Default"] {
        click(&mut h, &format!("header.workspace.{name}"));
        assert_eq!(h.state().ui.workspace, name);
        let learn = name == "Learn";
        assert_eq!(h.state().ui.start_screen, learn, "{name}: the Home screen shows only for Learn");
        assert_eq!(!h.state().auto.query("home.learn.").is_empty(), learn, "{name}: the tutorials show only for Learn");
    }
}

/// #284: Settings ▸ Appearance ▸ UI Scale sizes the whole interface as soon as it is chosen,
/// Cancel puts it back, OK keeps it; Ctrl+0 (Window ▸ Project) doesn't reset it.
#[test]
fn ui_scale_setting_sizes_the_interface() {
    let mut h = harness();
    let ctx = h.ctx.clone();
    let choose = |h: &mut Harness<'_, EffectcraftApp>, label: &str| {
        let ctx = h.ctx.clone();
        effectcraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "app.settings", json!({"page": "appearance"})).unwrap();
        h.run_steps(3);
        click(h, "settings.appearance.uiScale");
        let option = h.query_by_label(label).unwrap_or_else(|| panic!("the {label} choice")).rect().center();
        click_at(h, option);
        h.run_steps(2);
    };
    assert_eq!(ctx.zoom_factor(), 1.0);
    choose(&mut h, "150%");
    assert_eq!(h.state().session.prefs.appearance.ui_scale, 150);
    assert_eq!(ctx.zoom_factor(), 1.5, "live");
    click(&mut h, "settings.cancel");
    h.run_steps(2);
    assert_eq!((h.state().session.prefs.appearance.ui_scale, ctx.zoom_factor()), (100, 1.0), "Cancel put it back");
    choose(&mut h, "125%");
    click(&mut h, "settings.ok");
    h.run_steps(2);
    assert_eq!(h.state().dialog, None);
    assert_eq!((h.state().session.prefs.appearance.ui_scale, ctx.zoom_factor()), (125, 1.25), "OK kept it");
    // Ctrl+0 opens the Project panel; egui's own Ctrl+0 zoom reset stays out of it.
    for pressed in [true, false] {
        h.input_mut().events.push(Event::Key { key: egui::Key::Num0, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::COMMAND });
        h.step();
    }
    h.run_steps(2);
    assert_eq!(ctx.zoom_factor(), 1.25);
    // A saved scale applies from the first frame.
    let mut s = Session::default();
    s.prefs.set("appearance.uiScale", json!(175)).unwrap();
    let mut h2 = Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).build_eframe(|_| EffectcraftApp::new(s));
    h2.run_steps(3);
    assert_eq!(h2.ctx.zoom_factor(), 1.75);
}

/// Headless look at the panels (wgpu offscreen; needs a GPU adapter). Run with
/// `PANELS_SNAPSHOT=/abs/dir cargo test -p effectcraft-ui-egui --test ui_panels -- --ignored`.
#[test]
#[ignore]
fn panels_snapshot() {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    let solid = s.project.items.values().find(|i| matches!(i.kind, effectcraft_engine::project::ItemKind::Solid(_))).map(|i| i.id.0);
    let mut app = Some(EffectcraftApp::new(s));
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).build_eframe(|_| app.take().expect("app"));
    h.run_steps(3);
    let ctx = h.ctx.clone();
    effectcraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "window.workspace", json!({"name": "All Panels"})).unwrap();
    h.run_steps(3);
    let dir = std::env::var("PANELS_SNAPSHOT").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/test-out").into());
    std::fs::create_dir_all(&dir).unwrap();
    if let Some(id) = solid {
        h.state_mut().session.execute("footage.open", json!({"item": id})).unwrap();
    }
    for (panel, scope) in [
        ("lumetriScopes", "waveformRgb"),
        ("lumetriScopes", "vectorscopeYuv"),
        ("lumetriScopes", "histogram"),
        ("lumetriScopes", "paradeRgb"),
        ("contentAwareFill", ""),
        ("metadata", ""),
        ("progress", ""),
        ("mediaBrowser", ""),
        ("footage", ""),
    ] {
        if !scope.is_empty() {
            h.state_mut().ui.scopes.scope = scope.into();
        }
        open(&mut h, panel);
        h.run_steps(3);
        let img = h.render().expect("render");
        img.save(format!("{dir}/{panel}{scope}.png")).unwrap();
    }
}

/// Drag the automation element `id` horizontally by `dx` points.
fn drag_x(h: &mut Harness<'_, EffectcraftApp>, id: &str, dx: f32) {
    let e = h.state().auto.find(id).unwrap_or_else(|| panic!("no {id}")).clone();
    let from: Pos2 = pos2(e.rect[0] + e.rect[2] / 2.0, e.rect[1] + e.rect[3] / 2.0);
    h.input_mut().events.push(Event::PointerMoved(from));
    h.step();
    h.input_mut().events.push(Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.step();
    for k in 1..=10 {
        h.input_mut().events.push(Event::PointerMoved(from + egui::vec2(dx * k as f32 / 10.0, 0.0)));
        h.step();
    }
    let to = from + egui::vec2(dx, 0.0);
    h.input_mut().events.push(Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(3);
}

/// #338: an Essential Graphics Scale control keeps After Effects' Constrain Proportions link, on
/// by default: dragging X scales Y with it; unlinked, X moves alone.
#[test]
fn essential_graphics_scale_keeps_constrain_proportions() {
    let mut h = harness();
    let layer = h.state().session.active_comp().unwrap().layers[0].id.0;
    h.state_mut().session.execute("prop.set", json!({"layer": layer, "path": "transform/scale", "value": [100.0, 50.0]})).unwrap();
    let c =
        h.state_mut().session.execute("essential.addProperty", json!({"layer": layer, "path": "transform/scale"})).unwrap()["controls"][0].as_u64().unwrap();
    open(&mut h, "essentialGraphics");
    let value = format!("essential.control.{c}.value");
    let scale = |h: &Harness<'_, EffectcraftApp>| {
        let comp = h.state().session.active_comp().unwrap();
        let l = comp.layers.iter().find(|l| l.id.0 == layer).unwrap();
        l.props.prop("transform/scale").unwrap().value.components()
    };
    assert!(h.state().auto.find(&format!("{value}.link")).is_some(), "the chain link is drawn");
    drag_x(&mut h, &format!("{value}.0"), 40.0);
    let linked = scale(&h);
    assert!(linked[0] > 100.0, "{linked:?}");
    assert!((linked[1] / linked[0] - 0.5).abs() < 1e-6, "Y follows X in proportion: {linked:?}");
    click(&mut h, &format!("{value}.link"));
    drag_x(&mut h, &format!("{value}.0"), 40.0);
    let unlinked = scale(&h);
    assert!(unlinked[0] > linked[0] && unlinked[1] == linked[1], "unlinked, X moves alone: {unlinked:?}");
}
