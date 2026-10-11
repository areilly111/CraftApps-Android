//! Trained face models for face tracking, behind one swappable interface.
//!
//! - [`FaceModel`]: what the face tracker (`effectcraft-track`) asks of a model: find a face in a
//!   region of a frame, then follow it from frame to frame. A model reports its own points (a
//!   mesh, for MediaPipe) plus a [`Topology`] saying which of them are the tracker's named
//!   landmarks and which trace the face outline, so the tracker never depends on one model's
//!   layout. The classical tracker stays built in as the fallback when no model is chosen.
//! - [`Roi`] and [`crop`]: the rotated square crops face models look at.

use crate::{ModelInfo, Result};

/// Which of a model's points are the tracker's landmarks and its face outline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Topology {
    /// The points tracing the face outline, in order around the face.
    pub outline: &'static [usize],
    /// (landmark id, as the tracker names them, e.g. `leftEyeInner`; point index). "Left" is
    /// image-left. Chin and jaw come from the outline.
    pub landmarks: &'static [(&'static str, usize)],
}

/// A face in a frame: the model's points (frame pixels) and its confidence (0–1).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Face {
    pub points: Vec<[f32; 2]>,
    pub score: f32,
}

/// A trained face model (swappable: the face tracker only uses this interface). Frames are
/// straight RGB 0–1, row-major `w×h`, coordinates frame pixels.
pub trait FaceModel: Send + Sync {
    fn info(&self) -> &'static ModelInfo;
    fn topology(&self) -> &'static Topology;
    /// The most confident face whose centre lies in `region` (`[x0, y0, x1, y1]`).
    fn find(&self, rgb: &[[f32; 3]], w: usize, h: usize, region: [f32; 4]) -> Result<Option<Face>>;
    /// The face `prev` was in the previous frame, in this one; `None` when it is lost.
    fn follow(&self, rgb: &[[f32; 3]], w: usize, h: usize, prev: &Face) -> Result<Option<Face>>;
}

/// A rotated square region of a frame: centre, side and angle (radians, the direction of the
/// square's x axis in the frame, y down).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Roi {
    pub center: [f32; 2],
    pub size: f32,
    pub angle: f32,
}

impl Roi {
    /// Frame position of the normalised square position `(u, v)` (0–1).
    pub fn to_frame(&self, u: f32, v: f32) -> [f32; 2] {
        let (s, c) = self.angle.sin_cos();
        let (x, y) = ((u - 0.5) * self.size, (v - 0.5) * self.size);
        [self.center[0] + c * x - s * y, self.center[1] + s * x + c * y]
    }
}

/// What a [`crop`] reads outside the frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Border {
    /// Black (MediaPipe's face detector input).
    Zero,
    /// The nearest edge pixel (MediaPipe's face mesh input).
    Replicate,
}

/// Sample `roi` into an `n×n` RGB tensor (bilinear), each channel mapped from 0–1 to `lo`–`hi`.
///
/// The sampling follows MediaPipe's CPU `ImageToTensor`, so the landmarks match MediaPipe's to a
/// fraction of a pixel: the square's corners map to the output corners `(0,0)` to `(n,n)` (output
/// pixel `i` is at `u = i/n`, not at its centre), the source position is read as a pixel-centre
/// index with no half-pixel shift, and the result is rounded to a whole 0–255 value before the range
/// map. Outside the frame it reads `border`. A frame whose buffer is shorter than `w·h`, or whose
/// size overflows, yields `lo` everywhere.
pub fn crop(rgb: &[[f32; 3]], w: usize, h: usize, roi: &Roi, n: usize, [lo, hi]: [f32; 2], border: Border) -> Vec<f32> {
    let mut out = vec![lo; n.saturating_mul(n).saturating_mul(3)];
    if w == 0 || h == 0 || n == 0 || w.checked_mul(h).is_none_or(|len| rgb.len() < len) {
        return out;
    }
    let (wi, hi_) = (w as i64, h as i64);
    // In range after the border rule, so the index is below `w·h` (which fits in `usize`).
    let px = |x: i64, y: i64| -> [f32; 3] {
        let (x, y) = match border {
            Border::Zero if x < 0 || y < 0 || x >= wi || y >= hi_ => return [0.0; 3],
            Border::Zero => (x, y),
            Border::Replicate => (x.clamp(0, wi - 1), y.clamp(0, hi_ - 1)),
        };
        rgb.get(y as usize * w + x as usize).copied().unwrap_or([0.0; 3])
    };
    for j in 0..n {
        for i in 0..n {
            let [x, y] = roi.to_frame(i as f32 / n as f32, j as f32 / n as f32);
            let (x0, y0) = (x.floor(), y.floor());
            let (fx, fy) = (x - x0, y - y0);
            // Saturating casts: NaN becomes 0 and an absurd position reads the border.
            let (x0, y0) = (x0 as i64, y0 as i64);
            let (x1, y1) = (x0.saturating_add(1), y0.saturating_add(1));
            let (a, b, c, d) = (px(x0, y0), px(x1, y0), px(x0, y1), px(x1, y1));
            for k in 0..3 {
                let v = (a[k] * (1.0 - fx) + b[k] * fx) * (1.0 - fy) + (c[k] * (1.0 - fx) + d[k] * fx) * fy;
                let v = if v.is_finite() { (v * 255.0).round().clamp(0.0, 255.0) / 255.0 } else { 0.0 };
                out[(j * n + i) * 3 + k] = lo + v * (hi - lo);
            }
        }
    }
    out
}

pub(crate) fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roi_maps_corners_and_rotates() {
        let r = Roi { center: [10.0, 20.0], size: 4.0, angle: 0.0 };
        assert_eq!(r.to_frame(0.0, 0.0), [8.0, 18.0]);
        assert_eq!(r.to_frame(1.0, 1.0), [12.0, 22.0]);
        // A quarter turn: the square's x axis points down the frame.
        let q = Roi { angle: std::f32::consts::FRAC_PI_2, ..r };
        let p = q.to_frame(1.0, 0.5);
        assert!((p[0] - 10.0).abs() < 1e-5 && (p[1] - 22.0).abs() < 1e-5);
    }

    #[test]
    fn crop_follows_mediapipe_corner_aligned_sampling() {
        // A 4×4 ramp cropped 1:1 reproduces it: output pixel i is at u = i/n, a source pixel centre.
        let rgb: Vec<[f32; 3]> = (0..16).map(|i| [(i * 16) as f32 / 255.0, 0.0, 1.0]).collect();
        let at = |cx: f32| crop(&rgb, 4, 4, &Roi { center: [cx, 2.0], size: 4.0, angle: 0.0 }, 4, [0.0, 255.0], Border::Zero)[0];
        let c = crop(&rgb, 4, 4, &Roi { center: [2.0, 2.0], size: 4.0, angle: 0.0 }, 4, [0.0, 1.0], Border::Zero);
        for i in 0..16 {
            assert!((c[i * 3] - (i * 16) as f32 / 255.0).abs() < 1e-6);
        }
        // Half a pixel to the right is halfway between ramp values 0 and 16.
        assert!((at(2.5) - 8.0).abs() < 1e-3, "{}", at(2.5));
        // The result is rounded to a whole 0-255 value: a third of the way from 0 to 16 is 5.33.
        assert!((at(2.0 + 1.0 / 3.0) - 5.0).abs() < 1e-3, "{}", at(2.0 + 1.0 / 3.0));
    }

    #[test]
    fn crop_borders_are_zero_or_replicated() {
        let rgb: Vec<[f32; 3]> = (0..16).map(|i| [(i * 16) as f32 / 255.0, 0.0, 1.0]).collect();
        let far = Roi { center: [-10.0, -10.0], size: 4.0, angle: 0.0 };
        let c = crop(&rgb, 4, 4, &far, 2, [-1.0, 1.0], Border::Zero);
        assert!(c.iter().all(|v| *v == -1.0));
        // Replicated, the top-left pixel (0, 0, 255) fills the crop.
        let c = crop(&rgb, 4, 4, &far, 2, [0.0, 255.0], Border::Replicate);
        for p in c.chunks(3) {
            assert!(p[0].abs() < 1e-3 && p[1].abs() < 1e-3 && (p[2] - 255.0).abs() < 1e-3, "{p:?}");
        }
        // Half outside on the left: zero blends toward black, replicate keeps the edge value.
        let edge = Roi { center: [0.0, 2.0], size: 4.0, angle: 0.0 };
        let z = crop(&rgb, 4, 4, &edge, 4, [0.0, 255.0], Border::Zero);
        let r = crop(&rgb, 4, 4, &edge, 4, [0.0, 255.0], Border::Replicate);
        assert!(z[2] == 0.0 && (r[2] - 255.0).abs() < 1e-3, "{} {}", z[2], r[2]);
    }

    #[test]
    fn crop_survives_absurd_frame_sizes() {
        let rgb = [[0.5f32; 3]; 4];
        let roi = Roi { center: [1e30, -1e30], size: 1e30, angle: 1.0 };
        for (w, h) in [(usize::MAX, 2), (2, usize::MAX), (usize::MAX, usize::MAX), (1 << 40, 1 << 40), (0, 4), (4, 0)] {
            for border in [Border::Zero, Border::Replicate] {
                let c = crop(&rgb, w, h, &roi, 4, [-1.0, 1.0], border);
                assert_eq!(c.len(), 48);
                assert!(c.iter().all(|v| *v == -1.0), "{w}x{h}: an unreadable frame gives `lo`");
            }
        }
    }
}
