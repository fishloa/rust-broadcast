//! L1-pre signalling — EN 302 755 §7.2.2, Figure 25.
//!
//! The L1-pre block is exactly 168 information bits (21 bytes), followed by a
//! 32-bit CRC in the standalone (non-T2-MI) form. In T2-MI (ETSI TS 102 773
//! §5.2.4) the CRC is dropped; the T2-MI packet CRC covers the whole packet.

use alloc::vec::Vec;
use broadcast_common::bits::{BitReader, BitWriter};
use broadcast_common::len::{need_in, need_out};
use broadcast_common::{Parse, Serialize};

use super::enums::{
    GuardInterval, L1CodeRate, L1FecType, L1Modulation, PaprReduction, PaprReductionV0,
    PaprReductionVn, PilotPattern, T2Version, TxInputStreamType,
};
use crate::payload::fef_null::S1Field;

// ── bit widths (field order per Figure 25) ───────────────────────────────────

/// Total information bits in L1-pre (excluding CRC).
pub const L1PRE_BITS: usize = 168;
/// Byte length of the L1PRE block as carried in T2-MI.
pub const L1PRE_BYTES: usize = L1PRE_BITS / 8; // = 21
/// Byte length of the CRC appended in the standalone form.
pub const L1PRE_CRC_BYTES: usize = 4;
/// Byte length of the full standalone L1-pre block (info + CRC).
pub const L1PRE_WITH_CRC_BYTES: usize = L1PRE_BYTES + L1PRE_CRC_BYTES; // = 25

const TYPE_BITS: u32 = 8;
const BWT_EXT_BITS: u32 = 1;
const S1_BITS: u32 = 3;
const S2_BITS: u32 = 4;
const L1_REPETITION_FLAG_BITS: u32 = 1;
const GUARD_INTERVAL_BITS: u32 = 3;
const PAPR_BITS: u32 = 4;
const L1_MOD_BITS: u32 = 4;
const L1_COD_BITS: u32 = 2;
const L1_FEC_TYPE_BITS: u32 = 2;
const L1_POST_SIZE_BITS: u32 = 18;
const L1_POST_INFO_SIZE_BITS: u32 = 18;
const PILOT_PATTERN_BITS: u32 = 4;
const TX_ID_AVAILABILITY_BITS: u32 = 8;
const CELL_ID_BITS: u32 = 16;
const NETWORK_ID_BITS: u32 = 16;
const T2_SYSTEM_ID_BITS: u32 = 16;
const NUM_T2_FRAMES_BITS: u32 = 8;
const NUM_DATA_SYMBOLS_BITS: u32 = 12;
const REGEN_FLAG_BITS: u32 = 3;
const L1_POST_EXTENSION_BITS: u32 = 1;
const NUM_RF_BITS: u32 = 3;
const CURRENT_RF_IDX_BITS: u32 = 3;
const T2_VERSION_BITS: u32 = 4;
const L1_POST_SCRAMBLED_BITS: u32 = 1;
const T2_BASE_LITE_BITS: u32 = 1;
const RESERVED_BITS: u32 = 4;

// sanity: all fields sum to 168
const _: () = assert!(
    (TYPE_BITS
        + BWT_EXT_BITS
        + S1_BITS
        + S2_BITS
        + L1_REPETITION_FLAG_BITS
        + GUARD_INTERVAL_BITS
        + PAPR_BITS
        + L1_MOD_BITS
        + L1_COD_BITS
        + L1_FEC_TYPE_BITS
        + L1_POST_SIZE_BITS
        + L1_POST_INFO_SIZE_BITS
        + PILOT_PATTERN_BITS
        + TX_ID_AVAILABILITY_BITS
        + CELL_ID_BITS
        + NETWORK_ID_BITS
        + T2_SYSTEM_ID_BITS
        + NUM_T2_FRAMES_BITS
        + NUM_DATA_SYMBOLS_BITS
        + REGEN_FLAG_BITS
        + L1_POST_EXTENSION_BITS
        + NUM_RF_BITS
        + CURRENT_RF_IDX_BITS
        + T2_VERSION_BITS
        + L1_POST_SCRAMBLED_BITS
        + T2_BASE_LITE_BITS
        + RESERVED_BITS) as usize
        == L1PRE_BITS
);

// ── struct ────────────────────────────────────────────────────────────────────

/// L1-pre signalling fields — EN 302 755 §7.2.2, Figure 25.
///
/// Parses the 21-byte (168-bit) L1PRE block as carried inside a T2-MI
/// L1-current payload (ETSI TS 102 773 §5.2.4 Table 2). The CRC is not
/// included in this struct; use [`L1Pre::crc32`] / [`L1Pre::serialize_with_crc`]
/// for the standalone 200-bit form.
///
/// Field order matches the wire bit order (MSB-first within each field).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct L1Pre {
    /// TYPE — Tx input stream types (8 bits, Table 21).
    pub type_: TxInputStreamType,
    /// BWT_EXT — Extended carrier mode flag (1 bit).
    pub bwt_ext: bool,
    /// S1 — P1 preamble format (3 bits, Table 18).
    pub s1: S1Field,
    /// S2 — P1 signalling S2 field 1+2 (4 bits, Tables 19a/19b + 20).
    pub s2: u8,
    /// L1_REPETITION_FLAG — next-frame dynamic present in this frame (1 bit).
    pub l1_repetition_flag: bool,
    /// GUARD_INTERVAL (3 bits, Table 22).
    pub guard_interval: GuardInterval,
    /// PAPR raw 4-bit value; decode with [`L1Pre::papr_reduction`].
    pub papr: u8,
    /// L1_MOD — L1-post constellation (4 bits, Table 24).
    pub l1_mod: L1Modulation,
    /// L1_COD — L1-post code rate (2 bits, Table 25).
    pub l1_cod: L1CodeRate,
    /// L1_FEC_TYPE — L1 FEC type (2 bits, Table 26).
    pub l1_fec_type: L1FecType,
    /// L1_POST_SIZE — coded+modulated L1-post size in OFDM cells (18 bits).
    pub l1_post_size: u32,
    /// L1_POST_INFO_SIZE — L1-post information bits incl. extension, excl. CRC (18 bits).
    pub l1_post_info_size: u32,
    /// PILOT_PATTERN (4 bits, Table 27).
    pub pilot_pattern: PilotPattern,
    /// TX_ID_AVAILABILITY (8 bits).
    pub tx_id_availability: u8,
    /// CELL_ID — geographic cell identifier (16 bits; 0 = not provided).
    pub cell_id: u16,
    /// NETWORK_ID — DVB network identifier (16 bits).
    pub network_id: u16,
    /// T2_SYSTEM_ID — T2 system identifier (16 bits).
    pub t2_system_id: u16,
    /// NUM_T2_FRAMES — T2-frames per super-frame (8 bits; min 2).
    pub num_t2_frames: u8,
    /// NUM_DATA_SYMBOLS — data OFDM symbols per T2-frame excl. P1/P2 (12 bits).
    pub num_data_symbols: u16,
    /// REGEN_FLAG — regeneration count (3 bits; 0 = not regenerated).
    pub regen_flag: u8,
    /// L1_POST_EXTENSION — L1-post extension field present flag (1 bit).
    pub l1_post_extension: bool,
    /// NUM_RF — number of RF frequencies (3 bits).
    pub num_rf: u8,
    /// CURRENT_RF_IDX — index of current RF channel (3 bits; 0 if TFS unused).
    pub current_rf_idx: u8,
    /// T2_VERSION (4 bits, Table 28).
    pub t2_version: T2Version,
    /// L1_POST_SCRAMBLED — L1-post scrambling flag (1 bit).
    pub l1_post_scrambled: bool,
    /// T2_BASE_LITE — T2-Lite compatibility flag (1 bit).
    pub t2_base_lite: bool,
    /// RESERVED (4 bits; sometimes used for bias balancing).
    pub reserved: u8,
}

impl L1Pre {
    /// Decode the PAPR field using the version-appropriate table.
    ///
    /// When `t2_version` is `T2Version::V1_1_1` (wire value 0000), Table 23a applies.
    /// For all other versions, Table 23b applies.
    #[must_use]
    pub fn papr_reduction(&self) -> PaprReduction {
        if self.t2_version == T2Version::V1_1_1 {
            PaprReduction::V0(PaprReductionV0::from_u8(self.papr))
        } else {
            PaprReduction::Vn(PaprReductionVn::from_u8(self.papr))
        }
    }

    /// Compute the CRC-32/MPEG-2 over the 21-byte serialised L1-pre info block.
    ///
    /// This is the CRC defined in EN 302 755 Annex F; it is appended in the
    /// standalone (non-T2-MI) form of L1-pre.
    ///
    /// # Errors
    ///
    /// Propagates [`Self::to_bytes`]'s error: every field here is `pub` and
    /// mutable, so a caller-set value outside its wire bit width (e.g.
    /// `num_rf > 7`) cannot be serialised (W-T2-3).
    pub fn crc32(&self) -> crate::error::Result<u32> {
        let bytes = self.to_bytes()?;
        Ok(broadcast_common::crc32_mpeg2::compute(&bytes))
    }

    /// Serialise to the 21-byte T2-MI form (no CRC).
    ///
    /// # Errors
    ///
    /// [`L1Pre`]'s fields are all `pub` and mutable (it is `#[non_exhaustive]`
    /// but not otherwise guarded), so a value written outside its wire bit
    /// width (e.g. `num_rf > 7`, `l1_post_size >= 2^18`) fails here rather
    /// than panicking or silently masking into a misframed block (W-T2-3;
    /// same class as the truncation bug this sweep also fixes, W-T2-4 —
    /// reject out-of-range values instead of masking them).
    pub fn to_bytes(&self) -> crate::error::Result<[u8; L1PRE_BYTES]> {
        let mut buf = [0u8; L1PRE_BYTES];
        self.serialize_into(&mut buf)?;
        Ok(buf)
    }

    /// Parse the 21-byte info block and validate the 4-byte CRC-32 that follows
    /// it; the inverse of [`serialize_with_crc`](Self::serialize_with_crc).
    ///
    /// The CRC is read from the fixed offset `bytes[21..25]` and computed over
    /// the received info block; trailing bytes after the 25 are ignored.
    ///
    /// # Errors
    ///
    /// [`crate::Error::BufferTooShort`] if `bytes` is shorter than 25 bytes;
    /// [`crate::Error::CrcMismatch`] if the CRC does not match.
    pub fn parse_with_crc(bytes: &[u8]) -> crate::error::Result<L1Pre> {
        need_in(bytes, L1PRE_WITH_CRC_BYTES, "L1Pre with CRC")?;
        let (info, crc) = bytes[..L1PRE_WITH_CRC_BYTES].split_at(L1PRE_BYTES);
        let expected = u32::from_be_bytes([crc[0], crc[1], crc[2], crc[3]]);
        let computed = broadcast_common::crc32_mpeg2::compute(info);
        if computed != expected {
            return Err(crate::Error::CrcMismatch { computed, expected });
        }
        L1Pre::parse(info)
    }

    /// Serialise to a 25-byte buffer: 21-byte info block + 4-byte CRC.
    ///
    /// # Errors
    ///
    /// Propagates [`Self::to_bytes`]'s error (W-T2-3).
    pub fn serialize_with_crc(&self) -> crate::error::Result<Vec<u8>> {
        let mut out = Vec::with_capacity(L1PRE_WITH_CRC_BYTES);
        let bytes = self.to_bytes()?;
        out.extend_from_slice(&bytes);
        out.extend_from_slice(&broadcast_common::crc32_mpeg2::compute(&bytes).to_be_bytes());
        Ok(out)
    }
}

impl<'a> Parse<'a> for L1Pre {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> crate::error::Result<Self> {
        need_in(bytes, L1PRE_BYTES, "L1Pre")?;
        let mut r = BitReader::new(&bytes[..L1PRE_BYTES]);

        let type_ = TxInputStreamType::from_u8(r.read_bits(TYPE_BITS)? as u8);
        let bwt_ext = r.read_bits(BWT_EXT_BITS)? != 0;
        let s1_raw = r.read_bits(S1_BITS)? as u8;
        // S1Field covers all 8 values; TryFrom never fails on a 3-bit value
        let s1 = S1Field::try_from(s1_raw).unwrap_or(S1Field::V7);
        let s2 = r.read_bits(S2_BITS)? as u8;
        let l1_repetition_flag = r.read_bits(L1_REPETITION_FLAG_BITS)? != 0;
        let guard_interval = GuardInterval::from_u8(r.read_bits(GUARD_INTERVAL_BITS)? as u8);
        let papr = r.read_bits(PAPR_BITS)? as u8;
        let l1_mod = L1Modulation::from_u8(r.read_bits(L1_MOD_BITS)? as u8);
        let l1_cod = L1CodeRate::from_u8(r.read_bits(L1_COD_BITS)? as u8);
        let l1_fec_type = L1FecType::from_u8(r.read_bits(L1_FEC_TYPE_BITS)? as u8);
        let l1_post_size = r.read_bits(L1_POST_SIZE_BITS)? as u32;
        let l1_post_info_size = r.read_bits(L1_POST_INFO_SIZE_BITS)? as u32;
        let pilot_pattern = PilotPattern::from_u8(r.read_bits(PILOT_PATTERN_BITS)? as u8);
        let tx_id_availability = r.read_bits(TX_ID_AVAILABILITY_BITS)? as u8;
        let cell_id = r.read_bits(CELL_ID_BITS)? as u16;
        let network_id = r.read_bits(NETWORK_ID_BITS)? as u16;
        let t2_system_id = r.read_bits(T2_SYSTEM_ID_BITS)? as u16;
        let num_t2_frames = r.read_bits(NUM_T2_FRAMES_BITS)? as u8;
        let num_data_symbols = r.read_bits(NUM_DATA_SYMBOLS_BITS)? as u16;
        let regen_flag = r.read_bits(REGEN_FLAG_BITS)? as u8;
        let l1_post_extension = r.read_bits(L1_POST_EXTENSION_BITS)? != 0;
        let num_rf = r.read_bits(NUM_RF_BITS)? as u8;
        let current_rf_idx = r.read_bits(CURRENT_RF_IDX_BITS)? as u8;
        let t2_version = T2Version::from_u8(r.read_bits(T2_VERSION_BITS)? as u8);
        let l1_post_scrambled = r.read_bits(L1_POST_SCRAMBLED_BITS)? != 0;
        let t2_base_lite = r.read_bits(T2_BASE_LITE_BITS)? != 0;
        let reserved = r.read_bits(RESERVED_BITS)? as u8;

        debug_assert_eq!(r.bits_read(), L1PRE_BITS);

        Ok(L1Pre {
            type_,
            bwt_ext,
            s1,
            s2,
            l1_repetition_flag,
            guard_interval,
            papr,
            l1_mod,
            l1_cod,
            l1_fec_type,
            l1_post_size,
            l1_post_info_size,
            pilot_pattern,
            tx_id_availability,
            cell_id,
            network_id,
            t2_system_id,
            num_t2_frames,
            num_data_symbols,
            regen_flag,
            l1_post_extension,
            num_rf,
            current_rf_idx,
            t2_version,
            l1_post_scrambled,
            t2_base_lite,
            reserved,
        })
    }
}

impl Serialize for L1Pre {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        L1PRE_BYTES
    }

    fn serialize_into(&self, buf: &mut [u8]) -> crate::error::Result<usize> {
        need_out(buf, L1PRE_BYTES)?;
        // zero first to allow align_to_byte padding to be 0
        buf[..L1PRE_BYTES].fill(0);
        let mut w = BitWriter::new(&mut buf[..L1PRE_BYTES]);

        w.write_bits(u64::from(self.type_.to_u8()), TYPE_BITS)?;
        w.write_bits(u64::from(self.bwt_ext), BWT_EXT_BITS)?;
        w.write_bits(u64::from(u8::from(self.s1)), S1_BITS)?;
        w.write_bits(u64::from(self.s2), S2_BITS)?;
        w.write_bits(u64::from(self.l1_repetition_flag), L1_REPETITION_FLAG_BITS)?;
        w.write_bits(u64::from(self.guard_interval.to_u8()), GUARD_INTERVAL_BITS)?;
        w.write_bits(u64::from(self.papr), PAPR_BITS)?;
        w.write_bits(u64::from(self.l1_mod.to_u8()), L1_MOD_BITS)?;
        w.write_bits(u64::from(self.l1_cod.to_u8()), L1_COD_BITS)?;
        w.write_bits(u64::from(self.l1_fec_type.to_u8()), L1_FEC_TYPE_BITS)?;
        w.write_bits(u64::from(self.l1_post_size), L1_POST_SIZE_BITS)?;
        w.write_bits(u64::from(self.l1_post_info_size), L1_POST_INFO_SIZE_BITS)?;
        w.write_bits(u64::from(self.pilot_pattern.to_u8()), PILOT_PATTERN_BITS)?;
        w.write_bits(u64::from(self.tx_id_availability), TX_ID_AVAILABILITY_BITS)?;
        w.write_bits(u64::from(self.cell_id), CELL_ID_BITS)?;
        w.write_bits(u64::from(self.network_id), NETWORK_ID_BITS)?;
        w.write_bits(u64::from(self.t2_system_id), T2_SYSTEM_ID_BITS)?;
        w.write_bits(u64::from(self.num_t2_frames), NUM_T2_FRAMES_BITS)?;
        w.write_bits(u64::from(self.num_data_symbols), NUM_DATA_SYMBOLS_BITS)?;
        w.write_bits(u64::from(self.regen_flag), REGEN_FLAG_BITS)?;
        w.write_bits(u64::from(self.l1_post_extension), L1_POST_EXTENSION_BITS)?;
        w.write_bits(u64::from(self.num_rf), NUM_RF_BITS)?;
        w.write_bits(u64::from(self.current_rf_idx), CURRENT_RF_IDX_BITS)?;
        w.write_bits(u64::from(self.t2_version.to_u8()), T2_VERSION_BITS)?;
        w.write_bits(u64::from(self.l1_post_scrambled), L1_POST_SCRAMBLED_BITS)?;
        w.write_bits(u64::from(self.t2_base_lite), T2_BASE_LITE_BITS)?;
        w.write_bits(u64::from(self.reserved), RESERVED_BITS)?;

        debug_assert_eq!(w.bits_written(), L1PRE_BITS);
        Ok(L1PRE_BYTES)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_common::Parse;

    fn synthetic_pre() -> L1Pre {
        L1Pre {
            // TYPE=0x00 (TsOnly), BWT_EXT=0, S1=000 (T2_SISO), S2=0b0100
            // L1_REPETITION_FLAG=0, GUARD_INTERVAL=010 (1/8)
            // PAPR=0001 (ACE-PAPR only for V0), L1_MOD=0010 (16-QAM)
            // L1_COD=00 (1/2), L1_FEC_TYPE=00 (LDPC 16K)
            // L1_POST_SIZE=376, L1_POST_INFO_SIZE=318
            // PILOT_PATTERN=0010 (PP3), TX_ID_AVAILABILITY=0
            // CELL_ID=0x3003, NETWORK_ID=0x3003, T2_SYSTEM_ID=0x3003
            // NUM_T2_FRAMES=2, NUM_DATA_SYMBOLS=41, REGEN_FLAG=0
            // L1_POST_EXTENSION=0, NUM_RF=1, CURRENT_RF_IDX=0
            // T2_VERSION=0010 (v1.3.1), L1_POST_SCRAMBLED=0, T2_BASE_LITE=0
            // RESERVED=0
            type_: TxInputStreamType::TsOnly,
            bwt_ext: false,
            s1: S1Field::V0,
            s2: 0b0100,
            l1_repetition_flag: false,
            guard_interval: GuardInterval::G1_8,
            papr: 0,
            l1_mod: L1Modulation::Qam16,
            l1_cod: L1CodeRate::R1_2,
            l1_fec_type: L1FecType::Ldpc16K,
            l1_post_size: 376,
            l1_post_info_size: 318,
            pilot_pattern: PilotPattern::Pp3,
            tx_id_availability: 0,
            cell_id: 0x3003,
            network_id: 0x3003,
            t2_system_id: 0x3003,
            num_t2_frames: 2,
            num_data_symbols: 41,
            regen_flag: 0,
            l1_post_extension: false,
            num_rf: 1,
            current_rf_idx: 0,
            t2_version: T2Version::V1_3_1,
            l1_post_scrambled: false,
            t2_base_lite: false,
            reserved: 0,
        }
    }

    #[test]
    fn round_trip_parse_serialize() {
        let pre = synthetic_pre();
        // serialize → parse → equal
        let mut buf = [0u8; L1PRE_BYTES];
        pre.serialize_into(&mut buf).unwrap();
        let parsed = L1Pre::parse(&buf).unwrap();
        assert_eq!(pre, parsed);
    }

    #[test]
    fn serialize_parse_byte_identical() {
        let pre = synthetic_pre();
        let mut buf1 = [0u8; L1PRE_BYTES];
        pre.serialize_into(&mut buf1).unwrap();
        // parse the bytes back
        let parsed = L1Pre::parse(&buf1).unwrap();
        // re-serialize
        let mut buf2 = [0u8; L1PRE_BYTES];
        parsed.serialize_into(&mut buf2).unwrap();
        assert_eq!(buf1, buf2);
    }

    #[test]
    fn serialize_with_crc_appends_the_crc_word() {
        let pre = synthetic_pre();
        let bytes = pre.serialize_with_crc().unwrap();
        assert_eq!(bytes.len(), L1PRE_WITH_CRC_BYTES);
    }

    #[test]
    fn parse_with_crc_inverts_serialize_with_crc() {
        let pre = synthetic_pre();
        let bytes = pre.serialize_with_crc().unwrap();
        assert_eq!(L1Pre::parse_with_crc(&bytes).unwrap(), pre);
    }

    #[test]
    fn parse_with_crc_rejects_a_corrupted_crc_with_both_values() {
        let pre = synthetic_pre();
        let mut bytes = pre.serialize_with_crc().unwrap();
        let expected_crc = pre.crc32().unwrap();
        bytes[L1PRE_BYTES] ^= 0xFF;
        let flipped = u32::from_be_bytes([
            bytes[L1PRE_BYTES],
            bytes[L1PRE_BYTES + 1],
            bytes[L1PRE_BYTES + 2],
            bytes[L1PRE_BYTES + 3],
        ]);
        assert_eq!(
            L1Pre::parse_with_crc(&bytes),
            Err(crate::Error::CrcMismatch {
                computed: expected_crc,
                expected: flipped,
            })
        );
    }

    #[test]
    fn parse_with_crc_rejects_a_corrupted_info_block() {
        let pre = synthetic_pre();
        let mut bytes = pre.serialize_with_crc().unwrap();
        bytes[0] ^= 0x80;
        assert!(matches!(
            L1Pre::parse_with_crc(&bytes),
            Err(crate::Error::CrcMismatch { .. })
        ));
    }

    #[test]
    fn parse_with_crc_ignores_trailing_bytes_and_rejects_short_input() {
        let pre = synthetic_pre();
        let mut bytes = pre.serialize_with_crc().unwrap();
        assert!(matches!(
            L1Pre::parse_with_crc(&bytes[..L1PRE_WITH_CRC_BYTES - 1]),
            Err(crate::Error::BufferTooShort {
                need: L1PRE_WITH_CRC_BYTES,
                ..
            })
        ));
        bytes.extend_from_slice(&[0xAA, 0xBB, 0xCC]);
        assert_eq!(L1Pre::parse_with_crc(&bytes).unwrap(), pre);
    }

    #[test]
    fn parse_rejects_short_buffer() {
        assert_eq!(
            L1Pre::parse(&[0u8; 20]).err(),
            Some(crate::Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: 21,
                    have: 20,
                    what: "L1Pre"
                }
            ))
        );
    }

    /// W-T2-3: every field is `pub` and mutable; a caller can set one
    /// outside its wire bit width (`num_rf` is 3 bits, max 7). `to_bytes`
    /// must return `Err`, not panic via the old `.expect(...)`.
    #[test]
    fn to_bytes_rejects_out_of_range_field_instead_of_panicking() {
        let mut pre = synthetic_pre();
        pre.num_rf = 8; // 3-bit field: max value is 7
        assert_eq!(
            pre.to_bytes().err(),
            Some(crate::Error::L1Bits(
                broadcast_common::bits::BitError::ValueTooWide { value: 8, bits: 3 }
            )),
            "an out-of-range field must be rejected, not silently serialised or panic"
        );
    }

    /// Same bite via `crc32`/`serialize_with_crc`, which both go through
    /// `to_bytes` internally.
    #[test]
    fn crc32_and_serialize_with_crc_propagate_the_same_error() {
        let mut pre = synthetic_pre();
        pre.num_rf = 8;
        assert!(matches!(
            &(pre.crc32()),
            Err(crate::error::Error::L1Bits(_))
        ));
        assert!(matches!(
            &(pre.serialize_with_crc()),
            Err(crate::error::Error::L1Bits(_))
        ));
    }

    #[test]
    fn papr_reduction_v0() {
        let mut pre = synthetic_pre();
        pre.t2_version = T2Version::V1_1_1;
        pre.papr = 2; // TR-PAPR only
        assert_eq!(
            pre.papr_reduction(),
            PaprReduction::V0(PaprReductionV0::TrOnly)
        );
    }

    #[test]
    fn papr_reduction_vn() {
        let mut pre = synthetic_pre();
        pre.t2_version = T2Version::V1_3_1;
        pre.papr = 1; // L1AceAndAce
        assert_eq!(
            pre.papr_reduction(),
            PaprReduction::Vn(PaprReductionVn::L1AceAndAce)
        );
    }
}
