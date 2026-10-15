//! Hit testing primitives.

use std::borrow::Cow;

use kurbo::{BezPath, ParamCurveNearest, PathEl, Point, Rect, Shape};

use crate::path::{FillRule, PathData};

/// Is `p` inside the filled area of `path` under `rule`? Open subpaths are implicitly closed (as when filled).
pub fn fill_contains(path: &BezPath, rule: FillRule, p: Point) -> bool {
    let w = closed(path).winding(p);
    match rule {
        FillRule::NonZero => w != 0,
        FillRule::EvenOdd => w % 2 != 0,
    }
}

/// `path` with each open subpath closed by a straight line back to its start, the edge its fill
/// has. kurbo's winding leaves that edge out, so a point beside it counted as outside the fill:
/// an open path lost its fill clicks on that side (a reflection moved the edge to the other one).
fn closed(path: &BezPath) -> Cow<'_, BezPath> {
    let mut open = false;
    let mut needs = false;
    for el in path.elements() {
        match el {
            PathEl::MoveTo(_) => {
                needs |= open;
                open = true;
            }
            PathEl::ClosePath => open = false,
            _ => {}
        }
    }
    if !(needs || open) {
        return Cow::Borrowed(path);
    }
    let mut out = BezPath::new();
    let mut open = false;
    for &el in path.elements() {
        match el {
            PathEl::MoveTo(_) => {
                if open {
                    out.close_path();
                }
                open = true;
            }
            PathEl::ClosePath => open = false,
            _ => {}
        }
        out.push(el);
    }
    if open {
        out.close_path();
    }
    Cow::Owned(out)
}

/// Distance from `p` to the nearest point on the path outline.
pub fn distance_to_outline(path: &BezPath, p: Point) -> f64 {
    path.segments().map(|s| s.nearest(p, 1e-9).distance_sq).fold(f64::INFINITY, f64::min).sqrt()
}

/// Is `p` within `tol` of the path's stroke of width `width`?
pub fn stroke_contains(path: &BezPath, width: f64, tol: f64, p: Point) -> bool {
    let quick = path.bounding_box().inflate(width / 2.0 + tol, width / 2.0 + tol);
    quick.contains(p) && distance_to_outline(path, p) <= width / 2.0 + tol
}

/// Does the path touch the rect: an anchor inside it, or a segment crossing its edges? Used for
/// marquee selection, which (as in Illustrator) needs the path itself: a rect wholly inside a
/// filled shape doesn't touch it, so a marquee drawn over a background picks only what is on it
/// (#1103).
pub fn intersects_rect(path: &PathData, r: Rect) -> bool {
    let Some(b) = path.bounds() else { return false };
    if b.intersect(r).area() <= 0.0 && !(b.width() == 0.0 || b.height() == 0.0) {
        return false;
    }
    // Any anchor inside?
    if path.anchors().any(|(_, _, a)| r.contains(a.p)) {
        return true;
    }
    // Any segment crossing the rect edges?
    let edges = [
        kurbo::Line::new((r.x0, r.y0), (r.x1, r.y0)),
        kurbo::Line::new((r.x1, r.y0), (r.x1, r.y1)),
        kurbo::Line::new((r.x1, r.y1), (r.x0, r.y1)),
        kurbo::Line::new((r.x0, r.y1), (r.x0, r.y0)),
    ];
    path.to_bezpath().segments().any(|seg| edges.iter().any(|e| !seg.intersect_line(*e).is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shapes;

    #[test]
    fn fill_and_stroke() {
        let p = shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)).to_bezpath();
        assert!(fill_contains(&p, FillRule::NonZero, Point::new(5.0, 5.0)));
        assert!(!fill_contains(&p, FillRule::NonZero, Point::new(15.0, 5.0)));
        assert!(stroke_contains(&p, 2.0, 0.0, Point::new(10.5, 5.0)));
        assert!(!stroke_contains(&p, 2.0, 0.0, Point::new(12.0, 5.0)));
    }

    #[test]
    fn evenodd_hole() {
        let mut outer = shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0));
        outer.subpaths.extend(shapes::rectangle(Rect::new(3.0, 3.0, 7.0, 7.0)).subpaths);
        let bp = outer.to_bezpath();
        assert!(!fill_contains(&bp, FillRule::EvenOdd, Point::new(5.0, 5.0)));
        assert!(fill_contains(&bp, FillRule::NonZero, Point::new(5.0, 5.0)));
        assert!(fill_contains(&bp, FillRule::EvenOdd, Point::new(1.0, 5.0)));
    }

    /// An open path fills as if closed by a straight line back to its start, and is hit there,
    /// whichever side that line falls on (a reflected open path lost its fill clicks).
    #[test]
    fn open_paths_fill_as_closed_on_either_side() {
        // A filled sliver left open by a deleted anchor; its closing line runs down its left side.
        let open = BezPath::from_svg(
            "M86.311 69.612C100.544 78.261 110.506 88.64 110.506 100.748C110.506 111.127 100.544 118.046 86.311 118.046C93.214 102.045 91.407 102.403 91.407 97.61",
        )
        .unwrap();
        let mut closed = open.clone();
        closed.close_path();
        for p in [Point::new(100.0, 100.0), Point::new(95.0, 85.0), Point::new(92.0, 112.0)] {
            assert!(closed.winding(p) != 0, "{p:?} is inside the closed sliver");
            assert!(fill_contains(&open, FillRule::NonZero, p), "{p:?}: the open sliver's fill");
            assert!(fill_contains(&open, FillRule::EvenOdd, p), "{p:?}: even-odd too");
        }
        assert!(!fill_contains(&open, FillRule::NonZero, Point::new(80.0, 100.0)), "left of the closing line");
        // Mirrored, the closing line is on the other side: still filled.
        let flipped = kurbo::Affine::new([-1.0, 0.0, 0.0, 1.0, 200.0, 0.0]) * open.clone();
        assert!(fill_contains(&flipped, FillRule::NonZero, Point::new(100.0, 100.0)));
        // Two open subpaths: each closes on its own.
        let mut two = open.clone();
        two.extend(kurbo::Affine::translate((0.0, 100.0)) * open.clone());
        assert!(fill_contains(&two, FillRule::NonZero, Point::new(100.0, 100.0)) && fill_contains(&two, FillRule::NonZero, Point::new(100.0, 200.0)));
    }

    #[test]
    fn marquee() {
        let p = shapes::ellipse(Rect::new(0.0, 0.0, 10.0, 10.0));
        assert!(intersects_rect(&p, Rect::new(-1.0, 4.0, 1.0, 6.0)));
        assert!(!intersects_rect(&p, Rect::new(20.0, 20.0, 30.0, 30.0)));
        // corner region of the bbox that misses the ellipse outline
        assert!(!intersects_rect(&p, Rect::new(0.0, 0.0, 0.5, 0.5)));
        // Wholly inside the shape, touching neither its anchors nor its outline (#1103).
        assert!(!intersects_rect(&p, Rect::new(4.0, 4.0, 6.0, 6.0)));
        // Around the whole shape.
        assert!(intersects_rect(&p, Rect::new(-1.0, -1.0, 11.0, 11.0)));
        // crossing a line
        let l = shapes::line(Point::new(0.0, 0.0), Point::new(10.0, 10.0));
        assert!(intersects_rect(&l, Rect::new(4.0, 0.0, 6.0, 10.0)));
    }
}

#[cfg(test)]
mod regress_tests {
    use super::*;
    use kurbo::{CubicBez, ParamCurve};

    /// A cubic whose handles sit on its anchors on one side (cusp-like): points on the curve are
    /// at distance ~0 (proptest regression).
    #[test]
    fn nearest_on_degenerate_cubic() {
        let c = CubicBez::new(
            (131.69052599341336, 172.64518298885875),
            (104.00879084385166, 172.64518298885875),
            (145.71215277397286, 141.82358989609716),
            (131.4188160942415, 162.38678549385997),
        );
        let mut bp = BezPath::new();
        bp.move_to(c.p0);
        bp.curve_to(c.p1, c.p2, c.p3);
        let worst = (0..=200).map(|i| distance_to_outline(&bp, c.eval(i as f64 / 200.0))).fold(0.0, f64::max);
        assert!(worst < 1e-6, "{worst}");
    }
}
