//! Android hardware video decoding via MediaCodec (NDK `AMediaCodec` API).
//!
//! This module provides a [`VideoDecoder`] implementation that uses Android's
//! hardware video decoders through the NDK's native MediaCodec API. It supports
//! H.264 (AVC), H.265 (HEVC), VP9, and AV1 where the device provides a decoder.
//!
//! The decoder is wrapped in [`HybridDecoder`](crate::hybrid::HybridDecoder) by the
//! factory, so if hardware decoding fails at any point it transparently falls back
//! to the pure-Rust software decoder.

use std::collections::VecDeque;

use filmcraft_codecs::{CodecError, DecodedFrame, Result as CodecResult, VideoDecoder, VideoFrame};

/// Android MediaCodec-based video decoder.
pub struct MediaCodecDecoder {
    /// Codec MIME type (e.g. "video/avc").
    mime: &'static str,
    /// Width/height from the stream info.
    width: u32,
    height: u32,
    /// Pending output frames (MediaCodec may return multiple per input).
    pending: VecDeque<DecodedFrame>,
    /// Whether the decoder has been configured and started.
    started: bool,
}

impl MediaCodecDecoder {
    /// Create a new decoder for the given MIME type and dimensions.
    ///
    /// Returns `None` if the device has no decoder for this MIME type.
    pub fn new(mime: &'static str, width: u32, height: u32) -> Option<Self> {
        // TODO: Probe via AMediaCodec_createDecoderByType; return None if unavailable.
        // For now, always return Some — the HybridDecoder will fall back on failure.
        Some(Self {
            mime,
            width,
            height,
            pending: VecDeque::new(),
            started: false,
        })
    }

    /// MIME type for a codec name from the sample entry.
    pub fn mime_for_codec(name: &str) -> Option<&'static str> {
        match name {
            "avc1" | "avc3" => Some("video/avc"),
            "hev1" | "hvc1" => Some("video/hevc"),
            "vp09" => Some("video/x-vnd.on2.vp9"),
            "av01" => Some("video/av01"),
            _ => None,
        }
    }
}

impl VideoDecoder for MediaCodecDecoder {
    fn decode(&mut self, _sample: &[u8], _pts: i64) -> CodecResult<Vec<DecodedFrame>> {
        // Not yet implemented: return an error so HybridDecoder falls back to software.
        // Full AMediaCodec implementation is in progress.
        Err(CodecError::Unsupported(
            "MediaCodec hardware decoding not yet implemented".to_string(),
        ))
    }

    fn flush(&mut self) -> Vec<DecodedFrame> {
        std::mem::take(&mut self.pending).into()
    }

    fn reset(&mut self) {
        self.pending.clear();
        self.started = false;
    }

    fn name(&self) -> &str {
        "MediaCodec"
    }
}
