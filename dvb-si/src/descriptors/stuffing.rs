//! Stuffing Descriptor — ETSI EN 300 468 §6.2.41 (tag 0x42).
//!
//! Table 98 (PDF p. 105). A descriptor whose body is `N` arbitrary
//! `stuffing_byte`s of any value. Used to pad descriptor loops; carries no
//! semantic information. The bytes are kept verbatim as a borrowed slice.

use crate::error::{Error, Result};
use crate::traits::Descriptor;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for stuffing_descriptor.
pub const TAG: u8 = 0x42;
const HEADER_LEN: usize = 2;
/// Maximum body length expressible in the 8-bit `descriptor_length` field.
const MAX_BODY_LEN: usize = u8::MAX as usize;

/// Stuffing Descriptor.
///
/// EN 300 468 §6.2.41: `stuffing_byte` values are unconstrained ("This is an
/// 8-bit field whose value is not specified"), so the payload is stored as an
/// opaque borrowed slice and round-trips byte-for-byte.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct StuffingDescriptor<'a> {
    /// Arbitrary stuffing bytes (any value), in wire order.
    pub stuffing_bytes: &'a [u8],
}

impl<'a> Parse<'a> for StuffingDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN,
                have: bytes.len(),
                what: "StuffingDescriptor header",
            });
        }
        if bytes[0] != TAG {
            return Err(Error::InvalidDescriptor {
                tag: bytes[0],
                reason: "unexpected tag for stuffing_descriptor",
            });
        }
        let length = bytes[1] as usize;
        let end = HEADER_LEN + length;
        if bytes.len() < end {
            return Err(Error::BufferTooShort {
                need: end,
                have: bytes.len(),
                what: "StuffingDescriptor body",
            });
        }
        Ok(Self {
            stuffing_bytes: &bytes[HEADER_LEN..end],
        })
    }
}

impl Serialize for StuffingDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + self.stuffing_bytes.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        // 8-bit descriptor_length field: error rather than silently truncate.
        if self.stuffing_bytes.len() > MAX_BODY_LEN {
            return Err(Error::SectionLengthOverflow {
                declared: self.stuffing_bytes.len(),
                available: MAX_BODY_LEN,
            });
        }
        buf[0] = TAG;
        buf[1] = self.stuffing_bytes.len() as u8;
        buf[HEADER_LEN..len].copy_from_slice(self.stuffing_bytes);
        Ok(len)
    }
}

impl<'a> Descriptor<'a> for StuffingDescriptor<'a> {
    const TAG: u8 = TAG;
    fn descriptor_length(&self) -> u8 {
        self.stuffing_bytes.len() as u8
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for StuffingDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "STUFFING";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_arbitrary_bytes() {
        let bytes = [TAG, 4, 0x00, 0xFF, 0xAB, 0x7E];
        let d = StuffingDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.stuffing_bytes, &[0x00, 0xFF, 0xAB, 0x7E]);
    }

    #[test]
    fn parse_empty_body_valid() {
        let bytes = [TAG, 0];
        let d = StuffingDescriptor::parse(&bytes).unwrap();
        assert!(d.stuffing_bytes.is_empty());
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        assert!(matches!(
            StuffingDescriptor::parse(&[0x43, 0]).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x43, .. }
        ));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        // declared length 4 but only 2 body bytes present
        let bytes = [TAG, 4, 0x00, 0xFF];
        assert!(matches!(
            StuffingDescriptor::parse(&bytes).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn serialize_round_trip() {
        let d = StuffingDescriptor {
            stuffing_bytes: &[0xDE, 0xAD, 0xBE, 0xEF],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(StuffingDescriptor::parse(&buf).unwrap(), d);
    }

    #[test]
    fn serialize_rejects_small_buffer() {
        let d = StuffingDescriptor {
            stuffing_bytes: &[0x01, 0x02],
        };
        let mut tiny = [0u8; 3];
        assert!(matches!(
            d.serialize_into(&mut tiny).unwrap_err(),
            Error::OutputBufferTooSmall { .. }
        ));
    }

    #[test]
    fn serialize_rejects_over_range_body() {
        let big = vec![0u8; 256];
        let d = StuffingDescriptor {
            stuffing_bytes: &big,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::SectionLengthOverflow { .. }
        ));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_serialize_stable() {
        // Borrowed-byte fields cannot deserialize from a JSON array (serde_json
        // requires a borrowed-str for &[u8]); assert the Serialize half is
        // stable and captures the payload, matching how the other borrowed
        // descriptors (e.g. content_identifier) handle serde.
        let d = StuffingDescriptor {
            stuffing_bytes: &[0x11, 0x22, 0x33],
        };
        let json = serde_json::to_string(&d).unwrap();
        assert!(json.contains("17") && json.contains("34") && json.contains("51"));
        // Re-serializing an equal value yields identical JSON (no nondeterminism).
        let d2 = StuffingDescriptor {
            stuffing_bytes: &[0x11, 0x22, 0x33],
        };
        assert_eq!(json, serde_json::to_string(&d2).unwrap());
    }
}
