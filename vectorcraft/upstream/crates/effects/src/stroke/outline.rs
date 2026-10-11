//! Strokes as filled geometry outside the canvas:
//!
//! - [`for_writer`]: what a file format whose strokes only have a weight, caps, joins, a miter
//!   limit and a dash array (SVG, PDF) writes so a stroke looks as it does on the canvas: a plain
//!   stroke where that is exact, else the canvas's filled outlines (arrowheads, width profiles,
//!   dashes fitted to corners, dots);
//! - [`outline_region`]: the area a stroke paints as one clean path (Outline Stroke and the live
//!   Outline Stroke effect).

use kurbo::{BezPath, Rect, Shape};
use vectorcraft_doc::{LineCap, LineJoin, StrokeAlign, StrokeLayer};
use vectorcraft_geom::{FillRule, PathData};
use vectorcraft_pathops::BoolOp;

use super::{StrokePieces, aligned_width, is_closed, line_outline, stroke_pieces};

/// Flattening/fitting tolerance of written and outlined strokes (document points).
pub const OUTLINE_TOL: f64 = 0.01;

/// What a writer draws for one stroke (in the coordinate space of its path).
#[derive(Clone, Debug)]
pub struct Written {
    pub shape: WrittenShape,
    /// `Some` for an inside or outside stroke of a closed path: the shape is drawn twice as wide
    /// and clipped to the path's interior (inside) or has the interior removed (outside).
    pub side: Option<StrokeAlign>,
}

/// The geometry of a [`Written`] stroke.
#[derive(Clone, Debug)]
pub enum WrittenShape {
    /// A plain stroke of the path itself at `width`, with the layer's caps, joins, miter limit
    /// and dash pattern.
    Stroke { width: f64 },
    /// Filled outlines (non-zero rule): the line, then each arrowhead. They overlap, so they take
    /// the stroke's opacity once, as a group. A gradient along or across the stroke
    /// ([`StrokeLayer::path_gradient`]) paints its [`gradient_slices`](super::gradient_slices)
    /// clipped to them instead.
    Fill(Vec<BezPath>),
}

/// Does `st` draw as a plain centred stroke on every path (no alignment, arrowheads, width
/// profile, brush, fitted dashes, dots or gradient along or across it)? Writers then need no
/// geometry for it.
pub fn is_plain(st: &StrokeLayer) -> bool {
    st.align == StrokeAlign::Center
        && st.start_arrow.is_none()
        && st.end_arrow.is_none()
        && st.profile.is_none()
        && st.brush.is_none()
        && !needs_outline_dashes(st)
        && st.path_gradient().is_none()
}

/// Dashes a plain stroke can't draw like the canvas: fitted to corners, or zero-length dashes
/// (dots) with a cap that paints them.
fn needs_outline_dashes(st: &StrokeLayer) -> bool {
    st.dash.as_ref().is_some_and(|d| d.is_dashed() && (d.align_corners || (st.cap != LineCap::Butt && d.pattern.contains(&0.0))))
}

/// Do these pieces of `st` need filled outlines (see [`is_plain`])? Alignment alone doesn't; a
/// gradient along or across the stroke paints slices clipped to them.
fn needs_outline(pieces: &StrokePieces, st: &StrokeLayer) -> bool {
    !pieces.heads.is_empty() || st.profile.is_some() || needs_outline_dashes(st) || st.path_gradient().is_some()
}

/// How a writer draws stroke `st` along `bp` (brushes aside) to match the canvas.
pub fn for_writer(bp: &BezPath, st: &StrokeLayer) -> Written {
    if is_plain(st) {
        return Written { shape: WrittenShape::Stroke { width: st.width }, side: None };
    }
    let closed = is_closed(bp);
    let side = (st.align != StrokeAlign::Center && closed).then_some(st.align);
    let width = aligned_width(st, closed);
    let pieces = stroke_pieces(bp, st);
    let shape = if needs_outline(&pieces, st) {
        let mut fills = vec![line_outline(&pieces.line, st, width, OUTLINE_TOL)];
        fills.extend(pieces.heads.into_iter().map(|h| h.outline));
        fills.retain(|f| !f.elements().is_empty());
        WrittenShape::Fill(fills)
    } else {
        WrittenShape::Stroke { width }
    };
    Written { shape, side }
}

impl Written {
    /// Bounds of what is drawn before the alignment clip, for a path with bounds `path_bounds`
    /// (covers miter spikes and projecting caps).
    pub fn reach(&self, st: &StrokeLayer, path_bounds: Rect) -> Rect {
        match &self.shape {
            WrittenShape::Stroke { width } => {
                let miter = if st.join == LineJoin::Miter { st.miter_limit.max(1.0) } else { 1.0 };
                let cap = if st.cap == LineCap::Square { std::f64::consts::SQRT_2 } else { 1.0 };
                let r = width / 2.0 * miter.max(cap);
                path_bounds.inflate(r, r)
            }
            WrittenShape::Fill(fills) => fills.iter().map(Shape::bounding_box).fold(path_bounds, |a, b| a.union(b)),
        }
    }
}

/// The area stroke `st` paints along `path` (fill rule `rule`) as one clean path, as on the
/// canvas: the line (width profile, dashes, caps, joins) united with the arrowheads, and kept
/// inside or outside a closed path for inside or outside alignment. Brushes are not drawn.
pub fn outline_region(path: &PathData, rule: FillRule, st: &StrokeLayer) -> PathData {
    if !(st.width > 0.0 && st.width.is_finite()) || path.is_empty() {
        return PathData::default();
    }
    let bp = path.to_bezpath();
    let closed = is_closed(&bp);
    let pieces = stroke_pieces(&bp, st);
    let mut parts = vec![line_outline(&pieces.line, st, aligned_width(st, closed), OUTLINE_TOL)];
    parts.extend(pieces.heads.into_iter().map(|h| h.outline));
    let region = vectorcraft_pathops::stroke_region(&parts);
    match st.align {
        StrokeAlign::Inside if closed => vectorcraft_pathops::boolean(&region, FillRule::NonZero, path, rule, BoolOp::Intersect),
        StrokeAlign::Outside if closed => vectorcraft_pathops::boolean(&region, FillRule::NonZero, path, rule, BoolOp::Difference),
        _ => region,
    }
}
