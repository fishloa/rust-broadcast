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
/// still modulo `1 << 33`, *distinct* from `Sample::timing` (`.dts`)/`Sample::timing` (`.pts`)
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

/// The decode/presentation timing of a [`Sample`], in the track's media
/// timescale ticks: a [`SampleTiming::Timed`] sample has an absolute
/// `(dts, pts)` pair, a [`SampleTiming::DurationOnly`] one is timed only by
/// its span.
///
/// `WebM` carries no decode time at all (`crate::webm_demux`), and a
/// standalone `rtpdump` file has no `RTP-Info` to anchor one (`crate::rtp`).
/// Such a sample knows how long it lasts and nothing about when it starts, so
/// it says exactly that instead of borrowing a `0` timestamp. `None` on
/// [`Sample::timing`] is reserved for a section-carried track
/// (SCTE-35/DSM-CC/private sections), which carries neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SampleTiming {
    /// Absolute decode and presentation timestamps.
    Timed {
        /// Absolute decode timestamp.
        dts: i64,
        /// Absolute presentation timestamp.
        pts: i64,
    },
    /// A sample whose span is known and whose timestamps are not.
    DurationOnly {
        /// The sample's duration, in media-timescale ticks; a positive count.
        duration: u32,
    },
}

impl SampleTiming {
    /// Absolute decode/presentation timestamps.
    #[must_use]
    pub const fn timed(dts: i64, pts: i64) -> Self {
        Self::Timed { dts, pts }
    }

    /// A sample that knows its duration and not its timestamps. `duration`
    /// must be positive: a zero-tick span is not a duration a writer can
    /// declare, so it is reported as [`crate::Error::InvalidInput`].
    pub fn duration_only(duration: u32) -> crate::error::Result<Self> {
        if duration == 0 {
            return Err(crate::Error::InvalidInput(
                "SampleTiming::duration_only: a duration is a positive tick count",
            ));
        }
        Ok(Self::DurationOnly { duration })
    }

    /// The absolute decode timestamp, when the sample has one.
    #[must_use]
    pub const fn dts(&self) -> Option<i64> {
        match self {
            Self::Timed { dts, .. } => Some(*dts),
            Self::DurationOnly { .. } => None,
        }
    }

    /// The absolute presentation timestamp, when the sample has one.
    #[must_use]
    pub const fn pts(&self) -> Option<i64> {
        match self {
            Self::Timed { pts, .. } => Some(*pts),
            Self::DurationOnly { .. } => None,
        }
    }

    /// The sample's duration in ticks, when it is known.
    #[must_use]
    pub const fn duration(&self) -> Option<u32> {
        match self {
            Self::Timed { .. } => None,
            Self::DurationOnly { duration } => Some(*duration),
        }
    }

    /// Move the timestamps of a [`SampleTiming::Timed`] to `dts`/`pts` (one
    /// shared shift applied to both, so `pts - dts` is unchanged); a
    /// [`SampleTiming::DurationOnly`] sample has none to move and is returned
    /// as it is.
    pub fn shift(&mut self, delta: i64) {
        if let Self::Timed { dts, pts } = self {
            *dts = dts.saturating_add(delta);
            *pts = pts.saturating_add(delta);
        }
    }

    /// This (timed) timing with its timestamps set to `dts`/`pts`, its span
    /// left to whichever source states it next: the shape a decode-timestamp
    /// deriver writes back, its presentation time untouched.
    #[must_use]
    pub const fn with_composition(self, dts: i64, pts: i64) -> Self {
        match self {
            Self::Timed { .. } => Self::Timed { dts, pts },
            Self::DurationOnly { duration } => Self::DurationOnly { duration },
        }
    }

    /// This timing with its span set to `duration`; a timed sample has no
    /// span to keep, so the result states only the span.
    #[must_use]
    pub const fn with_duration(self, duration: u32) -> Self {
        match self {
            Self::Timed { .. } => Self::DurationOnly { duration },
            Self::DurationOnly { .. } => Self::DurationOnly { duration },
        }
    }

    /// A sample with `dts`/`pts` and the duration `duration`, when the
    /// caller has one to declare.
    #[must_use]
    pub const fn measured(dts: i64, pts: i64, duration: Option<u32>) -> Self {
        match duration {
            Some(duration) if duration == 0 => Self::timed(dts, pts),
            Some(duration) => Self::DurationOnly { duration },
            None => Self::timed(dts, pts),
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
    pub fn rescale_timescale(
        self,
        from: broadcast_common::timescale::Timescale,
        to: broadcast_common::timescale::Timescale,
    ) -> Self {
        if from == to {
            return self;
        }
        let timing = self.timing.map(|timing| match timing {
            SampleTiming::Timed { dts, pts } => SampleTiming::timed(
                rescale_i64(dts, from, to, Rounding::Floor),
                rescale_i64(pts, from, to, Rounding::Floor),
            ),
            SampleTiming::DurationOnly { duration } => SampleTiming::DurationOnly {
                duration: rescale_ticks(duration, from, to),
            },
        });
        Self { timing, ..self }
    }

    /// Build a sample from already-encoded bytes with every field explicit
    /// (issue #580: the general-purpose constructor now that `Sample` is
    /// `#[non_exhaustive]` and cannot be struct-literal-constructed outside
    /// this crate). `data` must already be in this crate's wire form
    /// (length-prefixed for AVC/HEVC) — use [`Sample::from_annexb`] to
    /// convert an Annex B access unit instead.
    ///
    /// `dts`/`pts` are **absolute**, in the track's media timescale; pass
    /// both `None` only for a sample whose span is all it knows (a
    /// `DurationOnly` timing) or, with no `duration` either, for a genuinely
    /// timestamp-less (section-carried) sample. Never fabricate a value.
    pub fn new(
        data: impl Into<Bytes>,
        dts: Option<i64>,
        pts: Option<i64>,
        duration: Option<u32>,
        is_sync: bool,
    ) -> Self {
        Self {
            data: data.into(),
            timing: timing_of(dts, pts, duration),
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
            timing: timing_of(dts, pts, duration),
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
            timing: timing_of(dts, pts, duration),
            flags: SampleFlags::SYNC,
            provenance: None,
            display: None,
        }
    }

    /// A sample that declares only its span (no timestamps at all): the
    /// reader measured how long it lasts and has no instant to give it.
    pub fn from_duration(data: impl Into<Bytes>, duration: u32, is_sync: bool) -> Self {
        Self {
            data: data.into(),
            timing: Some(SampleTiming::DurationOnly { duration }),
            flags: SampleFlags::new(is_sync),
            provenance: None,
            display: None,
        }
    }

    /// This sample with its timing replaced, returning `self` (builder
    /// style) — the reader-side way to fill in a span it measured only after
    /// the sample was built.
    #[must_use]
    pub fn with_timing(mut self, timing: Option<SampleTiming>) -> Self {
        self.timing = timing;
        self
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
    /// ticks, when it has timestamps to take one from; `None` for a sample
    /// that is `DurationOnly` or carries no timing at all (a section-carried
    /// track). The difference is computed with `checked_sub`, so a `pts`
    /// below `i64::MIN + dts` is
    /// [`crate::Error::InvalidInput`] rather than a wrap.
    pub fn composition_offset(&self) -> crate::error::Result<Option<i32>> {
        let Some(timing) = self.timing else {
            return Ok(None);
        };
        let (Some(dts), Some(pts)) = (timing.dts(), timing.pts()) else {
            return Ok(None);
        };
        let offset = pts.checked_sub(dts).ok_or(crate::Error::InvalidInput(
            "sample timestamps span more than i64",
        ))?;
        i32::try_from(offset).map(Some).map_err(|_| {
            crate::Error::InvalidInput("sample composition offset does not fit the 32-bit field")
        })
    }
}

/// The timing a sample's three optional wire values describe: both
/// timestamps for a `Timed` sample, a positive span alone for a
/// `DurationOnly` one, and `None` when the sample carries neither.
fn timing_of(dts: Option<i64>, pts: Option<i64>, duration: Option<u32>) -> Option<SampleTiming> {
    let timed = match (dts, pts) {
        (Some(dts), pts) => Some(SampleTiming::timed(dts, pts.unwrap_or(dts))),
        (None, Some(pts)) => Some(SampleTiming::timed(pts, pts)),
        (None, None) => None,
    };
    timed.or_else(|| duration.and_then(|duration| SampleTiming::duration_only(duration).ok()))
}

/// `ticks` rescaled from `from` to `to`, floored at one tick: a nonzero span
/// never rescales away to nothing.
fn rescale_ticks(
    ticks: u32,
    from: broadcast_common::timescale::Timescale,
    to: broadcast_common::timescale::Timescale,
) -> u32 {
    u32::try_from(rescale_i64(i64::from(ticks), from, to, Rounding::Floor))
        .unwrap_or(u32::MAX)
        .max(1)
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
