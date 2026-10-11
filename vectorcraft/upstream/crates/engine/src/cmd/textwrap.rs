//! Object → Text Wrap: area type flows around wrap objects stacked above it in the same layer.
//!
//! A wrap object carries [`TextWrap`] options (`Node::wrap`). After every edit [`refresh`]
//! resolves, for each area text object, the outlines of the wrap objects above it into
//! `TextObject::wrap` (text space); the layout engine subtracts them from each line's span.

use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind, TextKind, TextWrap, WrapShape};
use vectorcraft_geom::{PathData, shapes};

use super::edit::selected_roots;
use super::typecmd::refresh_bounds;
use super::*;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "object.textWrap.make",
            "Make",
            ["Object", "Text Wrap"],
            None,
            "{offset?: pt (6), invert?: bool, ids?} area type below the selected objects (same layer) wraps around them → {ids}",
            has_selection,
            make
        ),
        cmd!("object.textWrap.release", "Release", ["Object", "Text Wrap"], None, "{ids?} → {ids}", has_selection, release),
        cmd!(
            "object.textWrap.options",
            "Text Wrap Options…",
            ["Object", "Text Wrap"],
            None,
            "{offset?: pt, invert?: bool, ids?} set the options of the selected wrap objects; no options → the current {offset, invert}",
            has_selection,
            options
        ),
    ]
}

fn targets(s: &Session, p: &Value) -> Result<Vec<NodeId>> {
    match p.get("ids").and_then(Value::as_array) {
        Some(a) => Ok(a.iter().filter_map(Value::as_u64).map(NodeId).collect()),
        None => selected_roots(s),
    }
}

fn wrap_param(p: &Value, base: TextWrap) -> TextWrap {
    TextWrap { offset: f64_or(p, "offset", base.offset).clamp(-1000.0, 1000.0), invert: bool_or(p, "invert", base.invert) }
}

fn make(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = targets(s, p)?;
    let w = wrap_param(p, TextWrap::default());
    s.edit("Make Text Wrap", |d, _| {
        for id in &ids {
            let n = d.node_mut(*id).ok_or(EngineError::NoNode(*id))?;
            if matches!(n.kind, NodeKind::Layer { .. }) {
                return Err(EngineError::Other("layers can't be wrap objects".into()));
            }
            n.wrap = Some(w);
        }
        Ok(())
    })?;
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

fn release(s: &mut Session, p: &Value) -> Result<Value> {
    let ids: Vec<NodeId> = {
        let d = &s.doc()?.doc;
        targets(s, p)?.into_iter().filter(|i| d.node(*i).is_some_and(|n| n.wrap.is_some())).collect()
    };
    if ids.is_empty() {
        return Err(EngineError::Other("Text Wrap: select a wrap object".into()));
    }
    s.edit("Release Text Wrap", |d, _| {
        for id in &ids {
            if let Some(n) = d.node_mut(*id) {
                n.wrap = None;
            }
        }
        Ok(())
    })?;
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

fn options(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = targets(s, p)?;
    let current = {
        let d = &s.doc()?.doc;
        ids.iter().find_map(|i| d.node(*i).and_then(|n| n.wrap)).unwrap_or_default()
    };
    if p.get("offset").is_none() && p.get("invert").is_none() {
        return Ok(json!({ "offset": current.offset, "invert": current.invert }));
    }
    let w = wrap_param(p, current);
    s.edit("Text Wrap Options", |d, _| {
        for id in &ids {
            if let Some(n) = d.node_mut(*id)
                && !matches!(n.kind, NodeKind::Layer { .. })
            {
                n.wrap = Some(w);
            }
        }
        Ok(())
    })?;
    Ok(json!({ "offset": w.offset, "invert": w.invert }))
}

/// Outline of a wrap object in document coordinates: its paths, or its bounds for other art.
fn outline(n: &Node, out: &mut PathData) {
    match &n.kind {
        NodeKind::Path { path, guide: false, .. } => {
            out.subpaths.extend(path.subpaths.iter().filter(|sp| sp.closed || sp.anchors.len() > 2).cloned())
        }
        NodeKind::Compound { children, .. } | NodeKind::Group { children, clip: false } => {
            for c in children.iter().filter(|c| c.visible) {
                outline(c, out);
            }
        }
        // A clip group wraps by its clipping path.
        NodeKind::Group { children, clip: true } => {
            if let Some(c) = children.first() {
                outline(c, out);
            }
        }
        _ => {
            if let Some(b) = n.geometric_bounds() {
                out.subpaths.extend(shapes::rectangle(b).subpaths);
            }
        }
    }
}

enum Item<'a> {
    Wrap(&'a Node),
    Text(NodeId),
}

fn collect<'a>(n: &'a Node, out: &mut Vec<Item<'a>>) {
    if !n.visible {
        return;
    }
    if n.wrap.is_some() && !matches!(n.kind, NodeKind::Layer { .. }) {
        out.push(Item::Wrap(n));
        return;
    }
    match &n.kind {
        NodeKind::Text(t) if matches!(t.kind, TextKind::Area { .. }) => out.push(Item::Text(n.id)),
        NodeKind::Layer { children, .. } | NodeKind::Group { children, .. } => {
            for c in children {
                collect(c, out);
            }
        }
        _ => {}
    }
}

fn any_wrap_state(n: &Node) -> bool {
    n.wrap.is_some()
        || matches!(&n.kind, NodeKind::Text(t) if !t.wrap.is_empty())
        || n.children().is_some_and(|ch| ch.iter().any(|c| any_wrap_state(c)))
}

/// Re-resolve the wrap shapes of every area text object (called after each edit).
pub(crate) fn refresh(doc: &mut Document) {
    if !doc.layers.iter().any(|l| any_wrap_state(l)) {
        return;
    }
    let mut updates: Vec<(NodeId, Vec<WrapShape>)> = vec![];
    for layer in &doc.layers {
        let mut items = vec![];
        collect(layer, &mut items);
        for (i, it) in items.iter().enumerate() {
            let Item::Text(id) = it else { continue };
            let Some(NodeKind::Text(t)) = doc.node(*id).map(|n| &n.kind) else { continue };
            let inv = t.xf.inverse();
            let shapes: Vec<WrapShape> = items[i + 1..]
                .iter()
                .filter_map(|w| match w {
                    Item::Wrap(n) => {
                        let mut pd = PathData::default();
                        outline(n, &mut pd);
                        (!pd.is_empty()).then(|| {
                            pd.transform(inv);
                            WrapShape { path: pd, wrap: n.wrap.unwrap_or_default() }
                        })
                    }
                    Item::Text(_) => None,
                })
                .collect();
            if shapes != t.wrap {
                updates.push((*id, shapes));
            }
        }
    }
    for (id, shapes) in updates {
        if let Some(NodeKind::Text(t)) = doc.node_mut(id).map(|n| &mut n.kind) {
            t.wrap = shapes;
            refresh_bounds(t);
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vectorcraft_doc::NodeKind;

    use crate::{NodeId, Session};

    fn lines(s: &Session, id: u64) -> Vec<(f64, f64)> {
        let Some(NodeKind::Text(t)) = s.doc().unwrap().doc.node(NodeId(id)).map(|n| n.kind.clone()) else { panic!() };
        vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t).lines.iter().map(|l| l.avail).collect()
    }

    #[test]
    fn area_type_wraps_around_objects_above_and_follows_edits() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 600, "height": 600})).unwrap();
        let text = "word ".repeat(200);
        let t = s.execute("text.create", &json!({"x": 0, "y": 0, "text": text, "size": 12, "area": {"width": 400, "height": 400}})).unwrap()["id"]
            .as_u64()
            .unwrap();
        let plain = lines(&s, t);
        assert!(plain.iter().all(|l| l.1 > 300.0));
        // A square over the left half of the frame: lines next to it start right of it (+6 pt offset).
        let r = s.execute("shape.rectangle", &json!({"x": 0, "y": 50, "width": 200, "height": 100})).unwrap()["id"].as_u64().unwrap();
        s.execute("select.set", &json!({"ids": [r]})).unwrap();
        s.execute("object.textWrap.make", &json!({})).unwrap();
        let wrapped = lines(&s, t);
        assert!(wrapped.iter().any(|l| l.0 >= 205.0), "{wrapped:?}");
        assert!(wrapped.iter().filter(|l| l.0 < 1.0).count() >= 2);
        // Moving the wrap object away re-flows the text.
        s.execute("object.move", &json!({"dx": 500, "dy": 0})).unwrap();
        assert!(lines(&s, t).iter().all(|l| l.0 < 1.0));
        s.execute("object.move", &json!({"dx": -500, "dy": 0})).unwrap();
        // Options round-trip; Invert Wrap keeps text inside the object only.
        assert_eq!(s.execute("object.textWrap.options", &json!({})).unwrap()["offset"], json!(6.0));
        s.execute("object.textWrap.options", &json!({"invert": true, "offset": 0})).unwrap();
        let inside = lines(&s, t);
        assert!(inside.iter().all(|l| l.1 <= 200.0 + 1e-6), "{inside:?}");
        // Release: back to the plain layout.
        s.execute("object.textWrap.release", &json!({})).unwrap();
        assert_eq!(lines(&s, t), plain);
        // Objects below the text don't wrap it.
        let below = s.execute("shape.rectangle", &json!({"x": 0, "y": 50, "width": 200, "height": 100})).unwrap()["id"].as_u64().unwrap();
        s.execute("select.set", &json!({"ids": [below]})).unwrap();
        s.execute("object.textWrap.make", &json!({})).unwrap();
        s.execute("object.arrange.sendToBack", &json!({})).unwrap();
        assert_eq!(lines(&s, t), plain);
    }

    #[test]
    fn text_flows_on_both_sides_of_a_centred_object() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 600, "height": 600})).unwrap();
        let t = s
            .execute("text.create", &json!({"x": 0, "y": 0, "text": "word ".repeat(300), "size": 12, "area": {"width": 400, "height": 400}}))
            .unwrap()["id"]
            .as_u64()
            .unwrap();
        let r = s.execute("shape.ellipse", &json!({"x": 150, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
        s.execute("select.set", &json!({"ids": [r]})).unwrap();
        s.execute("object.textWrap.make", &json!({})).unwrap();
        let Some(NodeKind::Text(tx)) = s.doc().unwrap().doc.node(NodeId(t)).map(|n| n.kind.clone()) else { panic!() };
        let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &tx);
        // Beside the circle each line band holds two lines: left of it and right of it.
        let beside: Vec<_> = lay.lines.iter().filter(|l| l.baseline > 120.0 && l.baseline < 180.0).collect();
        assert!(
            beside.iter().any(|l| l.avail.1 < 150.0) && beside.iter().any(|l| l.avail.0 > 250.0),
            "{:?}",
            beside.iter().map(|l| l.avail).collect::<Vec<_>>()
        );
        let left = beside.iter().find(|l| l.avail.1 < 150.0).unwrap();
        assert!(beside.iter().any(|l| l.avail.0 > 250.0 && (l.baseline - left.baseline).abs() < 1e-9), "same baseline on both sides");
        // Reading order: the right-hand line follows the left-hand one.
        let li = lay.lines.iter().position(|l| std::ptr::eq(l, *left)).unwrap();
        assert!(lay.lines[li + 1].avail.0 > 250.0);
    }
}
