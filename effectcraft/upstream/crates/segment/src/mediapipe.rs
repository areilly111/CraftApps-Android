//! MediaPipe Face Landmarker (Google LLC, Apache-2.0) in plain Rust: the official
//! `face_landmarker.task` bundle's BlazeFace short-range detector and Face Mesh V2 (478 points)
//! run on the [`tflite`](crate::tflite) interpreter.
//!
//! The pipeline follows MediaPipe's published face landmarker graph (Apache-2.0): the detector
//! sees a 128×128 crop scaled to −1…1 and decodes 896 SSD anchors (strides 8, 16, 16, 16; two
//! anchors per cell per layer) with weighted non-maximum suppression; the mesh sees a 256×256
//! crop (0…1) of a square 1.5× the face, rotated so the eyes are level (detector keypoints 0 → 1,
//! then mesh points 33 → 263), and reports a presence score. The landmark indices in
//! [`TOPOLOGY`] are points of MediaPipe's canonical face mesh.

use crate::face::{Border, Face, FaceModel, Roi, Topology, crop, sigmoid};
use crate::tflite::Model;
use crate::{FACE_LANDMARKER, ModelInfo, Result};

pub const DETECTOR: &str = "face_detector.tflite";
pub const MESH: &str = "face_landmarks_detector.tflite";

const DET_SIZE: usize = 128;
const MESH_SIZE: usize = 256;
const MESH_POINTS: usize = 478;
/// Detection and presence thresholds (MediaPipe's defaults).
const MIN_SCORE: f32 = 0.5;
const NMS_IOU: f32 = 0.3;
/// Crops are this much larger than the face.
const ROI_SCALE: f32 = 1.5;
/// Mesh points whose direction levels the crop: the outer eye corners.
const EYE_LINE: [usize; 2] = [33, 263];

/// MediaPipe's face mesh: the face oval (`FACEMESH_FACE_OVAL`, in order) and the points the
/// tracker's landmarks are (inner lips for the mouth, iris centres for the pupils).
pub const TOPOLOGY: Topology = Topology {
    outline: &[
        10, 338, 297, 332, 284, 251, 389, 356, 454, 323, 361, 288, 397, 365, 379, 378, 400, 377, 152, 148, 176, 149, 150, 136, 172, 58, 132, 93, 234, 127, 162,
        21, 54, 103, 67, 109,
    ],
    landmarks: &[
        ("leftEyebrowInner", 107),
        ("leftEyebrowMiddle", 105),
        ("leftEyebrowOuter", 70),
        ("rightEyebrowInner", 336),
        ("rightEyebrowMiddle", 334),
        ("rightEyebrowOuter", 300),
        ("leftEyeInner", 133),
        ("leftEyeOuter", 33),
        ("leftEyeTop", 159),
        ("leftEyeBottom", 145),
        ("leftPupil", 468),
        ("rightEyeInner", 362),
        ("rightEyeOuter", 263),
        ("rightEyeTop", 386),
        ("rightEyeBottom", 374),
        ("rightPupil", 473),
        ("noseTip", 1),
        ("leftNostril", 79),
        ("rightNostril", 309),
        ("mouthLeft", 61),
        ("mouthRight", 291),
        ("mouthTop", 13),
        ("mouthBottom", 14),
    ],
};

/// A detected face: box `[x0, y0, x1, y1]`, six keypoints (eyes, nose tip, mouth, ears; image
/// left first) and score, in frame pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Detection {
    pub bbox: [f32; 4],
    pub keypoints: [[f32; 2]; 6],
    pub score: f32,
}

pub struct FaceLandmarker {
    detector: Model,
    mesh: Model,
    /// Anchor centres (normalised).
    anchors: Vec<[f32; 2]>,
}

/// SSD anchor centres: per stride group, `fm×fm` cells with two anchors per layer each.
fn anchors() -> Vec<[f32; 2]> {
    let mut out = vec![];
    for (stride, layers) in [(8usize, 1usize), (16, 3)] {
        let fm = DET_SIZE.div_ceil(stride);
        for y in 0..fm {
            for x in 0..fm {
                for _ in 0..2 * layers {
                    out.push([(x as f32 + 0.5) / fm as f32, (y as f32 + 0.5) / fm as f32]);
                }
            }
        }
    }
    out
}

fn iou(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let iw = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
    let ih = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    let inter = iw * ih;
    let union = (a[2] - a[0]) * (a[3] - a[1]) + (b[2] - b[0]) * (b[3] - b[1]) - inter;
    if union > 0.0 { inter / union } else { 0.0 }
}

/// Weighted non-maximum suppression: each cluster of overlapping detections becomes their
/// score-weighted average, with the best score.
fn weighted_nms(mut dets: Vec<Detection>) -> Vec<Detection> {
    dets.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut out = vec![];
    while let Some(top) = dets.first().copied() {
        // `top` always joins its own cluster: its overlap with itself is 0 for a degenerate box
        // (NaN or zero area), so partitioning it with the rest would never shrink `dets`.
        let (mut cluster, rest): (Vec<Detection>, Vec<Detection>) = dets.into_iter().skip(1).partition(|d| iou(&d.bbox, &top.bbox) > NMS_IOU);
        dets = rest;
        cluster.insert(0, top);
        let total: f32 = cluster.iter().map(|d| d.score).sum();
        if total <= 0.0 {
            out.push(top);
            continue;
        }
        let mut avg = Detection { bbox: [0.0; 4], keypoints: [[0.0; 2]; 6], score: top.score };
        for d in &cluster {
            let k = d.score / total;
            for i in 0..4 {
                avg.bbox[i] += d.bbox[i] * k;
            }
            for (a, p) in avg.keypoints.iter_mut().zip(&d.keypoints) {
                a[0] += p[0] * k;
                a[1] += p[1] * k;
            }
        }
        out.push(avg);
    }
    out
}

/// The crop a face's mesh is computed in: `bbox` grown to a square 1.5× its long side, turned so
/// `from → to` is level.
fn roi_of(bbox: [f32; 4], from: [f32; 2], to: [f32; 2]) -> Roi {
    Roi {
        center: [(bbox[0] + bbox[2]) / 2.0, (bbox[1] + bbox[3]) / 2.0],
        size: (bbox[2] - bbox[0]).max(bbox[3] - bbox[1]) * ROI_SCALE,
        angle: (to[1] - from[1]).atan2(to[0] - from[0]),
    }
}

fn bounds(points: &[[f32; 2]]) -> [f32; 4] {
    points
        .iter()
        .fold([f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])])
}

impl FaceLandmarker {
    /// From the official `face_landmarker.task` bundle (a zip archive of `.tflite` models).
    pub fn from_task(bytes: &[u8]) -> Result<FaceLandmarker> {
        let detector = Model::read(&crate::pt::zip_file(bytes, DETECTOR)?)?;
        let mesh = Model::read(&crate::pt::zip_file(bytes, MESH)?)?;
        let anchors = anchors();
        if detector.input_shape(0) != Some(&[1, DET_SIZE, DET_SIZE, 3]) || detector.output(0).map(|o| o.0) != Some(&[1, anchors.len(), 16]) {
            return Err("unexpected face detector".into());
        }
        if mesh.input_shape(0) != Some(&[1, MESH_SIZE, MESH_SIZE, 3]) || mesh.output(0).map(|o| o.0.iter().product::<usize>()) != Some(MESH_POINTS * 3) {
            return Err("unexpected face mesh".into());
        }
        Ok(FaceLandmarker { detector, mesh, anchors })
    }

    /// Faces in the square `roi` of a frame (frame pixels), best first.
    pub fn detect(&self, rgb: &[[f32; 3]], w: usize, h: usize, roi: &Roi) -> Result<Vec<Detection>> {
        let x = crop(rgb, w, h, roi, DET_SIZE, [-1.0, 1.0], Border::Zero);
        let out = self.detector.run(&[&x])?;
        let (Some(reg), Some(cls)) = (out.first(), out.get(1)) else { return Err("face detector: missing outputs".into()) };
        let s = DET_SIZE as f32;
        let mut dets = vec![];
        for (i, a) in self.anchors.iter().enumerate() {
            let (Some(r), Some(&c)) = (reg.get(i * 16..i * 16 + 16), cls.get(i)) else { break };
            let score = sigmoid(c.clamp(-100.0, 100.0));
            if score < MIN_SCORE {
                continue;
            }
            let (cx, cy, bw, bh) = (r[0] / s + a[0], r[1] / s + a[1], r[2] / s, r[3] / s);
            let to = |u: f32, v: f32| roi.to_frame(u, v);
            let corners =
                [to(cx - bw / 2.0, cy - bh / 2.0), to(cx + bw / 2.0, cy - bh / 2.0), to(cx - bw / 2.0, cy + bh / 2.0), to(cx + bw / 2.0, cy + bh / 2.0)];
            let mut keypoints = [[0.0f32; 2]; 6];
            for (k, p) in keypoints.iter_mut().enumerate() {
                *p = to(r[4 + 2 * k] / s + a[0], r[5 + 2 * k] / s + a[1]);
            }
            dets.push(Detection { bbox: bounds(&corners), keypoints, score });
        }
        Ok(weighted_nms(dets))
    }

    /// The face mesh in `roi`: 478 points (frame pixels) and the presence score.
    pub fn mesh(&self, rgb: &[[f32; 3]], w: usize, h: usize, roi: &Roi) -> Result<Face> {
        let x = crop(rgb, w, h, roi, MESH_SIZE, [0.0, 1.0], Border::Replicate);
        let out = self.mesh.run(&[&x])?;
        let (Some(pts), Some(flag)) = (out.first(), out.get(1).and_then(|f| f.first())) else { return Err("face mesh: missing outputs".into()) };
        let s = MESH_SIZE as f32;
        let points = pts.as_chunks::<3>().0.iter().take(MESH_POINTS).map(|p| roi.to_frame(p[0] / s, p[1] / s)).collect();
        Ok(Face { points, score: sigmoid(*flag) })
    }

    /// The crop that frames a face found earlier.
    fn roi_of_face(face: &Face) -> Option<Roi> {
        let (a, b) = (face.points.get(EYE_LINE[0])?, face.points.get(EYE_LINE[1])?);
        Some(roi_of(bounds(&face.points), *a, *b))
    }

    /// The mesh from `roi`, refined once from its own points (a detector box is coarse).
    fn mesh_refined(&self, rgb: &[[f32; 3]], w: usize, h: usize, roi: &Roi) -> Result<Option<Face>> {
        let first = self.mesh(rgb, w, h, roi)?;
        if first.score < MIN_SCORE {
            return Ok(None);
        }
        let Some(r) = Self::roi_of_face(&first) else { return Ok(None) };
        let refined = self.mesh(rgb, w, h, &r)?;
        Ok(Some(if refined.score >= MIN_SCORE { refined } else { first }))
    }
}

impl FaceModel for FaceLandmarker {
    fn info(&self) -> &'static ModelInfo {
        &FACE_LANDMARKER
    }

    fn topology(&self) -> &'static Topology {
        &TOPOLOGY
    }

    fn find(&self, rgb: &[[f32; 3]], w: usize, h: usize, region: [f32; 4]) -> Result<Option<Face>> {
        let inside = |d: &Detection| {
            let c = [(d.bbox[0] + d.bbox[2]) / 2.0, (d.bbox[1] + d.bbox[3]) / 2.0];
            c[0] >= region[0] && c[0] <= region[2] && c[1] >= region[1] && c[1] <= region[3]
        };
        // Around the region first (a face filling the mask), then the whole frame (a small face).
        let side = (region[2] - region[0]).max(region[3] - region[1]);
        let around = Roi { center: [(region[0] + region[2]) / 2.0, (region[1] + region[3]) / 2.0], size: side * ROI_SCALE, angle: 0.0 };
        let whole = Roi { center: [w as f32 / 2.0, h as f32 / 2.0], size: w.max(h) as f32, angle: 0.0 };
        for roi in [around, whole] {
            if !roi.size.is_finite() || roi.size <= 4.0 {
                continue;
            }
            if let Some(d) = self.detect(rgb, w, h, &roi)?.into_iter().find(inside) {
                return self.mesh_refined(rgb, w, h, &roi_of(d.bbox, d.keypoints[0], d.keypoints[1]));
            }
        }
        Ok(None)
    }

    fn follow(&self, rgb: &[[f32; 3]], w: usize, h: usize, prev: &Face) -> Result<Option<Face>> {
        let Some(roi) = Self::roi_of_face(prev) else { return Ok(None) };
        let face = self.mesh(rgb, w, h, &roi)?;
        if face.score >= MIN_SCORE {
            return Ok(Some(face));
        }
        // Lost by the mesh (a fast move): look for it again near where it was.
        let b = bounds(&prev.points);
        let (gw, gh) = ((b[2] - b[0]) * 0.5, (b[3] - b[1]) * 0.5);
        self.find(rgb, w, h, [b[0] - gw, b[1] - gh, b[2] + gw, b[3] + gh])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchors_match_the_detector_layout() {
        let a = anchors();
        assert_eq!(a.len(), 896);
        // Stride 8: 16×16 cells, two anchors each; then stride 16: 8×8 cells, six each.
        assert_eq!(a[0], [0.5 / 16.0, 0.5 / 16.0]);
        assert_eq!(a[1], a[0]);
        assert_eq!(a[2], [1.5 / 16.0, 0.5 / 16.0]);
        assert_eq!(a[512], [0.5 / 8.0, 0.5 / 8.0]);
        assert_eq!(a[517], a[512]);
        assert_eq!(a[518], [1.5 / 8.0, 0.5 / 8.0]);
    }

    #[test]
    fn nms_averages_overlapping_detections() {
        let d = |x: f32, s: f32| Detection { bbox: [x, 0.0, x + 10.0, 10.0], keypoints: [[x, 0.0]; 6], score: s };
        let out = weighted_nms(vec![d(0.0, 0.9), d(1.0, 0.9), d(50.0, 0.6)]);
        assert_eq!(out.len(), 2);
        assert!((out[0].bbox[0] - 0.5).abs() < 1e-5 && out[0].score == 0.9);
        assert_eq!(out[1].bbox[0], 50.0);
    }

    /// A degenerate box (NaN or zero area) overlaps nothing, not even itself: each still becomes
    /// its own output instead of looping forever (#367).
    #[test]
    fn nms_terminates_on_degenerate_boxes() {
        let d = Detection { bbox: [f32::NAN; 4], keypoints: [[f32::NAN; 2]; 6], score: 0.7 };
        assert_eq!(weighted_nms(vec![d, d, d]).len(), 3);
        let z = Detection { bbox: [0.0; 4], keypoints: [[0.0; 2]; 6], score: 0.0 };
        assert_eq!(weighted_nms(vec![z, z]).len(), 2);
        let n = Detection { bbox: [0.0; 4], keypoints: [[0.0; 2]; 6], score: f32::NAN };
        assert_eq!(weighted_nms(vec![n, z, d]).len(), 3);
    }

    /// The official bundle (`EFFECTCRAFT_FACE_LANDMARKER` = path to `face_landmarker.task`) on a
    /// fixed input matches TensorFlow Lite's own interpreter (`ai-edge-litert` 2.2.0, run once
    /// to record these sums).
    #[test]
    fn matches_tensorflow_lite_on_the_official_weights() {
        let Some(path) = std::env::var_os("EFFECTCRAFT_FACE_LANDMARKER") else { return };
        let bytes = std::fs::read(path).unwrap();
        let m = FaceLandmarker::from_task(&bytes).unwrap();
        let input = |n: usize| -> Vec<f32> { (0..n).map(|i| ((i as f32) * 0.013).sin() * 0.5).collect() };
        let close = |got: &[f32], want: f64| {
            let s: f64 = got.iter().map(|v| *v as f64).sum();
            assert!((s - want).abs() <= 1e-5 * want.abs().max(1.0), "{s} vs {want}");
        };
        let d = m.detector.run(&[&input(DET_SIZE * DET_SIZE * 3)]).unwrap();
        close(&d[0], 54_202.040_273);
        close(&d[1], -3_061.866_384);
        let y = m.mesh.run(&[&input(MESH_SIZE * MESH_SIZE * 3)]).unwrap();
        close(&y[0], 119_864.749_257);
        close(&y[1], -19.777_233);
    }

    #[test]
    fn topology_indices_are_mesh_points() {
        assert!(TOPOLOGY.outline.iter().chain(TOPOLOGY.landmarks.iter().map(|l| &l.1)).all(|&i| i < MESH_POINTS));
        assert_eq!(TOPOLOGY.outline.len(), 36);
    }
}
