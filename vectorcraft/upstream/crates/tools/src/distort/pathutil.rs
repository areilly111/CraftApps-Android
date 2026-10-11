//! Arc-length helpers: width points live at a fraction (0..1) of a subpath's length.

use vectorcraft_geom::kurbo::{ParamCurve, ParamCurveArclen, ParamCurveDeriv};
use vectorcraft_geom::{PathData, Point, SubPath, Vec2};

const ACC: f64 = 1e-4;

/// Arc length of each segment.
pub fn seg_lengths(sp: &SubPath) -> Vec<f64> {
    (0..sp.segment_count()).map(|i| sp.segment(i).arclen(ACC)).collect()
}

/// Fraction of the subpath's length at segment `seg`, parameter `t`.
pub fn fraction_at(sp: &SubPath, seg: usize, t: f64) -> f64 {
    let l = seg_lengths(sp);
    let total: f64 = l.iter().sum();
    if total <= 1e-12 {
        return 0.0;
    }
    let before: f64 = l[..seg.min(l.len())].iter().sum();
    let part = if seg < l.len() { sp.segment(seg).subsegment(0.0..t.clamp(0.0, 1.0)).arclen(ACC) } else { 0.0 };
    ((before + part) / total).clamp(0.0, 1.0)
}

/// Point and unit tangent at fraction `f` of the subpath's length.
pub fn eval_fraction(sp: &SubPath, f: f64) -> Option<(Point, Vec2)> {
    let l = seg_lengths(sp);
    let total: f64 = l.iter().sum();
    if l.is_empty() {
        return sp.anchors.first().map(|a| (a.p, Vec2::new(1.0, 0.0)));
    }
    let mut target = f.clamp(0.0, 1.0) * total;
    for (i, len) in l.iter().enumerate() {
        if target <= *len || i == l.len() - 1 {
            let c = sp.segment(i);
            let t = if *len > 1e-12 { c.inv_arclen(target.min(*len), ACC) } else { 0.0 };
            return Some((c.eval(t), tangent(&c, t)));
        }
        target -= len;
    }
    None
}

/// Unit tangent of a cubic at `t` (falls back to the chord for degenerate derivatives).
pub fn tangent(c: &vectorcraft_geom::CubicBez, t: f64) -> Vec2 {
    let d = c.deriv().eval(t).to_vec2();
    let d = if d.hypot() > 1e-9 {
        d
    } else {
        let d2 = c.eval((t + 0.01).min(1.0)) - c.eval((t - 0.01).max(0.0));
        if d2.hypot() > 1e-12 { d2 } else { c.p3 - c.p0 }
    };
    let h = d.hypot();
    if h > 1e-12 { d / h } else { Vec2::new(1.0, 0.0) }
}

/// The left normal of a tangent in y-down document space (left of the direction of travel).
pub fn left_normal(t: Vec2) -> Vec2 {
    Vec2::new(t.y, -t.x)
}

/// Nearest point of `path` to `p`: (subpath index, fraction along it, point, unit tangent, distance).
pub fn nearest_fraction(path: &PathData, p: Point) -> Option<(usize, f64, Point, Vec2, f64)> {
    let (si, seg, t, q, d) = path.nearest(p)?;
    let sp = &path.subpaths[si];
    Some((si, fraction_at(sp, seg, t), q, tangent(&sp.segment(seg), t), d))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fractions_round_trip_on_a_polyline() {
        let sp = SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(100.0, 0.0), Point::new(100.0, 100.0)], false);
        assert!((fraction_at(&sp, 0, 0.5) - 0.25).abs() < 1e-6);
        assert!((fraction_at(&sp, 1, 0.5) - 0.75).abs() < 1e-6);
        let (p, t) = eval_fraction(&sp, 0.75).unwrap();
        assert!((p - Point::new(100.0, 50.0)).hypot() < 1e-3);
        assert!((t - Vec2::new(0.0, 1.0)).hypot() < 1e-6);
        // Walking +x, the left side is up (y-down space).
        assert_eq!(left_normal(Vec2::new(1.0, 0.0)), Vec2::new(0.0, -1.0));
    }
}
