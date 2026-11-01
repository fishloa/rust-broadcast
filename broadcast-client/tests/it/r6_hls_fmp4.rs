use crate::support;

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

fn oracle_counts(bytes: &[u8]) -> std::collections::BTreeMap<u32, usize> {
    use broadcast_common::Unpackage;
    let media = transmux::Fmp4Demux::new().unpackage(bytes).unwrap();
    media
        .tracks
        .iter()
        .map(|t| (t.spec.track_id.value(), t.samples.len()))
        .collect()
}

fn split_counts(events: &[Event]) -> (Vec<Vec<u32>>, [std::collections::BTreeMap<u32, usize>; 2]) {
    let mut tracks = Vec::new();
    let mut counts = [
        std::collections::BTreeMap::new(),
        std::collections::BTreeMap::new(),
    ];
    let mut side = 0;
    for e in events {
        match e {
            Event::Tracks(t) => tracks.push(t.iter().map(|s| s.track_id.value()).collect()),
            Event::Discontinuity => side = 1,
            Event::Sample { track, .. } => *counts[side].entry(track.value()).or_default() += 1,
            _ => {}
        }
    }
    (tracks, counts)
}

#[test]
fn each_segment_is_decoded_with_the_init_of_its_own_map_run() {
    const FIRST: &str = "transmux/h264_aac_frag.mp4";
    const SECOND: &str = "transmux/hevc_frag.mp4";
    let first_bytes = test_fixtures::read(FIRST);
    let second_bytes = test_fixtures::read(SECOND);
    let first = split(&first_bytes).unwrap();
    let second = split(&second_bytes).unwrap();

    let mut playlist = String::from(
        "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:10\n#EXT-X-PLAYLIST-TYPE:VOD\n\
         #EXT-X-MAP:URI=\"i1.mp4\"\n",
    );
    let mut h = Host::new(PLAYLIST_URL, hls_config());
    h.ok(&format!("{BASE}/i1.mp4"), first.init.clone());
    h.ok(&format!("{BASE}/i2.mp4"), second.init.clone());
    for (i, s) in first.segments.iter().enumerate() {
        h.ok(&format!("{BASE}/a{i}.m4s"), s.clone());
        playlist.push_str(&format!("#EXTINF:1.0,\na{i}.m4s\n"));
    }
    playlist.push_str("#EXT-X-DISCONTINUITY\n#EXT-X-MAP:URI=\"i2.mp4\"\n");
    for (i, s) in second.segments.iter().enumerate() {
        h.ok(&format!("{BASE}/b{i}.m4s"), s.clone());
        playlist.push_str(&format!("#EXTINF:1.0,\nb{i}.m4s\n"));
    }
    playlist.push_str("#EXT-X-ENDLIST\n");
    h.ok(PLAYLIST_URL, playlist.into_bytes());
    h.run_to_end();

    assert_eq!(h.requests_to(&format!("{BASE}/i1.mp4")).len(), 1);
    assert_eq!(h.requests_to(&format!("{BASE}/i2.mp4")).len(), 1);
    let (tracks, counts) = split_counts(&h.events);
    assert_eq!(
        counts[0],
        oracle_counts(&first_bytes),
        "segments before the change use the first init"
    );
    assert_eq!(
        counts[1],
        oracle_counts(&second_bytes),
        "segments after the change use the second init"
    );
    assert_eq!(
        tracks,
        vec![
            oracle_counts(&first_bytes).into_keys().collect::<Vec<_>>(),
            oracle_counts(&second_bytes).into_keys().collect::<Vec<_>>(),
        ]
    );
    assert!(errors(&h.events).is_empty(), "{:?}", errors(&h.events));
    assert!(ended(&h.events));
}
