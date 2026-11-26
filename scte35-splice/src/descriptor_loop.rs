//! A splice descriptor loop — the `descriptor()` sequence a clear
//! splice_info_section carries between `descriptor_loop_length` and the CRC
//! (ANSI/SCTE 35 2023r1 §10.1, Table 5).
//!
//! Zero-copy: the loop wraps the wire bytes (`raw()`) and keeps an unknown
//! tag's body verbatim, so a section whose loop carries a descriptor this
//! crate does not implement still round-trips byte-exactly. [`entries`] walks
//! the TLV structure without decoding a body; [`typed`] dispatches every entry
//! through [`AnySpliceDescriptor`](crate::descriptors::AnySpliceDescriptor).
//!
//! [`entries`]: SpliceDescriptorLoop::entries
//! [`typed`]: SpliceDescriptorLoop::typed

use crate::descriptors::{SpliceDescriptorIter, parse_loop};
use crate::error::Result;

/// Bytes of a splice descriptor header: `splice_descriptor_tag` (1) +
/// `descriptor_length` (1), §10.1.
pub const SPLICE_DESCRIPTOR_HEADER_LEN: usize = 2;

/// One `splice_descriptor()` entry: its `splice_descriptor_tag` and the
/// `descriptor_length` bytes of its body, verbatim (the `identifier` and
/// typed fields are what
/// [`AnySpliceDescriptor`](crate::descriptors::AnySpliceDescriptor) reads out
/// of `body`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpliceDescriptorEntry<'a> {
    /// `splice_descriptor_tag` (§10.1, Table 16).
    pub tag: u8,
    /// The descriptor body, `descriptor_length` bytes verbatim.
    pub body: &'a [u8],
}

/// A raw splice descriptor loop, borrowed from the section. Zero-copy: the
/// bytes a serializer writes back are [`SpliceDescriptorLoop::raw`], unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SpliceDescriptorLoop<'a>(&'a [u8]);

impl<'a> SpliceDescriptorLoop<'a> {
    /// Wrap a raw splice descriptor loop (the `descriptor()` bytes only, no
    /// enclosing `descriptor_loop_length`).
    #[must_use]
    pub const fn new(raw: &'a [u8]) -> Self {
        Self(raw)
    }

    /// The wire bytes of the loop, verbatim.
    #[must_use]
    pub const fn raw(&self) -> &'a [u8] {
        self.0
    }

    /// Byte length of the loop (a byte count, not an entry count).
    #[must_use]
    pub const fn len(&self) -> usize {
        self.0.len()
    }

    /// True when the loop carries no descriptor.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Walk the loop's `(tag, body)` pairs by the TLV structure only, never
    /// decoding a body. A truncated final header or body is reported as one
    /// `Err` (with the loop offset it starts at) and then iteration fuses;
    /// the bytes stay available through [`raw`](Self::raw).
    #[must_use]
    pub fn entries(&self) -> SpliceDescriptorEntryIter<'a> {
        SpliceDescriptorEntryIter {
            bytes: self.0,
            pos: 0,
            fused: false,
        }
    }

    /// Dispatch every entry through the typed registry, yielding one
    /// [`AnySpliceDescriptor`](crate::descriptors::AnySpliceDescriptor) per
    /// entry (`Unknown` with the raw body for a
    /// tag this crate does not implement).
    ///
    /// This is [`crate::descriptors::parse_loop`]; it is the same walk
    /// [`SpliceInfoSection::descriptors`](crate::section::SpliceInfoSection::descriptors)
    /// performs.
    #[must_use]
    pub fn typed(&self) -> SpliceDescriptorIter<'a> {
        parse_loop(self.0)
    }
}

impl<'a> From<&'a [u8]> for SpliceDescriptorLoop<'a> {
    fn from(raw: &'a [u8]) -> Self {
        Self(raw)
    }
}

impl core::ops::Deref for SpliceDescriptorLoop<'_> {
    /// Derefs to the raw wire bytes — `len()`/indexing are byte counts for
    /// serialization, not entry counts.
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        self.0
    }
}

impl<'a> IntoIterator for &SpliceDescriptorLoop<'a> {
    type Item = Result<SpliceDescriptorEntry<'a>>;
    type IntoIter = SpliceDescriptorEntryIter<'a>;
    fn into_iter(self) -> Self::IntoIter {
        self.entries()
    }
}

/// Iterator over a raw splice descriptor loop; see
/// [`SpliceDescriptorLoop::entries`].
#[derive(Debug, Clone)]
pub struct SpliceDescriptorEntryIter<'a> {
    bytes: &'a [u8],
    pos: usize,
    fused: bool,
}

impl<'a> Iterator for SpliceDescriptorEntryIter<'a> {
    type Item = Result<SpliceDescriptorEntry<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.fused || self.pos >= self.bytes.len() {
            return None;
        }
        let rem = &self.bytes[self.pos..];
        if rem.len() < SPLICE_DESCRIPTOR_HEADER_LEN {
            self.fused = true;
            return Some(Err(crate::error::Error::BufferTooShort(
                broadcast_core::error::BufferTooShort {
                    need: SPLICE_DESCRIPTOR_HEADER_LEN,
                    have: rem.len(),
                    what: "splice_descriptor header in loop",
                },
            )));
        }
        let tag = rem[0];
        let len = usize::from(rem[1]);
        let total = SPLICE_DESCRIPTOR_HEADER_LEN + len;
        if rem.len() < total {
            self.fused = true;
            return Some(Err(crate::error::Error::BufferTooShort(
                broadcast_core::error::BufferTooShort {
                    need: total,
                    have: rem.len(),
                    what: "splice_descriptor body in loop",
                },
            )));
        }
        self.pos += total;
        Some(Ok(SpliceDescriptorEntry {
            tag,
            body: &rem[SPLICE_DESCRIPTOR_HEADER_LEN..total],
        }))
    }
}

impl core::iter::FusedIterator for SpliceDescriptorEntryIter<'_> {}

#[cfg(feature = "serde")]
impl serde::Serialize for SpliceDescriptorLoop<'_> {
    /// Serializes as the typed walk: each `Ok(d)` becomes the
    /// [`AnySpliceDescriptor`](crate::descriptors::AnySpliceDescriptor), and a
    /// parse error becomes a `{"parseError": "<Display>"}` map, so a malformed
    /// entry is surfaced rather than silently dropped.
    fn serialize<S: serde::Serializer>(&self, s: S) -> core::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        struct Entry<'a>(crate::error::Result<crate::descriptors::AnySpliceDescriptor<'a>>);
        impl serde::Serialize for Entry<'_> {
            fn serialize<S: serde::Serializer>(
                &self,
                s: S,
            ) -> core::result::Result<S::Ok, S::Error> {
                match &self.0 {
                    Ok(d) => d.serialize(s),
                    Err(e) => {
                        use serde::ser::SerializeMap;
                        let mut m = s.serialize_map(Some(1))?;
                        m.serialize_entry("parseError", &alloc::format!("{e}"))?;
                        m.end()
                    }
                }
            }
        }
        let count = self.typed().count();
        let mut seq = s.serialize_seq(Some(count))?;
        for item in self.typed() {
            seq.serialize_element(&Entry(item))?;
        }
        seq.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_loop_has_no_entries() {
        let loop_ = SpliceDescriptorLoop::new(&[]);
        assert!(loop_.is_empty());
        assert_eq!(loop_.len(), 0);
        assert_eq!(loop_.entries().count(), 0);
    }

    /// The wire bytes a loop's entries name are exactly the loop's bytes:
    /// each entry is its tag byte, its length byte, then its body.
    #[test]
    fn entries_reproduce_the_loop_bytes() {
        let raw = [0x6Cu8, 0x02, 0x01, 0x00];
        let loop_ = SpliceDescriptorLoop::new(&raw);
        let mut rebuilt = alloc::vec::Vec::new();
        for entry in loop_.entries() {
            let entry = entry.expect("framed loop");
            rebuilt.push(entry.tag);
            rebuilt.push(entry.body.len() as u8);
            rebuilt.extend_from_slice(entry.body);
        }
        assert_eq!(rebuilt, raw);
    }

    #[test]
    fn a_truncated_final_body_is_reported() {
        let raw = [0x00u8, 0x04, 0x43];
        let err = SpliceDescriptorLoop::new(&raw)
            .entries()
            .last()
            .expect("one entry")
            .expect_err("the body runs past the loop");
        assert!(matches!(
            err,
            crate::error::Error::BufferTooShort(broadcast_core::error::BufferTooShort {
                need: 6,
                have: 3,
                ..
            })
        ));
    }
}
