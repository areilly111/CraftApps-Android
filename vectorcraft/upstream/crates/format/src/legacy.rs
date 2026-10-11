//! Writing for older readers: format v1's anchor form.

use serde_json::{Value, json};

/// Rewrite every anchor in a serialized document (`anchors` arrays of path data) the way format v1
/// wrote it: `{p: {x, y}, in: {x, y}, out: {x, y}, kind}`, every member present (v1 readers need
/// them all; v2 writes `{p: [x, y], in?, out?, kind?}`).
pub(crate) fn anchors_to_v1(v: &mut Value) {
    match v {
        Value::Object(o) => {
            for (k, x) in o.iter_mut() {
                match x {
                    Value::Array(anchors) if k == "anchors" => anchors.iter_mut().for_each(anchor_to_v1),
                    _ => anchors_to_v1(x),
                }
            }
        }
        Value::Array(a) => a.iter_mut().for_each(anchors_to_v1),
        _ => {}
    }
}

fn anchor_to_v1(a: &mut Value) {
    let point = |v: &Value| match v.as_array().map(Vec::as_slice) {
        Some([x, y]) => Some(json!({ "x": x, "y": y })),
        _ => None,
    };
    let Some(o) = a.as_object() else { return };
    // Not a v2 anchor (already a map, or something else called `anchors`): leave it alone.
    let Some(p) = o.get("p").and_then(point) else { return };
    let handle = |k: &str| o.get(k).and_then(point).unwrap_or_else(|| p.clone());
    let kind = o.get("kind").cloned().unwrap_or_else(|| json!("Corner"));
    *a = json!({ "p": p, "in": handle("in"), "out": handle("out"), "kind": kind });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchors_are_written_the_v1_way() {
        let mut v = json!({"layers": [{"path": {"subpaths": [{"anchors": [{"p": [1.0, 2.0]}, {"p": [3.0, 4.0], "out": [5.0, 4.0], "kind": "Smooth"}], "closed": true}]}}], "effect": {"anchors": true}});
        anchors_to_v1(&mut v);
        let a = &v["layers"][0]["path"]["subpaths"][0]["anchors"];
        assert_eq!(a[0], json!({"p": {"x": 1.0, "y": 2.0}, "in": {"x": 1.0, "y": 2.0}, "out": {"x": 1.0, "y": 2.0}, "kind": "Corner"}));
        assert_eq!(a[1], json!({"p": {"x": 3.0, "y": 4.0}, "in": {"x": 3.0, "y": 4.0}, "out": {"x": 5.0, "y": 4.0}, "kind": "Smooth"}));
        assert_eq!(v["effect"]["anchors"], json!(true));
    }
}
