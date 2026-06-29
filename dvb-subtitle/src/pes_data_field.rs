//! PES data field parsing — ETSI EN 300 743 §7.2, Table 3.
//!
//! The top-level structure that wraps the subtitling segments within a
//! DVB subtitle PES packet.

use crate::any::AnySegment;
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// The required `data_identifier` value for DVB subtitles.
pub const DATA_IDENTIFIER: u8 = 0x20;
/// The required `subtitle_stream_id` value.
pub const SUBTITLE_STREAM_ID: u8 = 0x00;
/// The sync_byte that prefixes every subtitling_segment.
pub const SYNC_BYTE: u8 = 0x0F;
/// The `end_of_PES_data_field_marker` value.
pub const END_OF_PES_MARKER: u8 = 0xFF;

/// The minimum PES data field: data_identifier(1) + subtitle_stream_id(1) + end_marker(1) = 3 bytes.
const MIN_FIELD_LEN: usize = 3;
/// Generic segment header: sync_byte(1) + segment_type(1) + page_id(2) + segment_length(2) = 6 bytes.
const SEGMENT_HEADER_LEN: usize = 6;

/// The top-level PES data field structure for DVB subtitles.
///
/// Contains the data_identifier, subtitle_stream_id, one or more
/// subtitling segments, and the end-of-PES marker.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PesDataField<'a> {
    /// The subtitle_stream_id (must be 0x00).
    pub subtitle_stream_id: u8,
    /// The parsed subtitling segments.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub segments: alloc::vec::Vec<AnySegment<'a>>,
    /// Raw bytes after the last parsed segment up to and including the end marker.
    /// Preserved for byte-exact round-trip on data with trailing stuff or truncated segments.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub(crate) suffix: &'a [u8],
}

impl<'a> Parse<'a> for PesDataField<'a> {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < MIN_FIELD_LEN {
            return Err(Error::BufferTooShort {
                need: MIN_FIELD_LEN,
                have: bytes.len(),
                what: "PES_data_field",
            });
        }
        if bytes[0] != DATA_IDENTIFIER {
            return Err(Error::BadDataIdentifier(bytes[0]));
        }
        let subtitle_stream_id = bytes[1];
        if subtitle_stream_id != SUBTITLE_STREAM_ID {
            // Per spec it must be 0x00 but we parse it anyway
        }

        let mut pos: usize = 2;
        let mut segments = alloc::vec::Vec::new();

        // Read segments: each starts with sync_byte 0x0F
        while pos < bytes.len() && bytes[pos] == SYNC_BYTE {
            if pos + SEGMENT_HEADER_LEN > bytes.len() {
                // Truncated segment header — stop and let suffix capture
                break;
            }
            let segment_length = u16::from_be_bytes([bytes[pos + 4], bytes[pos + 5]]) as usize;
            let segment_end = pos + SEGMENT_HEADER_LEN + segment_length;
            if segment_end > bytes.len() {
                // Segment data exceeds available bytes (truncated PES) —
                // stop the loop and let suffix capture the remainder verbatim
                break;
            }

            let seg_bytes = &bytes[pos..segment_end];
            let segment_type = bytes[pos + 1];

            match AnySegment::dispatch(segment_type, seg_bytes) {
                Some(Ok(seg)) => segments.push(seg),
                Some(Err(_e)) => {
                    // Malformed but recognised segment — skip it per §7.2.0.2
                    segments.push(AnySegment::Unknown {
                        segment_type,
                        page_id: u16::from_be_bytes([seg_bytes[2], seg_bytes[3]]),
                        data: &seg_bytes[SEGMENT_HEADER_LEN..],
                    });
                }
                None => {
                    // Unknown segment_type: preserve as raw
                    segments.push(AnySegment::Unknown {
                        segment_type,
                        page_id: u16::from_be_bytes([seg_bytes[2], seg_bytes[3]]),
                        data: &seg_bytes[SEGMENT_HEADER_LEN..],
                    });
                }
            }

            pos = segment_end;
        }

        // Preserve trailing bytes (including end marker) for byte-exact round-trip
        let suffix = &bytes[pos..];

        Ok(PesDataField {
            subtitle_stream_id,
            segments,
            suffix,
        })
    }
}

impl Serialize for PesDataField<'_> {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        2 + self.suffix.len()
            + self
                .segments
                .iter()
                .map(|s| s.serialized_len())
                .sum::<usize>()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> core::result::Result<usize, Self::Error> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::BufferTooShort {
                need: len,
                have: buf.len(),
                what: "PES_data_field serialize",
            });
        }
        buf[0] = DATA_IDENTIFIER;
        buf[1] = self.subtitle_stream_id;
        let mut off = 2;
        for seg in &self.segments {
            let seg_len = seg.serialized_len();
            seg.serialize_into(&mut buf[off..off + seg_len])?;
            off += seg_len;
        }
        buf[off..off + self.suffix.len()].copy_from_slice(self.suffix);
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_common::Serialize;

    #[test]
    fn round_trip_single_end_of_display_set() {
        let bytes = [
            0x20, 0x00, // data_identifier + subtitle_stream_id
            0x0F, 0x80, 0x00, 0x01, 0x00, 0x00, // end of display set
            0xFF, // end marker
        ];
        let field = PesDataField::parse(&bytes).unwrap();
        assert_eq!(field.segments.len(), 1);
        let out = field.to_bytes();
        assert_eq!(out, bytes);
    }

    #[test]
    fn round_trip_multiple_segments() {
        let bytes = [
            0x20, 0x00, // Display definition
            0x0F, 0x14, 0x00, 0x01, 0x00, 0x05, 0x30, 0x02, 0xCF, 0x01, 0x1F,
            // Page composition: 1 region = 2 fixed + 6 = 8 body → seg_len=0x08
            0x0F, 0x10, 0x00, 0x01, 0x00, 0x08, 0x0A, 0x08, 0x01, 0x00, 0x00, 0x64, 0x00, 0x32,
            // End of display set
            0x0F, 0x80, 0x00, 0x01, 0x00, 0x00, 0xFF,
        ];
        let field = PesDataField::parse(&bytes).unwrap();
        assert_eq!(field.segments.len(), 3);
        assert_eq!(field.segments[0].name(), "DISPLAY_DEFINITION");
        assert_eq!(field.segments[1].name(), "PAGE_COMPOSITION");
        assert_eq!(field.segments[2].name(), "END_OF_DISPLAY_SET");
        let out = field.to_bytes();
        assert_eq!(out, bytes);
    }

    #[test]
    fn unknown_segment_preserved() {
        let bytes = [
            0x20, 0x00, 0x0F, 0xA0, 0x00, 0x01, 0x00, 0x02, 0xCA, 0xFE, // unknown 0xA0
            0xFF,
        ];
        let field = PesDataField::parse(&bytes).unwrap();
        assert_eq!(field.segments.len(), 1);
        assert_eq!(field.segments[0].name(), "UNKNOWN");
        let out = field.to_bytes();
        assert_eq!(out, bytes);
    }

    #[test]
    fn bad_data_identifier() {
        let bytes = [0x00, 0x00, 0xFF];
        let err = PesDataField::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::BadDataIdentifier(0x00)));
    }

    #[test]
    fn no_end_marker_is_ok() {
        // Per forward-compatibility §7.2.0.2, a missing end marker is tolerated
        let bytes = [0x20, 0x00, 0x00];
        let field = PesDataField::parse(&bytes).unwrap();
        assert_eq!(field.segments.len(), 0);
    }
}
