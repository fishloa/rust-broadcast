//! ULE (Unidirectional Lightweight Encapsulation, RFC 4326) with Extension Headers (RFC 5163): parse, build and reassemble IP (and other network-layer) datagrams carried over MPEG-2 Transport Streams.
//!
//! ## What it does / what it does not
//!
//! ULE is the IETF's lightweight alternative to MPE (multi-protocol encapsulation) for sending IP over DVB-S/-T/-C MPEG-2 TS links. This crate gives you:
//!
//! - the SNDU (SubNetwork Data Unit) wire format, parsed and serialized symmetrically from typed fields;
//! - the extension-header chain model (Optional and Mandatory headers);
//! - a stateful TS-payload receiver that de-fragments SNDUs spanning TS packets.
//!
//! It is a framing library. It does **not**:
//!
//! - parse the PDU itself (IPv4/IPv6/Ethernet bytes are returned opaque; use an IP stack for that);
//! - open or demultiplex a Transport Stream (you pick the ULE PID and hand over TS payloads; use `mpeg-ts` for TS framing);
//! - decode the inner structure of Mandatory extension headers (Test-SNDU, Bridged-Frame, TS-Concat, PDU-Concat bodies are kept as opaque bytes);
//! - perform IP/MAC filtering, address resolution, or the RFC 4326 §7.3 continuity-counter bookkeeping (call `UleReceiver::reset` yourself on a discontinuity);
//! - implement ULE over anything other than MPEG-2 TS (for example the generic-stream/GSE encapsulation of DVB-S2 is a different standard and is not covered).
//!
//! ## Standards supported
//!
//! | Standard | Edition | Scope covered | Status | Notes |
//! |----------|---------|---------------|--------|-------|
//! | RFC 4326 — Unidirectional Lightweight Encapsulation (ULE) for Transmission of IP Datagrams over an MPEG-2 Transport Stream | December 2005 (matches `specs/rfc4326_ule.txt`) | §4 SNDU format (D bit, Length, Type, optional 6-byte NPA, CRC-32), §4.4 Type-field split at `0x0600`, §4.6 CRC-32, §5 extension-header framing, §6/§7 TS encapsulation and receiver | Full | CRC verified byte-exact against the Appendix B worked example (CRC `0x7C171763`). |
//! | RFC 4326 §5.1, §5.2, §5.3 — Test-SNDU, Bridged-Frame, Extension-Padding | December 2005 | H-Type constants, framing, length rules | Typed-opaque | Header identity and length are typed; the body bytes are opaque. |
//! | RFC 5163 — Extension Formats for ULE | April 2008 (matches `specs/rfc5163_ule_ext.txt`) | §3.1 TS-Concat, §3.2 PDU-Concat (Mandatory), §3.3 TimeStamp (Optional) | Typed-opaque | Header identity and length are typed; bodies are not decoded further. |
//!
//! ## Feature flags
//!
//! | Feature | Default | Enables |
//! |---------|---------|---------|
//! | `std`   | yes     | Links the standard library. Without it the crate is `#![no_std]` + `alloc`. |
//! | `serde` | no      | `serde::Serialize` derives on the public types (serialize-only). |
//!
//! ## Quick start
//!
//! ```rust
//! use ule::{Sndu, TypeField};
//!
//! let pdu = [0x45u8, 0x00, 0x00, 0x14]; // start of an IPv4 header
//! let sndu = Sndu::new(
//!     TypeField::EtherType(0x0800),
//!     Some([0x00, 0x01, 0x02, 0x03, 0x04, 0x05]),
//!     &pdu,
//! );
//! let mut buf = vec![0u8; sndu.serialized_len()];
//! sndu.serialize_into(&mut buf).unwrap();
//! assert_eq!(Sndu::parse(&buf).unwrap(), sndu);
//! ```
//!
//! Two runnable examples live in `examples/`:
//!
//! ```sh
//! cargo run -p ule --example build_sndu
//! cargo run -p ule --example receive_sndu
//! ```
//!
//! ## What is implemented
//!
//! - [`Sndu`] — the SNDU wire structure (RFC 4326 §4): the `D` bit, 15-bit `Length` and 16-bit `Type`, an optional 6-byte Destination NPA address (present iff `D = 0`), the PDU, and the 4-byte CRC-32 trailer. `Length` and the CRC are **recomputed on serialize** from the typed fields. `is_end_indicator` recognises the `0xFFFF` End Indicator.
//! - [`TypeField`] — the §4.4 split at `0x0600`: a Next-Header (`H-LEN`/`H-Type`) below, an EtherType at or above (`ETHERTYPE_IPV4`, `ETHERTYPE_IPV6` helpers).
//! - [`ExtensionHeader`] / [`PayloadChain`] — chained extension headers: Optional headers (`H-LEN = 1..=5`, total `2*H-LEN` bytes) terminated by an EtherType or a Mandatory header. [`MandatoryHType`] names Test-SNDU `0x00`, Bridged-Frame `0x01`, TS-Concat `0x02`, PDU-Concat `0x03`; [`OptionalHType`] names Extension-Padding `0x00` and TimeStamp `0x01`. `ExtensionHeader::validate` rejects an Optional header whose body length is not `2*H-LEN - 2`.
//! - [`UleReceiver`] — TS-payload de-fragmentation (RFC 4326 §6, §7): PUSI plus 1-byte Payload Pointer handling, fragmentation across packets, several SNDUs packed in one packet, End-Indicator and `0xFF` padding. `push` returns the completed SNDUs as owned byte vectors (header through CRC) that you then feed to `Sndu::parse`.
//!
//! ## Design notes
//!
//! - **Zero-copy, symmetric.** [`Sndu`] and [`PayloadChain`] borrow from the input (`Parse`/`Serialize` style) and every parser has a serializer; round-trip is tested, including against the real RFC 4326 Appendix B vector committed as a fixture.
//! - **No raw passthrough.** Length and CRC are always recomputed on serialize; a wrong length or CRC on the wire is a parse error (`Error::CrcMismatch`, `Error::InvalidLength`).
//! - **Error model.** One `thiserror` enum, [`Error`], with context-carrying variants (`BufferTooShort`, `OutputBufferTooSmall`, `InvalidLength`, `CrcMismatch`, `FieldTooWide`, `InvalidExtensionHeader`); serializers check output space before writing.
//! - Public enums are `#[non_exhaustive]` and carry `name()` / `Display` labels; `#![no_std]` + `alloc`.
//!
//! ## Related crates
//!
//! Depends only on `broadcast-common` (CRC-32/MPEG-2, `Parse`/`Serialize` traits). TS packet framing and PSI live in `mpeg-ts`; ULE sits above it and is consumed by whatever extracts the IP payload.
//!
//! ## MSRV and licence
//!
//! Minimum supported Rust version 1.95.0. Licensed under MIT OR Apache-2.0.
//!
#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
// Runnable examples, embedded so they render on docs.rs and stay in sync with
// the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n## Runnable examples\n"]
#![doc = "Run with `cargo run -p ule --example <name>`.\n"]
#![doc = "\n### `build_sndu`\n\n```rust,ignore"]
#![doc = include_str!("../examples/build_sndu.rs")]
#![doc = "```\n\n### `receive_sndu`\n\n```rust,ignore"]
#![doc = include_str!("../examples/receive_sndu.rs")]
#![doc = "```"]

extern crate alloc;

mod error;
mod ext_header;
mod sndu;
mod ts;
mod type_field;

pub use error::{Error, Result};
pub use ext_header::{
    ExtensionHeader, H_TYPE_BRIDGED_FRAME, H_TYPE_EXT_PADDING, H_TYPE_PDU_CONCAT, H_TYPE_TEST_SNDU,
    H_TYPE_TIMESTAMP, H_TYPE_TS_CONCAT, MandatoryHType, OptionalHType, PayloadChain,
};
pub use sndu::{
    BASE_HEADER_LEN, CRC_LEN, END_INDICATOR, END_INDICATOR_LENGTH, NPA_LEN, PADDING_BYTE, Sndu,
    is_end_indicator,
};
pub use ts::{TS_PAYLOAD_LEN, UleReceiver};
pub use type_field::{ETHERTYPE_BOUNDARY, ETHERTYPE_IPV4, ETHERTYPE_IPV6, TypeField};
