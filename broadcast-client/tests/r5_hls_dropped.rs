mod support;

use broadcast_client::{ErrorKind, Event, Status};
use support::hls_host::{Host, hls_config};
use support::ts_split::split_at_pat;

const PLAYLIST_URL: &str = "http://origin.test/live/index.m3u8";
const TF1_SEGMENTS: usize = 4;
const TF1_PREFIX_BYTES: usize = 6_000_000;

fn playlist(segments: usize) -> Vec<u8> {
    let mut p = String::from(
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:4\n#EXT-X-MEDIA-SEQUENCE:0\n",
    );
    for i in 0..segments {
        p.push_str(&format!("#EXTINF:3.5,\nseg{i}.ts\n"));
    }
    p.push_str("#EXT-X-ENDLIST\n");
    p.into_bytes()
}

fn play(segments: Vec<Vec<u8>>) -> Vec<Event> {
    let mut h = Host::new(PLAYLIST_URL, hls_config());
    h.ok(PLAYLIST_URL, playlist(segments.len()));
    for (i, s) in segments.into_iter().enumerate() {
        h.ok(&format!("http://origin.test/live/seg{i}.ts"), s);
    }
    h.run_to_end();
    h.events
}

fn errors(events: &[Event]) -> Vec<&Event> {
    events
        .iter()
        .filter(|e| matches!(e, Event::Error { .. }))
        .collect()
}

fn dropped(events: &[Event]) -> Vec<(u32, Option<u8>)> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Error {
                kind:
                    ErrorKind::TrackDropped {
                        track_id,
                        stream_type,
                    },
                fatal: false,
            } => Some((*track_id, *stream_type)),
            _ => None,
        })
        .collect()
}

fn unmuxable_tracks(ts: &[u8]) -> Vec<(u32, Option<u8>)> {
    use broadcast_common::Unpackage;
    let media = transmux::TsDemux::new().unpackage(ts).unwrap();
    media
        .tracks
        .iter()
        .filter(|t| !t.spec.config.is_muxable_in_bmff())
        .map(|t| {
            let st = match &t.spec.config {
                transmux::CodecConfig::Data { stream_type, .. } => Some(*stream_type),
                _ => None,
            };
            (t.spec.track_id.value(), st)
        })
        .collect()
}

#[test]
fn a_data_only_hls_ts_stream_reports_each_dropped_track_then_ends() {
    let data = test_fixtures::read("ts/scte35-other-pid.ts");
    let expected = unmuxable_tracks(&data);
    assert!(!expected.is_empty());
    let events = play(vec![data]);
    let errs = errors(&events);
    assert_eq!(errs.len(), expected.len(), "{errs:?}");
    let mut got = dropped(&events);
    got.sort_unstable();
    assert_eq!(got, expected);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::Status(Status::Ended)))
    );
    assert!(!events.iter().any(|e| matches!(e, Event::Sample { .. })));
}

#[test]
fn each_dropped_data_track_is_reported_once_while_media_keeps_flowing() {
    let Some(bytes) = test_fixtures::require_bytes("zenith-http/tf1-live.ts", "r5_hls_dropped")
    else {
        return;
    };
    let prefix = &bytes[..TF1_PREFIX_BYTES.min(bytes.len())];
    let first_sync = (0..188)
        .find(|&o| (0..5).all(|k| prefix.get(o + k * 188) == Some(&0x47)))
        .unwrap();
    let usable = (prefix.len() - first_sync) / 188 * 188;
    let ts = &prefix[first_sync..first_sync + usable];
    let expected = unmuxable_tracks(ts);
    assert!(!expected.is_empty());
    let events = play(split_at_pat(ts, TF1_SEGMENTS));
    let errs = errors(&events);
    assert_eq!(errs.len(), expected.len(), "{errs:?}");
    let mut got = dropped(&events);
    got.sort_unstable();
    assert_eq!(
        got, expected,
        "one report per dropped track, not per segment"
    );
    assert!(events.iter().any(|e| matches!(e, Event::Sample { .. })));
}
