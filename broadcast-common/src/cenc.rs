//! Common Encryption (CENC) scheme identity — ISO/IEC 23001-7 §4.
//!
//! [`CencScheme`] names the protection scheme a protected track is encrypted
//! under: the `schm.scheme_type` four-CC of ISO/IEC 14496-12:2015 §8.12.5.
//!
//! # Why this lives in `broadcast-common`
//!
//! CENC is *Common* Encryption — a container-independent scheme identity, not
//! an ISOBMFF or an HLS concept. It is referenced from at least three places
//! that must agree on one type:
//!
//! - the **decrypt** path, which recovers it from a protected file's `schm`;
//! - the **encrypt** path, which is given it by the caller;
//! - **manifest/playlist signalling** (DASH `ContentProtection`, HLS
//!   `#EXT-X-KEY`), which renders a different tag per scheme.
//!
//! Those live in different crates (`transmux` owns the ISOBMFF crypto boxes,
//! `broadcast-hls` owns the M3U8 playlist syntax), so the scheme identity
//! belongs *below* both — here, alongside the [`Encrypt`](crate::Encrypt) /
//! [`Decrypt`](crate::Decrypt) traits it parameterises. Defining it once here
//! is what stops "two names for one thing" reappearing every time a new
//! container crate needs to name a scheme (issue #564 consolidated three
//! definitions into one; issue #878 keeps it that way across the
//! `transmux`/`broadcast-hls` split).
//!
//! The *box layouts* that carry this scheme (`tenc`, `senc`, `sinf`, `schm`,
//! …) are ISOBMFF-specific and stay in `transmux`.

/// Four-CC identifying the `cenc` (AES-128 full-block counter, CTR)
/// protection scheme, as it appears in `schm.scheme_type`.
pub const SCHEME_CENC: [u8; 4] = *b"cenc";

/// Four-CC identifying the `cbcs` (AES-128 pattern cipher-block-chaining)
/// protection scheme, as it appears in `schm.scheme_type`.
pub const SCHEME_CBCS: [u8; 4] = *b"cbcs";

/// A CENC protection scheme (`schm.scheme_type`) — ISO/IEC 23001-7 §4.
///
/// `#[non_exhaustive]`: ISO/IEC 23001-7 also defines `cens` and `cbc1`, which
/// this workspace does not implement yet; adding one must stay additive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum CencScheme {
    /// `cenc` — AES-128 full-block counter (CTR) mode.
    Cenc,
    /// `cbcs` — AES-128 pattern cipher-block-chaining mode.
    Cbcs,
}

impl CencScheme {
    /// The scheme's four-CC token as it appears in `schm` (`"cenc"` / `"cbcs"`).
    pub fn name(&self) -> &'static str {
        match self {
            CencScheme::Cenc => "cenc",
            CencScheme::Cbcs => "cbcs",
        }
    }

    /// The scheme's `schm.scheme_type` four-CC bytes — the inverse of
    /// [`CencScheme::from_four_cc`].
    pub fn to_four_cc(&self) -> [u8; 4] {
        match self {
            CencScheme::Cenc => SCHEME_CENC,
            CencScheme::Cbcs => SCHEME_CBCS,
        }
    }

    /// Map a `schm.scheme_type` four-CC to a known scheme, or `None` if it is
    /// one this workspace does not implement (e.g. `cens`/`cbc1`).
    pub fn from_four_cc(four_cc: &[u8; 4]) -> Option<Self> {
        match *four_cc {
            SCHEME_CENC => Some(CencScheme::Cenc),
            SCHEME_CBCS => Some(CencScheme::Cbcs),
            _ => None,
        }
    }
}

crate::impl_spec_display!(CencScheme);

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn name_matches_the_spec_token() {
        assert_eq!(CencScheme::Cenc.name(), "cenc");
        assert_eq!(CencScheme::Cbcs.name(), "cbcs");
    }

    #[test]
    fn display_delegates_to_name() {
        assert_eq!(CencScheme::Cenc.to_string(), "cenc");
        assert_eq!(CencScheme::Cbcs.to_string(), "cbcs");
    }

    #[test]
    fn four_cc_round_trips() {
        for scheme in [CencScheme::Cenc, CencScheme::Cbcs] {
            assert_eq!(CencScheme::from_four_cc(&scheme.to_four_cc()), Some(scheme));
        }
    }

    #[test]
    fn four_cc_is_the_name_bytes() {
        // The `schm` four-CC and the spec token are the same four characters —
        // a mismatch would mean one of the two tables drifted.
        for scheme in [CencScheme::Cenc, CencScheme::Cbcs] {
            assert_eq!(scheme.to_four_cc().as_slice(), scheme.name().as_bytes());
        }
    }

    #[test]
    fn unimplemented_schemes_are_rejected_not_guessed() {
        // `cens`/`cbc1` are real ISO/IEC 23001-7 schemes this workspace does
        // not implement: they must return None, never silently map to a
        // scheme with different crypto.
        assert_eq!(CencScheme::from_four_cc(b"cens"), None);
        assert_eq!(CencScheme::from_four_cc(b"cbc1"), None);
        assert_eq!(CencScheme::from_four_cc(b"\0\0\0\0"), None);
    }
}
