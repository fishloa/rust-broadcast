//! One element carrying one typed scalar payload.
//!
//! A leaf keeps its [`ElementHeader`] so the encoded size width survives a
//! re-serialize; the payload is the typed scalar from
//! [`scalar`](crate::ebml::scalar), never a raw blob (`Binary` is the one
//! genuinely opaque type, RFC 8794 §7.8).

use broadcast_common::{Parse, Serialize};

use crate::ebml::ElementHeader;
use crate::ebml::scalar::{Binary, Date, Float, Signed, Text, Unsigned};
use crate::error::{Error, Result};

/// Build the shared impls for a leaf whose payload has no lifetime.
macro_rules! leaf {
    ($ty:ident, $payload:ty, $name:literal) => {
        #[doc = concat!("An element whose payload is a ", $name, ".")]
        #[derive(Debug, Clone, Copy, PartialEq)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize))]
        pub struct $ty {
            header: ElementHeader,
            value: $payload,
        }

        impl $ty {
            /// Build one from a parsed header and payload.
            #[must_use]
            pub const fn new(header: ElementHeader, value: $payload) -> Self {
                Self { header, value }
            }

            /// The element header.
            #[must_use]
            pub const fn header(&self) -> ElementHeader {
                self.header
            }

            /// The typed payload.
            #[must_use]
            pub const fn value(&self) -> $payload {
                self.value
            }
        }

        impl<'d> Parse<'d> for $ty {
            type Error = Error;

            fn parse(bytes: &'d [u8]) -> Result<Self> {
                let (header, data, _) = ElementHeader::split(bytes)?;
                Ok(Self::new(header, <$payload>::parse(data)?))
            }
        }

        impl Serialize for $ty {
            type Error = Error;

            fn serialized_len(&self) -> usize {
                self.header.serialized_len() + self.value.serialized_len()
            }

            fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
                let payload_len =
                    u64::try_from(self.value.serialized_len()).map_err(|_| Error::SizeOverflow)?;
                let header = self.header.resized(payload_len)?;
                let used = header.write(buf)?;
                let mut written = 0;
                if let Some(rest) = buf.get_mut(used..) {
                    written = self.value.serialize_into(rest)?;
                }
                Ok(used + written)
            }
        }
    };
}

/// Build the shared impls for a leaf whose payload borrows.
macro_rules! leaf_borrowed {
    ($ty:ident, $payload:ty, $name:literal) => {
        #[doc = concat!("An element whose payload is a ", $name, ".")]
        #[derive(Debug, Clone, Copy, PartialEq)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize))]
        pub struct $ty<'a> {
            header: ElementHeader,
            value: $payload,
        }

        impl<'a> $ty<'a> {
            /// Build one from a parsed header and payload.
            #[must_use]
            pub const fn new(header: ElementHeader, value: $payload) -> Self {
                Self { header, value }
            }

            /// The element header.
            #[must_use]
            pub const fn header(&self) -> ElementHeader {
                self.header
            }

            /// The typed payload.
            #[must_use]
            pub const fn value(&self) -> $payload {
                self.value
            }
        }

        impl<'a> Parse<'a> for $ty<'a> {
            type Error = Error;

            fn parse(bytes: &'a [u8]) -> Result<Self> {
                let (header, data, _) = ElementHeader::split(bytes)?;
                Ok(Self::new(header, <$payload>::parse(data)?))
            }
        }

        impl Serialize for $ty<'_> {
            type Error = Error;

            fn serialized_len(&self) -> usize {
                self.header.serialized_len() + self.value.serialized_len()
            }

            fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
                let payload_len =
                    u64::try_from(self.value.serialized_len()).map_err(|_| Error::SizeOverflow)?;
                let header = self.header.resized(payload_len)?;
                let used = header.write(buf)?;
                let mut written = 0;
                if let Some(rest) = buf.get_mut(used..) {
                    written = self.value.serialize_into(rest)?;
                }
                Ok(used + written)
            }
        }
    };
}

leaf!(
    UnsignedElement,
    Unsigned,
    "unsigned integer (RFC 8794 §7.2)"
);
leaf!(SignedElement, Signed, "signed integer (RFC 8794 §7.1)");
leaf!(FloatElement, Float, "float (RFC 8794 §7.3)");
leaf!(DateElement, Date, "date (RFC 8794 §7.6)");
leaf_borrowed!(
    TextElement,
    Text<'a>,
    "string or UTF-8 text (RFC 8794 §7.4, §7.5)"
);
leaf_borrowed!(BinaryElement, Binary<'a>, "binary payload (RFC 8794 §7.8)");

/// An element whose id this crate does not name: its header and bytes are
/// preserved and serialise back unchanged.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Unknown<'a> {
    header: ElementHeader,
    payload: &'a [u8],
}

impl<'a> Unknown<'a> {
    /// Build one from a parsed header and payload.
    #[must_use]
    pub const fn new(header: ElementHeader, payload: &'a [u8]) -> Self {
        Self { header, payload }
    }

    /// The element header.
    #[must_use]
    pub const fn header(&self) -> ElementHeader {
        self.header
    }

    /// The uninterpreted payload.
    #[must_use]
    pub const fn payload(&self) -> &'a [u8] {
        self.payload
    }
}

impl<'a> Parse<'a> for Unknown<'a> {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (header, payload, _) = ElementHeader::split(bytes)?;
        Ok(Self::new(header, payload))
    }
}

impl Serialize for Unknown<'_> {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        self.header.serialized_len() + self.payload.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let payload_len = u64::try_from(self.payload.len()).map_err(|_| Error::SizeOverflow)?;
        let header = self.header.resized(payload_len)?;
        let used = header.write(buf)?;
        let have = buf.len();
        let target =
            buf.get_mut(used..used + self.payload.len())
                .ok_or(Error::OutputBufferTooSmall(
                    broadcast_common::error::OutputBufferTooSmall {
                        need: used + self.payload.len(),
                        have,
                    },
                ))?;
        target.copy_from_slice(self.payload);
        Ok(used + self.payload.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ebml::ElementId;

    #[test]
    fn unsigned_leaf_round_trips_at_its_width() {
        let bytes = [0x2Au8, 0xD7, 0xB1, 0x83, 0x0F, 0x42, 0x40];
        let parsed = UnsignedElement::parse(&bytes).unwrap();
        assert_eq!(parsed.value().value(), 1_000_000);
        assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
    }

    #[test]
    fn unknown_element_round_trips() {
        let id = ElementId::try_from_raw(0x4D80).unwrap();
        let header = ElementHeader::with_payload_len(id, 3).unwrap();
        let element = Unknown::new(header, b"abc");
        let bytes = element.try_to_bytes().unwrap();
        assert_eq!(bytes, [0x4D, 0x80, 0x83, b'a', b'b', b'c']);
        assert_eq!(Unknown::parse(&bytes).unwrap(), element);
    }
}
