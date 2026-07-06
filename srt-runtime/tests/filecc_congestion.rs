//! FileCC (§5.2 window-based congestion control) — integration tests.
//!
//! Assertions use hand-computed constants (mirroring `tests/livecc_pacing.rs`'s
//! style) so a trivial/no-op/constant implementation must fail every test
//! here. Spec grounding: `specs/rules/srt-congestion.md`.

use core::time::Duration;
use srt_runtime::filecc::{FileCc, Phase};

/// `RC_INTERVAL` = `SYN` = 10 ms (`specs/rules/srt-congestion.md` L3421-3425).
const RC_INTERVAL_US: f64 = 10_000.0;
/// The initial RTT estimate before any ACK is fed — 100 ms
/// (`specs/rules/srt-arq.md` rule 31, reused by `filecc`, not redefined).
const INITIAL_RTT_US: f64 = 100_000.0;

/// Test 1 — slow-start growth test: feed ACKs with no loss, assert
/// `CWND_SIZE` evolves per the exact spec formula (`CWND_SIZE += ACK_SEQNO -
/// LAST_ACK_SEQNO`, `specs/rules/srt-congestion.md` L3381-3385) and
/// `PKT_SND_PERIOD` stays fixed at 1 microsecond (L3338-3340). Also checks
/// the `RC_INTERVAL` rate-control gate (Step 1, L3365-3371): an ACK fed
/// before 10ms have elapsed since the last one must be a no-op.
#[test]
fn slow_start_growth_matches_hand_computed_formula() {
    let mut cc = FileCc::new(0);
    assert_eq!(cc.phase(), Phase::SlowStart);
    assert_eq!(cc.cwnd_size(), 16.0, "initial CWND_SIZE (rule 7)");
    assert_eq!(cc.pkt_snd_period_us(), 1.0, "fixed PKT_SND_PERIOD (rule 6)");

    // ACK 1: ack_seqno=5, LAST_ACK_SEQNO starts at initial_seqno=0.
    // CWND_SIZE = 16 + (5 - 0) = 21.
    cc.on_ack(
        Duration::from_millis(10),
        5,
        0,
        0,
        Duration::from_millis(100),
    );
    assert_eq!(cc.cwnd_size(), 21.0);
    assert_eq!(cc.pkt_snd_period_us(), 1.0, "still fixed during slow start");

    // ACK 2 too soon (only 1ms later, < RC_INTERVAL=10ms): Step 1 gate
    // must block this update entirely, even with an outrageous ack_seqno.
    cc.on_ack(
        Duration::from_millis(11),
        100_000,
        0,
        0,
        Duration::from_millis(100),
    );
    assert_eq!(
        cc.cwnd_size(),
        21.0,
        "RC_INTERVAL gate must block an early ACK"
    );

    // ACK 3: 21ms (>= RC_INTERVAL past the last *applied* update at 10ms).
    // ack_seqno=13, LAST_ACK_SEQNO=5 (unchanged by the blocked ACK 2).
    // CWND_SIZE = 21 + (13 - 5) = 29.
    cc.on_ack(
        Duration::from_millis(21),
        13,
        0,
        0,
        Duration::from_millis(100),
    );
    assert_eq!(cc.cwnd_size(), 29.0);

    // ACK 4: 32ms. ack_seqno=29. CWND_SIZE = 29 + (29 - 13) = 45.
    cc.on_ack(
        Duration::from_millis(32),
        29,
        0,
        0,
        Duration::from_millis(100),
    );
    assert_eq!(cc.cwnd_size(), 45.0);
    assert_eq!(
        cc.pkt_snd_period_us(),
        1.0,
        "PKT_SND_PERIOD must stay fixed at 1us throughout slow start"
    );
    assert_eq!(
        cc.phase(),
        Phase::SlowStart,
        "no loss/threshold crossed yet"
    );
}

/// Test 2 — loss triggers transition out of slow start into congestion
/// avoidance (`specs/rules/srt-congestion.md` rule 9, L3427-3432). A trivial
/// impl that never transitions must fail this. Also hand-checks the
/// slow-start-end `PKT_SND_PERIOD` formula (Step 5, L3392-3401): with no ACK
/// fed yet, `RECEIVING_RATE` is 0, so `PKT_SND_PERIOD = CWND_SIZE / (RTT +
/// RC_INTERVAL)` using the untouched initial CWND_SIZE (16) and RTT
/// (100ms, `specs/rules/srt-arq.md` rule 31).
#[test]
fn loss_during_slow_start_transitions_and_computes_step5_period() {
    let mut cc = FileCc::new(0);
    assert_eq!(cc.phase(), Phase::SlowStart);

    cc.on_loss(10, 10, 0.9);

    assert_eq!(
        cc.phase(),
        Phase::CongestionAvoidance,
        "a loss during slow start must end it (rule 9)"
    );

    let expected_period = 16.0_f64 / (INITIAL_RTT_US + RC_INTERVAL_US);
    assert_eq!(
        cc.pkt_snd_period_us(),
        expected_period,
        "slow-start-end PKT_SND_PERIOD must follow CWND_SIZE/(RTT+RC_INTERVAL)"
    );
}

/// Test 3 — congestion-avoidance CWND formula test: feed a known
/// RECEIVING_RATE/RTT and assert `CWND_SIZE` matches the exact formula
/// (`specs/rules/srt-congestion.md` Step 3, L3479-3481):
/// `CWND_SIZE = RECEIVING_RATE*(RTT+RC_INTERVAL)/1000000 + 16`.
#[test]
fn congestion_avoidance_cwnd_matches_exact_formula() {
    let mut cc = FileCc::new(0);
    // Move to Congestion Avoidance first (loss during slow start).
    cc.on_loss(1, 1, 0.9);
    assert_eq!(cc.phase(), Phase::CongestionAvoidance);

    // RECEIVING_RATE=5000 pkts/sec, RTT=50ms.
    // CWND_SIZE = 5000 * (50_000 + 10_000) / 1_000_000 + 16
    //           = 5000 * 60_000 / 1_000_000 + 16
    //           = 300_000_000 / 1_000_000 + 16
    //           = 300 + 16 = 316.
    cc.on_ack(
        Duration::from_millis(50),
        1,
        5_000,
        0,
        Duration::from_millis(50),
    );
    assert_eq!(cc.cwnd_size(), 316.0);
}

/// Test 4 — repeated-decrease rate-backoff test: feed consecutive loss
/// events within the same congestion period and assert `PKT_SND_PERIOD`
/// increases by the `1.03` factor per the `DecCount<=5` rule
/// (`specs/rules/srt-congestion.md` "Step 4", L3652-3658), and that it stops
/// once `DecCount` exceeds 5. A no-op/constant implementation fails every
/// assertion here; an implementation missing the `DecCount<=5` cutoff fails
/// the final assertion.
#[test]
fn repeated_decrease_backs_off_by_1_03_bounded_by_dec_count() {
    let mut cc = FileCc::new(0);

    // Move to Congestion Avoidance via a slow-start loss. No ACK fed yet,
    // so RECEIVING_RATE=0 -> Step 5's CWND_SIZE/(RTT+RC_INTERVAL) branch.
    cc.on_loss(10, 10, 0.9);
    assert_eq!(cc.phase(), Phase::CongestionAvoidance);
    let base = 16.0_f64 / (INITIAL_RTT_US + RC_INTERVAL_US);
    assert_eq!(cc.pkt_snd_period_us(), base);

    // First Congestion-Avoidance-phase NAK: LastDecSeq is None, so this is
    // always a "new congestion period" (rule 25) -> the big decrease
    // (rule 19: PKT_SND_PERIOD *= 1.03), regardless of loss_ratio (well
    // above the 2% tolerance here).
    let mut expected = base;
    cc.on_loss(20, 20, 0.9);
    expected *= 1.03;
    assert_eq!(
        cc.pkt_snd_period_us(),
        expected,
        "first CA NAK: big decrease"
    );
    assert_eq!(cc.dec_count(), 1);

    // Five repeat-decrease NAKs within the SAME congestion period: each
    // lost_seqno (15) is <= the running LastDecSeq, so Step 4 applies each
    // time (DecCount<=5 && NAKCount==DecCount*DecRandom; DecRandom=1 here
    // since AvgNAKNum started at 0). DecCount climbs 1->2->3->4->5->6.
    for sent in 21..=25u32 {
        cc.on_loss(15, sent, 0.9);
        expected *= 1.03;
        assert_eq!(
            cc.pkt_snd_period_us(),
            expected,
            "repeat-decrease NAK at largest_sent_seqno={sent}"
        );
    }
    assert_eq!(cc.dec_count(), 6);

    // A 7th same-period NAK: DecCount is now 6, so "DecCount<=5" fails and
    // Step 4 must NOT apply another decrease.
    cc.on_loss(15, 26, 0.9);
    assert_eq!(
        cc.pkt_snd_period_us(),
        expected,
        "must not decrease further once DecCount > 5"
    );
    assert_eq!(
        cc.dec_count(),
        6,
        "DecCount stops advancing once the cutoff is hit"
    );
}

/// Bonus: a NAK within the 2% loss-ratio tolerance must not change
/// `PKT_SND_PERIOD` at all (rule 16-17), only record `LastDecPeriod`.
#[test]
fn low_loss_ratio_nak_is_tolerated_and_does_not_decrease() {
    let mut cc = FileCc::new(0);
    cc.on_loss(1, 1, 0.9); // -> Congestion Avoidance
    let period = cc.pkt_snd_period_us();

    cc.on_loss(50, 50, 0.01); // 1% loss ratio, below the 2% tolerance.
    assert_eq!(
        cc.pkt_snd_period_us(),
        period,
        "a NAK within the 2% tolerance must not change PKT_SND_PERIOD"
    );
    assert!(
        cc.b_loss(),
        "bLoss is still set at Step 1 regardless of Step 2's early return"
    );
}
