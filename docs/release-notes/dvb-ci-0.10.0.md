---
status: DRAFT
---
# dvb-ci 0.10.0

_Released 2026-10-16._

**Breaking release.** Every length_field coding is preserved (a CAM may code `82 00 09` for 9, captured raw on hardware; parse used to accept it but every object re-serialized the minimal form), and every PID field is `mpeg_ts::pid::Pid`.

## Why this version

- **Proposed version:** 0.9.1 -> **0.10.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** major (length_form fields on every APDU/SPDU/TPDU object, typed Pid (semver-checks major)).
- **cargo-semver-checks vs live 0.9.1 (all features):** MAJOR (3 major, 0 minor lint failures). (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** none needed (own breaking change). Siblings it depends on that move epoch in this wave: broadcast-common, dvb-si, mpeg-ts.
- **Caret epoch:** 0.9 -> 0.10; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
-dvb-si = "11.1"
+dvb-si = "12.0"
-mpeg-ts = "0.5"
+mpeg-ts = "0.6"
```

## Breaking changes and migration

### 1. `length_form` on every object (#1091)

New typed `length::LengthForm` (`Minimal` default; `Long { octets }`), captured by `parse` and re-emitted by `serialize`, so parse -> serialize is byte-exact. Every APDU object, SPDU (`OpenSessionRequest`, `SessionNumber`, ...) and TPDU object (`TcObject`, `NewTc`, `TcError`, `CommandTpdu`, `ResponseTpdu` + `status_length_form`) gains `length_form: LengthForm`; header-only objects (`ApplicationInfoEnq`, `EnterMenu`, `ProfileEnq`, ...) are no longer unit structs.

```rust
// before
let msg = ApplicationInfoEnq;
// after
let msg = ApplicationInfoEnq::default();     // { length_form: LengthForm::Minimal }
```

`SessionNumber::header_len()` replaces the assumption that an APDU body starts at the minimal `HEADER_LEN`. `length::decode_form`, `encode_with` and `encoded_len_with` are the codec entry points; `decode` accepts up to 4 length octets (value still capped at 0xFFFF).

### 2. Typed PIDs (#1159)

`CaPmtStream::elementary_pid`, `CaPmtReplyStream::elementary_pid`, `Replace::{replaced_pid,replacement_pid}`, the CI+ `MsCaPmt*`/`PidSelect*`/`SampleTrack::track_pid`/`CommsInfoReply::input_delivery_pid` fields and `multistream::NULL_PID` are `mpeg_ts::pid::Pid`. New dependency: `mpeg-ts`.

## What changed (from CHANGELOG `[Unreleased]`)

#### Changed
- Internal: adapted to `broadcast-common`'s removal of the panicking `Serialize::to_bytes` (tests, examples and docs now use `try_to_bytes`); requires the new `broadcast-common` major (umbrella #1143).

#### Changed (BREAKING)

- **BREAKING (r10-W-4, #1091): `length_field` coding is preserved.** A CAM may code a length in a longer form than necessary (`82 00 09` for the value 9, captured raw on hardware, dvb-ci-runtime #337); parse accepted these but every object re-serialized the minimal form, changing real traffic bytes. New typed `length::LengthForm` (`Minimal` = default, `Long { octets }` for a non-minimal long form), captured by `parse` and re-emitted by `serialize`, so parse -> serialize is byte-exact. Every APDU object (the EN 50221, CI-extension and CI Plus structs, `AnyApdu::Unknown`), every SPDU (`OpenSessionRequest`, `OpenSessionResponse`, `CreateSession`, `CreateSessionResponse`, `CloseSessionRequest`, `CloseSessionResponse`, `SessionNumber`) and every TPDU object (`TcObject`, `NewTc`, `TcError`, `CommandTpdu`, `ResponseTpdu` plus its `status_length_form`) gains a `length_form: LengthForm` field (not serialized by serde); the header-only objects (`ApplicationInfoEnq`, `EnterMenu`, `ProfileEnq`, ...) are no longer unit structs (use `Default::default()` / `new`-style `{ length_form: Default::default() }`). `AnyApdu::Unknown` gains `length_form`. `length::decode_form`, `encode_with` and `encoded_len_with` are the codec entry points; `decode` accepts up to 4 length octets (value still capped at 0xFFFF); the indefinite form, more than 4 octets, a value above 0xFFFF and a truncated field are still rejected. New `SessionNumber::header_len()`: the APDU body of a `session_number` SPDU starts at that offset (`HEADER_LEN` is only the minimal-form length). Pinned per form (`81 NN`, `82 00 NN`, `83 00 00 NN`, `84 00 00 00 NN`) over every APDU, SPDU and TPDU, and the #337 capture round-trips byte-exactly. Not verified against real CAM hardware beyond that capture.

- **BREAKING (#1159, typed PIDs):** every 13-bit PID field is `mpeg_ts::pid::Pid` instead of `u16`: `CaPmtStream::elementary_pid`, `CaPmtReplyStream::elementary_pid`, `Replace::{replaced_pid,replacement_pid}`, the CI+ `MsCaPmt*`/`PidSelect*`/`SampleTrack::track_pid`/`CommsInfoReply::input_delivery_pid` fields and `multistream::NULL_PID`. The serialize-time "PID exceeds 13-bit range" checks are gone (unrepresentable); the former `oversized_*_rejected` tests now assert `Pid::try_new` refuses the value. New dependency: `mpeg-ts`.

## Merged changes since `dvb-ci-v0.9.1`

- Cleanups: remove panicking Serialize::to_bytes (BREAKING, #1143) + serde default-features off in 32 no_std manifests with CI thumbv7em serde matrix (#1183) (#1185)
- Systemic umbrellas T1/T2/T14: serializer width guards (28 crates), hostile-count DoS fix, secrets hygiene, origin-scoped credentials, secure ingest default (BREAKING items marked, NO release) (#1182)
- Audit per-crate rollups: verified against main, 54 commits of real fixes across ~40 crates (BREAKING items marked, NO release) (#1179)
- Typed ids wave 2: TrackId/ProgramNumber + HLS naming newtypes, Pid inside dvb-si (BREAKING, NO release) (#1178)
- Typed ids wave 1: Pid newtype (mpeg-ts and all non-dvb-si APIs) + reserved-bit preservation for all dvb-si descriptors and tables (BREAKING, NO release) (#1177)
