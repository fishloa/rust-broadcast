//! TVA_id Descriptor — ETSI TS 102 323 §11.2.4, Table 114 (tag 0x75).
//!
//! Lists one or more TV-Anytime identifiers, each with a running_status that
//! a receiver uses to drive its recording strategy. Per the TVA PDF
//! (etsi_ts_102_323_v01.04.01, p. 101, Table 114) each loop entry is 3 bytes:
//! TVA_id(16) + Reserved(5) + running_status(3). running_status values are
//! defined in Table 115 (0=reserved, 1=not yet running, 2=starts shortly,
//! 3=paused, 4=running, 5=cancelled, 6=completed, 7=reserved).

use crate::error::{Error, Result};
use crate::traits::Descriptor;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for TVA_id_descriptor.
pub const TAG: u8 = 0x75;
const HEADER_LEN: usize = 2;
const ENTRY_LEN: usize = 3;

/// Largest representable 3-bit running_status.
const RUNNING_STATUS_MAX: u8 = 0x07;

/// One TVA_id loop entry.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TvaIdEntry {
    /// 16-bit TVA_id referencing the item of content.
    pub tva_id: u16,
    /// 3-bit running_status (Table 115).
    pub running_status: u8,
}

/// TVA_id Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TvaIdDescriptor {
    /// Entries in wire order.
    pub entries: Vec<TvaIdEntry>,
}

impl<'a> Parse<'a> for TvaIdDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN,
                have: bytes.len(),
                what: "TvaIdDescriptor header",
            });
        }
        if bytes[0] != TAG {
            return Err(Error::InvalidDescriptor {
                tag: bytes[0],
                reason: "unexpected tag for TVA_id_descriptor",
            });
        }
        let length = bytes[1] as usize;
        let end = HEADER_LEN + length;
        if bytes.len() < end {
            return Err(Error::BufferTooShort {
                need: end,
                have: bytes.len(),
                what: "TvaIdDescriptor body",
            });
        }
        if length % ENTRY_LEN != 0 {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "TVA_id_descriptor length must be a multiple of 3",
            });
        }
        let body = &bytes[HEADER_LEN..end];
        let mut entries = Vec::with_capacity(length / ENTRY_LEN);
        for chunk in body.chunks_exact(ENTRY_LEN) {
            let tva_id = u16::from_be_bytes([chunk[0], chunk[1]]);
            // Reserved(5) ignored on parse.
            let running_status = chunk[2] & RUNNING_STATUS_MAX;
            entries.push(TvaIdEntry {
                tva_id,
                running_status,
            });
        }
        Ok(Self { entries })
    }
}

impl Serialize for TvaIdDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + self.entries.len() * ENTRY_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        for e in &self.entries {
            if e.running_status > RUNNING_STATUS_MAX {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "running_status exceeds 3 bits",
                });
            }
        }
        if self.entries.len() * ENTRY_LEN > u8::MAX as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "TVA_id_descriptor body exceeds 255 bytes",
            });
        }
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = TAG;
        buf[1] = (self.entries.len() * ENTRY_LEN) as u8;
        let mut pos = HEADER_LEN;
        for e in &self.entries {
            buf[pos..pos + 2].copy_from_slice(&e.tva_id.to_be_bytes());
            // Reserved(5) emitted as 1s.
            buf[pos + 2] = 0xF8 | (e.running_status & RUNNING_STATUS_MAX);
            pos += ENTRY_LEN;
        }
        Ok(len)
    }
}

impl<'a> Descriptor<'a> for TvaIdDescriptor {
    const TAG: u8 = TAG;
    fn descriptor_length(&self) -> u8 {
        (self.entries.len() * ENTRY_LEN) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_entry() {
        // TVA_id=0x1234, running_status=4 (running), reserved bits set.
        let bytes = [TAG, 3, 0x12, 0x34, 0xFC];
        let d = TvaIdDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 1);
        assert_eq!(d.entries[0].tva_id, 0x1234);
        assert_eq!(d.entries[0].running_status, 4);
    }

    #[test]
    fn parse_multiple_entries() {
        let bytes = [TAG, 6, 0x00, 0x01, 0x01, 0xAB, 0xCD, 0x06];
        let d = TvaIdDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 2);
        assert_eq!(d.entries[0].tva_id, 0x0001);
        assert_eq!(d.entries[0].running_status, 1);
        assert_eq!(d.entries[1].tva_id, 0xABCD);
        assert_eq!(d.entries[1].running_status, 6);
    }

    #[test]
    fn parse_ignores_reserved_bits() {
        let bytes = [TAG, 3, 0x00, 0x00, 0xFF];
        let d = TvaIdDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries[0].running_status, 0x07);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        assert!(matches!(
            TvaIdDescriptor::parse(&[0x74, 0]).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x74, .. }
        ));
    }

    #[test]
    fn parse_rejects_length_not_multiple_of_3() {
        let bytes = [TAG, 2, 0, 0];
        assert!(matches!(
            TvaIdDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { .. }
        ));
    }

    #[test]
    fn empty_descriptor_valid() {
        let bytes = [TAG, 0];
        let d = TvaIdDescriptor::parse(&bytes).unwrap();
        assert!(d.entries.is_empty());
    }

    #[test]
    fn serialize_round_trip() {
        let d = TvaIdDescriptor {
            entries: vec![
                TvaIdEntry {
                    tva_id: 0x1000,
                    running_status: 2,
                },
                TvaIdEntry {
                    tva_id: 0xFFFF,
                    running_status: 0,
                },
            ],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(TvaIdDescriptor::parse(&buf).unwrap(), d);
    }

    #[test]
    fn serialize_rejects_running_status_over_range() {
        let d = TvaIdDescriptor {
            entries: vec![TvaIdEntry {
                tva_id: 0,
                running_status: 0x08,
            }],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::InvalidDescriptor { .. }
        ));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trip() {
        let d = TvaIdDescriptor {
            entries: vec![TvaIdEntry {
                tva_id: 0x4242,
                running_status: 4,
            }],
        };
        let j = serde_json::to_string(&d).unwrap();
        let back: TvaIdDescriptor = serde_json::from_str(&j).unwrap();
        assert_eq!(back, d);
    }
}
