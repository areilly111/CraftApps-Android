//! Shape generators matching Illustrator's anchor layouts (so later edits behave the same).

use std::f64::consts::{PI, TAU};

use kurbo::{Point, Rect, Vec2};
use serde::{Deserialize, Serialize};

use crate::path::{Anchor, AnchorKind, PathData, SubPath};

/// Circle-approximation handle ratio for a quarter arc.
pub const KAPPA: f64 = 0.552_284_749_830_793_6;

/// Rectangle with 4 corner anchors, clockwise from the top-left.
pub fn rectangle(r: Rect) -> PathData {
    let r = r.abs();
    PathData::single(SubPath::polyline(&[Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)], true))
}

/// Rounded rectangle (8 anchors). The radius is clamped to half the shorter side.
pub fn rounded_rectangle(r: Rect, radius: f64) -> PathData {
    rectangle_with_corners(r, [radius; 4], [CornerKind::Round; 4])
}

/// How a rectangle's corner is cut (Live Corners).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CornerKind {
    /// A quarter circle, bulging outwards.
    #[default]
    Round,
    /// A quarter circle cut into the shape, centred on the corner.
    InvertedRound,
    /// A straight bevel.
    Chamfer,
}

impl CornerKind {
    pub const ALL: [CornerKind; 3] = [CornerKind::Round, CornerKind::InvertedRound, CornerKind::Chamfer];

    /// The kind after this one (Alt-clicking a corner widget cycles through them).
    pub fn next(self) -> Self {
        match self {
            CornerKind::Round => CornerKind::InvertedRound,
            CornerKind::InvertedRound => CornerKind::Chamfer,
            CornerKind::Chamfer => CornerKind::Round,
        }
    }
}

/// The largest corner radius a `w` × `h` rectangle draws: half its shorter side.
pub fn max_corner_radius(w: f64, h: f64) -> f64 {
    w.abs().min(h.abs()) / 2.0
}

/// The radius a corner of a `w` × `h` rectangle is drawn with: `radius`, no larger than
/// [`max_corner_radius`] (the same limit for every corner, so opposite corners match), 0 when
/// negative or not a number.
pub fn fitted_corner_radius(w: f64, h: f64, radius: f64) -> f64 {
    radius.max(0.0).min(max_corner_radius(w, h))
}

/// Rectangle whose corners (top-left, top-right, bottom-right, bottom-left, y down) each have
/// their own radius and kind, every radius clamped to half the shorter side. A corner without a
/// radius is one anchor, the others two (where the cut meets each side). Clockwise from the
/// top-left corner, its anchor on the top side first. Cut as any path's corners are
/// ([`crate::corners::cut_corners`]).
pub fn rectangle_with_corners(r: Rect, radii: [f64; 4], kinds: [CornerKind; 4]) -> PathData {
    crate::corners::cut_corners(&rectangle(r), &radii, &kinds).0
}

/// Ellipse inscribed in `r` with 4 smooth anchors (left, top, right, bottom).
pub fn ellipse(r: Rect) -> PathData {
    let r = r.abs();
    let c = r.center();
    let rx = r.width() / 2.0;
    let ry = r.height() / 2.0;
    let kx = rx * KAPPA;
    let ky = ry * KAPPA;
    let s = |p: Point, hin: Vec2| Anchor { p, h_in: p + hin, h_out: p - hin, kind: AnchorKind::Smooth };
    let anchors = vec![
        s(Point::new(c.x - rx, c.y), Vec2::new(0.0, ky)),
        s(Point::new(c.x, c.y - ry), Vec2::new(-kx, 0.0)),
        s(Point::new(c.x + rx, c.y), Vec2::new(0.0, -ky)),
        s(Point::new(c.x, c.y + ry), Vec2::new(kx, 0.0)),
    ];
    PathData::single(SubPath::new(anchors, true))
}

/// The ellipse inscribed in `r` cut as a pie: from `start` to `end` degrees, measured
/// counterclockwise on screen from 3 o'clock (as Illustrator's Pie Start and End Angle are), then
/// back through the centre. A pie spanning the whole turn (or nothing) is the whole [`ellipse`].
/// The arc is one cubic per quarter turn at most: the centre, then the arc's anchors.
pub fn ellipse_pie(r: Rect, start: f64, end: f64) -> PathData {
    let span = (end - start).rem_euclid(360.0);
    if !(start.is_finite() && end.is_finite()) || span < 1e-9 || 360.0 - span < 1e-9 {
        return ellipse(r);
    }
    let r = r.abs();
    let c = r.center();
    let (rx, ry) = (r.width() / 2.0, r.height() / 2.0);
    // Screen y runs down: counterclockwise on screen is a falling y.
    let at = |deg: f64| {
        let t = deg.to_radians();
        Point::new(c.x + rx * t.cos(), c.y - ry * t.sin())
    };
    let tangent = |deg: f64| {
        let t = deg.to_radians();
        Vec2::new(-rx * t.sin(), -ry * t.cos())
    };
    let pieces = if span <= 90.0 {
        1
    } else if span <= 180.0 {
        2
    } else if span <= 270.0 {
        3
    } else {
        4
    };
    let step = span / f64::from(pieces);
    let k = 4.0 / 3.0 * (step.to_radians() / 4.0).tan();
    let mut anchors = vec![Anchor::corner(c)];
    for i in 0..=pieces {
        let a = start + step * f64::from(i);
        let (p, d) = (at(a), tangent(a) * k);
        let h_in = if i == 0 { p } else { p - d };
        let h_out = if i == pieces { p } else { p + d };
        let kind = if i == 0 || i == pieces { AnchorKind::Corner } else { AnchorKind::Smooth };
        anchors.push(Anchor { p, h_in, h_out, kind });
    }
    PathData::single(SubPath::new(anchors, true))
}

/// Regular polygon with `sides` (≥ 3), first vertex straight up from the centre.
pub fn polygon(center: Point, radius: f64, sides: u32, rotation_deg: f64) -> PathData {
    let n = sides.max(3);
    let rot = rotation_deg.to_radians();
    let pts: Vec<Point> = (0..n)
        .map(|i| {
            let a = -PI / 2.0 + rot + TAU * i as f64 / n as f64;
            Point::new(center.x + radius * a.cos(), center.y + radius * a.sin())
        })
        .collect();
    PathData::single(SubPath::polyline(&pts, true))
}

/// Star with `points` tips (≥ 2), outer radius `r1`, inner radius `r2`, first tip up.
pub fn star(center: Point, r1: f64, r2: f64, points: u32, rotation_deg: f64) -> PathData {
    let n = points.max(2) * 2;
    let rot = rotation_deg.to_radians();
    let pts: Vec<Point> = (0..n)
        .map(|i| {
            let r = if i % 2 == 0 { r1 } else { r2 };
            let a = -PI / 2.0 + rot + TAU * i as f64 / n as f64;
            Point::new(center.x + r * a.cos(), center.y + r * a.sin())
        })
        .collect();
    PathData::single(SubPath::polyline(&pts, true))
}

/// Inner radius of a "straight-armed" star (Illustrator's Alt-drag behaviour): shoulders aligned.
pub fn straight_star_inner_radius(r1: f64, points: u32) -> f64 {
    let n = points.max(3) as f64;
    r1 * (PI / n * 2.0).cos() / (PI / n).cos()
}

/// Open straight line.
pub fn line(a: Point, b: Point) -> PathData {
    PathData::single(SubPath::polyline(&[a, b], false))
}

/// Quarter-ellipse arc from `a` to `b` (Arc tool default: concave, open, x-axis base).
pub fn arc(a: Point, b: Point, slope: f64, closed: bool) -> PathData {
    // Corner of the arc's bounding box is at (a.x, b.y): the arc bows away from it.
    let k = KAPPA * (1.0 + slope.clamp(-1.0, 1.0) * 0.0);
    let corner = Point::new(b.x, a.y);
    let h1 = a + (corner - a) * k;
    let h2 = b + (corner - b) * k;
    let mut anchors =
        vec![Anchor { p: a, h_in: a, h_out: h1, kind: AnchorKind::Corner }, Anchor { p: b, h_in: h2, h_out: b, kind: AnchorKind::Corner }];
    if closed {
        let o = Point::new(a.x, b.y);
        anchors.push(Anchor::corner(o));
    }
    PathData::single(SubPath::new(anchors, closed))
}

/// Archimedean-style spiral approximating Illustrator's Spiral tool (decay-based, `segments` quarter turns).
pub fn spiral(center: Point, radius: f64, decay_percent: f64, segments: u32, clockwise: bool) -> PathData {
    let decay = (decay_percent / 100.0).clamp(0.05, 0.9999);
    let dir = if clockwise { 1.0 } else { -1.0 };
    let mut anchors = Vec::new();
    let mut r = radius;
    for i in 0..=segments {
        let a = dir * (i as f64) * PI / 2.0;
        let p = Point::new(center.x + r * a.cos(), center.y + r * a.sin());
        // Tangent direction for a quarter arc of radius r.
        let t = Vec2::new(-a.sin(), a.cos()) * dir;
        let r_prev = r / decay;
        let r_next = r * decay;
        let h_in = p - t * (KAPPA * (r + r_prev) / 2.0);
        let h_out = p + t * (KAPPA * (r + r_next) / 2.0);
        anchors.push(Anchor { p, h_in: if i == 0 { p } else { h_in }, h_out: if i == segments { p } else { h_out }, kind: AnchorKind::Smooth });
        r *= decay;
    }
    // Illustrator spirals wind inward from the outer point; reverse so the path starts at the centre end.
    let mut sp = SubPath::new(anchors, false);
    sp.reverse();
    PathData::single(sp)
}

/// Rectangular grid as separate open lines (plus an optional frame rectangle).
pub fn rectangular_grid(r: Rect, h_dividers: u32, v_dividers: u32, frame: bool) -> Vec<PathData> {
    let r = r.abs();
    let mut out = Vec::new();
    for i in 1..=h_dividers {
        let y = r.y0 + r.height() * i as f64 / (h_dividers + 1) as f64;
        out.push(line(Point::new(r.x0, y), Point::new(r.x1, y)));
    }
    for i in 1..=v_dividers {
        let x = r.x0 + r.width() * i as f64 / (v_dividers + 1) as f64;
        out.push(line(Point::new(x, r.y0), Point::new(x, r.y1)));
    }
    if frame {
        out.push(rectangle(r));
    }
    out
}

/// Polar grid: concentric ellipses plus radial dividers.
pub fn polar_grid(r: Rect, concentric: u32, radial: u32) -> Vec<PathData> {
    let r = r.abs();
    let c = r.center();
    let mut out = Vec::new();
    for i in 1..=concentric + 1 {
        let f = i as f64 / (concentric + 1) as f64;
        out.push(ellipse(Rect::from_center_size(c, (r.width() * f, r.height() * f))));
    }
    // No dividers at 0 (the loop never divides by it).
    for i in 0..radial {
        let a = -PI / 2.0 + TAU * i as f64 / radial as f64;
        out.push(line(c, Point::new(c.x + r.width() / 2.0 * a.cos(), c.y + r.height() / 2.0 * a.sin())));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The corner (index into `radii`) of each anchor of [`rectangle_with_corners`].
    fn corner_sources(r: Rect, radii: [f64; 4]) -> Vec<usize> {
        crate::corners::cut_corners(&rectangle(r), &radii, &[]).1.concat()
    }

    /// A pie is the arc from its start to its end angle, counterclockwise on screen, closed through
    /// the centre; a whole turn (or none) is the ellipse.
    #[test]
    fn a_pie_is_its_arc_closed_through_the_centre() {
        let r = Rect::new(0.0, 0.0, 200.0, 100.0);
        assert_eq!(ellipse_pie(r, 0.0, 360.0), ellipse(r));
        assert_eq!(ellipse_pie(r, 90.0, 90.0), ellipse(r));
        // A quarter, from 3 o'clock up to 12: the centre, then the arc's two ends.
        let q = ellipse_pie(r, 0.0, 90.0);
        let a: Vec<Point> = q.subpaths[0].anchors.iter().map(|a| a.p).collect();
        assert_eq!(a.len(), 3);
        assert!(a[0].distance(Point::new(100.0, 50.0)) < 1e-9 && a[1].distance(Point::new(200.0, 50.0)) < 1e-9);
        assert!(a[2].distance(Point::new(100.0, 0.0)) < 1e-9, "counterclockwise on screen: up");
        assert_eq!(q.bounds().map(|b| (b.x0.round(), b.y0.round(), b.x1.round(), b.y1.round())), Some((100.0, 0.0, 200.0, 50.0)));
        // Three quarters, from 12 round to 3 o'clock: four pieces at most, the whole box covered.
        let big = ellipse_pie(r, 90.0, 0.0);
        assert!(big.subpaths[0].anchors.len() <= 6 && big.subpaths[0].closed);
        assert_eq!(big.bounds().map(|b| (b.x0.round(), b.y0.round(), b.x1.round(), b.y1.round())), Some((0.0, 0.0, 200.0, 100.0)));
        assert!(ellipse_pie(r, f64::NAN, 10.0).anchor_count() == 4, "not a number: the whole ellipse");
    }

    #[test]
    fn rect_is_4_corners_clockwise() {
        let p = rectangle(Rect::new(0.0, 0.0, 100.0, 50.0));
        assert_eq!(p.anchor_count(), 4);
        assert!(p.subpaths[0].area() > 0.0);
        assert_eq!(p.bounds(), Some(Rect::new(0.0, 0.0, 100.0, 50.0)));
    }

    #[test]
    fn rect_normalizes_negative() {
        let p = rectangle(Rect::new(10.0, 10.0, 0.0, 0.0));
        assert_eq!(p.bounds(), Some(Rect::new(0.0, 0.0, 10.0, 10.0)));
    }

    #[test]
    fn ellipse_bounds_and_area() {
        let p = ellipse(Rect::new(0.0, 0.0, 100.0, 60.0));
        let b = p.bounds().unwrap();
        assert!((b.width() - 100.0).abs() < 1e-6 && (b.height() - 60.0).abs() < 1e-6);
        let area = p.subpaths[0].area().abs();
        assert!((area - PI * 50.0 * 30.0).abs() / area < 0.001);
    }

    #[test]
    fn rounded_rect_clamps() {
        let p = rounded_rectangle(Rect::new(0.0, 0.0, 20.0, 10.0), 100.0);
        assert_eq!(p.anchor_count(), 8);
        assert_eq!(p.bounds(), Some(Rect::new(0.0, 0.0, 20.0, 10.0)));
        assert_eq!(rounded_rectangle(Rect::new(0.0, 0.0, 20.0, 10.0), 0.0).anchor_count(), 4);
    }

    #[test]
    fn uniform_corners_keep_the_rounded_rectangle_layout() {
        // The 8 anchors a rounded rectangle always had, from the top side's first one.
        let (r, rad) = (Rect::new(10.0, 20.0, 110.0, 70.0), 7.5);
        let (x0, y0, x1, y1, k) = (10.0, 20.0, 110.0, 70.0, rad * KAPPA);
        let a =
            |p: (f64, f64), hin: (f64, f64), hout: (f64, f64)| Anchor { p: p.into(), h_in: hin.into(), h_out: hout.into(), kind: AnchorKind::Corner };
        let want = vec![
            a((x0 + rad, y0), (x0 + rad - k, y0), (x0 + rad, y0)),
            a((x1 - rad, y0), (x1 - rad, y0), (x1 - rad + k, y0)),
            a((x1, y0 + rad), (x1, y0 + rad - k), (x1, y0 + rad)),
            a((x1, y1 - rad), (x1, y1 - rad), (x1, y1 - rad + k)),
            a((x1 - rad, y1), (x1 - rad + k, y1), (x1 - rad, y1)),
            a((x0 + rad, y1), (x0 + rad, y1), (x0 + rad - k, y1)),
            a((x0, y1 - rad), (x0, y1 - rad + k), (x0, y1 - rad)),
            a((x0, y0 + rad), (x0, y0 + rad), (x0, y0 + rad - k)),
        ];
        assert_eq!(rounded_rectangle(r, rad).subpaths[0].anchors, want);
        assert_eq!(corner_sources(r, [rad; 4]), [0, 1, 1, 2, 2, 3, 3, 0]);
        assert_eq!(rounded_rectangle(r, 0.0), rectangle(r));
    }

    #[test]
    fn each_corner_takes_its_own_radius_and_kind() {
        let r = Rect::new(0.0, 0.0, 100.0, 60.0);
        // Only the top-right corner rounded: 5 anchors, the corner's two on its sides.
        let radii = [0.0, 10.0, 0.0, 0.0];
        let p = rectangle_with_corners(r, radii, [CornerKind::Round; 4]);
        let pts: Vec<Point> = p.subpaths[0].anchors.iter().map(|a| a.p).collect();
        assert_eq!(pts, [Point::new(0.0, 0.0), Point::new(90.0, 0.0), Point::new(100.0, 10.0), Point::new(100.0, 60.0), Point::new(0.0, 60.0)]);
        assert_eq!(corner_sources(r, radii), [0, 1, 1, 2, 3]);
        let quarter = PI * 25.0;
        assert!((p.subpaths[0].area() - (6000.0 - 100.0 + quarter)).abs() < 0.1, "a quarter circle off one corner");
        // The top-left corner rounded: its top-side anchor first, its left-side one last.
        assert_eq!(corner_sources(r, [10.0, 0.0, 0.0, 200.0]), [0, 1, 2, 3, 3, 0]);
        // Inverted round cuts a quarter circle out; a chamfer cuts a triangle.
        let inverted = rectangle_with_corners(r, radii, [CornerKind::InvertedRound; 4]);
        assert!((inverted.subpaths[0].area() - (6000.0 - quarter)).abs() < 0.1);
        let chamfer = rectangle_with_corners(r, radii, [CornerKind::Chamfer; 4]);
        assert!((chamfer.subpaths[0].area() - (6000.0 - 50.0)).abs() < 1e-9);
        assert!(chamfer.subpaths[0].anchors.iter().all(|a| !a.has_in() && !a.has_out()), "straight");
        // Radii clamp to half the shorter side; negative and NaN radii are none.
        let p = rectangle_with_corners(r, [500.0, -5.0, f64::NAN, 0.0], [CornerKind::Round; 4]);
        assert_eq!(p.subpaths[0].anchors[0].p, Point::new(30.0, 0.0));
        assert_eq!(p.anchor_count(), 5);
        assert_eq!(CornerKind::ALL.map(CornerKind::next), [CornerKind::InvertedRound, CornerKind::Chamfer, CornerKind::Round]);
    }

    /// #442: on a rectangle that isn't square, every corner is a circular arc of the same radius,
    /// and radii past the limit stop at half the shorter side, whichever side a corner is on.
    #[test]
    fn corners_of_a_non_square_rectangle_are_alike() {
        let r = Rect::new(0.0, 0.0, 200.0, 80.0);
        assert_eq!(max_corner_radius(-200.0, 80.0), 40.0);
        assert_eq!([30.0, 60.0, -1.0, f64::NAN].map(|x| fitted_corner_radius(200.0, 80.0, x)), [30.0, 40.0, 0.0, 0.0]);
        for (radius, drawn) in [(30.0, 30.0), (60.0, 40.0)] {
            for kinds in [[CornerKind::Round; 4], [CornerKind::Chamfer, CornerKind::InvertedRound, CornerKind::Round, CornerKind::Chamfer]] {
                let a: Vec<Point> = rectangle_with_corners(r, [radius; 4], kinds).subpaths[0].anchors.iter().map(|a| a.p).collect();
                for (i, j) in [(7, 0), (1, 2), (3, 4), (5, 6)] {
                    let (x, y) = ((a[j].x - a[i].x).abs(), (a[j].y - a[i].y).abs());
                    assert!((x - drawn).abs() < 1e-9 && (y - drawn).abs() < 1e-9, "radius {radius}: a corner spans {x} × {y}");
                }
            }
        }
    }

    #[test]
    fn polygon_first_vertex_up() {
        let p = polygon(Point::new(0.0, 0.0), 10.0, 6, 0.0);
        assert_eq!(p.anchor_count(), 6);
        let a = p.subpaths[0].anchors[0].p;
        assert!(a.x.abs() < 1e-9 && (a.y + 10.0).abs() < 1e-9);
    }

    #[test]
    fn star_alternates_radii() {
        let p = star(Point::ZERO, 10.0, 5.0, 5, 0.0);
        assert_eq!(p.anchor_count(), 10);
        let d: Vec<f64> = p.subpaths[0].anchors.iter().map(|a| a.p.to_vec2().hypot()).collect();
        assert!((d[0] - 10.0).abs() < 1e-9 && (d[1] - 5.0).abs() < 1e-9);
    }

    #[test]
    fn straight_star_five_points() {
        let r2 = straight_star_inner_radius(100.0, 5);
        assert!((r2 - 38.196_601).abs() < 1e-3);
    }

    #[test]
    fn spiral_has_segments() {
        let p = spiral(Point::ZERO, 100.0, 80.0, 10, true);
        assert_eq!(p.anchor_count(), 11);
        assert!(!p.subpaths[0].closed);
    }

    #[test]
    fn grids() {
        assert_eq!(rectangular_grid(Rect::new(0.0, 0.0, 10.0, 10.0), 5, 5, true).len(), 11);
        assert_eq!(polar_grid(Rect::new(0.0, 0.0, 10.0, 10.0), 5, 5).len(), 11);
        // #1008: no radial dividers is none, only the outer ellipse without concentric ones.
        for (radial, len) in [(0, 1), (1, 2), (2, 3)] {
            assert_eq!(polar_grid(Rect::new(0.0, 0.0, 10.0, 10.0), 0, radial).len(), len, "{radial}");
        }
    }

    #[test]
    fn arc_open_and_closed() {
        assert_eq!(arc(Point::ZERO, Point::new(10.0, 10.0), 0.0, false).anchor_count(), 2);
        assert!(arc(Point::ZERO, Point::new(10.0, 10.0), 0.0, true).subpaths[0].closed);
    }
}
