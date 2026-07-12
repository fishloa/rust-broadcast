//! SMPTE ST 12-1:2014 "Time and Control Code" — the §9 Linear Time Code
//! (LTC) 80-bit logical codeword.
//!
//! This crate implements exactly the wire structure described in the curated
//! spec transcription at `st12-1/docs/st12-1.md` (fetched directly from
//! `https://pub.smpte.org/pub/st12-1/st0012-1-2014.pdf`) — cite that file,
//! not this doc comment, as the field-semantics oracle.
//!
//! - [`LtcFrame`] — the 80-bit LTC codeword (§9.2): BCD hours/minutes/
//!   seconds/frames, the drop-frame and color-frame flags, four
//!   rate-dependent flag bits (resolved via [`FrameRate`]), eight 4-bit
//!   binary groups ("user bits"), and the fixed synchronization word.
//! - [`FrameRate`] — which of ST 12-1 Table 3's three flag-bit-position
//!   columns (30-frame / 25-frame / 24-frame) applies; the codeword itself
//!   carries no self-describing frame-rate field.
//! - [`BinaryGroupUsage`] / [`BinaryGroupFlags`] — Table 1's classification
//!   of what the binary groups contain, from the three binary group flag
//!   bits.
//!
//! **Scope**: this crate models only the already-demodulated logical 80-bit
//! codeword — never the §9.3 biphase-mark-encoded physical/analog audio
//! waveform LTC is carried as on a wire. That line-encoding/clock-recovery
//! layer is out of scope for this project, the same way it never decodes PCM
//! or AC-3 audio samples. See `docs/st12-1.md`'s "Scope" section.
//!
//! Depends only on `broadcast-common`. `#![no_std]` when the `std` feature
//! is disabled (this crate needs no heap allocation at all — every field is
//! a fixed-size scalar).
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
