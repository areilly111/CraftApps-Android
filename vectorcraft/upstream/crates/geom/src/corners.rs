//! Live Corners on any path: the anchors where two straight sides meet at an angle, and the path
//! with those corners cut round, inverted round or chamfered. Rectangles cut theirs the same way.

use kurbo::{Point, Vec2};

use crate::path::{Anchor, AnchorKind, PathData, SubPath};
use crate::shapes::{CornerKind, KAPPA};

/// Lengths and radii at or below this are none at all.
const EPS: f64 = 1e-9;
/// Sides closer than this to straight on (or folded back), as the sine of their angle, make no corner.
const MIN_SINE: f64 = 1e-6;

/// A corner Live Corners can cut: an anchor without handles between two straight sides that
/// meet at an angle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Corner {
    /// The anchor's index in its path, counting the anchors of every subpath in order.
    pub index: usize,
    /// Where the sides meet.
    pub at: Point,
    /// Unit directions along the sides: towards the previous anchor, then towards the next one.
    pub u: Vec2,
    pub v: Vec2,
    /// The sides' lengths, in the same order.
    pub lu: f64,
    pub lv: f64,
}

impl Corner {
    /// The corner of anchor `i` of `sp`, whose index in the path is `index`; `None` when the
    /// anchor is an end of an open subpath, has handles or is smooth, when a side curves or has
    /// no length, or when the sides run straight on.
    fn of(sp: &SubPath, i: usize, index: usize) -> Option<Self> {
        let n = sp.anchors.len();
        let (prev, next) = if sp.closed { ((i + n).checked_sub(1)? % n, (i + 1) % n) } else { (i.checked_sub(1)?, i + 1) };
        let (a, prev, next) = (sp.anchors.get(i)?, sp.anchors.get(prev)?, sp.anchors.get(next)?);
        if a.kind == AnchorKind::Smooth || a.has_in() || a.has_out() || prev.has_out() || next.has_in() {
            return None;
        }
        let (du, dv) = (prev.p - a.p, next.p - a.p);
        let (lu, lv) = (du.hypot(), dv.hypot());
        if !(lu > EPS && lv > EPS && lu.is_finite() && lv.is_finite()) {
            return None;
        }
        let c = Self { index, at: a.p, u: du / lu, v: dv / lv, lu, lv };
        (c.sin() > MIN_SINE).then_some(c)
    }

    /// The cosine of the angle between the sides.
    pub fn cos(&self) -> f64 {
        self.u.dot(self.v)
    }

    /// The sine of the angle between the sides.
    pub fn sin(&self) -> f64 {
        self.u.cross(self.v).abs()
    }

    /// The angle between the sides, in degrees (90 at a rectangle's corners).
    pub fn angle_deg(&self) -> f64 {
        self.sin().atan2(self.cos()).to_degrees()
    }

    /// How far from the corner a round cut of radius `r` meets each side: r / tan(angle / 2).
    pub fn setback(&self, r: f64) -> f64 {
        r * (1.0 + self.cos()) / self.sin()
    }

    /// The largest radius the corner draws: its cut then reaches halfway along the shorter side,
    /// so neighbouring corners at their largest meet without overlapping (half the shorter side
    /// at a rectangle's corners).
    pub fn max_radius(&self) -> f64 {
        self.lu.min(self.lv) / 2.0 * self.sin() / (1.0 + self.cos())
    }

    /// The radius the corner is drawn with: `radius`, no larger than [`Corner::max_radius`], 0 when
    /// negative or not a number.
    pub fn fitted(&self, radius: f64) -> f64 {
        radius.max(0.0).min(self.max_radius())
    }

    /// The point `k` along the bisector, measured in (u + v): the centre of a round cut of radius
    /// `r` is at `k = r / sin`.
    pub fn on_bisector(&self, k: f64) -> Point {
        self.at + (self.u + self.v) * k
    }

    /// How much the radius grows when the centre of the round cut moves by `d`: `d`'s part along
    /// the bisector, in radius units.
    pub fn radius_change(&self, d: Vec2) -> f64 {
        d.dot(self.u + self.v) * self.sin() / (2.0 + 2.0 * self.cos())
    }

    /// The two anchors replacing the corner when cut with `radius` (> 0, fitted): where the cut
    /// meets the arriving side, then the leaving one.
    fn cut(&self, radius: f64, kind: CornerKind) -> [Anchor; 2] {
        let t = self.setback(radius);
        let (start, end) = (self.at + self.u * t, self.at + self.v * t);
        let (h_out, h_in) = match kind {
            // An arc of `radius` tangent to both sides, turning by 180° less the angle.
            CornerKind::Round => {
                let h = radius * arc_handle(-self.cos());
                (start - self.u * h, end - self.v * h)
            }
            // An arc centred on the corner through both ends, spanning the angle.
            CornerKind::InvertedRound => {
                let h = t * arc_handle(self.cos());
                (start + square_to(self.u, self.v) * h, end + square_to(self.v, self.u) * h)
            }
            CornerKind::Chamfer => (start, end),
        };
        [Anchor { p: start, h_in: start, h_out, kind: AnchorKind::Corner }, Anchor { p: end, h_in, h_out: end, kind: AnchorKind::Corner }]
    }
}

/// The unit direction square to unit `a`, on `b`'s side.
fn square_to(a: Vec2, b: Vec2) -> Vec2 {
    let w = b - a * a.dot(b);
    w / w.hypot()
}

/// The handle length, per unit of radius, of a cubic drawing a circular arc that spans an angle
/// whose cosine is `cos`: 4/3 · tan(angle / 4) ([`KAPPA`] for a quarter circle).
fn arc_handle(cos: f64) -> f64 {
    if cos.abs() < 1e-12 {
        return KAPPA;
    }
    let (sin_half, cos_half) = (((1.0 - cos) / 2.0).max(0.0).sqrt(), ((1.0 + cos) / 2.0).max(0.0).sqrt());
    4.0 / 3.0 * sin_half / (1.0 + cos_half)
}

/// The corners of `path` Live Corners can cut, in anchor order.
pub fn path_corners(path: &PathData) -> Vec<Corner> {
    let mut out = vec![];
    let mut first = 0;
    for sp in &path.subpaths {
        out.extend((0..sp.anchors.len()).filter_map(|i| Corner::of(sp, i, first + i)));
        first += sp.anchors.len();
    }
    out
}

/// `path` with each of its corners ([`path_corners`]) cut by the radius in `radii` and the kind in
/// `kinds` at its anchor index (missing: 0 and round), each radius no larger than fits
/// ([`Corner::fitted`]). A cut corner becomes two anchors; a closed subpath whose first corner
/// is cut starts where that cut ends, so the cut closes it. Also returns, per subpath, the
/// anchor index in `path` each anchor comes from.
pub fn cut_corners(path: &PathData, radii: &[f64], kinds: &[CornerKind]) -> (PathData, Vec<Vec<usize>>) {
    let corners = path_corners(path);
    let mut corners = corners.iter().peekable();
    let (mut subpaths, mut sources) = (Vec::with_capacity(path.subpaths.len()), Vec::with_capacity(path.subpaths.len()));
    let mut first = 0;
    for sp in &path.subpaths {
        let (mut anchors, mut from) = (Vec::with_capacity(sp.anchors.len()), Vec::with_capacity(sp.anchors.len()));
        let mut closing = None;
        for (i, a) in sp.anchors.iter().enumerate() {
            let index = first + i;
            let corner = corners.next_if(|c| c.index == index);
            let radius = corner.map_or(0.0, |c| c.fitted(radii.get(index).copied().unwrap_or(0.0)));
            match corner {
                Some(c) if radius > EPS => {
                    let [start, end] = c.cut(radius, kinds.get(index).copied().unwrap_or_default());
                    if i == 0 && sp.closed {
                        closing = Some(start);
                    } else {
                        anchors.push(start);
                        from.push(index);
                    }
                    anchors.push(end);
                    from.push(index);
                }
                _ => {
                    anchors.push(*a);
                    from.push(index);
                }
            }
        }
        if let Some(a) = closing {
            anchors.push(a);
            from.push(first);
        }
        first += sp.anchors.len();
        subpaths.push(SubPath::new(anchors, sp.closed));
        sources.push(from);
    }
    (PathData::new(subpaths), sources)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shapes;
    use kurbo::Rect;

    fn near(a: Point, b: Point) -> bool {
        a.distance(b) < 1e-9
    }

    /// A five-pointed star's tips are acute corners, its inner corners obtuse: all ten cut, each
    /// a circle of the radius tangent to both sides.
    #[test]
    fn a_star_has_ten_corners_cut_as_circles() {
        let star = shapes::star(Point::new(100.0, 100.0), 50.0, 25.0, 5, 0.0);
        let corners = path_corners(&star);
        assert_eq!(corners.len(), 10);
        let angles: Vec<_> = corners.iter().map(|c| c.angle_deg()).collect();
        assert!(angles.iter().step_by(2).all(|a| *a < 90.0), "tips are acute: {angles:?}");
        assert!(angles.iter().skip(1).step_by(2).all(|a| *a > 90.0), "inner corners are obtuse: {angles:?}");
        let (cut, from) = cut_corners(&star, &[4.0; 10], &[]);
        assert_eq!(cut.anchor_count(), 20);
        assert_eq!(from, [vec![0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 0]]);
        for c in &corners {
            // Both ends of the cut lie `setback` along the sides, as far from the arc's centre as
            // the radius, and that centre is `radius` from either side.
            let t = c.setback(4.0);
            let o = c.on_bisector(4.0 / c.sin());
            for p in [c.at + c.u * t, c.at + c.v * t] {
                assert!((p.distance(o) - 4.0).abs() < 1e-9);
                assert!(cut.anchors().any(|(_, _, a)| near(a.p, p)), "{p:?} is an anchor");
            }
        }
    }

    #[test]
    fn radii_stop_at_half_the_shorter_side_per_corner() {
        let tri = PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(100.0, 0.0), Point::new(0.0, 40.0)], true));
        let corners = path_corners(&tri);
        assert_eq!(corners.len(), 3);
        for c in &corners {
            let max = c.max_radius();
            assert!((c.setback(max) - c.lu.min(c.lv) / 2.0).abs() < 1e-9);
            assert_eq!(c.fitted(1e6), max);
            assert_eq!(c.fitted(-3.0), 0.0);
            assert_eq!(c.fitted(f64::NAN), 0.0);
        }
        // The right angle at (0, 0) takes up to 20, half its shorter side.
        assert!((corners[0].max_radius() - 20.0).abs() < 1e-9);
    }

    #[test]
    fn ends_curves_smooth_and_straight_anchors_are_no_corners() {
        // An open zig-zag: its ends aren't corners, nor is the anchor its sides run straight on through.
        let open = PathData::single(SubPath::polyline(
            &[Point::new(0.0, 0.0), Point::new(50.0, 0.0), Point::new(100.0, 0.0), Point::new(100.0, 50.0), Point::new(150.0, 80.0)],
            false,
        ));
        assert_eq!(path_corners(&open).iter().map(|c| c.index).collect::<Vec<_>>(), [2, 3]);
        // A curve on either side, or handles on the anchor, leave it alone.
        let mut sp = SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(100.0, 0.0), Point::new(100.0, 100.0), Point::new(0.0, 100.0)], true);
        sp.anchors[1].h_out = Point::new(110.0, 10.0);
        let p = PathData::single(sp);
        assert_eq!(path_corners(&p).iter().map(|c| c.index).collect::<Vec<_>>(), [0, 3]);
        assert!(path_corners(&shapes::ellipse(Rect::new(0.0, 0.0, 10.0, 10.0))).is_empty());
        // A second subpath counts on from the first one's anchors.
        let two = PathData::new(vec![
            SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(0.0, 10.0)], true),
            SubPath::polyline(&[Point::new(50.0, 0.0), Point::new(60.0, 0.0), Point::new(50.0, 10.0)], true),
        ]);
        let (cut, from) = cut_corners(&two, &[0.0, 0.0, 0.0, 1.0], &[]);
        assert_eq!(from, [vec![0, 1, 2], vec![3, 4, 5, 3]]);
        assert_eq!(cut.anchor_count(), 7);
    }

    /// Inverted round and chamfer cuts end where the round one does, at any angle.
    #[test]
    fn kinds_share_their_ends() {
        let star = shapes::star(Point::new(0.0, 0.0), 50.0, 25.0, 5, 0.0);
        let ends = |k: CornerKind| -> Vec<Point> { cut_corners(&star, &[3.0; 10], &[k; 10]).0.anchors().map(|(_, _, a)| a.p).collect() };
        let round = ends(CornerKind::Round);
        assert_eq!(ends(CornerKind::InvertedRound), round);
        assert_eq!(ends(CornerKind::Chamfer), round);
        // The inverted arc stays on the circle about the corner.
        let c = path_corners(&star)[0];
        let [s, e] = c.cut(3.0, CornerKind::InvertedRound);
        let bez = kurbo::CubicBez::new(s.p, s.h_out, e.h_in, e.p);
        let r = c.setback(3.0);
        for t in [0.25, 0.5, 0.75] {
            use kurbo::ParamCurve;
            assert!((bez.eval(t).distance(c.at) - r).abs() < r * 1e-3);
        }
    }
}
