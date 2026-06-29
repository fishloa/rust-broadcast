//! ULE — Unidirectional Lightweight Encapsulation (RFC 4326) + Extension
//! Headers (RFC 5163): IP over MPEG-2 Transport Streams.
//!
//! ULE encapsulates a network-layer PDU (an IP datagram, Ethernet frame, etc.)
//! into a **SubNetwork Data Unit** ([`Sndu`]) and maps it into the payload of
//! MPEG-2 TS packets on a single PID (RFC 4326 §3, §4).
//!
//! This crate implements:
//!
//! - [`Sndu`] — the SNDU wire structure (RFC 4326 §4, Figure 1): the `D` bit +
//!   15-bit `Length` + 16-bit `Type`, an optional 6-byte Destination NPA
//!   address (present iff `D = 0`), the PDU, and the 4-byte CRC-32 trailer.
//!   `Length` and the CRC are **recomputed on serialize** from the typed
//!   fields — there is no raw passthrough.
//! - [`TypeField`] — the §4.4 split at `0x0600`: a Next-Header (`H-LEN`/
//!   `H-Type`) below, an EtherType at or above.
//! - [`ExtensionHeader`] / [`PayloadChain`] — the chained extension-header
//!   model (RFC 4326 §5, RFC 5163 §3): Optional headers (`H-LEN = 1..=5`,
//!   total `2·H-LEN` bytes) and a terminating EtherType or Mandatory header
//!   (Test-SNDU 0x00, Bridged-Frame 0x01, TS-Concat 0x02, PDU-Concat 0x03).
//! - [`UleReceiver`] — TS-packet de-fragmentation/reassembly (RFC 4326 §6, §7):
//!   PUSI + 1-byte Payload Pointer handling, fragmentation across packets,
//!   packing of multiple SNDUs per packet, and End-Indicator / 0xFF padding.
//!
//! The CRC-32 is the MPEG-2 / DSM-CC CRC (poly `0x04C11DB7`, init
//! `0xFFFFFFFF`, MSB-first, no reflection, no final XOR — RFC 4326 §4.6),
//! reused from [`broadcast_common::crc32_mpeg2`]; this is verified byte-exact against
//! RFC 4326 Appendix B's worked example in the crate's fixture test.
//!
//! `#![no_std]` + `alloc`; depends only on `dvb-common`.
//!
//! # Examples
//!
//! Build an IPv4 SNDU (L2 filtering, `D = 0`) from typed fields and round-trip
//! it:
//!
//! ```
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
    ExtensionHeader, MandatoryHType, OptionalHType, PayloadChain, H_TYPE_BRIDGED_FRAME,
    H_TYPE_EXT_PADDING, H_TYPE_PDU_CONCAT, H_TYPE_TEST_SNDU, H_TYPE_TIMESTAMP, H_TYPE_TS_CONCAT,
};
pub use sndu::{
    is_end_indicator, Sndu, BASE_HEADER_LEN, CRC_LEN, END_INDICATOR, END_INDICATOR_LENGTH, NPA_LEN,
    PADDING_BYTE,
};
pub use ts::{UleReceiver, TS_PAYLOAD_LEN};
pub use type_field::{TypeField, ETHERTYPE_BOUNDARY, ETHERTYPE_IPV4, ETHERTYPE_IPV6};
