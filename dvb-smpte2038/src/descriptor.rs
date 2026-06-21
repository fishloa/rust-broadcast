//! `anc_data_descriptor` (tag `0xC4`) + the `"VANC"` registration_descriptor
//! `format_identifier` — SMPTE ST 2038:2021 §4.1 (Table 1, p. 4).
//!
//! The ANC data stream is signalled in the PMT with `stream_type == 0x06`
//! (PES private data, §4.1.1), preceded by an ISO/IEC 13818-1
//! `registration_descriptor` carrying `format_identifier == 0x56414E43`
//! (`"VANC"`, §4.1.3), and accompanied by the `anc_data_descriptor` (tag
//! `0xC4`, §4.1.2) whose body is an opaque descriptor loop currently undefined
//! by the spec ("Compliant receive devices shall ignore unrecognized
//! descriptors", §4.1.2.1).

use crate::error::{Error, Result};

/// `stream_type` for the ANC data ES in the PMT — `0x06` (PES private data),
/// ST 2038 §4.1.1.
pub const ANC_STREAM_TYPE: u8 = 0x06;

/// `anc_data_descriptor` tag — `0xC4` (user-defined in ATSC/DVB/SCTE),
/// ST 2038 §4.1.2.
pub const ANC_DATA_DESCRIPTOR_TAG: u8 = 0xC4;

/// `registration_descriptor` `format_identifier` for ST 2038 — `0x56414E43`
/// (the ASCII `"VANC"`), ST 2038 §4.1.3.
pub const VANC_FORMAT_IDENTIFIER: u32 = 0x5641_4E43;

/// The four ASCII bytes of [`VANC_FORMAT_IDENTIFIER`] (`b"VANC"`).
pub const VANC_FORMAT_IDENTIFIER_BYTES: [u8; 4] = *b"VANC";

/// Minimum wire size: `descriptor_tag` + `descriptor_length`.
const HEADER_LEN: usize = 2;

/// The `anc_data_descriptor` (Table 1): a tag/length wrapper around an opaque
/// inner descriptor loop. The `descriptor_length` body is retained verbatim in
/// [`inner_descriptors`](Self::inner_descriptors) — its content is currently
/// undefined by ST 2038 and parsed lazily by the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AncDataDescriptor<'a> {
    /// The `descriptor()` loop body (`descriptor_length` bytes), undefined by
    /// ST 2038 §4.1.2.1 and ignored by compliant receivers.
    #[cfg_attr(feature = "serde", serde(borrow, with = "serde_bytes_compat"))]
    pub inner_descriptors: &'a [u8],
}

#[cfg(feature = "serde")]
mod serde_bytes_compat {
    pub fn serialize<S: serde::Serializer>(b: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bytes(b)
    }
}

impl<'a> AncDataDescriptor<'a> {
    /// Parse an `anc_data_descriptor` from the bytes starting at its
    /// `descriptor_tag`.
    ///
    /// # Errors
    /// [`Error::BufferTooShort`] if truncated; [`Error::BadDescriptorTag`] if
    /// the tag byte is not [`ANC_DATA_DESCRIPTOR_TAG`].
    pub fn parse(b: &'a [u8]) -> Result<Self> {
        if b.len() < HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN,
                have: b.len(),
                what: "anc_data_descriptor header",
            });
        }
        if b[0] != ANC_DATA_DESCRIPTOR_TAG {
            return Err(Error::BadDescriptorTag(b[0]));
        }
        let len = usize::from(b[1]);
        let end = HEADER_LEN + len;
        if b.len() < end {
            return Err(Error::BufferTooShort {
                need: end,
                have: b.len(),
                what: "anc_data_descriptor body",
            });
        }
        Ok(Self {
            inner_descriptors: &b[HEADER_LEN..end],
        })
    }

    /// Serialized length in bytes (tag + length + body).
    #[must_use]
    pub fn serialized_len(&self) -> usize {
        HEADER_LEN + self.inner_descriptors.len()
    }

    /// Serialize back to bytes.
    ///
    /// # Errors
    /// [`Error::BufferTooShort`] if `buf` is too small; [`Error::FieldTooWide`]
    /// if the body exceeds the 8-bit `descriptor_length`.
    pub fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::BufferTooShort {
                need: len,
                have: buf.len(),
                what: "anc_data_descriptor serialize output",
            });
        }
        if self.inner_descriptors.len() > 0xFF {
            return Err(Error::FieldTooWide {
                what: "descriptor_length",
                value: self.inner_descriptors.len() as u32,
                bits: 8,
            });
        }
        buf[0] = ANC_DATA_DESCRIPTOR_TAG;
        buf[1] = self.inner_descriptors.len() as u8;
        buf[HEADER_LEN..len].copy_from_slice(self.inner_descriptors);
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate alloc;
    use alloc::vec;

    #[test]
    fn vanc_identifier_is_ascii() {
        assert_eq!(VANC_FORMAT_IDENTIFIER.to_be_bytes(), *b"VANC");
        assert_eq!(VANC_FORMAT_IDENTIFIER_BYTES, *b"VANC");
    }

    #[test]
    fn round_trip_empty_body() {
        let b = [ANC_DATA_DESCRIPTOR_TAG, 0x00];
        let d = AncDataDescriptor::parse(&b).unwrap();
        assert!(d.inner_descriptors.is_empty());
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, b);
    }

    #[test]
    fn round_trip_with_body() {
        let b = [ANC_DATA_DESCRIPTOR_TAG, 0x03, 0xAA, 0xBB, 0xCC];
        let d = AncDataDescriptor::parse(&b).unwrap();
        assert_eq!(d.inner_descriptors, &[0xAA, 0xBB, 0xCC]);
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, b);
    }

    #[test]
    fn rejects_bad_tag() {
        let b = [0xC5, 0x00];
        assert!(matches!(
            AncDataDescriptor::parse(&b),
            Err(Error::BadDescriptorTag(0xC5))
        ));
    }

    #[test]
    fn rejects_truncated_body() {
        let b = [ANC_DATA_DESCRIPTOR_TAG, 0x05, 0x01];
        assert!(matches!(
            AncDataDescriptor::parse(&b),
            Err(Error::BufferTooShort { .. })
        ));
    }
}
