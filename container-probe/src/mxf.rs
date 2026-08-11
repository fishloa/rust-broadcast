//! MXF prober — SMPTE ST 377-1.1:2004 §7.1 (Partition Pack).
//!
//! A Partition Pack begins with a 16-byte Universal Label Key. Its fixed
//! 7-byte prefix is `06 0E 2B 34 02 05 01` — taken from
//! `st377-1/src/partition.rs` (its `PARTITION_KEY_PREFIX`, which is private, so
//! the literal is carried here) where it is validated against real fixtures,
//! and cited to SMPTE ST 377-1. After the Key comes a BER Trailing Length:
//! short form (tag top bit clear) is a single length byte; long form (top bit
//! set) gives the count of following length bytes in the low 7 bits, which must
//! be 1..=8 and must not run past the region.
//!
//! - Key at offset 0 **plus** a well-formed BER length -> `CERTAIN`.
//! - Key at offset 0 with a malformed length -> `STRONG`.
//! - `Detail::None`.

use crate::{Confidence, Detail, Evidence, Outcome};

/// The fixed 7-byte prefix of the Partition Pack Key UL (SMPTE ST 377-1 §7.1 /
/// `st377-1/src/partition.rs` `PARTITION_KEY_PREFIX` — "Defined-Length Pack,
/// Set/Pack Registry" family, bytes 1-7 of the 16-byte UL).
const MXF_KEY_PREFIX: [u8; 7] = [0x06, 0x0E, 0x2B, 0x34, 0x02, 0x05, 0x01];
/// Length of the full Partition Pack Key UL: 16 bytes (SMPTE ST 377-1 §7.1).
const UL_KEY_LEN: usize = 16;
/// The low-7-bits mask of a BER length tag byte (ITU-T X.690 §8.1.3 / ST 377-1
/// §7.1 — long-form count field).
const BER_LONG_COUNT_MASK: u8 = 0x7F;
/// A BER long-form length's tag top bit: 1 when more length octets follow
/// (ITU-T X.690 §8.1.3.5).
const BER_LONG_FORM_FLAG: u8 = 0x80;
/// Maximum number of long-form BER length octets that make sense for a
/// partition pack (an 8-byte length), matching a `u64` payload bound.
const BER_LONG_MAX_OCTETS: usize = 8;

/// The registered MXF prober: Partition Pack key + BER length over `limit`.
pub(crate) fn probe(data: &[u8], limit: usize) -> Outcome {
    debug_assert!(limit <= data.len(), "harness caps limit at data.len()");
    let region = &data[..limit];

    // Key at offset 0, first the fixed 7-byte UL prefix, then a 16-byte key.
    if region.len() < UL_KEY_LEN || region[..MXF_KEY_PREFIX.len()] != MXF_KEY_PREFIX {
        return Outcome::None;
    }
    // The BER length begins immediately after the 16-byte key.
    let ber = &region[UL_KEY_LEN..];

    let detail = Detail::None;
    if ber_length_well_formed(ber) {
        Outcome::Match(Evidence {
            confidence: Confidence::CERTAIN,
            detail,
        })
    } else {
        // Distinct UL but a malformed BER length -> unambiguous MXF, weaker.
        Outcome::Match(Evidence {
            confidence: Confidence::STRONG,
            detail,
        })
    }
}

/// `true` when `ber` begins with a well-formed BER length (ITU-T X.690 §8.1.3):
/// short form (top bit clear, a single length octet) or long form whose octet
/// count is 1..=8 and fits in the region.
fn ber_length_well_formed(ber: &[u8]) -> bool {
    let Some(&first) = ber.first() else {
        return false;
    };
    if first & BER_LONG_FORM_FLAG == 0 {
        // Short form: a single length octet. Always available (first byte).
        return true;
    }
    let octets = (first & BER_LONG_COUNT_MASK) as usize;
    if !(1..=BER_LONG_MAX_OCTETS).contains(&octets) {
        return false;
    }
    // The declared octets must all be present in the region.
    ber.len() > octets
}

#[cfg(test)]
mod drift {
    //! Pins this module's `MXF_KEY_PREFIX` to `st377-1`.
    //!
    //! Lives here rather than in `tests/drift_guard.rs` because the constant is
    //! private: an integration test can only compare upstream against a
    //! literal, which catches upstream changing but not this crate's copy
    //! drifting. A unit test sees the real constant.

    /// `st377-1`'s own `PARTITION_KEY_PREFIX` (`st377-1/src/partition.rs`) is
    /// private, so the exact 7-byte partition prefix cannot be compared. Every
    /// MXF Universal Label shares the SMPTE organisation header `06 0E 2B 34`,
    /// which IS public via `op1a::OP1A_UL_PREFIX` and `klv::FILL_ITEM_KEY_PREFIX`
    /// — pin our first four bytes to both. The remaining `02 05 01` (partition
    /// pack) is SMPTE ST 377-1 and stays a cited literal here for want of a
    /// public upstream constant.
    #[test]
    fn ul_header_matches_st377_1() {
        use st377_1::FILL_ITEM_KEY_PREFIX;
        use st377_1::op1a::OP1A_UL_PREFIX;
        assert_eq!(
            super::MXF_KEY_PREFIX[..4],
            OP1A_UL_PREFIX[..4],
            "container-probe's MXF UL header has drifted from st377-1's"
        );
        assert_eq!(
            super::MXF_KEY_PREFIX[..4],
            FILL_ITEM_KEY_PREFIX[..4],
            "container-probe's MXF UL header has drifted from st377-1's"
        );
    }
}
