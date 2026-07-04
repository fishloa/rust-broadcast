//! TSBPD delivery integration tests.
//!
//! Sans-IO, sub-second: feeds `TsbpdScheduler` with out-of-order packet
//! arrivals and assorted timestamps, advances a virtual clock, and asserts
//! that packets are released in proper sequence order, each at or after its
//! `PktTsbpdTime`, and that too-late packets are dropped.
//!
//! Spec grounding: `specs/rules/srt-tsbpd.md` §4.5/§4.6 (curated from
//! `draft-sharabayko-srt-01`).

use core::time::Duration;
use srt_runtime::tsbpd::{TickOutcome, TsbpdScheduler};

/// Arbitrary TsbpdTimeBase in microseconds (rule 12).
const TIME_BASE_US: u64 = 42_000_000;
/// TsbpdDelay in milliseconds (rule 10, minimum 120).
const DELAY_MS: u64 = 120;
/// TsbpdDelay in microseconds.
const DELAY_US: u64 = DELAY_MS * 1000;
/// Too-late drop threshold at default 1.25 × delay.
fn drop_threshold_us() -> u64 {
    (DELAY_MS * 5).div_ceil(4) * 1000
}
/// Initial sequence number (peer ISN).
const ISN: u32 = 0;

/// A synthetic packet arrival to feed to the scheduler.
struct PacketInput {
    seq: u32,
    timestamp: u32,
}

/// Advance the clock to `now_us` and collect all output.
fn tick_to(sched: &mut TsbpdScheduler, now_us: u64) -> TickOutcome {
    sched.tick(Duration::from_micros(now_us))
}

/// Feed a packet and collect all output.
fn feed(sched: &mut TsbpdScheduler, seq: u32, ts: u32, now_us: u64) -> TickOutcome {
    sched.feed_data(seq, ts, Duration::from_micros(now_us))
}

// ---------------------------------------------------------------------------
// 1. Basic ordered delivery: packets arrive in order, are withheld until
//    PktTsbpdTime, then released in sequence.
// ---------------------------------------------------------------------------
#[test]
fn ordered_arrivals_withhold_then_release() {
    let mut sched = TsbpdScheduler::new(ISN, TIME_BASE_US, DELAY_MS, 0, false, None);

    let pkts = [
        PacketInput {
            seq: 0,
            timestamp: 0,
        },
        PacketInput {
            seq: 1,
            timestamp: 10_000,
        },
        PacketInput {
            seq: 2,
            timestamp: 20_000,
        },
    ];

    // Feed all three at time 0 — none should be delivered yet because
    // PktTsbpdTime is in the future.
    for p in &pkts {
        let out = feed(&mut sched, p.seq, p.timestamp, 0);
        assert!(out.delivered.is_empty(), "no delivery before PktTsbpdTime");
        assert!(out.dropped.is_empty());
    }
    assert_eq!(sched.buffered_count(), 3);

    // Tick at each packet's PktTsbpdTime. The delivered vector must be built
    // ONLY from what the scheduler releases.
    let tsbpd0 = TIME_BASE_US + DELAY_US;
    let out = tick_to(&mut sched, tsbpd0);
    assert_eq!(out.delivered, vec![0], "release seq 0 at its play time");
    assert!(out.dropped.is_empty());
    assert_eq!(sched.buffered_count(), 2);

    let tsbpd1 = TIME_BASE_US + 10_000 + DELAY_US;
    let out = tick_to(&mut sched, tsbpd1);
    assert_eq!(out.delivered, vec![1], "release seq 1 at its play time");
    assert_eq!(sched.buffered_count(), 1);

    let tsbpd2 = TIME_BASE_US + 20_000 + DELAY_US;
    let out = tick_to(&mut sched, tsbpd2);
    assert_eq!(out.delivered, vec![2], "release seq 2 at its play time");
    assert_eq!(sched.buffered_count(), 0);
}

// ---------------------------------------------------------------------------
// 2. Out-of-order arrival: packets 1, 2 arrive before 0; 0 fills the gap
//    and releases 0, 1, 2 in sequence order at their respective play times.
// ---------------------------------------------------------------------------
#[test]
fn out_of_order_arrivals_released_in_sequence() {
    let mut sched = TsbpdScheduler::new(ISN, TIME_BASE_US, DELAY_MS, 0, false, None);

    // Packet 1 arrives first.
    feed(&mut sched, 1, 10_000, 0);
    assert_eq!(sched.buffered_count(), 1);
    assert!(sched.has_gap());

    // Packet 2 arrives second.
    feed(&mut sched, 2, 20_000, 0);
    assert_eq!(sched.buffered_count(), 2);

    // Packet 0 arrives — at a time when PktTsbpdTime[0] has passed but
    // previous deliveries were waiting for it.
    let tsbpd0 = TIME_BASE_US + DELAY_US;
    let tsbpd2 = TIME_BASE_US + 20_000 + DELAY_US;
    let now = tsbpd2.max(tsbpd0); // past all three play times
    let out = feed(&mut sched, 0, 0, now);
    // All three should be delivered in order — the delivered vector must be
    // FROM the scheduler (not a pre-computed list).
    assert_eq!(
        out.delivered,
        vec![0, 1, 2],
        "out-of-order arrivals must release in sequence 0, 1, 2"
    );
    assert!(out.dropped.is_empty());
    assert_eq!(sched.buffered_count(), 0);
}

// ---------------------------------------------------------------------------
// 3. Withhold delivery: feed at t=0, tick at t=1ms (before PktTsbpdTime),
//    assert nothing released; then tick at PktTsbpdTime, assert released.
// ---------------------------------------------------------------------------
#[test]
fn packet_withheld_until_play_time() {
    let mut sched = TsbpdScheduler::new(ISN, TIME_BASE_US, DELAY_MS, 0, false, None);
    feed(&mut sched, 0, 0, 0);

    // Tick at 1 µs before PktTsbpdTime.
    let just_before = TIME_BASE_US + DELAY_US - 1;
    let out = tick_to(&mut sched, just_before);
    assert!(
        out.delivered.is_empty(),
        "packet must not be released before PktTsbpdTime"
    );

    // Tick exactly at PktTsbpdTime.
    let at_time = TIME_BASE_US + DELAY_US;
    let out = tick_to(&mut sched, at_time);
    assert_eq!(
        out.delivered,
        vec![0],
        "packet released exactly at play time"
    );
}

// ---------------------------------------------------------------------------
// 4. Too-late drop on arrival: a packet whose PktTsbpdTime is before
//    (now - TLPKTDROP_THRESHOLD) is dropped immediately.
// ---------------------------------------------------------------------------
#[test]
fn too_late_packet_dropped_on_arrival() {
    let mut sched = TsbpdScheduler::new(ISN, TIME_BASE_US, DELAY_MS, 0, true, None);

    // Arrive at now much later than PktTsbpdTime for seq 0.
    let pkt_tsbpd = TIME_BASE_US + DELAY_US;
    let now = pkt_tsbpd + drop_threshold_us() + 1; // past the drop window
    let out = feed(&mut sched, 0, 0, now);

    assert!(
        out.delivered.is_empty(),
        "too-late packet must not be delivered"
    );
    assert_eq!(
        out.dropped,
        vec![0],
        "too-late packet must be listed as dropped"
    );
    assert_eq!(
        sched.buffered_count(),
        0,
        "dropped packet must not be buffered"
    );
    // The next expected sequence advances past the dropped packet.
    assert_eq!(sched.next_release(), 1);
}

// ---------------------------------------------------------------------------
// 5. Too-late drop chain: feed 1, 2 out of order (they buffer); feed 0 very
//    late; 0 is delivered (on time), then 1, 2 are too late and dropped.
// ---------------------------------------------------------------------------
#[test]
fn drop_chain_after_gap_fill() {
    let mut sched = TsbpdScheduler::new(ISN, TIME_BASE_US, DELAY_MS, 0, true, None);

    // Packet 1 arrives on time but buffers because 0 is missing.
    feed(&mut sched, 1, 10_000, 0);
    feed(&mut sched, 2, 20_000, 0);

    // Packet 0 arrives very late.
    let pkt0_tsbpd = TIME_BASE_US + DELAY_US;
    let now = pkt0_tsbpd + drop_threshold_us() + 1;

    // At `now`, pkt0's PktTsbpdTime is past the drop threshold since
    // pkt0_tsbpd < (pkt0_tsbpd + drop_threshold_us() + 1) - drop_threshold_us()
    // → pkt0_tsbpd < pkt0_tsbpd + 1 → true, so pkt0 is dropped on arrival.
    // feed_data drops it immediately and advances next_release past it,
    // then release_ready handles the buffered packets 1 and 2.
    let _out = feed(&mut sched, 0, 0, now);

    assert_eq!(sched.buffered_count(), 0);

    // After the drops, next_release should have advanced past all three.
    assert_eq!(sched.next_release(), 3);
    assert_eq!(sched.buffered_count(), 0);
}

// ---------------------------------------------------------------------------
// 6. Timestamp wrap near MAX_TIMESTAMP: the scheduler uses u64 arithmetic,
//    so wrapped timestamps produce correctly-ordered play times.
// ---------------------------------------------------------------------------
#[test]
fn timestamp_wrap_handling() {
    let mut sched = TsbpdScheduler::new(ISN, TIME_BASE_US, DELAY_MS, 0, false, None);

    // Packet just before the 32-bit wrap.
    let near_wrap: u32 = 0xFFFF_FF00u32;
    // Packet just after the wrap.
    let after_wrap: u32 = 500;

    feed(&mut sched, 0, near_wrap, 0);
    feed(&mut sched, 1, after_wrap, 0);

    // Both are buffered.
    assert_eq!(sched.buffered_count(), 2);

    // PktTsbpdTime in u64 — the 32-bit timestamps are extended losslessly.
    // The pre-wrap timestamp (0xFFFF_FF00 ≈ 4.3B) is naturally larger than
    // the post-wrap one (500) in u64, so pkt0 has a later play time.
    let tsbpd0 = TIME_BASE_US + near_wrap as u64 + DELAY_US;
    let tsbpd1 = TIME_BASE_US + after_wrap as u64 + DELAY_US;
    assert!(
        tsbpd0 > tsbpd1,
        "pre-wrap packet has later play time in u64"
    );

    // Release both by ticking at pkt0's play time.
    let out = tick_to(&mut sched, tsbpd0);
    assert_eq!(
        out.delivered,
        vec![0, 1],
        "wrapped timestamps must deliver in sequence order"
    );
}

// ---------------------------------------------------------------------------
// 7. Disabled TLPKTDROP: a packet far past its play time is still delivered.
// ---------------------------------------------------------------------------
#[test]
fn disabled_drop_delivers_late_packets() {
    let mut sched = TsbpdScheduler::new(ISN, TIME_BASE_US, DELAY_MS, 0, false, None);

    let very_late = TIME_BASE_US + DELAY_US + drop_threshold_us() * 10;
    let out = feed(&mut sched, 0, 0, very_late);
    assert_eq!(
        out.delivered,
        vec![0],
        "late packet must be delivered when drop is disabled"
    );
    assert!(out.dropped.is_empty());
}

// ---------------------------------------------------------------------------
// 8. Gap blocks delivery even when clock is far advanced.
// ---------------------------------------------------------------------------
#[test]
fn gap_blocks_delivery_until_filled() {
    let mut sched = TsbpdScheduler::new(ISN, TIME_BASE_US, DELAY_MS, 0, false, None);

    feed(&mut sched, 2, 20_000, 0);
    feed(&mut sched, 1, 10_000, 0);

    // Tick far past the play times of 1 and 2 — but since packet 0 is
    // missing, nothing can be delivered.
    let far = TIME_BASE_US + 30_000 + DELAY_US + 1_000_000;
    let out = tick_to(&mut sched, far);
    assert!(out.delivered.is_empty(), "no delivery when gap present");

    // Fill the gap — all three packets are now past their play time and
    // there's no drop, so they all get delivered in order.
    let out = feed(&mut sched, 0, 0, far);
    assert_eq!(
        out.delivered,
        vec![0, 1, 2],
        "filling gap unblocks all buffered packets"
    );
    assert!(out.dropped.is_empty());
}

// ---------------------------------------------------------------------------
// 9. Custom TLPKTDROP threshold.
// ---------------------------------------------------------------------------
#[test]
fn custom_drop_threshold() {
    let custom_threshold = 50_000u64; // 50 ms
    let mut sched =
        TsbpdScheduler::new(ISN, TIME_BASE_US, DELAY_MS, 0, true, Some(custom_threshold));

    // PktTsbpdTime at TIME_BASE_US + 0 + DELAY_US.
    // Arrive at now = PktTsbpdTime + custom_threshold + 1 → past threshold.
    let pkt_tsbpd = TIME_BASE_US + DELAY_US;
    let now = pkt_tsbpd + custom_threshold + 1;
    let out = feed(&mut sched, 0, 0, now);
    assert!(
        out.delivered.is_empty(),
        "must not deliver when past custom threshold"
    );
    assert_eq!(out.dropped, vec![0]);
}

// ---------------------------------------------------------------------------
// 10. Non-zero drift is included in the PktTsbpdTime calculation.
// ---------------------------------------------------------------------------
#[test]
fn drift_affects_play_time() {
    let drift_us = 5000u64;
    let mut sched = TsbpdScheduler::new(ISN, TIME_BASE_US, DELAY_MS, drift_us, false, None);

    // PktTsbpdTime includes drift: TIME_BASE_US + 0 + DELAY_US + drift_us.
    let pkt_tsbpd = TIME_BASE_US + DELAY_US + drift_us;

    // Tick before drift-adjusted play time — should not release.
    let out = tick_to(&mut sched, pkt_tsbpd - 1);
    assert!(out.delivered.is_empty(), "drift delays release");

    // Feed packet and tick at the right time.
    feed(&mut sched, 0, 0, pkt_tsbpd);
    // Already past play time due to drift.
    assert_eq!(
        sched.buffered_count(),
        0,
        "drift-adjusted delivery at play time"
    );
}

// ---------------------------------------------------------------------------
// 11. Multiple ticks: feed a batch, then tick forward gradually, collecting
//     the delivered vectors. The concatenation should be the full sequence.
// ---------------------------------------------------------------------------
#[test]
fn gradual_tick_releases_incrementally() {
    let mut sched = TsbpdScheduler::new(ISN, TIME_BASE_US, DELAY_MS, 0, false, None);

    // Feed 10 packets with 5 ms spacing.
    for i in 0..10u32 {
        feed(&mut sched, i, i * 5000, 0);
    }

    let mut all_delivered = Vec::new();
    let start_tsbpd = TIME_BASE_US + DELAY_US;
    // Tick every 1 ms from just before play time.
    for offset in 0..60 {
        let out = tick_to(&mut sched, start_tsbpd + offset * 1000);
        all_delivered.extend(out.delivered);
    }

    assert_eq!(all_delivered, (0..10).collect::<Vec<_>>());
}

// ---------------------------------------------------------------------------
// 12. Duplicate arrival after already delivered is a no-op.
// ---------------------------------------------------------------------------
#[test]
fn duplicate_arrival_is_noop() {
    let mut sched = TsbpdScheduler::new(ISN, TIME_BASE_US, DELAY_MS, 0, false, None);

    let play = TIME_BASE_US + DELAY_US;
    feed(&mut sched, 0, 0, play);
    assert_eq!(sched.buffered_count(), 0);
    assert_eq!(sched.next_release(), 1);

    // Same packet arrives again.
    let out = feed(&mut sched, 0, 0, play);
    assert!(out.delivered.is_empty());
    assert_eq!(sched.next_release(), 1);
}
