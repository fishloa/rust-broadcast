//! True-peak measurement (ITU-R BS.1770-5 §Annex 2).
//!
//! The true-peak algorithm:
//! 1. Attenuate by 12.04 dB (only needed in fixed‑point; skipped in float).
//! 2. 4× over-sample (insert 3 zeros between samples).
//! 3. Apply a 48‑tap 4‑phase polyphase FIR low‑pass interpolation filter.
//! 4. Take absolute value, find maximum.
//! 5. Convert to dBTP: `20·log10(max)` then add back 12.04 dB (skipped in float).
//!
//! Since we work in f64, stages 1 and 5 cancel — the max sample directly gives
//! dBTP via `20·log10(max_sample)`.
//!
//! ## Streaming, not buffering (issue #1072 / audit LOUD-C2)
//!
//! Each phase's filter is causal (`input[i - t]` for `t` in `0..TAP_COUNT`,
//! never `i + t`), so a pushed sample's 4 interpolated output points depend
//! only on the last `TAP_COUNT` input samples. The meter therefore keeps
//! only that fixed-size window (a shift register) rather than every sample
//! ever pushed: memory is O(1) in stream length instead of O(N), and
//! `current_level()` is an O(1) read of the running max rather than an
//! O(N)-with-a-4N-allocation reprocess of the whole history on every call.

use crate::error::Error;

/// Number of FIR taps per phase (and the shift-register length): the causal
/// realization's dominant tap sits mid-array (`TAP_COUNT / 2`, empirically —
/// see `FIR_COEFFS`), so the last real sample's true nearest interpolated
/// peak position can fall up to `TAP_COUNT - 1` samples *after* it — the
/// "11-sample zero tail" `finish()` flushes below.
const TAP_COUNT: usize = 12;

/// Polyphase FIR coefficients for the BS.1770‑5 Annex 2 true‑peak interpolation filter.
///
/// The 48 coefficients are stored in **phase‑major** order:
/// `coeffs[phase * 12 + tap]` where `phase ∈ {0,1,2,3}` and `tap ∈ {0..12}`.
///
/// Derived from BS.1770‑5 Annex 2, Phase 0–3 columns (12 taps each).
#[rustfmt::skip]
const FIR_COEFFS: [[f64; 12]; 4] = [
    // Phase 0
    [
         0.001_708_984_375_0,
         0.010_986_328_125_0,
        -0.019_653_320_312_5,
         0.033_203_125_000_0,
        -0.059_448_242_187_5,
         0.137_329_101_562_5,
         0.972_167_968_750_0,
        -0.102_294_921_875_0,
         0.047_607_421_875_0,
        -0.026_611_328_125_0,
         0.014_892_578_125_0,
        -0.008_300_781_250_0,
    ],
    // Phase 1
    [
        -0.029_174_804_687_5,
         0.029_296_875_000_0,
        -0.051_757_812_500_0,
         0.089_111_328_125_0,
        -0.166_503_906_250_0,
         0.465_087_890_625_0,
         0.779_785_156_250_0,
        -0.200_317_382_812_5,
         0.101_562_500_000_0,
        -0.058_227_539_062_5,
         0.033_081_054_687_5,
        -0.018_920_898_437_5,
    ],
    // Phase 2
    [
        -0.018_920_898_437_5,
         0.033_081_054_687_5,
        -0.058_227_539_062_5,
         0.101_562_500_000_0,
        -0.200_317_382_812_5,
         0.779_785_156_250_0,
         0.465_087_890_625_0,
        -0.166_503_906_250_0,
         0.089_111_328_125_0,
        -0.051_757_812_500_0,
         0.029_296_875_000_0,
        -0.029_174_804_687_5,
    ],
    // Phase 3
    [
        -0.008_300_781_250_0,
         0.014_892_578_125_0,
        -0.026_611_328_125_0,
         0.047_607_421_875_0,
        -0.102_294_921_875_0,
         0.972_167_968_750_0,
         0.137_329_101_562_5,
        -0.059_448_242_187_5,
         0.033_203_125_000_0,
        -0.019_653_320_312_5,
         0.010_986_328_125_0,
         0.001_708_984_375_0,
    ],
];

/// True‑peak meter for a single channel.
///
/// Feed PCM samples (f32 or f64), query the maximum true‑peak level in dBTP.
/// This meter processes each channel independently; for a multichannel signal,
/// use one `TruePeakMeter` per channel and take the max.
///
/// Streaming (issue #1072 / audit LOUD-C2): holds only the last
/// `TAP_COUNT` samples (a fixed-size shift register), not the whole
/// measurement — memory is O(1), not O(stream length).
#[derive(Debug, Clone)]
pub struct TruePeakMeter {
    /// Shift register of the last `TAP_COUNT` raw input samples, most
    /// recent last (index `TAP_COUNT - 1`). Zero-initialized, which is
    /// exactly the causal filter's own zero-padding for the first
    /// `TAP_COUNT - 1` samples of a fresh measurement.
    history: [f64; TAP_COUNT],
    /// Current maximum absolute value after oversampling, across every
    /// sample pushed so far.
    max_sample: f64,
    /// Total samples pushed (for error-index reporting only).
    sample_count: usize,
}

impl TruePeakMeter {
    /// Create a new true‑peak meter.
    #[must_use]
    pub fn new() -> Self {
        Self {
            history: [0.0; TAP_COUNT],
            max_sample: 0.0,
            sample_count: 0,
        }
    }

    /// Reset the meter (clear history and max).
    pub fn reset(&mut self) {
        self.history = [0.0; TAP_COUNT];
        self.max_sample = 0.0;
        self.sample_count = 0;
    }

    /// Push one f32 sample.
    pub fn push_f32(&mut self, sample: f32) -> Result<(), Error> {
        self.push_f64(f64::from(sample))
    }

    /// Push one f64 sample.
    ///
    /// Returns an error if `sample` is non‑finite (NaN or ±Infinity),
    /// because it would propagate through the oversampling FIR filter
    /// and permanently poison the peak measurement.
    pub fn push_f64(&mut self, sample: f64) -> Result<(), Error> {
        if !sample.is_finite() {
            return Err(Error::NonFiniteSample {
                index: self.sample_count,
                channel: 0,
                value: sample,
            });
        }
        self.push_into_history(sample);
        self.update_max_for_current_window();
        self.sample_count += 1;
        Ok(())
    }

    /// Push a slice of f32 samples.
    pub fn push_f32_slice(&mut self, samples: &[f32]) -> Result<(), Error> {
        for (i, &s) in samples.iter().enumerate() {
            let v = f64::from(s);
            if !v.is_finite() {
                return Err(Error::NonFiniteSample {
                    index: self.sample_count + i,
                    channel: 0,
                    value: v,
                });
            }
            self.push_into_history(v);
            self.update_max_for_current_window();
        }
        self.sample_count += samples.len();
        Ok(())
    }

    /// Push a slice of f64 samples.
    pub fn push_f64_slice(&mut self, samples: &[f64]) -> Result<(), Error> {
        for (i, &s) in samples.iter().enumerate() {
            if !s.is_finite() {
                return Err(Error::NonFiniteSample {
                    index: self.sample_count + i,
                    channel: 0,
                    value: s,
                });
            }
            self.push_into_history(s);
            self.update_max_for_current_window();
        }
        self.sample_count += samples.len();
        Ok(())
    }

    /// Shift `sample` into the history register as the newest entry.
    fn push_into_history(&mut self, sample: f64) {
        self.history.rotate_left(1);
        self.history[TAP_COUNT - 1] = sample;
    }

    /// Compute all 4 phases for the CURRENT history window (i.e. the
    /// interpolated points anchored at the newest pushed sample) and fold
    /// their absolute values into the running max.
    fn update_max_for_current_window(&mut self) {
        for phase in &FIR_COEFFS {
            let sample = oversample_one(&self.history, phase);
            let abs = libm::fabs(sample);
            if abs > self.max_sample {
                self.max_sample = abs;
            }
        }
    }

    /// Finish measurement and compute the true‑peak level.
    ///
    /// Returns the maximum true‑peak level in dB TP, including the
    /// **11-sample zero tail** (`TAP_COUNT - 1`): because the filter is
    /// causal, the last few real samples' true nearest interpolated peak
    /// position can fall after them, so this flushes that many zero
    /// samples through a COPY of the history (never mutating the meter's
    /// own state) and includes those positions in the max search too.
    /// Idempotent and safe to call between further `push_*` calls — it
    /// never mutates `self`, so more real samples pushed afterward are
    /// unaffected by the (correctly speculative) zero-tail assumption.
    ///
    /// Returns `f64::NEG_INFINITY` if no samples were pushed.
    pub fn finish(&mut self) -> f64 {
        let mut max_with_tail = self.max_sample;
        let mut tail_history = self.history;
        for _ in 0..(TAP_COUNT - 1) {
            tail_history.rotate_left(1);
            tail_history[TAP_COUNT - 1] = 0.0;
            for phase in &FIR_COEFFS {
                let sample = oversample_one(&tail_history, phase);
                let abs = libm::fabs(sample);
                if abs > max_with_tail {
                    max_with_tail = abs;
                }
            }
        }
        dbtp(max_with_tail)
    }

    /// Return the current maximum true‑peak level without finishing (and
    /// without the zero-tail flush `finish()` performs) — an O(1) read of
    /// the running max, safe to poll live on every pushed sample.
    #[must_use]
    pub fn current_level(&self) -> f64 {
        dbtp(self.max_sample)
    }
}

/// Convert a linear max sample magnitude to dBTP, per BS.1770-5 Annex 2
/// (`20*log10(max)`; stages that cancel in float are documented at the
/// module top). `<= 0.0` (including the fresh-meter default) has no defined
/// peak yet.
#[inline]
fn dbtp(max_sample: f64) -> f64 {
    if max_sample <= 0.0 {
        f64::NEG_INFINITY
    } else {
        20.0 * libm::log10(max_sample)
    }
}

/// Compute one phase's oversampled output for the CURRENT window `history`
/// (12-tap polyphase decomposition point): `history[TAP_COUNT - 1]` is the
/// current sample, `history[TAP_COUNT - 1 - t]` is `t` samples in the past.
#[inline]
fn oversample_one(history: &[f64; TAP_COUNT], taps: &[f64; TAP_COUNT]) -> f64 {
    let mut sum = 0.0f64;
    for (t, &coeff) in taps.iter().enumerate() {
        sum += coeff * history[TAP_COUNT - 1 - t];
    }
    sum
}

impl Default for TruePeakMeter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_meter_returns_neg_infinity() {
        let mut m = TruePeakMeter::new();
        assert!(m.finish().is_infinite() && m.finish() < 0.0);
    }

    #[test]
    fn zero_input_returns_neg_infinity() {
        let mut m = TruePeakMeter::new();
        m.push_f64(0.0).unwrap();
        assert!(m.finish().is_infinite() && m.finish() < 0.0);
    }

    #[test]
    fn full_scale_dc() {
        let mut m = TruePeakMeter::new();
        for _ in 0..192 {
            m.push_f64(1.0).unwrap();
        }
        let level = m.finish();
        // DC 1.0 → approximately 0 dBTP (FIR is near unity at DC)
        assert!((level - 0.0).abs() < 1.1, "got {level}");
    }

    #[test]
    fn half_scale_dc() {
        // 0.5 → 20*log10(0.5) ≈ —6.02 dBTP
        let mut m = TruePeakMeter::new();
        for _ in 0..192 {
            m.push_f64(0.5).unwrap();
        }
        let level = m.finish();
        assert!((level - (-6.02)).abs() < 1.1, "got {level}");
    }

    #[test]
    fn half_scale_sine() {
        let mut m = TruePeakMeter::new();
        let fs = 48_000.0;
        let freq = 1000.0;
        let n = 19200;
        for i in 0..n {
            let t = i as f64 / fs;
            let val = 0.5 * (2.0 * core::f64::consts::PI * freq * t).sin();
            m.push_f64(val).unwrap();
        }
        let level = m.finish();
        // 0.5 amplitude → ~—6.02 dBTP
        assert!((level - (-6.02)).abs() < 0.5, "got {level}");
    }

    #[test]
    fn rejects_nan_f64() {
        let mut m = TruePeakMeter::new();
        let err = m.push_f64(f64::NAN).unwrap_err();
        assert!(format!("{err}").contains("non-finite"));
    }

    #[test]
    fn rejects_inf_f32() {
        let mut m = TruePeakMeter::new();
        let err = m.push_f32(f32::INFINITY).unwrap_err();
        assert!(format!("{err}").contains("non-finite"));
    }

    #[test]
    fn rejects_non_finite_in_slice() {
        let mut m = TruePeakMeter::new();
        let err = m
            .push_f64_slice(&[0.5, f64::NEG_INFINITY, 0.5])
            .unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("non-finite"), "got: {msg}");
    }

    #[test]
    fn meter_not_poisoned_after_nan_rejection() {
        let mut m = TruePeakMeter::new();
        m.push_f64(0.5).unwrap();
        let _ = m.push_f64(f64::NAN);
        for _ in 0..192 {
            m.push_f64(0.5).unwrap();
        }
        let level = m.finish();
        assert!(level.is_finite(), "meter was poisoned: got {level}");
        assert!((level - (-6.02)).abs() < 1.1, "got {level}");
    }
}
