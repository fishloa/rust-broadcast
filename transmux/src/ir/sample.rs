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

use crate::TrackId;
use alloc::sync::Arc;
use broadcast_common::timescale::{Rounding, rescale_i64};
use bytes::Bytes;

use super::DisplayInfo;

use crate::annexb::annexb_to_length_prefixed;

/// Per-sample flags (ISO/IEC 14496-12:2026 §8.8.3.1 `sample_flags`, reduced to
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
///
/// `#[non_exhaustive]`: construct with [`Provenance::new`] — a future
/// container's own raw stamp (e.g. an RTP 32-bit timestamp alongside the
/// TS/PES pair) should be an additive field, not a major bump.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Provenance {
    /// Raw wire decode timestamp, before rollover unwrap.
    pub wire_dts: Option<u64>,
    /// Raw wire presentation timestamp, before rollover unwrap.
    pub wire_pts: Option<u64>,
}

impl Provenance {
    /// Build a provenance record from a source container's raw, pre-unwrap
    /// wire stamps. Pass `None` for a stamp the container did not carry —
    /// never a fabricated value.
    pub fn new(wire_dts: Option<u64>, wire_pts: Option<u64>) -> Self {
        Self { wire_dts, wire_pts }
    }
}

/// Absolute decode/presentation timing for a [`Sample`], all in the track's
/// media timescale ticks. `None` on [`Sample::timing`] **only** for
/// section-carried tracks (SCTE-35/DSM-CC/private sections), which carry no
/// timestamp at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct SampleTiming {
    /// Absolute decode timestamp.
    pub dts: i64,
    /// Absolute presentation timestamp.
    pub pts: i64,
    /// Sample duration, when known.
    pub duration: u32,
}

impl SampleTiming {
    /// Build a timing triple.
    #[must_use]
    pub const fn new(dts: i64, pts: i64, duration: u32) -> Self {
        Self { dts, pts, duration }
    }

    /// Combine three optional wire values into one optional timing. `None`
    /// only when all three are `None` (section-carried tracks).
    #[must_use]
    pub fn from_options(dts: Option<i64>, pts: Option<i64>, duration: Option<u32>) -> Option<Self> {
        if dts.is_none() && pts.is_none() && duration.is_none() {
            None
        } else {
            Some(Self::new(
                dts.unwrap_or(0),
                pts.unwrap_or(dts.unwrap_or(0)),
                duration.unwrap_or(0),
            ))
        }
    }
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
    /// Absolute decode/presentation timestamps and duration, in the track's
    /// media timescale. `None` **only** for section-carried tracks.
    pub timing: Option<SampleTiming>,
    /// Per-sample flags (currently just the sync-sample bit).
    pub flags: SampleFlags,
    /// Debug-only original wire timestamps, when the source container
    /// carried an independently-observable raw stamp. See [`Provenance`].
    pub provenance: Option<Provenance>,
    /// The display state in force for **this** access unit
    /// (`docs/display-geometry/DESIGN.md` §5.3). `Some` for every sample of a
    /// video track from a demuxer that tracks display state (today the TS
    /// demuxer's AVC/HEVC paths); the first sample after a change shares the
    /// exact `Arc` with the [`DemuxEvent::DisplayChanged`](super::DemuxEvent::DisplayChanged)
    /// that preceded it. `None` means "this demuxer/sample does not track
    /// display state" — never "unchanged": a consumer falls back to
    /// [`TrackSpec::display`](super::TrackSpec::display) then.
    ///
    /// The tag travels with the sample, so it survives IR transforms and a
    /// `media-plane` `Trunk` unchanged.
    pub display: Option<Arc<DisplayInfo>>,
}

impl Sample {
    /// This sample with `dts`, `pts` and `duration` re-expressed from a
    /// `from` Hz media timescale in a `to` Hz one; the instant each describes
    /// is unchanged.
    #[must_use]
    pub fn rescale_timescale(mut self, from: broadcast_common::timescale::Timescale, to: broadcast_common::timescale::Timescale) -> Self {
        if from == to {
            return self;
        }
        if let Some(t) = self.timing.as_mut() {
            t.dts = rescale_i64(t.dts, from, to, Rounding::Floor);
            t.pts = rescale_i64(t.pts, from, to, Rounding::Floor);
            t.duration = u32::try_from(rescale_i64(i64::from(t.duration), from, to, Rounding::Floor))
                .unwrap_or(u32::MAX);
        }
        self
    }

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
            timing: if dts.is_none() && pts.is_none() && duration.is_none() {
                None
            } else {
                Some(SampleTiming::new(
                    dts.unwrap_or(0),
                    pts.unwrap_or(dts.unwrap_or(0)),
                    duration.unwrap_or(0),
                ))
            },
            flags: SampleFlags::new(is_sync),
            provenance: None,
            display: None,
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
            timing: if dts.is_none() && pts.is_none() && duration.is_none() {
                None
            } else {
                Some(SampleTiming::new(
                    dts.unwrap_or(0),
                    pts.unwrap_or(dts.unwrap_or(0)),
                    duration.unwrap_or(0),
                ))
            },
            flags: SampleFlags::new(is_sync),
            provenance: None,
            display: None,
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
            timing: if dts.is_none() && pts.is_none() && duration.is_none() {
                None
            } else {
                Some(SampleTiming::new(
                    dts.unwrap_or(0),
                    pts.unwrap_or(dts.unwrap_or(0)),
                    duration.unwrap_or(0),
                ))
            },
            flags: SampleFlags::SYNC,
            provenance: None,
            display: None,
        }
    }

    /// Attach debug-only [`Provenance`] (the source container's raw,
    /// pre-unwrap wire stamps), returning `self` (builder style).
    pub fn with_provenance(mut self, p: Provenance) -> Self {
        self.provenance = Some(p);
        self
    }

    /// Attach the display state in force for this access unit, returning
    /// `self` (builder style). See [`Sample::display`].
    pub fn with_display(mut self, d: Arc<DisplayInfo>) -> Self {
        self.display = Some(d);
        self
    }

    /// This sample's composition offset (`pts - dts`) in media-timescale
    /// ticks, when both are known; `0` when either is `None` (matches the
    /// old relative model's default for a sample carrying no explicit
    /// offset).
    pub fn composition_offset(&self) -> i32 {
        match self.timing {
            Some(t) => (t.pts - t.dts).clamp(i32::MIN as i64, i32::MAX as i64) as i32,
            None => 0,
        }
    }
}

/// One track's samples for a single media segment.
///
/// `#[non_exhaustive]`: construct with [`FragmentTrackData::new`].
#[non_exhaustive]
pub struct FragmentTrackData<'a> {
    /// Track ID matching a [`crate::ir::TrackSpec`] from the init segment.
    pub track_id: TrackId,
    /// The decode time of the first sample, in media-timescale ticks.
    pub base_media_decode_time: u64,
    /// The samples for this fragment, in decode order.
    pub samples: &'a [Sample],
}

impl<'a> FragmentTrackData<'a> {
    /// Build one track's fragment payload from its `tfdt` anchor and its
    /// samples in decode order.
    pub fn new(track_id: TrackId, base_media_decode_time: u64, samples: &'a [Sample]) -> Self {
        Self {
            track_id,
            base_media_decode_time,
            samples,
        }
    }
}
