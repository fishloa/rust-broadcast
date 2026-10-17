---
status: DRAFT
---
# dvb-ci 0.10.0

_Released 2026-10-16._

**Breaking release (its own break, plus the `broadcast-common` 10 / `dvb-si` 12 / `mpeg-ts` 0.6 epoch).** Three source-breaking changes: (1) `length_field` coding is preserved, so every APDU, SPDU and TPDU object gains a `length_form: LengthForm` field (a CAM may code `82 00 09` for 9, captured raw on hardware; parse used to accept it but every object re-serialized the minimal form); (2) every 13-bit PID field is now `mpeg_ts::pid::Pid` instead of `u16`; (3) parse -> serialize is now byte-exact for every object or the parse is an error: fixed and count-delimited layouts reject trailing bytes, and the reserved/unused bits a peer sent are kept in new public `reserved` fields and written back instead of being rewritten to the spec fill. The README and crate-root docs were rewritten to the workspace standard.

## Why this version

- **Proposed version: 0.9.1 -> 0.10.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** major. New public fields on most objects, changed field types, and stricter parsing.
- **cargo-semver-checks vs live 0.9.1 (all features):** MAJOR (3 major, 0 minor lint failures), consistent with the CHANGELOG BREAKING list.
- **Epoch cascade:** also from `broadcast-common` (9 -> 10), `dvb-si` (11 -> 12) and `mpeg-ts` (0.5 -> 0.6).
- **Caret epoch:** 0.9 -> 0.10; every workspace dependent (`dvb-ci-runtime`, `multimux` and others) must move with it.

## Dependency changes

```toml
-broadcast-common = { path = "../broadcast-common", version = "9.4",  default-features = false }
+broadcast-common = { path = "../broadcast-common", version = "10.0", default-features = false }
-dvb-si           = { path = "../dvb-si",           version = "11.1", default-features = false }
+dvb-si           = { path = "../dvb-si",           version = "12.0", default-features = false }
+mpeg-ts          = { path = "../mpeg-ts",          version = "0.6",  default-features = false }   # new dependency (typed Pid)
```

`mpeg-ts` is new in this release. The `std` feature now forwards `mpeg-ts/std` and `serde?/std`, and the `serde` feature forwards `mpeg-ts/serde`. The optional `serde` dependency is now `default-features = false` with `features = ["derive", "alloc"]`.

## Breaking changes and migration

### 1. `length_form` on every object (#1091)

A new typed `length::LengthForm` (`Minimal` is the default; `Long { octets }` is a non-minimal long form) is captured by `parse` and re-emitted by `serialize`, so parse -> serialize is byte-exact. Every APDU object (the EN 50221, CI-extension and CI Plus structs, and `AnyApdu::Unknown`), every SPDU (`OpenSessionRequest`, `OpenSessionResponse`, `CreateSession`, `CreateSessionResponse`, `CloseSessionRequest`, `CloseSessionResponse`, `SessionNumber`) and every TPDU object (`TcObject`, `NewTc`, `TcError`, `CommandTpdu`, `ResponseTpdu` plus its `status_length_form`) gains a `length_form: LengthForm` field. It is not serialized by serde. Header-only objects (`ApplicationInfoEnq`, `EnterMenu`, `ProfileEnq`, ...) are no longer unit structs.

```rust
use dvb_ci::objects::application_info::ApplicationInfoEnq;

// before: a unit struct
// let msg = ApplicationInfoEnq;
// after: build it with Default (length_form: LengthForm::Minimal)
let msg = ApplicationInfoEnq::default();
```

For objects with other fields, add `length_form: Default::default()` to struct literals (or use `..Default::default()` where the type implements `Default`). `SessionNumber::header_len()` replaces the assumption that an APDU body starts at the minimal `HEADER_LEN`. `length::decode_form`, `encode_with` and `encoded_len_with` are the codec entry points; `decode` accepts up to 4 length octets (the value is still capped at 0xFFFF), and the indefinite form, more than 4 octets, a value above 0xFFFF and a truncated field are still rejected.

`length::encode_with` (and so every `try_to_bytes`) now rejects a `LengthForm::Long` whose octet count is the minimal one for the value (`81 80` for 128, `82 01 00` for 256): those bytes equal `LengthForm::Minimal`, so `parse` can only return `Minimal`. Parsed objects never carry such a form; non-minimal real-data forms (`82 00 09`, `83 00 00 09`, ...) still round-trip.

### 2. Typed PIDs (#1159)

`CaPmtStream::elementary_pid`, `CaPmtReplyStream::elementary_pid`, `Replace::{replaced_pid,replacement_pid}`, the CI+ `MsCaPmt*` / `PidSelect*` / `SampleTrack::track_pid` / `CommsInfoReply::input_delivery_pid` fields and `multistream::NULL_PID` are `mpeg_ts::pid::Pid`. The serialize-time "PID exceeds 13-bit range" checks are gone because an out-of-range PID is now unrepresentable (`Pid::try_new` refuses it).

```rust
use mpeg_ts::pid::Pid;

// before: elementary_pid: 0x0100
// after
let elementary_pid = Pid::new(0x0100);   // masks to 13 bits; Pid::try_new rejects out-of-range
let raw: u16 = elementary_pid.value();
```

### 3. Byte-exact parse -> serialize, or a parse error

- Fixed-layout and count-delimited objects reject a longer `length_field` with `Error::InvalidObject` ("trailing bytes after the fixed body"): `menu_answ`, `keypress`, `keypad_control` (commands without a `key_code` list), `display_message`, `scene_end_mark`, `scene_done_message`, `scene_control`, `display_reply`, `clear_replace`, `application_info`, `open_session_request` (`length_field` = 4), and the CI-extension / CI Plus objects (`RequestStart`, `cp_query`, `ServiceListChanged`, `ServiceListVersionAck`, `ServiceListAck`, `ServiceDescReq/Ack`, `GetServiceReq`, every `StatusItem` object, `EITSectionAck`, `FileSystemOffer`, `CICAM_player_*` requests, `comms_IP_config_reply`, `PID_select_req/reply`, `sd_info_reply`, `sd_start`, `sd_update`, `tune_ip_req`, `tuner_status_reply`). A peer that appended bytes to those objects now produces a parse error instead of having them silently dropped.
- Reserved and unused bits a peer sent are kept in new public fields of type `reserved::Reserved<BITS, ONES>` (aliases `ReservedOnes` / `ReservedZero`; default is the spec fill; never part of serde output) and written back. They appear on `CaPmt` (`reserved_version`, `reserved_program_info`), `CaPmtStream` (`reserved_pid`, `reserved_es_info`), `CaPmtReply*`, `MsCaPmt*`, `Replace`, `Enq`, `SceneEndMark`, `SceneDoneMessage`, `SceneControl`, `PixelDepth`, `FileAck`, `ActivationStateChangeRequest`, `ModuleIdSend`, `ModuleIdCommand`, `ServiceDescAck`, `GetServiceAck`, `EitSectionReq/Ack`, `CcPinReply`, `CommsInfoReply`, `CommsIpConfigReply`, `PlayerStartReq`, `PlayerStatusError`, `PlayerAssetEnd`, `ComponentType`, `PidSelectRequest/Entry/Reply`, `SampleTrack`, `SdStart`, `SdUpdate`, `TuneTripletReq`, `TuneLcnReq`, `TuneIpReq` and `TunerStatusDsd`. A hand-built struct literal must now name them; `Default::default()` is the spec value:

```rust
// before
let s = CaPmtStream { stream_type, elementary_pid, cmd_id, ca_descriptors };
// after
let s = CaPmtStream {
    stream_type,
    elementary_pid: Pid::new(pid),
    cmd_id,
    ca_descriptors,
    reserved_pid: Default::default(),      // 0b111
    reserved_es_info: Default::default(),  // 0b1111
};
```

## What changed (from CHANGELOG `[Unreleased]`)

### Documentation
- README and crate-root docs rewritten to the cluster standard: what it does / does not, standards table (EN 50221, TS 101 699 V1.1.1, TS 103 205 V1.4.1), feature table, compiling quick start.

### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10, `dvb-si` 11 -> 12, `mpeg-ts` 0.5 -> 0.6). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

### Changed (BREAKING)

- **BREAKING (adversarial review `review/ci-cam-crypto`): parse -> serialize is
  byte-exact for every object, or the parse is an error.** The r10-O-1 "reject
  trailing bytes" pass had missed fixed-layout APDUs and every variable-layout
  object silently dropped whatever followed its last counted field, so the
  byte-exact claim above was not true for them. Now (a) every fixed or
  count-delimited layout rejects a longer `length_field`
  (`Error::InvalidObject`, "trailing bytes after the fixed body"): `menu_answ`,
  `keypress`, `keypad_control` (commands without a `key_code` list),
  `display_message`, `scene_end_mark`, `scene_done_message`, `scene_control`,
  `display_reply`, `clear_replace`, `application_info` (EN 50221 Table 21 and TS
  101 699 §5 define nothing after `menu_string`, so there is no later-version
  extension to preserve), `open_session_request` (`length_field` = 4), the CI
  extension / CI+ objects (`RequestStart`, `cp_query`, `ServiceListChanged`,
  `ServiceListVersionAck`, `ServiceListAck`, `ServiceDescReq/Ack`, `GetServiceReq`,
  every `StatusItem` object, `EITSectionAck`, `FileSystemOffer`, `CICAM_player_*`
  requests, `comms_IP_config_reply`, `PID_select_req/reply`, `sd_info_reply`,
  `sd_start`, `sd_update`, `tune_ip_req`, `tuner_status_reply`); (b) reserved and
  unused bits a peer sent are kept in typed fields and written back instead of
  being cleared or rewritten to `1`: new public `reserved` fields (type
  `reserved::Reserved<BITS, ONES>`, aliases `ReservedOnes` / `ReservedZero`,
  default = the spec fill, never part of serde output) on `CaPmt`
  (`reserved_version`, `reserved_program_info`), `CaPmtStream`, `CaPmtReply*`,
  `MsCaPmt*`, `Replace`, `Enq`, `SceneEndMark`, `SceneDoneMessage`,
  `SceneControl`, `PixelDepth`, `FileAck`, `ActivationStateChangeRequest`,
  `ModuleIdSend`, `ModuleIdCommand`, `ServiceDescAck`, `GetServiceAck`,
  `EitSectionReq/Ack`, `CcPinReply`, `CommsInfoReply`, `CommsIpConfigReply`,
  `PlayerStartReq`, `PlayerStatusError`, `PlayerAssetEnd`, `ComponentType`,
  `PidSelectRequest/Entry/Reply`, `SampleTrack`, `SdStart`, `SdUpdate`,
  `TuneTripletReq`, `TuneLcnReq`, `TuneIpReq`, `TunerStatusDsd` (a hand-built
  struct literal must now name them: `Default::default()` is the spec value).
  Guard: `roundtrip_property` drives every dispatch enum (`AnyApdu`, the 18
  resource enums, the two hand-written dispatchers in both host-control modes)
  and the SPDUs with 40 000 pseudo-random bodies per tag; any accepted input
  that does not re-serialize byte-for-byte fails the build.
- **BREAKING:** `length::encode_with` (and so every `try_to_bytes`) rejects a
  `LengthForm::Long` whose octet count is the *minimal* one for the value (`81 80`
  for 128, `82 01 00` for 256): the bytes equal `LengthForm::Minimal`, so `parse`
  can only return `Minimal` and `serialize -> parse` changed the value. Parsed
  objects never carry such a form; non-minimal real-data forms (`82 00 09`,
  `83 00 00 09`, ...) still round-trip.

- **BREAKING (r10-W-4, #1091): `length_field` coding is preserved.** A CAM may code a length in a longer form than necessary (`82 00 09` for the value 9, captured raw on hardware, dvb-ci-runtime #337); parse accepted these but every object re-serialized the minimal form, changing real traffic bytes. New typed `length::LengthForm` (`Minimal` = default, `Long { octets }` for a non-minimal long form), captured by `parse` and re-emitted by `serialize`, so parse -> serialize is byte-exact. Every APDU object (the EN 50221, CI-extension and CI Plus structs, `AnyApdu::Unknown`), every SPDU (`OpenSessionRequest`, `OpenSessionResponse`, `CreateSession`, `CreateSessionResponse`, `CloseSessionRequest`, `CloseSessionResponse`, `SessionNumber`) and every TPDU object (`TcObject`, `NewTc`, `TcError`, `CommandTpdu`, `ResponseTpdu` plus its `status_length_form`) gains a `length_form: LengthForm` field (not serialized by serde); the header-only objects (`ApplicationInfoEnq`, `EnterMenu`, `ProfileEnq`, ...) are no longer unit structs (use `Default::default()` / `new`-style `{ length_form: Default::default() }`). `AnyApdu::Unknown` gains `length_form`. `length::decode_form`, `encode_with` and `encoded_len_with` are the codec entry points; `decode` accepts up to 4 length octets (value still capped at 0xFFFF); the indefinite form, more than 4 octets, a value above 0xFFFF and a truncated field are still rejected. New `SessionNumber::header_len()`: the APDU body of a `session_number` SPDU starts at that offset (`HEADER_LEN` is only the minimal-form length). Pinned per form (`81 NN`, `82 00 NN`, `83 00 00 NN`, `84 00 00 00 NN`) over every APDU, SPDU and TPDU, and the #337 capture round-trips byte-exactly. Not verified against real CAM hardware beyond that capture.

- **BREAKING (#1159, typed PIDs):** every 13-bit PID field is `mpeg_ts::pid::Pid` instead of `u16`: `CaPmtStream::elementary_pid`, `CaPmtReplyStream::elementary_pid`, `Replace::{replaced_pid,replacement_pid}`, the CI+ `MsCaPmt*`/`PidSelect*`/`SampleTrack::track_pid`/`CommsInfoReply::input_delivery_pid` fields and `multistream::NULL_PID`. The serialize-time "PID exceeds 13-bit range" checks are gone (unrepresentable); the former `oversized_*_rejected` tests now assert `Pid::try_new` refuses the value. New dependency: `mpeg-ts`.
