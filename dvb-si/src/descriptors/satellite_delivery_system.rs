//! Satellite Delivery System Descriptor — ETSI EN 300 468 §6.2.13.2 (tag 0x43).
//!
//! Carried inside the NIT's `transport_stream_loop`'s second descriptor loop.
//! Conveys carrier tuning parameters for a DVB-S / DVB-S2 transponder.

use super::cable_delivery_system::FecInner;
use super::descriptor_body;
use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// Descriptor tag for satellite_delivery_system_descriptor.
pub const TAG: u8 = 0x43;
const HEADER_LEN: usize = 2;
const BODY_LEN: u8 = 11;

/// Polarization (§6.2.13.2 Table 38).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum Polarization {
    /// Linear horizontal.
    LinearHorizontal,
    /// Linear vertical.
    LinearVertical,
    /// Circular left.
    CircularLeft,
    /// Circular right.
    CircularRight,
}

impl Polarization {
    #[must_use]
    /// Construct from a raw `u8`; every value maps to a variant (total, lossless).
    pub fn from_u8(v: u8) -> Self {
        match v & 0x03 {
            0 => Polarization::LinearHorizontal,
            1 => Polarization::LinearVertical,
            2 => Polarization::CircularLeft,
            _ => Polarization::CircularRight,
        }
    }

    #[must_use]
    /// Inverse of `from_u8`; `Self::Reserved` emits its stored value.
    pub fn to_u8(self) -> u8 {
        match self {
            Polarization::LinearHorizontal => 0,
            Polarization::LinearVertical => 1,
            Polarization::CircularLeft => 2,
            Polarization::CircularRight => 3,
        }
    }
}

/// Modulation system (§6.2.13.2 Table 40: DVB-S or DVB-S2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum ModulationSystem {
    /// DVB-S (first generation).
    DvbS,
    /// DVB-S2 (second generation).
    DvbS2,
}

/// Modulation type (§6.2.13.2 Table 41).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum ModulationType {
    /// Auto-detect.
    Auto,
    /// QPSK.
    Qpsk,
    /// 8PSK.
    Psk8,
    /// 16QAM.
    Qam16,
}

/// Roll-off factor (§6.2.13.2 Table 39, DVB-S2 only; also used by S2X
/// with extended values per Table 144).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum RollOff {
    /// 0.35 (DVB-S default).
    Alpha035,
    /// 0.25 (DVB-S2 common).
    Alpha025,
    /// 0.20 (DVB-S2 narrow).
    Alpha020,
    /// Reserved — carries the raw value for forward compatibility (2-bit for
    /// DVB-S/S2, 3-bit for S2X).
    Reserved(u8),
}

impl RollOff {
    #[must_use]
    /// Construct from a raw `u8`; every value maps to a variant (total, lossless).
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => RollOff::Alpha035,
            1 => RollOff::Alpha025,
            2 => RollOff::Alpha020,
            other => RollOff::Reserved(other),
        }
    }

    #[must_use]
    /// Inverse of `from_u8`; `Self::Reserved` emits its stored value.
    pub fn to_u8(self) -> u8 {
        match self {
            RollOff::Alpha035 => 0,
            RollOff::Alpha025 => 1,
            RollOff::Alpha020 => 2,
            RollOff::Reserved(v) => v,
        }
    }
}

/// Satellite Delivery System Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SatelliteDeliverySystemDescriptor {
    /// 32-bit BCD frequency in GHz (e.g. `0x01172500` = 11.72500 GHz = 11_725_000_000 Hz).
    pub frequency_bcd: u32,
    /// 16-bit BCD orbital position tenths of a degree (e.g. 0x1920 = 192.0°).
    pub orbital_position_bcd: u16,
    /// False = west, true = east.
    pub east: bool,
    /// Polarization.
    pub polarization: Polarization,
    /// DVB-S2 roll-off factor. Meaningful only when `modulation_system` is
    /// DVB-S2 (Table 37); for DVB-S the bits are reserved_zero_future_use and
    /// serialize emits them as 0 regardless of this field.
    pub roll_off: RollOff,
    /// Modulation system.
    pub modulation_system: ModulationSystem,
    /// Modulation type.
    pub modulation_type: ModulationType,
    /// 28-bit BCD symbol rate in Msym/s (e.g. 0x0275_000 = 27.500 Msym/s).
    pub symbol_rate_bcd: u32,
    /// 4-bit FEC inner code — ETSI EN 300 468 Table 36.
    pub fec_inner: FecInner,
}

impl SatelliteDeliverySystemDescriptor {
    /// Decode the 32-bit BCD `frequency` to Hz (10 kHz field resolution,
    /// EN 300 468 §6.2.13.2). `None` if the BCD nibbles are out of range.
    ///
    /// e.g. `0x0117_2500` → `11_725_000_000` Hz (11.72500 GHz).
    #[must_use]
    pub fn frequency_hz(&self) -> Option<u64> {
        dvb_common::bcd::bcd_to_decimal(u64::from(self.frequency_bcd), 8).map(|v| v * 10_000)
    }

    /// Set `frequency` from Hz, encoding to the 8-digit BCD field at the field's
    /// 10 kHz resolution (sub-10 kHz precision is truncated).
    ///
    /// # Errors
    /// [`ValueOutOfRange`](crate::Error::ValueOutOfRange) if the value
    /// exceeds the 8-digit BCD field.
    pub fn set_frequency_hz(&mut self, hz: u64) -> crate::Result<()> {
        self.frequency_bcd = super::encode_bcd_field(
            hz / 10_000,
            8,
            "SatelliteDeliverySystemDescriptor::frequency",
        )? as u32;
        Ok(())
    }

    /// Decode the 28-bit BCD `symbol_rate` to symbols/second (100 sym/s
    /// resolution). `None` if the BCD nibbles are out of range.
    ///
    /// e.g. `0x027_5000` → `27_500_000` (27.5 Msym/s).
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
        self.symbol_rate_bcd = super::encode_bcd_field(
            sps / 100,
            7,
            "SatelliteDeliverySystemDescriptor::symbol_rate",
        )? as u32;
        Ok(())
    }

    /// Decode the 16-bit BCD `orbital_position` to degrees (tenths resolution).
    /// `None` if the BCD nibbles are out of range. e.g. `0x1920` → `192.0`.
    #[must_use]
    pub fn orbital_position_deg(&self) -> Option<f64> {
        dvb_common::bcd::bcd_to_decimal(u64::from(self.orbital_position_bcd), 4)
            .map(|tenths| tenths as f64 / 10.0)
    }

    /// Set `orbital_position` in degrees, rounded to the field's tenth-degree
    /// resolution. The east/west `east` flag is a separate field.
    ///
    /// # Errors
    /// [`ValueOutOfRange`](crate::Error::ValueOutOfRange) if negative or
    /// beyond the 4-digit BCD field.
    pub fn set_orbital_position_deg(&mut self, deg: f64) -> crate::Result<()> {
        if !(0.0..=6_553.5).contains(&deg) {
            return Err(crate::Error::ValueOutOfRange {
                field: "SatelliteDeliverySystemDescriptor::orbital_position",
                reason: "degrees must be in 0.0..=6553.5",
            });
        }
        let tenths = (deg * 10.0).round() as u64;
        self.orbital_position_bcd = super::encode_bcd_field(
            tenths,
            4,
            "SatelliteDeliverySystemDescriptor::orbital_position",
        )? as u16;
        Ok(())
    }
}

impl<'a> Parse<'a> for SatelliteDeliverySystemDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "SatelliteDeliverySystemDescriptor",
            "expected tag 0x43",
        )?;

        if body.len() != BODY_LEN as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "descriptor_length must equal 11",
            });
        }

        // Frequency: 4 bytes BCD (GHz.MMMM)
        let frequency_bcd = u32::from_be_bytes([body[0], body[1], body[2], body[3]]);

        // Orbital position: 2 bytes BCD (tenths of a degree)
        let orbital_position_bcd = u16::from_be_bytes([body[4], body[5]]);

        // Flags byte 6: bit 7 = west_east_flag, bits 5-6 = polarization,
        // bits 3-4 = roll_off, bit 2 = modulation_system, bits 1-0 = modulation_type
        let flags = body[6];
        let east = (flags & 0x80) != 0;

        let pol_raw = (flags >> 5) & 0x03;
        let polarization = match pol_raw {
            0 => Polarization::LinearHorizontal,
            1 => Polarization::LinearVertical,
            2 => Polarization::CircularLeft,
            _ => Polarization::CircularRight,
        };

        let roll_raw = (flags >> 3) & 0x03;
        let roll_off = match roll_raw {
            0 => RollOff::Alpha035,
            1 => RollOff::Alpha025,
            2 => RollOff::Alpha020,
            v => RollOff::Reserved(v),
        };

        let mod_sys_raw = (flags >> 2) & 0x01;
        let modulation_system = match mod_sys_raw {
            0 => ModulationSystem::DvbS,
            _ => ModulationSystem::DvbS2,
        };

        let mod_type_raw = flags & 0x03;
        let modulation_type = match mod_type_raw {
            0 => ModulationType::Auto,
            1 => ModulationType::Qpsk,
            2 => ModulationType::Psk8,
            _ => ModulationType::Qam16,
        };

        // Symbol rate: 28-bit BCD packed into 4 bytes (3.5 bytes + 4-bit FEC)
        let symbol_rate_and_fec = u32::from_be_bytes([body[7], body[8], body[9], body[10]]);
        let symbol_rate_bcd = symbol_rate_and_fec >> 4;
        let fec_inner = FecInner::from_u8((symbol_rate_and_fec & 0x0F) as u8);

        Ok(SatelliteDeliverySystemDescriptor {
            frequency_bcd,
            orbital_position_bcd,
            east,
            polarization,
            roll_off,
            modulation_system,
            modulation_type,
            symbol_rate_bcd,
            fec_inner,
        })
    }
}

impl Serialize for SatelliteDeliverySystemDescriptor {
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

        // Frequency: 4 bytes BCD
        let freq_bytes = self.frequency_bcd.to_be_bytes();
        buf[2..6].copy_from_slice(&freq_bytes);

        // Orbital position: 2 bytes BCD
        let orb_bytes = self.orbital_position_bcd.to_be_bytes();
        buf[6..8].copy_from_slice(&orb_bytes);

        // Flags byte: combine west_east, polarization, roll_off, modulation_system, modulation_type
        let mut flags: u8 = 0;
        if self.east {
            flags |= 0x80;
        }
        flags |= match self.polarization {
            Polarization::LinearHorizontal => 0x00,
            Polarization::LinearVertical => 0x20,
            Polarization::CircularLeft => 0x40,
            Polarization::CircularRight => 0x60,
        };
        // Table 37: roll_off exists only when modulation_system == DVB-S2;
        // for DVB-S those 2 bits are reserved_zero_future_use and SHALL be 0.
        if self.modulation_system == ModulationSystem::DvbS2 {
            flags |= match self.roll_off {
                RollOff::Alpha035 => 0x00,
                RollOff::Alpha025 => 0x08,
                RollOff::Alpha020 => 0x10,
                RollOff::Reserved(v) => (v & 0x03) << 3,
            };
        }
        flags |= match self.modulation_system {
            ModulationSystem::DvbS => 0x00,
            ModulationSystem::DvbS2 => 0x04,
        };
        flags |= match self.modulation_type {
            ModulationType::Auto => 0x00,
            ModulationType::Qpsk => 0x01,
            ModulationType::Psk8 => 0x02,
            ModulationType::Qam16 => 0x03,
        };
        buf[8] = flags;

        // Symbol rate + FEC_inner: 28-bit BCD shifted left 4 bits, then OR with FEC.
        // Mask to 28 bits so an over-range value can't spill past the field.
        let sym_freq = ((self.symbol_rate_bcd & 0x0FFF_FFFF) << 4)
            | (u32::from(self.fec_inner.to_u8()) & 0x0F);
        let sym_bytes = sym_freq.to_be_bytes();
        buf[9..13].copy_from_slice(&sym_bytes);

        Ok(len)
    }
}
impl<'a> crate::traits::DescriptorDef<'a> for SatelliteDeliverySystemDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "SATELLITE_DELIVERY_SYSTEM";
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a valid 13-byte descriptor (2 header + 11 body) and confirm
    /// parse extracts frequency and orbital position correctly.
    #[test]
    fn parse_extracts_frequency_and_orbital_position() {
        // frequency: 11.72500 GHz → BCD 0x01 0x17 0x25 0x00 (§6.2.13.2)
        // orbital: 192.0° → BCD 0x19 0x20
        let raw: Vec<u8> = vec![
            TAG, BODY_LEN, 0x01, 0x17, 0x25, 0x00, // frequency
            0x19, 0x20, // orbital position
            0x00, // flags (all defaults: west, linear-h, alpha-035, DVB-S, auto)
            0x02, 0x75, 0x00, 0x00, // symbol rate 27.500 Msym/s, FEC 0
        ];
        let desc = SatelliteDeliverySystemDescriptor::parse(&raw).unwrap();
        assert_eq!(desc.frequency_bcd, 0x01172500);
        assert_eq!(desc.orbital_position_bcd, 0x1920);
    }

    /// Flags byte bit 7 encodes west/east direction.
    #[test]
    fn parse_extracts_west_east_flag() {
        // East: bit 7 = 1
        let raw_east: Vec<u8> = vec![
            TAG, BODY_LEN, 0x01, 0x17, 0x25, 0x00, 0x19, 0x20,
            0x80, // east flag set, everything else zero
            0x02, 0x75, 0x00, 0x00,
        ];
        let desc_east = SatelliteDeliverySystemDescriptor::parse(&raw_east).unwrap();
        assert!(desc_east.east, "east should be true when bit 7 is set");

        // West: bit 7 = 0
        let raw_west: Vec<u8> = vec![
            TAG, BODY_LEN, 0x01, 0x17, 0x25, 0x00, 0x19, 0x20, 0x00, // east flag clear
            0x02, 0x75, 0x00, 0x00,
        ];
        let desc_west = SatelliteDeliverySystemDescriptor::parse(&raw_west).unwrap();
        assert!(!desc_west.east, "east should be false when bit 7 is clear");
    }

    /// All four polarization values are extracted correctly from bits 5-6.
    #[test]
    fn parse_extracts_polarization_variants() {
        let pol_pairs: [(u8, Polarization); 4] = [
            (0x00, Polarization::LinearHorizontal),
            (0x20, Polarization::LinearVertical),
            (0x40, Polarization::CircularLeft),
            (0x60, Polarization::CircularRight),
        ];

        for (offset, expected_pol) in pol_pairs {
            let raw: Vec<u8> = vec![
                TAG, BODY_LEN, 0x01, 0x17, 0x25, 0x00, 0x19, 0x20,
                offset, // polarization bits
                0x02, 0x75, 0x00, 0x00,
            ];
            let desc = SatelliteDeliverySystemDescriptor::parse(&raw).unwrap();
            assert_eq!(
                desc.polarization, expected_pol,
                "polarization mismatch for offset 0x{:02x}",
                offset
            );
        }
    }

    /// Modulation system (bit 2) and modulation type (bits 1-0) are extracted.
    #[test]
    fn parse_extracts_modulation_system_and_type() {
        // DVB-S (bit 2 = 0), QPSK (bits 1-0 = 01)
        let raw: Vec<u8> = vec![
            TAG, BODY_LEN, 0x01, 0x17, 0x25, 0x00, 0x19, 0x20, 0x01, // DVB-S, QPSK
            0x02, 0x75, 0x00, 0x00,
        ];
        let desc = SatelliteDeliverySystemDescriptor::parse(&raw).unwrap();
        assert_eq!(desc.modulation_system, ModulationSystem::DvbS);
        assert_eq!(desc.modulation_type, ModulationType::Qpsk);

        // DVB-S2 (bit 2 = 1), 8PSK (bits 1-0 = 10)
        let raw2: Vec<u8> = vec![
            TAG, BODY_LEN, 0x01, 0x17, 0x25, 0x00, 0x19, 0x20,
            0x06, // DVB-S2 (0x04) + 8PSK (0x02)
            0x02, 0x75, 0x00, 0x00,
        ];
        let desc2 = SatelliteDeliverySystemDescriptor::parse(&raw2).unwrap();
        assert_eq!(desc2.modulation_system, ModulationSystem::DvbS2);
        assert_eq!(desc2.modulation_type, ModulationType::Psk8);
    }

    /// Roll-off codes (bits 3-4) are extracted correctly.
    #[test]
    fn parse_extracts_roll_off() {
        let roll_pairs: [(u8, RollOff); 4] = [
            (0x00, RollOff::Alpha035),
            (0x08, RollOff::Alpha025),
            (0x10, RollOff::Alpha020),
            (0x18, RollOff::Reserved(3)),
        ];

        for (offset, expected_roll) in roll_pairs {
            let raw: Vec<u8> = vec![
                TAG, BODY_LEN, 0x01, 0x17, 0x25, 0x00, 0x19, 0x20, offset, // roll-off bits
                0x02, 0x75, 0x00, 0x00,
            ];
            let desc = SatelliteDeliverySystemDescriptor::parse(&raw).unwrap();
            assert_eq!(desc.roll_off, expected_roll);
        }
    }

    /// Symbol rate (28-bit BCD) and FEC inner (4 bits) are extracted from
    /// the last 4 bytes.
    #[test]
    fn parse_extracts_symbol_rate_and_fec() {
        // symbol_rate: 27.500 Msym/s → BCD 0x027500, FEC: 5/6 → 0x4
        let raw: Vec<u8> = vec![
            TAG, BODY_LEN, 0x01, 0x17, 0x25, 0x00, 0x19, 0x20, 0x00, 0x02, 0x75, 0x00,
            0x04, // symbol_rate_bcd = 0x027500, fec_inner = 4 (Rate5_6)
        ];
        let desc = SatelliteDeliverySystemDescriptor::parse(&raw).unwrap();
        assert_eq!(desc.symbol_rate_bcd, 0x0275000);
        assert_eq!(desc.fec_inner, FecInner::Rate5_6);

        // FEC = 0x0F (NoConvCoding)
        let raw2: Vec<u8> = vec![
            TAG, BODY_LEN, 0x01, 0x17, 0x25, 0x00, 0x19, 0x20, 0x00, 0x02, 0x75, 0x00,
            0x0F, // FEC = 0x0F
        ];
        let desc2 = SatelliteDeliverySystemDescriptor::parse(&raw2).unwrap();
        assert_eq!(desc2.fec_inner, FecInner::NoConvCoding);
    }

    /// Wrong tag byte should return InvalidDescriptor.
    #[test]
    fn parse_rejects_wrong_tag() {
        let raw: Vec<u8> = vec![
            0x44, // wrong tag (cable delivery system)
            BODY_LEN, 0x01, 0x17, 0x25, 0x00, 0x19, 0x20, 0x00, 0x02, 0x75, 0x00, 0x00,
        ];
        let err = SatelliteDeliverySystemDescriptor::parse(&raw).unwrap_err();
        assert!(
            matches!(err, Error::InvalidDescriptor { tag: 0x44, .. }),
            "expected InvalidDescriptor(tag=0x44), got {err:?}"
        );
    }

    /// Body length must be exactly 11. Wrong length returns InvalidDescriptor.
    #[test]
    fn parse_rejects_wrong_length() {
        let raw: Vec<u8> = vec![
            TAG, 0x05, // wrong length (should be 11)
            0x11, 0x72, 0x50, 0x00, 0x19, 0x20,
        ];
        let err = SatelliteDeliverySystemDescriptor::parse(&raw).unwrap_err();
        assert!(
            matches!(
                err,
                Error::InvalidDescriptor {
                    reason: "descriptor_length must equal 11",
                    ..
                }
            ),
            "expected InvalidDescriptor about length, got {err:?}"
        );
    }

    /// Parse → serialize → re-parse should yield an equal struct and
    /// identical bytes.
    #[test]
    fn serialize_round_trip() {
        let desc = SatelliteDeliverySystemDescriptor {
            frequency_bcd: 0x01172500,
            orbital_position_bcd: 0x1920,
            east: true,
            polarization: Polarization::CircularRight,
            roll_off: RollOff::Alpha025,
            modulation_system: ModulationSystem::DvbS2,
            modulation_type: ModulationType::Psk8,
            symbol_rate_bcd: 0x027500,
            fec_inner: FecInner::Rate5_6,
        };

        let mut buf = vec![0u8; desc.serialized_len()];
        let written = desc.serialize_into(&mut buf).unwrap();
        assert_eq!(written, desc.serialized_len());

        let reparsed = SatelliteDeliverySystemDescriptor::parse(&buf).unwrap();
        assert_eq!(desc, reparsed);
    }

    /// Reserved roll-off value round-trips with its raw 2-bit value preserved.
    #[test]
    fn reserved_roll_off_round_trips() {
        let desc = SatelliteDeliverySystemDescriptor {
            frequency_bcd: 0x01172500,
            orbital_position_bcd: 0x1920,
            east: true,
            polarization: Polarization::CircularRight,
            roll_off: RollOff::Reserved(3),
            modulation_system: ModulationSystem::DvbS2,
            modulation_type: ModulationType::Psk8,
            symbol_rate_bcd: 0x027500,
            fec_inner: FecInner::Rate5_6,
        };

        let mut buf = vec![0u8; desc.serialized_len()];
        desc.serialize_into(&mut buf).unwrap();
        assert_eq!(buf[8] & 0x18, 0x18); // roll_off bits = 0b11

        let reparsed = SatelliteDeliverySystemDescriptor::parse(&buf).unwrap();
        assert_eq!(reparsed.roll_off, RollOff::Reserved(3));
    }

    #[test]
    fn frequency_hz_round_trip() {
        let mut desc = SatelliteDeliverySystemDescriptor {
            frequency_bcd: 0,
            orbital_position_bcd: 0x1920,
            east: true,
            polarization: Polarization::LinearHorizontal,
            roll_off: RollOff::Alpha035,
            modulation_system: ModulationSystem::DvbS,
            modulation_type: ModulationType::Auto,
            symbol_rate_bcd: 0x027500,
            fec_inner: FecInner::Rate5_6,
        };
        desc.set_frequency_hz(11_725_000_000).unwrap();
        assert_eq!(desc.frequency_hz(), Some(11_725_000_000));
        assert_eq!(desc.frequency_bcd, 0x01172500);
    }
}
