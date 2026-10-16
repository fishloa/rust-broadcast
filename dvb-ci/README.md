# dvb-ci

[![crates.io](https://img.shields.io/crates/v/dvb-ci.svg)](https://crates.io/crates/dvb-ci)
[![docs.rs](https://img.shields.io/docsrs/dvb-ci)](https://docs.rs/dvb-ci)
[![MSRV](https://img.shields.io/badge/MSRV-1.95.0-blue.svg)](https://blog.rust-lang.org/)
[![license](https://img.shields.io/crates/l/dvb-ci.svg)](#license)

**DVB Common Interface (EN 50221)** — the host ↔ CICAM wire protocol. Parses and
builds the EN 50221 protocol objects across all three layers, plus a **`CA_PMT`
builder** that turns a [`dvb-si`](https://crates.io/crates/dvb-si) PMT into the
object handed to a Conditional Access Module.

Every wire type implements `broadcast_common::Parse` / `broadcast_common::Serialize`
symmetrically (parse → serialize is byte-identical; all length fields computed
from content). `#![no_std]` (+ `alloc`). Spec citations live in each module doc;
the render-verified transcription is in [`docs/en_50221/`](docs/en_50221).

```text
dvb-si PmtSection ──► dvb_ci::builder::build_ca_pmt ──► ca_pmt object (9F 80 32) ──► CICAM
```

## Quickstart

```rust
use dvb_ci::objects::ca_info::CaInfo;
use dvb_ci::AnyApdu;
use broadcast_common::Serialize;

// A ca_info() APDU advertising two CA_system_ids.
let info = CaInfo { ca_system_ids: vec![0x0500, 0x0B00] };
let wire = info.try_to_bytes().unwrap();                // 9F 80 31 04 05 00 0B 00

// Route any APDU by its 3-byte tag without knowing the type up front.
let any = AnyApdu::parse(&wire).unwrap();
assert_eq!(any.name(), "CA_INFO");
assert_eq!(any.try_to_bytes().unwrap(), wire);          // round-trips byte-for-byte
```

## What's implemented

| Layer | Objects |
|---|---|
| **APDU framework** | 3-byte `ApduTag` + Table 58 constants; ASN.1 `length_field` codec with the coding form preserved (`length::LengthForm`, see below); `AnyApdu` tag dispatch (Def-trait + `declare_apdus!` macro + drift test); 4-octet `ResourceId` |
| **CA support** | `ca_info_enq` / `ca_info`, `ca_pmt`, `ca_pmt_reply` |
| **CA_PMT builder** | `build_ca_pmt(PmtSection, list_management, cmd_id)` — strips all non-CA descriptors, keeps `CA_descriptor`s at programme + ES level; `build_ca_pmt_for_caids(…, allowed)` / `CaPmtBuilt::retain_caids(&[u16])` filter to the CAM's advertised `CA_system_id`s |
| **Application Information** | `application_info_enq` / `application_info` / `enter_menu` |
| **Resource Manager** | `profile_enq` / `profile` (reply) / `profile_change` |
| **Date-Time** | `date_time_enq` / `date_time` (optional `local_offset`) |
| **Host Control** | `objects::host_control` (tune / replace / clear_replace / ask_release) |
| **MMI** | low-level/display/keypad/subtitle/download objects (`objects::mmi_display`, `mmi_close`) and high-level menu/list/enq/answ (`objects::mmi_high`) |
| **Low-Speed Communications** | `objects::low_speed_comms` |
| **Session (SPDU)** | `open`/`create`/`close` session request+response, `session_number`, status values |
| **Transport (TPDU)** | C_TPDU / R_TPDU (+ Status Byte), Create/Delete/Request/New_T_C, T_C_Error, SB_value |
| **DVB CI extensions** (`ci_ext`, ETSI TS 101 699) | resource-scoped dispatch (`CiExtApdu`, keyed on `resource_identifier()` because the extension resources reuse `0x9F80xx` tags): Resource Manager v2, Application Information v2, Power Manager, Event Manager, Copy Protection, StreamInput, ServiceGateway, BroadcastServiceGateway, Status Query, Application MMI, Download, CA Pipeline |
| **CI Plus extensions** (`ci_plus`, ETSI TS 103 205) | resource-scoped dispatch (`CiPlusApdu`): Multi-stream, Content Control (`cc_PIN_*` + SAC datatype model), Multi-stream Host Control (tune APDUs), Sample Decryption, CICAM Player, Auxiliary File System; the extended CA Support `ca_pmt`, LSC v4, URI v3 and Sample-Mode descriptors are standalone typed structs outside `CiPlusApdu` dispatch |

### Typed PIDs and preserved length coding

- Every 13-bit PID field (`CaPmtStream::elementary_pid`, `CaPmtReplyStream::elementary_pid`, `Replace::{replaced_pid,replacement_pid}`, the CI+ PID-select / sample-track / comms fields) is `mpeg_ts::pid::Pid`, so an out-of-range PID is unrepresentable rather than checked at serialize time (new dependency: `mpeg-ts`).
- A CAM may code a length in a longer form than necessary (`82 00 09` for 9, captured on hardware). `length::LengthForm` (`Minimal`, or `Long { octets }`) is captured by `parse` on every APDU, SPDU and TPDU object and re-emitted by `serialize`, so parse -> serialize is byte-exact. The header-only objects are therefore no longer unit structs (use `Default::default()`).

### Out of scope

CI+ crypto (the CC resource's cryptography) and the PC-Card hardware transport are not implemented (separate specs). An unallocated/private `apdu_tag` is produced by `AnyApdu::parse` as `AnyApdu::Unknown` with the raw body preserved (lossless round-trip).

## Features

| Feature | Default | Effect |
|---|---|---|
| `std` | ✅ | Link `std`. Off → `#![no_std]` + `alloc`. |
| `serde` | – | `serde::Serialize` on the public types. |

## Examples

Run with `cargo run -p dvb-ci --example <name>`:

- **`build_ca_pmt`** — build a `ca_pmt` from a PMT, stripping non-CA descriptors.
- **`parse_apdu`** — dispatch + round-trip an APDU through `AnyApdu`.

## License

MIT OR Apache-2.0.
