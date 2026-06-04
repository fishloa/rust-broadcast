//! Bouquet Name Descriptor — ETSI EN 300 468 §6.2.6 (tag 0x47).

use crate::error::{Error, Result};
use crate::traits::Descriptor;
use dvb_common::{Parse, Serialize};

/// Wire tag for the Bouquet Name Descriptor.
pub const TAG: u8 = 0x47;

/// Header length (tag byte + length byte, no payload).
pub const HEADER_LEN: usize = 2;

/// Bouquet Name Descriptor (tag 0x47). Carries the human-readable name of
/// a bouquet in its BAT's `bouquet_descriptors_loop`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BouquetNameDescriptor<'a> {
    /// Raw DVB-encoded bouquet name bytes.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub bouquet_name: &'a [u8],
}

impl<'a> Parse<'a> for BouquetNameDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN,
                have: bytes.len(),
                what: "bouquet name descriptor header",
            });
        }

        let tag = bytes[0];
        if tag != TAG {
            return Err(Error::InvalidDescriptor {
                tag,
                reason: "expected tag 0x47",
            });
        }

        let length = bytes[1] as usize;
        let total = HEADER_LEN + length;

        if bytes.len() < total {
            return Err(Error::BufferTooShort {
                need: total,
                have: bytes.len(),
                what: "bouquet name descriptor payload",
            });
        }

        Ok(BouquetNameDescriptor {
            bouquet_name: &bytes[HEADER_LEN..total],
        })
    }
}

impl Serialize for BouquetNameDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + self.bouquet_name.len()
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
        buf[1] = self.bouquet_name.len() as u8;
        buf[HEADER_LEN..need].copy_from_slice(self.bouquet_name);

        Ok(need)
    }
}

impl<'a> Descriptor<'a> for BouquetNameDescriptor<'a> {
    const TAG: u8 = 0x47;

    fn descriptor_length(&self) -> u8 {
        self.bouquet_name.len() as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encode a 4-byte name, confirm parse extracts it.
    #[test]
    fn parse_extracts_bouquet_name() {
        let raw: Vec<u8> = vec![
            TAG, 0x04, // length = 4
            b'B', b'O', b'U', b'Q',
        ];
        let desc = BouquetNameDescriptor::parse(&raw).unwrap();
        assert_eq!(desc.bouquet_name, b"BOUQ");
    }

    /// Wrong tag byte should return InvalidDescriptor.
    #[test]
    fn parse_rejects_wrong_tag() {
        let raw: Vec<u8> = vec![
            0x48, // wrong tag
            0x04, b'B', b'O', b'U', b'Q',
        ];
        let err = BouquetNameDescriptor::parse(&raw).unwrap_err();
        assert!(
            matches!(err, Error::InvalidDescriptor { tag: 0x48, .. }),
            "expected InvalidDescriptor(tag=0x48), got {err:?}"
        );
    }

    /// Only the tag byte — too short for the 2-byte header.
    #[test]
    fn parse_rejects_buffer_shorter_than_header() {
        let raw: &[u8] = &[0x47];
        let err = BouquetNameDescriptor::parse(raw).unwrap_err();
        assert!(
            matches!(err, Error::BufferTooShort { need: 2, .. }),
            "expected BufferTooShort(need=2), got {err:?}"
        );
    }

    /// Length byte says 5 but only 2 bytes follow — mismatch.
    #[test]
    fn parse_rejects_length_overrunning_buffer() {
        let raw: Vec<u8> = vec![
            0x47, 0x05, // claims 5 bytes follow
            0xAA, 0xBB, // only 2 bytes available
        ];
        let err = BouquetNameDescriptor::parse(&raw).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    /// Descriptor with no bouquet name bytes is valid: `[0x47, 0x00]`.
    #[test]
    fn empty_bouquet_name_is_valid() {
        let raw: &[u8] = &[TAG, 0x00];
        let desc = BouquetNameDescriptor::parse(raw).unwrap();
        assert!(desc.bouquet_name.is_empty());
    }

    /// Parse → serialize → re-parse should yield an equal struct and
    /// identical bytes.
    #[test]
    fn serialize_round_trip_preserves_bytes() {
        let raw: Vec<u8> = vec![TAG, 0x04, b'B', b'O', b'U', b'Q'];
        let parsed = BouquetNameDescriptor::parse(&raw).unwrap();
        let mut buf = vec![0u8; parsed.serialized_len()];
        let written = parsed.serialize_into(&mut buf).unwrap();
        assert_eq!(written, parsed.serialized_len());

        let reparsed = BouquetNameDescriptor::parse(&buf).unwrap();
        assert_eq!(parsed, reparsed);
        assert_eq!(&raw, &buf[..]);
    }

    /// Serialise into a buffer smaller than `serialized_len()` must fail.
    #[test]
    fn serialize_rejects_too_small_buffer() {
        let raw: Vec<u8> = vec![TAG, 0x04, b'B', b'O', b'U', b'Q'];
        let parsed = BouquetNameDescriptor::parse(&raw).unwrap();
        let mut tiny = vec![0u8; 1];
        let err = parsed.serialize_into(&mut tiny).unwrap_err();
        assert!(
            matches!(err, Error::OutputBufferTooSmall { need, .. } if need == parsed.serialized_len()),
            "expected OutputBufferTooSmall(need={}), got {err:?}",
            parsed.serialized_len()
        );
    }
}
