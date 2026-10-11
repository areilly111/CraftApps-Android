//! Scene Edit Detection: find the shot changes (hard cuts) in a clip.
//!
//! Each frame is reduced to a [`Signature`]: a 32×18 grid of mean luma and a 16-bin histogram per
//! RGB channel. The difference of two consecutive frames ([`difference`]) mixes the histogram
//! distance (half the L1 distance of the normalised histograms, 0…1) with the mean absolute luma
//! difference of the grids (0…1). A hard cut changes both; camera or subject motion moves the grid
//! but keeps the histogram, and a lighting change shifts the histogram smoothly over many frames.
//!
//! [`detect_cuts`] marks frame *i* as the first frame of a new shot when its difference is above an
//! absolute threshold set by the sensitivity **and** clearly above the local activity (the median of
//! the differences around it), so busy footage needs a bigger jump than static footage. Shots
//! shorter than `min_shot_frames` are merged into the previous one (flashes, single bad frames).

/// Grid size of a signature.
const GW: usize = 32;
const GH: usize = 18;
const BINS: usize = 16;

/// The reduced picture of one frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Signature {
    grid: Vec<f32>,
    hist: [f32; 3 * BINS],
}

/// Reduce an RGBA8 frame (`w`×`h`, row-major, 4 bytes per pixel) to its signature.
pub fn signature(rgba: &[u8], w: usize, h: usize) -> Signature {
    let mut grid = vec![0f32; GW * GH];
    let mut count = vec![0u32; GW * GH];
    let mut hist = [0f32; 3 * BINS];
    if w == 0 || h == 0 || rgba.len() < w * h * 4 {
        return Signature { grid, hist };
    }
    // sample at most ~256×144 pixels: plenty for both measures and cheap for 4K frames
    let step_x = (w / 256).max(1);
    let step_y = (h / 144).max(1);
    let mut n = 0u32;
    for y in (0..h).step_by(step_y) {
        let gy = y * GH / h;
        for x in (0..w).step_by(step_x) {
            let i = (y * w + x) * 4;
            let (r, g, b) = (rgba[i] as f32, rgba[i + 1] as f32, rgba[i + 2] as f32);
            let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
            let gi = gy * GW + x * GW / w;
            grid[gi] += luma;
            count[gi] += 1;
            hist[rgba[i] as usize * BINS / 256] += 1.0;
            hist[BINS + rgba[i + 1] as usize * BINS / 256] += 1.0;
            hist[2 * BINS + rgba[i + 2] as usize * BINS / 256] += 1.0;
            n += 1;
        }
    }
    for (g, c) in grid.iter_mut().zip(&count) {
        *g = if *c > 0 { *g / *c as f32 / 255.0 } else { 0.0 };
    }
    let inv = 1.0 / n.max(1) as f32;
    hist.iter_mut().for_each(|v| *v *= inv);
    Signature { grid, hist }
}

/// How different two frames look, 0 (identical) … 1.
pub fn difference(a: &Signature, b: &Signature) -> f32 {
    // per channel: half the L1 distance of two distributions is in 0..1
    let h: f32 = a.hist.iter().zip(&b.hist).map(|(x, y)| (x - y).abs()).sum::<f32>() / 6.0;
    let g: f32 = a.grid.iter().zip(&b.grid).map(|(x, y)| (x - y).abs()).sum::<f32>() / (GW * GH) as f32;
    // the grid term rarely exceeds ~0.5 even between unrelated shots: weigh it up
    (0.5 * h + 0.5 * (2.0 * g).min(1.0)).clamp(0.0, 1.0)
}

/// Detection settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneOptions {
    /// 0 (only the most obvious cuts) … 100 (also subtle ones). Default 50.
    pub sensitivity: f32,
    /// Shortest shot kept, in frames (a cut closer than this to the previous one is ignored).
    pub min_shot_frames: usize,
}

impl Default for SceneOptions {
    fn default() -> Self {
        SceneOptions { sensitivity: 50.0, min_shot_frames: 6 }
    }
}

impl SceneOptions {
    /// The absolute difference a cut must exceed.
    pub fn threshold(&self) -> f32 {
        let s = self.sensitivity.clamp(0.0, 100.0) / 100.0;
        0.45 - 0.33 * s
    }
}

/// Frames that start a new shot. `diffs[i]` is the difference between frame `i - 1` and frame
/// `i` (`diffs[0]` is ignored).
pub fn detect_cuts(diffs: &[f32], opt: &SceneOptions) -> Vec<usize> {
    const WINDOW: usize = 5;
    let thr = opt.threshold();
    let mut cuts: Vec<usize> = Vec::new();
    for i in 1..diffs.len() {
        let d = diffs[i];
        if d < thr {
            continue;
        }
        // local activity: median of the neighbours (the candidate itself excluded)
        let lo = i.saturating_sub(WINDOW).max(1);
        let hi = (i + WINDOW + 1).min(diffs.len());
        let mut around: Vec<f32> = (lo..hi).filter(|&j| j != i).map(|j| diffs[j]).collect();
        let med = if around.is_empty() {
            0.0
        } else {
            around.sort_by(f32::total_cmp);
            around[around.len() / 2]
        };
        if d < 2.5 * med + 0.05 {
            continue;
        }
        // a local maximum (two-frame dissolves report their larger step once)
        if diffs.get(i + 1).is_some_and(|n| *n > d) || (i > 1 && diffs[i - 1] > d && cuts.last() != Some(&(i - 1))) {
            continue;
        }
        let since = cuts.last().copied().unwrap_or(0);
        if i - since < opt.min_shot_frames.max(1) {
            continue;
        }
        cuts.push(i);
    }
    cuts
}

/// Incremental detector: push frames in order, read the cuts at the end.
#[derive(Clone, Debug, Default)]
pub struct SceneDetector {
    prev: Option<Signature>,
    diffs: Vec<f32>,
}

impl SceneDetector {
    pub fn new() -> Self {
        Self::default()
    }
    /// Add the next frame; returns its difference to the previous one.
    pub fn push_rgba(&mut self, rgba: &[u8], w: usize, h: usize) -> f32 {
        self.push(signature(rgba, w, h))
    }
    pub fn push(&mut self, sig: Signature) -> f32 {
        let d = self.prev.as_ref().map(|p| difference(p, &sig)).unwrap_or(0.0);
        self.diffs.push(d);
        self.prev = Some(sig);
        d
    }
    pub fn diffs(&self) -> &[f32] {
        &self.diffs
    }
    pub fn cuts(&self, opt: &SceneOptions) -> Vec<usize> {
        detect_cuts(&self.diffs, opt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic shot: a pattern chosen by `shot`, moving with time `t` (pan + noise).
    fn frame(shot: u32, t: u32, w: usize, h: usize) -> Vec<u8> {
        let mut v = vec![0u8; w * h * 4];
        let mut seed = shot.wrapping_mul(2_654_435_761) ^ t.wrapping_mul(40_503);
        for y in 0..h {
            for x in 0..w {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                let noise = ((seed >> 16) % 9) as i32 - 4;
                let xx = (x as u32 + 2 * t) as f32;
                let (r, g, b) = match shot % 4 {
                    0 => (40.0 + 150.0 * (xx * 0.05).sin().abs(), 90.0, 160.0),
                    1 => (200.0, 180.0 - (y as f32) * 0.5, 40.0 + (xx * 0.1).cos().abs() * 60.0),
                    2 => (if (xx as usize / 8 + y / 8).is_multiple_of(2) { 230.0 } else { 20.0 }, 120.0, 60.0),
                    _ => (30.0, 30.0 + (y as f32 * 1.5).min(200.0), 30.0 + (xx * 0.03).sin().abs() * 200.0),
                };
                let i = (y * w + x) * 4;
                v[i] = (r as i32 + noise).clamp(0, 255) as u8;
                v[i + 1] = (g as i32 + noise).clamp(0, 255) as u8;
                v[i + 2] = (b as i32 + noise).clamp(0, 255) as u8;
                v[i + 3] = 255;
            }
        }
        v
    }

    fn run(shots: &[(u32, u32)], opt: &SceneOptions) -> Vec<usize> {
        let mut d = SceneDetector::new();
        let mut t = 0;
        for &(shot, len) in shots {
            for _ in 0..len {
                d.push_rgba(&frame(shot, t, 96, 54), 96, 54);
                t += 1;
            }
        }
        d.cuts(opt)
    }

    #[test]
    fn finds_hard_cuts_and_ignores_motion() {
        let cuts = run(&[(0, 20), (1, 15), (2, 25), (3, 12), (0, 10)], &SceneOptions::default());
        assert_eq!(cuts, vec![20, 35, 60, 72]);
        // one continuous moving shot: no cuts at any sensitivity
        for s in [0.0, 50.0, 100.0] {
            assert!(run(&[(2, 60)], &SceneOptions { sensitivity: s, ..Default::default() }).is_empty(), "sensitivity {s}");
        }
    }

    #[test]
    fn min_shot_length_merges_flashes() {
        // a 2-frame flash of another shot inside shot 0: one cut into the flash is kept, the cut
        // back out (2 frames later) is closer than the minimum shot length
        let cuts = run(&[(0, 20), (1, 2), (0, 20)], &SceneOptions::default());
        assert_eq!(cuts, vec![20]);
        let cuts = run(&[(0, 20), (1, 2), (0, 20)], &SceneOptions { min_shot_frames: 1, ..Default::default() });
        assert_eq!(cuts, vec![20, 22]);
    }

    #[test]
    fn sensitivity_controls_subtle_cuts() {
        // a subtle cut: the same pattern with a small global brightness change
        let mk = |k: i32| {
            let mut f = frame(0, 0, 64, 36);
            f.chunks_mut(4).for_each(|p| (0..3).for_each(|c| p[c] = (p[c] as i32 + k).clamp(0, 255) as u8));
            signature(&f, 64, 36)
        };
        let mut d = SceneDetector::new();
        for i in 0..30 {
            d.push(mk(if i < 15 { 0 } else { 12 }));
        }
        let diff = d.diffs()[15];
        assert!(diff > 0.1 && diff < 0.4, "subtle diff {diff}");
        assert!(d.cuts(&SceneOptions { sensitivity: 0.0, ..Default::default() }).is_empty());
        assert_eq!(d.cuts(&SceneOptions { sensitivity: 100.0, ..Default::default() }), vec![15]);
        assert!(SceneOptions { sensitivity: 100.0, ..Default::default() }.threshold() < SceneOptions::default().threshold());
    }

    #[test]
    fn degenerate_inputs() {
        assert!(detect_cuts(&[], &SceneOptions::default()).is_empty());
        assert!(detect_cuts(&[0.0], &SceneOptions::default()).is_empty());
        let s = signature(&[], 0, 0);
        assert_eq!(difference(&s, &s), 0.0);
    }
}
