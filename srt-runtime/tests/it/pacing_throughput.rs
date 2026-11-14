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
//! implies an achieved rate clearing twice the pre-fix per-packet-sleep
//! ceiling (1000 packets/s x 1316 bytes x 8 bits = ~10.5 Mbit/s, see the
//! pre-fix run documented below).
//!
//! The bound is deliberately *relative* to the bug's own ceiling, measured on
//! the same runtime immediately before the transfer, rather than an absolute
//! rate: the bug was one timer sleep per packet, so its ceiling is the
//! machine's timer wake latency at that moment, which grows with load exactly
//! as the transfer does. A fixed 1000 packets/s ceiling would measure the
//! machine instead and fail under load for reasons unrelated to the change
//! under test. Twice the measured ceiling is still far below what the fixed,
//! non-blocking pacing schedule sustains on loopback under the default 1 Gbps
//! MAX_BW. (The volume is
//! fixed rather than "spin-send for 2 wall-clock seconds": an unthrottled
//! spin loop would turn the test into an open-ended stress test instead of a
//! rate measurement.)
//!
//! This is the one deliberately real-time test in the crate: the bug it guards
//! was tokio's real timer granularity, which a paused clock cannot reproduce.
//! The pacing *schedule* itself is pinned deterministically by
//! `data_leaves_one_packet_per_pacing_period` in `src/io.rs`.
//!
//! HANG GUARD: wrapped in [`tokio::time::timeout`] (see `io_loopback.rs`).

#![cfg(feature = "tokio")]

use core::time::Duration;

use srt_runtime::handshake_sm::HandshakeConfig;
use srt_runtime::io::{SrtListener, SrtSocket, SrtSocketConfig};

const TEST_TIMEOUT: Duration = Duration::from_secs(60);
const PAYLOAD_LEN: usize = 1316;
const NOMINAL_DURATION: Duration = Duration::from_secs(2);
const TARGET_BITS_PER_SEC: u64 = 50_000_000; // 50 Mbit/s
/// The LiveCC pacing period the pre-fix `flush_outbound` slept per DATA packet
/// at the default 1 Gbps MAX_BW (about 10.5 us for [`PAYLOAD_LEN`] bytes).
const GIGABIT_PACKET_PERIOD: Duration = Duration::from_micros(11);
/// Sequential sleeps timed to measure what one such sleep really costs on this
/// runtime right now. Bounds the calibration at about a quarter of a second on
/// an idle machine, enough to average out a single late wake.
const CALIBRATION_SLEEPS: u32 = 200;
/// The achieved rate must clear the pre-fix ceiling by this factor, not just
/// barely beat it.
const CEILING_MARGIN: f64 = 2.0;
/// Payloads sent per 1 ms pause (~84 Mbit/s offered load, still ~1.7x the
/// target). Queueing the whole volume in one burst overflows the kernel's
/// default UDP receive buffer on Linux (~208 KB, ~160 packets): the datagrams
/// are lost, the sender's own TLPKTDROP discards them before an ARQ round trip
/// can recover them, and delivery never completes (`test timed out` on every
/// Linux CI run). A real feed is paced by its source; the pause also lets the
/// single-threaded runtime run the driver and the drain task.
const SENDS_PER_PAUSE: usize = 8;
const PAUSE: Duration = Duration::from_millis(1);
/// TSBPD latency both sides run with. Generous on purpose: under CPU
/// starvation a stalled driver would otherwise miss the (default 120 ms) play
/// window, Too-Late Packet Drop would discard what it had not yet delivered,
/// and the "everything delivered" assertion would fail for reasons unrelated
/// to pacing. The last packet is delivered one latency after it is sent, and
/// that is deliberately *charged* to the achieved rate (it can only make the
/// bound stricter): the pre-fix ceiling of ~10 Mbit/s is still far below it.
const LATENCY: Duration = Duration::from_millis(1_000);

/// Pre-fix run of this exact test (unfixed `flush_outbound`, real
/// `tokio::time::sleep` per DATA packet at the default 1 Gbps MAX_BW's ~11 us
/// computed period): delivering the same fixed volume took ~9.4 s instead of
/// ~2 s — an achieved rate of ~10.6 Mbit/s, 21% of the 50 Mbit/s target,
/// well under the 90% bar below (and consistent with the ~1000 pkt/s ceiling
/// a >= ~1 ms real sleep per packet implies). Verified via a copied pre-fix
/// `io.rs` (`git show HEAD:srt-runtime/src/io.rs`), never `git stash`
/// (shared worktree), swapped back immediately after.
#[tokio::test]
async fn sustained_volume_clears_twice_the_pre_fix_pacing_ceiling() {
    tokio::time::timeout(TEST_TIMEOUT, async {
        let listener_addr = "127.0.0.1:0".parse::<std::net::SocketAddr>().unwrap();
        let config = HandshakeConfig::default().with_latency(LATENCY);
        let mut listener = SrtListener::bind(listener_addr, SrtSocketConfig::new(config.clone()))
            .await
            .expect("listener bind");
        let bound_addr = listener.local_addr().expect("listener local addr");

        let accept_jh = tokio::spawn(async move { listener.accept().await.expect("accept") });
        let mut caller = SrtSocket::connect(bound_addr, SrtSocketConfig::new(config))
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

        let calibration_start = tokio::time::Instant::now();
        for _ in 0..CALIBRATION_SLEEPS {
            tokio::time::sleep(GIGABIT_PACKET_PERIOD).await;
        }
        let measured_sleep = calibration_start.elapsed() / CALIBRATION_SLEEPS;
        let pre_fix_ceiling_bits_per_sec = (PAYLOAD_LEN * 8) as f64 / measured_sleep.as_secs_f64();

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
        let required_bits_per_sec = CEILING_MARGIN * pre_fix_ceiling_bits_per_sec;

        eprintln!(
            "delivered {received_count} payloads ({} bytes) in {wall:?} \
             ({achieved_bits_per_sec:.0} bit/s, {:.1}x the pre-fix ceiling of \
             {pre_fix_ceiling_bits_per_sec:.0} bit/s ({measured_sleep:?} per sleep); {:.1}% of \
             {TARGET_BITS_PER_SEC} bit/s target)",
            received_count * PAYLOAD_LEN,
            achieved_bits_per_sec / pre_fix_ceiling_bits_per_sec,
            achieved_bits_per_sec / TARGET_BITS_PER_SEC as f64 * 100.0
        );

        assert!(
            achieved_bits_per_sec >= required_bits_per_sec,
            "achieved {achieved_bits_per_sec:.0} bit/s, under {CEILING_MARGIN}x the pre-fix \
             ceiling {pre_fix_ceiling_bits_per_sec:.0} bit/s (need >= {required_bits_per_sec:.0} \
             bit/s) — pacing is still capping throughput"
        );

        drop(caller);
        drop(receiver);
    })
    .await
    .expect("test timed out — deadlock or stalled pacing");
}
