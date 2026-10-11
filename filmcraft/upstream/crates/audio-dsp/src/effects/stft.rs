//! Streaming STFT overlap-add framework shared by the spectral effects.

use crate::fft::Fft;
use std::f64::consts::PI;

/// Per-channel streaming buffers. Latency is exactly `n` samples.
#[derive(Clone, Debug)]
pub(crate) struct StftChannel {
    input: Vec<f32>,
    output: Vec<f32>,
    pos: usize,
}

impl StftChannel {
    pub(crate) fn new(n: usize) -> Self {
        StftChannel { input: vec![0.0; n], output: vec![0.0; n], pos: 0 }
    }
    pub(crate) fn reset(&mut self) {
        self.input.iter_mut().for_each(|v| *v = 0.0);
        self.output.iter_mut().for_each(|v| *v = 0.0);
        self.pos = 0;
    }
    /// Push one sample and pop one output sample. Every `hop` samples `frame` is called with the
    /// last `n` input samples and must write an `n`-sample (windowed) frame that is overlap-added.
    #[inline]
    pub(crate) fn tick(&mut self, x: f32, hop: usize, frame: &mut impl FnMut(&[f32], &mut [f32])) -> f32 {
        let n = self.input.len();
        self.input[n - hop + self.pos] = x;
        let y = self.output[self.pos];
        self.pos += 1;
        if self.pos == hop {
            self.pos = 0;
            self.output.copy_within(hop.., 0);
            self.output[n - hop..].iter_mut().for_each(|v| *v = 0.0);
            frame(&self.input, &mut self.output);
            self.input.copy_within(hop.., 0);
        }
        y
    }
}

/// Periodic Hann window.
pub(crate) fn hann(n: usize) -> Vec<f32> {
    (0..n).map(|i| (0.5 - 0.5 * (2.0 * PI * i as f64 / n as f64).cos()) as f32).collect()
}

/// FFT size for spectral effects at a sample rate (~43 ms at 48 kHz).
pub(crate) fn fft_size(sample_rate: f32) -> usize {
    if sample_rate <= 50_000.0 {
        2048
    } else if sample_rate <= 100_000.0 {
        4096
    } else {
        8192
    }
}

/// Shared spectral scratch.
#[derive(Clone, Debug)]
pub(crate) struct Spectral {
    pub fft: Fft,
    pub window: Vec<f32>,
    pub re: Vec<f32>,
    pub im: Vec<f32>,
}

impl Spectral {
    /// `sqrt_window`: use √Hann for analysis and synthesis (sums to a constant at 75 % overlap);
    /// otherwise Hann for both.
    pub(crate) fn new(n: usize, sqrt_window: bool) -> Self {
        let mut window = hann(n);
        if sqrt_window {
            window.iter_mut().for_each(|w| *w = w.sqrt());
        }
        Spectral { fft: Fft::new(n), window, re: vec![0.0; n], im: vec![0.0; n] }
    }
    /// Window `input` into re/im and run the forward FFT.
    pub(crate) fn analyse(&mut self, input: &[f32]) {
        for i in 0..input.len() {
            self.re[i] = input[i] * self.window[i];
            self.im[i] = 0.0;
        }
        self.fft.forward(&mut self.re, &mut self.im);
    }
    /// Enforce Hermitian symmetry from bins 0..=n/2, inverse FFT, window, scale and add to `out`.
    pub(crate) fn synthesise_add(&mut self, out: &mut [f32], scale: f32) {
        let n = self.re.len();
        self.im[0] = 0.0;
        self.im[n / 2] = 0.0;
        for k in 1..n / 2 {
            self.re[n - k] = self.re[k];
            self.im[n - k] = -self.im[k];
        }
        self.fft.inverse(&mut self.re, &mut self.im);
        for i in 0..n {
            out[i] += self.re[i] * self.window[i] * scale;
        }
    }
}
