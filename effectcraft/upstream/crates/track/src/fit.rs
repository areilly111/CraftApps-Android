//! Robust motion-model fits between point correspondences: **RANSAC** (Fischler & Bolles,
//! "Random Sample Consensus", 1981) with minimal samples, followed by least-squares refits on the
//! consensus set (Hartley & Zisserman, *Multiple View Geometry*, §4.7–4.8).
//!
//! Models, in increasing freedom: translation, translation + uniform scale, similarity (rotation,
//! uniform scale, translation), affine (adds skew / non-uniform scale) and homography
//! (perspective). Every fit is returned as a [`Homography`] matrix mapping `src` to `dst`.

use serde::{Deserialize, Serialize};

use crate::solve::{Homography, solve_linear};

/// A 2D motion model.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Model {
    Translation,
    TranslationScale,
    #[default]
    Similarity,
    Affine,
    Homography,
}

impl Model {
    /// Points in a minimal sample.
    pub fn min_points(self) -> usize {
        match self {
            Model::Translation => 1,
            Model::TranslationScale | Model::Similarity => 2,
            Model::Affine => 3,
            Model::Homography => 4,
        }
    }
}

fn centroid(p: &[[f64; 2]]) -> [f64; 2] {
    let n = p.len().max(1) as f64;
    [p.iter().map(|q| q[0]).sum::<f64>() / n, p.iter().map(|q| q[1]).sum::<f64>() / n]
}

/// Least-squares fit of `model` taking `src` to `dst` (`None` when degenerate).
pub fn least_squares(model: Model, src: &[[f64; 2]], dst: &[[f64; 2]]) -> Option<Homography> {
    let n = src.len().min(dst.len());
    if n < model.min_points() {
        return None;
    }
    let (src, dst) = (&src[..n], &dst[..n]);
    let (cs, cd) = (centroid(src), centroid(dst));
    let h = match model {
        Model::Translation => Homography::translation([cd[0] - cs[0], cd[1] - cs[1]]),
        Model::TranslationScale | Model::Similarity => {
            let (mut ss, mut dot, mut cross) = (0.0, 0.0, 0.0);
            for (p, q) in src.iter().zip(dst) {
                let (a, b) = ([p[0] - cs[0], p[1] - cs[1]], [q[0] - cd[0], q[1] - cd[1]]);
                ss += a[0] * a[0] + a[1] * a[1];
                dot += a[0] * b[0] + a[1] * b[1];
                cross += a[0] * b[1] - a[1] * b[0];
            }
            if ss < 1e-12 {
                return None;
            }
            let (a, b) = if model == Model::Similarity { (dot / ss, cross / ss) } else { (dot / ss, 0.0) };
            if a.hypot(b) < 1e-9 {
                return None;
            }
            // q = R p + t with R = [[a, -b], [b, a]].
            let t = [cd[0] - (a * cs[0] - b * cs[1]), cd[1] - (b * cs[0] + a * cs[1])];
            Homography([[a, -b, t[0]], [b, a, t[1]], [0.0, 0.0, 1.0]])
        }
        Model::Affine => {
            let mut m = [[0.0; 3]; 3];
            let (mut rx, mut ry) = ([0.0; 3], [0.0; 3]);
            for (p, q) in src.iter().zip(dst) {
                let v = [p[0] - cs[0], p[1] - cs[1], 1.0];
                for i in 0..3 {
                    for j in 0..3 {
                        m[i][j] += v[i] * v[j];
                    }
                    rx[i] += v[i] * (q[0] - cd[0]);
                    ry[i] += v[i] * (q[1] - cd[1]);
                }
            }
            let x = solve_linear(m, rx)?;
            let y = solve_linear(m, ry)?;
            // q - cd = A (p - cs) + t'
            let t = [cd[0] + x[2] - x[0] * cs[0] - x[1] * cs[1], cd[1] + y[2] - y[0] * cs[0] - y[1] * cs[1]];
            Homography([[x[0], x[1], t[0]], [y[0], y[1], t[1]], [0.0, 0.0, 1.0]])
        }
        Model::Homography => {
            // Normalised DLT with h33 = 1 (normal equations).
            let norm = |p: &[[f64; 2]], c: [f64; 2]| {
                let d = p.iter().map(|q| (q[0] - c[0]).hypot(q[1] - c[1])).sum::<f64>() / p.len() as f64;
                if d > 1e-12 { std::f64::consts::SQRT_2 / d } else { 1.0 }
            };
            let (ks, kd) = (norm(src, cs), norm(dst, cd));
            let mut a = [[0.0; 8]; 8];
            let mut b = [0.0; 8];
            for (p, q) in src.iter().zip(dst) {
                let (x, y) = ((p[0] - cs[0]) * ks, (p[1] - cs[1]) * ks);
                let (u, v) = ((q[0] - cd[0]) * kd, (q[1] - cd[1]) * kd);
                let rows = [([x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y], u), ([0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y], v)];
                for (r, t) in rows {
                    for i in 0..8 {
                        b[i] += r[i] * t;
                        for j in 0..8 {
                            a[i][j] += r[i] * r[j];
                        }
                    }
                }
            }
            let h = solve_linear(a, b)?;
            let hn = Homography([[h[0], h[1], h[2]], [h[3], h[4], h[5]], [h[6], h[7], 1.0]]);
            let ts = Homography([[ks, 0.0, -ks * cs[0]], [0.0, ks, -ks * cs[1]], [0.0, 0.0, 1.0]]);
            let td_inv = Homography([[1.0 / kd, 0.0, cd[0]], [0.0, 1.0 / kd, cd[1]], [0.0, 0.0, 1.0]]);
            let m = td_inv.then_after(&hn.then_after(&ts));
            if m.0[2][2].abs() < 1e-12 {
                return None;
            }
            m.normalized()
        }
    };
    h.0.iter().flatten().all(|v| v.is_finite()).then_some(h)
}

fn residual2(h: &Homography, p: [f64; 2], q: [f64; 2]) -> f64 {
    let m = h.apply(p);
    (m[0] - q[0]).powi(2) + (m[1] - q[1]).powi(2)
}

/// Deterministic xorshift generator (reproducible fits).
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// A robust fit: the model, which correspondences are inliers, and the RMS inlier error.
#[derive(Clone, Debug)]
pub struct Fit {
    pub h: Homography,
    pub inliers: Vec<bool>,
    pub rms: f64,
}

impl Fit {
    pub fn inlier_count(&self) -> usize {
        self.inliers.iter().filter(|b| **b).count()
    }
}

/// RANSAC fit of `model` with inlier `threshold` (pixels) and at most `iterations` samples.
pub fn ransac(model: Model, src: &[[f64; 2]], dst: &[[f64; 2]], threshold: f64, iterations: usize, seed: u64) -> Option<Fit> {
    let n = src.len().min(dst.len());
    let k = model.min_points();
    if n < k {
        return None;
    }
    let t2 = threshold * threshold;
    let count = |h: &Homography| {
        let mut c = 0;
        let mut e = 0.0;
        for i in 0..n {
            let r = residual2(h, src[i], dst[i]);
            if r < t2 {
                c += 1;
                e += r;
            }
        }
        (c, e)
    };
    let mut best: Option<(usize, f64, Homography)> = None;
    if n == k {
        let h = least_squares(model, src, dst)?;
        let (c, e) = count(&h);
        best = Some((c, e, h));
    } else {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut max_iter = iterations.max(1);
        let mut it = 0;
        let mut idx = vec![0usize; k];
        let (mut ss, mut sd) = (vec![[0.0; 2]; k], vec![[0.0; 2]; k]);
        while it < max_iter {
            it += 1;
            // Distinct random sample.
            let mut j = 0;
            let mut guard = 0;
            while j < k && guard < 100 {
                guard += 1;
                let c = rng.below(n);
                if idx[..j].contains(&c) {
                    continue;
                }
                idx[j] = c;
                j += 1;
            }
            if j < k {
                continue;
            }
            for (m, &i) in idx.iter().enumerate() {
                ss[m] = src[i];
                sd[m] = dst[i];
            }
            let Some(h) = least_squares(model, &ss, &sd) else { continue };
            let (c, e) = count(&h);
            let better = match &best {
                None => true,
                Some((bc, be, _)) => c > *bc || (c == *bc && e < *be),
            };
            if better {
                best = Some((c, e, h));
                // Adaptive stopping (99 % confidence of an outlier-free sample).
                let w = c as f64 / n as f64;
                let p_good = w.powi(k as i32);
                if p_good > 1e-9 && p_good < 1.0 {
                    let need = ((1.0 - 0.99f64).ln() / (1.0 - p_good).ln()).ceil();
                    if need.is_finite() {
                        max_iter = max_iter.min((need as usize).max(it + 1));
                    }
                } else if p_good >= 1.0 {
                    break;
                }
            }
        }
    }
    let (_, _, mut h) = best?;
    // Refine on the consensus set (twice: the set can grow after the first refit).
    let mut inl: Vec<bool> = (0..n).map(|i| residual2(&h, src[i], dst[i]) < t2).collect();
    for _ in 0..2 {
        let (s2, d2): (Vec<[f64; 2]>, Vec<[f64; 2]>) = (0..n).filter(|i| inl[*i]).map(|i| (src[i], dst[i])).unzip();
        if s2.len() < k {
            break;
        }
        if let Some(r) = least_squares(model, &s2, &d2) {
            h = r;
        }
        inl = (0..n).map(|i| residual2(&h, src[i], dst[i]) < t2).collect();
    }
    let c = inl.iter().filter(|b| **b).count();
    let e: f64 = (0..n).filter(|i| inl[*i]).map(|i| residual2(&h, src[i], dst[i])).sum();
    Some(Fit { h, inliers: inl, rms: if c > 0 { (e / c as f64).sqrt() } else { f64::INFINITY } })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pts() -> Vec<[f64; 2]> {
        let mut v = vec![];
        let mut s = 12345u64;
        for _ in 0..80 {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let x = (s >> 33) as f64 / (1u64 << 31) as f64 * 300.0;
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let y = (s >> 33) as f64 / (1u64 << 31) as f64 * 200.0;
            v.push([x, y]);
        }
        v
    }

    #[test]
    fn ransac_rejects_outliers_for_every_model() {
        let truth = [
            (Model::Translation, Homography::translation([5.0, -3.0])),
            (Model::TranslationScale, Homography([[1.1, 0.0, -4.0], [0.0, 1.1, 2.0], [0.0, 0.0, 1.0]])),
            (Model::Similarity, Homography([[0.98, -0.17, 12.0], [0.17, 0.98, -6.0], [0.0, 0.0, 1.0]])),
            (Model::Affine, Homography([[1.05, 0.12, 3.0], [-0.04, 0.93, 1.0], [0.0, 0.0, 1.0]])),
            (Model::Homography, Homography([[1.02, 0.05, 4.0], [-0.03, 0.97, 2.0], [0.0004, -0.0003, 1.0]])),
        ];
        for (m, h) in truth {
            let src = pts();
            let mut dst: Vec<[f64; 2]> = src.iter().map(|p| h.apply(*p)).collect();
            // 30 % gross outliers.
            for (i, d) in dst.iter_mut().enumerate() {
                if i % 10 < 3 {
                    d[0] += 40.0 + i as f64;
                    d[1] -= 25.0;
                }
            }
            let f = ransac(m, &src, &dst, 1.0, 500, 7).unwrap();
            assert_eq!(f.inlier_count(), 56, "{m:?}");
            for p in [[0.0, 0.0], [150.0, 100.0], [300.0, 200.0]] {
                let (a, b) = (f.h.apply(p), h.apply(p));
                assert!((a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6, "{m:?} {a:?} {b:?}");
            }
        }
    }

    #[test]
    fn inverse_and_compose() {
        let h = Homography([[1.02, 0.05, 4.0], [-0.03, 0.97, 2.0], [0.0004, -0.0003, 1.0]]);
        let i = h.inverse().unwrap();
        let p = [123.0, 45.0];
        let q = i.apply(h.apply(p));
        assert!((q[0] - p[0]).abs() < 1e-9 && (q[1] - p[1]).abs() < 1e-9);
        let c = Homography::compose(&h, &i);
        assert!(c.max_diff(&Homography::IDENTITY) < 1e-9);
    }
}
