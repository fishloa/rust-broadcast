//! The binary wire formats of IETF Reliable Multicast Transport (RMT): ALC, LCT, FLUTE and NORM headers, used to deliver files and streams over IP multicast.
//!
//! ## What it does / what it does not
//!
//! Every format here is **IETF RMT** (RFC 5651, 5775, 6726, 5740); no broadcast-specific standard is implemented by this crate. Delivery systems that sit on top of RMT consume it rather than own it: DVB-IPTV and DVB-MABR (ETSI TS 103 769; the `dvb-mabr` crate parses its session XML and does not depend on this crate), 3GPP MBMS/eMBMS download delivery, and ATSC 3.0 ROUTE (A/331 Annex A, see `atsc3-route`).
//!
//! Use it to parse or build LCT/ALC packets, FLUTE extension headers and NORM messages byte-exactly, and to compute the scheme-agnostic source-block partition of a transport object. It does **not**:
//!
//! - implement a FEC scheme: the FEC Payload ID and Scheme-specific FEC OTI layouts depend on the FEC Encoding ID (RFC 5052 and the scheme documents) and travel as opaque byte slices whose length the caller supplies ([`FecPayloadId128`] is one concrete example layout);
//! - encode or decode FEC symbols (no Reed-Solomon, Raptor, LDPC or similar);
//! - parse the FLUTE FDT Instance: it is XML carried as the packet payload on TOI 0 and is out of scope of this binary crate (a caller supplies its own XML parser);
//! - run a sender/receiver session, congestion control, or NORM state machines (it is wire format only, no I/O);
//! - verify `EXT_AUTH` signatures (the extension is carried, not interpreted).
//!
//! > **Renamed from `dvb-flute` at 0.4.0.** The old name implied a DVB standard this crate does not implement. All `dvb-flute` versions are yanked; there is no shim.
//!
//! ## Standards supported
//!
//! | Standard | Edition | Scope covered | Status | Notes |
//! |----------|---------|---------------|--------|-------|
//! | RFC 5651 — Layered Coding Transport (LCT) Building Block | October 2009 | §5 LCT header (flag-driven CCI/TSI/TOI widths, HDR_LEN, Codepoint), §5.2 header-extension chain, EXT_NOP / EXT_AUTH / EXT_TIME | Full | EXT_AUTH body opaque; EXT_TIME carries SCT/ERT/SLC with reserved Use bits and trailing bytes preserved for byte-exact round-trip. |
//! | RFC 5775 — Asynchronous Layered Coding (ALC) Protocol Instantiation | April 2010 | §3 ALC packet (LCT header + FEC Payload ID + payload), EXT_FTI (HET 64), PSI SPI bit | Partial | FEC Payload ID and FTI body are scheme-dependent and opaque; [`FecPayloadId128`] provided as one layout. |
//! | RFC 6726 — FLUTE: File Delivery over Unidirectional Transport | November 2012 | EXT_FDT (HET 192), EXT_CENC (HET 193), TOI 0 FDT-Instance convention, FLUTE version and Instance-ID limit | Partial | The FDT Instance XML itself is not parsed. |
//! | RFC 5740 — NACK-Oriented Reliable Multicast (NORM) Protocol | November 2009 | §4 common header, NORM_INFO, NORM_DATA, NORM_CMD (typed per sub-type), NORM_NACK / NORM_ACK feedback, sender-word, header extensions (EXT_AUTH, EXT_CC, EXT_RATE, EXT_FTI) | Partial | NORM_REPORT has no defined wire format in the RFC (§4.4.1) and is exposed as opaque content. FEC Payload IDs opaque. |
//! | RFC 5052 — Forward Error Correction (FEC) Building Block | Revision referenced by the code (`fec.rs` cites §9.1) | §9.1 Block Partitioning Algorithm only | Partial | Edition not vendored under `specs/`: unverified. |
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
//! use rmt_flute::{LctHeader, LCT_VERSION};
//!
//! let cci = [0u8; 4]; // C = 0
//! let tsi = [0u8; 4]; // S = 1, H = 0
//! let hdr = LctHeader {
//!     version: LCT_VERSION,
//!     psi: 0,
//!     close_session: false,
//!     close_object: false,
//!     codepoint: 0,
//!     cci: &cci,
//!     tsi: &tsi,
//!     toi: &[],
//!     extensions: vec![],
//! };
//! let mut buf = vec![0u8; hdr.serialized_len()];
//! hdr.serialize_into(&mut buf).unwrap();
//! let (re, used) = LctHeader::parse(&buf).unwrap();
//! assert_eq!(used, buf.len());
//! assert_eq!(re, hdr);
//! ```
//!
//! Runnable examples in `examples/`:
//!
//! ```sh
//! cargo run -p rmt-flute --example build_lct
//! cargo run -p rmt-flute --example parse_flute
//! ```
//!
//! ## What is implemented
//!
//! - [`LctHeader`] (RFC 5651 §5): the fixed first word carries `V`/`C`/`PSI`/`S`/`O`/`H`/`A`/`B`, `HDR_LEN` and the Codepoint; the `C`, `S`, `O`, `H` flags drive the byte-widths of **CCI** (`4*(C+1)`), **TSI** (`4*S+2*H`) and **TOI** (`4*O+2*H`). The shared `H` half-word feeds both TSI and TOI. Flag bits and `HDR_LEN` are recomputed on serialize from the typed field lengths; there is no raw passthrough.
//! - [`HeaderExtension`] with [`parse_chain`] / [`serialize_chain`] / [`chain_len`]: variable-length (`HET` 0..=127, carries `HEL`) and fixed-length (`HET` 128..=255, one word) forms; [`ExtTime`] and the [`LctExtType`] registry (EXT_NOP / EXT_AUTH / EXT_TIME).
//! - [`AlcPacket`] (RFC 5775): LCT header, opaque FEC Payload ID, encoding-symbol payload; `EXT_FTI`; [`FecPayloadId128`].
//! - [`ExtFdt`] / [`ExtCenc`] / [`CencAlgorithm`] (RFC 6726): the FLUTE LCT extensions and the TOI 0 constant [`TOI_FDT`].
//! - [`NormCommonHeader`], [`NormInfo`], [`NormData`], [`NormCmd`] (with a typed [`NormCmdBody`] per command sub-type), [`NormFeedback`] (NACK/ACK), [`NormMessageType`], [`NormCmdType`], [`NormAckType`], [`SenderWord`].
//! - [`SourceBlockPartition`] (RFC 5052 §9.1): given Transfer-Length, Encoding-Symbol-Length and Maximum-Source-Block-Length, derive the number and length of source blocks. This is a scheme-agnostic substrate for `atsc3-route` (and any future DVB-MABR transport) that avoids hardcoding any FEC scheme.
//!
//! ## Design notes
//!
//! - **Zero-copy, symmetric.** Parsed headers borrow `&[u8]` slices from the input; every parser has a symmetric serializer, with round-trip tests and committed real fixtures (an FLUTE FDT packet and a ROUTE FDT/FEC packet).
//! - **Reserved bits are preserved, not rejected, where the RFC says receivers MUST ignore them** (for example EXT_TIME reserved Use bits and the EXT_CENC reserved field), so input round-trips byte-exactly.
//! - **Error model.** One `thiserror` enum, [`Error`], with context-carrying variants; length fields are written through checked helpers.
//! - Public enums are `#[non_exhaustive]` with `name()` / `Display` labels; `#![no_std]` + `alloc`; all integers big-endian.
//!
//! ## Related crates
//!
//! Depends only on `broadcast-common`. Consumed within the workspace by `atsc3-route` (ATSC 3.0 ROUTE profile). The `mpeg-ts`/`ule` crates are unrelated transports; ALC/LCT rides over UDP.
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
#![doc = "Run with `cargo run -p rmt-flute --example <name>`.\n"]
#![doc = "\n### `build_lct`\n\n```rust,ignore"]
#![doc = include_str!("../examples/build_lct.rs")]
#![doc = "```\n\n### `parse_flute`\n\n```rust,ignore"]
#![doc = include_str!("../examples/parse_flute.rs")]
#![doc = "```"]

extern crate alloc;

mod alc;
mod error;
mod ext;
mod fec;
mod flute;
mod lct;
mod lct_ext;
mod norm;

pub use alc::{
    AlcPacket, FEC_PAYLOAD_ID_128_LEN, FecPayloadId128, HET_EXT_FTI as ALC_HET_EXT_FTI, PSI_SPI,
};
pub use error::{Error, Result};
pub use ext::{FIXED_HET_MIN, HeaderExtension, WORD, chain_len, parse_chain, serialize_chain};
pub use fec::SourceBlockPartition;
pub use flute::{
    CencAlgorithm, ExtCenc, ExtFdt, FDT_INSTANCE_ID_MAX, FLUTE_VERSION, HET_EXT_CENC, HET_EXT_FDT,
    TOI_FDT,
};
pub use lct::{FIXED_HEADER_LEN, LCT_VERSION, LctHeader};
pub use lct_ext::{
    ExtTime, HET_EXT_AUTH as LCT_HET_EXT_AUTH, HET_EXT_NOP, HET_EXT_TIME, LctExtType, USE_ERT,
    USE_SCT_HIGH, USE_SCT_LOW, USE_SLC,
};
pub use norm::{
    COMMON_HEADER_LEN, FEEDBACK_FIXED_LEN, HET_EXT_AUTH as NORM_HET_EXT_AUTH, HET_EXT_CC,
    HET_EXT_FTI as NORM_HET_EXT_FTI, HET_EXT_RATE, NORM_FLAG_EXPLICIT, NORM_FLAG_FILE,
    NORM_FLAG_INFO, NORM_FLAG_REPAIR, NORM_FLAG_STREAM, NORM_FLAG_UNRELIABLE, NORM_INFO_FIXED_LEN,
    NORM_NODE_ANY, NORM_NODE_NONE, NORM_VERSION, NormAckType, NormCmd, NormCmdBody, NormCmdType,
    NormCommonHeader, NormData, NormFeedback, NormInfo, NormMessageType, SENDER_WORD_LEN,
    SenderWord,
};
