//! Video scopes (the Lumetri Scopes panel): waveform (RGB, Luma, YC), vectorscope (YUV, HLS),
//! histogram and parade (RGB, YUV), computed from a frame as density plots.
//!
//! The maths is the textbook broadcast definition: luma `Y = Kr·R + Kg·G + Kb·B` with the
//! ITU-R BT.601 / BT.709 / BT.2020 coefficients, colour differences `Cb = (B − Y) / (2(1 − Kb))`
//! and `Cr = (R − Y) / (2(1 − Kr))` (each in −0.5…0.5). Values are read straight (un-premultiplied)
//! from the frame. Every scope is a [`Scope`]: a `width × height` grid of three density planes
//! (one per trace colour) on a logarithmic scale where the busiest cell is 1, ready to be tinted
//! and drawn.
//!
//! Scales: **8-bit** maps 0…1 to the plot (0…255 graticule); **float** maps −0.25…1.25 so
//! super-white and sub-black values stay visible. **Clamp** limits the signal to 0…1 first.

use rayon::prelude::*;

use crate::Image;

/// Luma / colour-difference coefficients.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorStandard {
    Rec601,
    #[default]
    Rec709,
    Rec2020,
}

impl ColorStandard {
    pub const ALL: [ColorStandard; 3] = [ColorStandard::Rec601, ColorStandard::Rec709, ColorStandard::Rec2020];
    /// (Kr, Kb).
    pub fn coefficients(self) -> (f32, f32) {
        match self {
            ColorStandard::Rec601 => (0.299, 0.114),
            ColorStandard::Rec709 => (0.2126, 0.0722),
            ColorStandard::Rec2020 => (0.2627, 0.0593),
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            ColorStandard::Rec601 => "rec601",
            ColorStandard::Rec709 => "rec709",
            ColorStandard::Rec2020 => "rec2020",
        }
    }
    pub fn from_name(s: &str) -> Option<ColorStandard> {
        match s.to_ascii_lowercase().replace([' ', '.', '-', '_'], "").as_str() {
            "rec601" | "601" | "bt601" => Some(ColorStandard::Rec601),
            "rec709" | "709" | "bt709" => Some(ColorStandard::Rec709),
            "rec2020" | "2020" | "bt2020" => Some(ColorStandard::Rec2020),
            _ => None,
        }
    }
    /// (Y, Cb, Cr) of a straight RGB colour.
    pub fn ycbcr(self, rgb: [f32; 3]) -> [f32; 3] {
        let (kr, kb) = self.coefficients();
        let y = kr * rgb[0] + (1.0 - kr - kb) * rgb[1] + kb * rgb[2];
        [y, (rgb[2] - y) / (2.0 * (1.0 - kb)), (rgb[0] - y) / (2.0 * (1.0 - kr))]
    }
}

/// Which scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScopeKind {
    WaveformRgb,
    WaveformLuma,
    /// Luma (plane 0) and chroma amplitude `√(Cb² + Cr²)` (plane 2).
    WaveformYc,
    /// Cb across, Cr up.
    VectorscopeYuv,
    /// Hue around the circle, saturation outwards (HLS).
    VectorscopeHls,
    /// R, G, B histograms (bins across, counts up).
    Histogram,
    ParadeRgb,
    ParadeYuv,
}

impl ScopeKind {
    pub const ALL: [ScopeKind; 8] = [
        ScopeKind::WaveformRgb,
        ScopeKind::WaveformLuma,
        ScopeKind::WaveformYc,
        ScopeKind::VectorscopeYuv,
        ScopeKind::VectorscopeHls,
        ScopeKind::Histogram,
        ScopeKind::ParadeRgb,
        ScopeKind::ParadeYuv,
    ];
    pub fn name(self) -> &'static str {
        match self {
            ScopeKind::WaveformRgb => "waveformRgb",
            ScopeKind::WaveformLuma => "waveformLuma",
            ScopeKind::WaveformYc => "waveformYc",
            ScopeKind::VectorscopeYuv => "vectorscopeYuv",
            ScopeKind::VectorscopeHls => "vectorscopeHls",
            ScopeKind::Histogram => "histogram",
            ScopeKind::ParadeRgb => "paradeRgb",
            ScopeKind::ParadeYuv => "paradeYuv",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            ScopeKind::WaveformRgb => "Waveform (RGB)",
            ScopeKind::WaveformLuma => "Waveform (Luma)",
            ScopeKind::WaveformYc => "Waveform (YC)",
            ScopeKind::VectorscopeYuv => "Vectorscope YUV",
            ScopeKind::VectorscopeHls => "Vectorscope HLS",
            ScopeKind::Histogram => "Histogram",
            ScopeKind::ParadeRgb => "Parade (RGB)",
            ScopeKind::ParadeYuv => "Parade (YUV)",
        }
    }
    pub fn from_name(s: &str) -> Option<ScopeKind> {
        let n = s.to_ascii_lowercase().replace([' ', '(', ')', '_', '-'], "");
        Self::ALL.iter().copied().find(|k| k.name().to_ascii_lowercase() == n || k.label().to_ascii_lowercase().replace([' ', '(', ')'], "") == n)
    }
    /// Vectorscopes are square.
    pub fn is_vectorscope(self) -> bool {
        matches!(self, ScopeKind::VectorscopeYuv | ScopeKind::VectorscopeHls)
    }
}

/// Scope options.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScopeOpts {
    pub standard: ColorStandard,
    /// Float scale (−0.25…1.25) instead of 8-bit (0…1).
    pub float: bool,
    /// Clamp the signal to 0…1 before plotting.
    pub clamp: bool,
    /// Plot size.
    pub width: u32,
    pub height: u32,
}

impl Default for ScopeOpts {
    fn default() -> Self {
        ScopeOpts { standard: ColorStandard::Rec709, float: false, clamp: true, width: 256, height: 256 }
    }
}

impl ScopeOpts {
    /// Signal range shown on the value axis.
    pub fn range(&self) -> (f32, f32) {
        if self.float { (-0.25, 1.25) } else { (0.0, 1.0) }
    }
    /// Plot row (0 = top) of a value, or `None` when off-scale.
    fn row(&self, v: f32) -> Option<usize> {
        let (lo, hi) = self.range();
        let f = (v - lo) / (hi - lo);
        if !(0.0..=1.0).contains(&f) {
            return None;
        }
        let r = ((1.0 - f) * (self.height - 1) as f32).round() as usize;
        Some(r.min(self.height as usize - 1))
    }
}

/// A computed scope: three density planes (`planes[c][y * width + x]`, 0…1).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scope {
    pub width: u32,
    pub height: u32,
    pub planes: [Vec<f32>; 3],
    /// Raw counts before normalisation (same layout), for statistics and tests.
    pub counts: [Vec<u32>; 3],
}

impl Scope {
    fn new(w: u32, h: u32) -> Scope {
        let n = w as usize * h as usize;
        Scope { width: w, height: h, planes: [vec![0.0; n], vec![0.0; n], vec![0.0; n]], counts: [vec![0; n], vec![0; n], vec![0; n]] }
    }
    fn merge(mut self, o: Scope) -> Scope {
        for c in 0..3 {
            for (a, b) in self.counts[c].iter_mut().zip(&o.counts[c]) {
                *a += *b;
            }
        }
        self
    }
    fn normalise(&mut self) {
        // Logarithmic density (like a phosphor trace) so faint traces stay visible next to
        // dense ones.
        let max = self.counts.iter().flat_map(|p| p.iter()).copied().max().unwrap_or(0).max(1) as f32;
        let k = 1.0 / (1.0 + max).ln();
        for c in 0..3 {
            self.planes[c] = self.counts[c].iter().map(|&n| if n == 0 { 0.0 } else { (0.25 + 0.75 * (1.0 + n as f32).ln() * k).min(1.0) }).collect();
        }
    }
    /// Total count of plane `c`.
    pub fn total(&self, c: usize) -> u64 {
        self.counts[c].iter().map(|&n| n as u64).sum()
    }
    /// The cell of plane `c` with the most samples, as (x, y).
    pub fn peak(&self, c: usize) -> (u32, u32) {
        let (i, _) = self.counts[c].iter().enumerate().max_by_key(|(_, n)| **n).unwrap_or((0, &0));
        (i as u32 % self.width.max(1), i as u32 / self.width.max(1))
    }
    /// Straight RGBA8 picture: plane 0 red, 1 green, 2 blue (`mono`: all planes white).
    pub fn to_rgba8(&self, mono: bool) -> Vec<u8> {
        let n = self.width as usize * self.height as usize;
        let mut out = vec![0u8; n * 4];
        for i in 0..n {
            let (r, g, b) = (self.planes[0][i], self.planes[1][i], self.planes[2][i]);
            let (r, g, b) = if mono {
                let m = r.max(g).max(b);
                (m, m, m)
            } else {
                (r, g, b)
            };
            let a = r.max(g).max(b);
            out[i * 4] = (r.min(1.0) * 255.0) as u8;
            out[i * 4 + 1] = (g.min(1.0) * 255.0) as u8;
            out[i * 4 + 2] = (b.min(1.0) * 255.0) as u8;
            out[i * 4 + 3] = (a.min(1.0) * 255.0) as u8;
        }
        out
    }
}

/// Straight RGB of pixel `i` (transparent pixels read as black), clamped when asked.
#[inline]
fn rgb(img: &Image, i: usize, clamp: bool) -> [f32; 3] {
    let p = img.data[i];
    let mut c = if p[3] > 1e-6 { [p[0] / p[3], p[1] / p[3], p[2] / p[3]] } else { [0.0; 3] };
    // Composite over black, as a monitor shows the frame.
    let a = p[3].clamp(0.0, 1.0);
    for v in &mut c {
        *v *= a;
        if clamp {
            *v = v.clamp(0.0, 1.0);
        }
    }
    c
}

/// Compute a scope of `img`.
pub fn compute(img: &Image, kind: ScopeKind, o: &ScopeOpts) -> Scope {
    let (w, h) = (o.width.max(8), o.height.max(8));
    let o = ScopeOpts { width: w, height: h, ..*o };
    if img.is_empty() {
        return Scope::new(w, h);
    }
    let iw = img.width as usize;
    let rows: Vec<usize> = (0..img.height as usize).collect();
    let mut s = rows
        .par_chunks(16)
        .map(|chunk| {
            let mut s = Scope::new(w, h);
            for &y in chunk {
                for x in 0..iw {
                    plot(&mut s, kind, &o, rgb(img, y * iw + x, o.clamp), x as f32 / iw as f32);
                }
            }
            s
        })
        .reduce(|| Scope::new(w, h), Scope::merge);
    s.normalise();
    s
}

#[inline]
fn bump(s: &mut Scope, c: usize, x: usize, y: usize) {
    let i = y * s.width as usize + x.min(s.width as usize - 1);
    s.counts[c][i] += 1;
}

/// Add one pixel (straight RGB, at horizontal position `fx` 0…1 in the frame).
fn plot(s: &mut Scope, kind: ScopeKind, o: &ScopeOpts, c: [f32; 3], fx: f32) {
    let w = s.width as usize;
    let col = |fx: f32, x0: usize, span: usize| x0 + ((fx * span as f32) as usize).min(span - 1);
    match kind {
        ScopeKind::WaveformRgb => {
            let x = col(fx, 0, w);
            for (ch, v) in c.iter().enumerate() {
                if let Some(y) = o.row(*v) {
                    bump(s, ch, x, y);
                }
            }
        }
        ScopeKind::WaveformLuma => {
            let x = col(fx, 0, w);
            let yv = o.standard.ycbcr(c)[0];
            if let Some(y) = o.row(yv) {
                for ch in 0..3 {
                    bump(s, ch, x, y);
                }
            }
        }
        ScopeKind::WaveformYc => {
            let x = col(fx, 0, w);
            let [yv, cb, cr] = o.standard.ycbcr(c);
            if let Some(y) = o.row(yv) {
                bump(s, 0, x, y);
                bump(s, 1, x, y);
            }
            if let Some(y) = o.row((cb * cb + cr * cr).sqrt()) {
                bump(s, 2, x, y);
            }
        }
        ScopeKind::ParadeRgb | ScopeKind::ParadeYuv => {
            let third = (w / 3).max(1);
            let vals = if kind == ScopeKind::ParadeRgb {
                c
            } else {
                let [y, cb, cr] = o.standard.ycbcr(c);
                // Colour differences centred on mid-scale.
                [y, cb + 0.5, cr + 0.5]
            };
            for (ch, v) in vals.iter().enumerate() {
                let x = col(fx, ch * third, third);
                if let Some(y) = o.row(*v) {
                    // RGB parade: tinted per channel; YUV: Y white, Cb blue, Cr red.
                    match (kind, ch) {
                        (ScopeKind::ParadeYuv, 0) => {
                            for p in 0..3 {
                                bump(s, p, x, y);
                            }
                        }
                        (ScopeKind::ParadeYuv, 1) => bump(s, 2, x, y),
                        (ScopeKind::ParadeYuv, _) => bump(s, 0, x, y),
                        _ => bump(s, ch, x, y),
                    }
                }
            }
        }
        ScopeKind::VectorscopeYuv => {
            let [_, cb, cr] = o.standard.ycbcr(c);
            if let Some((x, y)) = vec_cell(s, cb, cr) {
                for ch in 0..3 {
                    bump(s, ch, x, y);
                }
            }
        }
        ScopeKind::VectorscopeHls => {
            let (hue, _, sat) = hls(c);
            let a = hue.to_radians();
            if let Some((x, y)) = vec_cell(s, sat * 0.5 * a.cos(), sat * 0.5 * a.sin()) {
                for ch in 0..3 {
                    bump(s, ch, x, y);
                }
            }
        }
        ScopeKind::Histogram => {
            let (lo, hi) = o.range();
            for (ch, v) in c.iter().enumerate() {
                let f = (v - lo) / (hi - lo);
                if (0.0..=1.0).contains(&f) {
                    let x = ((f * (w - 1) as f32).round() as usize).min(w - 1);
                    // Histograms keep their counts in row 0; the bars are drawn by `histogram_bars`.
                    bump(s, ch, x, 0);
                }
            }
        }
    }
}

/// Vectorscope cell of (u, v) in −0.5…0.5 (the disc fills the plot; +v up).
fn vec_cell(s: &Scope, u: f32, v: f32) -> Option<(usize, usize)> {
    let (w, h) = (s.width as f32, s.height as f32);
    let x = ((u + 0.5) * (w - 1.0)).round();
    let y = ((0.5 - v) * (h - 1.0)).round();
    if x < 0.0 || y < 0.0 || x >= w || y >= h {
        return None;
    }
    Some((x as usize, y as usize))
}

/// (hue degrees, lightness, saturation) of straight RGB (HLS).
pub fn hls(c: [f32; 3]) -> (f32, f32, f32) {
    let (r, g, b) = (c[0], c[1], c[2]);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) * 0.5;
    let d = max - min;
    if d < 1e-6 {
        return (0.0, l, 0.0);
    }
    let s = if l > 0.5 { d / (2.0 - max - min).max(1e-6) } else { d / (max + min).max(1e-6) };
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h * 60.0, l, s.min(1.0))
}

/// Histogram bars: per channel, `bins` counts across the value range (counts in a
/// [`ScopeKind::Histogram`] scope's first row).
pub fn histogram_bars(s: &Scope) -> [Vec<u32>; 3] {
    let w = s.width as usize;
    [s.counts[0][..w].to_vec(), s.counts[1][..w].to_vec(), s.counts[2][..w].to_vec()]
}

/// Where a vectorscope target (75 % colour bars' primaries and secondaries) sits, as (u, v) in
/// −0.5…0.5, for the graticule.
pub fn vector_targets(standard: ColorStandard, hls_mode: bool) -> Vec<(&'static str, [f32; 2])> {
    let cols: [(&str, [f32; 3]); 6] = [
        ("R", [0.75, 0.0, 0.0]),
        ("Mg", [0.75, 0.0, 0.75]),
        ("B", [0.0, 0.0, 0.75]),
        ("Cy", [0.0, 0.75, 0.75]),
        ("G", [0.0, 0.75, 0.0]),
        ("Yl", [0.75, 0.75, 0.0]),
    ];
    cols.iter()
        .map(|(n, c)| {
            if hls_mode {
                let (h, _, s) = hls(*c);
                let a = h.to_radians();
                (*n, [s * 0.5 * a.cos(), s * 0.5 * a.sin()])
            } else {
                let [_, cb, cr] = standard.ycbcr(*c);
                (*n, [cb, cr])
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, c: [f32; 3]) -> Image {
        Image::filled(w, h, [c[0], c[1], c[2], 1.0])
    }

    #[test]
    fn waveform_of_flat_grey_is_one_line() {
        let img = solid(64, 32, [0.5, 0.5, 0.5]);
        let o = ScopeOpts { width: 64, height: 101, ..Default::default() };
        let s = compute(&img, ScopeKind::WaveformLuma, &o);
        // 0.5 sits on row 50 of 0..=100; every sample lands there.
        assert_eq!(s.total(0), 64 * 32);
        for x in 0..64 {
            assert_eq!(s.counts[0][50 * 64 + x], 32);
        }
    }

    #[test]
    fn waveform_rgb_ramps_follow_columns() {
        // Horizontal ramp: column x has value x / (w - 1).
        let w = 101;
        let mut img = Image::new(w, 4);
        for y in 0..4 {
            for x in 0..w {
                let v = x as f32 / (w - 1) as f32;
                img.set(x, y, [v, 1.0 - v, 0.0, 1.0]);
            }
        }
        let o = ScopeOpts { width: w, height: 101, ..Default::default() };
        let s = compute(&img, ScopeKind::WaveformRgb, &o);
        for x in 0..w as usize {
            // Red: row 100 - x; green: row x.
            assert_eq!(s.counts[0][(100 - x) * w as usize + x], 4, "red column {x}");
            assert_eq!(s.counts[1][x * w as usize + x], 4, "green column {x}");
        }
    }

    #[test]
    fn luma_coefficients_per_standard() {
        let red = [1.0, 0.0, 0.0];
        assert!((ColorStandard::Rec601.ycbcr(red)[0] - 0.299).abs() < 1e-6);
        assert!((ColorStandard::Rec709.ycbcr(red)[0] - 0.2126).abs() < 1e-6);
        assert!((ColorStandard::Rec2020.ycbcr(red)[0] - 0.2627).abs() < 1e-6);
        // Pure red has Cr = +0.5 in every standard, Cb negative.
        for st in ColorStandard::ALL {
            let [_, cb, cr] = st.ycbcr(red);
            assert!((cr - 0.5).abs() < 1e-6);
            assert!(cb < 0.0);
            // White and grey have no chroma.
            let [_, cb, cr] = st.ycbcr([0.6, 0.6, 0.6]);
            assert!(cb.abs() < 1e-6 && cr.abs() < 1e-6);
        }
    }

    #[test]
    fn vectorscope_places_red_on_its_target() {
        let img = solid(16, 16, [1.0, 0.0, 0.0]);
        let o = ScopeOpts { width: 101, height: 101, ..Default::default() };
        let s = compute(&img, ScopeKind::VectorscopeYuv, &o);
        let [_, cb, cr] = ColorStandard::Rec709.ycbcr([1.0, 0.0, 0.0]);
        let (x, y) = s.peak(0);
        assert_eq!(x, ((cb + 0.5) * 100.0).round() as u32);
        assert_eq!(y, ((0.5 - cr) * 100.0).round() as u32);
        // Grey sits in the centre.
        let g = compute(&solid(8, 8, [0.3, 0.3, 0.3]), ScopeKind::VectorscopeYuv, &o);
        assert_eq!(g.peak(0), (50, 50));
        // HLS: fully saturated green at 120° on the rim.
        let h = compute(&solid(8, 8, [0.0, 1.0, 0.0]), ScopeKind::VectorscopeHls, &o);
        let a = 120f32.to_radians();
        assert_eq!(h.peak(0), (((0.5 * a.cos() + 0.5) * 100.0).round() as u32, ((0.5 - 0.5 * a.sin()) * 100.0).round() as u32));
    }

    #[test]
    fn histogram_counts_and_clamp() {
        // Half black, half white.
        let mut img = Image::new(10, 10);
        for y in 0..10 {
            for x in 0..10 {
                let v = if x < 5 { 0.0 } else { 1.0 };
                img.set(x, y, [v, v, v, 1.0]);
            }
        }
        let o = ScopeOpts { width: 256, height: 64, ..Default::default() };
        let s = compute(&img, ScopeKind::Histogram, &o);
        let bars = histogram_bars(&s);
        for b in &bars {
            assert_eq!(b[0], 50);
            assert_eq!(b[255], 50);
            assert_eq!(b.iter().sum::<u32>(), 100);
        }
        // Super-white: off scale in 8-bit unclamped, on the float scale, pinned to 1 clamped.
        let hot = solid(4, 4, [1.2, 1.2, 1.2]);
        let unclamped = ScopeOpts { clamp: false, ..o };
        assert_eq!(compute(&hot, ScopeKind::Histogram, &unclamped).total(0), 0);
        let fl = ScopeOpts { clamp: false, float: true, ..o };
        let f = histogram_bars(&compute(&hot, ScopeKind::Histogram, &fl));
        let expect = (((1.2 + 0.25) / 1.5) * 255.0f32).round() as usize;
        assert_eq!(f[0][expect], 16);
        let c = histogram_bars(&compute(&hot, ScopeKind::Histogram, &o));
        assert_eq!(c[0][255], 16);
    }

    #[test]
    fn parade_splits_channels_into_thirds() {
        let img = solid(30, 3, [1.0, 0.5, 0.0]);
        let o = ScopeOpts { width: 90, height: 101, ..Default::default() };
        let s = compute(&img, ScopeKind::ParadeRgb, &o);
        // Red trace in the first third at the top, green mid-height in the second, blue at the
        // bottom of the third.
        let (rx, ry) = s.peak(0);
        let (gx, gy) = s.peak(1);
        let (bx, by) = s.peak(2);
        assert!(rx < 30 && ry == 0);
        assert!((30..60).contains(&gx) && gy == 50);
        assert!((60..90).contains(&bx) && by == 100);
        let yuv = compute(&solid(30, 3, [0.5, 0.5, 0.5]), ScopeKind::ParadeYuv, &o);
        // Grey: Y = 0.5 and both colour differences at mid-scale.
        assert_eq!(yuv.peak(1).1, 50);
        assert_eq!(yuv.peak(2).1, 50);
    }

    #[test]
    fn yc_waveform_has_chroma_trace() {
        let img = solid(8, 8, [1.0, 0.0, 0.0]);
        let o = ScopeOpts { width: 8, height: 101, ..Default::default() };
        let s = compute(&img, ScopeKind::WaveformYc, &o);
        let [y, cb, cr] = ColorStandard::Rec709.ycbcr([1.0, 0.0, 0.0]);
        assert_eq!(s.peak(0).1, ((1.0 - y) * 100.0).round() as u32);
        assert_eq!(s.peak(2).1, ((1.0 - (cb * cb + cr * cr).sqrt()) * 100.0).round() as u32);
    }
}
