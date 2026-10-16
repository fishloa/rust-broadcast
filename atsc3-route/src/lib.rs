//! ATSC 3.0 ROUTE (A/331 Annex A) binary framing: the ROUTE-specific delta on top of the LCT/ALC/FLUTE wire formats in `rmt-flute`.
//!
//! > **ARCHIVED: ATSC 3.0 work in this workspace is abandoned.** The sibling `atsc3` signalling crate is permanently unpublished; `atsc3-route` is live on crates.io (0.2.0) and receives only maintenance releases. The market is closed enough that real captures are scarce: the two ROUTE header extensions this crate once decoded appear in zero of 14,000+ real captured packets. `rmt-flute` is useful independently.
//!
//! ```toml
//! [dependencies]
//! atsc3-route = "0.2"
//! ```
//!
//! ## What it does / what it does not
//!
//! A/331 Annex A defines ROUTE as a profile-and-delta on RFC 5651 (LCT), RFC 5775 (ALC) and RFC 6726 (FLUTE). This crate implements exactly the delta (the constrained LCT header, the two FEC Payload ID layouts, the Codepoint semantics, extension-type constants) and leaves the base RFCs to `rmt-flute`. It was split out of `atsc3` so a `no_std` ROUTE receiver does not need an XML stack.
//!
//! It does **not**:
//!
//! - parse the ROUTE signalling documents (S-TSID, USBD, MPD, FDT Instance XML) or any Service Layer Signalling: those ride as opaque payload bytes;
//! - implement RaptorQ encoding or decoding (RFC 6330 is not vendored here; only the SBN/ESI Payload ID field layout that A/331 Figure A.3.4 and RFC 6330 §3.2 agree on is implemented);
//! - decode `EXT_ROUTE_PRESENTATION_TIME` or `EXT_TOL`: only their HET constants remain, because no public capture carries them;
//! - run a receiver or sender session.
//!
//! ## Standards supported
//!
//! | Standard | Edition | Scope covered | Status | Notes |
//! |----------|---------|---------------|--------|-------|
//! | ATSC A/331 — Signaling, Delivery, Synchronization, and Error Protection, Annex A (ROUTE) | Code cites A/331:2025-06; the vendored transcription is A/331:2026-04 (the workspace transcription notes record no differing value in the clauses transcribed) | §A.3.4 / §A.3.6 constrained LCT fields (`V`=1, `C`=00, `S`=1, `O`=01, `H`=0, PSI/SPI rule); §A.3.5.1 / §A.3.5.2 source and repair FEC Payload IDs; §A.3.6 Table A.3.6 Codepoint semantics and Table A.3.2 format IDs; §A.3.7.1 and §A.3.8.1 extension HET constants | Partial | Extension bodies (`EXT_ROUTE_PRESENTATION_TIME`, `EXT_TOL`) not decoded: HET constants only. Repair-flow Payload ID is verified against the spec figure only, not a real capture. |
//! | IETF RFC 5651 / RFC 5775 / RFC 6726 (LCT / ALC / FLUTE) | October 2009 / April 2010 / November 2012 | Base wire formats | Delegated | Implemented by `rmt-flute`; this crate wraps its `LctHeader`. |
//! | IETF RFC 6330 — RaptorQ FEC Scheme §3.2 | Not vendored: unverified | `SBN`/`ESI` FEC Payload ID field widths only | Typed-opaque | No encode/decode procedure. |
//!
//! ## Feature flags
//!
//! | Feature | Default | Enables |
//! |---------|---------|---------|
//! | `std`   | yes     | Links the standard library (also enables `std` in `rmt-flute`, `broadcast-common`). Without it the crate is `#![no_std]` + `alloc`. |
//! | `serde` | no      | `serde::Serialize` derives on the public types (and `rmt-flute/serde`). |
//!
//! ## Quick start
//!
//! ```rust
//! use atsc3_route::{Codepoint, RoutePacket};
//! use broadcast_common::{Parse, Serialize};
//!
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
//! let bytes = pkt.try_to_bytes().unwrap();
//! let re = RoutePacket::parse(&bytes).unwrap();
//! assert_eq!(re, pkt);
//! assert!(matches!(re.codepoint(), Codepoint::Indirect(128)));
//! ```
//!
//! The packet above is hand-built. The crate's integration tests parse and byte-exact round-trip real captures (`fixtures/atsc3/route-*.bin`: an FDT instance, audio and video media fragments, and a signed SLS package). Runnable examples:
//!
//! ```sh
//! cargo run -p atsc3-route --example parse_route_fdt
//! cargo run -p atsc3-route --example build_route_media_fragment
//! ```
//!
//! ## What is implemented
//!
//! - [`RoutePacket`]: an `rmt_flute::LctHeader` constrained to A/331's mandated widths (`ROUTE_VERSION`, `ROUTE_CCI_LEN`, `ROUTE_TSI_LEN`, `ROUTE_TOI_LEN`, `ROUTE_PSI_SOURCE`), the SPI-dispatched [`RouteFecPayloadId`] and the opaque delivery-object payload; `spi()` and `codepoint()` accessors.
//! - `ext`: [`HET_EXT_ROUTE_PRESENTATION_TIME`] (66), [`HET_EXT_TOL_24`] (194), [`HET_EXT_TOL_48`] (67), for walking an LCT extension chain.
//! - `fec`: [`SourceFecPayloadId`] (Compact No-Code `start_offset`, source flows), [`RepairFecPayloadId`] (`SBN`/`ESI`, repair flows, [`MAX_ESI`]), dispatched by [`RouteFecPayloadId`] on the LCT SPI bit (`ROUTE_FEC_PAYLOAD_ID_LEN` bytes).
//! - `codepoint`: [`Codepoint`] (named values 1-9, reserved 0 and 10-127, indirect 128-255), [`FormatId`] (Table A.3.2: File, Entity, Unsigned Package, Signed Package), [`FragMode`], [`CodepointSemantics`] and `Codepoint::known_semantics()` for the directly-resolvable values 1-9.
//! - The `rmt_flute` crate is re-exported so a [`RoutePacket`] can be built without a direct `rmt-flute` dependency.
//!
//! ## Design notes
//!
//! - Zero-copy, symmetric `Parse`/`Serialize`; round-trip tested against the real captures. Fixture discipline is why the two extension decoders were removed (0.2.0): a type without a real-capture round-trip does not ship.
//! - **Known coverage gap:** the committed captures never had an active FEC-repair flow (the SPI bit was set on every packet across 8,885 frames), so [`RepairFecPayloadId`] is unit-tested against hand-built vectors from A/331 Figure A.3.4 only. See `fixtures/atsc3/PROVENANCE.md`.
//! - One `thiserror` enum, [`Error`]; public enums are `#[non_exhaustive]` with `name()` / `Display` labels; `#![no_std]` + `alloc`.
//!
//! ## Related crates
//!
//! Depends on `rmt-flute` and `broadcast-common` only. `atsc3` (archived, unpublished) is the signalling-side sibling; the ROUTE transcription lives in that crate's `docs/a331-route.md`.
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
pub use ext::{HET_EXT_ROUTE_PRESENTATION_TIME, HET_EXT_TOL_24, HET_EXT_TOL_48};
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
