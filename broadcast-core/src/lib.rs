//! The frozen wire-format contract shared by every rust-broadcast crate.
//!
//! This crate holds only settled, low-level pieces that do not change as the
//! DVB/MPEG format crates grow: the symmetric [`Parse`] / [`Serialize`]
//! traits, checked field-width helpers ([`len`]), MSB-first bit I/O
//! ([`bits`]), the shared error kinds ([`error`]), typed reserved-bit fields
//! ([`reserved`]), the media [`timescale`](mod@timescale) codec, the
//! container-independent CENC scheme identity ([`cenc`]) and the big-endian
//! wire field readers ([`wire`]). Everything here is
//! `#![no_std]` + `alloc` and depends on nothing but an optional `serde` derive.
//!
//! The API of this crate is **meant to be frozen**: it is the stable floor
//! that the wire-format crates build on, so they can add formats without a
//! breaking change here rippling up through all of them. Additions are still
//! possible (the crate reaches `1.0` before it is published), but a change to
//! a name or signature already here is a deliberate, reviewed event — the
//! `core_api_pin` workspace guard fails on any un-reviewed addition or
//! removal.
//!
//! This crate used to be part of `broadcast-common`; the modules now live here
//! and `broadcast-common` re-exports nothing from them.

#![no_std]
#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]

extern crate alloc;

pub mod bits;
pub mod cenc;
pub mod damage;
pub mod error;
pub mod len;
pub mod reserved;
pub mod timescale;
pub mod traits;
pub mod wire;

pub use cenc::CencScheme;
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
/// broadcast_core::impl_spec_display!(Mode, Reserved);
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
