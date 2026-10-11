//! Headless UI checks for the 3D viewer (egui_kittest). The `snapshot` test renders the window
//! with wgpu and writes PNGs when `EC_SNAPSHOT_DIR` is set (`cargo test -p effectcraft-ui-egui
//! --test ui_3d -- --ignored`); the other test only runs the UI logic.

use effectcraft_engine::Session;
use effectcraft_ui_egui::EffectcraftApp;
use egui_kittest::Harness;
use serde_json::json;

fn app() -> EffectcraftApp {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s.execute("comp.open", json!({"comp": "3D Showcase"})).unwrap();
    s.execute("time.set", json!({"time": 5.0})).unwrap();
    EffectcraftApp::new(s)
}

/// Step the UI until background frame renders have landed.
fn settle(h: &mut Harness<'_, EffectcraftApp>) {
    for _ in 0..600 {
        h.step();
        if h.state().frames.inflight() == 0 && h.state().frames.last_ms.lock().map(|v| *v > 0.0).unwrap_or(false) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    for _ in 0..4 {
        h.step();
    }
}

#[test]
fn viewer_registers_3d_controls() {
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).build_eframe(|_| app());
    settle(&mut h);
    h.step();
    let ids: Vec<String> = h.state().auto.previous.iter().chain(h.state().auto.elements.iter()).map(|e| e.id.clone()).collect();
    for want in ["viewer.view3d", "viewer.renderer3d"] {
        assert!(ids.iter().any(|i| i == want), "missing {want}");
    }
    assert!(ids.iter().any(|i| i.starts_with("viewer.light.")), "light wireframe registered");
    // In a custom view the active camera is drawn as a wireframe too.
    h.state_mut().session.execute("view.3d.custom1", json!({})).unwrap();
    settle(&mut h);
    h.step();
    let ids: Vec<String> = h.state().auto.previous.iter().chain(h.state().auto.elements.iter()).map(|e| e.id.clone()).collect();
    assert!(ids.iter().any(|i| i.starts_with("viewer.camera.")), "camera wireframe registered");
}

/// Click the centre of the automation element `id`.
fn click_id(h: &mut Harness<'_, EffectcraftApp>, id: &str) {
    let [x, y, w, hh] = h.state().auto.find(id).unwrap_or_else(|| panic!("{id} is registered")).rect;
    let p = egui::pos2(x + w / 2.0, y + hh / 2.0);
    h.input_mut().events.push(egui::Event::PointerMoved(p));
    h.input_mut().events.push(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.step();
    h.input_mut().events.push(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(3);
}

/// #307: the viewer's 3D Renderer menu shows the comp's renderer (it always read "Classic 3D")
/// and switches it, as After Effects' Composition panel does.
#[test]
fn renderer_menu_shows_and_switches_the_comp_renderer() {
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).build_eframe(|_| app());
    h.state_mut().session.execute("comp.settings", json!({"renderer": "classic3D"})).unwrap();
    h.run_steps(3);
    let label = |h: &Harness<'_, EffectcraftApp>| h.state().auto.find("viewer.renderer3d").map(|e| e.label.clone());
    assert_eq!(label(&h).as_deref(), Some("Classic 3D"));
    click_id(&mut h, "viewer.renderer3d");
    click_id(&mut h, "viewer.renderer3dItem.1");
    let renderer = h.state().session.active_comp().unwrap().renderer;
    assert_eq!(renderer, effectcraft_engine::project::Renderer::Advanced3D);
    h.run_steps(2);
    assert_eq!(label(&h).as_deref(), Some("Advanced 3D"));
    // One undoable Composition Settings step.
    h.state_mut().session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(h.state().session.active_comp().unwrap().renderer, effectcraft_engine::project::Renderer::Classic3D);
}

#[test]
fn camera_and_light_dialogs_create_layers() {
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).build_eframe(|_| app());
    h.step();
    let ctx = h.ctx.clone();
    let n0 = h.state().session.active_comp().unwrap().layers.len();
    effectcraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "layer.newCamera", json!({})).unwrap();
    assert_eq!(h.state().dialog, Some(effectcraft_ui_egui::Dialog::CameraSettings));
    h.step();
    let ok = h.state().auto.elements.iter().chain(h.state().auto.previous.iter()).find(|e| e.id == "dialog.camera.ok").map(|e| e.rect);
    assert!(ok.is_some(), "camera dialog OK registered");
    // OK through the dialog state (Enter).
    h.key_press(egui::Key::Enter);
    h.step();
    assert!(h.state().dialog.is_none());
    assert_eq!(h.state().session.active_comp().unwrap().layers.len(), n0 + 1);
    effectcraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "layer.newLight", json!({})).unwrap();
    assert_eq!(h.state().dialog, Some(effectcraft_ui_egui::Dialog::LightSettings));
    h.step();
    h.key_press(egui::Key::Enter);
    h.step();
    assert_eq!(h.state().session.active_comp().unwrap().layers.len(), n0 + 2);
    // Layer Settings on the new light reopens Light Settings for it.
    effectcraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "layer.settings", json!({})).unwrap();
    assert_eq!(h.state().dialog, Some(effectcraft_ui_egui::Dialog::LightSettings));
}

#[test]
#[ignore = "renders with wgpu; run with --ignored (set EC_SNAPSHOT_DIR to keep PNGs)"]
fn snapshot() {
    let dir = std::env::var("EC_SNAPSHOT_DIR").ok();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).with_pixels_per_point(1.0).wgpu().build_eframe(|_| app());
    let shots: [(&str, Option<&str>); 4] =
        [("active", None), ("custom1", Some("view.3d.custom1")), ("top", Some("view.3d.top")), ("left", Some("view.3d.left"))];
    for (name, cmd) in shots {
        if let Some(c) = cmd {
            h.state_mut().session.execute(c, json!({})).unwrap();
        }
        settle(&mut h);
        let img = h.render().expect("render");
        if let Some(d) = &dir {
            img.save(format!("{d}/ui3d_{name}.png")).unwrap();
        }
    }
    let ctx = h.ctx.clone();
    effectcraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "layer.newCamera", json!({})).unwrap();
    h.run_steps(3);
    let img = h.render().expect("render");
    if let Some(d) = &dir {
        img.save(format!("{d}/ui3d_camera_dialog.png")).unwrap();
    }
    h.state_mut().dialog = None;
    effectcraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "layer.newLight", json!({})).unwrap();
    h.run_steps(3);
    let img = h.render().expect("render");
    if let Some(d) = &dir {
        img.save(format!("{d}/ui3d_light_dialog.png")).unwrap();
    }
    // A 3D layer's rotation properties (R) and the selected layer's gizmo in the active camera.
    h.state_mut().dialog = None;
    h.state_mut().session.execute("view.3d.activeCamera", json!({})).unwrap();
    h.state_mut().session.execute("layer.select", json!({"layers": ["Card Blue"]})).unwrap();
    effectcraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "timeline.reveal.rotation", json!({})).unwrap();
    settle(&mut h);
    let img = h.render().expect("render");
    if let Some(d) = &dir {
        img.save(format!("{d}/ui3d_reveal_rotation.png")).unwrap();
    }
}

/// Settings ▸ 3D ▸ Extended Viewer: a custom 3D view renders the pasteboard around the comp
/// frame, and 3D layers reaching past the frame draw pixels there.
#[test]
fn extended_viewer_renders_past_the_comp_frame() {
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).build_eframe(|_| app());
    h.state_mut().session.execute("view.3d.custom1", json!({})).unwrap();
    // Zoom out so the viewer shows plenty of pasteboard.
    h.state_mut().ui.viewer.zoom = Some(0.15);
    settle(&mut h);
    assert!(h.state().ui.viewer.extended.is_none(), "off by default");
    let ids: Vec<String> = h.state().auto.previous.iter().chain(h.state().auto.elements.iter()).map(|e| e.id.clone()).collect();
    assert!(ids.iter().any(|i| i == "viewer.extendedViewer"), "toggle registered");
    assert_eq!(h.state_mut().session.execute("view.extendedViewer", json!({"value": true})).unwrap(), json!(true));
    h.step();
    settle(&mut h);
    h.step();
    settle(&mut h);
    let region = h.state().ui.viewer.extended.expect("extended region in a custom view");
    let (cw, ch) = {
        let c = h.state().session.active_comp().unwrap();
        (c.width as f64, c.height as f64)
    };
    assert!(region[0] < 0.0 && region[1] < 0.0 && region[0] + region[2] > cw && region[1] + region[3] > ch, "{region:?}");
    let ids: Vec<String> = h.state().auto.previous.iter().chain(h.state().auto.elements.iter()).map(|e| e.id.clone()).collect();
    assert!(ids.iter().any(|i| i == "viewer.extendedArea"));
    let img = h.state_mut().viewer_pixels().expect("frame");
    let s = img.size[0] as f64 / region[2];
    assert!((img.size[1] as f64 / region[3] - s).abs() < 0.05, "frame covers the region");
    // Opaque pixels outside the comp rectangle.
    let (fx0, fy0) = (-region[0] * s, -region[1] * s);
    let (fx1, fy1) = (fx0 + cw * s, fy0 + ch * s);
    let mut outside = 0;
    for y in 0..img.size[1] {
        for x in 0..img.size[0] {
            let (xf, yf) = (x as f64 + 0.5, y as f64 + 0.5);
            if (xf < fx0 - 1.0 || xf > fx1 + 1.0 || yf < fy0 - 1.0 || yf > fy1 + 1.0) && img.pixels[y * img.size[0] + x].a() > 0 {
                outside += 1;
            }
        }
    }
    assert!(outside > 50, "pixels drawn on the pasteboard: {outside}");
    // The Active Camera view (Draft 3D off) shows just the frame.
    h.state_mut().session.execute("view.3d.activeCamera", json!({})).unwrap();
    h.step();
    h.step();
    assert!(h.state().ui.viewer.extended.is_none());
}
