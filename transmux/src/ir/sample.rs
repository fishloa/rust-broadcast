//! Coded access units — [`Sample`], [`SampleFlags`], [`Provenance`],
//! [`FragmentTrackData`].
//!
//! Media plane step 2c
//! (`docs/superpowers/specs/2026-07-26-media-plane-architecture.md` §4): a
//! sample's decode/presentation time is now an **absolute, optional** tick
//! pair in the track's media timescale, not a running sum anchored on the
//! track's start decode time. `None` is the honest
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
use alloc::vec::Vec;
use broadcast_common::timescale::{Rounding, rescale_i64};
use bytes::Bytes;
use core::num::NonZeroU32;

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
/// `WebM` carries no decode time at all (`crate::matroska_demux`), and a
/// standalone `rtpdump` file has no `RTP-Info` to anchor one (`crate::rtp`).
/// Such a sample knows how long it lasts and nothing about when it starts, so
/// it says exactly that instead of borrowing a `0` timestamp. `None` on
/// [`Sample::timing`] is reserved for a section-carried track
/// (SCTE-35/DSM-CC/private sections), which carries neither. A timed sample
/// carries its own positive span where the source stated one, and `None`
/// where it did not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SampleTiming {
    /// Absolute decode and presentation timestamps.
    Timed {
        /// Absolute decode timestamp.
        dts: i64,
        /// Absolute presentation timestamp.
        pts: i64,
        /// The sample's span, when the source stated one or the demuxer
        /// derived one as before; a positive tick count.
        duration: Option<NonZeroU32>,
    },
    /// A sample whose span is known and whose timestamps are not.
    DurationOnly {
        /// The sample's duration, in media-timescale ticks; a positive count.
        duration: NonZeroU32,
    },
}

impl SampleTiming {
    /// Absolute decode/presentation timestamps, with no span stated.
    #[must_use]
    pub const fn timed(dts: i64, pts: i64) -> Self {
        Self::Timed {
            dts,
            pts,
            duration: None,
        }
    }

    /// Absolute decode/presentation timestamps with the span the source
    /// stated; a `duration` of zero stores `None` (no span stated).
    #[must_use]
    pub const fn timed_with_duration(dts: i64, pts: i64, duration: u32) -> Self {
        Self::Timed {
            dts,
            pts,
            duration: NonZeroU32::new(duration),
        }
    }

    /// A sample that knows its duration and not its timestamps. `duration`
    /// must be positive: a zero-tick span is not a duration a writer can
    /// declare, so it is reported as [`crate::Error::InvalidInput`].
    pub fn duration_only(duration: u32) -> crate::error::Result<Self> {
        match NonZeroU32::new(duration) {
            Some(duration) => Ok(Self::DurationOnly { duration }),
            None => Err(crate::Error::InvalidInput(
                "SampleTiming::duration_only: a duration is a positive tick count",
            )),
        }
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
            Self::Timed { duration, .. } => match duration {
                Some(duration) => Some(duration.get()),
                None => None,
            },
            Self::DurationOnly { duration } => Some(duration.get()),
        }
    }

    /// Move the timestamps of a [`SampleTiming::Timed`] to `dts`/`pts` (one
    /// shared shift applied to both, so `pts - dts` is unchanged); a
    /// [`SampleTiming::DurationOnly`] sample has none to move and is returned
    /// as it is.
    pub fn shift(&mut self, delta: i64) {
        if let Self::Timed { dts, pts, .. } = self {
            *dts = dts.saturating_add(delta);
            *pts = pts.saturating_add(delta);
        }
    }

    /// This (timed) timing with its timestamps set to `dts`/`pts`, its span
    /// kept: the shape a decode-timestamp deriver writes back, its
    /// presentation time untouched.
    #[must_use]
    pub const fn with_composition(self, dts: i64, pts: i64) -> Self {
        match self {
            Self::Timed { duration, .. } => Self::Timed { dts, pts, duration },
            Self::DurationOnly { duration } => Self::DurationOnly { duration },
        }
    }

    /// This timing with its span set to `duration`; a timed sample keeps its
    /// timestamps and takes the span, a duration-only one takes the span.
    ///
    /// A span is a positive tick count: no value of this method states "no
    /// duration", so an unknown span is communicated by not calling it (see
    /// [`Sample::timing`](super::Sample)). It cannot fail — a positive tick
    /// count is exactly what a span is — so callers never have to discard an
    /// error they could not act on.
    #[must_use]
    pub fn with_duration(self, duration: NonZeroU32) -> Self {
        match self {
            Self::Timed { dts, pts, .. } => Self::Timed {
                dts,
                pts,
                duration: Some(duration),
            },
            Self::DurationOnly { .. } => Self::DurationOnly { duration },
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
    /// This sample's CENC auxiliary information (initialization vector and
    /// subsample map), or `None` for a cleartext sample (ISO/IEC 23001-7
    /// §12.3). An `Arc` so cloning a sample stays a refcount bump.
    pub protection: Option<Arc<SampleProtection>>,
}

/// One sample's CENC auxiliary information (ISO/IEC 23001-7 §12.3): its
/// initialization vector and, for a subsample-encrypted sample, the
/// clear/protected ranges.
///
/// The IR-side dual of [`isobmff::writer::SampleAux`], the shape the fragment
/// writer takes: [`From`] converts one way and back, so the field list exists
/// once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SampleProtection {
    /// The sample's initialization vector, as carried: 0 bytes for a
    /// constant-IV track, else 8 or 16 (ISO/IEC 23001-7 §12.2).
    pub iv: Bytes,
    /// The clear/protected ranges of a subsample-encrypted sample; empty when
    /// the whole sample is protected.
    pub subsamples: Vec<isobmff::cenc::SubSampleEntry>,
}

impl From<&isobmff::writer::SampleAux> for SampleProtection {
    fn from(aux: &isobmff::writer::SampleAux) -> Self {
        Self {
            iv: Bytes::copy_from_slice(&aux.initialization_vector),
            subsamples: aux.subsamples.clone(),
        }
    }
}

impl From<&SampleProtection> for isobmff::writer::SampleAux {
    fn from(protection: &SampleProtection) -> Self {
        Self {
            initialization_vector: protection.iv.to_vec(),
            subsamples: protection.subsamples.clone(),
        }
    }
}

impl Sample {
    /// A timed sample with an absolute decode and presentation timestamp, and
    /// the span the source stated (`None` where it stated none).
    ///
    /// Infallible, because both timestamps are given: a decode timestamp is
    /// never invented from a presentation one. Use [`SampleTiming::timed`] to
    /// build the timing alone.
    #[must_use]
    pub fn timed(data: impl Into<Bytes>, dts: i64, pts: i64, duration: Option<NonZeroU32>) -> Self {
        Self {
            data: data.into(),
            timing: Some(SampleTiming::Timed { dts, pts, duration }),
            flags: SampleFlags::new(false),
            provenance: None,
            display: None,
            protection: None,
        }
    }

    /// A timed sample from an Annex B access unit (NAL units converted to the
    /// length-prefixed form), both timestamps given; see [`Self::timed`].
    ///
    /// # Errors
    /// [`crate::Error::InvalidInput`] when a NAL unit falls outside the
    /// length-prefixed `mdat` form (see [`Sample::from_annexb`]); the
    /// timestamps themselves are never the reason.
    pub fn timed_annexb(
        annexb: &[u8],
        dts: i64,
        pts: i64,
        duration: Option<NonZeroU32>,
        is_sync: bool,
    ) -> crate::error::Result<Self> {
        Ok(Self {
            data: annexb_to_length_prefixed(annexb).into(),
            timing: Some(SampleTiming::Timed { dts, pts, duration }),
            flags: SampleFlags::new(is_sync),
            provenance: None,
            display: None,
            protection: None,
        })
    }

    /// A timed sample from a raw coded frame (audio is always a sync sample),
    /// both timestamps given; see [`Self::timed`].
    #[must_use]
    pub fn timed_raw(
        data: impl Into<Bytes>,
        dts: i64,
        pts: i64,
        duration: Option<NonZeroU32>,
    ) -> Self {
        Self {
            data: data.into(),
            timing: Some(SampleTiming::Timed { dts, pts, duration }),
            flags: SampleFlags::SYNC,
            provenance: None,
            display: None,
            protection: None,
        }
    }

    /// This sample with `dts`, `pts` and `duration` re-expressed from a
    /// `from` Hz media timescale in a `to` Hz one; the instant each describes
    /// is unchanged.
    ///
    /// # Errors
    /// [`crate::Error::InvalidInput`] when a rescaled span no longer fits the
    /// 32-bit tick count a duration is (every span rescales to at least one
    /// tick, so a span never silently becomes nothing).
    pub fn rescale_timescale(
        self,
        from: broadcast_common::timescale::Timescale,
        to: broadcast_common::timescale::Timescale,
    ) -> crate::error::Result<Self> {
        if from == to {
            return Ok(self);
        }
        let timing = self
            .timing
            .map(|timing| -> crate::error::Result<SampleTiming> {
                Ok(match timing {
                    SampleTiming::Timed { dts, pts, duration } => SampleTiming::Timed {
                        dts: rescale_i64(dts, from, to, Rounding::Floor),
                        pts: rescale_i64(pts, from, to, Rounding::Floor),
                        duration: duration
                            .map(|duration| rescale_ticks(duration, from, to))
                            .transpose()?,
                    },
                    SampleTiming::DurationOnly { duration } => SampleTiming::DurationOnly {
                        duration: rescale_ticks(duration, from, to)?,
                    },
                })
            });
        let timing = timing.transpose()?;
        Ok(Self { timing, ..self })
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
    ///
    /// # Errors
    /// Whatever the timing builder reports: a `pts` without a `dts`, or a zero span
    /// with no timestamps. A caller that has **both** timestamps uses
    /// [`Sample::timed`], which cannot fail: nothing it is told is
    /// unrepresentable.
    pub fn new(
        data: impl Into<Bytes>,
        dts: Option<i64>,
        pts: Option<i64>,
        duration: Option<u32>,
        is_sync: bool,
    ) -> crate::error::Result<Self> {
        Ok(Self {
            data: data.into(),
            timing: timing_of(dts, pts, duration)?,
            flags: SampleFlags::new(is_sync),
            provenance: None,
            display: None,
            protection: None,
        })
    }

    /// Build a video sample from an Annex B access unit, converting its NAL
    /// units to the length-prefixed `mdat` form. `dts`/`pts` are absolute, in
    /// the track's media timescale.
    ///
    /// # Errors
    /// Whatever the timing builder reports: a `pts` without a `dts`, or a zero span
    /// with no timestamps. With both timestamps given, use
    /// [`Sample::timed_annexb`], whose only failure is a NAL unit that cannot
    /// be length-prefixed (never a timestamp).
    pub fn from_annexb(
        annexb: &[u8],
        dts: Option<i64>,
        pts: Option<i64>,
        duration: Option<u32>,
        is_sync: bool,
    ) -> crate::error::Result<Self> {
        Ok(Self {
            data: annexb_to_length_prefixed(annexb).into(),
            timing: timing_of(dts, pts, duration)?,
            flags: SampleFlags::new(is_sync),
            provenance: None,
            display: None,
            protection: None,
        })
    }

    /// Build an audio sample from a raw coded frame (e.g. an AAC access
    /// unit). Audio samples are always sync samples. `dts`/`pts` are
    /// absolute, in the track's media timescale.
    ///
    /// # Errors
    /// Whatever the timing builder reports: a `pts` without a `dts`, or a zero span
    /// with no timestamps. With both timestamps given, use
    /// [`Sample::timed_raw`], which cannot fail.
    pub fn from_raw(
        data: impl Into<Bytes>,
        dts: Option<i64>,
        pts: Option<i64>,
        duration: Option<u32>,
    ) -> crate::error::Result<Self> {
        Ok(Self {
            data: data.into(),
            timing: timing_of(dts, pts, duration)?,
            flags: SampleFlags::SYNC,
            provenance: None,
            display: None,
            protection: None,
        })
    }

    /// A sample that declares only its span (no timestamps at all): the
    /// reader measured how long it lasts and has no instant to give it.
    ///
    /// # Errors
    /// [`crate::Error::InvalidInput`] for a `duration` that is negative or
    /// too large for a 32-bit tick count; a zero span is refused like any
    /// other non-positive one, exactly as [`SampleTiming::duration_only`]
    /// reports it (turning it into one tick would invent a length the reader
    /// never measured).
    pub fn from_duration(
        data: impl Into<Bytes>,
        duration: NonZeroU32,
        is_sync: bool,
    ) -> crate::error::Result<Self> {
        Ok(Self {
            data: data.into(),
            timing: Some(SampleTiming::DurationOnly { duration }),
            flags: SampleFlags::new(is_sync),
            provenance: None,
            display: None,
            protection: None,
        })
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

    /// Attach this sample's CENC auxiliary information, returning `self`
    /// (builder style). See [`Sample::protection`].
    #[must_use]
    pub fn with_protection(mut self, protection: Arc<SampleProtection>) -> Self {
        self.protection = Some(protection);
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
        let offset = pts
            .checked_sub(dts)
            .ok_or(crate::Error::TimestampSpanOverflow)?;
        i32::try_from(offset)
            .map(Some)
            .map_err(|_| crate::Error::CompositionOffsetRange { offset })
    }
}

/// The timing a sample's three optional wire values describe: both
/// timestamps (with the stated span kept) for a `Timed` sample, a positive
/// span alone for a `DurationOnly` one, and `None` when the sample carries
/// neither.
///
/// # Errors
/// [`crate::Error::InvalidInput`] when `dts` is absent but `pts` is present:
/// no value of a decode timestamp can be read out of a presentation one, and
/// inventing `dts = pts` is the fabrication [`SampleTiming`] exists to
/// prevent — the caller either states both (§2.4.3.7 leaves the pair open) or
/// builds a span-only sample from a measured duration. A zero `duration` with
/// no timestamps is likewise refused by [`SampleTiming::duration_only`], not
/// swallowed. A timed sample keeps the span it was given, including an
/// explicit zero, exactly as stated.
pub fn timing_of(
    dts: Option<i64>,
    pts: Option<i64>,
    duration: Option<u32>,
) -> crate::error::Result<Option<SampleTiming>> {
    match (dts, pts) {
        (Some(dts), pts) => Ok(Some(SampleTiming::Timed {
            dts,
            pts: pts.unwrap_or(dts),
            duration: duration.and_then(NonZeroU32::new),
        })),
        (None, Some(_)) => Err(crate::Error::InvalidInput(
            "a sample with a PTS needs a DTS to be tied to the decode timeline",
        )),
        (None, None) => match duration {
            Some(duration) => SampleTiming::duration_only(duration).map(Some),
            None => Ok(None),
        },
    }
}

/// `ticks` rescaled from `from` to `to`, floored at one tick: a nonzero span
/// never rescales away to nothing.
fn rescale_ticks(
    ticks: NonZeroU32,
    from: broadcast_common::timescale::Timescale,
    to: broadcast_common::timescale::Timescale,
) -> crate::error::Result<NonZeroU32> {
    let rescaled = rescale_i64(i64::from(ticks.get()), from, to, Rounding::Floor);
    let rescaled = u32::try_from(rescaled).map_err(|_| {
        crate::Error::InvalidInput("a rescaled sample duration does not fit the 32-bit field")
    })?;
    NonZeroU32::new(rescaled.max(1)).ok_or(crate::Error::InvalidInput(
        "a rescaled sample duration is at least one tick",
    ))
}

/// One track's samples for a single media segment.
///
/// `#[non_exhaustive]`: construct with [`FragmentTrackData::new`].
#[non_exhaustive]
pub struct FragmentTrackData<'a> {
    /// Track ID matching a [`crate::TrackSpec`] from the init segment.
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

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_common::timescale::Timescale;

    fn nz(value: u32) -> NonZeroU32 {
        NonZeroU32::new(value).expect("a positive span")
    }

    /// A span is a positive tick count: setting one always states exactly
    /// that span, and an unknown span is expressed by not setting one.
    #[test]
    fn with_duration_states_a_positive_span_and_nothing_else() {
        let timing = SampleTiming::timed(5, 6).with_duration(nz(3));
        assert_eq!(timing.duration(), Some(3));
        let duration_only = SampleTiming::duration_only(7)
            .expect("a positive span")
            .with_duration(nz(9));
        assert_eq!(duration_only.duration(), Some(9));
    }

    /// A PTS with no DTS is refused, never resolved as `dts = pts`.
    #[test]
    fn a_pts_without_a_dts_is_refused_not_invented() {
        let err = Sample::new(vec![0u8], None, Some(1), None, false)
            .expect_err("a decode timestamp cannot be read out of a presentation one");
        assert!(matches!(err, crate::Error::InvalidInput(_)));
        let timed = Sample::new(vec![0u8], Some(1), None, None, false).expect("a dts alone");
        assert_eq!(timed.timing.and_then(|t| t.dts()), Some(1));
    }

    /// A zero span with no timestamps is reported, not swallowed into a
    /// spanless sample.
    #[test]
    fn a_zero_span_without_timestamps_is_refused() {
        assert!(Sample::new(vec![0u8], None, None, Some(0), false).is_err());
        assert!(Sample::from_raw(vec![0u8], None, None, Some(0)).is_err());
    }

    /// A timed span the source stated as zero stays that stated span.
    #[test]
    fn a_stated_zero_span_is_kept_on_a_timed_sample() {
        let timing = SampleTiming::timed_with_duration(0, 0, 0);
        assert_eq!(timing.dts(), Some(0));
        assert_eq!(timing.duration(), None);
    }

    /// Rescaling a span is a checked conversion: it never clamps, and the
    /// same timescale is the identity.
    #[test]
    fn rescaling_a_span_is_checked() {
        let from = Timescale::try_new(90_000).expect("90 kHz");
        let sample = Sample::timed(vec![0u8], 1, 1, NonZeroU32::new(90_000));
        let same = sample
            .clone()
            .rescale_timescale(from, from)
            .expect("the identity rescale");
        assert_eq!(same.timing.and_then(|t| t.duration()), Some(90_000));
        let to = Timescale::try_new(1_000).expect("1 kHz");
        let rescaled = sample
            .rescale_timescale(from, to)
            .expect("a representable span");
        assert_eq!(rescaled.timing.and_then(|t| t.duration()), Some(1_000));
    }
}
