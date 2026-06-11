//! Frequency List Descriptor — ETSI EN 300 468 §6.2.17 (tag 0x62).
//!
//! Carried inside the NIT's transport_stream_loop second descriptor loop.
//! Enumerates alternative centre frequencies on which the TS can be found,
//! for handover when coverage changes.

use super::descriptor_body;
use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// Descriptor tag for frequency_list_descriptor.
pub const TAG: u8 = 0x62;
/// Length of the header (tag byte + length byte).
pub const HEADER_LEN: usize = 2;
/// Length of the coding_type byte inside the descriptor body.
pub const CODING_BYTE_LEN: usize = 1;
/// Length of a single frequency entry in bytes.
pub const ENTRY_LEN: usize = 4;
/// Mask for the coding_type bits (bottom 2 bits); the top 6 bits are reserved.
pub const CODING_TYPE_MASK: u8 = 0x03;
/// Reserved bits (top 6 of the coding byte). Ignored on parse, set to 1 on serialize.
pub const RESERVED_BITS_MASK: u8 = 0xFC;

/// Coding type selects the interpretation of each 4-byte frequency entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum CodingType {
    /// Not defined (coding_type = 0b00).
    Undefined,
    /// Satellite — 8 BCD digits, 10 kHz resolution (§6.2.13.2).
    Satellite,
    /// Cable — 8 BCD digits, 100 Hz resolution (§6.2.13.3).
    Cable,
    /// Terrestrial — binary uimsbf 32-bit, 10 Hz resolution (§6.2.13.4).
    Terrestrial,
}

/// Frequency List Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct FrequencyListDescriptor {
    /// Interpretation of every `centre_frequencies_bcd` entry.
    pub coding_type: CodingType,
    /// Raw 4-byte centre_frequency entries in wire order. Interpretation
    /// depends on `coding_type`: BCD for Satellite/Cable, binary for Terrestrial.
    pub centre_frequencies_bcd: Vec<[u8; 4]>,
}

impl FrequencyListDescriptor {
    /// Hz per BCD-decoded unit for Satellite or Cable, or `None` for
    /// Undefined / Terrestrial (Terrestrial uses binary encoding, not BCD).
    fn hz_per_unit_bcd(&self) -> Option<u64> {
        match self.coding_type {
            CodingType::Satellite => Some(10_000),
            CodingType::Cable => Some(100),
            CodingType::Terrestrial | CodingType::Undefined => None,
        }
    }

    /// Decode every `centre_frequencies_bcd` entry to Hz, interpreted per
    /// `coding_type`. Each element is `None` if `coding_type` is `Undefined`
    /// or a BCD nibble is invalid (Satellite/Cable).
    #[must_use]
    pub fn centre_frequencies_hz(&self) -> Vec<Option<u64>> {
        match self.coding_type {
            CodingType::Satellite | CodingType::Cable => {
                let scale = self.hz_per_unit_bcd().unwrap();
                self.centre_frequencies_bcd
                    .iter()
                    .map(|b| {
                        let value =
                            dvb_common::bcd::bcd_to_decimal(u64::from(u32::from_be_bytes(*b)), 8)?;
                        Some(value * scale)
                    })
                    .collect()
            }
            CodingType::Terrestrial => self
                .centre_frequencies_bcd
                .iter()
                .map(|b| Some(u64::from(u32::from_be_bytes(*b)) * 10))
                .collect(),
            CodingType::Undefined => self.centre_frequencies_bcd.iter().map(|_| None).collect(),
        }
    }

    /// Replace the entries by encoding each Hz value per `coding_type`
    /// (values truncate to the field's resolution).
    ///
    /// # Errors
    /// [`ValueOutOfRange`](crate::Error::ValueOutOfRange) if `coding_type`
    /// is `Undefined`, a BCD value exceeds 8 digits (Satellite/Cable), or
    /// a binary value exceeds 32 bits (Terrestrial).
    pub fn set_centre_frequencies_hz(&mut self, frequencies_hz: &[u64]) -> crate::Result<()> {
        match self.coding_type {
            CodingType::Satellite | CodingType::Cable => {
                let scale = self
                    .hz_per_unit_bcd()
                    .ok_or(crate::Error::ValueOutOfRange {
                        field: "FrequencyListDescriptor::centre_frequency",
                        reason: "coding_type is Undefined; cannot encode frequencies",
                    })?;
                let mut out = Vec::with_capacity(frequencies_hz.len());
                for &hz in frequencies_hz {
                    let bcd = super::encode_bcd_field(
                        hz / scale,
                        8,
                        "FrequencyListDescriptor::centre_frequency",
                    )?;
                    out.push((bcd as u32).to_be_bytes());
                }
                self.centre_frequencies_bcd = out;
                Ok(())
            }
            CodingType::Terrestrial => {
                let mut out = Vec::with_capacity(frequencies_hz.len());
                for &hz in frequencies_hz {
                    let units = hz / 10;
                    if units > u64::from(u32::MAX) {
                        return Err(Error::ValueOutOfRange {
                            field: "frequency_list centre_frequency",
                            reason: "terrestrial frequency exceeds the 32-bit (×10 Hz) wire field",
                        });
                    }
                    out.push((units as u32).to_be_bytes());
                }
                self.centre_frequencies_bcd = out;
                Ok(())
            }
            CodingType::Undefined => Err(crate::Error::ValueOutOfRange {
                field: "FrequencyListDescriptor::centre_frequency",
                reason: "coding_type is Undefined; cannot encode frequencies",
            }),
        }
    }
}

impl<'a> Parse<'a> for FrequencyListDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(bytes, TAG, "FrequencyListDescriptor", "expected tag 0x62")?;

        if body.len() < CODING_BYTE_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "body too short (need at least coding_type byte)",
            });
        }

        if (body.len() - CODING_BYTE_LEN) % ENTRY_LEN != 0 {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "body length minus coding byte must be multiple of 4",
            });
        }

        let coding_byte = body[0];
        // Top 6 bits are reserved_future_use — ignored on parse
        // (EN 300 468 §5.1: decoders shall ignore reserved bits).
        let coding_type_value = coding_byte & CODING_TYPE_MASK;
        let coding_type = match coding_type_value {
            0b00 => CodingType::Undefined,
            0b01 => CodingType::Satellite,
            0b10 => CodingType::Cable,
            _ => CodingType::Terrestrial,
        };

        let entry_count = (body.len() - CODING_BYTE_LEN) / ENTRY_LEN;
        let mut centre_frequencies_bcd = Vec::with_capacity(entry_count);

        let mut offset = CODING_BYTE_LEN;
        for _ in 0..entry_count {
            let mut entry = [0u8; ENTRY_LEN];
            entry.copy_from_slice(&body[offset..offset + ENTRY_LEN]);
            centre_frequencies_bcd.push(entry);
            offset += ENTRY_LEN;
        }

        Ok(FrequencyListDescriptor {
            coding_type,
            centre_frequencies_bcd,
        })
    }
}

impl Serialize for FrequencyListDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + CODING_BYTE_LEN + self.centre_frequencies_bcd.len() * ENTRY_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }

        let coding_type_bits = match self.coding_type {
            CodingType::Undefined => 0b00,
            CodingType::Satellite => 0b01,
            CodingType::Cable => 0b10,
            CodingType::Terrestrial => 0b11,
        };

        let body_length = CODING_BYTE_LEN + self.centre_frequencies_bcd.len() * ENTRY_LEN;

        buf[0] = TAG;
        buf[1] = body_length as u8;
        buf[HEADER_LEN] = RESERVED_BITS_MASK | coding_type_bits;

        let mut offset = HEADER_LEN + CODING_BYTE_LEN;
        for entry in &self.centre_frequencies_bcd {
            buf[offset..offset + ENTRY_LEN].copy_from_slice(entry);
            offset += ENTRY_LEN;
        }

        Ok(need)
    }
}
impl<'a> crate::traits::DescriptorDef<'a> for FrequencyListDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "FREQUENCY_LIST";
}

#[cfg(test)]
mod tests {
    use super::*;

    /// [TAG, 1, 0xFC] → zero entries, coding=Undefined
    #[test]
    fn parse_empty_entries_is_valid() {
        let raw: Vec<u8> = vec![TAG, 0x01, 0xFC];
        let desc = FrequencyListDescriptor::parse(&raw).unwrap();
        assert!(desc.centre_frequencies_bcd.is_empty());
        assert!(matches!(desc.coding_type, CodingType::Undefined));
    }

    /// coding_type byte 0xFD → Satellite (0xFC | 0b01)
    #[test]
    fn parse_extracts_coding_type_satellite() {
        let raw: Vec<u8> = vec![TAG, 0x01, 0xFD];
        let desc = FrequencyListDescriptor::parse(&raw).unwrap();
        assert!(matches!(desc.coding_type, CodingType::Satellite));
    }

    /// coding_type byte 0xFE → Cable (0xFC | 0b10)
    #[test]
    fn parse_extracts_coding_type_cable() {
        let raw: Vec<u8> = vec![TAG, 0x01, 0xFE];
        let desc = FrequencyListDescriptor::parse(&raw).unwrap();
        assert!(matches!(desc.coding_type, CodingType::Cable));
    }

    /// coding_type byte 0xFF → Terrestrial (0xFC | 0b11)
    #[test]
    fn parse_extracts_coding_type_terrestrial() {
        let raw: Vec<u8> = vec![TAG, 0x01, 0xFF];
        let desc = FrequencyListDescriptor::parse(&raw).unwrap();
        assert!(matches!(desc.coding_type, CodingType::Terrestrial));
    }

    /// Multiple 4-byte entries parsed correctly.
    #[test]
    fn parse_extracts_multiple_frequency_entries() {
        let raw: Vec<u8> = vec![
            TAG, 0x09, // body length = 9 (1 coding byte + 2 entries × 4)
            0xFD, // satellite
            0x00, 0x30, 0x12, 0x34, // BCD 00301234 → 3_012_340 kHz = 30_123_400_000 Hz
            0x00, 0x30, 0x00, 0x00, // BCD 00300000 → 3_000_000 kHz = 30_000_000_000 Hz
        ];
        let desc = FrequencyListDescriptor::parse(&raw).unwrap();
        assert_eq!(desc.centre_frequencies_bcd.len(), 2);
        assert_eq!(desc.centre_frequencies_bcd[0], [0x00, 0x30, 0x12, 0x34]);
        assert_eq!(desc.centre_frequencies_bcd[1], [0x00, 0x30, 0x00, 0x00]);
    }

    /// Wrong tag byte should return InvalidDescriptor.
    #[test]
    fn parse_rejects_wrong_tag() {
        let raw: Vec<u8> = vec![0x63, 0x01, 0xFC];
        let err = FrequencyListDescriptor::parse(&raw).unwrap_err();
        assert!(
            matches!(err, Error::InvalidDescriptor { tag: 0x63, .. }),
            "expected InvalidDescriptor(tag=0x63), got {err:?}"
        );
    }

    /// Reserved bits set to zero must be ignored, not rejected (EN 300 468 §5.1).
    #[test]
    fn parse_ignores_reserved_bits() {
        // coding byte 0x03: top 6 reserved bits = 0, coding_type = 0b11 (terrestrial).
        let raw: Vec<u8> = vec![TAG, 0x01, 0x03];
        let d = FrequencyListDescriptor::parse(&raw).unwrap();
        assert_eq!(d.coding_type, CodingType::Terrestrial);
        assert!(d.centre_frequencies_bcd.is_empty());
    }

    /// Body length not 1 + multiple of 4 → InvalidDescriptor
    #[test]
    fn parse_rejects_length_not_1_plus_multiple_of_4() {
        let raw: Vec<u8> = vec![TAG, 0x03, 0xFC, 0x01, 0x02]; // body=3, need 1+4K
        let err = FrequencyListDescriptor::parse(&raw).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { .. }));
    }

    /// Buffer shorter than the 2-byte header → BufferTooShort
    #[test]
    fn parse_rejects_truncated_buffer() {
        let raw: &[u8] = &[TAG];
        let err = FrequencyListDescriptor::parse(raw).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { need: 2, .. }));
    }

    /// Serialize a descriptor with zero entries, re-parse, compare.
    #[test]
    fn serialize_round_trip_empty() {
        let desc = FrequencyListDescriptor {
            coding_type: CodingType::Satellite,
            centre_frequencies_bcd: vec![],
        };
        let raw: Vec<u8> = vec![TAG, 0x01, 0xFD];
        let mut buf = vec![0u8; desc.serialized_len()];
        let written = desc.serialize_into(&mut buf).unwrap();
        assert_eq!(written, raw.len());
        assert_eq!(buf, raw);

        let reparsed = FrequencyListDescriptor::parse(&buf).unwrap();
        assert_eq!(desc.coding_type, reparsed.coding_type);
        assert_eq!(desc.centre_frequencies_bcd, reparsed.centre_frequencies_bcd);
    }

    /// Serialize a descriptor with many entries, re-parse, compare.
    #[test]
    fn serialize_round_trip_many_entries() {
        let desc = FrequencyListDescriptor {
            coding_type: CodingType::Cable,
            centre_frequencies_bcd: vec![
                [0x03, 0x46, 0x00, 0x00],
                [0x04, 0x74, 0x00, 0x10],
                [0x01, 0x15, 0x50, 0x00],
                [0x04, 0x90, 0x25, 0x00],
            ],
        };
        let mut buf = vec![0u8; desc.serialized_len()];
        desc.serialize_into(&mut buf).unwrap();
        let reparsed = FrequencyListDescriptor::parse(&buf).unwrap();
        assert_eq!(desc.coding_type, reparsed.coding_type);
        assert_eq!(desc.centre_frequencies_bcd, reparsed.centre_frequencies_bcd);
    }

    #[test]
    fn satellite_frequency_hz_decodes_correctly() {
        let desc = FrequencyListDescriptor {
            coding_type: CodingType::Satellite,
            centre_frequencies_bcd: vec![[0x01, 0x17, 0x25, 0x00]], // 11.72500 GHz
        };
        assert_eq!(desc.centre_frequencies_hz(), vec![Some(11_725_000_000)]);
    }

    #[test]
    fn cable_frequency_hz_decodes_correctly() {
        let desc = FrequencyListDescriptor {
            coding_type: CodingType::Cable,
            centre_frequencies_bcd: vec![[0x03, 0x46, 0x00, 0x00]], // 346.0000 MHz
        };
        assert_eq!(desc.centre_frequencies_hz(), vec![Some(346_000_000)]);
    }

    #[test]
    fn terrestrial_frequency_hz_decodes_binary() {
        let desc = FrequencyListDescriptor {
            coding_type: CodingType::Terrestrial,
            // binary 0x04A858F0 = 78_141_680 × 10 Hz = 781_416_800 Hz
            centre_frequencies_bcd: vec![[0x04, 0xA8, 0x58, 0xF0]],
        };
        assert_eq!(desc.centre_frequencies_hz(), vec![Some(781_416_800)]);
    }

    #[test]
    fn set_satellite_frequencies_hz_round_trips() {
        let mut desc = FrequencyListDescriptor {
            coding_type: CodingType::Satellite,
            centre_frequencies_bcd: vec![],
        };
        desc.set_centre_frequencies_hz(&[11_725_000_000]).unwrap();
        assert_eq!(desc.centre_frequencies_hz(), vec![Some(11_725_000_000)]);
        assert_eq!(desc.centre_frequencies_bcd[0], [0x01, 0x17, 0x25, 0x00]);
    }

    #[test]
    fn set_terrestrial_frequencies_hz_round_trips() {
        let mut desc = FrequencyListDescriptor {
            coding_type: CodingType::Terrestrial,
            centre_frequencies_bcd: vec![],
        };
        desc.set_centre_frequencies_hz(&[781_416_800]).unwrap();
        assert_eq!(desc.centre_frequencies_hz(), vec![Some(781_416_800)]);
        assert_eq!(desc.centre_frequencies_bcd[0], [0x04, 0xA8, 0x58, 0xF0]);
    }

    #[test]
    fn set_cable_frequencies_hz_round_trips() {
        let mut desc = FrequencyListDescriptor {
            coding_type: CodingType::Cable,
            centre_frequencies_bcd: vec![],
        };
        desc.set_centre_frequencies_hz(&[346_000_000]).unwrap();
        assert_eq!(desc.centre_frequencies_hz(), vec![Some(346_000_000)]);
    }
}
