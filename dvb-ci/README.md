# dvb-ci

[![crates.io](https://img.shields.io/crates/v/dvb-ci.svg)](https://crates.io/crates/dvb-ci)
[![docs.rs](https://img.shields.io/docsrs/dvb-ci)](https://docs.rs/dvb-ci)
[![MSRV](https://img.shields.io/badge/MSRV-1.81-blue.svg)](https://blog.rust-lang.org/)
[![license](https://img.shields.io/crates/l/dvb-ci.svg)](#license)

**DVB Common Interface (EN 50221)** — the host ↔ CICAM wire protocol. Parses and
builds the EN 50221 protocol objects across all three layers, plus a **`CA_PMT`
builder** that turns a [`dvb-si`](https://crates.io/crates/dvb-si) PMT into the
object handed to a Conditional Access Module.

Every wire type implements `dvb_common::Parse` / `dvb_common::Serialize`
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
use dvb_common::Serialize;

// A ca_info() APDU advertising two CA_system_ids.
let info = CaInfo { ca_system_ids: vec![0x0500, 0x0B00] };
let wire = info.to_bytes();                // 9F 80 31 04 05 00 0B 00

// Route any APDU by its 3-byte tag without knowing the type up front.
let any = AnyApdu::parse(&wire).unwrap();
assert_eq!(any.name(), "CA_INFO");
assert_eq!(any.to_bytes(), wire);          // round-trips byte-for-byte
```

## What's implemented (v0.1.0)

| Layer | Objects |
|---|---|
| **APDU framework** | 3-byte `ApduTag` + Table 58 constants; ASN.1 `length_field` codec; `AnyApdu` tag dispatch (Def-trait + `declare_apdus!` macro + drift test); 4-octet `ResourceId` |
| **CA support** | `ca_info_enq` / `ca_info`, `ca_pmt`, `ca_pmt_reply` |
| **CA_PMT builder** | `build_ca_pmt(PmtSection, list_management, cmd_id)` — strips all non-CA descriptors, keeps `CA_descriptor`s at programme + ES level |
| **Application Information** | `application_info_enq` / `application_info` / `enter_menu` |
| **Resource Manager** | `profile_enq` / `profile` (reply) / `profile_change` |
| **Date-Time** | `date_time_enq` / `date_time` (optional `local_offset`) |
| **MMI** | `close_mmi` |
| **Session (SPDU)** | `open`/`create`/`close` session request+response, `session_number`, status values |
| **Transport (TPDU)** | C_TPDU / R_TPDU (+ Status Byte), Create/Delete/Request/New_T_C, T_C_Error, SB_value |

### Deferred to a follow-up

The MMI **high-level** objects (text / enq / answ / menu / list, Tables 46-51),
the MMI low-level/display objects, and the **Host Control** (tune / replace) and
**Low-Speed Communications** resources are not yet typed. Their `apdu_tag`s are
retained in [`docs/en_50221/apdu-tag-values.md`](docs/en_50221/apdu-tag-values.md);
until implemented, `AnyApdu::parse` yields them as `AnyApdu::Unknown` with the
raw body preserved (lossless round-trip). CI+ crypto (the CC resource) and the
PC-Card hardware transport are out of scope (separate specs).

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
