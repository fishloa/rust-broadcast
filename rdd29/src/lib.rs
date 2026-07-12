//! SMPTE RDD 29:2019 Dolby Atmos® bitstream — frame/element framing plus
//! bed/object metadata.
//!
//! This crate implements exactly the wire structures described in the
//! curated spec transcription at `rdd29/docs/rdd29.md` (fetched directly
//! from `https://pub.smpte.org/pub/rdd29/rdd29-2019.pdf`) — cite that file,
//! not this doc comment, as the field-semantics oracle.
//!
//! - [`AtmosFrame`] — one complete frame: the top-level `ATMOSFrame` element
//!   (§2.1/§4.2), containing zero or more sub-[`AnyElement`]s.
//! - [`BedDefinition1`] — a channel-based audio bed's channel-to-audio-asset
//!   mapping (§2.2/§4.3).
//! - [`ObjectDefinition1`] — one panned audio object's per-sub-block pan/
//!   rendering metadata: 3D position, snap, zone gain, spread, decorrelation,
//!   plus an optional text description (§2.3/§4.4).
//! - [`AudioDataDlc`] — one track's audio essence pointer + opaque payload
//!   (§2.4/§4.5).
//!
//! **What this crate is not**: an audio codec. `AudioDataDlc`'s payload is
//! the Dolby Lossless Coding (DLC) codec's own bit-packed bitstream (linear-
//! predictive + Rice-Golomb entropy-coded residual audio samples) — this
//! crate treats it as opaque bytes, the same "parse the container, not the
//! codec" discipline this workspace's `transmux`/`st337` crates use for
//! media containers and AES3 non-PCM bursts respectively. See
//! `docs/rdd29.md`'s "Scope decisions" for the exact boundary and the two
//! honest gaps in the source disclosure document this crate had to resolve
//! (the `Plex(8)` pseudocode's internal inconsistency, and the completely
//! undocumented `AudioDescription` field semantics).
//!
//! Depends only on `broadcast-common`. `#![no_std]` (+ `alloc`) when the
//! `std` feature is disabled.
//!
//! # Examples
//!
//! Build a frame with a bed and an audio-essence element, and round-trip it:
//!
//! ```
//! use broadcast_common::{Parse, Serialize};
//! use rdd29::{
//!     AtmosFrame, AudioDataDlc, BedChannel, BedDefinition1, BitDepth, ChannelId, FrameRate,
//!     SampleRate,
//! };
//!
//! let bed = BedDefinition1::new(
//!     1,
//!     vec![BedChannel {
//!         channel_id: ChannelId::LeftScreen,
//!         audio_data_id: 10,
//!     }],
//! );
//! let dlc = AudioDataDlc::new(10, &[0xDE, 0xAD, 0xBE, 0xEF]).unwrap();
//!
//! let frame = AtmosFrame::new(
//!     SampleRate::Hz48000,
//!     BitDepth::Bits24,
//!     FrameRate::Fps24,
//!     1,
//!     vec![
//!         rdd29::AnyElement::BedDefinition1(bed),
//!         rdd29::AnyElement::AudioDataDlc(dlc),
//!     ],
//! );
//!
//! let bytes = frame.to_bytes();
//! assert_eq!(AtmosFrame::parse(&bytes).unwrap(), frame);
//! ```
#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
// Runnable examples, embedded so they render on docs.rs and stay in sync with
// the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n## Runnable examples\n"]
#![doc = "Run with `cargo run -p rdd29 --example <name>`.\n"]
#![doc = "\n### `build_atmos_frame`\n\n```rust,ignore"]
#![doc = include_str!("../examples/build_atmos_frame.rs")]
#![doc = "```\n\n### `parse_atmos_frame`\n\n```rust,ignore"]
#![doc = include_str!("../examples/parse_atmos_frame.rs")]
#![doc = "```"]

extern crate alloc;

mod atmos_frame;
mod audio_data_dlc;
mod bed_definition;
pub mod distance;
mod element;
mod error;
mod frame_rate;
mod object_definition;
mod plex;
mod util;

pub use atmos_frame::{ATMOS_VERSION, AtmosFrame, BitDepth, SampleRate};
pub use audio_data_dlc::AudioDataDlc;
pub use bed_definition::{BedChannel, BedDefinition1, ChannelId};
pub use element::{
    AnyElement, ELEMENT_ID_ATMOS_FRAME, ELEMENT_ID_AUDIO_DATA_DLC, ELEMENT_ID_BED_DEFINITION1,
    ELEMENT_ID_OBJECT_DEFINITION1, ElementId,
};
pub use error::{Error, Result};
pub use frame_rate::FrameRate;
pub use object_definition::{
    AudioDescription, DecorCoefPrefix, MAX_KNOWN_ZONES, ObjectDefinition1, ObjectSpreadMode,
    PanInfo, PanSubBlock, ZoneGain, ZoneId,
};
pub use plex::PLEX_MAX_VALUE;
