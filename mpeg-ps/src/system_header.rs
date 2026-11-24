//! System Header — ISO/IEC 13818-1 §2.5.3.5, Table 2-40.
//!
//! The (optional) system header follows immediately after the pack header
//! stuffing bytes, in the first pack of the stream. It constrains the P-STD
//! model: `rate_bound`, `audio_bound`/`video_bound`, and per-stream P-STD
//! buffer-size bounds.

use alloc::vec::Vec;
use broadcast_common::len::{need_in, need_out};

use crate::PREFIX_LEN;
use crate::error::{Error, Result};
use crate::wire::{read_u16_at, read_u32_at};
use broadcast_common::{Parse, Serialize};

/// `system_header_start_code` — `0x000001BB`.
pub const SYSTEM_HEADER_START_CODE: u32 = 0x0000_01BB;

/// `stream_id` value that triggers the extension (extended_stream_id) form.
const EXT_STREAM_ID: u8 = 0xB7;
/// The fixed `'1011 0110'` byte that follows `stream_id_extension` (Table 2-40).
const STREAM_ID_EXTENSION_PREFIX: u8 = 0xB6;

/// Byte offset of `system_header_start_code`.
const START_CODE_OFFSET: usize = 0;
/// Byte offset of the 16-bit `header_length`.
const HEADER_LENGTH_OFFSET: usize = START_CODE_OFFSET + core::mem::size_of::<u32>();

/// Byte offsets within the system-header body (ISO/IEC 13818-1 §2.5.3.5,
/// Table 2-40).
const RATE_BOUND_OFFSET: usize = 0;
const RATE_BOUND_MID_OFFSET: usize = RATE_BOUND_OFFSET + 1;
const RATE_BOUND_LO_OFFSET: usize = RATE_BOUND_MID_OFFSET + 1;
const AUDIO_BOUND_OFFSET: usize = RATE_BOUND_LO_OFFSET + 1;
const STREAM_FLAGS_OFFSET: usize = AUDIO_BOUND_OFFSET + 1;
const PACKET_RATE_OFFSET: usize = STREAM_FLAGS_OFFSET + 1;
/// Byte length of the fixed system-header body, before the stream loop.
const FIXED_BODY_LEN: usize = PACKET_RATE_OFFSET + 1;

/// Byte offsets within an extended (6-byte) stream-bound entry.
const EXT_STREAM_ID_EXTENSION_OFFSET: usize = 2;
const EXT_STREAM_ID_PREFIX_OFFSET: usize = 3;
const EXT_BUFFER_FLAGS_OFFSET: usize = 4;
const EXT_BUFFER_SIZE_LO_OFFSET: usize = 5;
/// Byte offsets within a plain (3-byte) stream-bound entry.
const STREAM_BUFFER_FLAGS_OFFSET: usize = 1;
const STREAM_BUFFER_SIZE_LO_OFFSET: usize = 2;
/// Wire lengths of the two stream-bound entry forms.
const EXT_STREAM_ENTRY_LEN: usize = 6;
const STREAM_ENTRY_LEN: usize = 3;

/// Masks of the flags and marker bits that share a header byte.
const RATE_BOUND_HI_MARKER_MASK: u8 = 0x80;
const RATE_BOUND_LO_MARKER_MASK: u8 = 0x01;
const VIDEO_BOUND_MARKER_MASK: u8 = 0x20;
const FIXED_FLAG_MASK: u8 = 0x02;
const CSPS_FLAG_MASK: u8 = 0x01;
const SYSTEM_AUDIO_LOCK_FLAG_MASK: u8 = 0x80;
const SYSTEM_VIDEO_LOCK_FLAG_MASK: u8 = 0x40;
const PACKET_RATE_RESTRICTION_FLAG_MASK: u8 = 0x80;
const EXT_BUFFER_SCALE_FLAG_MASK: u8 = 0x20;
const STREAM_BUFFER_SCALE_FLAG_MASK: u8 = 0x20;
/// Mask of the MSB that starts a stream-bound entry and that a trailing
/// reserved byte must not carry (Table 2-40).
const STREAM_ENTRY_MSB_MASK: u8 = 0x80;
/// Mask of the low 7 bits of a byte, shared by the `reserved_bits` field, the
/// `rate_bound` high group and `stream_id_extension` (Table 2-40).
const RESERVED_BITS_MASK: u8 = 0x7F;
/// Mask of the low 6 bits of a byte.
const LOW_6_BITS_MASK: u8 = 0x3F;
/// Mask of the low 5 bits of a byte.
const LOW_5_BITS_MASK: u8 = 0x1F;
/// Mask of the leading `'11'` prefix of an extended stream-bound entry.
const STREAM_ENTRY_PREFIX_MASK: u8 = 0xC0;

/// Bit widths of the fixed header fields (Table 2-40).
const RATE_BOUND_BITS: u32 = 22;
const AUDIO_BOUND_BITS: u32 = 6;
const VIDEO_BOUND_BITS: u32 = 5;
const STREAM_ID_EXTENSION_BITS: u32 = 7;
const BUFFER_SIZE_BOUND_BITS: u32 = 13;

/// Bit shifts of the 22-bit `rate_bound` and the flags packed beside it.
const RATE_BOUND_HI_SHIFT: u32 = 15;
const RATE_BOUND_MID_SHIFT: u32 = 7;
const RATE_BOUND_LO_SHIFT: u32 = 1;
const AUDIO_BOUND_SHIFT: u32 = 2;
const SYSTEM_AUDIO_LOCK_SHIFT: u32 = 7;
const SYSTEM_VIDEO_LOCK_SHIFT: u32 = 6;
const PACKET_RATE_RESTRICTION_SHIFT: u32 = 7;
const BUFFER_BOUND_SCALE_SHIFT: u32 = 5;
const BUFFER_SIZE_BOUND_HI_SHIFT: u32 = 8;

/// Mask of the 22-bit `rate_bound` field.
const RATE_BOUND_MASK: u32 = (1 << RATE_BOUND_BITS) - 1;

/// A per-stream P-STD buffer bound entry.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct StdBufferBound {
    /// The stream's `stream_id` (ISO/IEC 13818-1 Table 2-22).
    pub stream_id: u8,
    /// If `stream_id == 0xB7`, this is the extended `stream_id_extension` field.
    /// Otherwise `None`.
    pub stream_id_extension: Option<u8>,
    /// `P-STD_buffer_bound_scale`: `false` = 128 bytes, `true` = 1024 bytes.
    pub buffer_bound_scale: bool,
    /// `P-STD_buffer_size_bound` in units of `buffer_bound_scale`.
    pub buffer_size_bound: u16,
}

/// A parsed system header.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SystemHeader {
    /// Upper bound on `program_mux_rate` across all packs (22-bit, 50 B/s units).
    pub rate_bound: u32,
    /// Upper bound on simultaneously-active audio streams (6-bit).
    pub audio_bound: u8,
    /// Fixed / variable bitrate indicator.
    pub fixed_flag: bool,
    /// Constrained system parameters flag.
    pub csps_flag: bool,
    /// System audio lock flag.
    pub system_audio_lock_flag: bool,
    /// System video lock flag.
    pub system_video_lock_flag: bool,
    /// Upper bound on simultaneously-active video streams (5-bit).
    pub video_bound: u8,
    /// Packet rate restriction flag.
    pub packet_rate_restriction_flag: bool,
    /// The 7 `reserved_bits` after `packet_rate_restriction_flag`
    /// (Table 2-40), preserved so a header whose encoder wrote something
    /// other than the conventional all-ones (`0x7F`) re-serializes
    /// byte-identically (audit r14-MPS-W8).
    pub reserved_bits: u8,
    /// Per-stream P-STD buffer bounds (the `while (nextbits() == '1')` loop).
    pub std_buffer_bounds: Vec<StdBufferBound>,
    /// Bytes of `header_length` that follow the stream-bound loop. Table
    /// 2-40 defines none, but a real encoder can pad the header; they are
    /// preserved verbatim so the header re-serializes byte-identically
    /// (audit r14-MPS-W8, #1119). Empty for a conformant header.
    pub trailing_reserved: Vec<u8>,
}

/// Bytes the stream loop occupies. Accumulated in `usize` (#1129): the
/// previous `u16` accumulator overflowed for more than 10 921
/// caller-constructed bounds, wrapping `header_length` and `serialized_len()`
/// together so they stayed mutually consistent but both silently wrong,
/// which then indexed past the (too-small) allocated buffer in
/// `serialize_into`. `serialize_into` separately validates that this fits
/// the 16-bit `header_length` wire field before writing it.
fn stream_loop_len(system_header: &SystemHeader) -> usize {
    let mut len: usize = 0;
    for b in &system_header.std_buffer_bounds {
        if b.stream_id_extension.is_some() {
            len += EXT_STREAM_ENTRY_LEN;
        } else {
            len += STREAM_ENTRY_LEN;
        }
    }
    len + system_header.trailing_reserved.len()
}

impl<'a> Parse<'a> for SystemHeader {
    type Error = Error;

    fn parse(b: &'a [u8]) -> Result<Self> {
        need_in(b, PREFIX_LEN, "system_header prefix")?;

        if read_u32_at(b, START_CODE_OFFSET) != SYSTEM_HEADER_START_CODE {
            return Err(Error::BadSystemHeaderStartCode(read_u32_at(
                b,
                START_CODE_OFFSET,
            )));
        }

        let header_length = usize::from(read_u16_at(b, HEADER_LENGTH_OFFSET));
        let body_end = PREFIX_LEN + header_length;
        if b.len() < body_end {
            return Err(Error::HeaderLengthOverflow {
                header_length,
                available: b.len().saturating_sub(PREFIX_LEN),
            });
        }
        let body = &b[PREFIX_LEN..body_end];

        // Wire layout (Table 2-40, after header_length):
        // byte 0: marker(1)  | rate_bound[21:15](7)
        // byte 1: rate_bound[14:7](8)
        // byte 2: rate_bound[6:0](7) | marker(1)
        // byte 3: audio_bound[5:0](6) | fixed_flag(1) | CSPS_flag(1)
        // byte 4: system_audio_lock_flag(1) | system_video_lock_flag(1) | marker(1) | video_bound[4:0](5)
        // byte 5: packet_rate_restriction_flag(1) | reserved(7)

        if body.len() < FIXED_BODY_LEN {
            return Err(Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: FIXED_BODY_LEN + PREFIX_LEN,
                    have: b.len(),
                    what: "system_header fixed body",
                },
            ));
        }

        // Marker bit checks
        if body[RATE_BOUND_OFFSET] & RATE_BOUND_HI_MARKER_MASK == 0 {
            return Err(Error::BadMarker("system_header rate_bound marker 1"));
        }
        if body[RATE_BOUND_LO_OFFSET] & RATE_BOUND_LO_MARKER_MASK == 0 {
            return Err(Error::BadMarker("system_header rate_bound marker 2"));
        }
        if body[STREAM_FLAGS_OFFSET] & VIDEO_BOUND_MARKER_MASK == 0 {
            return Err(Error::BadMarker("system_header video_bound marker"));
        }

        // rate_bound: 22 bits
        let rate_bound = ((u32::from(body[RATE_BOUND_OFFSET] & RESERVED_BITS_MASK)
            << RATE_BOUND_HI_SHIFT)
            | (u32::from(body[RATE_BOUND_MID_OFFSET]) << RATE_BOUND_MID_SHIFT)
            | u32::from(body[RATE_BOUND_LO_OFFSET] >> RATE_BOUND_LO_SHIFT))
            & RATE_BOUND_MASK;

        // byte 3: audio_bound(6) | fixed_flag(1) | CSPS_flag(1)
        let audio_bound = (body[AUDIO_BOUND_OFFSET] >> AUDIO_BOUND_SHIFT) & LOW_6_BITS_MASK;
        let fixed_flag = body[AUDIO_BOUND_OFFSET] & FIXED_FLAG_MASK != 0;
        let csps_flag = body[AUDIO_BOUND_OFFSET] & CSPS_FLAG_MASK != 0;

        // byte 4: system_audio_lock_flag(1) | system_video_lock_flag(1) | marker(1) | video_bound[4:0](5)
        let system_audio_lock_flag = body[STREAM_FLAGS_OFFSET] & SYSTEM_AUDIO_LOCK_FLAG_MASK != 0;
        let system_video_lock_flag = body[STREAM_FLAGS_OFFSET] & SYSTEM_VIDEO_LOCK_FLAG_MASK != 0;
        // W1 (#1119): `video_bound` is the low 5 bits of byte 4, not 4 —
        // `& 0x0F` dropped bit 4, so `video_bound` in 16..=31 round-tripped
        // to `video_bound - 16` (serialize already used the correct 5-bit
        // mask, Table 2-40).
        let video_bound = body[STREAM_FLAGS_OFFSET] & LOW_5_BITS_MASK;

        // byte 5: packet_rate_restriction_flag(1) | reserved(7)
        let packet_rate_restriction_flag =
            body[PACKET_RATE_OFFSET] & PACKET_RATE_RESTRICTION_FLAG_MASK != 0;
        let reserved_bits = body[PACKET_RATE_OFFSET] & RESERVED_BITS_MASK;

        // Stream loop — each entry starts with MSB=1 (nextbits()=='1')
        let mut pos = FIXED_BODY_LEN;
        let mut std_buffer_bounds = Vec::new();
        while pos < body.len() && body[pos] & STREAM_ENTRY_MSB_MASK != 0 {
            let stream_id = body[pos];
            if stream_id == EXT_STREAM_ID {
                // Extension form (6 bytes)
                need_in(
                    body,
                    pos + EXT_STREAM_ENTRY_LEN,
                    "system_header extended stream entry",
                )?;
                // byte pos+1: '11' + '000000'(6 bits)
                if body[pos + STREAM_BUFFER_FLAGS_OFFSET] & STREAM_ENTRY_PREFIX_MASK
                    != STREAM_ENTRY_PREFIX_MASK
                {
                    return Err(Error::BadStreamIdExtensionPrefix(
                        body[pos + STREAM_BUFFER_FLAGS_OFFSET],
                    ));
                }
                // byte pos+2: '0'(1) + stream_id_extension(7)
                let stream_id_extension =
                    body[pos + EXT_STREAM_ID_EXTENSION_OFFSET] & RESERVED_BITS_MASK;
                // byte pos+3: '1011 0110'
                if body[pos + EXT_STREAM_ID_PREFIX_OFFSET] != STREAM_ID_EXTENSION_PREFIX {
                    return Err(Error::BadStreamIdExtensionPrefix(
                        body[pos + EXT_STREAM_ID_PREFIX_OFFSET],
                    ));
                }
                // byte pos+4: '11' + scale(1) + size[12:7](5)
                if body[pos + EXT_BUFFER_FLAGS_OFFSET] & STREAM_ENTRY_PREFIX_MASK
                    != STREAM_ENTRY_PREFIX_MASK
                {
                    return Err(Error::BadMarker("P-STD_buffer_bound_scale prefix (ext)"));
                }
                let buffer_bound_scale =
                    body[pos + EXT_BUFFER_FLAGS_OFFSET] & EXT_BUFFER_SCALE_FLAG_MASK != 0;
                // bytes pos+4/pos+5: '11'(2) | scale(1) | size[12:8](5), then size[7:0](8):
                // the 13-bit size has no trailing marker bit (Table 2-40).
                let buffer_size_bound =
                    (u16::from(body[pos + EXT_BUFFER_FLAGS_OFFSET] & LOW_5_BITS_MASK)
                        << BUFFER_SIZE_BOUND_HI_SHIFT)
                        | u16::from(body[pos + EXT_BUFFER_SIZE_LO_OFFSET]);
                std_buffer_bounds.push(StdBufferBound {
                    stream_id,
                    stream_id_extension: Some(stream_id_extension),
                    buffer_bound_scale,
                    buffer_size_bound,
                });
                pos += EXT_STREAM_ENTRY_LEN;
            } else {
                // Normal form (3 bytes)
                need_in(body, pos + STREAM_ENTRY_LEN, "system_header stream entry")?;
                // byte pos+1: '11' + scale(1) + size[12:7](5)
                if body[pos + STREAM_BUFFER_FLAGS_OFFSET] & STREAM_ENTRY_PREFIX_MASK
                    != STREAM_ENTRY_PREFIX_MASK
                {
                    return Err(Error::BadMarker("P-STD_buffer_bound_scale prefix"));
                }
                let buffer_bound_scale =
                    body[pos + STREAM_BUFFER_FLAGS_OFFSET] & STREAM_BUFFER_SCALE_FLAG_MASK != 0;
                // bytes pos+1/pos+2: '11'(2) | scale(1) | size[12:8](5), then size[7:0](8).
                let buffer_size_bound =
                    (u16::from(body[pos + STREAM_BUFFER_FLAGS_OFFSET] & LOW_5_BITS_MASK)
                        << BUFFER_SIZE_BOUND_HI_SHIFT)
                        | u16::from(body[pos + STREAM_BUFFER_SIZE_LO_OFFSET]);
                std_buffer_bounds.push(StdBufferBound {
                    stream_id,
                    stream_id_extension: None,
                    buffer_bound_scale,
                    buffer_size_bound,
                });
                pos += STREAM_ENTRY_LEN;
            }
        }

        Ok(SystemHeader {
            rate_bound,
            audio_bound,
            fixed_flag,
            csps_flag,
            system_audio_lock_flag,
            system_video_lock_flag,
            video_bound,
            packet_rate_restriction_flag,
            reserved_bits,
            std_buffer_bounds,
            trailing_reserved: body[pos..].to_vec(),
        })
    }
}

impl SystemHeader {
    /// Checks that the stream loop and `trailing_reserved` would read back
    /// unchanged: the parser's loop runs while `nextbits() == '1'` (Table
    /// 2-40), and `stream_id == 0xB7` selects the 6-byte extension form.
    fn validate_stream_loop(&self) -> Result<()> {
        for b in &self.std_buffer_bounds {
            if b.stream_id & STREAM_ENTRY_MSB_MASK == 0 {
                return Err(Error::InvalidField(broadcast_common::error::InvalidField {
                    field: "system_header.stream_id",
                    reason: "stream_id MSB is 0, so the entry would end the stream loop on parse",
                }));
            }
            match b.stream_id_extension {
                Some(ext) => {
                    if b.stream_id != EXT_STREAM_ID {
                        return Err(Error::InvalidField(broadcast_common::error::InvalidField {
                            field: "system_header.stream_id_extension",
                            reason: "stream_id_extension requires stream_id 0xB7",
                        }));
                    }
                    broadcast_common::len::fit_bits(
                        u64::from(ext),
                        STREAM_ID_EXTENSION_BITS,
                        "system_header.stream_id_extension",
                    )?;
                }
                None => {
                    if b.stream_id == EXT_STREAM_ID {
                        return Err(Error::InvalidField(broadcast_common::error::InvalidField {
                            field: "system_header.stream_id",
                            reason: "stream_id 0xB7 requires stream_id_extension",
                        }));
                    }
                }
            }
            broadcast_common::len::fit_bits(
                u64::from(b.buffer_size_bound),
                BUFFER_SIZE_BOUND_BITS,
                "system_header.P-STD_buffer_size_bound",
            )?;
        }
        if self
            .trailing_reserved
            .first()
            .is_some_and(|f| f & STREAM_ENTRY_MSB_MASK != 0)
        {
            return Err(Error::InvalidField(broadcast_common::error::InvalidField {
                field: "system_header.trailing_reserved",
                reason: "first byte has its MSB set, so it would be re-read as a stream-bound entry",
            }));
        }
        Ok(())
    }
}

impl Serialize for SystemHeader {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        PREFIX_LEN + FIXED_BODY_LEN + stream_loop_len(self)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let total = self.serialized_len();
        need_out(buf, total)?;

        // system_header_start_code
        buf[START_CODE_OFFSET..HEADER_LENGTH_OFFSET]
            .copy_from_slice(&SYSTEM_HEADER_START_CODE.to_be_bytes());

        // header_length (bytes after this field)
        let header_length = broadcast_common::len::fit_u16(
            FIXED_BODY_LEN + stream_loop_len(self),
            "system_header.header_length",
        )?;
        buf[HEADER_LENGTH_OFFSET..PREFIX_LEN].copy_from_slice(&header_length.to_be_bytes());

        let rate_bound = broadcast_common::len::fit_bits(
            u64::from(self.rate_bound),
            RATE_BOUND_BITS,
            "system_header.rate_bound",
        )? as u32;
        let audio_bound = broadcast_common::len::fit_bits(
            u64::from(self.audio_bound),
            AUDIO_BOUND_BITS,
            "system_header.audio_bound",
        )? as u8;
        let video_bound = broadcast_common::len::fit_bits(
            u64::from(self.video_bound),
            VIDEO_BOUND_BITS,
            "system_header.video_bound",
        )? as u8;
        let reserved_bits = broadcast_common::len::fit_mask(
            self.reserved_bits,
            RESERVED_BITS_MASK,
            "system_header.reserved_bits",
        )?;
        self.validate_stream_loop()?;

        let body = &mut buf[PREFIX_LEN..total];
        // byte 6: marker(1) | rate_bound[21:15](7)
        body[RATE_BOUND_OFFSET] = RATE_BOUND_HI_MARKER_MASK
            | ((rate_bound >> RATE_BOUND_HI_SHIFT) & u32::from(RESERVED_BITS_MASK)) as u8;
        // byte 7: rate_bound[14:7]
        body[RATE_BOUND_MID_OFFSET] =
            ((rate_bound >> RATE_BOUND_MID_SHIFT) & u32::from(u8::MAX)) as u8;
        // byte 8: rate_bound[6:0](7) | marker(1)
        body[RATE_BOUND_LO_OFFSET] = (((rate_bound & u32::from(RESERVED_BITS_MASK)) as u8)
            << RATE_BOUND_LO_SHIFT)
            | RATE_BOUND_LO_MARKER_MASK;
        // byte 9: audio_bound[5:0](6) | fixed_flag(1) | CSPS_flag(1)
        body[AUDIO_BOUND_OFFSET] = audio_bound << AUDIO_BOUND_SHIFT
            | (u8::from(self.fixed_flag) << 1)
            | u8::from(self.csps_flag);
        // byte 10: system_audio_lock_flag(1) | system_video_lock_flag(1) | marker(1) | video_bound[4:0](5)
        body[STREAM_FLAGS_OFFSET] = (u8::from(self.system_audio_lock_flag) << SYSTEM_AUDIO_LOCK_SHIFT)
                | (u8::from(self.system_video_lock_flag) << SYSTEM_VIDEO_LOCK_SHIFT)
                | VIDEO_BOUND_MARKER_MASK // marker_bit (bit5 of byte 10)
                | video_bound;
        // byte 11: packet_rate_restriction_flag(1) | reserved(7)
        body[PACKET_RATE_OFFSET] = (u8::from(self.packet_rate_restriction_flag)
            << PACKET_RATE_RESTRICTION_SHIFT)
            | reserved_bits;

        // Stream loop
        let mut pos = FIXED_BODY_LEN;
        for bound in &self.std_buffer_bounds {
            if let Some(ext) = bound.stream_id_extension {
                body[pos] = bound.stream_id;
                body[pos + STREAM_BUFFER_FLAGS_OFFSET] = STREAM_ENTRY_PREFIX_MASK;
                body[pos + EXT_STREAM_ID_EXTENSION_OFFSET] = ext;
                body[pos + EXT_STREAM_ID_PREFIX_OFFSET] = STREAM_ID_EXTENSION_PREFIX;
                body[pos + EXT_BUFFER_FLAGS_OFFSET] = STREAM_ENTRY_PREFIX_MASK
                    | (u8::from(bound.buffer_bound_scale) << BUFFER_BOUND_SCALE_SHIFT)
                    | (bound.buffer_size_bound >> BUFFER_SIZE_BOUND_HI_SHIFT) as u8;
                body[pos + EXT_BUFFER_SIZE_LO_OFFSET] = bound.buffer_size_bound.to_be_bytes()[1];
                pos += EXT_STREAM_ENTRY_LEN;
            } else {
                body[pos] = bound.stream_id;
                body[pos + STREAM_BUFFER_FLAGS_OFFSET] = STREAM_ENTRY_PREFIX_MASK
                    | (u8::from(bound.buffer_bound_scale) << BUFFER_BOUND_SCALE_SHIFT)
                    | (bound.buffer_size_bound >> BUFFER_SIZE_BOUND_HI_SHIFT) as u8;
                body[pos + STREAM_BUFFER_SIZE_LO_OFFSET] = bound.buffer_size_bound.to_be_bytes()[1];
                pos += STREAM_ENTRY_LEN;
            }
        }
        body[pos..pos + self.trailing_reserved.len()].copy_from_slice(&self.trailing_reserved);

        Ok(total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    /// #2269: a too-small output buffer is an output-buffer error, not a
    /// parse-side input-truncation error.
    #[test]
    fn serialize_into_too_small_output_reports_output_buffer_error() {
        let bytes = vec![
            0x00, 0x00, 0x01, 0xBB, // start_code
            0x00, 0x06, // header_length = 6 (just the fixed part)
            0x80, 0x00, 0x01, // rate_bound=0, markers
            0x04, // audio_bound=1, fixed=0, CSPS=0
            0x20, // audio_lock=0, video_lock=0, marker=1, video_bound=0
            0xFF, // packet_rate=1, reserved=0x7F
        ];
        let h = SystemHeader::parse(&bytes).unwrap();
        let mut out = vec![0u8; h.serialized_len() - 1];
        assert!(matches!(
            h.serialize_into(&mut out),
            Err(Error::OutputBufferTooSmall(_))
        ));
    }

    #[test]
    fn system_header_round_trip_no_streams() {
        let bytes = vec![
            0x00, 0x00, 0x01, 0xBB, // start_code
            0x00, 0x06, // header_length = 6 (just the fixed part)
            0x80, 0x00, 0x01, // rate_bound=0, markers
            0x04, // audio_bound=1, fixed=0, CSPS=0
            0x20, // audio_lock=0, video_lock=0, marker=1, video_bound=0
            0xFF, // packet_rate=1, reserved=0x7F
        ];
        let h = SystemHeader::parse(&bytes).unwrap();
        assert_eq!(h.rate_bound, 0);
        assert_eq!(h.audio_bound, 1);
        assert!(!h.fixed_flag);
        assert!(!h.csps_flag);
        assert!(!h.system_audio_lock_flag);
        assert!(!h.system_video_lock_flag);
        assert_eq!(h.video_bound, 0);
        assert!(h.packet_rate_restriction_flag);
        assert!(h.std_buffer_bounds.is_empty());

        let mut out = vec![0u8; h.serialized_len()];
        h.serialize_into(&mut out).unwrap();
        assert_eq!(&out[..], &bytes[..]);

        let h2 = SystemHeader::parse(&out).unwrap();
        assert_eq!(h, h2);
    }

    /// W1 (#1119): `video_bound` (byte 4 bits `[4:0]`, Table 2-40) was
    /// masked with `0x0F` (4 bits) on parse but `0x1F` (5 bits) on
    /// serialize. `video_bound` in `16..=31` sets bit 4, which the old
    /// parse dropped — `video_bound - 16` came back instead.
    #[test]
    fn video_bound_bit_4_round_trips() {
        let bytes = vec![
            0x00,
            0x00,
            0x01,
            0xBB, // start_code
            0x00,
            0x06, // header_length = 6
            0x80,
            0x00,
            0x01,        // rate_bound=0, markers
            0x04,        // audio_bound=1, fixed=0, CSPS=0
            0x20 | 0x1F, // audio_lock=0, video_lock=0, marker=1, video_bound=31 (bit 4 set)
            0xFF,
        ];
        let h = SystemHeader::parse(&bytes).unwrap();
        assert_eq!(
            h.video_bound, 31,
            "bit 4 of video_bound must not be dropped"
        );

        let mut out = vec![0u8; h.serialized_len()];
        h.serialize_into(&mut out).unwrap();
        assert_eq!(&out[..], &bytes[..]);
    }

    #[test]
    fn system_header_round_trip_with_streams() {
        let bytes = vec![
            0x00, 0x00, 0x01, 0xBB, 0x00,
            0x0C, // header_length = 12 (6 fixed + 6 for 2 streams)
            0x80, 0x00, 0x01, 0x04, 0x20, 0xFF,
            // stream 1: stream_id=0xE0, scale=1, size=0x1FFF
            0xE0, 0xFF, 0xFF, // stream 2: stream_id=0xC0, scale=0, size=0x0100
            0xC0, 0xC1, 0x00,
        ];
        let h = SystemHeader::parse(&bytes).unwrap();
        assert_eq!(h.std_buffer_bounds.len(), 2);
        assert_eq!(h.std_buffer_bounds[0].stream_id, 0xE0);
        assert!(h.std_buffer_bounds[0].stream_id_extension.is_none());
        assert!(h.std_buffer_bounds[0].buffer_bound_scale);
        assert_eq!(h.std_buffer_bounds[0].buffer_size_bound, 0x1FFF);

        assert_eq!(h.std_buffer_bounds[1].stream_id, 0xC0);
        assert!(h.std_buffer_bounds[1].stream_id_extension.is_none());
        assert!(!h.std_buffer_bounds[1].buffer_bound_scale);
        assert_eq!(h.std_buffer_bounds[1].buffer_size_bound, 0x0100);

        let mut out = vec![0u8; h.serialized_len()];
        h.serialize_into(&mut out).unwrap();
        assert_eq!(&out[..], &bytes[..]);

        let h2 = SystemHeader::parse(&out).unwrap();
        assert_eq!(h, h2);

        // Mutation test
        let h_mut = SystemHeader {
            rate_bound: 12345,
            ..h.clone()
        };
        let mut out2 = vec![0u8; h_mut.serialized_len()];
        h_mut.serialize_into(&mut out2).unwrap();
        assert_ne!(&out[..], &out2[..]);
    }

    /// r14-MPS-W8 (#1119): padding that `header_length` declares after the
    /// stream-bound loop is preserved, not dropped (which shrank
    /// `header_length` on re-serialize).
    #[test]
    fn trailing_bytes_after_stream_loop_round_trip() {
        let bytes = vec![
            0x00,
            0x00,
            0x01,
            0xBB, // start_code
            0x00,
            0x0C, // header_length = 12: 6 fixed + 3 stream entry + 3 trailing
            0x80,
            0x00,
            0x01,
            0x04,
            0x20 | 0x01,
            0xFF, // fixed body
            0xE0,
            0xE0,
            0x80, // one stream entry: stream_id 0xE0, bound
            0x00,
            0x7F,
            0xAB, // trailing, not a stream entry (MSB clear)
        ];
        let h = SystemHeader::parse(&bytes).unwrap();
        assert_eq!(h.std_buffer_bounds.len(), 1);
        assert_eq!(h.trailing_reserved, vec![0x00, 0x7F, 0xAB]);
        let mut out = vec![0u8; h.serialized_len()];
        h.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    /// r14-MPS-W8: an encoder that wrote `reserved_bits` other than the
    /// conventional `0x7F` must re-serialize byte-identically.
    #[test]
    fn non_conventional_reserved_bits_round_trip() {
        let bytes = vec![
            0x00, 0x00, 0x01, 0xBB, // start_code
            0x00, 0x06, // header_length
            0x80, 0x00, 0x01, // rate_bound=0, markers
            0x04, // audio_bound=1
            0x20, // marker=1, video_bound=0
            0x2A, // packet_rate=0, reserved_bits=0b010_1010
        ];
        let h = SystemHeader::parse(&bytes).unwrap();
        assert_eq!(h.reserved_bits, 0x2A);
        assert!(!h.packet_rate_restriction_flag);
        let mut out = vec![0u8; h.serialized_len()];
        h.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes);
    }

    /// MPS-W7 (#1129): `stream_loop_len` accumulated in `u16`, overflowing
    /// (debug panic / release wrap) for more than 10 921 extension-form
    /// bounds (6 bytes each: true total 66 006 bytes > `u16::MAX`). The old
    /// accumulator's overflow point is the same 65 535 boundary the wire
    /// field itself has, so the only fixable difference is *how* an
    /// over-limit count fails: the old code panicked (arithmetic overflow in
    /// debug; an out-of-bounds buffer write in release, because
    /// `serialized_len()` used the same wrapped, too-small value the loop
    /// then wrote past). The fix must return a controlled `Err` here
    /// instead — never panic. Confirmed pre-fix: this exact case panicked
    /// with "attempt to add with overflow".
    #[test]
    fn many_bounds_past_old_u16_overflow_point_errs_not_panics() {
        let std_buffer_bounds = (0..11_000)
            .map(|_| StdBufferBound {
                stream_id: EXT_STREAM_ID,
                stream_id_extension: Some(0x05),
                buffer_bound_scale: false,
                buffer_size_bound: 0,
            })
            .collect::<Vec<_>>();
        let h = SystemHeader {
            rate_bound: 0,
            audio_bound: 0,
            fixed_flag: false,
            csps_flag: false,
            system_audio_lock_flag: false,
            system_video_lock_flag: false,
            video_bound: 0,
            packet_rate_restriction_flag: false,
            reserved_bits: RESERVED_BITS_MASK,
            std_buffer_bounds,
            trailing_reserved: Vec::new(),
        };
        // Must not panic building the (correctly-sized-in-usize) buffer or
        // writing into it; the real total exceeds u16::MAX so this is a
        // controlled Err, not a silently wrapped Ok.
        let mut buf = vec![0u8; h.serialized_len()];
        assert!(matches!(
            h.serialize_into(&mut buf),
            Err(Error::FieldOverflow(_))
        ));
    }

    /// A bound count whose real `header_length` exceeds the 16-bit wire
    /// field (`6 + 25_000*3 = 75_006 > 65_535`) must error rather than wrap.
    #[test]
    fn header_length_over_u16_is_rejected_not_wrapped() {
        let std_buffer_bounds = (0..25_000)
            .map(|_| StdBufferBound {
                stream_id: 0xC0,
                stream_id_extension: None,
                buffer_bound_scale: false,
                buffer_size_bound: 0,
            })
            .collect::<Vec<_>>();
        let h = SystemHeader {
            rate_bound: 0,
            audio_bound: 0,
            fixed_flag: false,
            csps_flag: false,
            system_audio_lock_flag: false,
            system_video_lock_flag: false,
            video_bound: 0,
            packet_rate_restriction_flag: false,
            reserved_bits: RESERVED_BITS_MASK,
            std_buffer_bounds,
            trailing_reserved: Vec::new(),
        };
        let mut buf = vec![0u8; h.serialized_len()];
        assert!(matches!(
            h.serialize_into(&mut buf),
            Err(Error::FieldOverflow(_))
        ));
    }

    /// The boundary: a bound count whose `header_length` still fits u16
    /// serializes and round-trips.
    #[test]
    fn many_bounds_within_u16_header_length_round_trips() {
        let std_buffer_bounds = (0..1000)
            .map(|_| StdBufferBound {
                stream_id: 0xC0,
                stream_id_extension: None,
                buffer_bound_scale: false,
                buffer_size_bound: 0x0100,
            })
            .collect::<Vec<_>>();
        let h = SystemHeader {
            rate_bound: 0,
            audio_bound: 0,
            fixed_flag: false,
            csps_flag: false,
            system_audio_lock_flag: false,
            system_video_lock_flag: false,
            video_bound: 0,
            packet_rate_restriction_flag: false,
            reserved_bits: RESERVED_BITS_MASK,
            std_buffer_bounds,
            trailing_reserved: Vec::new(),
        };
        let mut buf = vec![0u8; h.serialized_len()];
        h.serialize_into(&mut buf).unwrap();
        let back = SystemHeader::parse(&buf).unwrap();
        assert_eq!(back.std_buffer_bounds.len(), 1000);
    }

    #[test]
    fn system_header_round_trip_extended_stream_id() {
        let bytes = vec![
            0x00, 0x00, 0x01, 0xBB, 0x00, 0x0C, // header_length = 12
            0x80, 0x00, 0x01, 0x04, 0x20, 0xFF,
            // extended stream: stream_id=0xB7, ext=0x05, scale=1, size=0x0100
            0xB7, 0xC0, 0x05, 0xB6, 0xE1, 0x00,
        ];
        let h = SystemHeader::parse(&bytes).unwrap();
        assert_eq!(h.std_buffer_bounds.len(), 1);
        assert_eq!(h.std_buffer_bounds[0].stream_id, 0xB7);
        assert_eq!(h.std_buffer_bounds[0].stream_id_extension, Some(0x05));
        assert!(h.std_buffer_bounds[0].buffer_bound_scale);
        assert_eq!(h.std_buffer_bounds[0].buffer_size_bound, 0x100);

        let mut out = vec![0u8; h.serialized_len()];
        h.serialize_into(&mut out).unwrap();
        assert_eq!(&out[..], &bytes[..]);
    }

    fn ext_entry_header(entry: [u8; 6]) -> Vec<u8> {
        let mut bytes = vec![
            0x00, 0x00, 0x01, 0xBB, 0x00, 0x0C, 0x80, 0x00, 0x01, 0x04, 0x20, 0xFF,
        ];
        bytes.extend_from_slice(&entry);
        bytes
    }

    #[test]
    fn a_bad_stream_id_extension_prefix_is_reported_with_the_offending_byte() {
        assert_eq!(
            SystemHeader::parse(&ext_entry_header([0xB7, 0x00, 0x00, 0xB6, 0xC0, 0x00])),
            Err(Error::BadStreamIdExtensionPrefix(0x00))
        );
        assert_eq!(
            SystemHeader::parse(&ext_entry_header([0xB7, 0xC0, 0x00, 0x11, 0xC0, 0x00])),
            Err(Error::BadStreamIdExtensionPrefix(0x11))
        );
    }

    fn bare_header() -> SystemHeader {
        let bytes = vec![
            0x00, 0x00, 0x01, 0xBB, 0x00, 0x06, 0x80, 0x00, 0x01, 0x04, 0x20, 0xFF,
        ];
        SystemHeader::parse(&bytes).unwrap()
    }

    fn serialize_error(h: &SystemHeader) -> Error {
        let mut out = vec![0u8; h.serialized_len()];
        h.serialize_into(&mut out).unwrap_err()
    }

    fn invalid(field: &'static str, reason: &'static str) -> Error {
        Error::InvalidField(broadcast_common::error::InvalidField { field, reason })
    }

    #[test]
    fn a_stream_loop_that_would_not_read_back_is_an_invalid_field() {
        let bound = |stream_id, stream_id_extension| StdBufferBound {
            stream_id,
            stream_id_extension,
            buffer_bound_scale: false,
            buffer_size_bound: 1,
        };
        let mut h = bare_header();
        h.std_buffer_bounds = vec![bound(0x7F, None)];
        assert_eq!(
            serialize_error(&h),
            invalid(
                "system_header.stream_id",
                "stream_id MSB is 0, so the entry would end the stream loop on parse"
            )
        );
        h.std_buffer_bounds = vec![bound(0xE0, Some(1))];
        assert_eq!(
            serialize_error(&h),
            invalid(
                "system_header.stream_id_extension",
                "stream_id_extension requires stream_id 0xB7"
            )
        );
        h.std_buffer_bounds = vec![bound(0xB7, None)];
        assert_eq!(
            serialize_error(&h),
            invalid(
                "system_header.stream_id",
                "stream_id 0xB7 requires stream_id_extension"
            )
        );
        h.std_buffer_bounds = Vec::new();
        h.trailing_reserved = vec![0x80];
        assert_eq!(
            serialize_error(&h),
            invalid(
                "system_header.trailing_reserved",
                "first byte has its MSB set, so it would be re-read as a stream-bound entry"
            )
        );
    }
}
