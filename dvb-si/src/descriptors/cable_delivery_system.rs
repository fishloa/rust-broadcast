//! Cable Delivery System Descriptor — ETSI EN 300 468 §6.2.13.1 (tag 0x44).
//!
//! Carried inside NIT transport_stream_loop second descriptor loop for
//! DVB-C transponders.

use super::descriptor_body;
use crate::error::{Error, Result};
use crate::traits::Descriptor;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for cable_delivery_system_descriptor.
pub const TAG: u8 = 0x44;
const HEADER_LEN: usize = 2;
const BODY_LEN: u8 = 11;

/// Reserved future use bits (top 12 bits of bytes 6+7 in the descriptor body).
const RESERVED_FU_MASK: u16 = 0xFFF0;

/// FEC outer coding scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum FecOuter {
    /// Not defined.
    NotDefined,
    /// No outer FEC coding.
    NoOuterFec,
    /// Reed-Solomon (204, 188).
    ReedSolomon204_188,
    /// Reserved / future use.
    Reserved(u8),
}

/// Modulation scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum Modulation {
    /// Not defined.
    NotDefined,
    /// 16-QAM.
    Qam16,
    /// 32-QAM.
    Qam32,
    /// 64-QAM.
    Qam64,
    /// 128-QAM.
    Qam128,
    /// 256-QAM.
    Qam256,
    /// Reserved / future use.
    Reserved(u8),
}

/// FEC inner convolutional code rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum FecInner {
    /// Not defined.
    NotDefined,
    /// Code rate 1/2.
    Rate1_2,
    /// Code rate 2/3.
    Rate2_3,
    /// Code rate 3/4.
    Rate3_4,
    /// Code rate 5/6.
    Rate5_6,
    /// Code rate 7/8.
    Rate7_8,
    /// Code rate 8/9.
    Rate8_9,
    /// Code rate 3/5.
    Rate3_5,
    /// Code rate 4/5.
    Rate4_5,
    /// Code rate 9/10.
    Rate9_10,
    /// No convolutional coding.
    NoConvCoding,
    /// Reserved / future use.
    Reserved(u8),
}

/// Cable Delivery System Descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CableDeliverySystemDescriptor {
    /// 32-bit BCD frequency in 100 kHz (e.g. 0x03460000 = 346.0000 MHz).
    pub frequency_bcd: u32,
    /// FEC outer coding scheme.
    pub fec_outer: FecOuter,
    /// Modulation scheme.
    pub modulation: Modulation,
    /// 28-bit BCD symbol rate in Msym/s (value stored in low 28 bits of u32).
    pub symbol_rate_bcd: u32,
    /// FEC inner code rate.
    pub fec_inner: FecInner,
}

impl CableDeliverySystemDescriptor {
    /// Decode the 32-bit BCD `frequency` to Hz (100 Hz field resolution).
    /// `None` if the BCD nibbles are out of range.
    ///
    /// e.g. `0x0346_0000` → `346_000_000` Hz (346.0000 MHz).
    #[must_use]
    pub fn frequency_hz(&self) -> Option<u64> {
        dvb_common::bcd::bcd_to_decimal(u64::from(self.frequency_bcd), 8).map(|v| v * 100)
    }

    /// Set `frequency` from Hz, encoding to the 8-digit BCD field at the field's
    /// 100 Hz resolution (finer precision is truncated).
    ///
    /// # Errors
    /// [`ValueOutOfRange`](crate::Error::ValueOutOfRange) on overflow of
    /// the 8-digit BCD field.
    pub fn set_frequency_hz(&mut self, hz: u64) -> crate::Result<()> {
        self.frequency_bcd =
            super::encode_bcd_field(hz / 100, 8, "CableDeliverySystemDescriptor::frequency")?
                as u32;
        Ok(())
    }

    /// Decode the 28-bit BCD `symbol_rate` to symbols/second (100 sym/s
    /// resolution). `None` if the BCD nibbles are out of range.
    #[must_use]
    pub fn symbol_rate_sps(&self) -> Option<u64> {
        dvb_common::bcd::bcd_to_decimal(u64::from(self.symbol_rate_bcd), 7).map(|v| v * 100)
    }

    /// Set `symbol_rate` from symbols/second (100 sym/s field resolution).
    ///
    /// # Errors
    /// [`ValueOutOfRange`](crate::Error::ValueOutOfRange) on overflow of
    /// the 7-digit BCD field.
    pub fn set_symbol_rate_sps(&mut self, sps: u64) -> crate::Result<()> {
        self.symbol_rate_bcd =
            super::encode_bcd_field(sps / 100, 7, "CableDeliverySystemDescriptor::symbol_rate")?
                as u32;
        Ok(())
    }
}

fn parse_fec_outer(raw: u8) -> FecOuter {
    match raw {
        0x00 => FecOuter::NotDefined,
        0x01 => FecOuter::NoOuterFec,
        0x02 => FecOuter::ReedSolomon204_188,
        other => FecOuter::Reserved(other),
    }
}

fn parse_modulation(raw: u8) -> Modulation {
    match raw {
        0x00 => Modulation::NotDefined,
        0x01 => Modulation::Qam16,
        0x02 => Modulation::Qam32,
        0x03 => Modulation::Qam64,
        0x04 => Modulation::Qam128,
        0x05 => Modulation::Qam256,
        other => Modulation::Reserved(other),
    }
}

fn parse_fec_inner(raw: u8) -> FecInner {
    match raw {
        0x00 => FecInner::NotDefined,
        0x01 => FecInner::Rate1_2,
        0x02 => FecInner::Rate2_3,
        0x03 => FecInner::Rate3_4,
        0x04 => FecInner::Rate5_6,
        0x05 => FecInner::Rate7_8,
        0x06 => FecInner::Rate8_9,
        0x07 => FecInner::Rate3_5,
        0x08 => FecInner::Rate4_5,
        0x09 => FecInner::Rate9_10,
        0x0F => FecInner::NoConvCoding,
        other => FecInner::Reserved(other),
    }
}

fn serialize_fec_outer(fec: FecOuter) -> u8 {
    match fec {
        FecOuter::NotDefined => 0x00,
        FecOuter::NoOuterFec => 0x01,
        FecOuter::ReedSolomon204_188 => 0x02,
        FecOuter::Reserved(v) => v,
    }
}

fn serialize_modulation(m: Modulation) -> u8 {
    match m {
        Modulation::NotDefined => 0x00,
        Modulation::Qam16 => 0x01,
        Modulation::Qam32 => 0x02,
        Modulation::Qam64 => 0x03,
        Modulation::Qam128 => 0x04,
        Modulation::Qam256 => 0x05,
        Modulation::Reserved(v) => v,
    }
}

fn serialize_fec_inner(fec: FecInner) -> u8 {
    match fec {
        FecInner::NotDefined => 0x00,
        FecInner::Rate1_2 => 0x01,
        FecInner::Rate2_3 => 0x02,
        FecInner::Rate3_4 => 0x03,
        FecInner::Rate5_6 => 0x04,
        FecInner::Rate7_8 => 0x05,
        FecInner::Rate8_9 => 0x06,
        FecInner::Rate3_5 => 0x07,
        FecInner::Rate4_5 => 0x08,
        FecInner::Rate9_10 => 0x09,
        FecInner::NoConvCoding => 0x0F,
        FecInner::Reserved(v) => v,
    }
}

impl<'a> Parse<'a> for CableDeliverySystemDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "CableDeliverySystemDescriptor",
            "unexpected tag for cable_delivery_system_descriptor",
        )?;
        if body.len() != BODY_LEN as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "body length must equal 11",
            });
        }

        let frequency_bcd = u32::from_be_bytes(body[0..4].try_into().unwrap());

        let bytes_4_5 = u16::from_be_bytes([body[4], body[5]]);
        let fec_outer_raw = (bytes_4_5 & !RESERVED_FU_MASK) as u8;

        let modulation_byte = body[6];

        let spec_value = u32::from_be_bytes([0, body[7], body[8], body[9]]);
        let symbol_rate_bcd = (spec_value << 4) | ((body[10] >> 4) & 0x0F) as u32;

        let fec_inner_raw = body[10] & 0x0F;

        Ok(Self {
            frequency_bcd,
            fec_outer: parse_fec_outer(fec_outer_raw),
            modulation: parse_modulation(modulation_byte),
            symbol_rate_bcd,
            fec_inner: parse_fec_inner(fec_inner_raw),
        })
    }
}

impl Serialize for CableDeliverySystemDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + BODY_LEN as usize
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }

        buf[0] = TAG;
        buf[1] = BODY_LEN;

        buf[2..6].copy_from_slice(&self.frequency_bcd.to_be_bytes());

        let reserved_fu = RESERVED_FU_MASK;
        let fec_outer_byte = reserved_fu | serialize_fec_outer(self.fec_outer) as u16;
        let [fu_hi, fec_lo] = fec_outer_byte.to_be_bytes();
        buf[6] = fu_hi;
        buf[7] = fec_lo;

        buf[8] = serialize_modulation(self.modulation);

        let spec_value = self.symbol_rate_bcd >> 4;
        buf[9] = (spec_value >> 16) as u8;
        buf[10] = (spec_value >> 8) as u8;
        buf[11] = spec_value as u8;

        buf[12] = ((self.symbol_rate_bcd & 0x0F) as u8) << 4 | serialize_fec_inner(self.fec_inner);

        Ok(len)
    }
}

impl<'a> Descriptor<'a> for CableDeliverySystemDescriptor {
    const TAG: u8 = TAG;

    fn descriptor_length(&self) -> u8 {
        BODY_LEN
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for CableDeliverySystemDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "CABLE_DELIVERY_SYSTEM";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_frequency_bcd() {
        let raw: [u8; 13] = [
            TAG, BODY_LEN, 0x03, 0x46, 0x00, 0x00, 0xFF, 0xF1, 0x05, 0x00, 0x00, 0x00, 0x03,
        ];
        let d = CableDeliverySystemDescriptor::parse(&raw).unwrap();
        assert_eq!(d.frequency_bcd, 0x03460000);
    }

    #[test]
    fn parse_extracts_modulation_qam256() {
        let raw: [u8; 13] = [
            TAG, BODY_LEN, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xF0, 0x05, 0x00, 0x00, 0x00, 0x00,
        ];
        let d = CableDeliverySystemDescriptor::parse(&raw).unwrap();
        assert_eq!(d.modulation, Modulation::Qam256);
    }

    #[test]
    fn parse_extracts_fec_outer_reed_solomon() {
        let raw: [u8; 13] = [
            TAG, BODY_LEN, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xF2, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        let d = CableDeliverySystemDescriptor::parse(&raw).unwrap();
        assert_eq!(d.fec_outer, FecOuter::ReedSolomon204_188);
    }

    #[test]
    fn parse_extracts_fec_inner_rate_3_4() {
        let raw: [u8; 13] = [
            TAG, BODY_LEN, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xF0, 0x00, 0x00, 0x00, 0x00, 0x03,
        ];
        let d = CableDeliverySystemDescriptor::parse(&raw).unwrap();
        assert_eq!(d.fec_inner, FecInner::Rate3_4);
    }

    #[test]
    fn parse_extracts_symbol_rate_bcd() {
        let raw: [u8; 13] = [
            TAG, BODY_LEN, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xF0, 0x00, 0x06, 0x87, 0x50, 0x00,
        ];
        let d = CableDeliverySystemDescriptor::parse(&raw).unwrap();
        assert_eq!(d.symbol_rate_bcd, 0x0687500);
    }

    #[test]
    fn parse_preserves_reserved_modulation_in_reserved_variant() {
        let raw: [u8; 13] = [
            TAG, BODY_LEN, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xF0, 0x0A, 0x00, 0x00, 0x00, 0x00,
        ];
        let d = CableDeliverySystemDescriptor::parse(&raw).unwrap();
        assert_eq!(d.modulation, Modulation::Reserved(0x0A));
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let raw: [u8; 13] = [
            0x5B, BODY_LEN, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xF0, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        assert!(matches!(
            CableDeliverySystemDescriptor::parse(&raw).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x5B, .. }
        ));
    }

    #[test]
    fn parse_rejects_wrong_length() {
        // Declared length (12) exceeds available bytes → descriptor_body floors first.
        let raw: [u8; 13] = [
            TAG, 12, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xF0, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        let err = CableDeliverySystemDescriptor::parse(&raw).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));

        // Declared length fits the buffer but is not 11 → the body-length check bites.
        let raw: [u8; 12] = [TAG, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let err = CableDeliverySystemDescriptor::parse(&raw).unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidDescriptor {
                tag: TAG,
                reason: "body length must equal 11"
            }
        ));
    }

    #[test]
    fn serialize_round_trip() {
        let d = CableDeliverySystemDescriptor {
            frequency_bcd: 0x03460000,
            fec_outer: FecOuter::ReedSolomon204_188,
            modulation: Modulation::Qam256,
            symbol_rate_bcd: 0x0687500,
            fec_inner: FecInner::Rate3_4,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let parsed = CableDeliverySystemDescriptor::parse(&buf).unwrap();
        assert_eq!(parsed, d);
    }

    #[test]
    fn enum_round_trip_covers_every_defined_variant() {
        for fec_outer in [
            FecOuter::NotDefined,
            FecOuter::NoOuterFec,
            FecOuter::ReedSolomon204_188,
        ] {
            let v = serialize_fec_outer(fec_outer);
            assert_eq!(parse_fec_outer(v), fec_outer);
        }

        for mod_ in [
            Modulation::NotDefined,
            Modulation::Qam16,
            Modulation::Qam32,
            Modulation::Qam64,
            Modulation::Qam128,
            Modulation::Qam256,
        ] {
            let v = serialize_modulation(mod_);
            assert_eq!(parse_modulation(v), mod_);
        }

        for fec_inner in [
            FecInner::NotDefined,
            FecInner::Rate1_2,
            FecInner::Rate2_3,
            FecInner::Rate3_4,
            FecInner::Rate5_6,
            FecInner::Rate7_8,
            FecInner::Rate8_9,
            FecInner::Rate3_5,
            FecInner::Rate4_5,
            FecInner::Rate9_10,
            FecInner::NoConvCoding,
        ] {
            let v = serialize_fec_inner(fec_inner);
            assert_eq!(parse_fec_inner(v), fec_inner);
        }
    }
}
