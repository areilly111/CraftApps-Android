//! Projective (perspective) transforms of the plane: 3×3 homographies.
//!
//! The Perspective Grid maps each grid plane to the page with one, objects in perspective move by
//! conjugating plane-space affine maps with it, and type and symbol instances in perspective keep
//! one that projects their flat art.

use kurbo::{Affine, Point, Rect};

/// A 3×3 projective transform (row-major), acting on `(x, y, 1)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Homography(pub [[f64; 3]; 3]);

impl Homography {
    pub const IDENTITY: Self = Self([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);

    /// From the images of (1,0,0), (0,1,0) and (0,0,1).
    pub fn from_cols(c1: [f64; 3], c2: [f64; 3], c3: [f64; 3]) -> Self {
        Self([[c1[0], c2[0], c3[0]], [c1[1], c2[1], c3[1]], [c1[2], c2[2], c3[2]]])
    }

    /// The affine map `a` as a homography.
    pub fn from_affine(a: Affine) -> Self {
        let [m0, m1, m2, m3, m4, m5] = a.as_coeffs();
        Self([[m0, m2, m4], [m1, m3, m5], [0.0, 0.0, 1.0]])
    }

    /// From nine row-major numbers (as stored in documents); `None` unless all are finite and the
    /// map is invertible.
    pub fn from_array(v: [f64; 9]) -> Option<Self> {
        if v.iter().any(|x| !x.is_finite()) {
            return None;
        }
        let h = Self([[v[0], v[1], v[2]], [v[3], v[4], v[5]], [v[6], v[7], v[8]]]);
        (h.det().abs() > 1e-15).then_some(h)
    }

    /// The nine row-major numbers.
    pub fn to_array(&self) -> [f64; 9] {
        let m = &self.0;
        [m[0][0], m[0][1], m[0][2], m[1][0], m[1][1], m[1][2], m[2][0], m[2][1], m[2][2]]
    }

    /// Map a point; `None` when it lands on or beyond the horizon (w ≤ 0).
    pub fn apply(&self, p: Point) -> Option<Point> {
        let m = &self.0;
        let x = m[0][0] * p.x + m[0][1] * p.y + m[0][2];
        let y = m[1][0] * p.x + m[1][1] * p.y + m[1][2];
        let w = m[2][0] * p.x + m[2][1] * p.y + m[2][2];
        if w <= 1e-9 || !x.is_finite() || !y.is_finite() {
            return None;
        }
        Some(Point::new(x / w, y / w))
    }

    pub fn det(&self) -> f64 {
        let m = &self.0;
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    }

    /// Inverse.
    pub fn inverse(&self) -> Option<Self> {
        let m = &self.0;
        let d = self.det();
        if d.abs() < 1e-15 || !d.is_finite() {
            return None;
        }
        let c = |r0: usize, c0: usize, r1: usize, c1: usize| m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0];
        let adj = [
            [c(1, 1, 2, 2), -c(0, 1, 2, 2), c(0, 1, 1, 2)],
            [-c(1, 0, 2, 2), c(0, 0, 2, 2), -c(0, 0, 1, 2)],
            [c(1, 0, 2, 1), -c(0, 0, 2, 1), c(0, 0, 1, 1)],
        ];
        let mut out = [[0.0; 3]; 3];
        for (r, row) in adj.iter().enumerate() {
            for (k, v) in row.iter().enumerate() {
                out[r][k] = v / d;
            }
        }
        // H⁻¹·(x, y, 1) = (u, v, 1)/w_H, so points in front of the camera keep w > 0.
        Some(Self(out))
    }

    /// `self ∘ other`: `other` first.
    pub fn then_after(&self, other: &Self) -> Self {
        let (a, b) = (&self.0, &other.0);
        let mut out = [[0.0; 3]; 3];
        for (r, row) in out.iter_mut().enumerate() {
            for (k, v) in row.iter_mut().enumerate() {
                *v = a[r][0] * b[0][k] + a[r][1] * b[1][k] + a[r][2] * b[2][k];
            }
        }
        Self(out)
    }

    /// `self` seen through `a`: the map `a ∘ self ∘ a⁻¹` (what `self` becomes when everything it
    /// maps from and to is moved by `a`). `None` when `a` isn't invertible.
    pub fn conjugated(&self, a: Affine) -> Option<Self> {
        if a.determinant().abs() < 1e-15 {
            return None;
        }
        Some(Self::from_affine(a).then_after(self).then_after(&Self::from_affine(a.inverse())))
    }

    /// The affine map that matches `self` to first order at `p` (its derivative there), `None`
    /// beyond the horizon.
    pub fn affine_at(&self, p: Point) -> Option<Affine> {
        let m = &self.0;
        let w = m[2][0] * p.x + m[2][1] * p.y + m[2][2];
        let q = self.apply(p)?;
        // d(x/w) = (dx − (x/w)·dw) / w, and the same for y.
        let j = |r: usize, c: usize| (m[r][c] - [q.x, q.y][r] * m[2][c]) / w;
        let lin = Affine::new([j(0, 0), j(1, 0), j(0, 1), j(1, 1), 0.0, 0.0]);
        Some(Affine::translate(q.to_vec2()) * lin * Affine::translate(-p.to_vec2()))
    }

    /// The bounding box of the image of `r` (lines stay lines: its corners bound it), `None` when
    /// part of it crosses the horizon.
    pub fn map_rect_bbox(&self, r: Rect) -> Option<Rect> {
        let pts = [Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)];
        let mut out: Option<Rect> = None;
        for p in pts {
            let q = self.apply(p)?;
            out = Some(out.map_or(Rect::from_points(q, q), |b| b.union_pt(q)));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h() -> Homography {
        Homography([[1.2, 0.1, 5.0], [-0.2, 0.9, 3.0], [0.001, 0.002, 1.0]])
    }

    #[test]
    fn compose_invert_and_round_trip() {
        let a = h();
        let inv = a.inverse().unwrap();
        let id = a.then_after(&inv);
        for (r, row) in id.0.iter().enumerate() {
            for (k, v) in row.iter().enumerate() {
                assert!((v - if r == k { 1.0 } else { 0.0 }).abs() < 1e-9);
            }
        }
        let p = Point::new(30.0, -12.0);
        assert!(inv.apply(a.apply(p).unwrap()).unwrap().distance(p) < 1e-9);
        assert_eq!(Homography::from_array(a.to_array()), Some(a));
        assert!(Homography::from_array([f64::NAN; 9]).is_none());
        assert!(Homography::from_array([0.0; 9]).is_none());
    }

    #[test]
    fn affine_and_conjugation() {
        let t = Affine::translate((10.0, -4.0)) * Affine::scale(2.0);
        let p = Point::new(3.0, 7.0);
        assert!(Homography::from_affine(t).apply(p).unwrap().distance(t * p) < 1e-12);
        // Conjugating: moving the input and the output by `t` keeps the picture moved by `t`.
        let c = h().conjugated(t).unwrap();
        assert!(c.apply(t * p).unwrap().distance(t * h().apply(p).unwrap()) < 1e-9);
        assert!(h().conjugated(Affine::scale(0.0)).is_none());
    }

    #[test]
    fn affine_at_is_the_derivative() {
        let p = Point::new(12.0, -7.0);
        let a = h().affine_at(p).unwrap();
        assert!(a * p == h().apply(p).unwrap() || (a * p).distance(h().apply(p).unwrap()) < 1e-9);
        let e = 1e-4;
        for d in [kurbo::Vec2::new(e, 0.0), kurbo::Vec2::new(0.0, e)] {
            let exact = h().apply(p + d).unwrap();
            assert!((a * (p + d)).distance(exact) < 1e-7, "{d:?}");
        }
    }

    #[test]
    fn rect_bbox_is_the_corners_box() {
        let b = h().map_rect_bbox(Rect::new(0.0, 0.0, 10.0, 20.0)).unwrap();
        for p in [Point::new(0.0, 0.0), Point::new(10.0, 20.0), Point::new(10.0, 0.0), Point::new(5.0, 10.0)] {
            let q = h().apply(p).unwrap();
            assert!(b.inflate(1e-9, 1e-9).contains(q));
        }
    }
}
