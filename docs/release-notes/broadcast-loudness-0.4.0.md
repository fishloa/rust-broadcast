# broadcast-loudness 0.4.0

_Released 2026-10-05._

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

---

Published from tag `broadcast-loudness-v0.4.0`.
