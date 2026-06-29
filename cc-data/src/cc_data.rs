//! cc_data() — ETSI TS 101 154 §B.5, Table B.9.

use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

/// Largest `cc_count` the 5-bit field can carry.
const MAX_CC_COUNT: usize = 31;
/// Fixed `0xFF` reserved byte after cc_count, and the trailing marker byte.
const FF: u8 = 0xFF;

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
        match v & 0x03 {
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

broadcast_common::impl_spec_display!(CcType);

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
    /// `process_cc_data_flag` — when `true`, the cc_data is to be processed.
    pub process_cc_data_flag: bool,
    /// The caption constructs (`cc_count` is `triplets.len()`).
    pub triplets: Vec<CcTriplet>,
}

impl CcData {
    /// Iterator over the CEA-608 (line-21) triplets.
    pub fn cea608(&self) -> impl Iterator<Item = &CcTriplet> {
        self.triplets.iter().filter(|t| t.cc_type.is_cea608())
    }
    /// Iterator over the CEA-708 (DTVCC) triplets.
    pub fn cea708(&self) -> impl Iterator<Item = &CcTriplet> {
        self.triplets.iter().filter(|t| t.cc_type.is_cea708())
    }
}

impl<'a> Parse<'a> for CcData {
    type Error = Error;
    fn parse(b: &'a [u8]) -> Result<Self> {
        // byte0: reserved(1) | process_cc_data_flag(1) | zero_bit(1) | cc_count(5)
        // byte1: reserved 0xFF
        if b.len() < 2 {
            return Err(Error::BufferTooShort {
                need: 2,
                have: b.len(),
                what: "cc_data header",
            });
        }
        let process_cc_data_flag = (b[0] >> 6) & 0x01 != 0;
        let cc_count = usize::from(b[0] & 0x1F);
        let total = 2 + cc_count * 3 + 1; // header + triplets + marker
        if b.len() < total {
            return Err(Error::BufferTooShort {
                need: total,
                have: b.len(),
                what: "cc_data triplets",
            });
        }
        let mut triplets = Vec::with_capacity(cc_count);
        let mut pos = 2;
        for _ in 0..cc_count {
            let flags = b[pos];
            // one_bit(1) | reserved(4) | cc_valid(1) | cc_type(2)
            triplets.push(CcTriplet {
                cc_valid: (flags >> 2) & 0x01 != 0,
                cc_type: CcType::from_bits(flags),
                cc_data_1: b[pos + 1],
                cc_data_2: b[pos + 2],
            });
            pos += 3;
        }
        Ok(CcData {
            process_cc_data_flag,
            triplets,
        })
    }
}

impl Serialize for CcData {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        2 + self.triplets.len() * 3 + 1
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if self.triplets.len() > MAX_CC_COUNT {
            return Err(Error::TooManyTriplets(self.triplets.len()));
        }
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        // reserved=1, process_cc_data_flag, zero_bit=0, cc_count
        let cc_count = self.triplets.len() as u8 & 0x1F;
        buf[0] = 0x80 | (u8::from(self.process_cc_data_flag) << 6) | cc_count;
        buf[1] = FF;
        let mut pos = 2;
        for t in &self.triplets {
            // one_bit=1, reserved=1111, cc_valid, cc_type
            buf[pos] = 0xF8 | (u8::from(t.cc_valid) << 2) | t.cc_type.to_bits();
            buf[pos + 1] = t.cc_data_1;
            buf[pos + 2] = t.cc_data_2;
            pos += 3;
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
            process_cc_data_flag: true,
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
        let bytes = cc.to_bytes();
        // construct → serialize → re-parse → equal (no raw stash to lean on)
        assert_eq!(CcData::parse(&bytes).unwrap(), cc);
        // header sanity: reserved=1, pcdf=1, zero=0, cc_count=3
        assert_eq!(bytes[0], 0b1100_0011);
        assert_eq!(bytes[1], 0xFF);
        assert_eq!(*bytes.last().unwrap(), 0xFF);
    }

    #[test]
    fn mutate_field_changes_output() {
        let a = sample().to_bytes();
        let mut cc = sample();
        cc.triplets[0].cc_data_1 = 0x42;
        let b = cc.to_bytes();
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
            process_cc_data_flag: false,
            triplets: alloc::vec![],
        };
        let bytes = cc.to_bytes();
        assert_eq!(bytes, [0x80, 0xFF, 0xFF]); // reserved=1, pcdf=0, count=0; 0xFF; marker
        assert_eq!(CcData::parse(&bytes).unwrap(), cc);
    }

    #[test]
    fn serialize_rejects_over_31_triplets() {
        let cc = CcData {
            process_cc_data_flag: true,
            triplets: alloc::vec![tr(true, CcType::Ntsc608Field1, 0, 0); 32],
        };
        let mut buf = [0u8; 200];
        assert!(matches!(
            cc.serialize_into(&mut buf),
            Err(Error::TooManyTriplets(32))
        ));
    }
}
