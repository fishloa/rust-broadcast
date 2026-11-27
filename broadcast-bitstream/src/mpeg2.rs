//! MPEG-2 video (H.262) `sequence_header()` — ITU-T H.262 = ISO/IEC 13818-2
//! §6.2.2.1.
//!
//! The sequence header opens every MPEG-1/2 video elementary stream (and the
//! `esds` DecoderSpecificInfo a container carries) with the coded picture
//! geometry, which is the config-recovery fact a demuxer needs. The rest of
//! the sequence syntax — extensions included — is [`crate::h262`].

use crate::annexb::{START_CODE_PREFIX_LEN, find_start_code_prefix};
use crate::error::{Error, Result};
use broadcast_core::len::need_in;

/// The 32-bit `sequence_header_code` start code (`0x000001B3`) that prefixes a
/// MPEG-1/2 video `sequence_header()` — ISO/IEC 13818-2 §6.2.2.1, Table 6-1.
pub const SEQUENCE_HEADER_CODE: [u8; 4] = [0x00, 0x00, 0x01, 0xB3];

/// Bytes of `horizontal_size_value` + `vertical_size_value` following the
/// 4-byte start code: 12 + 12 = 24 bits = 3 bytes (§6.2.2.1).
const SEQ_SIZE_FIELD_BYTES: usize = 3;

/// Half a byte: the two 12-bit sizes of `sequence_header()` straddle a byte
/// boundary at four bits (§6.2.2.1).
const NIBBLE_BITS: u32 = u8::BITS / 2;

/// Mask selecting the low nibble of a `sequence_header()` size byte.
const NIBBLE_MASK: u8 = 0x0F;

/// Index of the third, all-`vertical_size_value` byte of the size fields.
const SEQ_SIZE_LOW_BYTE_INDEX: usize = 2;

/// The coded picture geometry read from a MPEG-2 video `sequence_header()`.
///
/// ISO/IEC 13818-2 §6.2.2.1: `horizontal_size_value` and `vertical_size_value`
/// are 12-bit fields; the full sizes may be extended by a
/// `sequence_extension()`, which [`crate::h262::SequenceHeaderGeometry`]
/// carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Mpeg2SeqHeader {
    width: u16,
    height: u16,
}

impl Mpeg2SeqHeader {
    /// `horizontal_size_value` (12 bits) — coded picture width in pixels.
    pub fn width(&self) -> u16 {
        self.width
    }

    /// `vertical_size_value` (12 bits) — coded picture height in pixels.
    pub fn height(&self) -> u16 {
        self.height
    }

    /// Find the first `sequence_header()` in an MPEG-2 video elementary stream
    /// (or `esds` DecoderSpecificInfo) and decode its picture geometry.
    ///
    /// Scans `es` for the `0x000001B3` start code, then reads the two 12-bit
    /// size fields. Returns [`Error::InvalidInput`] if no sequence header is
    /// present, or [`Error::BufferTooShort`] if it is truncated.
    pub fn find(es: &[u8]) -> Result<Self> {
        let start = find_start_code(es, SEQUENCE_HEADER_CODE[SEQUENCE_HEADER_CODE.len() - 1])
            .ok_or(Error::InvalidInput(
                "no MPEG-2 sequence_header (start code 0x000001B3) in elementary stream",
            ))?;
        // Fields begin immediately after the 4-byte start code.
        let fields_at = start + SEQUENCE_HEADER_CODE.len();
        need_in(
            es,
            fields_at + SEQ_SIZE_FIELD_BYTES,
            "MPEG-2 sequence_header size fields",
        )?;
        let b = &es[fields_at..fields_at + SEQ_SIZE_FIELD_BYTES];
        // horizontal_size_value = b[0] << 4 | b[1] >> 4 (12 bits)
        // vertical_size_value   = (b[1] & 0x0F) << 8 | b[2] (12 bits)
        let width = ((b[0] as u16) << NIBBLE_BITS) | ((b[1] as u16) >> NIBBLE_BITS);
        let height =
            (((b[1] & NIBBLE_MASK) as u16) << u8::BITS) | b[SEQ_SIZE_LOW_BYTE_INDEX] as u16;
        Ok(Self { width, height })
    }
}

/// Find the byte offset of a `0x000001 <code>` start code in `data`, or `None`.
fn find_start_code(data: &[u8], code: u8) -> Option<usize> {
    let mut from = 0usize;
    while let Some(at) = find_start_code_prefix(data, from) {
        if data.get(at + START_CODE_PREFIX_LEN) == Some(&code) {
            return Some(at);
        }
        from = at + 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seq_header_320x240() {
        // start code 00 00 01 B3, then 320 (0x140) / 240 (0x0F0):
        // 0x140 << 12 | 0x0F0 = 0x1400F0 → bytes 14 00 F0
        let es = [0x00, 0x00, 0x01, 0xB3, 0x14, 0x00, 0xF0, 0x23, 0xFF];
        let sh = Mpeg2SeqHeader::find(&es).unwrap();
        assert_eq!(sh.width(), 320);
        assert_eq!(sh.height(), 240);
    }

    #[test]
    fn seq_header_scans_past_leading_bytes() {
        let mut es = alloc::vec![0xDE, 0xAD, 0xBE, 0xEF];
        es.extend_from_slice(&[0x00, 0x00, 0x01, 0xB3, 0x28, 0x01, 0xE0]);
        let sh = Mpeg2SeqHeader::find(&es).unwrap();
        assert_eq!(sh.width(), 640);
        assert_eq!(sh.height(), 480);
    }

    #[test]
    fn seq_header_missing() {
        assert!(matches!(
            Mpeg2SeqHeader::find(&[0x00, 0x00, 0x01, 0xB8]),
            Err(crate::error::Error::InvalidInput(..))
        ));
    }

    /// #3797: a sequence header ending inside its 3 size bytes is
    /// `BufferTooShort { need: 7 }`, never a slice panic.
    #[test]
    fn seq_header_truncated_size_fields_report_the_need() {
        use broadcast_core::error::BufferTooShort;
        let base = [0x00, 0x00, 0x01, 0xB3];
        let size = [0x28, 0x01, 0xE0];
        for extra in 0..3usize {
            let mut es = base.to_vec();
            es.extend_from_slice(&size[..extra]);
            match Mpeg2SeqHeader::find(&es) {
                Err(crate::error::Error::BufferTooShort(BufferTooShort { need, have, .. })) => {
                    assert_eq!((need, have), (7, 4 + extra), "extra {extra}");
                }
                other => panic!("extra {extra}: expected BufferTooShort, got {other:?}"),
            }
        }
        let ok = Mpeg2SeqHeader::find(&[0x00, 0x00, 0x01, 0xB3, 0x28, 0x01, 0xE0]).unwrap();
        assert_eq!((ok.width(), ok.height()), (640, 480));
    }
}
