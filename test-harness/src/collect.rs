//! Collect a streaming demux stage's [`DemuxEvent`]s into the per-track vectors
//! a [`Collected`] carries.
//!
//! A test that asserts on a whole `Collected` drives the one streaming MP4 stage
//! over a fixture and folds its event vocabulary into one here; the other
//! whole-`Collected` bridges in this module do the same for the mux, splice and
//! trick-play stages. The fold keeps the init's track order and its
//! sample-owned summaries (`start_decode_time`, skipped and degraded
//! records), so a test can compare what the demuxer reported on those terms.

use std::collections::BTreeMap;

use broadcast_common::Stage;
use broadcast_common::stage::Timestamp;
use broadcast_core::timescale::Timescale;
use transmux::{
    AbandonReason, AfdPersistence, DegradationRecord, DemuxEvent, InputDegradation, PcrSample,
    Sample, SkippedTrack, SplicePlan, SplicePoint, SpliceStage, StreamingFlvDemux,
    StreamingTsDemux, TrackId, TrackSpec, TrickPlayStage,
};

/// One elementary track in a [`Collected`] capture: its identity/config plus
/// its decode-ordered samples. The test-only form of transmux's removed
/// whole-input `CollectedTrack`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct CollectedTrack {
    /// CollectedTrack identity + codec configuration.
    pub spec: TrackSpec,
    /// The track's coded access units, in decode order.
    pub samples: Vec<Sample>,
    /// Absolute decode time of the track's first sample, in its media
    /// timescale.
    pub start_decode_time: u64,
}

impl CollectedTrack {
    /// Create a track from its spec and samples, anchored at decode time `0`.
    pub fn new(spec: TrackSpec, samples: Vec<Sample>) -> Self {
        Self {
            spec,
            samples,
            start_decode_time: 0,
        }
    }

    /// Create a track from its spec and samples, anchored at an absolute
    /// `start_decode_time` (in the track's media timescale).
    pub fn new_at(spec: TrackSpec, samples: Vec<Sample>, start_decode_time: u64) -> Self {
        Self {
            spec,
            samples,
            start_decode_time,
        }
    }

    /// Set the absolute [`start_decode_time`](Self::start_decode_time) anchor,
    /// returning `self` (builder style).
    pub fn with_start_decode_time(mut self, start_decode_time: u64) -> Self {
        self.start_decode_time = start_decode_time;
        self
    }

    /// CollectedTrack ID.
    pub fn track_id(&self) -> TrackId {
        self.spec.track_id
    }

    /// Collected timescale (ticks per second); non-zero by construction.
    pub fn timescale(&self) -> Timescale {
        self.spec.timescale
    }

    /// Codec configuration.
    pub fn config(&self) -> &transmux::CodecConfig {
        &self.spec.config
    }

    /// Apply an explicit HE-AAC profile to the track's spec, re-expressing
    /// every sample's `dts`/`pts`/`duration` and its `start_decode_time` in
    /// the new media timescale so no presentation time moves.
    ///
    /// # Errors
    /// The spec's own error when the profile cannot be applied.
    pub fn with_explicit_he_aac(
        mut self,
        profile: transmux::HeAacProfile,
    ) -> Result<Self, transmux::Error> {
        use broadcast_core::timescale::{Rounding, rescale_i64};
        let from = self.spec.timescale;
        self.spec = self.spec.with_explicit_he_aac(profile)?;
        let to = self.spec.timescale;
        if from != to {
            self.samples = self
                .samples
                .into_iter()
                .map(|s| s.rescale_timescale(from, to))
                .collect::<Result<Vec<_>, _>>()?;
            self.start_decode_time = u64::try_from(rescale_i64(
                i64::try_from(self.start_decode_time).unwrap_or(i64::MAX),
                from,
                to,
                Rounding::Floor,
            ))
            .unwrap_or(u64::MAX);
        }
        Ok(self)
    }
}

/// A whole capture's tracks plus the movie timescale and the demuxer's
/// skipped/damaged records: the test-only collected form transmux's removed
/// whole-input `Collected` used to be.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Collected {
    /// Elementary tracks, in source order.
    pub tracks: Vec<CollectedTrack>,
    /// Movie timescale.
    pub movie_timescale: Timescale,
    /// PCR observations recovered from a TS source, in wire order.
    pub pcr: Vec<PcrSample>,
    /// Tracks the demuxer could not model.
    pub skipped: Vec<SkippedTrack>,
    /// Damage the demuxer skipped over.
    pub degraded: Vec<InputDegradation>,
    /// Where each `degraded` entry was observed.
    pub degraded_at: Vec<DegradationRecord>,
}

impl Collected {
    /// Create a collection from tracks and a movie timescale.
    pub fn new(tracks: Vec<CollectedTrack>, movie_timescale: impl Into<Timescale>) -> Self {
        Self {
            tracks,
            movie_timescale: movie_timescale.into(),
            pcr: Vec::new(),
            skipped: Vec::new(),
            degraded: Vec::new(),
            degraded_at: Vec::new(),
        }
    }

    /// Attach a PCR timeline, returning `self` (builder style).
    pub fn with_pcr(mut self, pcr: Vec<PcrSample>) -> Self {
        self.pcr = pcr;
        self
    }
}

/// Feed a whole buffer to a stage in one piece (`[u8]::chunks` of this size
/// yields exactly one chunk).
pub const WHOLE_INPUT: usize = usize::MAX;

/// Movie timescale [`media_from_events`] uses when the event stream names none
/// (no event carries it). The caller that knows the real value overwrites it.
const DEFAULT_MOVIE_TIMESCALE: core::num::NonZeroU32 = match core::num::NonZeroU32::new(1_000) {
    Some(value) => value,
    None => core::num::NonZeroU32::MIN,
};

/// How [`media_from_mp4`] feeds the streaming MP4 stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CollectMode {
    /// Answer the reader's byte-range requests from the in-memory image.
    Seekable,
    /// Feed the image forward-only in chunks of this many bytes.
    Sequential {
        /// Largest chunk handed to one `feed`.
        chunk: usize,
    },
}

/// Drive a demux stage with `feeder`, finish it, and return every event it
/// produced.
pub fn collect_events<S, F>(mut stage: S, feeder: F) -> Result<Vec<DemuxEvent>, S::Error>
where
    S: Stage<Out = DemuxEvent>,
    F: FnOnce(&mut S) -> Result<(), S::Error>,
{
    feeder(&mut stage)?;
    stage.finish()?;
    let mut events = Vec::new();
    while let Some(event) = stage.poll() {
        events.push(event);
    }
    Ok(events)
}

/// Feed a whole [`Collected`] to a muxer stage and collect what it produces.
///
/// Every track is announced first, then samples go in decode-time order
/// across tracks (ties to the earlier track), then the stage is finished.
/// This is the test-only bridge for a caller that still holds a whole
/// [`Collected`] capture.
///
/// # Errors
/// Whatever the stage returns from `feed` or `finish`.
pub fn drive<S>(
    stage: &mut S,
    media: &Collected,
    now: Timestamp,
) -> Result<Vec<transmux::mux_stage::MuxOutput>, S::Error>
where
    S: for<'a> Stage<In<'a> = transmux::mux_stage::MuxInput, Out = transmux::mux_stage::MuxOutput>,
{
    let mut out = Vec::new();
    for track in &media.tracks {
        stage.feed(
            transmux::mux_stage::MuxInput::Track(Box::new(track.spec.clone())),
            now,
        )?;
        drain(stage, &mut out);
    }
    let mut cursors: Vec<DecodeCursor> = media.tracks.iter().map(DecodeCursor::new).collect();
    while let Some(index) = earliest(&cursors) {
        let cursor = &mut cursors[index];
        let track = &media.tracks[index];
        let sample = track.samples[cursor.next].clone();
        cursor.advance(&sample);
        stage.feed(
            transmux::mux_stage::MuxInput::Sample {
                track: track.spec.track_id,
                sample,
            },
            now,
        )?;
        drain(stage, &mut out);
    }
    stage.finish()?;
    drain(stage, &mut out);
    Ok(out)
}

fn drain<S>(stage: &mut S, out: &mut Vec<transmux::mux_stage::MuxOutput>)
where
    S: for<'a> Stage<In<'a> = transmux::mux_stage::MuxInput, Out = transmux::mux_stage::MuxOutput>,
{
    while let Some(item) = stage.poll() {
        out.push(item);
    }
}

struct DecodeCursor<'m> {
    samples: &'m [Sample],
    timescale: i128,
    next: usize,
    decode: i64,
}

impl<'m> DecodeCursor<'m> {
    fn new(track: &'m CollectedTrack) -> Self {
        let mut decode = i64::try_from(track.start_decode_time).unwrap_or(i64::MAX);
        let mut head_ticks: u64 = 0;
        let mut has_timed = false;
        for sample in &track.samples {
            let Some(timing) = sample.timing else {
                continue;
            };
            if timing.dts().is_some() {
                has_timed = true;
                break;
            }
            head_ticks = head_ticks.saturating_add(u64::from(timing.duration().unwrap_or(0)));
        }
        if has_timed {
            decode = decode.saturating_sub(i64::try_from(head_ticks).unwrap_or(i64::MAX));
        }
        Self {
            samples: &track.samples,
            timescale: i128::from(track.spec.timescale),
            next: 0,
            decode,
        }
    }

    fn decode_time(&self) -> Option<i64> {
        let sample = self.samples.get(self.next)?;
        Some(
            sample
                .timing
                .and_then(|timing| timing.dts())
                .unwrap_or(self.decode),
        )
    }

    fn advance(&mut self, sample: &Sample) {
        let start = self.decode_time().unwrap_or(self.decode);
        let duration = sample
            .timing
            .and_then(|timing| timing.duration())
            .map_or(0, i64::from);
        self.decode = start.saturating_add(duration);
        self.next += 1;
    }
}

fn earliest(cursors: &[DecodeCursor<'_>]) -> Option<usize> {
    let mut best: Option<(usize, i128, i128)> = None;
    for (index, cursor) in cursors.iter().enumerate() {
        let Some(time) = cursor.decode_time() else {
            continue;
        };
        let time = i128::from(time);
        let earlier = best.is_none_or(|(_, best_time, best_scale)| {
            time * best_scale < best_time * cursor.timescale
        });
        if earlier {
            best = Some((index, time, cursor.timescale));
        }
    }
    best.map(|(index, ..)| index)
}

/// Fold a demux event stream into a [`Collected`].
///
/// `start_decode_time` is the track's first sample's `dts` (an MP4 decode
/// timeline starts at the anchor its first sample names, which is `0` for a
/// progressive file). `skipped` is filled from `TrackAbandoned`, `degraded`
/// from `InputDegraded`.
pub fn media_from_events(events: Vec<DemuxEvent>) -> Collected {
    let mut tracks: Vec<CollectedTrack> = Vec::new();
    let mut index_by_id: BTreeMap<TrackId, usize> = BTreeMap::new();
    let mut skipped = Vec::new();
    let mut degraded = Vec::new();
    for event in events {
        match event {
            DemuxEvent::TrackAdded(spec) => {
                index_by_id.insert(spec.track_id, tracks.len());
                tracks.push(CollectedTrack::new(spec, Vec::new()));
            }
            DemuxEvent::TrackUpdated(spec) => {
                if let Some(&index) = index_by_id.get(&spec.track_id) {
                    tracks[index].spec = spec;
                }
            }
            DemuxEvent::Sample {
                track_id, sample, ..
            } => {
                if let Some(&index) = index_by_id.get(&track_id) {
                    let track = &mut tracks[index];
                    if track.samples.is_empty()
                        && let Some(dts) = sample.timing.and_then(|timing| timing.dts())
                    {
                        track.start_decode_time = u64::try_from(dts).unwrap_or(0);
                    }
                    track.samples.push(sample);
                }
            }
            DemuxEvent::TrackAbandoned {
                track_id, reason, ..
            } => skipped.push(skipped_from_reason(reason, track_id)),
            DemuxEvent::InputDegraded { kind, .. } => degraded.push(kind),
            DemuxEvent::DamagedSpan { offset, .. } => {
                degraded.push(InputDegradation::MalformedUnit { position: offset })
            }
            _ => {}
        }
    }
    let mut media = Collected::new(
        tracks,
        broadcast_core::timescale::Timescale::from_nonzero(DEFAULT_MOVIE_TIMESCALE),
    );
    media.skipped = skipped;
    media.degraded = degraded;
    media
}

fn skipped_from_reason(reason: AbandonReason, track_id: Option<TrackId>) -> SkippedTrack {
    let (fourcc, text) = match reason {
        AbandonReason::UnsupportedSampleEntry { fourcc, detail } => {
            let fourcc = String::from_utf8_lossy(&fourcc).into_owned();
            let text = match detail {
                Some(detail) => format!("sample entry '{fourcc}' rejected: {detail}"),
                None => format!(
                    "sample entry '{fourcc}' has no CodecConfig reconstruction in this crate"
                ),
            };
            (fourcc, text)
        }
        AbandonReason::UnsupportedCodec => (
            String::from("unknown"),
            "sample entry 'unknown' has no CodecConfig reconstruction in this crate".into(),
        ),
        AbandonReason::MalformedTrack { expected } => (
            String::from("unknown"),
            format!("unexpected box: expected {expected}"),
        ),
        other => (String::from("unknown"), other.name().into()),
    };
    let mut skipped = SkippedTrack::new(fourcc, text);
    skipped.track_id = track_id;
    skipped
}

/// Drive [`Mp4Demux`](transmux::mp4_demux::Mp4Demux) over a whole in-memory
/// MP4 image and fold its events into a [`Collected`].
///
/// A whole small fixture in memory is fine **here only**: this crate is a test
/// harness. The stage itself never holds more than the reader asks for.
pub fn media_from_mp4(bytes: &[u8], mode: CollectMode) -> Result<Collected, transmux::Error> {
    let total_len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    let config = transmux::mp4_demux::Mp4DemuxConfig::default().with_total_len(Some(total_len));
    match mode {
        CollectMode::Seekable => {
            let mut demux = transmux::mp4_demux::Mp4Demux::seekable(config);
            drive_seekable(&mut demux, bytes)?;
            media_from_stage(demux)
        }
        CollectMode::Sequential { chunk } => {
            let mut demux = transmux::mp4_demux::Mp4Demux::sequential(config);
            drive_sequential(&mut demux, bytes, chunk.max(1))?;
            media_from_stage(demux)
        }
    }
}

fn drive_seekable(
    demux: &mut transmux::mp4_demux::Mp4Demux,
    bytes: &[u8],
) -> Result<(), transmux::Error> {
    loop {
        let demand = demux.demand();
        let Some(offset) = demand.offset else {
            break;
        };
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        let end = start.saturating_add(demand.want_bytes).min(bytes.len());
        let slice = bytes.get(start..end).unwrap_or(&[]);
        demux.feed(
            transmux::mp4_demux::Mp4Input::At {
                offset,
                bytes: slice.to_vec().into(),
            },
            broadcast_common::stage::Timestamp::ZERO,
        )?;
    }
    demux.finish()
}

fn drive_sequential(
    demux: &mut transmux::mp4_demux::Mp4Demux,
    bytes: &[u8],
    chunk: usize,
) -> Result<(), transmux::Error> {
    let mut offset = 0usize;
    while offset < bytes.len() {
        let demand = demux.demand();
        if demand.saturated {
            break;
        }
        let want = demand.want_bytes.max(1);
        let take = want.min(chunk).min(bytes.len() - offset);
        demux.feed(
            transmux::mp4_demux::Mp4Input::Next(&bytes[offset..offset + take]),
            broadcast_common::stage::Timestamp::ZERO,
        )?;
        offset += take;
    }
    demux.finish()
}

fn media_from_stage(
    mut demux: transmux::mp4_demux::Mp4Demux,
) -> Result<Collected, transmux::Error> {
    let fragmented = demux.is_fragmented();
    let movie_timescale = demux.movie_timescale();
    let init_order: Vec<TrackId> = demux.init_tracks().map(|spec| spec.track_id).collect();
    let mut events = Vec::new();
    while let Some(event) = demux.poll() {
        events.push(event);
    }
    let mut media = media_from_events(events);
    media.tracks.sort_by_key(|track| {
        init_order
            .iter()
            .position(|id| *id == track.spec.track_id)
            .unwrap_or(usize::MAX)
    });
    if let Some(timescale) = movie_timescale {
        media.movie_timescale = timescale;
    }
    if fragmented == Some(false) {
        for track in &mut media.tracks {
            track.start_decode_time = 0;
        }
    }
    Ok(media)
}

/// Drive [`StreamingPsDemux`](transmux::StreamingPsDemux) over a whole
/// in-memory Program Stream image, feeding it in `chunk`-sized pieces, and fold
/// its events into a [`Collected`].
///
/// A whole small fixture in memory is fine **here only**: this crate is a test
/// harness. The stage itself never holds the input.
pub fn media_from_ps(bytes: &[u8], chunk: usize) -> Result<Collected, transmux::Error> {
    let chunk = chunk.max(1);
    let mut demux = transmux::StreamingPsDemux::new();
    for piece in bytes.chunks(chunk) {
        demux.feed(piece, broadcast_common::stage::Timestamp::ZERO)?;
    }
    demux.finish()?;
    let mut events = Vec::new();
    while let Some(event) = demux.poll() {
        events.push(event);
    }
    let mut media = media_from_events(events);
    media.movie_timescale = transmux::rtp::VIDEO_CLOCK_TIMESCALE;
    Ok(media)
}

/// Drive [`StreamingTsDemux`] over a whole in-memory MPEG-2 TS image in one
/// feed and fold its events into a [`Collected`], exactly as the deleted
/// whole-input `TsDemux::demux` did: strict PMT track order, the program
/// `ClockReference`s as [`PcrSample`]s, and every damage record resolved
/// against the final track set by source PID.
///
/// A whole small fixture in memory is fine **here only**: this crate is a test
/// harness. The stage itself never holds the input.
pub fn media_from_ts(bytes: &[u8]) -> Result<Collected, transmux::Error> {
    media_from_ts_chunked(bytes, WHOLE_INPUT)
}

/// [`media_from_ts`] fed in `chunk`-sized pieces (the streaming-gate form).
pub fn media_from_ts_chunked(bytes: &[u8], chunk: usize) -> Result<Collected, transmux::Error> {
    media_from_ts_with_afd(bytes, chunk, AfdPersistence::UntilReplaced)
}

/// [`media_from_ts_chunked`] with an explicit AFD persistence policy.
pub fn media_from_ts_with_afd(
    bytes: &[u8],
    chunk: usize,
    afd: AfdPersistence,
) -> Result<Collected, transmux::Error> {
    let mut demux = StreamingTsDemux::new()
        .with_afd_persistence(afd)
        .with_strict_pmt_order();
    for piece in bytes.chunks(chunk.max(1)) {
        demux.feed(piece)?;
    }
    demux.finish()?;

    let mut tracks: Vec<CollectedTrack> = Vec::new();
    let mut index_by_id: BTreeMap<TrackId, usize> = BTreeMap::new();
    let mut pcr: Vec<PcrSample> = Vec::new();
    let mut degraded: Vec<InputDegradation> = Vec::new();
    let mut degraded_at: Vec<DegradationRecord> = Vec::new();
    while let Some(event) = demux.poll_event() {
        match event {
            DemuxEvent::TrackAdded(spec) => {
                index_by_id.insert(spec.track_id, tracks.len());
                tracks.push(CollectedTrack::new(spec, Vec::new()));
            }
            DemuxEvent::TrackUpdated(spec) => {
                if let Some(&index) = index_by_id.get(&spec.track_id) {
                    tracks[index].spec = spec;
                }
            }
            DemuxEvent::Sample {
                track_id, sample, ..
            } => {
                if let Some(&index) = index_by_id.get(&track_id) {
                    let track = &mut tracks[index];
                    if track.samples.is_empty()
                        && let Some(dts) = sample.timing.and_then(|timing| timing.dts())
                    {
                        track.start_decode_time = u64::try_from(dts).unwrap_or(0);
                    }
                    track.samples.push(sample);
                }
            }
            DemuxEvent::ClockReference {
                pcr: wire,
                discontinuous,
                provenance,
                ..
            } => {
                if let (Some(wire), Some(pid), Some(packet_index)) =
                    (wire, provenance.pid, provenance.packet_index)
                {
                    pcr.push(PcrSample::new(wire, pid, packet_index, discontinuous));
                }
            }
            DemuxEvent::InputDegraded {
                track,
                kind,
                provenance,
                ..
            } => {
                degraded.push(kind);
                degraded_at.push(DegradationRecord::new(kind, track, provenance));
            }
            DemuxEvent::TrackRemoved { .. }
            | DemuxEvent::TrackAbandoned { .. }
            | DemuxEvent::Discontinuity { .. }
            | DemuxEvent::TracksResolved { .. }
            | DemuxEvent::DisplayChanged { .. } => {}
            _ => {}
        }
    }
    for record in &mut degraded_at {
        if record.track.is_none()
            && let Some(pid) = record.provenance.pid
            && let Some(t) = tracks.iter().find(|t| t.spec.source_pid == Some(pid))
        {
            record.track = Some(t.spec.track_id);
        }
    }
    let mut media = Collected::new(tracks, transmux::rtp::VIDEO_CLOCK_TIMESCALE).with_pcr(pcr);
    media.degraded = degraded;
    media.degraded_at = degraded_at;
    Ok(media)
}

/// Drive [`StreamingFlvDemux`] over a whole in-memory FLV image and fold its
/// events into a [`Collected`] on FLV's 1 kHz clock.
///
/// A whole small fixture in memory is fine **here only**: this crate is a test
/// harness. The stage itself never holds the input.
///
/// # Errors
///
/// The first structural error the stage reports (bad signature, implausible
/// header, corrupt codec config).
pub fn media_from_flv(bytes: &[u8]) -> Result<Collected, transmux::flv::FlvError> {
    media_from_flv_chunked(bytes, WHOLE_INPUT)
}

/// [`media_from_flv`] fed in `chunk`-sized pieces.
///
/// # Errors
///
/// The first structural error the stage reports.
pub fn media_from_flv_chunked(
    bytes: &[u8],
    chunk: usize,
) -> Result<Collected, transmux::flv::FlvError> {
    let mut demux = StreamingFlvDemux::new();
    for piece in bytes.chunks(chunk.max(1)) {
        demux.feed(piece)?;
    }
    demux.finish()?;
    let mut events = Vec::new();
    while let Some(event) = demux.poll_event() {
        events.push(event);
    }
    let mut media = media_from_events(events);
    if let Some(track) = media.tracks.first() {
        media.movie_timescale = track.spec.timescale;
    }
    Ok(media)
}

/// Drive the RTMP chunk reassembly and the streaming FLV demux over a whole
/// post-handshake RTMP chunk stream, folding the result into a [`Collected`].
///
/// The reassembly half is exactly what the deleted whole-input `RtmpDemux` did:
/// collect the Audio (type 8), Video (type 9) and `onMetaData` (type 18)
/// message bodies, rebuild an FLV byte stream from them, and demux that.
///
/// # Errors
///
/// An RTMP wire error, a rebuilt tag body too large for FLV's UI24
/// `DataSize`, or the FLV stage's own error.
pub fn media_from_rtmp(bytes: &[u8]) -> Result<Collected, transmux::RtmpFlvError> {
    media_from_rtmp_chunked(bytes, WHOLE_INPUT)
}

/// [`media_from_rtmp`] fed in `chunk`-sized pieces.
///
/// # Errors
///
/// An RTMP wire error, a rebuilt tag body too large for FLV's UI24
/// `DataSize`, or the FLV stage's own error.
pub fn media_from_rtmp_chunked(
    bytes: &[u8],
    chunk: usize,
) -> Result<Collected, transmux::RtmpFlvError> {
    use broadcast_core::Serialize as _;
    use rtmp_runtime::chunk::ChunkAssembler;
    use rtmp_runtime::flv::{FlvHeader, FlvTag, TagType};
    use rtmp_runtime::message::{MessageTypeId, ProtocolControl};

    let mut assembler = ChunkAssembler::new();
    let mut tags: Vec<(TagType, u32, Vec<u8>)> = Vec::new();
    let mut has_video = false;
    let mut has_audio = false;
    for piece in bytes.chunks(chunk.max(1)) {
        assembler.feed(piece);
        while let Some(message) = assembler.next_message()? {
            match message.message_type_id {
                MessageTypeId::SetChunkSize => {
                    if let Some(ProtocolControl::SetChunkSize(size)) =
                        ProtocolControl::from_message(&message)?
                    {
                        assembler.compact();
                        assembler.set_chunk_size(size);
                    }
                }
                MessageTypeId::Audio => {
                    has_audio = true;
                    tags.push((TagType::Audio, message.timestamp.millis(), message.payload));
                }
                MessageTypeId::Video => {
                    has_video = true;
                    tags.push((TagType::Video, message.timestamp.millis(), message.payload));
                }
                MessageTypeId::DataAmf0 => {
                    tags.push((TagType::Script, message.timestamp.millis(), message.payload));
                }
                _ => {}
            }
        }
    }
    let mut flv = FlvHeader::new(has_audio, has_video).try_to_bytes()?;
    for (tag_type, timestamp, body) in &tags {
        flv.extend_from_slice(&FlvTag::new(*tag_type, *timestamp, body).try_to_bytes()?);
    }
    media_from_flv(&flv).map_err(transmux::RtmpFlvError::Flv)
}

/// The result of driving a [`SplicePlan`] over whole medias: the joined media
/// and the joins to signal. Field-compatible with the former whole-`Collected`
/// splice result, now produced by [`SpliceStage`].
pub struct SpliceOutput {
    /// The joined media on a single monotonic timeline.
    pub media: Collected,
    /// The join points to signal as `#EXT-X-DISCONTINUITY`.
    pub discontinuity_points: Vec<SplicePoint>,
}

/// Build the per-track splice index [`SplicePlan`] reads: identity plus each
/// sample's timing and sync flag, never its coded bytes.
pub fn splice_index(tracks: &[CollectedTrack]) -> transmux::SpliceIndex {
    let mut builder = transmux::SpliceIndexBuilder::new();
    for track in tracks {
        let index = builder.add_track(track.spec.clone(), track.start_decode_time);
        for sample in &track.samples {
            builder.push_sample(index, sample.timing, sample.flags.is_sync);
        }
    }
    builder.build()
}

/// Append `b` after `a` through [`SpliceStage`].
///
/// # Errors
/// Whatever [`SplicePlan::concat`] or the stage returns.
pub fn concat(a: &Collected, b: &Collected) -> Result<SpliceOutput, transmux::Error> {
    let plan = SplicePlan::concat(
        &splice_index(&a.tracks),
        &splice_index(&b.tracks),
        a.movie_timescale,
    )?;
    drive_splice(&plan, a, Some(b))
}

/// Splice `ad` into `base` through [`SpliceStage`].
///
/// # Errors
/// Whatever [`SplicePlan::insert`] or the stage returns.
pub fn splice_insert(
    base: &Collected,
    ad: &Collected,
    at_ticks: u64,
) -> Result<SpliceOutput, transmux::Error> {
    let plan = SplicePlan::insert(
        &splice_index(&base.tracks),
        &splice_index(&ad.tracks),
        base.movie_timescale,
        at_ticks,
    )?;
    drive_splice(&plan, base, Some(ad))
}

fn drive_splice(
    plan: &SplicePlan,
    base: &Collected,
    inserted: Option<&Collected>,
) -> Result<SpliceOutput, transmux::Error> {
    let mut per_track: Vec<Vec<Sample>> = vec![Vec::new(); plan.tracks().len()];
    for (index, slice) in plan.tracks().iter().enumerate() {
        let base_track = base
            .tracks
            .get(index)
            .ok_or(transmux::Error::InvalidInput("splice: base track vanished"))?;
        let mut stage = SpliceStage::new(
            plan.only(index)
                .ok_or(transmux::Error::InvalidInput("splice: plan track vanished"))?,
        );
        let split = slice.split_index.min(base_track.samples.len());
        for sample in &base_track.samples[..split] {
            stage.feed((slice.track_id, sample.clone()), Timestamp::ZERO)?;
        }
        if let Some(media) = inserted
            && let Some(track) = media.tracks.get(slice.insert_index)
        {
            for sample in &track.samples {
                stage.feed((slice.track_id, sample.clone()), Timestamp::ZERO)?;
            }
        }
        for sample in &base_track.samples[split..] {
            stage.feed((slice.track_id, sample.clone()), Timestamp::ZERO)?;
        }
        stage.finish()?;
        while let Some((_, sample)) = stage.poll() {
            per_track[index].push(sample);
        }
    }
    let mut tracks = Vec::with_capacity(base.tracks.len());
    for (track, samples) in base.tracks.iter().zip(per_track) {
        let start = samples
            .iter()
            .find_map(|s| s.timing.and_then(|t| t.dts()))
            .map(|dts| dts.max(0) as u64)
            .unwrap_or(track.start_decode_time);
        tracks.push(CollectedTrack::new_at(track.spec.clone(), samples, start));
    }
    Ok(SpliceOutput {
        media: Collected::new(tracks, plan.movie_timescale()),
        discontinuity_points: plan.discontinuity_points().to_vec(),
    })
}

/// Derive an I-frame-only track from `src` through [`TrickPlayStage`].
///
/// # Errors
/// [`transmux::Error::InvalidInput`] when `src` has no sync samples.
pub fn derive_iframe_track(src: &CollectedTrack) -> Result<CollectedTrack, transmux::Error> {
    let mut stage = TrickPlayStage::new(src.spec.track_id, src.spec.clone());
    for sample in &src.samples {
        stage.feed((src.spec.track_id, sample.clone()), Timestamp::ZERO)?;
    }
    stage.finish()?;
    let mut samples = Vec::new();
    let mut spec = None;
    while let Some(item) = stage.poll() {
        match item {
            transmux::mux_stage::MuxInput::Track(announced) => spec = Some(*announced),
            transmux::mux_stage::MuxInput::Sample { sample, .. } => samples.push(sample),
            _ => {}
        }
    }
    let spec = spec.ok_or(transmux::Error::InvalidInput(
        "trick-play derived no track to announce",
    ))?;
    let start = samples
        .iter()
        .find_map(|s| s.timing.and_then(|t| t.dts()))
        .map(|dts| dts.max(0) as u64)
        .unwrap_or(src.start_decode_time);
    Ok(CollectedTrack::new_at(spec, samples, start))
}

/// Append a track derived by [`derive_iframe_track`] at `index`.
///
/// # Errors
/// [`transmux::Error::InvalidInput`] when `index` is out of range or the
/// source has no sync samples.
pub fn append_iframe_track(media: &mut Collected, index: usize) -> Result<(), transmux::Error> {
    let source = media
        .tracks
        .get(index)
        .ok_or(transmux::Error::InvalidInput(
            "video_track_index out of bounds for media.tracks",
        ))?;
    let derived = derive_iframe_track(source)?;
    media.tracks.push(derived);
    Ok(())
}

/// A summary log for a whole [`Collected`] as one segment per track: the bridge
/// for a caller that still holds the samples.
pub fn segment_log(media: &Collected) -> transmux::segment_log::SegmentSummaryLog {
    let mut log = transmux::segment_log::SegmentSummaryLog::new();
    for track in &media.tracks {
        let duration: u64 = track
            .samples
            .iter()
            .map(|s| u64::from(s.timing.and_then(|t| t.duration()).unwrap_or(0)))
            .sum();
        let bytes: u64 = track.samples.iter().map(|s| s.data.len() as u64).sum();
        if track.samples.is_empty() {
            continue;
        }
        log.push_part(
            &transmux::mux_stage::TrackPart {
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

/// The whole-input helper methods tests use on a collected [`Collected`]: track
/// selection, a presentation-time window, and the anchor duration. These were
/// `transmux` production functions until Phase 6 slice 3a; they live here (a
/// `publish = false` test harness) so production code cannot hold a whole
/// input.
pub trait CollectedSelectExt {
    /// Select tracks by index, preserving the given order.
    fn select_tracks(&self, indices: &[usize]) -> Result<Collected, transmux::Error>;

    /// Select tracks by predicate, preserving source order.
    fn select_tracks_by<F>(&self, keep: F) -> Result<Collected, transmux::Error>
    where
        F: FnMut(&CollectedTrack) -> bool;

    /// Trim to the half-open presentation-time window `[start, end)` in the
    /// movie timescale, snapping the anchor back to its preceding sync sample.
    fn trim(&self, start: u64, end: u64) -> Result<Collected, transmux::Error>;

    /// Total duration of the segmentation anchor track, in its timescale.
    fn anchor_duration(&self) -> Option<(u64, u32)>;
}

impl CollectedSelectExt for Collected {
    fn select_tracks(&self, indices: &[usize]) -> Result<Collected, transmux::Error> {
        if indices.is_empty() {
            return Err(transmux::Error::InvalidInput(
                "select_tracks: empty track selection",
            ));
        }
        let mut tracks = Vec::with_capacity(indices.len());
        for &i in indices {
            let t = self.tracks.get(i).ok_or(transmux::Error::InvalidInput(
                "select_tracks: track index out of range",
            ))?;
            tracks.push(t.clone());
        }
        Ok(Collected::new(tracks, self.movie_timescale))
    }

    fn select_tracks_by<F>(&self, mut keep: F) -> Result<Collected, transmux::Error>
    where
        F: FnMut(&CollectedTrack) -> bool,
    {
        let tracks: Vec<CollectedTrack> = self.tracks.iter().filter(|t| keep(t)).cloned().collect();
        if tracks.is_empty() {
            return Err(transmux::Error::InvalidInput(
                "select_tracks_by: predicate kept no track",
            ));
        }
        Ok(Collected::new(tracks, self.movie_timescale))
    }

    fn trim(&self, start: u64, end: u64) -> Result<Collected, transmux::Error> {
        use broadcast_core::timescale::{Rounding, rescale_i64};
        if start >= end {
            return Err(transmux::Error::InvalidInput("trim: start must be < end"));
        }
        if self.tracks.is_empty() {
            return Err(transmux::Error::InvalidInput("trim: media has no tracks"));
        }
        let anchor =
            transmux::segmenter::choose_anchor(self.tracks.iter().map(|t| &t.spec.config))?;
        let origin = origin_of(&self.tracks);
        let anchor_track = &self.tracks[anchor];
        let anchor_ts = anchor_track.spec.timescale;
        let anchor_pts = presentation_times(anchor_track, origin);
        let anchor_lo = rescale_floor(start, self.movie_timescale, anchor_ts);
        let anchor_hi = rescale_floor(end, self.movie_timescale, anchor_ts);
        let snapped = anchor_pts
            .iter()
            .position(|&p| in_window(p, anchor_lo, anchor_hi))
            .map(|mut idx| {
                while idx > 0 && !anchor_track.samples[idx].flags.is_sync {
                    idx -= 1;
                }
                idx
            });
        let mut out_tracks = Vec::with_capacity(self.tracks.len());
        let mut kept_any = false;
        for (ti, track) in self.tracks.iter().enumerate() {
            let ts = track.spec.timescale;
            let pts = presentation_times(track, origin);
            let hi = rescale_floor(end, self.movie_timescale, ts);
            let start_idx = if ti == anchor {
                snapped
            } else {
                let lo = match snapped {
                    Some(idx) => rescale_i64(anchor_pts[idx], anchor_ts, ts, Rounding::Floor),
                    None => rescale_floor(start, self.movie_timescale, ts),
                };
                pts.iter().position(|&p| in_window(p, lo, hi))
            };
            let mut kept = Vec::new();
            if let Some(start_idx) = start_idx {
                for (s, &p) in track.samples[start_idx..].iter().zip(&pts[start_idx..]) {
                    if p >= hi {
                        break;
                    }
                    kept.push(s.clone());
                }
            }
            if !kept.is_empty() {
                kept_any = true;
            }
            let first_dts = kept
                .iter()
                .find_map(|s| s.timing.and_then(|t| t.dts()))
                .unwrap_or(0)
                .max(0) as u64;
            out_tracks.push(CollectedTrack::new_at(track.spec.clone(), kept, first_dts));
        }
        if !kept_any {
            return Err(transmux::Error::InvalidInput(
                "trim: window selected no samples on any track",
            ));
        }
        Ok(Collected::new(out_tracks, self.movie_timescale))
    }

    fn anchor_duration(&self) -> Option<(u64, u32)> {
        let anchor =
            transmux::segmenter::choose_anchor(self.tracks.iter().map(|t| &t.spec.config)).ok()?;
        let t = &self.tracks[anchor];
        let ticks: u64 = t
            .samples
            .iter()
            .map(|s| s.timing.and_then(|x| x.duration()).unwrap_or(0) as u64)
            .sum();
        Some((ticks, t.spec.timescale.get()))
    }
}

/// The earliest first-sample decode time across `tracks`, with its timescale.
fn origin_of(tracks: &[CollectedTrack]) -> (i64, broadcast_core::timescale::Timescale) {
    use broadcast_core::timescale::{Timescale, cmp_ticks};
    let mut best: Option<(i64, Timescale)> = None;
    for t in tracks {
        let Some(d) = t
            .samples
            .iter()
            .find_map(|s| s.timing.and_then(|x| x.dts()))
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
    best.unwrap_or_else(|| (0, Timescale::try_new(1).expect("a non-zero timescale")))
}

/// A track's presentation times relative to `origin`, in its own timescale.
fn presentation_times(
    track: &CollectedTrack,
    origin: (i64, broadcast_core::timescale::Timescale),
) -> Vec<i64> {
    use broadcast_core::timescale::{Rounding, rescale_i64};
    let base = rescale_i64(origin.0, origin.1, track.spec.timescale, Rounding::Floor);
    let mut next = 0i64;
    let mut out = Vec::with_capacity(track.samples.len());
    for s in &track.samples {
        let rel = match s.timing.and_then(|t| t.dts()) {
            Some(d) => d - base,
            None => next,
        };
        let duration = s.timing.and_then(|t| t.duration()).unwrap_or(0);
        next = rel.saturating_add(i64::from(duration));
        let offset = s.composition_offset().ok().flatten().unwrap_or(0);
        out.push(rel.saturating_add(i64::from(offset)));
    }
    out
}

fn rescale_floor(
    ticks: u64,
    from_timescale: broadcast_core::timescale::Timescale,
    to_timescale: broadcast_core::timescale::Timescale,
) -> i64 {
    use broadcast_core::timescale::{Rounding, rescale_i64};
    rescale_i64(
        i64::try_from(ticks).unwrap_or(i64::MAX),
        from_timescale,
        to_timescale,
        Rounding::Floor,
    )
}

fn in_window(presentation: i64, lo: i64, hi: i64) -> bool {
    presentation >= lo && presentation < hi
}

/// Build a whole CMAF output (init + media segments) from a collected [`Collected`]
/// by driving the one [`Segmenter`](transmux::Segmenter) stage — the test-only
/// replacement for the deleted whole-input repackager.
pub fn cmaf_from_media(
    media: &Collected,
    target: core::time::Duration,
) -> Result<transmux::RepackageOutput, transmux::Error> {
    let specs: Vec<_> = media.tracks.iter().map(|t| t.spec.clone()).collect();
    let mut segmenter = transmux::Segmenter::new(specs, media.movie_timescale, target)?;
    let origin = origin_of(&media.tracks);
    for track in &media.tracks {
        let first = track
            .samples
            .first()
            .and_then(|s| s.timing.and_then(|t| t.dts()))
            .map_or(0, |dts| {
                dts.saturating_sub(rescale_floor(
                    origin.0.max(0) as u64,
                    origin.1,
                    track.spec.timescale,
                ))
                .max(0) as u64
            });
        segmenter.set_start_decode_time(track.spec.track_id, first)?;
    }
    let init_segment = segmenter.init_segment()?;
    let mut adapter = SegmenterAdapter(segmenter);
    let mut media_segments = Vec::new();
    for output in drive(&mut adapter, media, Timestamp::ZERO)? {
        if let transmux::mux_stage::MuxOutput::Segment(segment) = output {
            media_segments.push(segment.bytes.to_vec());
        }
    }
    Ok(transmux::RepackageOutput {
        init_segment,
        media_segments,
    })
}

/// Drops the track announcements [`drive`] sends, since a
/// [`Segmenter`](transmux::Segmenter) is built with its track set.
struct SegmenterAdapter(transmux::Segmenter);

impl broadcast_common::Stage for SegmenterAdapter {
    type In<'a> = transmux::mux_stage::MuxInput;
    type Out = transmux::mux_stage::MuxOutput;
    type Error = transmux::Error;

    fn feed(&mut self, input: Self::In<'_>, now: Timestamp) -> Result<(), transmux::Error> {
        match input {
            transmux::mux_stage::MuxInput::Track(_) => Ok(()),
            other => self.0.feed(other, now),
        }
    }

    fn poll(&mut self) -> Option<transmux::mux_stage::MuxOutput> {
        self.0.poll()
    }

    fn finish(&mut self) -> Result<(), transmux::Error> {
        self.0.finish()
    }
}

/// Record a progressive sample plan from a collected [`Collected`] — the test-only
/// replacement for the deleted `PlanPass::for_media`.
pub fn plan_from_media(media: &Collected) -> Result<isobmff::plan::OwnedPlan, transmux::Error> {
    let mut pass = transmux::PlanPass::new(media.movie_timescale);
    for track in &media.tracks {
        pass.feed(
            transmux::mux_stage::MuxInput::Track(Box::new(track.spec.clone())),
            Timestamp::ZERO,
        )?;
    }
    for track in &media.tracks {
        for sample in &track.samples {
            pass.feed(
                transmux::mux_stage::MuxInput::Sample {
                    track: track.spec.track_id,
                    sample: sample.clone(),
                },
                Timestamp::ZERO,
            )?;
        }
    }
    pass.finish()?;
    pass.poll().ok_or(transmux::Error::InvalidInput(
        "progressive: the plan pass produced no plan",
    ))
}

/// Drive [`MatroskaDemux`](transmux::matroska_demux::MatroskaDemux) over a
/// whole in-memory Matroska/WebM image and fold its events into a [`Collected`].
///
/// A whole small fixture in memory is fine **here only**: this crate is a test
/// harness. The stage itself never holds the input.
pub fn media_from_matroska(bytes: &[u8], mode: CollectMode) -> Result<Collected, transmux::Error> {
    use transmux::matroska_demux::{MatroskaDemux, MatroskaDemuxConfig, MatroskaInput};
    let total_len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    let config = MatroskaDemuxConfig::default().with_total_len(Some(total_len));
    let mut demux = match mode {
        CollectMode::Seekable => MatroskaDemux::seekable(config),
        CollectMode::Sequential { .. } => MatroskaDemux::sequential(config),
    };
    match mode {
        CollectMode::Seekable => loop {
            let demand = demux.demand();
            let Some(offset) = demand.offset else {
                break;
            };
            let start = usize::try_from(offset).unwrap_or(usize::MAX);
            let end = start.saturating_add(demand.want_bytes).min(bytes.len());
            let slice = bytes.get(start..end).unwrap_or(&[]);
            demux.feed(
                MatroskaInput::At {
                    offset,
                    bytes: slice.to_vec().into(),
                },
                broadcast_common::stage::Timestamp::ZERO,
            )?;
        },
        CollectMode::Sequential { chunk } => {
            for piece in bytes.chunks(chunk.max(1)) {
                demux.feed(
                    MatroskaInput::Next(piece),
                    broadcast_common::stage::Timestamp::ZERO,
                )?;
            }
            demux.feed(MatroskaInput::Eof, broadcast_common::stage::Timestamp::ZERO)?;
        }
    }
    demux.finish()?;
    let mut events = Vec::new();
    while let Some(event) = demux.poll() {
        events.push(event);
    }
    let mut media = media_from_events(events);
    media.movie_timescale = transmux::matroska_demux::IR_TIMESCALE;
    Ok(media)
}

/// One RTP stream handed to [`media_from_rtp`]: its payload format, its
/// packets in arrival order, and the codec config / clock the SDP would carry.
///
/// The clock rules are the ones the deleted whole-input demuxer enforced:
/// an MPEG-4 audio stream's RTP clock is its sampling rate (RFC 3640 §3.1),
/// H.264's is 90 kHz (RFC 6184 §8.1), and a config for one payload format on a
/// stream of the other is a caller mistake.
pub struct RtpTestStream {
    /// The payload format carried on the stream.
    pub kind: transmux::RtpMediaKind,
    /// The stream's RTP clock rate in Hz.
    pub clock_rate: u32,
    /// The codec configuration (required for AAC).
    pub config: Option<transmux::CodecConfig>,
    /// The RTP packets, in arrival (sequence) order.
    pub packets: Vec<Vec<u8>>,
    /// Whether [`Self::with_clock_rate`] asserted the rate.
    clock_rate_explicit: bool,
}

impl RtpTestStream {
    /// A stream carrying `kind` with no codec config and its format's default
    /// clock (90 kHz for H.264, 44.1 kHz for AAC).
    pub fn new(kind: transmux::RtpMediaKind, packets: Vec<Vec<u8>>) -> Self {
        Self {
            kind,
            clock_rate: match kind {
                transmux::RtpMediaKind::H264 => transmux::rtp::VIDEO_CLOCK_RATE,
                _ => transmux::DEFAULT_AAC_CLOCK_RATE,
            },
            config: None,
            packets,
            clock_rate_explicit: false,
        }
    }

    /// Set the stream's codec configuration; for AAC an unasserted clock is
    /// replaced by the config's own sampling rate.
    #[must_use]
    pub fn with_config(mut self, config: transmux::CodecConfig) -> Self {
        if let transmux::CodecConfig::Aac { sample_rate, .. } = &config
            && !self.clock_rate_explicit
        {
            self.clock_rate = *sample_rate;
        }
        self.config = Some(config);
        self
    }

    /// Assert an explicit RTP clock rate (Hz).
    #[must_use]
    pub fn with_clock_rate(mut self, clock_rate: u32) -> Self {
        self.clock_rate = clock_rate;
        self.clock_rate_explicit = true;
        self
    }

    /// Validate the kind/clock/config against each other, returning the clock.
    ///
    /// # Errors
    ///
    /// A zero clock, an AAC config on a non-AAC stream (or vice versa), or a
    /// declared clock that disagrees with the config's sampling rate.
    fn validated_clock_rate(
        &self,
    ) -> Result<broadcast_core::timescale::Timescale, transmux::Error> {
        use transmux::Error;
        match &self.config {
            Some(transmux::CodecConfig::Aac { sample_rate, .. }) => {
                if self.kind != transmux::RtpMediaKind::Aac {
                    return Err(Error::InvalidInput(
                        "an RTP stream carrying an AAC config must have kind Aac",
                    ));
                }
                if self.clock_rate != *sample_rate {
                    return Err(Error::InvalidValue(broadcast_core::error::InvalidValue {
                        field: "rtp_clock_rate",
                        value: u64::from(self.clock_rate),
                        reason: "an MPEG-4 audio stream's RTP clock rate is its sampling rate",
                    }));
                }
                Ok(broadcast_core::timescale::Timescale::try_new(*sample_rate)?)
            }
            Some(transmux::CodecConfig::Avc { .. }) => {
                if self.kind != transmux::RtpMediaKind::H264 {
                    return Err(Error::InvalidInput(
                        "an RTP stream carrying an AVC config must have kind H264",
                    ));
                }
                if self.clock_rate != transmux::rtp::VIDEO_CLOCK_RATE {
                    return Err(Error::InvalidValue(broadcast_core::error::InvalidValue {
                        field: "rtp_clock_rate",
                        value: u64::from(self.clock_rate),
                        reason: "RFC 6184 §8.1 fixes the H.264 RTP clock rate at 90 kHz",
                    }));
                }
                Ok(transmux::rtp::VIDEO_CLOCK_TIMESCALE)
            }
            Some(_) | None => {
                if self.clock_rate == 0 {
                    return Err(Error::InvalidInput("RTP stream has no clock rate"));
                }
                Ok(broadcast_core::timescale::Timescale::try_new(
                    self.clock_rate,
                )?)
            }
        }
    }
}

/// The streams [`media_from_rtp`] depacketises.
pub struct RtpTestInput {
    /// One entry per RTP stream, in track-id order.
    pub streams: Vec<RtpTestStream>,
}

/// Drive [`RtpStreamDepacketiser`](transmux::rtp_stream::RtpStreamDepacketiser)
/// over one whole packet flow per stream and fold the samples into a [`Collected`].
///
/// # Errors
///
/// An AAC stream without its `AudioSpecificConfig`, an invalid clock rate, or
/// whatever the depacketiser reports.
pub fn media_from_rtp(input: RtpTestInput) -> Result<Collected, transmux::Error> {
    media_from_rtp_with_events(input).map(|(media, _)| media)
}

/// [`media_from_rtp`] also returning the depacketiser's loss/reorder events,
/// drained after the last packet.
///
/// # Errors
///
/// As [`media_from_rtp`].
pub fn media_from_rtp_with_events(
    input: RtpTestInput,
) -> Result<(Collected, Vec<transmux::RtpLossEvent>), transmux::Error> {
    let streams = input.streams;
    use transmux::rtp_stream::{RtpStreamDepacketiser, RtpStreamTrack};
    let mut tracks = Vec::with_capacity(streams.len());
    for (index, stream) in streams.iter().enumerate() {
        let track_id = TrackId::new(u32::try_from(index).unwrap_or(u32::MAX).saturating_add(1));
        let clock = stream.validated_clock_rate()?;
        let config = match &stream.config {
            Some(config) => config.clone(),
            None if matches!(stream.kind, transmux::RtpMediaKind::H264) => {
                rtp_placeholder_avc_config()
            }
            None if matches!(stream.kind, transmux::RtpMediaKind::Aac) => {
                return Err(transmux::Error::InvalidInput(
                    "an AAC RTP stream needs its AudioSpecificConfig",
                ));
            }
            None => {
                return Err(transmux::Error::InvalidInput(
                    "an RTP stream needs its codec config",
                ));
            }
        };
        tracks.push(RtpStreamTrack::new(track_id, stream.kind, config, clock));
    }
    let mut depack = RtpStreamDepacketiser::new(tracks);
    let mut per_track: Vec<Vec<Sample>> = vec![Vec::new(); streams.len()];
    for (index, stream) in streams.iter().enumerate() {
        let track_id = TrackId::new(u32::try_from(index).unwrap_or(u32::MAX).saturating_add(1));
        for packet in &stream.packets {
            per_track[index].extend(depack.push(track_id, packet)?);
        }
        per_track[index].extend(depack.flush(track_id)?);
    }
    let mut events = Vec::new();
    while let Some(event) = depack.poll_loss_event() {
        events.push(event);
    }
    // The streaming depacketiser emits each track's timestamps relative to its
    // first unwrapped presentation time, so a reordered video track's derived
    // `dts` can start below zero. The whole-input fold zero-based the whole
    // `Collected` at its lowest decode instant (the IR's `start_decode_time` is a
    // `u64` that must equal the first sample's `dts`), so translate every
    // sample by that one constant, exactly as the deleted batch path did.
    let origin = per_track
        .iter()
        .flat_map(|samples| samples.iter())
        .filter_map(|sample| sample.timing.and_then(|timing| timing.dts()))
        .min()
        .unwrap_or(0)
        .min(0);
    for samples in &mut per_track {
        for sample in samples.iter_mut() {
            if let Some(timing) = sample.timing {
                let dts = timing.dts().map(|dts| (dts - origin).max(0));
                let pts = timing.pts().map(|pts| (pts - origin).max(0));
                let duration = timing.duration();
                let (Some(dts), Some(pts), Some(duration)) = (dts, pts, duration) else {
                    continue;
                };
                sample.timing = Some(transmux::ir::SampleTiming::timed_with_duration(
                    dts, pts, duration,
                ));
            }
        }
    }
    let specs: Vec<transmux::TrackSpec> = streams
        .iter()
        .enumerate()
        .map(|(index, stream)| {
            let track_id = TrackId::new(u32::try_from(index).unwrap_or(u32::MAX).saturating_add(1));
            let clock = stream
                .validated_clock_rate()
                .unwrap_or(transmux::rtp::VIDEO_CLOCK_TIMESCALE);
            let config = match (&stream.config, stream.kind) {
                (Some(config), _) => config.clone(),
                (None, _) => rtp_placeholder_avc_config(),
            };
            (track_id, config, clock)
        })
        .map(|(track_id, config, clock)| transmux::TrackSpec::new(track_id, clock, config))
        .collect();
    let built = specs
        .into_iter()
        .zip(per_track)
        .map(|(spec, samples)| {
            let anchor = samples
                .first()
                .and_then(|sample| sample.timing.and_then(|timing| timing.dts()))
                .map(|dts| u64::try_from(dts.max(0)).unwrap_or(0))
                .unwrap_or(0);
            CollectedTrack::new_at(spec, samples, anchor)
        })
        .collect();
    Ok((
        Collected::new(built, transmux::rtp::VIDEO_CLOCK_TIMESCALE),
        events,
    ))
}

/// Minimal AVC config for a stream whose parameter sets live in the SDP.
fn rtp_placeholder_avc_config() -> transmux::CodecConfig {
    use isobmff::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
    transmux::CodecConfig::Avc {
        config: AVCConfigurationBox::new(AVCDecoderConfigurationRecord {
            configuration_version: 1,
            profile_indication: 0,
            profile_compatibility: 0,
            level_indication: 0,
            length_size_minus_one: 3,
            sps: Vec::new(),
            pps: Vec::new(),
            chroma_format: None,
            bit_depth_luma_minus8: None,
            bit_depth_chroma_minus8: None,
            sps_ext: Vec::new(),
        }),
        width: 0,
        height: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_common::stage::Timestamp;
    use transmux::ir::{CodecConfig, DataCarriage, DataIdentity, SampleFlags};
    use transmux::mux_stage::{MuxInput, MuxOutput};
    use transmux::{TrackId, TrackSpec};

    #[derive(Default)]
    struct Recorder {
        fed: Vec<(TrackId, Option<i64>)>,
        announced: usize,
        finished: bool,
        out: Vec<MuxOutput>,
    }

    impl Stage for Recorder {
        type In<'a> = MuxInput;
        type Out = MuxOutput;
        type Error = core::convert::Infallible;

        fn feed(&mut self, input: MuxInput, _now: Timestamp) -> Result<(), Self::Error> {
            match input {
                MuxInput::Track(_) => self.announced += 1,
                MuxInput::Sample { track, sample } => {
                    self.fed
                        .push((track, sample.timing.and_then(|timing| timing.dts())));
                    self.out.push(MuxOutput::Bytes(sample.data));
                }
                _ => {}
            }
            Ok(())
        }

        fn poll(&mut self) -> Option<MuxOutput> {
            self.out.pop()
        }

        fn finish(&mut self) -> Result<(), Self::Error> {
            self.finished = true;
            Ok(())
        }
    }

    fn timescale(value: u32) -> Timescale {
        Timescale::try_new(value).expect("non-zero")
    }

    fn data_spec(id: u32, scale: u32) -> TrackSpec {
        TrackSpec::new(
            TrackId::new(id),
            timescale(scale),
            CodecConfig::Data {
                identity: DataIdentity::Ts {
                    stream_type: 6,
                    descriptors: Vec::new(),
                    carriage: DataCarriage::Pes,
                },
            },
        )
    }

    fn track(id: u32, scale: u32, dts: &[i64]) -> CollectedTrack {
        let samples = dts
            .iter()
            .map(|&d| {
                let mut sample = Sample::timed(vec![0u8; 1], d, d, None);
                sample.flags = SampleFlags::SYNC;
                sample
            })
            .collect();
        CollectedTrack::new(data_spec(id, scale), samples)
    }

    #[test]
    fn drive_interleaves_by_decode_time_across_timescales() {
        let video = track(1, 90_000, &[0, 3_000, 6_000]);
        let audio = track(2, 48_000, &[0, 1_024, 2_048, 3_072]);
        let media = Collected::new(vec![video, audio], timescale(1_000));
        let mut stage = Recorder::default();
        let out = drive(&mut stage, &media, Timestamp::default()).expect("infallible");
        let v = TrackId::new(1);
        let a = TrackId::new(2);
        assert_eq!(stage.announced, 2);
        assert!(stage.finished);
        assert_eq!(out.len(), 7);
        assert_eq!(
            stage.fed,
            vec![
                (v, Some(0)),
                (a, Some(0)),
                (a, Some(1_024)),
                (v, Some(3_000)),
                (a, Some(2_048)),
                (a, Some(3_072)),
                (v, Some(6_000))
            ]
        );
    }

    /// A track's untimed head plays *before* its first timestamped sample, so
    /// it must sort there too. Sixteen 1024-tick head frames at 44.1 kHz span
    /// ~371 ms before the first timed sample at 64 243 ticks (~1.457 s), i.e.
    /// from ~1.086 s, which precedes the video's first sample at 1.4 s.
    #[test]
    fn an_untimed_head_sorts_before_the_first_timed_sample() {
        const HEAD: u32 = 16;
        const HEAD_DURATION: u32 = 1_024;
        const FIRST_TIMED: i64 = 64_243;
        let video = track(1, 90_000, &[126_000, 129_600]);
        let duration = std::num::NonZeroU32::new(HEAD_DURATION).expect("positive");
        let mut samples: Vec<Sample> = (0..HEAD)
            .map(|_| Sample::from_duration(vec![0u8; 1], duration, true).expect("valid sample"))
            .collect();
        let mut timed = Sample::timed(vec![0u8; 1], FIRST_TIMED, FIRST_TIMED, None);
        timed.flags = SampleFlags::SYNC;
        samples.push(timed);
        let audio = CollectedTrack::new_at(
            data_spec(2, 44_100),
            samples,
            u64::try_from(FIRST_TIMED).expect("non-negative"),
        );
        let media = Collected::new(vec![video, audio], timescale(1_000));
        let mut stage = Recorder::default();
        drive(&mut stage, &media, Timestamp::default()).expect("infallible");
        assert_eq!(
            stage.fed.first().copied(),
            Some((TrackId::new(2), None)),
            "the untimed audio head precedes the video that starts after it"
        );
        let video = stage
            .fed
            .iter()
            .position(|entry| *entry == (TrackId::new(1), Some(126_000)))
            .expect("video sample fed");
        let timed_audio = stage
            .fed
            .iter()
            .position(|entry| *entry == (TrackId::new(2), Some(FIRST_TIMED)))
            .expect("first timed audio sample fed");
        assert!(
            video < timed_audio,
            "the first timed audio sample still follows the video it plays after"
        );
    }
}

/// The collector equality test (Review Focus): `same` destructures every field
/// with no `..`, so adding a field to [`Collected`] / [`CollectedTrack`]
/// without extending it does not compile, and each field of a differing pair
/// must make the comparison unequal.
#[cfg(test)]
mod equality_tests {
    use super::*;
    use broadcast_core::timescale::Timescale;
    use mpeg_ts::pid::Pid;
    use mpeg_ts::ts::Pcr;
    use std::sync::Arc;
    use transmux::ir::{
        CodecConfig, DataCarriage, DataIdentity, DegradationRecord, DisplayInfo, EventProvenance,
        InputDegradation, Provenance, SampleFlags, SampleProtection, SampleTiming,
    };
    use transmux::{PcrSample, SkippedTrack, TrackId, TrackSpec};

    fn timescale(value: u32) -> Timescale {
        Timescale::try_new(value).expect("non-zero")
    }

    fn spec(id: u32) -> TrackSpec {
        TrackSpec::new(
            TrackId::new(id),
            timescale(1_000),
            CodecConfig::Data {
                identity: DataIdentity::Ts {
                    stream_type: 6,
                    descriptors: Vec::new(),
                    carriage: DataCarriage::Pes,
                },
            },
        )
    }

    fn sample() -> Sample {
        Sample::new(vec![1u8, 2, 3], Some(0), Some(0), Some(1), true).expect("valid timing")
    }

    fn base() -> Collected {
        let mut track = CollectedTrack::new(spec(1), vec![sample()]);
        track.start_decode_time = 90;
        let mut collected = Collected::new(vec![track], timescale(1_000));
        collected.pcr = vec![PcrSample::new(
            Pcr::from_parts(10_000, 0).expect("in range"),
            Pid::from_const(256),
            7,
            false,
        )];
        collected.skipped = vec![SkippedTrack::new(String::from("avc1"), String::from("why"))];
        collected.degraded = vec![InputDegradation::TransportError];
        collected.degraded_at = vec![DegradationRecord::new(
            InputDegradation::TransportError,
            Some(TrackId::new(1)),
            EventProvenance::default(),
        )];
        collected
    }

    fn same_sample(a: &Sample, b: &Sample) -> bool {
        a.data == b.data
            && a.timing == b.timing
            && a.flags == b.flags
            && a.provenance == b.provenance
            && a.display == b.display
            && a.protection == b.protection
    }

    fn same_track(a: &CollectedTrack, b: &CollectedTrack) -> bool {
        let CollectedTrack {
            spec: a_spec,
            samples: a_samples,
            start_decode_time: a_start,
        } = a;
        let CollectedTrack {
            spec: b_spec,
            samples: b_samples,
            start_decode_time: b_start,
        } = b;
        a_spec == b_spec
            && a_start == b_start
            && a_samples.len() == b_samples.len()
            && a_samples
                .iter()
                .zip(b_samples)
                .all(|(x, y)| same_sample(x, y))
    }

    fn same(a: &Collected, b: &Collected) -> bool {
        let Collected {
            tracks: a_tracks,
            movie_timescale: a_movie,
            pcr: a_pcr,
            skipped: a_skipped,
            degraded: a_degraded,
            degraded_at: a_degraded_at,
        } = a;
        let Collected {
            tracks: b_tracks,
            movie_timescale: b_movie,
            pcr: b_pcr,
            skipped: b_skipped,
            degraded: b_degraded,
            degraded_at: b_degraded_at,
        } = b;
        a_tracks.len() == b_tracks.len()
            && a_tracks.iter().zip(b_tracks).all(|(x, y)| same_track(x, y))
            && a_movie == b_movie
            && a_pcr == b_pcr
            && a_skipped == b_skipped
            && a_degraded == b_degraded
            && a_degraded_at == b_degraded_at
    }

    #[test]
    fn equal_values_compare_equal() {
        let base = base();
        assert!(same(&base, &base.clone()));
    }

    #[test]
    fn every_collected_field_changes_the_comparison() {
        let base = base();
        let mut m = base.clone();
        m.movie_timescale = timescale(1_001);
        assert!(!same(&base, &m), "movie_timescale");

        let mut m = base.clone();
        m.pcr[0].packet_index += 1;
        assert!(!same(&base, &m), "pcr");

        let mut m = base.clone();
        m.skipped[0].reason.push('!');
        assert!(!same(&base, &m), "skipped");

        let mut m = base.clone();
        m.degraded.clear();
        assert!(!same(&base, &m), "degraded");

        let mut m = base.clone();
        m.degraded_at[0].track = None;
        assert!(!same(&base, &m), "degraded_at");
    }

    #[test]
    fn every_track_and_sample_field_changes_the_comparison() {
        let base = base();
        let mut m = base.clone();
        m.tracks[0].start_decode_time += 1;
        assert!(!same(&base, &m), "start_decode_time");

        let mut m = base.clone();
        m.tracks[0].spec = spec(2);
        assert!(!same(&base, &m), "spec");

        let mut m = base.clone();
        m.tracks[0].samples.push(sample());
        assert!(!same(&base, &m), "sample count");

        let mut m = base.clone();
        m.tracks[0].samples[0].data = Sample::new(vec![9u8], Some(0), Some(0), Some(1), true)
            .expect("valid timing")
            .data;
        assert!(!same(&base, &m), "sample data");

        let mut m = base.clone();
        m.tracks[0].samples[0].timing = Some(SampleTiming::timed(1, 1));
        assert!(!same(&base, &m), "sample timing");

        let mut m = base.clone();
        m.tracks[0].samples[0].flags = SampleFlags::new(false);
        assert!(!same(&base, &m), "sample flags");

        let mut m = base.clone();
        m.tracks[0].samples[0].provenance = Some(Provenance::new(Some(1), None));
        assert!(!same(&base, &m), "sample provenance");

        let mut m = base.clone();
        m.tracks[0].samples[0].display = Some(Arc::new(DisplayInfo::new(None)));
        assert!(!same(&base, &m), "sample display");

        let mut m = base.clone();
        let iv = m.tracks[0].samples[0].data.clone();
        m.tracks[0].samples[0].protection = Some(Arc::new(SampleProtection {
            iv,
            subsamples: Vec::new(),
        }));
        assert!(!same(&base, &m), "sample protection");
    }
}
