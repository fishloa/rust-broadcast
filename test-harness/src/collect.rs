//! Collect a streaming demux stage's [`DemuxEvent`]s into the per-track vectors
//! a [`Media`] carries.
//!
//! Until the whole-input MP4 demuxers are deleted (streaming-first Phase 4),
//! the tests and the CLI still need a `Media`. This module is the one place
//! that folds the event vocabulary into it, so the differential tests compare
//! the streaming stage against the old demuxers on the same terms.

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
