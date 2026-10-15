//! Built-in animation presets (Effects & Presets ▸ * Animation Presets ▸ Transitions).

use effectcraft_time::Tick;
use serde_json::json;

use crate::Session;
use crate::render::RenderOpts;

/// A 160×90 comp with one white full-frame solid; returns (session, solid).
fn session() -> (Session, u64) {
    let mut s = Session::default();
    s.execute("comp.new", json!({"name": "T", "width": 160, "height": 90, "frameRate": 30, "duration": 5})).unwrap();
    let solid = s.execute("layer.newSolid", json!({"name": "S", "color": "#ffffff"})).unwrap()["layer"].as_u64().unwrap();
    (s, solid)
}

/// Share of the comp's pixels that are mostly opaque at `secs`.
fn coverage(s: &Session, secs: f64) -> f64 {
    let cid = s.active_comp_id().unwrap();
    let img = s.render(cid, Tick::from_seconds_f64(secs), RenderOpts { scale: 1.0, ..Default::default() });
    img.data.iter().filter(|p| p[3] > 0.5).count() as f64 / img.data.len().max(1) as f64
}

#[test]
fn every_transition_preset_applies_and_transitions() {
    let (mut s, solid) = session();
    let list = s.execute("anim.builtinPresets", json!({"category": "Transitions"})).unwrap();
    let list = list.as_array().unwrap().clone();
    assert!(list.len() >= 20, "{}", list.len());
    assert_eq!(crate::animation_presets().len(), list.len());
    let start = 0.5;
    for p in &list {
        let id = p["id"].as_str().unwrap();
        let incoming = match p["direction"].as_str() {
            Some("in") => true,
            Some("out") => false,
            d => panic!("{id}: direction {d:?}"),
        };
        s.execute("time.set", json!({"time": start})).unwrap();
        let r = s.execute("anim.applyBuiltinPreset", json!({"layers": [solid], "preset": id})).unwrap();
        assert_eq!(r["skipped"], json!([]), "{id}: every path in the preset exists and takes its values");
        let (a, b) = (coverage(&s, start), coverage(&s, start + 2.0));
        let (hidden, shown) = if incoming { (a, b) } else { (b, a) };
        assert!(hidden < 0.15, "{id}: hidden end covers {hidden:.3}");
        assert!(shown > 0.97, "{id}: shown end covers {shown:.3}");
        // Keys start at the current time: before it the layer holds the first key's state.
        assert!((coverage(&s, 0.0) - a).abs() < 0.02, "{id}");
        s.execute("edit.undo", json!({})).unwrap();
        let l = &s.active_comp().unwrap().layers[0];
        assert_eq!(l.effects().map_or(0, |fx| fx.groups().count()), 0, "{id}: undo removes the effects");
        assert!(l.props.prop("transform/opacity").unwrap().keys.is_empty(), "{id}: undo removes the keys");
    }
}

#[test]
fn transition_preset_by_name_and_bad_input() {
    let (mut s, solid) = session();
    // By name; a second application numbers the effect like Effect ▸ apply does.
    s.execute("anim.applyBuiltinPreset", json!({"layers": [solid], "preset": "Wipe In from Left"})).unwrap();
    let r = s.execute("anim.applyBuiltinPreset", json!({"layers": [solid], "preset": "wipeInFromLeft"})).unwrap();
    assert_eq!(r["effects"].as_array().unwrap().len(), 1);
    let names: Vec<String> = s.active_comp().unwrap().layers[0].effects().unwrap().groups().map(|g| g.name.clone()).collect();
    assert_eq!(names, ["Linear Wipe", "Linear Wipe 2"]);
    let fx = s.active_comp().unwrap().layers[0].effects().unwrap().groups().next().unwrap().clone();
    let completion = fx.prop("completion").unwrap();
    assert_eq!(completion.keys.len(), 2);
    assert_eq!(completion.keys[0].time, Tick::ZERO);
    // Unknown presets and missing params are errors, not panics.
    assert!(s.execute("anim.applyBuiltinPreset", json!({"layers": [solid], "preset": "nope"})).is_err());
    assert!(s.execute("anim.applyBuiltinPreset", json!({"layers": [solid]})).is_err());
    // A layer without effects (a camera) refuses effect presets.
    let cam = s.execute("layer.newCamera", json!({})).unwrap()["layer"].as_u64().unwrap();
    assert!(s.execute("anim.applyBuiltinPreset", json!({"layers": [cam], "preset": "clockReveal"})).is_err());
}
