//! MPEG-2 video (ITU-T H.262 | ISO/IEC 13818-2) start-code walk: the
//! sequence-level display syntax and the picture-level
//! `extension_and_user_data(2)` wrapper that carries AFD and bar data.
//!
//! # Spec citations
//!
//! - **H.262 §6.2.2.1** `sequence_header()`: `sequence_header_code`
//!   `0x000001B3`, `horizontal_size_value` `u(12)`,
//!   `vertical_size_value` `u(12)`, `aspect_ratio_information` `u(4)`.
//! - **H.262 §6.2.2.3** `sequence_extension()` (`extension_start_code_identifier`
//!   1): `horizontal_size_extension` `u(2)`, `vertical_size_extension` `u(2)`
//!   — the coded size is `(extension << 12) | value`.
//! - **H.262 §6.2.2.4** `sequence_display_extension()` (identifier 2):
//!   `video_format` `u(3)`, `colour_description` `u(1)`, the optional
//!   `colour_primaries` / `transfer_characteristics` / `matrix_coefficients`
//!   `u(8)` triple, then `display_horizontal_size` `u(14)`, `marker_bit` and
//!   `display_vertical_size` `u(14)`.
//! - **H.262 §6.2.3.1** `picture_header()`: `picture_start_code` `0x00000100`.
//! - **H.262 §6.2.2.5-6.2.2.8 / §6.2.3.3** `extension_and_user_data(i)` and
//!   `user_data()` (`0x000001B2` then `user_identifier` + `user_structure()`
//!   up to the next start code) — the AFD/bar-data carriage of ETSI TS 101 154
//!   Annex B.6 (`private/specs-verified/mpeg/transmux/codec/afd-bar-data.md` §4.4).
//!
//! Every syntax element above was page-checked against the rendered PDF and
//! is transcribed in `private/specs-verified/mpeg/transmux/codec/mpeg2-video-display.md`.
//!
//! The presentational conversion of [`SequenceHeaderGeometry`] (into a
//! container's display geometry) belongs to the consumer, not here.

use crate::annexb::{START_CODE_PREFIX_LEN, find_start_code_prefix};
use crate::bitreader::BitReader;
use crate::error::Result;
use crate::facts::ActiveFormatDescription;
use crate::mpeg2::{SEQUENCE_HEADER_CODE, check_size_value};
use crate::user_data::{AFD_ACTIVE_FORMAT_FLAG_BIT, AFD_CODE_MASK};

/// `sequence_header_code` `0x000001B3` (H.262 §6.2.2.1, Table 6-1).
pub const SEQUENCE_HEADER_CODE_BYTE: u8 = SEQUENCE_HEADER_CODE[3];
/// `picture_start_code` `0x00000100` (H.262 §6.2.3.1, Table 6-1).
pub const PICTURE_START_CODE_BYTE: u8 = 0x00;
/// First slice start code `0x00000101` (H.262 §6.2.4).
const SLICE_FIRST: u8 = 0x01;
/// Last slice start code `0x000001AF` (H.262 §6.2.4).
const SLICE_LAST: u8 = 0xAF;
/// `extension_start_code` `0x000001B5` (H.262 Table 6-1).
pub const EXTENSION_START_CODE_BYTE: u8 = 0xB5;
/// `user_data_start_code` `0x000001B2` (H.262 §6.2.2.8, Table 6-1).
pub const USER_DATA_START_CODE_BYTE: u8 = 0xB2;

/// `extension_start_code_identifier` of `sequence_extension()` (H.262
/// §6.2.2.3, Table 6-1).
pub const SEQUENCE_EXTENSION_ID: u8 = 0x1;
/// `extension_start_code_identifier` of `sequence_display_extension()`
/// (H.262 §6.2.2.4).
pub const SEQUENCE_DISPLAY_EXTENSION_ID: u8 = 0x2;

/// `aspect_ratio_information` (H.262 Table 6-3): square samples.
pub const ASPECT_SQUARE: u8 = 1;
/// `aspect_ratio_information` (Table 6-3): DAR 4:3.
pub const ASPECT_DAR_4_3: u8 = 2;
/// `aspect_ratio_information` (Table 6-3): DAR 16:9.
pub const ASPECT_DAR_16_9: u8 = 3;
/// `aspect_ratio_information` (Table 6-3): DAR 2.21:1.
pub const ASPECT_DAR_2_21: u8 = 4;
/// `aspect_ratio_information` values 5..=15 are "Reserved" (Table 6-3).
pub const FIRST_RESERVED_ASPECT: u8 = 5;

/// H.262 Table 6-4: `frame_rate_code` → frames per second, as an exact
/// numerator/denominator pair (`30000/1001` is the 29.97 Hz broadcast rate).
/// Index `code - 1`; code 0 and 9..=15 are "reserved".
const FRAME_RATES: [(u32, u32); FRAME_RATE_CODES] = [
    (24_000, 1001),
    (24, 1),
    (25, 1),
    (30_000, 1001),
    (30, 1),
    (50, 1),
    (60_000, 1001),
    (60, 1),
];

/// Table 6-4 defines `frame_rate_code` 1..=8 (the `FRAME_RATES` rows).
const FRAME_RATE_CODES: usize = 8;

/// `frame_rate_code` is `u(4)` (§6.2.2.1).
const FRAME_RATE_CODE_BITS: usize = 4;

/// The 12-bit base size (`horizontal_size_value` / `vertical_size_value`).
const SIZE_VALUE_BITS: usize = 12;
/// The 2-bit size extension (`horizontal_size_extension` /
/// `vertical_size_extension`) shifts the base by this many bits.
const SIZE_EXTENSION_SHIFT: u32 = 12;
/// The size extension value is `u(2)`.
const SIZE_EXTENSION_BITS: usize = 2;
/// `aspect_ratio_information` is `u(4)`.
const ASPECT_BITS: usize = 4;
/// `extension_start_code_identifier` is `u(4)`.
const EXTENSION_ID_BITS: usize = 4;
/// `video_format` is `u(3)`.
const VIDEO_FORMAT_BITS: usize = 3;
/// `display_horizontal_size` / `display_vertical_size` are `u(14)`.
const DISPLAY_SIZE_BITS: usize = 14;
/// `chroma_format` is `u(2)` (§6.3.3).
const CHROMA_FORMAT_BITS: usize = 2;
/// Byte-wide fields: `profile_and_level_indication`, `colour_primaries`,
/// `transfer_characteristics`, `matrix_coefficients` (§6.3.3).
const BYTE_BITS: usize = u8::BITS as usize;
/// Right shift that isolates `extension_start_code_identifier`.
const EXTENSION_ID_SHIFT: u32 = u8::BITS - EXTENSION_ID_BITS as u32;

/// `display_aspect_ratio` 4:3, as width:height (Table 6-3).
const DAR_RATIO_4_3: (u64, u64) = (4, 3);
/// `display_aspect_ratio` 16:9, as width:height (Table 6-3).
const DAR_RATIO_16_9: (u64, u64) = (16, 9);
/// `display_aspect_ratio` 2.21:1, as width:height (Table 6-3).
const DAR_RATIO_2_21: (u64, u64) = (221, 100);

/// Longest start-code-delimited section scanned for a `sequence_header()`
/// plus its extensions. A `sequence_header()` with both quantiser matrices is
/// at most this size; the scan stops at the next start code anyway.
const SEQUENCE_SECTION_MAX: usize = 256;

/// H.262 §6.3.3: the "Display Aspect Ratio" Table 6-3 names for its codes,
/// as width:height (the notation caveat is in `private/specs-verified/mpeg/transmux/codec/mpeg2-video-display.md`).
fn dar_ratio(aspect: u8) -> Option<(u64, u64)> {
    match aspect {
        ASPECT_DAR_4_3 => Some(DAR_RATIO_4_3),
        ASPECT_DAR_16_9 => Some(DAR_RATIO_16_9),
        ASPECT_DAR_2_21 => Some(DAR_RATIO_2_21),
        _ => None,
    }
}

/// Greatest common divisor of two non-zero values.
fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a.max(1)
}

/// Reduced sample aspect ratio from two size terms: the H.262 §6.3.3 relation
/// reads, in width:height terms verified against real captures,
/// `SAR = DAR_w * size_h / (DAR_h * size_w)`. Returns `None` when the result
/// does not fit the `u16` wire terms the sample-aspect-ratio model uses, or
/// when a size term is zero.
fn derived_sar(dar: (u64, u64), size_w: u32, size_h: u32) -> Option<(u16, u16)> {
    let num = dar.0.checked_mul(u64::from(size_h))?;
    let den = dar.1.checked_mul(u64::from(size_w))?;
    if num == 0 || den == 0 {
        return None;
    }
    // Reduce before the `u16` bound so a large-but-reducible result (e.g.
    // 221 * 576 / 90000) is still representable.
    let g = gcd(num, den);
    let (w, h) = (num / g, den / g);
    if w > u64::from(u16::MAX) || h > u64::from(u16::MAX) {
        return None;
    }
    Some((u16::try_from(w).ok()?, u16::try_from(h).ok()?))
}

/// A parsed `sequence_header()` plus the extensions that follow it
/// (`sequence_extension()` and `sequence_display_extension()`, H.262 §6.2.2.3,
/// §6.2.2.4).
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SequenceHeaderGeometry {
    coded_width: u32,
    coded_height: u32,
    aspect_ratio_information: u8,
    frame_rate_code: u8,
    display: Option<SequenceDisplayExtension>,
}

impl SequenceHeaderGeometry {
    /// Coded picture width: `(horizontal_size_extension << 12) | horizontal_size_value`.
    pub fn coded_width(&self) -> u32 {
        self.coded_width
    }

    /// Coded picture height: `(vertical_size_extension << 12) | vertical_size_value`.
    pub fn coded_height(&self) -> u32 {
        self.coded_height
    }

    /// Table 6-3 `aspect_ratio_information` as signalled.
    pub fn aspect_ratio_information(&self) -> u8 {
        self.aspect_ratio_information
    }

    /// Table 6-4 `frame_rate_code` as signalled (0 and 9..=15 are reserved).
    pub fn frame_rate_code(&self) -> u8 {
        self.frame_rate_code
    }

    /// The frame rate Table 6-4 gives for [`frame_rate_code`](Self::frame_rate_code),
    /// as an exact frames-per-second numerator/denominator pair (so
    /// `30_000/1001` stays exact). `None` for a reserved code.
    pub fn frame_rate(&self) -> Option<(u32, u32)> {
        FRAME_RATES
            .get(usize::from(self.frame_rate_code).checked_sub(1)?)
            .copied()
    }

    /// `sequence_display_extension()`, when present.
    pub fn display(&self) -> Option<&SequenceDisplayExtension> {
        self.display.as_ref()
    }

    /// The sample aspect ratio this sequence implies, or `None` when the
    /// sequence leaves it unspecified (aspect 0, 5..=15, square with the
    /// extension's display size disagreeing — never happens for a real
    /// square-pixel stream — or a derived ratio outside `u16`).
    ///
    /// H.262 §6.3.3: without `sequence_display_extension()` the relation uses
    /// the coded size; with it, the extension's display size.
    pub fn sample_aspect_ratio(&self) -> Option<(u16, u16)> {
        match self.aspect_ratio_information {
            ASPECT_SQUARE => Some((1, 1)),
            aspect => {
                let (size_w, size_h) = match &self.display {
                    Some(d) => (d.display_horizontal_size, d.display_vertical_size),
                    None => (self.coded_width, self.coded_height),
                };
                derived_sar(dar_ratio(aspect)?, size_w, size_h)
            }
        }
    }
}

/// H.262 §6.2.2.4 `sequence_display_extension()` fields that describe display.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SequenceDisplayExtension {
    video_format: u8,
    colour_description: Option<(u8, u8, u8)>,
    display_horizontal_size: u32,
    display_vertical_size: u32,
}

impl SequenceDisplayExtension {
    /// Table 6-6 `video_format`.
    pub fn video_format(&self) -> u8 {
        self.video_format
    }

    /// `colour_primaries`, `transfer_characteristics` and `matrix_coefficients`
    /// as signalled (absent when `colour_description == 0`).
    pub fn colour_description(&self) -> Option<(u8, u8, u8)> {
        self.colour_description
    }

    /// "Intended display" active rectangle width in samples (Table 6-3's
    /// `sequence_display_extension()` case).
    pub fn display_horizontal_size(&self) -> u32 {
        self.display_horizontal_size
    }

    /// "Intended display" active rectangle height in lines.
    pub fn display_vertical_size(&self) -> u32 {
        self.display_vertical_size
    }
}

/// Byte offset of the next `0x000001 <code>` start code at or after `from`,
/// whichever code it is.
fn find_any_start_code(data: &[u8], from: usize) -> Option<(usize, u8)> {
    let at = find_start_code_prefix(data, from)?;
    Some((at, *data.get(at + START_CODE_PREFIX_LEN)?))
}

/// The body of the start-code section that begins at `start` (after the
/// 4-byte start code), up to the next start code or the end of `data`.
fn section_body(data: &[u8], start: usize) -> &[u8] {
    let from = start + SEQUENCE_HEADER_CODE.len();
    let end = match find_start_code_prefix(data, from) {
        // `find_start_code_prefix` returns the offset of the `00 00 01`
        // prefix's first byte.
        Some(next) => next,
        None => data.len(),
    };
    data.get(from..end).unwrap_or_default()
}

/// Parse a `sequence_header()` body (the bytes after `0x000001B3`).
fn parse_sequence_header_body(body: &[u8]) -> Result<(u32, u32, u8, u8)> {
    let mut r = BitReader::from_rbsp(body, "MPEG-2 sequence_header")?;
    let width = r.read_bits(SIZE_VALUE_BITS, "horizontal_size_value")?;
    let height = r.read_bits(SIZE_VALUE_BITS, "vertical_size_value")?;
    check_size_value("horizontal_size_value", width)?;
    check_size_value("vertical_size_value", height)?;
    let aspect = r.read_bits(ASPECT_BITS, "aspect_ratio_information")?;
    let frame_rate = r.read_bits(FRAME_RATE_CODE_BITS, "frame_rate_code")?;
    Ok((
        u32::try_from(width).unwrap_or(u32::MAX),
        u32::try_from(height).unwrap_or(u32::MAX),
        u8::try_from(aspect).unwrap_or(u8::MAX),
        u8::try_from(frame_rate).unwrap_or(u8::MAX),
    ))
}

/// Parse a `sequence_extension()` body: the `(horizontal_size_extension,
/// vertical_size_extension)` pair.
fn parse_sequence_extension_body(body: &[u8]) -> Result<(u32, u32)> {
    let mut r = BitReader::from_rbsp(body, "MPEG-2 sequence_extension")?;
    let id = r.read_bits(EXTENSION_ID_BITS, "extension_start_code_identifier")?;
    debug_assert_eq!(u8::try_from(id).unwrap_or(u8::MAX), SEQUENCE_EXTENSION_ID);
    // profile_and_level_indication(8) progressive_sequence(1) chroma_format(2)
    let _ = r.read_bits(BYTE_BITS, "profile_and_level_indication")?;
    let _ = r.read_flag("progressive_sequence")?;
    let _ = r.read_bits(CHROMA_FORMAT_BITS, "chroma_format")?;
    let hx = r.read_bits(SIZE_EXTENSION_BITS, "horizontal_size_extension")?;
    let vx = r.read_bits(SIZE_EXTENSION_BITS, "vertical_size_extension")?;
    Ok((
        u32::try_from(hx).unwrap_or(0),
        u32::try_from(vx).unwrap_or(0),
    ))
}

/// Parse a `sequence_display_extension()` body.
fn parse_sequence_display_extension_body(body: &[u8]) -> Result<SequenceDisplayExtension> {
    let mut r = BitReader::from_rbsp(body, "MPEG-2 sequence_display_extension")?;
    let id = r.read_bits(EXTENSION_ID_BITS, "extension_start_code_identifier")?;
    debug_assert_eq!(
        u8::try_from(id).unwrap_or(u8::MAX),
        SEQUENCE_DISPLAY_EXTENSION_ID
    );
    let video_format = r.read_bits(VIDEO_FORMAT_BITS, "video_format")?;
    let colour = r.read_flag("colour_description")?;
    let colour_description = if colour {
        let p = r.read_bits(BYTE_BITS, "colour_primaries")?;
        let t = r.read_bits(BYTE_BITS, "transfer_characteristics")?;
        let m = r.read_bits(BYTE_BITS, "matrix_coefficients")?;
        Some((
            u8::try_from(p).unwrap_or(u8::MAX),
            u8::try_from(t).unwrap_or(u8::MAX),
            u8::try_from(m).unwrap_or(u8::MAX),
        ))
    } else {
        None
    };
    let h = r.read_bits(DISPLAY_SIZE_BITS, "display_horizontal_size")?;
    let _ = r.read_flag("marker_bit")?;
    let v = r.read_bits(DISPLAY_SIZE_BITS, "display_vertical_size")?;
    Ok(SequenceDisplayExtension {
        video_format: u8::try_from(video_format).unwrap_or(u8::MAX),
        colour_description,
        display_horizontal_size: u32::try_from(h).unwrap_or(u32::MAX),
        display_vertical_size: u32::try_from(v).unwrap_or(u32::MAX),
    })
}

/// Parse the `sequence_header()` at `start` and the extensions that follow it
/// (`sequence_extension()`, then `sequence_display_extension()`, in that
/// order per H.262 §6.2.2.2). Absent extensions leave the base size and no
/// display description. A zero base size value is rejected with
/// [`crate::error::Error::InvalidValue`] (H.262 §6.3.3).
pub fn parse_sequence_at(es: &[u8], start: usize) -> Result<SequenceHeaderGeometry> {
    let body_end = section_body(es, start).len();
    let header_end = start + SEQUENCE_HEADER_CODE.len() + body_end;
    let (width, height, aspect, frame_rate) =
        parse_sequence_header_body(&es[start + SEQUENCE_HEADER_CODE.len()..header_end])?;
    let mut geometry = SequenceHeaderGeometry {
        coded_width: width,
        coded_height: height,
        aspect_ratio_information: aspect,
        frame_rate_code: frame_rate,
        display: None,
    };
    // Walk the extensions up to `sequence_section_end`, following the
    // extension ids in the order H.262 gives: sequence_extension() first,
    // sequence_display_extension() last, both inside
    // `extension_and_user_data(0)`.
    let scan_end = start.saturating_add(SEQUENCE_SECTION_MAX);
    let mut from = header_end;
    while from < es.len() && from < scan_end {
        let Some((at, code)) = find_any_start_code(es, from) else {
            break;
        };
        if code != EXTENSION_START_CODE_BYTE || at >= scan_end {
            break;
        }
        let body = section_body(es, at);
        let Some(&id) = body.first() else {
            break;
        };
        match id >> EXTENSION_ID_SHIFT {
            SEQUENCE_EXTENSION_ID => {
                if let Ok((hx, vx)) = parse_sequence_extension_body(body) {
                    geometry.coded_width |= hx << SIZE_EXTENSION_SHIFT;
                    geometry.coded_height |= vx << SIZE_EXTENSION_SHIFT;
                }
            }
            SEQUENCE_DISPLAY_EXTENSION_ID if geometry.display.is_none() => {
                geometry.display = parse_sequence_display_extension_body(body).ok();
            }
            _ => {}
        }
        from = at + SEQUENCE_HEADER_CODE.len() + body.len().max(1);
    }
    Ok(geometry)
}

/// First `sequence_header()` in an MPEG-2 elementary-stream prefix.
pub fn find_sequence_header(es: &[u8]) -> Option<usize> {
    let mut from = 0usize;
    while let Some((at, code)) = find_any_start_code(es, from) {
        if code == SEQUENCE_HEADER_CODE_BYTE {
            return Some(at);
        }
        from = at + 1;
    }
    None
}

/// Parse an `afd_data()` user structure (ETSI TS 101 154 Table B.2) from the
/// bytes that follow a `'DTG1'` `user_identifier` in MPEG-2 picture user data:
/// `zero(1) active_format_flag(1) alignment_bits(6) [reserved(4)
/// active_format(4)]`.
///
/// `Some(Some(afd))` when the flag is set, `Some(None)` when it is clear,
/// `None` when the payload is too short (silently discarded, §B.6).
pub fn afd_from_mpeg2_user_data(payload: &[u8]) -> Option<Option<ActiveFormatDescription>> {
    let first = *payload.first()?;
    let active_format_flag = first & AFD_ACTIVE_FORMAT_FLAG_BIT != 0;
    if !active_format_flag {
        return Some(None);
    }
    let second = *payload.get(1)?;
    Some(Some(ActiveFormatDescription::from_code(
        second & AFD_CODE_MASK,
    )))
}

/// Split an MPEG-2 access unit into its `extension_and_user_data(2)` user-data
/// payloads that follow a picture header (each payload starts at the
/// `user_identifier`), plus whether a `sequence_header()` is present.
///
/// A `user_data_start_code` before the first `picture_start_code` belongs to
/// `extension_and_user_data(0)` (the sequence), not to a picture, and is not
/// returned (ETSI TS 101 154 §B.6.1: AFD/bar data are picture-level).
pub fn picture_user_data_payloads(au: &[u8]) -> (bool, alloc::vec::Vec<&[u8]>) {
    let mut has_sequence_header = false;
    // AFD sits before the picture's slices: `extension_and_user_data(2)`.
    // Before this access unit's first `picture_header()` any user data is
    // `extension_and_user_data(0)` and belongs to the sequence.
    let mut picture_seen = false;
    let mut payloads = alloc::vec::Vec::new();
    let mut from = 0usize;
    while let Some((at, code)) = find_any_start_code(au, from) {
        match code {
            SEQUENCE_HEADER_CODE_BYTE => has_sequence_header = true,
            PICTURE_START_CODE_BYTE | SLICE_FIRST..=SLICE_LAST => picture_seen = true,
            USER_DATA_START_CODE_BYTE if picture_seen => {
                payloads.push(section_body(au, at));
            }
            _ => {}
        }
        from = at + SEQUENCE_HEADER_CODE.len();
    }
    (has_sequence_header, payloads)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;
    use test_fixtures::bits::BitWriter;

    fn sequence_header(width: u64, height: u64, aspect: u64) -> Vec<u8> {
        let mut w = BitWriter::new();
        w.u(12, width)
            .u(12, height)
            .u(4, aspect)
            .u(4, 3) // frame_rate_code
            .u(18, 0x3FFFF) // bit_rate_value
            .u(1, 1) // marker_bit
            .u(10, 112) // vbv_buffer_size_value
            .u(1, 0) // constrained_parameters_flag
            .u(1, 0) // load_intra_quantiser_matrix
            .u(1, 0); // load_non_intra_quantiser_matrix
        let mut out = SEQUENCE_HEADER_CODE.to_vec();
        out.extend_from_slice(&w.finish());
        out
    }

    fn sequence_extension(hx: u64, vx: u64) -> Vec<u8> {
        let mut w = BitWriter::new();
        w.u(4, u64::from(SEQUENCE_EXTENSION_ID))
            .u(8, 0x48) // profile_and_level_indication: Main @ Main
            .u(1, 0) // progressive_sequence
            .u(2, 1) // chroma_format: 4:2:0
            .u(2, hx)
            .u(2, vx)
            .u(12, 0); // bit_rate_extension
        let mut out = vec![0x00, 0x00, 0x01, EXTENSION_START_CODE_BYTE];
        out.extend_from_slice(&w.finish());
        out
    }

    fn sequence_display_extension(
        video_format: u64,
        colour: Option<(u64, u64, u64)>,
        width: u64,
        height: u64,
    ) -> Vec<u8> {
        let mut w = BitWriter::new();
        w.u(4, u64::from(SEQUENCE_DISPLAY_EXTENSION_ID))
            .u(3, video_format)
            .u(1, u64::from(colour.is_some()));
        if let Some((p, t, m)) = colour {
            w.u(8, p).u(8, t).u(8, m);
        }
        w.u(14, width).u(1, 1).u(14, height);
        let mut out = vec![0x00, 0x00, 0x01, EXTENSION_START_CODE_BYTE];
        out.extend_from_slice(&w.finish());
        out
    }

    fn geometry(parts: &[Vec<u8>]) -> SequenceHeaderGeometry {
        let es: Vec<u8> = parts.concat();
        let start = find_sequence_header(&es).expect("a sequence_header()");
        parse_sequence_at(&es, start).expect("geometry")
    }

    #[test]
    fn aspect_codes_derive_the_table_6_3_ratios() {
        assert_eq!(
            geometry(&[sequence_header(704, 480, 1)]).sample_aspect_ratio(),
            Some((1, 1))
        );
        assert_eq!(
            geometry(&[sequence_header(704, 480, 2)]).sample_aspect_ratio(),
            Some((10, 11))
        );
        assert_eq!(
            geometry(&[sequence_header(704, 576, 3)]).sample_aspect_ratio(),
            Some((16, 11))
        );
        assert_eq!(
            geometry(&[sequence_header(720, 576, 3)]).sample_aspect_ratio(),
            Some((64, 45))
        );
        assert_eq!(
            geometry(&[sequence_header(720, 576, 2)]).sample_aspect_ratio(),
            Some((16, 15))
        );
        // 2.21:1 on 720x576: (221/100) * 576 / 720 = 1768/1000 = 221/125.
        assert_eq!(
            geometry(&[sequence_header(720, 576, 4)]).sample_aspect_ratio(),
            Some((221, 125))
        );
    }

    #[test]
    fn forbidden_and_reserved_aspects_are_unspecified() {
        for aspect in core::iter::once(0u64).chain(u64::from(FIRST_RESERVED_ASPECT)..16) {
            let g = geometry(&[sequence_header(704, 480, aspect)]);
            assert_eq!(g.sample_aspect_ratio(), None, "aspect {aspect}");
        }
    }

    /// The §6.3.3 relation has no ratio when either size term is zero, and the
    /// `u16` wire terms bound each reduced component inclusively at 65535.
    #[test]
    fn derived_sar_zero_terms_and_the_u16_bound() {
        assert_eq!(derived_sar((4, 3), 0, 480), None, "zero width");
        assert_eq!(derived_sar((4, 3), 480, 0), None, "zero height");
        assert_eq!(derived_sar((65535, 1), 1, 1), Some((65535, 1)));
        assert_eq!(derived_sar((1, 65535), 1, 1), Some((1, 65535)));
        assert_eq!(derived_sar((65536, 1), 1, 1), None, "width over u16");
        assert_eq!(derived_sar((1, 65536), 1, 1), None, "height over u16");
        assert_eq!(
            derived_sar((65536, 65536), 1, 1),
            Some((1, 1)),
            "reduced before the bound"
        );
    }

    #[test]
    fn frame_rate_codes_map_to_the_table_6_4_rates() {
        // `sequence_header()` writes `frame_rate_code` 3 (25 Hz).
        let g = geometry(&[sequence_header(704, 480, 2)]);
        assert_eq!(g.frame_rate_code(), 3);
        assert_eq!(g.frame_rate(), Some((25, 1)));
        // 29.97 stays exact rather than 29.97.
        let g = geometry(&[sequence_header(704, 480, 2), sequence_extension(0, 0)]);
        assert_eq!(g.frame_rate(), Some((25, 1)));
    }

    /// Table 6-4 codes 0 and 9..=15 are reserved: no rate is stated.
    #[test]
    fn reserved_frame_rate_codes_state_no_rate() {
        let mut w = BitWriter::new();
        w.u(12, 720)
            .u(12, 576)
            .u(4, 3)
            .u(4, 9) // reserved frame_rate_code
            .u(18, 0x3FFFF)
            .u(1, 1)
            .u(10, 112)
            .u(1, 0)
            .u(1, 0)
            .u(1, 0);
        let mut es = SEQUENCE_HEADER_CODE.to_vec();
        es.extend_from_slice(&w.finish());
        let g = parse_sequence_at(&es, 0).expect("geometry");
        assert_eq!(g.frame_rate_code(), 9);
        assert_eq!(g.frame_rate(), None);
    }

    #[test]
    fn sequence_extension_extends_the_coded_size() {
        let g = geometry(&[sequence_header(111, 222, 1), sequence_extension(1, 2)]);
        assert_eq!(
            (g.coded_width(), g.coded_height()),
            (1 << 12 | 111, 2 << 12 | 222)
        );
    }

    /// The extension walk steps past each section by its own length, so a
    /// `sequence_display_extension()` after a `sequence_extension()` is reached;
    /// and the first display extension wins over a second one.
    #[test]
    fn first_display_extension_after_a_sequence_extension_wins() {
        let es: Vec<u8> = [
            sequence_header(111, 222, 1),
            sequence_extension(1, 2),
            sequence_display_extension(1, Some((6, 6, 5)), 704, 480),
            sequence_display_extension(2, None, 640, 360),
        ]
        .concat();
        let g = parse_sequence_at(&es, 0).expect("geometry");
        assert_eq!(
            (g.coded_width(), g.coded_height()),
            (1 << 12 | 111, 2 << 12 | 222)
        );
        let display = g.display().expect("the first display extension is kept");
        assert_eq!(display.video_format(), 1);
        assert_eq!(display.colour_description(), Some((6, 6, 5)));
        assert_eq!(display.display_horizontal_size(), 704);
        assert_eq!(display.display_vertical_size(), 480);
    }

    #[test]
    fn display_extension_size_feeds_the_spec_relation() {
        // H.262 §6.3.3: with sequence_display_extension() present the relation
        // uses display_horizontal_size / display_vertical_size.
        let g = geometry(&[
            sequence_header(720, 576, 3),
            sequence_display_extension(5, None, 704, 576),
        ]);
        assert_eq!(g.coded_width(), 720);
        // (16/9) * 576 / 704 = 16/11.
        assert_eq!(g.sample_aspect_ratio(), Some((16, 11)));
    }

    /// `extension_and_user_data(2)` user data is returned only after the
    /// picture header, and the sequence-header flag reports a `sequence_header()`
    /// in the access unit (it gates the sequence-level versus picture-level
    /// split).
    #[test]
    fn picture_user_data_payloads_reports_the_sequence_header() {
        let picture: [u8; 8] = [
            0x00,
            0x00,
            0x01,
            PICTURE_START_CODE_BYTE,
            0x00,
            0x08,
            0x00,
            0x40,
        ];
        let user_data: [u8; 8] = [
            0x00,
            0x00,
            0x01,
            USER_DATA_START_CODE_BYTE,
            b'D',
            b'T',
            b'G',
            b'1',
        ];

        let mut with_header = sequence_header(704, 480, 2);
        with_header.extend_from_slice(&picture);
        with_header.extend_from_slice(&user_data);
        let (has_sequence_header, payloads) = picture_user_data_payloads(&with_header);
        assert!(has_sequence_header, "a sequence_header() is present");
        assert_eq!(payloads.len(), 1);
        assert_eq!(payloads[0], b"DTG1");

        let without_header: Vec<u8> = [picture.to_vec(), user_data.to_vec()].concat();
        let (has_sequence_header, payloads) = picture_user_data_payloads(&without_header);
        assert!(
            !has_sequence_header,
            "no sequence_header() in the access unit"
        );
        assert_eq!(payloads.len(), 1);
        assert_eq!(payloads[0], b"DTG1");
    }

    /// An MPEG-2 access unit whose user data precedes the picture header is
    /// sequence-level and is not returned as picture AFD/bar data.
    #[test]
    fn user_data_before_the_picture_header_is_not_picture_user_data() {
        let user_data: [u8; 8] = [
            0x00,
            0x00,
            0x01,
            USER_DATA_START_CODE_BYTE,
            b'D',
            b'T',
            b'G',
            b'1',
        ];
        let picture: [u8; 8] = [
            0x00,
            0x00,
            0x01,
            PICTURE_START_CODE_BYTE,
            0x00,
            0x08,
            0x00,
            0x40,
        ];
        let au: Vec<u8> = [user_data.to_vec(), picture.to_vec()].concat();
        let (_, payloads) = picture_user_data_payloads(&au);
        assert!(payloads.is_empty(), "{payloads:?}");
    }

    /// The `afd_data()` structure after a `'DTG1'` identifier: the flag gates
    /// the code, and a short payload is discarded.
    #[test]
    fn afd_user_data_reads_the_flag_and_code() {
        assert_eq!(
            afd_from_mpeg2_user_data(&[0x40, 0xF4]),
            Some(Some(ActiveFormatDescription::BoxWiderThan16x9Centre))
        );
        assert_eq!(afd_from_mpeg2_user_data(&[0x00, 0xFF]), Some(None));
        assert_eq!(afd_from_mpeg2_user_data(&[0x40]), None);
        assert_eq!(afd_from_mpeg2_user_data(&[]), None);
    }

    /// The FIRST sequence header a real capture carries still parses to the
    /// capture's own geometry (the first header of a stream is the one its
    /// opening pictures use). It is a weaker oracle than the demux-level tests:
    /// a capture may open with a sequence header that differs from the one a
    /// later picture uses.
    #[test]
    fn real_capture_sequence_header_matches_the_ffprobe_oracle() {
        for (fixture, expect) in [
            // ffprobe: 704x576 16:11 (SAR 16:11), display extension 704x576.
            (
                "suites/tsduck/input/test-205.ts",
                (704u32, 576u32, (16u16, 11u16)),
            ),
            ("ts/legacy/mpeg2-mp2.ts", (320, 240, (1, 1))),
        ] {
            let Some(ts) = test_fixtures::require_bytes(fixture, "h262") else {
                return;
            };
            let start = find_sequence_header(&ts).expect("a sequence_header()");
            let g = parse_sequence_at(&ts, start).expect("geometry");
            assert_eq!(
                (g.coded_width(), g.coded_height(), g.sample_aspect_ratio()),
                (expect.0, expect.1, Some(expect.2)),
                "{fixture}"
            );
        }
    }
}
