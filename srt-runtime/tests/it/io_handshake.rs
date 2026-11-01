//! Adapter handshake and connection-lifetime behaviour against scripted raw
//! UDP peers (r08-SRT-W5/W6/W9/W10/W12).
//!
//! Each test plays one side of the handshake by hand over a plain
//! `tokio::net::UdpSocket`, using the sans-IO [`ListenerHandshake`] /
//! [`CallerHandshake`] engines only to produce correctly formed packets, so it
//! can drop, duplicate, delay and corrupt exactly the datagrams the behaviour
//! under test depends on. Every wait is a bounded `tokio::time::timeout`;
//! nothing sleeps to "let the other side catch up".

#![cfg(feature = "tokio")]

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use srt_runtime::Error;
use srt_runtime::caller::{CallerHandshake, CallerHandshakeState};
use srt_runtime::handshake_sm::{HandshakeConfig, HandshakeOutput, RejectionReason};
use srt_runtime::io::{SrtListener, SrtSocket};
use srt_runtime::listener::ListenerHandshake;
use srt_runtime::packet::{
    ControlPacket, DataPacket, EncryptionField, EncryptionKeyField, HandshakeExtensionFlags,
    HandshakeExtensions, HandshakePacket, HandshakeType, PacketPosition, SrtPacket,
};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

/// Generous upper bound for any single wait on loopback.
const WAIT: Duration = Duration::from_secs(10);
const LISTENER_ID: u32 = 0x0123_4567;
const LISTENER_COOKIE: u32 = 0x0BAD_C0DE;

async fn recv_from(raw: &UdpSocket) -> (Vec<u8>, SocketAddr) {
    let mut buf = [0u8; 2048];
    let (n, src) = tokio::time::timeout(WAIT, raw.recv_from(&mut buf))
        .await
        .expect("timed out waiting for a datagram")
        .expect("recv_from");
    (buf[..n].to_vec(), src)
}

/// The next *handshake* datagram, skipping the ACK / Keep-Alive traffic an
/// already-accepted connection keeps sending to this address.
async fn recv_handshake(raw: &UdpSocket) -> (Vec<u8>, SocketAddr) {
    loop {
        let (datagram, src) = recv_from(raw).await;
        if matches!(
            SrtPacket::parse(&datagram),
            Ok(SrtPacket::Control(ControlPacket::Handshake(_)))
        ) {
            return (datagram, src);
        }
    }
}

fn listener_engine() -> ListenerHandshake {
    ListenerHandshake::new(LISTENER_ID, LISTENER_COOKIE, HandshakeConfig::default())
}

/// Feed `datagram` to `engine` and return the datagram it wants sent, if any.
fn answer(engine: &mut ListenerHandshake, datagram: &[u8]) -> Option<Vec<u8>> {
    engine
        .feed_bytes(datagram)
        .expect("feed")
        .into_iter()
        .find_map(|o| match o {
            HandshakeOutput::Send(b) => Some(b),
            _ => None,
        })
}

fn handshake_of(datagram: &[u8]) -> (HandshakeType, u32) {
    match SrtPacket::parse(datagram).expect("parse") {
        SrtPacket::Control(ControlPacket::Handshake(hp)) => (hp.handshake_type, hp.srt_socket_id),
        other => panic!("expected a handshake, got {other:?}"),
    }
}

/// r08-SRT-W10: a lost INDUCTION is re-sent after ~250 ms, not after fifteen
/// seconds (three five-second receive timeouts).
#[tokio::test]
async fn a_lost_induction_is_retransmitted_within_a_second() {
    let raw = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = raw.local_addr().unwrap();
    let connect = tokio::spawn(SrtSocket::connect(addr, HandshakeConfig::default()));

    // The first INDUCTION is "lost": read it, answer nothing.
    let (first, src) = recv_from(&raw).await;
    let first_at = Instant::now();
    assert_eq!(handshake_of(&first).0, HandshakeType::Induction);

    let (second, _) = recv_from(&raw).await;
    let gap = first_at.elapsed();
    assert_eq!(second, first, "the retransmission is the same INDUCTION");
    assert!(
        gap < Duration::from_secs(1),
        "the INDUCTION retransmit took {gap:?}; libsrt re-sends every 250 ms"
    );

    // Complete the handshake so the connect future resolves cleanly.
    let mut engine = listener_engine();
    let induction_response = answer(&mut engine, &second).expect("INDUCTION response");
    raw.send_to(&induction_response, src).await.unwrap();
    let (conclusion, _) = recv_from(&raw).await;
    let conclusion_response = answer(&mut engine, &conclusion).expect("CONCLUSION response");
    raw.send_to(&conclusion_response, src).await.unwrap();
    let socket = tokio::time::timeout(WAIT, connect)
        .await
        .expect("connect never finished")
        .unwrap()
        .expect("connect");
    assert_eq!(socket.peer_addr(), addr);
}

/// r08-SRT-W10: a duplicate INDUCTION response (the Listener answers every
/// repeated INDUCTION, and a delayed first answer can trail the second) must
/// not abort the connect — it used to be judged Rogue once the CONCLUSION was
/// out.
#[tokio::test]
async fn a_duplicate_induction_response_does_not_abort_the_connect() {
    let raw = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = raw.local_addr().unwrap();
    let connect = tokio::spawn(SrtSocket::connect(addr, HandshakeConfig::default()));

    let (induction, src) = recv_from(&raw).await;
    let mut engine = listener_engine();
    let response = answer(&mut engine, &induction).expect("INDUCTION response");
    // The same response twice, back to back: the caller handles them in order,
    // so the second reaches it already in its "CONCLUSION sent" state.
    raw.send_to(&response, src).await.unwrap();
    raw.send_to(&response, src).await.unwrap();

    let (conclusion, _) = recv_from(&raw).await;
    assert_eq!(handshake_of(&conclusion).0, HandshakeType::Conclusion);
    let conclusion_response = answer(&mut engine, &conclusion).expect("CONCLUSION response");
    raw.send_to(&conclusion_response, src).await.unwrap();

    tokio::time::timeout(WAIT, connect)
        .await
        .expect("connect never finished")
        .unwrap()
        .expect("a duplicate INDUCTION response must not fail the connect");
}

/// Datagrams that are not the peer's handshake — garbage and a Keep-Alive from
/// the peer's own address, a perfectly valid INDUCTION response from a
/// *different* address — are ignored, not fatal and not believed.
#[tokio::test]
async fn stray_datagrams_do_not_abort_or_hijack_the_connect() {
    let raw = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let impostor = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = raw.local_addr().unwrap();
    let connect = tokio::spawn(SrtSocket::connect(addr, HandshakeConfig::default()));

    let (induction, src) = recv_from(&raw).await;

    // Noise from the peer's own address.
    raw.send_to(&[0xFF; 3], src).await.unwrap();
    raw.send_to(&[0u8; 64], src).await.unwrap();
    // A well-formed INDUCTION response, but from the wrong address: if it
    // were believed the caller would move on to a CONCLUSION aimed at it.
    let mut impostor_engine = ListenerHandshake::new(0x0666, 0x0666, HandshakeConfig::default());
    let fake = answer(&mut impostor_engine, &induction).expect("response");
    impostor.send_to(&fake, src).await.unwrap();

    let mut engine = listener_engine();
    let response = answer(&mut engine, &induction).expect("INDUCTION response");
    raw.send_to(&response, src).await.unwrap();
    let (conclusion, from) = recv_from(&raw).await;
    assert_eq!(from, src);
    let conclusion_response = answer(&mut engine, &conclusion).expect("CONCLUSION response");
    raw.send_to(&conclusion_response, src).await.unwrap();

    let socket = tokio::time::timeout(WAIT, connect)
        .await
        .expect("connect never finished")
        .unwrap()
        .expect("stray datagrams must not fail the connect");
    assert_eq!(socket.peer_addr(), addr, "connected to the real peer");
}

fn rejection(to_socket_id: u32, reason: RejectionReason) -> Vec<u8> {
    let pkt = ControlPacket::Handshake(HandshakePacket {
        timestamp: 0,
        dest_socket_id: to_socket_id,
        version: 5,
        encryption_field: EncryptionField::NoEncryption,
        extension_field: HandshakeExtensionFlags(0),
        initial_seq_number: 0,
        mtu: 1500,
        max_flow_window_size: 8192,
        handshake_type: reason.to_handshake_type(),
        srt_socket_id: LISTENER_ID,
        syn_cookie: 0,
        peer_ip: [0; 4],
        extensions: HandshakeExtensions(&[]),
    });
    let mut buf = vec![0u8; pkt.serialized_len()];
    pkt.serialize_into(&mut buf).unwrap();
    buf
}

/// r08-SRT-W12: the peer's rejection reason reaches the caller as a typed
/// error, so a refused backlog is distinguishable from a wrong secret.
#[tokio::test]
async fn a_peer_rejection_surfaces_its_reason() {
    for reason in [
        RejectionReason::Backlog,
        RejectionReason::BadSecret,
        RejectionReason::Version,
    ] {
        let raw = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr = raw.local_addr().unwrap();
        let connect = tokio::spawn(SrtSocket::connect(addr, HandshakeConfig::default()));
        let (induction, src) = recv_from(&raw).await;
        let (_, caller_id) = handshake_of(&induction);
        raw.send_to(&rejection(caller_id, reason), src)
            .await
            .unwrap();
        let err = tokio::time::timeout(WAIT, connect)
            .await
            .expect("connect never finished")
            .unwrap()
            .expect_err("a rejected connect must fail");
        assert_eq!(err, Error::Rejected(reason), "reason {reason:?}");
    }
}

/// r08-SRT-W12: a peer that never answers ends the connect with
/// `HandshakeTimedOut`, not a generic invalid-field error.
#[tokio::test]
async fn a_silent_peer_times_out_with_a_typed_error() {
    let silent = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let config = HandshakeConfig {
        retransmit_after_ticks: 1,
        max_retries: 2,
        ..HandshakeConfig::default()
    };
    let err = tokio::time::timeout(
        WAIT,
        SrtSocket::connect(silent.local_addr().unwrap(), config),
    )
    .await
    .expect("the retry budget must end the connect")
    .expect_err("nobody answered");
    assert_eq!(
        err,
        Error::HandshakeTimedOut {
            stage: "caller retransmit budget"
        }
    );
}

/// An INDUCTION first, then drive a [`CallerHandshake`] by hand against a
/// running [`SrtListener`]; returns the raw socket, the engine and the bytes
/// of the CONCLUSION the caller sent.
async fn raw_caller_up_to_conclusion(
    listener_addr: SocketAddr,
    config: HandshakeConfig,
    caller_id: u32,
) -> (UdpSocket, CallerHandshake, Vec<u8>) {
    let raw = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mut hs = CallerHandshake::new(caller_id, config);
    let induction = hs.start().unwrap();
    raw.send_to(&induction, listener_addr).await.unwrap();
    let (response, _) = recv_from(&raw).await;
    let conclusion = hs
        .feed_bytes(&response)
        .unwrap()
        .into_iter()
        .find_map(|o| match o {
            HandshakeOutput::Send(b) => Some(b),
            _ => None,
        })
        .expect("CONCLUSION");
    (raw, hs, conclusion)
}

/// r08-SRT-W5: the Listener's CONCLUSION response is lost; the Caller repeats
/// its CONCLUSION, and the Listener — which has already handed the connection
/// to the application — must answer with the very same response so the Caller
/// can finish. (It used to say nothing: one lost datagram left the Caller
/// timing out against a connection the Listener considered established.)
#[tokio::test]
async fn a_repeated_conclusion_is_answered_after_the_connection_was_accepted() {
    let mut listener = SrtListener::bind("127.0.0.1:0", HandshakeConfig::default())
        .await
        .unwrap();
    let listener_addr = listener.local_addr().unwrap();
    // A server loop, as any real one is: keep accepting.
    let (accepted_tx, mut accepted_rx) = mpsc::unbounded_channel();
    let accept_loop = tokio::spawn(async move {
        while let Ok(socket) = listener.accept().await {
            if accepted_tx.send(socket).is_err() {
                break;
            }
        }
    });

    let caller_id = 0x0000_0777;
    let (raw, mut hs, conclusion) =
        raw_caller_up_to_conclusion(listener_addr, HandshakeConfig::default(), caller_id).await;

    // First CONCLUSION: the response comes back but the "network loses it" —
    // the caller engine is never fed it.
    raw.send_to(&conclusion, listener_addr).await.unwrap();
    let (lost_response, _) = recv_handshake(&raw).await;
    assert_eq!(handshake_of(&lost_response).0, HandshakeType::Conclusion);
    let accepted = tokio::time::timeout(WAIT, accepted_rx.recv())
        .await
        .expect("the listener never accepted")
        .expect("accept loop ended");

    // The Caller repeats itself and must get the identical response.
    raw.send_to(&conclusion, listener_addr).await.unwrap();
    let (again, _) = recv_handshake(&raw).await;
    assert_eq!(
        again, lost_response,
        "the repeat must be answered identically"
    );

    // And that response lets the caller engine finish the handshake.
    let outputs = hs.feed_bytes(&again).unwrap();
    assert!(
        outputs
            .iter()
            .any(|o| matches!(o, HandshakeOutput::Connected(_)))
    );
    assert_eq!(hs.state(), CallerHandshakeState::Connected);

    // The application got exactly one connection out of all this.
    assert!(accepted_rx.try_recv().is_err(), "no duplicate connection");
    drop(accepted);
    accept_loop.abort();
}

/// A CONCLUSION naming a *different* Caller Socket ID from an
/// already-accepted address is not a repeat and must get no answer — the
/// remembered response is not an oracle for strangers.
#[tokio::test]
async fn a_conclusion_with_another_socket_id_is_not_answered_from_memory() {
    let mut listener = SrtListener::bind("127.0.0.1:0", HandshakeConfig::default())
        .await
        .unwrap();
    let listener_addr = listener.local_addr().unwrap();
    let (accepted_tx, mut accepted_rx) = mpsc::unbounded_channel();
    let accept_loop = tokio::spawn(async move {
        while let Ok(socket) = listener.accept().await {
            if accepted_tx.send(socket).is_err() {
                break;
            }
        }
    });

    let (raw, _hs, conclusion) =
        raw_caller_up_to_conclusion(listener_addr, HandshakeConfig::default(), 0x0000_0777).await;
    raw.send_to(&conclusion, listener_addr).await.unwrap();
    let (_response, _) = recv_handshake(&raw).await;
    let _accepted = tokio::time::timeout(WAIT, accepted_rx.recv())
        .await
        .expect("the listener never accepted");

    // The same CONCLUSION bytes with the Caller Socket ID (the CIF's
    // `SRT Socket ID` word, 24 bytes into the CIF after the 16-byte header)
    // rewritten to someone else's.
    let mut forged = conclusion.clone();
    let id_offset = 16 + 24;
    forged[id_offset..id_offset + 4].copy_from_slice(&0x0000_0888u32.to_be_bytes());
    raw.send_to(&forged, listener_addr).await.unwrap();
    // The accepted connection keeps sending ACKs to this address, so look
    // only for a *handshake* answer, for a bounded while.
    let answered = tokio::time::timeout(Duration::from_millis(400), async {
        let mut buf = [0u8; 2048];
        loop {
            let (n, _) = raw.recv_from(&mut buf).await.expect("recv_from");
            if matches!(
                SrtPacket::parse(&buf[..n]),
                Ok(SrtPacket::Control(ControlPacket::Handshake(_)))
            ) {
                break;
            }
        }
    })
    .await;
    assert!(
        answered.is_err(),
        "a CONCLUSION from an unrelated socket must not be answered"
    );
    accept_loop.abort();
}

/// r08-SRT-W6: dropping a connected socket sends the peer a SHUTDOWN (§3.2.7)
/// addressed to the peer's socket id.
#[tokio::test]
async fn dropping_a_socket_sends_the_peer_a_shutdown() {
    let raw = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = raw.local_addr().unwrap();
    let connect = tokio::spawn(SrtSocket::connect(addr, HandshakeConfig::default()));
    let (induction, src) = recv_from(&raw).await;
    let mut engine = listener_engine();
    raw.send_to(&answer(&mut engine, &induction).unwrap(), src)
        .await
        .unwrap();
    let (conclusion, _) = recv_from(&raw).await;
    raw.send_to(&answer(&mut engine, &conclusion).unwrap(), src)
        .await
        .unwrap();
    let socket = tokio::time::timeout(WAIT, connect)
        .await
        .expect("connect never finished")
        .unwrap()
        .expect("connect");

    drop(socket);

    // Skip the ACK/Keep-Alive traffic the connection produced; a SHUTDOWN for
    // the listener's own socket id must show up.
    let shutdown = tokio::time::timeout(WAIT, async {
        loop {
            let (datagram, _) = recv_from(&raw).await;
            if let Ok(SrtPacket::Control(ControlPacket::Shutdown(s))) = SrtPacket::parse(&datagram)
            {
                break s;
            }
        }
    })
    .await
    .expect("no SHUTDOWN arrived after the socket was dropped");
    assert_eq!(shutdown.dest_socket_id, LISTENER_ID);
}

async fn connected_pair() -> (SrtSocket, SrtSocket) {
    let mut listener = SrtListener::bind("127.0.0.1:0", HandshakeConfig::default())
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let accept = tokio::spawn(async move { listener.accept().await.map(|s| (listener, s)) });
    let caller = SrtSocket::connect(addr, HandshakeConfig::default())
        .await
        .expect("connect");
    let (_listener, accepted) = tokio::time::timeout(WAIT, accept)
        .await
        .expect("accept never finished")
        .unwrap()
        .expect("accept");
    (caller, accepted)
}

/// r08-SRT-W6: the peer learns of a close at once. The accepted side's
/// `recv` ends promptly — well inside the five-second idle timeout it would
/// otherwise have had to wait out.
#[tokio::test]
async fn the_peer_of_a_dropped_socket_sees_the_close_immediately() {
    let (caller, mut accepted) = connected_pair().await;
    drop(caller);
    let ended = tokio::time::timeout(Duration::from_secs(2), accepted.recv())
        .await
        .expect("recv must end on SHUTDOWN, not wait for the 5 s idle timeout")
        .expect("recv");
    assert_eq!(ended, None);
}

/// r08-SRT-W9: the TSBPD time base is seeded from the peer's handshake
/// timestamp (rule 12). A Caller whose clock already reads 30 s when it sends
/// its CONCLUSION stamps its first data packet ~30 s too; the Listener must
/// deliver it one latency after arrival, not 30 s later (a hard-coded base of
/// zero made the play time `0 + 30 s + latency`).
#[tokio::test]
async fn a_caller_far_into_its_own_clock_is_delivered_promptly() {
    const CALLER_CLOCK_US: u32 = 30_000_000;
    const CALLER_ISN: u32 = 4_242;
    let mut listener = SrtListener::bind("127.0.0.1:0", HandshakeConfig::default())
        .await
        .unwrap();
    let listener_addr = listener.local_addr().unwrap();
    let (accepted_tx, mut accepted_rx) = mpsc::unbounded_channel();
    let accept_loop = tokio::spawn(async move {
        while let Ok(socket) = listener.accept().await {
            if accepted_tx.send(socket).is_err() {
                break;
            }
        }
    });

    let config = HandshakeConfig {
        initial_seq_number: CALLER_ISN,
        ..HandshakeConfig::default()
    };
    let (raw, mut hs, mut conclusion) =
        raw_caller_up_to_conclusion(listener_addr, config, 0x0000_0999).await;
    // Stamp the CONCLUSION with the caller's (advanced) clock: `Timestamp`
    // is the third header word.
    conclusion[8..12].copy_from_slice(&CALLER_CLOCK_US.to_be_bytes());
    raw.send_to(&conclusion, listener_addr).await.unwrap();
    let (response, _) = recv_handshake(&raw).await;
    let (_, listener_socket_id) = handshake_of(&response);
    let _ = hs.feed_bytes(&response).unwrap();
    let mut accepted = tokio::time::timeout(WAIT, accepted_rx.recv())
        .await
        .expect("the listener never accepted")
        .expect("accept loop ended");

    let payload = b"timestamped by a clock 30 s in";
    let pkt = DataPacket {
        seq_number: CALLER_ISN,
        position: PacketPosition::Solo,
        in_order: true,
        key_flag: EncryptionKeyField::NotEncrypted,
        retransmitted: false,
        message_number: 1,
        timestamp: CALLER_CLOCK_US + 1_000,
        dest_socket_id: listener_socket_id,
        data: payload,
    };
    let mut wire = vec![0u8; pkt.serialized_len()];
    pkt.serialize_into(&mut wire).unwrap();
    raw.send_to(&wire, listener_addr).await.unwrap();

    let got = tokio::time::timeout(Duration::from_secs(5), accepted.recv())
        .await
        .expect("the packet was held for its raw 30 s timestamp")
        .expect("recv");
    assert_eq!(got.as_deref(), Some(payload.as_slice()));
    accept_loop.abort();
}

/// §4.3.1.2: the TSBPD latency a connection runs with is the greater of the
/// two parties' requests, on *both* roles. A payload sent over a link where
/// the far side asked for one second must not be delivered before roughly one
/// second has passed (an adapter that used only its own, shorter, latency would
/// deliver it after ~120 ms). Only a lower bound is asserted, so the test
/// cannot be made flaky by a slow machine.
#[tokio::test]
async fn the_negotiated_latency_is_the_greater_of_both_sides() {
    const LONG_MS: u16 = 1_000;
    const AT_LEAST: Duration = Duration::from_millis(900);
    let short = HandshakeConfig::default(); // 120 ms
    let long = HandshakeConfig {
        latency_ms: LONG_MS,
        ..HandshakeConfig::default()
    };

    // Role 1: the Caller asks for a long latency; the accepted (Listener-role)
    // socket receives.
    let mut listener = SrtListener::bind("127.0.0.1:0", short.clone())
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let accept = tokio::spawn(async move { listener.accept().await.map(|s| (listener, s)) });
    let mut caller = SrtSocket::connect(addr, long.clone())
        .await
        .expect("connect");
    let (_listener, mut accepted) = tokio::time::timeout(WAIT, accept)
        .await
        .expect("accept never finished")
        .unwrap()
        .expect("accept");
    let sent_at = Instant::now();
    caller.send(b"to the listener role").await.unwrap();
    let got = tokio::time::timeout(WAIT, accepted.recv())
        .await
        .expect("never delivered")
        .unwrap();
    assert_eq!(got.as_deref(), Some(b"to the listener role".as_slice()));
    assert!(
        sent_at.elapsed() >= AT_LEAST,
        "the listener role delivered after {:?}, ignoring the caller's {LONG_MS} ms",
        sent_at.elapsed()
    );

    // Role 2: the Listener asks for the long latency; the Caller receives.
    let mut listener = SrtListener::bind("127.0.0.1:0", long).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let accept = tokio::spawn(async move { listener.accept().await.map(|s| (listener, s)) });
    let mut caller = SrtSocket::connect(addr, short).await.expect("connect");
    let (_listener, mut accepted) = tokio::time::timeout(WAIT, accept)
        .await
        .expect("accept never finished")
        .unwrap()
        .expect("accept");
    let sent_at = Instant::now();
    accepted.send(b"to the caller role").await.unwrap();
    let got = tokio::time::timeout(WAIT, caller.recv())
        .await
        .expect("never delivered")
        .unwrap();
    assert_eq!(got.as_deref(), Some(b"to the caller role".as_slice()));
    assert!(
        sent_at.elapsed() >= AT_LEAST,
        "the caller role delivered after {:?}, ignoring the listener's {LONG_MS} ms",
        sent_at.elapsed()
    );
}

/// A server loop around a fresh [`SrtListener`] (default config) and a raw
/// caller that completes the handshake with `config`; returns the raw socket,
/// the listener's socket id, and a guard keeping the accepted connection (and
/// the accept loop) alive.
async fn raw_caller_connected(config: HandshakeConfig) -> (UdpSocket, SocketAddr, u32, impl Drop) {
    struct Keep(
        tokio::task::JoinHandle<()>,
        mpsc::UnboundedReceiver<SrtSocket>,
        Option<SrtSocket>,
    );
    impl Drop for Keep {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let mut listener = SrtListener::bind("127.0.0.1:0", HandshakeConfig::default())
        .await
        .unwrap();
    let listener_addr = listener.local_addr().unwrap();
    let (tx, rx) = mpsc::unbounded_channel();
    let accept_loop = tokio::spawn(async move {
        while let Ok(socket) = listener.accept().await {
            if tx.send(socket).is_err() {
                break;
            }
        }
    });
    let mut keep = Keep(accept_loop, rx, None);
    let (raw, mut hs, conclusion) =
        raw_caller_up_to_conclusion(listener_addr, config, 0x0000_0555).await;
    raw.send_to(&conclusion, listener_addr).await.unwrap();
    let (response, _) = recv_handshake(&raw).await;
    let (_, listener_socket_id) = handshake_of(&response);
    hs.feed_bytes(&response).unwrap();
    assert_eq!(hs.state(), CallerHandshakeState::Connected);
    // Keep the accepted connection alive: dropping it would shut it down.
    keep.2 = Some(
        tokio::time::timeout(WAIT, keep.1.recv())
            .await
            .expect("the listener never accepted")
            .expect("accept loop ended"),
    );
    (raw, listener_addr, listener_socket_id, keep)
}

/// A tiny DATA packet. Its timestamp is ~1000 s on the sender's clock, so its
/// play time is far off: TSBPD never skips a gap during these tests (a skip
/// would clear the loss list the periodic NAK is built from).
fn send_data(seq: u32, dest: u32) -> Vec<u8> {
    let pkt = DataPacket {
        seq_number: seq,
        position: PacketPosition::Solo,
        in_order: true,
        key_flag: EncryptionKeyField::NotEncrypted,
        retransmitted: false,
        message_number: 1,
        timestamp: 1_000_000_000,
        dest_socket_id: dest,
        data: b"x",
    };
    let mut wire = vec![0u8; pkt.serialized_len()];
    pkt.serialize_into(&mut wire).unwrap();
    wire
}

/// The NAK datagrams (length, entries) arriving at `raw`, skipping everything
/// else, until `done` says enough have been seen (bounded by `WAIT`).
async fn collect_naks(
    raw: &UdpSocket,
    mut done: impl FnMut(&[(usize, Vec<srt_runtime::packet::LossListEntry>)]) -> bool,
) -> Vec<(usize, Vec<srt_runtime::packet::LossListEntry>)> {
    let mut naks = Vec::new();
    tokio::time::timeout(WAIT, async {
        loop {
            let (datagram, _) = recv_from(raw).await;
            if let Ok(SrtPacket::Control(ControlPacket::Nak(n))) = SrtPacket::parse(&datagram) {
                let entries = n.entries().map(|e| e.expect("well-formed")).collect();
                naks.push((datagram.len(), entries));
                if done(&naks) {
                    break;
                }
            }
        }
    })
    .await
    .expect("the expected NAKs never arrived");
    naks
}

/// §3.2.1: a peer that advertised a 576-byte MTU is never sent a datagram
/// larger than that. 400 isolated losses make the periodic NAK ~1.6 kB; it
/// must arrive split into datagrams that each fit the *peer's* MTU, not the
/// listener's own 1500.
#[tokio::test]
async fn nak_datagrams_fit_the_peers_smaller_mtu() {
    const ISN: u32 = 1_000;
    const PEER_MTU: usize = 576;
    const IP_UDP: usize = 28;
    let (raw, listener_addr, id, _keep) = raw_caller_connected(HandshakeConfig {
        mtu: u32::try_from(PEER_MTU).unwrap(),
        initial_seq_number: ISN,
        ..HandshakeConfig::default()
    })
    .await;
    // Every other packet: ISN, ISN+2, ... — 400 isolated losses in between.
    for k in 0..=400u32 {
        raw.send_to(&send_data(ISN + 2 * k, id), listener_addr)
            .await
            .unwrap();
    }
    // Wait for periodic NAK chunks (those carry more than a handful of
    // entries); the immediate per-gap NAKs are one entry each.
    let naks = collect_naks(&raw, |seen| {
        seen.iter().filter(|(_, e)| e.len() > 20).count() >= 3
    })
    .await;
    for (len, _) in &naks {
        assert!(
            len + IP_UDP <= PEER_MTU,
            "a {len}-byte NAK does not fit the peer's {PEER_MTU}-byte MTU"
        );
    }
}

/// §3.2.1: a peer that advertised a 64-packet flow window is tracked only 64
/// packets ahead: a packet 100 ahead of the ack point is ignored, so the NAK
/// for a later packet 50 ahead names only the 50 missing before it.
#[tokio::test]
async fn the_peers_smaller_flow_window_bounds_what_is_tracked() {
    use srt_runtime::packet::LossListEntry;
    const ISN: u32 = 2_000;
    let (raw, listener_addr, id, _keep) = raw_caller_connected(HandshakeConfig {
        max_flow_window_size: 64,
        initial_seq_number: ISN,
        ..HandshakeConfig::default()
    })
    .await;
    raw.send_to(&send_data(ISN + 100, id), listener_addr)
        .await
        .unwrap();
    raw.send_to(&send_data(ISN + 50, id), listener_addr)
        .await
        .unwrap();
    let naks = collect_naks(&raw, |seen| !seen.is_empty()).await;
    assert_eq!(
        naks[0].1,
        vec![LossListEntry::Range(ISN, ISN + 49)],
        "the packet 100 ahead (outside the 64-packet window) must not have been tracked"
    );
}
