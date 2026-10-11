//! Create Strokes: the parts of a colour layer no wider than the Stroke width, traced as stroked
//! centre lines instead of filled outlines.
//!
//! 1. **Distance**: a chamfer (3-4) distance transform gives each pixel its distance to the nearest
//!    pixel outside the layer.
//! 2. **Lines**: an 8-connected component whose widest point (twice its largest distance, less
//!    one) is within the stroke width is a line; wider ones stay filled areas.
//! 3. **Skeleton**: the line is thinned to one pixel (Zhang–Suen, applied pixel by pixel so it
//!    keeps its topology and two-pixel-wide lines don't vanish), then staircase corners that
//!    don't connect anything are dropped.
//! 4. **Graph**: runs of skeleton pixels between ends and junctions (touching junction pixels
//!    merged into one point) become polylines through the pixel centres; rings without ends
//!    become closed ones. Short spurs that thinning leaves at junctions are pruned and the runs
//!    on either side of a junction left with two of them are joined.
//! 5. **Curves**: each polyline is fitted like an outline ([`fit::fit_polyline`]); the stroke
//!    width is the line's mean width (its pixel count over its length).

use std::collections::HashSet;

use vectorcraft_geom::{PathData, Point};

use crate::fit::{self, FitOptions};

/// One line of a colour layer: its centre lines and the width to stroke them with.
pub(crate) struct Line {
    pub path: PathData,
    /// Stroke width in pixels.
    pub width: f64,
    pub pixels: usize,
}

/// The lines of colour `label` in `labels` (`w` × `h`, row-major) no wider than `max_width`
/// pixels, and which pixels they cover (to leave out of the filled areas).
pub(crate) fn lines(labels: &[u16], label: u16, w: usize, h: usize, max_width: f64, o: &FitOptions) -> (Vec<bool>, Vec<Line>) {
    let mut thin = vec![false; labels.len()];
    let mut out = vec![];
    if w == 0 || h == 0 || labels.len() != w * h {
        return (thin, out);
    }
    let mask = |i: usize| labels.get(i) == Some(&label);
    let dt = distance(&mask, w, h);
    let mut seen = vec![false; w * h];
    let mut stack = vec![];
    for start in 0..w * h {
        if !mask(start) || seen[start] {
            continue;
        }
        let mut comp = vec![];
        seen[start] = true;
        stack.push(start);
        while let Some(i) = stack.pop() {
            comp.push(i);
            for j in neighbours(i, w, h) {
                if mask(j) && !seen[j] {
                    seen[j] = true;
                    stack.push(j);
                }
            }
        }
        let widest = 2.0 * f64::from(comp.iter().map(|&i| dt[i]).max().unwrap_or(0)) / 3.0 - 1.0;
        if widest > max_width {
            continue;
        }
        let Some(line) = trace_line(&comp, w, max_width, o) else { continue };
        for &i in &comp {
            thin[i] = true;
        }
        out.push(line);
    }
    (thin, out)
}

/// The 8 neighbours of pixel `i` inside the `w` × `h` image.
fn neighbours(i: usize, w: usize, h: usize) -> impl Iterator<Item = usize> {
    let (x, y) = ((i % w) as i64, (i / w) as i64);
    const AROUND: [(i64, i64); 8] = [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];
    AROUND.into_iter().filter_map(move |(dx, dy)| {
        let (nx, ny) = (x + dx, y + dy);
        (nx >= 0 && ny >= 0 && nx < w as i64 && ny < h as i64).then(|| ny as usize * w + nx as usize)
    })
}

/// Chamfer 3-4 distance (3 per pixel step, saturating) of each pixel in `mask` to the nearest
/// pixel outside it; outside the image counts as outside.
fn distance(mask: &impl Fn(usize) -> bool, w: usize, h: usize) -> Vec<u16> {
    let mut d = vec![0u16; w * h];
    let at = |d: &[u16], x: i64, y: i64| {
        if x < 0 || y < 0 || x >= w as i64 || y >= h as i64 { 0 } else { d.get(y as usize * w + x as usize).copied().unwrap_or(0) }
    };
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if mask(i) {
                let (x, y) = (x as i64, y as i64);
                let steps = [(x - 1, y, 3), (x - 1, y - 1, 4), (x, y - 1, 3), (x + 1, y - 1, 4)];
                d[i] = steps.iter().map(|&(nx, ny, c)| at(&d, nx, ny).saturating_add(c)).min().unwrap_or(0);
            }
        }
    }
    for y in (0..h).rev() {
        for x in (0..w).rev() {
            let i = y * w + x;
            if mask(i) {
                let (xi, yi) = (x as i64, y as i64);
                let steps = [(xi + 1, yi, 3), (xi + 1, yi + 1, 4), (xi, yi + 1, 3), (xi - 1, yi + 1, 4)];
                let v = steps.iter().map(|&(nx, ny, c)| at(&d, nx, ny).saturating_add(c)).min().unwrap_or(0);
                d[i] = d[i].min(v);
            }
        }
    }
    d
}

/// A grid around one line: its pixels with a one-pixel empty border.
struct Grid {
    on: Vec<bool>,
    w: usize,
    h: usize,
    /// Image position of grid pixel (0, 0).
    x0: i64,
    y0: i64,
}

impl Grid {
    /// The 8 neighbours (N, NE, E, SE, S, SW, W, NW) of interior pixel `i`.
    fn ring(&self, i: usize) -> [bool; 8] {
        let w = self.w;
        let g = |j: usize| self.on.get(j).copied().unwrap_or(false);
        [g(i - w), g(i - w + 1), g(i + 1), g(i + w + 1), g(i + w), g(i + w - 1), g(i - 1), g(i - w - 1)]
    }
    fn interior(&self) -> impl Iterator<Item = usize> + use<> {
        let (w, h) = (self.w, self.h);
        (1..h.saturating_sub(1)).flat_map(move |y| (1..w.saturating_sub(1)).map(move |x| y * w + x))
    }
    fn neighbours(&self, i: usize) -> impl Iterator<Item = usize> + '_ {
        let w = self.w;
        [i - w, i - w + 1, i + 1, i + w + 1, i + w, i + w - 1, i - 1, i - w - 1].into_iter().filter(|&j| self.on.get(j).copied().unwrap_or(false))
    }
    fn degree(&self, i: usize) -> usize {
        self.neighbours(i).count()
    }
    /// The centre of pixel `i` in image coordinates.
    fn centre(&self, i: usize) -> Point {
        Point::new((self.x0 + (i % self.w) as i64) as f64 + 0.5, (self.y0 + (i / self.w) as i64) as f64 + 0.5)
    }
}

/// Zhang–Suen thinning. Each sub-iteration picks the pixels to remove from the same state, so the
/// skeleton stays centred; they are then removed one by one, each only if it still doesn't
/// disconnect anything, so a two-pixel-wide line thins to one pixel instead of vanishing.
fn thin(g: &mut Grid) {
    let removable = |p: &[bool; 8]| {
        let b = p.iter().filter(|v| **v).count();
        let a = (0..8).filter(|&k| !p[k] && p[(k + 1) % 8]).count();
        (2..=6).contains(&b) && a == 1
    };
    let mut picked = vec![];
    loop {
        let mut changed = false;
        for step in 0..2 {
            picked.clear();
            for i in g.interior() {
                if !g.on[i] {
                    continue;
                }
                let p = g.ring(i);
                let (n, e, s, w) = (p[0], p[2], p[4], p[6]);
                let side = if step == 0 { !(e && s && (n || w)) } else { !(n && w && (e || s)) };
                if side && removable(&p) {
                    picked.push(i);
                }
            }
            for &i in &picked {
                if removable(&g.ring(i)) {
                    g.on[i] = false;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    // Staircase corners: a pixel whose neighbours stay connected without it (Yokoi's
    // 8-connectivity number 1) and that doesn't end the line.
    loop {
        let mut changed = false;
        for i in g.interior() {
            if !g.on[i] || g.degree(i) < 2 {
                continue;
            }
            let p = g.ring(i);
            // x1 = E, x2 = NE, x3 = N, x4 = NW, x5 = W, x6 = SW, x7 = S, x8 = SE.
            let x = [p[2], p[1], p[0], p[7], p[6], p[5], p[4], p[3]];
            let off = |k: usize| !x[k % 8];
            let c8: usize = [0, 2, 4, 6].iter().map(|&k| usize::from(off(k)) - usize::from(off(k) && off(k + 1) && off(k + 2))).sum();
            if c8 == 1 {
                g.on[i] = false;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
}

/// A run of skeleton pixels between two nodes (`None`: none, a ring).
#[derive(Clone)]
struct Run {
    pts: Vec<Point>,
    ends: [Option<usize>; 2],
    closed: bool,
}

impl Run {
    fn length(&self) -> f64 {
        self.pts.windows(2).map(|p| p[0].distance(p[1])).sum::<f64>()
            + if self.closed { self.pts.first().zip(self.pts.last()).map_or(0.0, |(a, b)| a.distance(*b)) } else { 0.0 }
    }
}

/// The centre lines of one line component (pixel indices of the `w`-wide image).
fn trace_line(comp: &[usize], w: usize, max_width: f64, o: &FitOptions) -> Option<Line> {
    let xs = comp.iter().map(|&i| i % w);
    let ys = comp.iter().map(|&i| i / w);
    let (x0, x1, y0, y1) = (xs.clone().min()?, xs.max()?, ys.clone().min()?, ys.max()?);
    let (gw, gh) = (x1 - x0 + 3, y1 - y0 + 3);
    let mut g = Grid { on: vec![false; gw * gh], w: gw, h: gh, x0: x0 as i64 - 1, y0: y0 as i64 - 1 };
    for &i in comp {
        g.on[(i / w - y0 + 1) * gw + (i % w - x0 + 1)] = true;
    }
    thin(&mut g);
    let skeleton: Vec<usize> = g.interior().filter(|&i| g.on[i]).collect();
    if skeleton.len() < 2 {
        return None;
    }
    // Nodes: line ends, and junctions with the junction pixels touching them.
    let mut node = vec![usize::MAX; g.on.len()];
    let mut centres: Vec<Point> = vec![];
    for &i in &skeleton {
        let d = g.degree(i);
        if d == 2 || node[i] != usize::MAX {
            continue;
        }
        let id = centres.len();
        let mut members = vec![i];
        node[i] = id;
        if d > 2 {
            let mut k = 0;
            while let Some(&p) = members.get(k) {
                k += 1;
                for q in g.neighbours(p).collect::<Vec<_>>() {
                    if node[q] == usize::MAX && g.degree(q) > 2 {
                        node[q] = id;
                        members.push(q);
                    }
                }
            }
        }
        let sum = members.iter().fold(Point::ZERO, |s, &m| s + g.centre(m).to_vec2());
        centres.push(Point::new(sum.x / members.len() as f64, sum.y / members.len() as f64));
    }
    // Runs from each node, then the rings left.
    let mut used: HashSet<(usize, usize)> = HashSet::new();
    let edge = |a: usize, b: usize| (a.min(b), a.max(b));
    let mut runs: Vec<Run> = vec![];
    for &p in skeleton.iter().filter(|&&i| node[i] != usize::MAX) {
        for q in g.neighbours(p).collect::<Vec<_>>() {
            if node[q] == node[p] || !used.insert(edge(p, q)) {
                continue;
            }
            let mut pts = vec![centres[node[p]], g.centre(q)];
            let (mut prev, mut cur) = (p, q);
            while node[cur] == usize::MAX {
                let Some(next) = g.neighbours(cur).find(|&n| n != prev && !used.contains(&edge(cur, n))) else { break };
                used.insert(edge(cur, next));
                pts.push(g.centre(next));
                (prev, cur) = (cur, next);
            }
            let end = (node[cur] != usize::MAX).then_some(node[cur]);
            if let (Some(e), Some(last)) = (end, pts.last_mut()) {
                *last = centres[e];
            }
            runs.push(Run { pts, ends: [Some(node[p]), end], closed: false });
        }
    }
    for &s in &skeleton {
        if node[s] != usize::MAX || g.neighbours(s).any(|n| used.contains(&edge(s, n))) {
            continue;
        }
        let mut pts = vec![g.centre(s)];
        let (mut prev, mut cur) = (usize::MAX, s);
        let mut closed = false;
        while let Some(next) = g.neighbours(cur).find(|&n| n != prev && !used.contains(&edge(cur, n))) {
            used.insert(edge(cur, next));
            if next == s {
                closed = true;
                break;
            }
            pts.push(g.centre(next));
            (prev, cur) = (cur, next);
        }
        runs.push(Run { pts, ends: [None, None], closed });
    }
    // The line's mean width: its pixels over its length (the skeleton stops about half a width
    // short of each end).
    let width = |runs: &[Run]| {
        let length: f64 = runs.iter().map(Run::length).sum();
        let px = comp.len() as f64;
        ((-length + (length * length + 4.0 * px).sqrt()) / 2.0).clamp(1.0, max_width.max(1.0))
    };
    let spur = width(&runs);
    let runs = join(prune(runs, centres.len(), spur), centres.len());
    let width = width(&runs);
    let subs: Vec<_> = runs.iter().filter_map(|r| fit::fit_polyline(&r.pts, r.closed, o)).collect();
    if subs.is_empty() {
        return None;
    }
    Some(Line { path: PathData::new(subs), width, pixels: comp.len() })
}

/// The runs without the spurs thinning leaves: runs shorter than the line's `width` from a line end
/// to a junction that keeps at least two other runs.
fn prune(runs: Vec<Run>, nodes: usize, width: f64) -> Vec<Run> {
    let mut degree = vec![0usize; nodes];
    for e in runs.iter().flat_map(|r| r.ends.iter().flatten()) {
        degree[*e] += 1;
    }
    let mut order: Vec<usize> = (0..runs.len()).collect();
    order.sort_by(|a, b| runs[*a].length().total_cmp(&runs[*b].length()));
    let mut keep = vec![true; runs.len()];
    for i in order {
        let r = &runs[i];
        let [Some(a), Some(b)] = r.ends else { continue };
        let (tip, fork) = if degree[a] == 1 { (a, b) } else { (b, a) };
        if degree[tip] == 1 && degree[fork] >= 3 && r.length() < width.max(2.0) {
            keep[i] = false;
            degree[tip] -= 1;
            degree[fork] -= 1;
        }
    }
    runs.into_iter().zip(keep).filter_map(|(r, k)| k.then_some(r)).collect()
}

/// The runs joined through every node exactly two of them meet at (a run meeting itself closes).
fn join(runs: Vec<Run>, nodes: usize) -> Vec<Run> {
    let mut runs: Vec<Option<Run>> = runs.into_iter().map(Some).collect();
    for n in 0..nodes {
        let at: Vec<(usize, usize)> = runs
            .iter()
            .enumerate()
            .flat_map(|(i, r)| r.iter().flat_map(move |r| (0..2).filter(move |&k| r.ends[k] == Some(n)).map(move |k| (i, k))))
            .collect();
        let [(i, ki), (j, kj)] = at[..] else { continue };
        if i == j {
            if let Some(r) = runs[i].as_mut() {
                r.pts.pop();
                r.ends = [None, None];
                r.closed = r.pts.len() >= 3;
            }
            continue;
        }
        let (Some(mut a), Some(mut b)) = (runs[i].take(), runs[j].take()) else { continue };
        // a ends at n, b starts at n.
        if ki == 0 {
            a.pts.reverse();
            a.ends.reverse();
        }
        if kj == 1 {
            b.pts.reverse();
            b.ends.reverse();
        }
        a.pts.extend(b.pts.into_iter().skip(1));
        a.ends[1] = b.ends[1];
        runs[i] = Some(a);
    }
    runs.into_iter().flatten().collect()
}
