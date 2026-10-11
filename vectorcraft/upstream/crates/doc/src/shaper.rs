//! Persistent Shaper edits over retained source paths. Source indices (not document ids) keep
//! recipes valid when a group is copied. Selector anchors use source-relative bounding boxes,
//! so the intended face or stroke follows source art when it moves or resizes.
use crate::Appearance;
use serde::{Deserialize, Serialize};
use vectorcraft_geom::Point;

pub const SOURCES: &str = "Shaper Sources";
pub const FILL_PREFIX: &str = "Shaper Fill ";
pub const STROKE_PREFIX: &str = "Shaper Stroke ";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ShaperSpec {
    pub faces: Vec<FaceEdit>,
    pub edges: Vec<EdgeEdit>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FaceSelector {
    pub sources: Vec<usize>,
    pub anchor: Point,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FaceEdit {
    pub selector: FaceSelector,
    pub erase: bool,
    pub erase_edges: bool,
    pub paint_source: Option<usize>,
    pub merge: Option<u64>,
    pub paint: Option<Appearance>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EdgeEdit {
    pub source: usize,
    pub sides: [Vec<usize>; 2],
    pub anchor: Point,
    pub erase: bool,
    pub paint: Option<Appearance>,
}
