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

use alloc::vec::Vec;

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
#[derive(Debug, Clone)]
pub struct TruePeakMeter {
    /// Accumulated PCM samples (to be 4× oversampled).
    buffer: Vec<f64>,
    /// Current maximum absolute value after oversampling.
    max_sample: f64,
}

impl TruePeakMeter {
    /// Create a new true‑peak meter.
    #[must_use]
    pub fn new() -> Self {
        Self {
            buffer: Vec::new(),
            max_sample: 0.0,
        }
    }

    /// Reset the meter (clear accumulated samples and max).
    pub fn reset(&mut self) {
        self.buffer.clear();
        self.max_sample = 0.0;
    }

    /// Push one f32 sample.
    pub fn push_f32(&mut self, sample: f32) -> Result<(), crate::Error> {
        self.push_f64(f64::from(sample))
    }

    /// Push one f64 sample.
    ///
    /// Returns an error if `sample` is non‑finite (NaN or ±Infinity),
    /// because it would propagate through the oversampling FIR filter
    /// and permanently poison the peak measurement.
    pub fn push_f64(&mut self, sample: f64) -> Result<(), crate::Error> {
        if !sample.is_finite() {
            return Err(crate::Error::NonFiniteSample {
                index: self.buffer.len(),
                channel: 0,
                value: sample,
            });
        }
        self.buffer.push(sample);
        Ok(())
    }

    /// Push a slice of f32 samples.
    pub fn push_f32_slice(&mut self, samples: &[f32]) -> Result<(), crate::Error> {
        self.buffer.reserve(samples.len());
        for (i, &s) in samples.iter().enumerate() {
            let v = f64::from(s);
            if !v.is_finite() {
                return Err(crate::Error::NonFiniteSample {
                    index: self.buffer.len() + i,
                    channel: 0,
                    value: v,
                });
            }
            self.buffer.push(v);
        }
        Ok(())
    }

    /// Push a slice of f64 samples.
    pub fn push_f64_slice(&mut self, samples: &[f64]) -> Result<(), crate::Error> {
        for (i, &s) in samples.iter().enumerate() {
            if !s.is_finite() {
                return Err(crate::Error::NonFiniteSample {
                    index: self.buffer.len() + i,
                    channel: 0,
                    value: s,
                });
            }
        }
        self.buffer.extend_from_slice(samples);
        Ok(())
    }

    /// Finish measurement and compute the true‑peak level.
    ///
    /// Returns the maximum true‑peak level in dB TP.
    /// Returns `f64::NEG_INFINITY` if no samples were pushed.
    pub fn finish(&mut self) -> f64 {
        let oversampled = Self::oversample_4x(&self.buffer);
        for &sample in &oversampled {
            let abs = libm::fabs(sample);
            if abs > self.max_sample {
                self.max_sample = abs;
            }
        }
        if self.max_sample <= 0.0 {
            f64::NEG_INFINITY
        } else {
            20.0 * libm::log10(self.max_sample)
        }
    }

    /// Return the current maximum true‑peak level without finishing.
    #[must_use]
    pub fn current_level(&self) -> f64 {
        let oversampled = Self::oversample_4x(&self.buffer);
        let mut max_val = 0.0f64;
        for &sample in &oversampled {
            let abs = libm::fabs(sample);
            if abs > max_val {
                max_val = abs;
            }
        }
        if max_val <= 0.0 {
            f64::NEG_INFINITY
        } else {
            20.0 * libm::log10(max_val)
        }
    }

    /// 4× polyphase FIR oversampling.
    ///
    /// For each input sample, compute 4 output samples (phases 0–3)
    /// using a 48‑tap polyphase decomposition. Each phase uses 12 taps
    /// at strides of 4 through the coefficient array.
    fn oversample_4x(input: &[f64]) -> Vec<f64> {
        let n = input.len();
        if n == 0 {
            return Vec::new();
        }
        let mut output = Vec::with_capacity(n * 4);
        for i in 0..n {
            for (phase, taps) in FIR_COEFFS.iter().enumerate() {
                let mut sum = 0.0f64;
                for (t, &coeff) in taps.iter().enumerate() {
                    let input_idx = i as isize - t as isize;
                    let sample = if input_idx >= 0 {
                        input[input_idx as usize]
                    } else {
                        0.0
                    };
                    sum += coeff * sample;
                }
                let _ = phase; // used for ordering assurance
                output.push(sum);
            }
        }
        output
    }
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
