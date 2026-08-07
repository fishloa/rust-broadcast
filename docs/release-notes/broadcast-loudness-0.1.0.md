# broadcast-loudness 0.1.0

Released 2026-08-05.

## Initial release

EBU R 128 / ITU-R BS.1770-5 loudness measurement library: K-weighting
filter, gated integrated loudness (LUFS), momentary/short-term loudness,
loudness range (LRA, EBU Tech 3342), and true peak (dBTP).

### Features

- K-weighting biquad filter with exact BS.1770-5 Annex 1 coefficients
  (48 kHz only in this release — multi-rate support added in 0.2.0).
- `LoudnessMeter`: momentary (400 ms), short-term (3 s), and integrated
  (gated) loudness measurement in LUFS (ITU-R BS.1770-5, EBU Tech 3341).
- `TruePeakMeter`: 4× polyphase FIR oversampling per BS.1770-5 Annex 2.
- Loudness Range (LRA) per EBU Tech 3342 (percentile-based).
- `ChannelLayout` enum with BS.1770-5 Table 3 channel weights.
- Verified against the EBU Tech 3341 compliance test signals (cases 1–6,
  9–12, 15–19) and EBU Tech 3342 LRA test vectors (cases 1–4).
- `no_std` + `alloc` support; bare-metal `thumbv7em-none-eabi` target builds.
