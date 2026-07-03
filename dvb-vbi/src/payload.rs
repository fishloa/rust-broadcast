//! Typed data-unit payloads — ETSI EN 301 775 §4.5–§4.9 (Tables 4, 6, 8, 10,
//! 12).
//!
//! Each [`DataUnitPayload`] variant mirrors one data-field syntax table:
//!
//! - [`TeletextDataField`] — §4.5 (Table 4): EBU / Inverted Teletext, the shared
//!   [`LineHeader`] + an 8-bit `framing_code` + a 42-byte opaque `txt_data_block`
//!   (`0x02` / `0x03` / `0xC0`). EN 300 706 Teletext coding is out of scope, so
//!   the block bytes are opaque.
//! - [`VpsDataField`] — §4.6 (Table 6): VPS, the shared [`LineHeader`] + a
//!   13-byte `vps_data_block` (`0xC3`).
//! - [`WssDataField`] — §4.7 (Table 8): WSS, the shared [`LineHeader`] + a 14-bit
//!   `wss_data_block` + a trailing 2-bit `reserved_future_use` `11` (`0xC4`).
//! - [`ClosedCaptioningDataField`] — §4.8 (Table 10): Closed Captioning, the
//!   shared [`LineHeader`] + a 16-bit `closed_captioning_data_block` (`0xC5`).
//! - [`MonochromeDataField`] — §4.9 (Table 12): monochrome 4:2:2 luminance
//!   samples (`0xC6`) — its own first-byte packing (two segment flags +
//!   field_parity + line_offset), `first_pixel_position`, `n_pixels`, then the
//!   luminance `Y_value` samples.
//! - [`DataUnitPayload::Stuffing`] — §4.4.1: `0xFF`, no data field.
//! - [`DataUnitPayload::Opaque`] — reserved / user-defined data_unit_ids whose
//!   body this crate does not interpret (Table 3: discard); the raw bytes are
//!   retained for round-trip fidelity.

use alloc::vec::Vec;

use crate::data_unit_id::DataUnitId;
use crate::error::{Error, Result};
use crate::line_header::{LINE_HEADER_LEN, LineHeader};

/// Size in bytes of the EBU/Inverted Teletext `txt_data_block` (336 bits, §4.5).
pub const TXT_DATA_BLOCK_LEN: usize = 42;
/// Size in bytes of a Teletext data field (header + framing_code + block).
pub const TELETEXT_FIELD_LEN: usize = LINE_HEADER_LEN + 1 + TXT_DATA_BLOCK_LEN;
/// The fixed `data_unit_length` for `data_identifier` `0x10`–`0x1F` (`0x2C` =
/// 44, the Teletext data-field body length — §4.4.2).
pub const TELETEXT_DATA_UNIT_LENGTH: u8 = 0x2C;

/// EBU Teletext framing_code (`11100100`, §4.5.2).
pub const FRAMING_CODE_EBU: u8 = 0b1110_0100;
/// Inverted Teletext framing_code (`00011011`, §4.5.2).
pub const FRAMING_CODE_INVERTED: u8 = 0b0001_1011;

/// Size in bytes of the VPS `vps_data_block` (104 bits, §4.6).
pub const VPS_DATA_BLOCK_LEN: usize = 13;
/// Size in bytes of a VPS data field (header + block).
pub const VPS_FIELD_LEN: usize = LINE_HEADER_LEN + VPS_DATA_BLOCK_LEN;

/// Size in bytes of a WSS data field (header + 14 wss bits + 2-bit RFU = 3
/// bytes, §4.7).
pub const WSS_FIELD_LEN: usize = LINE_HEADER_LEN + 2;
/// Mask for the 14-bit `wss_data_block` (§4.7).
pub const WSS_DATA_BLOCK_MASK: u16 = 0x3FFF;
/// Mask for the lower 6 bits of `wss_data_block` packed into byte 2 of the WSS
/// field (bits `[5:0]` after the 2-bit RFU tail, §4.7.1).
const WSS_BYTE2_DATA_MASK: u8 = 0x3F;
/// The trailing 2-bit `reserved_future_use` (`11`) of a WSS data field (§4.7.1).
pub const WSS_RESERVED_TAIL: u8 = 0b11;

/// Size in bytes of a Closed Captioning data field (header + 16 CC bits = 3
/// bytes, §4.8).
pub const CC_FIELD_LEN: usize = LINE_HEADER_LEN + 2;

/// Size in bytes of the monochrome fixed header preceding the `Y_value` samples:
/// first byte (flags + parity + line_offset) + 16-bit first_pixel_position +
/// 8-bit n_pixels (§4.9.1).
pub const MONO_HEADER_LEN: usize = 4;

/// `first_segment_flag` bit (`[7]`) of the monochrome first byte (§4.9.1).
const MONO_FIRST_SEGMENT: u8 = 0b1000_0000;
/// `last_segment_flag` bit (`[6]`) of the monochrome first byte (§4.9.1).
const MONO_LAST_SEGMENT: u8 = 0b0100_0000;
/// `field_parity` bit (`[5]`) of the monochrome first byte (§4.9.1).
const MONO_FIELD_PARITY: u8 = 0b0010_0000;
/// `line_offset` mask (`[4:0]`) of the monochrome first byte (§4.9.1).
const MONO_LINE_OFFSET: u8 = 0b0001_1111;

/// EBU / Inverted Teletext data field — ETSI EN 301 775 §4.5.1, Table 4
/// (`data_unit_id` `0x02`, `0x03`, `0xC0`).
///
/// The `txt_data_block` (42 bytes) is the EN 300 706 magazine_and_packet_address
/// and data_block following the clock-run-in/framing-code; EN 300 706 decoding
/// is out of scope, so it is held opaquely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TeletextDataField {
    /// Shared first-byte header: reserved_future_use `11` | field_parity |
    /// line_offset.
    pub header: LineHeader,
    /// 8-bit `framing_code` (`0b11100100` EBU / `0b00011011` Inverted, §4.5.2).
    pub framing_code: u8,
    /// 42-byte `txt_data_block` (336 bits, §4.5; EN 300 706 — opaque here).
    #[cfg_attr(feature = "serde", serde(serialize_with = "serialize_txt_block"))]
    pub txt_data_block: [u8; TXT_DATA_BLOCK_LEN],
}

/// Serialize the 42-byte Teletext block as a byte slice (serde's `Serialize` for
/// arrays stops at 32 elements).
#[cfg(feature = "serde")]
fn serialize_txt_block<S>(
    block: &[u8; TXT_DATA_BLOCK_LEN],
    s: S,
) -> core::result::Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    s.serialize_bytes(block)
}

impl TeletextDataField {
    /// Bytes this data field occupies (the `data_unit_length`).
    pub fn serialized_len(&self) -> usize {
        TELETEXT_FIELD_LEN
    }

    /// Parse exactly one Teletext data field from `data` (`data` must be the
    /// data-unit body of `data_unit_length` bytes).
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < TELETEXT_FIELD_LEN {
            return Err(Error::BufferTooShort {
                need: TELETEXT_FIELD_LEN,
                have: data.len(),
                what: "txt_data_field",
            });
        }
        let header = LineHeader::from_byte(data[0]);
        let framing_code = data[1];
        let mut txt_data_block = [0u8; TXT_DATA_BLOCK_LEN];
        txt_data_block.copy_from_slice(&data[2..2 + TXT_DATA_BLOCK_LEN]);
        Ok(TeletextDataField {
            header,
            framing_code,
            txt_data_block,
        })
    }

    /// Serialize into `out`, returning the number of bytes written.
    pub fn serialize_into(&self, out: &mut [u8]) -> Result<usize> {
        if out.len() < TELETEXT_FIELD_LEN {
            return Err(Error::OutputBufferTooSmall {
                need: TELETEXT_FIELD_LEN,
                have: out.len(),
            });
        }
        out[0] = self.header.to_byte()?;
        out[1] = self.framing_code;
        out[2..2 + TXT_DATA_BLOCK_LEN].copy_from_slice(&self.txt_data_block);
        Ok(TELETEXT_FIELD_LEN)
    }
}

/// VPS data field — ETSI EN 301 775 §4.6.1, Table 6 (`data_unit_id` `0xC3`).
///
/// The `vps_data_block` (13 bytes) is bytes 3..=15 of an EN 300 231 VPS line,
/// excluding the run-in and start-code byte (§4.6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct VpsDataField {
    /// Shared first-byte header (§4.6.1: field_parity `1`, line_offset `16`).
    pub header: LineHeader,
    /// 13-byte `vps_data_block` (104 bits, §4.6; EN 300 231 — opaque here).
    pub vps_data_block: [u8; VPS_DATA_BLOCK_LEN],
}

impl VpsDataField {
    /// Bytes this data field occupies (the `data_unit_length`).
    pub fn serialized_len(&self) -> usize {
        VPS_FIELD_LEN
    }

    /// Parse exactly one VPS data field from `data`.
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < VPS_FIELD_LEN {
            return Err(Error::BufferTooShort {
                need: VPS_FIELD_LEN,
                have: data.len(),
                what: "vps_data_field",
            });
        }
        let header = LineHeader::from_byte(data[0]);
        let mut vps_data_block = [0u8; VPS_DATA_BLOCK_LEN];
        vps_data_block.copy_from_slice(&data[1..1 + VPS_DATA_BLOCK_LEN]);
        Ok(VpsDataField {
            header,
            vps_data_block,
        })
    }

    /// Serialize into `out`, returning the number of bytes written.
    pub fn serialize_into(&self, out: &mut [u8]) -> Result<usize> {
        if out.len() < VPS_FIELD_LEN {
            return Err(Error::OutputBufferTooSmall {
                need: VPS_FIELD_LEN,
                have: out.len(),
            });
        }
        out[0] = self.header.to_byte()?;
        out[1..1 + VPS_DATA_BLOCK_LEN].copy_from_slice(&self.vps_data_block);
        Ok(VPS_FIELD_LEN)
    }
}

/// WSS data field — ETSI EN 301 775 §4.7.1, Table 8 (`data_unit_id` `0xC4`).
///
/// Byte layout (24 bits): byte0 = shared header, then 14 `wss_data_block` bits
/// followed by a 2-bit `reserved_future_use` `11` tail. So byte1 holds wss bits
/// `[13:6]` and byte2 holds wss bits `[5:0]` then the 2-bit RFU tail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct WssDataField {
    /// Shared first-byte header (§4.7.1: field_parity `1`, line_offset `23`).
    pub header: LineHeader,
    /// 14-bit `wss_data_block` (§4.7; EN 300 294 — value in the low 14 bits).
    pub wss_data_block: u16,
}

impl WssDataField {
    /// Bytes this data field occupies (the `data_unit_length`).
    pub fn serialized_len(&self) -> usize {
        WSS_FIELD_LEN
    }

    /// Parse exactly one WSS data field from `data`. The trailing 2-bit RFU is
    /// not validated (decoders ignore RFU).
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < WSS_FIELD_LEN {
            return Err(Error::BufferTooShort {
                need: WSS_FIELD_LEN,
                have: data.len(),
                what: "wss_data_field",
            });
        }
        let header = LineHeader::from_byte(data[0]);
        // wss bits [13:6] in byte1, [5:0] in the high 6 bits of byte2.
        let wss_data_block =
            (((data[1] as u16) << 6) | ((data[2] as u16) >> 2)) & WSS_DATA_BLOCK_MASK;
        Ok(WssDataField {
            header,
            wss_data_block,
        })
    }

    /// Serialize into `out`, returning the number of bytes written. The 14-bit
    /// `wss_data_block` is masked and a `11` RFU tail is emitted.
    pub fn serialize_into(&self, out: &mut [u8]) -> Result<usize> {
        if out.len() < WSS_FIELD_LEN {
            return Err(Error::OutputBufferTooSmall {
                need: WSS_FIELD_LEN,
                have: out.len(),
            });
        }
        if self.wss_data_block > WSS_DATA_BLOCK_MASK {
            return Err(Error::FieldTooWide {
                what: "wss_data_block",
                value: self.wss_data_block as u32,
                bits: 14,
            });
        }
        out[0] = self.header.to_byte()?;
        out[1] = (self.wss_data_block >> 6) as u8;
        out[2] = (((self.wss_data_block as u8) & WSS_BYTE2_DATA_MASK) << 2) | WSS_RESERVED_TAIL;
        Ok(WSS_FIELD_LEN)
    }
}

/// Closed Captioning data field — ETSI EN 301 775 §4.8.1, Table 10
/// (`data_unit_id` `0xC5`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ClosedCaptioningDataField {
    /// Shared first-byte header (§4.8.1: line_offset `21`).
    pub header: LineHeader,
    /// 16-bit `closed_captioning_data_block` (EIA-608 Rev A, §4.8; the two CC
    /// bytes, MSB = byte one).
    pub closed_captioning_data_block: u16,
}

impl ClosedCaptioningDataField {
    /// Bytes this data field occupies (the `data_unit_length`).
    pub fn serialized_len(&self) -> usize {
        CC_FIELD_LEN
    }

    /// Parse exactly one Closed Captioning data field from `data`.
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < CC_FIELD_LEN {
            return Err(Error::BufferTooShort {
                need: CC_FIELD_LEN,
                have: data.len(),
                what: "closed_captioning_data_field",
            });
        }
        let header = LineHeader::from_byte(data[0]);
        let closed_captioning_data_block = u16::from_be_bytes([data[1], data[2]]);
        Ok(ClosedCaptioningDataField {
            header,
            closed_captioning_data_block,
        })
    }

    /// Serialize into `out`, returning the number of bytes written.
    pub fn serialize_into(&self, out: &mut [u8]) -> Result<usize> {
        if out.len() < CC_FIELD_LEN {
            return Err(Error::OutputBufferTooSmall {
                need: CC_FIELD_LEN,
                have: out.len(),
            });
        }
        out[0] = self.header.to_byte()?;
        out[1..3].copy_from_slice(&self.closed_captioning_data_block.to_be_bytes());
        Ok(CC_FIELD_LEN)
    }
}

/// Monochrome 4:2:2 luminance-sample data field — ETSI EN 301 775 §4.9.1,
/// Table 12 (`data_unit_id` `0xC6`).
///
/// Unlike the other units this packs its first byte as `[7]` first_segment_flag,
/// `[6]` last_segment_flag, `[5]` field_parity, `[4:0]` line_offset (no 2-bit
/// reserved prefix), then a 16-bit `first_pixel_position`, an 8-bit `n_pixels`,
/// and `n_pixels` luminance `Y_value` bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MonochromeDataField<'a> {
    /// `first_segment_flag` (`1` for the first segment of a line).
    pub first_segment: bool,
    /// `last_segment_flag` (`1` for the last segment of a line).
    pub last_segment: bool,
    /// `field_parity` (`1` first field, `0` second field).
    pub field_parity: bool,
    /// `line_offset` (5 bits, `7..=23` valid; coded per Table 13).
    pub line_offset: u8,
    /// `first_pixel_position` (16 bits, `0..=719`): position of the first coded
    /// luminance sample of this segment.
    pub first_pixel_position: u16,
    /// `Y_value` luminance samples; `n_pixels` is `samples.len()` (`1..=251`).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub samples: &'a [u8],
}

impl<'a> MonochromeDataField<'a> {
    /// Bytes this data field occupies (the `data_unit_length`).
    pub fn serialized_len(&self) -> usize {
        MONO_HEADER_LEN + self.samples.len()
    }

    /// Parse exactly one monochrome data field from `data` (`data` is the
    /// data-unit body of `data_unit_length` bytes). `n_pixels` is taken from the
    /// wire and the sample slice is borrowed from `data`.
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        if data.len() < MONO_HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: MONO_HEADER_LEN,
                have: data.len(),
                what: "monochrome_data_field header",
            });
        }
        let b0 = data[0];
        let first_segment = (b0 & MONO_FIRST_SEGMENT) != 0;
        let last_segment = (b0 & MONO_LAST_SEGMENT) != 0;
        let field_parity = (b0 & MONO_FIELD_PARITY) != 0;
        let line_offset = b0 & MONO_LINE_OFFSET;
        let first_pixel_position = u16::from_be_bytes([data[1], data[2]]);
        let n_pixels = data[3] as usize;
        // §4.9.2 mandates n_pixels > 0; a zero-n_pixels unit is non-conformant.
        if n_pixels == 0 {
            return Err(Error::InvalidField {
                what: "n_pixels",
                reason: "n_pixels shall be > 0 (ETSI EN 301 775 §4.9.2)",
            });
        }
        if data.len() < MONO_HEADER_LEN + n_pixels {
            return Err(Error::BufferTooShort {
                need: MONO_HEADER_LEN + n_pixels,
                have: data.len(),
                what: "monochrome Y_value samples",
            });
        }
        let samples = &data[MONO_HEADER_LEN..MONO_HEADER_LEN + n_pixels];
        Ok(MonochromeDataField {
            first_segment,
            last_segment,
            field_parity,
            line_offset,
            first_pixel_position,
            samples,
        })
    }

    /// Serialize into `out`, returning the number of bytes written. `n_pixels`
    /// is derived from `samples.len()`.
    pub fn serialize_into(&self, out: &mut [u8]) -> Result<usize> {
        let total = self.serialized_len();
        if out.len() < total {
            return Err(Error::OutputBufferTooSmall {
                need: total,
                have: out.len(),
            });
        }
        if self.line_offset > MONO_LINE_OFFSET {
            return Err(Error::FieldTooWide {
                what: "line_offset",
                value: self.line_offset as u32,
                bits: 5,
            });
        }
        if self.samples.len() > u8::MAX as usize {
            return Err(Error::FieldTooWide {
                what: "n_pixels",
                value: self.samples.len() as u32,
                bits: 8,
            });
        }
        let mut b0 = self.line_offset;
        if self.first_segment {
            b0 |= MONO_FIRST_SEGMENT;
        }
        if self.last_segment {
            b0 |= MONO_LAST_SEGMENT;
        }
        if self.field_parity {
            b0 |= MONO_FIELD_PARITY;
        }
        out[0] = b0;
        out[1..3].copy_from_slice(&self.first_pixel_position.to_be_bytes());
        out[3] = self.samples.len() as u8;
        out[MONO_HEADER_LEN..total].copy_from_slice(self.samples);
        Ok(total)
    }
}

/// The typed body of one data unit (ETSI EN 301 775 §4.4, dispatched on
/// `data_unit_id`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum DataUnitPayload<'a> {
    /// EBU / Inverted Teletext (`0x02`, `0x03`, `0xC0`) — §4.5.
    Teletext(TeletextDataField),
    /// VPS (`0xC3`) — §4.6.
    Vps(VpsDataField),
    /// WSS (`0xC4`) — §4.7.
    Wss(WssDataField),
    /// Closed Captioning (`0xC5`) — §4.8.
    ClosedCaptioning(ClosedCaptioningDataField),
    /// Monochrome 4:2:2 luminance samples (`0xC6`) — §4.9.
    Monochrome(#[cfg_attr(feature = "serde", serde(borrow))] MonochromeDataField<'a>),
    /// Stuffing (`0xFF`) — §4.4.1: no data field. `data_unit_length` stuffing
    /// bytes follow and are discarded; their count is retained for round-trip.
    Stuffing {
        /// The `data_unit_length` (number of `0xFF` stuffing bytes that follow).
        length: u8,
    },
    /// Reserved / user-defined data_unit_id whose body this crate does not
    /// interpret (Table 3: discard). The raw body bytes are retained.
    Opaque(#[cfg_attr(feature = "serde", serde(borrow))] &'a [u8]),
}

impl<'a> DataUnitPayload<'a> {
    /// The number of body bytes (`data_unit_length`) this payload serializes to.
    pub fn serialized_len(&self) -> usize {
        match self {
            DataUnitPayload::Teletext(f) => f.serialized_len(),
            DataUnitPayload::Vps(f) => f.serialized_len(),
            DataUnitPayload::Wss(f) => f.serialized_len(),
            DataUnitPayload::ClosedCaptioning(f) => f.serialized_len(),
            DataUnitPayload::Monochrome(f) => f.serialized_len(),
            DataUnitPayload::Stuffing { length } => *length as usize,
            DataUnitPayload::Opaque(b) => b.len(),
        }
    }

    /// Parse a data-unit body of `length` bytes against its `id` (§4.4, Table 1
    /// dispatch, resolved per Table 3). `body` must be exactly the
    /// `data_unit_length` bytes following the length field.
    pub fn parse(id: DataUnitId, body: &'a [u8]) -> Result<Self> {
        match id {
            DataUnitId::EbuTeletextNonSubtitle
            | DataUnitId::EbuTeletextSubtitle
            | DataUnitId::InvertedTeletext => {
                Ok(DataUnitPayload::Teletext(TeletextDataField::parse(body)?))
            }
            DataUnitId::Vps => Ok(DataUnitPayload::Vps(VpsDataField::parse(body)?)),
            DataUnitId::Wss => Ok(DataUnitPayload::Wss(WssDataField::parse(body)?)),
            DataUnitId::ClosedCaptioning => Ok(DataUnitPayload::ClosedCaptioning(
                ClosedCaptioningDataField::parse(body)?,
            )),
            DataUnitId::Monochrome422Samples => Ok(DataUnitPayload::Monochrome(
                MonochromeDataField::parse(body)?,
            )),
            DataUnitId::Stuffing => {
                if body.len() > u8::MAX as usize {
                    return Err(Error::InvalidDataUnitLength {
                        length: 0,
                        id: id.to_u8(),
                        reason: "stuffing length exceeds 8 bits",
                    });
                }
                Ok(DataUnitPayload::Stuffing {
                    length: body.len() as u8,
                })
            }
            DataUnitId::Reserved(_) | DataUnitId::UserDefined(_) => {
                Ok(DataUnitPayload::Opaque(body))
            }
        }
    }

    /// Serialize the body into `out`, returning the number of bytes written.
    pub fn serialize_into(&self, out: &mut [u8]) -> Result<usize> {
        match self {
            DataUnitPayload::Teletext(f) => f.serialize_into(out),
            DataUnitPayload::Vps(f) => f.serialize_into(out),
            DataUnitPayload::Wss(f) => f.serialize_into(out),
            DataUnitPayload::ClosedCaptioning(f) => f.serialize_into(out),
            DataUnitPayload::Monochrome(f) => f.serialize_into(out),
            DataUnitPayload::Stuffing { length } => {
                let n = *length as usize;
                if out.len() < n {
                    return Err(Error::OutputBufferTooSmall {
                        need: n,
                        have: out.len(),
                    });
                }
                for b in out.iter_mut().take(n) {
                    *b = crate::data_unit_id::ID_STUFFING; // 0xFF
                }
                Ok(n)
            }
            DataUnitPayload::Opaque(b) => {
                if out.len() < b.len() {
                    return Err(Error::OutputBufferTooSmall {
                        need: b.len(),
                        have: out.len(),
                    });
                }
                out[..b.len()].copy_from_slice(b);
                Ok(b.len())
            }
        }
    }
}

/// One data unit: a `data_unit_id`, its `data_unit_length`, and the typed body
/// (ETSI EN 301 775 §4.4.1, Table 1 loop body).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DataUnit<'a> {
    /// `data_unit_id` (Table 3).
    pub id: DataUnitId,
    /// The typed body.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub payload: DataUnitPayload<'a>,
}

impl<'a> DataUnit<'a> {
    /// `data_unit_length`: the number of body bytes after the length field.
    pub fn data_unit_length(&self) -> usize {
        self.payload.serialized_len()
    }

    /// Total wire size: `data_unit_id` (1) + `data_unit_length` (1) + body.
    pub fn serialized_len(&self) -> usize {
        2 + self.data_unit_length()
    }

    /// Build a Teletext data unit (EBU non-subtitle by default tag — supply the
    /// id explicitly via [`DataUnit`] if a subtitle/inverted id is wanted).
    pub fn teletext(id: DataUnitId, field: TeletextDataField) -> Self {
        DataUnit {
            id,
            payload: DataUnitPayload::Teletext(field),
        }
    }

    /// Build a VPS data unit.
    pub fn vps(field: VpsDataField) -> Self {
        DataUnit {
            id: DataUnitId::Vps,
            payload: DataUnitPayload::Vps(field),
        }
    }

    /// Build a WSS data unit.
    pub fn wss(field: WssDataField) -> Self {
        DataUnit {
            id: DataUnitId::Wss,
            payload: DataUnitPayload::Wss(field),
        }
    }

    /// Build a Closed Captioning data unit.
    pub fn closed_captioning(field: ClosedCaptioningDataField) -> Self {
        DataUnit {
            id: DataUnitId::ClosedCaptioning,
            payload: DataUnitPayload::ClosedCaptioning(field),
        }
    }

    /// Build a monochrome 4:2:2 sample data unit.
    pub fn monochrome(field: MonochromeDataField<'a>) -> Self {
        DataUnit {
            id: DataUnitId::Monochrome422Samples,
            payload: DataUnitPayload::Monochrome(field),
        }
    }

    /// Build a stuffing data unit of `length` `0xFF` bytes.
    pub fn stuffing(length: u8) -> Self {
        DataUnit {
            id: DataUnitId::Stuffing,
            payload: DataUnitPayload::Stuffing { length },
        }
    }

    /// Parse a single data unit from the start of `data`, returning it and the
    /// number of bytes consumed.
    pub fn parse(data: &'a [u8]) -> Result<(Self, usize)> {
        if data.len() < 2 {
            return Err(Error::BufferTooShort {
                need: 2,
                have: data.len(),
                what: "data_unit header (id + length)",
            });
        }
        let id = DataUnitId::from_u8(data[0]);
        let length = data[1] as usize;
        let body_end = 2 + length;
        if data.len() < body_end {
            return Err(Error::BufferTooShort {
                need: body_end,
                have: data.len(),
                what: "data_unit body",
            });
        }
        let body = &data[2..body_end];
        let payload = DataUnitPayload::parse(id, body)?;
        // Spec fidelity: the typed payload must occupy exactly data_unit_length
        // bytes (no truncation / no over-read).
        if payload.serialized_len() != length {
            return Err(Error::InvalidDataUnitLength {
                length: data[1],
                id: data[0],
                reason: "typed payload size does not match data_unit_length",
            });
        }
        Ok((DataUnit { id, payload }, body_end))
    }

    /// Serialize the data unit into `out`, returning bytes written.
    pub fn serialize_into(&self, out: &mut [u8]) -> Result<usize> {
        let total = self.serialized_len();
        if out.len() < total {
            return Err(Error::OutputBufferTooSmall {
                need: total,
                have: out.len(),
            });
        }
        let length = self.data_unit_length();
        if length > u8::MAX as usize {
            return Err(Error::FieldTooWide {
                what: "data_unit_length",
                value: length as u32,
                bits: 8,
            });
        }
        out[0] = self.id.to_u8();
        out[1] = length as u8;
        let written = self.payload.serialize_into(&mut out[2..total])?;
        debug_assert_eq!(written, length);
        Ok(2 + written)
    }
}

/// The PES data field — ETSI EN 301 775 §4.4.1, Table 1.
///
/// A `data_identifier` (Table 2) followed by a sequence of [`DataUnit`]s that
/// fill the PES packet payload. `parse` walks the units until the buffer is
/// exhausted; `serialize_into` emits them back-to-back.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DataField<'a> {
    /// `data_identifier` (8 bits, Table 2): `0x10`–`0x1F` or `0x99`–`0x9B` for
    /// VBI; other values are reserved/user-defined (retained verbatim).
    pub data_identifier: u8,
    /// The data units carried in this PES data field.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub data_units: Vec<DataUnit<'a>>,
}

impl<'a> DataField<'a> {
    /// Construct a data field from a `data_identifier` and its data units.
    pub fn new(data_identifier: u8, data_units: Vec<DataUnit<'a>>) -> Self {
        DataField {
            data_identifier,
            data_units,
        }
    }

    /// Total wire size: 1 (`data_identifier`) + every data unit.
    pub fn serialized_len(&self) -> usize {
        1 + self
            .data_units
            .iter()
            .map(DataUnit::serialized_len)
            .sum::<usize>()
    }

    /// Parse a PES data field from `data`: the `data_identifier` byte then a
    /// run of data units until the buffer is exhausted.
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        if data.is_empty() {
            return Err(Error::BufferTooShort {
                need: 1,
                have: 0,
                what: "data_identifier",
            });
        }
        let data_identifier = data[0];
        let mut data_units = Vec::new();
        let mut off = 1;
        while off < data.len() {
            let (unit, consumed) = DataUnit::parse(&data[off..])?;
            data_units.push(unit);
            off += consumed;
        }
        Ok(DataField {
            data_identifier,
            data_units,
        })
    }

    /// Serialize the data field into `out`, returning bytes written.
    pub fn serialize_into(&self, out: &mut [u8]) -> Result<usize> {
        let total = self.serialized_len();
        if out.len() < total {
            return Err(Error::OutputBufferTooSmall {
                need: total,
                have: out.len(),
            });
        }
        out[0] = self.data_identifier;
        let mut off = 1;
        for unit in &self.data_units {
            off += unit.serialize_into(&mut out[off..])?;
        }
        Ok(off)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::line_header::LineHeader;
    use alloc::vec;

    // Round-trip one data unit through exact wire bytes and reparse.
    fn unit_round_trip(unit: &DataUnit, expected_wire: &[u8]) {
        let mut out = vec![0u8; unit.serialized_len()];
        let n = unit.serialize_into(&mut out).unwrap();
        assert_eq!(n, unit.serialized_len());
        assert_eq!(out, expected_wire, "exact wire bytes");
        let (re, consumed) = DataUnit::parse(&out).unwrap();
        assert_eq!(consumed, out.len());
        assert_eq!(&re, unit, "reparse must equal the original");
    }

    #[test]
    fn teletext_exact_wire_bytes() {
        let block = [0xAAu8; TXT_DATA_BLOCK_LEN];
        let field = TeletextDataField {
            header: LineHeader::new(true, 7), // parity=1, line_offset=7
            framing_code: FRAMING_CODE_EBU,
            txt_data_block: block,
        };
        let unit = DataUnit::teletext(DataUnitId::EbuTeletextSubtitle, field);

        // id 0x03, length 0x2C (44), header byte 11 1 00111 = 0xE7, framing 0xE4.
        let mut expected = vec![0x03, 0x2C, 0xE7, FRAMING_CODE_EBU];
        expected.extend_from_slice(&block);
        assert_eq!(unit.data_unit_length(), TELETEXT_DATA_UNIT_LENGTH as usize);
        unit_round_trip(&unit, &expected);
    }

    #[test]
    fn vps_exact_wire_bytes() {
        let block = [
            0x01u8, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D,
        ];
        let field = VpsDataField {
            header: LineHeader::new(true, 16), // parity=1, line_offset=16
            vps_data_block: block,
        };
        let unit = DataUnit::vps(field);

        // id 0xC3, length 14 (0x0E), header 11 1 10000 = 0xF0.
        let mut expected = vec![0xC3, 0x0E, 0xF0];
        expected.extend_from_slice(&block);
        unit_round_trip(&unit, &expected);
    }

    #[test]
    fn wss_exact_wire_bytes_and_bit_packing() {
        // wss_data_block = 0x3A5C (14 bits: 11 1010 0101 1100).
        let field = WssDataField {
            header: LineHeader::new(true, 23), // parity=1, line_offset=23
            wss_data_block: 0x3A5C,
        };
        let unit = DataUnit::wss(field);

        // header 11 1 10111 = 0xF7.
        // byte1 = wss[13:6] = 0x3A5C >> 6 = 0xE9.
        // byte2 = (wss[5:0] << 2) | 11 = ((0x3A5C & 0x3F) << 2) | 3
        //       = (0x1C << 2) | 3 = 0x70 | 3 = 0x73.
        let expected = vec![0xC4, 0x03, 0xF7, 0xE9, 0x73];
        unit_round_trip(&unit, &expected);
    }

    #[test]
    fn cc_exact_wire_bytes() {
        let field = ClosedCaptioningDataField {
            header: LineHeader::new(false, 21), // parity=0, line_offset=21
            closed_captioning_data_block: 0x9425,
        };
        let unit = DataUnit::closed_captioning(field);

        // header 11 0 10101 = 0xD5, then 0x94 0x25.
        let expected = vec![0xC5, 0x03, 0xD5, 0x94, 0x25];
        unit_round_trip(&unit, &expected);
    }

    #[test]
    fn monochrome_exact_wire_bytes() {
        let samples = [0x10u8, 0x40, 0x80, 0xEB];
        let field = MonochromeDataField {
            first_segment: true,
            last_segment: false,
            field_parity: true,
            line_offset: 10,
            first_pixel_position: 0x0123,
            samples: &samples,
        };
        let unit = DataUnit::monochrome(field);

        // b0 = 1 0 1 01010 = 0xAA. fpp = 0x0123. n_pixels = 4.
        let mut expected = vec![0xC6, 0x08, 0xAA, 0x01, 0x23, 0x04];
        expected.extend_from_slice(&samples);
        unit_round_trip(&unit, &expected);
    }

    #[test]
    fn stuffing_exact_wire_bytes() {
        let unit = DataUnit::stuffing(3);
        // id 0xFF, length 3, then three 0xFF stuffing bytes.
        let expected = vec![0xFF, 0x03, 0xFF, 0xFF, 0xFF];
        unit_round_trip(&unit, &expected);
    }

    #[test]
    fn opaque_reserved_round_trips() {
        let body = [0xDEu8, 0xAD, 0xBE];
        let unit = DataUnit {
            id: DataUnitId::Reserved(0x55),
            payload: DataUnitPayload::Opaque(&body),
        };
        let expected = vec![0x55, 0x03, 0xDE, 0xAD, 0xBE];
        unit_round_trip(&unit, &expected);
    }

    // Mutation bite: changing a typed field changes the serialized wire bytes.
    #[test]
    fn mutating_a_field_changes_wire_bytes() {
        let block = [0u8; VPS_DATA_BLOCK_LEN];
        let a = DataUnit::vps(VpsDataField {
            header: LineHeader::new(true, 16),
            vps_data_block: block,
        });
        let mut block_b = block;
        block_b[0] = 0xFF;
        let b = DataUnit::vps(VpsDataField {
            header: LineHeader::new(true, 16),
            vps_data_block: block_b,
        });

        let mut out_a = vec![0u8; a.serialized_len()];
        a.serialize_into(&mut out_a).unwrap();
        let mut out_b = vec![0u8; b.serialized_len()];
        b.serialize_into(&mut out_b).unwrap();
        assert_ne!(
            out_a, out_b,
            "different vps_data_block must change wire bytes"
        );

        // Mutating the line header (parity) also bites.
        let c = DataUnit::vps(VpsDataField {
            header: LineHeader::new(false, 16),
            vps_data_block: block,
        });
        let mut out_c = vec![0u8; c.serialized_len()];
        c.serialize_into(&mut out_c).unwrap();
        assert_ne!(out_a, out_c, "field_parity must change the header byte");
        assert_eq!(out_a[2] & 0b0010_0000, 0b0010_0000);
        assert_eq!(out_c[2] & 0b0010_0000, 0);
    }

    // ≥2-data_unit boundary test: VPS + WSS + a teletext unit in one data field.
    #[test]
    fn multi_unit_data_field_round_trip() {
        let vps = DataUnit::vps(VpsDataField {
            header: LineHeader::new(true, 16),
            vps_data_block: [0x11; VPS_DATA_BLOCK_LEN],
        });
        let wss = DataUnit::wss(WssDataField {
            header: LineHeader::new(true, 23),
            wss_data_block: 0x1234,
        });
        let block = [0x42u8; TXT_DATA_BLOCK_LEN];
        let txt = DataUnit::teletext(
            DataUnitId::EbuTeletextNonSubtitle,
            TeletextDataField {
                header: LineHeader::new(false, 9),
                framing_code: FRAMING_CODE_EBU,
                txt_data_block: block,
            },
        );

        let field = DataField::new(0x10, vec![vps, wss, txt]);
        let mut out = vec![0u8; field.serialized_len()];
        let n = field.serialize_into(&mut out).unwrap();
        assert_eq!(n, field.serialized_len());

        // First byte = data_identifier.
        assert_eq!(out[0], 0x10);

        let parsed = DataField::parse(&out).unwrap();
        assert_eq!(parsed, field, "multi-unit data field must round-trip");
        assert_eq!(parsed.data_units.len(), 3);
        assert_eq!(parsed.data_units[0].id, DataUnitId::Vps);
        assert_eq!(parsed.data_units[1].id, DataUnitId::Wss);
        assert_eq!(parsed.data_units[2].id, DataUnitId::EbuTeletextNonSubtitle);

        // Byte-exact re-serialize.
        let mut out2 = vec![0u8; parsed.serialized_len()];
        parsed.serialize_into(&mut out2).unwrap();
        assert_eq!(out, out2);
    }

    #[test]
    fn rejects_truncated_data_unit() {
        // id 0xC3 (VPS) claims length 14 but body is short.
        let data = [0xC3u8, 0x0E, 0x00];
        assert!(DataUnit::parse(&data).is_err());
    }

    #[test]
    fn rejects_length_mismatch() {
        // VPS body that's longer than the fixed 14 -> typed size != length.
        let data = [
            0xC3u8, 0x0F, 0xF0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14,
        ];
        assert!(matches!(
            DataUnit::parse(&data),
            Err(Error::InvalidDataUnitLength { .. })
        ));
    }

    #[test]
    fn wss_bit_packing_recovers_value() {
        for v in [0u16, 1, 0x3FFF, 0x2A55, 0x1FFF] {
            let f = WssDataField {
                header: LineHeader::new(true, 23),
                wss_data_block: v,
            };
            let mut out = [0u8; WSS_FIELD_LEN];
            f.serialize_into(&mut out).unwrap();
            let re = WssDataField::parse(&out).unwrap();
            assert_eq!(re.wss_data_block, v, "wss value {v:#06X}");
            // RFU tail must be 11.
            assert_eq!(out[2] & 0b11, WSS_RESERVED_TAIL);
        }
    }

    // Finding 1: n_pixels=0 must be rejected (ETSI EN 301 775 §4.9.2).
    #[test]
    fn monochrome_rejects_zero_n_pixels() {
        // b0 = 1 1 1 00111 (first+last segment, field_parity=1, line_offset=7).
        // first_pixel_position = 0x0000. n_pixels = 0x00 (invalid).
        let data = [0b1110_0111u8, 0x00, 0x00, 0x00];
        let result = MonochromeDataField::parse(&data);
        assert!(
            matches!(
                result,
                Err(crate::error::Error::InvalidField {
                    what: "n_pixels",
                    ..
                })
            ),
            "n_pixels=0 must be rejected with InvalidField, got: {result:?}"
        );
    }

    // Finding 3: mutation-bite tests for each remaining typed payload.

    #[test]
    fn mutating_teletext_framing_code_changes_wire_bytes() {
        let block = [0xBBu8; TXT_DATA_BLOCK_LEN];
        let a = DataUnit::teletext(
            DataUnitId::EbuTeletextNonSubtitle,
            TeletextDataField {
                header: LineHeader::new(true, 7),
                framing_code: FRAMING_CODE_EBU,
                txt_data_block: block,
            },
        );
        let b = DataUnit::teletext(
            DataUnitId::EbuTeletextNonSubtitle,
            TeletextDataField {
                header: LineHeader::new(true, 7),
                framing_code: FRAMING_CODE_INVERTED,
                txt_data_block: block,
            },
        );
        let mut out_a = vec![0u8; a.serialized_len()];
        a.serialize_into(&mut out_a).unwrap();
        let mut out_b = vec![0u8; b.serialized_len()];
        b.serialize_into(&mut out_b).unwrap();
        assert_ne!(
            out_a, out_b,
            "different framing_code must change wire bytes"
        );
    }

    #[test]
    fn mutating_teletext_txt_data_block_changes_wire_bytes() {
        let block_a = [0x00u8; TXT_DATA_BLOCK_LEN];
        let mut block_b = block_a;
        block_b[0] = 0xFF;
        let a = DataUnit::teletext(
            DataUnitId::EbuTeletextNonSubtitle,
            TeletextDataField {
                header: LineHeader::new(true, 7),
                framing_code: FRAMING_CODE_EBU,
                txt_data_block: block_a,
            },
        );
        let b = DataUnit::teletext(
            DataUnitId::EbuTeletextNonSubtitle,
            TeletextDataField {
                header: LineHeader::new(true, 7),
                framing_code: FRAMING_CODE_EBU,
                txt_data_block: block_b,
            },
        );
        let mut out_a = vec![0u8; a.serialized_len()];
        a.serialize_into(&mut out_a).unwrap();
        let mut out_b = vec![0u8; b.serialized_len()];
        b.serialize_into(&mut out_b).unwrap();
        assert_ne!(
            out_a, out_b,
            "different txt_data_block must change wire bytes"
        );
    }

    #[test]
    fn mutating_wss_data_block_changes_wire_bytes() {
        let a = DataUnit::wss(WssDataField {
            header: LineHeader::new(true, 23),
            wss_data_block: 0x0000,
        });
        let b = DataUnit::wss(WssDataField {
            header: LineHeader::new(true, 23),
            wss_data_block: 0x3FFF,
        });
        let mut out_a = vec![0u8; a.serialized_len()];
        a.serialize_into(&mut out_a).unwrap();
        let mut out_b = vec![0u8; b.serialized_len()];
        b.serialize_into(&mut out_b).unwrap();
        assert_ne!(
            out_a, out_b,
            "different wss_data_block must change wire bytes"
        );
    }

    #[test]
    fn mutating_cc_data_block_changes_wire_bytes() {
        let a = DataUnit::closed_captioning(ClosedCaptioningDataField {
            header: LineHeader::new(false, 21),
            closed_captioning_data_block: 0x0000,
        });
        let b = DataUnit::closed_captioning(ClosedCaptioningDataField {
            header: LineHeader::new(false, 21),
            closed_captioning_data_block: 0xFFFF,
        });
        let mut out_a = vec![0u8; a.serialized_len()];
        a.serialize_into(&mut out_a).unwrap();
        let mut out_b = vec![0u8; b.serialized_len()];
        b.serialize_into(&mut out_b).unwrap();
        assert_ne!(
            out_a, out_b,
            "different closed_captioning_data_block must change wire bytes"
        );
    }

    #[test]
    fn mutating_monochrome_first_segment_flag_changes_wire_bytes() {
        let samples = [0x10u8, 0x80];
        let a = DataUnit::monochrome(MonochromeDataField {
            first_segment: true,
            last_segment: false,
            field_parity: true,
            line_offset: 10,
            first_pixel_position: 0,
            samples: &samples,
        });
        let b = DataUnit::monochrome(MonochromeDataField {
            first_segment: false,
            last_segment: false,
            field_parity: true,
            line_offset: 10,
            first_pixel_position: 0,
            samples: &samples,
        });
        let mut out_a = vec![0u8; a.serialized_len()];
        a.serialize_into(&mut out_a).unwrap();
        let mut out_b = vec![0u8; b.serialized_len()];
        b.serialize_into(&mut out_b).unwrap();
        assert_ne!(
            out_a, out_b,
            "different first_segment flag must change the first wire byte"
        );
    }

    #[test]
    fn mutating_monochrome_y_sample_changes_wire_bytes() {
        let samples_a = [0x10u8, 0x80];
        let mut samples_b = samples_a;
        samples_b[0] = 0xFF;
        let a = DataUnit::monochrome(MonochromeDataField {
            first_segment: true,
            last_segment: true,
            field_parity: true,
            line_offset: 10,
            first_pixel_position: 0,
            samples: &samples_a,
        });
        let b = DataUnit::monochrome(MonochromeDataField {
            first_segment: true,
            last_segment: true,
            field_parity: true,
            line_offset: 10,
            first_pixel_position: 0,
            samples: &samples_b,
        });
        let mut out_a = vec![0u8; a.serialized_len()];
        a.serialize_into(&mut out_a).unwrap();
        let mut out_b = vec![0u8; b.serialized_len()];
        b.serialize_into(&mut out_b).unwrap();
        assert_ne!(out_a, out_b, "different Y sample must change wire bytes");
    }

    // Finding 5: exhaustiveness cross-check that every non-opaque DataUnitId
    // variant maps to a typed DataUnitPayload arm. This is intentionally NOT a
    // declarative macro dispatch (the dispatch is a hand-written match in
    // DataUnitPayload::parse); this test ensures a future variant added to
    // DataUnitId without a matching payload branch fails CI rather than silently
    // falling to Opaque.
    #[test]
    fn every_non_opaque_data_unit_id_has_a_typed_payload() {
        use crate::data_unit_id::{
            ID_CLOSED_CAPTIONING, ID_EBU_TELETEXT_NON_SUBTITLE, ID_EBU_TELETEXT_SUBTITLE,
            ID_INVERTED_TELETEXT, ID_MONOCHROME_422_SAMPLES, ID_STUFFING, ID_VPS, ID_WSS,
        };
        // Every ID that must produce a typed (non-Opaque) payload.
        let typed_ids: &[u8] = &[
            ID_EBU_TELETEXT_NON_SUBTITLE,
            ID_EBU_TELETEXT_SUBTITLE,
            ID_INVERTED_TELETEXT,
            ID_VPS,
            ID_WSS,
            ID_CLOSED_CAPTIONING,
            // Monochrome needs n_pixels > 0; supply a minimal 1-pixel body.
            ID_MONOCHROME_422_SAMPLES,
            ID_STUFFING,
        ];

        // Minimal valid bodies for each (body = data_unit_length bytes).
        let teletext_body = {
            let mut b = vec![0u8; TELETEXT_FIELD_LEN];
            // header = canonical RFU=11, parity=1, line_offset=7.
            b[0] = 0xE7;
            b[1] = FRAMING_CODE_EBU;
            b
        };
        let vps_body = {
            let mut b = vec![0u8; VPS_FIELD_LEN];
            b[0] = 0xF0; // header
            b
        };
        let wss_body = {
            let mut b = vec![0u8; WSS_FIELD_LEN];
            b[0] = 0xF7; // header
            b
        };
        let cc_body = {
            let mut b = vec![0u8; CC_FIELD_LEN];
            b[0] = 0xD5; // header
            b
        };
        // Monochrome: 4-byte header + 1 Y sample (n_pixels=1).
        let mono_body = vec![0b1110_0111u8, 0x00, 0x00, 0x01, 0x80];
        let stuffing_body = vec![0xFFu8; 3];

        let bodies: &[&[u8]] = &[
            &teletext_body,
            &teletext_body,
            &teletext_body,
            &vps_body,
            &wss_body,
            &cc_body,
            &mono_body,
            &stuffing_body,
        ];

        for (&id, &body) in typed_ids.iter().zip(bodies.iter()) {
            let du_id = DataUnitId::from_u8(id);
            let payload = DataUnitPayload::parse(du_id, body)
                .unwrap_or_else(|e| panic!("id={id:#04X} parse failed: {e}"));
            assert!(
                !matches!(payload, DataUnitPayload::Opaque(_)),
                "id={id:#04X} ({}) fell to Opaque — add a typed dispatch arm",
                DataUnitId::from_u8(id).name()
            );
        }
    }
}
