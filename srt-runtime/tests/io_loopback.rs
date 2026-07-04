//! Loopback integration test over real UDP sockets — wires a Caller and
//! Listener [`SrtSocket`] over ephemeral 127.0.0.1 ports.
//!
//! Drives the full stack: UDP bind → HSv5 handshake → ARQ data transfer
//! (N >= 20 payloads) → TSBPD-ordered delivery.
//!
//! The test body is wrapped in [`tokio::time::timeout`] (10 s) so a deadlock
//! FAILS fast instead of hanging forever.

#![cfg(feature = "tokio")]

use core::time::Duration;

use srt_runtime::handshake_sm::HandshakeConfig;
use srt_runtime::io::{SrtListener, SrtSocket};

const CALLER_ISN: u32 = 500;
const LISTENER_ISN: u32 = 1000;
const NUM_PAYLOADS: usize = 20;
const TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Loopback test: listener binds, caller connects, they exchange N payloads,
/// receiver gets them all in order, byte-identical.
#[tokio::test]
async fn loopback_call_to_recv() {
    tokio::time::timeout(TEST_TIMEOUT, async {
        // --- Listener ---
        let listener_addr = "127.0.0.1:0".parse::<std::net::SocketAddr>().unwrap();
        let listener_config = HandshakeConfig {
            initial_seq_number: LISTENER_ISN,
            ..HandshakeConfig::default()
        };
        let mut listener = SrtListener::bind(listener_addr, listener_config)
            .await
            .expect("listener bind");

        let bound_addr = listener.local_addr().expect("listener local addr");

        // --- Caller ---
        let caller_config = HandshakeConfig {
            initial_seq_number: CALLER_ISN,
            ..HandshakeConfig::default()
        };

        // Spawn the listener accept in a background task.
        let jh = tokio::spawn(async move { listener.accept().await.expect("listener accept") });

        let mut caller = SrtSocket::connect(bound_addr, caller_config)
            .await
            .expect("caller connect");

        // Wait for the accepted socket.
        let mut receiver = jh.await.expect("join listener");

        // --- Send N distinct payloads from caller ---
        let payloads: Vec<Vec<u8>> = (0..NUM_PAYLOADS)
            .map(|i| {
                let mut p = vec![0u8; 100 + i * 10];
                // Fill with a recognizable pattern unique per payload.
                for (j, byte) in p.iter_mut().enumerate() {
                    *byte = (i as u8).wrapping_add(j as u8);
                }
                p
            })
            .collect();

        for payload in &payloads {
            caller.send(payload).await.expect("caller send");
            // Small delay to let the receiver process.
            tokio::time::sleep(Duration::from_millis(5)).await;
        }

        // --- Receive all N payloads from the receiver ---
        let mut received: Vec<Vec<u8>> = Vec::with_capacity(NUM_PAYLOADS);
        while received.len() < NUM_PAYLOADS {
            match receiver.recv().await.expect("receiver recv") {
                Some(payload) => {
                    received.push(payload);
                }
                None => break,
            }
        }

        assert_eq!(
            received.len(),
            NUM_PAYLOADS,
            "expected {NUM_PAYLOADS} payloads, got {}",
            received.len()
        );

        // Verify byte-identical delivery in order.
        for (i, (sent, recvd)) in payloads.iter().zip(received.iter()).enumerate() {
            assert_eq!(
                sent,
                recvd,
                "payload {i} mismatch: sent {} bytes, recv {} bytes",
                sent.len(),
                recvd.len()
            );
        }

        // Deterministic teardown: drop both sockets so `SrtSocket::Drop`
        // aborts their background driver tasks (and each driver also exits
        // cooperatively once its command channel closes). Without this a live
        // driver task — parked on `udp.recv` with a periodic 2 ms tick — can
        // keep the current-thread test runtime from shutting down, which under
        // `cargo test`'s parallel binary execution shows up as this test's
        // binary never exiting.
        drop(caller);
        drop(receiver);
    })
    .await
    .expect("test timed out — deadlock or stalled handshake");
}
