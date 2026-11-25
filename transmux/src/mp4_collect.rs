//! In-crate whole-image collector over [`Mp4Demux`](crate::mp4_demux::Mp4Demux).
//!
//! The muxers still take a [`Media`], so the CLI, the repackager and the
//! in-crate round-trip tests drive the streaming stage over an in-memory image
//! and fold its events into one. This is the twin of
//! `test_harness::collect::media_from_mp4` for code inside this crate; it
//! exists until the muxers consume samples directly (streaming-first Phase 4).

use alloc::collections::BTreeMap;
#[cfg(test)]
use alloc::string::String;
use alloc::vec::Vec;

use broadcast_common::Stage;
use broadcast_common::stage::Timestamp;
use broadcast_common::timescale::Timescale;

use crate::error::Result;
#[cfg(test)]
use crate::ir::{AbandonReason, SkippedTrack};
use crate::ir::{DemuxEvent, Media, Track, TrackId};
use crate::mp4_demux::{Mp4Demux, Mp4DemuxConfig, Mp4Input};
#[cfg(all(feature = "cli", feature = "cenc"))]
use crate::mux_stage::MuxInput;

/// Movie timescale used when the image names none.
const DEFAULT_MOVIE_TIMESCALE: core::num::NonZeroU32 = match core::num::NonZeroU32::new(1_000) {
    Some(value) => value,
    None => core::num::NonZeroU32::MIN,
};

/// Demux a whole in-memory MP4 image through a seekable [`Mp4Demux`] and fold
/// its events into a [`Media`].
#[cfg(test)]
pub(crate) fn media_from_mp4(bytes: &[u8]) -> Result<Media> {
    let total_len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    let mut demux = Mp4Demux::seekable(Mp4DemuxConfig::default().with_total_len(Some(total_len)));
    drive_seekable(&mut demux, bytes)?;
    media_from_stage(demux)
}

#[cfg(test)]
fn drive_seekable(demux: &mut Mp4Demux, bytes: &[u8]) -> Result<()> {
    loop {
        let demand = demux.demand();
        let Some(offset) = demand.offset else {
            break;
        };
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        let end = start.saturating_add(demand.want_bytes).min(bytes.len());
        let slice = bytes.get(start..end).unwrap_or(&[]);
        demux.feed(
            Mp4Input::At {
                offset,
                bytes: slice.to_vec().into(),
            },
            Timestamp::ZERO,
        )?;
    }
    demux.finish()
}

/// Demux a whole in-memory MP4 image and pass every track announcement and
/// sample through `stage` before folding the result into a [`Media`].
///
/// This is the whole-image bridge for a per-sample stage (a CENC encrypt or
/// decrypt stage); the muxer stages use
/// [`drive_media`](crate::mux_stage::drive_media) instead.
#[cfg(all(feature = "cli", feature = "cenc"))]
pub(crate) fn media_through_stage<S>(bytes: &[u8], stage: &mut S) -> Result<Media>
where
    S: for<'a> Stage<In<'a> = MuxInput, Out = MuxInput, Error = crate::Error>,
{
    let total_len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    let mut demux = Mp4Demux::seekable(Mp4DemuxConfig::default().with_total_len(Some(total_len)));
    drive_seekable_through(&mut demux, stage, bytes)?;
    let fragmented = demux.is_fragmented();
    let movie_timescale = demux.movie_timescale();
    let init_order: Vec<TrackId> = demux.init_tracks().map(|spec| spec.track_id).collect();
    let mut inputs = Vec::new();
    while let Some(event) = demux.poll() {
        match event {
            DemuxEvent::TrackAdded(spec) | DemuxEvent::TrackUpdated(spec) => {
                stage.feed(
                    MuxInput::Track(alloc::boxed::Box::new(spec)),
                    Timestamp::ZERO,
                )?;
            }
            DemuxEvent::Sample {
                track_id, sample, ..
            } => {
                stage.feed(
                    MuxInput::Sample {
                        track: track_id,
                        sample,
                    },
                    Timestamp::ZERO,
                )?;
            }
            _ => {}
        }
        while let Some(item) = stage.poll() {
            inputs.push(item);
        }
    }
    stage.finish()?;
    while let Some(item) = stage.poll() {
        inputs.push(item);
    }
    let mut media = media_from_inputs(inputs);
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

#[cfg(all(feature = "cli", feature = "cenc"))]
fn drive_seekable_through<S>(demux: &mut Mp4Demux, stage: &mut S, bytes: &[u8]) -> Result<()>
where
    S: for<'a> Stage<In<'a> = MuxInput, Out = MuxInput, Error = crate::Error>,
{
    loop {
        while let Some(event) = demux.poll() {
            match event {
                DemuxEvent::TrackAdded(spec) | DemuxEvent::TrackUpdated(spec) => {
                    stage.feed(
                        MuxInput::Track(alloc::boxed::Box::new(spec)),
                        Timestamp::ZERO,
                    )?;
                }
                DemuxEvent::Sample {
                    track_id, sample, ..
                } => {
                    stage.feed(
                        MuxInput::Sample {
                            track: track_id,
                            sample,
                        },
                        Timestamp::ZERO,
                    )?;
                }
                _ => {}
            }
        }
        let demand = demux.demand();
        let Some(offset) = demand.offset else {
            break;
        };
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        let end = start.saturating_add(demand.want_bytes).min(bytes.len());
        let slice = bytes.get(start..end).unwrap_or(&[]);
        demux.feed(
            Mp4Input::At {
                offset,
                bytes: slice.to_vec().into(),
            },
            Timestamp::ZERO,
        )?;
    }
    demux.finish()
}

#[cfg(all(feature = "cli", feature = "cenc"))]
fn media_from_inputs(inputs: Vec<MuxInput>) -> Media {
    let mut tracks: Vec<Track> = Vec::new();
    let mut index_by_id: BTreeMap<TrackId, usize> = BTreeMap::new();
    for input in inputs {
        match input {
            MuxInput::Track(spec) => {
                if let Some(&index) = index_by_id.get(&spec.track_id) {
                    tracks[index].spec = *spec;
                } else {
                    index_by_id.insert(spec.track_id, tracks.len());
                    tracks.push(Track::new(*spec, Vec::new()));
                }
            }
            MuxInput::Sample { track, sample } => {
                if let Some(&index) = index_by_id.get(&track) {
                    let track = &mut tracks[index];
                    if track.samples.is_empty()
                        && let Some(dts) = sample.timing.and_then(|timing| timing.dts())
                    {
                        track.start_decode_time = u64::try_from(dts).unwrap_or(0);
                    }
                    track.samples.push(sample);
                }
            }
            _ => {}
        }
    }
    Media::new(tracks, Timescale::from_nonzero(DEFAULT_MOVIE_TIMESCALE))
}

#[cfg(test)]
fn media_from_stage(mut demux: Mp4Demux) -> Result<Media> {
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

/// Fold a demux event stream into a [`Media`].
#[cfg(test)]
fn media_from_events(events: Vec<DemuxEvent>) -> Media {
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
    let mut media = Media::new(tracks, Timescale::from_nonzero(DEFAULT_MOVIE_TIMESCALE));
    media.skipped = skipped;
    media.degraded = degraded;
    media
}

#[cfg(test)]
fn skipped_from_reason(reason: AbandonReason, track_id: Option<TrackId>) -> SkippedTrack {
    let (fourcc, text) = match reason {
        AbandonReason::UnsupportedSampleEntry { fourcc, detail } => {
            let fourcc = String::from_utf8_lossy(&fourcc).into_owned();
            let text = match detail {
                Some(detail) => alloc::format!("sample entry '{fourcc}' rejected: {detail}"),
                None => alloc::format!(
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
            alloc::format!("unexpected box: expected {expected}"),
        ),
        other => (String::from("unknown"), other.name().into()),
    };
    let mut skipped = SkippedTrack::new(fourcc, text);
    skipped.track_id = track_id;
    skipped
}
