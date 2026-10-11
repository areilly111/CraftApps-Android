//! VectorCraft Image Trace: raster → vector.
//!
//! A clean-room tracer built from the published ideas behind bitmap tracers (the potrace paper
//! and classic colour quantisation), not from any existing implementation:
//!
//! 1. **Quantise** the image into a small palette: a luminance threshold (Black and White), 1-D
//!    k-means over the luminance histogram (Grayscale), or, in Color, over a 15-bit colour
//!    histogram as the [`Palette`] asks: median cut to a colour count followed by weighted k-means
//!    (Limited), median cut until every colour box is tight (Full Tone), one of those two by
//!    whether the image is flat art (Automatic), or the most used colours of a swatch library
//!    (Document Library). Transparent pixels are left out.
//! 2. **Remove noise**: 4-connected same-colour components smaller than `noise` pixels are merged
//!    into their most common neighbouring colour.
//! 3. **Follow boundaries**: for each colour layer, the pixel-crack edges between inside and
//!    outside are linked into closed loops (inside on the right, right turns at saddles, so
//!    diagonal pixels stay separate, matching 4-connectivity). Outer loops run clockwise and
//!    holes counter-clockwise (y down), so the result fills correctly with either fill rule.
//! 4. **Polygon**: each loop is reduced to a polygon whose vertices stay within a fidelity
//!    dependent distance of the pixel boundary (Douglas–Peucker on the crack vertices, which
//!    removes the staircase).
//! 5. **Curves**: the polygon is fitted with cubic Béziers (`vectorcraft_pathops::simplify_with`,
//!    least squares with corner detection); optionally nearly-straight curves snap to lines.
//!
//! With **Create: Strokes**, the parts of a colour layer no wider than the stroke width are traced
//! as stroked centre lines instead (see `centerline`); without Fills, wider parts become stroked
//! outlines.
//!
//! Colour layers are traced either *abutting* (each colour's own area; shapes share edges) or
//! *overlapping* (stacked: each layer also covers every layer above it, so no hairline gaps).
#![forbid(unsafe_code)]

mod centerline;
mod contour;
mod fit;
mod logo;
mod mosaic;
mod quantize;

use std::sync::atomic::{AtomicUsize, Ordering};

use serde::{Deserialize, Serialize};
use vectorcraft_geom::PathData;

pub use contour::{Component, Loop, trace_mask};
pub use mosaic::mosaic;
pub use quantize::{FULL_TONE_MAX, Quantized, TRANSPARENT, denoise, quantize};

/// Errors decoding a raster.
#[derive(Debug, thiserror::Error)]
pub enum TraceError {
    #[error("could not decode image: {0}")]
    Decode(String),
    #[error("image is empty")]
    Empty,
    #[error("the image is {width} × {height} pixels, more than Image Trace takes ({max} megapixels)")]
    TooLarge { width: u32, height: u32, max: u64 },
    #[error("this isn't a flat-colour logo ({:.0}% of its solid areas match a few flat colours), so Flat Logo mode would turn its gradients into patches: use Color mode instead", (.fraction * 100.0).floor())]
    NotFlat { fraction: f64 },
    #[error("\"{0}\" is not a colour: write it as #rrggbb")]
    BadPalette(String),
    #[error(
        "this trace would make more than {max} anchor points, more than the document can show smoothly: raise Noise, lower Paths or Colors, or pick a lower-fidelity preset"
    )]
    TooComplex { max: usize },
}

/// The most pixels an image to trace may have (64 megapixels: 256 MB decoded); larger images are
/// refused instead of running out of memory.
pub const MAX_PIXELS: u64 = 64 << 20;

/// An RGBA8 raster (row-major, top row first).
#[derive(Clone, Debug, PartialEq)]
pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Raster {
    /// A raster from RGBA bytes (`width * height * 4` of them).
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Self {
        assert_eq!(rgba.len(), width as usize * height as usize * 4, "rgba length must be width*height*4");
        Self { width, height, rgba }
    }
    /// A raster whose pixel `(x, y)` is `f(x, y)`.
    pub fn from_fn(width: u32, height: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Self {
        let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
        for y in 0..height {
            for x in 0..width {
                rgba.extend_from_slice(&f(x, y));
            }
        }
        Self { width, height, rgba }
    }
    /// Decode PNG / JPEG / WebP / GIF bytes (at most [`MAX_PIXELS`], checked before decoding).
    pub fn decode(bytes: &[u8]) -> Result<Self, TraceError> {
        let decode_err = |e: image::ImageError| TraceError::Decode(e.to_string());
        let reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().map_err(|e| TraceError::Decode(e.to_string()))?;
        let (w, h) = reader.into_dimensions().map_err(decode_err)?;
        if w == 0 || h == 0 {
            return Err(TraceError::Empty);
        }
        if u64::from(w) * u64::from(h) > MAX_PIXELS {
            return Err(TraceError::TooLarge { width: w, height: h, max: MAX_PIXELS >> 20 });
        }
        let img = image::load_from_memory(bytes).map_err(decode_err)?.to_rgba8();
        let (w, h) = img.dimensions();
        Ok(Self { width: w, height: h, rgba: img.into_raw() })
    }
    /// Encode as PNG.
    pub fn encode_png(&self) -> Vec<u8> {
        let mut out = Vec::new();
        if let Some(img) = image::RgbaImage::from_raw(self.width, self.height, self.rgba.clone()) {
            let _ = img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png);
        }
        out
    }
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = (y as usize * self.width as usize + x as usize) * 4;
        [self.rgba[i], self.rgba[i + 1], self.rgba[i + 2], self.rgba[i + 3]]
    }
}

/// Tracing mode (Illustrator's Mode popup).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    #[default]
    #[serde(alias = "bw", alias = "blackWhite", alias = "Black and White")]
    BlackAndWhite,
    #[serde(alias = "gray", alias = "Grayscale")]
    Grayscale,
    #[serde(alias = "Color")]
    Color,
    /// Flat-colour logos: sub-pixel edges, sharp corners restored, true lines and circles.
    #[serde(alias = "Logo", alias = "flat", alias = "flatLogo")]
    Logo,
}

/// How colour layers relate (Advanced → Method).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Method {
    /// Each colour traced on its own; neighbouring shapes share their edge.
    #[default]
    Abutting,
    /// Stacked: every layer extends under the layers above it.
    Overlapping,
}

/// Where Color mode's colours come from (the Palette popup).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Palette {
    /// The `colors` colours that best represent the image.
    #[default]
    #[serde(alias = "Limited")]
    Limited,
    /// As many colours as the image's tones need, up to [`FULL_TONE_MAX`] (photos, gradients):
    /// `color_detail` sets how close two colours may be and stay apart.
    #[serde(alias = "Full Tone")]
    FullTone,
    /// Flat art (a few colours cover nearly all of it) traces with exactly those colours, anything
    /// else as Full Tone.
    #[serde(alias = "Automatic")]
    Automatic,
    /// The most used colours of a swatch library ([`TraceParams::swatches`]), at most `colors` of
    /// them, exactly as they are.
    #[serde(alias = "library", alias = "Document Library")]
    DocumentLibrary,
}

/// Image Trace parameters (the Image Trace panel).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TraceParams {
    pub mode: Mode,
    /// Black and White: pixels darker than this (luminance 0–255) are black.
    pub threshold: u8,
    /// Color: where the colours come from.
    pub palette: Palette,
    /// Palette size for Color (Limited; at most this many for Document Library), number of grays
    /// for Grayscale (2–256).
    pub colors: u32,
    /// Full Tone and Automatic: how many colours, 0 (few) – 100 (many).
    pub color_detail: f64,
    /// Document Library: the swatch library, `"document"` (the document's swatches) or a library id
    /// or name. The caller looks its colours up into `swatches`.
    pub library: String,
    /// Document Library: the library's colours (sRGB). Not saved: they are looked up from
    /// `library` at every trace.
    #[serde(skip)]
    pub swatches: Vec<[u8; 3]>,
    /// Paths (fidelity) 0–100: higher follows the pixels more tightly.
    pub paths: f64,
    /// Corners 0–100: higher keeps more corners.
    pub corners: f64,
    /// Noise: areas smaller than this many pixels are ignored. (Flat Logo mode: areas smaller than
    /// `noise / 10` pixels of the source image.)
    pub noise: u32,
    pub method: Method,
    /// Drop white areas (no white background shape).
    pub ignore_white: bool,
    /// Replace nearly-straight curves with straight lines. In Logo mode this also snaps tight arcs
    /// to true circles.
    pub snap_curves_to_lines: bool,
    /// Create: trace areas as filled shapes.
    pub fills: bool,
    /// Create: trace lines no wider than `stroke_width` as stroked centre lines.
    pub strokes: bool,
    /// Stroke: the widest feature (px) traced as a stroke.
    pub stroke_width: f64,
    /// Flat Logo mode: the flat colours (`#rrggbb`) to use; empty finds them automatically. White is
    /// always traced on its own and cannot be named, colours closer than about 12 (RGB distance)
    /// count as one, and at most ten are used. (Not `palette`: that is Color mode's colour source.)
    pub logo_colors: Vec<String>,
}

impl Default for TraceParams {
    /// Illustrator's [Default]: black and white, threshold 128.
    fn default() -> Self {
        Self {
            mode: Mode::BlackAndWhite,
            threshold: 128,
            palette: Palette::Limited,
            colors: 6,
            color_detail: 50.0,
            library: "document".into(),
            swatches: vec![],
            paths: 50.0,
            corners: 75.0,
            noise: 25,
            method: Method::Abutting,
            ignore_white: false,
            snap_curves_to_lines: false,
            fills: true,
            strokes: false,
            stroke_width: 10.0,
            logo_colors: Vec::new(),
        }
    }
}

impl TraceParams {
    /// Maximum distance (px) of polygon vertices from the pixel boundary.
    fn polygon_tolerance(&self) -> f64 {
        let f = (self.paths / 100.0).clamp(0.0, 1.0);
        0.55 + 0.75 * (1.0 - f)
    }
    /// Curve fitting tolerance (px).
    fn fit_tolerance(&self) -> f64 {
        let f = (self.paths / 100.0).clamp(0.0, 1.0);
        0.2 + 1.6 * (1.0 - f)
    }
    /// Turn angle (degrees) above which a polygon vertex stays a corner.
    fn corner_angle(&self) -> f64 {
        let c = (self.corners / 100.0).clamp(0.0, 1.0);
        150.0 - 115.0 * c
    }
}

/// Built-in preset names in panel order (our own parameter sets).
pub const PRESET_NAMES: &[&str] = &[
    "Default",
    "High Fidelity Photo",
    "Low Fidelity Photo",
    "3 Colors",
    "6 Colors",
    "16 Colors",
    "Shades of Gray",
    "Black and White Logo",
    "Sketched Art",
    "Silhouettes",
    "Line Art",
    "Technical Drawing",
    "Flat Logo",
];

/// A built-in preset by name (case-insensitive; `[Default]` is accepted).
pub fn preset(name: &str) -> Option<TraceParams> {
    let key = name.trim().trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
    let d = TraceParams::default();
    let color = |colors: u32, paths: f64, corners: f64, noise: u32, method: Method| TraceParams {
        mode: Mode::Color,
        colors,
        paths,
        corners,
        noise,
        method,
        ..TraceParams::default()
    };
    let bw = |threshold: u8, paths: f64, corners: f64, noise: u32, snap: bool| TraceParams {
        mode: Mode::BlackAndWhite,
        threshold,
        paths,
        corners,
        noise,
        ignore_white: true,
        snap_curves_to_lines: snap,
        ..TraceParams::default()
    };
    Some(match key.as_str() {
        "default" => d,
        "high fidelity photo" => color(64, 90.0, 25.0, 4, Method::Overlapping),
        "low fidelity photo" => color(20, 60.0, 50.0, 12, Method::Overlapping),
        "3 colors" => color(3, 60.0, 60.0, 20, Method::Abutting),
        "6 colors" => color(6, 65.0, 60.0, 16, Method::Abutting),
        "16 colors" => color(16, 70.0, 55.0, 10, Method::Abutting),
        "shades of gray" => TraceParams { mode: Mode::Grayscale, colors: 8, paths: 60.0, corners: 50.0, noise: 10, method: Method::Overlapping, ..d },
        "black and white logo" => bw(128, 95.0, 80.0, 8, true),
        "sketched art" => bw(150, 50.0, 50.0, 100, false),
        "silhouettes" => bw(200, 40.0, 60.0, 30, false),
        "line art" => bw(128, 80.0, 70.0, 10, false),
        "technical drawing" => bw(128, 95.0, 90.0, 4, true),
        // Flat-colour logos: colours found automatically, edges placed to a fraction of a pixel.
        "flat logo" => TraceParams { mode: Mode::Logo, colors: 8, paths: 60.0, corners: 50.0, noise: 6, snap_curves_to_lines: true, ..d },
        _ => return None,
    })
}

/// All built-in presets in panel order.
pub fn presets() -> Vec<(&'static str, TraceParams)> {
    PRESET_NAMES.iter().filter_map(|n| Some((*n, preset(n)?))).collect()
}

/// One traced shape: an outer contour plus its holes, filled with `color`, or (with `stroke`)
/// lines stroked with it.
#[derive(Clone, Debug, PartialEq)]
pub struct TracedPath {
    /// In pixel coordinates (x right, y down; the image spans `0..width × 0..height`).
    pub path: PathData,
    pub color: [u8; 3],
    /// Filled pixel count of the component the path came from.
    pub pixels: usize,
    /// Create Strokes: the path is stroked this wide (px), not filled (centre lines, or the
    /// outlines of wider areas without Fills).
    pub stroke: Option<f64>,
}

/// The traced result, bottom-most path first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TraceResult {
    pub paths: Vec<TracedPath>,
    /// Colours actually used.
    pub palette: Vec<[u8; 3]>,
}

impl TraceResult {
    pub fn anchor_count(&self) -> usize {
        self.paths.iter().map(|p| p.path.anchor_count()).sum()
    }
}

fn is_white(c: [u8; 3]) -> bool {
    c.iter().all(|&v| v >= 245)
}

/// Trace `img` with `params`. Failures (an image Logo mode refuses, say) give an empty result: use
/// [`trace_within`] to see why.
pub fn trace(img: &Raster, params: &TraceParams) -> TraceResult {
    // No trace has more than `usize::MAX` anchors.
    trace_within(img, params, usize::MAX).unwrap_or_default()
}

/// Trace `img` with `params`, giving up with [`TraceError::TooComplex`] once the paths have more
/// than `max_anchors` anchor points (a photo traced at high fidelity can make millions). The
/// colour layers are traced in parallel where threads are available.
pub fn trace_within(img: &Raster, params: &TraceParams, max_anchors: usize) -> Result<TraceResult, TraceError> {
    let (w, h) = (img.width as usize, img.height as usize);
    if w == 0 || h == 0 || img.rgba.len() != w * h * 4 {
        return Ok(TraceResult::default());
    }
    if params.mode == Mode::Logo {
        return logo::trace_logo(img, params, max_anchors);
    }
    let mut q = quantize(img, params);
    denoise(&mut q.labels, w, h, params.noise as usize);
    // Pixel count per palette entry.
    let mut counts = vec![0usize; q.palette.len()];
    for &l in &q.labels {
        if let Some(c) = counts.get_mut(l as usize) {
            *c += 1;
        }
    }
    // Layers bottom → top: largest area first.
    let mut order: Vec<usize> = (0..q.palette.len()).filter(|&i| counts[i] > 0).collect();
    order.sort_by(|a, b| counts[*b].cmp(&counts[*a]).then(a.cmp(b)));
    let mut rank = vec![usize::MAX; q.palette.len()];
    for (r, &i) in order.iter().enumerate() {
        rank[i] = r;
    }
    let layers: Vec<(usize, usize)> =
        order.iter().copied().enumerate().filter(|&(_, ci)| !(params.ignore_white && is_white(q.palette[ci]))).collect();
    let job = Layers {
        q: &q,
        rank: &rank,
        params,
        opts: fit::FitOptions {
            polygon_tol: params.polygon_tolerance(),
            fit_tol: params.fit_tolerance(),
            corner_angle: params.corner_angle(),
            snap_lines: params.snap_curves_to_lines,
        },
        w,
        h,
        anchors: AtomicUsize::new(0),
        max_anchors,
    };
    let mut out = TraceResult::default();
    for (paths, &(_, ci)) in parallel_map(&layers, |&(r, ci)| job.layer(r, ci)).into_iter().zip(&layers) {
        out.paths.extend(paths.ok_or(TraceError::TooComplex { max: max_anchors })?);
        let color = q.palette[ci];
        if !out.palette.contains(&color) {
            out.palette.push(color);
        }
    }
    Ok(out)
}

/// What tracing one colour layer needs.
struct Layers<'a> {
    q: &'a Quantized,
    /// Each palette entry's layer, bottom (0) up.
    rank: &'a [usize],
    params: &'a TraceParams,
    opts: fit::FitOptions,
    w: usize,
    h: usize,
    /// Anchor points made so far, by every layer.
    anchors: AtomicUsize,
    max_anchors: usize,
}

impl Layers<'_> {
    /// The paths of layer `r` (palette entry `ci`), or `None` once the anchors made pass the
    /// maximum.
    fn layer(&self, r: usize, ci: usize) -> Option<Vec<TracedPath>> {
        let (params, opts) = (self.params, &self.opts);
        let color = *self.q.palette.get(ci)?;
        let overlapping = params.method == Method::Overlapping;
        let (w, h) = (self.w, self.h);
        // Create Strokes: this colour's lines, left out of its areas.
        let (thin, lines) = if params.strokes {
            centerline::lines(&self.q.labels, u16::try_from(ci).unwrap_or(TRANSPARENT), w, h, params.stroke_width, opts)
        } else {
            (vec![], vec![])
        };
        let mask: Vec<bool> = self
            .q
            .labels
            .iter()
            .enumerate()
            .map(|(i, &l)| {
                l != TRANSPARENT
                    && !thin.get(i).copied().unwrap_or(false)
                    && if overlapping { self.rank.get(l as usize).is_some_and(|&lr| lr >= r) } else { l as usize == ci }
            })
            .collect();
        // Without Fills, areas are outlined with a 1 px stroke.
        let area_stroke = (params.strokes && !params.fills).then_some(1.0);
        let min_hole = params.noise.max(1) as i64;
        let mut paths = vec![];
        let mut add = |path: PathData, pixels: usize, stroke: Option<f64>| {
            let n = path.anchor_count();
            if self.anchors.fetch_add(n, Ordering::Relaxed).saturating_add(n) > self.max_anchors {
                return false;
            }
            paths.push(TracedPath { path, color, pixels, stroke });
            true
        };
        for comp in trace_mask(&mask, w, h) {
            let Some(outer) = fit::fit_loop(&comp.outer, opts) else { continue };
            let mut subs = vec![outer];
            for hole in &comp.holes {
                if hole.area2.abs() / 2 < min_hole && params.noise > 0 {
                    continue;
                }
                subs.extend(fit::fit_loop(hole, opts));
            }
            if !add(PathData::new(subs), comp.pixels, area_stroke) {
                return None;
            }
        }
        for line in lines {
            if !add(line.path, line.pixels, Some(line.width)) {
                return None;
            }
        }
        Some(paths)
    }
}

/// `f` of each of `items`, in order, spread over the available cores.
#[cfg(not(target_arch = "wasm32"))]
fn parallel_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(items.len());
    if threads <= 1 {
        return items.iter().map(f).collect();
    }
    let next = AtomicUsize::new(0);
    let mut done: Vec<(usize, R)> = std::thread::scope(|s| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let mut mine = vec![];
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(item) = items.get(i) else { break };
                        mine.push((i, f(item)));
                    }
                    mine
                })
            })
            .collect();
        // A worker's panic goes on in this thread, as if it had run here (the engine's guard
        // reports it).
        workers.into_iter().flat_map(|w| w.join().unwrap_or_else(|e| std::panic::resume_unwind(e))).collect()
    });
    done.sort_by_key(|(i, _)| *i);
    done.into_iter().map(|(_, r)| r).collect()
}

/// `f` of each of `items`, in order (no threads on the web).
#[cfg(target_arch = "wasm32")]
fn parallel_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    items.iter().map(f).collect()
}

#[cfg(test)]
mod tests;
