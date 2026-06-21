//! VBI data carriage in DVB — ETSI EN 301 775 V1.2.1 §4 (the PES data field).
//!
//! EN 301 775 specifies how Vertical Blanking Information (VBI) is carried in
//! MPEG-2 / DVB Transport Streams using the private PES packet mechanism
//! (`stream_id = private_stream_1` `0xBD`). It extends EN 300 472 (EBU Teletext
//! carriage) with **Inverted Teletext**, **VPS** (EN 300 231), **WSS**
//! (EN 300 294), **Closed Captioning** (line 21, EIA-608 Rev A), and a generic
//! **monochrome 4:2:2 luminance-sample** transport.
//!
//! This crate decodes the **PES data field** ([`DataField`], §4.4.1, Table 1):
//! a [`DataField::data_identifier`] byte (Table 2) followed by a loop of
//! [`DataUnit`]s. Each data unit is a [`DataUnitId`] (Table 3) + an 8-bit
//! `data_unit_length` + a typed [`DataUnitPayload`]:
//!
//! - [`TeletextDataField`] — EBU (`0x02`/`0x03`) and Inverted (`0xC0`) Teletext
//!   (§4.5): a shared [`LineHeader`] + an 8-bit `framing_code` + a 42-byte
//!   opaque `txt_data_block`. EN 300 706 Teletext coding is out of scope.
//! - [`VpsDataField`] — VPS (`0xC3`, §4.6): shared header + 13-byte block.
//! - [`WssDataField`] — WSS (`0xC4`, §4.7): shared header + a 14-bit
//!   `wss_data_block` + a 2-bit `reserved_future_use` `11` tail.
//! - [`ClosedCaptioningDataField`] — Closed Captioning (`0xC5`, §4.8): shared
//!   header + a 16-bit data block.
//! - [`MonochromeDataField`] — monochrome 4:2:2 samples (`0xC6`, §4.9): its own
//!   first-byte packing (first/last segment flags + field_parity + line_offset),
//!   a `first_pixel_position`, `n_pixels`, and the luminance `Y_value` bytes.
//! - Stuffing (`0xFF`, §4.4.1) and an `Opaque` catch-all for reserved /
//!   user-defined ids (Table 3: discard) round-trip verbatim.
//!
//! ⚠ Table 1's parse branch routes `data_unit_id` `0xC1` to `txt_data_field()`,
//! but Table 3 marks `0xC1` as *reserved → discard*. Table 3 is authoritative,
//! so `0xC1` decodes to [`DataUnitId::Reserved`] (see `docs/vbi.md`).
//!
//! No raw passthrough: every typed field re-serializes from its parsed value,
//! `data_unit_length` is recomputed from the typed body on serialize, and a
//! committed fixture is byte-exact round-tripped in the crate's tests.
//!
//! `#![no_std]` + `alloc`; depends only on `dvb-common`.
//!
//! # Examples
//!
//! Build a multi-unit VBI PES data field (VPS + WSS) from typed fields and
//! round-trip it:
//!
//! ```
//! use dvb_vbi::{DataField, DataUnit, LineHeader, VpsDataField, WssDataField};
//!
//! let vps = DataUnit::vps(VpsDataField {
//!     header: LineHeader::new(true, 16),
//!     vps_data_block: [0u8; 13],
//! });
//! let wss = DataUnit::wss(WssDataField {
//!     header: LineHeader::new(true, 23),
//!     wss_data_block: 0x1234,
//! });
//! let field = DataField::new(0x10, vec![vps, wss]);
//!
//! let mut buf = vec![0u8; field.serialized_len()];
//! field.serialize_into(&mut buf).unwrap();
//! assert_eq!(DataField::parse(&buf).unwrap(), field);
//! ```
#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
// Runnable examples, embedded so they render on docs.rs and stay in sync with
// the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n## Runnable examples\n"]
#![doc = "Run with `cargo run -p dvb-vbi --example <name>`.\n"]
#![doc = "\n### `build_data_field`\n\n```rust,ignore"]
#![doc = include_str!("../examples/build_data_field.rs")]
#![doc = "```\n\n### `parse_data_field`\n\n```rust,ignore"]
#![doc = include_str!("../examples/parse_data_field.rs")]
#![doc = "```"]

extern crate alloc;

mod data_unit_id;
mod error;
mod line_header;
mod payload;

pub use data_unit_id::{
    DataUnitId, ID_CLOSED_CAPTIONING, ID_EBU_TELETEXT_NON_SUBTITLE, ID_EBU_TELETEXT_SUBTITLE,
    ID_INVERTED_TELETEXT, ID_MONOCHROME_422_SAMPLES, ID_STUFFING, ID_VPS, ID_WSS,
};
pub use error::{Error, Result};
pub use line_header::{LineHeader, LINE_HEADER_LEN, RESERVED_PREFIX};
pub use payload::{
    ClosedCaptioningDataField, DataField, DataUnit, DataUnitPayload, MonochromeDataField,
    TeletextDataField, VpsDataField, WssDataField, CC_FIELD_LEN, FRAMING_CODE_EBU,
    FRAMING_CODE_INVERTED, MONO_HEADER_LEN, TELETEXT_DATA_UNIT_LENGTH, TELETEXT_FIELD_LEN,
    TXT_DATA_BLOCK_LEN, VPS_DATA_BLOCK_LEN, VPS_FIELD_LEN, WSS_DATA_BLOCK_MASK, WSS_FIELD_LEN,
    WSS_RESERVED_TAIL,
};
