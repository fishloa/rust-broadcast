//! `mp4-emsg` — ISO BMFF / DASH Event Message Box (`emsg`): inband DASH/CMAF
//! timed events (SCTE 35 splice signalling, ID3 metadata, ad/tracking triggers).
//!
//! The `emsg` box delivers sparse, timed application events alongside the media
//! in a DASH/CMAF segment. This crate implements:
//!
//! - [`EmsgBox`] — the `'emsg'` ISOBMFF `FullBox` (`size` / `'emsg'` /
//!   `version` / `flags`) plus both version bodies: the two null-terminated
//!   UTF-8 strings (`scheme_id_uri`, `value`), the integer fields (`timescale`,
//!   `event_duration`, `id`), the version-discriminated presentation-time
//!   field, and the opaque `message_data[]`. `size` and `version` are
//!   **recomputed/derived on serialize** from the typed fields (no raw
//!   passthrough).
//! - [`PresentationTime`] — the version-discriminated timing field:
//!   `presentation_time_delta` (u32, version 0, segment-relative) vs
//!   `presentation_time` (u64, version 1, representation-relative). Selecting a
//!   variant *is* selecting the box version.
//! - [`EmsgVersion`] — the `version` byte (0 / 1) with its spec label.
//! - [`EmsgBox::is_scte35`] — recognises the SCTE 35 scheme
//!   ([`SCTE35_SCHEME_PREFIX`], `urn:scte:scte35…`), in which case
//!   `message_data` carries a SCTE 35 `splice_info_section`.
//!
//! Note the **v0/v1 field ordering differs**: version 0 places the two strings
//! *first* (before the integer fields); version 1 places the integer fields
//! first and the strings last. Both orderings are parsed and serialized.
//!
//! # ⚠ Source footing — softer than the fully-free crates
//!
//! The `emsg` **field semantics and types** (`scheme_id_uri`, `value`,
//! `timescale`, `presentation_time`/`presentation_time_delta`,
//! `event_duration`, `id`, `message_data`) are render-verified from a **free**
//! source: **DASH-IF IOP Part 10 (Events and Timed Metadata) V5.0.0, §6.1 +
//! Table 6-2** (transcribed in `mp4-emsg/docs/emsg.md`).
//!
//! However, the **normative ISOBMFF box syntax** —
//!
//! ```text
//! aligned(8) class EventMessageBox extends FullBox('emsg', version, flags = 0)
//! ```
//!
//! — i.e. the exact byte-level field *ordering*, the `version`-gated branch
//! (`presentation_time_delta` vs `presentation_time`), and the
//! null-terminated-string layout, lives in **ISO/IEC 23009-1 §5.10.3.3**, which
//! is **paid and NOT vendored** in this repo. DASH-IF Part 10 references it but
//! does not reproduce it. The box layout here is therefore implemented from the
//! **well-known public `emsg` structure** (widely reproduced in MPEG-DASH /
//! CMAF) combined with the free DASH-IF Part 10 semantics, with **ISO/IEC
//! 23009-1 §5.10.3.3 cited as the formal (paid) normative source**. This is
//! **softer footing** than the fully-free crates in this workspace — flagged
//! here per project policy. The v0/v1 ordering in particular is the part most
//! reliant on the non-vendored ISO source.
//!
//! Likewise, the SCTE 35 scheme-URI strings and the `message_data` binding are
//! defined by **SCTE 214-1 / ANSI/SCTE 35** (not vendored); [`is_scte35`] only
//! recognises the well-known `urn:scte:scte35…` URI prefix.
//!
//! [`is_scte35`]: EmsgBox::is_scte35
//!
//! `#![no_std]` + `alloc`; depends only on `broadcast-common`.
//!
//! # Examples
//!
//! Build a version 0 (segment-relative) `emsg` from typed fields and round-trip
//! it:
//!
//! ```
//! use mp4_emsg::{EmsgBox, PresentationTime};
//!
//! let scte35 = [0xFCu8, 0x30, 0x11]; // start of a splice_info_section
//! let b = EmsgBox {
//!     scheme_id_uri: "urn:scte:scte35:2013:bin",
//!     value: "",
//!     timescale: 90_000,
//!     presentation_time: PresentationTime::Delta(0),
//!     event_duration: 0xFFFF_FFFF,
//!     id: 1,
//!     message_data: &scte35,
//! };
//! assert!(b.is_scte35());
//! let bytes = b.to_vec().unwrap();
//! assert_eq!(EmsgBox::parse(&bytes).unwrap(), b);
//! ```
#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
// Runnable examples, embedded so they render on docs.rs and stay in sync with
// the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n## Runnable examples\n"]
#![doc = "Run with `cargo run -p mp4-emsg --example <name>`.\n"]
#![doc = "\n### `build_emsg`\n\n```rust,ignore"]
#![doc = include_str!("../examples/build_emsg.rs")]
#![doc = "```\n\n### `parse_emsg`\n\n```rust,ignore"]
#![doc = include_str!("../examples/parse_emsg.rs")]
#![doc = "```"]

extern crate alloc;

mod emsg;
mod error;
mod version;

pub use emsg::{
    EMSG_BOX_TYPE, EMSG_FLAGS, EmsgBox, FULLBOX_HEADER_LEN, PresentationTime, SCTE35_SCHEME_PREFIX,
    STRING_TERMINATOR,
};
pub use error::{Error, Result};
pub use version::{EmsgVersion, VERSION_0, VERSION_1};
