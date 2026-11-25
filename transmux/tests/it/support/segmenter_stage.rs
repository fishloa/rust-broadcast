//! Test-side drivers for the [`Segmenter`] stage.
//!
//! The former inherent `push`/`take_ready`/`take_ready_with_meta`/`flush`/
//! `mark_discontinuity` methods are gone: the stage speaks [`MuxInput`] and
//! emits [`MuxOutput`]. These helpers keep the tests' call sites readable.

use broadcast_common::{Stage, Timestamp};
use transmux::mux_stage::{MuxInput, MuxOutput, SegmentOut};
use transmux::{Sample, Segmenter, TrackId};

/// Feed one sample to `seg`.
pub fn feed(seg: &mut Segmenter, track: TrackId, sample: Sample) -> transmux::Result<()> {
    seg.feed(MuxInput::Sample { track, sample }, Timestamp::ZERO)
}

/// Drain every finished segment.
pub fn take(seg: &mut Segmenter) -> Vec<SegmentOut> {
    let mut out = Vec::new();
    while let Some(output) = seg.poll() {
        if let MuxOutput::Segment(segment) = output {
            out.push(*segment);
        }
    }
    out
}

/// Drain every finished segment's bytes.
pub fn take_bytes(seg: &mut Segmenter) -> Vec<Vec<u8>> {
    take(seg)
        .into_iter()
        .map(|segment| segment.bytes.to_vec())
        .collect()
}

/// Close the trailing partial segment (`MuxInput::Flush`).
pub fn flush(seg: &mut Segmenter) -> transmux::Result<()> {
    seg.feed(MuxInput::Flush, Timestamp::ZERO)
}

/// Mark the next cut discontinuous (`MuxInput::Discontinuity`).
pub fn mark_discontinuity(seg: &mut Segmenter) -> transmux::Result<()> {
    seg.feed(MuxInput::Discontinuity { track: None }, Timestamp::ZERO)
}
