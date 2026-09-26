# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
### Fixed
- **Loudness Range (LRA) was computed from 400 ms momentary blocks instead
  of 3 s short-term values** (EBU Tech 3342 defines it over the latter,
  sampled at ≥10 Hz). 400 ms loudness has far higher variance on real
  programme material than 3 s loudness, so LRA came out badly inflated on
  anything but a steady tone (the existing compliance tests only used
  steady tones, so they couldn't see it). Verified against ffmpeg's own
  `ebur128` filter on a synthetic fast-alternating-level fixture: ffmpeg
  reports `LRA: 0.2 LU`, this crate previously reported `~15 LU` on the
  identical file (issue #1051). Also cross-checked against a second,
  slower-moving (segment-scale) synthetic programme signal to a tighter
  ±0.1 LU (ffmpeg reports `LRA: 15.0 LU`), and confirmed the existing
  `lra_case_1`..`lra_case_4` compliance tests already are the EBU Tech 3342
  Table 1 "minimum requirements" signals verbatim (cases 5/6 need the
  EBU's own real-programme reference files, not synthesizable).
- **Both meters buffered every sample for the whole measurement** —
  `LoudnessMeter` stored one `f64` of weighted power per sample frame
  (~1.4 GB/hour at 48 kHz) and only analysed it in `finish()`;
  `TruePeakMeter` stored every raw sample and rebuilt a 4×-oversampled
  `Vec` from scratch on every `finish()`/`current_level()` call (~5.5 GB/hour
  per channel, and quadratic if polled live). Both are now streaming:
  `LoudnessMeter` accumulates 100 ms sub-blocks (O(duration), not
  O(sample count) — about 36 000 `f64`s/hour, not 172 million) plus two
  small fixed-size sliding-window rings (bounded to each window's sample
  count) for exact per-sample momentary/short-term maxima;
  `TruePeakMeter` keeps only a 12-sample shift register and a running max.
  `current_level()` is now an O(1) read instead of an O(N)-with-a-4N-
  allocation reprocess. Verified to still match the existing EBU Tech 3341
  compliance vectors exactly (including the per-sample-aligned Max M/Max S
  cases) — momentary/short-term maxima and true-peak levels are unchanged,
  computed the same way, just incrementally rather than from a full buffer
  (issue #1072).
- `TruePeakMeter::finish()` now also flushes an 11-sample zero tail through
  the FIR before taking the final max: the filter is causal, so the last
  few real samples' true nearest interpolated peak position can fall after
  them, and previously that contribution was never evaluated.

## [0.3.0] - 2026-08-11

### Changed
- MSRV raised to **1.95.0** (issue #949). This removes the workspace's MSRV
  split: `webrtc-runtime`'s optional `media` feature needed rustc 1.88 (via
  `rcgen`), which had grown a dedicated CI job, six `--exclude` lanes and a
  guard script to contain. Adopting let-chains and `is_multiple_of` where the
  1.95 lints require them; no functional or API change.
### Removed
- Dead public `Error::NotImplemented` variant (#941 row 4) — never
  constructed anywhere in the crate. `Error` is `#[non_exhaustive]`, so
  this is not a breaking change for well-formed `match` callers.

## [0.2.0] - 2026-08-07

### Changed
- **BREAKING:** `LoudnessMeter::new()` now accepts any positive sample rate
  (44100, 48000, 96000, 192000, etc.) by deriving K-weighting biquad
  coefficients via bilinear transform from the analog prototype filters
  (matching libebur128/ffmpeg). At 48 kHz the derived coefficients match the
  BS.1770-5 Annex 1 tabulated values to within 1e-12 epsilon (#907).
- **BREAKING:** `filter::shelving_coeffs()` and `filter::high_pass_coeffs()`
  replaced by `filter::k_weighting_coeffs(sample_rate)` which returns both
  stages for the given rate. `BiquadCoeffs` is now re-exported.
- **BREAKING:** `Error::UnsupportedSampleRate` renamed to
  `Error::InvalidSampleRate` (now only rejects sample rate 0).

## [0.1.0] — 2026-08-05

### Added
- Initial `broadcast-loudness` crate implementing EBU R 128 / ITU-R BS.1770-5.
- K-weighting biquad filter with exact BS.1770-5 Annex 1 coefficients (48 kHz).
- `LoudnessMeter`: momentary (400 ms), short-term (3 s), and integrated (gated)
  loudness measurement in LUFS (ITU-R BS.1770-5, EBU Tech 3341).
- `TruePeakMeter`: 4× polyphase FIR oversampling per BS.1770-5 Annex 2.
- Loudness Range (LRA) per EBU Tech 3342 (percentile-based).
- `ChannelLayout` enum with BS.1770-5 Table 3 G_i channel weights.
- EBU Tech 3341 compliance test vectors (cases 1–6, 9–12, 15–19).
- EBU Tech 3342 LRA compliance test vectors (cases 1–4).
- Cases 7–8 (authentic programme) and 20–23 (complex true-peak) skipped —
  require EBU reference WAV files not included in this repo.
- `no_std` + `alloc` support; bare-metal `thumbv7em-none-eabi` target builds.
- `#![warn(missing_docs)]`; spec citations in module docs.
