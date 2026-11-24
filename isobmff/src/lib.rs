//! `isobmff` — the ISO/IEC 14496-12:2026 (ISO Base Media File Format) box
//! layer: `size`/`type` box headers (including the 64-bit `largesize` and the
//! `uuid` extended type), the top-level `ftyp`/`styp`/`mdat` set, the
//! sample-table and movie-fragment boxes, the shared CENC box syntax
//! (ISO/IEC 23001-7), and the DASH Event Message Box (`emsg`, ISO/IEC 23009-1).
//!
//! This crate is deliberately below [`transmux`](https://docs.rs/transmux):
//! it knows box syntax only, never the media IR or a codec bitstream. Its one
//! workspace dependency is [`broadcast_common`], so a parser that only needs
//! to walk MP4 boxes — a validator, a segmenter, a probe — no longer pulls in
//! the whole container-muxing hub.
//!
//! Every wire structure implements the same symmetric pair as the rest of the
//! workspace: [`Parse`](broadcast_common::Parse) borrows from the input, and
//! [`Serialize`](broadcast_common::Serialize) writes it back byte-identically.
//!
//! # Layout
//!
//! - [`box_types`] — box / FullBox headers, [`BoxType`],
//!   the size-driven [`box_iter`] walker, the header
//!   writers, and the opaque round-trip carriers.
//! - [`box_support`] — the shared flat-table and container-child helpers every
//!   box module builds on.
//! - [`segments`], [`timing`], [`visual_ext`], [`cenc`], [`sample_groups`],
//!   [`movie_fragment`], [`frag_offsets`] — the box set and the §8.8.7/§8.8.8
//!   fragment sample-offset resolver.
//! - [`emsg`] — the MPEG-DASH Event Message Box (ISO/IEC 23009-1 §5.10.3.3,
//!   version 0 and 1) and its version enum.
//!
//! # Spec citations
//!
//! - **Box / FullBox**: ISO/IEC 14496-12:2026 §4.2.
//! - **Box order**: §7.3.4.
//! - **Skip-unknown rule**: §4.2 — unknown type → ignore + skip via `size`.
//! - **CENC boxes**: ISO/IEC 23001-7 §12.
//! - **`emsg`**: ISO/IEC 23009-1 §5.10.3.3; DASH-IF IOP Part 10 §6.1.
//!
//! # Feature flags
//!
//! | Feature | Default | Description |
//! |---------|---------|-------------|
//! | `std`   | yes     | `std` glue; without it the crate is `no_std` + `alloc` |
//! | `serde` | no      | `serde::Serialize` on the box types |

#![cfg_attr(not(any(feature = "std", test)), no_std)]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
extern crate alloc;

pub mod box_support;
pub mod box_types;
pub mod cenc;
pub mod emsg;
pub mod error;
pub mod frag_offsets;
pub mod movie_fragment;
pub mod sample_groups;
pub mod segments;
pub mod timing;
pub mod track_id;
pub mod visual_ext;

pub use box_types::{BoxHeader, BoxIter, BoxRef, BoxType, FullBoxHeader, box_iter, parse_box};
pub use error::{Error, Result};
pub use track_id::TrackId;

/// Serialize a [`Timescale`](broadcast_common::timescale::Timescale) as the
/// plain ticks-per-second integer it wraps, so a serde-JSON consumer sees the
/// same number the wire box carries (not a `NonZeroU32` newtype object).
#[cfg(feature = "serde")]
pub fn ser_timescale<S: serde::Serializer>(
    timescale: &broadcast_common::timescale::Timescale,
    serializer: S,
) -> core::result::Result<S::Ok, S::Error> {
    serializer.serialize_u32(timescale.get())
}
