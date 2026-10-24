mod support;

use broadcast_client::{Event, Status};
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

#[test]
fn a_data_only_hls_ts_stream_reports_each_dropped_track_then_ends() {
    let data = test_fixtures::read("ts/scte35-other-pid.ts");
    let events = play(vec![data]);
    let errs = errors(&events);
    assert!(!errs.is_empty(), "nothing told the host why no media came");
    assert!(
        errs.iter()
            .all(|e| matches!(e, Event::Error { fatal: false, .. }))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::Status(Status::Ended)))
    );
    assert!(!events.iter().any(|e| matches!(e, Event::Sample { .. })));
}

#[test]
fn a_dropped_data_track_is_reported_once_while_media_keeps_flowing() {
    let Some(bytes) = test_fixtures::require_bytes("zenith-http/tf1-live.ts", "r5_hls_dropped")
    else {
        return;
    };
    let prefix = &bytes[..TF1_PREFIX_BYTES.min(bytes.len())];
    let first_sync = (0..188)
        .find(|&o| (0..5).all(|k| prefix.get(o + k * 188) == Some(&0x47)))
        .unwrap();
    let usable = (prefix.len() - first_sync) / 188 * 188;
    let events = play(split_at_pat(
        &prefix[first_sync..first_sync + usable],
        TF1_SEGMENTS,
    ));
    let errs = errors(&events);
    assert_eq!(errs.len(), 1, "{errs:?}");
    assert!(matches!(errs[0], Event::Error { fatal: false, .. }));
    assert!(events.iter().any(|e| matches!(e, Event::Sample { .. })));
}
