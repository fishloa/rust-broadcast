//! LiveCC pacing — integration tests (`draft-sharabayko-srt-01` §5.1).
//!
//! Assertions use hand-computed constants so a trivial/constant impl must fail.
//! Spec grounding: `specs/rules/srt-livecc.md`.

use core::time::Duration;
use srt_runtime::livecc::{LiveCC, MaxBwConfig};

/// Default MAX_BW in bytes/sec (1 Gbps = 125_000_000 bytes/sec), per
/// `specs/rules/srt-livecc.md` §5.1.1 L3122-3123.
const ONE_GBPS_BYTES_PER_SEC: u64 = 125_000_000;

#[test]
fn pkt_snd_period_known_payload_and_bw() {
    // Hand-computed: with init AvgPayloadSize=1456 and default
    // MAX_BW=125_000_000:
    //   PktSize = 1456 + 16 = 1472
    //   period = 1472 * 1_000_000 / 125_000_000 = 11.776 → 11 (integer)
    let cc = LiveCC::new(Default::default());
    let got = cc.on_ack_received();
    assert_eq!(
        got,
        Duration::from_micros(11),
        "PKT_SND_PERIOD should be 11 us for init payload 1456 at 1 Gbps"
    );
}

#[test]
fn pkt_snd_period_with_smaller_avg_payload() {
    // After feeding 1316-byte payloads, the EWMA converges toward 1316.
    // After 7 identical feeds, the EWMA weight on the initial 1456 is
    // (7/8)^7 ≈ 0.393, leaving the converged value >1316. We use 64 feeds
    // to converge definitively, then hand-compute the period.
    //
    // After many 1316 feeds: AvgPayloadSize ≈ 1316.
    //   PktSize = 1316 + 16 = 1332
    //   period = 1332 * 1_000_000 / 125_000_000 = 10.656 → 10
    let mut cc = LiveCC::new(Default::default());
    for _ in 0..64 {
        cc.on_data_packet(1316);
    }
    // Converged within tolerance of 1316.
    let avg = cc.avg_payload_size();
    assert!(
        avg.abs_diff(1316) <= 1,
        "expected AvgPayloadSize ~1316, got {avg}"
    );
    let got = cc.on_ack_received();
    assert_eq!(
        got,
        Duration::from_micros(10),
        "PKT_SND_PERIOD should be 10 us for ~1316 avg payload at 1 Gbps"
    );
}

#[test]
fn maxbw_set_mode_period_matches_formula() {
    // Explicitly set MAX_BW = 62_500_000 bytes/sec (500 Mbps).
    // Init AvgPayloadSize = 1456.
    //   PktSize = 1472
    //   period = 1472 * 1_000_000 / 62_500_000 = 23.552 → 23
    //
    // This is NOT a simple division of a power-of-two — a trivial impl
    // that returns a constant or does `PktSize / MAX_BW` would fail.
    let cc = LiveCC::new(MaxBwConfig::Set(62_500_000));
    assert_eq!(cc.on_ack_received(), Duration::from_micros(23));
}

#[test]
fn maxbw_input_based_mode_period() {
    // INPUT_BW = 10_000_000 bytes/sec, OVERHEAD = 25%.
    // MAX_BW = 10_000_000 + 10_000_000 * 25 / 100 = 12_500_000 bytes/sec.
    // Init AvgPayloadSize = 1456 → PktSize = 1472
    // period = 1472 * 1_000_000 / 12_500_000 = 117.76 → 117
    let cc = LiveCC::new(MaxBwConfig::InputBased {
        input_bw: 10_000_000,
        overhead: 25,
    });
    // Trigger the period computation (as if on ACK).
    let got = cc.on_ack_received();
    assert_eq!(got, Duration::from_micros(117));
}

#[test]
fn maxbw_infinite_mode_zero_period() {
    // Infinite bandwidth → no pacing.
    let cc = LiveCC::new(MaxBwConfig::Infinite);
    assert_eq!(cc.on_ack_received(), Duration::ZERO);
}

#[test]
fn ewma_convergence_from_known_start() {
    // Initial AvgPayloadSize = 1456. Feed 1000-byte payload 64 times.
    // After many feeds the EWMA converges toward 1000.
    // We assert the direction (decreasing) and convergence.
    let mut cc = LiveCC::new(Default::default());
    assert_eq!(cc.avg_payload_size(), 1456, "init cap must be 1456");

    // After one feed of 1000: (7*1456 + 1000) / 8 = 11192 / 8 = 1399
    cc.on_data_packet(1000);
    assert_eq!(cc.avg_payload_size(), 1399);

    // After two: (7*1399 + 1000) / 8 = 10793 / 8 = 1349
    cc.on_data_packet(1000);
    assert_eq!(cc.avg_payload_size(), 1349);

    // After three: (7*1349 + 1000) / 8 = 10443 / 8 = 1305
    cc.on_data_packet(1000);
    assert_eq!(cc.avg_payload_size(), 1305);

    // Converge toward 1000 over many iterations.
    for _ in 0..61 {
        cc.on_data_packet(1000);
    }
    let avg = cc.avg_payload_size();
    assert!(
        avg.abs_diff(1000) <= 1,
        "expected convergence within 1 of 1000, got {avg}"
    );
}

#[test]
fn maxbw_estimated_mode_uses_overhead_formula() {
    // EST_INPUT_BW = 20_000_000 bytes/sec, OVERHEAD = 50%.
    // MAX_BW = 20_000_000 + 20_000_000 * 50 / 100 = 30_000_000 bytes/sec.
    // Init PktSize = 1472.
    // period = 1472 * 1_000_000 / 30_000_000 = 49.066... → 49
    let cc = LiveCC::new(MaxBwConfig::Estimated {
        est_input_bw: 20_000_000,
        overhead: 50,
    });
    assert_eq!(cc.on_ack_received(), Duration::from_micros(49));
}

#[test]
fn runtime_switch_between_modes_alters_period() {
    // Start infinite → no pacing.
    // Switch to 125_000_000 bytes/sec → period = 11 us.
    // Switch to 62_500_000 bytes/sec → period = 23 us.
    // Switch back to infinite → zero.
    let mut cc = LiveCC::new(MaxBwConfig::Infinite);
    assert_eq!(cc.on_ack_received(), Duration::ZERO);

    cc.set_max_bw_config(MaxBwConfig::Set(ONE_GBPS_BYTES_PER_SEC));
    assert_eq!(cc.on_ack_received(), Duration::from_micros(11));

    cc.set_max_bw_config(MaxBwConfig::Set(ONE_GBPS_BYTES_PER_SEC / 2));
    assert_eq!(cc.on_ack_received(), Duration::from_micros(23));

    cc.set_max_bw_config(MaxBwConfig::Infinite);
    assert_eq!(cc.on_ack_received(), Duration::ZERO);
}
