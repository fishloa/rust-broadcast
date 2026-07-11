//! Shared ST 291-1 ANC-packet **content** — the `DID`/`SDID`/`Data_Count`/
//! `User_Data_Words`/`Checksum_Word` 10-bit-word sequence, byte-for-byte
//! identical across every ST 291-1 transport this crate carries: SMPTE
//! ST 2038:2021 MPEG-2 TS/PES (the `ts` feature) and RFC 8331 / ST 2110-40 RTP
//! (the `rtp` feature). See `docs/anc_packet_291.md` for the full field
//! semantics and the parity/checksum derivation (not validated here — see
//! that doc's scope note).
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

#[cfg(any(feature = "ts", feature = "rtp"))]
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
    /// `Checksum_Word` (10-bit raw, ST 291-1 checksum — not validated here).
    pub checksum: u16,
}

impl AncContent {
    /// The `User_Data_Words` loop count actually used on the wire: the **low
    /// 8 bits** of `data_count`, independent of the stored `Vec`'s length.
    #[must_use]
    pub fn udw_loop_count(&self) -> usize {
        usize::from(self.data_count & 0xFF)
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
