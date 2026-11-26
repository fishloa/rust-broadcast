//! Drive a per-sample CENC stage over a whole `Media` in tests.
//!
//! The test harness's bridge (`drive`) is for muxer stages whose output
//! is `MuxOutput`; a CENC stage's output is another `MuxInput` stream, so this
//! is the test-side twin that rebuilds the transformed `Media`.

use broadcast_common::stage::{Stage, Timestamp};
use test_harness::collect::{Collected as Media, CollectedTrack as Track};
use transmux::TrackId;
use transmux::mux_stage::MuxInput;

/// Feed every track then every sample of `media` through `stage`, rebuilding
/// the resulting `Media` from its output.
pub fn run<S>(stage: &mut S, media: &Media) -> Result<Media, S::Error>
where
    S: for<'a> Stage<In<'a> = MuxInput, Out = MuxInput, Error = transmux::Error>,
{
    for track in &media.tracks {
        stage.feed(
            MuxInput::Track(Box::new(track.spec.clone())),
            Timestamp::ZERO,
        )?;
    }
    let mut order: Vec<(usize, TrackId)> = media
        .tracks
        .iter()
        .enumerate()
        .map(|(i, t)| (i, t.spec.track_id))
        .collect();
    order.sort_by_key(|&(_, id)| id);
    for (index, _) in &order {
        let track = &media.tracks[*index];
        for sample in &track.samples {
            stage.feed(
                MuxInput::Sample {
                    track: track.spec.track_id,
                    sample: sample.clone(),
                },
                Timestamp::ZERO,
            )?;
        }
    }
    let mut tracks: Vec<Track> = Vec::new();
    let mut index: std::collections::BTreeMap<TrackId, usize> = std::collections::BTreeMap::new();
    while let Some(item) = stage.poll() {
        match item {
            MuxInput::Track(spec) => {
                index.insert(spec.track_id, tracks.len());
                tracks.push(Track::new(*spec, Vec::new()));
            }
            MuxInput::Sample { track, sample } => {
                if let Some(&at) = index.get(&track) {
                    tracks[at].samples.push(sample);
                }
            }
            _ => {}
        }
    }
    Ok(Media::new(tracks, media.movie_timescale))
}
