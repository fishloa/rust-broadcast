---
status: DRAFT
---
# dvb-conformance 12.0.0

_Released 2026-10-16._

**Breaking release.** Lockstep major. `ConformanceEvent::pid` is `Option<mpeg_ts::pid::Pid>` (serde output unchanged: `Pid` serializes as the number); the 27 MHz PCR arithmetic moved to `broadcast_common::clock33`; the per-PID continuity state no longer allocates per packet (~9% faster on the 2.4 MB T2-MI fixture).

## Why this version

- **Proposed version:** 11.1.0 -> **12.0.0** (major-class: semver major).
- **Own API/behaviour change class:** major (ConformanceEvent::pid is Option<Pid>).
- **cargo-semver-checks vs live 11.1.0 (all features):** none flagged. (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** none needed (own breaking change). Siblings it depends on that move epoch in this wave: broadcast-common, dvb-si, mpeg-ts.
- **Caret epoch:** 11 -> 12; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
-dvb-si = "11.1"
+dvb-si = "12.0"
-mpeg-ts = "0.5"
+mpeg-ts = "0.6"
-broadcast-common = "9.3" # dev-dependency
+broadcast-common = "10.0" # dev-dependency
```

## Breaking changes and migration

```rust
// before
if let Some(pid) = event.pid { println!("{pid:04X}"); }     // pid: u16
// after
if let Some(pid) = event.pid { println!("{pid:04X}"); }     // pid: Pid (UpperHex kept); pid.value() for the u16
```

## What changed (from CHANGELOG `[Unreleased]`)

#### Changed
- Internal: the 27 MHz PCR constants / modular delta / `base * 300 + extension` arithmetic now come from `broadcast_common::clock33` (`PCR_HZ`, `PCR_WRAP_27MHZ`, `pcr_forward_distance`, `pcr_from_parts`, `pcr_split`) instead of a local copy; no behaviour change (#1206).

#### Changed
- The per-PID continuity-counter state reuses its `last_packet` buffer instead
  of allocating a fresh `Vec` on every payload-bearing packet (~9% less time
  per packet on the 2.4 MB T2-MI fixture, release build; r11-O-CONF-1, #1096).

#### Changed (BREAKING)

- **BREAKING (#1159):** `ConformanceEvent::pid` is `Option<mpeg_ts::pid::Pid>` instead of `Option<u16>` (serde output unchanged: `Pid` is transparent).

## Merged changes since `v11.1.0`

- tests: consume deterministic excerpts of the france-tnt/hotbird captures instead of the 540 MB LFS originals (#1263) (#1264)
- tests: one fixture resolver (public -> private/fixtures -> .test-streams); move local-only captures to the private repo (#1259) (#1260)
- test: shared test-fixtures loader crate, migrate hand-written loaders, drop CRC copies (#1214) (#1215)
- clock33: shared PCR constants + modular distance; migrate 5 crates; fix ts-fix post-wrap observations (#1206) [stacked on #1196] (#1207)
- Cleanups: remove panicking Serialize::to_bytes (BREAKING, #1143) + serde default-features off in 32 no_std manifests with CI thumbv7em serde matrix (#1183) (#1185)
- Audit critical-bug verification pass: 72 issues re-verified against main; fixes for #1012 #1022 #1034 #1035 #1056 + regression tests (NO release) (#1180)
- Audit per-crate rollups: verified against main, 54 commits of real fixes across ~40 crates (BREAKING items marked, NO release) (#1179)
- Typed ids wave 2: TrackId/ProgramNumber + HLS naming newtypes, Pid inside dvb-si (BREAKING, NO release) (#1178)
- Typed ids wave 1: Pid newtype (mpeg-ts and all non-dvb-si APIs) + reserved-bit preservation for all dvb-si descriptors and tables (BREAKING, NO release) (#1177)
