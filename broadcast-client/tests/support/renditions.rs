use std::collections::HashMap;

use broadcast_client::{Event, Sample, TrackId, TrackSpec};
use transmux::CodecConfig;
use transmux::ts_hls::StreamingTsHlsSegmenter;

use super::hls_host::*;

pub const VIDEO_DIR: &str = "http://origin.test/hls/v";
pub const AUDIO_DIR: &str = "http://origin.test/hls/a";
pub const MASTER_URL: &str = "http://origin.test/hls/master.m3u8";
pub const VIDEO_PLAYLIST_URL: &str = "http://origin.test/hls/v/index.m3u8";
pub const AUDIO_PLAYLIST_URL: &str = "http://origin.test/hls/a/index.m3u8";
const TARGET_SECS: u32 = 2;
const WINDOW: usize = 64;

pub struct TrackStream {
    pub playlist: Vec<u8>,
    pub segments: Vec<(String, Vec<u8>)>,
    pub spec: TrackSpec,
    pub samples: Vec<Sample>,
}

pub struct Fixture {
    pub video: TrackStream,
    pub audio: TrackStream,
    pub audio_stream_track_id: TrackId,
}

fn is_video(spec: &TrackSpec) -> bool {
    matches!(
        spec.config,
        CodecConfig::Avc { .. } | CodecConfig::Hevc { .. } | CodecConfig::Vvc { .. }
    )
}

fn restream(spec: &TrackSpec, samples: &[Sample]) -> TrackStream {
    let mut seg =
        StreamingTsHlsSegmenter::new(vec![spec.clone()], TARGET_SECS, WINDOW).expect("segmenter");
    for s in samples {
        seg.push(spec.track_id, s.clone()).expect("push");
    }
    seg.finish().expect("finish");
    let segments = seg
        .take_ready()
        .into_iter()
        .map(|s| (s.uri, s.bytes))
        .collect();
    TrackStream {
        playlist: seg.playlist().expect("playlist").into_bytes(),
        segments,
        spec: spec.clone(),
        samples: samples.to_vec(),
    }
}

pub fn fixture() -> Fixture {
    let events = oracle_events(TS_PLAYLIST_URL, &ts_bodies());
    let mut specs: Vec<TrackSpec> = Vec::new();
    let mut per_track: HashMap<TrackId, Vec<Sample>> = HashMap::new();
    for e in events {
        match e {
            Event::Tracks(t) => specs = t,
            Event::Sample { track, sample, .. } => per_track.entry(track).or_default().push(sample),
            _ => {}
        }
    }
    let video = specs.iter().find(|s| is_video(s)).expect("video track");
    let audio = specs.iter().find(|s| !is_video(s)).expect("audio track");
    let video = restream(video, &per_track[&video.track_id]);
    let audio = restream(audio, &per_track[&audio.track_id]);
    let audio_stream_track_id = oracle_events(
        AUDIO_PLAYLIST_URL,
        &audio.bodies(AUDIO_DIR, AUDIO_PLAYLIST_URL),
    )
    .into_iter()
    .find_map(|e| match e {
        Event::Tracks(t) => t.first().map(|s| s.track_id),
        _ => None,
    })
    .expect("audio-only stream tracks");
    Fixture {
        video,
        audio,
        audio_stream_track_id,
    }
}

impl TrackStream {
    pub fn bodies(&self, dir: &str, playlist_url: &str) -> HashMap<String, Vec<u8>> {
        let mut m = HashMap::new();
        m.insert(playlist_url.to_string(), self.playlist.clone());
        for (uri, bytes) in &self.segments {
            m.insert(format!("{dir}/{uri}"), bytes.clone());
        }
        m
    }
}

pub fn samples_of(events: &[Event], pick: impl Fn(TrackId) -> bool) -> Vec<Sample> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Sample { track, sample, .. } if pick(*track) => Some(sample.clone()),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
#[test]
#[ignore]
fn _touch_samples_of() {
    let _ = samples_of(&[] as &[Event], |_| true);
}

// SENTINEL-START
#[used]
static _USE_ALL_ITEMS: usize = {
    let _ = 0usize;
    let _ = is_video;
    let _ = restream;
    let _ = fixture;
    let _ = VIDEO_DIR;
    let _ = AUDIO_DIR;
    let _ = MASTER_URL;
    let _ = VIDEO_PLAYLIST_URL;
    let _ = AUDIO_PLAYLIST_URL;
    let _ = TARGET_SECS;
    let _ = WINDOW;
    let _ = std::mem::size_of::<TrackStream>();
    let _ = std::mem::size_of::<Fixture>();
    let _ = std::mem::offset_of!(TrackStream, playlist);
    let _ = std::mem::offset_of!(TrackStream, segments);
    let _ = std::mem::offset_of!(TrackStream, spec);
    let _ = std::mem::offset_of!(TrackStream, samples);
    let _ = std::mem::offset_of!(Fixture, video);
    let _ = std::mem::offset_of!(Fixture, audio);
    let _ = std::mem::offset_of!(Fixture, audio_stream_track_id);
    let _ = TrackStream::bodies;
    0
};
// SENTINEL-END
