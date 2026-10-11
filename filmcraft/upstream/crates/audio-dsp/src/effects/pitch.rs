//! Pitch shifter: phase vocoder with peak-region shifting and identity phase locking.
//!
//! Every STFT frame (8× overlap) is analysed for spectral peaks; each peak's region of influence
//! (down to the magnitude minima between peaks) is moved as a block so that the peak lands on
//! `k·ratio`, preserving the window's main-lobe shape. All bins of a region receive the same phase
//! rotation, accumulated per frame from the peak's instantaneous frequency so the partial's
//! frequency is scaled exactly by `ratio` while the within-lobe phase relations stay coherent
//! (low phasiness). Duration is unchanged. At ratio 1 the output is the input delayed by the
//! frame size.

use super::stft::{Spectral, StftChannel, fft_size};
use crate::{AudioEffect, ParamSpec, ParamValues, Unit, block_len, param_plumbing};
use std::f32::consts::PI;

const OVERLAP: usize = 8;

#[derive(Clone, Debug)]
struct PvState {
    last_phase: Vec<f32>,
    /// Accumulated phase rotation per output bin (from the previous frame).
    rot: Vec<f32>,
    new_rot: Vec<f32>,
}

/// Frame scratch shared by all channels.
#[derive(Clone, Debug)]
struct Scratch {
    mag: Vec<f32>,
    phase: Vec<f32>,
    out_re: Vec<f32>,
    out_im: Vec<f32>,
    peaks: Vec<usize>,
}

/// Semitone pitch shifter (±12 st, fractional allowed).
pub struct PitchShifter {
    pv: ParamValues,
    n: usize,
    hop: usize,
    spec: Spectral,
    stft: Vec<StftChannel>,
    state: Vec<PvState>,
    scratch: Scratch,
    target_ratio: f32,
    /// Ratio used by the current frame (glides per frame towards the target).
    ratio: f32,
}

/// Wrap a phase to (−π, π].
#[inline]
fn wrap(p: f32) -> f32 {
    p - 2.0 * PI * ((p + PI) / (2.0 * PI)).floor()
}

impl PitchShifter {
    pub const PARAMS: &'static [ParamSpec] = &[ParamSpec::new("semitones", "Pitch", -12.0, 12.0, 0.0, Unit::Semitones)];

    pub fn new(sample_rate: f32, channels: usize) -> Self {
        let n = fft_size(sample_rate);
        let bins = n / 2 + 1;
        let ch = channels.max(1);
        let mut s = PitchShifter {
            pv: ParamValues::new(Self::PARAMS),
            n,
            hop: n / OVERLAP,
            spec: Spectral::new(n, false),
            stft: vec![StftChannel::new(n); ch],
            state: vec![PvState { last_phase: vec![0.0; bins], rot: vec![0.0; bins], new_rot: vec![0.0; bins] }; ch],
            scratch: Scratch {
                mag: vec![0.0; bins],
                phase: vec![0.0; bins],
                out_re: vec![0.0; bins],
                out_im: vec![0.0; bins],
                peaks: Vec::with_capacity(bins),
            },
            target_ratio: 1.0,
            ratio: 1.0,
        };
        s.apply_params(true);
        s
    }

    fn apply_params(&mut self, snap: bool) {
        self.target_ratio = 2f32.powf(self.pv.v("semitones") / 12.0);
        if snap {
            self.ratio = self.target_ratio;
        }
    }
}

/// Process one analysed frame (spectrum in `spec.re/im`) into the shifted spectrum.
fn shift_frame(spec: &mut Spectral, st: &mut PvState, sc: &mut Scratch, ratio: f32, expect: f32) {
    let bins = sc.mag.len();
    let mut max_mag = 0.0f32;
    for k in 0..bins {
        let (re, im) = (spec.re[k], spec.im[k]);
        sc.mag[k] = (re * re + im * im).sqrt();
        sc.phase[k] = im.atan2(re);
        max_mag = max_mag.max(sc.mag[k]);
        sc.out_re[k] = 0.0;
        sc.out_im[k] = 0.0;
        st.new_rot[k] = 0.0;
    }
    // Peaks: local maxima over ±2 bins, above −100 dB of the frame maximum.
    sc.peaks.clear();
    let floor = max_mag * 1e-5;
    for k in 0..bins {
        let m = sc.mag[k];
        if m <= floor {
            continue;
        }
        let lo = k.saturating_sub(2);
        let hi = (k + 2).min(bins - 1);
        if (lo..k).all(|j| sc.mag[j] < m) && (k + 1..=hi).all(|j| sc.mag[j] <= m) {
            sc.peaks.push(k);
        }
    }
    let np = sc.peaks.len();
    let mut start = 0usize;
    for i in 0..np {
        let p = sc.peaks[i];
        // Region end: magnitude minimum between this peak and the next.
        let end = if i + 1 < np {
            let q = sc.peaks[i + 1];
            let mut mi = p + 1;
            for j in p + 1..q {
                if sc.mag[j] < sc.mag[mi] {
                    mi = j;
                }
            }
            mi
        } else {
            bins
        };
        // Instantaneous frequency of the peak (bins).
        let d = wrap(sc.phase[p] - st.last_phase[p] - p as f32 * expect);
        let f = p as f32 + d / expect;
        let target = (p as f32 * ratio).round() as isize;
        let delta = target - p as isize;
        let prev_rot = if (0..bins as isize).contains(&target) { st.rot[target as usize] } else { 0.0 };
        let rot = wrap(prev_rot + (ratio - 1.0) * f * expect);
        for k in start..end {
            let t = k as isize + delta;
            if t < 0 || t >= bins as isize {
                continue;
            }
            let t = t as usize;
            let (s, c) = (sc.phase[k] + rot).sin_cos();
            sc.out_re[t] += sc.mag[k] * c;
            sc.out_im[t] += sc.mag[k] * s;
            st.new_rot[t] = rot;
        }
        start = end;
    }
    for k in 0..bins {
        st.last_phase[k] = sc.phase[k];
        spec.re[k] = sc.out_re[k];
        spec.im[k] = sc.out_im[k];
    }
    std::mem::swap(&mut st.rot, &mut st.new_rot);
}

impl AudioEffect for PitchShifter {
    param_plumbing!("pitch_shifter");
    fn latency(&self) -> usize {
        self.n
    }
    fn reset(&mut self) {
        self.stft.iter_mut().for_each(StftChannel::reset);
        for s in &mut self.state {
            s.last_phase.iter_mut().for_each(|v| *v = 0.0);
            s.rot.iter_mut().for_each(|v| *v = 0.0);
        }
        self.ratio = self.target_ratio;
    }
    fn process(&mut self, channels: &mut [&mut [f32]]) {
        let len = block_len(channels);
        let nch = channels.len().min(self.stft.len());
        let (n, hop) = (self.n, self.hop);
        // Hann² summed over 8× overlap = 3/8 · 8 = 3.
        let scale = 1.0 / (n as f32 * 3.0 / 8.0 * OVERLAP as f32);
        let expect = 2.0 * PI * hop as f32 / n as f32;
        // Sample-major so every channel hits its frame boundary on the same sample and the ratio
        // glides identically for all channels (once per frame).
        for i in 0..len {
            let mut frame_done = false;
            for ch in 0..nch {
                let st = &mut self.state[ch];
                let spec = &mut self.spec;
                let sc = &mut self.scratch;
                let ratio = self.ratio;
                let mut frame = |input: &[f32], out: &mut [f32]| {
                    spec.analyse(input);
                    shift_frame(spec, st, sc, ratio, expect);
                    spec.synthesise_add(out, scale);
                    frame_done = true;
                };
                channels[ch][i] = self.stft[ch].tick(channels[ch][i], hop, &mut frame);
            }
            if frame_done {
                self.ratio += (self.target_ratio - self.ratio) * 0.5;
                if (self.ratio - self.target_ratio).abs() < 1e-5 {
                    self.ratio = self.target_ratio;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;

    const SR: f32 = 48000.0;

    fn shifted(semis: f32, f_in: f64) -> (Vec<f32>, usize) {
        let mut p = PitchShifter::new(SR, 2);
        p.set_param("semitones", semis);
        p.reset();
        let n = SR as usize * 2;
        let mut l = sine(f_in, 0.5, SR as f64, n, 0.0);
        let mut r = l.clone();
        p.process(&mut [&mut l, &mut r]);
        assert_eq!(l, r);
        (l, p.latency())
    }

    #[test]
    fn octave_up_and_down() {
        for (semis, f_in, f_out) in [(12.0, 440.0, 880.0), (-12.0, 440.0, 220.0), (7.0, 300.0, 300.0 * 1.498_307)] {
            let (y, lat) = shifted(semis, f_in);
            let tail = &y[lat + 4096..];
            let a_out = tone_amplitude(tail, f_out, SR as f64);
            let a_in = tone_amplitude(tail, f_in, SR as f64);
            assert!(db(a_out / 0.5).abs() < 3.0, "{semis}: level {} dB", db(a_out / 0.5));
            assert!(db(a_in / a_out) < -30.0, "{semis}: residual {} dB", db(a_in / a_out));
            // Output is a clean tone: almost all energy at the target frequency.
            let r = rms(tail);
            assert!((db(a_out / std::f64::consts::SQRT_2 / r)).abs() < 1.0);
        }
    }

    #[test]
    fn zero_shift_is_delayed_identity() {
        let (y, lat) = shifted(0.0, 440.0);
        let x = sine(440.0, 0.5, SR as f64, y.len(), 0.0);
        for i in lat + 2048..y.len() {
            assert!((y[i] - x[i - lat]).abs() < 2e-3, "i {i}");
        }
    }
}
