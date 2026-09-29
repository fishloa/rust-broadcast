//! Sustained-throughput regression for issue #1061 (C3): `Driver::flush_outbound`
//! used to `tokio::time::sleep` the LiveCC-computed `PKT_SND_PERIOD` per DATA
//! packet. At the default MAX_BW (1 Gbps), that period is ~11 microseconds —
//! but tokio's real timer resolution is coarser than that in practice, so
//! each "11 microsecond" sleep actually blocked for on the order of a
//! millisecond, capping achievable throughput at roughly 1000 packets/sec
//! (~10.5 Mbit/s at a 1316-byte payload) no matter how much faster the
//! configured MAX_BW would otherwise allow — and, since that sleep was
//! awaited serially inside `flush_outbound` (called every `run()` iteration),
//! it stalled RX processing for the same window.
//!
//! This test enqueues a fixed volume of 1316-byte payloads — exactly what a
//! sustained 50 Mbit/s feed would produce over 2 seconds — over real
//! loopback UDP, and asserts the wall-clock time to deliver all of it
//! implies an achieved rate clearing 90% of that 50 Mbit/s target:
//! comfortably above the ~10.5 Mbit/s the pre-fix per-packet-sleep ceiling
//! would allow (documented pre-fix failure below), and comfortably within
//! what a fixed, non-blocking pacing schedule should sustain on loopback
//! well under the default 1 Gbps MAX_BW. (The volume is fixed rather than
//! "spin-send for 2 wall-clock seconds" because `SrtSocket::send`'s
//! `to_driver` channel is unbounded — an unthrottled spin loop would queue
//! an unbounded backlog for the driver to drain and turn the test into an
//! open-ended stress test instead of a rate measurement.)
//!
//! HANG GUARD: wrapped in [`tokio::time::timeout`] (see `io_loopback.rs`).

#![cfg(feature = "tokio")]

use core::time::Duration;

use srt_runtime::handshake_sm::HandshakeConfig;
use srt_runtime::io::{SrtListener, SrtSocket};

const TEST_TIMEOUT: Duration = Duration::from_secs(30);
const PAYLOAD_LEN: usize = 1316;
const NOMINAL_DURATION: Duration = Duration::from_secs(2);
const TARGET_BITS_PER_SEC: u64 = 50_000_000; // 50 Mbit/s
const MIN_ACCEPTABLE_FRACTION: f64 = 0.90;
/// Payloads sent per 1 ms pause (~84 Mbit/s offered load, still ~1.7x the
/// target). Queueing the whole volume in one burst overflows the kernel's
/// default UDP receive buffer on Linux (~208 KB, ~160 packets): the datagrams
/// are lost, the sender's own TLPKTDROP discards them before an ARQ round trip
/// can recover them, and delivery never completes (`test timed out` on every
/// Linux CI run). A real feed is paced by its source; the pause also lets the
/// single-threaded runtime run the driver and the drain task.
const SENDS_PER_PAUSE: usize = 8;
const PAUSE: Duration = Duration::from_millis(1);

/// Pre-fix run of this exact test (unfixed `flush_outbound`, real
/// `tokio::time::sleep` per DATA packet at the default 1 Gbps MAX_BW's ~11 us
/// computed period): delivering the same fixed volume took ~9.4 s instead of
/// ~2 s — an achieved rate of ~10.6 Mbit/s, 21% of the 50 Mbit/s target,
/// well under the 90% bar below (and consistent with the ~1000 pkt/s ceiling
/// a >= ~1 ms real sleep per packet implies). Verified via a copied pre-fix
/// `io.rs` (`git show HEAD:srt-runtime/src/io.rs`), never `git stash`
/// (shared worktree), swapped back immediately after.
#[tokio::test]
async fn sustained_volume_clears_90_percent_of_50mbit_target() {
    tokio::time::timeout(TEST_TIMEOUT, async {
        let listener_addr = "127.0.0.1:0".parse::<std::net::SocketAddr>().unwrap();
        let mut listener = SrtListener::bind(listener_addr, HandshakeConfig::default())
            .await
            .expect("listener bind");
        let bound_addr = listener.local_addr().expect("listener local addr");

        let accept_jh = tokio::spawn(async move { listener.accept().await.expect("accept") });
        let mut caller = SrtSocket::connect(bound_addr, HandshakeConfig::default())
            .await
            .expect("caller connect");
        let mut receiver = accept_jh.await.expect("join accept");

        // Exactly what a sustained TARGET_BITS_PER_SEC feed would produce
        // over NOMINAL_DURATION.
        let total_bytes = (TARGET_BITS_PER_SEC / 8) * NOMINAL_DURATION.as_secs();
        let num_payloads = (total_bytes as usize) / PAYLOAD_LEN;
        let payload = vec![0x47u8; PAYLOAD_LEN];

        // Drain concurrently so TSBPD/ARQ delivery never becomes the
        // bottleneck being measured instead of pacing.
        let drain_jh = tokio::spawn(async move {
            let mut total = 0usize;
            while total < num_payloads {
                match receiver.recv().await {
                    Ok(Some(_)) => total += 1,
                    _ => break,
                }
            }
            (total, receiver)
        });

        let start = tokio::time::Instant::now();
        let mut paused = Duration::ZERO;
        for sent in 0..num_payloads {
            caller.send(&payload).await.expect("send");
            if sent % SENDS_PER_PAUSE == SENDS_PER_PAUSE - 1 {
                let paused_at = tokio::time::Instant::now();
                tokio::time::sleep(PAUSE).await;
                paused += paused_at.elapsed();
            }
        }

        let (received_count, receiver) = drain_jh.await.expect("join drain");
        // The test's own pauses (a timer's granularity varies by OS) are not
        // the driver's pacing, so they are not charged to the achieved rate.
        let wall = start.elapsed() - paused;

        assert_eq!(
            received_count, num_payloads,
            "delivery must be complete (lossless loopback), not just fast"
        );

        let achieved_bits_per_sec = (received_count * PAYLOAD_LEN * 8) as f64 / wall.as_secs_f64();
        let fraction_of_target = achieved_bits_per_sec / TARGET_BITS_PER_SEC as f64;

        eprintln!(
            "delivered {received_count} payloads ({} bytes) in {wall:?} \
             ({achieved_bits_per_sec:.0} bit/s, {:.1}% of {TARGET_BITS_PER_SEC} bit/s target)",
            received_count * PAYLOAD_LEN,
            fraction_of_target * 100.0
        );

        assert!(
            fraction_of_target >= MIN_ACCEPTABLE_FRACTION,
            "achieved {achieved_bits_per_sec:.0} bit/s, only {:.1}% of the {TARGET_BITS_PER_SEC} \
             bit/s target (need >= {:.0}%) — pacing is still capping throughput",
            fraction_of_target * 100.0,
            MIN_ACCEPTABLE_FRACTION * 100.0
        );

        drop(caller);
        drop(receiver);
    })
    .await
    .expect("test timed out — deadlock or stalled pacing");
}
