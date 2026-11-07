---
status: DRAFT
---

# broadcast-loudness 0.6.0

_Released 2026-11-07._

Breaking-class release (0.5 -> 0.6): the unused public function `integrated_lu` is removed, sample rates above 768 kHz are now rejected, and the EBU Tech 3341 true-peak cases 20-23 are verified.

Version: 0.5.0 -> 0.6.0.

## Breaking changes and migration

What the CHANGELOG records as breaking:

- Unused public items, with no use in the workspace beyond their own unit tests: `integrated_lu`.

### Fix: sample-rate memory blow-up

`LoudnessMeter::new` used to size its sliding-window trackers from the sample rate, so a `u32` rate near 4 GHz requested about 96 GB and aborted the process. It now returns `Error::InvalidSampleRate` for any rate above the new `MAX_SAMPLE_RATE` (768 kHz).

## Added

- `MAX_SAMPLE_RATE` (768 kHz).
- EBU Tech 3341 Table 1 true-peak cases 20-23 verified against the EBU reference WAVs (private fixtures; measured -0.15, -0.08, -0.20, -0.08 dBTP, all within 0.0 +0.2/-0.4), #1635.

## Fixed

- `LoudnessMeter::new` rejects sample rates above `MAX_SAMPLE_RATE` with `InvalidSampleRate`; a `u32` rate near 4 GHz used to request about 96 GB for the sliding-window trackers and abort the process.

Internal changes with no public effect (shared helpers replacing private copies, README as crate-root docs, `Clone`/`PartialEq`/`Eq` derives on error enums where every field allows it) are listed in the CHANGELOG.

## Dependencies

Workspace sibling requirements move to the new epochs of this wave:

- `broadcast-common` 10 -> 11

These are caret-epoch moves, so a project that names several of these crates must move them together.

---

Published from tag `broadcast-loudness-v0.6.0`.
