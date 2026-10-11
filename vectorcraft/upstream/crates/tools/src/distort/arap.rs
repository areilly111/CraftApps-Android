//! As-Rigid-As-Possible shape manipulation (Igarashi, Moscovich & Hughes, SIGGRAPH 2005),
//! implemented from the paper.
//!
//! - The mesh is a regular grid of triangles over the artwork: grid cells are kept when their
//!   centre is inside the filled outline or near any path (so separate limbs stay separate).
//! - Step 1 (similarity): every triangle vertex is expressed in the local frame of the opposite
//!   edge, `v2 = v0 + x·(v1 − v0) + y·R90(v1 − v0)`; the quadratic error of those relations over
//!   all triangles (rotation + uniform scale free) plus soft pin constraints is minimised.
//! - Step 2 (scale adjustment): each triangle's rest shape is rigidly fitted (optimal 2D rotation)
//!   to its step-1 result, then the vertices are solved so every edge vector matches its fitted
//!   triangle's, which removes the scaling step 1 allows.
//! - Pins are arbitrary points (barycentric combinations of a triangle's vertices), so both steps
//!   use weighted soft constraints. Systems are symmetric positive definite and banded (vertices
//!   are numbered row by row), solved with a banded Cholesky factorisation written here.
//! - Artwork points (anchors and handles) follow the mesh through barycentric coordinates in
//!   their rest triangle.

use vectorcraft_geom::kurbo::ParamCurveNearest;
use vectorcraft_geom::{BezPath, Point, Rect, Shape, Vec2};

/// Banded symmetric positive definite matrix (lower band stored row by row).
#[derive(Clone, Debug)]
pub struct BandMatrix {
    n: usize,
    bw: usize,
    /// Row i holds columns i-bw..=i at offsets 0..=bw.
    a: Vec<f64>,
}

impl BandMatrix {
    pub fn new(n: usize, bw: usize) -> Self {
        Self { n, bw, a: vec![0.0; n * (bw + 1)] }
    }
    pub fn len(&self) -> usize {
        self.n
    }
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }
    fn idx(&self, i: usize, j: usize) -> usize {
        // j <= i, i - j <= bw
        i * (self.bw + 1) + (j + self.bw - i)
    }
    /// Add `v` at (i, j) (and implicitly (j, i)); diagonal entries are added once.
    pub fn add(&mut self, i: usize, j: usize, v: f64) {
        let (i, j) = if i >= j { (i, j) } else { (j, i) };
        debug_assert!(i - j <= self.bw, "outside band");
        let k = self.idx(i, j);
        self.a[k] += v;
    }
    pub fn get(&self, i: usize, j: usize) -> f64 {
        let (i, j) = if i >= j { (i, j) } else { (j, i) };
        if i - j > self.bw { 0.0 } else { self.a[self.idx(i, j)] }
    }
    /// In-place Cholesky factorisation (L·Lᵀ). Returns false if the matrix isn't positive definite.
    pub fn factor(&mut self) -> bool {
        let (n, bw) = (self.n, self.bw);
        for i in 0..n {
            let j0 = i.saturating_sub(bw);
            for j in j0..=i {
                let mut s = self.a[self.idx(i, j)];
                let k0 = j0.max(j.saturating_sub(bw));
                for k in k0..j {
                    s -= self.a[self.idx(i, k)] * self.a[self.idx(j, k)];
                }
                if i == j {
                    if s <= 0.0 || !s.is_finite() {
                        return false;
                    }
                    let k = self.idx(i, i);
                    self.a[k] = s.sqrt();
                } else {
                    let k = self.idx(i, j);
                    self.a[k] = s / self.a[self.idx(j, j)];
                }
            }
        }
        true
    }
    /// Solve with a factored matrix.
    pub fn solve(&self, b: &mut [f64]) {
        let (n, bw) = (self.n, self.bw);
        for i in 0..n {
            let mut s = b[i];
            let k0 = i.saturating_sub(bw);
            for (k, bk) in b[k0..i].iter().enumerate() {
                s -= self.a[self.idx(i, k0 + k)] * bk;
            }
            b[i] = s / self.a[self.idx(i, i)];
        }
        for i in (0..n).rev() {
            let mut s = b[i];
            for (k, bk) in b[i + 1..(i + bw + 1).min(n)].iter().enumerate() {
                s -= self.a[self.idx(i + 1 + k, i)] * bk;
            }
            b[i] = s / self.a[self.idx(i, i)];
        }
    }
}

/// A triangle mesh on a regular grid.
#[derive(Clone, Debug)]
pub struct Mesh {
    pub verts: Vec<Point>,
    pub tris: Vec<[usize; 3]>,
    origin: Point,
    cell: f64,
    nx: usize,
    ny: usize,
    /// Per grid cell: its two triangles (None = cell not part of the mesh).
    cells: Vec<Option<[usize; 2]>>,
    /// Per grid cell: centre inside the art (not just the band around it).
    core: Vec<bool>,
}

/// Mesh construction options.
#[derive(Clone, Copy, Debug)]
pub struct MeshOptions {
    /// Cells along the longer side.
    pub resolution: usize,
    /// Expand Mesh: extra reach around the outline, points.
    pub expand: f64,
}

impl Default for MeshOptions {
    fn default() -> Self {
        Self { resolution: 20, expand: 3.0 }
    }
}

impl Mesh {
    /// Build a mesh covering `bounds` restricted to the region of `outlines` (filled interiors of
    /// closed subpaths plus a band around every path). With no outlines, the whole bounds.
    pub fn build(outlines: &[BezPath], bounds: Rect, opt: MeshOptions) -> Mesh {
        // The cells cover the expanded art, so tiny art with a wide Expand stays a small grid.
        let side = (bounds.width().max(bounds.height()) + 2.0 * opt.expand.max(0.0)).max(1e-3);
        let cell = (side / opt.resolution.max(1) as f64).max(1e-3);
        let pad = opt.expand.max(0.0) + cell * 0.01;
        let b = bounds.inflate(pad, pad);
        let nx = ((b.width() / cell).ceil() as usize).max(1);
        let ny = ((b.height() / cell).ceil() as usize).max(1);
        let origin = Point::new(b.x0, b.y0);
        let reach = cell * 0.75 + opt.expand.max(0.0);
        // 0 = outside, 1 = in the band around the art, 2 = core (inside a fill or on a path).
        let keep = |c: Point| -> u8 {
            if outlines.is_empty() {
                return 2;
            }
            let near = |o: &BezPath, r: f64| o.segments().any(|s| s.nearest(c, 1e-3).distance_sq <= r * r);
            if outlines.iter().any(|o| closed_winding(o, c) != 0 || near(o, cell * 0.25)) {
                return 2;
            }
            u8::from(outlines.iter().any(|o| {
                let bb = o.bounding_box().inflate(reach, reach);
                if !bb.contains(c) {
                    return false;
                }
                if closed_winding(o, c) != 0 {
                    return true;
                }
                near(o, reach)
            }))
        };
        let mut vid = vec![usize::MAX; (nx + 1) * (ny + 1)];
        let mut verts = vec![];
        let mut kept = vec![false; nx * ny];
        let mut core = vec![false; nx * ny];
        for j in 0..ny {
            for i in 0..nx {
                let c = Point::new(origin.x + (i as f64 + 0.5) * cell, origin.y + (j as f64 + 0.5) * cell);
                let k = keep(c);
                kept[j * nx + i] = k > 0;
                core[j * nx + i] = k == 2;
            }
        }
        // Number vertices row by row (keeps the systems banded).
        for j in 0..=ny {
            for i in 0..=nx {
                let used = [(i.wrapping_sub(1), j.wrapping_sub(1)), (i, j.wrapping_sub(1)), (i.wrapping_sub(1), j), (i, j)]
                    .iter()
                    .any(|&(ci, cj)| ci < nx && cj < ny && kept[cj * nx + ci]);
                if used {
                    vid[j * (nx + 1) + i] = verts.len();
                    verts.push(Point::new(origin.x + i as f64 * cell, origin.y + j as f64 * cell));
                }
            }
        }
        let mut tris = vec![];
        let mut cells = vec![None; nx * ny];
        for j in 0..ny {
            for i in 0..nx {
                if !kept[j * nx + i] {
                    continue;
                }
                let v = |di: usize, dj: usize| vid[(j + dj) * (nx + 1) + i + di];
                let (v00, v10, v01, v11) = (v(0, 0), v(1, 0), v(0, 1), v(1, 1));
                let t0 = tris.len();
                tris.push([v00, v10, v11]);
                tris.push([v00, v11, v01]);
                cells[j * nx + i] = Some([t0, t0 + 1]);
            }
        }
        Mesh { verts, tris, origin, cell, nx, ny, cells, core }
    }

    /// Band width (in vertices) of the vertex adjacency.
    fn vertex_band(&self) -> usize {
        self.tris.iter().filter_map(|t| Some(t.iter().max()? - t.iter().min()?)).max().unwrap_or(0)
    }

    /// The triangle and barycentric coordinates of `p` (extrapolated from the nearest mesh cell
    /// when `p` is outside the mesh).
    pub fn locate(&self, p: Point) -> Option<(usize, [f64; 3])> {
        let fi = ((p.x - self.origin.x) / self.cell).floor();
        let fj = ((p.y - self.origin.y) / self.cell).floor();
        let inside = fi >= 0.0 && fj >= 0.0 && (fi as usize) < self.nx && (fj as usize) < self.ny;
        let cell = if inside && self.cells[fj as usize * self.nx + fi as usize].is_some() {
            fj as usize * self.nx + fi as usize
        } else {
            // Nearest kept cell centre.
            let mut best = None;
            for (k, c) in self.cells.iter().enumerate() {
                if c.is_none() {
                    continue;
                }
                let cc =
                    Point::new(self.origin.x + ((k % self.nx) as f64 + 0.5) * self.cell, self.origin.y + ((k / self.nx) as f64 + 0.5) * self.cell);
                let d = cc.distance_squared(p);
                if best.is_none_or(|(bd, _)| d < bd) {
                    best = Some((d, k));
                }
            }
            best?.1
        };
        let [ta, tb] = self.cells[cell]?;
        // Pick the triangle on p's side of the diagonal v00–v11.
        let v00 = self.verts[self.tris[ta][0]];
        let local = (p - v00) / self.cell;
        let t = if local.x >= local.y { ta } else { tb };
        Some((t, self.bary(t, p)))
    }

    fn bary(&self, t: usize, p: Point) -> [f64; 3] {
        let [a, b, c] = self.tris[t].map(|i| self.verts[i]);
        let (v0, v1, v2) = (b - a, c - a, p - a);
        let den = v0.cross(v1);
        if den.abs() < 1e-18 {
            return [1.0, 0.0, 0.0];
        }
        let l1 = v2.cross(v1) / den;
        let l2 = v0.cross(v2) / den;
        [1.0 - l1 - l2, l1, l2]
    }

    /// Map a rest-space point through deformed vertex positions.
    pub fn map(&self, deformed: &[Point], p: Point) -> Point {
        match self.locate(p) {
            Some((t, w)) => {
                let [a, b, c] = self.tris[t].map(|i| deformed[i].to_vec2());
                (a * w[0] + b * w[1] + c * w[2]).to_point()
            }
            None => p,
        }
    }

    /// Signed area of triangle `t` for the given vertex positions.
    pub fn tri_area(&self, pos: &[Point], t: usize) -> f64 {
        let [a, b, c] = self.tris[t].map(|i| pos[i]);
        (b - a).cross(c - a) / 2.0
    }

    /// Is `p` (rest space) on the mesh?
    pub fn contains(&self, p: Point) -> bool {
        self.cell_at(p).is_some_and(|k| self.cells.get(k).is_some_and(Option::is_some))
    }

    /// The grid cell under `p`, if inside the grid.
    fn cell_at(&self, p: Point) -> Option<usize> {
        let fi = ((p.x - self.origin.x) / self.cell).floor();
        let fj = ((p.y - self.origin.y) / self.cell).floor();
        (fi >= 0.0 && fj >= 0.0 && fi < self.nx as f64 && fj < self.ny as f64).then(|| fj as usize * self.nx + fi as usize)
    }

    /// The rest-space point that the `deformed` mesh shows at `q` (None: `q` is off the mesh).
    /// Where the warp folds the mesh over itself, the triangle `q` is deepest inside wins.
    pub fn unmap(&self, deformed: &[Point], q: Point) -> Option<Point> {
        let mut best: Option<(f64, Point)> = None;
        for t in &self.tris {
            let [a, b, c] = t.map(|i| deformed.get(i).copied().unwrap_or(self.verts[i]));
            let den = (b - a).cross(c - a);
            if den.abs() < 1e-18 {
                continue;
            }
            let l1 = (q - a).cross(c - a) / den;
            let l2 = (b - a).cross(q - a) / den;
            let w = [1.0 - l1 - l2, l1, l2];
            let depth = w.iter().copied().fold(f64::MAX, f64::min);
            if depth >= -1e-9 && best.is_none_or(|(d, _)| depth > d) {
                let [ra, rb, rc] = t.map(|i| self.verts[i].to_vec2());
                best = Some((depth, (ra * w[0] + rb * w[1] + rc * w[2]).to_point()));
            }
        }
        best.map(|b| b.1)
    }

    /// How far (radians) the `deformed` mesh turned the art at rest point `p`.
    pub fn turn_at(&self, deformed: &[Point], p: Point) -> Option<f64> {
        let (t, _) = self.locate(p)?;
        let tri = self.tris.get(t)?;
        let cur = [*deformed.get(tri[0])?, *deformed.get(tri[1])?, *deformed.get(tri[2])?];
        Some(fit_angle(&tri.map(|i| self.verts[i]), &cur))
    }

    /// Centre of grid cell `k`.
    fn cell_centre(&self, k: usize) -> Point {
        Point::new(self.origin.x + ((k % self.nx) as f64 + 0.5) * self.cell, self.origin.y + ((k / self.nx) as f64 + 0.5) * self.cell)
    }

    /// The cells pins go in: those inside the art (or every kept cell when none is).
    fn pin_cells(&self) -> Vec<bool> {
        let any_core = self.core.iter().any(|c| *c);
        self.cells.iter().zip(&self.core).map(|(c, core)| c.is_some() && (*core || !any_core)).collect()
    }

    /// The 8 neighbours of cell `k` inside the grid, with the step length (in cells).
    fn neighbours(&self, k: usize) -> impl Iterator<Item = (usize, f64)> + use<> {
        let (nx, ny) = (self.nx as i64, self.ny as i64);
        let (i, j) = ((k % self.nx) as i64, (k / self.nx) as i64);
        (-1i64..=1).flat_map(move |dj| (-1i64..=1).map(move |di| (di, dj))).filter_map(move |(di, dj)| {
            let (a, b) = (i + di, j + dj);
            let step = if di != 0 && dj != 0 { std::f64::consts::SQRT_2 } else { 1.0 };
            ((di, dj) != (0, 0) && a >= 0 && b >= 0 && a < nx && b < ny).then_some(((b * nx + a) as usize, step))
        })
    }

    /// Distances (in cells, through the cells where `inside`) from `sources` (cells with their
    /// start distances) to every cell; cells out of reach stay at infinity.
    fn geodesic(&self, inside: &[bool], sources: &[(usize, f64)]) -> Vec<f64> {
        let mut dist = vec![f64::INFINITY; self.cells.len()];
        for &(k, d) in sources {
            if let Some(v) = dist.get_mut(k) {
                *v = v.min(d);
            }
        }
        let inside = |k: usize| inside.get(k).copied().unwrap_or(false);
        // Relax until nothing improves (the grid is small: a few dozen cells a side).
        let mut changed = true;
        while changed {
            changed = false;
            for k in 0..dist.len() {
                if !inside(k) || !dist[k].is_finite() {
                    continue;
                }
                for (m, step) in self.neighbours(k) {
                    if inside(m) && dist[k] + step < dist[m] - 1e-12 {
                        dist[m] = dist[k] + step;
                        changed = true;
                    }
                }
            }
        }
        dist
    }
}

/// Winding number of the closed subpaths of `o` at `p` (open subpaths ignored).
fn closed_winding(o: &BezPath, p: Point) -> i32 {
    let mut total = 0;
    let mut cur = BezPath::new();
    let mut closed = false;
    let flush = |cur: &mut BezPath, closed: bool, total: &mut i32| {
        if closed && !cur.elements().is_empty() {
            *total += cur.winding(p);
        }
        *cur = BezPath::new();
    };
    for el in o.elements() {
        match el {
            vectorcraft_geom::PathEl::MoveTo(_) => {
                flush(&mut cur, closed, &mut total);
                closed = false;
                cur.push(*el);
            }
            vectorcraft_geom::PathEl::ClosePath => {
                closed = true;
                cur.push(*el);
            }
            e => cur.push(*e),
        }
    }
    flush(&mut cur, closed, &mut total);
    total
}

/// Weight of the soft pin constraints relative to the shape terms.
const PIN_WEIGHT: f64 = 1.0e5;
/// Tikhonov regularisation toward the rest pose (keeps pin-less islands in place).
const REG: f64 = 1.0e-7;

/// A pin in rest space and where it should go.
#[derive(Clone, Copy, Debug)]
pub struct Pin {
    pub rest: Point,
    pub target: Point,
    /// The rotation (radians) the art takes around the pin; `None` leaves it free (the solve
    /// picks it), as for a pin that was only moved.
    pub angle: Option<f64>,
}

impl Pin {
    /// A pin free to turn.
    pub fn new(rest: Point, target: Point) -> Self {
        Self { rest, target, angle: None }
    }

    /// The soft constraints this pin puts on the mesh: the pin itself and, when its rotation is
    /// held, two points a cell away along the rest axes, turned by the angle around the target.
    fn constraints(&self, reach: f64) -> impl Iterator<Item = (Point, Point)> + use<> {
        let frame = self.angle.map(|a| {
            let (s, c) = a.sin_cos();
            [Vec2::new(reach, 0.0), Vec2::new(0.0, reach)].map(|d| (self.rest + d, self.target + Vec2::new(d.x * c - d.y * s, d.x * s + d.y * c)))
        });
        std::iter::once((self.rest, self.target)).chain(frame.into_iter().flatten())
    }
}

/// Run both ARAP steps. Returns the deformed vertex positions (the rest mesh if fewer than one
/// pin lands on it or a system is singular).
pub fn deform(mesh: &Mesh, pins: &[Pin]) -> Vec<Point> {
    let n = mesh.verts.len();
    if n == 0 || pins.is_empty() {
        return mesh.verts.clone();
    }
    let located: Vec<(usize, [f64; 3], Point)> =
        pins.iter().flat_map(|p| p.constraints(mesh.cell)).filter_map(|(rest, target)| mesh.locate(rest).map(|(t, w)| (t, w, target))).collect();
    let vb = mesh.vertex_band();
    // ---- step 1: similarity-invariant error, 2n unknowns interleaved (x0, y0, x1, y1, ...).
    let mut g = BandMatrix::new(2 * n, 2 * vb + 1);
    for tri in &mesh.tris {
        for k in 0..3 {
            let (a, b, c) = (tri[k], tri[(k + 1) % 3], tri[(k + 2) % 3]);
            let (pa, pb, pc) = (mesh.verts[a], mesh.verts[b], mesh.verts[c]);
            let e = pb - pa;
            let l2 = e.hypot2();
            if l2 < 1e-18 {
                continue;
            }
            let d = pc - pa;
            let x = d.dot(e) / l2;
            let y = d.dot(Vec2::new(-e.y, e.x)) / l2;
            // Residual coefficients over (ax, ay, bx, by, cx, cy); see module docs.
            let vars = [2 * a, 2 * a + 1, 2 * b, 2 * b + 1, 2 * c, 2 * c + 1];
            let rx = [x - 1.0, -y, -x, y, 1.0, 0.0];
            let ry = [y, x - 1.0, -y, -x, 0.0, 1.0];
            for r in [rx, ry] {
                for i in 0..6 {
                    for j in 0..=i {
                        let v = r[i] * r[j];
                        if v != 0.0 {
                            g.add(vars[i], vars[j], v);
                        }
                    }
                }
            }
        }
    }
    let mut rhs = vec![0.0; 2 * n];
    for (t, w, target) in &located {
        let tri = mesh.tris[*t];
        for comp in 0..2 {
            for i in 0..3 {
                for j in 0..=i {
                    g.add(2 * tri[i] + comp, 2 * tri[j] + comp, PIN_WEIGHT * w[i] * w[j]);
                }
                rhs[2 * tri[i] + comp] += PIN_WEIGHT * w[i] * if comp == 0 { target.x } else { target.y };
            }
        }
    }
    for (i, v) in mesh.verts.iter().enumerate() {
        g.add(2 * i, 2 * i, REG);
        g.add(2 * i + 1, 2 * i + 1, REG);
        rhs[2 * i] += REG * v.x;
        rhs[2 * i + 1] += REG * v.y;
    }
    if !g.factor() {
        return mesh.verts.clone();
    }
    g.solve(&mut rhs);
    let step1: Vec<Point> = (0..n).map(|i| Point::new(rhs[2 * i], rhs[2 * i + 1])).collect();

    // ---- step 2: fit rigid triangles, then match edge vectors (x and y separable).
    let mut l = BandMatrix::new(n, vb);
    let mut bx = vec![0.0; n];
    let mut by = vec![0.0; n];
    for tri in &mesh.tris {
        let rest = tri.map(|i| mesh.verts[i]);
        let cur = tri.map(|i| step1[i]);
        let (rc, cc) = (centroid(&rest), centroid(&cur));
        let (s, co) = fit_angle(&rest, &cur).sin_cos();
        let fit = rest.map(|r| {
            let d = r - rc;
            cc + Vec2::new(d.x * co - d.y * s, d.x * s + d.y * co)
        });
        for k in 0..3 {
            let (i, j) = (tri[k], tri[(k + 1) % 3]);
            let e = fit[k] - fit[(k + 1) % 3];
            // (v_i - v_j - e)^2
            l.add(i, i, 1.0);
            l.add(j, j, 1.0);
            l.add(i, j, -1.0);
            bx[i] += e.x;
            bx[j] -= e.x;
            by[i] += e.y;
            by[j] -= e.y;
        }
    }
    for (t, w, target) in &located {
        let tri = mesh.tris[*t];
        for i in 0..3 {
            for j in 0..=i {
                l.add(tri[i], tri[j], PIN_WEIGHT * w[i] * w[j]);
            }
            bx[tri[i]] += PIN_WEIGHT * w[i] * target.x;
            by[tri[i]] += PIN_WEIGHT * w[i] * target.y;
        }
    }
    for (i, v) in step1.iter().enumerate() {
        l.add(i, i, REG);
        bx[i] += REG * v.x;
        by[i] += REG * v.y;
    }
    if !l.factor() {
        return step1;
    }
    l.solve(&mut bx);
    l.solve(&mut by);
    (0..n).map(|i| Point::new(bx[i], by[i])).collect()
}

fn centroid(p: &[Point; 3]) -> Point {
    Point::new((p[0].x + p[1].x + p[2].x) / 3.0, (p[0].y + p[1].y + p[2].y) / 3.0)
}

/// The rotation (radians) that best fits triangle `rest` onto `cur` (least squares, about their
/// centroids).
fn fit_angle(rest: &[Point; 3], cur: &[Point; 3]) -> f64 {
    let (rc, cc) = (centroid(rest), centroid(cur));
    let (mut sc, mut ss) = (0.0, 0.0);
    for k in 0..3 {
        let (r, c) = (rest[k] - rc, cur[k] - cc);
        sc += r.dot(c);
        ss += r.cross(c);
    }
    ss.atan2(sc)
}

/// Pick `count` well-spread pin positions inside the mesh (farthest-point sampling over the
/// kept cell centres, starting from the one nearest the centroid).
pub fn auto_pins(mesh: &Mesh, count: usize) -> Vec<Point> {
    let mut out: Vec<Point> = centre_of(mesh).map(|c| c.1).into_iter().collect();
    spread_pins(mesh, &mut out, count);
    out
}

/// The centre of the cells pins go in: the cell nearest their centroid, and the centroid itself
/// (that cell's centre when the centroid falls off the art, as in a ring).
fn centre_of(mesh: &Mesh) -> Option<(usize, Point)> {
    let cells: Vec<usize> = mesh.pin_cells().iter().enumerate().filter(|(_, c)| **c).map(|(k, _)| k).collect();
    let c = (cells.iter().fold(Vec2::ZERO, |a, k| a + mesh.cell_centre(*k).to_vec2()) / cells.len().max(1) as f64).to_point();
    let k = cells.into_iter().min_by(|a, b| mesh.cell_centre(*a).distance_squared(c).total_cmp(&mesh.cell_centre(*b).distance_squared(c)))?;
    Some((k, if mesh.contains(c) { c } else { mesh.cell_centre(k) }))
}

/// The most pins placed automatically.
const MAX_AUTO_PINS: usize = 12;

/// The pins the Puppet Warp tool places by itself: one at the centre of the art and one at the
/// end of each limb (a part reaching well beyond the art's thickness, like the arms of a star or
/// the ends of a long bar); at least three in all where the art has room.
pub fn default_pins(mesh: &Mesh) -> Vec<Point> {
    let Some((centre, middle)) = centre_of(mesh) else { return vec![] };
    let inside = mesh.pin_cells();
    // Thickness: the largest distance from inside the art to its edge.
    let edge: Vec<(usize, f64)> = (0..inside.len())
        .filter(|k| inside[*k] && (mesh.neighbours(*k).count() < 8 || mesh.neighbours(*k).any(|(m, _)| !inside[m])))
        .map(|k| (k, 0.5))
        .collect();
    let radius = mesh.geodesic(&inside, &edge).into_iter().filter(|d| d.is_finite()).fold(0.0, f64::max);
    // Limb ends: the cells farthest from the centre (through the art), well beyond the thickness.
    let from_centre = mesh.geodesic(&inside, &[(centre, 0.0)]);
    let mut ends: Vec<usize> = (0..inside.len())
        .filter(|k| {
            let d = from_centre[*k];
            inside[*k] && d.is_finite() && d >= 1.5 * radius && mesh.neighbours(*k).all(|(m, _)| !inside[m] || from_centre[m] <= d)
        })
        .collect();
    ends.sort_by(|a, b| from_centre[*b].total_cmp(&from_centre[*a]));
    // One pin per limb: ends closer than about the art's width belong to the same limb.
    let apart = 2.2 * radius * mesh.cell;
    let mut out = vec![middle];
    let mut tips: Vec<Point> = vec![];
    for k in ends {
        let p = mesh.cell_centre(k);
        if out.len() >= MAX_AUTO_PINS || tips.iter().any(|q| q.distance(p) < apart) {
            continue;
        }
        tips.push(p);
        // The pin goes in the middle of the limb's end: the centroid of the cells about as far out
        // near its tip (the tip itself if that falls off the art).
        let cap: Vec<Point> = (0..inside.len())
            .filter(|m| inside[*m] && from_centre[*m] >= from_centre[k] - radius)
            .map(|m| mesh.cell_centre(m))
            .filter(|q| q.distance(p) < apart)
            .collect();
        let mid = (cap.iter().fold(Vec2::ZERO, |a, q| a + q.to_vec2()) / cap.len().max(1) as f64).to_point();
        out.push(if mesh.contains(mid) { mid } else { p });
    }
    spread_pins(mesh, &mut out, 3);
    out
}

/// Add pins to `out` until there are `count`, each as far as possible from the others
/// (farthest-point sampling over the cells pins go in).
fn spread_pins(mesh: &Mesh, out: &mut Vec<Point>, count: usize) {
    let centres: Vec<Point> = mesh.pin_cells().iter().enumerate().filter(|(_, c)| **c).map(|(k, _)| mesh.cell_centre(k)).collect();
    while out.len() < count.min(centres.len()) {
        let Some(&next) = centres.iter().max_by(|a, b| {
            let da = out.iter().map(|o| o.distance_squared(**a)).fold(f64::MAX, f64::min);
            let db = out.iter().map(|o| o.distance_squared(**b)).fold(f64::MAX, f64::min);
            da.total_cmp(&db)
        }) else {
            break;
        };
        out.push(next);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_geom::shapes;

    #[test]
    fn band_cholesky_solves_spd_systems() {
        // Tridiagonal [4 1; 1 4 1; ...] vs a dense reference.
        let n = 12;
        let mut m = BandMatrix::new(n, 1);
        for i in 0..n {
            m.add(i, i, 4.0);
            if i > 0 {
                m.add(i, i - 1, 1.0);
            }
        }
        let x: Vec<f64> = (0..n).map(|i| i as f64 * 0.5 - 2.0).collect();
        let mut b: Vec<f64> = (0..n).map(|i| (0..n).map(|j| m.get(i, j) * x[j]).sum()).collect();
        assert!(m.factor());
        m.solve(&mut b);
        for i in 0..n {
            assert!((b[i] - x[i]).abs() < 1e-12);
        }
        let mut bad = BandMatrix::new(2, 1);
        bad.add(0, 0, 1.0);
        bad.add(1, 0, 2.0);
        bad.add(1, 1, 1.0);
        assert!(!bad.factor(), "indefinite");
    }

    fn rect_mesh() -> Mesh {
        let r = Rect::new(0.0, 0.0, 200.0, 100.0);
        Mesh::build(&[shapes::rectangle(r).to_bezpath()], r, MeshOptions { resolution: 16, expand: 0.0 })
    }

    #[test]
    fn mesh_covers_shape_and_locates_points() {
        let m = rect_mesh();
        assert!(!m.tris.is_empty());
        for p in [Point::new(10.0, 10.0), Point::new(150.0, 70.0), Point::new(199.0, 99.0)] {
            let (t, w) = m.locate(p).unwrap();
            assert!(w.iter().all(|v| *v >= -1e-9), "{w:?}");
            assert!((m.map(&m.verts, p) - p).hypot() < 1e-9);
            let _ = t;
        }
        // An L-shaped outline drops the empty corner cells.
        let mut l = BezPath::new();
        l.move_to((0.0, 0.0));
        l.line_to((100.0, 0.0));
        l.line_to((100.0, 20.0));
        l.line_to((20.0, 20.0));
        l.line_to((20.0, 100.0));
        l.line_to((0.0, 100.0));
        l.close_path();
        let lm = Mesh::build(&[l], Rect::new(0.0, 0.0, 100.0, 100.0), MeshOptions { resolution: 10, expand: 0.0 });
        assert!(lm.tris.len() < m.tris.len() / 2, "{} tris", lm.tris.len());
        assert!(lm.cells.iter().filter(|c| c.is_some()).count() < 60);
    }

    #[test]
    fn pins_at_rest_leave_the_mesh_unchanged() {
        let m = rect_mesh();
        let pins: Vec<Pin> =
            [(20.0, 50.0), (180.0, 50.0), (100.0, 20.0)].iter().map(|&(x, y)| Pin::new(Point::new(x, y), Point::new(x, y))).collect();
        let d = deform(&m, &pins);
        let err = m.verts.iter().zip(&d).map(|(a, b)| a.distance(*b)).fold(0.0, f64::max);
        assert!(err < 1e-4, "{err}");
    }

    #[test]
    fn rigid_motion_is_reproduced_exactly() {
        let m = rect_mesh();
        let rot = |p: Point| {
            let (s, c) = 0.5f64.sin_cos();
            Point::new(p.x * c - p.y * s + 30.0, p.x * s + p.y * c - 10.0)
        };
        let pins: Vec<Pin> =
            [(20.0, 50.0), (180.0, 50.0), (100.0, 90.0)].iter().map(|&(x, y)| Pin::new(Point::new(x, y), rot(Point::new(x, y)))).collect();
        let d = deform(&m, &pins);
        let err = m.verts.iter().zip(&d).map(|(a, b)| rot(*a).distance(*b)).fold(0.0, f64::max);
        assert!(err < 1e-2, "{err}");
    }

    #[test]
    fn pins_are_satisfied_and_free_regions_stay_rigid() {
        let m = rect_mesh();
        // Hold the left end, lift the right end.
        let pins = vec![
            Pin::new(Point::new(10.0, 30.0), Point::new(10.0, 30.0)),
            Pin::new(Point::new(10.0, 70.0), Point::new(10.0, 70.0)),
            Pin::new(Point::new(190.0, 50.0), Point::new(180.0, -30.0)),
        ];
        let d = deform(&m, &pins);
        for p in &pins {
            let got = m.map(&d, p.rest);
            assert!(got.distance(p.target) < 0.05, "pin {:?} → {got:?}", p.target);
        }
        // Area is preserved (as-rigid-as-possible): total within a few percent, no flipped triangles.
        let (a0, a1): (f64, f64) =
            (0..m.tris.len()).map(|t| (m.tri_area(&m.verts, t), m.tri_area(&d, t))).fold((0.0, 0.0), |s, v| (s.0 + v.0, s.1 + v.1));
        assert!((a1 / a0 - 1.0).abs() < 0.05, "area ratio {}", a1 / a0);
        assert!((0..m.tris.len()).all(|t| m.tri_area(&d, t) * m.tri_area(&m.verts, t) > 0.0));
        // Each triangle stays nearly congruent: edge lengths change by < 25%.
        let worst = m
            .tris
            .iter()
            .flat_map(|t| (0..3).map(move |k| (t[k], t[(k + 1) % 3])))
            .map(|(i, j)| (d[i].distance(d[j]) / m.verts[i].distance(m.verts[j]) - 1.0).abs())
            .fold(0.0, f64::max);
        assert!(worst < 0.3, "edge stretch {worst}");
    }

    #[test]
    fn a_held_rotation_turns_the_art_around_its_pin() {
        let m = rect_mesh();
        let c = Point::new(100.0, 50.0);
        let rot = |p: Point| {
            let (s, co) = 0.4f64.sin_cos();
            let d = p - c;
            c + Vec2::new(d.x * co - d.y * s, d.x * s + d.y * co)
        };
        // One pin, held at 0.4 rad: the whole shape turns rigidly around it.
        let d = deform(&m, &[Pin { angle: Some(0.4), ..Pin::new(c, c) }]);
        let err = m.verts.iter().zip(&d).map(|(a, b)| rot(*a).distance(*b)).fold(0.0, f64::max);
        assert!(err < 0.05, "{err}");
        // A free pin alone leaves the shape where it is.
        let d = deform(&m, &[Pin::new(c, c)]);
        assert!(m.verts.iter().zip(&d).all(|(a, b)| a.distance(*b) < 1e-3));
        // Held ends: the turned end turns, the other end stays.
        let left = Point::new(10.0, 50.0);
        let pins = [Pin { angle: Some(0.5), ..Pin::new(left, left) }, Pin::new(Point::new(190.0, 50.0), Point::new(190.0, 50.0))];
        let d = deform(&m, &pins);
        let near = m.map(&d, Point::new(10.0, 62.0)) - left;
        assert!((near.atan2() - (Vec2::new(0.0, 12.0).atan2() + 0.5)).abs() < 0.1, "the left end turned: {near:?}");
        assert!(m.map(&d, Point::new(190.0, 80.0)).distance(Point::new(190.0, 80.0)) < 5.0);
    }

    #[test]
    fn default_pins_go_at_the_centre_and_the_end_of_each_limb() {
        // A long bar: the centre and both ends.
        let m = rect_mesh();
        let p = default_pins(&m);
        assert_eq!(p.len(), 3, "{p:?}");
        assert!(p[0].distance(Point::new(100.0, 50.0)) < 10.0, "{p:?}");
        let mut xs: Vec<f64> = p[1..].iter().map(|q| q.x).collect();
        xs.sort_by(f64::total_cmp);
        assert!(xs[0] < 25.0 && xs[1] > 175.0, "{p:?}");
        // A five-pointed star: the centre and its five tips.
        let star = vectorcraft_geom::shapes::star(Point::new(100.0, 100.0), 100.0, 35.0, 5, 0.0).to_bezpath();
        let sm = Mesh::build(std::slice::from_ref(&star), star.bounding_box(), MeshOptions { resolution: 30, expand: 0.0 });
        let p = default_pins(&sm);
        assert_eq!(p.len(), 6, "{p:?}");
        assert!(p[1..].iter().all(|q| q.distance(Point::new(100.0, 100.0)) > 55.0), "{p:?}");
        // A disc has no limbs: the centre, topped up to three.
        let disc = vectorcraft_geom::kurbo::Circle::new((50.0, 50.0), 50.0).to_path(0.1);
        let dm = Mesh::build(std::slice::from_ref(&disc), disc.bounding_box(), MeshOptions::default());
        assert_eq!(default_pins(&dm).len(), 3);
        // A point with a wide Expand stays a small grid.
        let dot = Mesh::build(&[], Rect::new(5.0, 5.0, 5.0, 5.0), MeshOptions { expand: 1000.0, ..Default::default() });
        assert!(dot.verts.len() < 30 * 30 && default_pins(&dot).len() == 3, "{}", dot.verts.len());
    }

    #[test]
    fn unmap_and_turn_read_a_warped_mesh_back() {
        let m = rect_mesh();
        let rot = |p: Point| {
            let (s, c) = 0.3f64.sin_cos();
            Point::new(p.x * c - p.y * s + 20.0, p.x * s + p.y * c)
        };
        let d: Vec<Point> = m.verts.iter().map(|v| rot(*v)).collect();
        let p = Point::new(60.0, 40.0);
        assert!(m.unmap(&d, rot(p)).unwrap().distance(p) < 1e-6);
        assert!(m.unmap(&d, Point::new(-500.0, 0.0)).is_none(), "off the mesh");
        assert!((m.turn_at(&d, p).unwrap() - 0.3).abs() < 1e-9);
        assert!(m.contains(p) && !m.contains(Point::new(300.0, 50.0)));
    }

    #[test]
    fn auto_pins_are_spread_inside_the_shape() {
        let m = rect_mesh();
        let p = auto_pins(&m, 3);
        assert_eq!(p.len(), 3);
        assert!(p.iter().all(|q| Rect::new(0.0, 0.0, 200.0, 100.0).contains(*q)));
        assert!(p[1].distance(p[2]) > 100.0);
    }
}
