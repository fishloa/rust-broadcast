//! Coded access units — [`Sample`], [`SampleFlags`], [`Provenance`],
//! [`FragmentTrackData`].
//!
//! Media plane step 2c
//! (`docs/superpowers/specs/2026-07-26-media-plane-architecture.md` §4): a
//! sample's decode/presentation time is now an **absolute, optional** tick
//! pair in the track's media timescale, not a running sum anchored on
//! [`crate::ir::Track::start_decode_time`]. `None` is the honest
//! representation for section-carried tracks (SCTE-35/DSM-CC/private
//! sections), which genuinely have no per-sample timestamp — it must never be
//! fabricated. Rollover (33-bit TS/PES, 32-bit RTP) is unwrapped **once, at
//! the demux edge** (see `crate::ts_demux`), matching
//! `timed_metadata::Timeline`'s semantics; nothing downstream re-derives it.
//!
//! The write-only `SourceTiming` this superseded is gone; the source
//! container's raw (pre-unwrap) wire stamps survive only as debug-only
//! [`Provenance`], which no mux/demux logic in this crate reads back.

use bytes::Bytes;

use crate::annexb::annexb_to_length_prefixed;

/// Per-sample flags (ISO/IEC 14496-12:2015 §8.8.3.1 `sample_flags`, reduced to
/// the one bit this crate's IR tracks today).
///
/// `#[non_exhaustive]`: a struct, not a bare `bool`, so a future flag (e.g.
/// `is_non_displayable`/`padding`) is additive, not a `Sample` field-list
/// break.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct SampleFlags {
    /// Whether this is a sync sample (random-access point / keyframe).
    pub is_sync: bool,
}

impl SampleFlags {
    /// A sync sample (random-access point / keyframe).
    pub const SYNC: SampleFlags = SampleFlags { is_sync: true };
    /// A non-sync sample.
    pub const NON_SYNC: SampleFlags = SampleFlags { is_sync: false };

    /// `SampleFlags` for the given sync-sample state.
    pub fn new(is_sync: bool) -> Self {
        Self { is_sync }
    }
}

/// Debug-only provenance: the source container's **raw wire-clock**
/// timestamps, before any rollover unwrap — e.g. the folded 33-bit TS/PES
/// value straight off the PES optional header (ISO/IEC 13818-1 §2.4.3.7),
/// still modulo `1 << 33`, *distinct* from [`Sample::dts`]/[`Sample::pts`]
/// which carry the unwrapped absolute value. Kept for inspection when
/// diagnosing an unwrap bug (`provenance.wire_dts % (1 << 33)` should always
/// equal `dts as u64 % (1 << 33)`); no mux or demux logic in this crate reads
/// it back — it replaces the write-only `SourceTiming` this crate carried
/// before media plane step 2c, without pretending a debug field is a timing
/// model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Provenance {
    /// Raw wire decode timestamp, before rollover unwrap.
    pub wire_dts: Option<u64>,
    /// Raw wire presentation timestamp, before rollover unwrap.
    pub wire_pts: Option<u64>,
}

/// A single coded sample (access unit) fed to [`crate::pipeline::build_media_segment`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Sample {
    /// Coded bytes: **length-prefixed** NAL data for AVC/HEVC, or the raw frame
    /// for AAC. Use [`Sample::from_annexb`] to convert an Annex B access unit.
    ///
    /// `Bytes` (not `Vec<u8>`, media plane step 2b, issue #564-adjacent /
    /// `docs/superpowers/specs/2026-07-26-media-plane-architecture.md` §4):
    /// fan-out to N consumers is a refcount bump, not a payload copy, and an
    /// RTP packetiser can slice a frame into packets without copying each
    /// one. A shared, immutable buffer costs something back on the in-place
    /// rewrite paths (`cenc_encrypt`/`cenc_decrypt`) — see those modules'
    /// `Bytes::try_into_mut` fast path.
    pub data: Bytes,
    /// Absolute decode timestamp, in the track's media timescale
    /// ([`crate::ir::TrackSpec::timescale`]) ticks. `None` **only** for
    /// section-carried tracks (SCTE-35 `stream_type` 0x86, DSM-CC, private
    /// sections — see `crate::ts_demux`), which carry no timestamp at all;
    /// never fabricated. 33-bit (TS) / 32-bit (RTP) rollover is unwrapped
    /// once, at the demux edge — this value is already absolute, not a wire
    /// wrap-modulo value (see [`Provenance`] for the pre-unwrap wire stamp).
    pub dts: Option<i64>,
    /// Absolute presentation timestamp, in the track's media timescale ticks
    /// (`dts + composition_offset` under the old relative model — the offset
    /// is now folded directly into the absolute pair). `None` under the same
    /// rule as [`Sample::dts`].
    pub pts: Option<i64>,
    /// Sample duration in the track's media timescale, when known. `None`
    /// alongside `dts`/`pts` for section-carried tracks (a section has no
    /// duration either); every timed (video/audio) sample from every
    /// demuxer in this crate carries `Some`.
    pub duration: Option<u32>,
    /// Per-sample flags (currently just the sync-sample bit).
    pub flags: SampleFlags,
    /// Debug-only original wire timestamps, when the source container
    /// carried an independently-observable raw stamp. See [`Provenance`].
    pub provenance: Option<Provenance>,
}

impl Sample {
    /// Build a sample from already-encoded bytes with every field explicit
    /// (issue #580: the general-purpose constructor now that `Sample` is
    /// `#[non_exhaustive]` and cannot be struct-literal-constructed outside
    /// this crate). `data` must already be in this crate's wire form
    /// (length-prefixed for AVC/HEVC) — use [`Sample::from_annexb`] to
    /// convert an Annex B access unit instead.
    ///
    /// `dts`/`pts` are **absolute**, in the track's media timescale; pass
    /// `None` only for a genuinely timestamp-less (section-carried) sample —
    /// never fabricate a value.
    pub fn new(
        data: impl Into<Bytes>,
        dts: Option<i64>,
        pts: Option<i64>,
        duration: Option<u32>,
        is_sync: bool,
    ) -> Self {
        Self {
            data: data.into(),
            dts,
            pts,
            duration,
            flags: SampleFlags::new(is_sync),
            provenance: None,
        }
    }

    /// Build a video sample from an Annex B access unit, converting its NAL
    /// units to the length-prefixed `mdat` form. `dts`/`pts` are absolute, in
    /// the track's media timescale.
    pub fn from_annexb(
        annexb: &[u8],
        dts: Option<i64>,
        pts: Option<i64>,
        duration: Option<u32>,
        is_sync: bool,
    ) -> Self {
        Self {
            data: annexb_to_length_prefixed(annexb).into(),
            dts,
            pts,
            duration,
            flags: SampleFlags::new(is_sync),
            provenance: None,
        }
    }

    /// Build an audio sample from a raw coded frame (e.g. an AAC access
    /// unit). Audio samples are always sync samples. `dts`/`pts` are
    /// absolute, in the track's media timescale.
    pub fn from_raw(
        data: impl Into<Bytes>,
        dts: Option<i64>,
        pts: Option<i64>,
        duration: Option<u32>,
    ) -> Self {
        Self {
            data: data.into(),
            dts,
            pts,
            duration,
            flags: SampleFlags::SYNC,
            provenance: None,
        }
    }

    /// Attach debug-only [`Provenance`] (the source container's raw,
    /// pre-unwrap wire stamps), returning `self` (builder style).
    pub fn with_provenance(mut self, p: Provenance) -> Self {
        self.provenance = Some(p);
        self
    }

    /// This sample's composition offset (`pts - dts`) in media-timescale
    /// ticks, when both are known; `0` when either is `None` (matches the
    /// old relative model's default for a sample carrying no explicit
    /// offset).
    pub fn composition_offset(&self) -> i32 {
        match (self.dts, self.pts) {
            (Some(d), Some(p)) => (p - d) as i32,
            _ => 0,
        }
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
