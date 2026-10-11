//! Scenario 1 — kinetic typography: text layers with per-character animators, range and wiggly
//! selectors, expressions (wiggle, valueAtTime, pick-whip links), motion blur, a time-remapped
//! precomp; rendered to an H.264 MP4 and a PNG sequence, both checked by re-importing them.

use serde_json::json;

use crate::harness::{Qa, centroid, coverage, luma, mean_diff};

#[test]
fn kinetic_typography() {
    let mut qa = Qa::new("kinetic");
    let comp = qa.exec("comp.new", json!({"name": "Kinetic", "width": 160, "height": 90, "frameRate": 12, "duration": 2}))["comp"].clone();

    // Title: named at creation (one call), characters fly in from above with a range selector.
    let title = qa.exec("layer.newText", json!({"text": "MOVE", "name": "Title", "size": 36, "fill": "#ffffff"}))["layer"].clone();
    assert!(title.is_u64(), "layer.newText returns the new layer's id");
    let anim = qa.exec("layer.addTextAnimator", json!({"layer": "Title", "properties": ["position", "opacity"]}));
    assert!(anim["animator"].is_u64());
    qa.tool("set_property", json!({"layer": "Title", "path": "text/animators/#1/properties/position", "value": [0, -60, 0]}));
    qa.tool("set_property", json!({"layer": "Title", "path": "text/animators/#1/properties/opacity", "value": 0}));
    let off = qa.tool(
        "add_keyframe",
        json!({"layer": "Title", "path": "text/animators/#1/selectors/#1/offset", "keys": [{"time": 0, "value": 0}, {"time": 1, "value": 100}], "interpolation": "easyEase"}),
    );
    assert_eq!(off["keys"].as_array().unwrap().len(), 2);
    assert_eq!(off["keys"][0]["out"], "Bezier", "easy ease makes Bezier keys");
    // A second, wiggly selector (Mode: Intersect) jitters the amounts; pinned to 100 % here so
    // frames stay predictable, then checked to really wiggle once released.
    let wig = qa.exec("layer.addTextSelector", json!({"layer": "Title", "animator": 1, "kind": "wiggly"}));
    assert!(wig["selector"].is_u64());
    qa.tool("set_property", json!({"layer": "Title", "path": "text/animators/#1/selectors/wigglySelector/minAmount", "value": 100}));
    qa.tool("add_keyframe", json!({"layer": "Title", "path": "transform/position", "keys": [{"time": 0, "value": [40, 50]}, {"time": 2, "value": [120, 50]}]}));
    qa.exec("layer.setSwitch", json!({"layers": ["Title"], "switch": "motionBlur", "value": true}));
    assert_eq!(qa.exec("comp.setSwitch", json!({"switch": "motionBlur", "value": true})), json!(true));

    // Sub line: pick-whip link, wiggle, valueAtTime.
    qa.exec("layer.newText", json!({"text": "type", "name": "Sub", "size": 18, "fill": "#ffcc00"}));
    let pos =
        qa.tool("set_property", json!({"layer": "Sub", "path": "transform/position", "expression": "thisComp.layer(\"Title\").transform.position + [0, 30]"}));
    assert_eq!(pos["evaluated"], json!([40.0, 80.0, 0.0]), "the reply shows the expression's result: {pos}");
    qa.tool("set_property", json!({"layer": "Sub", "path": "transform/rotation", "expression": "wiggle(4, 20)"}));
    qa.tool(
        "set_property",
        json!({"layer": "Sub", "path": "transform/scale", "expression": "var x = thisComp.layer(\"Title\").transform.position.valueAtTime(time - 0.5)[0]; [x, x]"}),
    );
    let p = qa.tool("get_property", json!({"layer": "Sub", "path": "transform/position", "time": 1}));
    assert_eq!(p["evaluated"], json!([80.0, 80.0, 0.0]), "{p}");
    let r1 = qa.tool("get_property", json!({"layer": "Sub", "path": "transform/rotation", "time": 1}))["evaluated"].as_f64().unwrap();
    let r2 = qa.tool("get_property", json!({"layer": "Sub", "path": "transform/rotation", "time": 1.5}))["evaluated"].as_f64().unwrap();
    assert!(r1 != r2 && r1.abs() <= 20.0 && r2.abs() <= 20.0, "wiggle moves within its amplitude: {r1} {r2}");
    let s = qa.tool("get_property", json!({"layer": "Sub", "path": "transform/scale", "time": 1.5}));
    assert_eq!(s["evaluated"], json!([80.0, 80.0, 100.0]), "valueAtTime(time - 0.5) of the title's x: {s}");
    let bad = qa.tool("set_property", json!({"layer": "Sub", "path": "transform/opacity", "expression": "thisComp.layer(\"Nope\").transform.opacity"}));
    assert!(bad["expressionError"].is_string(), "a broken expression reports why: {bad}");
    qa.tool("set_property", json!({"layer": "Sub", "path": "transform/opacity", "expression": ""}));

    // The characters arrive over time: little title coverage at the start, all of it at 1 s.
    let white = |p: &image::Rgba<u8>| p[0] > 180 && p[1] > 180 && p[2] > 180;
    let f0 = qa.frame(None, 0.0);
    let f1 = qa.frame(None, 1.2);
    let (c0, c1) = (coverage(&f0, white), coverage(&f1, white));
    assert!(c1 > 0.02, "the title's glyphs are drawn at 1.2 s: {c1}");
    assert!(c0 < c1 * 0.5, "the range selector hides the characters at 0 s: {c0} vs {c1}");
    let cx = centroid(&f1, white).unwrap().0;
    assert!((cx - 88.0).abs() < 12.0, "title follows its position keys (x≈88 at 1.2 s): {cx}");
    qa.tool("set_property", json!({"layer": "Title", "path": "text/animators/#1/selectors/wigglySelector/minAmount", "value": -100}));
    let jitter = qa.frame(None, 0.0);
    assert!(coverage(&jitter, white) > c0 + 0.002, "the wiggly selector lets some characters through at 0 s");
    qa.tool("undo", json!({}));

    // A precomp of everything, time-remapped to play backwards.
    let pre = qa.exec("layer.precompose", json!({"layers": ["Title", "Sub"], "name": "Type Pre", "mode": "move"}));
    assert!(pre["comp"].is_u64() && pre["layer"].is_u64(), "{pre}");
    // Nested switches pass the precomp layer's blur flag down to the animated title.
    qa.exec("layer.setSwitch", json!({"layers": [pre["layer"].clone()], "switch": "motionBlur", "value": true}));
    qa.exec("layer.enableTimeRemap", json!({"layers": [pre["layer"].clone()], "value": true}));
    qa.tool("add_keyframe", json!({"layer": pre["layer"].clone(), "path": "timeRemap", "keys": [{"time": 0, "value": 1.9}, {"time": 1.9, "value": 0}]}));
    // The 1.9 s key sits on frame 23 (1.917 s), so 0.3 s maps to 1.6026 s: compared at that time,
    // not 1.6 s, where the moving title (and text drawn at its on-screen scale) differs.
    let at = qa.tool("get_property", json!({"layer": pre["layer"].clone(), "path": "timeRemap", "time": 0.3}))["value"].as_f64().unwrap();
    assert!((at - 1.6).abs() < 0.01, "time remap 0.3 s → 1.6 s of the precomp: {at}");
    let fwd = qa.frame(Some(pre["comp"].clone()), at);
    let rev = qa.frame(Some(comp.clone()), 0.3);
    assert!(mean_diff(&fwd, &rev) < 1.0, "the remapped precomp shows its frame at {at} s: {}", mean_diff(&fwd, &rev));

    // Render: H.264 and a PNG sequence through the render queue.
    let mp4 = qa.path("kinetic.mp4");
    let seq = qa.path("seq/kin_[####].png");
    let a = qa.exec("renderQueue.add", json!({"comp": "Kinetic", "format": "h264", "output": mp4}));
    assert_eq!(a["frames"], 24);
    qa.exec("renderQueue.add", json!({"comp": "Kinetic", "format": "png", "output": seq}));
    let done = qa.exec("renderQueue.render", json!({"wait": true}));
    for it in done["items"].as_array().unwrap() {
        assert_eq!(it["status"], "Done", "{it}");
    }
    let frames: Vec<_> =
        std::fs::read_dir(qa.dir.join("seq")).unwrap().filter_map(|e| e.ok()).filter(|e| e.path().extension().is_some_and(|x| x == "png")).collect();
    assert_eq!(frames.len(), 24, "one PNG per frame");
    let first = crate::harness::load(qa.dir.join("seq/kin_0000.png"));
    let last = crate::harness::load(qa.dir.join("seq/kin_0023.png"));
    assert!(mean_diff(&first, &last) > 1.0, "the sequence changes over time");

    // Re-import the movie and look at it like footage.
    let imp = qa.exec("file.import", json!({"paths": [mp4]}));
    assert_eq!(imp["errors"], json!([]), "{imp}");
    let vc = qa.exec("file.newCompFromSelection", json!({}))["comps"][0].clone();
    let info = qa.tool("get_comp", json!({"comp": vc.clone()}));
    assert_eq!((info["width"].as_u64(), info["height"].as_u64()), (Some(160), Some(90)));
    assert!((info["duration"].as_f64().unwrap() - 2.0).abs() < 0.01, "{info}");
    let m0 = qa.frame(Some(vc.clone()), 0.0);
    let m1 = qa.frame(Some(vc.clone()), 1.25);
    assert!(mean_diff(&m0, &m1) > 1.0, "the movie's frames differ over time");
    // The decoded movie matches the comp (lossy, so loosely).
    let ref1 = qa.frame(Some(comp.clone()), 1.25);
    assert!(mean_diff(&m1, &ref1) < 12.0, "decoded H.264 ≈ the comp: {}", mean_diff(&m1, &ref1));
    let bright = |p: &image::Rgba<u8>| luma(p) > 150.0;
    assert!(coverage(&m1, bright) > 0.01, "glyphs survive encoding");
}
