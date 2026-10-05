# broadcast-loudness 0.4.0

_Released 2026-10-05._

Breaking release (0.3 -> 0.4) for the EBU R 128 / ITU-R BS.1770-5 meters. There is one API break, a rename, plus measurement-correctness fixes that change reported numbers and a rewrite of both meters to stream instead of buffering every sample. If you call `LoudnessMeter::push_interleaved_f32` you must rename it. If you rely on loudness range (LRA) from real programme material or on `integrated_lufs()` of short clips, expect different, more correct values (see Behaviour changes). Memory use drops from linear in sample count to linear in duration at 100 ms granularity.

## Breaking change: `push_interleaved_f32` is `push_stereo_planar_f32` (#1108 LOUD-W1)

The method takes two separate planar channel buffers (`left`, `right`), not one interleaved buffer, so the old name was wrong. No deprecated alias is kept (the changelog notes no in-workspace consumer used it outside this crate's own tests and examples). It also now validates the configured layout is stereo before indexing it.

```rust
// before (0.3)
meter.push_interleaved_f32(&left, &right)?;
// after (0.4)
meter.push_stereo_planar_f32(&left, &right)?;
```

If you meant to feed an interleaved buffer, that was never what this method did; use `push_f32` / `push_f64`, which take one frame's channel samples per call, or de-interleave first.

## Behaviour changes that alter results

- **Loudness range (LRA) is computed from 3 s short-term values, not 400 ms momentary blocks** (issue #1051). EBU Tech 3342 defines LRA over short-term loudness sampled at 10 Hz or more. 400 ms loudness has far higher variance on real programme material, so 0.3's LRA was badly inflated on anything but a steady tone; the existing compliance tests only used steady tones, so they could not see it. Verified against ffmpeg's `ebur128` filter on a synthetic fast-alternating-level fixture: ffmpeg reports `LRA: 0.2 LU`, 0.3 reported about 15 LU on the identical file. Also cross-checked on a slower, segment-scale synthetic programme to within 0.1 LU (ffmpeg reports `LRA: 15.0 LU`). The existing `lra_case_1` to `lra_case_4` compliance tests already are the EBU Tech 3342 Table 1 minimum-requirements signals verbatim; cases 5 and 6 need the EBU's own real-programme reference files and cannot be synthesized. Expect a lower `loudness_range()` than 0.3 on real material.
- **A trailing partial 100 ms sub-block is discarded on `finish()`** (#1108 LOUD-W2). It used to be averaged and folded into the last 400 ms gating block and 3 s short-term window as if it were a complete sub-block, but BS.1770-5 gating uses complete blocks only. A 20 ms burst at a very different level appended to a steady 700 ms tone could swing `integrated_lufs()` by several LU purely through this partial sub-block.
- **`TruePeakMeter::finish()` flushes an 11-sample zero tail through the FIR** before taking the final maximum. The filter is causal, so the true nearest interpolated peak position of the last few real samples can fall after them; that contribution was never evaluated before.
- Momentary and short-term maxima, and true-peak levels, are otherwise computed the same way as before and still match the EBU Tech 3341 compliance vectors exactly, including the per-sample-aligned Max M and Max S cases.

## Fixes

- **Panic on non-stereo layouts (#1108 LOUD-W1).** The stereo-push method indexed `self.filters[1]` unconditionally, so a `Mono` meter (a one-element `Vec`) panicked. It now returns `Error::ChannelMismatch`. Separately, `ChannelLayout::Mono.weight(i)` returned `1.0` for every `i`, not just channel 0; it now returns `0.0` past channel 0.
- **Unbounded memory growth (issue #1072).** `LoudnessMeter` stored one `f64` of weighted power per sample frame (about 1.4 GB per hour at 48 kHz) and analysed it only in `finish()`. `TruePeakMeter` stored every raw sample and rebuilt a 4x-oversampled `Vec` from scratch on every `finish()` or `current_level()` call (about 5.5 GB per hour per channel, and quadratic if polled live). Both are now streaming. `LoudnessMeter` accumulates 100 ms sub-blocks (about 36,000 `f64`s per hour instead of 172 million) plus two small fixed-size sliding-window rings, bounded to each window's sample count, for exact per-sample momentary and short-term maxima. `TruePeakMeter` keeps a 12-sample shift register and a running maximum, so `current_level()` is an O(1) read instead of an O(N) pass with a 4N allocation. Values are unchanged apart from the items above.

## Dependencies

`broadcast-common` `9.3` -> `9.4` (see `broadcast-common-9.4.0.md`). No other `Cargo.toml` change.

---

Published from tag `broadcast-loudness-v0.4.0`.
