//! The any-to-any hub impls over the [`crate::ir`] media IR.
//!
//! This module is the transmux side of the `broadcast-common` container-mux
//! vocabulary ([`broadcast_common::Unpackage`] / [`broadcast_common::Package`]).
//! The [`Media`] IR itself ([`Media`]/[`Track`]/[`TrackProtection`]/[`PcrSample`],
//! re-exported here from [`crate::ir`]) is elementary [`Track`]s of coded
//! [`Sample`]s; this module holds the packagers/depackagers that convert
//! between that IR and concrete container forms:
//!
//! - [`CmafMuxStage`] : [`Stage`] `<In = MuxInput, Out = MuxOutput>` — mux
//!   samples into a CMAF init segment plus one media segment per fragment,
//!   writing through [`isobmff::writer::FragmentedWriter`] (the same boxes
//!   [`build_init_segment`] and
//!   [`build_media_segment`](crate::pipeline::build_media_segment) emit).
//! - [`HlsPackager`] — render an RFC 8216 media playlist from a track-spec
//!   list and a [`SegmentSummaryLog`];
//!   it never sees a sample.
//!
//! The [`Track`] IR is a thin wrapper over the existing public
//! [`TrackSpec`] (codec config + timescale + track_id) and [`Sample`] (coded
//! access units); those types stay public and unchanged. `no_std` + `alloc`.
//!
//! Sample [`Decrypt`](broadcast_common::Decrypt) is implemented for CENC/CBCS
//! (ISO/IEC 23001-7 AES-CTR / AES-CBC-pattern) by
//! [`CencDecryptor`](crate::cenc_decrypt::CencDecryptor), and the inverse
//! [`Encrypt`](broadcast_common::Encrypt) by
//! [`CencEncryptor`](crate::cenc_encrypt::CencEncryptor) — both behind the
//! `cenc` feature (issues #465/#564). Either direction records its per-track
//! crypto metadata onto [`TrackSpec::protection`](crate::ir::TrackSpec::protection) and per-sample [`SampleProtection`](crate::ir::SampleProtection).

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use broadcast_common::timescale::{Rounding, Timescale, cmp_ticks, rescale_i64};
use broadcast_common::{Parse, Stage, Timestamp};

use crate::codec_config_ext::VvcRecordCodecInfo;
use broadcast_hls::{ByteRange, DecimalSeconds, MapTag, MediaPlaylist, MediaSegment};
use isobmff::ac3::{Ac3SpecificBox, Ec3SpecificBox};

use crate::display_track::{SpsTracker, TrackCodec};
use crate::error::{Error, Result};
use crate::ir::{
    AspectRatioIdc, CodecConfig, ColourDescription, DataIdentity, DisplayGeometry, DisplayInfo,
    EXTENDED_SAR_IDC, PictureRect, Sample, SubtitleFormat, SubtitleSampleEntry, TrackId, TrackSpec,
    VideoFormat, VideoSignalType,
};
use crate::mpeg_legacy::{Mpeg2SeqHeader, MpegAudioFrameHeader, MpegAudioLayer};
use crate::mux_stage::{MuxInput, MuxOutput, SegmentOut, SegmentSummary};
use crate::pipeline::build_init_segment;
use crate::segment_log::{SegmentSummaryLog, TrackSummary};
use crate::segmenter::{MediaClock, choose_anchor};
use alloc::sync::Arc;
use bytes::Bytes;
use isobmff::ac4::Ac4SpecificBox;
use isobmff::box_support::OpaqueBox;
use isobmff::box_types::BOX_TYPE_SIZE;
use isobmff::dts::DtsSpecificBox;
use isobmff::flac::FlacSpecificBox;
use isobmff::init_segment::{SampleEntryVariant, sampling_rate_override};
use isobmff::mp4esds::{EsdsBox, OTI_MPEG1_AUDIO, OTI_MPEG2_AUDIO};
use isobmff::mpegh::{MHAC_FOURCC, MHADecoderConfigurationRecord};
use isobmff::opus::OpusSpecificBox;
use isobmff::reader::TrackInfo;
use isobmff::visual_ext::{CleanApertureBox, ColourInformationBox, PixelAspectRatioBox};

/// [`Media`], [`Track`], [`TrackProtection`], [`PcrSample`] moved to
/// [`crate::ir`] (media plane step 2a) — re-exported here so every existing
/// `crate::media::`/`transmux::media::` path keeps resolving unchanged.
pub use crate::ir::{Media, PcrSample, SkippedTrack, Track, TrackProtection};

/// `#EXT-X-VERSION` for the fMP4 playlists [`HlsPackager`] writes (RFC 8216 §7).
const DEFAULT_HLS_VERSION: u8 = 7;

// ---------------------------------------------------------------------------
// CmafMuxStage — Stage<In = MuxInput, Out = MuxOutput>
// ---------------------------------------------------------------------------

/// How a [`CmafMuxStage`] decides to close the fragment it is building.
///
/// There is no default: a caller states the policy, because the fragment
/// duration is caller policy, not a property of the format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FragmentCut {
    /// Close the fragment only on [`MuxInput::Flush`] or the end of the
    /// stream — one media segment for the whole input, which is what the
    /// former whole-[`Media`] `CmafMux` emitted.
    OnFlush,
    /// Close a fragment before `track`'s next sync sample once the anchor
    /// track has buffered `target` of media time. The new fragment opens on
    /// that sync sample, as CMAF requires.
    AnchorKeyframe {
        /// The track whose sync samples open a fragment (the segmentation
        /// anchor).
        track: TrackId,
        /// The buffered media time that must elapse before a cut.
        target: core::time::Duration,
    },
}

/// Mux samples into CMAF fragments (ISO/IEC 14496-12:2026 §8.8) as a
/// streaming [`Stage`].
///
/// Announce every track with [`MuxInput::Track`], feed its coded samples in
/// decode order with [`MuxInput::Sample`], and read [`MuxOutput::Init`] once,
/// then one [`MuxOutput::Segment`] per fragment. The fragment's `moof`/`mdat`
/// is byte-identical to the crate's batch builders
/// ([`build_init_segment`] + [`build_media_segment`](crate::pipeline::build_media_segment)) for the same samples.
///
/// At most one fragment is buffered: [`FragmentCut`] decides when it closes,
/// and it is capped by [`isobmff::Limits::max_fragment_bytes`] (a hostile
/// input is refused with [`isobmff::Error::FragmentTooLarge`]).
pub struct CmafMuxStage {
    movie_timescale: Timescale,
    cut: FragmentCut,
    sequence_number: u32,
    limits: isobmff::Limits,
    tracks: Vec<TrackSpec>,
    writer: Option<isobmff::writer::FragmentedWriter>,
    track_started: Vec<bool>,
    /// Per-track elapsed-media accounting, so each fragment's `tfdt` advances
    /// with that track's own samples (a `duration: Some(0)` stream uses the
    /// `dts` delta instead).
    track_clock: Vec<MediaClock>,
    /// Each track's next absolute decode time — the `tfdt.baseMediaDecodeTime`
    /// a newly opened fragment starts it at.
    track_next_decode: Vec<u64>,
    anchor: Option<TrackId>,
    anchor_clock: MediaClock,
    anchor_duration: u64,
    anchor_start: Option<u64>,
    anchor_independent: bool,
    pending_discontinuity: bool,
    buffered: bool,
    /// Per-track bytes accumulated since the fragment opened.
    part_bytes: Vec<u64>,
    /// Per-track sample counts accumulated since the fragment opened.
    part_samples: Vec<u32>,
    /// Per-track decode time of the fragment's first sample, once seen.
    part_start: Vec<Option<u64>>,
    next_segment_sequence: u64,
    ready: alloc::collections::VecDeque<MuxOutput>,
}

impl core::fmt::Debug for CmafMuxStage {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CmafMuxStage")
            .field("movie_timescale", &self.movie_timescale)
            .field("cut", &self.cut)
            .field("sequence_number", &self.sequence_number)
            .field("tracks", &self.tracks)
            .field("started", &self.writer.is_some())
            .field("next_segment_sequence", &self.next_segment_sequence)
            .finish_non_exhaustive()
    }
}

impl CmafMuxStage {
    /// Create a muxer for `movie_timescale` (the `mvhd` timescale), cutting
    /// fragments as `cut` states and numbering the first `moof` with
    /// `sequence_number`.
    pub fn new(
        movie_timescale: impl Into<Timescale>,
        cut: FragmentCut,
        sequence_number: u32,
    ) -> Self {
        Self {
            movie_timescale: movie_timescale.into(),
            cut,
            sequence_number,
            limits: isobmff::Limits::default(),
            tracks: Vec::new(),
            writer: None,
            track_started: Vec::new(),
            track_clock: Vec::new(),
            track_next_decode: Vec::new(),
            anchor: None,
            anchor_clock: MediaClock::new(),
            anchor_duration: 0,
            anchor_start: None,
            anchor_independent: true,
            pending_discontinuity: false,
            buffered: false,
            part_bytes: Vec::new(),
            part_samples: Vec::new(),
            part_start: Vec::new(),
            next_segment_sequence: 0,
            ready: alloc::collections::VecDeque::new(),
        }
    }

    /// The initialization segment (`ftyp` + fragmented-init `moov`), or
    /// `None` until at least one track is announced.
    ///
    /// # Errors
    /// Whatever [`build_init_segment`] returns.
    pub fn init_segment(&self) -> Result<Option<Vec<u8>>> {
        if self.tracks.is_empty() {
            return Ok(None);
        }
        Ok(Some(build_init_segment(
            &self.tracks,
            self.movie_timescale,
        )?))
    }

    fn announce(&mut self, spec: TrackSpec) -> Result<()> {
        if self.writer.is_some() {
            return match self.tracks.iter().find(|t| t.track_id == spec.track_id) {
                Some(existing) if *existing == spec => Ok(()),
                Some(_) => Err(Error::InvalidInput(
                    "CmafMuxStage: a track's setup changed after its first sample; start \
                     a new CmafMuxStage for a changed track set",
                )),
                None => Err(Error::InvalidInput(
                    "CmafMuxStage: a track was announced after the first sample; the track \
                     set of a fragment sequence is fixed",
                )),
            };
        }
        match self.tracks.iter_mut().find(|t| t.track_id == spec.track_id) {
            Some(existing) => *existing = spec,
            None => self.tracks.push(spec),
        }
        self.part_bytes.resize(self.tracks.len(), 0);
        self.part_samples.resize(self.tracks.len(), 0);
        self.part_start.resize(self.tracks.len(), None);
        Ok(())
    }

    fn ensure_writer(&mut self) -> Result<()> {
        if self.writer.is_some() || self.tracks.is_empty() {
            return Ok(());
        }
        let setups = crate::pipeline::track_setups(&self.tracks)?;
        let mut writer =
            isobmff::writer::Writer::fragmented(setups, self.movie_timescale, self.limits)?;
        writer.set_sequence_number(self.sequence_number);
        for spec in &self.tracks {
            writer.set_base_decode_time(isobmff::TrackId::new(spec.track_id.value()), 0)?;
        }
        let init = writer.init()?;
        self.ready.push_back(MuxOutput::Init(Bytes::from(init)));
        self.anchor = choose_anchor(self.tracks.iter().map(|t| &t.config))
            .ok()
            .map(|index| self.tracks[index].track_id);
        self.track_started = alloc::vec![false; self.tracks.len()];
        self.track_clock = alloc::vec![MediaClock::new(); self.tracks.len()];
        self.track_next_decode = alloc::vec![0; self.tracks.len()];
        self.writer = Some(writer);
        Ok(())
    }

    fn sample(&mut self, track: TrackId, sample: Sample) -> Result<()> {
        let index =
            self.tracks
                .iter()
                .position(|t| t.track_id == track)
                .ok_or(Error::InvalidInput(
                    "CmafMuxStage: sample for an unannounced track",
                ))?;
        self.ensure_writer()?;
        let dts = sample.timing.and_then(|t| t.dts()).unwrap_or(0).max(0) as u64;
        let is_anchor = self.anchor == Some(track);
        if is_anchor {
            let cut = match self.cut {
                FragmentCut::AnchorKeyframe {
                    track: cut_track,
                    target,
                } => {
                    let timescale = self.tracks[index].timescale.get();
                    let target_ticks = duration_ticks(target, timescale);
                    cut_track == track
                        && sample.flags.is_sync
                        && self.buffered
                        && self.anchor_duration >= target_ticks
                }
                _ => false,
            };
            if cut {
                self.flush_fragment()?;
            }
        }
        if !self.buffered {
            self.begin_fragment()?;
        }
        if !self.track_started[index] {
            if let Some(writer) = self.writer.as_mut() {
                writer.set_base_decode_time(isobmff::TrackId::new(track.value()), dts)?;
            }
            self.track_next_decode[index] = dts;
            self.track_started[index] = true;
        }
        if is_anchor {
            if self.anchor_start.is_none() {
                self.anchor_start = Some(dts);
                self.anchor_independent = sample.flags.is_sync;
            }
            self.anchor_duration += self.anchor_clock.tick(&sample);
        }
        let meta = isobmff::writer::SampleMeta {
            duration: sample.timing.and_then(|t| t.duration()).unwrap_or(0),
            composition_offset: sample.composition_offset()?.unwrap_or(0),
            sync: sample.flags.is_sync,
            aux: sample
                .protection
                .as_deref()
                .map(isobmff::writer::SampleAux::from),
            description_index: isobmff::plan::DEFAULT_DESCRIPTION_INDEX,
        };
        let Some(writer) = self.writer.as_mut() else {
            return Ok(());
        };
        writer.push(isobmff::TrackId::new(track.value()), meta, &sample.data)?;
        let increment = self.track_clock[index].tick(&sample);
        self.track_next_decode[index] = self.track_next_decode[index].saturating_add(increment);
        if self.part_start.get(index).copied().flatten().is_none()
            && let Some(slot) = self.part_start.get_mut(index)
        {
            *slot = Some(dts);
        }
        if let Some(bytes) = self.part_bytes.get_mut(index) {
            *bytes = bytes.saturating_add(sample.data.len() as u64);
        }
        if let Some(count) = self.part_samples.get_mut(index) {
            *count = count.saturating_add(1);
        }
        self.buffered = true;
        Ok(())
    }

    /// Open a fragment: every track's `tfdt.baseMediaDecodeTime` is the decode
    /// time its next sample will carry, so consecutive fragments continue each
    /// track's timeline instead of restarting it.
    fn begin_fragment(&mut self) -> Result<()> {
        let Some(writer) = self.writer.as_mut() else {
            return Ok(());
        };
        for (index, spec) in self.tracks.iter().enumerate() {
            writer.set_base_decode_time(
                isobmff::TrackId::new(spec.track_id.value()),
                self.track_next_decode[index],
            )?;
        }
        Ok(())
    }

    fn flush_fragment(&mut self) -> Result<()> {
        if !self.buffered {
            return Ok(());
        }
        let brands = crate::pipeline::cmaf_segment_brands();
        let Some(writer) = self.writer.as_mut() else {
            return Ok(());
        };
        let Some(fragment) = writer.flush_with(Some(&brands), &[])? else {
            self.buffered = false;
            return Ok(());
        };
        let track = self
            .anchor
            .or_else(|| self.tracks.first().map(|t| t.track_id))
            .ok_or(Error::InvalidInput("CmafMuxStage: no track to describe"))?;
        let timescale = self
            .tracks
            .iter()
            .find(|t| t.track_id == track)
            .map_or(self.movie_timescale.get(), |t| t.timescale.get());
        let tracks: Vec<crate::mux_stage::TrackPart> = self
            .tracks
            .iter()
            .enumerate()
            .filter_map(|(index, spec)| {
                let start = self.part_start.get(index).copied().flatten()?;
                Some(crate::mux_stage::TrackPart {
                    track: spec.track_id,
                    start,
                    duration: self
                        .track_next_decode
                        .get(index)
                        .copied()
                        .unwrap_or(start)
                        .saturating_sub(start),
                    timescale: spec.timescale.get(),
                    bytes: self.part_bytes.get(index).copied().unwrap_or(0),
                    samples: self.part_samples.get(index).copied().unwrap_or(0),
                })
            })
            .collect();
        let summary = SegmentSummary {
            track,
            sequence: self.next_segment_sequence,
            start: self.anchor_start.unwrap_or(0),
            duration: self.anchor_duration,
            timescale,
            bytes: fragment.bytes.len() as u64,
            independent: self.anchor_independent,
            discontinuous: self.pending_discontinuity,
        };
        self.next_segment_sequence += 1;
        self.anchor_clock = MediaClock::new();
        self.anchor_duration = 0;
        self.anchor_start = None;
        self.anchor_independent = true;
        self.pending_discontinuity = false;
        self.buffered = false;
        self.part_bytes.iter_mut().for_each(|b| *b = 0);
        self.part_samples.iter_mut().for_each(|c| *c = 0);
        self.part_start.iter_mut().for_each(|s| *s = None);
        self.ready.push_back(MuxOutput::Segment(SegmentOut {
            bytes: Bytes::from(fragment.bytes),
            summary,
            tracks,
        }));
        Ok(())
    }
}

/// `duration` in `timescale` ticks, rounded down.
fn duration_ticks(duration: core::time::Duration, timescale: u32) -> u64 {
    let seconds = duration.as_secs();
    let nanos = u64::from(duration.subsec_nanos());
    seconds.saturating_mul(u64::from(timescale)).saturating_add(
        nanos.saturating_mul(u64::from(timescale)) / broadcast_common::units::NANOS_PER_SEC,
    )
}

impl Stage for CmafMuxStage {
    type In<'a> = MuxInput;
    type Out = MuxOutput;
    type Error = Error;

    fn feed(&mut self, input: MuxInput, _now: Timestamp) -> Result<()> {
        match input {
            MuxInput::Track(spec) => self.announce(*spec),
            MuxInput::Sample { track, sample } => self.sample(track, sample),
            MuxInput::Discontinuity { .. } => {
                self.pending_discontinuity = true;
                Ok(())
            }
            MuxInput::Flush => {
                self.ensure_writer()?;
                self.flush_fragment()
            }
        }
    }

    fn poll(&mut self) -> Option<Self::Out> {
        self.ready.pop_front()
    }

    fn finish(&mut self) -> Result<()> {
        if self.tracks.is_empty() {
            return Err(Error::InvalidInput("cannot package a Media with no tracks"));
        }
        self.ensure_writer()?;
        self.flush_fragment()
    }
}

// ---------------------------------------------------------------------------
// Shared output timeline (issues #1020-#1023)
// ---------------------------------------------------------------------------

/// The common decode-time origin of a set of tracks: the earliest first-sample
/// `Sample::timing` (`.dts`) across them, compared across timescales exactly.
///
/// Every `Sample::timing` (`.dts`) is absolute in its own track's timescale, so one
/// shift applied to every track keeps inter-track offsets and gaps intact —
/// which a per-track rebase (or a per-track `Σ duration` from zero) loses.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TimelineOrigin {
    ticks: i64,
    timescale: Timescale,
}

impl TimelineOrigin {
    /// The origin of `tracks`; `0` when no sample carries a `dts`.
    pub(crate) fn of<'t>(tracks: impl IntoIterator<Item = &'t Track>) -> Self {
        let mut best: Option<(i64, Timescale)> = None;
        for t in tracks {
            let Some(d) = t
                .samples
                .iter()
                .find_map(|s| s.timing.and_then(|t| t.dts()))
            else {
                continue;
            };
            let ts = t.spec.timescale;
            let earlier = match best {
                None => true,
                Some((bd, bts)) => cmp_ticks(d, ts, bd, bts).is_lt(),
            };
            if earlier {
                best = Some((d, ts));
            }
        }
        match best {
            Some((ticks, timescale)) => Self { ticks, timescale },
            None => Self {
                ticks: 0,
                timescale: Timescale::try_new(1).expect("non-zero timescale"),
            },
        }
    }

    /// The origin expressed in `timescale` ticks, rounded down.
    pub(crate) fn in_timescale(&self, timescale: Timescale) -> i64 {
        rescale_i64(self.ticks, self.timescale, timescale, Rounding::Floor)
    }
}

/// Each sample's decode time relative to `origin`, in the track's own
/// timescale: the sample's own absolute `dts`, falling back to the previous
/// sample's time + duration only for a track that carries no `dts` at all
/// (section-carried data), which then starts at the origin.
///
/// A sample more than the whole `i64` tick range from the origin has no
/// representable relative decode time and is [`Error::InvalidInput`] (one
/// shared timeline cannot express it; the old saturating subtraction placed
/// it at the wrong end of the timeline instead).
pub(crate) fn relative_decode_times(track: &Track, origin: &TimelineOrigin) -> Result<Vec<i64>> {
    let base = origin.in_timescale(track.spec.timescale);
    let mut out = Vec::with_capacity(track.samples.len());
    let mut next: i64 = 0;
    for s in &track.samples {
        let t = match s.timing.and_then(|t| t.dts()) {
            Some(d) => d.checked_sub(base).ok_or(Error::InvalidInput(
                "sample decode time is more than i64 ticks from the timeline origin",
            ))?,
            None => next,
        };
        out.push(t);
        next = t.saturating_add(i64::from(s.timing.and_then(|t| t.duration()).unwrap_or(0)));
    }
    Ok(out)
}

/// Presentation span in seconds of the segments `tracks` recorded: from the
/// earliest segment start to the latest segment end, each in its own track's
/// timescale.
///
/// A summary's times are decode times, so a uniform composition offset (which
/// every sample of a track carries through the summary) cancels in the span
/// exactly as it did when the span was measured per sample. A span is
/// non-negative by construction (`u64` ticks end no earlier than they start),
/// and `u64` ticks over a non-zero timescale are always a finite number of
/// seconds, so the only unrepresentable value is one the
/// [`DecimalSeconds`] constructor refuses.
pub(crate) fn presentation_span_secs_from_log(
    kept: &[(&TrackSpec, TrackSummary<'_>)],
) -> Result<f64> {
    let mut span: Option<(f64, f64)> = None;
    for (spec, summary) in kept {
        let Some(first) = summary.runs().next() else {
            continue;
        };
        if summary.timescale == 0 || spec.timescale.get() == 0 {
            return Err(Error::InvalidInput(
                "a segment summary names a zero timescale",
            ));
        }
        let timescale = f64::from(summary.timescale.max(spec.timescale.get()));
        let lo = first.start as f64 / timescale;
        let hi = summary.end() as f64 / timescale;
        span = Some(match span {
            None => (lo, hi),
            Some((low, high)) => (low.min(lo), high.max(hi)),
        });
    }
    let secs = match span {
        None => 0.0,
        Some((lo, hi)) if hi < lo => 0.0,
        Some((lo, hi)) => hi - lo,
    };
    if secs.is_finite() {
        Ok(secs)
    } else {
        Err(Error::InvalidInput("presentation span is not finite"))
    }
}

/// `secs` rounded up to whole seconds, without the std-only `f64::ceil`.
fn ceil_secs(secs: f64) -> u32 {
    let whole = secs as u32; // truncates toward zero; `secs` is non-negative
    if (whole as f64) < secs {
        whole.saturating_add(1)
    } else {
        whole
    }
}

// ---------------------------------------------------------------------------
// HlsPackager — Package<Output = String>
// ---------------------------------------------------------------------------

/// Render an RFC 8216 media playlist from the segments a muxer produced.
///
/// The playlist never sees a sample: `tracks` is the track set, and `log` the
/// [`SegmentSummary`]s [`CmafMuxStage`] emitted for it. What those describe is
/// one multi-track CMAF artifact, so the playlist carries a single `#EXTINF`
/// entry whose duration is the presentation span across every timed track,
/// pointing at `{uri_prefix}{track_id}.m4s` named after the first timed track.
/// Segments in a media playlist play one after another (RFC 8216 §3), so one
/// entry per track would play the content once per track.
#[derive(Debug, Clone)]
pub struct HlsPackager {
    /// `#EXT-X-VERSION`.
    pub version: u8,
    /// `#EXT-X-MEDIA-SEQUENCE`.
    pub msn: broadcast_hls::MediaSequence,
    /// URI prefix for the generated per-track segment entries.
    pub uri_prefix: String,
}

impl Default for HlsPackager {
    fn default() -> Self {
        Self {
            version: DEFAULT_HLS_VERSION,
            msn: broadcast_hls::MediaSequence::new(0),
            uri_prefix: String::from("seg"),
        }
    }
}

impl HlsPackager {
    /// Render the media playlist for `tracks` from the segments `log` recorded.
    ///
    /// # Errors
    /// [`Error::InvalidInput`] when `tracks` is empty, when every track is
    /// timestamp-less (a section-carried track records no duration, so it has
    /// no `#EXTINF`), when the presentation span is not a representable
    /// decimal seconds value, or when the shared init segment cannot be built.
    pub fn render(&self, tracks: &[TrackSpec], log: &SegmentSummaryLog) -> Result<String> {
        if tracks.is_empty() {
            return Err(Error::InvalidInput(
                "cannot render a playlist with no tracks",
            ));
        }
        // A section-carried track (SCTE-35 `stream_type` 0x86, DSM-CC, private
        // sections) has no timestamps at all (`Sample::duration` is `None` for
        // every sample, deliberately, so its summaries record a zero duration)
        // — see the per-track skip below for why it is omitted from the
        // playlist. Such a track is also frequently not BMFF-muxable at all (an
        // opaque `CodecConfig::Data` track), so it must be excluded here too:
        // only tracks that will actually become a `#EXTINF` segment below feed
        // the shared init segment's track list.
        let kept: Vec<(&TrackSpec, TrackSummary<'_>)> = tracks
            .iter()
            .filter_map(|spec| log.track(spec.track_id).map(|summary| (spec, summary)))
            .filter(|(_, summary)| summary.duration != 0)
            .collect();

        // Every segment this packager names (`{prefix}{track_id}.m4s`) is
        // produced downstream (`cli::package`'s `OutputFormat::Hls` arm) as a
        // *self-initializing* CMAF artifact: `build_init_segment` immediately
        // followed by `build_media_segment` in the same file — i.e. the exact
        // byte layout `CmafMux` builds (media.rs doc comment above), fed the
        // same kept-track set (`filter_for_bmff_mux` already stripped any
        // non-muxable track before either packager runs, in the real
        // pipeline). Apple's HLS Authoring Specification requires an
        // `#EXT-X-MAP` on every fMP4 Media Segment even when it is
        // self-initializing (mediastreamvalidator 1.25.36: "Each fMP4 Segment
        // in a Media Playlist MUST have an EXT-X-MAP tag applied to it" —
        // issue #870 caught this gap: no `#EXT-X-MAP` was emitted at all). The
        // Media Initialization Section is the leading `ftyp`+`moov` — exactly
        // `build_init_segment`'s output length — so every segment's
        // `#EXT-X-MAP` points at itself with a `BYTERANGE` covering just that
        // leading span; `build_init_segment` is a pure function of (specs,
        // movie_timescale), the same inputs `CmafMux` uses, so this computes
        // the true offset without needing the actual segment bytes in hand.
        let specs: Vec<TrackSpec> = kept.iter().map(|(spec, _)| (*spec).clone()).collect();
        // The init segment's length is what `#EXT-X-MAP` needs, and every box
        // in it is a fixed-size header, so the movie timescale (a four-byte
        // value inside `mvhd`) cannot change it: the first kept track's
        // timescale is a fine probe.
        let init_len = if specs.is_empty() {
            0
        } else {
            build_init_segment(&specs, specs[0].timescale)?.len() as u64
        };

        let Some((first, _)) = kept.first() else {
            return Err(Error::InvalidInput(
                "cannot render a playlist whose every track is timestamp-less \
                 (section-carried): an HLS media playlist needs at least one \
                 segment with a real EXTINF duration",
            ));
        };
        // One segment for the one multi-track CMAF artifact: RFC 8216 §3 plays
        // a media playlist's segments one after another, so a per-track entry
        // would repeat the content once per track.
        let duration = presentation_span_secs_from_log(&kept)?;
        // EXTINF rounded to the nearest integer must not exceed the target
        // duration (RFC 8216 §4.3.3.1); the ceiling always satisfies that.
        let target_secs = ceil_secs(duration);
        let uri = format!("{}{}.m4s", self.uri_prefix, first.track_id);
        let segments = vec![MediaSegment {
            map: Some(MapTag {
                uri: uri.clone(),
                byte_range: Some(ByteRange {
                    length: init_len,
                    offset: Some(0),
                }),
                extra_attrs: Vec::new(),
            }),
            uri,
            duration: DecimalSeconds::new(duration).map_err(|_| {
                Error::InvalidInput("presentation span is not a representable decimal seconds")
            })?,
            discontinuous: false,
            parts: vec![],
            ..Default::default()
        }];
        let playlist = MediaPlaylist {
            version: self.version,
            target_duration: target_secs,
            msn: self.msn,
            discontinuity_sequence: 0,
            segments,
            endlist: true,
            extra_tags: vec![],
            low_latency: None,
            iframes_only: false,
            open_segment: None,
            ..Default::default()
        };
        Ok(playlist.to_m3u8()?)
    }
}

// ---------------------------------------------------------------------------
// Helpers: TrackSpec reconstruction
// ---------------------------------------------------------------------------

/// Rebuild a [`TrackSpec`] from the reader's per-track view (identity + codec
/// config).
///
/// `pub(crate)`: the single shared sample-entry → [`CodecConfig`]
/// reconstruction path, used by [`crate::mp4_demux::Mp4Demux`], so the
/// seekable and forward-only reads never diverge on codec-config recovery.
pub(crate) fn track_spec_from_track_info(info: &TrackInfo) -> Result<TrackSpec> {
    let entry = info.entries.first().ok_or(Error::UnexpectedBox {
        expected: "stsd entry",
    })?;
    // A known four-CC whose typed parse failed is malformed, not merely a
    // sample entry transmux has no `CodecConfig` for: it keeps the
    // `TrackAbandoned` path (with the parser's own detail in the reason).
    if entry.error.is_some() {
        return Err(Error::UnsupportedSampleEntry {
            fourcc: String::from_utf8_lossy(&entry.fourcc).into_owned(),
        });
    }
    let config = match entry.typed.as_ref().map(codec_config_from_entry) {
        Some(Ok(config)) => config,
        // No `CodecConfig` for this entry (an unknown four-CC, or a typed
        // entry such as QuickTime `tmcd` this crate does not map): carry the
        // track as data, with the sample entry's own bytes so it re-muxes.
        None | Some(Err(Error::UnsupportedSampleEntry { .. })) => CodecConfig::Data {
            identity: DataIdentity::Mp4 {
                handler: info.handler,
                sample_entry: Bytes::copy_from_slice(&entry.body),
            },
        },
        Some(Err(err)) => return Err(err),
    };
    let mut spec = TrackSpec::new(info.track_id.into(), info.timescale, config);
    if let Some(protection) = track_protection_from_entry(entry)? {
        spec = spec.with_protection(protection);
    }
    // Overlay container-level display geometry (pasp / colr nclx) from the
    // sample entry onto the SPS-derived display — ISO/IEC 14496-12:2026
    // §12.1.4.1 (pasp) / §12.1.5.1 (colr). AVC/HEVC entries only; see
    // `overlay_sample_entry_display`.
    let spec = match entry.typed.as_ref() {
        Some(SampleEntryVariant::Avc1(e)) => overlay_sample_entry_display(spec, &e.extra_boxes),
        Some(SampleEntryVariant::Hevc1(e)) => overlay_sample_entry_display(spec, &e.extra_boxes),
        _ => spec,
    };
    Ok(spec)
}

/// The CENC/CBCS defaults a sample entry's `sinf` states, or `None` for a
/// cleartext entry (ISO/IEC 14496-12:2026 §13.4, ISO/IEC 23001-7 §12.2).
///
/// The per-sample IVs and subsample maps ride on the samples themselves; this
/// is only the track-level `tenc` (KID, IV size, `cbcs` pattern / constant IV)
/// and the scheme.
fn track_protection_from_entry(
    entry: &isobmff::reader::SampleEntryInfo,
) -> Result<Option<TrackProtection>> {
    let Some(sinf) = isobmff::cenc::sinf_of_sample_entry(&entry.body) else {
        return Ok(None);
    };
    let parsed = isobmff::cenc::ProtectionSchemeInfoBox::parse(sinf)?;
    let Some(scheme) = parsed
        .scheme_type
        .as_ref()
        .and_then(|ty| isobmff::cenc::CencScheme::from_four_cc(&ty.scheme_type))
    else {
        return Ok(None);
    };
    let Some(tenc) = parsed.scheme_info.and_then(|info| info.tenc) else {
        return Ok(None);
    };
    Ok(Some(TrackProtection::new(
        scheme,
        tenc,
        crate::ir::ConstantIvSenc::default(),
    )))
}

/// Overlay container-level display geometry (a `pasp` / `colr nclx` box found
/// in an AVC/HEVC sample entry) onto an SPS-derived [`DisplayGeometry`].
///
/// ISO/IEC 14496-12:2026 §12.1.4.1 (`pasp`) / §12.1.5.1 (`colr`)
/// (private/specs-verified/mpeg/isobmff/sample-entry-ext.md): both boxes
/// over-ride the declarations in the video codec's own structures. `pasp`
/// replaces the VUI sample aspect ratio, specified or not; `colr nclx`
/// replaces the bitstream colour description.
///
/// `clap` replaces the codec's crop in `output_rect` (§12.1.4.1, issue #1918).
/// Its coordinates are those of a decoded picture of the sample entry's
/// `width` x `height`: that picture is the whole coded picture when the entry
/// has the coded size, or the codec's cropped output when it has that size; an
/// entry that matches neither leaves the geometry alone, as does a `clap` that
/// is not a rectangle of whole samples inside the picture
/// ([`CleanApertureBox::aperture_within`]).
/// Returns the input unchanged when no box applies.
pub(crate) fn overlay_sample_entry_geometry(
    display: Option<DisplayGeometry>,
    config: &CodecConfig,
    extra_boxes: &[isobmff::sample_entries::OpaqueBox],
) -> Option<DisplayGeometry> {
    // First `pasp` in entry order.
    let pasp = extra_boxes
        .iter()
        .filter(|b| b.box_type == PASP_FOURCC)
        .find_map(|b| PixelAspectRatioBox::parse(&b.data).ok());

    // First `colr` whose colour_type is 'nclx' (§12.1.5.1: listed from most to
    // least accurate, so the first nclx one wins). Its code points are u16 on
    // the wire but the model's enums are built from the 8-bit code points of
    // ITU-T H.273, so a box with a code point above 255 cannot be represented
    // and is ignored (the bitstream colour is kept).
    let colour = extra_boxes
        .iter()
        .filter(|b| b.box_type == *b"colr")
        .find_map(|b| {
            let c = ColourInformationBox::parse(&b.data).ok()?;
            if c.colour_type != *b"nclx" {
                return None;
            }
            let n = c.nclx?;
            let colour = ColourDescription::from_wire(
                u8::try_from(n.colour_primaries).ok()?,
                u8::try_from(n.transfer_characteristics).ok()?,
                u8::try_from(n.matrix_coefficients).ok()?,
            );
            Some((colour, n.full_range_flag))
        });

    let clap = extra_boxes
        .iter()
        .filter(|b| b.box_type == *b"clap")
        .find_map(|b| CleanApertureBox::parse(&b.data).ok());

    // The pasp spacings reduced by their gcd to a u16 pair (both must be
    // positive, §12.1.4); a ratio of 1 is the Square idc.
    let pasp_ratio = pasp.and_then(|p| {
        if p.h_spacing == 0 || p.v_spacing == 0 {
            return None;
        }
        let g = gcd_u32(p.h_spacing, p.v_spacing);
        let (rw, rh) = (
            u16::try_from(p.h_spacing / g).ok()?,
            u16::try_from(p.v_spacing / g).ok()?,
        );
        Some(if rw == rh {
            AspectRatioIdc::from_wire(1, None)
        } else {
            AspectRatioIdc::from_wire(EXTENDED_SAR_IDC, Some((rw, rh)))
        })
    });

    if pasp_ratio.is_none() && colour.is_none() && clap.is_none() {
        return display;
    }

    // The display the SPS yielded, or (when it yielded none) one built from
    // the sample entry's own visual size.
    let (visual_w, visual_h) = match config {
        CodecConfig::Avc { width, height, .. } | CodecConfig::Hevc { width, height, .. } => {
            (u32::from(*width), u32::from(*height))
        }
        _ => (0, 0),
    };
    let signalled_format = display
        .as_ref()
        .and_then(|g| g.video_signal.map(|v| v.video_format));
    let mut display = display.unwrap_or_else(|| DisplayGeometry::new(visual_w, visual_h, None));

    if let Some(ratio) = pasp_ratio
        && ratio.sample_aspect_ratio().map(<(u16, u16)>::from) != display.sample_aspect_ratio()
    {
        display.aspect_ratio = Some(ratio);
    }

    if let Some(clap) = clap {
        let (frame_w, frame_h) = (visual_w, visual_h);
        let origin = if (frame_w, frame_h) == (display.coded_width, display.coded_height) {
            Some((0, 0))
        } else if (frame_w, frame_h) == (display.output_rect.width, display.output_rect.height) {
            Some((display.output_rect.left, display.output_rect.top))
        } else {
            None
        };
        if let (Some((x, y)), Some((left, top, width, height))) =
            (origin, clap.aperture_within(frame_w, frame_h))
        {
            display.output_rect = PictureRect::new(x + left, y + top, width, height);
        }
    }

    // colr nclx overrides the bitstream colour; the VUI's video_format is kept
    // when one was signalled (colr carries none), else Unspecified.
    if let Some((colour, full_range)) = colour {
        display.video_signal = Some(VideoSignalType::new(
            signalled_format.unwrap_or(VideoFormat::Unspecified),
            full_range,
            Some(colour),
        ));
    }

    Some(display)
}

/// Overlay container-level display geometry from a `pasp` / `colr nclx` box
/// found in an AVC/HEVC sample entry onto the SPS-derived
/// [`TrackSpec::display`].
fn overlay_sample_entry_display(
    mut spec: TrackSpec,
    extra_boxes: &[isobmff::sample_entries::OpaqueBox],
) -> TrackSpec {
    let config = spec.config.clone();
    spec.display = overlay_sample_entry_geometry(spec.display, &config, extra_boxes);
    if let Some(hdr) = crate::hdr::HdrStaticMetadata::from_sample_entry_boxes(
        extra_boxes.iter().map(|b| (&b.box_type, b.data.as_slice())),
    ) {
        spec.hdr = Some(hdr);
    }
    if let Some(dovi) = isobmff::dovi::DoviDecoderConfigurationRecord::from_sample_entry_boxes(
        extra_boxes.iter().map(|b| (&b.box_type, b.data.as_slice())),
    ) {
        spec.dovi = Some(dovi);
    }
    spec
}

/// Whether a sample entry carries parameter sets in-band (avc3/avc4/hev1),
/// plus its entry-level extra boxes for the display overlay.
pub(crate) fn track_entry_inband_and_extras(
    entry: &SampleEntryVariant,
) -> (bool, Vec<isobmff::sample_entries::OpaqueBox>) {
    match entry {
        SampleEntryVariant::Avc1(e) => (
            e.codec_type == *b"avc3" || e.codec_type == *b"avc4",
            e.extra_boxes.clone(),
        ),
        SampleEntryVariant::Hevc1(e) => (
            e.codec_type == *b"hev1" || e.codec_type == isobmff::dovi::DVHE_FOURCC,
            e.extra_boxes.clone(),
        ),
        _ => (false, Vec::new()),
    }
}

const PASP_FOURCC: [u8; 4] = *b"pasp";

/// Tag an in-band (avc3/avc4/hev1) track's samples with per-AU display state,
/// mirroring the TS demux path (`SpsTracker`).
///
/// The sample entry's `pasp` over-rides the sample aspect ratio its own
/// parameter sets declare (ISO/IEC 14496-12:2026 §12.1.4.1), so it applies to
/// every access unit whose SPS declares that same ratio. An in-band SPS that
/// declares a different ratio is a later declaration than the sample entry and
/// is reported as it is; a conforming file starts a new sample entry there.
///
/// The state is incremental: [`tag`](Self::tag) observes one access unit at a
/// time in decode order, so a streaming demuxer (which never holds the whole
/// track) produces exactly the tags the whole-input demuxer does.
pub(crate) struct InbandDisplayTagger {
    tracker: SpsTracker,
    config: CodecConfig,
    seed_sar: Option<(u16, u16)>,
    extra: Vec<isobmff::sample_entries::OpaqueBox>,
    extra_without_pasp: Vec<isobmff::sample_entries::OpaqueBox>,
    cached_raw: Option<Arc<DisplayInfo>>,
    cached_overlaid: Option<Arc<DisplayInfo>>,
}

impl InbandDisplayTagger {
    /// A tagger for an AVC/HEVC track, or `None` for any other codec.
    pub(crate) fn new(
        config: &CodecConfig,
        extra: &[isobmff::sample_entries::OpaqueBox],
    ) -> Option<Self> {
        let codec = match config {
            CodecConfig::Avc { .. } => TrackCodec::Avc,
            CodecConfig::Hevc { .. } => TrackCodec::Hevc,
            _ => return None,
        };
        let mut tracker = SpsTracker::new(codec);
        // Seed from the track's raw SPS geometry (before the container overlay).
        let raw_seed = DisplayGeometry::from_codec_config(config);
        let seed_sar = raw_seed.as_ref().and_then(|g| g.sample_aspect_ratio());
        tracker.seed_state(raw_seed);
        let extra = extra.to_vec();
        let extra_without_pasp = extra
            .iter()
            .filter(|x| x.box_type != PASP_FOURCC)
            .cloned()
            .collect();
        Some(Self {
            tracker,
            config: config.clone(),
            seed_sar,
            extra,
            extra_without_pasp,
            cached_raw: None,
            cached_overlaid: None,
        })
    }

    /// Observe one access unit and set its display tag.
    pub(crate) fn tag(&mut self, sample: &mut Sample) {
        // Convert length-prefixed -> Annex B for observation only; sample bytes
        // stay unchanged. A malformed length prefix must not panic and leaves
        // the previous tag in place.
        let length_size = nal_length_size(&self.config).unwrap_or(crate::annexb::NAL_LENGTH_SIZE);
        if let Ok(nals) = crate::annexb::iter_length_prefixed_nals_with(&sample.data, length_size) {
            let mut annexb = Vec::with_capacity(sample.data.len());
            for nal in nals {
                annexb.extend_from_slice(&[0, 0, 0, 1]);
                annexb.extend_from_slice(nal);
            }
            let _ = self.tracker.observe(&annexb);
        }
        let raw_state = self.tracker.state().cloned();
        let display_arc = match raw_state {
            None => None,
            Some(raw) => {
                let overlaid = if let (Some(prev_raw), Some(prev_ov)) =
                    (&self.cached_raw, &self.cached_overlaid)
                    && Arc::ptr_eq(prev_raw, &raw)
                {
                    prev_ov.clone()
                } else {
                    let pasp_in_force = raw.geometry.as_ref().and_then(|g| g.sample_aspect_ratio())
                        == self.seed_sar;
                    let boxes = if pasp_in_force {
                        &self.extra
                    } else {
                        &self.extra_without_pasp
                    };
                    let ov = overlay_display_info(&raw, &self.config, boxes);
                    self.cached_raw = Some(raw.clone());
                    self.cached_overlaid = Some(ov.clone());
                    ov
                };
                Some(overlaid)
            }
        };
        sample.display = display_arc;
    }
}

/// Tag every sample of an in-band track, in one pass (the whole-input demuxers).
fn overlay_display_info(
    raw: &Arc<DisplayInfo>,
    config: &CodecConfig,
    extra: &[isobmff::sample_entries::OpaqueBox],
) -> Arc<DisplayInfo> {
    let mut info = (**raw).clone();
    info.geometry = overlay_sample_entry_geometry(info.geometry, config, extra);
    Arc::new(info)
}

fn gcd_u32(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a.max(1)
}

/// Build a [`SkippedTrack`] record for a `trak` [`track_spec_from_trak`]
/// rejected (media plane step-2 fix wave 1, B2/B3).
///
/// Prefers the [`Error::UnsupportedSampleEntry`] FourCC — the common
/// real-world case: an unrecognised `stsd` entry (a QuickTime hint/chapter
/// track, `c608`/`c708`, GoPro `gpmd`, ...) is the *only* way
/// `track_spec_from_trak` can fail once it has reached a `stsd` entry at all
/// ([`codec_config_from_entry`] only errors on
/// [`SampleEntryVariant::Unknown`](isobmff::init_segment::SampleEntryVariant::Unknown)).
/// Any other error means the `trak` was too structurally malformed to even
/// reach an entry (missing `mdia`/`mdhd`/`minf`/`stbl`/`stsd`), so there is no
/// FourCC to recover; `"unknown"` names that case honestly rather than
/// guessing.
/// The real sampling rate of an audio sample entry.
///
/// Prefers a `SamplingRateBox` (`srat`, ISO/IEC 14496-12:2026 §12.2.3.1) in the
/// entry's config children over the entry's 16.16 fixed-point `samplerate`
/// field: the box is present precisely when the rate does not fit that field
/// (rates above 65535 Hz — 96 kHz, 192 kHz, …), and it "overrides the
/// samplerate field and documents the actual sampling rate". Without it,
/// `192000 << 16` reads back as 60928 Hz.
fn entry_sampling_rate(samplerate_16_16: u32, config_boxes: &[OpaqueBox]) -> u32 {
    sampling_rate_override(config_boxes).unwrap_or(samplerate_16_16 >> u16::BITS)
}

/// Reconstruct a [`CodecConfig`] from an `stsd` sample entry.
///
/// Every codec the crate can output reconstructs losslessly by re-parsing the
/// config record out of the sample entry: video codecs carry a typed config
/// box on the sample entry (`avcC`/`hvcC`/`av1C`/`vpcC`); audio codecs carry
/// the config box as an [`OpaqueBox`] body (`esds`/`dac3`/`dec3`/`dOps`/
/// `dfLa`/`ddts`/`dac4`/`mhaC`) which is re-parsed here; `stpp`/`wvtt`
/// subtitle entries carry no per-track config this crate needs to decode —
/// only their format tag — and become [`CodecConfig::Subtitle`] (media plane
/// step 2d; samples stay opaque TTML/WebVTT payloads, never parsed).
///
/// Only `SampleEntryVariant::Unknown` — a sample entry naming a codec this
/// crate genuinely does not implement — yields
/// [`Error::UnsupportedSampleEntry`], propagated by
/// [`crate::mp4_demux::Mp4Demux`] rather than
/// silently skipping the track (media plane step 2d: codec coverage lands
/// *before* this strictness, so `stpp`/`wvtt`/`ac-4` — which used to hit this
/// path — no longer do) (ISO/IEC 14496-12:2026 §8.5.2 sample entries; -15
/// §5.4/§8.4 for AVC/HEVC; ISO/IEC 23008-3 §20 for MPEG-H `mha*`; ISO/IEC
/// 14496-30 §7.2/§9.2 for `stpp`/`wvtt`; ETSI TS 103 190-2 Annex E for `ac-4`).
fn codec_config_from_entry(entry: &SampleEntryVariant) -> Result<CodecConfig> {
    match entry {
        SampleEntryVariant::Avc1(avc) => Ok(CodecConfig::Avc {
            config: avc.config.clone(),
            width: avc.visual.width,
            height: avc.visual.height,
        }),
        SampleEntryVariant::Hevc1(hevc) => Ok(CodecConfig::Hevc {
            config: hevc.config.clone(),
            width: hevc.visual.width,
            height: hevc.visual.height,
        }),
        SampleEntryVariant::Vvc(vvc) => {
            // Prefer the SPS-decoded dimensions from the vvcC NAL array (the
            // authoritative coded geometry); fall back to the sample-entry
            // visual dims when the SPS is absent, cannot be decoded, or
            // reports a coded size that does not fit the IR's `u16` field (the
            // SPS dimensions are `ue(v)`, so an oversized one must not truncate).
            let (width, height) = vvc
                .config
                .config
                .dimensions()
                .and_then(|(w, h)| Some((u16::try_from(w).ok()?, u16::try_from(h).ok()?)))
                .unwrap_or((vvc.visual.width, vvc.visual.height));
            Ok(CodecConfig::Vvc {
                config: vvc.config.clone(),
                width,
                height,
            })
        }
        SampleEntryVariant::Av01(av1) => Ok(CodecConfig::Av1 {
            config: av1.config.clone(),
            width: av1.visual.width,
            height: av1.visual.height,
        }),
        SampleEntryVariant::Vp09(vp9) => Ok(CodecConfig::Vp9 {
            config: vp9.config.clone(),
            width: vp9.visual.width,
            height: vp9.visual.height,
        }),
        SampleEntryVariant::Mp4v(mp4v) => {
            // The esds is the mp4v's first child box body (no 8-byte header).
            let esds = EsdsBox::parse_body(config_box_body(&mp4v.config_boxes, b"esds")?)?;
            Ok(CodecConfig::Mpeg2Video {
                esds,
                // Provisional geometry from the visual sample entry; refined
                // from the in-band sequence_header() once samples are collected.
                width: mp4v.visual.width,
                height: mp4v.visual.height,
            })
        }
        SampleEntryVariant::Mp4a(mp4a) => {
            let esds = esds_from_config_boxes(&mp4a.config_boxes)?;
            let oti = esds
                .es_descriptor
                .decoder_config
                .as_ref()
                .map(|dc| dc.object_type_indication.0);
            // OTI 0x69 (MPEG-2 audio) / 0x6B (MPEG-1 audio) → legacy MPEG audio;
            // otherwise the mp4a carries AAC (ISO/IEC 14496-1 §7.2.6.6 Table 5).
            if oti == Some(OTI_MPEG2_AUDIO) || oti == Some(OTI_MPEG1_AUDIO) {
                Ok(CodecConfig::MpegAudio {
                    esds,
                    // Provisional layer; refined from the first frame header.
                    layer: MpegAudioLayer::LayerII,
                    channel_count: mp4a.channelcount,
                    sample_rate: entry_sampling_rate(mp4a.samplerate, &mp4a.config_boxes),
                    sample_size: mp4a.samplesize,
                })
            } else {
                Ok(CodecConfig::Aac {
                    esds,
                    channel_count: mp4a.channelcount,
                    sample_rate: entry_sampling_rate(mp4a.samplerate, &mp4a.config_boxes),
                    sample_size: mp4a.samplesize,
                })
            }
        }
        SampleEntryVariant::Ac3(ac3) => {
            let config = Ac3SpecificBox::parse(config_box_body(&ac3.config_boxes, b"dac3")?)?;
            Ok(CodecConfig::Ac3 {
                config,
                channel_count: ac3.channelcount,
                sample_rate: entry_sampling_rate(ac3.samplerate, &ac3.config_boxes),
                sample_size: ac3.samplesize,
            })
        }
        SampleEntryVariant::Ec3(ec3) => {
            let config = Ec3SpecificBox::parse(config_box_body(&ec3.config_boxes, b"dec3")?)?;
            Ok(CodecConfig::Eac3 {
                config,
                channel_count: ec3.channelcount,
                sample_rate: entry_sampling_rate(ec3.samplerate, &ec3.config_boxes),
                sample_size: ec3.samplesize,
            })
        }
        SampleEntryVariant::Opus(opus) => {
            let config = OpusSpecificBox::parse(config_box_body(&opus.config_boxes, b"dOps")?)?;
            Ok(CodecConfig::Opus {
                config,
                channel_count: opus.channelcount,
                sample_rate: entry_sampling_rate(opus.samplerate, &opus.config_boxes),
                sample_size: opus.samplesize,
            })
        }
        SampleEntryVariant::Flac(flac) => {
            let config = FlacSpecificBox::parse(config_box_body(&flac.config_boxes, b"dfLa")?)?;
            Ok(CodecConfig::Flac {
                config,
                channel_count: flac.channelcount,
                sample_rate: entry_sampling_rate(flac.samplerate, &flac.config_boxes),
                sample_size: flac.samplesize,
            })
        }
        SampleEntryVariant::Dts(dts) => {
            let config = DtsSpecificBox::parse(config_box_body(&dts.config_boxes, b"ddts")?)?;
            Ok(CodecConfig::Dts {
                config,
                codec_fourcc: dts.codec_type,
                channel_count: dts.channelcount,
                sample_rate: entry_sampling_rate(dts.samplerate, &dts.config_boxes),
                sample_size: dts.samplesize,
            })
        }
        SampleEntryVariant::Mha(mha) => {
            // The mhaC record is carried as an OpaqueBox body (no 8-byte header)
            // in the MPEG-H sample entry — re-parse it verbatim (ISO/IEC
            // 23008-3 §20). channel_count / sample_rate come from the
            // AudioSampleEntry fixed fields (the reference channel layout lives
            // in the record's `reference_channel_layout`).
            let config = MHADecoderConfigurationRecord::parse(config_box_body(
                &mha.config_boxes,
                &MHAC_FOURCC,
            )?)?;
            Ok(CodecConfig::MpegH {
                config,
                channel_count: mha.channelcount,
                sample_rate: entry_sampling_rate(mha.samplerate, &mha.config_boxes),
                sample_size: mha.samplesize,
            })
        }
        SampleEntryVariant::Ac4(ac4) => {
            let config = Ac4SpecificBox::parse(config_box_body(
                &ac4.config_boxes,
                &isobmff::ac4::DAC4_FOURCC,
            )?)?;
            Ok(CodecConfig::Ac4 {
                config,
                channel_count: ac4.channelcount,
                sample_rate: entry_sampling_rate(ac4.samplerate, &ac4.config_boxes),
                sample_size: ac4.samplesize,
            })
        }
        // Subtitle sample entries (media plane step 2d): the whole entry (TTML
        // namespace / WebVTT `vttC` header block / trailing boxes) is carried
        // in `CodecConfig::Subtitle::sample_entry` so it re-muxes (#1186).
        // Samples stay opaque (TTML XML / WebVTT `vttc`/`vtte` boxes), never parsed.
        SampleEntryVariant::Stpp(s) => Ok(CodecConfig::Subtitle {
            format: SubtitleFormat::Ttml,
            sample_entry: Some(SubtitleSampleEntry::Ttml((**s).clone())),
        }),
        SampleEntryVariant::Wvtt(w) => Ok(CodecConfig::Subtitle {
            format: SubtitleFormat::WebVtt,
            sample_entry: Some(SubtitleSampleEntry::WebVtt((**w).clone())),
        }),
        // Genuinely unimplemented sample entry: name it so the caller learns
        // which track was rejected, rather than a generic message.
        SampleEntryVariant::Unknown(entry) => Err(Error::UnsupportedSampleEntry {
            fourcc: String::from_utf8_lossy(&entry.box_type).into_owned(),
        }),
        SampleEntryVariant::AvcParameterSet(entry) => Err(Error::UnsupportedSampleEntry {
            fourcc: String::from_utf8_lossy(&entry.entry.codec_type).into_owned(),
        }),
        // A variant added to this non-exhaustive enum above this crate.
        _ => Err(Error::UnsupportedSampleEntry {
            fourcc: String::from("unknown"),
        }),
    }
}

/// Refine the provisional geometry/layer of a legacy-codec config from a
/// coded sample's in-band header (the sample entry alone cannot carry it):
/// MPEG-2 video takes width/height from the `sequence_header()`, MPEG audio
/// takes its layer from the first frame header. A no-op for other codecs and
/// when the header cannot be decoded (the provisional values then stand).
///
/// Only the **first** sample of a track is consulted; pass exactly that one.
///
/// `pub(crate)`: used by [`crate::mp4_demux::Mp4Demux`].
pub(crate) fn refine_legacy_config(config: &mut CodecConfig, first: &Sample) {
    match config {
        CodecConfig::Mpeg2Video { width, height, .. }
            if let Ok(sh) = Mpeg2SeqHeader::find(&first.data) =>
        {
            *width = sh.width;
            *height = sh.height;
        }
        CodecConfig::MpegAudio { layer, .. }
            if let Ok(hdr) = MpegAudioFrameHeader::parse(&first.data) =>
        {
            *layer = hdr.layer;
        }
        _ => {}
    }
}

/// The NAL length-prefix size (bytes) an AVC/HEVC config declares, or `None`
/// for a codec that is not length-prefixed. ISO/IEC 14496-15:2017 §4.3.3.1, §5.3.3.1.3:
/// `lengthSizeMinusOne + 1`.
pub(crate) fn nal_length_size(config: &CodecConfig) -> Option<usize> {
    match config {
        CodecConfig::Avc { config, .. } => {
            Some(usize::from(config.config.length_size_minus_one) + 1)
        }
        CodecConfig::Hevc { config, .. } => {
            Some(usize::from(config.config.length_size_minus_one) + 1)
        }
        _ => None,
    }
}

/// Rewrite one sample's NAL prefixes from `length_size` bytes to the crate's
/// canonical 4, when the source declared a different size
/// (ISO/IEC 14496-15:2017 §4.3.3.1, §5.3.3.1.3: 1, 2 or 4 bytes).
///
/// The rest of the pipeline — [`crate::annexb::iter_length_prefixed_nals`],
/// keyframe detection, the TS/RTP writers — is fixed at 4 bytes, so a 1- or
/// 2-byte-length fMP4 would otherwise parse every sample as garbage (r04-W43).
/// A `length_size` of 3 is reserved by §5.3.3, so such a track is rejected
/// rather than guessed at. Callers take `length_size` from
/// [`nal_length_size`] **before** [`normalise_spec_nal_length`] rewrites the
/// config to canonical 4.
pub(crate) fn normalise_nal_lengths(length_size: usize, sample: &mut Sample) -> Result<()> {
    if length_size == crate::annexb::NAL_LENGTH_SIZE {
        // Already canonical. Samples stay opaque here (this demuxer never
        // walks coded data for the common 4-byte case), so a container whose
        // sample bytes are not NAL-framed still demuxes as before.
        return Ok(());
    }
    let data = crate::annexb::normalise_nal_length_size(&sample.data, length_size)?;
    sample.data = data.into();
    Ok(())
}

/// Set an AVC/HEVC config's `lengthSizeMinusOne` to the crate's canonical 4-byte
/// prefix, after every sample has been rewritten by [`normalise_nal_lengths`].
pub(crate) fn normalise_spec_nal_length(spec: &mut TrackSpec) {
    let Some(length_size) = nal_length_size(&spec.config) else {
        return;
    };
    if length_size == crate::annexb::NAL_LENGTH_SIZE {
        return;
    }
    match &mut spec.config {
        CodecConfig::Avc { config, .. } => {
            config.config.length_size_minus_one = crate::annexb::NAL_LENGTH_SIZE_MINUS_ONE;
        }
        CodecConfig::Hevc { config, .. } => {
            config.config.length_size_minus_one = crate::annexb::NAL_LENGTH_SIZE_MINUS_ONE;
        }
        _ => {}
    }
}

/// Re-parse the `esds` box body preserved as an [`OpaqueBox`] in an `mp4a`
/// sample entry's `config_boxes` into an owned [`EsdsBox`].
fn esds_from_config_boxes(boxes: &[OpaqueBox]) -> Result<EsdsBox> {
    // `OpaqueBox.data` is the FullBox *body* (no size/type header); parse it
    // directly (EsdsBox is fully owned, so no lifetime is retained).
    EsdsBox::parse_body(config_box_body(boxes, b"esds")?).map_err(Into::into)
}

/// Locate a config box by FourCC among a sample entry's `config_boxes` and
/// return its body bytes (the [`OpaqueBox`] holds the box body, no 8-byte
/// header), so a codec-specific `Parse` can re-parse the decoder config record.
fn config_box_body<'b>(boxes: &'b [OpaqueBox], fourcc: &[u8; BOX_TYPE_SIZE]) -> Result<&'b [u8]> {
    boxes
        .iter()
        .find(|b| &b.box_type == fourcc)
        .map(|b| b.data.as_slice())
        .ok_or(Error::UnexpectedBox {
            expected: "config box in audio sample entry",
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TrackId;
    use crate::ir::DataIdentity;
    use crate::pipeline::{FragmentTrackData, build_media_segment};
    use isobmff::subtitle_entries::WvttSampleEntry;

    /// ffmpeg's `mov` muxer cannot produce a `wvtt` sample entry (only
    /// `stpp`/TTML — see `../fixtures/mp4/cmaf-av-subtitle/PROVENANCE.md`), so WebVTT
    /// demux coverage is exercised directly here: synthesise a real `wvtt`
    /// sample entry with the crate's own builder (already covered by
    /// `subtitle_entries::tests::wvtt_sample_entry_round_trip`) and feed it
    /// through the same reconstruction path the `stpp` fixture exercises.
    /// A summary log for a whole [`Media`] as one segment per track.
    fn one_segment_log(media: &Media) -> crate::segment_log::SegmentSummaryLog {
        let mut log = crate::segment_log::SegmentSummaryLog::new();
        for track in &media.tracks {
            if track.samples.is_empty() {
                continue;
            }
            let duration: u64 = track
                .samples
                .iter()
                .map(|s| u64::from(s.timing.and_then(|t| t.duration()).unwrap_or(0)))
                .sum();
            let bytes: u64 = track.samples.iter().map(|s| s.data.len() as u64).sum();
            log.push_part(
                &crate::mux_stage::TrackPart {
                    track: track.spec.track_id,
                    start: 0,
                    duration,
                    timescale: track.spec.timescale.get(),
                    bytes,
                    samples: u32::try_from(track.samples.len()).unwrap_or(u32::MAX),
                },
                track.samples.first().is_some_and(|s| s.flags.is_sync),
            )
            .expect("one part per track");
        }
        log
    }

    #[test]
    fn wvtt_sample_entry_demuxes_to_subtitle_webvtt() {
        let wvtt = WvttSampleEntry::new("WEBVTT\n");
        let entry = SampleEntryVariant::Wvtt(alloc::boxed::Box::new(wvtt));
        let config = codec_config_from_entry(&entry).expect("wvtt must now demux");
        assert!(
            matches!(
                config,
                CodecConfig::Subtitle {
                    format: SubtitleFormat::WebVtt,
                    sample_entry: Some(SubtitleSampleEntry::WebVtt(ref e)),
                } if e.config.config == "WEBVTT\n"
            ),
            "wvtt must map to CodecConfig::Subtitle{{format: SubtitleFormat::WebVtt}}, got {config:?}"
        );
    }

    /// A genuinely unrecognised sample entry must be named in the error
    /// (media plane step 2d Phase 2), not just a generic "unsupported"
    /// message.
    #[test]
    fn unknown_sample_entry_errors_naming_the_fourcc() {
        let unknown = OpaqueBox::new(*b"xyz9", vec![1, 2, 3]);
        let entry = SampleEntryVariant::Unknown(unknown);
        let err = codec_config_from_entry(&entry).unwrap_err();
        match err {
            Error::UnsupportedSampleEntry { fourcc } => assert_eq!(fourcc, "xyz9"),
            other => panic!("expected UnsupportedSampleEntry, got {other:?}"),
        }
    }

    // ── Gapped / discontinuous fMP4 (per-fragment `tfdt` re-seed) ───────────

    /// A minimal but real `avcC` — the same Baseline SPS/PPS pair
    /// `avc_config::tests::make_minimal_avcc_body` builds, expressed
    /// structurally so this test needs no fixture file.
    fn minimal_avc_config() -> isobmff::avc_config::AVCConfigurationBox {
        use isobmff::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
        use isobmff::nalu_types::{AvcPps, AvcSps};
        AVCConfigurationBox {
            config: AVCDecoderConfigurationRecord {
                configuration_version: 1,
                profile_indication: 66,
                profile_compatibility: 0,
                level_indication: 0x1E,
                length_size_minus_one: 3,
                sps: vec![AvcSps(vec![0x67, 0x42, 0x00, 0x1E, 0xAB, 0x40])],
                pps: vec![AvcPps(vec![0x68, 0xCE, 0x3C, 0x80])],
                chroma_format: None,
                bit_depth_luma_minus8: None,
                bit_depth_chroma_minus8: None,
                sps_ext: Vec::new(),
            },
        }
    }

    /// PROVENANCE: synthesised, not a captured file. No committed fixture in
    /// this workspace carries a *gapped* fMP4 (every CMAF capture here is
    /// contiguous), so the two fragments are built with this crate's own
    /// [`build_init_segment`] / [`build_media_segment`] — i.e. the exact
    /// `moof`/`tfdt`/`trun` layout the muxer emits (ISO/IEC 14496-12:2026 §8.8),
    /// not hand-rolled bytes — with the second fragment's
    /// `base_media_decode_time` deliberately advanced past the running sum of
    /// the first fragment's `trun` durations.
    ///
    /// Seeding `next_dts` from only the FIRST `tfdt` and then pure-summing
    /// durations makes every post-gap sample's absolute `dts` short by exactly
    /// the gap, which re-muxes to a wrong `tfdt`/PES stamp and permanently
    /// desyncs A/V. `tfdt` is per-fragment and authoritative (§8.8.12).
    #[test]
    fn gapped_fmp4_reseeds_absolute_dts_from_each_fragments_own_tfdt() {
        const TIMESCALE: Timescale = broadcast_common::timescale!(90_000);
        const FRAME_DUR: u32 = 3_000;
        const FRAMES_PER_FRAGMENT: u32 = 4;
        /// Decode time the second fragment declares — one whole extra frame
        /// beyond where the first fragment's durations leave off.
        const GAP_TICKS: u64 = FRAME_DUR as u64;

        let spec = TrackSpec::new(
            TrackId::new(1),
            TIMESCALE,
            CodecConfig::Avc {
                config: minimal_avc_config(),
                width: 16,
                height: 16,
            },
        );
        let init = build_init_segment(core::slice::from_ref(&spec), TIMESCALE)
            .expect("init segment builds");

        let frames = |base: i64| -> Vec<Sample> {
            (0..FRAMES_PER_FRAGMENT)
                .map(|i| {
                    // One length-prefixed non-IDR NAL; content is irrelevant
                    // here (this test is about timing, not codec bytes).
                    let nal = [0x41u8, 0xAA, 0xBB];
                    let mut data = (nal.len() as u32).to_be_bytes().to_vec();
                    data.extend_from_slice(&nal);
                    let dts = base + i64::from(i) * i64::from(FRAME_DUR);
                    Sample::new(data, Some(dts), Some(dts), Some(FRAME_DUR), i == 0)
                })
                .collect::<crate::error::Result<Vec<_>>>()
                .expect("valid timing")
        };

        let first_base = 0u64;
        let contiguous_second_base = u64::from(FRAMES_PER_FRAGMENT * FRAME_DUR);
        let gapped_second_base = contiguous_second_base + GAP_TICKS;

        let f1 = frames(first_base as i64);
        let f2 = frames(gapped_second_base as i64);
        let seg1 = build_media_segment(
            1,
            &[FragmentTrackData::new(TrackId::new(1), first_base, &f1)],
        )
        .expect("fragment 1 builds");
        let seg2 = build_media_segment(
            2,
            &[FragmentTrackData::new(
                TrackId::new(1),
                gapped_second_base,
                &f2,
            )],
        )
        .expect("fragment 2 builds");

        let mut file = init;
        file.extend_from_slice(&seg1);
        file.extend_from_slice(&seg2);

        let media = crate::mp4_collect::media_from_mp4(&file).expect("demux gapped fMP4");
        let track = &media.tracks[0];
        assert_eq!(
            track.samples.len(),
            (FRAMES_PER_FRAGMENT * 2) as usize,
            "both fragments' samples must be present"
        );
        assert_eq!(
            track.start_decode_time, first_base,
            "start_decode_time stays the FIRST fragment's anchor, not the last"
        );

        // The bite: the first post-gap sample must land on the second
        // fragment's own declared tfdt, NOT on the running duration sum.
        let first_post_gap = &track.samples[FRAMES_PER_FRAGMENT as usize];
        assert_eq!(
            first_post_gap.timing.and_then(|t| t.dts()),
            Some(gapped_second_base as i64),
            "post-gap dts must come from the second fragment's tfdt \
             ({gapped_second_base}), not the pure duration sum \
             ({contiguous_second_base})"
        );

        // ...and so must every later sample in that fragment.
        for (i, s) in track.samples[FRAMES_PER_FRAGMENT as usize..]
            .iter()
            .enumerate()
        {
            let expected = gapped_second_base as i64 + (i as i64) * i64::from(FRAME_DUR);
            assert_eq!(
                s.timing.and_then(|t| t.dts()),
                Some(expected),
                "sample {i} after the gap"
            );
        }
    }

    /// Regression guard for the fix above: a **contiguous** two-fragment
    /// stream (every `tfdt` exactly equal to the running duration sum) must
    /// demux to exactly the same absolute dts sequence it always did —
    /// re-seeding per fragment must be a no-op when there is no gap.
    #[test]
    fn contiguous_fmp4_dts_is_unchanged_by_the_per_fragment_tfdt_reseed() {
        const TIMESCALE: Timescale = broadcast_common::timescale!(90_000);
        const FRAME_DUR: u32 = 3_000;
        const FRAMES: u32 = 3;

        let spec = TrackSpec::new(
            TrackId::new(7),
            TIMESCALE,
            CodecConfig::Avc {
                config: minimal_avc_config(),
                width: 16,
                height: 16,
            },
        );
        let init = build_init_segment(core::slice::from_ref(&spec), TIMESCALE).unwrap();
        let frames = |base: i64| -> Vec<Sample> {
            (0..FRAMES)
                .map(|i| {
                    let nal = [0x41u8, 0xAA, 0xBB];
                    let mut data = (nal.len() as u32).to_be_bytes().to_vec();
                    data.extend_from_slice(&nal);
                    let dts = base + i64::from(i) * i64::from(FRAME_DUR);
                    Sample::new(data, Some(dts), Some(dts), Some(FRAME_DUR), i == 0)
                })
                .collect::<crate::error::Result<Vec<_>>>()
                .expect("valid timing")
        };
        let second_base = u64::from(FRAMES * FRAME_DUR);
        let f1 = frames(0);
        let f2 = frames(second_base as i64);
        let mut file = init;
        file.extend_from_slice(
            &build_media_segment(1, &[FragmentTrackData::new(TrackId::new(7), 0, &f1)]).unwrap(),
        );
        file.extend_from_slice(
            &build_media_segment(
                2,
                &[FragmentTrackData::new(TrackId::new(7), second_base, &f2)],
            )
            .unwrap(),
        );

        let media = crate::mp4_collect::media_from_mp4(&file).unwrap();
        let got: Vec<i64> = media.tracks[0]
            .samples
            .iter()
            .filter_map(|s| s.timing.and_then(|t| t.dts()))
            .collect();
        let want: Vec<i64> = (0..FRAMES * 2)
            .map(|i| i64::from(i) * i64::from(FRAME_DUR))
            .collect();
        assert_eq!(got, want, "a gapless stream must be unaffected by the fix");
    }

    // ── HlsPackager: no knowingly-wrong #EXTINF:0.000 ──────────────────────

    /// A timestamp-less (section-carried) track has `duration: None` on every
    /// sample and therefore no truthful `EXTINF` — it is omitted from the
    /// playlist rather than rendered as `#EXTINF:0.000`, which RFC 8216
    /// §4.3.2.1 defines as a real playback duration a player would honour.
    #[test]
    fn hls_packager_omits_a_timestampless_track_instead_of_emitting_extinf_zero() {
        const TIMESCALE: Timescale = broadcast_common::timescale!(90_000);
        const FRAME_DUR: u32 = 3_000;

        let timed = Track::new(
            TrackSpec::new(
                TrackId::new(1),
                TIMESCALE,
                CodecConfig::Avc {
                    config: minimal_avc_config(),
                    width: 16,
                    height: 16,
                },
            ),
            (0..3)
                .map(|i| {
                    Sample::new(
                        vec![0u8; 4],
                        Some(i64::from(i) * i64::from(FRAME_DUR)),
                        Some(i64::from(i) * i64::from(FRAME_DUR)),
                        Some(FRAME_DUR),
                        true,
                    )
                })
                .collect::<crate::error::Result<Vec<_>>>()
                .expect("valid timing"),
        );
        // A section-carried data track: no dts, no pts, no duration — ever.
        let sections = Track::new(
            TrackSpec::new(
                TrackId::new(2),
                TIMESCALE,
                CodecConfig::Data {
                    identity: DataIdentity::Ts {
                        stream_type: 0x86,
                        descriptors: Vec::new(),
                        carriage: crate::ir::DataCarriage::Sections,
                    },
                },
            ),
            vec![Sample::new(vec![0xFCu8; 8], None, None, None, true).expect("valid timing")],
        );

        let media = Media::new(vec![timed, sections], TIMESCALE);
        let specs: Vec<TrackSpec> = media.tracks.iter().map(|t| t.spec.clone()).collect();
        let playlist = HlsPackager::default()
            .render(&specs, &one_segment_log(&media))
            .expect("a Media with one timed track still packages");
        let zero_extinf = playlist.lines().any(|l| {
            l.strip_prefix("#EXTINF:")
                .and_then(|rest| rest.trim_end_matches(',').parse::<f64>().ok())
                .is_some_and(|secs| secs == 0.0)
        });
        assert!(
            !zero_extinf,
            "no zero-duration EXTINF may be emitted, got:\n{playlist}"
        );
        assert!(
            !playlist.contains("seg2.m4s"),
            "the timestamp-less track must be omitted entirely, got:\n{playlist}"
        );
        assert!(
            playlist.contains("seg1.m4s"),
            "the timed track must still be present, got:\n{playlist}"
        );

        // Every track timestamp-less => nothing truthful to render at all.
        let only_sections = Track::new(
            TrackSpec::new(
                TrackId::new(2),
                TIMESCALE,
                CodecConfig::Data {
                    identity: DataIdentity::Ts {
                        stream_type: 0x86,
                        descriptors: Vec::new(),
                        carriage: crate::ir::DataCarriage::Sections,
                    },
                },
            ),
            vec![Sample::new(vec![0xFCu8; 8], None, None, None, true).expect("valid timing")],
        );
        let only = Media::new(vec![only_sections], TIMESCALE);
        let only_specs: Vec<TrackSpec> = only.tracks.iter().map(|t| t.spec.clone()).collect();
        assert!(
            HlsPackager::default()
                .render(&only_specs, &one_segment_log(&only))
                .is_err(),
            "an all-timestamp-less Media must error, not render an empty playlist"
        );
    }

    /// §4.3.3.1: `#EXT-X-TARGETDURATION` must be at least the rounded
    /// `#EXTINF`, which the ceiling always satisfies — and a whole number of
    /// seconds is its own ceiling, not one second more.
    #[test]
    fn ceil_secs_rounds_up_only_a_fractional_span() {
        assert_eq!(ceil_secs(0.0), 0);
        assert_eq!(ceil_secs(2.0), 2);
        assert_eq!(ceil_secs(2.0001), 3);
    }

    /// r04-W43: an fMP4 whose `avcC` declares 2-byte NAL lengths must have its
    /// samples normalised to the crate's 4-byte form, and the config the
    /// demuxer reports must be updated to match — the rest of the pipeline
    /// (`iter_length_prefixed_nals`, the TS/RTP writers) is fixed at 4 bytes.
    ///
    /// The file is a genuine one: an init segment built from a config that
    /// really declares `lengthSizeMinusOne = 1`, concatenated with a media
    /// segment whose samples really carry 2-byte prefixes.
    #[test]
    fn fmp4_demux_normalises_two_byte_nal_lengths() {
        use crate::annexb::NAL_LENGTH_SIZE_MINUS_ONE;
        const TIMESCALE: Timescale = broadcast_common::timescale!(90_000);
        use crate::ir::{FragmentTrackData, Sample, SampleFlags};
        use crate::pipeline::{CodecConfig, TrackSpec, build_init_segment, build_media_segment};

        // The same Baseline SPS/PPS pair, but declaring 2-byte NAL lengths.
        let mut avcc = minimal_avc_config();
        avcc.config.length_size_minus_one = 1;
        let spec = TrackSpec::new(
            TrackId::new(1),
            TIMESCALE,
            CodecConfig::Avc {
                config: avcc,
                width: 640,
                height: 360,
            },
        );
        // One access unit, two NALs, each with a 2-byte length prefix.
        let sample = Sample {
            data: vec![0x00, 0x05, 0x65, 0x88, 0x84, 0x00, 0x21, 0x00, 0x01, 0x41].into(),
            timing: Some(crate::ir::SampleTiming::timed_with_duration(0, 0, 1000)),
            flags: SampleFlags::new(true),
            provenance: None,
            display: None,
            protection: None,
        };
        let init = build_init_segment(std::slice::from_ref(&spec), TIMESCALE).expect("init");
        // The init segment really declares the 2-byte size.
        let hdr = init
            .windows(4)
            .position(|w| w == b"avcC")
            .expect("avcC present");
        // avcC body: configurationVersion(1) profile(1) compat(1) level(1)
        // reserved(6)+lengthSizeMinusOne(2).
        assert_eq!(
            init[hdr + 8] & 0x03,
            1,
            "the built init segment must declare lengthSizeMinusOne = 1"
        );
        let frag = build_media_segment(
            1,
            &[FragmentTrackData::new(
                TrackId::new(1),
                0,
                std::slice::from_ref(&sample),
            )],
        )
        .expect("media segment");
        let mut file = init;
        file.extend_from_slice(&frag);

        let media = crate::mp4_collect::media_from_mp4(&file).expect("demux the 2-byte fMP4");
        let track = &media.tracks[0];
        assert_eq!(
            track.samples[0].data.as_ref(),
            [
                0x00, 0x00, 0x00, 0x05, 0x65, 0x88, 0x84, 0x00, 0x21, 0x00, 0x00, 0x00, 0x01, 0x41,
            ],
            "2-byte NAL lengths must be rewritten to 4-byte"
        );
        let CodecConfig::Avc { config, .. } = &track.spec.config else {
            panic!("expected CodecConfig::Avc");
        };
        assert_eq!(
            config.config.length_size_minus_one, NAL_LENGTH_SIZE_MINUS_ONE,
            "the reported avcC must declare the 4-byte length size"
        );
    }

    /// The HEVC twin of the test above: an `hvcC` declaring
    /// `lengthSizeMinusOne = 1` (2-byte NAL lengths, ISO/IEC 14496-15:2017
    /// §8.3.3.1.2) must have its samples rewritten to the crate's 4-byte form
    /// and its record updated to say so.
    #[test]
    fn fmp4_demux_normalises_two_byte_nal_lengths_hevc() {
        use crate::annexb::NAL_LENGTH_SIZE_MINUS_ONE;
        use crate::ir::{FragmentTrackData, Sample, SampleFlags};
        use crate::pipeline::{CodecConfig, TrackSpec, build_init_segment, build_media_segment};
        use isobmff::hevc_config::HEVCDecoderConfigurationRecord;
        const TIMESCALE: Timescale = broadcast_common::timescale!(90_000);

        let record = HEVCDecoderConfigurationRecord {
            configuration_version: 1,
            general_profile_space: 0,
            general_tier_flag: false,
            general_profile_idc: 1,
            general_profile_compatibility_flags: 0x6000_0000,
            general_constraint_indicator_flags: 0,
            general_level_idc: 30,
            min_spatial_segmentation_idc: 0,
            parallelism_type: 0,
            chroma_format_idc: 1,
            bit_depth_luma_minus8: 0,
            bit_depth_chroma_minus8: 0,
            avg_frame_rate: 0,
            constant_frame_rate: 0,
            num_temporal_layers: 1,
            temporal_id_nested: true,
            length_size_minus_one: 1,
            arrays: Vec::new(),
        };
        let spec = TrackSpec::new(
            TrackId::new(1),
            TIMESCALE,
            CodecConfig::Hevc {
                config: isobmff::hevc_config::HEVCConfigurationBox::new(record),
                width: 320,
                height: 240,
            },
        );
        // One access unit, two NALs, each with a 2-byte length prefix.
        let sample = Sample {
            data: vec![
                0x00, 0x05, 0x26, 0x01, 0xAF, 0x09, 0x40, 0x00, 0x02, 0x44, 0x01,
            ]
            .into(),
            timing: Some(crate::ir::SampleTiming::timed_with_duration(0, 0, 1000)),
            flags: SampleFlags::new(true),
            provenance: None,
            display: None,
            protection: None,
        };
        let init = build_init_segment(std::slice::from_ref(&spec), TIMESCALE).expect("init");
        let hdr = init
            .windows(4)
            .position(|w| w == b"hvcC")
            .expect("hvcC present");
        // hvcC body: configurationVersion/profile/compat/constraints/level,
        // min_spatial_segmentation_idc, parallelism/chroma, bit depths,
        // avgFrameRate, then the byte holding lengthSizeMinusOne in its low
        // two bits — 21 bytes into the body.
        assert_eq!(
            init[hdr + 4 + 21] & 0x03,
            1,
            "the built init segment must declare lengthSizeMinusOne = 1"
        );
        let frag = build_media_segment(
            1,
            &[FragmentTrackData::new(
                TrackId::new(1),
                0,
                std::slice::from_ref(&sample),
            )],
        )
        .expect("media segment");
        let mut file = init;
        file.extend_from_slice(&frag);

        let media = crate::mp4_collect::media_from_mp4(&file).expect("demux the 2-byte HEVC fMP4");
        let track = &media.tracks[0];
        assert_eq!(
            track.samples[0].data.as_ref(),
            [
                0x00, 0x00, 0x00, 0x05, 0x26, 0x01, 0xAF, 0x09, 0x40, 0x00, 0x00, 0x00, 0x02, 0x44,
                0x01,
            ],
            "2-byte NAL lengths must be rewritten to 4-byte"
        );
        let CodecConfig::Hevc { config, .. } = &track.spec.config else {
            panic!("expected CodecConfig::Hevc");
        };
        assert_eq!(
            config.config.length_size_minus_one, NAL_LENGTH_SIZE_MINUS_ONE,
            "the reported hvcC must declare the 4-byte length size"
        );
    }
}
