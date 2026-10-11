//! Image Trace objects' View: how a live trace draws on screen.
//!
//! An Image Trace object is a group whose `trace` holds `{preset, params, view}` (see
//! `imageTrace.make`). The view is a viewing aid only: exports, printing and Expand always use the
//! tracing result, whatever it is set to.

use crate::node::Node;

/// Image Trace › View.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TraceView {
    /// The traced shapes as they are filled.
    #[default]
    Result,
    /// The traced shapes, with their outlines over them.
    ResultWithOutlines,
    /// Only the traced shapes' outlines.
    Outlines,
    /// The source image, with the traced shapes' outlines over it.
    OutlinesWithSource,
    /// Only the source image.
    Source,
}

impl TraceView {
    /// Panel order.
    pub const ALL: [Self; 5] = [Self::Result, Self::ResultWithOutlines, Self::Outlines, Self::OutlinesWithSource, Self::Source];

    /// The id used in command params and in the saved `trace` object.
    pub fn id(self) -> &'static str {
        match self {
            Self::Result => "tracingResult",
            Self::ResultWithOutlines => "tracingResultWithOutlines",
            Self::Outlines => "outlines",
            Self::OutlinesWithSource => "outlinesWithSourceImage",
            Self::Source => "sourceImage",
        }
    }

    /// The view an id names (any case).
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.id().eq_ignore_ascii_case(id))
    }

    /// The view an Image Trace object's `trace` settings ask for (the tracing result when they
    /// name none, or an unknown one).
    pub fn of(trace: &serde_json::Value) -> Self {
        trace.get("view").and_then(serde_json::Value::as_str).and_then(Self::from_id).unwrap_or_default()
    }

    /// Whether the source image shows.
    pub fn shows_source(self) -> bool {
        matches!(self, Self::OutlinesWithSource | Self::Source)
    }

    /// Whether the traced shapes show filled.
    pub fn shows_result(self) -> bool {
        matches!(self, Self::Result | Self::ResultWithOutlines)
    }

    /// Whether the traced shapes' outlines show.
    pub fn shows_outlines(self) -> bool {
        matches!(self, Self::ResultWithOutlines | Self::Outlines | Self::OutlinesWithSource)
    }
}

impl Node {
    /// The View of an Image Trace object (the tracing result for anything else, or an unknown id).
    pub fn trace_view(&self) -> TraceView {
        self.trace.as_deref().map(TraceView::of).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_and_unknown_views_show_the_result() {
        for v in TraceView::ALL {
            assert_eq!(TraceView::from_id(v.id()), Some(v));
            assert_eq!(TraceView::from_id(&v.id().to_ascii_uppercase()), Some(v));
            assert!(v.shows_result() || v.shows_outlines() || v.shows_source(), "{v:?} shows something");
        }
        let mut g = Node::group(crate::NodeId(1), vec![]);
        assert_eq!(g.trace_view(), TraceView::Result);
        g.trace = Some(Box::new(serde_json::json!({"view": "outlines"})));
        assert_eq!(g.trace_view(), TraceView::Outlines);
        g.trace = Some(Box::new(serde_json::json!({"view": "sepia"})));
        assert_eq!(g.trace_view(), TraceView::Result);
    }
}
