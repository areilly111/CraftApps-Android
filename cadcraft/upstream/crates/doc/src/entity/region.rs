//! Regions (REGION): a closed planar area bounded by loops of lines and arcs. Outer loops add
//! area, island loops (holes) take it away; loops don't cross.

use cadcraft_geom::{Bounds2, PolyVertex, Polyline, Segment, Vec2, point_in_polygon};
use serde::{Deserialize, Serialize};

use super::HatchLoop;

/// Most loops one region has (hostile files and edits).
pub const MAX_REGION_LOOPS: usize = 10_000;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Region {
    /// Closed loops with bulges; `outer` false for islands.
    pub loops: Vec<HatchLoop>,
}

/// Area properties of a region (MASSPROP), in drawing units.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RegionProps {
    pub area: f64,
    pub perimeter: f64,
    pub bounds: Bounds2,
    pub centroid: Vec2,
    /// ∫y² dA and ∫x² dA about the origin.
    pub ixx: f64,
    pub iyy: f64,
    /// ∫xy dA about the origin.
    pub ixy: f64,
    /// The same three about the centroid.
    pub centroid_moments: [f64; 3],
}

impl Region {
    /// A region bounded by one closed loop.
    pub fn from_loop(vertices: Vec<PolyVertex>) -> Region {
        Region { loops: vec![HatchLoop { vertices, outer: true, sources: Vec::new() }] }
    }

    pub fn polylines(&self) -> Vec<Polyline> {
        self.loops.iter().map(|l| Polyline { vertices: l.vertices.clone(), closed: true }).collect()
    }

    /// The boundary: every loop's lines and arcs.
    pub fn segments(&self) -> Vec<Segment> {
        self.polylines().iter().flat_map(Polyline::segments).collect()
    }

    /// Each loop as points, arcs within `tol`.
    pub fn tessellate(&self, tol: f64) -> Vec<Vec<Vec2>> {
        self.polylines().iter().map(|p| p.tessellate(tol)).collect()
    }

    pub fn bounds(&self) -> Bounds2 {
        self.polylines().iter().fold(Bounds2::EMPTY, |b, p| b.union(&p.bounds()))
    }

    /// Outer loops' area less the islands'.
    pub fn area(&self) -> f64 {
        self.loops.iter().map(|l| Polyline { vertices: l.vertices.clone(), closed: true }.area().abs() * if l.outer { 1.0 } else { -1.0 }).sum()
    }

    pub fn perimeter(&self) -> f64 {
        self.polylines().iter().map(Polyline::len).sum()
    }

    /// A chord tolerance fine enough for measuring: 1e-5 of the region's size.
    fn fine_tol(&self) -> f64 {
        let b = self.bounds();
        if b.is_empty() { 1e-6 } else { ((b.width() + b.height()) * 1e-5).max(1e-12) }
    }

    /// Whether `p` lies inside the region (odd parity over its loops).
    pub fn contains(&self, p: Vec2) -> bool {
        self.tessellate(self.fine_tol()).iter().filter(|l| point_in_polygon(l, p)).count() % 2 == 1
    }

    /// Set each loop's `outer` flag from how deeply other loops enclose it: an even depth is
    /// an outer loop, odd an island.
    pub fn renest(&mut self) {
        let polys = self.tessellate(self.fine_tol());
        let depth: Vec<usize> = polys
            .iter()
            .enumerate()
            .map(|(i, l)| {
                let Some(p) = probe(l) else { return 0 };
                polys.iter().enumerate().filter(|(j, o)| *j != i && point_in_polygon(o, p)).count()
            })
            .collect();
        for (l, d) in self.loops.iter_mut().zip(depth) {
            l.outer = d % 2 == 0;
        }
    }

    /// Area, perimeter, bounding box, centroid and second moments (outer loops counted
    /// counter-clockwise, islands clockwise).
    pub fn props(&self) -> RegionProps {
        let tol = self.fine_tol();
        let (mut a, mut cx, mut cy, mut ixx, mut iyy, mut ixy) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
        for (l, pts) in self.loops.iter().zip(self.tessellate(tol)) {
            let signed = cadcraft_geom::shoelace(&pts);
            // Outer loops add, islands subtract, whatever way they run.
            let flip = if (signed >= 0.0) == l.outer { 1.0 } else { -1.0 };
            let n = pts.len();
            for i in 0..n {
                let (Some(p), Some(q)) = (pts.get(i), pts.get((i + 1) % n)) else { continue };
                let c = (p.x * q.y - q.x * p.y) * flip;
                a += c / 2.0;
                cx += (p.x + q.x) * c / 6.0;
                cy += (p.y + q.y) * c / 6.0;
                ixx += (p.y * p.y + p.y * q.y + q.y * q.y) * c / 12.0;
                iyy += (p.x * p.x + p.x * q.x + q.x * q.x) * c / 12.0;
                ixy += (p.x * q.y + 2.0 * p.x * p.y + 2.0 * q.x * q.y + q.x * p.y) * c / 24.0;
            }
        }
        let c = if a.abs() > 1e-300 { Vec2::new(cx / a, cy / a) } else { Vec2::ZERO };
        // Shifted with the area of the same outline the moments were taken from.
        let centroid_moments = [ixx - a * c.y * c.y, iyy - a * c.x * c.x, ixy - a * c.x * c.y];
        RegionProps { area: self.area(), perimeter: self.perimeter(), bounds: self.bounds(), centroid: c, ixx, iyy, ixy, centroid_moments }
    }
}

/// A point of a loop to test its nesting with: the middle of its first non-degenerate edge.
fn probe(l: &[Vec2]) -> Option<Vec2> {
    let n = l.len();
    (0..n).find_map(|i| {
        let (a, b) = (*l.get(i)?, *l.get((i + 1) % n)?);
        (a.dist(b) > 1e-12).then(|| (a + b) * 0.5).filter(|m| m.is_finite())
    })
}
