//! `broadcast-matroska` — EBML (RFC 8794) and Matroska/WebM (RFC 9559)
//! element syntax: element framing, the scalar payload codecs, and the typed
//! element model with byte-identical parse/serialize round trips.
//!
//! This crate is deliberately below [`transmux`](https://docs.rs/transmux): it
//! knows element syntax only, never a media IR or a codec bitstream. Its one
//! workspace dependency is [`broadcast_common`], so a consumer that only needs
//! to read a Matroska file does not pull in the whole container-muxing hub.
//!
//! Every wire structure implements the same symmetric pair as the rest of the
//! workspace: [`Parse`](broadcast_common::Parse) borrows from the input, and
//! [`Serialize`](broadcast_common::Serialize) writes it back byte-identically.
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
//! # Feature flags
//!
//! | Feature | Default | Description |
//! |---------|---------|-------------|
//! | `std`   | yes     | `std` glue; without it the crate is `no_std` + `alloc` |
//! | `serde` | no      | `serde::Serialize` on the element types |

#![cfg_attr(not(any(feature = "std", test)), no_std)]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
extern crate alloc;

pub mod ebml;
pub mod error;
pub mod limits;
pub mod matroska;

pub use ebml::{ElementHeader, ElementId, VintSize};
pub use error::{Error, Result};
pub use limits::Limits;
pub use matroska::Element;
