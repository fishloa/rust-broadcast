mod support;

use broadcast_client::{Config, Event, TrackId};
use support::ts_host::Host;

const CHUNK: usize = 8192;
const PRIVATE_PREFIX_BYTES: usize = 1_000_000;

#[test]
fn a_reconnect_to_another_stream_announces_only_the_new_connections_tracks() {
    let first = test_fixtures::require_bytes("zenith-http/tf1-live.ts", "r6_ts_tracks");
    let Some(first) = first else { return };
    let second = test_fixtures::read("ts/h264_aac.ts");
    let mut host = Host::open(Config::default());
    host.feed(&first[..PRIVATE_PREFIX_BYTES], CHUNK);
    let old = host.announced().expect("tracks announced");
    assert!(old.len() > 2, "{old:?}");
    host.reconnect();
    host.feed(&second, CHUNK);
    assert_eq!(host.announced(), Some(vec![1, 2]));
    assert_eq!(
        host.session.default_rendered_tracks(),
        (Some(TrackId::new(1)), Some(TrackId::new(2)))
    );
}

#[test]
fn a_track_that_vanishes_across_a_reconnect_is_no_longer_a_default_rendered_phantom() {
    let first = test_fixtures::read("ts/h264_aac_40s.ts");
    let second = test_fixtures::read("ts/h264/high_1080_cropped.ts");
    let mut host = Host::open(Config::default());
    host.feed(&first[..first.len() / 3], CHUNK);
    assert_eq!(host.announced(), Some(vec![1, 2]));
    host.reconnect();
    let mark = host.mark();
    host.feed(&second, CHUNK);
    assert_eq!(host.announced(), Some(vec![1]));
    assert_eq!(
        host.session.default_rendered_tracks(),
        (Some(TrackId::new(1)), None)
    );
    let stray: Vec<_> = host.log[mark..]
        .iter()
        .filter(|e| matches!(e, Event::Sample { track, .. } if track.value() != 1))
        .collect();
    assert!(stray.is_empty());
}
