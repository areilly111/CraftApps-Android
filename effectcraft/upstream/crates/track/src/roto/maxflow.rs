//! Min-cut / max-flow on sparse graphs with source and sink terminals, by the
//! Boykov–Kolmogorov algorithm (Y. Boykov and V. Kolmogorov, "An Experimental Comparison of
//! Min-Cut/Max-Flow Algorithms for Energy Minimization in Vision", IEEE PAMI 26(9), 2004).
//!
//! Two search trees grow from the terminals (S from the source, T from the sink) through
//! non-saturated edges; when they touch, the path found is augmented; saturated tree edges turn
//! the nodes below them into *orphans*, which the adoption stage re-attaches to their tree (or
//! frees). Trees are reused between augmentations, which is what makes the method fast on the
//! grid graphs of image segmentation. The timestamp/distance heuristic of section 3.2.3 keeps
//! adoption paths short.
//!
//! Capacities are integers (callers scale their energies), so saturation tests are exact and
//! the result is deterministic.

use std::collections::VecDeque;

const NONE: u32 = u32::MAX;
const TERMINAL: u32 = u32::MAX - 1;
const ORPHAN: u32 = u32::MAX - 2;

#[derive(Clone, Copy)]
struct Node {
    first: u32,
    /// Arc to the parent (from this node), or TERMINAL / ORPHAN / NONE (free).
    parent: u32,
    /// Residual terminal capacity: > 0 towards the source, < 0 towards the sink.
    tr_cap: i64,
    is_sink: bool,
    active: bool,
    ts: u32,
    dist: u32,
}

#[derive(Clone, Copy)]
struct Arc {
    head: u32,
    next: u32,
    r_cap: i32,
}

/// A graph for one min-cut computation.
pub struct Graph {
    nodes: Vec<Node>,
    arcs: Vec<Arc>,
    flow: i64,
    time: u32,
    queue: VecDeque<u32>,
    orphans: VecDeque<u32>,
}

/// Which side of the cut a node ended on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Segment {
    Source,
    Sink,
}

impl Graph {
    pub fn new(nodes: usize, edges_hint: usize) -> Graph {
        Graph {
            nodes: vec![Node { first: NONE, parent: NONE, tr_cap: 0, is_sink: false, active: false, ts: 0, dist: 0 }; nodes],
            arcs: Vec::with_capacity(edges_hint * 2),
            flow: 0,
            time: 0,
            queue: VecDeque::new(),
            orphans: VecDeque::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Terminal capacities of node `i`: `source` is paid when `i` ends on the sink side, `sink`
    /// when it ends on the source side.
    pub fn add_tweights(&mut self, i: usize, source: i64, sink: i64) {
        let n = &mut self.nodes[i];
        let d = n.tr_cap;
        let (s, t) = if d > 0 { (source + d, sink) } else { (source, sink - d) };
        self.flow += s.min(t);
        n.tr_cap = s - t;
    }

    /// An edge between `i` and `j` with capacity `cap` from i to j and `rev` from j to i.
    pub fn add_edge(&mut self, i: usize, j: usize, cap: i32, rev: i32) {
        debug_assert!(i != j);
        let a = self.arcs.len() as u32;
        self.arcs.push(Arc { head: j as u32, next: self.nodes[i].first, r_cap: cap });
        self.nodes[i].first = a;
        self.arcs.push(Arc { head: i as u32, next: self.nodes[j].first, r_cap: rev });
        self.nodes[j].first = a + 1;
    }

    #[inline]
    fn set_active(&mut self, i: u32) {
        let n = &mut self.nodes[i as usize];
        if !n.active {
            n.active = true;
            self.queue.push_back(i);
        }
    }

    /// Compute the maximum flow (= the minimum cut's cost).
    pub fn maxflow(&mut self) -> i64 {
        for i in 0..self.nodes.len() {
            let n = &mut self.nodes[i];
            if n.tr_cap != 0 {
                n.is_sink = n.tr_cap < 0;
                n.parent = TERMINAL;
                n.ts = 0;
                n.dist = 1;
                self.set_active(i as u32);
            }
        }
        let mut current: Option<u32> = None;
        loop {
            let i = match current.take() {
                Some(i) if self.nodes[i as usize].parent != NONE => i,
                _ => loop {
                    let Some(i) = self.queue.pop_front() else { return self.flow };
                    self.nodes[i as usize].active = false;
                    if self.nodes[i as usize].parent != NONE {
                        break i;
                    }
                },
            };
            // Growth.
            let mut found = NONE;
            let ni = self.nodes[i as usize];
            let mut a = ni.first;
            if !ni.is_sink {
                while a != NONE {
                    let arc = self.arcs[a as usize];
                    if arc.r_cap > 0 {
                        let j = arc.head as usize;
                        let nj = self.nodes[j];
                        if nj.parent == NONE {
                            let n = &mut self.nodes[j];
                            n.is_sink = false;
                            n.parent = a ^ 1;
                            n.ts = ni.ts;
                            n.dist = ni.dist + 1;
                            self.set_active(j as u32);
                        } else if nj.is_sink {
                            found = a;
                            break;
                        } else if nj.ts <= ni.ts && nj.dist > ni.dist {
                            let n = &mut self.nodes[j];
                            n.parent = a ^ 1;
                            n.ts = ni.ts;
                            n.dist = ni.dist + 1;
                        }
                    }
                    a = arc.next;
                }
            } else {
                while a != NONE {
                    let arc = self.arcs[a as usize];
                    if self.arcs[(a ^ 1) as usize].r_cap > 0 {
                        let j = arc.head as usize;
                        let nj = self.nodes[j];
                        if nj.parent == NONE {
                            let n = &mut self.nodes[j];
                            n.is_sink = true;
                            n.parent = a ^ 1;
                            n.ts = ni.ts;
                            n.dist = ni.dist + 1;
                            self.set_active(j as u32);
                        } else if !nj.is_sink {
                            found = a ^ 1;
                            break;
                        } else if nj.ts <= ni.ts && nj.dist > ni.dist {
                            let n = &mut self.nodes[j];
                            n.parent = a ^ 1;
                            n.ts = ni.ts;
                            n.dist = ni.dist + 1;
                        }
                    }
                    a = arc.next;
                }
            }
            self.time = self.time.wrapping_add(1);
            if found != NONE {
                // Keep `i` current: it may have more paths.
                current = Some(i);
                self.augment(found);
                self.adopt();
            }
        }
    }

    fn augment(&mut self, middle: u32) {
        let mut b = self.arcs[middle as usize].r_cap as i64;
        // Source side.
        let mut i = self.arcs[(middle ^ 1) as usize].head;
        loop {
            let a = self.nodes[i as usize].parent;
            if a == TERMINAL {
                break;
            }
            b = b.min(self.arcs[(a ^ 1) as usize].r_cap as i64);
            i = self.arcs[a as usize].head;
        }
        b = b.min(self.nodes[i as usize].tr_cap);
        // Sink side.
        let mut i = self.arcs[middle as usize].head;
        loop {
            let a = self.nodes[i as usize].parent;
            if a == TERMINAL {
                break;
            }
            b = b.min(self.arcs[a as usize].r_cap as i64);
            i = self.arcs[a as usize].head;
        }
        b = b.min(-self.nodes[i as usize].tr_cap);
        let bi = b as i32;
        self.arcs[(middle ^ 1) as usize].r_cap += bi;
        self.arcs[middle as usize].r_cap -= bi;
        // Source side.
        let mut i = self.arcs[(middle ^ 1) as usize].head;
        loop {
            let a = self.nodes[i as usize].parent;
            if a == TERMINAL {
                break;
            }
            self.arcs[a as usize].r_cap += bi;
            self.arcs[(a ^ 1) as usize].r_cap -= bi;
            if self.arcs[(a ^ 1) as usize].r_cap == 0 {
                self.make_orphan(i);
            }
            i = self.arcs[a as usize].head;
        }
        self.nodes[i as usize].tr_cap -= b;
        if self.nodes[i as usize].tr_cap == 0 {
            self.make_orphan(i);
        }
        // Sink side.
        let mut i = self.arcs[middle as usize].head;
        loop {
            let a = self.nodes[i as usize].parent;
            if a == TERMINAL {
                break;
            }
            self.arcs[(a ^ 1) as usize].r_cap += bi;
            self.arcs[a as usize].r_cap -= bi;
            if self.arcs[a as usize].r_cap == 0 {
                self.make_orphan(i);
            }
            i = self.arcs[a as usize].head;
        }
        self.nodes[i as usize].tr_cap += b;
        if self.nodes[i as usize].tr_cap == 0 {
            self.make_orphan(i);
        }
        self.flow += b;
    }

    fn make_orphan(&mut self, i: u32) {
        self.nodes[i as usize].parent = ORPHAN;
        self.orphans.push_back(i);
    }

    /// Whether node `j`'s path to its terminal is intact; returns its distance (marking the
    /// path with the current time) or None.
    fn origin_dist(&mut self, j: u32) -> Option<u32> {
        let mut d = 0u32;
        let mut k = j;
        loop {
            let n = self.nodes[k as usize];
            if n.ts == self.time {
                d += n.dist;
                break;
            }
            let a = n.parent;
            d += 1;
            if a == TERMINAL {
                let n = &mut self.nodes[k as usize];
                n.ts = self.time;
                n.dist = 1;
                break;
            }
            if a == ORPHAN || a == NONE {
                return None;
            }
            k = self.arcs[a as usize].head;
        }
        // Mark the path.
        let total = d;
        let mut k = j;
        let mut dd = d;
        while self.nodes[k as usize].ts != self.time {
            let n = &mut self.nodes[k as usize];
            n.ts = self.time;
            n.dist = dd;
            dd -= 1;
            k = self.arcs[n.parent as usize].head;
        }
        Some(total)
    }

    fn adopt(&mut self) {
        while let Some(i) = self.orphans.pop_front() {
            let sink = self.nodes[i as usize].is_sink;
            let mut best = NONE;
            let mut best_d = u32::MAX;
            let mut a = self.nodes[i as usize].first;
            while a != NONE {
                let arc = self.arcs[a as usize];
                // S orphan: the parent must push flow into i (residual on j→i = sister of i→j).
                // T orphan: i must push flow into the parent (residual on i→j).
                let cap = if sink { arc.r_cap } else { self.arcs[(a ^ 1) as usize].r_cap };
                if cap > 0 {
                    let j = arc.head;
                    let nj = self.nodes[j as usize];
                    if nj.is_sink == sink
                        && nj.parent != NONE
                        && let Some(d) = self.origin_dist(j)
                        && d < best_d
                    {
                        best = a;
                        best_d = d;
                    }
                }
                a = arc.next;
            }
            if best != NONE {
                let n = &mut self.nodes[i as usize];
                n.parent = best;
                n.ts = self.time;
                n.dist = best_d + 1;
                continue;
            }
            // No parent: free the node; its children become orphans, neighbours that could
            // reach it become active.
            self.nodes[i as usize].ts = 0;
            let mut a = self.nodes[i as usize].first;
            while a != NONE {
                let arc = self.arcs[a as usize];
                let j = arc.head;
                let nj = self.nodes[j as usize];
                if nj.is_sink == sink && nj.parent != NONE {
                    let cap = if sink { arc.r_cap } else { self.arcs[(a ^ 1) as usize].r_cap };
                    if cap > 0 {
                        self.set_active(j);
                    }
                    let pa = nj.parent;
                    if pa != TERMINAL && pa != ORPHAN && self.arcs[pa as usize].head == i {
                        self.make_orphan(j);
                    }
                }
                a = arc.next;
            }
            self.nodes[i as usize].parent = NONE;
        }
    }

    /// The side of node `i` after [`Graph::maxflow`] (free nodes go to the sink).
    pub fn segment(&self, i: usize) -> Segment {
        let n = &self.nodes[i];
        if n.parent != NONE && !n.is_sink { Segment::Source } else { Segment::Sink }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Brute-force min cut over all labelings of a small graph.
    fn brute(n: usize, t: &[(i64, i64)], e: &[(usize, usize, i32, i32)]) -> i64 {
        let mut best = i64::MAX;
        for m in 0..(1u32 << n) {
            let src = |i: usize| m & (1 << i) != 0;
            let mut c = 0i64;
            for (i, (s, k)) in t.iter().enumerate() {
                c += if src(i) { *k } else { *s };
            }
            for (i, j, a, b) in e {
                if src(*i) && !src(*j) {
                    c += *a as i64;
                }
                if src(*j) && !src(*i) {
                    c += *b as i64;
                }
            }
            best = best.min(c);
        }
        best
    }

    #[test]
    fn matches_brute_force_on_random_graphs() {
        let mut s = 12345u64;
        let mut rnd = |m: u64| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s % m
        };
        for _ in 0..300 {
            let n = 2 + rnd(9) as usize;
            let t: Vec<(i64, i64)> = (0..n).map(|_| (rnd(20) as i64, rnd(20) as i64)).collect();
            let mut e = vec![];
            for _ in 0..rnd(20) {
                let (i, j) = (rnd(n as u64) as usize, rnd(n as u64) as usize);
                if i != j {
                    e.push((i, j, rnd(15) as i32, rnd(15) as i32));
                }
            }
            let mut g = Graph::new(n, e.len());
            for (i, (a, b)) in t.iter().enumerate() {
                g.add_tweights(i, *a, *b);
            }
            for (i, j, a, b) in &e {
                g.add_edge(*i, *j, *a, *b);
            }
            let f = g.maxflow();
            assert_eq!(f, brute(n, &t, &e));
            // The labeling realises the cut.
            let src = |i: usize| g.segment(i) == Segment::Source;
            let mut c = 0i64;
            for (i, (a, b)) in t.iter().enumerate() {
                c += if src(i) { *b } else { *a };
            }
            for (i, j, a, b) in &e {
                if src(*i) && !src(*j) {
                    c += *a as i64;
                }
                if src(*j) && !src(*i) {
                    c += *b as i64;
                }
            }
            assert_eq!(c, f);
        }
    }
}
