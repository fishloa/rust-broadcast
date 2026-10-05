# compliance-probe 0.2.0

_Released 2026-10-05._

### Fixed
- `scte35::check_section` now applies `pts_adjustment` (ANSI/SCTE 35 §9.6.1)
  to `pts_time` before judging future-vs-past. A cue passed through
  equipment that rebases splice times via `pts_adjustment` was previously
  judged on the raw, un-rebased `pts_time` (#1097, W-CP-1).
- The PCR drift tracker now resets a PID's baseline when the adaptation
  field's `discontinuity_indicator` is set, instead of measuring drift
  across a broadcaster-signalled discontinuity (e.g. an ad-insertion
  splice), which previously reported tens of millions of ppm of spurious
  drift for several seconds after every such splice (#1097, W-CP-2).
- The Trunk-cursor SCTE-35 path (`trunk_bridge::check_event`) now compares
  its already wrap-unrolled `target`/`now` 90 kHz values directly instead of
  reusing `scte35::judge` (which reduces modulo 2^33). On a long-running
  Trunk, a `target` more than ~13.3 hours ahead of `now` was previously
  folded back onto the 33-bit ring and misjudged `InPast` (#1097, W-CP-3).

---

Published from tag `compliance-probe-v0.2.0`.
