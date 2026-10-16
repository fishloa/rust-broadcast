//! Shared ST 291-1 ANC-packet **content** — the `DID`/`SDID`/`Data_Count`/
//! `User_Data_Words`/`Checksum_Word` 10-bit-word sequence, byte-for-byte
//! identical across every ST 291-1 transport this crate carries: SMPTE
//! ST 2038:2021 MPEG-2 TS/PES (the `ts` feature) and RFC 8331 / ST 2110-40 RTP
//! (the `rtp` feature). See `docs/anc_packet_291.md` for the full field
//! semantics and the parity/checksum derivation. Parsing/serializing a wire
//! sequence never validates or computes them (a bit-flipped packet still
//! parses `Ok`, matching this crate's transport-layer error model); use
//! [`AncContent::verify_checksum`]/[`AncContent::build`] explicitly where
//! that validation is wanted (S291-W1, #1116).
//!
//! # Architecture (issue #648)
//!
//! Only the **placement** fields differ between transports — ST 2038 has
//! three (`c_not_y_channel_flag`/`line_number`/`horizontal_offset`, Table 2),
//! RFC 8331 has five (`C`/`Line_Number`/`Horizontal_Offset`/`S`/`StreamNum`,
//! §2.1) — and the **padding** scheme differs (ST 2038 byte-aligns with `'1'`
//! bits; RFC 8331 32-bit-word-aligns with `'0'` bits). The content in between
//! (this module) is exactly the same wire sequence either way, so it is
//! implemented once, here, and is **always compiled** — gated behind neither
//! `ts` nor `rtp` — so enabling one transport never pulls in the other.
//!
//! `ts::AncPacket` (this crate's already-shipped, flat public struct) is
//! **not** restructured to embed [`AncContent`] as a public field: doing so
//! would break its existing field layout for zero benefit (its own
//! `read_from`/`write_into` already delegate the DID..Checksum bit sequence to
//! this module internally, which is the actual duplication this module
//! exists to avoid). The new `rtp::RtpAncPacket` wrapper embeds
//! [`AncContent`] directly as its `content` field, per the "shared core +
//! transport-specific placement wrapper" design.

use alloc::vec::Vec;

#[cfg(any(feature = "ts", feature = "rtp"))]
use broadcast_common::bits::{BitReader, BitWriter};

// Parity/checksum (below) is transport-independent ST 291-1 material, so
// `Error`/`Result` are imported unconditionally, not just under `ts`/`rtp`.
use crate::error::{Error, Result};

// Field widths (bits) of the per-ANC-packet **content** (RFC 8331 §2.1 /
// ST 2038 Table 2 — identical in both transports). Only used by a transport
// (`ts` and/or `rtp`); with neither enabled the shared [`AncContent`] data
// type still exists (see the module doc), but nothing needs to (de)serialize
// it, hence the cfg gate on these wire-format internals only.
#[cfg(any(feature = "ts", feature = "rtp"))]
pub(crate) const W_DID: u32 = 10;
#[cfg(any(feature = "ts", feature = "rtp"))]
pub(crate) const W_SDID: u32 = 10;
#[cfg(any(feature = "ts", feature = "rtp"))]
pub(crate) const W_DATA_COUNT: u32 = 10;
#[cfg(any(feature = "ts", feature = "rtp"))]
pub(crate) const W_USER_DATA_WORD: u32 = 10;
#[cfg(any(feature = "ts", feature = "rtp"))]
pub(crate) const W_CHECKSUM: u32 = 10;

/// Check that `value` fits in `bits`, returning it widened to `u64` for
/// [`BitWriter::write_bits`]. Shared by every transport's placement-field
/// validation as well as this module's content fields.
#[cfg(any(feature = "ts", feature = "rtp"))]
pub(crate) fn check_field_width(what: &'static str, value: u64, bits: u32) -> Result<u64> {
    if bits < 64 && value >= (1u64 << bits) {
        return Err(Error::FieldTooWide {
            what,
            value: value as u32,
            bits,
        });
    }
    Ok(value)
}

/// Mask of a 10-bit ANC word.
const WORD_MASK: u16 = 0x3FF;
/// Mask of the 8-bit payload value `b7..b0` of a parity-protected word.
const PAYLOAD_MASK: u16 = 0xFF;

/// Result of [`AncContent::validate`]: one flag per ST 291-1:2011 rule,
/// `true` = the rule holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ValidationReport {
    /// `DID` parity: `b8` even parity of `b7..b0`, `b9 = NOT b8` (§6.1).
    pub did_parity: bool,
    /// `SDID` (Type 2) / `DBN` (Type 1) parity, same rule (§6.2, §6.4).
    pub sdid_parity: bool,
    /// `Data_Count` parity, same rule (§6.5).
    pub data_count_parity: bool,
    /// `Data_Count & 0xFF` equals the number of stored `User_Data_Words`.
    pub data_count_matches_udw: bool,
    /// `Checksum_Word` equals the §6.7 computation (including `b9 = NOT b8`).
    pub checksum: bool,
}

impl ValidationReport {
    /// Assemble a report from its five rule results.
    #[must_use]
    pub fn new(
        did_parity: bool,
        sdid_parity: bool,
        data_count_parity: bool,
        data_count_matches_udw: bool,
        checksum: bool,
    ) -> Self {
        Self {
            did_parity,
            sdid_parity,
            data_count_parity,
            data_count_matches_udw,
            checksum,
        }
    }

    /// `true` when every rule holds.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.did_parity
            && self.sdid_parity
            && self.data_count_parity
            && self.data_count_matches_udw
            && self.checksum
    }
}

/// One ST 291-1 ANC data packet's **content**: `DID`/`SDID`/`Data_Count`/
/// `User_Data_Words`/`Checksum_Word`, every value the raw 10-bit wire word
/// (including the ST 291-1 parity bits) stored verbatim — parity/checksum are
/// not computed or validated here (`docs/anc_packet_291.md` scope note).
///
/// Identical across ST 2038 (MPEG-2 TS/PES) and RFC 8331 (RTP) carriage; see
/// the module doc for why this type is always compiled.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AncContent {
    /// `DID` (10-bit raw, incl. ST 291-1 parity bits).
    pub did: u16,
    /// `SDID` (10-bit raw). For a "Type 1" ANC packet this word actually
    /// carries the data block number (DBN).
    pub sdid: u16,
    /// `Data_Count` (10-bit raw). The `User_Data_Words` loop counter uses
    /// only the low 8 bits (`data_count & 0xFF`).
    pub data_count: u16,
    /// `User_Data_Words` (each 10-bit raw). Length is `data_count & 0xFF`.
    pub user_data_words: Vec<u16>,
    /// `Checksum_Word` (10-bit raw, ST 291-1 checksum). Not validated on
    /// parse; call [`AncContent::verify_checksum`] explicitly.
    pub checksum: u16,
}

impl AncContent {
    /// The `User_Data_Words` loop count actually used on the wire: the **low
    /// 8 bits** of `data_count`, independent of the stored `Vec`'s length.
    #[must_use]
    pub fn udw_loop_count(&self) -> usize {
        usize::from(self.data_count & 0xFF)
    }

    /// Encode an 8-bit payload value into the ST 291-1 10-bit word form: an
    /// even-parity bit `b8` over `b0..b7`, then `b9 = !b8`
    /// (`docs/anc_packet_291.md` "10-bit-word parity rule"). Used to build
    /// `did`/`sdid`/`data_count`/each `user_data_words` entry from its raw
    /// 8-bit value (S291-W1, #1116).
    #[must_use]
    pub fn with_parity(value: u8) -> u16 {
        let b8 = u16::from((value.count_ones() % 2) as u8);
        let b9 = 1 - b8;
        (b9 << 9) | (b8 << 8) | u16::from(value)
    }

    /// `true` if `word` is a well-formed ST 291-1 parity-protected word
    /// (`DID`/`SDID`-or-`DBN`/`Data_Count`, §6.1/§6.2/§6.4/§6.5): a 10-bit
    /// value whose `b8` is the even parity of `b7..b0` and whose `b9` is
    /// `NOT b8`. `User_Data_Words` and the `Checksum_Word` are **not** of this
    /// form (§6.6 gives a UDW all ten bits; §6.7 defines the checksum
    /// separately), so do not call this on them.
    #[must_use]
    pub fn has_valid_parity(word: u16) -> bool {
        word <= WORD_MASK && Self::with_parity((word & PAYLOAD_MASK) as u8) == word
    }

    /// Check the parity bits (`b8`/`b9`) of `DID`, `SDID`/`DBN` and
    /// `Data_Count` (ST 291-1:2011 §6.1, §6.2/§6.4, §6.5). `User_Data_Words`
    /// carry no parity rule (§6.6) and are not checked. Like
    /// [`Self::verify_checksum`] this is explicit; parsing never calls it.
    ///
    /// # Errors
    /// [`Error::ParityMismatch`] naming the first offending word.
    pub fn verify_parity(&self) -> Result<()> {
        for (what, word) in [
            ("DID", self.did),
            ("SDID", self.sdid),
            ("Data_Count", self.data_count),
        ] {
            if !Self::has_valid_parity(word) {
                return Err(Error::ParityMismatch { what, word });
            }
        }
        Ok(())
    }

    /// Check everything ST 291-1:2011 defines as checkable on this content
    /// and report each result separately (never short-circuits, never an
    /// error): word parity, `Data_Count` agreeing with the number of stored
    /// `User_Data_Words`, and the `Checksum_Word`. A caller handling traffic
    /// with known-bad parity can see exactly which rule failed.
    #[must_use]
    pub fn validate(&self) -> ValidationReport {
        ValidationReport::new(
            Self::has_valid_parity(self.did),
            Self::has_valid_parity(self.sdid),
            Self::has_valid_parity(self.data_count),
            self.user_data_words.len() == self.udw_loop_count(),
            self.verify_checksum().is_ok(),
        )
    }

    /// Compute the ST 291-1 `Checksum_Word` for this content's current
    /// `did`/`sdid`/`data_count`/`user_data_words`: the low 9 bits (`b8..b0`)
    /// of each summed mod 2⁹ (end carry discarded), with `b9 = !b8` of the
    /// result (`docs/anc_packet_291.md` "Checksum_Word computation").
    #[must_use]
    pub fn compute_checksum(&self) -> u16 {
        const LOW9: u16 = 0x1FF;
        let mut sum: u32 = u32::from(self.did & LOW9)
            + u32::from(self.sdid & LOW9)
            + u32::from(self.data_count & LOW9);
        for &udw in &self.user_data_words {
            sum += u32::from(udw & LOW9);
        }
        let checksum_value = (sum & u32::from(LOW9)) as u16;
        let b9 = 1 - (checksum_value >> 8);
        (b9 << 9) | checksum_value
    }

    /// `Ok(())` if the stored [`Self::checksum`] matches
    /// [`Self::compute_checksum`], else [`Error::ChecksumMismatch`] (S291-W1,
    /// #1116). Neither `parse`/`read_from` nor `write_into` call this
    /// automatically — a bit-flipped packet still parses `Ok`, per the
    /// crate's transport-layer error model; a caller that needs the RFC 8331
    /// §7 validation guidance calls this explicitly.
    ///
    /// # Errors
    /// [`Error::ChecksumMismatch`] on a mismatch.
    pub fn verify_checksum(&self) -> Result<()> {
        let computed = self.compute_checksum();
        if self.checksum == computed {
            Ok(())
        } else {
            Err(Error::ChecksumMismatch {
                stored: self.checksum,
                computed,
            })
        }
    }

    /// Build a content sequence from raw (unparitied) 8-bit payload values,
    /// filling in every field's parity bits ([`Self::with_parity`]) and
    /// computing [`Self::checksum`] ([`Self::compute_checksum`]) — the
    /// producer-side counterpart to [`Self::verify_checksum`] (S291-W1,
    /// #1116).
    ///
    /// `udw8s` are the raw (unparitied) `User_Data_Word` payload bytes; its
    /// length also sets the `Data_Count` value.
    ///
    /// # Errors
    /// [`Error::FieldTooWide`] if `udw8s.len()` exceeds `u8::MAX`
    /// (`Data_Count`'s 8-bit value field).
    pub fn build(did8: u8, sdid8: u8, udw8s: &[u8]) -> Result<Self> {
        let count = u8::try_from(udw8s.len()).map_err(|_| Error::FieldTooWide {
            what: "Data_Count",
            value: udw8s.len() as u32,
            bits: 8,
        })?;
        let mut content = Self {
            did: Self::with_parity(did8),
            sdid: Self::with_parity(sdid8),
            data_count: Self::with_parity(count),
            user_data_words: udw8s.iter().map(|&b| Self::with_parity(b)).collect(),
            checksum: 0,
        };
        content.checksum = content.compute_checksum();
        Ok(content)
    }

    /// Bit width of this content sequence on the wire: the fixed 40-bit
    /// `DID`+`SDID`+`Data_Count`+`Checksum_Word` plus 10 bits per
    /// `User_Data_Word` (counted as `data_count & 0xFF`).
    #[cfg(any(feature = "ts", feature = "rtp"))]
    pub(crate) fn content_bit_width(&self) -> usize {
        (W_DID + W_SDID + W_DATA_COUNT + W_CHECKSUM) as usize
            + self.udw_loop_count() * W_USER_DATA_WORD as usize
    }

    /// Write `DID`/`SDID`/`Data_Count`/`User_Data_Words`/`Checksum_Word` into
    /// `w`, MSB-first, with no leading/trailing padding (the caller's
    /// transport owns alignment/padding).
    ///
    /// # Errors
    /// [`Error::InconsistentUdwLength`] if `user_data_words.len()` does not
    /// equal `data_count & 0xFF`; [`Error::FieldTooWide`] if any field
    /// exceeds its 10-bit wire width.
    #[cfg(any(feature = "ts", feature = "rtp"))]
    pub(crate) fn write_into(&self, w: &mut BitWriter<'_>) -> Result<()> {
        let need = self.udw_loop_count();
        let have = self.user_data_words.len();
        if have != need {
            return Err(Error::InconsistentUdwLength { have, need });
        }
        w.write_bits(check_field_width("DID", u64::from(self.did), W_DID)?, W_DID)?;
        w.write_bits(
            check_field_width("SDID", u64::from(self.sdid), W_SDID)?,
            W_SDID,
        )?;
        w.write_bits(
            check_field_width("Data_Count", u64::from(self.data_count), W_DATA_COUNT)?,
            W_DATA_COUNT,
        )?;
        for udw in &self.user_data_words {
            w.write_bits(
                check_field_width("User_Data_Word", u64::from(*udw), W_USER_DATA_WORD)?,
                W_USER_DATA_WORD,
            )?;
        }
        w.write_bits(
            check_field_width("Checksum_Word", u64::from(self.checksum), W_CHECKSUM)?,
            W_CHECKSUM,
        )?;
        Ok(())
    }

    /// Read `DID`/`SDID`/`Data_Count`/`User_Data_Words`/`Checksum_Word` from
    /// `r`, MSB-first; the caller's transport has already consumed any
    /// placement fields ahead of this content and owns skipping any padding
    /// after it.
    #[cfg(any(feature = "ts", feature = "rtp"))]
    pub(crate) fn read_from(r: &mut BitReader<'_>) -> Result<Self> {
        let did = r.read_bits(W_DID)? as u16;
        let sdid = r.read_bits(W_SDID)? as u16;
        let data_count = r.read_bits(W_DATA_COUNT)? as u16;
        let n = usize::from(data_count & 0xFF);
        let mut user_data_words = Vec::with_capacity(n);
        for _ in 0..n {
            user_data_words.push(r.read_bits(W_USER_DATA_WORD)? as u16);
        }
        let checksum = r.read_bits(W_CHECKSUM)? as u16;
        Ok(Self {
            did,
            sdid,
            data_count,
            user_data_words,
            checksum,
        })
    }
}

#[cfg(all(test, any(feature = "ts", feature = "rtp")))]
mod tests {
    use super::*;
    use alloc::vec;

    fn sample() -> AncContent {
        AncContent {
            did: 0x161,
            sdid: 0x101,
            data_count: 0x002,
            user_data_words: vec![0x2CF, 0x101],
            checksum: 0x233,
        }
    }

    #[test]
    fn round_trip() {
        let c = sample();
        let bits = c.content_bit_width();
        assert_eq!(bits, 40 + 2 * 10);
        let mut buf = vec![0u8; bits.div_ceil(8)];
        {
            let mut w = BitWriter::new(&mut buf);
            c.write_into(&mut w).unwrap();
        }
        let mut r = BitReader::new(&buf);
        let reparsed = AncContent::read_from(&mut r).unwrap();
        assert_eq!(reparsed, c);
    }

    #[test]
    fn rejects_inconsistent_udw_length() {
        let mut c = sample();
        c.user_data_words.pop();
        let mut buf = vec![0u8; 8];
        let mut w = BitWriter::new(&mut buf);
        assert!(matches!(
            c.write_into(&mut w),
            Err(Error::InconsistentUdwLength { have: 1, need: 2 })
        ));
    }
}

/// Parity/checksum (S291-W1, #1116) is transport-independent, so its tests
/// run regardless of the `ts`/`rtp` feature gate. Expected values are hand
/// computed independently from `docs/anc_packet_291.md`'s formulas (not
/// derived by calling the functions under test), since neither RFC 8331 nor
/// ST 291-1 publishes a full worked numeric example and no local tool
/// (ffmpeg/TSDuck/…) generates ST 291-1 ANC content.
#[cfg(test)]
mod parity_checksum_tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn with_parity_matches_hand_computed_even_parity() {
        // b8 = even parity of b0..b7 (count_ones % 2), b9 = !b8.
        assert_eq!(AncContent::with_parity(0x00), 0x200); // 0 ones -> b8=0, b9=1
        assert_eq!(AncContent::with_parity(0xFF), 0x2FF); // 8 ones -> b8=0, b9=1
        assert_eq!(AncContent::with_parity(0x01), 0x101); // 1 one  -> b8=1, b9=0
        assert_eq!(AncContent::with_parity(0x80), 0x180); // 1 one  -> b8=1, b9=0
        // 0x61 = 0b0110_0001, 3 ones -> b8=1, b9=0. Also the raw `did` value
        // the crate's own fixture_anc.rs / anc.bin fixture carries for its
        // first ANC packet (0x161), confirming this matches real data.
        assert_eq!(AncContent::with_parity(0x61), 0x161);
    }

    #[test]
    fn compute_checksum_matches_hand_computed_sum_no_wrap() {
        // Sum of low-9-bits: 0x100 + 0x001 + 0x000 (no UDW) = 0x101 = 257.
        // bit8 of 257 is 1, so b9 = !1 = 0: Checksum_Word = 0x101.
        let c = AncContent {
            did: 0x100,
            sdid: 0x001,
            data_count: 0x000,
            user_data_words: vec![],
            checksum: 0x101,
        };
        assert_eq!(c.compute_checksum(), 0x101);
        assert!(c.verify_checksum().is_ok());
    }

    #[test]
    fn compute_checksum_matches_hand_computed_sum_with_end_carry_discarded() {
        // Sum of low-9-bits: 0x1FF + 0x1FF + 0x1FF (no UDW) = 1533.
        // 1533 mod 512 = 509 = 0x1FD (the discarded end carry is 1024).
        // bit8 of 509 is 1, so b9 = !1 = 0: Checksum_Word = 0x1FD.
        let c = AncContent {
            did: 0x1FF,
            sdid: 0x1FF,
            data_count: 0x1FF,
            user_data_words: vec![],
            checksum: 0x1FD,
        };
        assert_eq!(c.compute_checksum(), 0x1FD);
        assert!(c.verify_checksum().is_ok());
    }

    #[test]
    fn compute_checksum_includes_user_data_words() {
        // 0x055 + 0x0AA + 0x001 + 0x001 (one UDW) = 257 = 0x101 (same result
        // as the no-UDW vector above, arrived at a different way).
        let c = AncContent {
            did: 0x055,
            sdid: 0x0AA,
            data_count: 0x001,
            user_data_words: vec![0x001],
            checksum: 0x101,
        };
        assert_eq!(c.compute_checksum(), 0x101);
    }

    #[test]
    fn verify_checksum_rejects_a_bit_flipped_packet() {
        // audit issue #1116: pre-fix, a bit-flipped ANC packet (wrong
        // checksum) parsed `Ok` and had no way to be told apart from a valid
        // one. Observed pre-fix (no `verify_checksum` existed): this
        // mismatch was silently unreachable to callers.
        let mut c = AncContent {
            did: 0x100,
            sdid: 0x001,
            data_count: 0x000,
            user_data_words: vec![],
            checksum: 0x101,
        };
        assert!(c.verify_checksum().is_ok());
        c.checksum ^= 0x001; // flip one bit of the stored checksum
        assert_eq!(
            c.verify_checksum(),
            Err(Error::ChecksumMismatch {
                stored: 0x100,
                computed: 0x101,
            })
        );
    }

    #[test]
    fn build_fills_in_parity_and_a_verifiable_checksum() {
        let c = AncContent::build(0x61, 0x01, &[0xCF, 0x01]).unwrap();
        assert_eq!(c.did, 0x161);
        assert_eq!(c.sdid, 0x101);
        assert_eq!(c.data_count, AncContent::with_parity(2));
        assert_eq!(c.user_data_words.len(), 2);
        assert!(
            c.verify_checksum().is_ok(),
            "a built content sequence must satisfy its own checksum"
        );
    }

    /// ST 291-1:2011 Annex C (PDF p.17, informative) lists real DID/DBN/DC
    /// words "inclusive of parity bits": each must satisfy §6.1/§6.2/§6.5.
    #[test]
    fn annex_c_words_have_valid_parity() {
        let words = [
            // DID row
            0x2FF, 0x1FD, 0x2FC, 0x180, 0x2F5, 0x260, 0x2C0, 0x140, 0x104,
            // DBN/SDID row
            0x101, 0x200, 0x110, // DC row
            0x230, 0x224, 0x206, 0x108,
        ];
        for w in words {
            assert!(AncContent::has_valid_parity(w), "{w:#05X}");
        }
        // And they are exactly what with_parity produces from the 8-bit value.
        assert_eq!(AncContent::with_parity(0xF5), 0x2F5);
        assert_eq!(AncContent::with_parity(0x30), 0x230); // 48 words
        assert_eq!(AncContent::with_parity(0x24), 0x224); // 36 words
        assert_eq!(AncContent::with_parity(0x08), 0x108); // 8 words
    }

    #[test]
    fn parity_rejects_each_flipped_bit() {
        for good in [0x2F5u16, 0x108, 0x200, 0x1FD] {
            for bit in 0..10 {
                let bad = good ^ (1 << bit);
                // Flipping any one bit changes the data parity or b8/b9, so
                // it must always be caught (b9 and b8 included).
                assert!(
                    !AncContent::has_valid_parity(bad),
                    "{good:#05X} with bit {bit} flipped ({bad:#05X}) wrongly accepted"
                );
            }
        }
        // b9 == b8 (both set / both clear) is invalid.
        assert!(!AncContent::has_valid_parity(0x300));
        assert!(!AncContent::has_valid_parity(0x000));
        // Not a 10-bit word.
        assert!(!AncContent::has_valid_parity(0x400 | 0x200));
    }

    /// A fully valid packet built by hand from the spec text: DID 0xF5,
    /// DBN 0x00, 8 UDW of 0x0AA. Checksum: low-9-bit sum
    /// 0x0F5 + 0x000 + 0x108 + 8*0x0AA = 0x74D, mod 2^9 = 0x14D, b8 = 1 so
    /// b9 = 0 -> 0x14D.
    fn valid_packet() -> AncContent {
        AncContent {
            did: 0x2F5,
            sdid: 0x200,
            data_count: 0x108,
            user_data_words: vec![0x0AA; 8],
            checksum: 0x14D,
        }
    }

    #[test]
    fn validate_accepts_hand_computed_valid_packet() {
        let c = valid_packet();
        assert_eq!(c.compute_checksum(), 0x14D);
        assert!(c.verify_parity().is_ok());
        let r = c.validate();
        assert!(r.is_valid());
        assert_eq!(r, ValidationReport::new(true, true, true, true, true));
        // The producer-side builder agrees with the hand computation (it
        // also sets parity on the UDWs, 0xAA -> 0x2AA, which does not change
        // the low-9-bit checksum sum).
        let built = AncContent::build(0xF5, 0x00, &[0xAA; 8]).unwrap();
        assert_eq!(built.user_data_words, vec![0x2AA; 8]);
        assert_eq!(
            (built.did, built.sdid, built.data_count, built.checksum),
            (c.did, c.sdid, c.data_count, c.checksum)
        );
        assert!(built.validate().is_valid());
    }

    #[test]
    fn validate_flags_each_flipped_bit_on_its_own_rule() {
        // b8 and b9 of DID / SDID / Data_Count.
        for bit in [8u16, 9] {
            let mut c = valid_packet();
            c.did ^= 1 << bit;
            let r = c.validate();
            assert!(!r.did_parity && r.sdid_parity && r.data_count_parity);
            assert_eq!(
                c.verify_parity(),
                Err(Error::ParityMismatch {
                    what: "DID",
                    word: c.did
                })
            );

            let mut c = valid_packet();
            c.sdid ^= 1 << bit;
            let r = c.validate();
            assert!(r.did_parity && !r.sdid_parity && r.data_count_parity);
            assert!(matches!(
                c.verify_parity(),
                Err(Error::ParityMismatch { what: "SDID", .. })
            ));

            let mut c = valid_packet();
            c.data_count ^= 1 << bit;
            let r = c.validate();
            assert!(r.did_parity && r.sdid_parity && !r.data_count_parity);
            assert!(matches!(
                c.verify_parity(),
                Err(Error::ParityMismatch {
                    what: "Data_Count",
                    ..
                })
            ));
        }
        // Checksum b9 (NOT b8) and b8 each flipped: caught by the checksum
        // rule only.
        for bit in [8u16, 9] {
            let mut c = valid_packet();
            c.checksum ^= 1 << bit;
            let r = c.validate();
            assert!(!r.checksum && r.did_parity && r.sdid_parity && r.data_count_parity);
            assert!(!r.is_valid());
        }
    }

    #[test]
    fn udw_has_no_parity_rule_but_is_covered_by_checksum() {
        // §6.6: a UDW is a free 10-bit word; one with b9 == b8 is legal and
        // must not trip a parity rule, only (if altered) the checksum.
        let mut c = valid_packet();
        c.user_data_words[0] = 0x300;
        c.checksum = c.compute_checksum();
        assert!(c.validate().is_valid());
        c.user_data_words[0] ^= 0x001;
        let r = c.validate();
        assert!(!r.checksum && r.did_parity);
    }

    #[test]
    fn validate_reports_data_count_vs_udw_len() {
        let mut c = valid_packet();
        c.user_data_words.pop();
        assert!(!c.validate().data_count_matches_udw);
    }

    #[test]
    fn build_rejects_more_than_255_user_data_words() {
        let udws = alloc::vec![0u8; 256];
        assert!(matches!(
            AncContent::build(0x00, 0x00, &udws),
            Err(Error::FieldTooWide {
                what: "Data_Count",
                ..
            })
        ));
    }
}
