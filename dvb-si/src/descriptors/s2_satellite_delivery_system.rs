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

use super::descriptor_body;
use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

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
/// TS_GS_mode occupies the bottom 2 bits of the flags byte.
const TS_GS_MODE_MASK: u8 = 0x03;
/// Bit 5 of the flags byte: `reserved_zero_future_use` (V1.16.1+; in
/// V1.7.1-V1.15.1 it was `backwards_compatibility_indicator`).
const FLAG_RESERVED_ZERO: u8 = 0x20;
/// Bits 3..2 of the flags byte: `reserved_future_use(2)`.
const FLAG_RESERVED_FU_MASK: u8 = 0x0C;
const FLAG_RESERVED_FU_SHIFT: u8 = 2;
/// `reserved(6)` ahead of `scrambling_sequence_index`: top 6 bits of its first byte.
const SCRAMBLING_RESERVED_SHIFT: u8 = 2;
const SCRAMBLING_RESERVED_WIDTH_MASK: u8 = 0x3F;

/// Spec default of the 1-bit `reserved_zero_future_use` (bit 5): `0`
/// (EN 300 468 Table 42, V1.16.1-V1.20.1, audit §4 row 4). Editions
/// V1.7.1-V1.15.1 defined this bit as `backwards_compatibility_indicator`
/// (1 = low-priority stream of a backward-compatible carrier); it is kept
/// verbatim on parse so such a sender round-trips.
pub const RESERVED_ZERO_FUTURE_USE_DEFAULT: u8 = 0;
/// Spec default of `reserved_future_use(2)` (bits 3..2): `11` (all editions).
pub const RESERVED_FUTURE_USE_DEFAULT: u8 = 0b11;
/// Default of the 6-bit `reserved` ahead of `scrambling_sequence_index`:
/// all ones (EN 300 468 Table 42, global "reserved = 1" rule).
pub const RESERVED_6_DEFAULT: u8 = 0x3F;

const SCRAMBLING_INDEX_HI_MASK: u8 = 0x03;
const SCRAMBLING_INDEX_MAX: u32 = 0x3FFFF;

/// TS/GS mode — ETSI EN 300 468 Table 43.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum TsGsMode {
    /// Generic Packetized.
    GenericPacketized,
    /// Generic Stream Encapsulation (GSE).
    Gse,
    /// DVB transport stream.
    DvbTransportStream,
    /// Reserved / future use.
    Reserved(u8),
}

impl TsGsMode {
    #[must_use]
    /// Construct from a raw `u8`; every value maps to a variant (total, lossless).
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => TsGsMode::GenericPacketized,
            1 => TsGsMode::Gse,
            2 => TsGsMode::DvbTransportStream,
            other => TsGsMode::Reserved(other),
        }
    }

    #[must_use]
    /// Inverse of `from_u8`; `Self::Reserved` emits its stored value.
    pub const fn to_u8(self) -> u8 {
        match self {
            TsGsMode::GenericPacketized => 0,
            TsGsMode::Gse => 1,
            TsGsMode::DvbTransportStream => 2,
            TsGsMode::Reserved(v) => v,
        }
    }

    #[must_use]
    /// Human-readable spec name per the governing Table.
    pub fn name(self) -> &'static str {
        match self {
            TsGsMode::GenericPacketized => "Generic Packetized",
            TsGsMode::Gse => "Generic Stream Encapsulation (GSE)",
            TsGsMode::DvbTransportStream => "DVB transport stream",
            TsGsMode::Reserved(_) => "reserved",
        }
    }
}
broadcast_common::impl_spec_display!(TsGsMode, Reserved);

/// S2 Satellite Delivery System Descriptor (§6.2.13.3, Table 42).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct S2SatelliteDeliverySystemDescriptor {
    /// `reserved_zero_future_use(1)` (flags bit 5, right-aligned 1 bit).
    /// Parsed verbatim and re-emitted (an old `backwards_compatibility_indicator`
    /// survives); `Default` is [`RESERVED_ZERO_FUTURE_USE_DEFAULT`] (`0`).
    pub reserved_zero_future_use: u8,
    /// `reserved_future_use(2)` (flags bits 3..2, right-aligned). Parsed
    /// verbatim; `Default` is [`RESERVED_FUTURE_USE_DEFAULT`] (`0b11`).
    pub reserved_future_use: u8,
    /// `reserved(6)` ahead of `scrambling_sequence_index` (6 bits,
    /// right-aligned); only on the wire when `scrambling_sequence_selector`
    /// is set. Parsed verbatim; `Default` is [`RESERVED_6_DEFAULT`] (`0x3F`).
    pub reserved_6: u8,
    /// Bytes after the last defined field inside the descriptor body
    /// (tolerated and previously dropped); re-emitted verbatim. Empty for a
    /// conformant descriptor.
    pub trailing: Vec<u8>,
    /// When set, `scrambling_sequence_index` is present.
    pub scrambling_sequence_selector: bool,
    /// When set, `input_stream_identifier` is present.
    pub multiple_input_stream_flag: bool,
    /// When set, timeslicing is NOT used and `timeslice_number` is absent.
    pub not_timeslice_flag: bool,
    /// 2-bit TS/GS mode (Table 43).
    pub ts_gs_mode: TsGsMode,
    /// 18-bit PLS Gold scrambling sequence index.
    pub scrambling_sequence_index: Option<u32>,
    /// 8-bit input_stream_identifier (ISI).
    pub input_stream_identifier: Option<u8>,
    /// 8-bit timeslice_number, present when `not_timeslice_flag` is false.
    pub timeslice_number: Option<u8>,
}

impl Default for S2SatelliteDeliverySystemDescriptor {
    /// No scrambling selector / ISI / timeslicing (`not_timeslice_flag` set),
    /// generic-packetized TS/GS mode, spec-default reserved bits.
    fn default() -> Self {
        Self {
            reserved_zero_future_use: RESERVED_ZERO_FUTURE_USE_DEFAULT,
            reserved_future_use: RESERVED_FUTURE_USE_DEFAULT,
            reserved_6: RESERVED_6_DEFAULT,
            trailing: Vec::new(),
            scrambling_sequence_selector: false,
            multiple_input_stream_flag: false,
            not_timeslice_flag: true,
            ts_gs_mode: TsGsMode::GenericPacketized,
            scrambling_sequence_index: None,
            input_stream_identifier: None,
            timeslice_number: None,
        }
    }
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
        let body = descriptor_body(
            bytes,
            TAG,
            "S2SatelliteDeliverySystemDescriptor",
            "unexpected tag for s2_satellite_delivery_system_descriptor",
        )?;
        if body.len() < FLAGS_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "body must contain at least the flags byte",
            });
        }

        // Reserved bits are captured (EN 300 468 §5.1 tolerance) and re-emitted.
        let flags = body[0];
        let reserved_zero_future_use = (flags & FLAG_RESERVED_ZERO) >> 5;
        let reserved_future_use = (flags & FLAG_RESERVED_FU_MASK) >> FLAG_RESERVED_FU_SHIFT;
        let scrambling_sequence_selector = (flags & FLAG_SCRAMBLING_SELECTOR) != 0;
        let multiple_input_stream_flag = (flags & FLAG_MULTIPLE_INPUT_STREAM) != 0;
        let not_timeslice_flag = (flags & FLAG_NOT_TIMESLICE) != 0;
        let ts_gs_mode = TsGsMode::from_u8(flags & TS_GS_MODE_MASK);

        let mut pos = FLAGS_LEN;

        let mut reserved_6 = RESERVED_6_DEFAULT;
        let scrambling_sequence_index = if scrambling_sequence_selector {
            if pos + SCRAMBLING_FIELD_LEN > body.len() {
                return Err(Error::BufferTooShort {
                    need: pos + SCRAMBLING_FIELD_LEN,
                    have: body.len(),
                    what: "S2SatelliteDeliverySystemDescriptor scrambling_sequence_index",
                });
            }
            let b0 = body[pos];
            reserved_6 = b0 >> SCRAMBLING_RESERVED_SHIFT;
            let index = (u32::from(b0 & SCRAMBLING_INDEX_HI_MASK) << 16)
                | (u32::from(body[pos + 1]) << 8)
                | u32::from(body[pos + 2]);
            pos += SCRAMBLING_FIELD_LEN;
            Some(index)
        } else {
            None
        };

        let input_stream_identifier = if multiple_input_stream_flag {
            if pos + ISI_FIELD_LEN > body.len() {
                return Err(Error::BufferTooShort {
                    need: pos + ISI_FIELD_LEN,
                    have: body.len(),
                    what: "S2SatelliteDeliverySystemDescriptor input_stream_identifier",
                });
            }
            let isi = body[pos];
            pos += ISI_FIELD_LEN;
            Some(isi)
        } else {
            None
        };

        // timeslice_number is present when timeslicing IS used (not_timeslice_flag == 0).
        let timeslice_number = if !not_timeslice_flag {
            if pos + TIMESLICE_FIELD_LEN > body.len() {
                return Err(Error::BufferTooShort {
                    need: pos + TIMESLICE_FIELD_LEN,
                    have: body.len(),
                    what: "S2SatelliteDeliverySystemDescriptor timeslice_number",
                });
            }
            pos += TIMESLICE_FIELD_LEN;
            Some(body[pos - TIMESLICE_FIELD_LEN])
        } else {
            None
        };

        Ok(Self {
            reserved_zero_future_use,
            reserved_future_use,
            reserved_6,
            trailing: body[pos..].to_vec(),
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
            + self.trailing.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        crate::descriptors::write_descriptor_header(buf, TAG, len - HEADER_LEN)?;

        let mut flags = ((self.reserved_zero_future_use & 1) << 5)
            | ((self.reserved_future_use & 0b11) << FLAG_RESERVED_FU_SHIFT)
            | broadcast_common::len::fit_mask(
                self.ts_gs_mode.to_u8(),
                TS_GS_MODE_MASK,
                "ts_gs_mode",
            )?;
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
            // The selector promises 18 bits on the wire; fabricating 0 would
            // silently invent an index (r03-W5).
            let index = self
                .scrambling_sequence_index
                .ok_or(Error::ValueOutOfRange {
                    field: "scrambling_sequence_index",
                    reason: "required when scrambling_sequence_selector is set",
                })?;
            let index = broadcast_common::len::fit_mask(
                index,
                SCRAMBLING_INDEX_MAX,
                "scrambling_sequence_index",
            )?;
            buf[pos] = (broadcast_common::len::fit_mask(
                self.reserved_6,
                SCRAMBLING_RESERVED_WIDTH_MASK,
                "reserved_6",
            )? << SCRAMBLING_RESERVED_SHIFT)
                | ((index >> 16) as u8 & SCRAMBLING_INDEX_HI_MASK);
            buf[pos + 1] = (index >> 8) as u8;
            buf[pos + 2] = index as u8;
            pos += SCRAMBLING_FIELD_LEN;
        }
        if self.multiple_input_stream_flag {
            let isi = self.input_stream_identifier.ok_or(Error::ValueOutOfRange {
                field: "input_stream_identifier",
                reason: "required when multiple_input_stream_flag is set",
            })?;
            buf[pos] = isi;
            pos += ISI_FIELD_LEN;
        }
        if !self.not_timeslice_flag {
            let ts = self.timeslice_number.ok_or(Error::ValueOutOfRange {
                field: "timeslice_number",
                reason: "required when not_timeslice_flag is clear",
            })?;
            buf[pos] = ts;
            pos += TIMESLICE_FIELD_LEN;
        }
        buf[pos..pos + self.trailing.len()].copy_from_slice(&self.trailing);
        Ok(len)
    }
}
impl crate::traits::DescriptorDef<'_> for S2SatelliteDeliverySystemDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "S2_SATELLITE_DELIVERY_SYSTEM";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ts_gs_mode_from_u8_to_u8_roundtrip() {
        for b in 0..=0xFFu8 {
            assert_eq!(
                TsGsMode::from_u8(b).to_u8(),
                b,
                "round-trip fail for {b:#04x}"
            );
        }
    }

    /// flags 0x2C = reserved bits set, all feature flags clear, ts_gs_mode 0,
    /// not_timeslice_flag clear → timeslice_number present.
    #[test]
    fn parse_minimal_with_timeslice() {
        let raw = [TAG, 2, 0x2C, 0x07];
        let d = S2SatelliteDeliverySystemDescriptor::parse(&raw).unwrap();
        assert!(!d.scrambling_sequence_selector);
        assert!(!d.multiple_input_stream_flag);
        assert!(!d.not_timeslice_flag);
        assert_eq!(d.ts_gs_mode, TsGsMode::GenericPacketized);
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
        assert_eq!(d.ts_gs_mode, TsGsMode::DvbTransportStream);
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
            ts_gs_mode: TsGsMode::DvbTransportStream,
            scrambling_sequence_index: Some(0x2BCDE),
            input_stream_identifier: Some(0x42),
            timeslice_number: Some(0x11),
            ..Default::default()
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
            ts_gs_mode: TsGsMode::Gse,
            scrambling_sequence_index: None,
            input_stream_identifier: None,
            timeslice_number: None,
            ..Default::default()
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(S2SatelliteDeliverySystemDescriptor::parse(&buf).unwrap(), d);
    }
    #[test]
    fn serialize_rejects_missing_selector_conditional_fields() {
        // r03-W5: each selector flag reserves wire bytes; the value used to be
        // `unwrap_or(0)`-ed, inventing an index/ISI/timeslice out of thin air.
        let d = S2SatelliteDeliverySystemDescriptor {
            scrambling_sequence_selector: true,
            multiple_input_stream_flag: false,
            not_timeslice_flag: true,
            ts_gs_mode: TsGsMode::from_u8(0),
            scrambling_sequence_index: None,
            input_stream_identifier: None,
            timeslice_number: None,
            ..Default::default()
        };
        let mut buf = vec![0u8; d.serialized_len()];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::ValueOutOfRange {
                field: "scrambling_sequence_index",
                ..
            }
        ));
    }

    /// Every reserved region (`reserved_zero_future_use(1)`,
    /// `reserved_future_use(2)`, `reserved(6)`) and the trailing tail
    /// re-serialize byte-identically, for all values of the narrow regions.
    #[test]
    fn reserved_preserved() {
        for rz in 0u8..=1 {
            for rfu in 0u8..=3 {
                for r6 in [0u8, 0x3F, 0x15, 0x2A, 1] {
                    // selector + not_timeslice, ts_gs_mode 1
                    let flags = 0x80 | 0x10 | (rz << 5) | (rfu << 2) | 0x01;
                    let raw = [TAG, 4, flags, (r6 << 2) | 0x01, 0x23, 0x45];
                    let d = S2SatelliteDeliverySystemDescriptor::parse(&raw).unwrap();
                    assert_eq!(d.reserved_zero_future_use, rz);
                    assert_eq!(d.reserved_future_use, rfu);
                    assert_eq!(d.reserved_6, r6);
                    let mut buf = vec![0u8; d.serialized_len()];
                    d.serialize_into(&mut buf).unwrap();
                    assert_eq!(buf, raw, "rz {rz} rfu {rfu} r6 {r6:#x}");
                }
            }
        }
    }

    /// Bytes past the last defined field are kept and re-emitted.
    #[test]
    fn trailing_bytes_preserved() {
        let raw = [TAG, 4, 0x3C, 0xDE, 0xAD, 0xBE];
        let d = S2SatelliteDeliverySystemDescriptor::parse(&raw).unwrap();
        assert_eq!(d.trailing, [0xDE, 0xAD, 0xBE]);
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, raw);
    }

    /// `Default` serializes the spec values: bit 5 = 0, rfu = 11, reserved(6) = all ones.
    #[test]
    fn default_reserved_is_spec_value() {
        let d = S2SatelliteDeliverySystemDescriptor {
            scrambling_sequence_selector: true,
            scrambling_sequence_index: Some(0x12345),
            ..Default::default()
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        // flags: selector(0x80) | not_timeslice(0x10) | rfu(0x0C), bit5 = 0.
        assert_eq!(buf, [TAG, 4, 0x80 | 0x10 | 0x0C, 0xFD, 0x23, 0x45]);
    }
}
