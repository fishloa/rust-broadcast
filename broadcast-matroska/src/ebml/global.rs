//! The two global elements every EBML document type may carry: `Void`
//! (RFC 8794 §11.3.2) and `CRC-32` (§11.3.1).

use broadcast_common::{Parse, Serialize};

use crate::ebml::ElementHeader;
use crate::error::{Error, Result};

/// Element ID of the global `CRC-32` element (RFC 8794 §11.3.1).
pub const CRC32_ID: u32 = 0xBF;
/// Element ID of the global `Void` element (RFC 8794 §11.3.2).
pub const VOID_ID: u32 = 0xEC;
/// Octets in a `CRC-32` payload.
pub const CRC32_OCTETS: usize = 4;

/// A `CRC-32` element's payload, kept rather than verified: this crate checks
/// syntax, and a caller that wants integrity verification recomputes the
/// enclosing master's CRC. RFC 8794 §11.3.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Crc32 {
    header: ElementHeader,
    value: u32,
}

impl Crc32 {
    /// Build from a parsed header and value.
    pub fn new(header: ElementHeader, value: u32) -> Result<Self> {
        if header.id().value() != CRC32_ID {
            return Err(Error::UnexpectedElement {
                expected: CRC32_ID,
                found: header.id().value(),
            });
        }
        Ok(Self { header, value })
    }

    /// Build from a parsed header and its four-octet payload.
    pub fn from_payload(header: ElementHeader, payload: &[u8]) -> Result<Self> {
        if header.id().value() != CRC32_ID {
            return Err(Error::UnexpectedElement {
                expected: CRC32_ID,
                found: header.id().value(),
            });
        }
        if payload.len() != CRC32_OCTETS {
            return Err(Error::InvalidField(broadcast_common::error::InvalidField {
                field: "CRC-32",
                reason: "payload is not four octets",
            }));
        }
        let raw: [u8; CRC32_OCTETS] = payload.try_into().map_err(|_| Error::IntegerWidth {
            width: payload.len(),
        })?;
        Ok(Self {
            header,
            value: u32::from_be_bytes(raw),
        })
    }

    /// The element header.
    #[must_use]
    pub const fn header(&self) -> ElementHeader {
        self.header
    }

    /// The stored CRC-32 value.
    #[must_use]
    pub const fn value(&self) -> u32 {
        self.value
    }
}

impl<'a> Parse<'a> for Crc32 {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (header, payload, _) = ElementHeader::split(bytes)?;
        Self::from_payload(header, payload)
    }
}

impl Serialize for Crc32 {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        self.header.serialized_len() + CRC32_OCTETS
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let header = self.header.resized(CRC32_OCTETS as u64)?;
        let used = header.write(buf)?;
        let have = buf.len();
        let target = buf
            .get_mut(used..used + CRC32_OCTETS)
            .ok_or(Error::OutputBufferTooSmall(
                broadcast_common::error::OutputBufferTooSmall {
                    need: used + CRC32_OCTETS,
                    have,
                },
            ))?;
        target.copy_from_slice(&self.value.to_be_bytes());
        Ok(used + CRC32_OCTETS)
    }
}

/// A `Void` element: padding whose bytes are preserved, never interpreted
/// (RFC 8794 §11.3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Void<'a> {
    header: ElementHeader,
    padding: &'a [u8],
}

impl<'a> Void<'a> {
    /// Build from a parsed header and payload.
    pub fn new(header: ElementHeader, padding: &'a [u8]) -> Result<Self> {
        if header.id().value() != VOID_ID {
            return Err(Error::UnexpectedElement {
                expected: VOID_ID,
                found: header.id().value(),
            });
        }
        Ok(Self { header, padding })
    }

    /// The element header.
    #[must_use]
    pub const fn header(&self) -> ElementHeader {
        self.header
    }

    /// The padding bytes.
    #[must_use]
    pub const fn padding(&self) -> &'a [u8] {
        self.padding
    }
}

impl<'a> Parse<'a> for Void<'a> {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (header, padding, _) = ElementHeader::split(bytes)?;
        Self::new(header, padding)
    }
}

impl Serialize for Void<'_> {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        self.header.serialized_len() + self.padding.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = u64::try_from(self.padding.len()).map_err(|_| Error::SizeOverflow)?;
        let header = self.header.resized(len)?;
        let used = header.write(buf)?;
        let have = buf.len();
        let target =
            buf.get_mut(used..used + self.padding.len())
                .ok_or(Error::OutputBufferTooSmall(
                    broadcast_common::error::OutputBufferTooSmall {
                        need: used + self.padding.len(),
                        have,
                    },
                ))?;
        target.copy_from_slice(self.padding);
        Ok(used + self.padding.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ebml::ElementId;

    #[test]
    fn void_round_trips() {
        let bytes = [0xECu8, 0x83, 0x00, 0x00, 0x00];
        let parsed = Void::parse(&bytes).unwrap();
        assert_eq!(parsed.padding(), &[0, 0, 0]);
        assert_eq!(parsed.try_to_bytes().unwrap(), bytes);

        let id = ElementId::try_from_raw(VOID_ID).unwrap();
        let header = ElementHeader::with_payload_len(id, 0).unwrap();
        let empty = Void::new(header, &[]).unwrap();
        assert_eq!(empty.try_to_bytes().unwrap(), [0xEC, 0x80]);
    }

    #[test]
    fn crc32_round_trips() {
        let bytes = [0xBFu8, 0x84, 0xDE, 0xAD, 0xBE, 0xEF];
        let parsed = Crc32::parse(&bytes).unwrap();
        assert_eq!(parsed.value(), 0xDEAD_BEEF);
        assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
    }

    #[test]
    fn wrong_id_is_refused() {
        assert_eq!(
            Crc32::parse(&[0xECu8, 0x80]),
            Err(Error::UnexpectedElement {
                expected: CRC32_ID,
                found: VOID_ID,
            })
        );
    }
}
