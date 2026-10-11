//! Numeric summaries of scopes, for agents checking colour without looking (`scopes.read`).
//! Levels are in percent of full scale (IRE-like: code value / 255 × 100 for 8-bit signals).

use filmcraft_color::rgb_to_ycbcr;
use serde::Serialize;

use crate::{Grid, Matrix, Signal, Vectorscope, Waveform, angle_deg};

/// Minimum, maximum and mean of a set of levels (percent).
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Level {
    pub min: f64,
    pub max: f64,
    pub mean: f64,
}

#[derive(Default)]
struct Acc {
    min: f32,
    max: f32,
    sum: f64,
    n: u64,
}

impl Acc {
    fn add(&mut self, v: f32, count: u64) {
        if self.n == 0 {
            self.min = v;
            self.max = v;
        } else {
            self.min = self.min.min(v);
            self.max = self.max.max(v);
        }
        self.sum += v as f64 * count as f64;
        self.n += count;
    }
    fn level(&self) -> Option<Level> {
        (self.n > 0).then(|| Level { min: round2(self.min), max: round2(self.max), mean: round2(self.sum / self.n as f64) })
    }
}

/// Rounded to 0.01 (as f64, so JSON shows `78.43`, not the f32's `78.43000030517578`).
fn round2(v: impl Into<f64>) -> f64 {
    (v.into() * 100.0).round() / 100.0
}

/// Per-channel levels of the signal itself (R', G', B', Y'), exact (not from grid cells).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelStats {
    pub r: Level,
    pub g: Level,
    pub b: Level,
    pub y: Level,
    /// Mean chroma (Cb, Cr) in −50..50 and its angle / magnitude (percent of 0.5).
    pub chroma_mean: [f64; 2],
    pub hue_deg: f64,
    pub saturation: f64,
}

pub fn channel_stats(s: &Signal, m: Matrix) -> Option<ChannelStats> {
    if s.is_empty() {
        return None;
    }
    let mut acc: [Acc; 4] = Default::default();
    let (mut cb_sum, mut cr_sum) = (0f64, 0f64);
    for &[r, g, b] in &s.rgb {
        let [y, cb, cr] = rgb_to_ycbcr(r, g, b, m);
        for (a, v) in acc.iter_mut().zip([r, g, b, y]) {
            a.add(v * 100.0, 1);
        }
        cb_sum += cb as f64;
        cr_sum += cr as f64;
    }
    let n = s.len() as f64;
    let (cb, cr) = ((cb_sum / n) as f32, (cr_sum / n) as f32);
    Some(ChannelStats {
        r: acc[0].level()?,
        g: acc[1].level()?,
        b: acc[2].level()?,
        y: acc[3].level()?,
        chroma_mean: [round2(cb * 100.0), round2(cr * 100.0)],
        hue_deg: round2(angle_deg(cb, cr)),
        saturation: round2((cb * cb + cr * cr).sqrt() / 0.5 * 100.0),
    })
}

/// A trace split into `buckets` column ranges, each with the levels its cells hold (None when a
/// range holds no samples).
pub fn trace_columns(w: &Waveform, g: &Grid, buckets: usize) -> Vec<Option<Level>> {
    let buckets = buckets.clamp(1, g.cols.max(1));
    (0..buckets)
        .map(|b| {
            let (c0, c1) = (b * g.cols / buckets, ((b + 1) * g.cols / buckets).max(b * g.cols / buckets + 1).min(g.cols));
            let mut a = Acc::default();
            for col in c0..c1 {
                for row in 0..g.rows {
                    let n = g.at(col, row);
                    if n > 0 {
                        a.add(w.level(row, g.rows) * 100.0, n as u64);
                    }
                }
            }
            a.level()
        })
        .collect()
}

/// A dense spot of a vectorscope.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Peak {
    /// Cell coordinates on the scope's grid.
    pub x: usize,
    pub y: usize,
    /// Cb, Cr at the cell centre (percent: −50..50 is the nominal range).
    pub cb: f64,
    pub cr: f64,
    pub angle_deg: f64,
    /// Distance from the centre in percent of 0.5 (100 % red in BT.709 ≈ 102.6).
    pub magnitude: f64,
    /// Share of the plotted samples in this cell (0..1).
    pub share: f64,
}

/// The `n` densest cells holding at least `min_share` of the samples, densest first.
pub fn peaks(v: &Vectorscope, n: usize, min_share: f32) -> Vec<Peak> {
    let total = v.samples.max(1) as f32;
    let mut cells: Vec<(u32, usize)> = v.data.iter().enumerate().filter(|(_, c)| **c > 0).map(|(i, c)| (*c, i)).collect();
    cells.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    cells
        .into_iter()
        .filter(|(c, _)| *c as f32 / total >= min_share)
        .take(n)
        .map(|(c, i)| {
            let (x, y) = (i % v.size, i / v.size);
            let [cb, cr] = v.point(x, y);
            Peak {
                x,
                y,
                cb: round2(cb * 100.0),
                cr: round2(cr * 100.0),
                angle_deg: round2(angle_deg(cb, cr)),
                magnitude: round2((cb * cb + cr * cr).sqrt() / 0.5 * 100.0),
                share: (c as f64 / total as f64 * 10_000.0).round() / 10_000.0,
            }
        })
        .collect()
}

/// Sparse cells of a vectorscope re-binned to `size`² (for a coarse numeric picture): `[x, y, count]`.
pub fn coarse(v: &Vectorscope, size: usize) -> Vec<[u32; 3]> {
    let size = size.clamp(1, v.size);
    let mut out = vec![0u32; size * size];
    for y in 0..v.size {
        for x in 0..v.size {
            let c = v.at(x, y);
            if c > 0 {
                out[(y * size / v.size) * size + x * size / v.size] += c;
            }
        }
    }
    out.iter().enumerate().filter(|(_, c)| **c > 0).map(|(i, c)| [(i % size) as u32, (i / size) as u32, *c]).collect()
}
