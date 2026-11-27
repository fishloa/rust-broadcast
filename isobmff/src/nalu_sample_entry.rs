//! The NAL-unit video sample entries — ISO/IEC 14496-15:2017 clauses 5, 6, 8,
//! 9, 10 and F.
//!
//! Every registered NAL-unit video sample entry (`mvc1`…`mvc4`, `svc1`,
//! `svc2`, `svcM`, `mvd1`…`mvd4`, `a3d1`…`a3d4`, `hvc2`/`hev2`, `hvt1`,
//! `lhv1`, `lht1`) has the same `VisualSampleEntry` fixed part; the four-CC
//! names the codec variant and the codec's configuration boxes follow as
//! children. One type carries them all, with `codec_type` recording which.
//!
//! Source: `private/specs/iso_iec_14496-15_nal_file_format_2017.pdf`; the
//! 2010 transcription `private/specs-verified/mpeg/isobmff/iso-iec-14496-15-2010.md`
//! covers the same structures for the AVC extensions.

use alloc::vec::Vec;
use broadcast_core::len::{need_in, need_out};
use broadcast_core::{Parse, Serialize};

use crate::box_support::OpaqueBox;
use crate::box_types::{
    BOX_HEADER_MIN_SIZE, BOX_TYPE_OFFSET, BOX_TYPE_SIZE, box_len, compact_form, write_box_header,
};
use crate::error::{Error, Result};
use crate::sample_entries::{
    SAMPLE_ENTRY_RESERVED_LEN, VISUAL_SAMPLE_ENTRY_SIZE, VisualSampleEntryFields,
};

/// Fewest bytes a NAL-unit video entry can have: the box header and the fixed
/// `VisualSampleEntry` fields.
const MIN_ENTRY_LEN: usize = BOX_HEADER_MIN_SIZE + VISUAL_SAMPLE_ENTRY_SIZE;
/// Child region begins after the box header and the fixed fields.
const CHILDREN_OFFSET: usize = BOX_HEADER_MIN_SIZE + VISUAL_SAMPLE_ENTRY_SIZE;

/// The four-CCs this crate parses as a [`NaluVideoSampleEntry`].
pub const NALU_VIDEO_ENTRIES: &[[u8; BOX_TYPE_SIZE]] = &[
    *b"mvc1", *b"mvc2", *b"mvc3", *b"mvc4", *b"svc1", *b"svc2", *b"svcM", *b"mvd1", *b"mvd2",
    *b"mvd3", *b"mvd4", *b"a3d1", *b"a3d2", *b"a3d3", *b"a3d4", *b"hvc2", *b"hev2", *b"hvt1",
    *b"lhv1", *b"lht1",
];

/// A NAL-unit video sample entry, any of [`NALU_VIDEO_ENTRIES`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct NaluVideoSampleEntry {
    codec_type: [u8; BOX_TYPE_SIZE],
    visual: VisualSampleEntryFields,
    children: Vec<OpaqueBox>,
}

impl NaluVideoSampleEntry {
    /// Build an entry for `codec_type` with default fixed fields.
    ///
    /// # Errors
    /// `codec_type` is not one this crate types here.
    pub fn new(codec_type: [u8; BOX_TYPE_SIZE]) -> Result<Self> {
        if !NALU_VIDEO_ENTRIES.contains(&codec_type) {
            return Err(Error::UnexpectedBox {
                expected: "a NAL-unit video sample entry",
            });
        }
        Ok(Self {
            codec_type,
            visual: VisualSampleEntryFields::default(),
            children: Vec::new(),
        })
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

    /// Every child after the fixed part, verbatim (the codec configuration
    /// boxes included; the typed box types parse them on demand).
    #[must_use]
    pub fn children(&self) -> &[OpaqueBox] {
        &self.children
    }

    /// The SampleEntry `reserved[6]` bytes, carried verbatim.
    #[must_use]
    pub const fn reserved(&self) -> &[u8; SAMPLE_ENTRY_RESERVED_LEN] {
        &self.visual.reserved
    }

    /// Parse a whole entry, box header included.
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        need_in(bytes, MIN_ENTRY_LEN, "NAL-unit video sample entry")?;
        let compact = compact_form(bytes)?;
        let bytes: &[u8] = &compact;
        let mut codec_type = [0u8; BOX_TYPE_SIZE];
        codec_type.copy_from_slice(&bytes[BOX_TYPE_OFFSET..BOX_HEADER_MIN_SIZE]);
        if !NALU_VIDEO_ENTRIES.contains(&codec_type) {
            return Err(Error::UnexpectedBox {
                expected: "a NAL-unit video sample entry",
            });
        }
        let mut visual = VisualSampleEntryFields::parse_body(bytes, "NAL-unit video entry")?;
        let region = bytes.get(CHILDREN_OFFSET..).unwrap_or_default();
        let (children, trailing) = crate::box_support::capture_children(region).into_parts();
        visual.trailing = trailing.into_boxed_slice();
        Ok(Self {
            codec_type,
            visual,
            children,
        })
    }
}

impl<'a> Parse<'a> for NaluVideoSampleEntry {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::parse_box(bytes)
    }
}

impl Serialize for NaluVideoSampleEntry {
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
        let mut c = write_box_header(buf, &self.codec_type, need)?;
        c += self.visual.serialize_body_into(&mut buf[c..])?;
        for child in &self.children {
            c += child.serialize_into(&mut buf[c..])?;
        }
        let end = c + self.visual.trailing.len();
        need_out(buf, end)?;
        buf[c..end].copy_from_slice(&self.visual.trailing);
        Ok(end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn every_listed_entry_round_trips() {
        for code in NALU_VIDEO_ENTRIES {
            let entry = NaluVideoSampleEntry::new(*code)
                .expect("builds")
                .with_dimensions(64, 48);
            let mut out = vec![0u8; entry.serialized_len()];
            entry.serialize_into(&mut out).expect("serializes");
            let back = NaluVideoSampleEntry::parse_box(&out).expect("parses");
            assert_eq!(back, entry, "{code:?}");
        }
    }

    #[test]
    fn an_untyped_entry_is_refused() {
        assert!(NaluVideoSampleEntry::new(*b"avc1").is_err());
    }
}
