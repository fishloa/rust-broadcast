//! Real `libsrt` interop over loopback UDP, both directions (libsrt interop).
//!
//! `tests/libsrt_fixtures.rs` already proves real libsrt KEEPALIVE/ACKACK
//! bytes parse correctly (issue #1060). This file proves the *handshake and
//! data path* actually completes end-to-end against a genuine libsrt peer —
//! which, until this fix, it did not in the caller direction: our
//! `CallerHandshake` echoed the Listener's own captured Socket ID back as
//! the CONCLUSION's `dest_socket_id` instead of `0` (libsrt's real Caller
//! always sends `0` there — verified against `srtcore/core.cpp`
//! `CUDT::processAsyncConnectRequest`, cited in `caller.rs`), which a real
//! libsrt Listener silently drops, stalling the handshake forever right
//! after the INDUCTION round (a genuine bug in *our* code, not the libsrt
//! build's own `ConsoleSource` defect `tests/libsrt_fixtures.rs`'s sibling
//! investigation ran into).
//!
//! Skips itself loudly (prints why, `--nocapture`) when `srt-live-transmit`
//! is not on `PATH`, so this is a no-op pass on a host without it.

#![cfg(feature = "tokio")]

use std::process::{Child, Command, Stdio};
use std::time::Duration;

use srt_runtime::handshake_sm::HandshakeConfig;
use srt_runtime::io::{SrtListener, SrtSocket};
use tokio::net::UdpSocket as TokioUdpSocket;

const TEST_TIMEOUT: Duration = Duration::from_secs(30);
const PAYLOAD_LEN: usize = 1316;

fn srt_live_transmit_available() -> bool {
    Command::new("srt-live-transmit")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success() || !o.stdout.is_empty() || !o.stderr.is_empty())
}

macro_rules! skip_unless_srt_live_transmit_available {
    () => {
        if !srt_live_transmit_available() {
            eprintln!(
                "SKIP libsrt_interop: `srt-live-transmit` not on PATH (libsrt/srt package, \
                 e.g. `brew install srt`). This test is a no-op result on this host, not real \
                 coverage — install it to get the genuine libsrt-interop check."
            );
            return;
        }
    };
}

async fn free_udp_port() -> u16 {
    let s = TokioUdpSocket::bind("127.0.0.1:0")
        .await
        .expect("bind free port");
    s.local_addr().expect("local addr").port()
}

fn pattern(n: usize, seed: u8) -> Vec<u8> {
    (0..n).map(|i| seed.wrapping_add(i as u8)).collect()
}

/// Kills the wrapped child on drop (panic, early `?`, or a `tokio::time::timeout`
/// dropping the future all count) — a plain `.kill()` at the end of the async
/// block is skipped by any of those paths, leaking a real `srt-live-transmit`
/// process that then spins retrying forever.
struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
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

/// Our [`SrtSocket`] (caller) connects to a genuine `srt-live-transmit`
/// listener, sends two bursts separated by an idle window, and both must
/// arrive byte-identical through libsrt's UDP target. Pre-fix: the
/// handshake never got past the caller's CONCLUSION (silently dropped by
/// the real Listener) — `caller.connect` timed out.
#[tokio::test]
async fn our_caller_completes_real_handshake_and_data_with_libsrt_listener() {
    skip_unless_srt_live_transmit_available!();

    tokio::time::timeout(TEST_TIMEOUT, async {
        let listen_port = free_udp_port().await;
        let discard_port = free_udp_port().await;
        let discard = TokioUdpSocket::bind(("127.0.0.1", discard_port))
            .await
            .expect("bind discard socket");

        let _listener_proc = KillOnDrop(
            Command::new("srt-live-transmit")
                .arg("-loglevel:error")
                .arg(format!("srt://:{listen_port}"))
                .arg(format!("udp://127.0.0.1:{discard_port}"))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn srt-live-transmit listener"),
        );

        tokio::time::sleep(Duration::from_millis(300)).await;

        let mut caller = tokio::time::timeout(
            Duration::from_secs(10),
            SrtSocket::connect(
                format!("127.0.0.1:{listen_port}")
                    .parse::<std::net::SocketAddr>()
                    .unwrap(),
                HandshakeConfig::default(),
            ),
        )
        .await
        .expect("caller connect timed out — handshake never completed")
        .expect("caller connect to real libsrt listener");

        let first_burst = pattern(PAYLOAD_LEN * 10, 0);
        for c in first_burst.chunks(PAYLOAD_LEN) {
            caller.send(c).await.expect("send first burst chunk");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let received_first =
            recv_at_least(&discard, first_burst.len(), Duration::from_secs(10)).await;
        assert_eq!(
            received_first, first_burst,
            "first burst must be forwarded byte-identical by the real libsrt listener"
        );

        // Idle window: real KEEPALIVE/ACK traffic crosses the wire.
        tokio::time::sleep(Duration::from_secs(3)).await;

        let second_burst = pattern(PAYLOAD_LEN * 5, 100);
        for c in second_burst.chunks(PAYLOAD_LEN) {
            caller.send(c).await.expect("send second burst chunk");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let received_second =
            recv_at_least(&discard, second_burst.len(), Duration::from_secs(10)).await;
        assert_eq!(
            received_second, second_burst,
            "second burst (after the idle window) must still arrive byte-identical"
        );

        eprintln!("PASS: our_caller_completes_real_handshake_and_data_with_libsrt_listener");

        drop(caller);
    })
    .await
    .expect("test timed out — real libsrt listener/session stalled");
}

/// A genuine `srt-live-transmit` caller connects to our [`SrtListener`],
/// completing the real HSv5 handshake and delivering a real DATA payload
/// through the fixed `ingress()` path. (This direction already worked before
/// this fix — kept as a regression test per the review.)
///
/// The source is `udp://:PORT`, not `file://con` (stdin) or an arbitrary
/// `file://` path (`--help`'s own "Supported schemes" only lists `file://con`
/// — any other path is "Unsupported source type"). Independently reproduced
/// against two genuine `srt-live-transmit` processes with no code from this
/// crate involved at all (real listener, real caller, `file://con` source):
/// zero bytes ever reach the `srt://` output, pipe-buffered-before-spawn or
/// not — a real libsrt/macOS `ConsoleSource` defect (see
/// `tests/libsrt_fixtures.rs`'s module doc) that plain `udp://` sourcing
/// avoids entirely, confirmed against the same two real processes to forward
/// the full burst byte-identical.
#[tokio::test]
async fn real_libsrt_caller_completes_handshake_and_data_with_our_listener() {
    skip_unless_srt_live_transmit_available!();

    tokio::time::timeout(TEST_TIMEOUT, async {
        let listener_addr = "127.0.0.1:0".parse::<std::net::SocketAddr>().unwrap();
        let mut listener = SrtListener::bind(listener_addr, HandshakeConfig::default())
            .await
            .expect("listener bind");
        let bound_addr = listener.local_addr().expect("listener local addr");

        let accept_jh =
            tokio::spawn(async move { listener.accept().await.expect("listener accept") });

        let src_port = free_udp_port().await;
        let _caller = KillOnDrop(
            Command::new("srt-live-transmit")
                .arg("-loglevel:error")
                .arg(format!("-chunk:{PAYLOAD_LEN}"))
                .arg(format!("udp://:{src_port}"))
                .arg(format!("srt://127.0.0.1:{}", bound_addr.port()))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn srt-live-transmit caller"),
        );
        // Give the caller a moment to bind its udp:// source before we feed it.
        tokio::time::sleep(Duration::from_millis(300)).await;

        let feed = TokioUdpSocket::bind("127.0.0.1:0")
            .await
            .expect("bind feed socket");
        let burst = pattern(PAYLOAD_LEN * 10, 0);
        for c in burst.chunks(PAYLOAD_LEN) {
            feed.send_to(c, ("127.0.0.1", src_port))
                .await
                .expect("feed burst chunk");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }

        let mut receiver = tokio::time::timeout(Duration::from_secs(10), accept_jh)
            .await
            .expect("accept timed out — real libsrt caller handshake never completed")
            .expect("join listener accept");

        let mut received = Vec::new();
        while received.len() < burst.len() {
            match tokio::time::timeout(Duration::from_secs(15), receiver.recv())
                .await
                .expect("recv timed out")
                .expect("receiver recv")
            {
                Some(p) => received.extend_from_slice(&p),
                None => break,
            }
        }
        assert_eq!(
            received, burst,
            "real libsrt caller's data must arrive byte-identical through our listener"
        );

        eprintln!("PASS: real_libsrt_caller_completes_handshake_and_data_with_our_listener");

        drop(receiver);
    })
    .await
    .expect("test timed out — real libsrt handshake/session stalled");
}
