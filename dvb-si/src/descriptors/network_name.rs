//! Network Name Descriptor — ETSI EN 300 468 §6.2.28 (tag 0x40).

use crate::error::{Error, Result};
use crate::traits::Descriptor;
use dvb_common::{Parse, Serialize};

/// Wire tag for the Network Name Descriptor.
pub const TAG: u8 = 0x40;

/// Header length (tag byte + length byte, no payload).
pub const HEADER_LEN: usize = 2;

/// Network Name Descriptor (tag 0x40). Carries the human-readable name of
/// a DVB network in its NIT's `network_descriptors_loop`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct NetworkNameDescriptor<'a> {
    /// Raw DVB-encoded network name bytes. Decoding via `crate::text` lands
    /// in a later phase; keep the bytes borrowed for now.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub network_name: &'a [u8],
}

impl<'a> Parse<'a> for NetworkNameDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN,
                have: bytes.len(),
                what: "network name descriptor header",
            });
        }

        let tag = bytes[0];
        if tag != TAG {
            return Err(Error::InvalidDescriptor {
                tag,
                reason: "expected tag 0x40",
            });
        }

        let length = bytes[1] as usize;
        let total = HEADER_LEN + length;

        if bytes.len() < total {
            return Err(Error::BufferTooShort {
                need: total,
                have: bytes.len(),
                what: "network name descriptor payload",
            });
        }

        Ok(NetworkNameDescriptor {
            network_name: &bytes[HEADER_LEN..total],
        })
    }
}

impl Serialize for NetworkNameDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + self.network_name.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }

        buf[0] = TAG;
        buf[1] = self.network_name.len() as u8;
        buf[HEADER_LEN..need].copy_from_slice(self.network_name);

        Ok(need)
    }
}

impl<'a> Descriptor<'a> for NetworkNameDescriptor<'a> {
    const TAG: u8 = 0x40;

    fn descriptor_length(&self) -> u8 {
        self.network_name.len() as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encode a 4-byte name, confirm parse extracts it.
    #[test]
    fn parse_extracts_network_name() {
        let raw: Vec<u8> = vec![
            TAG, 0x04, // length = 4
            b'E', b'U', b'T', b'E',
        ];
        let desc = NetworkNameDescriptor::parse(&raw).unwrap();
        assert_eq!(desc.network_name, b"EUTE");
    }

    /// Wrong tag byte should return InvalidDescriptor.
    #[test]
    fn parse_rejects_wrong_tag() {
        let raw: Vec<u8> = vec![
            0x41, // wrong tag
            0x04, b'E', b'U', b'T', b'E',
        ];
        let err = NetworkNameDescriptor::parse(&raw).unwrap_err();
        assert!(
            matches!(err, Error::InvalidDescriptor { tag: 0x41, .. }),
            "expected InvalidDescriptor(tag=0x41), got {err:?}"
        );
    }

    /// Only the tag byte — too short for the 2-byte header.
    #[test]
    fn parse_rejects_buffer_shorter_than_header() {
        let raw: &[u8] = &[0x40];
        let err = NetworkNameDescriptor::parse(raw).unwrap_err();
        assert!(
            matches!(err, Error::BufferTooShort { need: 2, .. }),
            "expected BufferTooShort(need=2), got {err:?}"
        );
    }

    /// Length byte says 5 but only 2 bytes follow — mismatch.
    #[test]
    fn parse_rejects_length_byte_overflowing_buffer() {
        let raw: Vec<u8> = vec![
            0x40, 0x05, // claims 5 bytes follow
            0xAA, 0xBB, // only 2 bytes available
        ];
        let err = NetworkNameDescriptor::parse(&raw).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    /// Parse → serialize → re-parse should yield an equal struct and
    /// identical bytes.
    #[test]
    fn serialize_round_trip_preserves_bytes() {
        let raw: Vec<u8> = vec![TAG, 0x04, b'E', b'U', b'T', b'E'];
        let parsed = NetworkNameDescriptor::parse(&raw).unwrap();
        let mut buf = vec![0u8; parsed.serialized_len()];
        let written = parsed.serialize_into(&mut buf).unwrap();
        assert_eq!(written, parsed.serialized_len());

        let reparsed = NetworkNameDescriptor::parse(&buf).unwrap();
        assert_eq!(parsed, reparsed);
        assert_eq!(&raw, &buf[..]);
    }

    /// Serialise into a buffer smaller than `serialized_len()` must fail.
    #[test]
    fn serialize_rejects_too_small_buffer() {
        let raw: Vec<u8> = vec![TAG, 0x04, b'E', b'U', b'T', b'E'];
        let parsed = NetworkNameDescriptor::parse(&raw).unwrap();
        let mut tiny = vec![0u8; 1];
        let err = parsed.serialize_into(&mut tiny).unwrap_err();
        assert!(
            matches!(err, Error::OutputBufferTooSmall { need, .. } if need == parsed.serialized_len()),
            "expected OutputBufferTooSmall(need={}), got {err:?}",
            parsed.serialized_len()
        );
    }

    /// Descriptor with no network name bytes is valid: `[0x40, 0x00]`.
    #[test]
    fn empty_network_name_is_valid() {
        let raw: &[u8] = &[TAG, 0x00];
        let desc = NetworkNameDescriptor::parse(raw).unwrap();
        assert!(desc.network_name.is_empty());
    }

    /// `descriptor_length()` must equal `network_name.len()` cast to u8.
    #[test]
    fn descriptor_length_getter_matches_payload() {
        let raw: Vec<u8> = vec![TAG, 0x07, b'F', b'R', b'A', b'N', b'C', b'E', b'2'];
        let desc = NetworkNameDescriptor::parse(&raw).unwrap();
        assert_eq!(desc.descriptor_length(), desc.network_name.len() as u8);
    }
}
