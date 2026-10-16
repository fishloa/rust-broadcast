---
status: DRAFT
---
# dvb-t2mi 12.0.0

_Released 2026-10-16._

**Breaking release.** Lockstep major. `T2miPump::new` and `InnerTsRecovery::{new,new_for_plp}` take `mpeg_ts::pid::Pid`; `InnerTsRecovery` gains `plps_seen()` and keeps its 256-slot extractor table on the heap (66 KB -> under 4 KB of stack).

## Why this version

- **Proposed version:** 11.1.0 -> **12.0.0** (major-class: semver major).
- **Own API/behaviour change class:** major (T2miPump::new / InnerTsRecovery::{new,new_for_plp} take Pid).
- **cargo-semver-checks vs live 11.1.0 (all features):** none flagged. (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** none needed (own breaking change). Siblings it depends on that move epoch in this wave: broadcast-common, dvb-bbframe, mpeg-ts.
- **Caret epoch:** 11 -> 12; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
-dvb-bbframe = "11.1"
+dvb-bbframe = "12.0"
-mpeg-ts = "0.5"
+mpeg-ts = "0.6"
```

## Breaking changes and migration

```rust
// before
let pump = T2miPump::new(0x1000);
// after
let pump = T2miPump::new(Pid::try_new(0x1000)?);
```

New optional `mpeg-ts` dependency behind the `ts` feature. `dvb-bbframe`'s `Bbheader::serialize()` now returns `Result` (see that note); in-tree callers are migrated. A multi-PLP capture fed to `InnerTsRecovery::new` interleaves independent transport streams: check `plps_seen()` and use `new_for_plp`.

## What changed (from CHANGELOG `[Unreleased]`)

#### Added
- `InnerTsRecovery::plps_seen()` — every `plp_id` a BBFrame was seen for (#1034); with more than one the unfiltered output interleaves independent TSes, so select one with `new_for_plp`. Docs on the type say so. Per-PLP carry-over (the other half of #1034) was already in place.

#### Changed
- `T2miPump` decodes the TS header/payload through `mpeg_ts::ts::TsPacket` instead of a second hand-rolled bit-mask decode (r11-W-T2-2, #1095); behaviour (TEI drop, duplicate skip) unchanged.
- `InnerTsRecovery` keeps its 256-slot per-PLP extractor table on the heap: the struct was 66 KB, now under 4 KB (the same stack hazard `BbframePump` boxed away). Pinned by a size test.
- `dvb-bbframe`'s `Bbheader::serialize()` now returns `Result` (BREAKING there); this crate's in-tree callers are migrated.

#### Changed (BREAKING)

- **BREAKING (#1159):** `T2miPump::new` and `InnerTsRecovery::{new,new_for_plp}` take `mpeg_ts::pid::Pid` instead of `u16` (new optional `mpeg-ts` dependency behind the `ts` feature). A PID above `0x1FFF` can no longer be passed (use `Pid::try_new`).

## Merged changes since `v11.1.0`

- O1: ordering hygiene — registry tables in wire-value order + ordering guard (#1238)
- test: shared test-fixtures loader crate, migrate hand-written loaders, drop CRC copies (#1214) (#1215)
- Cleanups: remove panicking Serialize::to_bytes (BREAKING, #1143) + serde default-features off in 32 no_std manifests with CI thumbv7em serde matrix (#1183) (#1185)
- Systemic umbrellas T1/T2/T14: serializer width guards (28 crates), hostile-count DoS fix, secrets hygiene, origin-scoped credentials, secure ingest default (BREAKING items marked, NO release) (#1182)
- Audit critical-bug verification pass: 72 issues re-verified against main; fixes for #1012 #1022 #1034 #1035 #1056 + regression tests (NO release) (#1180)
- Audit per-crate rollups: verified against main, 54 commits of real fixes across ~40 crates (BREAKING items marked, NO release) (#1179)
- Typed ids wave 2: TrackId/ProgramNumber + HLS naming newtypes, Pid inside dvb-si (BREAKING, NO release) (#1178)
- Typed ids wave 1: Pid newtype (mpeg-ts and all non-dvb-si APIs) + reserved-bit preservation for all dvb-si descriptors and tables (BREAKING, NO release) (#1177)
