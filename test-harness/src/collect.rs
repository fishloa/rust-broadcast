//! Collect a streaming demux stage's [`DemuxEvent`]s into the per-track vectors
//! a [`Media`] carries.
//!
//! The muxers still take a `Media` (streaming-first Phase 4 will change them),
//! so the tests drive the one streaming MP4 stage over a fixture and fold its
//! event vocabulary into one here. The fold keeps the init's track order and
//! its sample-owned summaries (`start_decode_time`, skipped and degraded
//! records), so a test compares exactly what the old whole-input demuxers
//! reported on the same terms.

use std::collections::BTreeMap;

use broadcast_common::Stage;
use broadcast_common::stage::Timestamp;
use transmux::{
    AbandonReason, DemuxEvent, Media, Sample, SkippedTrack, SplicePlan, SplicePoint, SpliceStage,
    Track, TrackId, TrickPlayStage,
};

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

/// The result of driving a [`SplicePlan`] over whole medias: the joined media
/// and the joins to signal. Field-compatible with the former whole-`Media`
/// splice result, now produced by [`SpliceStage`].
pub struct SpliceOutput {
    /// The joined media on a single monotonic timeline.
    pub media: Media,
    /// The join points to signal as `#EXT-X-DISCONTINUITY`.
    pub discontinuity_points: Vec<SplicePoint>,
}

/// Append `b` after `a` through [`SpliceStage`].
///
/// # Errors
/// Whatever [`SplicePlan::concat`] or the stage returns.
pub fn concat(a: &Media, b: &Media) -> Result<SpliceOutput, transmux::Error> {
    let plan = SplicePlan::concat(a, b)?;
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
    let plan = SplicePlan::insert(base, ad, at_ticks)?;
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

/// The track-selection helpers [`Media`] used to carry, as a test-only
/// extension so existing tests keep their method-call shape while the
/// production functions live in `transmux::repackage`.
pub trait MediaSelectExt {
    /// Select tracks by index; see [`transmux::repackage::select_tracks`].
    fn select_tracks(&self, indices: &[usize]) -> Result<Media, transmux::Error>;

    /// Select tracks by predicate; see
    /// [`transmux::repackage::select_tracks_by`].
    fn select_tracks_by<F>(&self, keep: F) -> Result<Media, transmux::Error>
    where
        F: FnMut(&Track) -> bool;

    /// Trim to a presentation-time window; see [`transmux::repackage::trim`].
    fn trim(&self, start: u64, end: u64) -> Result<Media, transmux::Error>;
}

impl MediaSelectExt for Media {
    fn select_tracks(&self, indices: &[usize]) -> Result<Media, transmux::Error> {
        transmux::repackage::select_tracks(self, indices)
    }

    fn select_tracks_by<F>(&self, keep: F) -> Result<Media, transmux::Error>
    where
        F: FnMut(&Track) -> bool,
    {
        transmux::repackage::select_tracks_by(self, keep)
    }

    fn trim(&self, start: u64, end: u64) -> Result<Media, transmux::Error> {
        transmux::repackage::trim(self, start, end)
    }
}
