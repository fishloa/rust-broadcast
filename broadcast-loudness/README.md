# broadcast-loudness

[![crates.io](https://img.shields.io/crates/v/broadcast-loudness.svg)](https://crates.io/crates/broadcast-loudness)
[![docs.rs](https://docs.rs/broadcast-loudness/badge.svg)](https://docs.rs/broadcast-loudness)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](#msrv-and-license)

EBU R 128 / ITU-R BS.1770-5 loudness measurement for Rust: K-weighting,
momentary / short-term / integrated loudness (LUFS), loudness range (LU) and
true-peak (dBTP), plus the Chinese NRTA loudness target profiles.

## What it does / what it does not

**Does.** Measures audio the way broadcast loudness standards define it: feed it
PCM samples (f32 / f64, mono / stereo / 5.1 / custom channel weights, any sample
rate above zero) and read back loudness and peak numbers. Used by QC tools,
ingest checks, normalisers and compliance probes. `check` / `classify_*` compare
a measurement with the Chinese GY/T targets.

**Does not.**

- No audio decoding or resampling: it takes PCM. Decoding AAC/AC-3/Opus is the
  caller's job.
- It does not normalise or modify audio; it only measures.
- No wire format, so the workspace round-trip invariant does not apply; the EBU
  compliance signals are its equivalent hard gate.
- ATSC A/85 is out of scope. The EBU R 128 normalisation *targets* (-23 LUFS and
  so on) are not encoded as a profile; read the meter and compare yourself.
- GY/T 262-2012 (the Chinese programme-loudness algorithm, based on BS.1770-2)
  and GY/T 263-2012 (meter requirements) are not implemented as such: the crate
  measures with BS.1770-5, which the GY/T 282 / 377 / 379 standards accept within
  a stated 2 dB measurement-device tolerance.
- Tech 3341 / 3342 cases that need the EBU reference WAV files are not exercised.

## Standards supported

| Standard | Edition | Scope | Status | Notes |
|---|---|---|---|---|
| ITU-R BS.1770 — Algorithms to measure audio programme loudness and true-peak audio level | BS.1770-5 (11/2023) | K-weighting (Annex 1 biquads, coefficients derived for any rate by bilinear transform; match the tabulated 48 kHz values to floating-point epsilon), channel weights G_i (mono, stereo, 5.1, custom), 400 ms blocks, absolute (-70 LUFS) and relative (-10 LU) gating, true-peak by 4x oversampling with a 48-tap polyphase FIR (Annex 2) | Full | Layouts beyond 5.1 only through `ChannelLayout::Custom` weights |
| EBU R 128 — Loudness normalisation and permitted maximum level of audio signals | v5 (Nov 2023) | The measurement set it mandates: momentary (400 ms), short-term (3 s), integrated, LRA, true peak | Partial | Targets / tolerances not encoded |
| EBU Tech 3341 — Loudness metering: 'EBU Mode' metering to supplement EBU R 128 | v4 (Nov 2023) | Minimum-requirements compliance tests: cases 1-6, 9-12 (synthesised signals) and true-peak cases 15-19 pass | Partial | Cases 7, 8, 13, 14 are not exercised; cases 20-23 need the EBU reference WAVs (tests are `#[ignore]`d) |
| EBU Tech 3342 — Loudness Range: a measure to supplement EBU R 128 loudness normalisation | v4 (Nov 2023) | LRA (relative-gated percentile range); test cases 1-4 pass | Partial | Cases 5-6 need real programme files; not exercised |
| NRTA GY/T 282-2014 — DTV programme loudness and true-peak requirements | 2014 | `GY_T_282_2014` profile: -24 LKFS +-2 LU, <= -2 dBTP (clauses 4.3, 4.4, 4.6) | Full (profile constants) | Constants pinned to the standard text by unit tests |
| NRTA GY/T 377-2023 — online audio/video programme loudness requirements | 2023 | `GY_T_377_2023_QUIET` (-24 LKFS +-2 LU, <= -1 dBTP) and `GY_T_377_2023_NOISY` (-15 LKFS +-2 LU, <= -1 dBTP), clause 5.1 a), c) | Full (profile constants) | |
| NRTA GY/T 379-2023 — DTV audio loudness monitoring | 2023 | Channel monitoring (clauses 9, 10): over-limit at -22 LKFS, M10 / S3 ratios, Table 1 verdicts (<= 20 % acceptable, <= 40 % over limit, > 40 % severely over limit) | Full (helpers) | `MonitorUnit`, `MonitorVerdict` |

Editions were checked against the vendored texts in `private/specs-md/`
(`itu_r_bs1770-5_202311_loudness`, `ebu_r128_v5_202311_...`,
`ebu_tech_3341_v4_202311_...`, `ebu_tech_3342_v4_202311_...`, and the
`nrta_gyt_282_2014` / `377_2023` / `379_2023` conversions).

## Features

| Feature | Default | Effect |
|---|---|---|
| `std` | yes | Links `std` (and `broadcast-common` / `thiserror` `std`). Off gives `no_std` + `alloc`; math goes through `libm`. |

## Quick start

```rust
use broadcast_loudness::{ChannelLayout, GY_T_282_2014, LoudnessMeter, TruePeakMeter};

// 20 s of a 1 kHz stereo sine at -23 dBFS (EBU Tech 3341 case 1), 48 kHz.
let rate = 48_000_u32;
let amplitude = 10.0_f64.powf(-23.0 / 20.0);
let sine: Vec<f32> = (0..20 * rate as usize)
    .map(|i| {
        let t = i as f64 / f64::from(rate);
        (amplitude * (2.0 * std::f64::consts::PI * 1000.0 * t).sin()) as f32
    })
    .collect();

// Loudness: planar stereo in, LUFS / LU out.
let mut meter = LoudnessMeter::new(rate, ChannelLayout::Stereo).unwrap();
meter.push_stereo_planar_f32(&sine, &sine).unwrap();
meter.finish();
assert!((meter.integrated_lufs() - (-23.0)).abs() <= 0.1);

// True peak (one meter per channel), then check against a national target.
let mut tp = TruePeakMeter::new();
tp.push_f32_slice(&sine).unwrap();
let peak_dbtp = tp.finish();
assert!(peak_dbtp < -22.0 && peak_dbtp > -24.0);

let report = GY_T_282_2014.check_meters(&meter, peak_dbtp);
assert!(report.is_compliant()); // -24 LKFS +-2 LU, <= -2 dBTP
```

Two runnable examples: `cargo run -p broadcast-loudness --example measure_loudness`
and `--example measure_true_peak`. For interleaved or multichannel input use
`push_f32` / `push_f64` instead of `push_stereo_planar_f32`.

## What is implemented

- **`LoudnessMeter`**: `new(sample_rate, ChannelLayout)`, `push_f32` / `push_f64`
  (one slice per channel), `push_stereo_planar_f32`, `finish`, `reset`, then
  `integrated_lufs`, `integrated_lu`, `max_momentary_lufs`, `max_short_term_lufs`,
  `loudness_range`, `frame_count`, `duration_seconds`.
- **`TruePeakMeter`**: per-channel; `push_f32` / `push_f64` and slice forms,
  `finish` (includes a zero-tail flush on a copy, so it is idempotent),
  `current_level` (live O(1) read), `reset`.
- **`ChannelLayout`**: `Mono`, `Stereo`, `Surround51`, `Custom { weights }`.
- **`kfilter`**: `k_weighting_coeffs`, `BiquadCoeffs`, `BiquadState`,
  `apply_biquad` for consumers who want the raw filter.
- **`profile`**: `LoudnessProfile` (`GY_T_282_2014`, `GY_T_377_2023_QUIET`,
  `GY_T_377_2023_NOISY`, `CHINESE_PROFILES`) with `check` / `check_meters` returning a
  `ProfileReport`; GY/T 379 monitoring helpers (`MONITOR_OVER_LIMIT_LKFS`,
  `over_limit_ratio`, `consecutive_over_limit_ratio`, `classify_ratio`,
  `classify_units`, `ACCEPTABLE_MAX_RATIO`, `OVER_LIMIT_MAX_RATIO`).
- **`Error`**: `Finished`, `ChannelMismatch`, `InvalidSampleRate` (only for 0),
  `NonFiniteSample`.

## Design notes

- Streaming and bounded-memory: 30 minutes of audio through both meters stays
  under a 16 MiB live-allocation cap (`tests/bounded_memory.rs`).
- Non-finite input is rejected: one NaN would poison the IIR state permanently.
- `no_std` + `alloc`, depends only on `broadcast-common` (plus `thiserror`,
  `libm`); builds for `thumbv7em-none-eabi`.
- Spec enums carry the workspace `name()` + `Display` label pair;
  `#[non_exhaustive]` on `LoudnessProfile`, `ProfileReport` and the enums.
- LKFS and LUFS are the same unit.

## Related crates

Leaf crate: depends only on `broadcast-common`. Consumers that probe live
streams (for example `compliance-probe`) can feed it decoded PCM.

## MSRV and license

MSRV 1.95.0. Licensed under MIT OR Apache-2.0.
