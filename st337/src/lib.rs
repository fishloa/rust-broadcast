//! SMPTE ST 337:2015 non-PCM audio/data burst-preamble framing over AES3.
//!
//! This crate implements exactly the wire structures described in the
//! curated spec transcription at `st337/docs/st337.md` (fetched directly
//! from `https://pub.smpte.org/latest/st337/st0337-2015.pdf`) — cite that
//! file, not this doc comment, as the field-semantics oracle. See also
//! `st337/docs/st337-PROVENANCE.md` for a real-fixture / independent-oracle
//! (`ffmpeg -f spdif`) cross-check of the constants and bit layout below.
//!
//! - [`Burst`] — one complete non-PCM data burst: [`BurstPreamble`] (`Pa`..
//!   `Pd`, or `Pa`..`Pf` for the "extended" six-word form) followed by the
//!   opaque `burst_payload` bytes (§7.1/§7.2).
//! - [`DataMode`] — the `data_mode` field (§7.2.4.3 Table 8): this crate
//!   supports only [`DataMode::Mode16`] for parsing/building (see the
//!   "Scope decisions" section of `docs/st337.md`).
//!
//! **What this crate is not**: an AES3 physical-layer (biphase-mark line
//! code, subframe/timeslot bit placement) codec. It parses/builds the
//! *logical* burst-preamble/burst-payload word sequence as a plain byte
//! stream (`&[u8]`, 2 bytes per 16-bit preamble word) — the same "parse the
//! container, not the physical/codec layer" discipline this workspace's
//! `transmux` crate uses for media containers. It also does not define a
//! `data_type` -> codec enum: that mapping is registered in the companion
//! spec SMPTE ST 338, which was not available to verify truthfully (see
//! `docs/st337.md`).
//!
//! Depends only on `broadcast-common`. `#![no_std]` (+ `alloc`) when the
//! `std` feature is disabled.
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
