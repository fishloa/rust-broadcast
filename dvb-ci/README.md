# dvb-ci

[![crates.io](https://img.shields.io/crates/v/dvb-ci.svg)](https://crates.io/crates/dvb-ci)
[![docs.rs](https://img.shields.io/docsrs/dvb-ci)](https://docs.rs/dvb-ci)
[![MSRV](https://img.shields.io/badge/MSRV-1.95.0-blue.svg)](https://blog.rust-lang.org/)
[![license](https://img.shields.io/crates/l/dvb-ci.svg)](#license)

**DVB Common Interface (EN 50221)** — the host ↔ CICAM wire protocol, as typed Rust.
Parses and builds the EN 50221 protocol objects across all three layers
(transport, session, application), the DVB CI extensions (ETSI TS 101 699) and
the CI Plus extension APDUs (ETSI TS 103 205), plus a **`CA_PMT` builder** that
turns a [`dvb-si`](https://crates.io/crates/dvb-si) PMT into the object handed to
a Conditional Access Module.

```text
dvb-si PmtSection --> dvb_ci::builder::build_ca_pmt --> ca_pmt object (9F 80 32) --> CICAM
```

## What it does / what it does not

**Does.** Turns bytes into typed objects and back, losslessly: every TPDU, SPDU
and APDU (including the CI-extension and CI Plus resource objects) parses into a
struct with every spec field, and serializes back to identical bytes. It builds
`ca_pmt` objects from a parsed PMT and filters them to the CAM's advertised
`CA_system_id`s. It is sans-IO: it never touches a device.

**Does not.**

- It does not talk to hardware. The PC-Card / CA-device transport, the TPDU
  poll loop and the resource state machines live in
  [`dvb-ci-runtime`](https://crates.io/crates/dvb-ci-runtime).
- It does not implement CI Plus cryptography (the Content Control resource's
  authentication, key derivation and SAC encryption). The CI Plus APDU *framing*
  is typed; the cryptography is not, and the Content Control SAC payloads are
  carried as typed datatypes, never decrypted.
- It does not implement CI+ over USB (ETSI TS 103 605) or the PCMCIA electrical
  layer.
- It is not a descrambler: it never sees a control word being applied (see
  [`dvb-csa`](https://crates.io/crates/dvb-csa)).

## Standards supported

| Standard | Edition | Scope | Status | Notes |
|---|---|---|---|---|
| CENELEC EN 50221 — Common Interface Specification for Conditional Access and other DVB Decoder Applications | February 1997 (the vendored text; verified against the spec header) | Transport (`T_C_*`, C_TPDU/R_TPDU, status byte), session (SPDU), application layer: Resource Manager, Application Information, CA Support (`ca_info`, `ca_pmt`, `ca_pmt_reply`), Host Control, Date-Time, MMI (low-level, display/keypad/subtitle/download, high-level menu/list/enq/answ), Low-Speed Communications; all Table 58 `apdu_tag`s | Full | Wire/protocol layer only; PC-Card physical layer out of scope |
| ETSI TS 101 699 — Extensions to the Common Interface Specification | V1.1.1 (1999-11) | Resource-scoped objects: Resource Manager v2, Application Information v2, Power Manager, Event Manager, Copy Protection, StreamInput, ServiceGateway, BroadcastServiceGateway, Status Query, Application MMI, Download (incl. DSM-CC U-N messages), CA Pipeline | Full (printed APDU syntax) | Dispatched by resource identifier (`ci_ext::CiExtApdu`) because the tags collide with EN 50221 |
| ETSI TS 103 205 — Extensions to the CI Plus Specification | V1.4.1 (2019-05) | Multi-stream, Content Control APDUs + SAC datatype model, Multi-stream Host Control (tune), Sample Decryption, CICAM Player (16 APDUs), Auxiliary File System, extended CA Support `ca_pmt`, LSC v4, URI v3, Sample-Mode descriptors | Partial | `FileRequest`/`FileAcknowledge` bodies and `tune_broadcast_req`/`tune_reply`/`ask_release(_reply)` defer to CI Plus V1.3 (not in TS 103 205) and are not encoded; DRM metadata/UUID payloads are opaque; no cryptography |

## Features

| Feature | Default | Effect |
|---|---|---|
| `std` | yes | Links `std` (and the siblings' `std`). Off gives `#![no_std]` + `alloc`. |
| `serde` | no | `serde::Serialize` on the public types (not the preserved `length_form` / reserved bits). |

## Quick start

```rust
use broadcast_common::Serialize;
use dvb_ci::AnyApdu;
use dvb_ci::length::LengthForm;
use dvb_ci::objects::ca_info::CaInfo;

// A ca_info() APDU advertising two CA_system_ids.
let info = CaInfo {
    length_form: LengthForm::Minimal,
    ca_system_ids: vec![0x0500, 0x0B00],
};
let wire = info.try_to_bytes().unwrap(); // 9F 80 31 04 05 00 0B 00
assert_eq!(wire, [0x9F, 0x80, 0x31, 0x04, 0x05, 0x00, 0x0B, 0x00]);

// Route any APDU by its 3-byte tag without knowing the type up front.
let any = AnyApdu::parse(&wire).unwrap();
assert_eq!(any.name(), "CA_INFO");
assert_eq!(any.try_to_bytes().unwrap(), wire); // round-trips byte-for-byte
```

Two runnable examples ship with the crate (`cargo run -p dvb-ci --example <name>`):
`build_ca_pmt` (PMT to `ca_pmt`, stripping non-CA descriptors) and `parse_apdu`
(dispatch and round-trip, including an unknown tag).

## What is implemented

| Layer | Objects |
|---|---|
| APDU framework | `ApduTag` (3 bytes, Table 58 constants); ASN.1 `length_field` codec with the coding form preserved (`length::LengthForm`); `AnyApdu` tag dispatch (`declare_apdus!` + drift test, 40 typed `apdu_tag`s plus an `Unknown` lossless fallthrough); 4-octet `ResourceId` |
| Resource Manager | `profile_enq` / `profile` / `profile_change` |
| Application Information | `application_info_enq` / `application_info` / `enter_menu` |
| CA Support | `ca_info_enq` / `ca_info`, `ca_pmt`, `ca_pmt_reply` |
| Date-Time | `date_time_enq` / `date_time` (optional `local_offset`) |
| Host Control | `objects::host_control`: `tune`, `replace`, `clear_replace`, `ask_release` |
| MMI | `close_mmi`; low-level display/keypad/subtitle/download objects (`objects::mmi_display`); high-level `text`, `enq`, `answ`, `menu`, `menu_answ`, `list` (`objects::mmi_high`) |
| Low-Speed Communications | `objects::low_speed_comms` (`comms_cmd`, `connection_descriptor`, `comms_reply`, `comms_send`, `comms_rcv`) |
| Session (SPDU) | open / create / close session request + response, `session_number`, status values |
| Transport (TPDU) | C_TPDU / R_TPDU (+ status byte), Create / Delete / Request / New_T_C, T_C_Error, SB_value |
| CA_PMT builder | `build_ca_pmt(&PmtSection, list_management, cmd_id)` keeps only `CA_descriptor`s at programme and ES level; `build_ca_pmt_for_caids(..., allowed)` / `CaPmtBuilt::retain_caids(&[u16])` filter to the CAM's CAIDs |
| TS 101 699 (`ci_ext`) | `CiExtApdu`, keyed on `resource_identifier()`; one module per resource (see the standards table) |
| TS 103 205 (`ci_plus`) | `CiPlusApdu`, keyed on `resource_identifier()`: Multi-stream, Content Control, Multi-stream Host Control, Sample Decryption, CICAM Player, Auxiliary File System; extended CA Support `ca_pmt`, LSC v4, URI v3 and Sample-Mode descriptors are standalone typed structs outside `CiPlusApdu` dispatch because TS 103 205 prints no resource_id for them |

## Design notes

- **Symmetric Parse/Serialize.** Every wire type implements
  `broadcast_common::Parse` / `Serialize`; parse to serialize is byte-identical or
  the parse is an error (a longer `length_field` than a fixed layout allows is
  rejected). A property test drives every dispatch enum with pseudo-random bodies.
- **Preserved framing.** A CAM may code a length in a longer form than necessary
  (`82 00 09` for 9, captured on real hardware); `LengthForm` is kept on every
  object and re-emitted. Reserved bits a peer sent are kept in typed `reserved`
  fields (`reserved::Reserved`), defaulting to the spec fill.
- **Typed PIDs.** Every 13-bit PID field is `mpeg_ts::pid::Pid`.
- **Resource-scoped dispatch.** The extension resources reuse `0x9F80xx` tags, so
  they cannot join the global `AnyApdu`; dispatch is by resource id first.
- **`#![no_std]` + `alloc`**, structured `thiserror` errors, `#[non_exhaustive]`
  on dispatch enums.
- Not verified against real CAM hardware beyond the captured length-coding case;
  hardware work is in `dvb-ci-runtime`.

## Related crates

`dvb-ci` depends on `broadcast-common` (traits), `dvb-si` (PMT / CA descriptors)
and `mpeg-ts` (`Pid`). [`dvb-ci-runtime`](https://crates.io/crates/dvb-ci-runtime)
builds the driver runtime on top of it.

## MSRV and license

MSRV 1.95.0. Licensed under MIT OR Apache-2.0.
