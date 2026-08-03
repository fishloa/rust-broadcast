# broadcast-loudness

EBU R 128 / ITU-R BS.1770-5 loudness measurement for Rust.

[![crates.io](https://img.shields.io/crates/v/broadcast-loudness.svg)](https://crates.io/crates/broadcast-loudness)
[![docs.rs](https://docs.rs/broadcast-loudness/badge.svg)](https://docs.rs/broadcast-loudness)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](#license)

K-weighting filter, integrated/short-term/momentary loudness (LUFS),
Loudness Range (LRA, LU), and true-peak level (dBTP).

`no_std` + `alloc`, depends only on `broadcast-common`.

## Quick start

```rust
use broadcast_loudness::{ChannelLayout, LoudnessMeter};

let mut meter = LoudnessMeter::new(48_000, ChannelLayout::Stereo).unwrap();
meter.push_interleaved_f32(&left_samples, &right_samples).unwrap();
meter.finish();

println!("Integrated: {:.1} LUFS", meter.integrated_lufs());
println!("LRA:       {:.1} LU",   meter.loudness_range());
println!("Max M:     {:.1} LUFS", meter.max_momentary_lufs());
println!("Max S:     {:.1} LUFS", meter.max_short_term_lufs());
```

## Features

- **K-weighting filter** — exact BS.1770-5 Annex 1 biquad coefficients (48 kHz)
- **Channel weighting** — Table 3 G_i weights (mono, stereo, 5.1, custom)
- **Gating** — absolute (−70 LUFS) and relative (−10 LU) per BS.1770-5 Annex 1
- **Three time scales** — momentary (400 ms), short-term (3 s), integrated (gated)
- **Loudness Range (LRA)** — EBU Tech 3342 percentile-based
- **True-peak** — 4× oversampling, 48-tap polyphase FIR (BS.1770-5 Annex 2)
- **`no_std` + `alloc`** — runs on bare-metal (`thumbv7em-none-eabi`)

## Compliance

Passes EBU Tech 3341 minimum-requirements compliance tests (cases 1–6, 9, 11,
15–19) and EBU Tech 3342 LRA tests (cases 1–3). Spec citations in `docs/`.

**ATSC A/85 is out of scope.** Only 48 kHz input is supported (the K-weighting
coefficients in BS.1770-5 are tabulated only for 48 kHz).

## License

MIT OR Apache-2.0
