//! Input validation of the two meters: the sample-rate range of
//! `LoudnessMeter::new` (c5 R5-1) and the slice pushers of `TruePeakMeter` (c5 CV5).

use broadcast_loudness::{
    ChannelLayout, Error, LoudnessMeter, MAX_SAMPLE_RATE, MIN_SAMPLE_RATE, TruePeakMeter,
};

#[test]
fn sample_rate_below_the_minimum_and_above_the_cap_are_rejected_with_the_offending_rate() {
    for bad in [
        0,
        1,
        100,
        1_000,
        2_000,
        MIN_SAMPLE_RATE - 1,
        MAX_SAMPLE_RATE + 1,
        4_000_000_000,
        u32::MAX,
    ] {
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
fn sample_rates_from_the_minimum_to_the_cap_are_accepted() {
    for ok in [
        MIN_SAMPLE_RATE,
        8_000,
        44_100,
        48_000,
        192_000,
        MAX_SAMPLE_RATE,
    ] {
        assert!(
            LoudnessMeter::new(ok, ChannelLayout::Stereo).is_ok(),
            "rate {ok}"
        );
    }
    assert_eq!(MAX_SAMPLE_RATE, 768_000);
    assert_eq!(MIN_SAMPLE_RATE, 3_364);
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

#[test]
fn every_rate_from_1_hz_to_384_khz_is_rejected_or_yields_finite_results() {
    const SWEEP_POINTS: usize = 400;
    const SWEEP_MAX_HZ: f64 = 384_000.0;
    const TONE_HZ: f64 = 1_000.0;
    const TONE_SECONDS: f64 = 5.0;
    const TONE_AMPLITUDE: f64 = 0.5;

    let mut rates: Vec<u32> = (0..SWEEP_POINTS)
        .map(|i| {
            SWEEP_MAX_HZ
                .powf(i as f64 / (SWEEP_POINTS - 1) as f64)
                .round() as u32
        })
        .collect();
    rates.extend([
        1,
        MIN_SAMPLE_RATE - 1,
        MIN_SAMPLE_RATE,
        MIN_SAMPLE_RATE + 1,
        MAX_SAMPLE_RATE,
    ]);
    rates.dedup();

    let (mut accepted, mut rejected) = (0, 0);
    for rate in rates {
        let Ok(mut meter) = LoudnessMeter::new(rate, ChannelLayout::Stereo) else {
            assert!(
                !(MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE).contains(&rate),
                "rate {rate}"
            );
            assert!(matches!(
                LoudnessMeter::new(rate, ChannelLayout::Stereo),
                Err(Error::InvalidSampleRate { got }) if got == rate
            ));
            rejected += 1;
            continue;
        };
        assert!((MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE).contains(&rate));
        accepted += 1;

        let tone_hz = TONE_HZ.min(f64::from(rate) / 4.0);
        let frames = (f64::from(rate) * TONE_SECONDS) as usize;
        let mut peak = TruePeakMeter::new();
        for i in 0..frames {
            let t = i as f64 / f64::from(rate);
            let v = TONE_AMPLITUDE * (2.0 * std::f64::consts::PI * tone_hz * t).sin();
            meter.push_f64(&[v, v]).unwrap();
            peak.push_f64(v).unwrap();
        }
        meter.finish();
        let values = [
            ("momentary", meter.max_momentary_lufs()),
            ("short-term", meter.max_short_term_lufs()),
            ("integrated", meter.integrated_lufs()),
            ("lra", meter.loudness_range()),
            ("true peak", peak.finish()),
        ];
        for (name, value) in values {
            assert!(value.is_finite(), "rate {rate}: {name} = {value}");
        }
    }
    assert!(accepted > 0 && rejected > 0);
}

#[test]
fn k_weighting_coeffs_rejects_rates_below_the_minimum_without_panicking() {
    use broadcast_loudness::kfilter::k_weighting_coeffs;
    for bad in [0, 1, 2_000, MIN_SAMPLE_RATE - 1] {
        assert!(
            matches!(k_weighting_coeffs(bad), Err(Error::InvalidSampleRate { got }) if got == bad),
            "rate {bad}"
        );
    }
    for ok in [MIN_SAMPLE_RATE, 48_000, MAX_SAMPLE_RATE] {
        assert!(k_weighting_coeffs(ok).is_ok(), "rate {ok}");
    }
}

#[test]
fn invalid_sample_rate_message_names_the_accepted_range() {
    let msg = Error::InvalidSampleRate { got: 7 }.to_string();
    assert_eq!(
        msg,
        format!("sample rate must be between {MIN_SAMPLE_RATE} and {MAX_SAMPLE_RATE} Hz, got 7 Hz")
    );
}

#[test]
fn samples_up_to_the_measurable_magnitude_are_accepted_and_louder_ones_rejected() {
    for (layout, dbfs) in [
        (ChannelLayout::Mono, 19.8),
        (ChannelLayout::Stereo, 16.8),
        (ChannelLayout::Surround51, 12.2),
    ] {
        let mut m = LoudnessMeter::new(48_000, layout).unwrap();
        let max = m.max_sample_magnitude();
        assert!((20.0 * max.log10() - dbfs).abs() < 0.05, "{layout}: {max}");
        let n = layout.channel_count();
        let mut frame = vec![0.0; n];
        frame[0] = max;
        m.push_f64(&frame).unwrap();
        frame[0] = -max;
        m.push_f64(&frame).unwrap();
        frame[n - 1] = max.next_up();
        match m.push_f64(&frame) {
            Err(Error::SampleTooLoud {
                channel,
                value,
                max: reported,
                ..
            }) => {
                assert_eq!((channel, value, reported), (n - 1, max.next_up(), max));
            }
            other => panic!("{layout}: {other:?}"),
        }
        let loud = (2.0 * max) as f32;
        let mut frame = vec![0.0f32; n];
        frame[0] = -loud;
        assert!(matches!(
            m.push_f32(&frame),
            Err(Error::SampleTooLoud { .. })
        ));
    }
    let mut m = LoudnessMeter::new(48_000, ChannelLayout::Stereo).unwrap();
    let loud = (2.0 * m.max_sample_magnitude()) as f32;
    assert!(matches!(
        m.push_stereo_planar_f32(&[0.0, 0.0], &[0.0, loud]),
        Err(Error::SampleTooLoud {
            index: 1,
            channel: 1,
            ..
        })
    ));
}

#[test]
fn a_loud_two_level_signal_inside_the_bound_keeps_its_loudness_range() {
    let rate = 48_000u32;
    let mut m = LoudnessMeter::new(rate, ChannelLayout::Mono).unwrap();
    let loud = 0.99 * m.max_sample_magnitude();
    let n = 10 * rate as usize;
    for (amp, start) in [(loud, 0), (loud * 10f64.powf(-12.0 / 20.0), n)] {
        for i in 0..n {
            let t = (start + i) as f64 / f64::from(rate);
            m.push_f64(&[amp * (2.0 * std::f64::consts::PI * 997.0 * t).sin()])
                .unwrap();
        }
    }
    m.finish();
    assert!(
        (m.loudness_range() - 12.0).abs() < 0.1,
        "{}",
        m.loudness_range()
    );
}
