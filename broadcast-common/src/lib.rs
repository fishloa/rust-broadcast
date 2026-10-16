#![doc = include_str!("../README.md")]
#![doc = "\n"]
//! # Reserved-bit and trailing-byte policy
//!
//! One workspace-wide rule for every wire crate (issue #1139):
//!
//! - **Parse is tolerant.** A non-conformant reserved bit, or trailing data
//!   that real senders emit, is accepted rather than rejected (a crate may
//!   choose stricter validation, as `dvb-t2mi` does for RFU bits, but must
//!   document it).
//! - **Serialize preserves what was parsed.** Reserved bits are captured on
//!   parse in typed, spec-named fields and re-emitted verbatim, so a stream
//!   from a sender that deviates from the governing edition re-serializes
//!   **byte-identically** (`dvb-si` descriptors, `mpeg-ps`, `rmt-flute`,
//!   `cc-data` fixed bits, `rtcp-packet` `profile_extensions`, `mp4-emsg`
//!   flags). There is no raw passthrough blob.
//! - **Constructed values default to the edition value**: a struct built by
//!   hand (`new(..)` / `Default`) carries `1` for `reserved_future_use` and
//!   H.222.0 "reserved", `0` for `reserved_zero_future_use`, with any explicit
//!   per-field exception documented on the field. (A crate that has not
//!   adopted preserve-on-parse documents which fields it normalizes to the
//!   spec value on serialize; `mpeg-ts`, `mpeg-pes` and `mpeg-ps` preserve.)
//! - A caller that needs byte-for-byte preservation of reserved bits in a
//!   not-yet-migrated crate **keeps the original bytes** rather than
//!   re-serializing.
//! - Trailing bytes inside a descriptor or section body are never silently
//!   dropped without a documented decision: hold them in a typed field, or
//!   document the field as "ignored".
//! - Reserved bytes are always **written explicitly** by `serialize_into`;
//!   never rely on a pre-zeroed output buffer.
//!
//! The round-trip gate is byte-exact against the original fixture slice
//! (see `docs/CRATE-ACCEPTANCE.md`).

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(not(feature = "std"), no_std)]
// The crate's runnable examples, embedded so they render on docs.rs and stay in
// sync with the actual `examples/*.rs` files (shown, not compiled).
#![doc = "\n# Examples\n"]
#![doc = "Two runnable examples ship with this crate (`cargo run -p broadcast-common --example <name>`).\n"]
#![doc = "\n## `crc_and_bcd`\n\n```rust,ignore"]
#![doc = include_str!("../examples/crc_and_bcd.rs")]
#![doc = "```\n\n## `implement_parse_serialize`\n\n```rust,ignore"]
#![doc = include_str!("../examples/implement_parse_serialize.rs")]
#![doc = "```"]

extern crate alloc;

pub mod bcd;
pub mod bits;
pub mod cenc;
pub mod clock33;
pub mod crc32_mpeg2;
pub mod hex;
pub mod len;
pub mod mux;
pub mod parity;
pub mod pusi;
pub mod seq;
pub mod stage;
pub mod time;
pub mod traits;
pub mod ts_dup;

pub use cenc::CencScheme;
pub use mux::{Decrypt, Encrypt, Package, Unpackage};
pub use stage::{Demand, Stage, Timestamp};
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
/// broadcast_common::impl_spec_display!(Mode, Reserved);
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
