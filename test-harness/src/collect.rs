//! Collect a streaming demux stage's [`DemuxEvent`]s into the per-track vectors
//! a [`Media`] carries.
//!
//! A test that asserts on a whole `Media` drives the one streaming MP4 stage
//! over a fixture and folds its event vocabulary into one here; the other
//! whole-`Media` bridges in this module do the same for the mux, splice and
//! trick-play stages. The fold keeps the init's track order and its
//! sample-owned summaries (`start_decode_time`, skipped and degraded
//! records), so a test can compare what the demuxer reported on those terms.

use std::collections::BTreeMap;

use broadcast_common::Stage;
use broadcast_common::stage::Timestamp;
use transmux::{
    AbandonReason, AfdPersistence, DegradationRecord, DemuxEvent, InputDegradation, Media,
    PcrSample, Sample, SkippedTrack, SplicePlan, SplicePoint, SpliceStage, StreamingFlvDemux,
    StreamingTsDemux, Track, TrackId, TrickPlayStage,
};

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

/// Fold a demux event stream into a [`Media`].
///
/// `start_decode_time` is the track's first sample's `dts` (an MP4 decode
/// timeline starts at the anchor its first sample names, which is `0` for a
/// progressive file). `skipped` is filled from `TrackAbandoned`, `degraded`
/// from `InputDegraded`.
pub fn media_from_events(events: Vec<DemuxEvent>) -> Media {
    let mut tracks: Vec<Track> = Vec::new();
    let mut index_by_id: BTreeMap<TrackId, usize> = BTreeMap::new();
    let mut skipped = Vec::new();
    let mut degraded = Vec::new();
    for event in events {
        match event {
            DemuxEvent::TrackAdded(spec) => {
                index_by_id.insert(spec.track_id, tracks.len());
                tracks.push(Track::new(spec, Vec::new()));
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
    let mut media = Media::new(
        tracks,
        broadcast_common::timescale::Timescale::from_nonzero(DEFAULT_MOVIE_TIMESCALE),
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
/// MP4 image and fold its events into a [`Media`].
///
/// A whole small fixture in memory is fine **here only**: this crate is a test
/// harness. The stage itself never holds more than the reader asks for.
pub fn media_from_mp4(bytes: &[u8], mode: CollectMode) -> Result<Media, transmux::Error> {
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

fn media_from_stage(mut demux: transmux::mp4_demux::Mp4Demux) -> Result<Media, transmux::Error> {
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
/// its events into a [`Media`].
///
/// A whole small fixture in memory is fine **here only**: this crate is a test
/// harness. The stage itself never holds the input.
pub fn media_from_ps(bytes: &[u8], chunk: usize) -> Result<Media, transmux::Error> {
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
/// feed and fold its events into a [`Media`], exactly as the deleted
/// whole-input `TsDemux::demux` did: strict PMT track order, the program
/// `ClockReference`s as [`PcrSample`]s, and every damage record resolved
/// against the final track set by source PID.
///
/// A whole small fixture in memory is fine **here only**: this crate is a test
/// harness. The stage itself never holds the input.
pub fn media_from_ts(bytes: &[u8]) -> Result<Media, transmux::Error> {
    media_from_ts_chunked(bytes, WHOLE_INPUT)
}

/// [`media_from_ts`] fed in `chunk`-sized pieces (the streaming-gate form).
pub fn media_from_ts_chunked(bytes: &[u8], chunk: usize) -> Result<Media, transmux::Error> {
    media_from_ts_with_afd(bytes, chunk, AfdPersistence::UntilReplaced)
}

/// [`media_from_ts_chunked`] with an explicit AFD persistence policy.
pub fn media_from_ts_with_afd(
    bytes: &[u8],
    chunk: usize,
    afd: AfdPersistence,
) -> Result<Media, transmux::Error> {
    let mut demux = StreamingTsDemux::new()
        .with_afd_persistence(afd)
        .with_strict_pmt_order();
    for piece in bytes.chunks(chunk.max(1)) {
        demux.feed(piece)?;
    }
    demux.finish()?;

    let mut tracks: Vec<Track> = Vec::new();
    let mut index_by_id: BTreeMap<TrackId, usize> = BTreeMap::new();
    let mut pcr: Vec<PcrSample> = Vec::new();
    let mut degraded: Vec<InputDegradation> = Vec::new();
    let mut degraded_at: Vec<DegradationRecord> = Vec::new();
    while let Some(event) = demux.poll_event() {
        match event {
            DemuxEvent::TrackAdded(spec) => {
                index_by_id.insert(spec.track_id, tracks.len());
                tracks.push(Track::new(spec, Vec::new()));
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
    let mut media = Media::new(tracks, transmux::rtp::VIDEO_CLOCK_TIMESCALE).with_pcr(pcr);
    media.degraded = degraded;
    media.degraded_at = degraded_at;
    Ok(media)
}

/// Drive [`StreamingFlvDemux`] over a whole in-memory FLV image and fold its
/// events into a [`Media`] on FLV's 1 kHz clock.
///
/// A whole small fixture in memory is fine **here only**: this crate is a test
/// harness. The stage itself never holds the input.
///
/// # Errors
///
/// The first structural error the stage reports (bad signature, implausible
/// header, corrupt codec config).
pub fn media_from_flv(bytes: &[u8]) -> Result<Media, transmux::flv::FlvError> {
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
) -> Result<Media, transmux::flv::FlvError> {
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
/// post-handshake RTMP chunk stream, folding the result into a [`Media`].
///
/// The reassembly half is exactly what the deleted whole-input `RtmpDemux` did:
/// collect the Audio (type 8), Video (type 9) and `onMetaData` (type 18)
/// message bodies, rebuild an FLV byte stream from them, and demux that.
///
/// # Errors
///
/// An RTMP wire error, a rebuilt tag body too large for FLV's UI24
/// `DataSize`, or the FLV stage's own error.
pub fn media_from_rtmp(bytes: &[u8]) -> Result<Media, transmux::RtmpFlvError> {
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
) -> Result<Media, transmux::RtmpFlvError> {
    use broadcast_common::Serialize as _;
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
/// and the joins to signal. Field-compatible with the former whole-`Media`
/// splice result, now produced by [`SpliceStage`].
pub struct SpliceOutput {
    /// The joined media on a single monotonic timeline.
    pub media: Media,
    /// The join points to signal as `#EXT-X-DISCONTINUITY`.
    pub discontinuity_points: Vec<SplicePoint>,
}

/// Build the per-track splice index [`SplicePlan`] reads: identity plus each
/// sample's timing and sync flag, never its coded bytes.
pub fn splice_index(tracks: &[Track]) -> transmux::SpliceIndex {
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
pub fn concat(a: &Media, b: &Media) -> Result<SpliceOutput, transmux::Error> {
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
    base: &Media,
    ad: &Media,
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
    base: &Media,
    inserted: Option<&Media>,
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
        tracks.push(Track::new_at(track.spec.clone(), samples, start));
    }
    Ok(SpliceOutput {
        media: Media::new(tracks, plan.movie_timescale()),
        discontinuity_points: plan.discontinuity_points().to_vec(),
    })
}

/// Derive an I-frame-only track from `src` through [`TrickPlayStage`].
///
/// # Errors
/// [`transmux::Error::InvalidInput`] when `src` has no sync samples.
pub fn derive_iframe_track(src: &Track) -> Result<Track, transmux::Error> {
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
    Ok(Track::new_at(spec, samples, start))
}

/// Append a track derived by [`derive_iframe_track`] at `index`.
///
/// # Errors
/// [`transmux::Error::InvalidInput`] when `index` is out of range or the
/// source has no sync samples.
pub fn append_iframe_track(media: &mut Media, index: usize) -> Result<(), transmux::Error> {
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

/// A summary log for a whole [`Media`] as one segment per track: the bridge
/// for a caller that still holds the samples.
pub fn segment_log(media: &transmux::Media) -> transmux::segment_log::SegmentSummaryLog {
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

/// The whole-input helper methods tests use on a collected [`Media`]: track
/// selection, a presentation-time window, and the anchor duration. These were
/// `transmux` production functions until Phase 6 slice 3a; they live here (a
/// `publish = false` test harness) so production code cannot hold a whole
/// input.
pub trait MediaSelectExt {
    /// Select tracks by index, preserving the given order.
    fn select_tracks(&self, indices: &[usize]) -> Result<Media, transmux::Error>;

    /// Select tracks by predicate, preserving source order.
    fn select_tracks_by<F>(&self, keep: F) -> Result<Media, transmux::Error>
    where
        F: FnMut(&Track) -> bool;

    /// Trim to the half-open presentation-time window `[start, end)` in the
    /// movie timescale, snapping the anchor back to its preceding sync sample.
    fn trim(&self, start: u64, end: u64) -> Result<Media, transmux::Error>;

    /// Total duration of the segmentation anchor track, in its timescale.
    fn anchor_duration(&self) -> Option<(u64, u32)>;
}

impl MediaSelectExt for Media {
    fn select_tracks(&self, indices: &[usize]) -> Result<Media, transmux::Error> {
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
        Ok(Media::new(tracks, self.movie_timescale))
    }

    fn select_tracks_by<F>(&self, mut keep: F) -> Result<Media, transmux::Error>
    where
        F: FnMut(&Track) -> bool,
    {
        let tracks: Vec<Track> = self.tracks.iter().filter(|t| keep(t)).cloned().collect();
        if tracks.is_empty() {
            return Err(transmux::Error::InvalidInput(
                "select_tracks_by: predicate kept no track",
            ));
        }
        Ok(Media::new(tracks, self.movie_timescale))
    }

    fn trim(&self, start: u64, end: u64) -> Result<Media, transmux::Error> {
        use broadcast_common::timescale::{Rounding, rescale_i64};
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
            out_tracks.push(Track::new_at(track.spec.clone(), kept, first_dts));
        }
        if !kept_any {
            return Err(transmux::Error::InvalidInput(
                "trim: window selected no samples on any track",
            ));
        }
        Ok(Media::new(out_tracks, self.movie_timescale))
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
fn origin_of(tracks: &[Track]) -> (i64, broadcast_common::timescale::Timescale) {
    use broadcast_common::timescale::{Timescale, cmp_ticks};
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
    track: &Track,
    origin: (i64, broadcast_common::timescale::Timescale),
) -> Vec<i64> {
    use broadcast_common::timescale::{Rounding, rescale_i64};
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
    from_timescale: broadcast_common::timescale::Timescale,
    to_timescale: broadcast_common::timescale::Timescale,
) -> i64 {
    use broadcast_common::timescale::{Rounding, rescale_i64};
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

/// Build a whole CMAF output (init + media segments) from a collected [`Media`]
/// by driving the one [`Segmenter`](transmux::Segmenter) stage — the test-only
/// replacement for the deleted `Repackage::run_media`.
pub fn cmaf_from_media(
    media: &Media,
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
    for output in transmux::mux_stage::drive_media(&mut adapter, media, Timestamp::ZERO)? {
        if let transmux::mux_stage::MuxOutput::Segment(segment) = output {
            media_segments.push(segment.bytes.to_vec());
        }
    }
    Ok(transmux::RepackageOutput {
        init_segment,
        media_segments,
    })
}

/// Drops the `Track` announcements [`drive_media`](transmux::mux_stage::drive_media)
/// sends, since a [`Segmenter`](transmux::Segmenter) is built with its track set.
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

/// Record a progressive sample plan from a collected [`Media`] — the test-only
/// replacement for the deleted `PlanPass::for_media`.
pub fn plan_from_media(media: &Media) -> Result<isobmff::plan::OwnedPlan, transmux::Error> {
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
/// whole in-memory Matroska/WebM image and fold its events into a [`Media`].
///
/// A whole small fixture in memory is fine **here only**: this crate is a test
/// harness. The stage itself never holds the input.
pub fn media_from_matroska(bytes: &[u8], mode: CollectMode) -> Result<Media, transmux::Error> {
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
    ) -> Result<broadcast_common::timescale::Timescale, transmux::Error> {
        use transmux::Error;
        match &self.config {
            Some(transmux::CodecConfig::Aac { sample_rate, .. }) => {
                if self.kind != transmux::RtpMediaKind::Aac {
                    return Err(Error::InvalidInput(
                        "an RTP stream carrying an AAC config must have kind Aac",
                    ));
                }
                if self.clock_rate != *sample_rate {
                    return Err(Error::InvalidValue(broadcast_common::error::InvalidValue {
                        field: "rtp_clock_rate",
                        value: u64::from(self.clock_rate),
                        reason: "an MPEG-4 audio stream's RTP clock rate is its sampling rate",
                    }));
                }
                Ok(broadcast_common::timescale::Timescale::try_new(
                    *sample_rate,
                )?)
            }
            Some(transmux::CodecConfig::Avc { .. }) => {
                if self.kind != transmux::RtpMediaKind::H264 {
                    return Err(Error::InvalidInput(
                        "an RTP stream carrying an AVC config must have kind H264",
                    ));
                }
                if self.clock_rate != transmux::rtp::VIDEO_CLOCK_RATE {
                    return Err(Error::InvalidValue(broadcast_common::error::InvalidValue {
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
                Ok(broadcast_common::timescale::Timescale::try_new(
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
/// over one whole packet flow per stream and fold the samples into a [`Media`].
///
/// # Errors
///
/// An AAC stream without its `AudioSpecificConfig`, an invalid clock rate, or
/// whatever the depacketiser reports.
pub fn media_from_rtp(input: RtpTestInput) -> Result<Media, transmux::Error> {
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
) -> Result<(Media, Vec<transmux::RtpLossEvent>), transmux::Error> {
    let streams = input.streams;
    use transmux::rtp_stream::{RtpStreamDepacketiser, RtpStreamTrack};
    let mut tracks = Vec::with_capacity(streams.len());
    for (index, stream) in streams.iter().enumerate() {
        let track_id = TrackId::new(u32::try_from(index).unwrap_or(u32::MAX).saturating_add(1));
        let clock = stream.validated_clock_rate()?;
        let config = match (&stream.config, stream.kind) {
            (Some(config), _) => config.clone(),
            (None, transmux::RtpMediaKind::H264) => rtp_placeholder_avc_config(),
            (None, transmux::RtpMediaKind::Aac) => {
                return Err(transmux::Error::InvalidInput(
                    "an AAC RTP stream needs its AudioSpecificConfig",
                ));
            }
            (None, _) => {
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
    // `Media` at its lowest decode instant (the IR's `start_decode_time` is a
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
            Track::new_at(spec, samples, anchor)
        })
        .collect();
    Ok((
        Media::new(built, transmux::rtp::VIDEO_CLOCK_TIMESCALE),
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
