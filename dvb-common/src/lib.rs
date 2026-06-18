//! Shared primitives for the dvb_si / dvb_t2mi / dvb_bbframe family.
//!
//! See individual modules for documentation: the [`Parse`] / [`Serialize`]
//! traits every wire type implements, the MPEG-2 [`crc32_mpeg2`] CRC, and the
//! [`bcd`] / [`time`] codecs.
//!
//! # Quick start
//! ```
//! use dvb_common::{bcd, crc32_mpeg2};
//!
//! // Binary-coded decimal (as used in MJD/BCD time fields):
//! assert_eq!(bcd::from_bcd_byte(0x42), Some(42));
//! assert_eq!(bcd::to_bcd_byte(42), Some(0x42));
//!
//! // MPEG-2 CRC-32 over a section body (deterministic):
//! let crc = crc32_mpeg2::compute(&[0xDE, 0xAD, 0xBE, 0xEF]);
//! assert_eq!(crc, crc32_mpeg2::compute(&[0xDE, 0xAD, 0xBE, 0xEF]));
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(not(feature = "std"), no_std)]
// The crate's runnable examples, embedded so they render on docs.rs and stay in
// sync with the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n# Examples\n"]
#![doc = "Two runnable examples ship with this crate (`cargo run -p dvb-common --example <name>`).\n"]
#![doc = "\n## `crc_and_bcd`\n\n```rust,ignore"]
#![doc = include_str!("../examples/crc_and_bcd.rs")]
#![doc = "```\n\n## `implement_parse_serialize`\n\n```rust,ignore"]
#![doc = include_str!("../examples/implement_parse_serialize.rs")]
#![doc = "```"]

extern crate alloc;

pub mod bcd;
pub mod bits;
pub mod crc32_mpeg2;
pub mod time;
pub mod traits;

pub use traits::{Parse, Serialize};

/// Generate a [`core::fmt::Display`] impl for a spec/field enum that delegates
/// to an inherent `fn name(&self) -> &'static str`.
///
/// This is the project-wide convention for every public spec/field enum across
/// the `dvb-*` crates (see issue #204): `name()` is the hand-written,
/// zero-alloc static spec token (lossy on the reserved/unknown arm, which
/// returns `"reserved"`), and `Display` is the lossless, composable view that
/// delegates to it. The labels themselves live in `name()` in source — never in
/// this macro — so they sit next to the variant docs and stay greppable. This
/// macro carries no labels; it only removes the otherwise-identical `Display`
/// boilerplate and keeps the two in lockstep.
///
/// # Forms
/// - `impl_spec_display!(Ty)` — every variant's `Display` is exactly `name()`.
///   Use when there is no byte-bearing catch-all (or its byte need not be
///   shown), e.g. a unit `Reserved` variant.
/// - `impl_spec_display!(Ty, Var1, Var2, …)` — each named variant is a
///   single-field tuple binding a byte; `Display` renders it as
///   `"{name}(0x{:02X})"` so the value is preserved (e.g. `Reserved(0x1A)` →
///   `reserved(0x1A)`, `UserDefined(0x1A)` → `user defined(0x1A)`). All other
///   variants delegate to `name()`.
///
/// ```
/// pub enum Mode { Normal, HighEfficiency, Reserved(u8) }
/// impl Mode {
///     pub fn name(&self) -> &'static str {
///         match self {
///             Self::Normal => "normal",
///             Self::HighEfficiency => "high efficiency",
///             Self::Reserved(_) => "reserved",
///         }
///     }
/// }
/// dvb_common::impl_spec_display!(Mode, Reserved);
/// assert_eq!(Mode::Normal.to_string(), "normal");
/// assert_eq!(Mode::Reserved(0x1A).to_string(), "reserved(0x1A)");
/// ```
#[macro_export]
macro_rules! impl_spec_display {
    ($ty:ty) => {
        impl ::core::fmt::Display for $ty {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                f.write_str(self.name())
            }
        }
    };
    ($ty:ty, $($resv:ident),+ $(,)?) => {
        impl ::core::fmt::Display for $ty {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                match self {
                    $( Self::$resv(v) => ::core::write!(f, "{}(0x{:02X})", self.name(), v), )+
                    other => f.write_str(other.name()),
                }
            }
        }
    };
}
