use crate::support::replay::{SampleRec, replay_paced};
use crate::support::srt_peer::{Cipher, PEER_ISN, SrtPeer};
use crate::support::srt_sim::{Fate, SEED, SrtSim, config_with_seed};
use crate::support::udp_host::{TS_PACKET, ts_datagrams};
use broadcast_client::{
    Config, Error, ErrorKind, Event, Session, SourceKind, SrtEncryption, SrtEntropySeed,
    SrtKeyLength, Status,
};

const URL: &str = "srt://127.0.0.1:9000";
const SMALL: &str = "ts/h264_aac.ts";
const PASS: &str = "correct horse battery";
const LATENCY_MS: u16 = 120;
const DATA_INTERVAL_US: u64 = 2_000;
const FAST_BPS: u64 = 100_000_000;
const SECOND_US: u64 = 1_000_000;

fn small() -> Vec<u8> {
    test_fixtures::read(SMALL)
}

fn baseline(ts: &[u8]) -> Vec<SampleRec> {
    replay_paced(ts, Config::default(), 7 * TS_PACKET, FAST_BPS).samples()
}

fn cfg(f: impl FnOnce(&mut Config)) -> Config {
    let mut c = config_with_seed(Config::default());
    f(&mut c);
    c
}

fn encrypted(length: SrtKeyLength) -> Config {
    cfg(|c| c.srt.encryption = Some(SrtEncryption::new(PASS, length)))
}

fn clear_sim() -> SrtSim {
    SrtSim::open(
        URL,
        cfg(|_| {}),
        SrtPeer::new(Cipher::Clear, None, LATENCY_MS),
    )
}

fn stream(sim: &mut SrtSim, ts: &[u8], per: usize) {
    for d in ts_datagrams(ts, per) {
        sim.send_data(&d);
        sim.advance_by(DATA_INTERVAL_US);
    }
}

fn finish(sim: &mut SrtSim) {
    sim.advance_by(SECOND_US);
    let bye = {
        let now = sim.now;
        sim.peer().shutdown(now)
    };
    sim.raw_to_session(bye);
    sim.advance_by(SECOND_US / 10);
}

fn play(mut sim: SrtSim, ts: &[u8]) -> SrtSim {
    sim.connect();
    stream(&mut sim, ts, 7);
    finish(&mut sim);
    sim
}

fn last_status(sim: &SrtSim) -> Option<Status> {
    sim.events.iter().rev().find_map(|t| match &t.event {
        Event::Status(s) => Some(*s),
        _ => None,
    })
}

#[test]
fn a_clear_srt_stream_plays_exactly_what_ts_over_http_plays() {
    let ts = small();
    let want = baseline(&ts);
    assert!(want.len() > 50);
    let sim = play(clear_sim(), &ts);
    assert_eq!(sim.replay().samples(), want);
    let stats = sim.session.datagram_stats();
    assert_eq!(
        (stats.lost, stats.late, stats.malformed, stats.undecryptable),
        (0, 0, 0, 0)
    );
    assert!(stats.datagrams > 0);
    assert!(sim.count(|e| matches!(e, Event::Status(Status::Establishing))) >= 1);
    assert!(sim.count(|e| matches!(e, Event::Status(Status::Live))) >= 1);
}

#[test]
fn every_key_length_decrypts_to_the_same_media() {
    let ts = small();
    let want = baseline(&ts);
    for (length, cipher) in [
        (SrtKeyLength::Aes128, Cipher::Aes128),
        (SrtKeyLength::Aes192, Cipher::Aes192),
        (SrtKeyLength::Aes256, Cipher::Aes256),
    ] {
        let sim = SrtSim::open(
            URL,
            encrypted(length),
            SrtPeer::new(cipher, Some(PASS), LATENCY_MS),
        );
        let sim = play(sim, &ts);
        assert_eq!(sim.replay().samples(), want, "{length}");
        assert_eq!(sim.session.datagram_stats().undecryptable, 0, "{length}");
    }
}

#[test]
fn a_wrong_passphrase_is_refused_by_name_and_retried_after_a_backoff() {
    let sim = SrtSim::open_with_peers(
        URL,
        {
            let mut c = encrypted(SrtKeyLength::Aes128);
            c.srt.encryption = Some(SrtEncryption::new(
                "not the passphrase",
                SrtKeyLength::Aes128,
            ));
            c
        },
        vec![
            SrtPeer::new(Cipher::Aes128, Some(PASS), LATENCY_MS),
            SrtPeer::new(Cipher::Aes128, Some(PASS), LATENCY_MS),
        ],
    );
    let mut sim = sim;
    sim.advance_by(SECOND_US / 2);
    let reasons: Vec<&'static str> = sim
        .events
        .iter()
        .filter_map(|t| match &t.event {
            Event::Error {
                kind: ErrorKind::SrtRejected { reason },
                fatal: false,
            } => Some(*reason),
            _ => None,
        })
        .collect();
    assert_eq!(
        reasons,
        vec![srt_runtime::RejectionReason::BadSecret.name()]
    );
    assert_eq!(sim.closes.len(), 1);
    assert!(sim.count(|e| matches!(e, Event::Status(Status::Reconnecting { .. }))) >= 1);
    assert_eq!(sim.binds_count(), 1, "the retry waits for the backoff");
    sim.advance_by(3 * SECOND_US);
    assert_eq!(sim.binds_count(), 2);
}

#[test]
fn a_clear_caller_against_a_protected_listener_is_refused() {
    let mut sim = SrtSim::open(
        URL,
        cfg(|_| {}),
        SrtPeer::new(Cipher::Aes128, Some(PASS), LATENCY_MS),
    );
    sim.advance_by(SECOND_US / 2);
    assert_eq!(
        sim.count(|e| matches!(
            e,
            Event::Error {
                kind: ErrorKind::SrtRejected { .. },
                ..
            }
        )),
        1
    );
}

#[test]
fn lost_packets_are_asked_for_again_and_the_media_is_complete() {
    let ts = small();
    let want = baseline(&ts);
    let mut sim = clear_sim();
    sim.fate = Box::new(|seq, retransmit| {
        if !retransmit && (seq - PEER_ISN) % 7 == 3 {
            Fate::Drop
        } else {
            Fate::Deliver
        }
    });
    let sim = play(sim, &ts);
    assert_eq!(sim.replay().samples(), want);
    let stats = sim.session.datagram_stats();
    assert_eq!(stats.lost, 0, "every loss was recovered inside the latency");
    let mut sim = sim;
    assert!(sim.peer().naks_received > 0 && sim.peer().retransmits > 0);
}

#[test]
fn a_packet_that_never_arrives_is_dropped_at_its_deadline_and_counted() {
    let ts = small();
    let want = baseline(&ts);
    let missing = PEER_ISN + 12;
    let mut sim = clear_sim();
    sim.fate = Box::new(move |seq, _| {
        if seq == missing {
            Fate::Drop
        } else {
            Fate::Deliver
        }
    });
    let mut sim = play(sim, &ts);
    assert_eq!(sim.session.datagram_stats().lost, 1);
    let acked = sim.peer().max_acked.expect("the peer was acknowledged");
    assert!(
        srt_runtime::arq::seq::seq_lt(missing, acked),
        "the ack point must move past a packet given up on, or the peer keeps it forever: {acked}"
    );
    let got = sim.replay().samples();
    assert!(!got.is_empty() && got.len() <= want.len());
    assert!(
        got.len() * 10 >= want.len() * 8,
        "{} of {}",
        got.len(),
        want.len()
    );
}

#[test]
fn a_drop_request_skips_the_gap_without_counting_a_loss() {
    let ts = small();
    let missing = PEER_ISN + 12;
    let mut sim = clear_sim();
    sim.fate = Box::new(move |seq, _| {
        if seq == missing {
            Fate::Drop
        } else {
            Fate::Deliver
        }
    });
    sim.connect();
    let datagrams = ts_datagrams(&ts, 7);
    for (i, d) in datagrams.iter().enumerate() {
        sim.send_data(d);
        if i == 14 {
            let now = sim.now;
            let req = sim.peer().drop_request(missing, missing, now);
            sim.raw_to_session(req);
        }
        sim.advance_by(DATA_INTERVAL_US);
    }
    finish(&mut sim);
    assert_eq!(sim.session.datagram_stats().lost, 0);
    assert!(sim.count(|e| matches!(e, Event::Sample { .. })) > 40);
    let acked = sim.peer().max_acked.expect("the peer was acknowledged");
    assert!(srt_runtime::arq::seq::seq_lt(missing, acked));
}

#[test]
fn packets_that_arrive_out_of_order_are_put_back() {
    let ts = small();
    let want = baseline(&ts);
    let mut sim = clear_sim();
    sim.fate = Box::new(|seq, retransmit| {
        if !retransmit && (seq - PEER_ISN) % 5 == 1 {
            Fate::DelayUs(6_000)
        } else {
            Fate::Deliver
        }
    });
    let sim = play(sim, &ts);
    assert_eq!(sim.replay().samples(), want);
    assert_eq!(sim.session.datagram_stats().lost, 0);
}

#[test]
fn duplicated_packets_are_dropped_and_counted() {
    let ts = small();
    let want = baseline(&ts);
    let mut sim = clear_sim();
    sim.fate = Box::new(|seq, retransmit| {
        if !retransmit && (seq - PEER_ISN).is_multiple_of(4) {
            Fate::Duplicate
        } else {
            Fate::Deliver
        }
    });
    let sim = play(sim, &ts);
    assert_eq!(sim.replay().samples(), want);
    let stats = sim.session.datagram_stats();
    assert!(stats.duplicates >= 10, "{stats:?}");
}

#[test]
fn an_idle_link_is_kept_alive_then_dropped_and_rebuilt_with_fresh_identifiers() {
    let ts = small();
    let mut sim = SrtSim::open_with_peers(
        URL,
        cfg(|_| {}),
        vec![
            SrtPeer::new(Cipher::Clear, None, LATENCY_MS),
            SrtPeer::new(Cipher::Clear, None, LATENCY_MS),
        ],
    );
    sim.connect();
    stream(&mut sim, &ts, 7);
    let first_id = sim.peer().caller_socket_id;
    assert_ne!(first_id, 0);
    let acks = sim.peer().acks_received;
    sim.advance_by(2 * SECOND_US);
    assert!(
        sim.peer().acks_received >= acks + 100,
        "an idle link is kept alive by the receiver's 10 ms ACKs"
    );
    sim.peer_listening = false;
    sim.advance_by(7 * SECOND_US);
    assert_eq!(sim.closes.len(), 1, "the silent link is closed");
    assert_eq!(sim.binds_count(), 2, "and rebuilt at once");
    sim.peer_listening = true;
    sim.advance_by(SECOND_US);
    let second_id = sim.peer().caller_socket_id;
    assert_ne!(second_id, 0);
    assert_ne!(first_id, second_id, "a reconnect never reuses a socket id");
    stream(&mut sim, &ts, 7);
    finish(&mut sim);
    assert!(sim.count(|e| matches!(e, Event::Discontinuity)) >= 1);
}

#[test]
fn a_peer_shutdown_ends_the_link_and_a_played_stream_reconnects_at_once() {
    let ts = small();
    let mut sim = SrtSim::open_with_peers(
        URL,
        cfg(|_| {}),
        vec![
            SrtPeer::new(Cipher::Clear, None, LATENCY_MS),
            SrtPeer::new(Cipher::Clear, None, LATENCY_MS),
        ],
    );
    sim.connect();
    stream(&mut sim, &ts, 7);
    finish(&mut sim);
    assert_eq!(sim.closes.len(), 1);
    assert_eq!(sim.binds_count(), 2);
    assert!(sim.count(|e| matches!(e, Event::Sample { .. })) > 0);
}

#[test]
fn a_silent_peer_fails_the_handshake_after_retransmissions() {
    let mut sim = clear_sim();
    sim.peer_listening = false;
    sim.advance_by(SECOND_US);
    let induction_sends = sim.from_session.len();
    assert!(
        induction_sends >= 3,
        "the induction is retransmitted, saw {induction_sends}"
    );
    sim.advance_by(2 * SECOND_US + SECOND_US / 2);
    assert_eq!(
        sim.count(|e| matches!(
            e,
            Event::Error {
                kind: ErrorKind::SrtHandshakeTimeout,
                fatal: false
            }
        )),
        1
    );
    assert_eq!(sim.closes.len(), 1);
    assert!(sim.count(|e| matches!(e, Event::Status(Status::Reconnecting { .. }))) >= 1);
    sim.advance_by(3 * SECOND_US);
    assert_eq!(sim.binds_count(), 2);
}

#[test]
fn close_says_goodbye_to_a_connected_peer_once_and_then_closes_the_socket() {
    let ts = small();
    let mut sim = clear_sim();
    sim.connect();
    stream(&mut sim, &ts, 7);
    let peer_id = sim.peer().caller_socket_id;
    assert_ne!(peer_id, 0);
    let before = sim.from_session.len();
    sim.session.close();
    let mut sends = Vec::new();
    let mut closes = Vec::new();
    while let Some(a) = sim.session.poll(sim.now) {
        match a {
            broadcast_client::Action::Send { bytes, .. } => sends.push(bytes),
            broadcast_client::Action::Close { conn } => closes.push(conn),
            _ => {}
        }
    }
    let _ = before;
    assert_eq!(sends.len(), 1);
    const CONTROL_TYPE_SHUTDOWN: u16 = 0x0005;
    let shutdown = &sends[0];
    assert_eq!(shutdown[0] & 0x80, 0x80, "a control packet");
    assert_eq!(
        u16::from_be_bytes([shutdown[0] & 0x7F, shutdown[1]]),
        CONTROL_TYPE_SHUTDOWN
    );
    let dest = u32::from_be_bytes([shutdown[12], shutdown[13], shutdown[14], shutdown[15]]);
    assert_eq!(dest, sim.peers[0].connected_peer_socket_id());
    assert_eq!(closes.len(), 1);
    assert!(sim.session.poll(sim.now + 1).is_none());
}

#[test]
fn closing_before_the_handshake_completes_sends_nothing_but_the_close() {
    let mut sim = clear_sim();
    sim.peer_listening = false;
    sim.advance_by(100_000);
    sim.session.close();
    let mut sends = 0;
    let mut closes = 0;
    while let Some(a) = sim.session.poll(sim.now) {
        match a {
            broadcast_client::Action::Send { .. } => sends += 1,
            broadcast_client::Action::Close { .. } => closes += 1,
            _ => {}
        }
    }
    assert_eq!((sends, closes), (0, 1));
}

#[test]
fn the_entropy_seed_is_required_and_every_attempt_derives_fresh_identifiers() {
    assert!(matches!(
        Session::open(URL, Config::default(), 0).unwrap_err(),
        Error::InvalidConfig {
            field: "srt.entropy_seed"
        }
    ));
    let ids = |seed: [u8; 32]| {
        let mut sim = SrtSim::open_with_peers(
            URL,
            cfg(|c| c.srt.entropy_seed = Some(SrtEntropySeed::new(seed))),
            vec![
                SrtPeer::new(Cipher::Clear, None, LATENCY_MS),
                SrtPeer::new(Cipher::Clear, None, LATENCY_MS),
            ],
        );
        sim.advance_by(SECOND_US / 2);
        let first = sim.peers[0].caller_socket_id;
        sim.session.reconnect(sim.now);
        sim.advance_by(SECOND_US / 2);
        (first, sim.peers[1].caller_socket_id)
    };
    let a = ids(SEED);
    assert_eq!(
        a,
        ids(SEED),
        "the same seed and attempt give the same identifiers"
    );
    assert_ne!(a.0, a.1);
    assert_ne!(
        a.0,
        ids([0xA5; 32]).0,
        "a different seed gives different identifiers"
    );
}

#[test]
fn bad_srt_settings_are_typed_errors() {
    let open = |f: fn(&mut Config)| Session::open(URL, cfg(f), 0).unwrap_err();
    for (err, field) in [
        (open(|c| c.srt.latency_ms = 119), "srt.latency_ms"),
        (
            open(|c| c.srt.handshake_timeout_ms = 0),
            "srt.handshake_timeout_ms",
        ),
        (
            open(|c| c.srt.peer_idle_timeout_ms = 0),
            "srt.peer_idle_timeout_ms",
        ),
        (
            open(|c| c.srt.encryption = Some(SrtEncryption::new("", SrtKeyLength::Aes128))),
            "srt.encryption.passphrase",
        ),
    ] {
        match err {
            Error::InvalidConfig { field: f } => assert_eq!(f, field),
            other => panic!("{field}: {other:?}"),
        }
    }
    assert!(matches!(
        Session::open("srt://127.0.0.1", cfg(|_| {}), 0).unwrap_err(),
        Error::InvalidUrl(_)
    ));
    assert!(matches!(
        Session::open(URL, cfg(|c| c.kind = SourceKind::Udp), 0).unwrap_err(),
        Error::UnsupportedScheme(_)
    ));
}

#[test]
fn secrets_never_appear_in_debug_output() {
    let config = cfg(|c| {
        c.srt.encryption = Some(SrtEncryption::new(
            "hunter2-passphrase",
            SrtKeyLength::Aes256,
        ));
    });
    let text = format!("{config:?} {:?}", config.srt.entropy_seed);
    assert!(!text.contains("hunter2"), "{text}");
    assert!(text.contains("redacted"));
    let sim = SrtSim::open(URL, config, SrtPeer::new(Cipher::Clear, None, LATENCY_MS));
    assert!(!format!("{:?}", sim.session).contains("hunter2"));
}

#[test]
fn packets_under_a_key_the_session_does_not_hold_are_dropped_and_counted() {
    use srt_runtime::packet::EncryptionKeyField;
    let ts = small();
    let payload = &ts_datagrams(&ts, 7)[0];
    let mut enc = SrtSim::open(
        URL,
        encrypted(SrtKeyLength::Aes128),
        SrtPeer::new(Cipher::Aes128, Some(PASS), LATENCY_MS),
    );
    enc.connect();
    let odd = {
        let now = enc.now;
        enc.peer()
            .data_with(payload, now, Some(EncryptionKeyField::Odd))
    };
    enc.raw_to_session(odd);
    let plain = {
        let now = enc.now;
        enc.peer()
            .data_with(payload, now, Some(EncryptionKeyField::NotEncrypted))
    };
    enc.raw_to_session(plain);
    enc.advance_by(SECOND_US / 2);
    assert_eq!(enc.session.datagram_stats().undecryptable, 2);

    let mut clear = clear_sim();
    clear.connect();
    let even = {
        let now = clear.now;
        clear
            .peer()
            .data_with(payload, now, Some(EncryptionKeyField::Even))
    };
    clear.raw_to_session(even);
    clear.advance_by(SECOND_US / 2);
    assert_eq!(clear.session.datagram_stats().undecryptable, 1);
    assert_eq!(
        clear.count(|e| matches!(e, Event::Sample { .. } | Event::Tracks(_))),
        0
    );
}

#[test]
fn a_payload_that_is_not_whole_ts_packets_is_dropped_and_counted() {
    let mut sim = clear_sim();
    sim.connect();
    sim.send_data(&[0x47; 100]);
    sim.advance_by(SECOND_US / 2);
    assert_eq!(sim.session.datagram_stats().malformed, 1);
}

#[test]
fn garbage_datagrams_before_and_after_the_handshake_are_ignored() {
    let mut sim = clear_sim();
    sim.raw_to_session(vec![0xFF; 3]);
    sim.advance_by(10_000);
    sim.connect();
    sim.raw_to_session(vec![0x00; 40]);
    sim.raw_to_session(vec![0x80; 7]);
    sim.advance_by(SECOND_US / 10);
    assert!(sim.session.datagram_stats().malformed >= 1);
    assert_ne!(last_status(&sim), Some(Status::Ended));
}

#[test]
fn a_copy_that_arrives_after_its_packet_was_delivered_is_counted_late() {
    let ts = small();
    let mut sim = clear_sim();
    sim.connect();
    stream(&mut sim, &ts, 7);
    sim.advance_by(SECOND_US);
    let old = sim.peer().sent_datagram(PEER_ISN + 3);
    sim.raw_to_session(old);
    sim.advance_by(SECOND_US / 10);
    assert_eq!(sim.session.datagram_stats().late, 1);
}

#[test]
fn the_session_announces_a_wake_for_the_next_ack_so_a_wake_driven_host_keeps_the_link() {
    let mut sim = clear_sim();
    sim.connect();
    for _ in 0..20 {
        sim.advance_by(1_000);
        let wake = sim.last_wake.expect("a wake is outstanding");
        assert!(
            wake <= sim.now + 12_000,
            "the next ACK is due within 10 ms, the wake is {} us away",
            wake.saturating_sub(sim.now)
        );
    }
}

#[test]
fn the_handshake_budget_is_the_configured_timeout_not_the_retransmit_count() {
    let mut sim = SrtSim::open(
        URL,
        cfg(|c| c.srt.handshake_timeout_ms = 1_000),
        SrtPeer::new(Cipher::Clear, None, LATENCY_MS),
    );
    sim.peer_listening = false;
    sim.advance_by(900_000);
    assert_eq!(
        sim.count(|e| matches!(
            e,
            Event::Error {
                kind: ErrorKind::SrtHandshakeTimeout,
                ..
            }
        )),
        0
    );
    sim.advance_by(200_000);
    assert_eq!(
        sim.count(|e| matches!(
            e,
            Event::Error {
                kind: ErrorKind::SrtHandshakeTimeout,
                ..
            }
        )),
        1
    );
}

#[test]
fn a_packet_far_beyond_the_flow_window_is_ignored_not_staged() {
    let ts = small();
    let want = baseline(&ts);
    let datagrams = ts_datagrams(&ts, 7);
    let mut sim = clear_sim();
    sim.connect();
    for (i, d) in datagrams.iter().enumerate() {
        sim.send_data(d);
        if i == 10 {
            let now = sim.now;
            let far = sim.peer().far_ahead_datagram(&datagrams[0], 20_000, now);
            sim.raw_to_session(far);
        }
        sim.advance_by(DATA_INTERVAL_US);
    }
    finish(&mut sim);
    assert_eq!(sim.replay().samples(), want);
    assert_eq!(sim.session.datagram_stats().lost, 0);
    assert_eq!(
        sim.peer().naks_received,
        0,
        "a crafted sequence number must not open a NAK gap"
    );
}

#[test]
fn a_gap_is_reported_at_once_not_at_the_next_periodic_nak() {
    let ts = small();
    let datagrams = ts_datagrams(&ts, 7);
    let mut sim = clear_sim();
    sim.connect();
    sim.send_data(&datagrams[0]);
    sim.advance_by(10_000);
    let skipped = {
        let now = sim.now;
        sim.peer().data(&datagrams[1], now)
    };
    drop(skipped);
    sim.send_data(&datagrams[2]);
    sim.advance_by(2_500);
    assert_eq!(
        sim.peer().naks_received,
        0,
        "the datagram is still in flight"
    );
    sim.advance_by(2_500);
    assert!(
        sim.peer().naks_received >= 1,
        "the gap is NAKed on arrival, well before the 20 ms periodic NAK"
    );
}

#[test]
fn a_rejection_is_reported_from_the_callback_without_a_poll() {
    use broadcast_client::Action;
    let mut peer = SrtPeer::new(Cipher::Aes128, Some(PASS), LATENCY_MS);
    let mut session = Session::open(
        URL,
        {
            let mut c = encrypted(SrtKeyLength::Aes128);
            c.srt.encryption = Some(SrtEncryption::new("wrong passphrase", SrtKeyLength::Aes128));
            c
        },
        0,
    )
    .unwrap();
    let mut conn = None;
    while let Some(a) = session.poll(0) {
        if let Action::UdpBind { conn: c, .. } = a {
            conn = Some(c);
        }
    }
    let conn = conn.unwrap();
    session.on_connected(conn);
    let mut now = 0;
    let mut inflight: Vec<Vec<u8>> = Vec::new();
    for _ in 0..10 {
        while let Some(a) = session.poll(now) {
            if let Action::Send { bytes, .. } = a {
                inflight.push(bytes);
            }
        }
        for b in inflight.drain(..) {
            for reply in peer.on_datagram(&b, now) {
                now += 1_000;
                session.on_data(conn, &reply, now);
                if let Some(Event::Error { kind, .. }) = session.next_event() {
                    assert!(matches!(kind, ErrorKind::SrtRejected { .. }));
                    return;
                }
            }
        }
        now += 1_000;
    }
    panic!("no rejection surfaced");
}
