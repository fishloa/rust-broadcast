//! UtcClock and UTCTiming sampling: mapping, saturation, replacement order and
//! the exchange-midpoint arithmetic (DESIGN 3.2, 3.3, 3.6).

use core::time::Duration;

use broadcast_common::stage::Timestamp;

use crate::common;

use common::Rng;
use dash_runtime::timing::TimingError;

use dash_runtime::timing::clock::{
    AdoptReason, Anchor, ClockSource, OfferOutcome, OfferPolicy, RejectReason, UtcClock,
};
use dash_runtime::timing::utc_timing::{
    Exchange, UtcSample, UtcScheme, sample_from_body, sample_from_date_header, sample_from_direct,
    sample_from_exchange,
};

const MAX_RTT: Duration = Duration::from_secs(2);

fn at(micros: u64) -> Timestamp {
    Timestamp::from_micros(micros)
}

fn micros(d: Duration) -> u64 {
    u64::try_from(d.as_micros()).unwrap()
}

fn sample(mono: u64, utc: i64, unc: u64) -> UtcSample {
    sample_from_exchange(
        Exchange::new(at(mono), at(mono)),
        utc,
        Duration::from_micros(unc),
        MAX_RTT,
    )
    .unwrap()
}

fn clock_with(mono: u64, utc: i64, source: ClockSource, unc: u64) -> UtcClock {
    let mut c = UtcClock::new();
    c.offer(
        sample(mono, utc, unc),
        source,
        at(mono),
        OfferPolicy::new(Duration::from_secs(600), false),
    );
    c
}

#[test]
fn no_anchor_no_time() {
    let c = UtcClock::new();
    assert_eq!(c.utc_at(at(5)), None);
    assert_eq!(c.mono_for_utc(5), None);
    assert!(c.anchor().is_none());
}

#[test]
fn mapping_is_monotone_and_inverts() {
    let mut rng = Rng(7);
    for _ in 0..50_000 {
        let mono = rng.below(1 << 50);
        let utc = i64::try_from(rng.below(1 << 52)).unwrap() - (1 << 51);
        let a = Anchor::from_sample(sample(mono, utc, 0), ClockSource::Host);
        let t1 = rng.below(1 << 51);
        let t2 = t1 + rng.below(1 << 40);
        assert!(a.utc_at(at(t1)) <= a.utc_at(at(t2)), "monotone in now");
        let target = utc + i64::try_from(rng.below(1 << 40)).unwrap();
        let back = a.mono_for_utc(target).unwrap();
        assert_eq!(a.utc_at(back), target);
        assert_eq!(a.utc_at(at(mono)), utc);
    }
}

#[test]
fn extremes_saturate() {
    let up = Anchor::from_sample(sample(0, i64::MAX, 0), ClockSource::Host);
    assert_eq!(up.utc_at(at(u64::MAX)), i64::MAX);
    let down = Anchor::from_sample(sample(u64::MAX, i64::MIN, 0), ClockSource::Host);
    assert_eq!(down.utc_at(at(0)), i64::MIN);
    let a = Anchor::from_sample(sample(1_000, 5, 0), ClockSource::Host);
    assert_eq!(a.mono_for_utc(i64::MIN), Some(at(0)));
    assert_eq!(
        a.mono_for_utc(i64::MAX),
        Some(at(1_000 + (i64::MAX as u64 - 5)))
    );
    let late = Anchor::from_sample(sample(u64::MAX - 10, 0, 0), ClockSource::Host);
    assert_eq!(late.mono_for_utc(i64::MAX), Some(at(u64::MAX)));
    assert_eq!(a.utc_at(at(0)), 5 - 1_000);
}

#[test]
fn midpoint_and_uncertainty_follow_the_exchange() {
    let s = sample_from_exchange(
        Exchange::new(at(1_000), at(1_400)),
        777,
        Duration::from_micros(1_000),
        MAX_RTT,
    )
    .unwrap();
    assert_eq!(s.mono_us, 1_200, "midpoint of sent and received");
    assert_eq!(
        s.uncertainty_us,
        200 + 1_000,
        "half the round trip plus the resolution"
    );
    assert_eq!(s.utc_us, 777);
    let odd =
        sample_from_exchange(Exchange::new(at(10), at(15)), 0, Duration::ZERO, MAX_RTT).unwrap();
    assert_eq!((odd.mono_us, odd.uncertainty_us), (12, 2));
    assert_eq!(
        sample_from_exchange(Exchange::new(at(10), at(9)), 0, Duration::ZERO, MAX_RTT),
        Err(TimingError::ExchangeReversed)
    );
    assert_eq!(
        sample_from_exchange(
            Exchange::new(at(0), at(micros(MAX_RTT) + 1)),
            0,
            Duration::ZERO,
            MAX_RTT
        ),
        Err(TimingError::RttTooLarge {
            rtt: MAX_RTT + Duration::from_micros(1),
            max: MAX_RTT
        })
    );
    assert!(
        sample_from_exchange(
            Exchange::new(at(0), at(micros(MAX_RTT))),
            0,
            Duration::ZERO,
            MAX_RTT
        )
        .is_ok()
    );
}

#[test]
fn date_header_gets_the_half_second() {
    let s = sample_from_date_header(
        "Mon, 26 Oct 2026 08:39:04 GMT",
        Exchange::new(at(0), at(100)),
        MAX_RTT,
    )
    .unwrap();
    assert_eq!(s.utc_us, 1_793_003_944 * 1_000_000 + 500_000);
    assert_eq!(s.uncertainty_us, 50 + 500_000);
    assert!(matches!(
        sample_from_date_header("garbage", Exchange::new(at(0), at(1)), MAX_RTT),
        Err(dash_runtime::timing::TimingError::HttpDate(..))
    ));
}

#[test]
fn bodies_by_scheme() {
    let ex = Exchange::new(at(0), at(200));
    let s = sample_from_body(
        UtcScheme::HttpXsdate,
        b"2026-10-26T08:39:04.952Z\n",
        ex,
        MAX_RTT,
    )
    .unwrap();
    assert_eq!(s.utc_us, 1_793_003_944 * 1_000_000 + 952_000);
    assert_eq!(
        s.uncertainty_us,
        100 + 1_000,
        "three fraction digits: 1 ms resolution"
    );
    let s = sample_from_body(UtcScheme::HttpIso, b"2026-10-26T08:39:04Z", ex, MAX_RTT).unwrap();
    assert_eq!(
        s.utc_us,
        1_793_003_944 * 1_000_000 + 500_000,
        "a whole-second body is centred like a whole-second Date header"
    );
    assert_eq!(
        s.uncertainty_us,
        100 + 500_000,
        "no fraction: half a second"
    );
    let mut ntp = 2_208_988_800u32.to_be_bytes().to_vec();
    ntp.extend_from_slice(&[0; 4]);
    let s = sample_from_body(UtcScheme::HttpNtp, &ntp, ex, MAX_RTT).unwrap();
    assert_eq!(s.utc_us, 0);
    assert_eq!(s.uncertainty_us, 100 + 1_000);
    assert!(matches!(
        sample_from_body(UtcScheme::HttpXsdate, &[0xff, 0xfe], ex, MAX_RTT),
        Err(TimingError::NotUtf8)
    ));
    assert!(matches!(
        sample_from_body(UtcScheme::HttpXsdate, &vec![b'x'; 5_000], ex, MAX_RTT),
        Err(TimingError::BodyTooLarge { .. })
    ));
    assert!(matches!(
        sample_from_body(UtcScheme::Ntp, b"x", ex, MAX_RTT),
        Err(TimingError::SchemeNotSampled(UtcScheme::Ntp))
    ));
    assert!(matches!(
        sample_from_body(UtcScheme::HttpXsdate, b"yesterday", ex, MAX_RTT),
        Err(dash_runtime::timing::TimingError::DateTime(_))
    ));
    let d = sample_from_direct(
        "2026-10-26T08:39:04Z",
        Exchange::new(at(0), at(400)),
        MAX_RTT,
    )
    .unwrap();
    assert_eq!(d.mono_us, 200);
    assert_eq!(d.utc_us, 1_793_003_944 * 1_000_000 + 500_000);
}

#[test]
fn replacement_order() {
    let p = OfferPolicy::new(Duration::from_secs(600), false);
    // A higher-trust source replaces at once, even when less precise.
    let mut c = clock_with(0, 1_000, ClockSource::MpdDate, 10);
    assert_eq!(
        c.offer(sample(5, 1_005, 999), ClockSource::Host, at(5), p),
        OfferOutcome::Adopted(AdoptReason::HigherTrust)
    );
    assert_eq!(
        c.offer(sample(10, 1_010, 999), ClockSource::UtcTiming, at(10), p),
        OfferOutcome::Adopted(AdoptReason::HigherTrust)
    );
    // A lower source never replaces a higher one ...
    assert_eq!(
        c.offer(sample(20, 1_020, 1), ClockSource::Host, at(20), p),
        OfferOutcome::Rejected(RejectReason::LowerTrust)
    );
    assert_eq!(c.anchor().unwrap().source, ClockSource::UtcTiming);
    // ... unless the better source failed.
    assert_eq!(
        c.offer(
            sample(20, 1_020, 1),
            ClockSource::Host,
            at(20),
            OfferPolicy::new(Duration::from_secs(600), true)
        ),
        OfferOutcome::Adopted(AdoptReason::Downgrade)
    );
    assert_eq!(c.anchor().unwrap().source, ClockSource::Host);
}

#[test]
fn same_source_refines_only_when_at_least_as_precise_unless_stale_or_stepped() {
    let p = OfferPolicy::new(Duration::from_secs(1), false);
    let mut c = clock_with(0, 1_000_000, ClockSource::UtcTiming, 100);
    // Consistent with the prediction, less precise: kept.
    assert_eq!(
        c.offer(
            sample(10, 1_000_010, 150),
            ClockSource::UtcTiming,
            at(10),
            p
        ),
        OfferOutcome::Rejected(RejectReason::LessPrecise)
    );
    assert_eq!(c.anchor().unwrap().mono, at(0));
    // Consistent, equally precise: adopted (<=).
    assert_eq!(
        c.offer(
            sample(20, 1_000_020, 100),
            ClockSource::UtcTiming,
            at(20),
            p
        ),
        OfferOutcome::Adopted(AdoptReason::Refined)
    );
    // Consistent and more precise.
    assert_eq!(
        c.offer(sample(30, 1_000_030, 50), ClockSource::UtcTiming, at(30), p),
        OfferOutcome::Adopted(AdoptReason::Refined)
    );
    // Disagreeing by exactly the two error bars refines; one more is a step.
    assert_eq!(
        c.offer(
            sample(40, 1_000_040 + 100, 50),
            ClockSource::UtcTiming,
            at(40),
            p
        ),
        OfferOutcome::Adopted(AdoptReason::Refined),
        "|diff| == bars is not a step"
    );
    let mut c = clock_with(0, 1_000_000, ClockSource::UtcTiming, 50);
    assert_eq!(
        c.offer(
            sample(40, 1_000_040 + 101, 50),
            ClockSource::UtcTiming,
            at(40),
            p
        ),
        OfferOutcome::Adopted(AdoptReason::Step)
    );
    // A step is adopted even when far less precise; the anchor moves.
    let mut c = clock_with(0, 1_000_000, ClockSource::UtcTiming, 1);
    assert_eq!(
        c.offer(
            sample(5, 9_000_000, 900_000),
            ClockSource::UtcTiming,
            at(5),
            p
        ),
        OfferOutcome::Adopted(AdoptReason::Step)
    );
    assert_eq!(c.utc_at(at(5)), Some(9_000_000));
    // An old anchor is replaced by any consistent same-source sample.
    let mut c = clock_with(0, 1_000_000, ClockSource::UtcTiming, 1);
    assert_eq!(
        c.offer(
            sample(1_000_000, 2_000_000, 999),
            ClockSource::UtcTiming,
            at(1_000_000),
            p
        ),
        OfferOutcome::Adopted(AdoptReason::Stale),
        "resync_us elapsed"
    );
    let mut c = clock_with(0, 1_000_000, ClockSource::UtcTiming, 1);
    assert_eq!(
        c.offer(
            sample(999_999, 1_999_999, 999),
            ClockSource::UtcTiming,
            at(999_999),
            p
        ),
        OfferOutcome::Rejected(RejectReason::LessPrecise),
        "one microsecond short of resync_us"
    );
}

#[test]
fn a_never_resync_policy_keeps_a_consistent_anchor_however_old() {
    let mut c = clock_with(0, 1_000_000, ClockSource::UtcTiming, 1);
    let outcome = c.offer(
        sample(10_000_000_000, 10_001_000_000, 999),
        ClockSource::UtcTiming,
        at(10_000_000_000),
        OfferPolicy::new_or_never(None, false),
    );
    assert_eq!(
        outcome,
        OfferOutcome::Rejected(RejectReason::LessPrecise),
        "age alone never replaces an anchor under `None`"
    );
    assert_eq!(c.anchor().map(|a| a.mono), Some(at(0)));
    let same = OfferPolicy::new(Duration::from_secs(1), false);
    let mut aged = clock_with(0, 1_000_000, ClockSource::UtcTiming, 1);
    assert_eq!(
        aged.offer(
            sample(10_000_000_000, 10_001_000_000, 999),
            ClockSource::UtcTiming,
            at(10_000_000_000),
            same,
        ),
        OfferOutcome::Adopted(AdoptReason::Stale),
        "the same offer is stale once a resync period is set"
    );
}

#[test]
fn a_deadline_past_the_end_of_the_clock_never_fires() {
    let late = at(u64::MAX - 1);
    assert_eq!(late.checked_add(Duration::from_micros(2)), None);
    assert_eq!(
        late.checked_add(Duration::from_micros(1)),
        Some(at(u64::MAX))
    );
}

#[test]
fn replacement_never_loses_the_most_trusted_anchor_under_random_offers() {
    let mut rng = Rng(99);
    let p = OfferPolicy::new(Duration::from_secs(1 << 20), false);
    for _ in 0..2_000 {
        let mut c = UtcClock::new();
        let mut best = ClockSource::MpdDate;
        for i in 0..40u64 {
            let source = match rng.below(3) {
                0 => ClockSource::MpdDate,
                1 => ClockSource::Host,
                _ => ClockSource::UtcTiming,
            };
            let before = c.anchor().map(|a| a.source);
            c.offer(
                sample(i, i64::try_from(i).unwrap(), rng.below(1_000)),
                source,
                at(i),
                p,
            );
            best = best.max(source);
            let now = c.anchor().unwrap().source;
            assert_eq!(
                now, best,
                "without downgrade the anchor source is the max offered"
            );
            if let Some(b) = before {
                assert!(now >= b);
            }
        }
    }
}
