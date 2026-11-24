//! Object Data Segment — ETSI EN 300 743 §7.2.5, Table 17 (segment_type 0x13).
//!
//! Contains object data: either interlaced/bitmap pixel-data sub-blocks,
//! a character string, or a progressive (zlib-compressed) pixel block.

use crate::error::{Error, Result};
use crate::segments::{
    PAGE_ID_LEN, PAGE_ID_OFFSET, SEGMENT_LENGTH_LEN, SEGMENT_LENGTH_OFFSET, SEGMENT_TYPE_OFFSET,
    be_u16,
};
use broadcast_common::len::{need_in, need_out};
use broadcast_common::{Parse, Serialize};

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
/// 7-bit `run_length` field mask in the 8-bit pixel-data run codes
/// (EN 300 743 §7.2.5.2.3, Table 22).
const RUN_LENGTH_7BIT_MASK: u8 = 0x7F;

/// Bit width of the 2-bit depth's pixel code.
const NARROW_CODE_BITS: usize = 2;
/// Bit width of a 3-bit run count.
const RUN_COUNT_NARROW_BITS: usize = 3;
/// Bit width of a 4-bit run count or 4-bit pixel code.
const WIDE_CODE_BITS: usize = 4;
/// Bit width of an 8-bit run count or pixel code.
const BYTE_CODE_BITS: usize = 8;
/// Bit width of a single switch bit.
const SWITCH_BITS: usize = 1;
/// Shift selecting the 8-bit code string's switch bit.
const SWITCH_SHIFT: u32 = 7;
/// Bytes of the 8-bit code string's two-byte run header.
const RUN_HEADER_BYTES: usize = 2;
/// Bytes of the 8-bit code string's three-byte coloured-run header.
const COLOURED_RUN_BYTES: usize = 3;

/// Run-length bias of a two-pixel zero-colour run (Tables 22-25).
const RUN_LENGTH_PIXEL_PAIR: u32 = 2;
/// Run-length biases of the 2-bit-depth codes (Tables 22-23).
const RUN_LENGTH_2BIT_COLOURED: u32 = 3;
const RUN_LENGTH_2BIT_MEDIUM: u32 = 12;
const RUN_LENGTH_2BIT_LONG: u32 = 29;
/// Run-length biases of the 4-bit-depth codes (Tables 24-25).
const RUN_LENGTH_4BIT_SHORT: u32 = 2;
const RUN_LENGTH_4BIT_NARROW: u32 = 4;
const RUN_LENGTH_4BIT_MEDIUM: u32 = 9;
const RUN_LENGTH_4BIT_LONG: u32 = 25;

/// Offset of `object_id` within the segment body.
const OBJECT_ID_OFFSET: usize = 0;
/// Offset of the version/coding-method/flags byte within the segment body.
const OBJECT_FLAGS_OFFSET: usize = 2;
/// Shift placing `object_version_number` at the top of its byte.
const OBJECT_VERSION_SHIFT: u32 = 4;
/// Mask of `object_version_number`.
const OBJECT_VERSION_MASK: u8 = 0x0F;
/// Shift placing `object_coding_method` below the version nibble.
const CODING_METHOD_SHIFT: u32 = 2;
/// Mask of `object_coding_method`.
const CODING_METHOD_MASK: u8 = 0x03;
/// Mask of `non_modifying_colour_flag`.
const NON_MODIFYING_COLOUR_FLAG_MASK: u8 = 0x02;
/// Shift placing `non_modifying_colour_flag`.
const NON_MODIFYING_COLOUR_FLAG_SHIFT: u32 = 1;
/// Mask of the object_data reserved bit.
const OBJECT_RFU_MASK: u8 = 0x01;
/// Offset of the top field length within an interlaced payload.
const TOP_FIELD_LEN_OFFSET: usize = 0;
/// Offset of the bottom field length within an interlaced payload.
const BOTTOM_FIELD_LEN_OFFSET: usize = 2;
/// Bytes of the `number_of_codes` field.
const CHARACTER_CODE_COUNT_LEN: usize = 1;
/// Bytes of one `character_code`.
const CHARACTER_CODE_LEN: usize = 2;

/// Offset of `bitmap_width` within a progressive pixel block.
const BITMAP_WIDTH_OFFSET: usize = 0;
/// Offset of `bitmap_height` within a progressive pixel block.
const BITMAP_HEIGHT_OFFSET: usize = 2;
/// Offset of `compressed_data_length` within a progressive pixel block.
const COMPRESSED_LEN_OFFSET: usize = 4;

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
            Self::Reserved(v) => v,
        }
    }
}

broadcast_common::impl_spec_display!(ObjectCodingMethod, Reserved);

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

broadcast_common::impl_spec_display!(DataType, Reserved);

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

impl<'a> PixelDataSubBlock<'a> {
    /// Decode this sub-block's pixel runs (Tables 22-26), if it is a
    /// 2/4/8-bit pixel-data code string — `None` for a map-table or
    /// end-of-line sub-block, neither of which is an RLE token stream.
    ///
    /// Yields typed [`PixelRun`]s instead of exposing `data` as raw RLE
    /// bytes for the caller to decode itself (#1108/DS-W5). `data` (the
    /// undecoded token stream, including any trailing byte-alignment
    /// stuffing) is kept as-is alongside this, since round-tripping the
    /// segment byte-exact depends on preserving it verbatim.
    #[must_use]
    pub fn runs(&self) -> Option<PixelRunIter<'a>> {
        let depth = match self.data_type {
            DataType::CodeString2Bit => CodeStringDepth::Two,
            DataType::CodeString4Bit => CodeStringDepth::Four,
            DataType::CodeString8Bit => CodeStringDepth::Eight,
            DataType::MapTable2To4
            | DataType::MapTable2To8
            | DataType::MapTable4To8
            | DataType::EndOfLine
            | DataType::Reserved(_) => return None,
        };
        Some(PixelRunIter {
            data: self.data,
            bitpos: 0,
            depth,
            done: false,
        })
    }

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
    /// Trailing `8_stuff_bits` (Table 17/19): normally 0 or 1 byte per the
    /// spec's own `stuffing_length` formula, but kept as a slice rather than
    /// `Option<u8>` because the spec itself notes some legacy encoders don't
    /// follow that (#1108/DS-W6) — 2+ trailing bytes are preserved for a
    /// byte-exact round-trip instead of being silently dropped.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub stuffing: &'a [u8],
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
    /// Character string. `number_of_codes` (Table 20) is derived from
    /// `character_codes.len()` on serialize, rather than stored — a stored
    /// count could diverge from the data it's supposed to describe (#1108/
    /// DS-W3).
    Characters {
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

/// One decoded run from a 2/4/8-bit pixel-data code string (Tables 22-26):
/// `run_length` (>= 1) consecutive pixels painted with CLUT entry `pixel_code`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelRun {
    /// The CLUT entry index for this run.
    pub pixel_code: u8,
    /// How many consecutive pixels carry `pixel_code`.
    pub run_length: u32,
}

/// Bit depth of a pixel-data code string (Tables 22, 24, 26).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodeStringDepth {
    Two,
    Four,
    Eight,
}

/// A lazy decoder over one 2/4/8-bit pixel-data code string's RLE pixel runs
/// (Tables 22-26), returned by [`PixelDataSubBlock::runs`]. Stops at the
/// string's `end_of_string_signal`, or at the data boundary if that comes
/// first (never reads past the sub-block's own `data`).
#[derive(Debug, Clone)]
pub struct PixelRunIter<'a> {
    data: &'a [u8],
    bitpos: usize,
    depth: CodeStringDepth,
    done: bool,
}

impl PixelRunIter<'_> {
    /// Read `n` (<= 8) bits starting at bit offset `bp`, big-endian,
    /// zero-padded past the end of `data` (never panics/reads out of bounds).
    fn bits(&self, bp: usize, n: usize) -> u8 {
        let byte_idx = bp / u8::BITS as usize;
        let bit_idx = bp % u8::BITS as usize;
        if byte_idx >= self.data.len() {
            return 0;
        }
        if bit_idx + n <= u8::BITS as usize {
            (self.data[byte_idx] >> (u8::BITS as usize - bit_idx - n)) & ((1u16 << n) - 1) as u8
        } else {
            let first_bits = u8::BITS as usize - bit_idx;
            let v1 = ((self.data[byte_idx] & ((1u8 << first_bits) - 1)) as u16) << (n - first_bits);
            let v2 = if byte_idx + 1 < self.data.len() {
                (self.data[byte_idx + 1] >> (u8::BITS as usize - (n - first_bits))) as u16
            } else {
                0
            };
            (v1 | v2) as u8
        }
    }

    /// Table 22/23.
    fn next_2bit(&mut self) -> Option<PixelRun> {
        let b2 = self.bits(self.bitpos, NARROW_CODE_BITS);
        self.bitpos += NARROW_CODE_BITS;
        if b2 != 0 {
            return Some(PixelRun {
                pixel_code: b2,
                run_length: 1,
            });
        }
        let s1 = self.bits(self.bitpos, SWITCH_BITS);
        self.bitpos += SWITCH_BITS;
        if s1 == 1 {
            let n = self.bits(self.bitpos, RUN_COUNT_NARROW_BITS);
            self.bitpos += RUN_COUNT_NARROW_BITS;
            let code = self.bits(self.bitpos, NARROW_CODE_BITS);
            self.bitpos += NARROW_CODE_BITS;
            return Some(PixelRun {
                pixel_code: code,
                run_length: u32::from(n) + RUN_LENGTH_2BIT_COLOURED,
            });
        }
        let s2 = self.bits(self.bitpos, SWITCH_BITS);
        self.bitpos += SWITCH_BITS;
        if s2 == 1 {
            return Some(PixelRun {
                pixel_code: 0,
                run_length: 1,
            });
        }
        let s3 = self.bits(self.bitpos, NARROW_CODE_BITS);
        self.bitpos += NARROW_CODE_BITS;
        match s3 {
            0b01 => Some(PixelRun {
                pixel_code: 0,
                run_length: RUN_LENGTH_PIXEL_PAIR,
            }),
            0b10 => {
                let n = self.bits(self.bitpos, WIDE_CODE_BITS);
                self.bitpos += WIDE_CODE_BITS;
                let code = self.bits(self.bitpos, NARROW_CODE_BITS);
                self.bitpos += NARROW_CODE_BITS;
                Some(PixelRun {
                    pixel_code: code,
                    run_length: u32::from(n) + RUN_LENGTH_2BIT_MEDIUM,
                })
            }
            0b11 => {
                let n = self.bits(self.bitpos, BYTE_CODE_BITS);
                self.bitpos += BYTE_CODE_BITS;
                let code = self.bits(self.bitpos, NARROW_CODE_BITS);
                self.bitpos += NARROW_CODE_BITS;
                Some(PixelRun {
                    pixel_code: code,
                    run_length: u32::from(n) + RUN_LENGTH_2BIT_LONG,
                })
            }
            _ => {
                // 0b00: end_of_string.
                self.done = true;
                None
            }
        }
    }

    /// Table 24/25.
    fn next_4bit(&mut self) -> Option<PixelRun> {
        let b4 = self.bits(self.bitpos, WIDE_CODE_BITS);
        self.bitpos += WIDE_CODE_BITS;
        if b4 != 0 {
            return Some(PixelRun {
                pixel_code: b4,
                run_length: 1,
            });
        }
        let s1 = self.bits(self.bitpos, SWITCH_BITS);
        self.bitpos += SWITCH_BITS;
        if s1 == 0 {
            let n3 = self.bits(self.bitpos, RUN_COUNT_NARROW_BITS);
            self.bitpos += RUN_COUNT_NARROW_BITS;
            if n3 == 0 {
                self.done = true;
                return None;
            }
            return Some(PixelRun {
                pixel_code: 0,
                run_length: u32::from(n3) + RUN_LENGTH_4BIT_SHORT,
            });
        }
        let s2 = self.bits(self.bitpos, SWITCH_BITS);
        self.bitpos += SWITCH_BITS;
        if s2 == 0 {
            let n2 = self.bits(self.bitpos, NARROW_CODE_BITS);
            self.bitpos += NARROW_CODE_BITS;
            let code = self.bits(self.bitpos, WIDE_CODE_BITS);
            self.bitpos += WIDE_CODE_BITS;
            return Some(PixelRun {
                pixel_code: code,
                run_length: u32::from(n2) + RUN_LENGTH_4BIT_NARROW,
            });
        }
        let s3 = self.bits(self.bitpos, NARROW_CODE_BITS);
        self.bitpos += NARROW_CODE_BITS;
        match s3 {
            0b00 => Some(PixelRun {
                pixel_code: 0,
                run_length: 1,
            }),
            0b01 => Some(PixelRun {
                pixel_code: 0,
                run_length: RUN_LENGTH_PIXEL_PAIR,
            }),
            0b10 => {
                let n = self.bits(self.bitpos, WIDE_CODE_BITS);
                self.bitpos += WIDE_CODE_BITS;
                let code = self.bits(self.bitpos, WIDE_CODE_BITS);
                self.bitpos += WIDE_CODE_BITS;
                Some(PixelRun {
                    pixel_code: code,
                    run_length: u32::from(n) + RUN_LENGTH_4BIT_MEDIUM,
                })
            }
            _ => {
                // 0b11
                let n = self.bits(self.bitpos, BYTE_CODE_BITS);
                self.bitpos += BYTE_CODE_BITS;
                let code = self.bits(self.bitpos, WIDE_CODE_BITS);
                self.bitpos += WIDE_CODE_BITS;
                Some(PixelRun {
                    pixel_code: code,
                    run_length: u32::from(n) + RUN_LENGTH_4BIT_LONG,
                })
            }
        }
    }

    /// Table 26. Always byte-aligned, so this indexes `data` directly rather
    /// than going through the bit reader.
    fn next_8bit(&mut self) -> Option<PixelRun> {
        let pos = self.bitpos / u8::BITS as usize;
        if pos >= self.data.len() {
            self.done = true;
            return None;
        }
        let b0 = self.data[pos];
        if b0 != 0 {
            self.bitpos += BYTE_CODE_BITS;
            return Some(PixelRun {
                pixel_code: b0,
                run_length: 1,
            });
        }
        if pos + 1 >= self.data.len() {
            self.done = true;
            return None;
        }
        let b1 = self.data[pos + 1];
        if (b1 >> SWITCH_SHIFT) & 1 == 0 {
            let rl = b1 & RUN_LENGTH_7BIT_MASK;
            self.bitpos += RUN_HEADER_BYTES * u8::BITS as usize;
            if rl == 0 {
                self.done = true;
                return None;
            }
            return Some(PixelRun {
                pixel_code: 0,
                run_length: u32::from(rl),
            });
        }
        // switch_1 == 1: run_length_3-127 + 8-bit pixel-code.
        if pos + RUN_HEADER_BYTES >= self.data.len() {
            self.done = true;
            return None;
        }
        let rl = b1 & RUN_LENGTH_7BIT_MASK;
        let code = self.data[pos + RUN_HEADER_BYTES];
        self.bitpos += COLOURED_RUN_BYTES * u8::BITS as usize;
        Some(PixelRun {
            pixel_code: code,
            run_length: u32::from(rl),
        })
    }
}

impl Iterator for PixelRunIter<'_> {
    type Item = PixelRun;

    fn next(&mut self) -> Option<PixelRun> {
        if self.done {
            return None;
        }
        match self.depth {
            CodeStringDepth::Two => self.next_2bit(),
            CodeStringDepth::Four => self.next_4bit(),
            CodeStringDepth::Eight => self.next_8bit(),
        }
    }
}

/// Scan a 2-bit/pixel_code_string to find its end marker and total byte length.
/// Returns the number of bytes consumed INCLUDING any trailing 2-bit stuffing.
/// Table 22, 23.
fn scan_2bit_code_string(data: &[u8]) -> usize {
    let mut bitpos: usize = 0;
    let bits = |bp: usize, n: usize| -> u8 {
        let byte_idx = bp / u8::BITS as usize;
        let bit_idx = bp % u8::BITS as usize;
        if byte_idx >= data.len() {
            return 0;
        }
        if bit_idx + n <= u8::BITS as usize {
            (data[byte_idx] >> (u8::BITS as usize - bit_idx - n)) & ((1u16 << n) - 1) as u8
        } else {
            let first_bits = u8::BITS as usize - bit_idx;
            let v1 = ((data[byte_idx] & ((1u8 << first_bits) - 1)) as u16) << (n - first_bits);
            let v2 = if byte_idx + 1 < data.len() {
                (data[byte_idx + 1] >> (u8::BITS as usize - (n - first_bits))) as u16
            } else {
                0
            };
            (v1 | v2) as u8
        }
    };
    loop {
        let b2 = bits(bitpos, NARROW_CODE_BITS);
        bitpos += NARROW_CODE_BITS;
        if b2 != 0 {
            continue;
        }
        // 2-bit_zero, check switch_1
        let s1 = bits(bitpos, SWITCH_BITS);
        bitpos += SWITCH_BITS;
        if s1 == 1 {
            // run_length_3-10 (3 bits) + 2-bitpixel-code (2 bits)
            bitpos += RUN_COUNT_NARROW_BITS + NARROW_CODE_BITS;
            continue;
        }
        let s2 = bits(bitpos, SWITCH_BITS);
        bitpos += SWITCH_BITS;
        if s2 == 1 {
            // 1 pixel in colour 0
            continue;
        }
        let s3 = bits(bitpos, NARROW_CODE_BITS);
        bitpos += NARROW_CODE_BITS;
        match s3 {
            0b00 => break,    // end of string
            0b01 => continue, // 2 pixels in colour 0
            0b10 => {
                bitpos += WIDE_CODE_BITS + NARROW_CODE_BITS; // run_length_12-27 + pixel-code
            }
            0b11 => {
                bitpos += BYTE_CODE_BITS + NARROW_CODE_BITS; // run_length_29-284 + pixel-code
            }
            _ => {}
        }
    }
    // Byte-align: the sub-block includes 2_stuff_bits if not byte-aligned
    let byte_len = bitpos.div_ceil(u8::BITS as usize);
    byte_len.min(data.len())
}

/// Scan a 4-bit/pixel_code_string to find its end marker and total byte length.
/// Returns number of bytes consumed INCLUDING any trailing 4-bit stuffing.
/// Table 24, 25.
fn scan_4bit_code_string(data: &[u8]) -> usize {
    let mut bitpos: usize = 0;
    let bits = |bp: usize, n: usize| -> u8 {
        let byte_idx = bp / u8::BITS as usize;
        let bit_idx = bp % u8::BITS as usize;
        if byte_idx >= data.len() {
            return 0;
        }
        if bit_idx + n <= u8::BITS as usize {
            (data[byte_idx] >> (u8::BITS as usize - bit_idx - n)) & ((1u16 << n) - 1) as u8
        } else {
            let first_bits = u8::BITS as usize - bit_idx;
            let v1 = ((data[byte_idx] & ((1u8 << first_bits) - 1)) as u16) << (n - first_bits);
            let v2 = if byte_idx + 1 < data.len() {
                (data[byte_idx + 1] >> (u8::BITS as usize - (n - first_bits))) as u16
            } else {
                0
            };
            (v1 | v2) as u8
        }
    };
    loop {
        let b4 = bits(bitpos, WIDE_CODE_BITS);
        bitpos += WIDE_CODE_BITS;
        if b4 != 0 {
            continue;
        }
        // 4-bit_zero
        let s1 = bits(bitpos, SWITCH_BITS);
        bitpos += SWITCH_BITS;
        if s1 == 0 {
            let n3 = bits(bitpos, RUN_COUNT_NARROW_BITS);
            bitpos += RUN_COUNT_NARROW_BITS;
            if n3 == 0 {
                // end_of_string_signal
                break;
            }
            // run_length_3-9 in colour 0
            continue;
        }
        let s2 = bits(bitpos, SWITCH_BITS);
        bitpos += SWITCH_BITS;
        if s2 == 0 {
            // run_length_4-7 (2 bits) + 4-bit_pixel-code
            bitpos += NARROW_CODE_BITS + WIDE_CODE_BITS;
            continue;
        }
        let s3 = bits(bitpos, NARROW_CODE_BITS);
        bitpos += NARROW_CODE_BITS;
        match s3 {
            0b00 | 0b01 => continue, // 1 or 2 pixels in colour 0
            0b10 => {
                bitpos += WIDE_CODE_BITS + WIDE_CODE_BITS; // run_length_9-24 + pixel-code
            }
            0b11 => {
                bitpos += BYTE_CODE_BITS + WIDE_CODE_BITS; // run_length_25-280 + pixel-code
            }
            _ => {}
        }
    }
    // Byte-align: if not byte-aligned, 4_stuff_bits are implied
    let byte_len = bitpos.div_ceil(u8::BITS as usize);
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
        if data[pos] != 0 {
            pos += 1;
            continue;
        }
        if pos + 1 >= data.len() {
            pos += 1;
            break;
        }
        let b = data[pos + 1];
        let s1 = (b >> SWITCH_SHIFT) & 1;
        if s1 == 0 {
            let rl = b & RUN_LENGTH_7BIT_MASK;
            if rl == 0 {
                // end_of_string_signal
                pos += RUN_HEADER_BYTES;
                break;
            }
            // run_length_1-127 in colour 0
            pos += RUN_HEADER_BYTES;
        } else {
            // run_length_3-127 + 8-bitpixel-code
            pos += COLOURED_RUN_BYTES;
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
            0x10 => scan_2bit_code_string(&field[pos..]),
            0x11 => scan_4bit_code_string(&field[pos..]),
            0x12 => scan_8bit_code_string(&field[pos..]),
            0x20 => MAP_TABLE_2TO4_BYTES,
            0x21 => MAP_TABLE_2TO8_BYTES,
            0x22 => MAP_TABLE_4TO8_BYTES,
            0xF0 => 0,
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
    if pos != end {
        return Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: end,
                have: pos,
                what,
            },
        ));
    }
    Ok(blocks)
}

impl<'a> Parse<'a> for ObjectDataSegment<'a> {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(bytes, HEADER_LEN + FIXED_LEN, "object_data_segment")?;
        if bytes[0] != crate::pes_data_field::SYNC_BYTE {
            return Err(Error::InvalidSync(broadcast_common::error::InvalidSync {
                what: "sync_byte",
                expected: u32::from(crate::pes_data_field::SYNC_BYTE),
                found: u32::from(bytes[0]),
            }));
        }
        if bytes[SEGMENT_TYPE_OFFSET] != SEGMENT_TYPE {
            return Err(Error::UnknownSegmentType(bytes[SEGMENT_TYPE_OFFSET]));
        }
        let page_id = be_u16(bytes, PAGE_ID_OFFSET);
        let segment_length = be_u16(bytes, SEGMENT_LENGTH_OFFSET) as usize;
        let total = HEADER_LEN + segment_length;
        need_in(bytes, total, "object_data_segment data")?;
        let body = &bytes[HEADER_LEN..HEADER_LEN + segment_length];
        need_in(body, FIXED_LEN, "object_data_segment body")?;
        let object_id = be_u16(body, OBJECT_ID_OFFSET);
        let object_version_number = body[OBJECT_FLAGS_OFFSET] >> OBJECT_VERSION_SHIFT;
        let coding_method_bits =
            (body[OBJECT_FLAGS_OFFSET] >> CODING_METHOD_SHIFT) & CODING_METHOD_MASK;
        let non_modifying_colour_flag =
            (body[OBJECT_FLAGS_OFFSET] & NON_MODIFYING_COLOUR_FLAG_MASK) != 0;
        let reserved = body[OBJECT_FLAGS_OFFSET] & OBJECT_RFU_MASK;
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
                need_in(
                    payload_data,
                    INTERLACE_LEN_LEN,
                    "object_data interlace lengths",
                )?;
                let top_len = be_u16(payload_data, TOP_FIELD_LEN_OFFSET) as usize;
                let bottom_len = be_u16(payload_data, BOTTOM_FIELD_LEN_OFFSET) as usize;

                let top_start = INTERLACE_LEN_LEN;
                let top_end = top_start + top_len;
                let bottom_start = top_end;
                let bottom_end = bottom_start + bottom_len;

                need_in(payload_data, bottom_end, "object_data pixel sub-blocks")?;

                // stuffing_length (Table 18) = segment_length - 7 - top_len -
                // bottom_len (7 = FIXED_LEN + INTERLACE_LEN_LEN); this is
                // exactly `payload_data.len() - bottom_end`, since
                // `payload_data` was already sliced to `segment_length -
                // FIXED_LEN`. The spec says "shall be either zero or one",
                // but notes some legacy encoders don't follow that — so
                // whatever trailing bytes remain are captured (not just the
                // first one) to keep the round-trip byte-exact (#1108/DS-W6).
                let stuffing = &payload_data[bottom_end..];
                if let Some(&b) = stuffing.first()
                    && b != 0
                {
                    return Err(Error::BadStuffingByte(b));
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
                    stuffing,
                })
            }
            0x01 => {
                need_in(payload_data, CHARACTER_CODE_COUNT_LEN, "character count")?;
                let number_of_codes = payload_data[0] as usize;
                let codes_len = CHARACTER_CODE_COUNT_LEN + number_of_codes * CHARACTER_CODE_LEN;
                need_in(payload_data, codes_len, "character codes")?;
                let mut codes = alloc::vec::Vec::with_capacity(number_of_codes);
                for i in 0..number_of_codes {
                    let ci = CHARACTER_CODE_COUNT_LEN + i * CHARACTER_CODE_LEN;
                    codes.push(be_u16(payload_data, ci));
                }
                ObjectDataPayload::Characters {
                    character_codes: codes,
                }
            }
            0x02 => {
                need_in(
                    payload_data,
                    PROGRESSIVE_HEADER_LEN,
                    "progressive pixel block",
                )?;
                let bitmap_width = be_u16(payload_data, BITMAP_WIDTH_OFFSET);
                let bitmap_height = be_u16(payload_data, BITMAP_HEIGHT_OFFSET);
                let compressed_len = be_u16(payload_data, COMPRESSED_LEN_OFFSET) as usize;
                need_in(
                    payload_data,
                    PROGRESSIVE_HEADER_LEN + compressed_len,
                    "compressed bitmap data",
                )?;
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
                    INTERLACE_LEN_LEN + top_len + bottom_len + ip.stuffing.len()
                }
                ObjectDataPayload::Characters {
                    character_codes, ..
                } => CHARACTER_CODE_COUNT_LEN + character_codes.len() * CHARACTER_CODE_LEN,
                ObjectDataPayload::ProgressivePixels(pp) => {
                    PROGRESSIVE_HEADER_LEN + pp.compressed_data.len()
                }
                ObjectDataPayload::Reserved { data, .. } => data.len(),
            };
        HEADER_LEN + body_len
    }

    fn serialize_into(&self, buf: &mut [u8]) -> core::result::Result<usize, Self::Error> {
        let len = self.serialized_len();
        need_out(buf, len)?;
        buf[0] = crate::pes_data_field::SYNC_BYTE;
        buf[SEGMENT_TYPE_OFFSET] = SEGMENT_TYPE;
        buf[PAGE_ID_OFFSET..PAGE_ID_OFFSET + PAGE_ID_LEN]
            .copy_from_slice(&self.page_id.to_be_bytes());
        let seg_len = crate::segments::check_segment_length(len - HEADER_LEN)?;
        buf[SEGMENT_LENGTH_OFFSET..SEGMENT_LENGTH_OFFSET + SEGMENT_LENGTH_LEN]
            .copy_from_slice(&seg_len.to_be_bytes());

        buf[HEADER_LEN + OBJECT_ID_OFFSET..HEADER_LEN + OBJECT_FLAGS_OFFSET]
            .copy_from_slice(&self.object_id.to_be_bytes());
        buf[HEADER_LEN + OBJECT_FLAGS_OFFSET] = (broadcast_common::len::fit_mask(
            self.object_version_number,
            OBJECT_VERSION_MASK,
            "object_version_number",
        )? << OBJECT_VERSION_SHIFT)
            | (broadcast_common::len::fit_mask(
                self.object_coding_method.to_bits(),
                CODING_METHOD_MASK,
                "object_coding_method",
            )? << CODING_METHOD_SHIFT)
            | (u8::from(self.non_modifying_colour_flag) << NON_MODIFYING_COLOUR_FLAG_SHIFT);

        let mut off = HEADER_LEN + FIXED_LEN;
        match &self.payload {
            ObjectDataPayload::InterlacedPixels(ip) => {
                let top_len: usize = ip.top_sub_blocks.iter().map(|s| s.serialized_len()).sum();
                let bottom_len: usize = ip
                    .bottom_sub_blocks
                    .iter()
                    .map(|s| s.serialized_len())
                    .sum();
                let top_len = broadcast_common::len::fit_u16(top_len, "top_field_data_length")?;
                let bottom_len =
                    broadcast_common::len::fit_u16(bottom_len, "bottom_field_data_length")?;
                buf[off + TOP_FIELD_LEN_OFFSET..off + BOTTOM_FIELD_LEN_OFFSET]
                    .copy_from_slice(&top_len.to_be_bytes());
                buf[off + BOTTOM_FIELD_LEN_OFFSET..off + INTERLACE_LEN_LEN]
                    .copy_from_slice(&bottom_len.to_be_bytes());
                off += INTERLACE_LEN_LEN;
                for sub in &ip.top_sub_blocks {
                    sub.serialize_into(&mut buf[off..]);
                    off += sub.serialized_len();
                }
                for sub in &ip.bottom_sub_blocks {
                    sub.serialize_into(&mut buf[off..]);
                    off += sub.serialized_len();
                }
                buf[off..off + ip.stuffing.len()].copy_from_slice(ip.stuffing);
                off += ip.stuffing.len();
            }
            ObjectDataPayload::Characters { character_codes } => {
                buf[off] = broadcast_common::len::fit_u8(character_codes.len(), "number_of_codes")?;
                off += CHARACTER_CODE_COUNT_LEN;
                for code in character_codes {
                    buf[off..off + CHARACTER_CODE_LEN].copy_from_slice(&code.to_be_bytes());
                    off += CHARACTER_CODE_LEN;
                }
            }
            ObjectDataPayload::ProgressivePixels(pp) => {
                buf[off + BITMAP_WIDTH_OFFSET..off + BITMAP_HEIGHT_OFFSET]
                    .copy_from_slice(&pp.bitmap_width.to_be_bytes());
                buf[off + BITMAP_HEIGHT_OFFSET..off + COMPRESSED_LEN_OFFSET]
                    .copy_from_slice(&pp.bitmap_height.to_be_bytes());
                let clen = broadcast_common::len::fit_u16(
                    pp.compressed_data.len(),
                    "compressed_data_length",
                )?;
                buf[off + COMPRESSED_LEN_OFFSET..off + PROGRESSIVE_HEADER_LEN]
                    .copy_from_slice(&clen.to_be_bytes());
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
    use broadcast_common::{Parse, Serialize};

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
        let out = seg.try_to_bytes().unwrap();
        assert_eq!(out, bytes);

        // Biting test
        let mut seg2 = seg.clone();
        seg2.object_id = 20;
        let out2 = seg2.try_to_bytes().unwrap();
        assert_ne!(out2, bytes);
        let reparse = ObjectDataSegment::parse(&out2).unwrap();
        assert_eq!(reparse.object_id, 20);
    }

    /// DS-W6 (#1108): stuffing_length (Table 18) is "either zero or one" per
    /// spec, but the spec itself notes legacy encoders may not follow that.
    /// Pre-fix, only exactly 1 trailing byte was captured (as
    /// `stuffing_byte: Option<u8>`); 2+ were silently dropped, so a segment
    /// with 2 trailing stuffing bytes didn't round-trip byte-exact.
    #[test]
    fn preserves_multi_byte_stuffing() {
        let bytes = [
            0x0F, 0x13, 0x00, 0x01, 0x00, 0x09, // header, segment_length=9
            0x00, 0x0A, 0x00, // object_id=10, version/method/flags=0 (Pixels)
            0x00, 0x00, 0x00, 0x00, // top_len=0, bottom_len=0
            0x00, 0x00, // 2 trailing stuffing bytes
        ];
        let seg = ObjectDataSegment::parse(&bytes).unwrap();
        match &seg.payload {
            ObjectDataPayload::InterlacedPixels(ip) => {
                assert_eq!(ip.stuffing, &[0x00, 0x00]);
            }
            _ => panic!("expected InterlacedPixels"),
        }
        let out = seg.try_to_bytes().unwrap();
        assert_eq!(out, bytes, "2 trailing stuffing bytes must round-trip");
    }

    #[test]
    fn nonzero_first_stuffing_byte_is_rejected_with_its_value() {
        let bytes = [
            0x0F, 0x13, 0x00, 0x01, 0x00, 0x08, // header, segment_length=8
            0x00, 0x0A, 0x00, // object_id=10, version/method/flags=0 (Pixels)
            0x00, 0x00, 0x00, 0x00, // top_len=0, bottom_len=0
            0x5A, // stuffing_byte must be 0x00
        ];
        assert_eq!(
            ObjectDataSegment::parse(&bytes).unwrap_err(),
            Error::BadStuffingByte(0x5A)
        );
    }

    /// DS-W2 (#1108): sync_byte wasn't checked by any typed segment parser.
    #[test]
    fn rejects_bad_sync_byte() {
        let bytes = [
            0x00, 0x13, 0x00, 0x01, 0x00, 0x0A, 0x00, 0x0A, 0x00, 0x00, 0x03, 0x00, 0x00, 0xF0,
            0x10, 0x0A,
        ];
        assert!(matches!(
            ObjectDataSegment::parse(&bytes),
            Err(Error::InvalidSync(broadcast_common::error::InvalidSync {
                found: 0x00,
                ..
            }))
        ));
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
                // DS-W5 (#1108): the pixel data is also available as typed
                // runs, not just the raw RLE bytes. Cross-check that the
                // decoder consumes exactly as many bits as the (separately
                // tested, framing-critical) scanner does, on the same real
                // fixture bytes: the two must never disagree about where the
                // code string ends.
                let sub = &ip.top_sub_blocks[2];
                let scanned_len = scan_4bit_code_string(sub.data);
                let mut it = sub.runs().expect("code string sub-block has runs()");
                let mut iterations = 0;
                while it.next().is_some() {
                    iterations += 1;
                    assert!(iterations < 1_000, "runaway iterator");
                }
                assert_eq!(it.bitpos.div_ceil(8), scanned_len);
            }
            _ => panic!("expected InterlacedPixels"),
        }
        let out = seg.try_to_bytes().unwrap();
        assert_eq!(out, bytes);
    }

    /// DS-W5 (#1108): map-table and end-of-line sub-blocks aren't RLE token
    /// streams, so `runs()` must say so rather than misdecoding fixed-size
    /// table bytes as a code string.
    #[test]
    fn runs_is_none_for_non_code_string_data_types() {
        let map = PixelDataSubBlock {
            data_type: DataType::MapTable2To4,
            data: &[0x00, 0x00],
        };
        assert!(map.runs().is_none());
        let eol = PixelDataSubBlock {
            data_type: DataType::EndOfLine,
            data: &[],
        };
        assert!(eol.runs().is_none());
    }

    /// DS-W5 (#1108): exact decode of a hand-computed 2-bit/pixel_code_string
    /// (Table 22/23) covering the non-zero direct code and the
    /// run_length_3-10 + switch_3 branches, per §7.2.5.2.1.
    #[test]
    fn pixel_run_iter_2bit_exact() {
        // "10" (direct code 2, run 1) then "00 0 0 00" (2-bit_zero, s1=0,
        // s2=0, s3=00 -> end_of_string): 0b1000_0000 = 0x80.
        let direct = PixelDataSubBlock {
            data_type: DataType::CodeString2Bit,
            data: &[0x80],
        };
        let mut it = direct.runs().unwrap();
        assert_eq!(
            it.next(),
            Some(PixelRun {
                pixel_code: 2,
                run_length: 1
            })
        );
        assert_eq!(it.next(), None);

        // "00" (2-bit_zero) "1" (s1=1) "101" (run_length field = 5 -> 5+3=8)
        // "11" (pixel_code=3): 0b0011_0111 = 0x37.
        let run = PixelDataSubBlock {
            data_type: DataType::CodeString2Bit,
            data: &[0x37],
        };
        let mut it = run.runs().unwrap();
        assert_eq!(
            it.next(),
            Some(PixelRun {
                pixel_code: 3,
                run_length: 8
            })
        );
    }

    /// DS-W5 (#1108): exact decode of a hand-computed 8-bit/pixel_code_string
    /// (Table 26): a direct code, a colour-0 run, another direct code, then
    /// the end_of_string_signal.
    #[test]
    fn pixel_run_iter_8bit_exact() {
        let data = [0x05, 0x00, 0x03, 0x2A, 0x00, 0x00];
        let sub = PixelDataSubBlock {
            data_type: DataType::CodeString8Bit,
            data: &data,
        };
        let mut it = sub.runs().unwrap();
        assert_eq!(
            it.next(),
            Some(PixelRun {
                pixel_code: 5,
                run_length: 1
            })
        );
        assert_eq!(
            it.next(),
            Some(PixelRun {
                pixel_code: 0,
                run_length: 3
            })
        );
        assert_eq!(
            it.next(),
            Some(PixelRun {
                pixel_code: 0x2A,
                run_length: 1
            })
        );
        assert_eq!(it.next(), None);
        // scan_8bit_code_string must agree on the consumed length.
        assert_eq!(scan_8bit_code_string(&data), it.bitpos.div_ceil(8));
    }

    #[test]
    fn round_trip_characters() {
        let bytes = [
            0x0F, 0x13, 0x00, 0x01, 0x00, 0x08, 0x00, 0x0B, 0x04, 0x02, 0x00, 0x41, 0x00, 0x42,
        ];
        let seg = ObjectDataSegment::parse(&bytes).unwrap();
        assert_eq!(seg.object_coding_method, ObjectCodingMethod::Characters);
        match &seg.payload {
            ObjectDataPayload::Characters { character_codes } => {
                assert_eq!(character_codes.len(), 2);
                assert_eq!(character_codes[0], 0x0041);
                assert_eq!(character_codes[1], 0x0042);
            }
            _ => panic!("expected Characters"),
        }
        let out = seg.try_to_bytes().unwrap();
        assert_eq!(out, bytes);

        // Biting test: `number_of_codes` (Table 20) is derived from
        // `character_codes.len()`, not stored (#1108/DS-W3), so appending a
        // code must grow the on-wire count byte, and the result must
        // re-parse to the same, now-3-code, list.
        let mut seg2 = seg.clone();
        if let ObjectDataPayload::Characters { character_codes } = &mut seg2.payload {
            character_codes.push(0x0043);
        }
        let out2 = seg2.try_to_bytes().unwrap();
        assert_ne!(out2, bytes);
        assert_eq!(
            out2[HEADER_LEN + FIXED_LEN],
            3,
            "wire number_of_codes must track the new length"
        );
        let reparse = ObjectDataSegment::parse(&out2).unwrap();
        match &reparse.payload {
            ObjectDataPayload::Characters { character_codes } => {
                assert_eq!(character_codes.as_slice(), &[0x0041, 0x0042, 0x0043]);
            }
            _ => panic!("expected Characters"),
        }
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
        let out = seg.try_to_bytes().unwrap();
        assert_eq!(out, bytes);

        // Biting test
        let mut seg2 = seg.clone();
        if let ObjectDataPayload::ProgressivePixels(pp) = &mut seg2.payload {
            pp.bitmap_width = 200;
        }
        let out2 = seg2.try_to_bytes().unwrap();
        assert_ne!(out2, bytes);
        let reparse = ObjectDataSegment::parse(&out2).unwrap();
        match &reparse.payload {
            ObjectDataPayload::ProgressivePixels(pp) => {
                assert_eq!(pp.bitmap_width, 200);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn oversized_segment_is_rejected_not_wrapped() {
        // Before the fix, a 64 KiB+ body silently wrapped segment_length and
        // returned Ok.
        let big = alloc::vec![0u8; 65_536];
        let seg = ObjectDataSegment {
            page_id: 1,
            object_id: 1,
            object_version_number: 0,
            object_coding_method: ObjectCodingMethod::ProgressivePixels,
            non_modifying_colour_flag: false,
            payload: ObjectDataPayload::ProgressivePixels(ProgressivePixelBlock {
                bitmap_width: 1,
                bitmap_height: 1,
                compressed_data: &big,
            }),
        };
        let mut buf = alloc::vec![0u8; 65_600];
        assert_eq!(seg.serialize_into(&mut buf), Err(Error::SegmentTooLarge));
    }
}
