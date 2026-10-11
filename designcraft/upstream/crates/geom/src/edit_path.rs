//! Path surgery for the Erase and Smooth tools: cut a subpath at curve parameters, remove the
//! span between two places, re-fit a stretch more smoothly.

use kurbo::{ParamCurve, Point};

use crate::path::{Anchor, AnchorKind, SubPath};

/// A place on a subpath: segment index and parameter in it.
pub type At = (usize, f64);

fn key(a: At) -> f64 {
    a.0 as f64 + a.1
}

/// Anchors of an open walk around `sp` (a closed subpath gets its first anchor again at the end).
fn walk(sp: &SubPath) -> Vec<Anchor> {
    let mut v = sp.anchors.clone();
    if sp.closed && !v.is_empty() {
        v.push(v[0]);
    }
    v
}

/// The part of the walk `w` before `at`.
fn prefix(sp: &SubPath, w: &[Anchor], at: At) -> Vec<Anchor> {
    let (seg, t) = at;
    let c = sp.segment(seg);
    let left = c.subsegment(0.0..t);
    let mut out: Vec<Anchor> = w[..=seg].to_vec();
    if let Some(l) = out.last_mut() {
        l.h_out = left.p1;
    }
    out.push(Anchor::with_handles(left.p3, left.p2, left.p3));
    out
}

/// The part of the walk `w` after `at`.
fn suffix(sp: &SubPath, w: &[Anchor], at: At) -> Vec<Anchor> {
    let (seg, t) = at;
    let c = sp.segment(seg);
    let right = c.subsegment(t..1.0);
    let mut out = vec![Anchor::with_handles(right.p0, right.p0, right.p1)];
    let mut rest: Vec<Anchor> = w[seg + 1..].to_vec();
    if let Some(f) = rest.first_mut() {
        f.h_in = right.p2;
    }
    out.extend(rest);
    out
}

fn tidy(mut v: Vec<Anchor>) -> Vec<Anchor> {
    for a in &mut v {
        *a = Anchor::with_handles(a.p, a.h_in, a.h_out);
    }
    v.dedup_by(|b, a| {
        (a.p - b.p).hypot() < 1e-6 && {
            a.h_out = b.h_out;
            true
        }
    });
    v
}

/// Remove the stretch of `sp` between `a` and `b` (either order). An open path leaves up to two
/// pieces; a closed one opens into one.
pub fn erase(sp: &SubPath, a: At, b: At) -> Vec<SubPath> {
    let (a, b) = if key(a) <= key(b) { (a, b) } else { (b, a) };
    let w = walk(sp);
    if sp.closed {
        // From b round to a, through the start.
        let mut tail = suffix(sp, &w, b);
        let head = prefix(sp, &w, a);
        if let (Some(last), Some(first)) = (tail.last_mut(), head.first()) {
            last.h_out = first.h_out;
        }
        tail.extend(head.into_iter().skip(1));
        let t = tidy(tail);
        return if t.len() >= 2 { vec![SubPath::new(t, false)] } else { vec![] };
    }
    [tidy(prefix(sp, &w, a)), tidy(suffix(sp, &w, b))]
        .into_iter()
        .filter(|p| p.len() >= 2 && length(p) > 1e-3)
        .map(|p| SubPath::new(p, false))
        .collect()
}

fn length(a: &[Anchor]) -> f64 {
    a.windows(2).map(|w| (w[1].p - w[0].p).hypot()).sum()
}

/// Re-fit the stretch between `a` and `b` (either order) through fewer, smoother anchors.
pub fn smooth(sp: &SubPath, a: At, b: At, tol: f64) -> SubPath {
    let (a, b) = if key(a) <= key(b) { (a, b) } else { (b, a) };
    // Samples along the stretch.
    let mut pts: Vec<Point> = Vec::new();
    let steps = 64;
    let (k0, k1) = (key(a), key(b));
    for i in 0..=steps {
        let k = k0 + (k1 - k0) * i as f64 / steps as f64;
        let seg = (k.floor() as usize).min(sp.segment_count().saturating_sub(1));
        let t = (k - seg as f64).clamp(0.0, 1.0);
        pts.push(sp.segment(seg).eval(t));
    }
    let mut mid = crate::freehand::fit(&pts, tol.max(0.5) * 2.0, false);
    if mid.len() < 2 {
        return sp.clone();
    }
    let w = walk(sp);
    let mut head = prefix(sp, &w, a);
    let tail = suffix(sp, &w, b);
    // The fitted ends meet the untouched parts smoothly.
    if let Some(h) = head.last_mut() {
        h.h_out = mid[0].h_out;
    }
    head.pop();
    let n = mid.len();
    mid[n - 1].h_out = tail[0].h_out;
    head.extend(mid);
    head.extend(tail.into_iter().skip(1));
    let mut out = tidy(head);
    if sp.closed {
        // The walk repeated the first anchor at the end.
        if out.len() > 2
            && (out[0].p - out[out.len() - 1].p).hypot() < 1e-6
            && let Some(last) = out.pop()
        {
            out[0].h_in = last.h_in;
        }
    }
    for x in &mut out {
        if x.has_in() && x.has_out() && x.kind == AnchorKind::Corner {
            *x = Anchor::with_handles(x.p, x.h_in, x.h_out);
        }
    }
    SubPath::new(out, sp.closed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(n: usize) -> SubPath {
        SubPath::polyline(&(0..=n).map(|i| Point::new(i as f64 * 10.0, 0.0)).collect::<Vec<_>>(), false)
    }

    #[test]
    fn erase_middle_of_open_and_closed_paths() {
        let sp = line(10);
        let parts = erase(&sp, (3, 0.5), (6, 0.5));
        assert_eq!(parts.len(), 2);
        assert!((parts[0].anchors.last().unwrap().p.x - 35.0).abs() < 1e-9);
        assert!((parts[1].anchors[0].p.x - 65.0).abs() < 1e-9);
        assert!((parts[1].anchors.last().unwrap().p.x - 100.0).abs() < 1e-9);
        // Erasing the very end leaves one piece.
        assert_eq!(erase(&sp, (8, 0.0), (9, 1.0)).len(), 1);
        let sq = SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(100.0, 0.0), Point::new(100.0, 100.0), Point::new(0.0, 100.0)], true);
        let open = erase(&sq, (0, 0.25), (0, 0.75));
        assert_eq!(open.len(), 1);
        assert!(!open[0].closed);
        // Parameters are the segment's own (a straight cubic isn't linear in t).
        assert!((open[0].anchors[0].p - sq.segment(0).eval(0.75)).hypot() < 1e-9);
        assert!((open[0].anchors.last().unwrap().p - sq.segment(0).eval(0.25)).hypot() < 1e-9);
    }

    #[test]
    fn smooth_reduces_a_zigzag() {
        let zig = SubPath::polyline(&(0..=20).map(|i| Point::new(i as f64 * 10.0, if i % 2 == 0 { 0.0 } else { 3.0 })).collect::<Vec<_>>(), false);
        let s = smooth(&zig, (2, 0.0), (18, 0.0), 4.0);
        assert!(s.anchors.len() < zig.anchors.len(), "{}", s.anchors.len());
        assert!((s.anchors[0].p - zig.anchors[0].p).hypot() < 1e-9);
        assert!((s.anchors.last().unwrap().p - zig.anchors.last().unwrap().p).hypot() < 1e-9);
    }
}
