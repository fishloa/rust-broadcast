# compliance-probe 0.11.0

_Released 2026-11-22._

Minor for a 0.x crate. No API change of its own: the crate builds against the wave's epochs (`broadcast-common` 14, `dvb-conformance` 16, `dvb-si` 16, `media-doctor` 0.18, `media-plane` 0.14, `mpeg-ts` 0.10, `scte35-splice` 8, `timed-metadata` 0.12), which is the breaking part. The work is the audit-23 test pass over the PCR drift/jitter tracker and the probe configuration.

## Breaking changes

- Builds against new caret epochs of workspace siblings. A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2) whatever this crate's own API diff; consumers must resolve a single copy of each, and the `Trunk`/indicator/`splice_insert` types this crate takes are those versions' types.

## Tests

- **The PCR drift/jitter tracker's early-return guard, discontinuity boundary and jitter sign are pinned.** Only a first observation reached the repeat guard, and every other test either ran a 0 ppm stream or read drift only; a repeated PCR at a later arrival, a new PCR at the same arrival, an exact duplicate, a 20 s PCR-side jump, the exact 10 s guard (exclusive) and `|interval - drift|` versus `interval + drift` now each have a test asserting the literal drift/jitter (audit 23 findings 117-119, #3425/#3426/#3427).
- **`Probe::with_config` and `Probe::indicator_counts` are pinned.** No test built a probe with caller thresholds, so a probe silently using the default TR 101 290 intervals could have shipped; the only `indicator_counts` test compared against media-doctor on a one-packet fixture that raises no events. A 100 µs PAT interval now raises its indicator only through `with_config`, and a TEI-carrying stream pins the per-indicator map and its sum (audit 23 findings 120-121, #3428/#3429).

## Pre-release audit

Issues fixed in this crate by the 2026-11-22 pre-release audit: #4022 (test bullets under a heading).

---

Published from tag `compliance-probe-v0.11.0`.
