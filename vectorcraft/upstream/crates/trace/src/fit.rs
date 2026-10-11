//! Loop → polygon → Bézier curves.

use vectorcraft_geom::{AnchorKind, CubicBez, ParamCurve, PathData, Point, SubPath};
use vectorcraft_pathops::{SimplifyOptions, simplify_with};

use crate::contour::Loop;

#[derive(Clone, Copy, Debug)]
pub(crate) struct FitOptions {
    pub polygon_tol: f64,
    pub fit_tol: f64,
    pub corner_angle: f64,
    pub snap_lines: bool,
}

fn seg_dist(p: Point, a: Point, b: Point) -> f64 {
    let ab = b - a;
    let l2 = ab.hypot2();
    if l2 < 1e-18 {
        return p.distance(a);
    }
    let t = ((p - a).dot(ab) / l2).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

/// Douglas–Peucker on `pts[a..=b]`, marking kept indices.
fn rdp(pts: &[Point], a: usize, b: usize, tol: f64, keep: &mut [bool]) {
    let mut stack = vec![(a, b)];
    while let Some((a, b)) = stack.pop() {
        if b <= a + 1 {
            continue;
        }
        let (pa, pb) = (pts[a], pts[b % pts.len()]);
        let mut best = (0.0, a);
        for (i, &q) in pts.iter().enumerate().take(b).skip(a + 1) {
            let d = seg_dist(q, pa, pb);
            if d > best.0 {
                best = (d, i);
            }
        }
        if best.0 > tol {
            keep[best.1] = true;
            stack.push((a, best.1));
            stack.push((best.1, b));
        }
    }
}

/// The polygon approximating a closed loop of pixel-corner vertices within `tol`.
pub(crate) fn polygon(loop_pts: &[(i32, i32)], tol: f64) -> Vec<Point> {
    let raw: Vec<Point> = loop_pts.iter().map(|&(x, y)| Point::new(x as f64, y as f64)).collect();
    if raw.len() < 4 {
        return raw;
    }
    let pts = dejag(loop_pts);
    let pts = if pts.len() < 3 { raw.clone() } else { pts };
    let n = pts.len();
    if n < 4 {
        return pts;
    }
    // Split the closed loop at vertex 0 and the vertex farthest from it.
    let far = (1..n).max_by(|&i, &j| pts[i].distance_squared(pts[0]).total_cmp(&pts[j].distance_squared(pts[0]))).unwrap_or(n / 2);
    let mut ext = pts.clone();
    ext.push(pts[0]);
    let mut keep = vec![false; n + 1];
    keep[0] = true;
    keep[far] = true;
    rdp(&ext, 0, far, tol, &mut keep);
    rdp(&ext, far, n, tol, &mut keep);
    let mut out: Vec<Point> = (0..n).filter(|&i| keep[i]).map(|i| pts[i]).collect();
    // Vertex 0 was forced; drop it when it's not needed.
    if out.len() > 3 {
        let (p, a, b) = (out[0], out[out.len() - 1], out[1]);
        if seg_dist(p, a, b) <= tol * 0.5 {
            out.remove(0);
        }
    }
    if out.len() < 3 { raw } else { out }
}

/// Replace pixel-staircase jogs by their midpoints.
///
/// A unit-length edge whose two ends turn in opposite directions (an S-shaped step) is a jog of
/// a staircase approximating a sloped line or curve: its midpoint lies on the ideal boundary, its
/// end vertices don't. Vertices not adjacent to any jog (true corners, e.g. of rectangles, or the
/// U-turns at the end of one-pixel-wide lines) are kept.
fn dejag(v: &[(i32, i32)]) -> Vec<Point> {
    let n = v.len();
    let edge = |i: usize| {
        let (a, b) = (v[i % n], v[(i + 1) % n]);
        (b.0 - a.0, b.1 - a.1)
    };
    // Turn sign at vertex i (between edge i-1 and edge i).
    let turn = |i: usize| {
        let (a, b) = (edge((i + n - 1) % n), edge(i));
        (a.0 as i64 * b.1 as i64 - a.1 as i64 * b.0 as i64).signum()
    };
    let jog: Vec<bool> = (0..n)
        .map(|i| {
            let (dx, dy) = edge(i);
            dx.abs() + dy.abs() == 1 && turn(i) * turn(i + 1) < 0
        })
        .collect();
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        if !jog[i] && !jog[(i + n - 1) % n] {
            out.push(Point::new(v[i].0 as f64, v[i].1 as f64));
        }
        if jog[i] {
            let (a, b) = (v[i], v[(i + 1) % n]);
            out.push(Point::new((a.0 + b.0) as f64 / 2.0, (a.1 + b.1) as f64 / 2.0));
        }
    }
    out
}

/// Fit one loop into a closed Bézier subpath (pixel coordinates).
pub(crate) fn fit_loop(l: &Loop, o: &FitOptions) -> Option<SubPath> {
    let poly = polygon(&l.pts, o.polygon_tol);
    if poly.len() < 3 {
        return None;
    }
    let sp = SubPath::polyline(&poly, true);
    let fitted =
        simplify_with(&PathData::single(sp), &SimplifyOptions { tolerance: o.fit_tol, corner_angle_deg: o.corner_angle, straight_lines: false });
    let mut sp = fitted.subpaths.into_iter().next()?;
    if sp.anchors.len() < 2 {
        return None;
    }
    if o.snap_lines {
        snap_to_lines(&mut sp, (o.fit_tol * 2.0).max(1.5));
    }
    Some(sp)
}

/// Fit a centre line through `pts` (pixel-centre points in order) into a Bézier subpath: the
/// points within `polygon_tol` of it are dropped (Douglas–Peucker), then curves are fitted. A
/// `closed` line runs back to its first point.
pub(crate) fn fit_polyline(pts: &[Point], closed: bool, o: &FitOptions) -> Option<SubPath> {
    let n = pts.len();
    let first = *pts.first()?;
    if n < 2 {
        return None;
    }
    let mut ext = pts.to_vec();
    let mut keep = vec![false; n + 1];
    keep[0] = true;
    if closed {
        // Split the ring at its first point and the point farthest from it.
        ext.push(first);
        let far = (1..n).max_by(|&i, &j| pts[i].distance_squared(first).total_cmp(&pts[j].distance_squared(first))).unwrap_or(n / 2);
        keep[far] = true;
        rdp(&ext, 0, far, o.polygon_tol, &mut keep);
        rdp(&ext, far, n, o.polygon_tol, &mut keep);
    } else {
        keep[n - 1] = true;
        rdp(&ext, 0, n - 1, o.polygon_tol, &mut keep);
    }
    let mut poly: Vec<Point> = (0..n).filter(|&i| keep[i]).map(|i| pts[i]).collect();
    if closed && poly.len() < 3 {
        poly = pts.to_vec();
    }
    if poly.len() < if closed { 3 } else { 2 } {
        return None;
    }
    let sp = SubPath::polyline(&poly, closed);
    let fitted =
        simplify_with(&PathData::single(sp), &SimplifyOptions { tolerance: o.fit_tol, corner_angle_deg: o.corner_angle, straight_lines: false });
    let mut sp = fitted.subpaths.into_iter().next()?;
    if sp.anchors.len() < 2 {
        return None;
    }
    if o.snap_lines {
        snap_to_lines(&mut sp, (o.fit_tol * 2.0).max(1.5));
    }
    Some(sp)
}

/// Retract the handles of curves that stay within `tol` of their chord.
fn snap_to_lines(sp: &mut SubPath, tol: f64) {
    let n = sp.anchors.len();
    for i in 0..sp.segment_count() {
        let j = (i + 1) % n;
        let (a, b) = (sp.anchors[i], sp.anchors[j]);
        if !a.has_out() && !b.has_in() {
            continue;
        }
        let c = CubicBez::new(a.p, a.h_out, b.h_in, b.p);
        if (1..8).all(|k| seg_dist(c.eval(k as f64 / 8.0), a.p, b.p) <= tol) {
            sp.anchors[i].h_out = a.p;
            sp.anchors[j].h_in = b.p;
        }
    }
    for a in &mut sp.anchors {
        if !a.has_in() || !a.has_out() {
            a.kind = AnchorKind::Corner;
        }
    }
}
