# broadcast-loudness 0.2.0

Released 2026-08-07.

### Changed (BREAKING)

- `LoudnessMeter::new()` now accepts **any positive sample rate** (44100,
  48000, 96000, 192000, etc.) by deriving K-weighting biquad coefficients via
  bilinear transform from the analog prototype filters (matching
  libebur128/ffmpeg). At 48 kHz the derived coefficients match the BS.1770-5
  Annex 1 tabulated values to within 1e-12 (#907, #913).
- `filter::shelving_coeffs()` and `filter::high_pass_coeffs()` replaced by
  `filter::k_weighting_coeffs(sample_rate)` which returns both stages.
  `BiquadCoeffs` re-exported.
- `Error::UnsupportedSampleRate` renamed to `Error::InvalidSampleRate` (now
  only rejects sample rate 0).
