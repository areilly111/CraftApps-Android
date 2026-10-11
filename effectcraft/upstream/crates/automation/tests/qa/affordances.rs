//! Agent affordances added during QA: the `batch` tool, ids and paths in replies, depth-limited
//! flat property lists, `@uid` paths.

use serde_json::json;

use crate::harness::Qa;

#[test]
fn batch_tool_builds_in_one_call_and_one_undo_step() {
    let mut qa = Qa::new("batch");
    qa.exec("comp.new", json!({"name": "B", "width": 64, "height": 64}));
    let undo = qa.tool("get_project", json!({}))["undo"].as_array().unwrap().len();
    let r = qa.tool(
        "batch",
        json!({"label": "Ring", "steps": [
            {"command": "layer.newShape", "params": {"kind": "ellipse", "name": "Ring", "size": [40, 40]}},
            {"command": "layer.addShapeItem", "params": {"layer": "$1.layer", "kind": "trim"}},
            {"command": "prop.addKey", "params": {"layer": "$1.layer", "path": "transform/opacity", "time": 0, "value": 0}},
            {"command": "effect.apply", "params": {"layer": "$1.layer", "effect": "Glow"}},
        ]}),
    );
    assert_eq!(r["steps"], 4);
    assert_eq!(r["results"][1]["path"], "contents/trim");
    assert_eq!(r["results"][3]["paths"], json!(["effects/#1"]));
    let p = qa.tool("get_project", json!({}));
    assert_eq!(p["undo"].as_array().unwrap().len(), undo + 1);
    assert_eq!(p["undo"].as_array().unwrap().last().unwrap(), "Ring");
    // A failing step: nothing changes, and the message names the step.
    let e = qa
        .try_tool(
            "batch",
            json!({"steps": [{"command": "layer.newNull", "params": {}}, {"command": "layer.rename", "params": {"layer": "Nope", "name": "x"}}]}),
        )
        .unwrap_err();
    assert!(e.starts_with("step 2 (layer.rename)") && e.contains("rolled back"), "{e}");
    assert_eq!(qa.tool("get_comp", json!({}))["layers"].as_array().unwrap().len(), 1);

    // depth-limited flat listings keep cut-off groups visible; `@uid` reaches any property.
    let flat = qa.tool("get_layer", json!({"layer": "Ring", "flat": true, "depth": 1}));
    let props = flat["properties"].as_array().unwrap();
    assert!(props.iter().any(|p| p["path"] == "effects" && p["truncated"] == true), "{flat}");
    let uid = qa.tool("get_property", json!({"layer": "Ring", "path": "contents/trim/end"}))["uid"].as_u64().unwrap();
    let byuid = qa.tool("get_property", json!({"layer": "Ring", "path": format!("@{uid}")}));
    assert_eq!(byuid["name"], "End");
}
