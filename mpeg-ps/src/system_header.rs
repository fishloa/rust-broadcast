//! System Header — ISO/IEC 13818-1 §2.5.3.5, Table 2-40.
//!
//! The (optional) system header follows immediately after the pack header
//! stuffing bytes, in the first pack of the stream. It constrains the P-STD
//! model: `rate_bound`, `audio_bound`/`video_bound`, and per-stream P-STD
//! buffer-size bounds.

use alloc::vec::Vec;

use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// `system_header_start_code` — `0x000001BB`.
pub const SYSTEM_HEADER_START_CODE: u32 = 0x0000_01BB;

/// `stream_id` value that triggers the extension (extended_stream_id) form.
const EXT_STREAM_ID: u8 = 0xB7;

/// Fixed bytes before the stream loop: start_code(4) + header_length(2).
const PREFIX_LEN: usize = 6;

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
    /// Per-stream P-STD buffer bounds (the `while (nextbits() == '1')` loop).
    pub std_buffer_bounds: Vec<StdBufferBound>,
}

fn stream_loop_len(system_header: &SystemHeader) -> u16 {
    let mut len: u16 = 0;
    for b in &system_header.std_buffer_bounds {
        if b.stream_id_extension.is_some() {
            len += 6;
        } else {
            len += 3;
        }
    }
    len
}

impl<'a> Parse<'a> for SystemHeader {
    type Error = Error;

    fn parse(b: &'a [u8]) -> Result<Self> {
        if b.len() < PREFIX_LEN {
            return Err(Error::BufferTooShort {
                need: PREFIX_LEN,
                have: b.len(),
                what: "system_header prefix",
            });
        }

        if u32::from_be_bytes([b[0], b[1], b[2], b[3]]) != SYSTEM_HEADER_START_CODE {
            return Err(Error::BadSystemHeaderStartCode(u32::from_be_bytes([
                b[0], b[1], b[2], b[3],
            ])));
        }

        let header_length = u16::from_be_bytes([b[4], b[5]]) as usize;
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

        if body.len() < 6 {
            return Err(Error::BufferTooShort {
                need: 6 + PREFIX_LEN,
                have: b.len(),
                what: "system_header fixed body",
            });
        }

        // Marker bit checks
        if body[0] & 0x80 == 0 {
            return Err(Error::BadMarker("system_header rate_bound marker 1"));
        }
        if body[2] & 0x01 == 0 {
            return Err(Error::BadMarker("system_header rate_bound marker 2"));
        }
        if body[4] & 0x20 == 0 {
            return Err(Error::BadMarker("system_header video_bound marker"));
        }

        // rate_bound: 22 bits
        let rate_bound = ((u32::from(body[0] & 0x7F) << 15)
            | (u32::from(body[1]) << 7)
            | u32::from(body[2] >> 1))
            & 0x3F_FFFF;

        // byte 3: audio_bound(6) | fixed_flag(1) | CSPS_flag(1)
        let audio_bound = (body[3] >> 2) & 0x3F;
        let fixed_flag = body[3] & 0x02 != 0;
        let csps_flag = body[3] & 0x01 != 0;

        // byte 4: system_audio_lock_flag(1) | system_video_lock_flag(1) | marker(1) | video_bound[4:0](5)
        let system_audio_lock_flag = body[4] & 0x80 != 0;
        let system_video_lock_flag = body[4] & 0x40 != 0;
        let video_bound = body[4] & 0x0F;

        // byte 5: packet_rate_restriction_flag(1) | reserved(7)
        let packet_rate_restriction_flag = body[5] & 0x80 != 0;

        // Stream loop — each entry starts with MSB=1 (nextbits()=='1')
        let mut pos = 6;
        let mut std_buffer_bounds = Vec::new();
        while pos < body.len() && body[pos] & 0x80 != 0 {
            let stream_id = body[pos];
            if stream_id == EXT_STREAM_ID {
                // Extension form (6 bytes)
                if pos + 6 > body.len() {
                    return Err(Error::BufferTooShort {
                        need: pos + 6,
                        have: body.len(),
                        what: "system_header extended stream entry",
                    });
                }
                // byte pos+1: '11' + '000 0000'(5 bits)
                if body[pos + 1] & 0xC0 != 0xC0 {
                    return Err(Error::BadStreamIdExtensionPrefix(body[pos + 1]));
                }
                // byte pos+2: '000 0000'(1) + stream_id_extension(7)
                let stream_id_extension = body[pos + 2] & 0x7F;
                // byte pos+3: '1011 0110'
                if body[pos + 3] != 0xB6 {
                    return Err(Error::BadStreamIdExtensionPrefix(body[pos + 3]));
                }
                // byte pos+4: '11' + scale(1) + size[12:7](5)
                if body[pos + 4] & 0xC0 != 0xC0 {
                    return Err(Error::BadMarker("P-STD_buffer_bound_scale prefix (ext)"));
                }
                let buffer_bound_scale = body[pos + 4] & 0x20 != 0;
                // byte pos+5: size[6:0](7) + marker(1)? No — the 13-bit size fits in the remaining bits
                // byte pos+4 has 5 bits of size; byte pos+5 has 7 bits + marker at bit0?
                // Actually: P-STD_buffer_bound_scale(1) + P-STD_buffer_size_bound(13) = 14 bits
                // byte pos+4: '11'(2) | scale(1) | size[12:7](5) = 8 bits
                // byte pos+5: size[6:0](7) | marker?
                // From Table 2-40: after the scale+size, the stream loop tests nextbits()=='1' so
                // the next byte's MSB must be set. But the size is 13 bits — only 12 fit in bytes 4-5.
                // Wait: scale(1) + size(13) = 14 bits. byte4 has 5 bits of size after 3 used bits.
                // byte5 has all 8 bits = 5+8=13 bits of size. No marker.
                let buffer_size_bound =
                    (u16::from(body[pos + 4] & 0x1F) << 8) | u16::from(body[pos + 5]);
                std_buffer_bounds.push(StdBufferBound {
                    stream_id,
                    stream_id_extension: Some(stream_id_extension),
                    buffer_bound_scale,
                    buffer_size_bound,
                });
                pos += 6;
            } else {
                // Normal form (3 bytes)
                if pos + 3 > body.len() {
                    return Err(Error::BufferTooShort {
                        need: pos + 3,
                        have: body.len(),
                        what: "system_header stream entry",
                    });
                }
                // byte pos+1: '11' + scale(1) + size[12:7](5)
                if body[pos + 1] & 0xC0 != 0xC0 {
                    return Err(Error::BadMarker("P-STD_buffer_bound_scale prefix"));
                }
                let buffer_bound_scale = body[pos + 1] & 0x20 != 0;
                // byte pos+2: size[6:0](7) — no marker in normal form either
                // Actually: for the non-ext form too, the 13-bit size spans 5 bits in byte1 + 8 in byte2
                let buffer_size_bound =
                    (u16::from(body[pos + 1] & 0x1F) << 8) | u16::from(body[pos + 2]);
                std_buffer_bounds.push(StdBufferBound {
                    stream_id,
                    stream_id_extension: None,
                    buffer_bound_scale,
                    buffer_size_bound,
                });
                pos += 3;
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
            std_buffer_bounds,
        })
    }
}

impl Serialize for SystemHeader {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        PREFIX_LEN + 6 + stream_loop_len(self) as usize
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let total = self.serialized_len();
        if buf.len() < total {
            return Err(Error::BufferTooShort {
                need: total,
                have: buf.len(),
                what: "system_header serialize output",
            });
        }

        // system_header_start_code
        buf[0..4].copy_from_slice(&SYSTEM_HEADER_START_CODE.to_be_bytes());

        // header_length (bytes after this field)
        let header_length = 6 + stream_loop_len(self);
        buf[4..6].copy_from_slice(&header_length.to_be_bytes());

        let rate_bound = self.rate_bound & 0x3F_FFFF;
        // byte 6: marker(1) | rate_bound[21:15](7)
        buf[6] = 0x80 | ((rate_bound >> 15) & 0x7F) as u8;
        // byte 7: rate_bound[14:7]
        buf[7] = ((rate_bound >> 7) & 0xFF) as u8;
        // byte 8: rate_bound[6:0](7) | marker(1)
        buf[8] = (((rate_bound & 0x7F) as u8) << 1) | 0x01;
        // byte 9: audio_bound[5:0](6) | fixed_flag(1) | CSPS_flag(1)
        buf[9] = (self.audio_bound & 0x3F) << 2
            | (u8::from(self.fixed_flag) << 1)
            | u8::from(self.csps_flag);
        // byte 10: system_audio_lock_flag(1) | system_video_lock_flag(1) | marker(1) | video_bound[4:0](5)
        buf[10] = (u8::from(self.system_audio_lock_flag) << 7)
            | (u8::from(self.system_video_lock_flag) << 6)
            | 0x20 // marker_bit (bit5 of byte 10)
            | (self.video_bound & 0x1F);
        // byte 11: packet_rate_restriction_flag(1) | reserved(7)
        buf[11] = (u8::from(self.packet_rate_restriction_flag) << 7) | 0x7F;

        // Stream loop
        let mut pos = 12;
        for bound in &self.std_buffer_bounds {
            if let Some(ext) = bound.stream_id_extension {
                buf[pos] = bound.stream_id;
                buf[pos + 1] = 0xC0;
                buf[pos + 2] = ext & 0x7F;
                buf[pos + 3] = 0xB6;
                buf[pos + 4] = 0xC0
                    | (u8::from(bound.buffer_bound_scale) << 5)
                    | ((bound.buffer_size_bound >> 8) & 0x1F) as u8;
                buf[pos + 5] = (bound.buffer_size_bound & 0xFF) as u8;
                pos += 6;
            } else {
                buf[pos] = bound.stream_id;
                buf[pos + 1] = 0xC0
                    | (u8::from(bound.buffer_bound_scale) << 5)
                    | ((bound.buffer_size_bound >> 8) & 0x1F) as u8;
                buf[pos + 2] = (bound.buffer_size_bound & 0xFF) as u8;
                pos += 3;
            }
        }

        Ok(total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

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
}
