//! Anchor-point editing (the Pen tool group): Add Anchor Point, Delete Anchor Point and Convert
//! Direction Point. Positions are in spread coordinates; anchors are `(subpath, index)`.

use designcraft_doc::{Document, ItemId, Shape};
use designcraft_geom::{Affine, Anchor, AnchorKind, Point};
use serde_json::{Value, json};

use super::{CommandSpec, bad, cmd, has_doc, point_param};
use crate::{Result, Session};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "path.addAnchor",
            "Add Anchor Point",
            [],
            None,
            "{id, at: [x,y] (spread coords; the nearest point on the path), tolerance?: pt (default 4)} → {subpath, anchor}",
            has_doc,
            add_anchor
        ),
        cmd!(
            "path.deleteAnchor",
            "Delete Anchor Point",
            [],
            None,
            "{id, subpath, anchor} — removes the point (and the object when no path is left)",
            has_doc,
            delete_anchor
        ),
        cmd!(
            "path.convertAnchor",
            "Convert Direction Point",
            [],
            None,
            "{id, subpath, anchor, to?: [x,y] (spread coords: drag out symmetric handles to there), corner?: bool} — without `to`, a smooth point becomes a corner and a corner gets smooth handles",
            has_doc,
            convert_anchor
        ),
        cmd!(
            "path.erase",
            "Erase",
            [],
            None,
            "{id, points: [[x,y], …] (spread coords: the Erase tool's drag), tolerance?: pt (default 4)} — removes the stretch of path the drag ran along",
            has_doc,
            |s, p| erase_or_smooth(s, p, false)
        ),
        cmd!(
            "path.smooth",
            "Smooth",
            [],
            None,
            "{id, points: [[x,y], …] (spread coords), tolerance?: pt (default 4)} — re-fits the stretch the drag ran along with fewer, smoother anchors",
            has_doc,
            |s, p| erase_or_smooth(s, p, true)
        ),
    ]
}

/// Erase / Smooth: the stretch of one subpath touched by the drag.
fn erase_or_smooth(s: &mut Session, p: &Value, smooth: bool) -> Result<Value> {
    let cmd = if smooth { "path.smooth" } else { "path.erase" };
    let id = super::id_param(p, "id").ok_or_else(|| bad(cmd, "missing id"))?;
    let tol = p.get("tolerance").and_then(Value::as_f64).unwrap_or(4.0);
    let pts: Vec<Point> = p
        .get("points")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| Some(Point::new(v.get(0)?.as_f64()?, v.get(1)?.as_f64()?))).collect())
        .unwrap_or_default();
    s.edit(|d, _| {
        let inv = to_inner(d, id).ok_or(designcraft_doc::DocError::NoItem(id))?;
        let scale = inv.determinant().abs().sqrt().max(1e-9);
        let it = d.item_mut(id).ok_or(designcraft_doc::DocError::NoItem(id))?;
        // Where the drag touched the path, per subpath.
        let mut hits: Vec<(usize, usize, f64)> = Vec::new();
        for q in &pts {
            if let Some((si, seg, t, _, dist)) = it.path.nearest(inv * *q)
                && dist <= tol * scale
            {
                hits.push((si, seg, t));
            }
        }
        let Some(si) =
            (0..it.path.subpaths.len()).max_by_key(|i| hits.iter().filter(|h| h.0 == *i).count()).filter(|i| hits.iter().any(|h| h.0 == *i))
        else {
            return Err(bad(cmd, "the drag didn't touch the path"));
        };
        let on: Vec<(usize, f64)> = hits.iter().filter(|h| h.0 == si).map(|h| (h.1, h.2)).collect();
        let k = |a: &(usize, f64)| a.0 as f64 + a.1;
        let (Some(&a), Some(&b)) = (on.iter().min_by(|x, y| k(x).total_cmp(&k(y))), on.iter().max_by(|x, y| k(x).total_cmp(&k(y)))) else {
            return Err(bad(cmd, "the drag didn't touch the path"));
        };
        let sp = it.path.subpaths[si].clone();
        if smooth {
            it.path.subpaths[si] = designcraft_geom::edit_path::smooth(&sp, a, b, tol * scale);
        } else {
            let parts = designcraft_geom::edit_path::erase(&sp, a, b);
            it.path.subpaths.remove(si);
            for (k, part) in parts.into_iter().enumerate() {
                it.path.subpaths.insert(si + k, part);
            }
        }
        as_path(&mut it.shape);
        if it.path.subpaths.is_empty() {
            d.remove_item(id)?;
            return Ok(json!({"deleted": true}));
        }
        Ok(json!({"subpaths": it.path.subpaths.len()}))
    })
}

/// Spread → item-space transform of `id`.
fn to_inner(d: &Document, id: ItemId) -> Option<Affine> {
    let loc = d.find(id)?;
    Some((d.parent_xf(&loc) * d.item_at(&loc)?.xf).inverse())
}

fn anchor_param(p: &Value, cmd: &str) -> Result<(ItemId, usize, usize)> {
    let id = super::id_param(p, "id").ok_or_else(|| bad(cmd, "missing id"))?;
    let si = p.get("subpath").and_then(Value::as_u64).unwrap_or(0) as usize;
    let ai = p.get("anchor").and_then(Value::as_u64).ok_or_else(|| bad(cmd, "missing anchor"))? as usize;
    Ok((id, si, ai))
}

/// Editing points makes a drawn rectangle/ellipse/polygon an ordinary path.
fn as_path(shape: &mut Shape) {
    if matches!(shape, Shape::Rectangle | Shape::Oval | Shape::Polygon) {
        *shape = Shape::Path;
    }
}

fn add_anchor(s: &mut Session, p: &Value) -> Result<Value> {
    let id = super::id_param(p, "id").ok_or_else(|| bad("path.addAnchor", "missing id"))?;
    let at = point_param(p, "at").ok_or_else(|| bad("path.addAnchor", "missing at"))?;
    let tol = p.get("tolerance").and_then(Value::as_f64).unwrap_or(4.0);
    s.edit(|d, _| {
        let inv = to_inner(d, id).ok_or(designcraft_doc::DocError::NoItem(id))?;
        let it = d.item_mut(id).ok_or(designcraft_doc::DocError::NoItem(id))?;
        let q = inv * at;
        let (si, seg, t, _, dist) = it.path.nearest(q).ok_or_else(|| bad("path.addAnchor", "the object has no path"))?;
        // Tolerance is in spread units; the item space may be scaled.
        let scale = inv.determinant().abs().sqrt().max(1e-9);
        if dist > tol * scale {
            return Err(bad("path.addAnchor", "not on the path"));
        }
        let t = t.clamp(1e-4, 1.0 - 1e-4);
        let ai = it.path.subpaths[si].insert_anchor(seg, t);
        as_path(&mut it.shape);
        Ok(json!({"subpath": si, "anchor": ai}))
    })
}

fn delete_anchor(s: &mut Session, p: &Value) -> Result<Value> {
    let (id, si, ai) = anchor_param(p, "path.deleteAnchor")?;
    s.edit(|d, sel| {
        let it = d.item_mut(id).ok_or(designcraft_doc::DocError::NoItem(id))?;
        let sp = it.path.subpaths.get_mut(si).ok_or_else(|| bad("path.deleteAnchor", "no such subpath"))?;
        if ai >= sp.anchors.len() {
            return Err(bad("path.deleteAnchor", "no such anchor"));
        }
        sp.anchors.remove(ai);
        // A subpath needs two points (three to stay closed).
        if sp.anchors.len() < 2 {
            it.path.subpaths.remove(si);
        } else if sp.closed && sp.anchors.len() < 3 {
            sp.closed = false;
        }
        as_path(&mut it.shape);
        if it.path.subpaths.is_empty() {
            d.remove_item(id)?;
            sel.items.retain(|i| *i != id);
            return Ok(json!({"deleted": true}));
        }
        Ok(json!({"deleted": false}))
    })
}

fn convert_anchor(s: &mut Session, p: &Value) -> Result<Value> {
    let (id, si, ai) = anchor_param(p, "path.convertAnchor")?;
    let to = point_param(p, "to");
    let corner = p.get("corner").and_then(Value::as_bool);
    s.edit(|d, _| {
        let inv = to_inner(d, id).ok_or(designcraft_doc::DocError::NoItem(id))?;
        let it = d.item_mut(id).ok_or(designcraft_doc::DocError::NoItem(id))?;
        let sp = it.path.subpaths.get_mut(si).ok_or_else(|| bad("path.convertAnchor", "no such subpath"))?;
        let n = sp.anchors.len();
        if ai >= n {
            return Err(bad("path.convertAnchor", "no such anchor"));
        }
        let closed = sp.closed;
        let a = sp.anchors[ai];
        let has_handles = a.has_in() || a.has_out();
        let new = match (to, corner) {
            (Some(t), _) => Anchor::smooth(a.p, inv * t),
            (None, Some(true)) => Anchor::corner(a.p),
            (None, Some(false)) => smooth_auto(&sp.anchors, ai, closed),
            (None, None) if has_handles => Anchor::corner(a.p),
            (None, None) => smooth_auto(&sp.anchors, ai, closed),
        };
        sp.anchors[ai] = new;
        as_path(&mut it.shape);
        Ok(json!({"kind": if new.kind == AnchorKind::Smooth { "smooth" } else { "corner" }}))
    })
}

/// Smooth handles for a corner: parallel to the line through its neighbours, a third of the way
/// to each.
fn smooth_auto(anchors: &[Anchor], i: usize, closed: bool) -> Anchor {
    let n = anchors.len();
    let p = anchors[i].p;
    let prev = if i > 0 {
        Some(anchors[i - 1].p)
    } else if closed {
        Some(anchors[n - 1].p)
    } else {
        None
    };
    let next = if i + 1 < n {
        Some(anchors[i + 1].p)
    } else if closed {
        Some(anchors[0].p)
    } else {
        None
    };
    let (a, b) = (prev.unwrap_or(p), next.unwrap_or(p));
    let dir = b - a;
    let len = dir.hypot();
    if len < 1e-9 {
        return anchors[i];
    }
    let u = dir / len;
    let (lin, lout) = ((p - a).hypot() / 3.0, (b - p).hypot() / 3.0);
    let lin = if prev.is_some() { lin } else { lout };
    let lout = if next.is_some() { lout } else { lin };
    Anchor { p, h_in: p - u * lin, h_out: p + u * lout, kind: AnchorKind::Smooth }
}

/// The anchor of item `id` nearest to `at` (spread coords), within `tol`.
pub fn anchor_near(d: &Document, id: ItemId, at: Point, tol: f64) -> Option<(usize, usize)> {
    let loc = d.find(id)?;
    let m = d.parent_xf(&loc) * d.item_at(&loc)?.xf;
    d.item(id)?
        .path
        .anchors()
        .map(|(si, ai, a)| (si, ai, ((m * a.p) - at).hypot()))
        .filter(|x| x.2 <= tol)
        .min_by(|a, b| a.2.total_cmp(&b.2))
        .map(|x| (x.0, x.1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn erase_and_smooth_along_a_drag() {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let pts: Vec<Value> = (0..=20).map(|i| json!({"p": [100.0 + i as f64 * 20.0, if i % 2 == 0 { 300.0 } else { 306.0 }]})).collect();
        let id = s.execute("path.create", &json!({"anchors": pts})).unwrap()["id"].as_u64().unwrap();
        let n = s.doc().unwrap().doc.item(ItemId(id)).unwrap().path.anchor_count();
        let drag: Vec<Value> = (0..=12).map(|i| json!([150.0 + i as f64 * 20.0, 303.0])).collect();
        s.execute("path.smooth", &json!({"id": id, "points": drag})).unwrap();
        let after = s.doc().unwrap().doc.item(ItemId(id)).unwrap().path.anchor_count();
        assert!(after < n, "{after} < {n}");
        let drag: Vec<Value> = (0..=5).map(|i| json!([200.0 + i as f64 * 20.0, 303.0])).collect();
        let r = s.execute("path.erase", &json!({"id": id, "points": drag})).unwrap();
        assert_eq!(r["subpaths"], 2, "a gap in the middle");
        assert!(s.execute("path.erase", &json!({"id": id, "points": [[100, 600]]})).is_err());
    }

    #[test]
    fn add_convert_delete_anchor_points() {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let id = ItemId(s.execute("frame.create", &json!({"rect": [100, 100, 200, 200]})).unwrap()["id"].as_u64().unwrap());
        let count = |s: &Session| s.doc().unwrap().doc.item(id).unwrap().path.anchor_count();
        assert_eq!(count(&s), 4);
        assert!(s.execute("path.addAnchor", &json!({"id": id.0, "at": [150, 150]})).is_err(), "not on the path");
        let r = s.execute("path.addAnchor", &json!({"id": id.0, "at": [150, 101]})).unwrap();
        assert_eq!(count(&s), 5);
        let ai = r["anchor"].as_u64().unwrap() as usize;
        let d = s.doc().unwrap().doc.clone();
        let it = d.item(id).unwrap();
        assert_eq!(it.shape, Shape::Path);
        assert!((it.path.subpaths[0].anchors[ai].p.y - 100.0).abs() < 1e-6);
        assert_eq!(anchor_near(&d, id, Point::new(150.5, 100.5), 3.0), Some((0, ai)));
        // Drag out handles, then click again to make it a corner.
        s.execute("path.convertAnchor", &json!({"id": id.0, "anchor": ai, "to": [170, 90]})).unwrap();
        let a = s.doc().unwrap().doc.item(id).unwrap().path.subpaths[0].anchors[ai];
        assert_eq!(a.kind, AnchorKind::Smooth);
        assert!((a.h_out - Point::new(170.0, 90.0)).hypot() < 1e-9 && (a.h_in - Point::new(130.0, 110.0)).hypot() < 1e-9);
        assert_eq!(s.execute("path.convertAnchor", &json!({"id": id.0, "anchor": ai})).unwrap()["kind"], "corner");
        assert_eq!(s.execute("path.convertAnchor", &json!({"id": id.0, "anchor": 0})).unwrap()["kind"], "smooth");
        s.execute("path.deleteAnchor", &json!({"id": id.0, "anchor": ai})).unwrap();
        assert_eq!(count(&s), 4);
        // Deleting down to one point removes the object.
        for _ in 0..2 {
            s.execute("path.deleteAnchor", &json!({"id": id.0, "anchor": 0})).unwrap();
        }
        assert!(!s.doc().unwrap().doc.item(id).unwrap().path.is_closed(), "two points can't stay closed");
        let r = s.execute("path.deleteAnchor", &json!({"id": id.0, "anchor": 0})).unwrap();
        assert_eq!(r["deleted"], true);
        assert!(s.doc().unwrap().doc.item(id).is_none());
    }
}
