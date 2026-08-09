# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
