# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] — unreleased

### Added
- Initial `broadcast-loudness` crate implementing EBU R 128 / ITU-R BS.1770-5.
- K-weighting biquad filter with exact BS.1770-5 Annex 1 coefficients (48 kHz).
- `LoudnessMeter`: momentary (400 ms), short-term (3 s), and integrated (gated)
  loudness measurement in LUFS (ITU-R BS.1770-5, EBU Tech 3341).
- `TruePeakMeter`: 4× polyphase FIR oversampling per BS.1770-5 Annex 2.
- Loudness Range (LRA) per EBU Tech 3342 (percentile-based).
- `ChannelLayout` enum with BS.1770-5 Table 3 G_i channel weights.
- EBU Tech 3341 compliance test vectors (cases 1–6, 9, 11, 15–19).
- EBU Tech 3342 LRA compliance test vectors (cases 1–3).
- `no_std` + `alloc` support; bare-metal `thumbv7em-none-eabi` target builds.
- `#![warn(missing_docs)]`; spec citations in module docs.
