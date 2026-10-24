mod support;

use broadcast_client::{ErrorKind, Event, Status};
use support::fmp4_split::{drop_track, group, split};
use support::hls_host::{Host, hls_config};

const BASE: &str = "http://origin.test/v";
const PLAYLIST_URL: &str = "http://origin.test/v/index.m3u8";
const SEGMENTS: usize = 3;
const ENCRYPTED_VIDEO_TRACK: u32 = 1;
const AUDIO_TRACK: u32 = 2;
const AUDIO_SAMPLES: usize = 88;

fn vod_playlist(segments: usize) -> Vec<u8> {
    let mut p = String::from(
        "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:10\n#EXT-X-PLAYLIST-TYPE:VOD\n\
         #EXT-X-MAP:URI=\"init.mp4\"\n",
    );
    for i in 0..segments {
        p.push_str(&format!("#EXTINF:1.0,\ns{i}.m4s\n"));
    }
    p.push_str("#EXT-X-ENDLIST\n");
    p.into_bytes()
}

fn play_fixture(fixture: &str) -> Vec<Event> {
    play_split(fixture, None)
}

fn play_split(fixture: &str, without_track: Option<u32>) -> Vec<Event> {
    let bytes = test_fixtures::read(fixture);
    let mut parts = split(&bytes).expect("fragmented fixture");
    if let Some(track) = without_track {
        parts.init = drop_track(&parts.init, track);
    }
    let segments = group(&parts.segments, SEGMENTS);
    let mut h = Host::new(PLAYLIST_URL, hls_config());
    h.ok(PLAYLIST_URL, vod_playlist(segments.len()));
    h.ok(&format!("{BASE}/init.mp4"), parts.init);
    for (i, s) in segments.into_iter().enumerate() {
        h.ok(&format!("{BASE}/s{i}.m4s"), s);
    }
    h.run_to_end();
    h.events
}

fn errors(events: &[Event]) -> Vec<(ErrorKind, bool)> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Error { kind, fatal } => Some((kind.clone(), *fatal)),
            _ => None,
        })
        .collect()
}

fn samples_of(events: &[Event], track: u32) -> usize {
    events
        .iter()
        .filter(|e| matches!(e, Event::Sample { track: t, .. } if t.value() == track))
        .count()
}

fn ended(events: &[Event]) -> bool {
    events
        .iter()
        .any(|e| matches!(e, Event::Status(Status::Ended)))
}

fn encrypted_track_is_reported_once_while_audio_plays(fixture: &str) {
    let events = play_fixture(fixture);
    let errs = errors(&events);
    let unsupported: Vec<_> = errs
        .iter()
        .filter(|(k, _)| matches!(k, ErrorKind::UnsupportedEncryption { .. }))
        .collect();
    assert_eq!(unsupported.len(), 1, "{errs:?}");
    assert!(
        !unsupported[0].1,
        "a clear audio track still plays: non-fatal"
    );
    assert_eq!(samples_of(&events, ENCRYPTED_VIDEO_TRACK), 0);
    assert_eq!(samples_of(&events, AUDIO_TRACK), AUDIO_SAMPLES);
    assert!(ended(&events));
}

#[test]
fn a_cenc_video_track_in_a_plain_playlist_is_reported_not_dropped_silently() {
    encrypted_track_is_reported_once_while_audio_plays("transmux/h264_cenc.mp4");
}

#[test]
fn a_cbcs_video_track_in_a_plain_playlist_is_reported_not_dropped_silently() {
    encrypted_track_is_reported_once_while_audio_plays("transmux/h264_cbcs.mp4");
}

#[test]
fn a_stream_whose_every_track_is_skipped_ends_fatal_not_ended() {
    let events = play_split("transmux/h264_cenc.mp4", Some(AUDIO_TRACK));
    let errs = errors(&events);
    assert!(
        errs.iter()
            .any(|(k, fatal)| *fatal && matches!(k, ErrorKind::UnsupportedEncryption { .. })),
        "{errs:?}"
    );
    assert!(!ended(&events), "must not end as a clean empty stream");
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, Event::Sample { .. }))
            .count(),
        0
    );
}
