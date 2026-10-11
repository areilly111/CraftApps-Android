//! Gaussian mixture colour models (full 3 × 3 covariances), as used by GrabCut (C. Rother,
//! V. Kolmogorov and A. Blake, "GrabCut: Interactive Foreground Extraction using Iterated Graph
//! Cuts", SIGGRAPH 2004).
//!
//! Fitting is deterministic: components are initialised by recursive splitting of the cluster
//! with the largest variance along its principal axis (Orchard & Bouman, "Color Quantization of
//! Images", IEEE TSP 1991), then refined by a few hard-assignment EM iterations.

use serde::{Deserialize, Serialize};

/// One Gaussian: weight, mean, inverse covariance and log normaliser.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Gauss {
    pub weight: f64,
    pub mean: [f64; 3],
    pub inv: [[f64; 3]; 3],
    /// −log(weight) + ½·log(det Σ) (the 2π term is dropped: it cancels between models).
    pub log_norm: f64,
}

/// A colour model.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Gmm {
    pub comps: Vec<Gauss>,
}

/// Regularisation added to every covariance's diagonal (colour noise floor).
const REG: f64 = 2e-4;

fn det3(m: &[[f64; 3]; 3]) -> f64 {
    m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
}

fn inv3(m: &[[f64; 3]; 3]) -> Option<[[f64; 3]; 3]> {
    let d = det3(m);
    if d.abs() < 1e-30 {
        return None;
    }
    let mut r = [[0.0; 3]; 3];
    r[0][0] = (m[1][1] * m[2][2] - m[1][2] * m[2][1]) / d;
    r[0][1] = (m[0][2] * m[2][1] - m[0][1] * m[2][2]) / d;
    r[0][2] = (m[0][1] * m[1][2] - m[0][2] * m[1][1]) / d;
    r[1][0] = (m[1][2] * m[2][0] - m[1][0] * m[2][2]) / d;
    r[1][1] = (m[0][0] * m[2][2] - m[0][2] * m[2][0]) / d;
    r[1][2] = (m[0][2] * m[1][0] - m[0][0] * m[1][2]) / d;
    r[2][0] = (m[1][0] * m[2][1] - m[1][1] * m[2][0]) / d;
    r[2][1] = (m[0][1] * m[2][0] - m[0][0] * m[2][1]) / d;
    r[2][2] = (m[0][0] * m[1][1] - m[0][1] * m[1][0]) / d;
    Some(r)
}

/// Sufficient statistics of a cluster.
#[derive(Clone, Copy, Default)]
struct Stats {
    n: f64,
    s: [f64; 3],
    ss: [[f64; 3]; 3],
}

impl Stats {
    fn add(&mut self, c: [f32; 3]) {
        let c = [c[0] as f64, c[1] as f64, c[2] as f64];
        self.n += 1.0;
        for i in 0..3 {
            self.s[i] += c[i];
            for j in 0..3 {
                self.ss[i][j] += c[i] * c[j];
            }
        }
    }
    fn mean(&self) -> [f64; 3] {
        let n = self.n.max(1.0);
        [self.s[0] / n, self.s[1] / n, self.s[2] / n]
    }
    fn cov(&self) -> [[f64; 3]; 3] {
        let n = self.n.max(1.0);
        let m = self.mean();
        let mut c = [[0.0; 3]; 3];
        for i in 0..3 {
            for j in 0..3 {
                c[i][j] = self.ss[i][j] / n - m[i] * m[j];
            }
            c[i][i] += REG;
        }
        c
    }
}

/// Principal eigenvector and eigenvalue of a symmetric 3 × 3 matrix (power iteration).
fn principal(c: &[[f64; 3]; 3]) -> ([f64; 3], f64) {
    let mut v = [0.577, 0.577, 0.577];
    let mut lambda = 0.0;
    for _ in 0..40 {
        let w = [
            c[0][0] * v[0] + c[0][1] * v[1] + c[0][2] * v[2],
            c[1][0] * v[0] + c[1][1] * v[1] + c[1][2] * v[2],
            c[2][0] * v[0] + c[2][1] * v[1] + c[2][2] * v[2],
        ];
        let n = (w[0] * w[0] + w[1] * w[1] + w[2] * w[2]).sqrt();
        if n < 1e-30 {
            break;
        }
        lambda = n;
        v = [w[0] / n, w[1] / n, w[2] / n];
    }
    (v, lambda)
}

impl Gmm {
    /// Fit `k` components to `samples` (at most `max_samples` are used, taken at a fixed stride).
    pub fn fit(samples: &[[f32; 3]], k: usize, max_samples: usize) -> Gmm {
        if samples.is_empty() {
            return Gmm::default();
        }
        let stride = samples.len().div_ceil(max_samples.max(1)).max(1);
        let xs: Vec<[f32; 3]> = samples.iter().step_by(stride).copied().collect();
        // Orchard–Bouman splitting.
        let mut label = vec![0usize; xs.len()];
        let mut ncl = 1;
        while ncl < k {
            let mut st = vec![Stats::default(); ncl];
            for (x, l) in xs.iter().zip(&label) {
                st[*l].add(*x);
            }
            let (mut best, mut best_l, mut axis) = (usize::MAX, 0.0, [0.0; 3]);
            for (c, s) in st.iter().enumerate() {
                if s.n < 2.0 {
                    continue;
                }
                let (v, l) = principal(&s.cov());
                if l > best_l {
                    best = c;
                    best_l = l;
                    axis = v;
                }
            }
            if best == usize::MAX || best_l <= REG * 1.01 {
                break;
            }
            let m = st[best].mean();
            let thr = axis[0] * m[0] + axis[1] * m[1] + axis[2] * m[2];
            for (x, l) in xs.iter().zip(label.iter_mut()) {
                if *l == best && axis[0] * x[0] as f64 + axis[1] * x[1] as f64 + axis[2] * x[2] as f64 > thr {
                    *l = ncl;
                }
            }
            ncl += 1;
        }
        let mut g = Gmm::from_labels(&xs, &label, ncl);
        // Hard EM.
        for _ in 0..3 {
            for (x, l) in xs.iter().zip(label.iter_mut()) {
                *l = g.best(*x);
            }
            let ng = Gmm::from_labels(&xs, &label, g.comps.len());
            if ng.comps.is_empty() {
                break;
            }
            g = ng;
        }
        g
    }

    fn from_labels(xs: &[[f32; 3]], label: &[usize], k: usize) -> Gmm {
        let mut st = vec![Stats::default(); k];
        for (x, l) in xs.iter().zip(label) {
            st[*l].add(*x);
        }
        let total: f64 = st.iter().map(|s| s.n).sum::<f64>().max(1.0);
        let comps = st
            .iter()
            .filter(|s| s.n >= 1.0)
            .filter_map(|s| {
                let c = s.cov();
                let inv = inv3(&c)?;
                let det = det3(&c).max(1e-300);
                let weight = s.n / total;
                Some(Gauss { weight, mean: s.mean(), inv, log_norm: -weight.ln() + 0.5 * det.ln() })
            })
            .collect();
        Gmm { comps }
    }

    #[inline]
    fn comp_cost(g: &Gauss, c: [f32; 3]) -> f64 {
        let d = [c[0] as f64 - g.mean[0], c[1] as f64 - g.mean[1], c[2] as f64 - g.mean[2]];
        let mut q = 0.0;
        for i in 0..3 {
            q += d[i] * (g.inv[i][0] * d[0] + g.inv[i][1] * d[1] + g.inv[i][2] * d[2]);
        }
        g.log_norm + 0.5 * q
    }

    /// The most likely component of colour `c`.
    pub fn best(&self, c: [f32; 3]) -> usize {
        let mut b = 0;
        let mut bv = f64::INFINITY;
        for (i, g) in self.comps.iter().enumerate() {
            let v = Self::comp_cost(g, c);
            if v < bv {
                bv = v;
                b = i;
            }
        }
        b
    }

    /// −log p(c) (up to a constant shared by all models), clamped to a finite range.
    pub fn cost(&self, c: [f32; 3]) -> f64 {
        if self.comps.is_empty() {
            return 10.0;
        }
        // log-sum-exp over components.
        let mut m = f64::INFINITY;
        let mut costs = [0.0f64; 16];
        let n = self.comps.len().min(16);
        for (i, g) in self.comps.iter().take(n).enumerate() {
            costs[i] = Self::comp_cost(g, c);
            m = m.min(costs[i]);
        }
        let s: f64 = costs[..n].iter().map(|v| (m - v).exp()).sum();
        (m - s.ln()).clamp(-20.0, 60.0)
    }

    pub fn is_empty(&self) -> bool {
        self.comps.is_empty()
    }
}
