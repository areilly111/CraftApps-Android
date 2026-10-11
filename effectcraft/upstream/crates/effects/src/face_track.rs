//! Face tracking results (Tracker panel ▸ Method ▸ Face Tracking (Detailed Features)):
//!
//! - **Face Track Points** holds one point control per facial landmark (eyebrows, eyes and
//!   pupils, nose, nostrils, mouth, chin and jaw), keyed on every tracked frame by the engine's
//!   mask tracker ([`effectcraft_track::face`]); expressions and other layers can follow them.
//! - **Face Measurements** holds the measurements derived from those points (head position,
//!   scale and orientation, eye openness, eyebrow raise, mouth openness, width and offset), keyed
//!   by Extract & Copy Face Measurements (`track.extractFaceMeasurements`).
//!
//! Both pass their input through unchanged: they are data, like expression controls.

use effectcraft_keyframe::Value;
use effectcraft_project::ParamUi;
use effectcraft_track::face::{LANDMARKS, MEASUREMENTS};

use crate::{Buf, EffectCtx, EffectSpec, ParamSpec, num, p, slider};

/// Effect ids.
pub const POINTS_ID: &str = "ec.utility.facetrackpoints";
pub const MEASUREMENTS_ID: &str = "ec.utility.facemeasurements";

fn passthrough(_: &EffectCtx, b: Buf) -> Buf {
    b
}

fn measurement_ui(id: &str) -> ParamUi {
    match id {
        "headPositionX" | "headPositionY" => slider(-100_000.0, 100_000.0, 0.0, 1920.0, 1),
        "headOrientationX" | "headOrientationY" | "headOrientationZ" => slider(-180.0, 180.0, -90.0, 90.0, 1),
        _ => slider(-100_000.0, 100_000.0, 0.0, 200.0, 1),
    }
}

pub fn specs() -> Vec<EffectSpec> {
    let points: Vec<ParamSpec> = LANDMARKS
        .iter()
        // Point defaults are fractions of the layer size: the canonical face in the middle.
        .map(|(id, name, q)| p(id, name, Value::Vec2([0.5 + 0.25 * q[0], 0.5 + 0.25 * q[1]]), ParamUi::Point))
        .collect();
    let measurements: Vec<ParamSpec> = MEASUREMENTS.iter().map(|(id, name)| p(id, name, num(0.0), measurement_ui(id))).collect();
    vec![
        EffectSpec { id: POINTS_ID, name: "Face Track Points", category: "Utility", params: points, render: passthrough, gpu: false, float: true },
        EffectSpec { id: MEASUREMENTS_ID, name: "Face Measurements", category: "Utility", params: measurements, render: passthrough, gpu: false, float: true },
    ]
}
