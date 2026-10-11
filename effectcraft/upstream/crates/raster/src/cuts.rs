//! Scene edit (cut) detection (Layer ▸ Scene Edit Detection).
//!
//! Each frame is reduced to a [`Signature`]: a joint RGB colour histogram (4 × 4 × 4 bins,
//! normalised) and a small luma thumbnail. Two consecutive frames are compared with
//!
//! - the **histogram distance** (half the L1 distance, 0…1): large when the colour content
//!   changes, insensitive to motion;
//! - the **motion-compensated difference**: block optical flow ([`crate::flow::block_flow`])
//!   from the earlier thumbnail to the later one, the earlier one warped along it, and the mean
//!   absolute luma error that remains. Inside a shot motion explains the change and the error
//!   stays small; across a cut nothing matches.
//!
//! The dissimilarity is the geometric mean of the two (both must agree, which rejects flashes and
//! fast pans). A frame starts a new shot when its dissimilarity exceeds the absolute threshold
//! and is a clear local peak: several times the median of its neighbours (the adaptive threshold
//! of the classic shot-boundary literature), with a minimum shot length.

use rayon::prelude::*;

use crate::Image;
use crate::flow::block_flow;

const BINS: usize = 4;
const THUMB: u32 = 96;

/// The per-frame data compared by [`dissimilarity`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Signature {
    pub hist: Vec<f32>,
    pub thumb: Image,
}

/// Compute a frame's signature.
pub fn signature(img: &Image) -> Signature {
    let mut hist = vec![0f32; BINS * BINS * BINS];
    let mut n = 0.0;
    for p in &img.data {
        let a = p[3];
        let c = if a > 1e-6 { [p[0] / a * a.min(1.0), p[1] / a * a.min(1.0), p[2] / a * a.min(1.0)] } else { [0.0; 3] };
        let b = |v: f32| ((v.clamp(0.0, 1.0) * BINS as f32) as usize).min(BINS - 1);
        hist[b(c[0]) * BINS * BINS + b(c[1]) * BINS + b(c[2])] += 1.0;
        n += 1.0;
    }
    if n > 0.0 {
        for h in &mut hist {
            *h /= n;
        }
    }
    let long = img.width.max(img.height).max(1);
    let thumb = if long > THUMB {
        let s = THUMB as f64 / long as f64;
        crate::resample(img, ((img.width as f64 * s).round() as u32).max(8), ((img.height as f64 * s).round() as u32).max(8))
    } else {
        img.clone()
    };
    Signature { hist, thumb }
}

fn luma(p: [f32; 4]) -> f32 {
    0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]
}

/// Dissimilarity of consecutive frames, 0 (same) … 1 (unrelated).
pub fn dissimilarity(a: &Signature, b: &Signature) -> f32 {
    let hist: f32 = a.hist.iter().zip(&b.hist).map(|(x, y)| (x - y).abs()).sum::<f32>() * 0.5;
    let (ta, tb) = (&a.thumb, &b.thumb);
    let motion = if ta.width == tb.width && ta.height == tb.height && !ta.is_empty() {
        let f = block_flow(tb, ta, 8, 4);
        // For each pixel of B, where it came from in A.
        let mut err = 0.0f32;
        let mut n = 0.0f32;
        for y in 0..tb.height {
            for x in 0..tb.width {
                let v = f.at(x as f64 + 0.5, y as f64 + 0.5);
                let pa = ta.sample_bilinear_clamped(x as f64 + 0.5 + v[0] as f64, y as f64 + 0.5 + v[1] as f64);
                err += (luma(pa) - luma(tb.data[tb.idx(x, y)])).abs();
                n += 1.0;
            }
        }
        // Mean absolute error of 0.25 (a quarter of full scale) counts as completely different.
        (err / n.max(1.0) * 4.0).min(1.0)
    } else {
        1.0
    };
    (hist * motion).sqrt()
}

/// Cut detection options.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CutOpts {
    /// Absolute dissimilarity a cut must exceed (0…1).
    pub threshold: f32,
    /// A cut must be this many times the median of its neighbours.
    pub peak_ratio: f32,
    /// Neighbours on each side used for the local median.
    pub window: usize,
    /// Minimum frames between cuts.
    pub min_shot: usize,
}

impl Default for CutOpts {
    fn default() -> Self {
        CutOpts { threshold: 0.25, peak_ratio: 3.0, window: 4, min_shot: 2 }
    }
}

/// Dissimilarities of every frame to the previous one (`scores[0]` = 0).
pub fn scores(frames: &[Image]) -> Vec<f32> {
    let sigs: Vec<Signature> = frames.par_iter().map(signature).collect();
    let mut out = vec![0.0];
    out.extend((1..sigs.len()).into_par_iter().map(|i| dissimilarity(&sigs[i - 1], &sigs[i])).collect::<Vec<_>>());
    out
}

/// Frames that start a new shot (indices into `scores`; never 0).
pub fn detect(scores: &[f32], o: &CutOpts) -> Vec<usize> {
    let mut cuts: Vec<usize> = vec![];
    for i in 1..scores.len() {
        let s = scores[i];
        if s < o.threshold {
            continue;
        }
        let lo = i.saturating_sub(o.window).max(1);
        let hi = (i + o.window).min(scores.len() - 1);
        let mut nb: Vec<f32> = (lo..=hi).filter(|&j| j != i).map(|j| scores[j]).collect();
        nb.sort_by(f32::total_cmp);
        let med = nb.get(nb.len() / 2).copied().unwrap_or(0.0);
        if s < o.peak_ratio * med.max(0.02) {
            continue;
        }
        match cuts.last() {
            Some(&last) if i - last < o.min_shot.max(1) => {
                // Keep the stronger of two close candidates.
                if s > scores[last] {
                    cuts.pop();
                    cuts.push(i);
                }
            }
            _ => cuts.push(i),
        }
    }
    cuts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash_noise;

    /// A textured "shot": `kind` picks the palette and texture, panned by `dx` pixels.
    fn frame(kind: u32, dx: f32, w: u32, h: u32) -> Image {
        let mut img = Image::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let fx = x as f32 + dx;
                let v = match kind {
                    0 => 0.5 + 0.4 * (fx * 0.15).sin() * (y as f32 * 0.11).cos(),
                    1 => {
                        if ((fx / 12.0).floor() as i32 + (y / 12) as i32) % 2 == 0 {
                            0.85
                        } else {
                            0.15
                        }
                    }
                    _ => hash_noise((fx.floor() as i64 / 4) as u32, y / 4, 7),
                };
                let c = match kind {
                    0 => [v, v * 0.6, 0.2],
                    1 => [0.1, v * 0.8, v],
                    _ => [v * 0.9, v * 0.9, v * 0.4 + 0.5],
                };
                img.set(x, y, [c[0], c[1], c[2], 1.0]);
            }
        }
        img
    }

    #[test]
    fn finds_synthetic_cuts_exactly() {
        // Three shots with camera pans (12, 9 and 11 frames) and a brightness flicker inside the
        // second.
        let mut frames = vec![];
        for i in 0..12 {
            frames.push(frame(0, i as f32 * 2.0, 128, 72));
        }
        for i in 0..9 {
            let mut f = frame(1, i as f32 * 3.0, 128, 72);
            if i == 4 {
                f.map_straight(|c| [c[0] * 1.08, c[1] * 1.08, c[2] * 1.08]);
            }
            frames.push(f);
        }
        for i in 0..11 {
            frames.push(frame(2, -(i as f32) * 4.0, 128, 72));
        }
        let s = scores(&frames);
        let cuts = detect(&s, &CutOpts::default());
        assert_eq!(cuts, vec![12, 21], "scores {s:?}");
    }

    #[test]
    fn a_static_shot_has_no_cuts() {
        let frames: Vec<Image> = (0..8).map(|i| frame(2, i as f32, 64, 48)).collect();
        assert!(detect(&scores(&frames), &CutOpts::default()).is_empty());
    }
}
