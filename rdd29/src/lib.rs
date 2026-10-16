//! SMPTE RDD 29:2019 Dolby Atmos bitstream: frame and element framing plus
//! bed/object rendering metadata, `no_std` + `alloc`.
//!
//! The wire structures follow the curated transcription at
//! `rdd29/docs/rdd29.md` (from `https://pub.smpte.org/pub/rdd29/rdd29-2019.pdf`);
//! cite that file, not this comment, as the field-semantics oracle.
//!
//! # What it does / what it does not
//!
//! RDD 29 is the disclosure document for the frame-based Dolby Atmos bitstream:
//! per frame, one `ATMOSFrame` element holding bed definitions, object
//! definitions and audio-data elements. This crate reads and builds that
//! framing and metadata. It does **not**:
//!
//! - decode audio: [`AudioDataDlc`]'s payload is the Dolby Lossless Coding
//!   (DLC) codec's own bitstream, kept as opaque bytes after the two
//!   byte-aligned fields `AudioDataID` and `DLCSize`;
//! - render or mix;
//! - cover any other Dolby Atmos carriage (RDD 29's cinema bitstream only);
//! - implement §3.3 Amplitude Gain (`A10`) coding: transcribed, but no
//!   implemented field uses it.
//!
//! # Standards supported
//!
//! | Standard | Edition | Scope / clauses | Status | Notes |
//! |---|---|---|---|---|
//! | SMPTE RDD 29, Dolby Atmos Bitstream Specification | 2019 (vendored; revision of RDD 29:2014) | §2-§5: `ATMOSFrame` (§4.2), `BedDefinition1` (§4.3), `ObjectDefinition1` (§4.4), `AudioDataDLC` (§4.5), Plex coding (§3.4), relative distance coding (§3.2), Table 1 element IDs | Partial | DLC payload opaque; §3.3 gain coding not implemented; reserved/unknown `ElementID`s round-trip as [`AnyElement::Unknown`] |
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
//! - [`AtmosFrame`] ([`ATMOS_VERSION`], [`SampleRate`], [`BitDepth`],
//!   [`FrameRate`]) holding a list of [`AnyElement`]s; [`ElementId`] with the
//!   Table 1 IDs.
//! - [`BedDefinition1`]: [`BedChannel`]s mapping a [`ChannelId`] (14 named
//!   channels plus `Reserved(u8)`) to an `AudioDataID`.
//! - [`ObjectDefinition1`]: [`PanSubBlock`]s ([`PanInfo`] position, snap,
//!   [`ZoneGain`] per [`ZoneId`], [`ObjectSpreadMode`], decorrelation via
//!   [`DecorCoefPrefix`]) and an optional [`AudioDescription`] text; the
//!   sub-block count is derived from [`FrameRate`].
//! - [`AudioDataDlc`]: `AudioDataID`, `DLCSize`, opaque payload.
//! - [`distance`]: [`distance::distance_xy`] / [`distance::distance_z`],
//!   read-only derived views of the §3.2 codes (the raw `u16` stays the
//!   round-tripped value; shifts and core `f64` only, identical under
//!   `no_std`).
//! - Plex coding ([`PLEX_MAX_VALUE`]); non-minimal escapes are rejected
//!   ([`Error::NonMinimalPlex`]).
//!
//! # Design notes
//!
//! - Symmetric `Parse`/`Serialize`; every reserved field is a hard-validated
//!   constant so a corrupt frame cannot re-serialize differently.
//! - Real-data caveat: RDD 29 has no capturable file format outside a licensed
//!   encoder, so the committed fixture (`tests/fixtures/eac3_frame0.bin`) is a
//!   real E-AC-3 syncframe used as an opaque `AudioDataDLC` payload inside
//!   framing built through this crate's own typed API; there is no real Atmos
//!   capture behind the tests.
//! - Public enums are `#[non_exhaustive]` with `name()` + `Display` labels.
//! - `no_std` + `alloc`; one `thiserror` enum, [`Error`]. MSRV 1.95.0; licence
//!   MIT OR Apache-2.0.
//!
//! # Related crates
//!
//! Depends only on `broadcast-common`. Shares the "container, not codec"
//! boundary with `st337` and `st377-1`.
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
//! let bytes = frame.try_to_bytes().unwrap();
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
