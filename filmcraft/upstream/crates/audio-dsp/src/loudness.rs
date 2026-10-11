//! ITU-R BS.1770-4 / EBU R128 loudness metering.
//!
//! * K-weighting: the two BS.1770 stages (high-shelf "pre-filter" and the RLB high-pass) are
//!   derived from their analog prototypes (centre frequency, Q, gain) through the bilinear
//!   transform, so any sample rate is supported; at 48 kHz they reproduce the coefficient table in
//!   the recommendation.
//! * Momentary (400 ms), short-term (3 s) windows are evaluated every 100 ms (75 % overlap for the
//!   400 ms gating blocks, as specified).
//! * Integrated loudness uses the two-stage gate (−70 LUFS absolute, −10 LU relative). Gating
//!   blocks are accumulated in a fixed 0.01 LU histogram that also stores exact energy sums, so the
//!   meter can run for arbitrarily long programmes without allocating and the result is exact up
//!   to the placement of blocks within 0.01 LU of the relative threshold.
//! * Loudness range (EBU Tech 3342): short-term values, −70 LUFS absolute and −20 LU relative
//!   gate, LRA = 95th − 10th percentile.
//! * Sample peak and true peak (polyphase oversampling to ≥ 176.4 kHz, see [`crate::oversample`]).

use crate::biquad::{Biquad, Coeffs};
use crate::oversample::{Interpolator, PolyphaseBank, true_peak_factor};
use std::f64::consts::PI;

const HIST_MIN: f64 = -70.0;
const HIST_MAX: f64 = 20.0;
const HIST_STEP: f64 = 0.01;
const HIST_BINS: usize = ((HIST_MAX - HIST_MIN) / HIST_STEP) as usize;
/// Short-term window in 100 ms sub-blocks.
const ST_SUBBLOCKS: usize = 30;
/// Momentary window in 100 ms sub-blocks.
const M_SUBBLOCKS: usize = 4;

/// Loudness of a mean-square energy (already channel-weighted), in LUFS.
#[inline]
pub fn energy_to_lufs(e: f64) -> f64 {
    if e <= 0.0 { f64::NEG_INFINITY } else { -0.691 + 10.0 * e.log10() }
}

#[cfg(test)]
fn lufs_to_energy(l: f64) -> f64 {
    10f64.powf((l + 0.691) / 10.0)
}

/// Stage 1 of K-weighting: the high-shelf ("head") pre-filter for `sample_rate`.
pub fn k_prefilter(sample_rate: f64) -> Coeffs {
    // Analog prototype parameters of the BS.1770 shelving stage.
    let f0 = 1_681.974_450_955_533;
    let gain_db = 3.999_843_853_973_347;
    let q = 0.707_175_236_955_419_6;
    let k = (PI * f0 / sample_rate).tan();
    let vh = 10f64.powf(gain_db / 20.0);
    let vb = vh.powf(0.499_666_774_154_541_6);
    let a0 = 1.0 + k / q + k * k;
    Coeffs {
        b0: (vh + vb * k / q + k * k) / a0,
        b1: 2.0 * (k * k - vh) / a0,
        b2: (vh - vb * k / q + k * k) / a0,
        a1: 2.0 * (k * k - 1.0) / a0,
        a2: (1.0 - k / q + k * k) / a0,
    }
}

/// Stage 2 of K-weighting: the RLB high-pass for `sample_rate`.
pub fn k_rlb(sample_rate: f64) -> Coeffs {
    let f0 = 38.135_470_876_024_44;
    let q = 0.500_327_037_323_877_3;
    let k = (PI * f0 / sample_rate).tan();
    let a0 = 1.0 + k / q + k * k;
    Coeffs { b0: 1.0, b1: -2.0, b2: 1.0, a1: 2.0 * (k * k - 1.0) / a0, a2: (1.0 - k / q + k * k) / a0 }
}

/// Default BS.1770 channel weights for a channel count (L R C LFE Ls Rs [Lb Rb] order for 5.1 /
/// 7.1; LFE is excluded; surrounds get 1.41 ≈ +1.5 dB).
pub fn default_channel_weights(channels: usize) -> Vec<f64> {
    match channels {
        5 => vec![1.0, 1.0, 1.0, 1.41, 1.41],
        6 => vec![1.0, 1.0, 1.0, 0.0, 1.41, 1.41],
        8 => vec![1.0, 1.0, 1.0, 0.0, 1.41, 1.41, 1.41, 1.41],
        n => vec![1.0; n],
    }
}

/// Histogram of block loudness values with exact energy sums per bin.
#[derive(Clone, Debug)]
struct Histogram {
    count: Vec<u64>,
    energy: Vec<f64>,
}

impl Histogram {
    fn new() -> Self {
        Histogram { count: vec![0; HIST_BINS], energy: vec![0.0; HIST_BINS] }
    }
    fn clear(&mut self) {
        self.count.iter_mut().for_each(|c| *c = 0);
        self.energy.iter_mut().for_each(|e| *e = 0.0);
    }
    fn bin_of(l: f64) -> usize {
        (((l - HIST_MIN) / HIST_STEP).floor().max(0.0) as usize).min(HIST_BINS - 1)
    }
    fn bin_centre(i: usize) -> f64 {
        HIST_MIN + (i as f64 + 0.5) * HIST_STEP
    }
    /// Add a block if it passes the absolute gate.
    fn add(&mut self, energy: f64) {
        let l = energy_to_lufs(energy);
        if l >= HIST_MIN {
            let b = Self::bin_of(l);
            self.count[b] += 1;
            self.energy[b] += energy;
        }
    }
    /// Power mean (energy) of bins from `from` upward; None if empty.
    fn mean_energy_from(&self, from: usize) -> Option<(f64, u64)> {
        let (mut e, mut n) = (0.0, 0u64);
        for i in from..HIST_BINS {
            e += self.energy[i];
            n += self.count[i];
        }
        (n > 0).then(|| (e / n as f64, n))
    }
    /// Relative-gate start bin for a gate of `rel` LU below the absolute-gated mean.
    fn relative_gate_bin(&self, rel: f64) -> Option<usize> {
        let (e, _) = self.mean_energy_from(0)?;
        let thr = energy_to_lufs(e) + rel;
        Some(if thr < HIST_MIN { 0 } else { Self::bin_of(thr) })
    }
}

/// Streaming BS.1770-4 / EBU R128 loudness meter for planar or interleaved f32 audio.
#[derive(Clone, Debug)]
pub struct LoudnessMeter {
    sample_rate: f64,
    channels: usize,
    weights: Vec<f64>,
    pre: Coeffs,
    rlb: Coeffs,
    filt: Vec<(Biquad, Biquad)>,
    /// Per-channel sum of squares in the current 100 ms sub-block.
    acc: Vec<f64>,
    sub_len: usize,
    sub_pos: usize,
    /// Ring of (weighted energy sum, sample count) for the last 30 sub-blocks.
    ring: [(f64, usize); ST_SUBBLOCKS],
    ring_pos: usize,
    subblocks_seen: u64,
    momentary: f64,
    short_term: f64,
    max_momentary: f64,
    max_short_term: f64,
    integrated_hist: Histogram,
    lra_hist: Histogram,
    sample_peak: Vec<f32>,
    true_peak: Vec<f32>,
    bank: PolyphaseBank,
    interp: Vec<Interpolator>,
}

impl LoudnessMeter {
    /// A meter for `channels` channels at `sample_rate` Hz with default BS.1770 weights.
    pub fn new(sample_rate: f64, channels: usize) -> Self {
        let channels = channels.max(1);
        let sub_len = ((sample_rate * 0.1).round() as usize).max(1);
        LoudnessMeter {
            sample_rate,
            channels,
            weights: default_channel_weights(channels),
            pre: k_prefilter(sample_rate),
            rlb: k_rlb(sample_rate),
            filt: vec![(Biquad::new(), Biquad::new()); channels],
            acc: vec![0.0; channels],
            sub_len,
            sub_pos: 0,
            ring: [(0.0, 0); ST_SUBBLOCKS],
            ring_pos: 0,
            subblocks_seen: 0,
            momentary: f64::NEG_INFINITY,
            short_term: f64::NEG_INFINITY,
            max_momentary: f64::NEG_INFINITY,
            max_short_term: f64::NEG_INFINITY,
            integrated_hist: Histogram::new(),
            lra_hist: Histogram::new(),
            sample_peak: vec![0.0; channels],
            true_peak: vec![0.0; channels],
            bank: PolyphaseBank::new(true_peak_factor(sample_rate)),
            interp: vec![Interpolator::default(); channels],
        }
    }

    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Override the per-channel weights (missing entries are 1.0, extra ignored).
    pub fn set_channel_weights(&mut self, weights: &[f64]) {
        for (i, w) in self.weights.iter_mut().enumerate() {
            *w = weights.get(i).copied().unwrap_or(1.0);
        }
    }

    /// Clear all measurements and filter state.
    pub fn reset(&mut self) {
        self.integrated_hist.clear();
        self.lra_hist.clear();
        self.filt.iter_mut().for_each(|(a, b)| {
            a.reset();
            b.reset()
        });
        self.acc.iter_mut().for_each(|a| *a = 0.0);
        self.sample_peak.iter_mut().for_each(|p| *p = 0.0);
        self.true_peak.iter_mut().for_each(|p| *p = 0.0);
        self.interp.iter_mut().for_each(Interpolator::reset);
        self.sub_pos = 0;
        self.ring = [(0.0, 0); ST_SUBBLOCKS];
        self.ring_pos = 0;
        self.subblocks_seen = 0;
        self.momentary = f64::NEG_INFINITY;
        self.short_term = f64::NEG_INFINITY;
        self.max_momentary = f64::NEG_INFINITY;
        self.max_short_term = f64::NEG_INFINITY;
    }

    #[inline(always)]
    fn feed(&mut self, ch: usize, x: f32) {
        let (pre, rlb) = &mut self.filt[ch];
        let y = rlb.tick(&self.rlb, pre.tick(&self.pre, x as f64));
        self.acc[ch] += y * y;
        let a = x.abs();
        if a > self.sample_peak[ch] {
            self.sample_peak[ch] = a;
        }
        let tp = self.interp[ch].push_peak(&self.bank, x).max(a);
        if tp > self.true_peak[ch] {
            self.true_peak[ch] = tp;
        }
    }

    /// Feed a block of planar audio (one slice per channel; extra channels are ignored, missing
    /// channels count as silence).
    pub fn process(&mut self, channels: &[&[f32]]) {
        let n = channels.iter().take(self.channels).map(|c| c.len()).min().unwrap_or(0);
        let mut pos = 0;
        while pos < n {
            let take = (n - pos).min(self.sub_len - self.sub_pos);
            for (ch, data) in channels.iter().take(self.channels).enumerate() {
                for &x in &data[pos..pos + take] {
                    self.feed(ch, x);
                }
            }
            for ch in channels.len()..self.channels {
                for _ in 0..take {
                    self.feed(ch, 0.0);
                }
            }
            pos += take;
            self.sub_pos += take;
            if self.sub_pos == self.sub_len {
                self.finish_subblock();
            }
        }
    }

    /// Feed a block of interleaved audio with `self.channels()` channels per frame.
    pub fn process_interleaved(&mut self, data: &[f32]) {
        let nch = self.channels;
        let frames = data.len() / nch;
        let mut f = 0;
        while f < frames {
            let take = (frames - f).min(self.sub_len - self.sub_pos);
            for i in f..f + take {
                for ch in 0..nch {
                    self.feed(ch, data[i * nch + ch]);
                }
            }
            f += take;
            self.sub_pos += take;
            if self.sub_pos == self.sub_len {
                self.finish_subblock();
            }
        }
    }

    fn finish_subblock(&mut self) {
        let mut e = 0.0;
        for ch in 0..self.channels {
            e += self.weights[ch] * self.acc[ch];
            self.acc[ch] = 0.0;
        }
        self.ring[self.ring_pos] = (e, self.sub_pos);
        self.ring_pos = (self.ring_pos + 1) % ST_SUBBLOCKS;
        self.sub_pos = 0;
        self.subblocks_seen += 1;

        let window = |k: usize, ring: &[(f64, usize); ST_SUBBLOCKS], pos: usize, sub_len: usize| {
            let mut s = 0.0;
            for i in 0..k {
                s += ring[(pos + ST_SUBBLOCKS - 1 - i) % ST_SUBBLOCKS].0;
            }
            s / (k * sub_len) as f64
        };
        let em = window(M_SUBBLOCKS, &self.ring, self.ring_pos, self.sub_len);
        let es = window(ST_SUBBLOCKS, &self.ring, self.ring_pos, self.sub_len);
        self.momentary = energy_to_lufs(em);
        self.short_term = energy_to_lufs(es);
        if self.subblocks_seen >= M_SUBBLOCKS as u64 {
            self.integrated_hist.add(em);
            self.max_momentary = self.max_momentary.max(self.momentary);
        }
        if self.subblocks_seen >= ST_SUBBLOCKS as u64 {
            self.lra_hist.add(es);
            self.max_short_term = self.max_short_term.max(self.short_term);
        }
    }

    /// Momentary loudness (last 400 ms), LUFS; updated every 100 ms. −∞ for silence.
    pub fn momentary(&self) -> f64 {
        self.momentary
    }
    /// Short-term loudness (last 3 s), LUFS; updated every 100 ms.
    pub fn short_term(&self) -> f64 {
        self.short_term
    }
    /// Maximum momentary loudness so far.
    pub fn max_momentary(&self) -> f64 {
        self.max_momentary
    }
    /// Maximum short-term loudness so far.
    pub fn max_short_term(&self) -> f64 {
        self.max_short_term
    }

    /// Gated integrated loudness (BS.1770-4), LUFS. −∞ if nothing passed the gates.
    pub fn integrated(&self) -> f64 {
        let h = &self.integrated_hist;
        match h.relative_gate_bin(-10.0).and_then(|b| h.mean_energy_from(b)) {
            Some((e, _)) => energy_to_lufs(e),
            None => f64::NEG_INFINITY,
        }
    }

    /// Relative gate threshold used for the integrated loudness (LUFS).
    pub fn relative_threshold(&self) -> f64 {
        match self.integrated_hist.mean_energy_from(0) {
            Some((e, _)) => energy_to_lufs(e) - 10.0,
            None => f64::NEG_INFINITY,
        }
    }

    /// Loudness range (EBU Tech 3342), LU. 0 when too little signal.
    pub fn loudness_range(&self) -> f64 {
        let h = &self.lra_hist;
        let Some(start) = h.relative_gate_bin(-20.0) else { return 0.0 };
        let n: u64 = h.count[start..].iter().sum();
        if n == 0 {
            return 0.0;
        }
        let percentile = |p: f64| {
            // Nearest-rank on the sorted gated values.
            let rank = ((p * (n - 1) as f64).round() as u64).min(n - 1);
            let mut c = 0u64;
            for i in start..HIST_BINS {
                c += h.count[i];
                if c > rank {
                    return Histogram::bin_centre(i);
                }
            }
            Histogram::bin_centre(HIST_BINS - 1)
        };
        (percentile(0.95) - percentile(0.10)).max(0.0)
    }

    /// Maximum absolute sample value over all channels (linear).
    pub fn sample_peak(&self) -> f64 {
        self.sample_peak.iter().fold(0.0f32, |a, &b| a.max(b)) as f64
    }
    /// Per-channel sample peak (linear).
    pub fn sample_peak_channel(&self, ch: usize) -> f64 {
        self.sample_peak.get(ch).copied().unwrap_or(0.0) as f64
    }
    /// Maximum true peak over all channels (linear).
    pub fn true_peak(&self) -> f64 {
        self.true_peak.iter().fold(0.0f32, |a, &b| a.max(b)) as f64
    }
    /// Per-channel true peak (linear).
    pub fn true_peak_channel(&self, ch: usize) -> f64 {
        self.true_peak.get(ch).copied().unwrap_or(0.0) as f64
    }
    /// Maximum true peak in dBTP.
    pub fn true_peak_dbtp(&self) -> f64 {
        20.0 * self.true_peak().log10()
    }

    /// Snapshot of all programme measurements.
    pub fn summary(&self) -> LoudnessSummary {
        LoudnessSummary {
            integrated_lufs: self.integrated(),
            loudness_range_lu: self.loudness_range(),
            max_momentary_lufs: self.max_momentary,
            max_short_term_lufs: self.max_short_term,
            sample_peak_dbfs: 20.0 * self.sample_peak().log10(),
            true_peak_dbtp: self.true_peak_dbtp(),
        }
    }
}

/// Programme loudness measurements (EBU R128 set).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoudnessSummary {
    pub integrated_lufs: f64,
    pub loudness_range_lu: f64,
    pub max_momentary_lufs: f64,
    pub max_short_term_lufs: f64,
    pub sample_peak_dbfs: f64,
    pub true_peak_dbtp: f64,
}

/// Measure a whole planar programme in one call.
pub fn measure(channels: &[&[f32]], sample_rate: f64) -> LoudnessSummary {
    let mut m = LoudnessMeter::new(sample_rate, channels.len());
    m.process(channels);
    m.summary()
}

/// Gain (dB) that moves a programme measured at `measured_lufs` to `target_lufs`.
/// Returns 0 for silent / unmeasurable input.
pub fn normalize_gain_db(measured_lufs: f64, target_lufs: f64) -> f64 {
    if measured_lufs.is_finite() && target_lufs.is_finite() { target_lufs - measured_lufs } else { 0.0 }
}

/// Like [`normalize_gain_db`] but limited so that the resulting true peak does not exceed
/// `max_true_peak_dbtp` (a linear gain can't fix peaks, so the loudness target is sacrificed —
/// the behaviour of "Normalize" / Essential Sound auto-match without a limiter).
pub fn normalize_gain_db_peak_limited(measured_lufs: f64, target_lufs: f64, true_peak_dbtp: f64, max_true_peak_dbtp: f64) -> f64 {
    let g = normalize_gain_db(measured_lufs, target_lufs);
    if true_peak_dbtp.is_finite() { g.min(max_true_peak_dbtp - true_peak_dbtp) } else { g }
}

/// Expected loudness energy weighting of a pure tone at `freq` through K-weighting (|H|², for
/// tests and analytical tooling).
pub fn k_weighting_power(freq: f64, sample_rate: f64) -> f64 {
    let a = k_prefilter(sample_rate).magnitude(freq, sample_rate);
    let b = k_rlb(sample_rate).magnitude(freq, sample_rate);
    (a * b).powi(2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;

    fn dbfs(db: f64) -> f64 {
        10f64.powf(db / 20.0)
    }

    /// Stereo programme made of (seconds, dBFS) sine segments.
    fn segments(freq: f64, sr: f64, segs: &[(f64, f64)]) -> Vec<f32> {
        let mut out = Vec::new();
        let mut phase_idx = 0usize;
        for &(secs, level) in segs {
            let n = (secs * sr).round() as usize;
            let a = dbfs(level);
            for _ in 0..n {
                out.push((a * (2.0 * PI * freq * phase_idx as f64 / sr).sin()) as f32);
                phase_idx += 1;
            }
        }
        out
    }

    fn stereo_measure(x: &[f32], sr: f64) -> LoudnessMeter {
        let mut m = LoudnessMeter::new(sr, 2);
        // Feed in odd-sized blocks to exercise the sub-block splitting.
        for chunk in x.chunks(1237) {
            m.process(&[chunk, chunk]);
        }
        m
    }

    #[test]
    fn k_weighting_matches_bs1770_table_at_48k() {
        let p = k_prefilter(48000.0);
        assert!((p.b0 - 1.535_124_859_586_97).abs() < 1e-8);
        assert!((p.b1 + 2.691_696_189_406_38).abs() < 1e-8);
        assert!((p.b2 - 1.198_392_810_852_85).abs() < 1e-8);
        assert!((p.a1 + 1.690_659_293_182_41).abs() < 1e-8);
        assert!((p.a2 - 0.732_480_774_215_85).abs() < 1e-8);
        let r = k_rlb(48000.0);
        assert!((r.a1 + 1.990_047_454_833_98).abs() < 1e-8);
        assert!((r.a2 - 0.990_072_250_366_21).abs() < 1e-8);
    }

    #[test]
    fn k_weighting_shape_is_sample_rate_independent() {
        for sr in [32000.0, 44100.0, 48000.0, 96000.0, 192000.0] {
            let db = |f| 10.0 * k_weighting_power(f, sr).log10();
            // ~+0.69 dB at 1 kHz, ~+4 dB high shelf, strong low-cut.
            assert!((db(1000.0) - 0.691).abs() < 0.03, "sr {sr}: {}", db(1000.0));
            assert!((db(10000.0) - 4.0).abs() < 0.3, "sr {sr}");
            assert!(db(20.0) < -10.0, "sr {sr}");
        }
    }

    #[test]
    fn tech3341_case1_and_2_stereo_sine() {
        for (sr, freq) in [(48000.0, 1000.0), (44100.0, 997.0), (96000.0, 1000.0)] {
            for level in [-23.0, -33.0] {
                let x = segments(freq, sr, &[(20.0, level)]);
                let m = stereo_measure(&x, sr);
                assert!((m.integrated() - level).abs() < 0.1, "sr {sr} I {}", m.integrated());
                assert!((m.momentary() - level).abs() < 0.1, "M {}", m.momentary());
                assert!((m.short_term() - level).abs() < 0.1, "S {}", m.short_term());
                assert!((m.max_momentary() - level).abs() < 0.1);
                assert!(m.loudness_range() < 0.1);
            }
        }
    }

    #[test]
    fn interleaved_matches_planar() {
        let sr = 48000.0;
        let x = segments(1000.0, sr, &[(5.0, -20.0)]);
        let il: Vec<f32> = x.iter().flat_map(|&v| [v, v]).collect();
        let mut m = LoudnessMeter::new(sr, 2);
        m.process_interleaved(&il);
        let p = stereo_measure(&x, sr);
        assert!((m.integrated() - p.integrated()).abs() < 1e-9);
    }

    #[test]
    fn tech3341_case3_4_gating() {
        let sr = 48000.0;
        // Case 3: -36 / -23 / -36 dBFS for 10 / 60 / 10 s → -23.0 LUFS.
        let x = segments(1000.0, sr, &[(10.0, -36.0), (60.0, -23.0), (10.0, -36.0)]);
        let m = stereo_measure(&x, sr);
        assert!((m.integrated() + 23.0).abs() < 0.1, "{}", m.integrated());
        // Case 4: -72 / -36 / -23 / -36 / -72 dBFS for 10/10/60/10/10 s → -23.0 LUFS.
        let x = segments(1000.0, sr, &[(10.0, -72.0), (10.0, -36.0), (60.0, -23.0), (10.0, -36.0), (10.0, -72.0)]);
        let m = stereo_measure(&x, sr);
        assert!((m.integrated() + 23.0).abs() < 0.1, "{}", m.integrated());
    }

    #[test]
    fn tech3341_case5_relative_gate() {
        let sr = 48000.0;
        let x = segments(1000.0, sr, &[(20.0, -26.0), (20.1, -20.0), (20.0, -26.0)]);
        let m = stereo_measure(&x, sr);
        assert!((m.integrated() + 23.0).abs() < 0.1, "{}", m.integrated());
    }

    #[test]
    fn silence_and_below_gate() {
        let sr = 48000.0;
        let mut m = LoudnessMeter::new(sr, 2);
        let z = vec![0.0f32; 48000 * 2];
        m.process(&[&z, &z]);
        assert_eq!(m.integrated(), f64::NEG_INFINITY);
        assert_eq!(m.loudness_range(), 0.0);
        let x = segments(1000.0, sr, &[(5.0, -80.0)]);
        let m = stereo_measure(&x, sr);
        assert_eq!(m.integrated(), f64::NEG_INFINITY);
    }

    #[test]
    fn tech3342_loudness_range() {
        let sr = 48000.0;
        let cases: [(&[(f64, f64)], f64); 4] = [
            (&[(20.0, -20.0), (20.0, -30.0)], 10.0),
            (&[(20.0, -20.0), (20.0, -15.0)], 5.0),
            (&[(20.0, -40.0), (20.0, -20.0)], 20.0),
            (&[(20.0, -50.0), (20.0, -35.0), (20.0, -20.0), (20.0, -35.0), (20.0, -50.0)], 15.0),
        ];
        for (segs, expect) in cases {
            let x = segments(1000.0, sr, segs);
            let m = stereo_measure(&x, sr);
            assert!((m.loudness_range() - expect).abs() < 1.0, "LRA {} expected {expect}", m.loudness_range());
        }
    }

    #[test]
    fn log_sweep_matches_analytic_k_weighting() {
        let sr = 48000.0;
        let secs = 20.0;
        let n = (sr * secs) as usize;
        let (f1, f2) = (20.0f64, 20000.0f64);
        let k = (f2 / f1).ln();
        let amp = dbfs(-20.0);
        let mut phase = 0.0f64;
        let mut x = Vec::with_capacity(n);
        let mut expected_e = 0.0;
        for i in 0..n {
            let f = f1 * (k * i as f64 / n as f64).exp();
            x.push((amp * phase.sin()) as f32);
            phase += 2.0 * PI * f / sr;
            // Stereo: 2 channels × A²/2 × |H(f)|².
            expected_e += amp * amp * k_weighting_power(f, sr);
        }
        expected_e /= n as f64;
        // Ungated average of the same signal: compare with the mean of momentary energies, i.e.
        // the integrated value is gated, so build the expectation the same way using short blocks.
        let m = stereo_measure(&x, sr);
        let expect = energy_to_lufs(expected_e);
        // Gating removes the quiet low-frequency start, so integrated ≥ ungated expectation; the
        // difference must be small for a sweep that is mostly well above the gates.
        let i = m.integrated();
        assert!(i >= expect - 0.1 && i - expect < 0.6, "I {i} vs ungated analytic {expect}");
    }

    #[test]
    fn surround_weights() {
        let sr = 48000.0;
        let x = segments(1000.0, sr, &[(10.0, -30.0)]);
        let z = vec![0.0f32; x.len()];
        // Signal only in Ls of a 5.1 layout → +1.5 dB weighting vs. front channel.
        let mut front = LoudnessMeter::new(sr, 6);
        front.process(&[&x, &z, &z, &z, &z, &z]);
        let mut surr = LoudnessMeter::new(sr, 6);
        surr.process(&[&z, &z, &z, &z, &x, &z]);
        let mut lfe = LoudnessMeter::new(sr, 6);
        lfe.process(&[&z, &z, &z, &x, &z, &z]);
        assert!((surr.integrated() - front.integrated() - 10.0 * 1.41f64.log10()).abs() < 0.01);
        assert_eq!(lfe.integrated(), f64::NEG_INFINITY);
    }

    #[test]
    fn true_peak_of_intersample_peak_signal() {
        for sr in [48000.0, 44100.0] {
            // fs/4 sine with 45° phase: every sample is at ±0.5·√2·A, true peak A (0 dBFS).
            let n = sr as usize;
            let x: Vec<f32> = (0..n).map(|i| (PI / 2.0 * i as f64 + PI / 4.0).sin() as f32).collect();
            let m = stereo_measure(&x, sr);
            assert!((20.0 * m.sample_peak().log10() + 3.01).abs() < 0.01);
            // EBU Tech 3341 true-peak tolerance: +0.2 / −0.4 dB.
            let tp = m.true_peak_dbtp();
            assert!((-0.4..=0.2).contains(&tp), "sr {sr} TP {tp}");
        }
        // A tone at an off-grid frequency with a 1 kHz-ish content: TP ≈ amplitude.
        let x = sine(997.0, dbfs(-6.0), 48000.0, 48000, 0.3);
        let m = stereo_measure(&x, 48000.0);
        assert!((m.true_peak_dbtp() + 6.0).abs() < 0.2);
        // Tech 3341 case 15-ish: 0 dBFS 1 kHz-ish tone at 48 kHz sampled off-peak.
        let x = sine(11025.0, 1.0, 44100.0, 44100, 0.0);
        let m = stereo_measure(&x, 44100.0);
        assert!((-0.4..=0.2).contains(&m.true_peak_dbtp()), "{}", m.true_peak_dbtp());
    }

    #[test]
    fn reset_clears_everything() {
        let sr = 48000.0;
        let x = segments(1000.0, sr, &[(5.0, -20.0)]);
        let mut m = stereo_measure(&x, sr);
        m.reset();
        assert_eq!(m.integrated(), f64::NEG_INFINITY);
        assert_eq!(m.true_peak(), 0.0);
        let x = segments(1000.0, sr, &[(5.0, -30.0)]);
        m.process(&[&x, &x]);
        assert!((m.integrated() + 30.0).abs() < 0.1);
    }

    #[test]
    fn normalize_helpers() {
        assert_eq!(normalize_gain_db(-30.0, -23.0), 7.0);
        assert_eq!(normalize_gain_db(f64::NEG_INFINITY, -23.0), 0.0);
        assert_eq!(normalize_gain_db_peak_limited(-30.0, -23.0, -3.0, -1.0), 2.0);
        assert_eq!(normalize_gain_db_peak_limited(-30.0, -23.0, -10.0, -1.0), 7.0);
        // Applying the gain lands on target.
        let sr = 48000.0;
        let x = segments(1000.0, sr, &[(10.0, -31.3)]);
        let m = stereo_measure(&x, sr);
        let g = 10f64.powf(normalize_gain_db(m.integrated(), -16.0) / 20.0) as f32;
        let y: Vec<f32> = x.iter().map(|v| v * g).collect();
        let m2 = stereo_measure(&y, sr);
        assert!((m2.integrated() + 16.0).abs() < 0.02);
        assert!((lufs_to_energy(energy_to_lufs(0.3)) - 0.3).abs() < 1e-12);
    }

    #[test]
    fn tiny_blocks_equal_big_blocks() {
        let sr = 48000.0;
        let x = segments(440.0, sr, &[(4.0, -18.0)]);
        let mut a = LoudnessMeter::new(sr, 1);
        a.process(&[&x]);
        let mut b = LoudnessMeter::new(sr, 1);
        b.process(&[&[]]);
        for s in x.chunks(1) {
            b.process(&[s]);
        }
        assert_eq!(a.integrated(), b.integrated());
        assert_eq!(a.true_peak(), b.true_peak());
        // Mono is summed without the dual-mono +3 dB (BS.1770).
        let expect = -0.691 + 20.0 * rms(&x).log10() + 10.0 * k_weighting_power(440.0, sr).log10();
        assert!((a.integrated() - expect).abs() < 0.1, "{} vs {expect}", a.integrated());
    }
}
