# broadcast-loudness 0.10.0

_Released 2026-11-22._

Minor for a 0.x crate. No API change of its own: the release moves a class because the crate builds against `broadcast-common` 14 (the jiff 1.0 epoch). The work is the audit-21 test-hardening pass over the BS.1770-5 filter and the EBU Tech 3342 measurements.

## Breaking changes

- Builds against `broadcast-common` 14 (its `jiff` 1.0 epoch). A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2) whatever this crate's own API diff; a consumer must resolve a single `broadcast-common` 14.

## Tests

- The true-peak polyphase FIR table is pinned against the BS.1770-5 Annex 2 transcription (plus phase symmetry), together with the lone-impulse tail flush, both resets and the non-finite error indices (#3233).
- Meter resets, frame/duration accounting on every entry point, the 30-sub-block short-term boundary, the Tech 3342 percentile ranks in both histogram modes, the straddling-bin cut and the empty-measurement result are pinned (#3234).
- `.cargo/mutants.toml` excludes the test-only `loudness_meter::tests::gate_edge` helpers, which cargo-mutants otherwise treats as production code because the test module also gates on `std` (#3235).

## Pre-release audit

This crate carries no crate-specific finding; it takes the workspace-wide release-documentation issues #4004 (release note) and #4008 (epoch entry).

---

Published from tag `broadcast-loudness-v0.10.0`.
