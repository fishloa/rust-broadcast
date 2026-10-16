---
status: DRAFT
---
# mpeg-ts 0.6.0

_Released 2026-10-16._

**Breaking release.** `mpeg-ts` is the first of the typed-id crates: `TsHeader::pid` and every PID-taking API is the validated 13-bit `pid::Pid` instead of a bare `u16`, `AdaptationField`/`AdaptationFieldExtension` now carry every Table 2-6 bit so parse -> serialize is byte-exact, and `From<u16> for Pid` (which silently masked) is removed. Real-capture evidence: `tests/pid_typed.rs` checks every packet of every committed `.ts` fixture (and 7.1 M packets / 294 files from the local archive with `--ignored`) against an independently extracted 13-bit PID.

## Why this version

- **Proposed version:** 0.5.0 -> **0.6.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** major (TsHeader::pid is Pid, From<u16> for Pid removed, AdaptationField fields).
- **cargo-semver-checks vs live 0.5.0 (all features):** MAJOR (3 major, 0 minor lint failures). (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** none needed (own breaking change). Siblings it depends on that move epoch in this wave: broadcast-common, mp4-emsg.
- **Caret epoch:** 0.5 -> 0.6; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
-mp4-emsg = "0.4" # dev-dependency
+mp4-emsg = "0.5" # dev-dependency
```

## Breaking changes and migration

### 1. PIDs are `Pid` (#1159, typed PIDs)

```rust
// before
let pid: u16 = header.pid;
let mut mux = SiMux::new();   mux.upsert(0x0011, section);
// after
let pid: Pid = header.pid;                                // .value() for the raw u16
let mut mux = SiMux::new();   mux.upsert(Pid::try_new(0x0011)?, section);
```

See the dvb-si note for the table constructors. Specific to `mpeg-ts`:

- `TsHeader::pid`, `SectionPacketiser::{new,with_continuity,pid}`, `SiMux::{upsert,upsert_pmt}`, `PusiReassembler::{new,with_max_unit_size,push}` and `OwnedTsPacket::{pid,serialize_with_payload}` take/return `Pid`.
- `impl From<u16> for Pid` is **removed**. Use `Pid::new` (masks; for already-masked wire bits) or the new checked `Pid::try_new` / `TryFrom<u16>`. `Pid` gains `UpperHex`/`LowerHex`, so `{:04X}` formatting of a PID is unchanged; serde output is unchanged (`Pid` is `serde(transparent)`).

```rust
// before
let raw: u16 = header.pid;                    let p: Pid = 0x100u16.into();
// after
let raw: u16 = header.pid.value();            let p = Pid::try_new(0x100)?;
```

### Adaptation field: every bit is typed (umbrella #1139 T11 / audit r01-W2)

```rust
// before
AdaptationField { stuffing_len: 4, .. }          af.stuffing_len
// after: the stuffing bytes are kept verbatim (non-0xFF filler round-trips)
AdaptationField { stuffing: &[0xFF; 4], .. }     af.stuffing.len()
```

`AdaptationFieldExtension<'a>` gains `piecewise_rate_reserved`, `af_descriptor_not_present_flag`, `reserved_flags` and `trailing: &'a [u8]`; it and `SeamlessSplice` are `#[non_exhaustive]` with `new()`/`Default` (reserved bits all ones). `AdaptationField` gains `pcr_reserved`/`opcr_reserved` (real TSDuck `test-002.ts` carries `0` where the writer used to force `1`) and `AdaptationField::new()`/`Default`. Struct literals outside the crate no longer compile: use `new()` and assign fields.

## What changed (from CHANGELOG `[Unreleased]`)

#### Changed
- Internal: the 5-byte marker-bit PTS/DTS/`DTS_next_AU` field is encoded/decoded by `broadcast_common::clock33::{pack_marked, pack_marked_with, unpack_marked}` instead of a local copy (each crate keeps its own prefix / marker validation and error variants); no behaviour change (#1208).

#### Changed
- Internal: the 27 MHz PCR constants / modular delta / `base * 300 + extension` arithmetic now come from `broadcast_common::clock33` (`PCR_HZ`, `PCR_WRAP_27MHZ`, `pcr_forward_distance`, `pcr_from_parts`, `pcr_split`) instead of a local copy; no behaviour change (#1206).

#### Added
- `TsResync::feed_into(&mut self, data, &mut Vec<[u8; 188]>)`: `feed` appending to a caller-owned vector; `feed` now reserves its output once from the buffered byte count (2000-packet feed: 11 -> at most 4 allocations; pinned by `tests/resync_alloc.rs`, audit r01-O4, #1074).

#### Changed (BREAKING)

- **BREAKING (#1159, typed PIDs):** `TsHeader::pid` is `pid::Pid`, not `u16`. `SectionPacketiser::{new,with_continuity,pid}`, `SiMux::{upsert,upsert_pmt}`, `PusiReassembler::{new,with_max_unit_size,push}`, `OwnedTsPacket::{pid,serialize_with_payload}` take/return `Pid`. Migrate with `header.pid.value()` for the raw `u16`, or `Pid::try_new(u16)` at API boundaries that take a user-supplied number.
- **BREAKING:** `impl From<u16> for Pid` is removed (it silently masked an out-of-range value). Use `Pid::new` (masks; for already-masked wire bits) or the new checked `Pid::try_new` / `TryFrom<u16>`.
- Added `Pid::try_new` (rejects `> 0x1FFF`), `TryFrom<u16> for Pid`, the `#[non_exhaustive]` `pid::PidOutOfRange` error, and `UpperHex`/`LowerHex` for `Pid` so `{:04X}`/`{:#06x}` formatting of a PID is unchanged. Serde output is unchanged (`Pid` is `serde(transparent)`).
- Tests: `tests/pid_typed.rs` parses every packet of every committed `.ts` fixture (and, `--ignored`, of the local `.test-streams` archive: 7.1 M packets / 294 files) and checks `header.pid.value()` against the 13-bit PID computed independently from the wire bytes, plus header round-trip.

#### Changed (breaking, umbrella #1139 T11 / audit r01-W2)
- `AdaptationFieldExtension` now carries every bit of ISO/IEC 13818-1 Table 2-6
  so parse -> serialize is byte-exact: new `piecewise_rate_reserved` (2 bits),
  `af_descriptor_not_present_flag`, `reserved_flags` (4 bits) and
  `trailing: &'a [u8]` (the `af_descriptor()` loop or `reserved` bytes after the
  optional fields). Previously these were dropped, so a packet with trailing
  extension bytes re-serialized with a shorter extension and a shifted payload.
  The type is now `AdaptationFieldExtension<'a>`, `#[non_exhaustive]`, with
  `AdaptationFieldExtension::new()` / `Default` (spec-convention reserved bits =
  all `1`); `AdaptationField::extension` is `Option<AdaptationFieldExtension<'a>>`.
  The writer no longer forces the flags-byte reserved bits to `0`.
- `AdaptationField::stuffing_len: usize` is replaced by
  `AdaptationField::stuffing: &'a [u8]` (the stuffing bytes verbatim; use `&[]`
  for none), so non-`0xFF` filler also round-trips. Migration:
  `stuffing_len: n` -> `stuffing: &[0xFF; n]`, `af.stuffing_len` ->
  `af.stuffing.len()`.

- `AdaptationField` gains `pcr_reserved` / `opcr_reserved` (the 6 reserved bits
  of PCR/OPCR; real streams, e.g. TSDuck `test-002.ts`, carry `0` where the
  writer previously forced `1`) and `AdaptationField::new()` / `Default`.
- `SeamlessSplice` is `#[non_exhaustive]`, gains `marker_bits` (real
  `test-147.ts` carries `0` markers) and `SeamlessSplice::new(type, dts)`.

#### Added
- `tests/af_extension_roundtrip.rs`: spec-built Table 2-6 extension vectors plus
  an exhaustive gate asserting every adaptation field of every committed `.ts`
  fixture re-serializes byte-exactly. (No committed capture carries
  `adaptation_field_extension_flag == 1`; a test pins that fact.)

## Merged changes since `mpeg-ts-v0.5.0`

- tests: consume deterministic excerpts of the france-tnt/hotbird captures instead of the 540 MB LFS originals (#1263) (#1264)
- tests: one fixture resolver (public -> private/fixtures -> .test-streams); move local-only captures to the private repo (#1259) (#1260)
- test: shared test-fixtures loader crate, migrate hand-written loaders, drop CRC copies (#1214) (#1215)
- clock33: pack_marked/unpack_marked for the 5-byte marker-bit timestamp; migrate mpeg-pes, mpeg-ts, st291, transmux (#1208) [stacked on #1207] (#1209)
- clock33: shared PCR constants + modular distance; migrate 5 crates; fix ts-fix post-wrap observations (#1206) [stacked on #1196] (#1207)
- Systemic umbrellas T1/T2/T14: serializer width guards (28 crates), hostile-count DoS fix, secrets hygiene, origin-scoped credentials, secure ingest default (BREAKING items marked, NO release) (#1182)
- Audit per-crate rollups: verified against main, 54 commits of real fixes across ~40 crates (BREAKING items marked, NO release) (#1179)
- Typed ids wave 1: Pid newtype (mpeg-ts and all non-dvb-si APIs) + reserved-bit preservation for all dvb-si descriptors and tables (BREAKING, NO release) (#1177)
- Audit systemic wave 2: T16 resync, largesize, ttml drop, WHIP/WHEP DELETE/PATCH/ICE restart, mpeg-ts/pes reserved+stuffing preservation, real-capture guards (NO release) (#1175)
- Audit remediation: unsafe quarantine, panic removal, workspace lints/deps (#1156)
