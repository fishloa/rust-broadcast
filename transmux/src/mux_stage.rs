//! The one input and output vocabulary every muxer [`Stage`](broadcast_common::Stage)
//! speaks: a muxer is `Stage<In<'a> = MuxInput, Out = MuxOutput>`.
//!
//! A muxer holds at most one output unit (a fragment, a TS PES, an FLV tag,
//! a Matroska cluster, an RTP packet group). Manifest packagers never see
//! samples: they render from the [`SegmentSummary`] each [`MuxOutput::Segment`]
//! carries.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use bytes::Bytes;

use crate::ir::{Sample, TrackId, TrackSpec};

/// What a muxer stage is fed.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum MuxInput {
    /// A track is announced, or an announced track's setup changed.
    Track(Box<TrackSpec>),
    /// One coded sample of an announced track, in decode order per track.
    Sample {
        /// The announced track this sample belongs to.
        track: TrackId,
        /// The sample.
        sample: Sample,
    },
    /// The media timeline is discontinuous before the next sample of
    /// `track`, or of every track when `None`.
    ///
    /// A stage that can express the break records it: an HLS
    /// `#EXT-X-DISCONTINUITY` on the next cut, the next fragment's
    /// `SegmentSummary`, or the MPEG-2 TS adaptation-field
    /// `discontinuity_indicator` on the next packet of each affected
    /// elementary PID. One that cannot refuses it with
    /// [`Error::UnsupportedFeature`](crate::Error::UnsupportedFeature) rather
    /// than returning `Ok(())` and dropping the mark.
    Discontinuity {
        /// The track the discontinuity applies to; `None` for all tracks.
        track: Option<TrackId>,
    },
    /// Close the unit being built now, without waiting for its natural cut.
    Flush,
}

/// What a muxer stage produces.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum MuxOutput {
    /// Bytes appended to the output stream.
    Bytes(Bytes),
    /// Bytes written over an earlier part of the output at `offset`; only a
    /// muxer writing to a seekable output emits this.
    BytesAt {
        /// Absolute byte offset in the output.
        offset: u64,
        /// The bytes to write there.
        bytes: Bytes,
    },
    /// An initialisation segment.
    Init(Bytes),
    /// One finished media segment and its summary.
    Segment(Box<SegmentOut>),
    /// A rendered playlist or manifest.
    Text(String),
}

/// One finished media segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentOut {
    /// The segment's bytes.
    pub bytes: Bytes,
    /// What a manifest needs to describe the segment as a whole.
    pub summary: SegmentSummary,
    /// One entry per track the segment carries, in the segment's track order:
    /// what a per-representation manifest needs (its own start, duration, byte
    /// count and sample count), which the segment-level summary cannot express.
    pub tracks: Vec<TrackPart>,
}

/// One track's share of a segment: the per-representation view a DASH or Smooth
/// manifest describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrackPart {
    /// The track this part belongs to.
    pub track: TrackId,
    /// Decode time of this track's first sample in the segment, in
    /// [`Self::timescale`] ticks.
    pub start: u64,
    /// This track's duration in the segment, in [`Self::timescale`] ticks.
    pub duration: u64,
    /// Ticks per second of [`Self::start`] and [`Self::duration`].
    pub timescale: u32,
    /// This track's bytes in the segment: its own fragment bytes where the
    /// container separates tracks, its coded sample bytes where it multiplexes
    /// them (an MPEG-2 TS segment).
    pub bytes: u64,
    /// The samples this track contributed to the segment.
    pub samples: u32,
}

/// What a manifest needs to know about one segment, without its samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SegmentSummary {
    /// The track whose timeline cut the segment (the anchor track of a
    /// multiplexed segment).
    pub track: TrackId,
    /// 0-based, ever-increasing segment number for this output.
    pub sequence: u64,
    /// Decode time of the segment's first sample, in `timescale` ticks.
    pub start: u64,
    /// Segment duration, in `timescale` ticks.
    pub duration: u64,
    /// Ticks per second of `start` and `duration`.
    pub timescale: u32,
    /// Size of the segment's bytes.
    pub bytes: u64,
    /// The segment starts with a sync sample.
    pub independent: bool,
    /// The segment follows a timeline discontinuity.
    pub discontinuous: bool,
}
