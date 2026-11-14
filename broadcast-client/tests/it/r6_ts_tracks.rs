use crate::support;

use broadcast_client::{Config, Event, TrackId};
use broadcast_common::stage::Timestamp;
use support::ts_host::Host;

/// The harness's microsecond clock as the session's `now`.
fn ts(us: u64) -> Timestamp {
    Timestamp::from_micros(us)
}

const CHUNK: usize = 8192;
const PRIVATE_PREFIX_BYTES: usize = 1_000_000;

#[test]
fn a_reconnect_to_another_stream_announces_only_the_new_connections_tracks() {
    let first = test_fixtures::require_bytes("http/live-channels/tf1-live.ts", "r6_ts_tracks");
    let Some(first) = first else { return };
    let second = test_fixtures::read("ts/h264-aac.ts");
    let mut host = Host::open(recording());
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
    let first = test_fixtures::read("ts/h264-aac-40s.ts");
    let second = test_fixtures::read("ts/h264/high-1080-cropped.ts");
    let mut host = Host::open(recording());
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

fn recording() -> Config {
    let mut config = Config::default();
    config.record_buffer_inputs = true;
    config
}

fn registrations(host: &Host) -> Vec<String> {
    host.session
        .buffer_input_log()
        .iter()
        .filter(|i| {
            matches!(
                i,
                broadcast_client::BufferInput::Register { .. }
                    | broadcast_client::BufferInput::Unregister { .. }
            )
        })
        .map(|i| match i {
            broadcast_client::BufferInput::Register { track, .. } => format!("Register({track})"),
            broadcast_client::BufferInput::Unregister { track, .. } => {
                format!("Unregister({track})")
            }
            other => format!("{other:?}"),
        })
        .collect()
}

fn unknown_reports(host: &Host) -> Vec<TrackId> {
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
    let bytes = test_fixtures::read("ts/h264-aac-40s.ts");
    let mut host = Host::open(recording());
    host.feed_playing(&bytes[..CHUNK * 3], CHUNK);
    if let Some(ids) = declared {
        let ids: Vec<TrackId> = ids.iter().map(|i| TrackId::new(*i)).collect();
        host.session.set_rendered_tracks(ts(host.now), &ids);
        host.settle();
    }
    host.feed_playing(&bytes[CHUNK * 3..], CHUNK);
    host
}

fn samples_before_start(host: &Host) -> Option<usize> {
    let start = host.log.iter().position(|e| {
        matches!(
            e,
            Event::Buffer(broadcast_client::BufferEvent::Start { .. })
        )
    })?;
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
    assert!(
        samples_before_start(&baseline).is_some(),
        "baseline never started"
    );
    assert_eq!(samples_before_start(&host), samples_before_start(&baseline));
    assert!(
        !registrations(&host).iter().any(|r| r.contains("999")),
        "{:?}",
        registrations(&host)
    );
    assert_eq!(unknown_reports(&host), vec![TrackId::new(999)]);
}

#[test]
fn an_unknown_id_beside_a_real_one_is_ignored_and_reported_once() {
    let host = run_declared(Some(&[1, 999]));
    let video_only = run_declared(Some(&[1]));
    assert!(samples_before_start(&host).is_some());
    assert_eq!(
        samples_before_start(&host),
        samples_before_start(&video_only)
    );
    assert!(!registrations(&host).iter().any(|r| r.contains("999")));
    assert_eq!(unknown_reports(&host), vec![TrackId::new(999)]);
    assert!(unknown_reports(&video_only).is_empty());
}

#[test]
fn a_declaration_made_before_the_first_announcement_applies_when_it_arrives_without_noise() {
    let bytes = test_fixtures::read("ts/h264-aac-40s.ts");
    let mut host = Host::open(recording());
    host.session
        .set_rendered_tracks(ts(host.now), &[TrackId::new(1), TrackId::new(2)]);
    host.settle();
    host.feed_playing(&bytes, CHUNK);
    assert!(unknown_reports(&host).is_empty());
    assert!(samples_before_start(&host).is_some());
    let regs = registrations(&host);
    assert!(regs.iter().any(|r| r.contains("Register(1)")), "{regs:?}");
    assert!(regs.iter().any(|r| r.contains("Register(2)")), "{regs:?}");
}

const DROP_AT: f64 = 0.4;
const RESTORE_AT: f64 = 0.7;

fn flipped_h264_aac() -> Vec<u8> {
    let bytes = test_fixtures::read("ts/h264-aac-40s.ts");
    let pmt = support::ts_pmt::find(&bytes).expect("a PMT");
    assert_eq!(pmt.es.len(), 2);
    let all = pmt.es.clone();
    let video_only = vec![pmt.es[0].clone()];
    let v = pmt.version;
    support::ts_pmt::rewrite(&bytes, &pmt, &|at| {
        if at < DROP_AT {
            None
        } else if at < RESTORE_AT {
            Some((video_only.clone(), v.wrapping_add(1)))
        } else {
            Some((all.clone(), v.wrapping_add(2)))
        }
    })
}

#[test]
fn an_explicit_video_audio_declaration_survives_a_pmt_drop_and_restore() {
    let bytes = flipped_h264_aac();
    let mut host = Host::open(recording());
    host.feed_playing(&bytes[..CHUNK * 3], CHUNK);
    host.session
        .set_rendered_tracks(ts(host.now), &[TrackId::new(1), TrackId::new(2)]);
    host.settle();
    host.feed_playing(&bytes[CHUNK * 3..], CHUNK);

    let announced: Vec<Vec<u32>> = host
        .log
        .iter()
        .filter_map(|e| match e {
            Event::Tracks(t) => Some(t.iter().map(|s| s.track_id.value()).collect()),
            _ => None,
        })
        .collect();
    assert_eq!(announced, vec![vec![1, 2], vec![1], vec![1, 3]]);
    let regs = registrations(&host);
    assert!(
        regs.iter().any(|r| r.contains("Register(3)")),
        "the restored audio is never judged by the controller: {regs:?}"
    );
    assert!(host.samples_on(3) > 0);
    assert!(unknown_reports(&host).is_empty());
}

#[test]
fn a_drop_and_restore_without_a_declaration_follows_the_default_set() {
    let bytes = flipped_h264_aac();
    let mut host = Host::open(recording());
    host.feed_playing(&bytes, CHUNK);
    assert_eq!(
        host.session.default_rendered_tracks(),
        (Some(TrackId::new(1)), Some(TrackId::new(3)))
    );
}
