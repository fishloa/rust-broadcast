//! cc_data() — ETSI TS 101 154 §B.5, Table B.9.

use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_core::len::{need_in, need_out};
use broadcast_core::{Parse, Serialize};

/// Largest `cc_count` the 5-bit field can carry.
const MAX_CC_COUNT: usize = 31;
/// Fixed `0xFF` reserved byte after cc_count, and the trailing marker byte.
const FF: u8 = 0xFF;
/// Byte 0 bit 7: `reserved (set to "1")` in TS 101 154 Table B.9, the
/// `process_em_data_flag` of the ATSC A/53 / CEA-708 lineage the syntax is
/// shared with.
const PROCESS_EM_DATA_FLAG_BIT: u8 = 0x80;
/// Byte 0 bit 6: `process_cc_data_flag`.
const PROCESS_CC_DATA_FLAG_BIT: u8 = 0x40;
/// Byte 0 bit 5: `zero_bit (set to "0")` in Table B.9, the
/// `additional_data_flag` of the A/53 / CEA-708 lineage.
const ADDITIONAL_DATA_FLAG_BIT: u8 = 0x20;
/// `cc_count` — byte 0 bits `[4:0]`.
const CC_COUNT_MASK: u8 = 0x1F;
/// Triplet byte 0 bits `[7:3]`: `one_bit`(1) + `reserved`(4), all set to 1
/// (TS 101 154 Table B.9); also the value they must hold.
const TRIPLET_FIXED_MASK: u8 = 0xF8;

/// Byte offsets of the cc_data() header (Table B.9).
const PROCESSING_BYTE_OFFSET: usize = 0;
const RESERVED_BYTE_OFFSET: usize = PROCESSING_BYTE_OFFSET + 1;
/// Byte length of the cc_data() header and of its trailing `marker_bits` byte.
const HEADER_LEN: usize = 2;
const MARKER_LEN: usize = 1;
/// Triplet layout: flags(1) + `cc_data_1`(1) + `cc_data_2`(1).
const TRIPLET_LEN: usize = 3;
const TRIPLET_FLAGS_OFFSET: usize = 0;
const CC_DATA_1_OFFSET: usize = TRIPLET_FLAGS_OFFSET + 1;
const CC_DATA_2_OFFSET: usize = CC_DATA_1_OFFSET + 1;
/// Bit shift and mask of `cc_valid` within a triplet's flags byte.
const CC_VALID_SHIFT: u32 = 2;
const CC_VALID_MASK: u8 = 0x01;
/// Mask of the 2-bit `cc_type` field.
const CC_TYPE_MASK: u8 = 0x03;

/// `cc_type` — the type of the caption data byte pair (TS 101 154 Table B.9 /
/// CEA-708-E). 2-bit field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum CcType {
    /// CEA-608 NTSC line-21 field 1 (cc_type 0).
    Ntsc608Field1,
    /// CEA-608 NTSC line-21 field 2 (cc_type 1).
    Ntsc608Field2,
    /// DTVCC (CEA-708) channel-packet data (cc_type 2).
    Dtvcc708Data,
    /// DTVCC (CEA-708) channel-packet start (cc_type 3).
    Dtvcc708Start,
}

impl CcType {
    /// From the 2-bit wire value.
    #[must_use]
    pub fn from_bits(v: u8) -> Self {
        match v & CC_TYPE_MASK {
            0 => Self::Ntsc608Field1,
            1 => Self::Ntsc608Field2,
            2 => Self::Dtvcc708Data,
            _ => Self::Dtvcc708Start,
        }
    }
    /// The 2-bit wire value.
    #[must_use]
    pub fn to_bits(self) -> u8 {
        match self {
            Self::Ntsc608Field1 => 0,
            Self::Ntsc608Field2 => 1,
            Self::Dtvcc708Data => 2,
            Self::Dtvcc708Start => 3,
        }
    }
    /// `true` for the CEA-608 (line-21) types.
    #[must_use]
    pub fn is_cea608(self) -> bool {
        matches!(self, Self::Ntsc608Field1 | Self::Ntsc608Field2)
    }
    /// `true` for the CEA-708 (DTVCC) types.
    #[must_use]
    pub fn is_cea708(self) -> bool {
        matches!(self, Self::Dtvcc708Data | Self::Dtvcc708Start)
    }
    /// Label per the project's `name()` convention.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Ntsc608Field1 => "ntsc_608_field1",
            Self::Ntsc608Field2 => "ntsc_608_field2",
            Self::Dtvcc708Data => "dtvcc_708_data",
            Self::Dtvcc708Start => "dtvcc_708_start",
        }
    }
}

broadcast_core::impl_spec_display!(CcType);

/// One closed-caption construct (the per-`cc_count` loop entry of Table B.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CcTriplet {
    /// `cc_valid` — the two caption bytes are valid.
    pub cc_valid: bool,
    /// `cc_type` — type of the caption byte pair.
    pub cc_type: CcType,
    /// `cc_data_1` — first caption byte (contents per CEA-708-E).
    pub cc_data_1: u8,
    /// `cc_data_2` — second caption byte.
    pub cc_data_2: u8,
}

/// `cc_data()` — the DVB closed-caption carriage structure (Table B.9).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CcData {
    /// Byte 0 bit 7. Table B.9 calls it `reserved (set to "1")`; in the ATSC
    /// A/53 / CEA-708 lineage the same bit is `process_em_data_flag`, and real
    /// A/53 captures (shaka-player's `captions-test.ts`) carry `0` here.
    /// Parsed tolerantly and preserved so the round-trip stays byte-exact;
    /// construct with `true` to match Table B.9's own default.
    pub process_em_data_flag: bool,
    /// `process_cc_data_flag` — when `true`, the cc_data is to be processed.
    pub process_cc_data_flag: bool,
    /// Byte 0 bit 5. Table B.9 calls it `zero_bit (set to "0")`; in the A/53 /
    /// CEA-708 lineage it is `additional_data_flag`. Parsed tolerantly and
    /// preserved; construct with `false` to match Table B.9's default.
    pub additional_data_flag: bool,
    /// The byte after `cc_count`, Table B.9's `reserved (set to "1111 1111")`.
    /// Real streams (this byte is `em_data` in the ATSC A/53 lineage this
    /// syntax is shared with) don't always hold `0xFF` here, so it is
    /// preserved verbatim rather than assumed, to keep the round-trip
    /// byte-exact. Construct with `0xFF` to match Table B.9's own default.
    pub reserved_byte1: u8,
    /// The caption constructs (`cc_count` is `triplets.len()`).
    pub triplets: Vec<CcTriplet>,
}

impl CcData {
    /// The triplets to process: all of them when `process_cc_data_flag` is
    /// set, none when it is clear (ETSI TS 101 154 Table B.9, see
    /// `private/specs-verified/captions/cc-data/ts_101_154/b9-cc-data.md`).
    #[must_use]
    pub fn processed_triplets(&self) -> &[CcTriplet] {
        if self.process_cc_data_flag {
            &self.triplets
        } else {
            &[]
        }
    }
    /// Iterator over the CEA-608 (line-21) triplets; empty when
    /// `process_cc_data_flag` is clear.
    pub fn cea608(&self) -> impl Iterator<Item = &CcTriplet> {
        self.processed_triplets()
            .iter()
            .filter(|t| t.cc_type.is_cea608())
    }
    /// Iterator over the CEA-708 (DTVCC) triplets; empty when
    /// `process_cc_data_flag` is clear.
    pub fn cea708(&self) -> impl Iterator<Item = &CcTriplet> {
        self.processed_triplets()
            .iter()
            .filter(|t| t.cc_type.is_cea708())
    }
}

impl<'a> Parse<'a> for CcData {
    type Error = Error;
    fn parse(b: &'a [u8]) -> Result<Self> {
        // byte0: reserved(1) | process_cc_data_flag(1) | zero_bit(1) | cc_count(5)
        // byte1: reserved 0xFF
        need_in(b, HEADER_LEN, "cc_data header")?;
        // byte0 bits 7 and 5 are reserved/zero in Table B.9 but carry
        // `process_em_data_flag` / `additional_data_flag` in real A/53
        // streams: accept either value and keep it (reserved-bit policy).
        let process_em_data_flag = b[PROCESSING_BYTE_OFFSET] & PROCESS_EM_DATA_FLAG_BIT != 0;
        let process_cc_data_flag = b[PROCESSING_BYTE_OFFSET] & PROCESS_CC_DATA_FLAG_BIT != 0;
        let additional_data_flag = b[PROCESSING_BYTE_OFFSET] & ADDITIONAL_DATA_FLAG_BIT != 0;
        let cc_count = usize::from(b[PROCESSING_BYTE_OFFSET] & CC_COUNT_MASK);
        let reserved_byte1 = b[RESERVED_BYTE_OFFSET];
        let total = HEADER_LEN + cc_count * TRIPLET_LEN + MARKER_LEN;
        need_in(b, total, "cc_data triplets")?;
        let mut triplets = Vec::with_capacity(cc_count);
        let mut pos = HEADER_LEN;
        for _ in 0..cc_count {
            let flags = b[pos];
            // one_bit(1) | reserved(4, set to "1111") | cc_valid(1) | cc_type(2)
            if flags & TRIPLET_FIXED_MASK != TRIPLET_FIXED_MASK {
                return Err(Error::ReservedBits(broadcast_core::error::ReservedBits {
                    field: "one_bit/reserved",
                    expected: u64::from(TRIPLET_FIXED_MASK),
                    found: u64::from(flags & TRIPLET_FIXED_MASK),
                }));
            }
            triplets.push(CcTriplet {
                cc_valid: (flags >> CC_VALID_SHIFT) & CC_VALID_MASK != 0,
                cc_type: CcType::from_bits(flags),
                cc_data_1: b[pos + CC_DATA_1_OFFSET],
                cc_data_2: b[pos + CC_DATA_2_OFFSET],
            });
            pos += TRIPLET_LEN;
        }
        if b[pos] != FF {
            return Err(Error::ReservedBits(broadcast_core::error::ReservedBits {
                field: "marker_bits",
                expected: u64::from(FF),
                found: u64::from(b[pos]),
            }));
        }
        Ok(CcData {
            process_em_data_flag,
            process_cc_data_flag,
            additional_data_flag,
            reserved_byte1,
            triplets,
        })
    }
}

impl Serialize for CcData {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + self.triplets.len() * TRIPLET_LEN + MARKER_LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if self.triplets.len() > MAX_CC_COUNT {
            return Err(Error::TooManyTriplets(self.triplets.len()));
        }
        let len = self.serialized_len();
        need_out(buf, len)?;
        // process_em_data_flag, process_cc_data_flag, additional_data_flag,
        // cc_count (<= MAX_CC_COUNT, checked above).
        let cc_count = self.triplets.len() as u8;
        buf[0] = (u8::from(self.process_em_data_flag) * PROCESS_EM_DATA_FLAG_BIT)
            | (u8::from(self.process_cc_data_flag) * PROCESS_CC_DATA_FLAG_BIT)
            | (u8::from(self.additional_data_flag) * ADDITIONAL_DATA_FLAG_BIT)
            | cc_count;
        buf[1] = self.reserved_byte1;
        let mut pos = HEADER_LEN;
        for t in &self.triplets {
            // one_bit=1, reserved=1111, cc_valid, cc_type
            buf[pos + TRIPLET_FLAGS_OFFSET] =
                TRIPLET_FIXED_MASK | (u8::from(t.cc_valid) << CC_VALID_SHIFT) | t.cc_type.to_bits();
            buf[pos + CC_DATA_1_OFFSET] = t.cc_data_1;
            buf[pos + CC_DATA_2_OFFSET] = t.cc_data_2;
            pos += TRIPLET_LEN;
        }
        buf[pos] = FF; // marker_bits
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tr(cc_valid: bool, cc_type: CcType, d1: u8, d2: u8) -> CcTriplet {
        CcTriplet {
            cc_valid,
            cc_type,
            cc_data_1: d1,
            cc_data_2: d2,
        }
    }

    fn sample() -> CcData {
        CcData {
            process_em_data_flag: true,
            process_cc_data_flag: true,
            additional_data_flag: false,
            reserved_byte1: FF,
            triplets: alloc::vec![
                tr(true, CcType::Dtvcc708Start, 0xC1, 0x02),
                tr(true, CcType::Ntsc608Field1, 0x94, 0x2C),
                tr(false, CcType::Dtvcc708Data, 0x00, 0x00),
            ],
        }
    }

    #[test]
    fn round_trip_constructed() {
        let cc = sample();
        let bytes = cc.try_to_bytes().unwrap();
        // construct → serialize → re-parse → equal (no raw stash to lean on)
        assert_eq!(CcData::parse(&bytes).unwrap(), cc);
        // header sanity: reserved=1, pcdf=1, zero=0, cc_count=3
        assert_eq!(bytes[0], 0b1100_0011);
        assert_eq!(bytes[1], 0xFF);
        assert_eq!(*bytes.last().unwrap(), 0xFF);
    }

    #[test]
    fn mutate_field_changes_output() {
        let a = sample().try_to_bytes().unwrap();
        let mut cc = sample();
        cc.triplets[0].cc_data_1 = 0x42;
        let b = cc.try_to_bytes().unwrap();
        assert_ne!(a, b, "mutating a field must change serialized bytes");
    }

    #[test]
    fn splits_608_708() {
        let cc = sample();
        assert_eq!(cc.cea608().count(), 1);
        assert_eq!(cc.cea708().count(), 2);
    }

    #[test]
    fn empty_round_trip() {
        let cc = CcData {
            process_em_data_flag: true,
            process_cc_data_flag: false,
            additional_data_flag: false,
            reserved_byte1: FF,
            triplets: alloc::vec![],
        };
        let bytes = cc.try_to_bytes().unwrap();
        assert_eq!(bytes, [0x80, 0xFF, 0xFF]); // reserved=1, pcdf=0, count=0; 0xFF; marker
        assert_eq!(CcData::parse(&bytes).unwrap(), cc);
    }

    #[test]
    fn reserved_byte1_round_trips_verbatim() {
        // A real-world (ATSC-lineage) stream where byte 1 after cc_count isn't
        // 0xFF: pre-fix, parse discarded it and serialize always wrote 0xFF,
        // so this round-trip was NOT byte-exact (bytes[1] silently changed).
        let mut cc = sample();
        cc.reserved_byte1 = 0x3A;
        let bytes = cc.try_to_bytes().unwrap();
        assert_eq!(bytes[1], 0x3A);
        let reparsed = CcData::parse(&bytes).unwrap();
        assert_eq!(reparsed.reserved_byte1, 0x3A);
        assert_eq!(reparsed, cc);
    }

    /// A real frame: the first `GA94` cc_data() of shaka-player's
    /// `captions-test.ts` (private submodule, `private/fixtures/captions/shaka/`; the media licence is unclear). Its header
    /// byte is `0x54`: `process_em_data_flag = 0`, `process_cc_data_flag = 1`,
    /// `additional_data_flag = 0`, `cc_count = 20`, and its em_data byte is
    /// `0x00`, not Table B.9's `0xFF`. A header check that insists on bit 7 = 1
    /// rejects every frame of that capture.
    const SHAKA_REAL_FRAME: &str = "5400fc94aefd8080ff4527fe9800fe0000fe0116fe1100fa0000fa0000fa0000fa0000fa0000fa0000fa0000fa0000fa0000fa0000fa0000fa0000fa0000ff";

    #[test]
    fn real_a53_header_with_clear_em_data_flag_parses_and_round_trips() {
        let bytes = test_fixtures::hex_bytes(SHAKA_REAL_FRAME);
        let cc = CcData::parse(&bytes).unwrap();
        assert!(!cc.process_em_data_flag);
        assert!(cc.process_cc_data_flag);
        assert!(!cc.additional_data_flag);
        assert_eq!(cc.reserved_byte1, 0x00);
        assert_eq!(cc.triplets.len(), 20);
        assert_eq!(cc.try_to_bytes().unwrap(), bytes);
    }

    #[test]
    fn additional_data_flag_set_is_preserved() {
        let mut bytes = sample().try_to_bytes().unwrap();
        bytes[0] |= 0x20; // additional_data_flag = 1 (Table B.9 "zero_bit")
        bytes[0] &= !0x80; // process_em_data_flag = 0
        let cc = CcData::parse(&bytes).unwrap();
        assert!(cc.additional_data_flag);
        assert!(!cc.process_em_data_flag);
        assert_eq!(cc.try_to_bytes().unwrap(), bytes);
    }

    #[test]
    fn hand_built_header_defaults_to_the_table_b9_values() {
        let bytes = sample().try_to_bytes().unwrap();
        assert_eq!(bytes[0] & 0xA0, 0x80);
    }

    #[test]
    fn parse_rejects_bad_triplet_fixed_bits() {
        let mut bytes = sample().try_to_bytes().unwrap();
        bytes[2] &= !0x80; // clear the triplet's "one_bit=1"
        assert!(matches!(
            CcData::parse(&bytes),
            Err(Error::ReservedBits(broadcast_core::error::ReservedBits {
                field: "one_bit/reserved",
                ..
            }))
        ));
    }

    #[test]
    fn parse_rejects_bad_marker_bits() {
        let mut bytes = sample().try_to_bytes().unwrap();
        let last = bytes.len() - 1;
        bytes[last] = 0x00;
        assert!(matches!(
            CcData::parse(&bytes),
            Err(Error::ReservedBits(broadcast_core::error::ReservedBits {
                field: "marker_bits",
                ..
            }))
        ));
    }

    #[test]
    fn serialize_rejects_over_31_triplets() {
        let cc = CcData {
            process_em_data_flag: true,
            process_cc_data_flag: true,
            additional_data_flag: false,
            reserved_byte1: FF,
            triplets: alloc::vec![tr(true, CcType::Ntsc608Field1, 0, 0); 32],
        };
        let mut buf = [0u8; 200];
        assert!(matches!(
            cc.serialize_into(&mut buf),
            Err(Error::TooManyTriplets(32))
        ));
    }

    #[test]
    fn exactly_31_triplets_is_the_maximum_and_round_trips() {
        let cc = CcData {
            process_em_data_flag: false,
            process_cc_data_flag: true,
            additional_data_flag: false,
            reserved_byte1: FF,
            triplets: alloc::vec![tr(true, CcType::Dtvcc708Data, 0x42, 0x99); 31],
        };
        let bytes = cc.try_to_bytes().unwrap();
        assert_eq!(bytes.len(), 2 + 31 * 3 + 1);
        assert_eq!(bytes[0] & CC_COUNT_MASK, 31);
        assert_eq!(CcData::parse(&bytes).unwrap(), cc);
    }

    #[test]
    fn cc_type_names_and_bit_mapping() {
        let table = [
            (CcType::Ntsc608Field1, 0u8, "ntsc_608_field1"),
            (CcType::Ntsc608Field2, 1, "ntsc_608_field2"),
            (CcType::Dtvcc708Data, 2, "dtvcc_708_data"),
            (CcType::Dtvcc708Start, 3, "dtvcc_708_start"),
        ];
        for (ty, bits, name) in table {
            assert_eq!(ty.to_bits(), bits);
            assert_eq!(CcType::from_bits(bits), ty);
            assert_eq!(ty.name(), name);
        }
    }

    #[test]
    fn bad_triplet_fixed_bits_report_the_exact_found_value() {
        let mut bytes = sample().try_to_bytes().unwrap();
        // Triplet 0 header for cc_type 3, cc_valid: 1111_1111 = 0xFF. Clear
        // bit 7 so the fixed 1111_1 prefix is 0111_1111 = 0x7F.
        bytes[2] = 0x7F;
        assert_eq!(
            CcData::parse(&bytes).unwrap_err(),
            Error::ReservedBits(broadcast_core::error::ReservedBits {
                field: "one_bit/reserved",
                expected: 0xF8,
                found: 0x78,
            })
        );
    }
}
