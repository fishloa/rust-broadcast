//! A raw program-stream descriptor loop — the `descriptor()` sequence ISO/IEC
//! 13818-1 §2.5.4.1/§2.5.4.2 carries inside a Program Stream Map, and whose
//! generic form is §2.6 (Tables 2-42 through 2-45).
//!
//! Zero-copy: the loop wraps the wire bytes and keeps every non-conformant
//! byte verbatim, so a PSM whose descriptor bytes no known tag explains still
//! round-trips. Walk the TLV structure with [`DescriptorLoop::entries`], or
//! decode each entry through `dvb-si`'s descriptor registry with
//! [`DescriptorLoop::parse_with`] — the caller passes the parser, so mpeg-ps
//! keeps no dependency on `dvb-si` (`dvb-si` takes `mpeg-ts`, which takes
//! `dvb-si`; a direct edge would close the workspace cycle).

/// Bytes of a descriptor header: `descriptor_tag` (1) + `descriptor_length`
/// (1), ISO/IEC 13818-1 §2.6.
pub const DESCRIPTOR_HEADER_LEN: usize = 2;

/// A `descriptor()` entry: its `descriptor_tag`, the declared
/// `descriptor_length`, and the `descriptor_body` of that many bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DescriptorEntry<'a> {
    /// `descriptor_tag` (§2.6).
    pub tag: u8,
    /// `descriptor_body`, `descriptor_length` bytes verbatim.
    pub body: &'a [u8],
}

/// A raw `descriptor()` loop, borrowed from the PSM. Zero-copy: the bytes a
/// serializer writes back are [`DescriptorLoop::raw`], unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DescriptorLoop<'a>(&'a [u8]);

impl<'a> DescriptorLoop<'a> {
    /// Wrap a raw descriptor loop (the `descriptor()` bytes only, no
    /// enclosing length field).
    #[must_use]
    pub const fn new(raw: &'a [u8]) -> Self {
        Self(raw)
    }

    /// The wire bytes of the loop, verbatim.
    #[must_use]
    pub const fn raw(&self) -> &'a [u8] {
        self.0
    }

    /// Walk the loop's `(tag, body)` pairs by the TLV structure only — never
    /// decoding a body. An entry whose `descriptor_length` runs past the end
    /// of the loop is reported as `Err` carrying the loop offset it starts
    /// at, and iteration stops there; the bytes stay available through
    /// [`raw`](Self::raw).
    pub fn entries(&self) -> DescriptorEntryIter<'a> {
        DescriptorEntryIter {
            bytes: self.0,
            pos: 0,
            fused: false,
        }
    }

    /// True when the loop is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Byte length of the loop (a byte count, not an entry count).
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Walk the loop through `parse`'s decoder, handing every entry exactly
    /// the bytes [`entries`](Self::entries) frames for it: `parse` receives
    /// one entry's `(tag, body)` and returns whatever the caller's typed
    /// representation of it is. Nothing is decoded here, so an unknown tag is
    /// the caller's decision (e.g. keep the body verbatim).
    ///
    /// A truncated entry ends the walk after its `Err` (the same rule
    /// [`entries`](Self::entries) uses), so the decoder never sees a partial
    /// body.
    pub fn parse_with<T>(
        &self,
        parse: impl FnMut(u8, &'a [u8]) -> T,
    ) -> ParseWith<'a, impl FnMut(u8, &'a [u8]) -> T, T> {
        ParseWith {
            inner: self.entries(),
            parse,
            out: core::marker::PhantomData,
        }
    }
}

impl<'a> From<&'a [u8]> for DescriptorLoop<'a> {
    fn from(raw: &'a [u8]) -> Self {
        Self(raw)
    }
}

impl core::ops::Deref for DescriptorLoop<'_> {
    /// Derefs to the raw wire bytes — `len()`/indexing are byte counts for
    /// serialization, not entry counts.
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        self.0
    }
}

impl<'a> IntoIterator for &DescriptorLoop<'a> {
    type Item = crate::Result<DescriptorEntry<'a>>;
    type IntoIter = DescriptorEntryIter<'a>;
    fn into_iter(self) -> Self::IntoIter {
        self.entries()
    }
}

/// Iterator over a raw descriptor loop with the caller's decoder; see
/// [`DescriptorLoop::parse_with`].
pub struct ParseWith<'a, F, T> {
    inner: DescriptorEntryIter<'a>,
    parse: F,
    out: core::marker::PhantomData<fn(&'a [u8]) -> T>,
}

impl<'a, F, T> Iterator for ParseWith<'a, F, T>
where
    F: FnMut(u8, &'a [u8]) -> T,
{
    type Item = crate::Result<T>;

    fn next(&mut self) -> Option<Self::Item> {
        let entry = self.inner.next()?;
        Some(entry.map(|e| (self.parse)(e.tag, e.body)))
    }
}

/// Iterator over a raw descriptor loop; see [`DescriptorLoop::entries`].
#[derive(Debug, Clone)]
pub struct DescriptorEntryIter<'a> {
    bytes: &'a [u8],
    pos: usize,
    fused: bool,
}

impl<'a> Iterator for DescriptorEntryIter<'a> {
    type Item = crate::Result<DescriptorEntry<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.fused || self.pos >= self.bytes.len() {
            return None;
        }
        let rem = &self.bytes[self.pos..];
        if rem.len() < DESCRIPTOR_HEADER_LEN {
            self.fused = true;
            return Some(Err(crate::Error::DescriptorHeaderTruncated {
                offset: self.pos,
                need: DESCRIPTOR_HEADER_LEN,
                have: rem.len(),
            }));
        }
        let tag = rem[0];
        let len = usize::from(rem[1]);
        let total = DESCRIPTOR_HEADER_LEN + len;
        if rem.len() < total {
            self.fused = true;
            return Some(Err(crate::Error::DescriptorBodyTruncated {
                offset: self.pos,
                tag,
                declared: len,
                have: rem.len() - DESCRIPTOR_HEADER_LEN,
            }));
        }
        self.pos += total;
        let body = &rem[DESCRIPTOR_HEADER_LEN..total];
        Some(Ok(DescriptorEntry { tag, body }))
    }
}

impl core::iter::FusedIterator for DescriptorEntryIter<'_> {}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use super::*;

    #[test]
    fn an_empty_loop_has_no_entries() {
        let loop_ = DescriptorLoop::new(&[]);
        assert!(loop_.is_empty());
        assert_eq!(loop_.entries().count(), 0);
    }

    #[test]
    fn entries_carry_tag_and_body_verbatim() {
        // registration descriptor (0x05) then an unknown tag (0xC0).
        let raw = [0x05, 0x04, b'H', b'D', b'M', b'V', 0xC0, 0x01, 0x7F];
        let loop_ = DescriptorLoop::new(&raw);
        let entries: Vec<_> = loop_.entries().map(|e| e.unwrap()).collect();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].tag, 0x05);
        assert_eq!(entries[0].body, b"HDMV");
        assert_eq!(entries[1].tag, 0xC0);
        assert_eq!(entries[1].body, &[0x7F]);
        assert_eq!(loop_.raw(), &raw[..]);
    }

    #[test]
    fn a_truncated_header_is_reported_with_its_offset() {
        let raw = [0x05, 0x01, 0x00, 0x06];
        let err = DescriptorLoop::new(&raw)
            .entries()
            .last()
            .unwrap()
            .unwrap_err();
        assert_eq!(
            err,
            crate::Error::DescriptorHeaderTruncated {
                offset: 3,
                need: DESCRIPTOR_HEADER_LEN,
                have: 1,
            }
        );
    }

    #[test]
    fn a_body_running_past_the_loop_is_reported_not_shortened() {
        let raw = [0x05, 0x06, b'H', b'D'];
        let err = DescriptorLoop::new(&raw)
            .entries()
            .last()
            .unwrap()
            .unwrap_err();
        assert_eq!(
            err,
            crate::Error::DescriptorBodyTruncated {
                offset: 0,
                tag: 0x05,
                declared: 6,
                have: 2,
            }
        );
    }

    #[test]
    fn parse_with_hands_each_entry_its_own_classification() {
        // registration descriptor (tag 0x05, "HDMV") then an unknown tag.
        let raw = [0x05u8, 0x04, b'H', b'D', b'M', b'V', 0xC0, 0x01, 0x7F];
        let kinds: Vec<_> = DescriptorLoop::new(&raw)
            .parse_with(|tag, _| match tag {
                0x05 => "registration",
                0xC0 => "unknown",
                _ => "other",
            })
            .map(|k| k.expect("framed loop"))
            .collect();
        assert_eq!(kinds, ["registration", "unknown"]);

        let bodies: Vec<&[u8]> = DescriptorLoop::new(&raw)
            .parse_with(|_, body| body)
            .map(|b| b.expect("framed loop"))
            .collect();
        assert_eq!(bodies, [&b"HDMV"[..], &[0x7F][..]]);
    }
}
