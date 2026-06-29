//! audio_descriptor() — ANSI/SCTE 35 2023r1 §10.3.5, Table 28 (tag 0x04).
//!
//! Dynamically signals the audios in use in the stream: an `audio_count` loop
//! of per-component language + ATSC A/52 audio attributes.

use alloc::vec::Vec;

use super::header::{self, CUEI, HEADER_LEN};
use crate::error::{Error, Result};
use crate::traits::SpliceDescriptorDef;
use broadcast_common::{Parse, Serialize};

/// `splice_descriptor_tag` for audio_descriptor (§10.1, Table 16).
pub const TAG: u8 = 0x04;

/// Wire length of one audio component entry: component_tag (1) + ISO_code (3)
/// + a packed byte (Bit_Stream_Mode/Num_Channels/Full_Srvc_Audio).
const COMPONENT_LEN: usize = 5;

/// One entry in the `audio_count` loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AudioComponent {
    /// 8-bit `component_tag` (0xFF if not used; order then from PMT).
    pub component_tag: u8,
    /// 3-byte ISO 639-2 language code.
    pub iso_code: [u8; 3],
    /// 3-bit `Bit_Stream_Mode` (ATSC A/52 Table 5.7).
    pub bit_stream_mode: u8,
    /// 4-bit `Num_Channels` (ATSC A/52 Table A4.5).
    pub num_channels: u8,
    /// `Full_Srvc_Audio` flag (ATSC A/52 Annex A.4.3).
    pub full_srvc_audio: bool,
}

/// audio_descriptor() — §10.3.5, Table 28.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AudioDescriptor {
    /// 32-bit `identifier` (shall be "CUEI").
    pub identifier: u32,
    /// The audio components; `audio_count` is its length (max 15, 4-bit).
    pub components: Vec<AudioComponent>,
}

impl Default for AudioDescriptor {
    fn default() -> Self {
        Self {
            identifier: CUEI,
            components: Vec::new(),
        }
    }
}

impl<'a> Parse<'a> for AudioDescriptor {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (identifier, body) = header::descriptor_body(bytes, TAG, "audio_descriptor")?;
        if body.is_empty() {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN + 1,
                have: bytes.len(),
                what: "audio_descriptor audio_count",
            });
        }
        let audio_count = (body[0] >> 4) as usize; // 4-bit count, 4 reserved
        let mut pos = 1;
        let mut components = Vec::with_capacity(audio_count);
        for _ in 0..audio_count {
            if body.len() < pos + COMPONENT_LEN {
                return Err(Error::BufferTooShort {
                    need: HEADER_LEN + pos + COMPONENT_LEN,
                    have: bytes.len(),
                    what: "audio_descriptor component",
                });
            }
            let component_tag = body[pos];
            let iso_code = [body[pos + 1], body[pos + 2], body[pos + 3]];
            let packed = body[pos + 4];
            components.push(AudioComponent {
                component_tag,
                iso_code,
                bit_stream_mode: packed >> 5,        // 3 bits
                num_channels: (packed >> 1) & 0x0F,  // 4 bits
                full_srvc_audio: packed & 0x01 != 0, // 1 bit
            });
            pos += COMPONENT_LEN;
        }
        Ok(Self {
            identifier,
            components,
        })
    }
}

impl Serialize for AudioDescriptor {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + 1 + self.components.len() * COMPONENT_LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        if self.components.len() > 15 {
            return Err(Error::InvalidValue {
                field: "audio_descriptor.audio_count",
                reason: "more than 15 components (4-bit count)",
            });
        }
        let body_len = 1 + self.components.len() * COMPONENT_LEN;
        header::write_header(buf, TAG, self.identifier, body_len);
        // 4-bit audio_count, 4 reserved bits = 1.
        buf[HEADER_LEN] = ((self.components.len() as u8) << 4) | 0x0F;
        let mut pos = HEADER_LEN + 1;
        for c in &self.components {
            buf[pos] = c.component_tag;
            buf[pos + 1..pos + 4].copy_from_slice(&c.iso_code);
            buf[pos + 4] = ((c.bit_stream_mode & 0x07) << 5)
                | ((c.num_channels & 0x0F) << 1)
                | u8::from(c.full_srvc_audio);
            pos += COMPONENT_LEN;
        }
        Ok(need)
    }
}

impl<'a> SpliceDescriptorDef<'a> for AudioDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "AUDIO";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let d = AudioDescriptor {
            identifier: CUEI,
            components: vec![
                AudioComponent {
                    component_tag: 0xFF,
                    iso_code: *b"eng",
                    bit_stream_mode: 0b101,
                    num_channels: 0b1010,
                    full_srvc_audio: true,
                },
                AudioComponent {
                    component_tag: 0x12,
                    iso_code: *b"spa",
                    bit_stream_mode: 0,
                    num_channels: 2,
                    full_srvc_audio: false,
                },
            ],
        };
        let bytes = d.to_bytes();
        assert_eq!(bytes[0], TAG);
        let back = AudioDescriptor::parse(&bytes).unwrap();
        assert_eq!(d, back);
        assert_eq!(back.to_bytes(), bytes);
    }

    #[test]
    fn round_trip_empty() {
        let d = AudioDescriptor::default();
        assert_eq!(AudioDescriptor::parse(&d.to_bytes()).unwrap(), d);
    }
}
