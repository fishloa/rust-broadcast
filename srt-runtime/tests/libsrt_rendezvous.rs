//! Real `libsrt` interop for the Rendezvous handshake (`draft-sharabayko-srt-01`
//! §4.3.2) over loopback UDP, through the tokio adapter
//! ([`SrtSocket::rendezvous`]).
//!
//! `tests/rendezvous_round_trip.rs` only runs the sans-IO engine against
//! ITSELF; the cookie contest, the Initiator/Responder message tables and the
//! missing-packet recovery rules are exactly where two implementations of an
//! ambiguous spec diverge, so the independent oracle here is a genuine
//! `srt-live-transmit` started in `mode=rendezvous` (the URI is
//! `srt://<peer ip>:<peer port>?mode=rendezvous&port=<local port>`). Each
//! test repeats the handshake several times: our side draws a fresh random
//! cookie every attempt, so across the iterations the contest lands both ways
//! (we are Initiator, libsrt is Responder, and vice versa) - a divergence in
//! either role's table fails an iteration.
//!
//! Skips itself LOUDLY (prints why, `--nocapture`) when `srt-live-transmit` is
//! not on `PATH` (so it is a no-op on CI hosts without libsrt, exactly like
//! `libsrt_interop.rs`); set `SRT_REQUIRE_LIBSRT=1` to make the skip a
//! failure.

#![cfg(feature = "tokio")]

use std::net::{SocketAddr, UdpSocket};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use srt_runtime::handshake_sm::HandshakeConfig;
use srt_runtime::io::{IoConfig, SrtSocket};
use tokio::net::UdpSocket as TokioUdpSocket;

include!("support/skip.rs");

const TEST_TIMEOUT: Duration = Duration::from_secs(120);
const PAYLOAD_LEN: usize = 1316;
/// Handshake attempts per test (random cookie each, so both roles occur).
const ITERATIONS: usize = 6;

/// A handshake budget generous enough for a loaded CI/dev host: the default
/// (12 retransmissions = ~3 s) can be eaten entirely by the time it takes to
/// spawn the external `srt-live-transmit` process under heavy machine load.
fn patient_config() -> HandshakeConfig {
    HandshakeConfig {
        max_retries: 60,
        ..HandshakeConfig::default()
    }
}

fn patient_io() -> IoConfig {
    IoConfig::default().with_handshake(Duration::from_secs(20))
}

fn free_udp_port() -> u16 {
    UdpSocket::bind("127.0.0.1:0")
        .expect("bind free port")
        .local_addr()
        .expect("local addr")
        .port()
}

fn pattern(n: usize, seed: u8) -> Vec<u8> {
    (0..n).map(|i| seed.wrapping_add(i as u8)).collect()
}

/// Kills the wrapped child on drop (panic, early return, or a timeout
/// dropping the future) so a real `srt-live-transmit` is never leaked.
struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn(cmd: &mut Command) -> KillOnDrop {
    KillOnDrop(
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn srt-live-transmit"),
    )
}

/// libsrt rendezvous URI: its remote is `peer_port`, its own bound port
/// `local_port`.
fn libsrt_rdv_uri(peer_port: u16, local_port: u16, options: &str) -> String {
    format!("srt://127.0.0.1:{peer_port}?mode=rendezvous&port={local_port}{options}")
}

async fn recv_at_least(discard: &TokioUdpSocket, want_len: usize, timeout: Duration) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = vec![0u8; 65536];
    let deadline = tokio::time::Instant::now() + timeout;
    while out.len() < want_len {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, discard.recv(&mut buf)).await {
            Ok(Ok(n)) => out.extend_from_slice(&buf[..n]),
            _ => break,
        }
    }
    out
}

fn numbered_chunk(index: u32) -> Vec<u8> {
    let mut c = pattern(PAYLOAD_LEN, 7);
    c[..4].copy_from_slice(&index.to_be_bytes());
    c
}

/// Our Rendezvous side completes a real handshake with a genuine libsrt
/// Rendezvous peer, then sends a burst that libsrt forwards byte-identical to
/// its UDP target.
#[tokio::test]
async fn our_rendezvous_completes_real_handshake_and_data_with_libsrt() {
    skip_unless_tools!("srt-live-transmit" => "-version");

    tokio::time::timeout(TEST_TIMEOUT, async {
        for iteration in 0..ITERATIONS {
            let our_port = free_udp_port();
            let lib_port = free_udp_port();
            let discard = TokioUdpSocket::bind("127.0.0.1:0")
                .await
                .expect("bind discard socket");
            let discard_port = discard.local_addr().unwrap().port();
            let _libsrt = spawn(
                Command::new("srt-live-transmit")
                    .arg("-loglevel:error")
                    .arg(libsrt_rdv_uri(our_port, lib_port, "&latency=120"))
                    .arg(format!("udp://127.0.0.1:{discard_port}")),
            );

            // No wait for the libsrt process to come up: both sides
            // retransmit their WAVEAHAND until the other answers.
            let mut ours = tokio::time::timeout(
                Duration::from_secs(25),
                SrtSocket::rendezvous_with(
                    format!("127.0.0.1:{our_port}")
                        .parse::<SocketAddr>()
                        .unwrap(),
                    format!("127.0.0.1:{lib_port}")
                        .parse::<SocketAddr>()
                        .unwrap(),
                    patient_config(),
                    patient_io(),
                ),
            )
            .await
            .unwrap_or_else(|_| panic!("iteration {iteration}: rendezvous never completed"))
            .unwrap_or_else(|e| panic!("iteration {iteration}: rendezvous with libsrt: {e}"));

            let burst = pattern(PAYLOAD_LEN * 8, iteration as u8);
            for chunk in burst.chunks(PAYLOAD_LEN) {
                ours.send(chunk).await.expect("send");
            }
            let got = recv_at_least(&discard, burst.len(), Duration::from_secs(10)).await;
            assert_eq!(
                got, burst,
                "iteration {iteration}: burst must be forwarded byte-identical by libsrt"
            );
            drop(ours);
        }
        eprintln!("PASS: our_rendezvous_completes_real_handshake_and_data_with_libsrt");
    })
    .await
    .expect("test timed out - real libsrt rendezvous stalled");
}

/// The other direction: a libsrt Rendezvous peer sources `udp://` data and
/// our side receives it through `recv()` after the real Rendezvous handshake.
#[tokio::test]
async fn real_libsrt_rendezvous_sender_delivers_data_to_our_rendezvous_side() {
    skip_unless_tools!("srt-live-transmit" => "-version");

    tokio::time::timeout(TEST_TIMEOUT, async {
        for iteration in 0..ITERATIONS {
            let our_port = free_udp_port();
            let lib_port = free_udp_port();
            let src_port = free_udp_port();
            let _libsrt = spawn(
                Command::new("srt-live-transmit")
                    .arg("-loglevel:error")
                    .arg(format!("-chunk:{PAYLOAD_LEN}"))
                    .arg(format!("udp://:{src_port}"))
                    .arg(libsrt_rdv_uri(our_port, lib_port, "&latency=120")),
            );

            let mut ours = tokio::time::timeout(
                Duration::from_secs(25),
                SrtSocket::rendezvous_with(
                    format!("127.0.0.1:{our_port}").parse::<SocketAddr>().unwrap(),
                    format!("127.0.0.1:{lib_port}")
                        .parse::<SocketAddr>()
                        .unwrap(),
                    patient_config(),
                    patient_io(),
                ),
            )
            .await
            .unwrap_or_else(|_| panic!("iteration {iteration}: rendezvous never completed"))
            .unwrap_or_else(|e| panic!("iteration {iteration}: rendezvous with libsrt: {e}"));

            // libsrt discards what its `udp://` source delivers while its SRT
            // side is still connecting, and there is no "connected" signal
            // for it: feed every 50 ms until 6 chunks have come out.
            let feed = TokioUdpSocket::bind("127.0.0.1:0").await.unwrap();
            let mut next = 0u32;
            let mut received: Vec<u32> = Vec::new();
            let mut feeder = tokio::time::interval(Duration::from_millis(50));
            let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
            while received.len() < 6 {
                tokio::select! {
                    _ = feeder.tick() => {
                        feed.send_to(&numbered_chunk(next), ("127.0.0.1", src_port))
                            .await
                            .unwrap();
                        next += 1;
                    }
                    r = ours.recv() => {
                        let payload = r.expect("recv").expect("connection ended");
                        assert_eq!(payload.len(), PAYLOAD_LEN);
                        let index = u32::from_be_bytes(payload[..4].try_into().unwrap());
                        assert_eq!(payload, numbered_chunk(index), "chunk {index} corrupted");
                        received.push(index);
                    }
                    _ = tokio::time::sleep_until(deadline) => {
                        panic!("iteration {iteration}: no data (got {received:?} after feeding {next})");
                    }
                }
            }
            assert!(
                received.windows(2).all(|w| w[1] == w[0] + 1),
                "iteration {iteration}: chunks must arrive in order, got {received:?}"
            );
            drop(ours);
        }
        eprintln!("PASS: real_libsrt_rendezvous_sender_delivers_data_to_our_rendezvous_side");
    })
    .await
    .expect("test timed out - real libsrt rendezvous stalled");
}
