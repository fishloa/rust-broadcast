---
status: DRAFT
---
# media-doctor 0.11.0

_Released 2026-10-16._

**Breaking release.** PID-typed diagnostics: `Location::at_pid(packet, Pid)` / `Location::at_track(packet, TrackId)` make the kind explicit, `PidFlag::pid` is a `Pid`, the CC anomaly check is O(PIDs) allocations instead of O(packets), and the CLI-spawning tests no longer run without the `cli` feature.

## Why this version

- **Proposed version:** 0.10.0 -> **0.11.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** major (Location::at_track(TrackId), PidFlag::pid is Pid).
- **cargo-semver-checks vs live 0.10.0 (all features):** none flagged. (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** none needed (own breaking change). Siblings it depends on that move epoch in this wave: broadcast-common, broadcast-hls, container-probe, dvb-conformance, dvb-si, mpeg-pes, mpeg-ts, scte35-splice, timed-metadata, transmux.
- **Caret epoch:** 0.10 -> 0.11; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
-broadcast-hls = "0.3"
+broadcast-hls = "0.4"
-container-probe = "0.1"
+container-probe = "0.2"
-dvb-conformance = "11.1"
+dvb-conformance = "12.0"
-dvb-si = "11.1"
+dvb-si = "12.0"
-mpeg-pes = "0.5"
+mpeg-pes = "0.6"
-mpeg-ts = "0.5"
+mpeg-ts = "0.6"
-scte35-splice = "3.0"
+scte35-splice = "4.0"
-timed-metadata = "0.6"
+timed-metadata = "0.7"
-transmux = "0.26"
+transmux = "0.27"
-dvb-si = "11" # dev-dependency
+dvb-si = "12.0" # dev-dependency
-mpeg-ts = "0.5" # dev-dependency
+mpeg-ts = "0.6" # dev-dependency
-transmux = "0.26" # dev-dependency
+transmux = "0.27" # dev-dependency
```

## Breaking changes and migration

```rust
// before
Location::at_track(packet, 1u32)       flag.pid: u16
// after
Location::at_track(packet, TrackId::new(1))      flag.pid: Pid     // Location::pid stays u32 (it also carries a 32-bit media track_ID)
```

## What changed (from CHANGELOG `[Unreleased]`)

#### Documentation
- README rewritten to the workspace crate-README standard: what the crate does and does not do, a standards table (edition, scope, status), feature flags, a doctested quick start, the implemented API surface, design notes and related crates. Where the crate has a library target the crate-root docs are now `include_str!("../README.md")`, so docs.rs and crates.io show the same text.

#### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10, `broadcast-hls` 0.3 -> 0.4, `container-probe` 0.1 -> 0.2, `dvb-conformance` 11 -> 12, `dvb-si` 11 -> 12, `mpeg-pes` 0.5 -> 0.6, `mpeg-ts` 0.5 -> 0.6, `scte35-splice` 3 -> 4, `timed-metadata` 0.6 -> 0.7, `transmux` 0.26 -> 0.27). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

#### Changed
- Internal: the 27 MHz PCR constants / modular delta / `base * 300 + extension` arithmetic now come from `broadcast_common::clock33` (`PCR_HZ`, `PCR_WRAP_27MHZ`, `pcr_forward_distance`, `pcr_from_parts`, `pcr_split`) instead of a local copy; no behaviour change (#1206).
#### Changed
- `CcAnomalyCheck` keeps the previous packet of each PID in a fixed 188-byte array instead of a fresh `Vec` per payload packet: a clean stream is now O(PIDs) allocations, not O(packets) (audit MD-O2, #1112; `tests/cc_anomaly_allocs.rs`).

#### Changed

- **BREAKING (#1159):** `Location::at_track(packet, track_id)` takes a `transmux::TrackId` (the report's `pid` field still carries the raw number).

#### Changed (BREAKING)

- **BREAKING (#1159):** `WatchSnapshot` `PidFlag::pid` is `mpeg_ts::pid::Pid`. `Location::pid` stays `u32` (it also carries a 32-bit media `track_ID`, so narrowing it to a PID would alias track ids); new `Location::at_pid(packet, Pid)` / `Location::at_track(packet, track_id)` constructors make the kind explicit and all TS diagnostics use `at_pid`.

## Merged changes since `media-doctor-v0.10.0`

- docs: README coverage gaps (audit 24), media-doctor examples doc (#1317)
- docs: correct false 'not vendored' / 'edition unverified' claims (#1311)
- docs: cluster A crate READMEs to the crate-README standard (what/not, standards table, features, doctested quick start, API, design notes) (#1289)
- Fix the confirmed signalling/captions review findings (supersedes #1279) (#1282)
- chore: pre-release hygiene (README coverage, snippet tool, docsrs, examples, CLAUDE.md) (#1278)
- test: gate binary-spawning tests on the cli feature (required-features) (#1258)
- test: shared test-fixtures loader crate, migrate hand-written loaders, drop CRC copies (#1214) (#1215)
- clock33: shared PCR constants + modular distance; migrate 5 crates; fix ts-fix post-wrap observations (#1206) [stacked on #1196] (#1207)
- Cleanups: remove panicking Serialize::to_bytes (BREAKING, #1143) + serde default-features off in 32 no_std manifests with CI thumbv7em serde matrix (#1183) (#1185)
- Audit critical-bug verification pass: 72 issues re-verified against main; fixes for #1012 #1022 #1034 #1035 #1056 + regression tests (NO release) (#1180)
- Audit per-crate rollups: verified against main, 54 commits of real fixes across ~40 crates (BREAKING items marked, NO release) (#1179)
- Typed ids wave 2: TrackId/ProgramNumber + HLS naming newtypes, Pid inside dvb-si (BREAKING, NO release) (#1178)
- Typed ids wave 1: Pid newtype (mpeg-ts and all non-dvb-si APIs) + reserved-bit preservation for all dvb-si descriptors and tables (BREAKING, NO release) (#1177)
