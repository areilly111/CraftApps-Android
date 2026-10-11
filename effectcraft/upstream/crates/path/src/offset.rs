//! Offset Paths: grow or shrink the filled region of a path by a distance, with true curve
//! offsetting (kurbo's stroker offsets curves analytically and refits them) and robust overlap
//! removal (the boolean sweep).
//!
//! Adapted from VectorCraft's `vectorcraft-pathops` (`crates/pathops/src/offset.rs`; our own code,
//! MIT OR Apache-2.0).

use kurbo::{BezPath, PathEl, Stroke, StrokeOpts};
use linesweeper::topology::{ContourIdx, Contours};

use crate::boolean::{Arrangement, DEFAULT_PRECISION, Tidy, contours_to_path};
use crate::{FillRule, Join};

/// Flattening/fit tolerance handed to kurbo's stroker.
const STROKE_TOL: f64 = 1e-3;

fn kjoin(j: Join) -> kurbo::Join {
    match j {
        Join::Miter => kurbo::Join::Miter,
        Join::Round => kurbo::Join::Round,
        Join::Bevel => kurbo::Join::Bevel,
    }
}

fn stroke_bez(bp: &BezPath, width: f64, join: Join, miter_limit: f64) -> BezPath {
    let style = Stroke::new(width).with_join(kjoin(join)).with_caps(kurbo::Cap::Butt).with_miter_limit(miter_limit.max(1.0));
    crate::boolean::close_subpaths(&kurbo::stroke(bp.iter(), &style, &StrokeOpts::default(), STROKE_TOL))
}

/// Split a path into its closed and open subpaths.
fn split_closed(p: &BezPath) -> (BezPath, BezPath) {
    let (mut closed, mut open) = (BezPath::new(), BezPath::new());
    for sp in p.subpaths() {
        let n = sp.iter().filter(|e| !matches!(e, PathEl::MoveTo(_) | PathEl::ClosePath)).count();
        if n == 0 {
            continue;
        }
        let dst = if matches!(sp.last(), Some(PathEl::ClosePath)) { &mut closed } else { &mut open };
        dst.extend(sp.iter().copied());
    }
    (closed, open)
}

/// Offset a path by `delta`: positive grows the (non-zero) filled region, negative insets it.
/// Closed subpaths are treated as a filled region; open subpaths are outlined with a stroke of
/// width `2|delta|`. The result is a clean compound path of closed contours.
pub fn offset_path(path: &BezPath, delta: f64, join: Join, miter_limit: f64) -> BezPath {
    if !delta.is_finite() || path.elements().is_empty() {
        return BezPath::new();
    }
    if delta == 0.0 {
        return path.clone();
    }
    let (closed, open) = split_closed(path);
    let d = delta.abs();
    let ring = if closed.elements().is_empty() { BezPath::new() } else { stroke_bez(&closed, 2.0 * d, join, miter_limit) };
    let open_bp = if open.elements().is_empty() { BezPath::new() } else { stroke_bez(&open, 2.0 * d, join, miter_limit) };
    let Some(arr) = Arrangement::new(&[(closed, FillRule::NonZero), (ring, FillRule::NonZero), (open_bp, FillRule::NonZero)]) else {
        return BezPath::new();
    };
    let c = if delta >= 0.0 { arr.contours(|m| m[0] || m[1] || m[2]) } else { arr.contours(|m| m[0] && !m[1]) };
    let tidy = Tidy::free(DEFAULT_PRECISION);
    // When `d` exceeds a curvature radius the stroker's inner offset inverts and its loops cancel
    // winding, leaving faces uncovered that are really within `d` of the path. A contour is
    // spurious iff a point deep inside it lies closer than `d`: holes when growing, islands when
    // insetting.
    let drop_outer = delta < 0.0;
    let spurious: Vec<bool> =
        c.contours().map(|k| k.outer == drop_outer && deep_point(&k.path).is_some_and(|p| dist_to(path, p) < d * (1.0 - 1e-4) - 1e-6)).collect();
    let keep: Vec<&BezPath> = (0..spurious.len()).filter(|&i| !has_marked_ancestor(&c, i, &spurious)).map(|i| &c[ContourIdx(i)].path).collect();
    contours_to_path(keep, &tidy)
}

fn has_marked_ancestor(c: &Contours, mut i: usize, marked: &[bool]) -> bool {
    loop {
        if marked[i] {
            return true;
        }
        match c[ContourIdx(i)].parent {
            Some(ContourIdx(p)) => i = p,
            None => return false,
        }
    }
}

/// A point well inside a closed contour: the midpoint of the widest span on a few horizontal
/// lines through it.
fn deep_point(bp: &BezPath) -> Option<kurbo::Point> {
    use kurbo::Shape;
    let r = bp.bounding_box();
    if !(r.width() > 0.0 && r.height() > 0.0) {
        return None;
    }
    let mut best: Option<(f64, kurbo::Point)> = None;
    for f in [0.5, 0.3, 0.7] {
        let y = r.y0 + r.height() * f;
        let mut xs = vec![];
        let mut prev: Option<kurbo::Point> = None;
        let mut start = kurbo::Point::ZERO;
        let edge = |a: kurbo::Point, b: kurbo::Point, xs: &mut Vec<f64>| {
            if (a.y <= y) != (b.y <= y) {
                xs.push(a.x + (y - a.y) / (b.y - a.y) * (b.x - a.x));
            }
        };
        kurbo::flatten(bp.iter(), 0.01 * r.width().min(r.height()).max(1e-6), |el| match el {
            PathEl::MoveTo(p) => {
                start = p;
                prev = Some(p);
            }
            PathEl::LineTo(p) => {
                if let Some(a) = prev {
                    edge(a, p, &mut xs);
                }
                prev = Some(p);
            }
            PathEl::ClosePath => {
                if let Some(a) = prev {
                    edge(a, start, &mut xs);
                }
                prev = Some(start);
            }
            _ => {}
        });
        xs.sort_by(f64::total_cmp);
        for w in xs.as_chunks::<2>().0 {
            let span = w[1] - w[0];
            if best.is_none_or(|(b, _)| span > b) {
                best = Some((span, kurbo::Point::new((w[0] + w[1]) / 2.0, y)));
            }
        }
    }
    best.map(|(_, p)| p)
}

fn dist_to(bp: &BezPath, p: kurbo::Point) -> f64 {
    use kurbo::ParamCurveNearest;
    bp.segments().map(|s| s.nearest(p, 1e-6).distance_sq).fold(f64::INFINITY, f64::min).sqrt()
}
