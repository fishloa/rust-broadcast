# broadcast-loudness

EBU R 128 / ITU-R BS.1770-5 loudness measurement for Rust.

[![crates.io](https://img.shields.io/crates/v/broadcast-loudness.svg)](https://crates.io/crates/broadcast-loudness)
[![docs.rs](https://docs.rs/broadcast-loudness/badge.svg)](https://docs.rs/broadcast-loudness)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](#license)

K-weighting filter, integrated/short-term/momentary loudness (LUFS),
Loudness Range (LRA, LU), and true-peak level (dBTP).

`no_std` + `alloc`, depends only on `broadcast-common`.

Chinese broadcast targets (NRTA GY/T 282-2014 / GY/T 377-2023) are available as
named profiles (`GY_T_282_2014`, `GY_T_377_2023_QUIET`, `GY_T_377_2023_NOISY`) with
`check(integrated_lkfs, true_peak_dbtp)`, plus the GY/T 379-2023 monitoring
helpers (`over_limit_ratio`, `consecutive_over_limit_ratio`, `classify_units`).

## Quick start

```rust
use broadcast_loudness::{ChannelLayout, LoudnessMeter};

// One second of planar stereo silence at 48 kHz.
let left_samples = vec![0.0_f32; 48_000];
let right_samples = vec![0.0_f32; 48_000];

let mut meter = LoudnessMeter::new(48_000, ChannelLayout::Stereo).unwrap();
meter.push_stereo_planar_f32(&left_samples, &right_samples).unwrap();
meter.finish();

println!("Integrated: {:.1} LUFS", meter.integrated_lufs());
println!("LRA:       {:.1} LU",   meter.loudness_range());
println!("Max M:     {:.1} LUFS", meter.max_momentary_lufs());
println!("Max S:     {:.1} LUFS", meter.max_short_term_lufs());
```

This snippet is compiled and run as a doctest on `LoudnessMeter::push_stereo_planar_f32`
(for interleaved or multichannel input use `push_f32` / `push_f64`).

## Features

- **K-weighting filter** — BS.1770-5 Annex 1 biquads; any sample rate greater than zero (44.1/48/96/192 kHz, ...) is accepted, the coefficients being derived by a bilinear transform of the analog prototype (matching the tabulated 48 kHz values to within floating-point epsilon); `Error::InvalidSampleRate` only for a rate of 0
- **Channel weighting** — Table 3 G_i weights (mono, stereo, 5.1, custom)
- **Gating** — absolute (−70 LUFS) and relative (−10 LU) per BS.1770-5 Annex 1
- **Three time scales** — momentary (400 ms), short-term (3 s), integrated (gated)
- **Loudness Range (LRA)** — EBU Tech 3342 percentile-based
- **True-peak** — 4× oversampling, 48-tap polyphase FIR (BS.1770-5 Annex 2)
- **Chinese target profiles** (`profile` module) — `LoudnessProfile::{GY_T_282_2014 (-24 LKFS +-2 LU, <= -2 dBTP), GY_T_377_2023_QUIET (-24 LKFS +-2 LU, <= -1 dBTP), GY_T_377_2023_NOISY (-15 LKFS +-2 LU, <= -1 dBTP)}` with `check`/`check_meters` returning a `ProfileReport`, and the GY/T 379-2023 channel-monitoring helpers (`MONITOR_OVER_LIMIT_LKFS` = -22, `over_limit_ratio` = M10, `consecutive_over_limit_ratio` = S3, `classify_ratio`/`classify_units` with the Table 1 20 % / 40 % verdicts as `MonitorVerdict`). Every constant is pinned to the standard text by a unit test.
- **Bounded memory** — a 30-minute stream through both meters stays under a 16 MiB live-allocation cap (`tests/bounded_memory.rs`).
- **Non-finite input rejected** — one NaN would poison the IIR state permanently.
- **`no_std` + `alloc`** — runs on bare-metal (`thumbv7em-none-eabi`)

## Compliance

Passes EBU Tech 3341 minimum-requirements compliance tests (cases 1–6, 9, 11,
15–19) and EBU Tech 3342 LRA tests (cases 1–3). Spec citations in `docs/`.

**ATSC A/85 is out of scope.**

## License

MIT OR Apache-2.0
