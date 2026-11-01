use crate::support::replay::{SampleRec, replay_paced};
use crate::support::udp_host::{
    MP2T_PAYLOAD_TYPE, TS_PACKET, UdpHost, rtp_datagram, rtp_datagrams, ts_datagrams,
};
use broadcast_client::{
    Action, Config, Error, Event, MulticastJoin, Session, SourceKind, Status, UdpConfig, UdpMode,
};
use std::net::{IpAddr, Ipv4Addr};

const SMALL: &str = "ts/h264_aac.ts";
const LIVE_CAPTURE: &str = "zenith-live/tf-1-8MB.ts";
const GROUP: &str = "udp://239.1.1.1:5000";
const SSRC_A: u32 = 0xA1A1_A1A1;
const SSRC_B: u32 = 0xB2B2_B2B2;
const LONG_WAIT_US: u64 = 60_000_000;
const FAST_BPS: u64 = 100_000_000;
const STALL_NOTICE_US: u64 = 5_000_000;
const STALL_RECONNECT_US: u64 = 30_000_000;
const START_TIMEOUT_US: u64 = 15_000_000;
const FIRST_BACKOFF_US: u64 = 2_000_000;

fn udp_cfg(f: impl FnOnce(&mut UdpConfig)) -> UdpConfig {
    let mut c = UdpConfig::default();
    f(&mut c);
    c
}

fn config_with(f: impl FnOnce(&mut Config)) -> Config {
    let mut c = Config::default();
    f(&mut c);
    c
}

fn small() -> Vec<u8> {
    test_fixtures::read(SMALL)
}

fn baseline(ts: &[u8]) -> Vec<SampleRec> {
    replay_paced(ts, Config::default(), 7 * TS_PACKET, FAST_BPS).samples()
}

fn play(url: &str, datagrams: &[Vec<u8>]) -> UdpHost {
    let mut host = UdpHost::open(url, Config::default());
    for d in datagrams {
        host.datagram(d);
    }
    let flush = host.now + 1_000_000;
    host.advance_to(flush);
    host.close_socket();
    host
}

fn bind_of(session: &mut Session) -> Action {
    let mut found = None;
    while let Some(a) = session.poll(0) {
        if matches!(a, Action::UdpBind { .. }) {
            found = Some(a);
        }
    }
    found.expect("a UdpBind")
}

#[test]
fn a_multicast_url_asks_the_host_to_bind_and_join_the_group() {
    let config = config_with(|cfg| {
        cfg.udp.interface = Some(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)));
        cfg.udp.source = Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 7)));
        cfg.udp.recv_buffer_bytes = Some(4 << 20);
    });
    let mut s = Session::open(GROUP, config, 0).unwrap();
    let Action::UdpBind {
        host,
        port,
        mode,
        multicast,
        recv_buffer_bytes,
        ..
    } = bind_of(&mut s)
    else {
        unreachable!()
    };
    assert_eq!(
        (host.as_str(), port, mode),
        ("239.1.1.1", 5000, UdpMode::Receive)
    );
    assert_eq!(recv_buffer_bytes, Some(4 << 20));
    let join: MulticastJoin = multicast.expect("a multicast join");
    assert_eq!(
        join.interface,
        Some(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)))
    );
    assert_eq!(join.source, Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 7))));
}

#[test]
fn a_unicast_or_at_prefixed_url_binds_without_a_join() {
    for url in [
        "udp://127.0.0.1:5000",
        "udp://@127.0.0.1:5000",
        "rtp://[::1]:5000",
    ] {
        let mut s = Session::open(url, Config::default(), 0).unwrap();
        let Action::UdpBind {
            multicast, mode, ..
        } = bind_of(&mut s)
        else {
            unreachable!()
        };
        assert_eq!(multicast, None, "{url}");
        assert_eq!(mode, UdpMode::Receive, "{url}");
    }
    let mut s = Session::open("rtp://[ff02::1]:5000", Config::default(), 0).unwrap();
    let Action::UdpBind {
        host, multicast, ..
    } = bind_of(&mut s)
    else {
        unreachable!()
    };
    assert_eq!(host, "ff02::1");
    assert!(multicast.is_some());
}

#[test]
fn bad_udp_urls_and_configs_are_typed_errors() {
    let open = |url: &str, config: Config| Session::open(url, config, 0).unwrap_err();
    assert!(matches!(
        open("udp://example.com:5000", Config::default()),
        Error::InvalidUrl(_)
    ));
    assert!(matches!(
        open("udp://239.1.1.1", Config::default()),
        Error::InvalidUrl(_)
    ));
    assert!(matches!(
        open("udp://239.1.1.1:0", Config::default()),
        Error::InvalidUrl(_)
    ));
    let with = |udp: UdpConfig| config_with(|c| c.udp = udp);
    let v4 = |a, b, c, d| Some(IpAddr::V4(Ipv4Addr::new(a, b, c, d)));
    for (url, udp, field) in [
        (
            "udp://127.0.0.1:5000",
            udp_cfg(|c| c.source = v4(10, 0, 0, 1)),
            "udp.source",
        ),
        (
            "udp://127.0.0.1:5000",
            udp_cfg(|c| c.interface = v4(10, 0, 0, 1)),
            "udp.interface",
        ),
        (
            GROUP,
            udp_cfg(|c| c.source = Some("::1".parse().unwrap())),
            "udp.source",
        ),
        (
            GROUP,
            udp_cfg(|c| c.source = v4(239, 1, 1, 2)),
            "udp.source",
        ),
        (
            GROUP,
            udp_cfg(|c| c.reorder_packets = 0),
            "udp.reorder_packets",
        ),
        (
            GROUP,
            udp_cfg(|c| c.reorder_wait_us = 0),
            "udp.reorder_wait_us",
        ),
        (
            GROUP,
            udp_cfg(|c| c.recv_buffer_bytes = Some(0)),
            "udp.recv_buffer_bytes",
        ),
    ] {
        match open(url, with(udp)) {
            Error::InvalidConfig { field: f } => assert_eq!(f, field, "{url}"),
            other => panic!("{url}: {other:?}"),
        }
    }
}

#[test]
fn the_udp_kinds_and_schemes_must_agree() {
    let kind = |kind| config_with(|c| c.kind = kind);
    assert!(Session::open(GROUP, kind(SourceKind::Udp), 0).is_ok());
    assert!(Session::open(GROUP, kind(SourceKind::Auto), 0).is_ok());
    assert!(matches!(
        Session::open(GROUP, kind(SourceKind::Rtp), 0).unwrap_err(),
        Error::UnsupportedScheme(_)
    ));
    assert!(matches!(
        Session::open("rtp://239.1.1.1:5000", kind(SourceKind::Udp), 0).unwrap_err(),
        Error::UnsupportedScheme(_)
    ));
    assert!(matches!(
        Session::open("http://h/live.ts", kind(SourceKind::Udp), 0).unwrap_err(),
        Error::UnsupportedScheme(_)
    ));
    assert!(matches!(
        Session::open(GROUP, kind(SourceKind::TsHttp), 0).unwrap_err(),
        Error::UnsupportedScheme(_)
    ));
}

#[test]
fn udp_plays_exactly_what_ts_over_http_plays_whatever_the_datagram_size() {
    let ts = small();
    let want = baseline(&ts);
    assert!(want.len() > 50, "the fixture must carry real media");
    for per in [1, 7] {
        let got = play(GROUP, &ts_datagrams(&ts, per)).replay().samples();
        assert_eq!(got, want, "{per} TS packets per datagram");
    }
}

#[test]
fn a_real_live_capture_plays_the_same_over_udp_and_rtp() {
    let Some(ts) = test_fixtures::require_bytes(LIVE_CAPTURE, "udp") else {
        return;
    };
    let ts = &ts[..4_000_000 / TS_PACKET * TS_PACKET];
    let want = baseline(ts);
    assert!(want.len() > 100, "{} samples", want.len());
    assert_eq!(play(GROUP, &ts_datagrams(ts, 7)).replay().samples(), want);
    let rtp = rtp_datagrams(ts, 7, 40_000, SSRC_A);
    assert_eq!(play("rtp://239.1.1.1:5000", &rtp).replay().samples(), want);
}

#[test]
fn rtp_plays_the_same_as_udp_and_survives_sequence_wrap() {
    let ts = small();
    let want = baseline(&ts);
    for first in [0u16, 65_500] {
        let host = play(
            "rtp://239.1.1.1:5000",
            &rtp_datagrams(&ts, 7, first, SSRC_A),
        );
        assert_eq!(host.replay().samples(), want, "first seq {first}");
        let stats = host.session.datagram_stats();
        assert_eq!(
            (stats.lost, stats.late, stats.duplicates, stats.malformed),
            (0, 0, 0, 0)
        );
    }
}

#[test]
fn rtp_neighbours_swapped_inside_the_window_are_put_back() {
    let ts = small();
    let want = baseline(&ts);
    let mut rtp = rtp_datagrams(&ts, 7, 65_530, SSRC_A);
    let mut swaps = 0;
    for i in (3..rtp.len() - 1).step_by(9) {
        rtp.swap(i, i + 1);
        swaps += 1;
    }
    assert!(swaps > 3);
    let host = play("rtp://239.1.1.1:5000", &rtp);
    assert_eq!(host.replay().samples(), want);
    let stats = host.session.datagram_stats();
    assert_eq!(stats.reordered, swaps);
    assert_eq!(stats.lost, 0);
}

#[test]
fn rtp_duplicates_are_dropped_and_counted() {
    let ts = small();
    let want = baseline(&ts);
    let rtp = rtp_datagrams(&ts, 7, 100, SSRC_A);
    let mut with_dups = Vec::new();
    let mut dups = 0;
    for (i, d) in rtp.iter().enumerate() {
        with_dups.push(d.clone());
        if i % 10 == 5 {
            with_dups.push(d.clone());
            dups += 1;
        }
    }
    let swapped: Vec<Vec<u8>> = {
        let mut v = with_dups.clone();
        let i = v.iter().position(|d| d == &rtp[20]).unwrap();
        v.swap(i, i + 1);
        v
    };
    let host = play("rtp://239.1.1.1:5000", &with_dups);
    assert_eq!(host.replay().samples(), want);
    let stats = host.session.datagram_stats();
    assert_eq!(
        stats.late, dups,
        "a duplicate of a delivered packet is behind the delivery point"
    );
    let host = play("rtp://239.1.1.1:5000", &swapped);
    assert_eq!(host.replay().samples(), want);
    let held_dup = host.session.datagram_stats();
    assert!(held_dup.duplicates + held_dup.late >= dups);
}

#[test]
fn rtp_loss_is_counted_once_the_gap_is_given_up_and_the_stream_continues() {
    let ts = small();
    let want = baseline(&ts);
    let rtp = rtp_datagrams(&ts, 7, 1_000, SSRC_A);
    let kept: Vec<Vec<u8>> = rtp
        .iter()
        .enumerate()
        .filter(|(i, _)| i % 40 != 17)
        .map(|(_, d)| d.clone())
        .collect();
    let dropped = (rtp.len() - kept.len()) as u64;
    assert!(dropped >= 2);
    let host = play("rtp://239.1.1.1:5000", &kept);
    let stats = host.session.datagram_stats();
    assert_eq!(stats.lost, dropped);
    let got = host.replay().samples();
    assert!(!got.is_empty() && got.len() <= want.len());
    assert!(
        got.len() * 10 >= want.len() * 8,
        "{} of {} samples survived {dropped} lost packets",
        got.len(),
        want.len()
    );
}

#[test]
fn a_gap_is_held_for_the_reorder_wait_and_the_wake_is_announced() {
    let ts = small();
    let rtp = rtp_datagrams(&ts, 7, 10, SSRC_A);
    let mut host = UdpHost::open("rtp://239.1.1.1:5000", Config::default());
    host.datagram(&rtp[0]);
    host.datagram(&rtp[2]);
    let held_at = host.now;
    assert_eq!(host.session.datagram_stats().lost, 0);
    let wait = UdpConfig::default().reorder_wait_us;
    assert!(
        host.wake.is_some_and(|w| w <= held_at + wait),
        "a host sleeping on WakeAt must wake when the held packet is due: {:?}",
        host.wake
    );
    host.advance_to(held_at + wait - 1);
    assert_eq!(host.session.datagram_stats().lost, 0, "not due yet");
    host.advance_to(held_at + wait);
    assert_eq!(host.session.datagram_stats().lost, 1);
    host.datagram(&rtp[1]);
    assert_eq!(
        host.session.datagram_stats().late,
        1,
        "the packet that came after its slot"
    );
}

#[test]
fn a_packet_beyond_the_window_gives_up_everything_held() {
    let ts = small();
    let window = UdpConfig::default().reorder_packets;
    let rtp = rtp_datagrams(&ts, 1, 0, SSRC_A);
    assert!(rtp.len() > window + 5);
    let mut host = UdpHost::open(
        "rtp://239.1.1.1:5000",
        config_with(|c| c.udp.reorder_wait_us = LONG_WAIT_US),
    );
    host.datagram(&rtp[0]);
    for d in &rtp[2..window + 2] {
        host.datagram(d);
    }
    assert_eq!(
        host.session.datagram_stats().lost,
        0,
        "a full window is still waiting for the missing packet"
    );
    host.datagram(&rtp[window + 2]);
    assert_eq!(host.session.datagram_stats().lost, 1);
    host.datagram(&rtp[window]);
    assert_eq!(host.session.datagram_stats().late, 1);
}

#[test]
fn a_duplicate_of_a_held_packet_is_counted_and_not_delivered_twice() {
    let ts = small();
    let want = baseline(&ts);
    let rtp = rtp_datagrams(&ts, 7, 7, SSRC_A);
    let mut host = UdpHost::open("rtp://239.1.1.1:5000", Config::default());
    host.datagram(&rtp[0]);
    host.datagram(&rtp[2]);
    host.datagram(&rtp[2]);
    assert_eq!(host.session.datagram_stats().duplicates, 1);
    host.datagram(&rtp[1]);
    for d in &rtp[3..] {
        host.datagram(d);
    }
    let end = host.now + 1_000_000;
    host.advance_to(end);
    host.close_socket();
    assert_eq!(host.replay().samples(), want);
    let stats = host.session.datagram_stats();
    assert_eq!((stats.duplicates, stats.lost, stats.reordered), (1, 0, 1));
}

#[test]
fn a_new_ssrc_restarts_the_sequence_space_with_a_discontinuity() {
    let ts = small();
    let half = ts.len() / TS_PACKET / 2 * TS_PACKET;
    let mut dgrams = rtp_datagrams(&ts[..half], 7, 500, SSRC_A);
    let next_seq = 500 + dgrams.len() as u16;
    dgrams.extend(rtp_datagrams(&ts[half..], 7, next_seq, SSRC_B));
    let host = play("rtp://239.1.1.1:5000", &dgrams);
    let stats = host.session.datagram_stats();
    assert_eq!((stats.resyncs, stats.lost), (1, 0));
    assert!(host.count(|e| matches!(e, Event::Discontinuity)) >= 1);
}

#[test]
fn a_backward_jump_beyond_the_window_is_a_restart_and_a_forward_jump_is_loss() {
    let ts = small();
    let rtp = rtp_datagrams(&ts, 7, 0, SSRC_A);
    let mut host = UdpHost::open("rtp://239.1.1.1:5000", Config::default());
    host.datagram(&rtp[0]);
    host.datagram(&rtp[1]);
    let window = UdpConfig::default().reorder_packets as u16;
    let ahead = rtp_datagram(
        2 + window + 100,
        SSRC_A,
        MP2T_PAYLOAD_TYPE,
        &ts[..7 * TS_PACKET],
    );
    host.datagram(&ahead);
    let stats = host.session.datagram_stats();
    assert_eq!(stats.lost, u64::from(window) + 100);
    assert_eq!(stats.resyncs, 0);
    let behind = rtp_datagram(10, SSRC_A, MP2T_PAYLOAD_TYPE, &ts[..7 * TS_PACKET]);
    host.datagram(&behind);
    let stats = host.session.datagram_stats();
    assert_eq!(stats.resyncs, 1);
    assert!(host.count(|e| matches!(e, Event::Discontinuity)) >= 1);
}

#[test]
fn datagrams_that_are_not_whole_ts_packets_are_dropped_whole_and_counted() {
    let ts = small();
    let good = ts_datagrams(&ts, 7);
    let mut host = UdpHost::open(GROUP, Config::default());
    let mut broken_sync = good[3].clone();
    broken_sync[2 * TS_PACKET] = 0x00;
    let bad: Vec<Vec<u8>> = vec![
        Vec::new(),
        good[0][..TS_PACKET - 1].to_vec(),
        good[0][..TS_PACKET + 4].to_vec(),
        broken_sync,
    ];
    for d in &bad {
        host.datagram(d);
    }
    let stats = host.session.datagram_stats();
    assert_eq!((stats.datagrams, stats.malformed), (4, 4));
    assert_eq!(
        host.count(|e| matches!(e, Event::Sample { .. } | Event::Tracks(_))),
        0
    );

    let mut rtp = UdpHost::open("rtp://239.1.1.1:5000", Config::default());
    let payload = &good[0];
    for d in [
        rtp_datagram(1, SSRC_A, 96, payload),
        rtp_datagram(2, SSRC_A, MP2T_PAYLOAD_TYPE, &payload[..TS_PACKET - 1]),
        good[0].clone(),
        vec![0x80; 5],
    ] {
        rtp.datagram(&d);
    }
    assert_eq!(rtp.session.datagram_stats().malformed, 4);
}

fn warmed(ts: &[u8]) -> (UdpHost, u64) {
    let mut host = UdpHost::open(GROUP, Config::default());
    for d in ts_datagrams(ts, 7) {
        host.datagram(&d);
    }
    let samples = host.count(|e| matches!(e, Event::Sample { .. }));
    assert!(samples > 0);
    let last = host.now;
    (host, last as u64)
}

#[test]
fn silence_is_a_stall_then_live_again_then_a_rebind() {
    let ts = small();
    let (mut host, last) = warmed(&ts);
    let datagrams = ts_datagrams(&ts, 7);
    host.advance_to(last + STALL_NOTICE_US);
    assert!(host.count(|e| matches!(e, Event::Starved(_))) >= 1);
    assert!(host.count(|e| matches!(e, Event::Status(Status::Stalled { .. }))) >= 1);
    host.datagram(&datagrams[0]);
    assert!(
        matches!(
            host.events.iter().rev().find_map(|t| match &t.event {
                Event::Status(s) => Some(*s),
                _ => None,
            }),
            Some(Status::Live)
        ),
        "the next datagram makes the stream live again"
    );
    let resumed = host.now;
    host.advance_to(resumed + STALL_RECONNECT_US);
    assert_eq!(host.closes.len(), 1, "a stalled socket is closed");
    assert_eq!(host.binds.len(), 2, "and bound again");
    assert_eq!(host.bind_modes(), vec![UdpMode::Receive; 2]);
    assert_ne!(
        host.closes[0],
        host.conn.unwrap(),
        "the new bind has a new conn id"
    );
}

#[test]
fn a_rebound_stream_starts_a_new_timeline_with_a_discontinuity() {
    let ts = small();
    let (mut host, last) = warmed(&ts);
    let before = host.events.len();
    host.advance_to(last + STALL_RECONNECT_US + 1_000_000);
    assert_eq!(host.binds.len(), 2);
    for d in ts_datagrams(&ts, 7) {
        host.datagram(&d);
    }
    host.advance_to(host.now + 1_000_000);
    let after: Vec<&Event> = host.events[before..].iter().map(|t| &t.event).collect();
    let disc = after
        .iter()
        .position(|e| matches!(e, Event::Discontinuity))
        .expect("a discontinuity");
    assert!(
        after[disc..]
            .iter()
            .any(|e| matches!(e, Event::Sample { .. })),
        "media follows the announcement of the new timeline"
    );
}

#[test]
fn no_datagram_before_the_start_timeout_closes_and_rebinds_with_backoff() {
    let mut host = UdpHost::open(GROUP, Config::default());
    host.advance_to(START_TIMEOUT_US);
    assert_eq!(host.closes.len(), 1);
    assert_eq!(host.binds.len(), 1, "the retry waits for the backoff");
    assert!(host.count(|e| matches!(e, Event::Status(Status::Reconnecting { .. }))) >= 1);
    host.advance_to(START_TIMEOUT_US + FIRST_BACKOFF_US);
    assert_eq!(host.binds.len(), 2);
}

#[test]
fn a_bind_failure_backs_off_and_retries() {
    let mut host = UdpHost::open(GROUP, Config::default());
    host.auto_connect = false;
    let conn = host.conn.unwrap();
    host.session
        .on_closed(conn, Some("address in use".to_string()), host.now);
    host.settle();
    assert!(host.count(|e| matches!(e, Event::Status(Status::Reconnecting { .. }))) >= 1);
    assert_eq!(host.binds.len(), 1);
    host.advance_to(host.now + FIRST_BACKOFF_US);
    assert_eq!(host.binds.len(), 2);
}

#[test]
fn caller_reconnect_and_close_each_end_the_socket_once() {
    let ts = small();
    let (mut host, _) = warmed(&ts);
    let first = host.conn.unwrap();
    host.session.reconnect(host.now);
    host.settle();
    assert_eq!(host.closes, vec![first]);
    assert_eq!(host.binds.len(), 2);
    let second = host.conn.unwrap();
    host.session.close();
    host.settle();
    assert_eq!(host.closes, vec![first, second]);
    host.session
        .on_data(second, &ts_datagrams(&ts, 7)[0], host.now + 1);
    assert!(host.session.poll(host.now + 2).is_none());
}

#[test]
fn datagrams_for_an_unknown_socket_are_ignored() {
    let ts = small();
    let mut host = UdpHost::open(GROUP, Config::default());
    let conn = host.conn.unwrap();
    host.session.on_data(conn + 99, &ts_datagrams(&ts, 7)[0], 1);
    assert_eq!(host.session.datagram_stats().datagrams, 0);
}

#[test]
fn stats_are_zero_for_a_source_with_no_datagrams() {
    let s = Session::open("http://h/live.ts", Config::default(), 0).unwrap();
    assert_eq!(
        s.datagram_stats(),
        broadcast_client::DatagramStats::default()
    );
}
