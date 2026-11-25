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
use transmux::{AbandonReason, DemuxEvent, Media, SkippedTrack, Track, TrackId};

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
        AbandonReason::UnsupportedSampleEntry { fourcc } => {
            let fourcc = String::from_utf8_lossy(&fourcc).into_owned();
            let text =
                format!("sample entry '{fourcc}' has no CodecConfig reconstruction in this crate");
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
