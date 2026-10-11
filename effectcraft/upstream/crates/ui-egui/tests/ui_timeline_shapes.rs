//! Timeline rows for shape layers, masks and keyframing: the Contents "Add:" menu and the Classic
//! 3D "Change Renderer…" row (#206), Alt+Shift+P revealing the key it adds and clicks below the
//! layers deselecting them (#205), Mask Feather's linked values (#203), and copying shape items
//! between shape layers (#227).

use effectcraft_engine::Session;
use effectcraft_engine::project::LayerId;
use effectcraft_ui_egui::{Dialog, EffectcraftApp};
use egui::{Event, Key, Modifiers, PointerButton, Pos2, pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

fn harness(s: Session) -> Harness<'static, EffectcraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).build_eframe(|_| EffectcraftApp::new(s));
    h.run_steps(3);
    h
}

fn session() -> Session {
    let mut s = Session::default();
    s.execute("comp.new", json!({"name": "Main", "width": 320, "height": 180, "duration": 4})).unwrap();
    s
}

fn center(h: &Harness<'_, EffectcraftApp>, id: &str) -> Pos2 {
    let e = h.state().auto.find(id).unwrap_or_else(|| panic!("no {id}"));
    pos2(e.rect[0] + e.rect[2] / 2.0, e.rect[1] + e.rect[3] / 2.0)
}

fn click_at(h: &mut Harness<'_, EffectcraftApp>, p: Pos2) {
    h.input_mut().events.push(Event::PointerMoved(p));
    h.step();
    h.input_mut().events.push(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.input_mut().events.push(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
}

fn click(h: &mut Harness<'_, EffectcraftApp>, id: &str) {
    let p = center(h, id);
    click_at(h, p);
}

fn open_layer(h: &mut Harness<'_, EffectcraftApp>, id: u64) {
    h.state_mut().ui.timeline.open_layers.insert(id);
    h.run_steps(2);
}

#[test]
fn shape_contents_add_menu_adds_path_operations() {
    let mut s = session();
    let l = s.execute("layer.newShape", json!({"kind": "ellipse"})).unwrap()["layer"].as_u64().unwrap();
    let contents = s.active_comp().unwrap().layer(LayerId(l)).unwrap().props.sub("contents").unwrap().uid;
    let mut h = harness(s);
    open_layer(&mut h, l);
    click(&mut h, &format!("timeline.group.{contents}.add"));
    let entry = h.get_by_label("Trim Paths").rect();
    assert!(entry.max.y <= 1000.0, "the whole menu fits in the window: {entry:?}");
    click_at(&mut h, entry.center());
    let layer = h.state().session.active_comp().unwrap().layer(LayerId(l)).unwrap().clone();
    let items: Vec<&str> = layer.props.sub("contents").unwrap().groups().map(|g| g.match_id.as_str()).collect();
    assert_eq!(items, ["group", "trim"]);
}

#[test]
fn classic_3d_shape_layers_offer_change_renderer() {
    let mut s = session();
    let l = s.execute("layer.newShape", json!({"kind": "rect"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.setSwitch", json!({"layers": [l], "switch": "threeD", "value": true})).unwrap();
    let mut h = harness(s);
    open_layer(&mut h, l);
    click(&mut h, &format!("timeline.layer.{l}.changeRenderer"));
    assert_eq!(h.state().dialog, Some(Dialog::CompSettings));
    assert!(h.state().auto.find("dialog.comp.renderer").is_some(), "on the 3D Renderer tab");
    // Advanced 3D shows the extrusion options instead.
    h.state_mut().dialog = None;
    h.state_mut().session.execute("comp.renderer", json!({"renderer": "advanced3d"})).unwrap();
    h.run_steps(3);
    assert!(h.state().auto.find(&format!("timeline.layer.{l}.changeRenderer")).is_none());
}

#[test]
fn add_position_key_reveals_position() {
    let mut s = session();
    let l = s.execute("layer.newSolid", json!({"color": "#808080"})).unwrap()["layer"].as_u64().unwrap();
    let mut h = harness(s);
    let ctx = h.ctx.clone();
    effectcraft_ui_egui::menus::invoke(h.state_mut(), &ctx, "timeline.keyAt.position", json!({})).unwrap();
    h.run_steps(2);
    let app = h.state();
    let position = app.session.active_comp().unwrap().layer(LayerId(l)).unwrap().props.prop("transform/position").unwrap().clone();
    assert_eq!(position.keys.len(), 1);
    assert_eq!(app.ui.timeline.layer_reveal.get(&l), Some(&vec!["position".to_string()]));
    assert!(app.auto.find(&format!("timeline.prop.{}.value.0", position.uid)).is_some(), "the Position row shows");
}

#[test]
fn clicking_below_the_layers_deselects_them() {
    let mut s = session();
    let l = s.execute("layer.newSolid", json!({"color": "#808080"})).unwrap()["layer"].as_u64().unwrap();
    let mut h = harness(s);
    assert_eq!(h.state().session.state.selected_layers, vec![LayerId(l)]);
    // In the time graph, below the only layer.
    let bar = center(&h, &format!("timeline.layer.{l}.bar"));
    click_at(&mut h, bar + egui::vec2(0.0, 80.0));
    assert!(h.state().session.state.selected_layers.is_empty());
}

#[test]
fn mask_feather_values_are_linked_and_not_negative() {
    let mut s = session();
    let l = s.execute("layer.newSolid", json!({"color": "#808080"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.addMask", json!({"layer": l, "shape": "rect"})).unwrap();
    let feather = |h: &Harness<'_, EffectcraftApp>| {
        let p = h.state().session.active_comp().unwrap().layer(LayerId(l)).unwrap().props.prop("masks/#1/feather").unwrap().clone();
        (p.uid, p.value.components())
    };
    let mut h = harness(s);
    h.state_mut().ui.timeline.open_layers.insert(l);
    h.state_mut().ui.timeline.layer_reveal.insert(l, vec!["feather".into()]);
    h.run_steps(2);
    let (uid, _) = feather(&h);
    let type_value = |h: &mut Harness<'_, EffectcraftApp>, d: usize, v: &str| {
        click(h, &format!("timeline.prop.{uid}.value.{d}"));
        h.input_mut().events.push(Event::Text(v.into()));
        h.step();
        h.input_mut().events.push(Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
        h.run_steps(3);
    };
    type_value(&mut h, 0, "12");
    assert_eq!(feather(&h).1, vec![12.0, 12.0], "linked by default");
    type_value(&mut h, 1, "-5");
    assert_eq!(feather(&h).1, vec![0.0, 0.0], "never negative");
    click(&mut h, &format!("timeline.prop.{uid}.link"));
    type_value(&mut h, 1, "7");
    assert_eq!(feather(&h).1, vec![0.0, 7.0], "unlinked");
}

/// Two shape layers, "Rect" (Rectangle 1) and "Oval" (Ellipse 1, selected): (harness with Rect's
/// Contents open, Rect, Oval, Rectangle 1's uid).
fn two_shape_layers() -> (Harness<'static, EffectcraftApp>, u64, u64, u64) {
    let mut s = session();
    let a = s.execute("layer.newShape", json!({"kind": "rect", "name": "Rect"})).unwrap()["layer"].as_u64().unwrap();
    let b = s.execute("layer.newShape", json!({"kind": "ellipse", "name": "Oval"})).unwrap()["layer"].as_u64().unwrap();
    let contents = s.active_comp().unwrap().layer(LayerId(a)).unwrap().props.sub("contents").unwrap().clone();
    let rect = contents.groups().next().unwrap().uid;
    let mut h = harness(s);
    h.state_mut().ui.timeline.open_groups.insert(contents.uid);
    open_layer(&mut h, a);
    (h, a, b, rect)
}

fn contents_names(h: &Harness<'_, EffectcraftApp>, l: u64) -> Vec<String> {
    h.state().session.active_comp().unwrap().layer(LayerId(l)).unwrap().props.sub("contents").unwrap().groups().map(|g| g.name.clone()).collect()
}

#[test]
fn ctrl_c_and_ctrl_v_copy_a_shape_group_into_another_shape_layer() {
    let (mut h, a, b, rect) = two_shape_layers();
    click(&mut h, &format!("timeline.group.{rect}.name"));
    assert_eq!(h.state().session.state.selected_props, vec![(LayerId(a), rect)]);
    h.input_mut().events.push(Event::Copy);
    h.step();
    click(&mut h, &format!("timeline.layer.{b}.row"));
    assert_eq!(h.state().session.state.selected_layers, vec![LayerId(b)]);
    h.input_mut().events.push(Event::Paste("EffectCraft: 1 shape item".into()));
    h.run_steps(3);
    assert_eq!(h.state().session.active_comp().unwrap().layers.len(), 2, "no new layer");
    assert_eq!(contents_names(&h, b), ["Rectangle 1", "Ellipse 1"]);
    assert_eq!(contents_names(&h, a), ["Rectangle 1"]);
}

#[test]
fn ctrl_d_duplicates_the_selected_shape_group_in_its_layer() {
    let (mut h, a, _, rect) = two_shape_layers();
    click(&mut h, &format!("timeline.group.{rect}.name"));
    h.input_mut().events.push(Event::Key { key: Key::D, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND });
    h.run_steps(3);
    assert_eq!(h.state().session.active_comp().unwrap().layers.len(), 2, "no new layer");
    assert_eq!(contents_names(&h, a), ["Rectangle 2", "Rectangle 1"]);
}

#[test]
fn toolbar_stroke_edits_the_selected_shape_layer() {
    let mut s = session();
    let l = s.execute("layer.newShape", json!({"kind": "rect", "fill": "#ff0000"})).unwrap()["layer"].as_u64().unwrap();
    let mut h = harness(s);
    // The Selection tool is active: the options show for the selected shape layer.
    click(&mut h, "header.strokeWidth");
    h.input_mut().events.push(Event::Text("4".into()));
    h.step();
    h.input_mut().events.push(Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    let layer = h.state().session.active_comp().unwrap().layer(LayerId(l)).unwrap().clone();
    let width = layer.props.prop("contents/group/contents/stroke/width").map(|p| p.value.as_f64());
    assert_eq!(width, Some(4.0), "a stroke was added with the toolbar's width");
    assert_eq!(h.state().session.state.shape_tool.stroke_width, 4.0);
}
