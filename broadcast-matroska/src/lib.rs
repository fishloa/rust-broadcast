//! `broadcast-matroska` — EBML (RFC 8794) and Matroska/WebM (RFC 9559)
//! element syntax plus the sans-IO reader and writer over it: element framing,
//! the scalar payload codecs, the typed element model with byte-identical
//! parse/serialize round trips, a piece-at-a-time reader with Cues-driven
//! seeking, and a piece-at-a-time writer.
//!
//! This crate is deliberately below [`transmux`](https://docs.rs/transmux): it
//! knows element syntax only, never a media IR or a codec bitstream. Its one
//! workspace dependency is [`broadcast_common`], so a consumer that only needs
//! to read a Matroska file does not pull in the whole container-muxing hub.
//!
//! Every wire structure implements the same symmetric pair as the rest of the
//! workspace: [`Parse`](broadcast_core::Parse) borrows from the input, and
//! [`Serialize`](broadcast_core::Serialize) writes it back byte-identically.
//! A file read and rewritten through this crate comes back byte for byte: the
//! encoded width of every size and integer is preserved, field order is the
//! file's order, and an element the crate does not type stays
//! [`ebml::Unknown`] with its id and payload.
//!
//! # Layout
//!
//! - [`ebml`] — RFC 8794: [`ElementId`],
//!   [`VintSize`] (including the unknown size, preserving its
//!   encoded width), [`ElementHeader`], the size-driven
//!   [`ebml::ElementIter`] walker, the scalar payload codecs
//!   (unsigned/signed integer, float, string, UTF-8, date, binary), the EBML
//!   header, and the global `Void` and `CRC-32` elements.
//! - [`matroska`] — RFC 9559: [`ids`](matroska::ids) (every element id, its
//!   type and its parent), the typed element model rooted at
//!   [`matroska::Segment`], and the `SimpleBlock`/`Block`
//!   header with all four lacing forms.
//! - [`reader`] — the sans-IO reader: one Cluster header and one Block header
//!   at a time, `Step::Need`/`Step::supply`, Cues-driven seeking, and the
//!   permissive/unknown-size rules.
//! - [`writer`] — the sans-IO writer: header, tracks, Clusters as Blocks
//!   arrive, Cues and the `Info.Duration` patch, returned as [`writer::Write`]
//!   values.
//! - `driver` (feature `std`) — blocking [`driver::FileReader`] and
//!   [`driver::ForwardReader`] that answer the reader's demands themselves.
//! - [`limits`] — the hostile-input caps every count or depth read from a file
//!   is checked against before anything is allocated.
//!
//! # Spec citations
//!
//! - **EBML**: RFC 8794 (`private/specs-verified/ietf/specs/rfc8794_ebml.txt`),
//!   §4 (VINT), §5 (Element ID), §6 (Element Data Size), §7 (element types),
//!   §8 (document structure), §11.2 (the EBML header's elements), §11.3
//!   (global elements).
//! - **Matroska**: RFC 9559 (`private/specs-verified/ietf/specs/rfc9559_matroska.txt`),
//!   §5 (element definitions), §6 (Segment), §11 (timestamps), §12 (Block
//!   structure), and §27.1 (the element-ID registry). The demux-relevant
//!   subset is transcribed at
//!   `private/specs-verified/mpeg/transmux/webm/ebml-matroska.md`.
//!
//! # Examples
//!
//! ```text
//! cargo run -p broadcast-matroska --example dump_elements -- fixtures/mkv/vp9-opus.webm
//! cargo run -p broadcast-matroska --example list_tracks -- fixtures/mkv/vp9-opus.webm
//! cargo run -p broadcast-matroska --example list_samples -- fixtures/mkv/vp9-opus.webm
//! cargo run -p broadcast-matroska --example remux -- fixtures/mkv/vp9-opus.webm out.mkv
//! ```
//!
//! # Feature flags
//!
//! | Feature | Default | Description |
//! |---------|---------|-------------|
//! | `std`   | yes     | the blocking `driver` module; without it the crate is `no_std` + `alloc` |
//! | `serde` | no      | `serde::Serialize` on the element types |

#![cfg_attr(not(any(feature = "std", test)), no_std)]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
extern crate alloc;

#[cfg(feature = "std")]
pub mod driver;
pub mod ebml;
pub mod error;
pub mod limits;
pub mod matroska;
pub mod reader;
pub mod writer;

pub use ebml::{ElementHeader, ElementId, VintSize};
pub use error::{Error, Result};
pub use limits::Limits;
pub use matroska::Element;
