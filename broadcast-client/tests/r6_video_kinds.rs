mod support;

use broadcast_client::{Config, Event, TrackId};
use support::ts_host::Host;
use transmux::JoinDecision;

const CHUNK: usize = 8192;

#[test]
fn mpeg2_video_is_the_default_rendered_video_and_gets_a_join() {
    let bytes = test_fixtures::read("ts/legacy/mpeg2_mp2.ts");
    let mut host = Host::open(Config::default());
    host.feed(&bytes, CHUNK);
    let announced = host.announced().expect("tracks");
    assert_eq!(announced.len(), 2, "{announced:?}");
    let (video, audio) = host.session.default_rendered_tracks();
    assert_eq!(video, Some(TrackId::new(announced[0])));
    assert_eq!(audio, Some(TrackId::new(announced[1])));

    let joins: Vec<Option<JoinDecision>> = host
        .log
        .iter()
        .filter_map(|e| match e {
            Event::Sample { track, join, .. } if Some(*track) == video => Some(*join),
            _ => None,
        })
        .collect();
    assert!(!joins.is_empty());
    assert!(joins.iter().all(Option::is_some), "{joins:?}");
    let first_kept = joins.iter().position(|j| j.is_some_and(|j| j.is_keep()));
    assert!(matches!(
        first_kept.map(|i| joins[i]),
        Some(Some(JoinDecision::JoinPoint(_)))
    ));
}
