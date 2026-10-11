//! Every arrowhead's outline: drawn, inside its box (4 × 4 weights; the open arrow's square-cut
//! arms reach a little further back), without self-intersections, and covering the end of the
//! stroke under it.

use kurbo::{BezPath, PathEl, Point, Shape};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{ArrowAlign, Arrowhead, StrokeLayer};

use super::stroke_pieces;

/// A 2 pt line along +x ending at (100, 0) with `kind` at its end, tip on the end point: heads
/// of weight 2, so 8 long and 8 wide.
fn head(kind: Arrowhead) -> super::Arrow {
    let mut st = StrokeLayer::new(Paint::solid(Color::BLACK), 2.0);
    st.end_arrow = Some(kind);
    st.arrow_align = ArrowAlign::Tip;
    let mut l = BezPath::new();
    l.move_to((0.0, 0.0));
    l.line_to((100.0, 0.0));
    stroke_pieces(&l, &st).heads.remove(0)
}

/// The outline's closed contours as polygons (curves flattened).
fn contours(bp: &BezPath) -> Vec<Vec<Point>> {
    let mut out: Vec<Vec<Point>> = vec![];
    kurbo::flatten(bp.iter(), 1e-3, |el| match el {
        PathEl::MoveTo(p) => out.push(vec![p]),
        PathEl::LineTo(p) => out.last_mut().unwrap().push(p),
        PathEl::ClosePath => {
            let c = out.last_mut().unwrap();
            if c.len() > 1 && c[0].distance(*c.last().unwrap()) < 1e-9 {
                c.pop();
            }
        }
        _ => panic!("flattened"),
    });
    out
}

/// Do segments `a` and `b` cross at a point inside both (touching ends don't count)?
fn cross(a: (Point, Point), b: (Point, Point)) -> bool {
    let orient = |p: Point, q: Point, r: Point| (q - p).cross(r - p);
    let (d1, d2) = (orient(b.0, b.1, a.0), orient(b.0, b.1, a.1));
    let (d3, d4) = (orient(a.0, a.1, b.0), orient(a.0, a.1, b.1));
    d1 * d2 < -1e-12 && d3 * d4 < -1e-12
}

#[test]
fn every_outline_is_drawn_inside_its_box_without_self_intersections() {
    let (o, d) = (Point::ORIGIN, Point::new(2.0, 2.0));
    assert!(cross((o, d), (Point::new(0.0, 2.0), Point::new(2.0, 0.0))) && !cross((o, d), (d, Point::new(4.0, 0.0))), "the check itself");
    for kind in Arrowhead::ALL {
        let h = head(kind);
        assert!(h.outline.area().abs() > 4.0, "{kind:?} is drawn");
        let b = h.outline.bounding_box();
        assert!(b.x0 >= 91.0 - 1e-6 && b.x1 <= 100.0 + 1e-6 && b.y0 >= -4.0 - 1e-6 && b.y1 <= 4.0 + 1e-6, "{kind:?} leaves its box: {b:?}");
        let edges: Vec<(Point, Point)> = contours(&h.outline).iter().flat_map(|c| (0..c.len()).map(|i| (c[i], c[(i + 1) % c.len()]))).collect();
        assert!(edges.len() >= 3, "{kind:?}");
        for (i, a) in edges.iter().enumerate() {
            for b in &edges[i + 1..] {
                assert!(!cross(*a, *b), "{kind:?}: {a:?} crosses {b:?}");
            }
        }
    }
}

#[test]
fn the_stroke_ends_inside_every_head() {
    for kind in Arrowhead::ALL {
        let h = head(kind);
        // Just in front of where the line stops, on its centre line.
        let end = h.tip - h.dir * (h.inset - 0.1);
        assert_ne!(h.outline.winding(end), 0, "{kind:?}: the line ends outside the head (inset {})", h.inset);
    }
}

#[test]
fn half_arrows_barb_on_their_own_side() {
    // Pointing along +x on a y-down page, left is −y.
    assert!(head(Arrowhead::HalfArrowLeft).outline.bounding_box().y0 < -3.0);
    assert!(head(Arrowhead::HalfArrowLeft).outline.bounding_box().y1 < 1.5);
    assert!(head(Arrowhead::HalfArrowRight).outline.bounding_box().y1 > 3.0);
}
