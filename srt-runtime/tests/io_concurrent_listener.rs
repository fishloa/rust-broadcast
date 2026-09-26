//! Two concurrent callers into one listener (issue #1029, C8) — regression
//! for `SrtListener` sharing its one bound socket across `accept()` and
//! every connection it has already accepted. Before the fix, each accepted
//! connection's driver task called `udp.recv_from` directly on that shared
//! socket, racing every other task (the listener's own `accept` loop, and
//! every *other* accepted connection's driver) for the same socket's next
//! datagram — so a datagram could be handed to the wrong task entirely and
//! silently lost for its rightful connection.
//!
//! This test connects two real callers to one listener at the same time and
//! sends each stream's payloads interleaved (`tokio::join!`, not
//! sequentially), then asserts each accepted connection received its *own*
//! caller's payloads complete and byte-identical — cross-talk or loss would
//! show up as a missing/wrong payload on one side.
//!
//! HANG GUARD: wrapped in [`tokio::time::timeout`] (see `io_loopback.rs`).
//!
//! **Pre-fix run** (`git show HEAD:srt-runtime/src/io.rs` swapped in over
//! the working copy, then restored — never `git stash`, shared worktree):
//! `recv_a timed out: Elapsed(())` after 10s — one side's payloads were
//! misrouted/dropped often enough that delivery stalled well short of
//! `NUM_PAYLOADS`.

#![cfg(feature = "tokio")]

use core::time::Duration;

use srt_runtime::handshake_sm::HandshakeConfig;
use srt_runtime::io::{SrtListener, SrtSocket};
use tokio::net::UdpSocket as TokioUdpSocket;

const TEST_TIMEOUT: Duration = Duration::from_secs(30);
const NUM_PAYLOADS: usize = 30;

/// Grab a free local port by binding then dropping (see `libsrt_interop.rs`
/// for the same trick), so each caller can be given a distinct, known local
/// address up front — letting the accepted connections' `peer_addr()` be
/// matched back to "caller A" / "caller B" deterministically instead of by
/// arrival order (which the very bug under test can scramble).
async fn free_local_addr() -> std::net::SocketAddr {
    let s = TokioUdpSocket::bind("127.0.0.1:0")
        .await
        .expect("bind free port");
    s.local_addr().expect("local addr")
}

/// A recognizable, position- and stream-dependent payload — not all-zero, so
/// a cross-talk (wrong stream) or corruption is detectable byte-for-byte.
fn payload(stream: u8, i: usize) -> Vec<u8> {
    let mut v = vec![stream; 64];
    v[1] = i as u8;
    v
}

#[tokio::test]
async fn two_concurrent_callers_do_not_cross_talk() {
    tokio::time::timeout(TEST_TIMEOUT, async {
        let listener_addr = "127.0.0.1:0".parse::<std::net::SocketAddr>().unwrap();
        let mut listener = SrtListener::bind(listener_addr, HandshakeConfig::default())
            .await
            .expect("listener bind");
        let bound_addr = listener.local_addr().expect("listener local addr");

        let local_a = free_local_addr().await;
        let local_b = free_local_addr().await;

        // Accept exactly two connections (whichever handshake completes
        // first goes first — order doesn't matter, `peer_addr()` resolves
        // identity below).
        let accept_jh = tokio::spawn(async move {
            let c1 = listener.accept().await.expect("accept 1");
            let c2 = listener.accept().await.expect("accept 2");
            (c1, c2, listener)
        });

        let (caller_a, caller_b) = tokio::join!(
            SrtSocket::connect_from(local_a, bound_addr, HandshakeConfig::default()),
            SrtSocket::connect_from(local_b, bound_addr, HandshakeConfig::default()),
        );
        let mut caller_a = caller_a.expect("caller A connect");
        let mut caller_b = caller_b.expect("caller B connect");

        let (accepted1, accepted2, _listener) = accept_jh.await.expect("join accept");

        // Resolve identity by peer address, not arrival order.
        let (mut recv_a, mut recv_b) = if accepted1.peer_addr() == local_a {
            (accepted1, accepted2)
        } else {
            (accepted2, accepted1)
        };
        assert_eq!(recv_a.peer_addr(), local_a);
        assert_eq!(recv_b.peer_addr(), local_b);

        let payloads_a: Vec<Vec<u8>> = (0..NUM_PAYLOADS).map(|i| payload(0xAA, i)).collect();
        let payloads_b: Vec<Vec<u8>> = (0..NUM_PAYLOADS).map(|i| payload(0xBB, i)).collect();

        // Interleave sends from both callers concurrently — this is what
        // actually stresses the shared-socket race: without the fix, both
        // connections' driver tasks and (while more handshakes are still
        // pending) the listener's own accept loop all call `recv_from` on
        // the same socket at the same time.
        let pa = payloads_a.clone();
        let pb = payloads_b.clone();
        let send_a = tokio::spawn(async move {
            for p in &pa {
                caller_a.send(p).await.expect("send a");
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
            caller_a
        });
        let send_b = tokio::spawn(async move {
            for p in &pb {
                caller_b.send(p).await.expect("send b");
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
            caller_b
        });
        let (caller_a, caller_b) = tokio::join!(send_a, send_b);
        let caller_a = caller_a.expect("join send a");
        let caller_b = caller_b.expect("join send b");

        let mut received_a = Vec::with_capacity(NUM_PAYLOADS);
        while received_a.len() < NUM_PAYLOADS {
            match tokio::time::timeout(Duration::from_secs(10), recv_a.recv())
                .await
                .expect("recv_a timed out")
                .expect("recv_a")
            {
                Some(p) => received_a.push(p),
                None => break,
            }
        }
        let mut received_b = Vec::with_capacity(NUM_PAYLOADS);
        while received_b.len() < NUM_PAYLOADS {
            match tokio::time::timeout(Duration::from_secs(10), recv_b.recv())
                .await
                .expect("recv_b timed out")
                .expect("recv_b")
            {
                Some(p) => received_b.push(p),
                None => break,
            }
        }

        assert_eq!(
            received_a.len(),
            NUM_PAYLOADS,
            "connection A must receive all of its own caller's payloads, none lost to cross-talk"
        );
        assert_eq!(
            received_b.len(),
            NUM_PAYLOADS,
            "connection B must receive all of its own caller's payloads, none lost to cross-talk"
        );
        assert_eq!(
            received_a, payloads_a,
            "connection A must never see connection B's bytes"
        );
        assert_eq!(
            received_b, payloads_b,
            "connection B must never see connection A's bytes"
        );

        drop(caller_a);
        drop(caller_b);
        drop(recv_a);
        drop(recv_b);
    })
    .await
    .expect("test timed out — deadlock or a datagram misrouted to the wrong task");
}
