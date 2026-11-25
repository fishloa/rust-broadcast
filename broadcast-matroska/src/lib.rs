//! `broadcast-matroska` — EBML (RFC 8794) and Matroska/WebM (RFC 9559)
//! element syntax: element framing, the scalar payload codecs, and the typed
//! element model with byte-identical parse/serialize round trips.
//!
//! # Spec citations
//!
//! - **EBML**: RFC 8794 (`private/specs-verified/ietf/specs/rfc8794_ebml.txt`),
//!   §4 (VINT), §5 (Element ID), §6 (Element Data Size), §7 (element types),
//!   §8 (document structure), §11.2 (the EBML header's elements), §11.3
//!   (global elements).

#![cfg_attr(not(any(feature = "std", test)), no_std)]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
extern crate alloc;

pub mod ebml;
pub mod error;
pub mod limits;

pub use ebml::{ElementHeader, ElementId, VintSize};
pub use error::{Error, Result};
pub use limits::Limits;
