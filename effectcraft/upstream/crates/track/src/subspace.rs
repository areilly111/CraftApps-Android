//! Warp Stabilizer ▸ Method ▸ **Subspace Warp**: content-preserving warps fitted to smoothed
//! feature trajectories, clean-room from the published method:
//!
//! - Liu, Gleicher, Wang, Jin & Agarwala, "Subspace Video Stabilization", ACM TOG 30(1), 2011:
//!   the feature trajectories of a short window form a matrix `M` (two rows per trajectory, one
//!   column per frame) that is close to low rank (rank 9 for a moving camera over a static
//!   scene); its factorisation `M ≈ W C` gives *eigen-trajectories* `C`, which are smoothed (here
//!   with a Gaussian) instead of the trajectories themselves. Every trajectory's smoothed
//!   position is `W_i Ĉ_t`; trajectories that do not span the whole window get their
//!   coefficients `W_i` by least squares over the frames they cover (the paper's handling of
//!   incomplete trajectories in a moving factorisation). Smoothing in the subspace keeps the
//!   smoothed trajectories consistent with one another, so the warp does not tear the scene.
//! - Liu, Gleicher, Jin & Agarwala, "Content-Preserving Warps for 3D Video Stabilization", ACM
//!   TOG 28(3), 2009: each frame is warped by a grid mesh fitted to the feature correspondences
//!   (original → smoothed position) in least squares, with a **similarity term** asking every
//!   grid triangle to keep its shape (`V1 = V2 + u (V3 − V2) + v R90 (V3 − V2)` with `u, v` from
//!   the undeformed grid) so the warp bends where the scene's depth demands it but stays locally
//!   rigid elsewhere.
//!
//! The mesh is stored *inversely*: a regular grid over the stabilised frame whose vertices hold
//! the source position they show ([`Mesh`]), so rendering is a bilinear lookup per output pixel.
//! The data term is the same linear constraint read the other way (the smoothed position's grid
//! cell, bilinearly, must land on the original position). A weak prior ties every vertex to the
//! frame's perspective correction, which fills feature-less regions and keeps the system well
//! posed.

use rayon::prelude::*;

use crate::camtrack::Track2D;
use crate::camtrack::linalg::{cholesky_solve, sym_eigen};
use crate::fit::{Model, least_squares};
use crate::solve::Homography;

/// A warp mesh: a regular `cols × rows` grid over the stabilised frame (`size`), each vertex
/// holding the source-frame position it shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Mesh {
    pub cols: usize,
    pub rows: usize,
    pub size: [f64; 2],
    /// `(rows + 1) × (cols + 1)` source positions, row-major.
    pub verts: Vec<[f64; 2]>,
}

impl Mesh {
    /// The identity mesh.
    pub fn identity(cols: usize, rows: usize, size: [f64; 2]) -> Mesh {
        let mut m = Mesh { cols: cols.max(1), rows: rows.max(1), size, verts: vec![] };
        m.verts = (0..=m.rows).flat_map(|j| (0..=m.cols).map(move |i| (i, j))).map(|(i, j)| m.grid(i, j)).collect();
        m
    }

    /// The mesh of a homography (`h`: source → stabilised).
    pub fn from_homography(cols: usize, rows: usize, size: [f64; 2], h: &Homography) -> Mesh {
        let inv = h.inverse().unwrap_or(Homography::IDENTITY);
        let mut m = Mesh::identity(cols, rows, size);
        for v in m.verts.iter_mut() {
            *v = inv.apply(*v);
        }
        m
    }

    fn cell(&self) -> [f64; 2] {
        [self.size[0] / self.cols as f64, self.size[1] / self.rows as f64]
    }

    /// The undeformed position of vertex `(i, j)` (column, row).
    pub fn grid(&self, i: usize, j: usize) -> [f64; 2] {
        let c = self.cell();
        [i as f64 * c[0], j as f64 * c[1]]
    }

    fn idx(&self, i: usize, j: usize) -> usize {
        j * (self.cols + 1) + i
    }

    /// Cell and bilinear coordinates of a stabilised point (the outer cells extend beyond the
    /// frame, so points outside it extrapolate).
    fn locate(&self, p: [f64; 2]) -> (usize, usize, f64, f64) {
        let c = self.cell();
        let fx = p[0] / c[0];
        let fy = p[1] / c[1];
        let i = (fx.floor().max(0.0) as usize).min(self.cols - 1);
        let j = (fy.floor().max(0.0) as usize).min(self.rows - 1);
        (i, j, fx - i as f64, fy - j as f64)
    }

    /// The source position shown at stabilised point `p`.
    pub fn sample(&self, p: [f64; 2]) -> [f64; 2] {
        let (i, j, u, v) = self.locate(p);
        let a = self.verts[self.idx(i, j)];
        let b = self.verts[self.idx(i + 1, j)];
        let c = self.verts[self.idx(i, j + 1)];
        let d = self.verts[self.idx(i + 1, j + 1)];
        let w = [(1.0 - u) * (1.0 - v), u * (1.0 - v), (1.0 - u) * v, u * v];
        [0, 1].map(|k| w[0] * a[k] + w[1] * b[k] + w[2] * c[k] + w[3] * d[k])
    }

    /// Where source point `q` lands in the stabilised frame (Newton iterations on
    /// [`Mesh::sample`]); `None` when it does not converge (a folded mesh).
    pub fn forward(&self, q: [f64; 2]) -> Option<[f64; 2]> {
        // Start from the mesh's own offset at q.
        let s = self.sample(q);
        let mut p = [2.0 * q[0] - s[0], 2.0 * q[1] - s[1]];
        for _ in 0..30 {
            let f = self.sample(p);
            let e = [f[0] - q[0], f[1] - q[1]];
            if e[0].hypot(e[1]) < 1e-6 {
                return Some(p);
            }
            let h = 0.5;
            let fx = self.sample([p[0] + h, p[1]]);
            let fy = self.sample([p[0], p[1] + h]);
            let j = [[(fx[0] - f[0]) / h, (fy[0] - f[0]) / h], [(fx[1] - f[1]) / h, (fy[1] - f[1]) / h]];
            let det = j[0][0] * j[1][1] - j[0][1] * j[1][0];
            if det.abs() < 1e-12 {
                return None;
            }
            p[0] -= (j[1][1] * e[0] - j[0][1] * e[1]) / det;
            p[1] -= (-j[1][0] * e[0] + j[0][0] * e[1]) / det;
        }
        let f = self.sample(p);
        ((f[0] - q[0]).hypot(f[1] - q[1]) < 1e-3).then_some(p)
    }

    /// `(1 − t) · identity + t · self` per vertex.
    pub fn lerp_identity(&self, t: f64) -> Mesh {
        let mut m = self.clone();
        for j in 0..=self.rows {
            for i in 0..=self.cols {
                let g = self.grid(i, j);
                let k = self.idx(i, j);
                m.verts[k] = [g[0] + (self.verts[k][0] - g[0]) * t, g[1] + (self.verts[k][1] - g[1]) * t];
            }
        }
        m
    }

    /// Largest `s ∈ [0, 1]` such that the centred `s·W × s·H` rectangle of the stabilised frame
    /// shows only source pixels (its outline, sampled densely, maps inside the source frame).
    pub fn valid_fraction(&self) -> f64 {
        let [w, h] = self.size;
        let c = [w / 2.0, h / 2.0];
        let tol = 1e-6;
        let inside = |q: [f64; 2]| q[0] >= -tol && q[1] >= -tol && q[0] <= w + tol && q[1] <= h + tol;
        let fits = |s: f64| {
            let (hw, hh) = (s * w / 2.0, s * h / 2.0);
            let n = 48;
            (0..=n).all(|k| {
                let t = k as f64 / n as f64;
                let x = c[0] - hw + 2.0 * hw * t;
                let y = c[1] - hh + 2.0 * hh * t;
                inside(self.sample([x, c[1] - hh]))
                    && inside(self.sample([x, c[1] + hh]))
                    && inside(self.sample([c[0] - hw, y]))
                    && inside(self.sample([c[0] + hw, y]))
            })
        };
        if fits(1.0) {
            return 1.0;
        }
        if !fits(0.0) {
            return 0.0;
        }
        let (mut lo, mut hi) = (0.0, 1.0);
        for _ in 0..24 {
            let mid = 0.5 * (lo + hi);
            if fits(mid) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        lo
    }

    /// The homography (source → stabilised) closest to the mesh in least squares.
    pub fn homography(&self) -> Homography {
        let mut src = vec![];
        let mut dst = vec![];
        for j in 0..=self.rows {
            for i in 0..=self.cols {
                src.push(self.verts[self.idx(i, j)]);
                dst.push(self.grid(i, j));
            }
        }
        least_squares(Model::Homography, &src, &dst).unwrap_or(Homography::IDENTITY)
    }
}

/// Mesh resolution and rigidity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshOpts {
    pub cols: usize,
    pub rows: usize,
    /// Weight of the similarity (shape-preserving) term relative to one feature.
    pub alpha: f64,
    /// Weight of the prior towards the frame's perspective correction, per vertex.
    pub prior: f64,
}

impl Default for MeshOpts {
    fn default() -> Self {
        MeshOpts { cols: 12, rows: 8, alpha: 1.0, prior: 0.02 }
    }
}

/// Normal equations `AᵀA x = Aᵀb`, accumulated row by row.
struct Normal {
    n: usize,
    a: Vec<f64>,
    b: Vec<f64>,
}

impl Normal {
    fn row(&mut self, coefs: &[(usize, f64)], rhs: f64, w: f64) {
        for (ia, ca) in coefs {
            self.b[*ia] += w * ca * rhs;
            for (ib, cb) in coefs {
                self.a[ia * self.n + ib] += w * ca * cb;
            }
        }
    }
}

/// Fit the content-preserving warp of one frame: `pairs` are (source position, wanted
/// stabilised position) of its features; `base` is the frame's perspective correction (source →
/// stabilised), the prior.
pub fn solve_mesh(size: [f64; 2], o: &MeshOpts, pairs: &[([f64; 2], [f64; 2])], base: &Homography) -> Mesh {
    let mut m = Mesh::from_homography(o.cols, o.rows, size, base);
    let nv = (m.cols + 1) * (m.rows + 1);
    let n = 2 * nv;
    let mut ne = Normal { n, a: vec![0.0; n * n], b: vec![0.0; n] };
    // Prior.
    for k in 0..nv {
        let v = m.verts[k];
        ne.row(&[(2 * k, 1.0)], v[0], o.prior);
        ne.row(&[(2 * k + 1, 1.0)], v[1], o.prior);
    }
    // Data: the bilinear combination of the wanted position's cell lands on the source position.
    let mut used = 0;
    for (src, dst) in pairs {
        if !(dst[0] >= 0.0 && dst[1] >= 0.0 && dst[0] <= size[0] && dst[1] <= size[1] && src[0].is_finite() && src[1].is_finite()) {
            continue;
        }
        let (i, j, u, v) = m.locate(*dst);
        let ks = [m.idx(i, j), m.idx(i + 1, j), m.idx(i, j + 1), m.idx(i + 1, j + 1)];
        let w = [(1.0 - u) * (1.0 - v), u * (1.0 - v), (1.0 - u) * v, u * v];
        for d in 0..2 {
            let coefs: Vec<(usize, f64)> = ks.iter().zip(w).map(|(k, w)| (2 * k + d, w)).collect();
            ne.row(&coefs, src[d], 1.0);
        }
        used += 1;
    }
    if used == 0 {
        return m;
    }
    // Similarity: four triangles per cell, each corner against its two cell neighbours.
    let a2 = o.alpha * o.alpha;
    for j in 0..m.rows {
        for i in 0..m.cols {
            let (a, b, c, d) = ((i, j), (i + 1, j), (i + 1, j + 1), (i, j + 1));
            for (v1, v2, v3) in [(a, b, d), (b, c, a), (c, d, b), (d, a, c)] {
                let (g1, g2, g3) = (m.grid(v1.0, v1.1), m.grid(v2.0, v2.1), m.grid(v3.0, v3.1));
                let dv = [g3[0] - g2[0], g3[1] - g2[1]];
                let e = [g1[0] - g2[0], g1[1] - g2[1]];
                let l2 = dv[0] * dv[0] + dv[1] * dv[1];
                // R90(d) = (−d.y, d.x).
                let u = (e[0] * dv[0] + e[1] * dv[1]) / l2;
                let v = (e[0] * -dv[1] + e[1] * dv[0]) / l2;
                let (k1, k2, k3) = (m.idx(v1.0, v1.1), m.idx(v2.0, v2.1), m.idx(v3.0, v3.1));
                // x: V1x − V2x − u (V3x − V2x) + v (V3y − V2y) = 0
                ne.row(&[(2 * k1, 1.0), (2 * k2, u - 1.0), (2 * k3, -u), (2 * k3 + 1, v), (2 * k2 + 1, -v)], 0.0, a2);
                // y: V1y − V2y − u (V3y − V2y) − v (V3x − V2x) = 0
                ne.row(&[(2 * k1 + 1, 1.0), (2 * k2 + 1, u - 1.0), (2 * k3 + 1, -u), (2 * k3, -v), (2 * k2, v)], 0.0, a2);
            }
        }
    }
    let mut x = ne.b;
    if cholesky_solve(&mut ne.a, n, &mut x) {
        for k in 0..nv {
            m.verts[k] = [x[2 * k], x[2 * k + 1]];
        }
    }
    m
}

/// Gaussian weights of the frames `lo..=hi` around `t`.
fn gauss(lo: usize, hi: usize, t: usize, sigma: f64) -> Vec<f64> {
    let g: Vec<f64> = (lo..=hi).map(|j| (-((j as f64 - t as f64).powi(2)) / (2.0 * sigma * sigma)).exp()).collect();
    let s: f64 = g.iter().sum();
    g.into_iter().map(|v| v / s.max(1e-300)).collect()
}

/// Subspace-smoothed feature positions: for every frame `t < frames`, the (original, smoothed)
/// positions of the trajectories seen on it. `sigma`: Gaussian smoothing in frames; `rank`: the
/// subspace dimension (9 in the paper). Frames without enough trajectories get no pairs.
pub fn smooth_trajectories(tracks: &[Track2D], frames: usize, sigma: f64, rank: usize) -> Vec<Vec<([f64; 2], [f64; 2])>> {
    let max_r = ((3.0 * sigma).ceil() as usize).clamp(1, 30);
    (0..frames)
        .into_par_iter()
        .map(|t| {
            let t32 = t as u32;
            let mut r = max_r;
            // The window around t: shrink it until enough trajectories span it.
            let (lo, hi, complete) = loop {
                let lo = t.saturating_sub(r);
                let hi = (t + r).min(frames.saturating_sub(1));
                let complete: Vec<&Track2D> = tracks.iter().filter(|tr| tr.start as usize <= lo && tr.end() as usize > hi).collect();
                if complete.len() >= 8 || r <= 2 {
                    break (lo, hi, complete);
                }
                r = (r * 2 / 3).max(2);
            };
            if complete.len() < 4 {
                return vec![];
            }
            let m = hi - lo + 1;
            let rows = 2 * complete.len();
            let k = rank.min(m).min(rows).max(1);
            // M: rows × m (row-major).
            let mut mat = vec![0.0; rows * m];
            for (i, tr) in complete.iter().enumerate() {
                for c in 0..m {
                    let p = tr.at((lo + c) as u32).unwrap_or([0.0; 2]);
                    mat[(2 * i) * m + c] = p[0];
                    mat[(2 * i + 1) * m + c] = p[1];
                }
            }
            // Gram MᵀM and its top eigenvectors: the eigen-trajectories (orthonormal rows of C).
            let mut gram = vec![0.0; m * m];
            for a in 0..m {
                for b in a..m {
                    let s: f64 = (0..rows).map(|r| mat[r * m + a] * mat[r * m + b]).sum();
                    gram[a * m + b] = s;
                    gram[b * m + a] = s;
                }
            }
            let (_, vecs) = sym_eigen(&gram, m);
            // C[q][c] = vecs[c][m − 1 − q] (largest first).
            let cmat: Vec<Vec<f64>> = (0..k).map(|q| (0..m).map(|c| vecs[c * m + (m - 1 - q)]).collect()).collect();
            let g = gauss(lo, hi, t, sigma.max(1e-6));
            let chat: Vec<f64> = cmat.iter().map(|row| row.iter().zip(&g).map(|(a, b)| a * b).sum()).collect();
            let tc = t - lo;
            let mut out = Vec::new();
            // Complete trajectories: W = M Cᵀ.
            for (i, _) in complete.iter().enumerate() {
                let src = [mat[(2 * i) * m + tc], mat[(2 * i + 1) * m + tc]];
                let mut dst = [0.0; 2];
                for (d, slot) in dst.iter_mut().enumerate() {
                    let row = &mat[(2 * i + d) * m..(2 * i + d + 1) * m];
                    for q in 0..k {
                        let w: f64 = row.iter().zip(&cmat[q]).map(|(a, b)| a * b).sum();
                        *slot += w * chat[q];
                    }
                }
                out.push((src, dst));
            }
            // Incomplete trajectories seen on t: coefficients by least squares over the frames
            // they cover in the window.
            let need = (k + 3).max(m / 2);
            for tr in tracks.iter().filter(|tr| tr.at(t32).is_some() && !(tr.start as usize <= lo && tr.end() as usize > hi)) {
                let cols: Vec<usize> = (0..m).filter(|c| tr.at((lo + c) as u32).is_some()).collect();
                if cols.len() < need {
                    continue;
                }
                let mut ata = vec![0.0; k * k];
                let mut atb = [vec![0.0; k], vec![0.0; k]];
                for &c in &cols {
                    let Some(p) = tr.at((lo + c) as u32) else { continue };
                    for a in 0..k {
                        for d in 0..2 {
                            atb[d][a] += cmat[a][c] * p[d];
                        }
                        for b in 0..k {
                            ata[a * k + b] += cmat[a][c] * cmat[b][c];
                        }
                    }
                }
                for a in 0..k {
                    ata[a * k + a] += 1e-9;
                }
                let mut dst = [0.0; 2];
                let mut ok = true;
                for d in 0..2 {
                    let mut x = atb[d].clone();
                    let mut aa = ata.clone();
                    if !cholesky_solve(&mut aa, k, &mut x) {
                        ok = false;
                        break;
                    }
                    dst[d] = x.iter().zip(&chat).map(|(a, b)| a * b).sum();
                }
                if ok && let Some(p) = tr.at(t32) {
                    out.push((p, dst));
                }
            }
            out
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mesh_identity_sample_and_forward() {
        let m = Mesh::identity(4, 3, [100.0, 60.0]);
        assert_eq!(m.sample([37.0, 21.0]), [37.0, 21.0]);
        assert_eq!(m.valid_fraction(), 1.0);
        let h = Homography([[1.0, 0.02, 5.0], [-0.01, 1.0, -3.0], [0.0, 0.0, 1.0]]);
        let mh = Mesh::from_homography(4, 3, [100.0, 60.0], &h);
        // Affine maps are exact on a bilinear mesh.
        let q = [40.0, 30.0];
        let p = mh.forward(q).unwrap();
        let e = h.apply(q);
        assert!((p[0] - e[0]).abs() < 1e-6 && (p[1] - e[1]).abs() < 1e-6, "{p:?} {e:?}");
        assert!(mh.homography().max_diff(&h) < 1e-6);
        assert!(mh.valid_fraction() < 1.0);
    }

    #[test]
    fn mesh_fits_a_translation_and_bends_for_local_motion() {
        let size = [200.0, 120.0];
        let mut pairs = vec![];
        for y in 0..12 {
            for x in 0..20 {
                let p = [5.0 + x as f64 * 10.0, 5.0 + y as f64 * 10.0];
                // Everything moves by (3, −2); a blob near the centre by (8, −2).
                let bump = (-((p[0] - 100.0).powi(2) + (p[1] - 60.0).powi(2)) / (2.0 * 20.0f64.powi(2))).exp();
                pairs.push((p, [p[0] + 3.0 + 5.0 * bump, p[1] - 2.0]));
            }
        }
        let base = Homography::translation([3.0, -2.0]);
        let m = solve_mesh(size, &MeshOpts { cols: 10, rows: 6, alpha: 0.5, prior: 0.01 }, &pairs, &base);
        let mut err_mesh = 0.0;
        let mut err_h = 0.0;
        for (s, d) in &pairs {
            let q = m.sample(*d);
            err_mesh += (q[0] - s[0]).hypot(q[1] - s[1]);
            let qh = base.inverse().unwrap().apply(*d);
            err_h += (qh[0] - s[0]).hypot(qh[1] - s[1]);
        }
        assert!(err_mesh < 0.35 * err_h, "{err_mesh} vs {err_h}");
    }
}
