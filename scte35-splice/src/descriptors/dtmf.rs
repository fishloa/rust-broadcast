//! DTMF_descriptor() — ANSI/SCTE 35 2023r1 §10.3.2, Table 19 (tag 0x01).
//!
//! Lets a receiver generate a legacy analog DTMF sequence on a splice. Carries
//! a `preroll` (tenths of a second) and `dtmf_count` ASCII DTMF characters.

use alloc::vec::Vec;

use super::header::{self, CUEI, HEADER_LEN};
use crate::error::{Error, Result};
use crate::traits::SpliceDescriptorDef;
use broadcast_common::{Parse, Serialize};

/// `splice_descriptor_tag` for DTMF_descriptor (§10.1, Table 16).
pub const TAG: u8 = 0x01;

/// DTMF_descriptor() — §10.3.2, Table 19.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DtmfDescriptor {
    /// 32-bit `identifier` (shall be "CUEI").
    pub identifier: u32,
    /// `preroll` in tenths of a second (0..=25.5 s).
    pub preroll: u8,
    /// The DTMF characters ('0'..='9', '*', '#'); `dtmf_count` is its length.
    pub dtmf_chars: Vec<u8>,
}

impl Default for DtmfDescriptor {
    fn default() -> Self {
        Self {
            identifier: CUEI,
            preroll: 0,
            dtmf_chars: Vec::new(),
        }
    }
}

impl DtmfDescriptor {
    /// `preroll` decoded to a [`Duration`](core::time::Duration) (tenths of a
    /// second).
    #[must_use]
    pub fn preroll_duration(&self) -> core::time::Duration {
        core::time::Duration::from_millis(u64::from(self.preroll) * 100)
    }
}

impl<'a> Parse<'a> for DtmfDescriptor {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (identifier, body) = header::descriptor_body(bytes, TAG, "DTMF_descriptor")?;
        if body.len() < 2 {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN + 2,
                have: bytes.len(),
                what: "DTMF_descriptor preroll/count",
            });
        }
        let preroll = body[0];
        let dtmf_count = (body[1] >> 5) as usize; // 3-bit count, 5 reserved
        if body.len() < 2 + dtmf_count {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN + 2 + dtmf_count,
                have: bytes.len(),
                what: "DTMF_descriptor chars",
            });
        }
        Ok(Self {
            identifier,
            preroll,
            dtmf_chars: body[2..2 + dtmf_count].to_vec(),
        })
    }
}

impl Serialize for DtmfDescriptor {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + 2 + self.dtmf_chars.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        if self.dtmf_chars.len() > 7 {
            return Err(Error::InvalidValue {
                field: "DTMF_descriptor.dtmf_count",
                reason: "more than 7 DTMF characters (3-bit count)",
            });
        }
        header::write_header(buf, TAG, self.identifier, 2 + self.dtmf_chars.len());
        buf[HEADER_LEN] = self.preroll;
        // 3-bit dtmf_count, 5 reserved bits = 1.
        buf[HEADER_LEN + 1] = ((self.dtmf_chars.len() as u8) << 5) | 0x1F;
        buf[HEADER_LEN + 2..need].copy_from_slice(&self.dtmf_chars);
        Ok(need)
    }
}

impl<'a> SpliceDescriptorDef<'a> for DtmfDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "DTMF";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let d = DtmfDescriptor {
            identifier: CUEI,
            preroll: 40,
            dtmf_chars: vec![b'1', b'2', b'3', b'*'],
        };
        let bytes = d.to_bytes();
        assert_eq!(bytes[0], TAG);
        let back = DtmfDescriptor::parse(&bytes).unwrap();
        assert_eq!(d, back);
        assert_eq!(back.to_bytes(), bytes);
        assert_eq!(back.preroll_duration(), core::time::Duration::from_secs(4));
    }

    #[test]
    fn round_trip_empty_chars() {
        let d = DtmfDescriptor::default();
        let back = DtmfDescriptor::parse(&d.to_bytes()).unwrap();
        assert_eq!(d, back);
    }
}
