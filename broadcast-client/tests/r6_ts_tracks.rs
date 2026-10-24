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

fn registrations(host: &Host) -> Vec<String> {
    host.session
        .buffer_input_log()
        .iter()
        .filter(|i| {
            matches!(
                i,
                broadcast_client::BufferInput::Register(_)
                    | broadcast_client::BufferInput::Unregister(_)
            )
        })
        .map(|i| format!("{i:?}"))
        .collect()
}

fn unknown_reports(host: &Host) -> Vec<u32> {
    host.log
        .iter()
        .filter_map(|e| match e {
            Event::Error {
                kind: broadcast_client::ErrorKind::UnknownRenderedTrack { track_id },
                fatal: false,
            } => Some(*track_id),
            _ => None,
        })
        .collect()
}

fn run_declared(declared: Option<&[u32]>) -> Host {
    let bytes = test_fixtures::read("ts/h264_aac_40s.ts");
    let mut host = Host::open(Config::default());
    host.feed_playing(&bytes[..CHUNK * 3], CHUNK);
    if let Some(ids) = declared {
        let ids: Vec<TrackId> = ids.iter().map(|i| TrackId::new(*i)).collect();
        host.session.set_rendered_tracks(&ids);
        host.settle();
    }
    host.feed_playing(&bytes[CHUNK * 3..], CHUNK);
    host
}

fn samples_before_start(host: &Host) -> Option<usize> {
    let start = host
        .log
        .iter()
        .position(|e| matches!(e, Event::Buffer(broadcast_client::BufferEvent::Start { .. })))?;
    Some(
        host.log[..start]
            .iter()
            .filter(|e| matches!(e, Event::Sample { .. }))
            .count(),
    )
}

#[test]
fn declaring_only_an_unknown_track_neither_blocks_start_nor_registers_it() {
    let baseline = run_declared(None);
    let host = run_declared(Some(&[999]));
    assert!(samples_before_start(&baseline).is_some(), "baseline never started");
    assert_eq!(samples_before_start(&host), samples_before_start(&baseline));
    assert!(
        !registrations(&host).iter().any(|r| r.contains("999")),
        "{:?}",
        registrations(&host)
    );
    assert_eq!(unknown_reports(&host), vec![999]);
}

#[test]
fn an_unknown_id_beside_a_real_one_is_ignored_and_reported_once() {
    let host = run_declared(Some(&[1, 999]));
    let video_only = run_declared(Some(&[1]));
    assert!(samples_before_start(&host).is_some());
    assert_eq!(samples_before_start(&host), samples_before_start(&video_only));
    assert!(!registrations(&host).iter().any(|r| r.contains("999")));
    assert_eq!(unknown_reports(&host), vec![999]);
    assert!(unknown_reports(&video_only).is_empty());
}

#[test]
fn a_declaration_made_before_the_first_announcement_applies_when_it_arrives_without_noise() {
    let bytes = test_fixtures::read("ts/h264_aac_40s.ts");
    let mut host = Host::open(Config::default());
    host.session
        .set_rendered_tracks(&[TrackId::new(1), TrackId::new(2)]);
    host.settle();
    host.feed_playing(&bytes, CHUNK);
    assert!(unknown_reports(&host).is_empty());
    assert!(samples_before_start(&host).is_some());
    let regs = registrations(&host);
    assert!(regs.iter().any(|r| r.contains("Register(1)")), "{regs:?}");
    assert!(regs.iter().any(|r| r.contains("Register(2)")), "{regs:?}");
}
