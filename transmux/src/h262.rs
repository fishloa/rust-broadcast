//! MPEG-2 video (ITU-T H.262 | ISO/IEC 13818-2) start-code walk: the
//! sequence-level display syntax that [`DisplayGeometry`] needs and the
//! picture-level [`extension_and_user_data(2)`] wrapper that carries AFD and
//! bar data (Phase 3b of `docs/display-geometry/DESIGN.md`).
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
//!   Annex B.6 (`docs/codec/afd-bar-data.md` §4.4).
//!
//! Every syntax element above was page-checked against the rendered PDF and
//! is transcribed in `transmux/docs/codec/mpeg2-video-display.md`.
//!
//! [`extension_and_user_data(2)`]: SequenceDisplayExtension
//! [`DisplayGeometry`]: crate::ir::DisplayGeometry

use crate::annexb::find_start_code_prefix;
use crate::bitreader::BitReader;
use crate::error::{Error, Result};
use crate::ir::{
    AspectRatioIdc, ColourDescription, ColourPrimaries, DisplayGeometry, MatrixCoefficients,
    PictureRect, TransferCharacteristics, VideoFormat, VideoSignalType,
};
use crate::mpeg_legacy::SEQUENCE_HEADER_CODE;

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

/// Longest start-code-delimited section scanned for a `sequence_header()`
/// plus its extensions. A `sequence_header()` with both quantiser matrices is
/// at most this size; the scan stops at the next start code anyway.
const SEQUENCE_SECTION_MAX: usize = 256;

/// H.262 §6.3.3: the "Display Aspect Ratio" Table 6-3 names for its codes,
/// as width:height (the notation caveat is in `docs/codec/mpeg2-video-display.md`).
fn dar_ratio(aspect: u8) -> Option<(u64, u64)> {
    match aspect {
        ASPECT_DAR_4_3 => Some((4, 3)),
        ASPECT_DAR_16_9 => Some((16, 9)),
        ASPECT_DAR_2_21 => Some((221, 100)),
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
/// does not fit the `u16` wire terms the [`AspectRatioIdc::ExtendedSar`] model
/// uses, or when a size term is zero.
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
    /// Coded picture width: `(horizontal_size_extension << 12) | horizontal_size_value`.
    pub coded_width: u32,
    /// Coded picture height: `(vertical_size_extension << 12) | vertical_size_value`.
    pub coded_height: u32,
    /// Table 6-3 `aspect_ratio_information` as signalled.
    pub aspect_ratio_information: u8,
    /// `sequence_display_extension()`, when present.
    pub display: Option<SequenceDisplayExtension>,
}

/// H.262 §6.2.2.4 `sequence_display_extension()` fields that describe display.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SequenceDisplayExtension {
    /// Table 6-6 `video_format`.
    pub video_format: u8,
    /// `colour_primaries`, `transfer_characteristics` and `matrix_coefficients`
    /// as signalled (absent when `colour_description == 0`).
    pub colour_description: Option<(u8, u8, u8)>,
    /// "Intended display" active rectangle width in samples (Table 6-3's
    /// `sequence_display_extension()` case).
    pub display_horizontal_size: u32,
    /// "Intended display" active rectangle height in lines.
    pub display_vertical_size: u32,
}

impl SequenceHeaderGeometry {
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

    /// [`DisplayGeometry`] for this sequence. `output_rect` is the coded
    /// picture: H.262 has no crop syntax, and `display_*_size` describes the
    /// intended display rectangle, never a decoder crop.
    ///
    /// Colour: `video_signal` records the `sequence_display_extension()` exactly
    /// as signalled; the MPEG-2 inferred colour (BT.709, 1/1/1) is supplied by
    /// [`DisplayGeometry::colour_description`] through
    /// [`DisplayGeometry::is_mpeg2`]. MPEG-2 has no full-range flag, so
    /// `full_range` is always `false`.
    pub fn to_display_geometry(&self) -> DisplayGeometry {
        let video_signal = self.display.as_ref().map(|d| VideoSignalType {
            video_format: VideoFormat::from_wire(d.video_format)
                .unwrap_or(VideoFormat::Unspecified),
            full_range: false,
            colour: d.colour_description.map(|(p, t, m)| ColourDescription {
                primaries: ColourPrimaries::from_wire(p),
                transfer: TransferCharacteristics::from_wire(t),
                matrix: MatrixCoefficients::from_wire(m),
            }),
        });
        DisplayGeometry {
            coded_width: self.coded_width,
            coded_height: self.coded_height,
            output_rect: PictureRect::new(0, 0, self.coded_width, self.coded_height),
            default_display_rect: None,
            aspect_ratio: match (self.aspect_ratio_information, self.sample_aspect_ratio()) {
                (ASPECT_SQUARE, _) => Some(AspectRatioIdc::Square),
                (_, Some((sar_width, sar_height))) => Some(AspectRatioIdc::ExtendedSar {
                    sar_width,
                    sar_height,
                }),
                _ => None,
            },
            overscan_appropriate: None,
            video_signal,
            mpeg2: true,
        }
    }
}

/// Byte offset of the next `0x000001 <code>` start code at or after `from`,
/// whichever code it is.
fn find_any_start_code(data: &[u8], from: usize) -> Option<(usize, u8)> {
    let at = find_start_code_prefix(data, from)?;
    Some((at, *data.get(at + 3)?))
}

/// The body of the start-code section that begins at `start` (after the
/// 4-byte start code), up to the next start code or the end of `data`.
pub(crate) fn section_body(data: &[u8], start: usize) -> &[u8] {
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
fn parse_sequence_header_body(body: &[u8]) -> Result<(u32, u32, u8)> {
    let mut r = BitReader::from_rbsp(body, "MPEG-2 sequence_header")?;
    let width = r.read_bits(SIZE_VALUE_BITS, "horizontal_size_value")?;
    let height = r.read_bits(SIZE_VALUE_BITS, "vertical_size_value")?;
    let aspect = r.read_bits(ASPECT_BITS, "aspect_ratio_information")?;
    Ok((
        u32::try_from(width).unwrap_or(u32::MAX),
        u32::try_from(height).unwrap_or(u32::MAX),
        u8::try_from(aspect).unwrap_or(u8::MAX),
    ))
}

/// Parse a `sequence_extension()` body: the `(horizontal_size_extension,
/// vertical_size_extension)` pair.
fn parse_sequence_extension_body(body: &[u8]) -> Result<(u32, u32)> {
    let mut r = BitReader::from_rbsp(body, "MPEG-2 sequence_extension")?;
    let id = r.read_bits(EXTENSION_ID_BITS, "extension_start_code_identifier")?;
    debug_assert_eq!(u8::try_from(id).unwrap_or(u8::MAX), SEQUENCE_EXTENSION_ID);
    // profile_and_level_indication(8) progressive_sequence(1) chroma_format(2)
    let _ = r.read_bits(8, "profile_and_level_indication")?;
    let _ = r.read_flag("progressive_sequence")?;
    let _ = r.read_bits(2, "chroma_format")?;
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
        let p = r.read_bits(8, "colour_primaries")?;
        let t = r.read_bits(8, "transfer_characteristics")?;
        let m = r.read_bits(8, "matrix_coefficients")?;
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
/// display description.
pub(crate) fn parse_sequence_at(es: &[u8], start: usize) -> Result<SequenceHeaderGeometry> {
    let body_end = section_body(es, start).len();
    let header_end = start + SEQUENCE_HEADER_CODE.len() + body_end;
    let (width, height, aspect) =
        parse_sequence_header_body(&es[start + SEQUENCE_HEADER_CODE.len()..header_end])?;
    let mut geometry = SequenceHeaderGeometry {
        coded_width: width,
        coded_height: height,
        aspect_ratio_information: aspect,
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
        match id >> (8 - EXTENSION_ID_BITS) {
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
pub(crate) fn find_sequence_header(es: &[u8]) -> Option<usize> {
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
pub(crate) fn afd_from_mpeg2_user_data(
    payload: &[u8],
) -> Option<Option<crate::ir::ActiveFormatDescription>> {
    let first = *payload.first()?;
    let active_format_flag = first & crate::user_data::AFD_ACTIVE_FORMAT_FLAG_BIT != 0;
    if !active_format_flag {
        return Some(None);
    }
    let second = *payload.get(1)?;
    Some(Some(crate::ir::ActiveFormatDescription::from_code(
        second & crate::user_data::AFD_CODE_MASK,
    )))
}

/// Split an MPEG-2 access unit into its `extension_and_user_data(2)` user-data
/// payloads that follow a picture header (each payload starts at the
/// `user_identifier`), plus whether a `sequence_header()` is present.
///
/// A `user_data_start_code` before the first `picture_start_code` belongs to
/// `extension_and_user_data(0)` (the sequence), not to a picture, and is not
/// returned (ETSI TS 101 154 §B.6.1: AFD/bar data are picture-level).
pub(crate) fn picture_user_data_payloads(au: &[u8]) -> (bool, alloc::vec::Vec<&[u8]>) {
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

impl DisplayGeometry {
    /// Geometry of an MPEG-2 video elementary-stream prefix: the first
    /// `sequence_header()` it contains (H.262 §6.2.2.1) plus the
    /// `sequence_extension()` / `sequence_display_extension()` that follow it.
    ///
    /// The input may be a raw elementary-stream prefix or an `esds`
    /// DecoderSpecificInfo that carries the sequence header
    /// (`docs/codec/mpeg2-video-h262.md`). [`Error::InvalidInput`] when no
    /// sequence header is present; nothing is fabricated for a truncated one.
    ///
    /// `aspect_ratio` is [`AspectRatioIdc::ExtendedSar`] carrying the reduced
    /// SAR derived from Table 6-3 and H.262 §6.3.3 ([`AspectRatioIdc::Square`]
    /// for square samples), or `None` for aspect codes 0 and 5..=15.
    pub fn from_mpeg2_es(prefix: &[u8]) -> Result<Self> {
        let start = find_sequence_header(prefix).ok_or(Error::InvalidInput(
            "no MPEG-2 sequence_header (start code 0x000001B3) in elementary stream",
        ))?;
        let sequence = parse_sequence_at(prefix, start)?;
        Ok(sequence.to_display_geometry())
    }
}

#[cfg(test)]
mod tests;
