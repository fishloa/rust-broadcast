---
status: DRAFT
---
# compliance-probe 0.4.0

_Released 2026-10-16._

**Epoch-forced major-class release.** Epoch-forced release. Own change since `compliance-probe-v0.3.0`: PcrSample::pid typed (public field type; crate is unpublished-in-practice per CLAUDE.md but live 0.3.0). Its public API does not change in a way that needs source migration; it ships because a workspace sibling it depends on moves to a new caret epoch (see below), and the workspace's epoch-purity rule forbids publishing a crate that still requires the old epoch next to the new one.

## Why this version

- **Proposed version:** 0.3.0 -> **0.4.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** minor (PcrSample::pid typed (public field type; crate is unpublished-in-practice per CLAUDE.md but live 0.3.0)).
- **cargo-semver-checks vs live 0.3.0 (all features):** none flagged. (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** crate is unpublished-in-practice per CLAUDE.md but live 0.3.0); epoch cascade from broadcast-common. Siblings it depends on that move epoch in this wave: broadcast-common, dvb-conformance, media-doctor, media-plane, mpeg-ts, scte35-splice, timed-metadata.
- **Caret epoch:** 0.3 -> 0.4; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
-dvb-conformance = "11.1"
+dvb-conformance = "12.0"
-media-doctor = "0.10"
+media-doctor = "0.11"
-media-plane = "0.6"
+media-plane = "0.7"
-mpeg-ts = "0.5"
+mpeg-ts = "0.6"
-scte35-splice = "3.0"
+scte35-splice = "4.0"
-timed-metadata = "0.6"
+timed-metadata = "0.7"
```

## Breaking changes and migration

No source-level migration is required beyond bumping the version requirement and, if the crate exposes `broadcast-common` types in its API, moving to `broadcast-common` 10 in the same dependency graph.

## What changed (from CHANGELOG `[Unreleased]`)

#### Changed
- Internal: the 27 MHz PCR constants / modular delta / `base * 300 + extension` arithmetic now come from `broadcast_common::clock33` (`PCR_HZ`, `PCR_WRAP_27MHZ`, `pcr_forward_distance`, `pcr_from_parts`, `pcr_split`) instead of a local copy; no behaviour change (#1206).

#### Fixed

- PCR drift estimate: the inter-PCR delta used `u64::wrapping_sub(..) % (2^33*300)`, which is wrong whenever the PCR wraps (or steps backward) because 2^64 is not a multiple of the PCR modulus; the delta is now the true modular forward distance, so a steady PCR crossing the ~26.5 h wrap keeps ~0 ppm drift instead of being discarded as a discontinuity.

#### Changed

- `PcrSample::pid` is `mpeg_ts::pid::Pid` (#1159); the `TsHeader::pid` type change is absorbed internally.

## Merged changes since `compliance-probe-v0.3.0`

- test: shared test-fixtures loader crate, migrate hand-written loaders, drop CRC copies (#1214) (#1215)
- clock33: shared PCR constants + modular distance; migrate 5 crates; fix ts-fix post-wrap observations (#1206) [stacked on #1196] (#1207)
- fix(compliance-probe): PCR delta across wrap/backward step used u64 wrapping_sub % non-power-of-two modulus (#1196)
- Audit critical-bug verification pass: 72 issues re-verified against main; fixes for #1012 #1022 #1034 #1035 #1056 + regression tests (NO release) (#1180)
- Typed ids wave 1: Pid newtype (mpeg-ts and all non-dvb-si APIs) + reserved-bit preservation for all dvb-si descriptors and tables (BREAKING, NO release) (#1177)
