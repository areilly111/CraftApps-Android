//! Polygon triangulation with holes: holes are bridged into the outer contour (each hole's
//! rightmost vertex joined to a visible outer vertex), then ears are clipped.

type P = [f64; 2];

/// Twice the signed area (positive = counter-clockwise in a Y-up frame).
pub fn signed_area(c: &[P]) -> f64 {
    let n = c.len();
    (0..n)
        .map(|i| {
            let (a, b) = (c[i], c[(i + 1) % n]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum()
}

#[inline]
fn cross(o: P, a: P, b: P) -> f64 {
    (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
}

fn in_triangle(p: P, a: P, b: P, c: P) -> bool {
    let d1 = cross(a, b, p);
    let d2 = cross(b, c, p);
    let d3 = cross(c, a, p);
    d1 >= 0.0 && d2 >= 0.0 && d3 >= 0.0
}

/// Triangulate an outer contour with holes. Contours are closed polylines (last point not
/// repeated); orientation is normalised internally. Returns triangles as indices into the
/// contours concatenated in order (`outer`, then `holes`), counter-clockwise (Y up).
pub fn triangulate(outer: &[P], holes: &[Vec<P>]) -> Vec<[usize; 3]> {
    if outer.len() < 3 {
        return vec![];
    }
    let mut pts: Vec<P> = outer.to_vec();
    for h in holes {
        pts.extend_from_slice(h);
    }
    // Ring of global indices, outer counter-clockwise.
    let mut ring: Vec<usize> = (0..outer.len()).collect();
    if signed_area(outer) < 0.0 {
        ring.reverse();
    }
    // Holes clockwise, rightmost first.
    let mut hs: Vec<Vec<usize>> = vec![];
    let mut off = outer.len();
    for h in holes {
        let mut r: Vec<usize> = (off..off + h.len()).collect();
        off += h.len();
        if h.len() < 3 {
            continue;
        }
        if signed_area(h) > 0.0 {
            r.reverse();
        }
        hs.push(r);
    }
    hs.sort_by(|a, b| {
        let mx = |r: &Vec<usize>| r.iter().map(|&i| pts[i][0]).fold(f64::NEG_INFINITY, f64::max);
        mx(b).total_cmp(&mx(a))
    });
    for h in hs {
        bridge(&pts, &mut ring, &h);
    }
    ear_clip(&pts, ring)
}

/// Join hole `h` into `ring` through a bridge from the hole's rightmost vertex.
fn bridge(pts: &[P], ring: &mut Vec<usize>, h: &[usize]) {
    let (hk, &m) = h.iter().enumerate().max_by(|a, b| pts[*a.1][0].total_cmp(&pts[*b.1][0])).unwrap_or((0, &h[0]));
    let mp = pts[m];
    // Nearest ring edge hit by the ray +x from M.
    let n = ring.len();
    let mut best: Option<(f64, usize)> = None;
    for i in 0..n {
        let (a, b) = (pts[ring[i]], pts[ring[(i + 1) % n]]);
        if (a[1] > mp[1]) == (b[1] > mp[1]) && a[1] != mp[1] && b[1] != mp[1] {
            continue;
        }
        if a[1] == b[1] {
            continue;
        }
        let t = (mp[1] - a[1]) / (b[1] - a[1]);
        if !(0.0..=1.0).contains(&t) {
            continue;
        }
        let x = a[0] + t * (b[0] - a[0]);
        if x >= mp[0] && best.is_none_or(|(bx, _)| x < bx) {
            best = Some((x, i));
        }
    }
    let Some((x, i)) = best else { return };
    let (a, b) = (ring[i], ring[(i + 1) % n]);
    // Candidate: the edge endpoint with the larger x.
    let mut cand = if pts[a][0] > pts[b][0] { (i, a) } else { ((i + 1) % n, b) };
    let ip = [x, mp[1]];
    // Reflex ring vertices inside triangle (M, I, P) block the view: take the one with the
    // smallest angle to the ray (closest on ties).
    let pp = pts[cand.1];
    let (t0, t1, t2) = if cross(mp, ip, pp) >= 0.0 { (mp, ip, pp) } else { (mp, pp, ip) };
    let mut best_ang = f64::INFINITY;
    for k in 0..n {
        let v = ring[k];
        if v == cand.1 {
            continue;
        }
        let q = pts[v];
        let prev = pts[ring[(k + n - 1) % n]];
        let next = pts[ring[(k + 1) % n]];
        let reflex = cross(prev, q, next) <= 0.0;
        if reflex && in_triangle(q, t0, t1, t2) {
            let ang = (q[1] - mp[1]).abs().atan2(q[0] - mp[0]);
            let d = (q[0] - mp[0]).hypot(q[1] - mp[1]);
            let cd = (pts[cand.1][0] - mp[0]).hypot(pts[cand.1][1] - mp[1]);
            if ang < best_ang || (ang == best_ang && d < cd) {
                best_ang = ang;
                cand = (k, v);
            }
        }
    }
    let (k, _) = cand;
    // ring[..=k], hole from M around back to M, ring[k] again, rest.
    let mut out = Vec::with_capacity(ring.len() + h.len() + 2);
    out.extend_from_slice(&ring[..=k]);
    for j in 0..=h.len() {
        out.push(h[(hk + j) % h.len()]);
    }
    out.extend_from_slice(&ring[k..]);
    *ring = out;
}

fn ear_clip(pts: &[P], mut ring: Vec<usize>) -> Vec<[usize; 3]> {
    let mut tris = vec![];
    let mut guard = 0usize;
    let mut i = 0usize;
    while ring.len() > 3 {
        let n = ring.len();
        if guard > 2 * n {
            // No ear found (degenerate/self-intersecting input): clip anyway to finish.
            let (a, b, c) = (ring[(i + n - 1) % n], ring[i % n], ring[(i + 1) % n]);
            tris.push([a, b, c]);
            ring.remove(i % n);
            guard = 0;
            continue;
        }
        let ii = i % n;
        let (a, b, c) = (ring[(ii + n - 1) % n], ring[ii], ring[(ii + 1) % n]);
        let (pa, pb, pc) = (pts[a], pts[b], pts[c]);
        let cr = cross(pa, pb, pc);
        let scale = 1.0 + pa[0].abs() + pa[1].abs() + pc[0].abs() + pc[1].abs();
        // A straight-through vertex is clipped as a zero-area triangle (keeps every contour edge
        // in the triangulation, so caps meet walls without T-junctions).
        let straight = cr.abs() <= 1e-12 * scale * scale && (pa[0] - pb[0]) * (pc[0] - pb[0]) + (pa[1] - pb[1]) * (pc[1] - pb[1]) <= 0.0;
        let convex = cr > 0.0 && !straight;
        let mut ear = convex || straight;
        if convex {
            for &v in &ring {
                if v == a || v == b || v == c {
                    continue;
                }
                let q = pts[v];
                // Bridged duplicates share positions with the ear's corners.
                if q == pa || q == pb || q == pc {
                    continue;
                }
                if in_triangle(q, pa, pb, pc) {
                    ear = false;
                    break;
                }
            }
        }
        if ear {
            tris.push([a, b, c]);
            ring.remove(ii);
            guard = 0;
            i = if ii == 0 { 0 } else { ii - 1 };
        } else {
            i = ii + 1;
            guard += 1;
        }
    }
    if ring.len() == 3 {
        tris.push([ring[0], ring[1], ring[2]]);
    }
    tris
}
