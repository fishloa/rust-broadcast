//! Unit-level tests against independent oracles: a brute-force sliding window,
//! an exact rational / floating Q15 reference, the RFC 3550 A.8 algorithm
//! (`specs/rfc3550_rtp.txt`, hand-ported below), the RFC 5481 PDV definition
//! computed directly, and a brute-force replay of the stall max-hold.
//! No test compares a component to itself.

mod support;

use buffering_runtime::histogram::ForgettingHistogram;
use buffering_runtime::window::MinWindow;
use buffering_runtime::{DeliveryJitter, JitterConfig, SampleOutcome, StallMemorySnapshot};
use support::*;

// ------------------------------------------------------------------ window

#[test]
fn min_window_matches_a_brute_force_scan() {
    let samples = if std::env::var_os("RUST_BROADCAST_SOAK").is_some() {
        20_000
    } else {
        3_000
    };
    for (window, seed) in [(1_000u64, 1u64), (2_000_000, 2), (17, 3), (500, 4)] {
        let mut w = MinWindow::new(window, 4096).unwrap();
        let mut rng = Rng(seed);
        let mut all: Vec<(u64, i64)> = Vec::new();
        let mut t = 0u64;
        for _ in 0..samples {
            let prev_t = all.last().map(|a| a.0);
            t += rng.below(window / 4 + 2); // equal timestamps happen
            if rng.below(200) == 0 {
                t += window + rng.below(window); // a stall at least as long as the window
            }
            let v = rng.below(2001) as i64 - 1000;
            all.push((t, v));
            let got = w.push(t, v);
            // brute force: min over samples not yet expired (`s.t + window > ref`), where
            // ref is `t`, except after a silence >= window, when expiry is judged at the
            // PREVIOUS sample's time (long-gap rule: the pre-stall baseline survives once)
            let long_gap = prev_t.is_some_and(|p| t - p >= window);
            let reference = if long_gap { prev_t.unwrap() } else { t };
            let want = all
                .iter()
                .filter(|(s, _)| s + window > reference)
                .map(|&(_, x)| x)
                .min()
                .unwrap();
            assert_eq!(got, want, "window {window} t {t}");
            assert_eq!(w.min(), Some(want));
        }
    }
}

#[test]
fn min_window_cap_drops_the_oldest_and_counts() {
    let mut w = MinWindow::new(u64::MAX / 2, 8).unwrap();
    for i in 0..20 {
        w.push(i, 100 + i64::try_from(i).unwrap()); // ascending: nothing popped by dominance
    }
    assert_eq!(w.len(), 8);
    assert_eq!(w.overflows(), 12);
    assert_eq!(w.min(), Some(112));
    assert!(MinWindow::new(0, 8).is_err());
    w.clear();
    assert!(w.is_empty());
    assert_eq!(w.min(), None);
}

#[test]
fn min_window_survives_extreme_times() {
    let mut w = MinWindow::new(1_000, 16).unwrap();
    w.push(u64::MAX - 1, 5);
    w.push(u64::MAX, 7);
    assert!(w.min().is_some());
}

// --------------------------------------------------------------- histogram

#[test]
fn q15_forgetting_is_within_one_lsb_per_step_of_the_exact_rational() {
    let (buckets, bucket_us, forget) = (400u16, 10_000u32, 32113u64);
    let mut h = ForgettingHistogram::new(buckets, bucket_us, forget as u16, 990);
    let mut rng = Rng(99);
    for step in 0..3000u64 {
        let before: Vec<u32> = (0..400).map(|i| h.weight(i)).collect();
        let v = if rng.below(10) == 0 {
            rng.below(3_900_000)
        } else {
            rng.below(40_000)
        };
        h.add(v);
        // ramped factor, exact integer rational
        let f = forget.min((step << 15) / (step + 1));
        let idx = ((v / u64::from(bucket_us)) as usize).min(399);
        for (i, &prev) in before.iter().enumerate() {
            let gain = if i == idx { 32768 - f } else { 0 };
            // exact next weight * 32768 = w_prev * f + gain * 32768
            let exact_num = u64::from(prev) * f + gain * 32768;
            let got = u64::from(h.weight(i));
            assert!(
                got * 32768 <= exact_num && exact_num < (got + 1) * 32768,
                "step {step} bucket {i}: got {got}, exact {}",
                exact_num as f64 / 32768.0
            );
        }
    }
}

#[test]
fn quantile_agrees_with_a_float_reference_within_one_bucket() {
    for q_permille in [500u16, 900, 990] {
        let mut h = ForgettingHistogram::new(400, 10_000, 32113, q_permille);
        let mut rng = Rng(5 + u64::from(q_permille));
        let mut vals = Vec::new();
        for _ in 0..4000 {
            let v = match rng.below(100) {
                0..=2 => 200_000 + rng.below(300_000),
                3..=14 => 40_000 + rng.below(60_000),
                _ => rng.below(30_000),
            };
            vals.push(v);
            h.add(v);
        }
        // float reference: the same exponential weights, ramp included, sort + walk
        let f_max: f64 = 32113.0 / 32768.0;
        let n = vals.len();
        let mut weight = vec![0.0f64; n];
        for k in 0..n {
            let f = f_max.min(k as f64 / (k as f64 + 1.0));
            for w in weight.iter_mut().take(k) {
                *w *= f;
            }
            weight[k] = 1.0 - f;
        }
        let mut pairs: Vec<(u64, f64)> = vals
            .iter()
            .zip(&weight)
            .map(|(&v, &w)| (v / 10_000 * 10_000 + 10_000, w))
            .collect();
        pairs.sort_by_key(|p| p.0);
        let total: f64 = pairs.iter().map(|p| p.1).sum();
        let mut cum = 0.0;
        let mut want = 0;
        for (edge, w) in &pairs {
            cum += w;
            if cum >= f64::from(q_permille) / 1000.0 * total {
                want = *edge;
                break;
            }
        }
        let got = h.quantile_us();
        assert!(
            got.abs_diff(want) <= 10_000,
            "q{q_permille}: quantile {got} vs reference {want}"
        );
    }
}

#[test]
fn histogram_edges() {
    let mut h = ForgettingHistogram::new(4, 10, 100, 500);
    assert_eq!(h.quantile_us(), 0, "empty histogram reads 0");
    h.add(u64::MAX); // clamps to the last bucket
    assert_eq!(h.quantile_us(), 40);
    h.clear();
    assert_eq!(h.total_weight(), 0);
    // degenerate construction never panics
    let mut z = ForgettingHistogram::new(0, 0, 0, 0);
    z.add(5);
    let _ = z.quantile_us();
}

// ----------------------------------------------------------- RFC 3550 A.8

/// RFC 3550 A.8, float form, hand-ported.
struct RefJitter {
    transit: Option<i64>,
    jitter: f64,
}
impl RefJitter {
    fn feed(&mut self, arrival: i64, ts: i64) {
        let transit = arrival - ts;
        if let Some(prev) = self.transit {
            let d = (transit - prev).abs();
            self.jitter += (1.0 / 16.0) * (d as f64 - self.jitter);
        }
        self.transit = Some(transit);
    }
}
/// RFC 3550 A.8, scaled integer form, hand-ported.
struct RefJitterInt {
    transit: Option<i64>,
    jitter: i64,
}
impl RefJitterInt {
    fn feed(&mut self, arrival: i64, ts: i64) {
        let transit = arrival - ts;
        if let Some(prev) = self.transit {
            let d = (transit - prev).abs();
            self.jitter += d - ((self.jitter + 8) >> 4);
        }
        self.transit = Some(transit);
    }
}

fn run_j(transits_us: &[i64]) -> (u64, f64) {
    let mut dj = DeliveryJitter::new(JitterConfig::DEFAULT).unwrap();
    let (mut rf, mut ri) = (
        RefJitter {
            transit: None,
            jitter: 0.0,
        },
        RefJitterInt {
            transit: None,
            jitter: 0,
        },
    );
    let mut arrival_clock = 0i64;
    let mut last = 0u64;
    for (k, &tr) in transits_us.iter().enumerate() {
        let media_us = (k as i64) * 35_000;
        // arrival = media + transit, kept non-decreasing by construction
        let arrival = (media_us + tr + 5_000_000).max(arrival_clock);
        arrival_clock = arrival;
        // the estimator sees the clamped arrival; reference uses the same
        let media_27 = shift27(0, media_us * 27);
        assert!(!matches!(
            dj.on_chunk(arrival as u64, media_27, 0),
            SampleOutcome::NonMonotonicArrival | SampleOutcome::Duplicate
        ));
        rf.feed(arrival, media_us);
        ri.feed(arrival, media_us);
        let got = dj.estimate().rfc3550_jitter_us;
        assert_eq!(
            got as i64,
            ri.jitter >> 4,
            "bit-exact vs the A.8 integer form at {k}"
        );
        assert!(
            (got as f64 - rf.jitter).abs() <= 2.0,
            "within 2 us of the float form at {k}"
        );
        last = got;
    }
    (last, rf.jitter)
}

#[test]
fn rfc3550_a8_constant_transit_has_zero_jitter() {
    let (j, _) = run_j(&[1234; 400]);
    assert_eq!(j, 0);
}

#[test]
fn rfc3550_a8_alternating_transit_converges_to_the_step() {
    let v: Vec<i64> = (0..600)
        .map(|i| if i % 2 == 0 { 0 } else { 10_000 })
        .collect();
    let (j, jf) = run_j(&v);
    assert!(
        (jf - 10_000.0).abs() < 1.0 && (j as i64 - 10_000).abs() <= 1,
        "J {j} float {jf}"
    );
}

#[test]
fn rfc3550_a8_ramping_transit_converges_to_the_slope() {
    let v: Vec<i64> = (0..600).map(|i| i * 1_000).collect();
    let (j, _) = run_j(&v);
    assert!((j as i64 - 1_000).abs() <= 1, "J {j}");
}

#[test]
fn rfc3550_a8_random_transit_tracks_both_references() {
    let mut rng = Rng(7);
    let v: Vec<i64> = (0..5_000).map(|_| rng.below(40_000) as i64).collect();
    let _ = run_j(&v);
}

// ------------------------------------------- RFC 5481 PDV, direct computation

#[test]
fn pdv_equals_d_minus_the_windowed_minimum_on_a_long_random_trace() {
    let cfg = JitterConfig::DEFAULT;
    let mut dj = DeliveryJitter::new(cfg).unwrap();
    let mut rng = Rng(2024);
    // a PCR stream that crosses the 27 MHz wrap, 35 ms spacing, random delay
    let pcr0 = buffering_runtime::WRAP_27MHZ - 20 * 27_000_000;
    let mut hist: Vec<(u64, i64)> = Vec::new(); // (arrival, D)
    let mut max_hold: Vec<(u64, u64)> = Vec::new();
    let n = 300_000u64;
    for k in 0..n {
        let media_us = k * 35_000;
        let delay = if rng.below(500) == 0 {
            40_000 + rng.below(300_000)
        } else {
            rng.below(30_000)
        };
        // arrival strictly follows media + delay but must be non-decreasing
        let arrival = (media_us + delay).max(hist.last().map_or(0, |h| h.0));
        let pcr = (pcr0 + media_us * 27) % buffering_runtime::WRAP_27MHZ;
        assert_eq!(dj.on_chunk(arrival, pcr, 1316), SampleOutcome::Accepted);
        let d = arrival as i64 - media_us as i64;
        hist.push((arrival, d));
        let from = hist.partition_point(|h| h.0 + cfg.min_window_us <= arrival);
        let dmin = hist[from..].iter().map(|h| h.1).min().unwrap();
        let pdv = (d - dmin) as u64;
        assert_eq!(dj.estimate().pdv_last_us, pdv, "sample {k}");
        // brute-force max-hold replay (RFC-independent reference of the memory rule)
        let cur = max_hold
            .last()
            .map_or(0, |&(at, p)| decayed(p, arrival - at, &cfg));
        if max_hold.is_empty() || pdv >= cur {
            max_hold.push((arrival, pdv));
        }
        let (at, p) = *max_hold.last().unwrap();
        assert_eq!(
            dj.stall_peak_at(arrival),
            decayed(p, arrival - at, &cfg),
            "peak at {k}"
        );
    }
    // the memory decays after the hold and is 0 after hold + decay
    let (at, p) = *max_hold.last().unwrap();
    let t = hist.last().unwrap().0;
    for later in [
        0,
        cfg.hold_us,
        cfg.hold_us + cfg.decay_us / 2,
        cfg.hold_us + cfg.decay_us,
        cfg.hold_us + cfg.decay_us * 3,
    ] {
        assert_eq!(
            dj.stall_peak_at(t + later),
            decayed(p, t + later - at, &cfg)
        );
    }
    assert_eq!(dj.stall_peak_at(at + cfg.hold_us + cfg.decay_us), 0);
}

/// Hand-written decay: `peak` for `hold`, then linear to 0 over `decay`.
fn decayed(peak: u64, age: u64, cfg: &JitterConfig) -> u64 {
    if age <= cfg.hold_us {
        peak
    } else if age >= cfg.hold_us + cfg.decay_us {
        0
    } else {
        (u128::from(peak) * u128::from(cfg.hold_us + cfg.decay_us - age) / u128::from(cfg.decay_us))
            as u64
    }
}

#[test]
fn stall_memory_snapshot_ages_across_a_restore() {
    let cfg = JitterConfig::DEFAULT;
    let mut a = DeliveryJitter::new(cfg).unwrap();
    // a 1.4 s outlier at t = 10 s after steady chunks
    let mut t = 0u64;
    for k in 0..400u64 {
        t = k * 35_000;
        a.on_chunk(t, shift27(1000, (k * 35_000 * 27) as i64), 0);
    }
    let late = t + 1_400_000;
    a.on_chunk(late, shift27(1000, (400 * 35_000 * 27) as i64), 0);
    let snap = a.snapshot();
    assert!(snap.peak_us >= 1_300_000 && snap.age_us == 0);
    // restore 30 s later: still within the hold; 90 s later: half decayed
    let mut b = DeliveryJitter::new(cfg).unwrap();
    b.restore(snap.aged(30_000_000));
    assert_eq!(b.stall_peak_at(0), snap.peak_us);
    let mut c = DeliveryJitter::new(cfg).unwrap();
    c.restore(snap.aged(90_000_000));
    assert_eq!(c.stall_peak_at(0), snap.peak_us / 2);
    let mut d = DeliveryJitter::new(cfg).unwrap();
    d.restore(snap.aged(200_000_000));
    assert_eq!(d.stall_peak_at(0), 0);
    // anchored at the first clock reading: no extra age from the restore itself
    d.restore(StallMemorySnapshot::new(1_000_000, 0));
    d.on_chunk(77_000_000, 5, 0);
    assert!(d.stall_peak_at(77_000_000 + cfg.hold_us) >= 1_000_000);
}

#[test]
fn duplicate_and_non_monotonic_samples_are_ignored_not_fed() {
    let mut dj = DeliveryJitter::new(JitterConfig::DEFAULT).unwrap();
    assert_eq!(dj.on_chunk(1000, 100, 0), SampleOutcome::Accepted);
    assert_eq!(dj.on_chunk(1010, 100, 0), SampleOutcome::Duplicate);
    assert_eq!(dj.on_chunk(900, 200, 0), SampleOutcome::NonMonotonicArrival);
    assert_eq!(dj.estimate().samples, 1);
}

#[test]
fn media_forward_jump_beyond_the_threshold_is_a_discontinuity_but_late_data_is_not() {
    let cfg = JitterConfig::DEFAULT;
    let mut dj = DeliveryJitter::new(cfg).unwrap();
    for k in 0..100u64 {
        dj.on_chunk(k * 35_000, 27_000 + k * 35_000 * 27, 0);
    }
    // the tf-1 shape: arrival advances 1.43 s, media 35 ms -> a gap, NOT a jump
    let base = 99 * 35_000;
    assert_eq!(
        dj.on_chunk(base + 1_433_000, 27_000 + 100 * 35_000 * 27, 0),
        SampleOutcome::Accepted
    );
    // the servustv shape: media +1.3 s ahead of arrival -> discontinuity
    let a = base + 1_433_000 + 35_000;
    let m = 27_000 + (101 * 35_000 + 1_300_000) * 27;
    assert_eq!(dj.on_chunk(a, m, 0), SampleOutcome::Discontinuity);
    // media running backwards by more than the threshold -> discontinuity
    assert_eq!(
        dj.on_chunk(a + 35_000, shift27(m, -(5_000_000 * 27)), 0),
        SampleOutcome::Discontinuity
    );
    // the stall memory survived the resets
    assert!(dj.stall_peak_at(a + 35_000) >= 1_300_000);
}
