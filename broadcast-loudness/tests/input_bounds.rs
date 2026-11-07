//! Input validation of the two meters: the sample-rate range of
//! `LoudnessMeter::new` (c5 R5-1) and the slice pushers of `TruePeakMeter` (c5 CV5).

use broadcast_loudness::{ChannelLayout, Error, LoudnessMeter, MAX_SAMPLE_RATE, TruePeakMeter};

#[test]
fn sample_rate_zero_and_above_the_cap_are_rejected_with_the_offending_rate() {
    for bad in [0, MAX_SAMPLE_RATE + 1, 4_000_000_000, u32::MAX] {
        match LoudnessMeter::new(bad, ChannelLayout::Stereo) {
            Err(Error::InvalidSampleRate { got }) => assert_eq!(got, bad),
            other => panic!(
                "rate {bad}: expected InvalidSampleRate, got {:?}",
                other.map(|_| ())
            ),
        }
    }
}

#[test]
fn sample_rates_up_to_the_cap_are_accepted() {
    for ok in [1, 8_000, 44_100, 48_000, 192_000, MAX_SAMPLE_RATE] {
        assert!(
            LoudnessMeter::new(ok, ChannelLayout::Stereo).is_ok(),
            "rate {ok}"
        );
    }
    assert_eq!(MAX_SAMPLE_RATE, 768_000);
}

fn tone(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| 0.8 * (2.0 * std::f64::consts::PI * 997.0 * i as f64 / 48_000.0).sin() as f32)
        .collect()
}

#[test]
fn push_f32_slice_equals_the_per_sample_path() {
    let samples = tone(4_800);
    let mut one_by_one = TruePeakMeter::new();
    for &s in &samples {
        one_by_one.push_f32(s).unwrap();
    }
    let mut sliced = TruePeakMeter::new();
    sliced.push_f32_slice(&samples[..1_000]).unwrap();
    sliced.push_f32_slice(&samples[1_000..]).unwrap();
    assert_eq!(sliced.finish(), one_by_one.finish());
}

#[test]
fn push_f32_slice_rejects_a_non_finite_sample_with_its_index() {
    let mut samples = tone(100);
    samples[37] = f32::NAN;
    let mut meter = TruePeakMeter::new();
    meter.push_f32_slice(&tone(10)).unwrap();
    match meter.push_f32_slice(&samples) {
        Err(Error::NonFiniteSample {
            index,
            channel,
            value,
        }) => {
            assert_eq!((index, channel), (10 + 37, 0));
            assert!(value.is_nan());
        }
        other => panic!("expected NonFiniteSample, got {other:?}"),
    }
    samples[37] = f32::INFINITY;
    assert!(matches!(
        TruePeakMeter::new().push_f32_slice(&samples),
        Err(Error::NonFiniteSample { index: 37, .. })
    ));
}

#[test]
fn pushing_after_finish_is_rejected_on_every_pusher() {
    let mut meter = LoudnessMeter::new(48_000, ChannelLayout::Stereo).unwrap();
    meter.finish();
    assert_eq!(meter.push_f32(&[0.0, 0.0]), Err(Error::Finished));
    assert_eq!(meter.push_f64(&[0.0, 0.0]), Err(Error::Finished));
    assert_eq!(
        meter.push_stereo_planar_f32(&[0.0], &[0.0]),
        Err(Error::Finished)
    );
}
