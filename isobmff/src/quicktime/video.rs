//! QuickTime video sample descriptions — read and written.
//!
//! A QuickTime video sample description has the same fixed layout as the ISO
//! `VisualSampleEntry` (ISO/IEC 14496-12:2026 §12.1.3): the standard sample
//! description header, then a 78-byte fixed part, then optional atoms. The
//! four-character code names the compression scheme; the crate types the layout
//! once here and records which code the entry carried.
//!
//! Source: `private/specs-verified/apple/isobmff/quicktime-file-format-2001.md`,
//! "Video Sample Description" and "Video sample data".

use alloc::vec::Vec;
use broadcast_core::Serialize;
use broadcast_core::len::{need_in, need_out};

use crate::box_support::OpaqueBox;
use crate::box_types::{
    BOX_HEADER_MIN_SIZE, BOX_TYPE_OFFSET, BOX_TYPE_SIZE, box_len, compact_form, write_box_header,
};
use crate::error::{Error, Result};
use crate::sample_entries::{
    SAMPLE_ENTRY_RESERVED_LEN, VISUAL_SAMPLE_ENTRY_SIZE, VisualSampleEntryFields,
};

/// Fewest bytes a QuickTime video entry can have: the box header and the
/// fixed `VisualSampleEntry` fields.
const MIN_ENTRY_LEN: usize = BOX_HEADER_MIN_SIZE + VISUAL_SAMPLE_ENTRY_SIZE;
/// Child region begins after the box header and the fixed fields.
const CHILDREN_OFFSET: usize = BOX_HEADER_MIN_SIZE + VISUAL_SAMPLE_ENTRY_SIZE;

/// A QuickTime video sample description (`avc1`-class layout, any codec code).
///
/// Every registered QuickTime video code has this same structure, so one type
/// carries them all and `codec_type` records which one was read.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct QuickTimeVideoEntry {
    codec_type: [u8; BOX_TYPE_SIZE],
    visual: VisualSampleEntryFields,
    children: Vec<OpaqueBox>,
}

impl QuickTimeVideoEntry {
    /// Build an entry for `codec_type` with default fixed fields and no
    /// children.
    #[must_use]
    pub fn new(codec_type: [u8; BOX_TYPE_SIZE]) -> Self {
        Self {
            codec_type,
            visual: VisualSampleEntryFields::default(),
            children: Vec::new(),
        }
    }

    /// Set the coded dimensions.
    #[must_use]
    pub fn with_dimensions(mut self, width: u16, height: u16) -> Self {
        self.visual.width = width;
        self.visual.height = height;
        self
    }

    /// The entry's four-CC.
    #[must_use]
    pub const fn codec_type(&self) -> [u8; BOX_TYPE_SIZE] {
        self.codec_type
    }

    /// The fixed sample description fields.
    #[must_use]
    pub const fn visual(&self) -> &VisualSampleEntryFields {
        &self.visual
    }

    /// Every child atom after the fixed part, verbatim.
    #[must_use]
    pub fn children(&self) -> &[OpaqueBox] {
        &self.children
    }

    /// Parse a whole entry, box header included.
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        need_in(bytes, MIN_ENTRY_LEN, "QuickTime video sample entry")?;
        let compact = compact_form(bytes)?;
        let bytes: &[u8] = &compact;
        let mut codec_type = [0u8; BOX_TYPE_SIZE];
        codec_type.copy_from_slice(&bytes[BOX_TYPE_OFFSET..BOX_HEADER_MIN_SIZE]);
        let mut visual = VisualSampleEntryFields::parse_body(bytes, "QuickTime video entry")?;
        let region = bytes.get(CHILDREN_OFFSET..).unwrap_or_default();
        let (children, trailing) = crate::box_support::capture_children(region).into_parts();
        visual.trailing = trailing.into_boxed_slice();
        Ok(Self {
            codec_type,
            visual,
            children,
        })
    }

    /// The SampleEntry `reserved[6]` bytes, carried verbatim.
    #[must_use]
    pub const fn reserved(&self) -> &[u8; SAMPLE_ENTRY_RESERVED_LEN] {
        &self.visual.reserved
    }

    /// The `data_reference_index`.
    #[must_use]
    pub const fn data_reference_index(&self) -> u16 {
        self.visual.data_reference_index
    }
}

impl Serialize for QuickTimeVideoEntry {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let mut n = MIN_ENTRY_LEN + self.visual.trailing.len();
        for child in &self.children {
            n += child.serialized_len();
        }
        box_len(n)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_box_header(buf, &self.codec_type, need)?;
        cursor += self.visual.serialize_body_into(&mut buf[cursor..])?;
        for child in &self.children {
            cursor += child.serialize_into(&mut buf[cursor..])?;
        }
        let end = cursor + self.visual.trailing.len();
        need_out(buf, end)?;
        buf[cursor..end].copy_from_slice(&self.visual.trailing);
        Ok(end)
    }
}

#[cfg(test)]
mod tests {
    use super::QuickTimeVideoEntry;
    use crate::box_support::OpaqueBox;
    use crate::error::Error;
    use alloc::vec;
    use broadcast_core::Serialize;

    fn boxed(codec: [u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let size = u32::try_from(8 + body.len()).expect("test body fits");
        out.extend_from_slice(&size.to_be_bytes());
        out.extend_from_slice(&codec);
        out.extend_from_slice(body);
        out
    }

    #[test]
    fn parse_and_round_trip_a_video_entry() {
        let mut body = vec![0u8; 78];
        body[24..26].copy_from_slice(&64u16.to_be_bytes());
        body[26..28].copy_from_slice(&48u16.to_be_bytes());
        let child = boxed(*b"pasp", &[0, 1, 0, 1]);
        let bytes = boxed(*b"apch", &[&body[..], &child[..]].concat());
        let entry = QuickTimeVideoEntry::parse_box(&bytes).expect("parses");
        assert_eq!(entry.codec_type(), *b"apch");
        assert_eq!(entry.visual().width, 64);
        assert_eq!(entry.visual().height, 48);
        assert_eq!(entry.children().len(), 1);
        let mut out = vec![0u8; entry.serialized_len()];
        entry.serialize_into(&mut out).expect("serializes");
        assert_eq!(out, bytes);
    }

    #[test]
    fn builder_round_trips() {
        let entry = QuickTimeVideoEntry::new(*b"jpeg").with_dimensions(16, 9);
        let mut out = vec![0u8; entry.serialized_len()];
        entry.serialize_into(&mut out).expect("serializes");
        let back = QuickTimeVideoEntry::parse_box(&out).expect("parses");
        assert_eq!(back, entry);
    }

    #[test]
    fn trailing_bytes_after_a_broken_child_are_kept() {
        let mut body = vec![0u8; 78];
        body.extend_from_slice(&[0, 0, 0, 2, b'x']);
        let bytes = boxed(*b"gif ", &body);
        let entry = QuickTimeVideoEntry::parse_box(&bytes).expect("parses");
        assert!(entry.children().is_empty());
        assert_eq!(entry.visual().trailing.len(), 5);
    }

    #[test]
    fn short_entry_is_an_error() {
        let bytes = boxed(*b"png ", &[0u8; 8]);
        assert!(matches!(
            QuickTimeVideoEntry::parse_box(&bytes),
            Err(Error::BufferTooShort(_))
        ));
    }

    #[test]
    fn opaque_child_type_is_public() {
        let child = OpaqueBox {
            box_type: *b"pasp",
            data: vec![0; 8],
            to_end: false,
            largesize: false,
        };
        assert_eq!(child.box_type, *b"pasp");
    }
}
