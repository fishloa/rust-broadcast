//! Terrestrial Delivery System Descriptor — ETSI EN 300 468 §6.2.13.4 (tag 0x5A).
//!
//! Carried inside the NIT's transport\_stream\_loop second descriptor loop for
//! DVB-T transponders. Expresses the full DVB-T PHY configuration needed to
//! tune the carrier.

use crate::error::{Error, Result};
use crate::traits::Descriptor;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for terrestrial\_delivery\_system\_descriptor.
pub const TAG: u8 = 0x5A;
const HEADER_LEN: usize = 2;
const BODY_LEN: u8 = 11;

const BW_SHIFT: u8 = 5;
const PRIORITY_MASK: u8 = 0b0001_0000;
const TIME_SLICING_MASK: u8 = 0b0000_1000;
const MPE_FEC_MASK: u8 = 0b0000_0100;
const RESERVED_FU_MASK: u8 = 0b0000_0011;
const BW_MASK: u8 = 0b1110_0000;

const CONSTELLATION_SHIFT: u8 = 6;
const HIERARCHY_SHIFT: u8 = 3;
const CONSTELLATION_MASK: u8 = 0b1100_0000;
const HIERARCHY_MASK: u8 = 0b0011_1000;
const CODE_RATE_HP_MASK: u8 = 0b0000_0111;

const CODE_RATE_LP_SHIFT: u8 = 5;
const GUARD_INTERVAL_SHIFT: u8 = 3;
const TRANSMISSION_MODE_SHIFT: u8 = 1;
const CODE_RATE_LP_MASK: u8 = 0b1110_0000;
const GUARD_INTERVAL_MASK: u8 = 0b0001_1000;
const TRANSMISSION_MODE_MASK: u8 = 0b0000_0110;
const OTHER_FREQ_FLAG_MASK: u8 = 0b0000_0001;

const TRAILING_RESERVED: u32 = 0xFFFF_FFFF;

/// Channel bandwidth (§6.2.13.4 Table 52).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Bandwidth {
    /// 8 MHz.
    Mhz8,
    /// 7 MHz.
    Mhz7,
    /// 6 MHz.
    Mhz6,
    /// 5 MHz.
    Mhz5,
    /// Unspecified / reserved value.
    Reserved(u8),
}

/// Constellation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Constellation {
    /// QPSK.
    Qpsk,
    /// 16-QAM.
    Qam16,
    /// 64-QAM.
    Qam64,
    /// Unspecified / reserved value.
    Reserved(u8),
}

/// Hierarchy mode — combines native/in-depth interleaver and α.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Hierarchy {
    /// Non-hierarchical + native.
    NonHierarchicalNative,
    /// α=1 + native.
    Alpha1Native,
    /// α=2 + native.
    Alpha2Native,
    /// α=4 + native.
    Alpha4Native,
    /// Non-hierarchical + in-depth.
    NonHierarchicalInDepth,
    /// α=1 + in-depth.
    Alpha1InDepth,
    /// α=2 + in-depth.
    Alpha2InDepth,
    /// α=4 + in-depth.
    Alpha4InDepth,
    /// Unspecified / reserved value.
    Reserved(u8),
}

/// Convolutional code rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CodeRate {
    /// 1/2.
    Rate1_2,
    /// 2/3.
    Rate2_3,
    /// 3/4.
    Rate3_4,
    /// 5/6.
    Rate5_6,
    /// 7/8.
    Rate7_8,
    /// Unspecified / reserved value.
    Reserved(u8),
}

/// Guard interval fraction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum GuardInterval {
    /// 1/32.
    G1_32,
    /// 1/16.
    G1_16,
    /// 1/8.
    G1_8,
    /// 1/4.
    G1_4,
}

/// Transmission mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TransmissionMode {
    /// 2k mode.
    Mode2k,
    /// 8k mode.
    Mode8k,
    /// 4k mode.
    Mode4k,
    /// Unspecified / reserved value.
    Reserved(u8),
}

/// Terrestrial Delivery System Descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TerrestrialDeliverySystemDescriptor {
    /// Centre frequency in units of 10 Hz.
    pub centre_frequency_10hz: u32,
    /// Channel bandwidth.
    pub bandwidth: Bandwidth,
    /// High-priority stream indicator.
    pub priority: bool,
    /// Time slicing used (spec field polarity: 0 = used → store as bool "used").
    pub time_slicing_used: bool,
    /// MPE-FEC used (spec field polarity: 0 = used → store as bool "used").
    pub mpe_fec_used: bool,
    /// Constellation.
    pub constellation: Constellation,
    /// Hierarchy mode.
    pub hierarchy: Hierarchy,
    /// High-priority stream FEC code rate.
    pub code_rate_hp: CodeRate,
    /// Low-priority stream FEC code rate (ignored for non-hierarchical).
    pub code_rate_lp: CodeRate,
    /// Guard interval fraction.
    pub guard_interval: GuardInterval,
    /// Transmission mode (FFT size).
    pub transmission_mode: TransmissionMode,
    /// Set when alternative frequencies listed in a frequency_list_descriptor.
    pub other_frequency_flag: bool,
}

fn parse_bandwidth(raw: u8) -> Bandwidth {
    match raw {
        0 => Bandwidth::Mhz8,
        1 => Bandwidth::Mhz7,
        2 => Bandwidth::Mhz6,
        3 => Bandwidth::Mhz5,
        other => Bandwidth::Reserved(other),
    }
}

fn parse_constellation(raw: u8) -> Constellation {
    match raw {
        0 => Constellation::Qpsk,
        1 => Constellation::Qam16,
        2 => Constellation::Qam64,
        other => Constellation::Reserved(other),
    }
}

fn parse_hierarchy(raw: u8) -> Hierarchy {
    match raw {
        0 => Hierarchy::NonHierarchicalNative,
        1 => Hierarchy::Alpha1Native,
        2 => Hierarchy::Alpha2Native,
        3 => Hierarchy::Alpha4Native,
        4 => Hierarchy::NonHierarchicalInDepth,
        5 => Hierarchy::Alpha1InDepth,
        6 => Hierarchy::Alpha2InDepth,
        7 => Hierarchy::Alpha4InDepth,
        other => Hierarchy::Reserved(other),
    }
}

fn parse_code_rate(raw: u8) -> CodeRate {
    match raw {
        0 => CodeRate::Rate1_2,
        1 => CodeRate::Rate2_3,
        2 => CodeRate::Rate3_4,
        3 => CodeRate::Rate5_6,
        4 => CodeRate::Rate7_8,
        other => CodeRate::Reserved(other),
    }
}

fn parse_guard_interval(raw: u8) -> GuardInterval {
    match raw {
        0 => GuardInterval::G1_32,
        1 => GuardInterval::G1_16,
        2 => GuardInterval::G1_8,
        3 => GuardInterval::G1_4,
        _ => GuardInterval::G1_32,
    }
}

fn parse_transmission_mode(raw: u8) -> TransmissionMode {
    match raw {
        0 => TransmissionMode::Mode2k,
        1 => TransmissionMode::Mode8k,
        2 => TransmissionMode::Mode4k,
        other => TransmissionMode::Reserved(other),
    }
}

fn serialize_bandwidth(bw: Bandwidth) -> u8 {
    match bw {
        Bandwidth::Mhz8 => 0,
        Bandwidth::Mhz7 => 1,
        Bandwidth::Mhz6 => 2,
        Bandwidth::Mhz5 => 3,
        Bandwidth::Reserved(v) => v,
    }
}

fn serialize_constellation(c: Constellation) -> u8 {
    match c {
        Constellation::Qpsk => 0,
        Constellation::Qam16 => 1,
        Constellation::Qam64 => 2,
        Constellation::Reserved(v) => v,
    }
}

fn serialize_hierarchy(h: Hierarchy) -> u8 {
    match h {
        Hierarchy::NonHierarchicalNative => 0,
        Hierarchy::Alpha1Native => 1,
        Hierarchy::Alpha2Native => 2,
        Hierarchy::Alpha4Native => 3,
        Hierarchy::NonHierarchicalInDepth => 4,
        Hierarchy::Alpha1InDepth => 5,
        Hierarchy::Alpha2InDepth => 6,
        Hierarchy::Alpha4InDepth => 7,
        Hierarchy::Reserved(v) => v,
    }
}

fn serialize_code_rate(cr: CodeRate) -> u8 {
    match cr {
        CodeRate::Rate1_2 => 0,
        CodeRate::Rate2_3 => 1,
        CodeRate::Rate3_4 => 2,
        CodeRate::Rate5_6 => 3,
        CodeRate::Rate7_8 => 4,
        CodeRate::Reserved(v) => v,
    }
}

fn serialize_guard_interval(gi: GuardInterval) -> u8 {
    match gi {
        GuardInterval::G1_32 => 0,
        GuardInterval::G1_16 => 1,
        GuardInterval::G1_8 => 2,
        GuardInterval::G1_4 => 3,
    }
}

fn serialize_transmission_mode(tm: TransmissionMode) -> u8 {
    match tm {
        TransmissionMode::Mode2k => 0,
        TransmissionMode::Mode8k => 1,
        TransmissionMode::Mode4k => 2,
        TransmissionMode::Reserved(v) => v,
    }
}

impl<'a> Parse<'a> for TerrestrialDeliverySystemDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN + BODY_LEN as usize {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN + BODY_LEN as usize,
                have: bytes.len(),
                what: "TerrestrialDeliverySystemDescriptor",
            });
        }
        if bytes[0] != TAG {
            return Err(Error::InvalidDescriptor {
                tag: bytes[0],
                reason: "unexpected tag for terrestrial_delivery_system_descriptor",
            });
        }
        let length = bytes[1];
        if length != BODY_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "body length must equal 11",
            });
        }

        let centre_frequency_10hz = u32::from_be_bytes(bytes[2..6].try_into().unwrap());

        let byte6 = bytes[6];
        let bw_raw = (byte6 & BW_MASK) >> BW_SHIFT;
        let priority = (byte6 & PRIORITY_MASK) != 0;
        let time_slicing_used = (byte6 & TIME_SLICING_MASK) == 0;
        let mpe_fec_used = (byte6 & MPE_FEC_MASK) == 0;

        let byte7 = bytes[7];
        let constellation_raw = (byte7 & CONSTELLATION_MASK) >> CONSTELLATION_SHIFT;
        let hierarchy_raw = (byte7 & HIERARCHY_MASK) >> HIERARCHY_SHIFT;
        let code_rate_hp_raw = byte7 & CODE_RATE_HP_MASK;

        let byte8 = bytes[8];
        let code_rate_lp_raw = (byte8 & CODE_RATE_LP_MASK) >> CODE_RATE_LP_SHIFT;
        let guard_interval_raw = (byte8 & GUARD_INTERVAL_MASK) >> GUARD_INTERVAL_SHIFT;
        let transmission_mode_raw = (byte8 & TRANSMISSION_MODE_MASK) >> TRANSMISSION_MODE_SHIFT;
        let other_frequency_flag = (byte8 & OTHER_FREQ_FLAG_MASK) != 0;

        Ok(Self {
            centre_frequency_10hz,
            bandwidth: parse_bandwidth(bw_raw),
            priority,
            time_slicing_used,
            mpe_fec_used,
            constellation: parse_constellation(constellation_raw),
            hierarchy: parse_hierarchy(hierarchy_raw),
            code_rate_hp: parse_code_rate(code_rate_hp_raw),
            code_rate_lp: parse_code_rate(code_rate_lp_raw),
            guard_interval: parse_guard_interval(guard_interval_raw),
            transmission_mode: parse_transmission_mode(transmission_mode_raw),
            other_frequency_flag,
        })
    }
}

impl Serialize for TerrestrialDeliverySystemDescriptor {
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

        buf[2..6].copy_from_slice(&self.centre_frequency_10hz.to_be_bytes());

        let byte6 = (serialize_bandwidth(self.bandwidth) << BW_SHIFT)
            | if self.priority { PRIORITY_MASK } else { 0 }
            | if !self.time_slicing_used {
                TIME_SLICING_MASK
            } else {
                0
            }
            | if !self.mpe_fec_used { MPE_FEC_MASK } else { 0 }
            | RESERVED_FU_MASK;
        buf[6] = byte6;

        let byte7 = (serialize_constellation(self.constellation) << CONSTELLATION_SHIFT)
            | (serialize_hierarchy(self.hierarchy) << HIERARCHY_SHIFT)
            | serialize_code_rate(self.code_rate_hp);
        buf[7] = byte7;

        let byte8 = (serialize_code_rate(self.code_rate_lp) << CODE_RATE_LP_SHIFT)
            | (serialize_guard_interval(self.guard_interval) << GUARD_INTERVAL_SHIFT)
            | (serialize_transmission_mode(self.transmission_mode) << TRANSMISSION_MODE_SHIFT)
            | if self.other_frequency_flag {
                OTHER_FREQ_FLAG_MASK
            } else {
                0
            };
        buf[8] = byte8;

        buf[9..13].copy_from_slice(&TRAILING_RESERVED.to_be_bytes());

        Ok(len)
    }
}

impl<'a> Descriptor<'a> for TerrestrialDeliverySystemDescriptor {
    const TAG: u8 = TAG;

    fn descriptor_length(&self) -> u8 {
        BODY_LEN
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_centre_frequency_10hz() {
        let raw = [
            TAG, BODY_LEN, 0x04, 0xA8, 0x58, 0xF0, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF,
        ];
        let d = TerrestrialDeliverySystemDescriptor::parse(&raw).unwrap();
        assert_eq!(d.centre_frequency_10hz, 0x04A858F0);
    }

    #[test]
    fn parse_extracts_bandwidth_8mhz() {
        let raw = [
            TAG, BODY_LEN, 0x04, 0xA8, 0x58, 0xF0, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF,
        ];
        let d = TerrestrialDeliverySystemDescriptor::parse(&raw).unwrap();
        assert_eq!(d.bandwidth, Bandwidth::Mhz8);
    }

    #[test]
    fn parse_extracts_bandwidth_7mhz() {
        let raw = [
            TAG,
            BODY_LEN,
            0x04,
            0xA8,
            0x58,
            0xF0,
            (0b001 << BW_SHIFT),
            0x00,
            0x00,
            0x00,
            0xFF,
            0xFF,
            0xFF,
            0xFF,
        ];
        let d = TerrestrialDeliverySystemDescriptor::parse(&raw).unwrap();
        assert_eq!(d.bandwidth, Bandwidth::Mhz7);
    }

    #[test]
    fn parse_extracts_constellation_qam64() {
        let raw = [
            TAG,
            BODY_LEN,
            0x04,
            0xA8,
            0x58,
            0xF0,
            0x00,
            (0b10 << CONSTELLATION_SHIFT),
            0x00,
            0xFF,
            0xFF,
            0xFF,
            0xFF,
        ];
        let d = TerrestrialDeliverySystemDescriptor::parse(&raw).unwrap();
        assert_eq!(d.constellation, Constellation::Qam64);
    }

    #[test]
    fn parse_extracts_code_rate_hp_and_lp() {
        let raw = [
            TAG,
            BODY_LEN,
            0x04,
            0xA8,
            0x58,
            0xF0,
            0x00,
            0b10 << CONSTELLATION_SHIFT,
            0b100 << CODE_RATE_LP_SHIFT,
            0xFF,
            0xFF,
            0xFF,
            0xFF,
        ];
        let d = TerrestrialDeliverySystemDescriptor::parse(&raw).unwrap();
        assert_eq!(d.code_rate_hp, CodeRate::Rate1_2);
        assert_eq!(d.code_rate_lp, CodeRate::Rate7_8);
    }

    #[test]
    fn parse_extracts_guard_interval_1_4() {
        let raw = [
            TAG,
            BODY_LEN,
            0x04,
            0xA8,
            0x58,
            0xF0,
            0x00,
            0x00,
            0b11 << GUARD_INTERVAL_SHIFT,
            0xFF,
            0xFF,
            0xFF,
            0xFF,
        ];
        let d = TerrestrialDeliverySystemDescriptor::parse(&raw).unwrap();
        assert_eq!(d.guard_interval, GuardInterval::G1_4);
    }

    #[test]
    fn parse_extracts_transmission_mode_8k() {
        let raw = [
            TAG,
            BODY_LEN,
            0x04,
            0xA8,
            0x58,
            0xF0,
            0x00,
            0x00,
            0b01 << TRANSMISSION_MODE_SHIFT,
            0xFF,
            0xFF,
            0xFF,
            0xFF,
        ];
        let d = TerrestrialDeliverySystemDescriptor::parse(&raw).unwrap();
        assert_eq!(d.transmission_mode, TransmissionMode::Mode8k);
    }

    #[test]
    fn parse_extracts_other_frequency_flag() {
        let raw = [
            TAG,
            BODY_LEN,
            0x04,
            0xA8,
            0x58,
            0xF0,
            0x00,
            0x00,
            OTHER_FREQ_FLAG_MASK,
            0xFF,
            0xFF,
            0xFF,
            0xFF,
        ];
        let d = TerrestrialDeliverySystemDescriptor::parse(&raw).unwrap();
        assert!(d.other_frequency_flag);
    }

    #[test]
    fn parse_preserves_reserved_bandwidth_in_reserve_variant() {
        let raw = [
            TAG,
            BODY_LEN,
            0x04,
            0xA8,
            0x58,
            0xF0,
            (0b111 << BW_SHIFT),
            0x00,
            0x00,
            0x00,
            0xFF,
            0xFF,
            0xFF,
            0xFF,
        ];
        let d = TerrestrialDeliverySystemDescriptor::parse(&raw).unwrap();
        assert_eq!(d.bandwidth, Bandwidth::Reserved(0b111));
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let raw = [
            0x5B, BODY_LEN, 0x04, 0xA8, 0x58, 0xF0, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF,
        ];
        assert!(matches!(
            TerrestrialDeliverySystemDescriptor::parse(&raw).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x5B, .. }
        ));
    }

    #[test]
    fn parse_rejects_wrong_length() {
        let raw = [
            TAG, 12, 0x04, 0xA8, 0x58, 0xF0, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF,
        ];
        assert!(matches!(
            TerrestrialDeliverySystemDescriptor::parse(&raw).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[test]
    fn serialize_round_trip_full_set() {
        let d = TerrestrialDeliverySystemDescriptor {
            centre_frequency_10hz: 0x04A858F0,
            bandwidth: Bandwidth::Mhz8,
            priority: true,
            time_slicing_used: false,
            mpe_fec_used: true,
            constellation: Constellation::Qam64,
            hierarchy: Hierarchy::Alpha2Native,
            code_rate_hp: CodeRate::Rate3_4,
            code_rate_lp: CodeRate::Rate7_8,
            guard_interval: GuardInterval::G1_4,
            transmission_mode: TransmissionMode::Mode8k,
            other_frequency_flag: true,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let parsed = TerrestrialDeliverySystemDescriptor::parse(&buf).unwrap();
        assert_eq!(parsed, d);
    }
}
