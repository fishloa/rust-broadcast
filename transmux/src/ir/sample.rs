//! Coded access units — [`Sample`], [`SourceTiming`], [`FragmentTrackData`].
//!
//! Moved out of `pipeline.rs` (media plane step 2a, no-op): same types, same
//! fields, same impls.

use alloc::vec::Vec;

use crate::annexb::annexb_to_length_prefixed;

/// Explicit per-sample timestamps recovered from the source container, in the
/// source's own clock — for TS/PES sources the 33-bit-unwrapped 90 kHz PES
/// clock (ISO/IEC 13818-1 §2.4.3.7). `None` when the source carries no
/// per-sample timestamps or the sample's time was synthesized.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceTiming {
    /// Decode timestamp (90 kHz for TS sources), unwrapped.
    pub dts: u64,
    /// Presentation timestamp (90 kHz for TS sources), unwrapped.
    pub pts: u64,
}

/// A single coded sample (access unit) fed to [`crate::pipeline::build_media_segment`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Sample {
    /// Coded bytes: **length-prefixed** NAL data for AVC/HEVC, or the raw frame
    /// for AAC. Use [`Sample::from_annexb`] to convert an Annex B access unit.
    pub data: Vec<u8>,
    /// Sample duration in the track's media timescale.
    pub duration: u32,
    /// Whether this is a sync sample (random-access point / keyframe).
    pub is_sync: bool,
    /// Composition time offset (`pts − dts`) in media-timescale ticks.
    pub composition_offset: i32,
    /// Explicit source-container timestamps, when the source carries them
    /// per-sample (see [`SourceTiming`]). All mux paths in this crate ignore
    /// this field — fMP4 output timing stays duration-based
    /// ([`FragmentTrackData::base_media_decode_time`] + running `duration` sum).
    pub source_timing: Option<SourceTiming>,
}

impl Sample {
    /// Build a sample from already-encoded bytes with every field explicit
    /// (issue #580: the general-purpose constructor now that `Sample` is
    /// `#[non_exhaustive]` and cannot be struct-literal-constructed outside
    /// this crate). `data` must already be in this crate's wire form
    /// (length-prefixed for AVC/HEVC) — use [`Sample::from_annexb`] to
    /// convert an Annex B access unit instead.
    pub fn new(data: Vec<u8>, duration: u32, is_sync: bool, composition_offset: i32) -> Self {
        Self {
            data,
            duration,
            is_sync,
            composition_offset,
            source_timing: None,
        }
    }

    /// Build a video sample from an Annex B access unit, converting its NAL
    /// units to the length-prefixed `mdat` form.
    pub fn from_annexb(
        annexb: &[u8],
        duration: u32,
        is_sync: bool,
        composition_offset: i32,
    ) -> Self {
        Self {
            data: annexb_to_length_prefixed(annexb),
            duration,
            is_sync,
            composition_offset,
            source_timing: None,
        }
    }

    /// Build an audio sample from a raw coded frame (e.g. an AAC access unit).
    pub fn from_raw(data: Vec<u8>, duration: u32) -> Self {
        Self {
            data,
            duration,
            is_sync: true,
            composition_offset: 0,
            source_timing: None,
        }
    }

    /// Attach explicit [`SourceTiming`] recovered from the source container,
    /// returning `self` (builder style).
    pub fn with_source_timing(mut self, t: SourceTiming) -> Self {
        self.source_timing = Some(t);
        self
    }
}

/// One track's samples for a single media segment.
pub struct FragmentTrackData<'a> {
    /// Track ID matching a [`crate::ir::TrackSpec`] from the init segment.
    pub track_id: u32,
    /// The decode time of the first sample, in media-timescale ticks.
    pub base_media_decode_time: u64,
    /// The samples for this fragment, in decode order.
    pub samples: &'a [Sample],
}
