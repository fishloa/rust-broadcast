//! SMPTE ST 12-1:2014 Linear Time Code (LTC): parse and build the 80-bit
//! logical codeword (BCD time address, drop/color flags, binary groups, sync
//! word), `no_std`.
//!
//! The field semantics follow the curated transcription at
//! `st12-1/docs/st12-1.md` (from `https://pub.smpte.org/pub/st12-1/st0012-1-2014.pdf`);
//! cite that file, not this comment, as the oracle.
//!
//! # What it does / what it does not
//!
//! The crate takes the **already-demodulated 10-byte (80-bit) codeword** and
//! returns typed fields, and builds the same 10 bytes from typed fields. It
//! does **not** decode the section 9.3 biphase-mark audio waveform, implement
//! VITC (section 10) / sections 11-12, implement ST 12-2 (timecode in ANC) or
//! ST 12-3 (high frame rate), or detect the frame rate (the codeword has no
//! rate field; rate-dependent flag meanings are resolved against a
//! caller-supplied [`FrameRate`]).
//!
//! # Standards supported
//!
//! | Standard | Edition | Scope / clauses | Status | Notes |
//! |---|---|---|---|---|
//! | SMPTE ST 12-1, Time and Control Code | 2014 (vendored spec verified) | §8.1-8.5 numeric code, drop/color flags, Table 1 binary-group usage; §9.2 LTC codeword, Tables 2-4, Table 3 rate-dependent flags, sync word `0xFCBF` | Partial (LTC codeword only) | §9.3 waveform, §10 VITC, §11, §12 not implemented |
//! | SMPTE ST 12-2, Time Code in the ANC space | 2014 (vendored) | none | Not implemented | see the `st291` crate for the ANC container |
//! | SMPTE ST 12-3, High Frame Rate time code | 2016 (vendored) | none | Not implemented | |
//!
//! # Feature flags
//!
//! | Feature | Default | Description |
//! |---|---|---|
//! | `std` | yes | Link the standard library; without it the crate is `#![no_std]` (no allocation needed). |
//! | `serde` | no | `serde::Serialize`/`Deserialize` derives on public types. |
//!
//! # What is implemented
//!
//! - [`LtcFrame`]: hours, minutes, seconds, frames (BCD), drop-frame (bit 10)
//!   and color-frame (bit 11) flags, the four rate-dependent flag bits at
//!   their wire positions, eight 4-bit binary groups (`user_bits`), and the
//!   fixed [`SYNC_WORD`] (validated on parse, always written on serialize);
//!   [`FRAME_LEN`] is 10.
//! - [`FrameRate`] (30/25/24-frame): selects Table 3's column;
//!   [`LtcFrame::polarity_correction`] and [`LtcFrame::binary_group_flags`]
//!   resolve the flag bits; [`FrameRate::max_frame`] and
//!   [`LtcFrame::validate_for_rate`] give the rate-aware frame bound.
//! - [`BinaryGroupFlags`] / [`BinaryGroupUsage`]: Table 1's eight-way
//!   classification of the binary-group contents.
//!
//! # Design notes
//!
//! - `Parse`/`Serialize` are symmetric with byte-exact round-trips, tested
//!   against spec vectors.
//! - Parsing is strict: BCD digits are range-checked
//!   ([`Error::InvalidBcdDigit`]), field maxima enforced
//!   ([`Error::InvalidValue`]) and the sync word must match
//!   ([`Error::SyncWordMismatch`]), so a malformed codeword cannot
//!   re-serialize to different bytes.
//! - `parse` validates frames against the widest bound ([`MAX_FRAMES`]); the
//!   tighter per-rate check is [`LtcFrame::validate_for_rate`].
//! - Public enums are `#[non_exhaustive]` with `name()` + `Display` labels.
//!   MSRV 1.95.0; licence MIT OR Apache-2.0.
//!
//! # Related crates
//!
//! Depends only on `broadcast-common`. Timecode inside ANC packets would sit
//! on top of the `st291` crate.
//!
//! # Examples
//!
//! Build a frame and round-trip it:
//!
//! ```
//! use broadcast_common::{Parse, Serialize};
//! use st12_1::LtcFrame;
//!
//! let frame = LtcFrame {
//!     hours: 1,
//!     minutes: 23,
//!     seconds: 45,
//!     frames: 13,
//!     drop_frame_flag: false,
//!     color_frame_flag: true,
//!     flag_bit_27: true,
//!     flag_bit_43: true,
//!     flag_bit_58: false,
//!     flag_bit_59: true,
//!     user_bits: [1, 2, 3, 4, 5, 6, 7, 8],
//! };
//! let mut bytes = [0u8; st12_1::FRAME_LEN];
//! frame.serialize_into(&mut bytes).unwrap();
//! assert_eq!(LtcFrame::parse(&bytes).unwrap(), frame);
//! ```
#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
// Runnable examples, embedded so they render on docs.rs and stay in sync with
// the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n## Runnable examples\n"]
#![doc = "Run with `cargo run -p st12-1 --example <name>`.\n"]
#![doc = "\n### `build_frame`\n\n```rust,ignore"]
#![doc = include_str!("../examples/build_frame.rs")]
#![doc = "```\n\n### `parse_frame`\n\n```rust,ignore"]
#![doc = include_str!("../examples/parse_frame.rs")]
#![doc = "```"]

mod error;
mod frame;

pub use error::{Error, Result};
pub use frame::{
    BinaryGroupFlags, BinaryGroupUsage, FRAME_LEN, FrameRate, LtcFrame, MAX_BINARY_GROUP,
    MAX_FRAMES, MAX_HOURS, MAX_MINUTES_SECONDS, SYNC_WORD,
};
