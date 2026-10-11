//! As-rigid-as-possible mesh deformation, after Igarashi, Moscovich & Hughes,
//! "As-Rigid-As-Possible Shape Manipulation" (SIGGRAPH 2005).
//!
//! Two linear least-squares steps per frame:
//!
//! 1. **Similarity step.** Each triangle edge `(i, j)` expresses the third vertex `k` in the
//!    edge's local frame, `v_k = v_i + x·(v_j − v_i) + y·R90(v_j − v_i)`. Minimising the error of
//!    that relation over all three edges of every triangle, with the pinned vertices fixed,
//!    gives a deformation that is locally a similarity (rotation + uniform scale).
//! 2. **Scale-adjustment step.** Each triangle of the step-1 result is fitted with a rigid copy
//!    of its rest shape (2D Procrustes: best rotation about the centroid). The final positions
//!    minimise the difference between every triangle's edge vectors and its fitted edge vectors.
//!
//! Optional extra iterations repeat step 2 on its own result (rotation refinement). Triangle
//! weights stiffen regions (Starch); edge-vector targets implement Bend/Advanced pin rotation
//! and scale. Both systems are symmetric positive definite (a tiny regulariser towards the
//! initial guess keeps under-constrained meshes well-posed) and are solved with Jacobi-
//! preconditioned conjugate gradients on a sparse matrix.

use std::collections::BTreeMap;

/// A soft target for the vector `v_j − v_i`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgeTarget {
    pub i: usize,
    pub j: usize,
    pub vec: [f64; 2],
    pub weight: f64,
}

/// Constraints for one solve.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Handles {
    /// Hard position constraints (vertex, position).
    pub fixed: Vec<(usize, [f64; 2])>,
    pub edges: Vec<EdgeTarget>,
}

/// Sparse symmetric matrix in row maps (assembly), then CSR for the solve.
struct Sparse {
    rows: Vec<BTreeMap<usize, f64>>,
}

impl Sparse {
    fn new(n: usize) -> Sparse {
        Sparse { rows: vec![BTreeMap::new(); n] }
    }
    fn add(&mut self, i: usize, j: usize, v: f64) {
        *self.rows[i].entry(j).or_insert(0.0) += v;
    }
}

struct Csr {
    ptr: Vec<usize>,
    col: Vec<usize>,
    val: Vec<f64>,
    diag: Vec<f64>,
}

impl Csr {
    fn mul(&self, x: &[f64], y: &mut [f64]) {
        for (r, yr) in y.iter_mut().enumerate() {
            let mut s = 0.0;
            for k in self.ptr[r]..self.ptr[r + 1] {
                s += self.val[k] * x[self.col[k]];
            }
            *yr = s;
        }
    }
}

/// Solve `A x = b` restricted to the free unknowns (`free[i]`), with the fixed unknowns at
/// their values in `x`. Jacobi-preconditioned conjugate gradients, starting from `x`.
fn solve(a: &Sparse, b: &[f64], x: &mut [f64], free: &[bool]) {
    let n = b.len();
    let idx: Vec<usize> = (0..n).filter(|i| free[*i]).collect();
    if idx.is_empty() {
        return;
    }
    let mut pos = vec![usize::MAX; n];
    for (k, i) in idx.iter().enumerate() {
        pos[*i] = k;
    }
    let m = idx.len();
    // Reduced system: A_ff x_f = b_f − A_fc x_c.
    let mut ptr = vec![0];
    let mut col = vec![];
    let mut val = vec![];
    let mut diag = vec![1.0; m];
    let mut rhs = vec![0.0; m];
    for (k, &i) in idx.iter().enumerate() {
        let mut r = b[i];
        for (&j, &v) in &a.rows[i] {
            if free[j] {
                col.push(pos[j]);
                val.push(v);
                if j == i {
                    diag[k] = if v.abs() > 1e-300 { v } else { 1.0 };
                }
            } else {
                r -= v * x[j];
            }
        }
        rhs[k] = r;
        ptr.push(col.len());
    }
    let csr = Csr { ptr, col, val, diag };
    let mut xf: Vec<f64> = idx.iter().map(|i| x[*i]).collect();
    let mut ax = vec![0.0; m];
    csr.mul(&xf, &mut ax);
    let mut r: Vec<f64> = rhs.iter().zip(&ax).map(|(b, a)| b - a).collect();
    let bnorm = rhs.iter().map(|v| v * v).sum::<f64>().sqrt().max(1e-30);
    let mut z: Vec<f64> = r.iter().zip(&csr.diag).map(|(r, d)| r / d).collect();
    let mut p = z.clone();
    let mut rz: f64 = r.iter().zip(&z).map(|(a, b)| a * b).sum();
    let mut ap = vec![0.0; m];
    for _ in 0..(4 * m + 50) {
        if r.iter().map(|v| v * v).sum::<f64>().sqrt() <= 1e-13 * bnorm {
            break;
        }
        csr.mul(&p, &mut ap);
        let pap: f64 = p.iter().zip(&ap).map(|(a, b)| a * b).sum();
        if pap.abs() < 1e-300 {
            break;
        }
        let alpha = rz / pap;
        for k in 0..m {
            xf[k] += alpha * p[k];
            r[k] -= alpha * ap[k];
        }
        for k in 0..m {
            z[k] = r[k] / csr.diag[k];
        }
        let rz2: f64 = r.iter().zip(&z).map(|(a, b)| a * b).sum();
        let beta = rz2 / rz;
        rz = rz2;
        for k in 0..m {
            p[k] = z[k] + beta * p[k];
        }
    }
    for (k, i) in idx.iter().enumerate() {
        x[*i] = xf[k];
    }
}

const REG: f64 = 1e-9;

/// Deform a mesh: `rest` vertices, CCW `tris` with per-triangle `weights` (1 = normal,
/// larger = stiffer), constraints, and `refine` extra rigid-fit iterations. Returns the
/// deformed vertex positions.
pub fn deform(rest: &[[f64; 2]], tris: &[[usize; 3]], weights: &[f64], h: &Handles, refine: usize) -> Vec<[f64; 2]> {
    let n = rest.len();
    if n == 0 {
        return vec![];
    }
    // Initial guess: rest translated by the mean handle displacement.
    let mut t = [0.0, 0.0];
    for (i, p) in &h.fixed {
        t[0] += p[0] - rest[*i][0];
        t[1] += p[1] - rest[*i][1];
    }
    if !h.fixed.is_empty() {
        t[0] /= h.fixed.len() as f64;
        t[1] /= h.fixed.len() as f64;
    }
    let x0: Vec<[f64; 2]> = rest.iter().map(|p| [p[0] + t[0], p[1] + t[1]]).collect();
    // One pin and no edge targets: every rotation and scale about the pin costs nothing, and
    // the regulariser that picks the translation is too weak to stop CG turning round-off
    // into a visible rotation (#313). The translation is the exact solution.
    if h.fixed.len() == 1 && h.edges.is_empty() {
        return x0;
    }
    let mut fixed = vec![false; n];
    for (i, _) in &h.fixed {
        fixed[*i] = true;
    }
    let w = |k: usize| weights.get(k).copied().unwrap_or(1.0);
    let scale2 = mean_edge2(rest, tris).max(1e-12);

    // ---- Step 1: similarity-invariant energy over 2n unknowns (x0, y0, x1, y1, …).
    let mut a = Sparse::new(2 * n);
    let mut b = vec![0.0; 2 * n];
    for (k, t) in tris.iter().enumerate() {
        let wk = w(k);
        for (i, j, l) in [(t[0], t[1], t[2]), (t[1], t[2], t[0]), (t[2], t[0], t[1])] {
            let e = [rest[j][0] - rest[i][0], rest[j][1] - rest[i][1]];
            let d = [rest[l][0] - rest[i][0], rest[l][1] - rest[i][1]];
            let ee = e[0] * e[0] + e[1] * e[1];
            if ee <= 1e-18 {
                continue;
            }
            // R90(u) = (-u.y, u.x).
            let r90 = [-e[1], e[0]];
            let x = (d[0] * e[0] + d[1] * e[1]) / ee;
            let y = (d[0] * r90[0] + d[1] * r90[1]) / ee;
            // residual = B_l v_l + B_i v_i + B_j v_j with 2x2 blocks; R = [[0,-1],[1,0]].
            let bl = [[1.0, 0.0], [0.0, 1.0]];
            let bi = [[x - 1.0, -y], [y, x - 1.0]];
            let bj = [[-x, y], [-y, -x]];
            let blocks = [(l, bl), (i, bi), (j, bj)];
            for (va, ba) in &blocks {
                for (vb, bb) in &blocks {
                    // ba^T bb
                    for r in 0..2 {
                        for c in 0..2 {
                            let v = ba[0][r] * bb[0][c] + ba[1][r] * bb[1][c];
                            if v != 0.0 {
                                a.add(2 * va + r, 2 * vb + c, wk * v);
                            }
                        }
                    }
                }
            }
        }
    }
    for e in &h.edges {
        let ww = e.weight;
        for c in 0..2 {
            a.add(2 * e.i + c, 2 * e.i + c, ww);
            a.add(2 * e.j + c, 2 * e.j + c, ww);
            a.add(2 * e.i + c, 2 * e.j + c, -ww);
            a.add(2 * e.j + c, 2 * e.i + c, -ww);
            b[2 * e.j + c] += ww * e.vec[c];
            b[2 * e.i + c] -= ww * e.vec[c];
        }
    }
    let reg = REG * scale2.recip().min(1.0);
    for i in 0..n {
        for c in 0..2 {
            a.add(2 * i + c, 2 * i + c, reg);
            b[2 * i + c] += reg * x0[i][c];
        }
    }
    let mut x: Vec<f64> = x0.iter().flat_map(|p| [p[0], p[1]]).collect();
    for (i, p) in &h.fixed {
        x[2 * i] = p[0];
        x[2 * i + 1] = p[1];
    }
    let free2: Vec<bool> = (0..2 * n).map(|k| !fixed[k / 2]).collect();
    solve(&a, &b, &mut x, &free2);
    let mut cur: Vec<[f64; 2]> = (0..n).map(|i| [x[2 * i], x[2 * i + 1]]).collect();

    // ---- Step 2: fit rigid triangles, then match edge vectors (x and y separately).
    let mut l = Sparse::new(n);
    for (k, t) in tris.iter().enumerate() {
        let wk = w(k);
        for (i, j) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            l.add(i, i, wk);
            l.add(j, j, wk);
            l.add(i, j, -wk);
            l.add(j, i, -wk);
        }
    }
    for e in &h.edges {
        l.add(e.i, e.i, e.weight);
        l.add(e.j, e.j, e.weight);
        l.add(e.i, e.j, -e.weight);
        l.add(e.j, e.i, -e.weight);
    }
    for i in 0..n {
        l.add(i, i, reg);
    }
    let free1: Vec<bool> = (0..n).map(|i| !fixed[i]).collect();
    for _ in 0..=refine {
        let mut bx = vec![0.0; n];
        let mut by = vec![0.0; n];
        for (k, t) in tris.iter().enumerate() {
            let wk = w(k);
            let f = fit_rigid([rest[t[0]], rest[t[1]], rest[t[2]]], [cur[t[0]], cur[t[1]], cur[t[2]]]);
            for (a2, b2) in [(0, 1), (1, 2), (2, 0)] {
                let (i, j) = (t[a2], t[b2]);
                let d = [f[b2][0] - f[a2][0], f[b2][1] - f[a2][1]];
                bx[j] += wk * d[0];
                bx[i] -= wk * d[0];
                by[j] += wk * d[1];
                by[i] -= wk * d[1];
            }
        }
        for e in &h.edges {
            bx[e.j] += e.weight * e.vec[0];
            bx[e.i] -= e.weight * e.vec[0];
            by[e.j] += e.weight * e.vec[1];
            by[e.i] -= e.weight * e.vec[1];
        }
        for i in 0..n {
            bx[i] += reg * x0[i][0];
            by[i] += reg * x0[i][1];
        }
        let mut xs: Vec<f64> = cur.iter().map(|p| p[0]).collect();
        let mut ys: Vec<f64> = cur.iter().map(|p| p[1]).collect();
        for (i, p) in &h.fixed {
            xs[*i] = p[0];
            ys[*i] = p[1];
        }
        solve(&l, &bx, &mut xs, &free1);
        solve(&l, &by, &mut ys, &free1);
        cur = (0..n).map(|i| [xs[i], ys[i]]).collect();
    }
    cur
}

fn mean_edge2(rest: &[[f64; 2]], tris: &[[usize; 3]]) -> f64 {
    if tris.is_empty() {
        return 1.0;
    }
    let mut s = 0.0;
    for t in tris {
        let (a, b) = (rest[t[0]], rest[t[1]]);
        s += (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2);
    }
    s / tris.len() as f64
}

/// Best rigid copy of triangle `rest` over triangle `cur` (rotation about the centroids).
pub fn fit_rigid(rest: [[f64; 2]; 3], cur: [[f64; 2]; 3]) -> [[f64; 2]; 3] {
    let cr = [(rest[0][0] + rest[1][0] + rest[2][0]) / 3.0, (rest[0][1] + rest[1][1] + rest[2][1]) / 3.0];
    let cc = [(cur[0][0] + cur[1][0] + cur[2][0]) / 3.0, (cur[0][1] + cur[1][1] + cur[2][1]) / 3.0];
    let (mut sd, mut sc) = (0.0, 0.0);
    for k in 0..3 {
        let p = [rest[k][0] - cr[0], rest[k][1] - cr[1]];
        let q = [cur[k][0] - cc[0], cur[k][1] - cc[1]];
        sd += p[0] * q[0] + p[1] * q[1];
        sc += p[0] * q[1] - p[1] * q[0];
    }
    let ang = sc.atan2(sd);
    let (s, c) = ang.sin_cos();
    let mut out = [[0.0; 2]; 3];
    for k in 0..3 {
        let p = [rest[k][0] - cr[0], rest[k][1] - cr[1]];
        out[k] = [cc[0] + p[0] * c - p[1] * s, cc[1] + p[0] * s + p[1] * c];
    }
    out
}
