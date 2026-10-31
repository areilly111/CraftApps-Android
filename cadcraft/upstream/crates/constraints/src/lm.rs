//! Damped, weighted minimum-norm Gauss–Newton (Levenberg–Marquardt style) with a numerical
//! Jacobian.
//!
//! Each step solves `min ‖W^{1/2} Δx‖` subject to `J Δx = −r` (damped):
//! `Δx = −W⁻¹ Jᵀ (J W⁻¹ Jᵀ + μ I)⁻¹ r`. Under-constrained systems therefore move as little as
//! possible, and heavily weighted parameters (geometry the user just edited, or the first object
//! picked) move least.

/// Most residual rows one solve may have (the normal matrix is rows × rows).
pub const MAX_ROWS: usize = 800;

pub(crate) struct Outcome {
    pub x: Vec<f64>,
    pub converged: bool,
}

pub(crate) fn max_abs(v: &[f64]) -> f64 {
    v.iter().fold(0.0f64, |m, a| if a.is_finite() { m.max(a.abs()) } else { f64::INFINITY })
}

fn norm2(v: &[f64]) -> f64 {
    v.iter().map(|a| a * a).sum::<f64>()
}

/// Numerical Jacobian over the free parameters (row-major, rows × free.len()).
pub(crate) fn jacobian(f: &dyn Fn(&[f64], &mut Vec<f64>), x: &[f64], free: &[usize], rows: usize) -> Vec<f64> {
    let nf = free.len();
    let mut jac = vec![0.0; rows * nf];
    let mut xp = x.to_vec();
    let (mut rp, mut rm) = (Vec::with_capacity(rows), Vec::with_capacity(rows));
    for (k, &j) in free.iter().enumerate() {
        let Some(&xj) = x.get(j) else { continue };
        let h = 1e-7 * xj.abs().max(1.0);
        if let Some(v) = xp.get_mut(j) {
            *v = xj + h;
        }
        rp.clear();
        f(&xp, &mut rp);
        if let Some(v) = xp.get_mut(j) {
            *v = xj - h;
        }
        rm.clear();
        f(&xp, &mut rm);
        if let Some(v) = xp.get_mut(j) {
            *v = xj;
        }
        for i in 0..rows {
            let d = (rp.get(i).copied().unwrap_or(0.0) - rm.get(i).copied().unwrap_or(0.0)) / (2.0 * h);
            if let Some(slot) = jac.get_mut(i * nf + k) {
                *slot = if d.is_finite() { d } else { 0.0 };
            }
        }
    }
    jac
}

/// Solve the dense system `a y = b` (n × n, row-major) by Gaussian elimination with partial
/// pivoting. `None` when singular.
fn solve_dense(mut a: Vec<f64>, mut b: Vec<f64>, n: usize) -> Option<Vec<f64>> {
    let scale = a.iter().fold(0.0f64, |m, v| m.max(v.abs())).max(1e-300);
    for col in 0..n {
        let mut piv = col;
        let mut best = 0.0;
        for r in col..n {
            let v = a.get(r * n + col)?.abs();
            if v > best {
                best = v;
                piv = r;
            }
        }
        if best <= scale * 1e-15 {
            return None;
        }
        if piv != col {
            for c in 0..n {
                a.swap(piv * n + c, col * n + c);
            }
            b.swap(piv, col);
        }
        let p = *a.get(col * n + col)?;
        for r in col + 1..n {
            let f = *a.get(r * n + col)? / p;
            if f == 0.0 {
                continue;
            }
            for c in col..n {
                let v = *a.get(col * n + c)?;
                *a.get_mut(r * n + c)? -= f * v;
            }
            let bv = *b.get(col)?;
            *b.get_mut(r)? -= f * bv;
        }
    }
    let mut y = vec![0.0; n];
    for r in (0..n).rev() {
        let mut s = *b.get(r)?;
        for c in r + 1..n {
            s -= a.get(r * n + c)? * y.get(c)?;
        }
        *y.get_mut(r)? = s / a.get(r * n + r)?;
    }
    y.iter().all(|v| v.is_finite()).then_some(y)
}

/// Householder QR with column pivoting of the columns `cols` (each of the same length). Stops
/// when the largest remaining column norm drops to `rel` times the first pivot's (or below
/// `abs`). Returns the numerical rank and the Householder vectors (full length, zero above the
/// diagonal; each reflector is `I − 2vvᵀ/(vᵀv)`).
pub(crate) fn qr(mut cols: Vec<Vec<f64>>, rel: f64, abs: f64) -> (usize, Vec<Vec<f64>>) {
    let m = cols.first().map_or(0, Vec::len);
    let n = cols.len();
    let mut refl = Vec::new();
    let mut first = None;
    for k in 0..m.min(n) {
        // Pivot: the column with the largest remaining norm.
        let mut best = (k, -1.0f64);
        for (j, c) in cols.iter().enumerate().skip(k) {
            let v = c.get(k..).map_or(0.0, norm2);
            if v.is_finite() && v > best.1 {
                best = (j, v);
            }
        }
        let norm = best.1.max(0.0).sqrt();
        let lim = first.map_or(abs, |f: f64| (f * rel).max(abs));
        if crate::curves::not_above(norm, lim) {
            break;
        }
        first.get_or_insert(norm);
        cols.swap(k, best.0);
        let Some(col) = cols.get(k) else { break };
        let mut v = vec![0.0; m];
        for i in k..m {
            if let (Some(d), Some(s)) = (v.get_mut(i), col.get(i)) {
                *d = *s;
            }
        }
        let x0 = v.get(k).copied().unwrap_or(0.0);
        let alpha = if x0 >= 0.0 { -norm } else { norm };
        if let Some(d) = v.get_mut(k) {
            *d -= alpha;
        }
        let vv = norm2(&v);
        if crate::curves::not_above(vv, 0.0) {
            break;
        }
        for c in cols.iter_mut().skip(k) {
            reflect(&v, vv, c);
        }
        refl.push(v);
    }
    (refl.len(), refl)
}

fn reflect(v: &[f64], vv: f64, c: &mut [f64]) {
    let d: f64 = v.iter().zip(c.iter()).map(|(a, b)| a * b).sum();
    let f = 2.0 * d / vv;
    if f != 0.0 {
        for (ci, vi) in c.iter_mut().zip(v) {
            *ci -= f * vi;
        }
    }
}

/// Rows and columns scaled to unit norm (zero ones left alone), as columns of the transpose:
/// `out[i]` is row `i` of the row-major `rows × cols` matrix `m`.
fn balanced_rows(m: &[f64], rows: usize, cols: usize) -> Vec<Vec<f64>> {
    let mut r: Vec<Vec<f64>> = (0..rows).map(|i| m.get(i * cols..(i + 1) * cols).map(<[f64]>::to_vec).unwrap_or_else(|| vec![0.0; cols])).collect();
    for row in &mut r {
        let n = norm2(row).sqrt();
        let f = if n > 1e-12 && n.is_finite() { 1.0 / n } else { 0.0 };
        row.iter_mut().for_each(|v| *v *= f);
    }
    // Columns: unit norm, except numerical noise (an exactly zero derivative evaluated by
    // differences), which scaling would turn into a spurious independent direction.
    for c in 0..cols {
        let n = r.iter().map(|row| row.get(c).map_or(0.0, |v| v * v)).sum::<f64>().sqrt();
        let f = if n > COL_NOISE && n.is_finite() { 1.0 / n } else { 0.0 };
        for row in &mut r {
            if let Some(v) = row.get_mut(c) {
                *v *= f;
            }
        }
    }
    r
}

/// Column norm (after row balancing) below which a column is difference noise.
const COL_NOISE: f64 = 1e-6;
/// Relative threshold below which a pivot counts as zero (the Jacobian is numerical).
const RANK_TOL: f64 = 1e-7;

/// Numerical rank of a row-major `rows × cols` Jacobian (rows and columns balanced first,
/// Householder QR with column pivoting).
pub(crate) fn rank_qr(m: &[f64], rows: usize, cols: usize) -> usize {
    if rows == 0 || cols == 0 {
        return 0;
    }
    // QR of the transpose: its columns are the Jacobian's rows.
    qr(balanced_rows(m, rows, cols), RANK_TOL, 1e-12).0
}

/// Rank and an orthonormal basis of the null space (each vector of length `cols`) of a
/// row-major `rows × cols` Jacobian, in balanced column coordinates (a diagonal scaling, so
/// which parameters a null vector moves is preserved).
pub(crate) fn null_space(m: &[f64], rows: usize, cols: usize) -> (usize, Vec<Vec<f64>>) {
    let (rank, refl) = if rows == 0 { (0, Vec::new()) } else { qr(balanced_rows(m, rows, cols), RANK_TOL, 1e-12) };
    let mut basis = Vec::with_capacity(cols.saturating_sub(rank));
    for j in rank..cols {
        let mut e = vec![0.0; cols];
        if let Some(v) = e.get_mut(j) {
            *v = 1.0;
        }
        // Q e_j with Q = H_0 H_1 … H_{r−1}.
        for v in refl.iter().rev() {
            let vv = norm2(v);
            if vv > 0.0 {
                reflect(v, vv, &mut e);
            }
        }
        basis.push(e);
    }
    (rank, basis)
}

/// Run the solver from `x0`. Only `free` parameters change.
pub(crate) fn solve(f: &dyn Fn(&[f64], &mut Vec<f64>), x0: &[f64], free: &[usize], weights: &[f64], tol: f64, max_iter: usize) -> Outcome {
    let mut x = x0.to_vec();
    let mut r = Vec::new();
    f(&x, &mut r);
    let rows = r.len();
    if rows == 0 || max_abs(&r) <= tol {
        return Outcome { x, converged: true };
    }
    if free.is_empty() || rows > MAX_ROWS {
        return Outcome { x, converged: false };
    }
    let nf = free.len();
    let winv: Vec<f64> = free.iter().map(|&j| 1.0 / weights.get(j).copied().unwrap_or(1.0).max(1e-12)).collect();
    let mut mu = 1e-10;
    let mut cost = norm2(&r);
    let mut iter = 0;
    let mut rn = Vec::with_capacity(rows);
    while iter < max_iter {
        iter += 1;
        let jac = jacobian(f, &x, free, rows);
        // A = J W⁻¹ Jᵀ
        let mut a = vec![0.0; rows * rows];
        for i in 0..rows {
            for k in i..rows {
                let mut s = 0.0;
                for (c, w) in winv.iter().enumerate() {
                    s += jac.get(i * nf + c).copied().unwrap_or(0.0) * jac.get(k * nf + c).copied().unwrap_or(0.0) * w;
                }
                if let Some(v) = a.get_mut(i * rows + k) {
                    *v = s;
                }
                if let Some(v) = a.get_mut(k * rows + i) {
                    *v = s;
                }
            }
        }
        let diag_max = (0..rows).map(|i| a.get(i * rows + i).copied().unwrap_or(0.0)).fold(0.0f64, f64::max).max(1e-12);
        let mut improved = false;
        for _ in 0..12 {
            let mut ad = a.clone();
            for i in 0..rows {
                if let Some(v) = ad.get_mut(i * rows + i) {
                    *v += mu * diag_max;
                }
            }
            let Some(y) = solve_dense(ad, r.clone(), rows) else {
                mu *= 10.0;
                continue;
            };
            let mut xn = x.clone();
            for (c, &j) in free.iter().enumerate() {
                let mut s = 0.0;
                for (i, yi) in y.iter().enumerate() {
                    s += jac.get(i * nf + c).copied().unwrap_or(0.0) * yi;
                }
                if let Some(v) = xn.get_mut(j) {
                    *v -= winv.get(c).copied().unwrap_or(1.0) * s;
                }
            }
            rn.clear();
            f(&xn, &mut rn);
            let cn = norm2(&rn);
            if cn.is_finite() && cn < cost {
                x = xn;
                std::mem::swap(&mut r, &mut rn);
                cost = cn;
                mu = (mu * 0.1).max(1e-15);
                improved = true;
                break;
            }
            mu *= 10.0;
        }
        if max_abs(&r) <= tol {
            return Outcome { x, converged: true };
        }
        if !improved {
            break;
        }
    }
    let converged = max_abs(&r) <= tol;
    Outcome { x, converged }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dense_and_rank() {
        let y = solve_dense(vec![2.0, 1.0, 1.0, 3.0], vec![3.0, 5.0], 2).unwrap();
        assert!((y[0] - 0.8).abs() < 1e-12 && (y[1] - 1.4).abs() < 1e-12);
        assert!(solve_dense(vec![1.0, 2.0, 2.0, 4.0], vec![1.0, 1.0], 2).is_none());
        assert_eq!(rank_qr(&[1.0, 0.0, 0.0, 1.0, 1.0, 1.0], 3, 2), 2);
        assert_eq!(rank_qr(&[1.0, 2.0, 2.0, 4.0], 2, 2), 1);
        assert_eq!(rank_qr(&[0.0; 4], 2, 2), 0);
        assert_eq!(rank_qr(&[1e-9, 0.0, 0.0, 1e6], 2, 2), 2, "balanced: scale does not hide rank");
    }

    #[test]
    fn null_space_spans_free_motion() {
        // x0 + x1 = c fixes one combination; x2 is untouched.
        let (r, n) = null_space(&[1.0, 1.0, 0.0], 1, 3);
        assert_eq!((r, n.len()), (1, 2));
        for v in &n {
            assert!((v[0] + v[1]).abs() < 1e-12);
        }
        // x2's own motion survives in the null space; x0 alone does not.
        assert!(n.iter().any(|v| v[2].abs() > 0.5));
        let (r, n) = null_space(&[], 0, 2);
        assert_eq!((r, n.len()), (0, 2));
    }

    #[test]
    fn minimum_norm_step() {
        // One equation x + y = 2 from (0,0): the minimum-norm solution is (1,1).
        let f = |x: &[f64], out: &mut Vec<f64>| out.push(x[0] + x[1] - 2.0);
        let o = solve(&f, &[0.0, 0.0], &[0, 1], &[1.0, 1.0], 1e-12, 50);
        assert!(o.converged);
        assert!((o.x[0] - 1.0).abs() < 1e-9 && (o.x[1] - 1.0).abs() < 1e-9);
        // Heavy weight on x keeps it in place.
        let o = solve(&f, &[0.0, 0.0], &[0, 1], &[1e6, 1.0], 1e-12, 50);
        assert!(o.x[0].abs() < 1e-5 && (o.x[1] - 2.0).abs() < 1e-5);
    }

    #[test]
    fn inconsistent_does_not_converge() {
        let f = |x: &[f64], out: &mut Vec<f64>| {
            out.push(x[0] - 1.0);
            out.push(x[0] - 2.0);
        };
        let o = solve(&f, &[0.0], &[0], &[1.0], 1e-10, 100);
        assert!(!o.converged);
        assert!(o.x[0].is_finite());
    }
}
