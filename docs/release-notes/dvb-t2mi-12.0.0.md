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

#### Changed (BREAKING)
- **BREAKING:** individual-addressing function tags 0x00-0x06 (TS 101 191 §6.1 Tables 7-13, which TS 102 773 §5.2.8.1 defers to) are now typed instead of `FunctionBody::Raw`: new variants `FunctionBody::{TimeOffset, FrequencyOffset, Power, PrivateData, CellId, Enable, Bandwidth}` over `TimeOffsetBody`, `FrequencyOffsetBody` (24-bit signed), `PowerBody`, `PrivateDataBody`, `CellIdBody`, `EnableBody` and `BandwidthBody` (all `#[non_exhaustive]` with `new()`). Callers that matched `Raw` for these tags must match the typed variants; a fixed-size tag whose length does not fit still falls back to `Raw`. Every table was verified on the rendered PDF page; see `docs/tables/ts_101_191/`.
- **BREAKING (fix):** `function_length` is the length of the whole `function()` including its 2-byte tag+length header (TS 102 773 §5.2.8.2.0), not just the body. It was parsed/serialized as the body length, which made the real Colombia capture's 0x21 packets fail to parse and mis-framed every typed T2 function (0x10-0x17) on the wire. A `function_length` below 2 is rejected. Pinned by a byte-exact round trip over the capture's 11 real individual-addressing packets.
- Spec note recorded, not silently corrected: TS 101 191 prose calls `ch_bandwidth` an "8-bit field" but Table 13 gives it 7 bits; the table width is implemented.

#### Changed (BREAKING, round-2 audit)
- **BREAKING:** `Error` gains `InvalidFunctionLength { function_length }` and `FunctionTagBodyMismatch { tag, body, reason }` (the enum is `#[non_exhaustive]`, so only exhaustive matches without a wildcard are affected). A parsed `function_length` below 2 is now `InvalidFunctionLength` instead of the misleading `ReservedBitsViolation`.
- **BREAKING:** serializing an individual-addressing `FunctionEntry` whose `tag` disagrees with its `body` now fails with `FunctionTagBodyMismatch` instead of writing bytes that re-parse differently: a typed `FunctionBody` variant under another tag, or a `FunctionBody::Raw` under a tag/length that parses as a typed body (any length for private data 0x03 / enable 0x05; the exact size for the fixed-size tags). Legitimate `Raw` (unknown tag, or a known fixed-size tag with a non-matching length) is unaffected. No in-tree construction (dvb-tools, tests) needed migrating. serialize -> parse is now the identity for every serializable entry.
- Doc fix: `FunctionEntry` documented the body as `function_length` bytes; it is `function_length - 2` (the length includes the tag+length header).

#### Documentation
- README rewritten to the workspace crate-README standard: what the crate does and does not do, a standards table (edition, scope, status), feature flags, a doctested quick start, the implemented API surface, design notes and related crates. Where the crate has a library target the crate-root docs are now `include_str!("../README.md")`, so docs.rs and crates.io show the same text.

#### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10, `dvb-bbframe` 11 -> 12, `mpeg-ts` 0.5 -> 0.6). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

#### Added
- `InnerTsRecovery::plps_seen()` — every `plp_id` a BBFrame was seen for (#1034); with more than one the unfiltered output interleaves independent TSes, so select one with `new_for_plp`. Docs on the type say so. Per-PLP carry-over (the other half of #1034) was already in place.

#### Changed
- `T2miPump` decodes the TS header/payload through `mpeg_ts::ts::TsPacket` instead of a second hand-rolled bit-mask decode (r11-W-T2-2, #1095); behaviour (TEI drop, duplicate skip) unchanged.
- `InnerTsRecovery` keeps its 256-slot per-PLP extractor table on the heap: the struct was 66 KB, now under 4 KB (the same stack hazard `BbframePump` boxed away). Pinned by a size test.
- `dvb-bbframe`'s `Bbheader::serialize()` now returns `Result` (BREAKING there); this crate's in-tree callers are migrated.

#### Changed (BREAKING)

- **BREAKING (#1159):** `T2miPump::new` and `InnerTsRecovery::{new,new_for_plp}` take `mpeg_ts::pid::Pid` instead of `u16` (new optional `mpeg-ts` dependency behind the `ts` feature). A PID above `0x1FFF` can no longer be passed (use `Pid::try_new`).

## Merged changes since `v11.1.0`

- fix: round-2 audit (transmux demux ids, MPEG-H descriptor, st291 fixtures, E-AC-3 probe, dvb-t2mi function entries) (#1322)
- docs: README coverage gaps (audit 24), media-doctor examples doc (#1317)
- feat(dvb-t2mi): type individual-addressing functions 0x00-0x06 + fix function_length framing (#1312)
- docs: correct false 'not vendored' / 'edition unverified' claims (#1311)
- docs: cluster A crate READMEs to the crate-README standard (what/not, standards table, features, doctested quick start, API, design notes) (#1289)
- Fix the confirmed signalling/captions review findings (supersedes #1279) (#1282)
- O1: ordering hygiene — registry tables in wire-value order + ordering guard (#1238)
- test: shared test-fixtures loader crate, migrate hand-written loaders, drop CRC copies (#1214) (#1215)
- Cleanups: remove panicking Serialize::to_bytes (BREAKING, #1143) + serde default-features off in 32 no_std manifests with CI thumbv7em serde matrix (#1183) (#1185)
- Systemic umbrellas T1/T2/T14: serializer width guards (28 crates), hostile-count DoS fix, secrets hygiene, origin-scoped credentials, secure ingest default (BREAKING items marked, NO release) (#1182)
- Audit critical-bug verification pass: 72 issues re-verified against main; fixes for #1012 #1022 #1034 #1035 #1056 + regression tests (NO release) (#1180)
- Audit per-crate rollups: verified against main, 54 commits of real fixes across ~40 crates (BREAKING items marked, NO release) (#1179)
- Typed ids wave 2: TrackId/ProgramNumber + HLS naming newtypes, Pid inside dvb-si (BREAKING, NO release) (#1178)
- Typed ids wave 1: Pid newtype (mpeg-ts and all non-dvb-si APIs) + reserved-bit preservation for all dvb-si descriptors and tables (BREAKING, NO release) (#1177)
