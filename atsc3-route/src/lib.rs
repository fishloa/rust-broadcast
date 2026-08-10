//! ATSC A/331 Annex A **ROUTE** (Real-time Object delivery over Unidirectional
//! Transport) — the binary delivery layer, split out of the `atsc3` crate
//! (issue #943) so a `no_std` ROUTE receiver does not have to pull in an XML
//! stack (`atsc3`'s LLS/SLS signalling needs `roxmltree` + `flate2`).
//!
//! A/331 Annex A is written as a **profile-and-delta** on RFC 5651 (LCT), RFC
//! 5775 (ALC) and RFC 6726 (FLUTE) — this crate implements exactly the delta,
//! on top of [`rmt_flute`]'s implementation of the base RFCs. It does
//! **not** re-implement LCT/ALC/FLUTE parsing; see `rmt-flute`'s own crate
//! docs for that layer. The full transcription of the ROUTE delta this crate
//! covers lives at `atsc3/docs/a331-route.md` (fetched by the sibling `atsc3`
//! crate, which keeps the signalling half of A/331 — LLS today, SLS XML
//! planned).
//!
//! Implements:
//!
//! - [`RoutePacket`] — a composed ROUTE ALC/LCT packet: an
//!   [`rmt_flute::LctHeader`] constrained to A/331's mandated field widths
//!   (§A.3.4/§A.3.6) plus PSI-bit validation, the SPI-bit-dispatched
//!   [`RouteFecPayloadId`], and the opaque delivery-object payload.
//! - [`ext::ExtRoutePresentationTime`] — `EXT_ROUTE_PRESENTATION_TIME` (HET
//!   66, §A.3.7.1): the full 64-bit NTP presentation time of an MDE Random
//!   Access Point.
//! - [`ext::ExtTol`] — `EXT_TOL` (HET 194 fixed / 67 variable, §A.3.8.1): the
//!   delivery object's post-content-encoding transfer length, in its 24-bit
//!   and 48-bit forms.
//! - [`fec::SourceFecPayloadId`] / [`fec::RepairFecPayloadId`] /
//!   [`RouteFecPayloadId`] — the two ROUTE FEC Payload ID layouts
//!   (§A.3.5.1/§A.3.5.2): Compact No-Code `start_offset` for source flows,
//!   RaptorQ `SBN`/`ESI` (RFC 6330 §3.2) for repair flows.
//! - [`codepoint::Codepoint`] / [`codepoint::FormatId`] /
//!   [`codepoint::FragMode`] — the LCT Codepoint field's ROUTE-defined
//!   delivery-object semantics (§A.3.6, Table A.3.6).
//!
//! # Scope
//!
//! Out of scope, same as `rmt-flute`: the FDT/S-TSID/USBD/MPD **XML**
//! documents that ROUTE's binary framing carries as opaque payload bytes
//! (that is the `atsc3` crate's signalling half, or a consumer's own XML
//! layer), and the actual RaptorQ encode/decode procedure (RFC 6330 is not
//! vendored in this repository — only the `SBN`/`ESI` FEC Payload ID field
//! widths it and A/331's Figure A.3.4 agree on are implemented here).
//!
//! # ⚠ Repair-flow coverage gap
//!
//! The real ROUTE capture fixtures this crate is verified against
//! (`fixtures/atsc3/route-*.bin`) come from a session that ran with the LCT
//! SPI bit set on every packet across both source `.pcap` files (8,885
//! frames) — i.e. **no FEC-repair flow was ever active** in that capture.
//! [`fec::RepairFecPayloadId`] is implemented directly from A/331's own
//! Figure A.3.4 bit-diagram (matching RFC 6330 §3.2) and unit-tested against
//! hand-built vectors, but has no real-capture corroboration. See
//! `fixtures/atsc3/PROVENANCE.md`'s "What was not obtained" section.
//!
//! `#![no_std]` + `alloc` (via `rmt-flute`); depends only on `rmt-flute` and
//! `broadcast-common`.
//!
//! # Examples
//!
//! Parse a real ROUTE source-flow media fragment and decode its Codepoint:
//!
//! ```
//! use atsc3_route::{Codepoint, RoutePacket};
//! use broadcast_common::{Parse, Serialize};
//!
//! // A minimal, hand-built ROUTE source packet: CCI=4B, TSI=3000, TOI=6034,
//! // CP=128 (indirect — real streams resolve this via the S-TSID XML).
//! let cci = [0u8; 4];
//! let tsi = 3000u32.to_be_bytes();
//! let toi = 6034u32.to_be_bytes();
//! let lct = rmt_flute::LctHeader {
//!     version: rmt_flute::LCT_VERSION,
//!     psi: rmt_flute::PSI_SPI,
//!     close_session: false,
//!     close_object: false,
//!     codepoint: 128,
//!     cci: &cci,
//!     tsi: &tsi,
//!     toi: &toi,
//!     extensions: vec![],
//! };
//! let payload = [0xDEu8, 0xAD, 0xBE, 0xEF];
//! let pkt = RoutePacket {
//!     lct,
//!     fec_payload_id: atsc3_route::RouteFecPayloadId::Source(
//!         atsc3_route::SourceFecPayloadId { start_offset: 1408 },
//!     ),
//!     payload: &payload,
//! };
//!
//! let bytes = pkt.to_bytes();
//! let re = RoutePacket::parse(&bytes).unwrap();
//! assert_eq!(re, pkt);
//! assert!(matches!(re.codepoint(), Codepoint::Indirect(128)));
//! ```
#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
// Runnable examples, embedded so they render on docs.rs and stay in sync with
// the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n## Runnable examples\n"]
#![doc = "Run with `cargo run -p atsc3-route --example <name>`.\n"]
#![doc = "\n### `parse_route_fdt`\n\n```rust,ignore"]
#![doc = include_str!("../examples/parse_route_fdt.rs")]
#![doc = "```\n\n### `build_route_media_fragment`\n\n```rust,ignore"]
#![doc = include_str!("../examples/build_route_media_fragment.rs")]
#![doc = "```"]

extern crate alloc;

pub mod codepoint;
mod error;
pub mod ext;
pub mod fec;
mod packet;

pub use codepoint::{Codepoint, CodepointSemantics, FormatId, FragMode};
pub use error::{Error, Result};
pub use ext::{
    EXT_ROUTE_PRESENTATION_TIME_CONTENT_LEN, EXT_TOL_24_CONTENT_LEN, EXT_TOL_48_CONTENT_LEN,
    ExtRoutePresentationTime, ExtTol, HET_EXT_ROUTE_PRESENTATION_TIME, HET_EXT_TOL_24,
    HET_EXT_TOL_48, MAX_TOL_24, MAX_TOL_48,
};
pub use fec::{
    MAX_ESI, ROUTE_FEC_PAYLOAD_ID_LEN, RepairFecPayloadId, RouteFecPayloadId, SourceFecPayloadId,
};
pub use packet::{
    ROUTE_CCI_LEN, ROUTE_PSI_SOURCE, ROUTE_TOI_LEN, ROUTE_TSI_LEN, ROUTE_VERSION, RoutePacket,
};

// Re-export the underlying `rmt-flute` types this crate's public API is
// expressed in terms of, so a consumer of `RoutePacket` (which embeds an
// `rmt_flute::LctHeader`) never needs an explicit `rmt-flute` dependency of
// its own to build one.
pub use rmt_flute;
