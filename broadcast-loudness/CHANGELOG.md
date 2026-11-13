# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.8.0] - 2026-11-14
### Fixed
- The `Max M`/`Max S` sliding-window sum is re-based on its ring once per window, so the incremental `+= new - outgoing` rounding no longer accumulates over a long run.
- A rejected push leaves the meter exactly as it was. `push_f32`, `push_f64` and `push_stereo_planar_f32` validate every channel and frame before filtering any, and `TruePeakMeter::push_f32_slice`/`push_f64_slice` validate the whole slice first; they used to keep the frames before the bad sample (or the earlier channels' filter state), which a retry of the corrected input then counted twice.
- `LoudnessMeter::new` rejects a `ChannelLayout::Custom` whose weights are non-finite, negative or all zero with the new `Error::InvalidChannelWeights` (`Error` is `#[non_exhaustive]`, so the variant is additive); such weights made `max_sample_magnitude` infinite or NaN and disabled the loudness ceiling check.

### Dependencies
- Sibling dependencies move to new caret epochs: `broadcast-common` 11 -> 12.

## [0.7.0] - 2026-11-08
### Fixed
- BREAKING: `kfilter::k_weighting_coeffs` returns `Result<(BiquadCoeffs, BiquadCoeffs)>` and reports a rate below `MIN_SAMPLE_RATE` as `Error::InvalidSampleRate` instead of panicking. 0.6.0 returned NaN coefficients at 0 Hz and unstable ones from 1 to 3363 Hz.
- BREAKING: `LoudnessMeter::new` rejects sample rates below the new `MIN_SAMPLE_RATE` (3364 Hz) with `InvalidSampleRate`. At or below Nyquist of the 1681.97 Hz K-weighting shelf corner the bilinear pre-warp `tan(pi*fc/fs)` wraps negative and the stage 1 poles leave the unit circle (radius 1.12 at 1 Hz, 2.14 at 1 kHz, 2.15 at 2 kHz), so 100, 1000 and 2000 Hz returned NaN/inf (#1734). Results at 44.1/48/96/192 kHz are unchanged.
- `LoudnessMeter` no longer grows its gated-loudness history without bound on a long-running stream (#1729). Each of the two block histories (400 ms gating blocks, 3 s short-term values) keeps an exact list of up to 40 000 blocks (66 min 40 s at the 100 ms hop), preallocated, and integrated loudness and LRA from it are identical to 0.6.0 bit for bit (all 27 Tech 3341/3342 compliance tests and a 0.0001 dB sweep across both relative gates). The next block folds the list once into a fixed-size 0.01 LU histogram of the same size, holding per-bin counts, exact energy sums and the lowest and highest block energy. Memory is 640 000 bytes per meter in either form (peak 960 000 bytes during a fold), allocated by `new` and `clone`. In histogram form the relative gates are applied at the exact threshold, so results match the block-list computation unless the one bin containing a relative-gate threshold holds blocks on both sides of it. Then up to all but two of that bin's blocks (all within 0.01 LU of the gate) can be gated wrongly, which can move the result by up to the size of the block-list result's own jump at that gate. LRA percentiles in histogram form are reported per bin, within 0.01 LU of the exact block value.
- The LRA absolute gate keeps a short-term value of exactly -70 LKFS (`>=`, as the EBU Tech 3342 reference does); the histogram change had made it strict. Integrated loudness keeps the strict BS.1770-5 `>`.
- BREAKING: the pushers reject a sample whose magnitude could carry a block to or above +30 LKFS, the top of the loudness histograms, with the new `Error::SampleTooLoud`, the way non-finite samples are rejected; `LoudnessMeter::max_sample_magnitude` reports the limit (about +19.8 dBFS mono, +16.8 dBFS stereo, +12.2 dBFS 5.1, from the K-weighting filter's largest gain, an impulse-response l1 norm below 3.5 at every accepted rate, and the layout's channel weights). Louder blocks used to share the top bin, so LRA collapsed toward 0 (0.0 instead of 12 at 60 dB over full scale) and the relative gate stopped working.
- `tests/no_allocation_while_metering.rs` replaces the twenty-hour run in `tests/bounded_memory.rs`, which only checked for a finite result. Counting with `test_alloc::ThreadCounting`, it requires no allocation in the first hour, exactly the two 320 000-byte histogram folds (each freeing its list) past 66 min 40 s, and nothing after. The per-block `Vec` design fails it. It runs 75 minutes of audio by default and twenty hours under `RUST_BROADCAST_SOAK=1`.
- The crate's unit tests are gated on `std`, so `cargo test -p broadcast-loudness --no-default-features` builds; it used to fail on `format!`/`Vec` in test code.

## [0.6.0] - 2026-11-07
### Removed
- BREAKING: Unused public items, with no use in the workspace beyond their own unit tests: `integrated_lu`.
### Added
- `MAX_SAMPLE_RATE` (768 kHz).
- EBU Tech 3341 Table 1 true-peak cases 20-23 verified against the EBU reference WAVs (private fixtures; measured -0.15, -0.08, -0.20, -0.08 dBTP, all within 0.0 +0.2/-0.4), #1635.
### Fixed
- `LoudnessMeter::new` rejects sample rates above `MAX_SAMPLE_RATE` with `InvalidSampleRate`; a `u32` rate near 4 GHz used to request about 96 GB for the sliding-window trackers and abort the process (c5 R5-1).
### Changed
- Error enums derive `Clone`, `PartialEq` and `Eq` wherever every field allows it (c6 C6-14).
- The crate-root docs are the README (`#![doc = include_str!("../README.md")]`) instead of a hand-synced `//!` copy (c3 DOC1).
### Dependencies
- Sibling crates move to a new epoch in this release wave: `broadcast-common` 10 -> 11. The `Cargo.toml` requirements name the new versions; no other manifest change.

## [0.5.0] - 2026-10-17
### Documentation
- README and new crate-root docs rewritten to the cluster standard: what it does / does not, standards table (BS.1770-5, R 128 v5, Tech 3341/3342 v4, GY/T 282/377/379), feature table, compiling quick start; Tech 3341 case coverage corrected to what the test suite exercises.

### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

### Added (#1236 - Chinese loudness profiles)
- `profile` module: `LoudnessProfile` target profiles `GY_T_282_2014` (-24 LKFS +-2 LU, <= -2 dBTP), `GY_T_377_2023_QUIET` (-24 LKFS +-2 LU, <= -1 dBTP) and `GY_T_377_2023_NOISY` (-15 LKFS +-2 LU, <= -1 dBTP) with `check`/`check_meters` -> `ProfileReport`; GY/T 379-2023 channel-monitoring helpers (`MONITOR_OVER_LIMIT_LKFS` -22, `over_limit_ratio` = M10, `consecutive_over_limit_ratio` = S3, `classify_ratio`/`classify_units` with the Table 1 20%/40% verdicts `MonitorVerdict`, `MonitorUnit`). Unit tests pin every constant to the standard text and the inclusive tolerance/limit edges.
### Removed
- **BREAKING:** the never-constructed `Error::ChannelOutOfRange` variant (r14-LOUD-O1, #1124).
### Added
- `tests/bounded_memory.rs`: 30 minutes of audio through both meters under a
  16 MiB live-allocation cap, proving the streaming design of #1072.

## [0.4.0] - 2026-10-05
### Changed (breaking)
- **#1108 (LOUD-W1)**: `LoudnessMeter::push_interleaved_f32` is renamed to
  `push_stereo_planar_f32` — it takes two separate planar channel buffers,
  not one interleaved buffer, and now validates the configured layout is
  stereo before indexing it (see Fixed, below). No deprecated alias is kept
  (no in-workspace consumer used the old name outside this crate's own
  tests/examples).

### Fixed
- **#1108 (LOUD-W1)**: `push_interleaved_f32` (now `push_stereo_planar_f32`)
  panicked on any non-stereo `ChannelLayout` (e.g. `Mono`, whose
  `self.filters` is a 1-element `Vec`, indexed at `[1]` unconditionally).
  It now returns `Error::ChannelMismatch`. Separately, `ChannelLayout::Mono
  .weight(i)` returned `1.0` for every `i`, not just channel 0.
- **#1108 (LOUD-W2)**: a final, in-progress 100 ms sub-block with fewer
  than a full sub-block's samples was flushed on `finish()` by averaging
  whatever had accumulated and folding it into the last 400 ms gating-block
  / 3 s short-term window on equal footing with genuinely complete
  sub-blocks — BS.1770-5 gating uses complete blocks only. A 20 ms burst at
  a very different level appended to an otherwise steady 700 ms tone could
  swing `integrated_lufs()` by several LU purely from this trailing
  partial sub-block. It's now discarded instead of flushed.
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
