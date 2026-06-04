//! S2 Satellite Delivery System Descriptor — ETSI EN 300 468 §6.2.13.3 (tag 0x79).
//!
//! Companion to the satellite_delivery_system_descriptor (tag 0x43) when the
//! carrier is DVB-S2. Carries a PLS Gold scrambling sequence index, an
//! input_stream_identifier (ISI) for multi-stream carriers, the TS/GS mode, and
//! an optional timeslice number.
//!
//! Wire layout (§6.2.13.3 Table 42):
//!
//! ```text
//!  byte 0 (flags):
//!    bit 7      scrambling_sequence_selector
//!    bit 6      multiple_input_stream_flag
//!    bit 5      reserved_zero_future_use
//!    bit 4      not_timeslice_flag
//!    bits 3..2  reserved_future_use
//!    bits 1..0  TS_GS_mode
//!  if scrambling_sequence_selector == 1 (3 bytes):
//!    reserved(6) + scrambling_sequence_index(18)
//!  if multiple_input_stream_flag == 1 (1 byte):
//!    input_stream_identifier(8)
//!  if not_timeslice_flag == 0 (1 byte):
//!    timeslice_number(8)
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
const TIMESLICE_FIELD_LEN: usize = 1;

const FLAG_SCRAMBLING_SELECTOR: u8 = 0x80;
const FLAG_MULTIPLE_INPUT_STREAM: u8 = 0x40;
const FLAG_NOT_TIMESLICE: u8 = 0x10;
/// TS_GS_mode occupies the bottom 2 bits of the flags byte; its value coding is
/// Table 43 ("Coding of the TS GS mode"). The descriptor layout itself is Table 42.
const TS_GS_MODE_MASK: u8 = 0x03;
/// Reserved flag bits set to 1 on serialize. Per §6.2.13.3 Table 42, bit 5 is
/// `reserved_zero_future_use` (MUST be 0); only bits 3..2 (`reserved_future_use`)
/// are set, giving 0x0C. All reserved bits are ignored on parse (§5.1).
const FLAG_RESERVED_BITS: u8 = 0x0C;

const SCRAMBLING_RESERVED_MASK: u8 = 0xFC;
const SCRAMBLING_INDEX_HI_MASK: u8 = 0x03;
const SCRAMBLING_INDEX_MAX: u32 = 0x3FFFF;

/// S2 Satellite Delivery System Descriptor (§6.2.13.3, Table 42).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct S2SatelliteDeliverySystemDescriptor {
    /// When set, `scrambling_sequence_index` is present.
    pub scrambling_sequence_selector: bool,
    /// When set, `input_stream_identifier` is present.
    pub multiple_input_stream_flag: bool,
    /// When set, timeslicing is NOT used and `timeslice_number` is absent.
    pub not_timeslice_flag: bool,
    /// 2-bit TS/GS mode (Table 43: 0 generic packetized, 1 GSE, 2 DVB TS, 3 reserved).
    pub ts_gs_mode: u8,
    /// 18-bit PLS Gold scrambling sequence index.
    pub scrambling_sequence_index: Option<u32>,
    /// 8-bit input_stream_identifier (ISI).
    pub input_stream_identifier: Option<u8>,
    /// 8-bit timeslice_number, present when `not_timeslice_flag` is false.
    pub timeslice_number: Option<u8>,
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

        // Reserved bits are ignored on parse (EN 300 468 §5.1).
        let flags = bytes[HEADER_LEN];
        let scrambling_sequence_selector = (flags & FLAG_SCRAMBLING_SELECTOR) != 0;
        let multiple_input_stream_flag = (flags & FLAG_MULTIPLE_INPUT_STREAM) != 0;
        let not_timeslice_flag = (flags & FLAG_NOT_TIMESLICE) != 0;
        let ts_gs_mode = flags & TS_GS_MODE_MASK;

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
            let index = (u32::from(b0 & SCRAMBLING_INDEX_HI_MASK) << 16)
                | (u32::from(bytes[pos + 1]) << 8)
                | u32::from(bytes[pos + 2]);
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
            pos += ISI_FIELD_LEN;
            Some(isi)
        } else {
            None
        };

        // timeslice_number is present when timeslicing IS used (not_timeslice_flag == 0).
        let timeslice_number = if !not_timeslice_flag {
            if pos + TIMESLICE_FIELD_LEN > end {
                return Err(Error::BufferTooShort {
                    need: pos + TIMESLICE_FIELD_LEN - HEADER_LEN,
                    have: length,
                    what: "S2SatelliteDeliverySystemDescriptor timeslice_number",
                });
            }
            Some(bytes[pos])
        } else {
            None
        };

        Ok(Self {
            scrambling_sequence_selector,
            multiple_input_stream_flag,
            not_timeslice_flag,
            ts_gs_mode,
            scrambling_sequence_index,
            input_stream_identifier,
            timeslice_number,
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
            + if self.not_timeslice_flag {
                0
            } else {
                TIMESLICE_FIELD_LEN
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

        let mut flags = FLAG_RESERVED_BITS | (self.ts_gs_mode & TS_GS_MODE_MASK);
        if self.scrambling_sequence_selector {
            flags |= FLAG_SCRAMBLING_SELECTOR;
        }
        if self.multiple_input_stream_flag {
            flags |= FLAG_MULTIPLE_INPUT_STREAM;
        }
        if self.not_timeslice_flag {
            flags |= FLAG_NOT_TIMESLICE;
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
            pos += ISI_FIELD_LEN;
        }
        if !self.not_timeslice_flag {
            buf[pos] = self.timeslice_number.unwrap_or(0);
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

    /// flags 0x2C = reserved bits set, all feature flags clear, ts_gs_mode 0,
    /// not_timeslice_flag clear → timeslice_number present.
    #[test]
    fn parse_minimal_with_timeslice() {
        let raw = [TAG, 2, 0x2C, 0x07];
        let d = S2SatelliteDeliverySystemDescriptor::parse(&raw).unwrap();
        assert!(!d.scrambling_sequence_selector);
        assert!(!d.multiple_input_stream_flag);
        assert!(!d.not_timeslice_flag);
        assert_eq!(d.ts_gs_mode, 0);
        assert_eq!(d.timeslice_number, Some(0x07));
        assert_eq!(d.scrambling_sequence_index, None);
        assert_eq!(d.input_stream_identifier, None);
    }

    #[test]
    fn parse_not_timeslice_omits_timeslice_number() {
        // not_timeslice_flag (0x10) set + reserved (0x2C) + ts_gs_mode 2 = 0x3E
        let raw = [TAG, 1, 0x3E];
        let d = S2SatelliteDeliverySystemDescriptor::parse(&raw).unwrap();
        assert!(d.not_timeslice_flag);
        assert_eq!(d.ts_gs_mode, 2);
        assert_eq!(d.timeslice_number, None);
    }

    #[test]
    fn parse_extracts_isi_and_timeslice() {
        // flags = MIS(0x40) | reserved(0x2C) = 0x6C; ISI byte, then timeslice byte
        let raw = [TAG, 3, 0x6C, 0x05, 0x09];
        let d = S2SatelliteDeliverySystemDescriptor::parse(&raw).unwrap();
        assert!(d.multiple_input_stream_flag);
        assert_eq!(d.input_stream_identifier, Some(5));
        assert_eq!(d.timeslice_number, Some(9));
    }

    #[test]
    fn parse_extracts_scrambling_index() {
        // flags = scrambling(0x80) | not_timeslice(0x10) | reserved(0x2C) = 0xBC
        let raw = [TAG, 4, 0xBC, 0xFD, 0x23, 0x45];
        let d = S2SatelliteDeliverySystemDescriptor::parse(&raw).unwrap();
        assert!(d.scrambling_sequence_selector);
        assert_eq!(d.scrambling_sequence_index, Some(0x12345));
        assert!(d.not_timeslice_flag);
        assert_eq!(d.timeslice_number, None);
    }

    #[test]
    fn parse_full_18_bit_index() {
        let raw = [TAG, 4, 0xBC, 0xFF, 0xFF, 0xFF];
        let d = S2SatelliteDeliverySystemDescriptor::parse(&raw).unwrap();
        assert_eq!(d.scrambling_sequence_index, Some(0x3FFFF));
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let raw = [0x44, 1, 0x3E];
        assert!(matches!(
            S2SatelliteDeliverySystemDescriptor::parse(&raw).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x44, .. }
        ));
    }

    #[test]
    fn parse_rejects_truncated_scrambling() {
        let raw = [TAG, 1, 0x9C]; // scrambling flag set, but no scrambling bytes
        assert!(matches!(
            S2SatelliteDeliverySystemDescriptor::parse(&raw).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn serialize_round_trip_all_fields() {
        let d = S2SatelliteDeliverySystemDescriptor {
            scrambling_sequence_selector: true,
            multiple_input_stream_flag: true,
            not_timeslice_flag: false,
            ts_gs_mode: 2,
            scrambling_sequence_index: Some(0x2BCDE),
            input_stream_identifier: Some(0x42),
            timeslice_number: Some(0x11),
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(S2SatelliteDeliverySystemDescriptor::parse(&buf).unwrap(), d);
    }

    #[test]
    fn serialize_round_trip_not_timeslice() {
        let d = S2SatelliteDeliverySystemDescriptor {
            scrambling_sequence_selector: false,
            multiple_input_stream_flag: false,
            not_timeslice_flag: true,
            ts_gs_mode: 1,
            scrambling_sequence_index: None,
            input_stream_identifier: None,
            timeslice_number: None,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(S2SatelliteDeliverySystemDescriptor::parse(&buf).unwrap(), d);
    }
}
