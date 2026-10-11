//! Colour reduction for palette formats (PNG-8, GIF): a palette of at most 256 colours chosen by
//! median cut over a 15-bit histogram (refined by a few k-means passes) or a fixed palette, then
//! each pixel mapped to its nearest entry with optional dithering (Floyd–Steinberg diffusion, an
//! 8×8 ordered pattern, or noise). Pixels under half opacity can become one transparent entry;
//! the others are blended over the matte colour.

use std::collections::HashMap;

/// How the palette is chosen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Reduction {
    /// Median cut weighted towards the colours the eye tells apart best.
    Perceptual,
    /// Like perceptual, keeping rarer distinct colours too, and snapping near web colours to them.
    #[default]
    Selective,
    /// Median cut by how often colours occur.
    Adaptive,
    /// The 216 web-safe colours (the most used of them when fewer are allowed).
    Web,
    /// Black and white.
    BlackWhite,
    /// Evenly spaced greys.
    Gray,
}

impl Reduction {
    pub const ALL: [Reduction; 6] =
        [Reduction::Perceptual, Reduction::Selective, Reduction::Adaptive, Reduction::Web, Reduction::BlackWhite, Reduction::Gray];

    pub fn id(self) -> &'static str {
        match self {
            Reduction::Perceptual => "perceptual",
            Reduction::Selective => "selective",
            Reduction::Adaptive => "adaptive",
            Reduction::Web => "web",
            Reduction::BlackWhite => "blackWhite",
            Reduction::Gray => "gray",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Reduction::Perceptual => "Perceptual",
            Reduction::Selective => "Selective",
            Reduction::Adaptive => "Adaptive",
            Reduction::Web => "Web",
            Reduction::BlackWhite => "Black & White",
            Reduction::Gray => "Grayscale",
        }
    }

    pub fn from_id(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.id().eq_ignore_ascii_case(s) || r.label().eq_ignore_ascii_case(s))
    }

    /// Channel weights of colour distances.
    fn weights(self) -> [f32; 3] {
        match self {
            Reduction::Perceptual | Reduction::Selective => [0.55, 0.77, 0.34],
            _ => [1.0; 3],
        }
    }
}

/// How colours between palette entries are approximated.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Dither {
    None,
    /// Floyd–Steinberg error diffusion.
    #[default]
    Diffusion,
    /// An 8×8 ordered (Bayer) pattern.
    Pattern,
    /// Random noise.
    Noise,
}

impl Dither {
    pub const ALL: [Dither; 4] = [Dither::None, Dither::Diffusion, Dither::Pattern, Dither::Noise];

    pub fn id(self) -> &'static str {
        match self {
            Dither::None => "none",
            Dither::Diffusion => "diffusion",
            Dither::Pattern => "pattern",
            Dither::Noise => "noise",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Dither::None => "No Dither",
            Dither::Diffusion => "Diffusion",
            Dither::Pattern => "Pattern",
            Dither::Noise => "Noise",
        }
    }

    pub fn from_id(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|d| d.id().eq_ignore_ascii_case(s) || d.label().eq_ignore_ascii_case(s))
    }
}

/// The palette options of a raster export (PNG-8, GIF).
#[derive(Clone, Debug, PartialEq)]
pub struct PaletteOptions {
    /// Most palette entries, the transparent one included: 2–256.
    pub colors: u16,
    pub reduction: Reduction,
    pub dither: Dither,
    /// Dither strength 0–100.
    pub dither_amount: u8,
    /// Pixels under half opacity become a transparent entry; otherwise every pixel is opaque.
    pub transparency: bool,
    /// The colour partly transparent pixels are blended over. `None`: with transparency, they
    /// keep their own colour (hard edges); without, white.
    pub matte: Option<[u8; 3]>,
}

impl Default for PaletteOptions {
    fn default() -> Self {
        Self {
            colors: 256,
            reduction: Reduction::Selective,
            dither: Dither::Diffusion,
            dither_amount: 100,
            transparency: true,
            matte: Some([255; 3]),
        }
    }
}

/// An image as palette indices.
#[derive(Clone, Debug, PartialEq)]
pub struct Indexed {
    pub width: u32,
    pub height: u32,
    pub palette: Vec<[u8; 3]>,
    /// The transparent entry (always 0 when there is one).
    pub transparent: Option<u8>,
    /// One index per pixel, row-major.
    pub indices: Vec<u8>,
}

/// Reduce straight-alpha RGBA pixels (`width`×`height`) to a palette.
pub fn quantize(rgba: &[u8], width: u32, height: u32, o: &PaletteOptions) -> Indexed {
    quantize_locked(rgba, width, height, o, &[])
}

/// [`quantize`], keeping the `locked` colours in a reduced palette (the colour table's locked
/// colours: reducing the colours further never drops them). Art with few enough colours keeps
/// them all anyway; locked colours no pixel ends up using are left out.
pub fn quantize_locked(rgba: &[u8], width: u32, height: u32, o: &PaletteOptions, locked: &[[u8; 3]]) -> Indexed {
    let px = rgba.as_chunks::<4>().0;
    let clear = |p: &[u8; 4]| o.transparency && p[3] < 128;
    let gray = matches!(o.reduction, Reduction::Gray | Reduction::BlackWhite);
    let matte = o.matte.unwrap_or([255; 3]);
    let blend = o.matte.is_some() || !o.transparency;
    // The opaque colours: blended over the matte, greys for the grey reductions.
    let colors: Vec<[u8; 3]> = px
        .iter()
        .map(|p| {
            let a = if blend { p[3] as u32 } else { 255 };
            let c = [0, 1, 2].map(|i| ((p[i] as u32 * a + matte[i] as u32 * (255 - a) + 127) / 255) as u8);
            if gray { [luma(c); 3] } else { c }
        })
        .collect();
    let has_clear = px.iter().any(clear);
    let k = (o.colors.clamp(2, 256) as usize - usize::from(has_clear)).max(1);
    let opaque = || colors.iter().zip(px).filter(|(_, p)| !clear(p)).map(|(c, _)| *c);
    // Locked colours take their slots first (at least one is left for the reduction).
    let mut lock: Vec<[u8; 3]> = Vec::with_capacity(locked.len().min(k));
    for c in locked {
        if lock.len() + 1 < k && !lock.contains(c) {
            lock.push(*c);
        }
    }
    let free = k - lock.len();
    let (mut palette, exact) = match o.reduction {
        Reduction::BlackWhite => (vec![[0; 3], [255; 3]], false),
        Reduction::Gray => ((0..free).map(|i| [(i * 255 / (free - 1).max(1)) as u8; 3]).collect(), false),
        Reduction::Web => (web(opaque(), free), false),
        r => match distinct(opaque(), k) {
            Some(exact) => (exact, true),
            None => (median_cut(opaque(), free, r), false),
        },
    };
    if !exact && !lock.is_empty() {
        palette.retain(|c| !lock.contains(c));
        lock.append(&mut palette);
        lock.truncate(k);
        palette = lock;
    }
    if palette.is_empty() {
        palette.push(matte);
    }
    let weights = o.reduction.weights();
    let dither = if exact { Dither::None } else { o.dither };
    let mut indices = map(&colors, width as usize, &palette, weights, dither, f32::from(o.dither_amount.min(100)) / 100.0, |i| clear(&px[i]));
    // Drop the entries no pixel uses (fixed palettes), keeping the order.
    let mut used = [false; 256];
    for (i, &ix) in indices.iter().enumerate() {
        if !clear(&px[i]) {
            used[ix as usize] = true;
        }
    }
    let mut remap = [0u8; 256];
    let mut kept = Vec::with_capacity(palette.len() + 1);
    if has_clear {
        kept.push(matte);
    }
    for (i, c) in palette.iter().enumerate() {
        if used[i] {
            remap[i] = kept.len() as u8;
            kept.push(*c);
        }
    }
    for (i, ix) in indices.iter_mut().enumerate() {
        *ix = if clear(&px[i]) { 0 } else { remap[*ix as usize] };
    }
    Indexed { width, height, palette: kept, transparent: has_clear.then_some(0), indices }
}

/// Lightness as a grey value (Rec. 601 luma).
pub(super) fn luma([r, g, b]: [u8; 3]) -> u8 {
    ((r as u32 * 299 + g as u32 * 587 + b as u32 * 114 + 500) / 1000) as u8
}

/// The colours themselves, most used first, when there are at most `k`.
fn distinct(colors: impl Iterator<Item = [u8; 3]>, k: usize) -> Option<Vec<[u8; 3]>> {
    let mut seen: HashMap<[u8; 3], u32> = HashMap::with_capacity(k + 1);
    for c in colors {
        *seen.entry(c).or_insert(0) += 1;
        if seen.len() > k {
            return None;
        }
    }
    let mut v: Vec<([u8; 3], u32)> = seen.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    Some(v.into_iter().map(|(c, _)| c).collect())
}

/// A web-safe colour: each channel a multiple of 51.
pub fn web_safe(c: [u8; 3]) -> [u8; 3] {
    c.map(|v| ((v as u32 + 25) / 51 * 51) as u8)
}

/// The `k` most used web-safe colours.
fn web(colors: impl Iterator<Item = [u8; 3]>, k: usize) -> Vec<[u8; 3]> {
    let mut count: HashMap<[u8; 3], u32> = HashMap::new();
    for c in colors {
        *count.entry(web_safe(c)).or_insert(0) += 1;
    }
    let mut v: Vec<([u8; 3], u32)> = count.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v.into_iter().take(k).map(|(c, _)| c).collect()
}

/// One histogram cell: its mean colour and pixel count.
#[derive(Clone, Copy)]
struct Cell {
    mean: [f32; 3],
    count: u32,
}

fn dist(a: [f32; 3], b: [f32; 3], w: [f32; 3]) -> f32 {
    (0..3).map(|i| (w[i] * (a[i] - b[i])).powi(2)).sum()
}

/// `k` colours by median cut over a 5-bit-per-channel histogram, then k-means passes.
fn median_cut(colors: impl Iterator<Item = [u8; 3]>, k: usize, r: Reduction) -> Vec<[u8; 3]> {
    let mut hist = vec![(0u32, [0u64; 3]); 1 << 15];
    for c in colors {
        let h = &mut hist[(c[0] as usize >> 3) << 10 | (c[1] as usize >> 3) << 5 | c[2] as usize >> 3];
        h.0 += 1;
        for (sum, v) in h.1.iter_mut().zip(c) {
            *sum += v as u64;
        }
    }
    let mut cells: Vec<Cell> = hist.iter().filter(|h| h.0 > 0).map(|&(n, s)| Cell { mean: s.map(|v| v as f32 / n as f32), count: n }).collect();
    let w = r.weights();
    // How much a box gains from a split: its widest weighted range, times its pixel count (how
    // often its colours occur) or a root of it (keeping rarer colours apart too).
    let score = |b: &[Cell]| -> (f32, usize) {
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        let mut n = 0u64;
        for c in b {
            for i in 0..3 {
                lo[i] = lo[i].min(c.mean[i]);
                hi[i] = hi[i].max(c.mean[i]);
            }
            n += c.count as u64;
        }
        let axis = (0..3).max_by(|&a, &b| (w[a] * (hi[a] - lo[a])).total_cmp(&(w[b] * (hi[b] - lo[b])))).unwrap_or(0);
        let range = w[axis] * (hi[axis] - lo[axis]);
        let weight = match r {
            Reduction::Adaptive => n as f32,
            Reduction::Perceptual => (n as f32).sqrt(),
            _ => (n as f32).sqrt().sqrt(),
        };
        (if b.len() < 2 { 0.0 } else { range * weight }, axis)
    };
    // (start, end, score, axis) over `cells`.
    let mut boxes = vec![{
        let (s, axis) = score(&cells);
        (0, cells.len(), s, axis)
    }];
    while boxes.len() < k {
        let Some(at) = boxes.iter().enumerate().max_by(|x, y| x.1.2.total_cmp(&y.1.2)).map(|(i, _)| i) else { break };
        let (a, b, s, axis) = boxes[at];
        if s <= 0.0 {
            break;
        }
        let part = &mut cells[a..b];
        part.sort_by(|x, y| x.mean[axis].total_cmp(&y.mean[axis]));
        // Split at the median pixel, leaving at least one cell on each side.
        let total: u64 = part.iter().map(|c| c.count as u64).sum();
        let mut acc = 0u64;
        let mut mid = 1;
        for (i, c) in part.iter().enumerate() {
            acc += c.count as u64;
            if acc * 2 >= total {
                mid = (i + 1).clamp(1, part.len() - 1);
                break;
            }
        }
        for (x, y) in [(a, a + mid), (a + mid, b)] {
            let (s, axis) = score(&cells[x..y]);
            if x == a {
                boxes[at] = (x, y, s, axis);
            } else {
                boxes.push((x, y, s, axis));
            }
        }
    }
    let mean = |b: &[Cell]| {
        let n: f64 = b.iter().map(|c| c.count as f64).sum();
        [0, 1, 2].map(|i| (b.iter().map(|c| c.mean[i] as f64 * c.count as f64).sum::<f64>() / n.max(1.0)) as f32)
    };
    let mut palette: Vec<[f32; 3]> = boxes.iter().map(|&(a, b, ..)| mean(&cells[a..b])).collect();
    // k-means: move each entry to the mean of the cells nearest to it.
    for _ in 0..3 {
        let mut sums = vec![([0f64; 3], 0f64); palette.len()];
        for c in &cells {
            let i = nearest(&palette, c.mean, w);
            for ch in 0..3 {
                sums[i].0[ch] += c.mean[ch] as f64 * c.count as f64;
            }
            sums[i].1 += c.count as f64;
        }
        for (p, (s, n)) in palette.iter_mut().zip(sums) {
            if n > 0.0 {
                *p = s.map(|v| (v / n) as f32);
            }
        }
    }
    let mut out: Vec<[u8; 3]> = palette.iter().map(|p| p.map(|v| v.round().clamp(0.0, 255.0) as u8)).collect();
    if r == Reduction::Selective {
        // Colours close to a web-safe one become it (they display without dithering in a browser
        // limited to those).
        for c in &mut out {
            let s = web_safe(*c);
            if (0..3).all(|i| c[i].abs_diff(s[i]) <= 6) {
                *c = s;
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

fn nearest(palette: &[[f32; 3]], c: [f32; 3], w: [f32; 3]) -> usize {
    palette.iter().enumerate().min_by(|a, b| dist(*a.1, c, w).total_cmp(&dist(*b.1, c, w))).map_or(0, |(i, _)| i)
}

/// The 8×8 ordered-dither threshold at (x, y), in 0..1.
fn bayer(x: usize, y: usize) -> f32 {
    let mut v = 0;
    for i in 0..3 {
        let (xb, yb) = ((x >> i) & 1, (y >> i) & 1);
        v |= ((xb ^ yb) << 1 | yb) << (2 * (2 - i));
    }
    (v as f32 + 0.5) / 64.0
}

/// A noise threshold at (x, y), in 0..1 (a hash, so exports are reproducible).
fn noise(x: usize, y: usize) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x9E37_79B1) ^ (y as u32).wrapping_mul(0x85EB_CA77);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    (h >> 8) as f32 / (1 << 24) as f32
}

/// Map each colour to its nearest palette entry, dithering by `amount` (0–1). Pixels `skip`
/// names (transparent) get index 0 and take no diffused error.
fn map(colors: &[[u8; 3]], width: usize, palette: &[[u8; 3]], w: [f32; 3], dither: Dither, amount: f32, skip: impl Fn(usize) -> bool) -> Vec<u8> {
    let pal: Vec<[f32; 3]> = palette.iter().map(|c| c.map(f32::from)).collect();
    // Nearest entries by colour: exact colours without dithering, a 6-bit grid with it.
    let mut exact: HashMap<[u8; 3], u8> = HashMap::new();
    let mut grid = vec![u16::MAX; if dither == Dither::None { 0 } else { 1 << 18 }];
    let mut lookup = |c: [f32; 3]| -> u8 {
        let q = c.map(|v| v.round().clamp(0.0, 255.0) as u8);
        if grid.is_empty() {
            return *exact.entry(q).or_insert_with(|| nearest(&pal, c, w) as u8);
        }
        let key = (q[0] as usize >> 2) << 12 | (q[1] as usize >> 2) << 6 | q[2] as usize >> 2;
        match grid.get(key) {
            Some(&v) if v != u16::MAX => v as u8,
            _ => {
                let i = nearest(&pal, c, w) as u8;
                if let Some(g) = grid.get_mut(key) {
                    *g = i as u16;
                }
                i
            }
        }
    };
    // Ordered and noise dithering spread a pixel by about one palette step.
    let step = 255.0 / (pal.len() as f32).cbrt().max(1.0) * amount;
    let width = width.max(1);
    let mut out = vec![0u8; colors.len()];
    let mut err = vec![[0f32; 3]; 2 * (width + 2)];
    for (y, row) in colors.chunks(width).enumerate() {
        let (cur, next) = err.split_at_mut(width + 2);
        if dither == Dither::Diffusion {
            next.fill([0.0; 3]);
        }
        for (x, c) in row.iter().enumerate() {
            let i = y * width + x;
            if skip(i) {
                continue;
            }
            let mut v = c.map(f32::from);
            match dither {
                Dither::None => {}
                Dither::Diffusion => {
                    for ch in 0..3 {
                        v[ch] = (v[ch] + cur[x + 1][ch]).clamp(0.0, 255.0);
                    }
                }
                Dither::Pattern | Dither::Noise => {
                    let t = if dither == Dither::Pattern { bayer(x % 8, y % 8) } else { noise(x, y) };
                    v = v.map(|ch| (ch + (t - 0.5) * step).clamp(0.0, 255.0));
                }
            }
            let ix = lookup(v);
            out[i] = ix;
            if dither == Dither::Diffusion {
                let p = pal.get(ix as usize).copied().unwrap_or_default();
                let e = [0, 1, 2].map(|ch| (v[ch] - p[ch]) * amount);
                // Floyd–Steinberg: 7/16 right, 3/16 below left, 5/16 below, 1/16 below right.
                for ch in 0..3 {
                    cur[x + 2][ch] += e[ch] * 7.0 / 16.0;
                    next[x][ch] += e[ch] * 3.0 / 16.0;
                    next[x + 1][ch] += e[ch] * 5.0 / 16.0;
                    next[x + 2][ch] += e[ch] / 16.0;
                }
            }
        }
        if dither == Dither::Diffusion {
            cur.copy_from_slice(next);
        }
    }
    out
}
