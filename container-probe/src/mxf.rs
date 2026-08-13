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
//! - Full Partition Pack Key at offset 0 **plus** a well-formed BER length ->
//!   `CERTAIN`.
//! - Key prefix at offset 0 but a malformed full key or a malformed length ->
//!   `STRONG`.
//! - `Detail::Mxf { partition_kind }`.

use crate::{Confidence, Detail, Evidence, Outcome};

/// The fixed 7-byte prefix of the Partition Pack Key UL (SMPTE ST 377-1 §7.1 /
/// `st377-1/src/partition.rs` `PARTITION_KEY_PREFIX` — "Defined-Length Pack,
/// Set/Pack Registry" family, bytes 1-7 of the 16-byte UL).
const MXF_KEY_PREFIX: [u8; 7] = [0x06, 0x0E, 0x2B, 0x34, 0x02, 0x05, 0x01];
/// The fixed "mid" section of the Partition Pack Key UL (bytes 9-12), `0D 01 02
/// 01` (`st377-1` `PARTITION_KEY_MID`). The prefix alone cannot separate a
/// Partition Pack from a Primer Pack or Random Index Pack — all three share it
/// — so the mid section and the Partition Kind byte must be checked too.
const MXF_KEY_MID: [u8; 4] = [0x0D, 0x01, 0x02, 0x01];
/// Byte 13 of the UL (byte 13 1-indexed): the "Structure Kind" (`0x01`).
const MXF_KEY_STRUCTURE_KIND: u8 = 0x01;
/// Byte 14 of the UL carries the Partition Kind (`PartitionKind`), one of
/// `0x02` Header / `0x03` Body / `0x04` Footer (SMPTE ST 377-1 §7.2-7.4).
const MXF_PARTITION_KIND_HEADER: u8 = 0x02;
const MXF_PARTITION_KIND_FOOTER: u8 = 0x04;
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
    if region.len() < UL_KEY_LEN {
        // Shorter than the 16-byte Partition Pack Key (the fixture's key runs
        // through byte 15): a truncated .mxf read a few bytes at a time is
        // undecided (`Insufficient`), not `Unknown`.
        return Outcome::Insufficient(UL_KEY_LEN);
    }
    if region[..MXF_KEY_PREFIX.len()] != MXF_KEY_PREFIX {
        return Outcome::None;
    }
    // The BER length begins immediately after the 16-byte key.
    let ber = &region[UL_KEY_LEN..];

    let detail = Detail::Mxf {
        partition_kind: region[13],
    };
    if is_partition_pack_key(region) && ber_length_well_formed(ber) {
        Outcome::Match(Evidence {
            confidence: Confidence::CERTAIN,
            detail,
        })
    } else {
        // The UL prefix matched but it is not a complete Partition Pack Key, or
        // the BER length is malformed -> unambiguous MXF, weaker.
        Outcome::Match(Evidence {
            confidence: Confidence::STRONG,
            detail,
        })
    }
}

/// `true` when `key` is a complete Partition Pack Key (SMPTE ST 377-1 §7.1,
/// Table 4): the fixed 7-byte prefix, the `0D 01 02 01` mid section, the
/// `0x01` Structure Kind byte, and a valid Partition Kind byte (`0x02`/`0x03`/
/// `0x04`). `st377-1`'s `PartitionPack::is_partition_key` applies exactly this
/// check and is drift-guarded against it in the test module below; the prefix
/// alone cannot separate a Partition Pack from a Primer/Random-Index Pack.
fn is_partition_pack_key(key: &[u8]) -> bool {
    key[..MXF_KEY_PREFIX.len()] == MXF_KEY_PREFIX
        && key[8..8 + MXF_KEY_MID.len()] == MXF_KEY_MID
        && key[12] == MXF_KEY_STRUCTURE_KIND
        && (MXF_PARTITION_KIND_HEADER..=MXF_PARTITION_KIND_FOOTER).contains(&key[13])
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

    /// `is_partition_pack_key` must agree exactly with `st377-1`'s own
    /// `PartitionPack::is_partition_key` (its private `PARTITION_KEY_MID` /
    /// Structure-Kind / Partition-Kind literals are the authoritative source).
    /// Agreement is checked over the real fixture key, a synthesized valid key,
    /// and adversarial mutations of each of the MID/Structure-Kind/Partition-Kind
    /// bytes, so a drift in any one of them (not just the shared prefix) fails.
    #[test]
    fn partition_key_predicate_matches_st377_1() {
        let mut base = [0u8; 16];
        base[0..7].copy_from_slice(&super::MXF_KEY_PREFIX);
        base[7] = 0x01;
        base[8..12].copy_from_slice(&super::MXF_KEY_MID);
        base[12] = 0x01;
        base[13] = 0x02; // Header partition
        base[14] = 0x04; // Closed and Complete
        base[15] = 0x00;

        // The synthesized valid key.
        assert_eq!(
            super::is_partition_pack_key(&base),
            st377_1::PartitionPack::is_partition_key(&base),
        );

        // Real fixture key (extracted from the file).
        let real = std::fs::read(std::format!(
            "{}/../fixtures/mxf/op1a_mpeg2_pcm.mxf",
            env!("CARGO_MANIFEST_DIR")
        ))
        .expect("fixture");
        let real_key: [u8; 16] = real[..16].try_into().expect("16 bytes");
        assert_eq!(
            super::is_partition_pack_key(&real_key),
            st377_1::PartitionPack::is_partition_key(&real_key),
        );
        assert!(
            super::is_partition_pack_key(&real_key),
            "real fixture must be a partition pack"
        );

        // Muate each discriminating byte in turn and require both predicates to
        // stay in agreement.
        for pos in [8usize, 9, 10, 11, 12, 13] {
            for val in [0x00u8, 0x01, 0xFF] {
                let mut m = base;
                m[pos] = val;
                assert_eq!(
                    super::is_partition_pack_key(&m),
                    st377_1::PartitionPack::is_partition_key(&m),
                    "predicates disagree at byte {pos}={val:#x}"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_bytes(rel: &str) -> std::vec::Vec<u8> {
        std::fs::read(std::format!("{}/../{}", env!("CARGO_MANIFEST_DIR"), rel))
            .unwrap_or_else(|e| panic!("failed to read {rel}: {e}"))
    }

    /// Finding 4: a 15-byte prefix of a real MXF fixture (1 byte short of the
    /// 16-byte Partition Pack Key) is `Insufficient`, not `Unknown`.
    #[test]
    fn short_prefix_is_insufficient() {
        let data = fixture_bytes("fixtures/mxf/op1a_mpeg2_pcm.mxf");
        let region = &data[..UL_KEY_LEN - 1];
        match probe(region, region.len()) {
            Outcome::Insufficient(n) => assert_eq!(n, UL_KEY_LEN),
            other => panic!("15-byte MXF prefix must be Insufficient(16), got {other:?}"),
        }
    }

    /// Finding 8: a UL with the 7-byte prefix but a wrong mid section (i.e. not
    /// a Partition Pack Key) must not score `CERTAIN` — the prefix alone cannot
    /// separate a Partition Pack from a Primer/Random-Index Pack, which is the
    /// whole reason for the full-key check. It still matches MXF (`STRONG`),
    /// just not the "magic **plus** structural confirmation" tier.
    #[test]
    fn prefix_but_not_partition_key_is_strong_not_certain() {
        let mut key = [0u8; 32]; // 16-byte key + a well-formed short BER length
        key[0..7].copy_from_slice(&MXF_KEY_PREFIX);
        key[7] = 0x01;
        // Wrong mid section: 00 00 00 00 instead of 0D 01 02 01.
        key[8..12].copy_from_slice(&[0x00, 0x00, 0x00, 0x00]);
        key[12] = 0x01;
        key[13] = 0x02;
        // BER short-form length follows the key (byte 16): 0x10 (top bit clear).
        key[16] = 0x10;
        match probe(&key, key.len()) {
            Outcome::Match(ev) => assert_eq!(ev.confidence, Confidence::STRONG),
            other => panic!("prefix-only MXF must be STRONG, got {other:?}"),
        }
    }
}
