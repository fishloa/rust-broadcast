//! Object Data Segment — ETSI EN 300 743 §7.2.5, Table 17 (segment_type 0x13).
//!
//! Contains object data: either interlaced/bitmap pixel-data sub-blocks,
//! a character string, or a progressive (zlib-compressed) pixel block.

use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// The object_data_segment segment_type.
pub const SEGMENT_TYPE: u8 = 0x13;
/// Header: 6 bytes.
pub const HEADER_LEN: usize = 6;
/// Fixed after header: object_id(2) + version/coding/flags(1) = 3 bytes.
pub const FIXED_LEN: usize = 3;
/// Length fields for interlaced coding: top_field_data_block_length(2) + bottom_field_data_block_length(2).
pub const INTERLACE_LEN_LEN: usize = 4;
/// Progressive pixel block header: bitmap_width(2) + bitmap_height(2) + compressed_len(2).
pub const PROGRESSIVE_HEADER_LEN: usize = 6;

/// Object coding method as defined in Table 18.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[repr(u8)]
#[non_exhaustive]
pub enum ObjectCodingMethod {
    /// Coding of pixels (interlaced).
    Pixels = 0x00,
    /// Coded as a string of characters.
    Characters = 0x01,
    /// Progressive coding of pixels.
    ProgressivePixels = 0x02,
    /// Reserved.
    Reserved(u8),
}

impl ObjectCodingMethod {
    /// Human-readable name for this coding method.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Pixels => "pixels",
            Self::Characters => "characters",
            Self::ProgressivePixels => "progressive_pixels",
            Self::Reserved(_) => "reserved",
        }
    }

    fn to_bits(self) -> u8 {
        match self {
            Self::Pixels => 0x00,
            Self::Characters => 0x01,
            Self::ProgressivePixels => 0x02,
            Self::Reserved(v) => v & 0x03,
        }
    }
}

dvb_common::impl_spec_display!(ObjectCodingMethod, Reserved);

/// Data type for pixel-data sub-blocks as defined in Table 21.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[repr(u8)]
#[non_exhaustive]
pub enum DataType {
    /// 2-bit/pixel code string.
    CodeString2Bit = 0x10,
    /// 4-bit/pixel code string.
    CodeString4Bit = 0x11,
    /// 8-bit/pixel code string.
    CodeString8Bit = 0x12,
    /// 2-to-4-bit map table (2 bytes).
    MapTable2To4 = 0x20,
    /// 2-to-8-bit map table (4 bytes).
    MapTable2To8 = 0x21,
    /// 4-to-8-bit map table (16 bytes).
    MapTable4To8 = 0x22,
    /// End of object line code (0 bytes of data).
    EndOfLine = 0xF0,
    /// Reserved.
    Reserved(u8),
}

impl DataType {
    /// Human-readable name for this data type.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::CodeString2Bit => "2-bit_code_string",
            Self::CodeString4Bit => "4-bit_code_string",
            Self::CodeString8Bit => "8-bit_code_string",
            Self::MapTable2To4 => "2_to_4_map_table",
            Self::MapTable2To8 => "2_to_8_map_table",
            Self::MapTable4To8 => "4_to_8_map_table",
            Self::EndOfLine => "end_of_line",
            Self::Reserved(_) => "reserved",
        }
    }
}

dvb_common::impl_spec_display!(DataType, Reserved);

/// Size of a 2-to-4 bit map table in bytes.
const MAP_TABLE_2TO4_BYTES: usize = 2;
/// Size of a 2-to-8 bit map table in bytes.
const MAP_TABLE_2TO8_BYTES: usize = 4;
/// Size of a 4-to-8 bit map table in bytes.
const MAP_TABLE_4TO8_BYTES: usize = 16;

/// A pixel-data sub-block (Table 20).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PixelDataSubBlock<'a> {
    /// The data_type.
    pub data_type: DataType,
    /// The payload data following the data_type byte.
    /// For code strings this is the entire RLE token stream including
    /// the end-of-string marker and byte-alignment stuffing.
    /// For map tables this is the fixed-size table data.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub data: &'a [u8],
}

impl PixelDataSubBlock<'_> {
    fn serialized_len(&self) -> usize {
        1 + self.data.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) {
        buf[0] = match self.data_type {
            DataType::CodeString2Bit => 0x10,
            DataType::CodeString4Bit => 0x11,
            DataType::CodeString8Bit => 0x12,
            DataType::MapTable2To4 => 0x20,
            DataType::MapTable2To8 => 0x21,
            DataType::MapTable4To8 => 0x22,
            DataType::EndOfLine => 0xF0,
            DataType::Reserved(v) => v,
        };
        buf[1..1 + self.data.len()].copy_from_slice(self.data);
    }
}

/// An interlaced-pixels object data payload (coding method 0x00).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct InterlacedPixelsData<'a> {
    /// Top-field pixel-data sub-blocks.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub top_sub_blocks: alloc::vec::Vec<PixelDataSubBlock<'a>>,
    /// Bottom-field pixel-data sub-blocks.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub bottom_sub_blocks: alloc::vec::Vec<PixelDataSubBlock<'a>>,
    /// Stuffing byte if present.
    pub stuffing_byte: Option<u8>,
}

/// A progressive pixel block (Table 27, coding method 0x02).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ProgressivePixelBlock<'a> {
    /// Bitmap width in pixels.
    pub bitmap_width: u16,
    /// Bitmap height in pixels.
    pub bitmap_height: u16,
    /// Compressed data (zlib/DEFLATE) — opaque.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub compressed_data: &'a [u8],
}

/// Object data payload variants.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ObjectDataPayload<'a> {
    /// Interlaced/bitmap pixel data.
    #[cfg_attr(feature = "serde", serde(borrow))]
    InterlacedPixels(InterlacedPixelsData<'a>),
    /// Character string.
    Characters {
        /// Number of character codes.
        number_of_codes: u8,
        /// Character codes (16-bit each).
        character_codes: alloc::vec::Vec<u16>,
    },
    /// Progressive pixel block (zlib-compressed).
    #[cfg_attr(feature = "serde", serde(borrow))]
    ProgressivePixels(ProgressivePixelBlock<'a>),
    /// Reserved coding method — raw data preserved.
    Reserved {
        /// The coding method value (0x03 or unknown).
        coding_method: u8,
        /// Raw payload bytes.
        #[cfg_attr(feature = "serde", serde(skip))]
        data: &'a [u8],
    },
}

/// Object Data Segment.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ObjectDataSegment<'a> {
    /// The page_id from the segment header.
    pub page_id: u16,
    /// Object identifier.
    pub object_id: u16,
    /// Object version number (modulo 16).
    pub object_version_number: u8,
    /// Object coding method.
    pub object_coding_method: ObjectCodingMethod,
    /// Non-modifying colour flag.
    pub non_modifying_colour_flag: bool,
    /// The payload data.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub payload: ObjectDataPayload<'a>,
}

/// Scan a 2-bit/pixel_code_string to find its end marker and total byte length.
/// Returns the number of bytes consumed INCLUDING any trailing 2-bit stuffing.
/// Table 22, 23.
fn scan_2bit_code_string(data: &[u8]) -> usize {
    let mut bitpos: usize = 0;
    let bits = |bp: usize, n: usize| -> u8 {
        let byte_idx = bp / 8;
        let bit_idx = bp % 8;
        if byte_idx >= data.len() {
            return 0;
        }
        if bit_idx + n <= 8 {
            (data[byte_idx] >> (8 - bit_idx - n)) & ((1u16 << n) - 1) as u8
        } else {
            let first_bits = 8 - bit_idx;
            let v1 = ((data[byte_idx] & ((1u8 << first_bits) - 1)) as u16) << (n - first_bits);
            let v2 = if byte_idx + 1 < data.len() {
                (data[byte_idx + 1] >> (8 - (n - first_bits))) as u16
            } else {
                0
            };
            (v1 | v2) as u8
        }
    };
    loop {
        let b2 = bits(bitpos, 2);
        bitpos += 2;
        if b2 != 0 {
            continue;
        }
        // 2-bit_zero, check switch_1
        let s1 = bits(bitpos, 1);
        bitpos += 1;
        if s1 == 1 {
            // run_length_3-10 (3 bits) + 2-bitpixel-code (2 bits)
            bitpos += 3 + 2;
            continue;
        }
        let s2 = bits(bitpos, 1);
        bitpos += 1;
        if s2 == 1 {
            // 1 pixel in colour 0
            continue;
        }
        let s3 = bits(bitpos, 2);
        bitpos += 2;
        match s3 {
            0b00 => break,    // end of string
            0b01 => continue, // 2 pixels in colour 0
            0b10 => {
                bitpos += 4 + 2; // run_length_12-27 + pixel-code
            }
            0b11 => {
                bitpos += 8 + 2; // run_length_29-284 + pixel-code
            }
            _ => {}
        }
    }
    // Byte-align: the sub-block includes 2_stuff_bits if not byte-aligned
    let byte_len = bitpos.div_ceil(8);
    byte_len.min(data.len())
}

/// Scan a 4-bit/pixel_code_string to find its end marker and total byte length.
/// Returns number of bytes consumed INCLUDING any trailing 4-bit stuffing.
/// Table 24, 25.
fn scan_4bit_code_string(data: &[u8]) -> usize {
    let mut bitpos: usize = 0;
    let bits = |bp: usize, n: usize| -> u8 {
        let byte_idx = bp / 8;
        let bit_idx = bp % 8;
        if byte_idx >= data.len() {
            return 0;
        }
        if bit_idx + n <= 8 {
            (data[byte_idx] >> (8 - bit_idx - n)) & ((1u16 << n) - 1) as u8
        } else {
            let first_bits = 8 - bit_idx;
            let v1 = ((data[byte_idx] & ((1u8 << first_bits) - 1)) as u16) << (n - first_bits);
            let v2 = if byte_idx + 1 < data.len() {
                (data[byte_idx + 1] >> (8 - (n - first_bits))) as u16
            } else {
                0
            };
            (v1 | v2) as u8
        }
    };
    loop {
        let b4 = bits(bitpos, 4);
        bitpos += 4;
        if b4 != 0 {
            continue;
        }
        // 4-bit_zero
        let s1 = bits(bitpos, 1);
        bitpos += 1;
        if s1 == 0 {
            let n3 = bits(bitpos, 3);
            bitpos += 3;
            if n3 == 0 {
                // end_of_string_signal
                break;
            }
            // run_length_3-9 in colour 0
            continue;
        }
        let s2 = bits(bitpos, 1);
        bitpos += 1;
        if s2 == 0 {
            // run_length_4-7 (2 bits) + 4-bit_pixel-code
            bitpos += 2 + 4;
            continue;
        }
        let s3 = bits(bitpos, 2);
        bitpos += 2;
        match s3 {
            0b00 | 0b01 => continue, // 1 or 2 pixels in colour 0
            0b10 => {
                bitpos += 4 + 4; // run_length_9-24 + pixel-code
            }
            0b11 => {
                bitpos += 8 + 4; // run_length_25-280 + pixel-code
            }
            _ => {}
        }
    }
    // Byte-align: if not byte-aligned, 4_stuff_bits are implied
    let byte_len = bitpos.div_ceil(8);
    byte_len.min(data.len())
}

/// Scan an 8-bit/pixel_code_string to find its end marker and total byte length.
/// Table 26.
fn scan_8bit_code_string(data: &[u8]) -> usize {
    let mut pos: usize = 0;
    loop {
        if pos >= data.len() {
            break;
        }
        if data[pos] != 0x00 {
            pos += 1;
            continue;
        }
        if pos + 1 >= data.len() {
            pos += 1;
            break;
        }
        let b = data[pos + 1];
        let s1 = (b >> 7) & 1;
        if s1 == 0 {
            let rl = b & 0x7F;
            if rl == 0 {
                // end_of_string_signal
                pos += 2;
                break;
            }
            // run_length_1-127 in colour 0
            pos += 2;
        } else {
            // run_length_3-127 + 8-bitpixel-code
            pos += 3;
        }
    }
    pos.min(data.len())
}

/// Parse pixel-data sub-blocks from raw bytes, bounded by the given field length.
///
/// Walks data_type-delimited sub-blocks: code strings are scanned to their
/// terminator, map tables have fixed sizes, 0xF0 (end-of-line) has no data.
fn parse_pixel_sub_blocks<'a>(
    bytes: &'a [u8],
    field_len: usize,
    what: &'static str,
) -> Result<alloc::vec::Vec<PixelDataSubBlock<'a>>> {
    let end = field_len.min(bytes.len());
    let field = &bytes[..end];
    let mut blocks = alloc::vec::Vec::new();
    let mut pos: usize = 0;
    while pos < field.len() {
        if pos >= field.len() {
            break;
        }
        let data_type_byte = field[pos];
        let data_type = match data_type_byte {
            0x10 => DataType::CodeString2Bit,
            0x11 => DataType::CodeString4Bit,
            0x12 => DataType::CodeString8Bit,
            0x20 => DataType::MapTable2To4,
            0x21 => DataType::MapTable2To8,
            0x22 => DataType::MapTable4To8,
            0xF0 => DataType::EndOfLine,
            v => DataType::Reserved(v),
        };
        pos += 1; // consume data_type byte

        let data_len = match data_type_byte {
            0x20 => MAP_TABLE_2TO4_BYTES,
            0x21 => MAP_TABLE_2TO8_BYTES,
            0x22 => MAP_TABLE_4TO8_BYTES,
            0xF0 => 0,
            0x10 => scan_2bit_code_string(&field[pos..]),
            0x11 => scan_4bit_code_string(&field[pos..]),
            0x12 => scan_8bit_code_string(&field[pos..]),
            _ => {
                // Unknown: consume rest of field
                field.len() - pos
            }
        };

        let block_end = (pos + data_len).min(field.len());
        let block_data = &field[pos..block_end];

        blocks.push(PixelDataSubBlock {
            data_type,
            data: block_data,
        });
        pos = block_end;
    }
    // Validate field length was exactly consumed
    if pos != end {
        return Err(Error::BufferTooShort {
            need: end,
            have: pos,
            what,
        });
    }
    Ok(blocks)
}

impl<'a> Parse<'a> for ObjectDataSegment<'a> {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN + FIXED_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN + FIXED_LEN,
                have: bytes.len(),
                what: "object_data_segment",
            });
        }
        if bytes[1] != SEGMENT_TYPE {
            return Err(Error::UnknownSegmentType(bytes[1]));
        }
        let page_id = u16::from_be_bytes([bytes[2], bytes[3]]);
        let segment_length = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
        let total = HEADER_LEN + segment_length;
        if bytes.len() < total {
            return Err(Error::BufferTooShort {
                need: total,
                have: bytes.len(),
                what: "object_data_segment data",
            });
        }
        let body = &bytes[HEADER_LEN..HEADER_LEN + segment_length];
        if body.len() < FIXED_LEN {
            return Err(Error::BufferTooShort {
                need: FIXED_LEN,
                have: body.len(),
                what: "object_data_segment body",
            });
        }
        let object_id = u16::from_be_bytes([body[0], body[1]]);
        let object_version_number = body[2] >> 4;
        let coding_method_bits = (body[2] >> 2) & 0x03;
        let non_modifying_colour_flag = (body[2] & 0x02) != 0;
        let reserved = body[2] & 0x01;
        // reserved bit tolerated per §7.2.0.2 forward compatibility
        let _ = reserved;

        let object_coding_method = match coding_method_bits {
            0x00 => ObjectCodingMethod::Pixels,
            0x01 => ObjectCodingMethod::Characters,
            0x02 => ObjectCodingMethod::ProgressivePixels,
            v => ObjectCodingMethod::Reserved(v),
        };

        let payload_data = &body[FIXED_LEN..];

        let payload = match coding_method_bits {
            0x00 => {
                if payload_data.len() < INTERLACE_LEN_LEN {
                    return Err(Error::BufferTooShort {
                        need: INTERLACE_LEN_LEN,
                        have: payload_data.len(),
                        what: "object_data interlace lengths",
                    });
                }
                let top_len = u16::from_be_bytes([payload_data[0], payload_data[1]]) as usize;
                let bottom_len = u16::from_be_bytes([payload_data[2], payload_data[3]]) as usize;

                let top_start = INTERLACE_LEN_LEN;
                let top_end = top_start + top_len;
                let bottom_start = top_end;
                let bottom_end = bottom_start + bottom_len;

                if payload_data.len() < bottom_end {
                    return Err(Error::BufferTooShort {
                        need: bottom_end,
                        have: payload_data.len(),
                        what: "object_data pixel sub-blocks",
                    });
                }

                // stuffing_length = segment_length - 7 - top_len - bottom_len
                // computed as: segment_length - (HEADER_LEN + FIXED_LEN + INTERLACE_LEN_LEN + top_len + bottom_len - HEADER_LEN - FIXED_LEN)?
                // simpler: stuffing = segment_length - 7 - top_len - bottom_len
                // 7 = 3 (fixed) + 4 (interlace lens)
                let stuffing_length = segment_length
                    .wrapping_sub(7)
                    .wrapping_sub(top_len)
                    .wrapping_sub(bottom_len);

                let mut stuffing_byte = None;
                if stuffing_length == 1 {
                    if bottom_end + 1 > payload_data.len() {
                        return Err(Error::BufferTooShort {
                            need: bottom_end + 1,
                            have: payload_data.len(),
                            what: "stuffing byte",
                        });
                    }
                    if payload_data[bottom_end] != 0x00 {
                        return Err(Error::BadStuffingByte(payload_data[bottom_end]));
                    }
                    stuffing_byte = Some(payload_data[bottom_end]);
                    let _ = stuffing_byte;
                    // Use the actual field length check
                }

                let top_sub_blocks = parse_pixel_sub_blocks(
                    &payload_data[top_start..top_end],
                    top_len,
                    "top pixel sub-blocks",
                )?;
                let bottom_sub_blocks = if bottom_len > 0 {
                    parse_pixel_sub_blocks(
                        &payload_data[bottom_start..bottom_end],
                        bottom_len,
                        "bottom pixel sub-blocks",
                    )?
                } else {
                    alloc::vec::Vec::new()
                };

                ObjectDataPayload::InterlacedPixels(InterlacedPixelsData {
                    top_sub_blocks,
                    bottom_sub_blocks,
                    stuffing_byte,
                })
            }
            0x01 => {
                if payload_data.is_empty() {
                    return Err(Error::BufferTooShort {
                        need: 1,
                        have: 0,
                        what: "character count",
                    });
                }
                let number_of_codes = payload_data[0] as usize;
                let codes_len = 1 + number_of_codes * 2;
                if payload_data.len() < codes_len {
                    return Err(Error::BufferTooShort {
                        need: codes_len,
                        have: payload_data.len(),
                        what: "character codes",
                    });
                }
                let mut codes = alloc::vec::Vec::with_capacity(number_of_codes);
                for i in 0..number_of_codes {
                    let ci = 1 + i * 2;
                    codes.push(u16::from_be_bytes([payload_data[ci], payload_data[ci + 1]]));
                }
                ObjectDataPayload::Characters {
                    number_of_codes: number_of_codes as u8,
                    character_codes: codes,
                }
            }
            0x02 => {
                if payload_data.len() < PROGRESSIVE_HEADER_LEN {
                    return Err(Error::BufferTooShort {
                        need: PROGRESSIVE_HEADER_LEN,
                        have: payload_data.len(),
                        what: "progressive pixel block",
                    });
                }
                let bitmap_width = u16::from_be_bytes([payload_data[0], payload_data[1]]);
                let bitmap_height = u16::from_be_bytes([payload_data[2], payload_data[3]]);
                let compressed_len =
                    u16::from_be_bytes([payload_data[4], payload_data[5]]) as usize;
                if payload_data.len() < PROGRESSIVE_HEADER_LEN + compressed_len {
                    return Err(Error::BufferTooShort {
                        need: PROGRESSIVE_HEADER_LEN + compressed_len,
                        have: payload_data.len(),
                        what: "compressed bitmap data",
                    });
                }
                ObjectDataPayload::ProgressivePixels(ProgressivePixelBlock {
                    bitmap_width,
                    bitmap_height,
                    compressed_data: &payload_data
                        [PROGRESSIVE_HEADER_LEN..PROGRESSIVE_HEADER_LEN + compressed_len],
                })
            }
            _ => ObjectDataPayload::Reserved {
                coding_method: coding_method_bits,
                data: payload_data,
            },
        };

        Ok(ObjectDataSegment {
            page_id,
            object_id,
            object_version_number,
            object_coding_method,
            non_modifying_colour_flag,
            payload,
        })
    }
}

impl Serialize for ObjectDataSegment<'_> {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let body_len = FIXED_LEN
            + match &self.payload {
                ObjectDataPayload::InterlacedPixels(ip) => {
                    let top_len: usize = ip.top_sub_blocks.iter().map(|s| s.serialized_len()).sum();
                    let bottom_len: usize = ip
                        .bottom_sub_blocks
                        .iter()
                        .map(|s| s.serialized_len())
                        .sum();
                    INTERLACE_LEN_LEN
                        + top_len
                        + bottom_len
                        + if ip.stuffing_byte.is_some() { 1 } else { 0 }
                }
                ObjectDataPayload::Characters {
                    character_codes, ..
                } => 1 + character_codes.len() * 2,
                ObjectDataPayload::ProgressivePixels(pp) => {
                    PROGRESSIVE_HEADER_LEN + pp.compressed_data.len()
                }
                ObjectDataPayload::Reserved { data, .. } => data.len(),
            };
        HEADER_LEN + body_len
    }

    fn serialize_into(&self, buf: &mut [u8]) -> core::result::Result<usize, Self::Error> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::BufferTooShort {
                need: len,
                have: buf.len(),
                what: "object_data_segment serialize",
            });
        }
        buf[0] = 0x0F;
        buf[1] = SEGMENT_TYPE;
        buf[2..4].copy_from_slice(&self.page_id.to_be_bytes());
        let seg_len = (len - HEADER_LEN) as u16;
        buf[4..6].copy_from_slice(&seg_len.to_be_bytes());

        buf[6..8].copy_from_slice(&self.object_id.to_be_bytes());
        buf[8] = (self.object_version_number << 4)
            | (self.object_coding_method.to_bits() << 2)
            | (u8::from(self.non_modifying_colour_flag) << 1);

        let mut off = HEADER_LEN + FIXED_LEN;
        match &self.payload {
            ObjectDataPayload::InterlacedPixels(ip) => {
                let top_len: u16 = ip
                    .top_sub_blocks
                    .iter()
                    .map(|s| s.serialized_len() as u16)
                    .sum();
                let bottom_len: u16 = ip
                    .bottom_sub_blocks
                    .iter()
                    .map(|s| s.serialized_len() as u16)
                    .sum();
                buf[off..off + 2].copy_from_slice(&top_len.to_be_bytes());
                buf[off + 2..off + 4].copy_from_slice(&bottom_len.to_be_bytes());
                off += INTERLACE_LEN_LEN;
                for sub in &ip.top_sub_blocks {
                    sub.serialize_into(&mut buf[off..]);
                    off += sub.serialized_len();
                }
                for sub in &ip.bottom_sub_blocks {
                    sub.serialize_into(&mut buf[off..]);
                    off += sub.serialized_len();
                }
                if ip.stuffing_byte.is_some() {
                    buf[off] = 0x00;
                    off += 1;
                }
            }
            ObjectDataPayload::Characters {
                number_of_codes,
                character_codes,
            } => {
                buf[off] = *number_of_codes;
                off += 1;
                for code in character_codes {
                    buf[off..off + 2].copy_from_slice(&code.to_be_bytes());
                    off += 2;
                }
            }
            ObjectDataPayload::ProgressivePixels(pp) => {
                buf[off..off + 2].copy_from_slice(&pp.bitmap_width.to_be_bytes());
                buf[off + 2..off + 4].copy_from_slice(&pp.bitmap_height.to_be_bytes());
                let clen = pp.compressed_data.len() as u16;
                buf[off + 4..off + 6].copy_from_slice(&clen.to_be_bytes());
                off += PROGRESSIVE_HEADER_LEN;
                buf[off..off + pp.compressed_data.len()].copy_from_slice(pp.compressed_data);
                off += pp.compressed_data.len();
            }
            ObjectDataPayload::Reserved { data, .. } => {
                buf[off..off + data.len()].copy_from_slice(data);
                off += data.len();
            }
        }
        debug_assert_eq!(off, len);
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dvb_common::{Parse, Serialize};

    #[test]
    fn round_trip_pixels_no_stuffing() {
        let bytes = [
            0x0F, 0x13, 0x00, 0x01, 0x00, 0x0A, 0x00, 0x0A, 0x00, 0x00, 0x03, 0x00, 0x00, 0xF0,
            0x10, 0x0A,
        ];
        let seg = ObjectDataSegment::parse(&bytes).unwrap();
        assert_eq!(seg.object_id, 10);
        assert_eq!(seg.object_coding_method, ObjectCodingMethod::Pixels);
        match &seg.payload {
            ObjectDataPayload::InterlacedPixels(ip) => {
                assert_eq!(ip.top_sub_blocks.len(), 2);
                assert_eq!(ip.top_sub_blocks[0].data_type, DataType::EndOfLine);
                assert_eq!(ip.top_sub_blocks[1].data_type, DataType::CodeString2Bit);
            }
            _ => panic!("expected InterlacedPixels"),
        }
        let out = seg.to_bytes();
        assert_eq!(out, bytes);

        // Biting test
        let mut seg2 = seg.clone();
        seg2.object_id = 20;
        let out2 = seg2.to_bytes();
        assert_ne!(out2, bytes);
        let reparse = ObjectDataSegment::parse(&out2).unwrap();
        assert_eq!(reparse.object_id, 20);
    }

    #[test]
    fn round_trip_multiple_sub_blocks() {
        // Top field: map-table(0x20) + end-of-line(0xF0) + 4-bit code string
        let map_table: [u8; 2] = [0x00, 0x00];
        // 4-bit code string: 5 pixels of colour 1, then end-of-string
        // 0001 0001 0001 0001 0001 0000 000 → 5 ones, zero, then switch_1=0, next 3 bits=000 → end
        let four_bit_data = [0x11, 0x11, 0x11, 0x00]; // 4 bytes
        let body: alloc::vec::Vec<u8> = [
            &[0x00u8, 0x0B, 0x00][..],
            &[0x00u8, 0x09, 0x00, 0x00][..], // top_len=9 (1+2+1+1+4)
            &[0x20u8][..],
            &map_table[..],
            &[0xF0u8][..],
            &[0x11u8][..],
            &four_bit_data[..],
        ]
        .concat();

        let mut bytes = alloc::vec![0x0F, 0x13, 0x00, 0x01]; // header up to segment_length
        let body_len = body.len() as u16;
        bytes.extend_from_slice(&body_len.to_be_bytes());
        bytes.extend_from_slice(&body);

        let seg = ObjectDataSegment::parse(&bytes).unwrap();
        match &seg.payload {
            ObjectDataPayload::InterlacedPixels(ip) => {
                assert_eq!(ip.top_sub_blocks.len(), 3, "expected 3 sub-blocks");
                assert_eq!(ip.top_sub_blocks[0].data_type, DataType::MapTable2To4);
                assert_eq!(ip.top_sub_blocks[0].data.len(), 2);
                assert_eq!(ip.top_sub_blocks[1].data_type, DataType::EndOfLine);
                assert_eq!(ip.top_sub_blocks[2].data_type, DataType::CodeString4Bit);
                assert_eq!(ip.top_sub_blocks[2].data.len(), 4);
            }
            _ => panic!("expected InterlacedPixels"),
        }
        let out = seg.to_bytes();
        assert_eq!(out, bytes);
    }

    #[test]
    fn round_trip_characters() {
        let bytes = [
            0x0F, 0x13, 0x00, 0x01, 0x00, 0x08, 0x00, 0x0B, 0x04, 0x02, 0x00, 0x41, 0x00, 0x42,
        ];
        let seg = ObjectDataSegment::parse(&bytes).unwrap();
        assert_eq!(seg.object_coding_method, ObjectCodingMethod::Characters);
        match &seg.payload {
            ObjectDataPayload::Characters {
                number_of_codes,
                character_codes,
                ..
            } => {
                assert_eq!(*number_of_codes, 2);
                assert_eq!(character_codes[0], 0x0041);
                assert_eq!(character_codes[1], 0x0042);
            }
            _ => panic!("expected Characters"),
        }
        let out = seg.to_bytes();
        assert_eq!(out, bytes);

        // Biting test
        let mut seg2 = seg.clone();
        if let ObjectDataPayload::Characters {
            number_of_codes, ..
        } = &mut seg2.payload
        {
            *number_of_codes = 1;
        }
        let out2 = seg2.to_bytes();
        assert_ne!(out2, bytes);
    }

    #[test]
    fn round_trip_progressive() {
        let bytes = [
            0x0F, 0x13, 0x00, 0x01, 0x00, 0x0F, 0x00, 0x0C, 0x08, 0x00, 0x64, 0x00, 0x32, 0x00,
            0x06, 0x78, 0xDA, 0x63, 0x60, 0x60, 0x60,
        ];
        let seg = ObjectDataSegment::parse(&bytes).unwrap();
        assert_eq!(
            seg.object_coding_method,
            ObjectCodingMethod::ProgressivePixels
        );
        match &seg.payload {
            ObjectDataPayload::ProgressivePixels(pp) => {
                assert_eq!(pp.bitmap_width, 100);
                assert_eq!(pp.bitmap_height, 50);
                assert_eq!(pp.compressed_data.len(), 6);
            }
            _ => panic!("expected ProgressivePixels"),
        }
        let out = seg.to_bytes();
        assert_eq!(out, bytes);

        // Biting test
        let mut seg2 = seg.clone();
        if let ObjectDataPayload::ProgressivePixels(pp) = &mut seg2.payload {
            pp.bitmap_width = 200;
        }
        let out2 = seg2.to_bytes();
        assert_ne!(out2, bytes);
        let reparse = ObjectDataSegment::parse(&out2).unwrap();
        match &reparse.payload {
            ObjectDataPayload::ProgressivePixels(pp) => {
                assert_eq!(pp.bitmap_width, 200);
            }
            _ => panic!(),
        }
    }
}
