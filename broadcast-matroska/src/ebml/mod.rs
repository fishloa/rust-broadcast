//! EBML element framing and scalar codecs (RFC 8794).
//!
//! - [`vint`] — the Variable-Size Integers an element ID and an element size
//!   are made of (§4–§6).
//! - [`header`] — [`ElementHeader`] and the size-driven [`ElementIter`]
//!   walker.
//! - [`scalar`] — the payload codecs (§7).
//! - [`leaf`] — one element carrying one typed scalar.
//! - [`global`] — the `Void` and `CRC-32` elements every document may carry
//!   (§11.3).
//! - [`document`] — the EBML header (§8.1, §11.2).

pub mod document;
pub mod global;
pub mod header;
pub mod leaf;
pub mod scalar;
pub mod vint;

pub use document::EbmlHeader;
pub use header::{ElementHeader, ElementIter, RawElement};
pub use leaf::{Unknown, UnsignedElement};
pub use vint::{ElementId, VintSize, VintWidth};
