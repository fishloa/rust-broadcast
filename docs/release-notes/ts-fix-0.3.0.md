# ts-fix 0.3.0 — 2026-07-04

MPEG-2 TS stream-conditioning CLI + library. Adds PCR-discontinuity handling
alongside the existing continuity-counter / timestamp repair. Additive (minor).

## Added — PCR-discontinuity detection + repair (#562)

- **Detect** — `discontinuity::detect_pcr_discontinuities` + `PcrDiscontinuity`:
  scan a TS buffer for PCR jumps on every PCR-bearing PID, classified **flagged**
  (`discontinuity_indicator == 1`, ISO/IEC 13818-1 §2.4.3.5 — legal) vs
  **unflagged** (ETSI TR 101 290 §5.2.2 indicator 2.3b — a genuine defect). The
  threshold is reused verbatim from `dvb_conformance::ConformanceMonitor`, never
  re-derived.
- **restamp** — rewrites PCR onto one continuous timeline across a genuine
  unflagged break (freezes the pre-break rate) so the output has no PCR_disc.
  (Fixes a real bug: `Interpolate` mode previously let a sub-half-modulus
  forward jump pass through unrepaired.)
- **honor** — `TsFixBuilder::honor_pcr_discontinuity()` / `--honor-pcr-discontinuity`:
  sets `discontinuity_indicator` on genuine unflagged breaks, touching no
  timestamp byte (only the adaptation-field flag bit changes).

Verified: restamp output passes `dvb-conformance`'s PCR checks clean; honor mode
is a one-byte delta; both are byte-identical no-ops on a clean stream.

## Compatibility

Adds a dependency on `dvb-conformance` (in-workspace, ≥ 8.x) to reuse its
TR 101 290 PCR checks. Requires broadcast-common ≥ 8.4. MSRV 1.86.
