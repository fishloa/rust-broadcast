//! Loss-recovery integration test — proves ARQ NAK->retransmit recovery
//! (`draft-sharabayko-srt-01` §4.8.2) end to end **through the tokio glue in
//! `io.rs`**, not just the sans-IO engine in isolation (see
//! `tests/arq_recovery.rs` for that in-memory version).
//!
//! A small loss-injecting UDP relay sits between the [`SrtSocket`] Caller and
//! the [`SrtListener`]-accepted connection, forwarding datagrams both ways
//! but deterministically DROPPING a subset of first-time DATA packets. If
//! `io.rs`'s send path never drains inbound NAKs (so the sans-IO
//! [`srt_runtime::arq::Sender`] never learns about the loss), or if
//! retransmits don't take priority over new first-time data in the outbound
//! queue, the dropped payloads are never recovered and this test times out /
//! fails on a byte mismatch.
//!
//! Wrapped in [`tokio::time::timeout`] (15 s) so a regression FAILS fast
//! instead of hanging forever.

#![cfg(feature = "tokio")]

use core::time::Duration;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use srt_runtime::handshake_sm::HandshakeConfig;
use srt_runtime::io::{SrtListener, SrtSocket};
use srt_runtime::packet::SrtPacket;

const CALLER_ISN: u32 = 500;
const LISTENER_ISN: u32 = 1000;
const NUM_PAYLOADS: usize = 40;
/// Drop every `DROP_MODULUS`-th first-time DATA packet (by sequence offset
/// from `CALLER_ISN`), but only within the first `DROP_WINDOW` payloads —
/// leaving a "settle" tail of real sends after the last drop so the
/// caller's send path (which drains inbound NAKs from within
/// `SrtSocket::send`, see `io.rs`'s `drive_send`) gets several more chances
/// to see the NAK and retransmit before this test starts asserting
/// delivery.
const DROP_MODULUS: usize = 7;
const DROP_REMAINDER: usize = 3;
const DROP_WINDOW: usize = NUM_PAYLOADS - 10;
const TEST_TIMEOUT: Duration = Duration::from_secs(15);

/// A minimal loss-injecting UDP relay: forwards datagrams in both directions
/// between a "client-facing" socket (the address the Caller connects to) and
/// the real Listener, but deterministically drops a subset of
/// caller->listener first-time DATA packets (never a retransmission, and
/// never a control packet — so ACK/NAK/ACKACK feedback always gets through,
/// matching a lossy-but-not-fully-broken network).
async fn run_relay(
    client_side: tokio::net::UdpSocket,
    server_side: tokio::net::UdpSocket,
    listener_addr: SocketAddr,
    dropped_seqs: Arc<Mutex<Vec<u32>>>,
) {
    let mut caller_addr: Option<SocketAddr> = None;
    let mut buf_from_caller = [0u8; 2048];
    let mut buf_from_listener = [0u8; 2048];

    loop {
        tokio::select! {
            res = client_side.recv_from(&mut buf_from_caller) => {
                let Ok((len, src)) = res else { break };
                caller_addr = Some(src);
                let bytes = &buf_from_caller[..len];

                let drop_seq = match SrtPacket::parse(bytes) {
                    Ok(SrtPacket::Data(d)) if !d.retransmitted => {
                        let idx = d.seq_number.wrapping_sub(CALLER_ISN) as usize;
                        if idx < DROP_WINDOW && idx % DROP_MODULUS == DROP_REMAINDER {
                            Some(d.seq_number)
                        } else {
                            None
                        }
                    }
                    _ => None,
                };

                if let Some(seq) = drop_seq {
                    dropped_seqs.lock().unwrap().push(seq);
                    continue; // simulated loss: never forwarded.
                }

                let _ = server_side.send_to(bytes, listener_addr).await;
            }
            res = server_side.recv_from(&mut buf_from_listener) => {
                let Ok((len, _src)) = res else { break };
                if let Some(caller) = caller_addr {
                    let _ = client_side.send_to(&buf_from_listener[..len], caller).await;
                }
            }
        }
    }
}

/// Loss-recovery test: 40 payloads sent through a lossy relay must ALL
/// arrive, in order, byte-identical — proving the NAK->retransmit path works
/// through `io.rs`, not just in the sans-IO engine.
#[tokio::test]
async fn loss_recovery_through_io_layer() {
    tokio::time::timeout(TEST_TIMEOUT, async {
        // --- Real listener ---
        let listener_bind_addr = "127.0.0.1:0".parse::<SocketAddr>().unwrap();
        let listener_config = HandshakeConfig {
            initial_seq_number: LISTENER_ISN,
            ..HandshakeConfig::default()
        };
        let mut listener = SrtListener::bind(listener_bind_addr, listener_config)
            .await
            .expect("listener bind");
        let listener_addr = listener.local_addr().expect("listener local addr");

        // --- Loss-injecting relay between caller and listener ---
        let client_side = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("relay client-side bind");
        let relay_addr = client_side.local_addr().expect("relay addr");
        let server_side = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("relay server-side bind");
        let dropped_seqs = Arc::new(Mutex::new(Vec::new()));
        let relay_dropped = Arc::clone(&dropped_seqs);
        // Keep the relay's `JoinHandle` so it is explicitly aborted at the end
        // of the test — a spawned task left running could keep the
        // current-thread runtime from shutting down cleanly.
        let relay_handle = tokio::spawn(run_relay(
            client_side,
            server_side,
            listener_addr,
            relay_dropped,
        ));

        // --- Caller connects THROUGH the relay, not directly to the listener ---
        let caller_config = HandshakeConfig {
            initial_seq_number: CALLER_ISN,
            ..HandshakeConfig::default()
        };
        let jh = tokio::spawn(async move { listener.accept().await.expect("listener accept") });
        let mut caller = SrtSocket::connect(relay_addr, caller_config)
            .await
            .expect("caller connect");
        let receiver = jh.await.expect("join listener");

        // --- Send N distinct payloads; a deterministic subset is dropped in transit ---
        let payloads: Vec<Vec<u8>> = (0..NUM_PAYLOADS)
            .map(|i| {
                let mut p = vec![0u8; 80 + i * 3];
                for (j, byte) in p.iter_mut().enumerate() {
                    *byte = (i as u8).wrapping_add(j as u8).wrapping_add(0x5A);
                }
                p
            })
            .collect();

        // Drive the receiver CONCURRENTLY with the caller's sends (as a real
        // SRT deployment does): the receiver task keeps producing NAKs and the
        // caller's driver keeps servicing retransmits throughout, and — via
        // the driver task's periodic tick — recovery still completes after the
        // caller's last send. The receive task owns `receiver` and drops it on
        // return (aborting its driver task).
        let recv_task = tokio::spawn(async move {
            let mut received: Vec<Vec<u8>> = Vec::with_capacity(NUM_PAYLOADS);
            let mut receiver = receiver;
            while received.len() < NUM_PAYLOADS {
                match receiver.recv().await.expect("receiver recv") {
                    Some(payload) => received.push(payload),
                    None => break,
                }
            }
            received
        });

        for payload in &payloads {
            caller.send(payload).await.expect("caller send");
            // Localhost RTT is sub-millisecond; a small gap keeps the loss /
            // NAK / retransmit round trip flowing while sending.
            tokio::time::sleep(Duration::from_millis(8)).await;
        }

        // Wait for the receiver to collect everything (recovery of the last
        // dropped packet may complete slightly after the final send, driven
        // by the caller driver's periodic tick).
        let received = recv_task.await.expect("join receiver task");

        // Deterministic teardown: stop the relay and the caller's driver task
        // so the runtime has nothing left running.
        relay_handle.abort();
        drop(caller);

        // Sanity: confirm the relay actually exercised the drop path — a
        // test that never drops anything would trivially pass even with
        // retransmission completely broken.
        let dropped_count = dropped_seqs.lock().unwrap().len();
        assert!(
            dropped_count > 0,
            "relay never dropped a packet — this test does not exercise loss recovery"
        );

        assert_eq!(
            received.len(),
            NUM_PAYLOADS,
            "expected all {NUM_PAYLOADS} payloads to arrive (recovered via NAK-driven \
             retransmission after {dropped_count} simulated drop(s)), got {}",
            received.len()
        );

        for (i, (sent, recvd)) in payloads.iter().zip(received.iter()).enumerate() {
            assert_eq!(
                sent,
                recvd,
                "payload {i} mismatch after loss recovery: sent {} bytes, recv {} bytes",
                sent.len(),
                recvd.len()
            );
        }
    })
    .await
    .expect(
        "test timed out — retransmission is likely broken (NAK never drained by the send path, \
         or retransmit priority lost)",
    );
}
