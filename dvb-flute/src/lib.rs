//! Multicast object-delivery wire formats: **ALC / LCT / FLUTE / NORM**.
//!
//! This crate parses and serializes the binary headers used to deliver files
//! and streams over IP multicast (the building blocks beneath DVB-IPTV /
//! DVB-MABR file delivery and IETF RMT):
//!
//! - [`LctHeader`] — the **Layered Coding Transport** header (RFC 5651 §5). The
//!   fixed first word carries `V`/`C`/`PSI`/`S`/`O`/`H`/`A`/`B`, `HDR_LEN` and
//!   the Codepoint; the `C`, `S`, `O` and `H` flags then drive the byte-widths
//!   of the **CCI**, **TSI** and **TOI** fields (`4*(C+1)`, `4*S+2*H`,
//!   `4*O+2*H` bytes). The shared `H` half-word feeds both TSI *and* TOI. Flag
//!   bits and `HDR_LEN` are recomputed on serialize from the typed field
//!   lengths — there is no raw passthrough.
//! - [`HeaderExtension`] — the LCT/NORM header-extension chain (RFC 5651 §5.2):
//!   variable-length (`HET` 0..=127, carries `HEL`) and fixed-length (`HET`
//!   128..=255, one word) forms; with [`ExtTime`] (EXT_TIME) and the
//!   [`LctExtType`] registry (EXT_NOP/EXT_AUTH/EXT_TIME).
//! - [`AlcPacket`] — an **Asynchronous Layered Coding** packet (RFC 5775):
//!   LCT header + an opaque FEC Payload ID + the encoding-symbol payload, plus
//!   `EXT_FTI` (HET 64) and the Small-Block-Systematic [`FecPayloadId128`].
//! - [`ExtFdt`] / [`ExtCenc`] — the **FLUTE** (RFC 6726) fixed-length LCT
//!   extensions `EXT_FDT` (HET 192) and `EXT_CENC` (HET 193), plus the TOI = 0
//!   FDT-Instance convention. The FDT Instance body is XML and is **out of
//!   scope** of this binary crate — it rides as the packet payload.
//! - [`NormCommonHeader`] + [`NormData`] / [`NormCmd`] / [`NormFeedback`] — the
//!   **NORM** (RFC 5740) common header and message types (NORM_DATA / INFO /
//!   CMD / NACK / ACK / REPORT).
//!
//! ⚠ **FEC Payload ID** bit layouts are FEC-scheme dependent (RFC 5052 / the FEC
//! Scheme document) and are **not** defined by ALC/NORM themselves; this crate
//! exposes them as opaque byte slices (the caller supplies the length), with
//! [`FecPayloadId128`] provided as one concrete illustrative layout.
//!
//! All integer fields are big-endian. `#![no_std]` + `alloc`; depends only on
//! `dvb-common`.
//!
//! # Examples
//!
//! Build an LCT header from typed fields (flag-driven CCI/TSI/TOI widths) and
//! round-trip it:
//!
//! ```
//! use dvb_flute::{LctHeader, LCT_VERSION};
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
#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
// Runnable examples, embedded so they render on docs.rs and stay in sync with
// the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n## Runnable examples\n"]
#![doc = "Run with `cargo run -p dvb-flute --example <name>`.\n"]
#![doc = "\n### `build_lct`\n\n```rust,ignore"]
#![doc = include_str!("../examples/build_lct.rs")]
#![doc = "```\n\n### `parse_flute`\n\n```rust,ignore"]
#![doc = include_str!("../examples/parse_flute.rs")]
#![doc = "```"]

extern crate alloc;

mod alc;
mod error;
mod ext;
mod flute;
mod lct;
mod lct_ext;
mod norm;

pub use alc::{
    AlcPacket, FecPayloadId128, FEC_PAYLOAD_ID_128_LEN, HET_EXT_FTI as ALC_HET_EXT_FTI, PSI_SPI,
};
pub use error::{Error, Result};
pub use ext::{chain_len, parse_chain, serialize_chain, HeaderExtension, FIXED_HET_MIN, WORD};
pub use flute::{
    CencAlgorithm, ExtCenc, ExtFdt, FDT_INSTANCE_ID_MAX, FLUTE_VERSION, HET_EXT_CENC, HET_EXT_FDT,
    TOI_FDT,
};
pub use lct::{LctHeader, FIXED_HEADER_LEN, LCT_VERSION};
pub use lct_ext::{
    ExtTime, LctExtType, HET_EXT_AUTH as LCT_HET_EXT_AUTH, HET_EXT_NOP, HET_EXT_TIME, USE_ERT,
    USE_SCT_HIGH, USE_SCT_LOW, USE_SLC,
};
pub use norm::{
    NormAckType, NormCmd, NormCmdType, NormCommonHeader, NormData, NormFeedback, NormMessageType,
    SenderWord, COMMON_HEADER_LEN, FEEDBACK_FIXED_LEN, HET_EXT_AUTH as NORM_HET_EXT_AUTH,
    HET_EXT_CC, HET_EXT_FTI as NORM_HET_EXT_FTI, HET_EXT_RATE, NORM_FLAG_EXPLICIT, NORM_FLAG_FILE,
    NORM_FLAG_INFO, NORM_FLAG_REPAIR, NORM_FLAG_STREAM, NORM_FLAG_UNRELIABLE, NORM_NODE_ANY,
    NORM_NODE_NONE, NORM_VERSION, SENDER_WORD_LEN,
};
