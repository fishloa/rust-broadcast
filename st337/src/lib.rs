//! SMPTE ST 337:2015 burst-preamble framing for non-PCM audio and data carried
//! in an AES3 serial digital audio interface: typed parse/serialize of the
//! `Pa`/`Pb`/`Pc`/`Pd` (and extended `Pe`/`Pf`) preamble plus the opaque
//! payload, `no_std` + `alloc`.
//!
//! The field semantics follow the curated transcription at
//! `st337/docs/st337.md` (from `https://pub.smpte.org/latest/st337/st0337-2015.pdf`);
//! see also `st337/docs/st337-PROVENANCE.md` for a real-fixture,
//! independent-oracle (`ffmpeg -f spdif`) cross-check of the constants and
//! bit layout.
//!
//! # What it does / what it does not
//!
//! ST 337 defines how compressed or non-PCM data (for example AC-3, E-AC-3 or
//! DTS audio) rides inside the sample words of an AES3 stream: a repeating
//! **data burst** of a preamble plus a payload. This crate reads and builds
//! those bursts from the word sequence as bytes. It does **not**:
//!
//! - implement the AES3 physical layer (biphase-mark coding, subframe/time-slot
//!   placement, channel-status words);
//! - map `data_type` to a codec (that registry is SMPTE ST 338, not vendored),
//!   so `data_type` is a validated `u8` (0 to 31);
//! - support the 20-bit/24-bit `data_mode` values: [`DataMode::Mode20`] and
//!   [`DataMode::Mode24`] parse as enum values but [`BurstPreamble`] rejects
//!   them with [`Error::UnsupportedDataMode`];
//! - decode or re-encode the payload (it stays opaque bytes);
//! - implement IEC 61937: `length_code` is read as **bits** per ST 337's own
//!   text, whereas a real IEC 61937 E-AC-3 capture writes bytes for the same
//!   data type.
//!
//! # Standards supported
//!
//! | Standard | Edition | Scope / clauses | Status | Notes |
//! |---|---|---|---|---|
//! | SMPTE ST 337, Non-PCM Audio and Data in an AES3 Serial Digital Audio Interface | 2015 (vendored; revision of 337-2008) | §7.1/§7.2 burst and preamble: `Pa`/`Pb` sync, `Pc` fields, `Pd` length code, extended-type `Pe`/`Pf` (§7.2.4), Table 8 `data_mode` | Partial | 16-bit mode only; words as little-endian 16-bit values in a byte stream |
//! | SMPTE ST 338, Non-PCM data_type registry | not vendored | none | Not implemented | `data_type` is an opaque validated number |
//! | AES3-3 / IEC 61937 | n/a | none | Not implemented | sync constants and `Pc` layout cross-checked against a real E-AC-3 capture |
//!
//! # Feature flags
//!
//! | Feature | Default | Description |
//! |---|---|---|
//! | `std` | yes | Link the standard library; without it the crate is `#![no_std]` + `alloc`. |
//! | `serde` | no | `serde::Serialize`/`Deserialize` derives on public types. |
//!
//! # What is implemented
//!
//! - [`Burst`]: a [`BurstPreamble`] plus the opaque payload slice (borrowed on
//!   parse); [`Burst::new`] computes `length_code` for you.
//! - [`BurstPreamble`]: `data_type`, `data_mode`, `error_flag`,
//!   `data_type_dependent`, `data_stream_number`, `length_code`, and an
//!   optional [`ExtendedPreamble`] (`Pe`/`Pf`) used when `data_type` equals
//!   [`EXTENDED_DATA_TYPE_MARKER`].
//! - [`DataMode`] (Table 8) and the constants [`SYNC_WORD_PA`],
//!   [`SYNC_WORD_PB`], [`MAX_5_BIT_FIELD`], [`MAX_DATA_STREAM_NUMBER`],
//!   [`MAX_LENGTH_CODE_BITS`].
//! - A committed real E-AC-3 burst (`tests/fixtures/eac3_frame0.bin`)
//!   round-trips byte-exact.
//!
//! # Design notes
//!
//! - Symmetric `Parse`/`Serialize` with round-trip tests; the preamble is
//!   validated ([`Error::InvalidSync`], [`Error::InvalidValue`],
//!   [`Error::ExtendedPreambleMismatch`], [`Error::PayloadTooLarge`]) so a
//!   corrupt burst cannot re-serialize differently.
//! - Public enums are `#[non_exhaustive]` with `name()` + `Display` labels.
//! - `no_std` + `alloc`; one `thiserror` error enum. MSRV 1.95.0; licence
//!   MIT OR Apache-2.0.
//!
//! # Related crates
//!
//! Depends only on `broadcast-common`. The payloads it carries are compressed
//! audio or data that this crate never parses.
//!
//! # Examples
//!
//! Build a burst from a payload and round-trip it:
//!
//! ```
//! use broadcast_common::{Parse, Serialize};
//! use st337::{Burst, DataMode};
//!
//! let payload = [0xDE, 0xAD, 0xBE, 0xEF];
//! let burst = Burst::new(1, DataMode::Mode16, false, 0, 0, None, &payload).unwrap();
//!
//! let mut bytes = vec![0u8; burst.serialized_len()];
//! burst.serialize_into(&mut bytes).unwrap();
//! assert_eq!(Burst::parse(&bytes).unwrap(), burst);
//! ```
#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
// Runnable examples, embedded so they render on docs.rs and stay in sync with
// the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n## Runnable examples\n"]
#![doc = "Run with `cargo run -p st337 --example <name>`.\n"]
#![doc = "\n### `build_burst`\n\n```rust,ignore"]
#![doc = include_str!("../examples/build_burst.rs")]
#![doc = "```\n\n### `parse_burst`\n\n```rust,ignore"]
#![doc = include_str!("../examples/parse_burst.rs")]
#![doc = "```"]

extern crate alloc;

mod burst;
mod error;

pub use burst::{
    Burst, BurstPreamble, DataMode, EXTENDED_DATA_TYPE_MARKER, ExtendedPreamble, MAX_5_BIT_FIELD,
    MAX_DATA_STREAM_NUMBER, MAX_LENGTH_CODE_BITS, SYNC_WORD_PA, SYNC_WORD_PB,
};
pub use error::{Error, Result};
