//! S2 Satellite Delivery System Descriptor — ETSI EN 300 468 §6.2.13.3 (tag 0x79).
//!
//! Companion to the satellite_delivery_system_descriptor (tag 0x43) when the
//! carrier is DVB-S2. Carries a PLS Gold scrambling sequence index and/or
//! an input_stream_identifier (ISI) for multi-stream carriers.
//!
//! Wire layout (all fields variable, conditional on flag bits):
//!
//! ```text
//!  byte 0 (flags):
//!    bit 7   scrambling_sequence_selector
//!    bit 6   multiple_input_stream_flag
//!    bit 5   backwards_compatibility_indicator
//!    bits 4..0  reserved_future_use (must be 0b11111)
//!
//!  if scrambling_sequence_selector == 1 (3 bytes):
//!    byte 1 bits 7..2  reserved_future_use (must be 0b111111)
//!    byte 1 bits 1..0 | byte 2 | byte 3  scrambling_sequence_index (18 bits)
//!
//!  if multiple_input_stream_flag == 1 (1 byte):
//!    input_stream_identifier (8 bits)
//! ```

use crate::error::{Error, Result};
use crate::traits::Descriptor;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for s2_satellite_delivery_system_descriptor.
pub const TAG: u8 = 0x79;
const HEADER_LEN: usize = 2;
const FLAGS_LEN: usize = 1;
const SCRAMBLING_FIELD_LEN: usize = 3;
const ISI_FIELD_LEN: usize = 1;

const FLAG_SCRAMBLING_SELECTOR: u8 = 0x80;
const FLAG_MULTIPLE_INPUT_STREAM: u8 = 0x40;
const FLAG_BACKWARDS_COMPATIBLE: u8 = 0x20;
const FLAG_RESERVED_BITS_MASK: u8 = 0x1F;

const SCRAMBLING_RESERVED_MASK: u8 = 0xFC;
const SCRAMBLING_INDEX_HI_MASK: u8 = 0x03;
const SCRAMBLING_INDEX_MAX: u32 = 0x3FFFF;

/// S2 Satellite Delivery System Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S2SatelliteDeliverySystemDescriptor {
    /// When set, `scrambling_sequence_index` is present.
    pub scrambling_sequence_selector: bool,
    /// When set, `input_stream_identifier` is present.
    pub multiple_input_stream_flag: bool,
    /// DVB-S2 backwards-compatible mode indicator.
    pub backwards_compatibility_indicator: bool,
    /// 18-bit PLS Gold scrambling sequence index.
    pub scrambling_sequence_index: Option<u32>,
    /// 8-bit input_stream_identifier (ISI).
    pub input_stream_identifier: Option<u8>,
}

impl<'a> Parse<'a> for S2SatelliteDeliverySystemDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN + FLAGS_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN + FLAGS_LEN,
                have: bytes.len(),
                what: "S2SatelliteDeliverySystemDescriptor header",
            });
        }
        if bytes[0] != TAG {
            return Err(Error::InvalidDescriptor {
                tag: bytes[0],
                reason: "unexpected tag for s2_satellite_delivery_system_descriptor",
            });
        }
        let length = bytes[1] as usize;
        let end = HEADER_LEN + length;
        if bytes.len() < end {
            return Err(Error::BufferTooShort {
                need: end,
                have: bytes.len(),
                what: "S2SatelliteDeliverySystemDescriptor body",
            });
        }
        if length < FLAGS_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "body must contain at least the flags byte",
            });
        }

        let flags = bytes[HEADER_LEN];
        let scrambling_sequence_selector = (flags & FLAG_SCRAMBLING_SELECTOR) != 0;
        let multiple_input_stream_flag = (flags & FLAG_MULTIPLE_INPUT_STREAM) != 0;
        let backwards_compatibility_indicator = (flags & FLAG_BACKWARDS_COMPATIBLE) != 0;

        if flags & FLAG_RESERVED_BITS_MASK != FLAG_RESERVED_BITS_MASK {
            return Err(Error::ReservedBitsViolation {
                field: "s2_satellite_delivery flags byte bits 4..0",
                reason: "reserved_future_use must be 0b11111",
            });
        }

        let mut pos = HEADER_LEN + FLAGS_LEN;

        let scrambling_sequence_index = if scrambling_sequence_selector {
            if pos + SCRAMBLING_FIELD_LEN > end {
                return Err(Error::BufferTooShort {
                    need: pos + SCRAMBLING_FIELD_LEN - HEADER_LEN,
                    have: length,
                    what: "S2SatelliteDeliverySystemDescriptor scrambling_sequence_index",
                });
            }
            let b0 = bytes[pos];
            let b1 = bytes[pos + 1];
            let b2 = bytes[pos + 2];
            if b0 & SCRAMBLING_RESERVED_MASK != SCRAMBLING_RESERVED_MASK {
                return Err(Error::ReservedBitsViolation {
                    field: "scrambling_sequence_index reserved 6 bits",
                    reason: "must be 0b111111",
                });
            }
            let index = (u32::from(b0 & SCRAMBLING_INDEX_HI_MASK) << 16)
                | (u32::from(b1) << 8)
                | u32::from(b2);
            pos += SCRAMBLING_FIELD_LEN;
            Some(index)
        } else {
            None
        };

        let input_stream_identifier = if multiple_input_stream_flag {
            if pos + ISI_FIELD_LEN > end {
                return Err(Error::BufferTooShort {
                    need: pos + ISI_FIELD_LEN - HEADER_LEN,
                    have: length,
                    what: "S2SatelliteDeliverySystemDescriptor input_stream_identifier",
                });
            }
            let isi = bytes[pos];
            Some(isi)
        } else {
            None
        };

        Ok(Self {
            scrambling_sequence_selector,
            multiple_input_stream_flag,
            backwards_compatibility_indicator,
            scrambling_sequence_index,
            input_stream_identifier,
        })
    }
}

impl Serialize for S2SatelliteDeliverySystemDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN
            + FLAGS_LEN
            + if self.scrambling_sequence_selector {
                SCRAMBLING_FIELD_LEN
            } else {
                0
            }
            + if self.multiple_input_stream_flag {
                ISI_FIELD_LEN
            } else {
                0
            }
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
        buf[1] = (len - HEADER_LEN) as u8;

        let mut flags = FLAG_RESERVED_BITS_MASK;
        if self.scrambling_sequence_selector {
            flags |= FLAG_SCRAMBLING_SELECTOR;
        }
        if self.multiple_input_stream_flag {
            flags |= FLAG_MULTIPLE_INPUT_STREAM;
        }
        if self.backwards_compatibility_indicator {
            flags |= FLAG_BACKWARDS_COMPATIBLE;
        }
        buf[HEADER_LEN] = flags;

        let mut pos = HEADER_LEN + FLAGS_LEN;
        if self.scrambling_sequence_selector {
            let index = self.scrambling_sequence_index.unwrap_or(0) & SCRAMBLING_INDEX_MAX;
            buf[pos] = SCRAMBLING_RESERVED_MASK | ((index >> 16) as u8 & SCRAMBLING_INDEX_HI_MASK);
            buf[pos + 1] = (index >> 8) as u8;
            buf[pos + 2] = index as u8;
            pos += SCRAMBLING_FIELD_LEN;
        }
        if self.multiple_input_stream_flag {
            buf[pos] = self.input_stream_identifier.unwrap_or(0);
        }
        Ok(len)
    }
}

impl<'a> Descriptor<'a> for S2SatelliteDeliverySystemDescriptor {
    const TAG: u8 = TAG;
    fn descriptor_length(&self) -> u8 {
        (self.serialized_len() - HEADER_LEN) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_just_flags_byte() {
        let raw = [TAG, 1, 0x1F];
        let d = S2SatelliteDeliverySystemDescriptor::parse(&raw).unwrap();
        assert!(!d.scrambling_sequence_selector);
        assert!(!d.multiple_input_stream_flag);
        assert!(!d.backwards_compatibility_indicator);
        assert_eq!(d.scrambling_sequence_index, None);
        assert_eq!(d.input_stream_identifier, None);
    }

    #[test]
    fn parse_extracts_backwards_compatibility_indicator() {
        // flags = BC(0x20) | reserved(0x1F) = 0x3F
        let raw = [TAG, 1, 0x3F];
        let d = S2SatelliteDeliverySystemDescriptor::parse(&raw).unwrap();
        assert!(d.backwards_compatibility_indicator);
    }

    #[test]
    fn parse_extracts_multiple_input_stream_flag_with_isi() {
        // flags = MIS(0x40) | reserved(0x1F) = 0x5F; ISI byte = 0x05
        let raw = [TAG, 2, 0x5F, 0x05];
        let d = S2SatelliteDeliverySystemDescriptor::parse(&raw).unwrap();
        assert!(d.multiple_input_stream_flag);
        assert_eq!(d.input_stream_identifier, Some(5));
        assert_eq!(d.scrambling_sequence_index, None);
    }

    #[test]
    fn parse_extracts_scrambling_sequence_selector_with_index() {
        // flags = scrambling(0x80) | reserved(0x1F) = 0x9F
        // scrambling field: byte0 = 0xFC | index_hi2, byte1 = index_mid8, byte2 = index_lo8
        // want index = 0x12345 → high 2 bits = 01, mid 8 = 0x23, lo 8 = 0x45
        // byte0 = 0xFC | 0x01 = 0xFD; byte1 = 0x23; byte2 = 0x45
        let raw = [TAG, 4, 0x9F, 0xFD, 0x23, 0x45];
        let d = S2SatelliteDeliverySystemDescriptor::parse(&raw).unwrap();
        assert!(d.scrambling_sequence_selector);
        assert_eq!(d.scrambling_sequence_index, Some(0x12345));
        assert!(!d.multiple_input_stream_flag);
    }

    #[test]
    fn parse_extracts_both_scrambling_and_isi() {
        // flags = scrambling(0x80) | MIS(0x40) | BC(0x20) | reserved(0x1F) = 0xFF
        // scrambling index 0x12345 + ISI 0x42
        let raw = [TAG, 5, 0xFF, 0xFD, 0x23, 0x45, 0x42];
        let d = S2SatelliteDeliverySystemDescriptor::parse(&raw).unwrap();
        assert!(d.scrambling_sequence_selector);
        assert!(d.multiple_input_stream_flag);
        assert!(d.backwards_compatibility_indicator);
        assert_eq!(d.scrambling_sequence_index, Some(0x12345));
        assert_eq!(d.input_stream_identifier, Some(0x42));
    }

    #[test]
    fn parse_extracts_scrambling_sequence_index_full_18_bit_range() {
        // index = 0x3FFFF (all 18 bits set).
        // byte0 = 0xFC | 0x03 = 0xFF; byte1 = 0xFF; byte2 = 0xFF
        let raw = [TAG, 4, 0x9F, 0xFF, 0xFF, 0xFF];
        let d = S2SatelliteDeliverySystemDescriptor::parse(&raw).unwrap();
        assert_eq!(d.scrambling_sequence_index, Some(0x3FFFF));
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let raw = [0x44, 1, 0x1F];
        assert!(matches!(
            S2SatelliteDeliverySystemDescriptor::parse(&raw).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x44, .. }
        ));
    }

    #[test]
    fn parse_rejects_length_too_short_for_declared_flags() {
        // scrambling flag set but length=1 covers only the flags byte.
        let raw = [TAG, 1, 0x9F];
        assert!(matches!(
            S2SatelliteDeliverySystemDescriptor::parse(&raw).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn parse_rejects_bad_reserved_bits_in_flags() {
        // reserved bits 4..0 must be 0b11111 — flags=0x80 has them zero.
        let raw = [TAG, 4, 0x80, 0xFC, 0x00, 0x00];
        assert!(matches!(
            S2SatelliteDeliverySystemDescriptor::parse(&raw).unwrap_err(),
            Error::ReservedBitsViolation { .. }
        ));
    }

    #[test]
    fn serialize_round_trip_minimal() {
        let d = S2SatelliteDeliverySystemDescriptor {
            scrambling_sequence_selector: false,
            multiple_input_stream_flag: false,
            backwards_compatibility_indicator: false,
            scrambling_sequence_index: None,
            input_stream_identifier: None,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(S2SatelliteDeliverySystemDescriptor::parse(&buf).unwrap(), d);
    }

    #[test]
    fn serialize_round_trip_with_both_optional_fields() {
        let d = S2SatelliteDeliverySystemDescriptor {
            scrambling_sequence_selector: true,
            multiple_input_stream_flag: true,
            backwards_compatibility_indicator: true,
            scrambling_sequence_index: Some(0x2BCDE),
            input_stream_identifier: Some(0x42),
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(S2SatelliteDeliverySystemDescriptor::parse(&buf).unwrap(), d);
    }
}
