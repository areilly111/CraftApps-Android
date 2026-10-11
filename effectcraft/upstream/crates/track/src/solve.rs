//! Small geometric solves for applying tracks: a 2-point similarity (Transform track rotation and
//! scale), a 3-point affine (Affine Corner Pin) and a 4-point homography (Perspective Corner Pin,
//! the direct linear transform with `h33 = 1`, Hartley & Zisserman §4.1).

/// Solve `a x = b` (n × n, row-major) by Gaussian elimination with partial pivoting.
pub fn solve_linear<const N: usize>(mut a: [[f64; N]; N], mut b: [f64; N]) -> Option<[f64; N]> {
    for col in 0..N {
        let piv = (col..N).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        if a[piv][col].abs() < 1e-12 {
            return None;
        }
        a.swap(col, piv);
        b.swap(col, piv);
        for r in col + 1..N {
            let f = a[r][col] / a[col][col];
            if f == 0.0 {
                continue;
            }
            for c in col..N {
                a[r][c] -= f * a[col][c];
            }
            b[r] -= f * b[col];
        }
    }
    let mut x = [0.0; N];
    for r in (0..N).rev() {
        let mut s = b[r];
        for c in r + 1..N {
            s -= a[r][c] * x[c];
        }
        x[r] = s / a[r][r];
    }
    x.iter().all(|v| v.is_finite()).then_some(x)
}

/// A 3 × 3 projective transform (row-major), mapping `[x, y, 1]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Homography(pub [[f64; 3]; 3]);

impl Default for Homography {
    fn default() -> Self {
        Homography::IDENTITY
    }
}

impl Homography {
    pub const IDENTITY: Homography = Homography([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);

    pub fn apply(&self, p: [f64; 2]) -> [f64; 2] {
        let m = &self.0;
        let w = m[2][0] * p[0] + m[2][1] * p[1] + m[2][2];
        let w = if w.abs() < 1e-12 { 1e-12 } else { w };
        [(m[0][0] * p[0] + m[0][1] * p[1] + m[0][2]) / w, (m[1][0] * p[0] + m[1][1] * p[1] + m[1][2]) / w]
    }

    /// `self ∘ other` (apply `other` first).
    pub fn then_after(&self, other: &Homography) -> Homography {
        Homography(mul(self.0, other.0)).normalized()
    }

    /// Composition `b ∘ a`: apply `a`, then `b`.
    pub fn compose(a: &Homography, b: &Homography) -> Homography {
        b.then_after(a)
    }

    /// Scale so that `h33 = 1` (when it isn't ≈ 0).
    pub fn normalized(self) -> Homography {
        let s = self.0[2][2];
        if s.abs() < 1e-12 || (s - 1.0).abs() < 1e-15 {
            return self;
        }
        Homography(self.0.map(|r| r.map(|v| v / s)))
    }

    /// The inverse map (`None` when singular).
    pub fn inverse(&self) -> Option<Homography> {
        let m = &self.0;
        let c00 = m[1][1] * m[2][2] - m[1][2] * m[2][1];
        let c01 = m[1][2] * m[2][0] - m[1][0] * m[2][2];
        let c02 = m[1][0] * m[2][1] - m[1][1] * m[2][0];
        let det = m[0][0] * c00 + m[0][1] * c01 + m[0][2] * c02;
        if det.abs() < 1e-18 || !det.is_finite() {
            return None;
        }
        let inv = [
            [c00, m[0][2] * m[2][1] - m[0][1] * m[2][2], m[0][1] * m[1][2] - m[0][2] * m[1][1]],
            [c01, m[0][0] * m[2][2] - m[0][2] * m[2][0], m[0][2] * m[1][0] - m[0][0] * m[1][2]],
            [c02, m[0][1] * m[2][0] - m[0][0] * m[2][1], m[0][0] * m[1][1] - m[0][1] * m[1][0]],
        ];
        Some(Homography(inv.map(|r| r.map(|v| v / det))).normalized())
    }

    /// Translation by `t`.
    pub fn translation(t: [f64; 2]) -> Homography {
        Homography([[1.0, 0.0, t[0]], [0.0, 1.0, t[1]], [0.0, 0.0, 1.0]])
    }

    /// Uniform scale `k` about `c`.
    pub fn scale_about(k: f64, c: [f64; 2]) -> Homography {
        Homography([[k, 0.0, c[0] * (1.0 - k)], [0.0, k, c[1] * (1.0 - k)], [0.0, 0.0, 1.0]])
    }

    /// `(1 - t) · self + t · other` element-wise (both normalised to `h33 = 1`): blends between
    /// nearby transforms.
    pub fn lerp(&self, other: &Homography, t: f64) -> Homography {
        let (a, b) = (self.normalized().0, other.normalized().0);
        let mut o = [[0.0; 3]; 3];
        for i in 0..3 {
            for j in 0..3 {
                o[i][j] = a[i][j] + (b[i][j] - a[i][j]) * t;
            }
        }
        Homography(o)
    }

    /// Map a Bezier tangent (relative to vertex `v`) through the transform: the tangent's end
    /// point is mapped and made relative to the mapped vertex.
    pub fn apply_tangent(&self, v: [f64; 2], tangent: [f64; 2]) -> [f64; 2] {
        if tangent == [0.0, 0.0] {
            return tangent;
        }
        let a = self.apply(v);
        let b = self.apply([v[0] + tangent[0], v[1] + tangent[1]]);
        [b[0] - a[0], b[1] - a[1]]
    }

    /// Largest absolute difference of the matrices (normalised).
    pub fn max_diff(&self, other: &Homography) -> f64 {
        let (a, b) = (self.normalized().0, other.normalized().0);
        let mut d: f64 = 0.0;
        for i in 0..3 {
            for j in 0..3 {
                d = d.max((a[i][j] - b[i][j]).abs());
            }
        }
        d
    }

    /// The homography taking `src[i]` to `dst[i]` (four points, no three collinear).
    pub fn from_points(src: [[f64; 2]; 4], dst: [[f64; 2]; 4]) -> Option<Homography> {
        // Normalise both sets (centroid at 0, mean distance √2) for conditioning.
        let (ns, ts) = normalise(&src);
        let (nd, td) = normalise(&dst);
        let mut a = [[0.0; 8]; 8];
        let mut b = [0.0; 8];
        for i in 0..4 {
            let ([x, y], [u, v]) = (ns[i], nd[i]);
            a[2 * i] = [x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y];
            b[2 * i] = u;
            a[2 * i + 1] = [0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y];
            b[2 * i + 1] = v;
        }
        let h = solve_linear(a, b)?;
        let hn = [[h[0], h[1], h[2]], [h[3], h[4], h[5]], [h[6], h[7], 1.0]];
        // H = Td⁻¹ · Hn · Ts
        let m = mul(inv_similarity(td), mul(hn, ts));
        let s = m[2][2];
        if s.abs() < 1e-12 {
            return None;
        }
        Some(Homography(m.map(|r| r.map(|v| v / s))))
    }

    /// The affine map taking three points to three points.
    pub fn affine_from_points(src: [[f64; 2]; 3], dst: [[f64; 2]; 3]) -> Option<Homography> {
        let mut a = [[0.0; 3]; 3];
        for i in 0..3 {
            a[i] = [src[i][0], src[i][1], 1.0];
        }
        let x = solve_linear(a, [dst[0][0], dst[1][0], dst[2][0]])?;
        let y = solve_linear(a, [dst[0][1], dst[1][1], dst[2][1]])?;
        Some(Homography([x, y, [0.0, 0.0, 1.0]]))
    }

    /// The similarity (rotation, uniform scale, translation) taking two points to two points.
    pub fn similarity_from_points(src: [[f64; 2]; 2], dst: [[f64; 2]; 2]) -> Option<Homography> {
        let (ds, dd) = (sub(src[1], src[0]), sub(dst[1], dst[0]));
        let n = ds[0] * ds[0] + ds[1] * ds[1];
        if n < 1e-12 {
            return None;
        }
        // dd = [[a, -b], [b, a]] ds
        let a = (dd[0] * ds[0] + dd[1] * ds[1]) / n;
        let b = (dd[1] * ds[0] - dd[0] * ds[1]) / n;
        let tx = dst[0][0] - (a * src[0][0] - b * src[0][1]);
        let ty = dst[0][1] - (b * src[0][0] + a * src[0][1]);
        Some(Homography([[a, -b, tx], [b, a, ty], [0.0, 0.0, 1.0]]))
    }
}

/// Angle (degrees, y down = clockwise positive like After Effects' Rotation) and length of `b - a`.
pub fn angle_and_length(a: [f64; 2], b: [f64; 2]) -> (f64, f64) {
    let d = sub(b, a);
    (d[1].atan2(d[0]).to_degrees(), (d[0] * d[0] + d[1] * d[1]).sqrt())
}

/// `angle` moved by multiples of 360° to be closest to `prev` (continuous rotation curves).
pub fn unwrap_degrees(prev: f64, angle: f64) -> f64 {
    angle + ((prev - angle) / 360.0).round() * 360.0
}

fn sub(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

pub(crate) fn mul(a: [[f64; 3]; 3], b: [[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut o = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            o[i][j] = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    o
}

fn normalise(p: &[[f64; 2]; 4]) -> ([[f64; 2]; 4], [[f64; 3]; 3]) {
    let c = [p.iter().map(|q| q[0]).sum::<f64>() / 4.0, p.iter().map(|q| q[1]).sum::<f64>() / 4.0];
    let d = p.iter().map(|q| ((q[0] - c[0]).powi(2) + (q[1] - c[1]).powi(2)).sqrt()).sum::<f64>() / 4.0;
    let s = if d > 1e-12 { std::f64::consts::SQRT_2 / d } else { 1.0 };
    let t = [[s, 0.0, -s * c[0]], [0.0, s, -s * c[1]], [0.0, 0.0, 1.0]];
    (p.map(|q| [s * (q[0] - c[0]), s * (q[1] - c[1])]), t)
}

fn inv_similarity(t: [[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let s = t[0][0];
    [[1.0 / s, 0.0, -t[0][2] / s], [0.0, 1.0 / s, -t[1][2] / s], [0.0, 0.0, 1.0]]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f64; 2], b: [f64; 2], tol: f64) -> bool {
        (a[0] - b[0]).abs() < tol && (a[1] - b[1]).abs() < tol
    }

    #[test]
    fn homography_recovers_known_projection() {
        let h = Homography([[0.9, 0.12, 30.0], [-0.08, 1.1, -12.0], [0.0004, -0.0002, 1.0]]);
        let src = [[100.0, 100.0], [700.0, 120.0], [680.0, 500.0], [90.0, 520.0]];
        let dst = src.map(|p| h.apply(p));
        let got = Homography::from_points(src, dst).unwrap();
        for i in 0..3 {
            for j in 0..3 {
                assert!((got.0[i][j] - h.0[i][j]).abs() < 1e-6 * (1.0 + h.0[i][j].abs()), "{:?}", got.0);
            }
        }
        for p in [[0.0, 0.0], [400.0, 300.0], [1000.0, 50.0]] {
            assert!(close(got.apply(p), h.apply(p), 1e-6));
        }
        // Degenerate (collinear) input has no solution.
        assert!(Homography::from_points([[0.0, 0.0], [1.0, 1.0], [2.0, 2.0], [3.0, 3.0]], dst).is_none());
    }

    #[test]
    fn affine_and_similarity() {
        let a = Homography::affine_from_points([[0.0, 0.0], [10.0, 0.0], [0.0, 10.0]], [[5.0, 5.0], [15.0, 7.0], [3.0, 15.0]]).unwrap();
        assert!(close(a.apply([10.0, 10.0]), [13.0, 17.0], 1e-9));
        let s = Homography::similarity_from_points([[0.0, 0.0], [10.0, 0.0]], [[1.0, 1.0], [1.0, 21.0]]).unwrap();
        assert!(close(s.apply([0.0, 10.0]), [-19.0, 1.0], 1e-9));
        let (ang, len) = angle_and_length([1.0, 1.0], [1.0, 21.0]);
        assert!((ang - 90.0).abs() < 1e-9 && (len - 20.0).abs() < 1e-9);
        assert!((unwrap_degrees(350.0, -5.0) - 355.0).abs() < 1e-9);
    }
}
