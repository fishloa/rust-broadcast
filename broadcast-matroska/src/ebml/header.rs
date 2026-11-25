//! Element framing: the `(id, size, data)` triplet every EBML element is
//! (RFC 8794 §4–§6), and the size-driven walker over a bounded byte slice.
//!
//! [`ElementIter`] is schema-free, so it cannot resolve an unknown-size master
//! on its own: RFC 8794 §6.2 ends such an element at the next element the
//! schema makes a sibling, which needs [`matroska::ids`](crate::matroska::ids).
//! The Matroska walker in [`matroska`](crate::matroska) supplies that schema;
//! this iterator returns [`Error::UnknownSizeNotAllowed`] instead.

use crate::ebml::vint::{ElementId, VintSize};
use crate::error::{Error, Result};
use crate::limits::Limits;

/// An element's id and encoded size (RFC 8794 §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ElementHeader {
    id: ElementId,
    size: VintSize,
}

impl ElementHeader {
    /// Build a header from a parsed id and size.
    #[must_use]
    pub const fn new(id: ElementId, size: VintSize) -> Self {
        Self { id, size }
    }

    /// Build a known-size header at the size's narrowest width.
    pub fn with_payload_len(id: ElementId, payload_len: u64) -> Result<Self> {
        Ok(Self {
            id,
            size: VintSize::minimal(payload_len)?,
        })
    }

    /// Parse an id and a size from the front of `bytes`, returning the header
    /// and the octets consumed.
    pub fn parse(bytes: &[u8]) -> Result<(Self, usize)> {
        let (id, id_len) = ElementId::parse(bytes)?;
        let rest = bytes.get(id_len..).ok_or(Error::Truncated {
            what: "Element Data Size",
            need: id_len,
            have: bytes.len(),
        })?;
        let (size, size_len) = VintSize::parse(rest)?;
        Ok((Self { id, size }, id_len + size_len))
    }

    /// Parse a header and its payload from a slice that may continue past the
    /// element, returning the header, the payload and the octets the element
    /// occupies.
    pub fn split(bytes: &[u8]) -> Result<(Self, &[u8], usize)> {
        let (header, used) = Self::parse(bytes)?;
        let declared = header.payload_len().ok_or(Error::UnknownSizeNotAllowed {
            id: header.id().value(),
        })?;
        let declared = usize::try_from(declared).map_err(|_| Error::SizeOverflow)?;
        let end = used.checked_add(declared).ok_or(Error::SizeOverflow)?;
        let data = bytes.get(used..end).ok_or(Error::ElementOverruns {
            id: header.id().value(),
            declared: declared as u64,
            available: bytes.len().saturating_sub(used),
        })?;
        Ok((header, data, end))
    }

    /// The element's id.
    #[must_use]
    pub const fn id(&self) -> ElementId {
        self.id
    }

    /// The element's encoded size.
    #[must_use]
    pub const fn size(&self) -> VintSize {
        self.size
    }

    /// The known payload length, or `None` for an unknown-size master.
    #[must_use]
    pub const fn payload_len(&self) -> Option<u64> {
        self.size.value()
    }

    /// This header re-encoded for a new payload length, keeping the parsed
    /// width where it fits.
    pub fn resized(&self, payload_len: u64) -> Result<Self> {
        Ok(Self {
            id: self.id,
            size: self.size.resized(payload_len)?,
        })
    }

    /// The length of the encoded header.
    #[must_use]
    pub const fn serialized_len(&self) -> usize {
        self.id.octets() + self.size.serialized_len()
    }

    /// Write the header, returning the octets written.
    pub fn write(&self, out: &mut [u8]) -> Result<usize> {
        let id_len = self.id.write(out)?;
        let have = out.len();
        let rest = out.get_mut(id_len..).ok_or(Error::Truncated {
            what: "Element header output",
            need: id_len,
            have,
        })?;
        Ok(id_len + self.size.write(rest)?)
    }
}

impl<'a> broadcast_common::Parse<'a> for ElementHeader {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Ok(Self::parse(bytes)?.0)
    }
}

impl broadcast_common::Serialize for ElementHeader {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        Self::serialized_len(self)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        self.write(buf)
    }
}

/// One element as read: its header and its payload bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawElement<'a> {
    header: ElementHeader,
    data: &'a [u8],
}

impl<'a> RawElement<'a> {
    /// Build one from a parsed header and its payload.
    #[must_use]
    pub const fn new(header: ElementHeader, data: &'a [u8]) -> Self {
        Self { header, data }
    }

    /// The element's header.
    #[must_use]
    pub const fn header(&self) -> ElementHeader {
        self.header
    }

    /// The element's payload.
    #[must_use]
    pub const fn data(&self) -> &'a [u8] {
        self.data
    }
}

/// A forward walk over the elements in a bounded byte slice.
///
/// The slice is the parent's payload: the walker never reads past it, and an
/// element that declares more than the slice holds is a typed
/// [`Error::ElementOverruns`].
#[derive(Debug, Clone)]
pub struct ElementIter<'a> {
    bytes: &'a [u8],
    position: usize,
    limits: Limits,
}

impl<'a> ElementIter<'a> {
    /// Walk the elements in `bytes` under `limits`.
    #[must_use]
    pub const fn new(bytes: &'a [u8], limits: Limits) -> Self {
        Self {
            bytes,
            position: 0,
            limits,
        }
    }

    /// The walker's current offset inside the parent payload.
    #[must_use]
    pub const fn position(&self) -> usize {
        self.position
    }

    /// The bytes the walker has not consumed.
    #[must_use]
    pub fn remaining(&self) -> &'a [u8] {
        self.bytes.get(self.position..).unwrap_or(&[])
    }

    /// Read the next element.
    pub fn next_element(&mut self) -> Option<Result<RawElement<'a>>> {
        let rest = self.remaining();
        if rest.is_empty() {
            return None;
        }
        Some(self.read_element(rest))
    }

    /// Read one element from `rest` and advance past it.
    fn read_element(&mut self, rest: &'a [u8]) -> Result<RawElement<'a>> {
        let (header, header_len) = ElementHeader::parse(rest)?;
        let payload_len = header.payload_len().ok_or(Error::UnknownSizeNotAllowed {
            id: header.id().value(),
        })?;
        if payload_len > self.limits.max_element_bytes() {
            return Err(Error::LimitExceeded {
                limit: "max_element_bytes",
                value: payload_len,
            });
        }
        let available = rest.len().saturating_sub(header_len);
        let declared = usize::try_from(payload_len).unwrap_or(usize::MAX);
        if declared > available {
            return Err(Error::ElementOverruns {
                id: header.id().value(),
                declared: payload_len,
                available,
            });
        }
        let data = rest
            .get(header_len..header_len + declared)
            .ok_or(Error::ElementOverruns {
                id: header.id().value(),
                declared: payload_len,
                available,
            })?;
        self.position += header_len + declared;
        Ok(RawElement::new(header, data))
    }
}

impl<'a> Iterator for ElementIter<'a> {
    type Item = Result<RawElement<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ebml::vint::VintWidth;

    fn id(raw: u32) -> ElementId {
        ElementId::try_from_raw(raw).unwrap()
    }

    fn header(raw: u32, len: u64) -> Vec<u8> {
        let h = ElementHeader::with_payload_len(id(raw), len).unwrap();
        let mut out = vec![0u8; h.serialized_len()];
        h.write(&mut out).unwrap();
        out
    }

    #[test]
    fn header_round_trips() {
        let bytes = [0x18u8, 0x53, 0x80, 0x67, 0x01, 0, 0, 0, 0, 0x01, 0x67, 0x02];
        let (parsed, used) = ElementHeader::parse(&bytes).unwrap();
        assert_eq!(used, bytes.len());
        assert_eq!(parsed.id().value(), 0x1853_8067);
        assert_eq!(parsed.size().width(), VintWidth::MAX);
        let mut out = vec![0u8; parsed.serialized_len()];
        assert_eq!(parsed.write(&mut out).unwrap(), bytes.len());
        assert_eq!(out, bytes);
    }

    #[test]
    fn walker_reads_a_parent_sequentially() {
        let mut bytes = header(0x4286, 1);
        bytes.push(1);
        bytes.extend_from_slice(&header(0x4282, 4));
        bytes.extend_from_slice(b"webm");
        let mut walker = ElementIter::new(&bytes, Limits::new());
        let first = walker.next().unwrap().unwrap();
        assert_eq!(first.header().id().value(), 0x4286);
        assert_eq!(first.data(), &[1]);
        let second = walker.next().unwrap().unwrap();
        assert_eq!(second.header().id().value(), 0x4282);
        assert_eq!(second.data(), b"webm");
        assert!(walker.next().is_none());
    }

    #[test]
    fn walker_refuses_an_unknown_size() {
        let bytes = [0x18u8, 0x53, 0x80, 0x67, 0xFF];
        let mut walker = ElementIter::new(&bytes, Limits::new());
        assert_eq!(
            walker.next().unwrap(),
            Err(Error::UnknownSizeNotAllowed { id: 0x1853_8067 })
        );
    }

    #[test]
    fn walker_refuses_an_element_past_its_parent() {
        let bytes = [0x42u8, 0x86, 0x88, 0x00];
        let mut walker = ElementIter::new(&bytes, Limits::new());
        assert_eq!(
            walker.next().unwrap(),
            Err(Error::ElementOverruns {
                id: 0x4286,
                declared: 8,
                available: 1,
            })
        );
    }

    #[test]
    fn walker_honours_the_element_cap() {
        let bytes = [0x42u8, 0x86, 0x88, 0x00];
        let limits = Limits::new().with_max_element_bytes(4);
        let mut walker = ElementIter::new(&bytes, limits);
        assert_eq!(
            walker.next().unwrap(),
            Err(Error::LimitExceeded {
                limit: "max_element_bytes",
                value: 8,
            })
        );
    }
}
