//! Curve-preserving boolean operations built on `linesweeper`'s robust sweep-line topology.
//!
//! Inputs are converted to closed [`BezPath`]s (open subpaths are implicitly closed, as a filled
//! open path is painted in Illustrator). The sweep splits curves at intersections and y-extrema;
//! [`tidy_segments`] then re-joins pieces that came from the same smooth curve by refitting, so
//! results have roughly as few anchors as the inputs.

use std::collections::BTreeSet;

use kurbo::{BezPath, CubicBez, PathSeg, Point, Shape as _};
use linesweeper::topology::{ContourIdx, Contours, Topology, WindingNumber};
use vectorcraft_geom::{Anchor, FillRule, PathData, SubPath};

use crate::PathOpsError;
use crate::fit::{end_tangent, fit_single, is_straight, sample, start_tangent};

/// Default curve-refit tolerance (document points) used to merge split curve pieces back together.
/// Similar in spirit to Illustrator's Pathfinder "Precision" option (0.028 pt).
pub const DEFAULT_PRECISION: f64 = 0.01;

/// Binary boolean operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BoolOp {
    /// A ∪ B
    Union,
    /// A ∩ B
    Intersect,
    /// A − B
    Difference,
    /// A ⊕ B (symmetric difference)
    Xor,
}

impl BoolOp {
    /// Evaluate the operation on "inside A" / "inside B" flags.
    pub fn apply(self, a: bool, b: bool) -> bool {
        match self {
            BoolOp::Union => a || b,
            BoolOp::Intersect => a && b,
            BoolOp::Difference => a && !b,
            BoolOp::Xor => a != b,
        }
    }
}

pub(crate) fn inside(rule: FillRule, w: i32) -> bool {
    match rule {
        FillRule::NonZero => w != 0,
        FillRule::EvenOdd => w % 2 != 0,
    }
}

/// Winding numbers for an arbitrary number of input sets (tag = input index).
#[derive(Clone, Debug, Default)]
pub(crate) struct Multi(pub(crate) Vec<i32>);

impl Multi {
    fn get(&self, i: usize) -> i32 {
        self.0.get(i).copied().unwrap_or(0)
    }
}

impl PartialEq for Multi {
    fn eq(&self, other: &Self) -> bool {
        let n = self.0.len().max(other.0.len());
        (0..n).all(|i| self.get(i) == other.get(i))
    }
}
impl Eq for Multi {}

impl std::ops::Add for Multi {
    type Output = Multi;
    fn add(mut self, rhs: Self) -> Multi {
        self += rhs;
        self
    }
}

impl std::ops::AddAssign for Multi {
    fn add_assign(&mut self, rhs: Self) {
        if self.0.len() < rhs.0.len() {
            self.0.resize(rhs.0.len(), 0);
        }
        for (a, b) in self.0.iter_mut().zip(rhs.0) {
            *a += b;
        }
    }
}

impl WindingNumber for Multi {
    type Tag = usize;
    fn single(tag: usize, positive: bool) -> Self {
        let mut v = vec![0; tag + 1];
        v[tag] = if positive { 1 } else { -1 };
        Multi(v)
    }
    fn of_tag(&self, tag: usize) -> Self {
        let mut v = vec![0; tag + 1];
        v[tag] = self.get(tag);
        Multi(v)
    }
}

/// Convert a (filled) path to a closed BezPath: every subpath with ≥ 2 anchors is closed.
pub(crate) fn fill_bezpath(p: &PathData) -> BezPath {
    let mut bp = BezPath::new();
    for sp in &p.subpaths {
        if sp.anchors.len() < 2 {
            continue;
        }
        if sp.closed {
            sp.to_bezpath_into(&mut bp);
        } else {
            let mut c = sp.clone();
            c.closed = true;
            c.to_bezpath_into(&mut bp);
        }
    }
    bp
}

/// Sweep tolerance for a set of paths (same scaling rule as linesweeper's own `binary_op`).
pub(crate) fn eps_for<'a>(paths: impl IntoIterator<Item = &'a BezPath>) -> Result<f64, PathOpsError> {
    let mut m: f64 = 0.0;
    for p in paths {
        for el in p.elements() {
            for pt in el_points(el) {
                if !pt.x.is_finite() || !pt.y.is_finite() {
                    return Err(PathOpsError::NonFinite);
                }
                m = m.max(pt.x.abs()).max(pt.y.abs());
            }
        }
    }
    Ok((m * f64::EPSILON * 64.0).max(1e-6))
}

fn el_points(el: &kurbo::PathEl) -> impl Iterator<Item = Point> {
    use kurbo::PathEl::*;
    let v: [Option<Point>; 3] = match *el {
        MoveTo(p) | LineTo(p) => [Some(p), None, None],
        QuadTo(a, b) => [Some(a), Some(b), None],
        CurveTo(a, b, c) => [Some(a), Some(b), Some(c)],
        ClosePath => [None, None, None],
    };
    v.into_iter().flatten()
}

/// Make edges that rise by less than the sweep tolerance `eps` exactly horizontal.
///
/// The sweep orders a curve against another segment over a y-span shorter than `eps` by bounding
/// boxes alone, so an almost-horizontal edge (typically a boolean result's edge whose end, an
/// intersection point, is a rounding error off the input's horizontal) can be mis-ordered against
/// a curve it passes and the output loses a corner; exact horizontals are handled exactly. So
/// on-curve y coordinates within `eps` of each other snap to one value (the lowest of each
/// cluster, keeping distinct values at least `eps` apart), handles move with their anchors, and a
/// handle within `eps` of its anchor's height is levelled with it (no y-extremum is left in a
/// sub-`eps` sliver of a curve). Anchors move by less than `eps` and handles by less than `2·eps`,
/// the order of the sweep's own tolerance.
pub(crate) fn snap_horizontals(paths: &mut [&mut BezPath], eps: f64) {
    use kurbo::PathEl::*;
    let mut ys: Vec<f64> = paths.iter().flat_map(|p| p.elements().iter().filter_map(kurbo::PathEl::end_point)).map(|p| p.y).collect();
    ys.sort_by(f64::total_cmp);
    ys.dedup();
    let mut start = f64::NEG_INFINITY;
    let levels: Vec<f64> = ys
        .iter()
        .map(|&y| {
            if y - start >= eps {
                start = y;
            }
            start
        })
        .collect();
    let snap = |p: &mut Point| {
        let y = levels[ys.partition_point(|&v| v < p.y)];
        let moved = y - p.y;
        p.y = y;
        moved
    };
    // Shift a handle with its anchor, then level it with the anchor when within `eps`.
    let follow = |h: &mut Point, anchor: Point, moved: f64| {
        h.y += moved;
        if (h.y - anchor.y).abs() < eps {
            h.y = anchor.y;
        }
    };
    for p in paths.iter_mut() {
        // The current point (snapped) and how far snapping moved it; the subpath's start.
        let (mut cur, mut moved, mut first) = (Point::ZERO, 0.0, Point::ZERO);
        for el in p.elements_mut() {
            match el {
                MoveTo(q) => {
                    moved = snap(q);
                    (cur, first) = (*q, *q);
                }
                LineTo(q) => {
                    moved = snap(q);
                    cur = *q;
                }
                QuadTo(c, q) => {
                    let m = snap(q);
                    follow(c, cur, moved);
                    follow(c, *q, 0.0);
                    (cur, moved) = (*q, m);
                }
                CurveTo(c1, c2, q) => {
                    let m = snap(q);
                    follow(c1, cur, moved);
                    follow(c2, *q, m);
                    (cur, moved) = (*q, m);
                }
                ClosePath => {
                    // A later segment without a MoveTo starts from the subpath's (snapped) start.
                    (cur, moved) = (first, 0.0);
                }
            }
        }
    }
}

/// An arrangement (planar subdivision) of N filled paths.
pub(crate) struct Arrangement {
    pub(crate) top: Topology<Multi>,
    pub(crate) rules: Vec<FillRule>,
    /// Output cleanup that keeps the inputs' anchors.
    pub(crate) tidy: Tidy,
}

/// Output cleanup settings: refit `precision`, and anchor positions that must survive (the
/// inputs' own anchors — only joints the sweep introduced are merged away).
#[derive(Clone, Debug, Default)]
pub(crate) struct Tidy {
    pub(crate) precision: f64,
    keep: Vec<Point>,
    /// Input curves (with inflated bounds) used to rebuild split pieces exactly.
    curves: Vec<(kurbo::Rect, CubicBez)>,
}

impl Tidy {
    /// Merge any smooth joint.
    pub(crate) fn free(precision: f64) -> Self {
        Self { precision, keep: Vec::new(), curves: Vec::new() }
    }
    /// Keep every on-curve point of `paths`.
    pub(crate) fn keeping<'a>(precision: f64, paths: impl IntoIterator<Item = &'a BezPath>) -> Self {
        let mut keep = Vec::new();
        let mut curves = Vec::new();
        for p in paths {
            for s in p.segments() {
                let seg = to_seg(s);
                keep.push(seg.c.p0);
                if !seg.line {
                    let r = seg.c.bounding_box().inflate(1e-6, 1e-6);
                    curves.push((r, seg.c));
                }
            }
        }
        keep.sort_by(|a, b| a.x.total_cmp(&b.x));
        Self { precision, keep, curves }
    }
    fn is_kept(&self, p: Point) -> bool {
        let tol = 1e-6 * (1.0 + p.x.abs().max(p.y.abs()));
        let lo = self.keep.partition_point(|q| q.x < p.x - tol);
        self.keep[lo..].iter().take_while(|q| q.x <= p.x + tol).any(|q| (q.y - p.y).abs() <= tol)
    }
}

impl Arrangement {
    pub(crate) fn new(mut paths: Vec<(BezPath, FillRule)>) -> Result<Self, PathOpsError> {
        let eps = eps_for(paths.iter().map(|p| &p.0))?;
        snap_horizontals(&mut paths.iter_mut().map(|p| &mut p.0).collect::<Vec<_>>(), eps);
        // The sweep can panic on rare near-degenerate input; that must never take the app down,
        // so it becomes an error (callers fall back to leaving the art unchanged).
        let sweep = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            Topology::<Multi>::from_paths(paths.iter().enumerate().map(|(i, (p, _))| (p, i)), eps)
        }));
        let top = sweep.map_err(|_| PathOpsError::Degenerate)?.map_err(|_| PathOpsError::OpenPath)?;
        let tidy = Tidy::keeping(DEFAULT_PRECISION, paths.iter().map(|p| &p.0));
        Ok(Self { top, rules: paths.iter().map(|p| p.1).collect(), tidy })
    }

    pub(crate) fn mask(&self, w: &Multi) -> Vec<bool> {
        self.rules.iter().enumerate().map(|(i, &r)| inside(r, w.get(i))).collect()
    }

    pub(crate) fn contours(&self, pred: impl Fn(&[bool]) -> bool) -> Contours {
        self.top.contours(|w| pred(&self.mask(w)))
    }

    /// All distinct non-empty coverage masks that occur next to some edge, sorted.
    pub(crate) fn distinct_masks(&self) -> Vec<Vec<bool>> {
        let mut set = BTreeSet::new();
        for s in self.top.segment_indices() {
            let h = s.first_half();
            for w in [self.top.winding_clockwise(h), self.top.winding_counter_clockwise(h)] {
                let m = self.mask(w);
                if m.iter().any(|&b| b) {
                    set.insert(m);
                }
            }
        }
        set.into_iter().collect()
    }

    /// Vertices where three or more edges meet (true intersections / touch points).
    pub(crate) fn junctions(&self) -> Vec<Point> {
        let mut count: std::collections::HashMap<(u64, u64), (usize, Point)> = std::collections::HashMap::new();
        for s in self.top.segment_indices() {
            for h in [s.first_half(), s.second_half()] {
                let p = self.top.point(h).to_kurbo();
                count.entry((p.x.to_bits(), p.y.to_bits())).or_insert((0, p)).0 += 1;
            }
        }
        let mut v: Vec<Point> = count.into_values().filter(|(c, _)| *c >= 3).map(|(_, p)| p).collect();
        v.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
        v
    }
}

/// Convert selected contours into a tidy compound path.
pub(crate) fn contours_to_path(c: &Contours, idx: impl IntoIterator<Item = ContourIdx>, tidy: &Tidy) -> PathData {
    let mut subpaths = Vec::new();
    for i in idx {
        if is_sliver(&c[i].path, tidy.precision) {
            continue;
        }
        if let Some(sp) = tidy_bezpath(&c[i].path, tidy) {
            subpaths.push(sp);
        }
    }
    PathData::new(subpaths)
}

pub(crate) fn all_contours_to_path(c: &Contours, tidy: &Tidy) -> PathData {
    contours_to_path(c, (0..c.contours().count()).map(ContourIdx), tidy)
}

/// Contours whose mean width (2·area / perimeter) is below `precision` are numerical debris
/// (e.g. the lens left between two coincident curves) and are dropped, like Illustrator's
/// Pathfinder precision setting does.
pub(crate) fn is_sliver(bp: &BezPath, precision: f64) -> bool {
    let a = bp.area().abs();
    let per = bp.perimeter(1e-6);
    per <= 0.0 || 2.0 * a / per < precision
}

/// One segment of a path: a cubic plus whether it is a straight line.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Seg {
    pub(crate) c: CubicBez,
    pub(crate) line: bool,
}

impl Seg {
    pub(crate) fn line(a: Point, b: Point) -> Self {
        Self { c: CubicBez::new(a, a, b, b), line: true }
    }
}

pub(crate) fn to_seg(s: PathSeg) -> Seg {
    match s {
        PathSeg::Line(l) => Seg::line(l.p0, l.p1),
        PathSeg::Quad(q) => {
            let c = q.raise();
            Seg { c, line: is_straight(&c, 1e-9) }
        }
        PathSeg::Cubic(c) => Seg { c, line: is_straight(&c, 1e-9 * (1.0 + c.p0.distance(c.p3))) },
    }
}

fn tidy_bezpath(bp: &BezPath, tidy: &Tidy) -> Option<SubPath> {
    let segs: Vec<Seg> = bp.segments().map(to_seg).collect();
    let segs = tidy_segments(segs, true, tidy);
    segs_to_subpath(&segs, true)
}

/// Build a subpath from a chain of segments.
pub(crate) fn segs_to_subpath(segs: &[Seg], closed: bool) -> Option<SubPath> {
    if segs.is_empty() {
        return None;
    }
    let n = segs.len();
    let mut anchors = Vec::with_capacity(n + 1);
    for i in 0..n {
        let s = segs[i];
        let p = s.c.p0;
        let h_out = if s.line { p } else { s.c.p1 };
        let h_in = if i == 0 && !closed {
            p
        } else {
            let prev = segs[(i + n - 1) % n];
            if prev.line { p } else { prev.c.p2 }
        };
        anchors.push(Anchor::with_handles(p, h_in, h_out));
    }
    if !closed {
        let last = segs[n - 1];
        let p = last.c.p3;
        let h_in = if last.line { p } else { last.c.p2 };
        anchors.push(Anchor::with_handles(p, h_in, p));
    }
    if closed && anchors.len() < 2 {
        return None;
    }
    Some(SubPath::new(anchors, closed))
}

/// Merge segments that were split from one curve (or collinear lines) back together, dropping
/// degenerate pieces. `closed` segments form a loop.
pub(crate) fn tidy_segments(segs: Vec<Seg>, closed: bool, tidy: &Tidy) -> Vec<Seg> {
    let scale = segs.iter().map(|s| s.c.p0.distance(s.c.p3)).fold(0.0, f64::max);
    let tiny = (scale * 1e-9).max(1e-9);
    let mut segs: Vec<Seg> = segs
        .into_iter()
        .filter(|s| {
            let r = kurbo::Rect::from_points(s.c.p0, s.c.p3).union_pt(s.c.p1).union_pt(s.c.p2);
            r.width() + r.height() > tiny
        })
        .collect();
    let n = segs.len();
    if n < 2 {
        return segs;
    }
    // Joint i sits at the start of segment i.
    let mergeable = |a: &Seg, b: &Seg| -> bool {
        if a.line != b.line || tidy.is_kept(b.c.p0) {
            return false;
        }
        let ta = end_tangent(&a.c);
        let tb = start_tangent(&b.c);
        ta.dot(tb) > 0.9998 && ta.cross(tb).abs() < 0.02
    };
    if closed {
        // Rotate so we start at a real corner (if there is one).
        if let Some(k) = (0..n).find(|&i| !mergeable(&segs[(i + n - 1) % n], &segs[i])) {
            segs.rotate_left(k);
        }
    }
    let mut out: Vec<Seg> = Vec::with_capacity(n);
    let mut i = 0;
    while i < n {
        let mut cur = segs[i];
        let mut j = i + 1;
        while j < n && mergeable(&segs[j - 1], &segs[j]) {
            match try_merge(&segs[i..=j], tidy) {
                Some(m) => {
                    cur = m;
                    j += 1;
                }
                None => break,
            }
        }
        out.push(cur);
        i = j;
    }
    out
}

fn try_merge(run: &[Seg], tidy: &Tidy) -> Option<Seg> {
    let precision = tidy.precision;
    let first = run[0].c;
    let last = run[run.len() - 1].c;
    if run[0].line {
        let chord = last.p3 - first.p0;
        let len = chord.hypot();
        if len < 1e-12 {
            return None;
        }
        let ok = run.iter().all(|s| ((s.c.p3 - first.p0).cross(chord) / len).abs() <= precision * 0.5);
        return ok.then(|| Seg::line(first.p0, last.p3));
    }
    if let Some(c) = exact_merge(run, tidy) {
        return Some(Seg { c, line: false });
    }
    let mut pts = Vec::with_capacity(run.len() * 12 + 1);
    for (k, s) in run.iter().enumerate() {
        sample(&s.c, 12, &mut pts, k == 0);
    }
    let (c, err, _) = fit_single(&pts, start_tangent(&first), end_tangent(&last));
    if err > precision {
        return None;
    }
    Some(Seg { c, line: false })
}

/// If every piece of `run` lies on one input cubic, return the exact sub-curve spanning the run.
fn exact_merge(run: &[Seg], tidy: &Tidy) -> Option<CubicBez> {
    use kurbo::{ParamCurve, ParamCurveNearest};
    let a = run[0].c.p0;
    let b = run[run.len() - 1].c.p3;
    let scale = 1.0 + a.x.abs().max(a.y.abs()).max(b.x.abs()).max(b.y.abs());
    let tol = 1e-6 * scale;
    for (r, c) in &tidy.curves {
        if !r.contains(a) || !r.contains(b) {
            continue;
        }
        let na = c.nearest(a, 1e-12);
        let nb = c.nearest(b, 1e-12);
        if na.distance_sq.sqrt() > tol || nb.distance_sq.sqrt() > tol {
            continue;
        }
        let on_curve = run.iter().all(|s| c.nearest(s.c.eval(0.5), 1e-12).distance_sq.sqrt() <= tol * 10.0);
        if !on_curve || (na.t - nb.t).abs() < 1e-12 {
            continue;
        }
        let sub = if na.t < nb.t {
            c.subsegment(na.t..nb.t)
        } else {
            let r = c.subsegment(nb.t..na.t);
            CubicBez::new(r.p3, r.p2, r.p1, r.p0)
        };
        return Some(CubicBez::new(a, sub.p1, sub.p2, b));
    }
    None
}

/// Resolve self-intersections / overlaps of a single filled path into simple, consistently
/// oriented contours (outer contours and holes).
pub fn normalize(path: &PathData, rule: FillRule) -> PathData {
    try_normalize(path, rule).unwrap_or_default()
}

/// Fallible [`normalize`].
pub fn try_normalize(path: &PathData, rule: FillRule) -> Result<PathData, PathOpsError> {
    let bp = fill_bezpath(path);
    let c = normalize_bez(&bp, rule)?;
    Ok(all_contours_to_path(&c, &Tidy::keeping(DEFAULT_PRECISION, [&bp])))
}

pub(crate) fn normalize_bez(bp: &BezPath, rule: FillRule) -> Result<Contours, PathOpsError> {
    let eps = eps_for([bp])?;
    let mut bp = bp.clone();
    snap_horizontals(&mut [&mut bp], eps);
    let top = Topology::<i32>::from_path(&bp, eps).map_err(|_| PathOpsError::OpenPath)?;
    Ok(top.contours(|w| inside(rule, *w)))
}

/// Boolean operation between two filled paths. Returns an empty path on invalid (non-finite) input.
pub fn boolean(a: &PathData, a_rule: FillRule, b: &PathData, b_rule: FillRule, op: BoolOp) -> PathData {
    try_boolean(a, a_rule, b, b_rule, op, DEFAULT_PRECISION).unwrap_or_default()
}

/// Boolean operation with an explicit refit `precision`; errors on non-finite input.
pub fn try_boolean(a: &PathData, a_rule: FillRule, b: &PathData, b_rule: FillRule, op: BoolOp, precision: f64) -> Result<PathData, PathOpsError> {
    let arr = Arrangement::new(vec![(fill_bezpath(a), a_rule), (fill_bezpath(b), b_rule)])?;
    let c = arr.contours(|m| op.apply(m[0], m[1]));
    let tidy = Tidy { precision, ..arr.tidy };
    Ok(all_contours_to_path(&c, &tidy))
}

/// N-ary boolean: the area where `pred(inside_flags)` holds, `inside_flags[i]` telling whether a
/// point is inside `paths[i]` under its fill rule.
pub fn boolean_n(paths: &[(&PathData, FillRule)], pred: impl Fn(&[bool]) -> bool) -> PathData {
    let bps: Vec<(BezPath, FillRule)> = paths.iter().map(|(p, r)| (fill_bezpath(p), *r)).collect();
    match Arrangement::new(bps) {
        Ok(arr) => all_contours_to_path(&arr.contours(pred), &arr.tidy),
        Err(_) => PathData::default(),
    }
}

/// Union of many filled paths. Each input is first normalised on its own, then all are swept
/// together with a single integer winding number, which keeps this fast for thousands of shapes.
pub fn unite_all(paths: &[(&PathData, FillRule)]) -> PathData {
    let mut all = BezPath::new();
    for (p, r) in paths {
        let bp = fill_bezpath(p);
        if bp.elements().is_empty() {
            continue;
        }
        let Ok(c) = normalize_bez(&bp, *r) else { continue };
        for ct in c.contours() {
            all.extend(ct.path.iter());
        }
    }
    if all.elements().is_empty() {
        return PathData::default();
    }
    let tidy = Tidy::keeping(DEFAULT_PRECISION, paths.iter().map(|(p, _)| fill_bezpath(p)).collect::<Vec<_>>().iter());
    match normalize_bez(&all, FillRule::NonZero) {
        Ok(c) => all_contours_to_path(&c, &tidy),
        Err(_) => PathData::default(),
    }
}

/// Unsigned filled area of a path under a fill rule (normalises first, so it is exact for
/// self-overlapping input).
pub fn area(path: &PathData, rule: FillRule) -> f64 {
    let bp = fill_bezpath(path);
    match normalize_bez(&bp, rule) {
        Ok(c) => c.contours().map(|ct| ct.path.area()).sum::<f64>().abs(),
        Err(_) => 0.0,
    }
}
