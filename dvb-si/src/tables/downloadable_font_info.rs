//! Downloadable Font Information Section (DFIS) — ETSI EN 303 560 v1.1.1
//! §5.3.2.3.1 (table_id 0x7C).
//!
//! The DFIS conveys download location and font metadata for a single font or
//! font family. Sections of a Downloadable Font Information Table (DFIT) are
//! carried together on one PID, signalled in the PMT by a `data_broadcast_id`
//! descriptor with `data_broadcast_id` 0x000D (§5.3.2.3.1) — there is no
//! well-known PID, so [`PID`] follows the `dsmcc.rs` "no fixed PID" convention.
//!
//! ## table_id
//!
//! EN 303 560 v1.1.1 §5.3.2.3.1 says `table_id` 0x4C — an acknowledged
//! allocation accident: 0x4C was already the INT. EN 300 468 V1.19.1 Table 2
//! NOTE 2 ("table_id 0x4C was previously accidentally assigned to both of
//! these two DVB specifications, this has now been corrected") reassigns the
//! DFIS to **0x7C**, which is what [`TABLE_ID`] and the crate registry use.
//!
//! ## The 0x02 conditional (resolved against the PDF, pp. 30-31)
//!
//! Table 22's syntax has two consecutive conditionals that both fire for
//! `font_info_type == 0x02`:
//!
//! ```text
//! if (font_info_type == 0x02) { font_size (16) }
//! if (font_info_type >= 0x02) { font_info_length (8) + text_char ... }
//! ```
//!
//! This is **not** a typo and **not** double-handling: a type-0x02 entry
//! carries the 16-bit `font_size` *followed by* the length-prefixed string
//! block. Every `font_info_type >= 0x02` is length-delimited by
//! `font_info_length` (the 0x02 case additionally prefixes the 2-byte
//! `font_size`). This makes types 0x03 (`font_family`) and all reserved types
//! 0x04..=0xFF safely skippable, so they round-trip as
//! [`FontInfo::LengthDelimited`]. Verified against EN 303 560 v1.1.1 PDF
//! pp. 30-31 (Table 22) and p. 32 (Table 23 type allocation).
//!
//! Per crate contract this parser does NOT verify CRC_32 (use
//! `Section::validate_crc`). Reserved bits are ignored on parse; spec-mandated
//! zero fields (`font_id_extension`, `reserved_zero_future_use`) are emitted 0.

use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// table_id for the DFIS — the crate registry value (see module docs re. spec 0x4C).
pub const TABLE_ID: u8 = 0x7C;
/// DFIS has no well-known PID; carried on the PID signalled by the
/// `data_broadcast_id` (0x000D) descriptor in the PMT (§5.3.2.3.1).
pub const PID: u16 = 0x0000;

/// `font_info_type` for style/weight (§5.3.2.3.2.1 Table 23).
pub const FONT_INFO_TYPE_STYLE_WEIGHT: u8 = 0x00;
/// `font_info_type` for a font file URI (Table 23).
pub const FONT_INFO_TYPE_FILE_URI: u8 = 0x01;
/// `font_info_type` for font size in pixels (Table 23).
pub const FONT_INFO_TYPE_FONT_SIZE: u8 = 0x02;

/// table_id(1) + section_length(2) + font_id_extension/font_id(2)
/// + version/cni(1) + section_number(1) + last_section_number(1) = 8-byte header.
const HEADER_LEN: usize = 8;
/// `section_length` counts from just after the field (byte 3) to end of section.
const SECTION_LENGTH_PREFIX: usize = 3;
/// CRC_32 trailer.
const CRC_LEN: usize = 4;

/// One entry in the DFIS font_info loop (§5.3.2.3.1 Table 22).
///
/// Variant is selected by `font_info_type` (Table 23). Reserved types
/// (0x04..=0xFF) are length-delimited and round-trip via [`FontInfo::LengthDelimited`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum FontInfo<'a> {
    /// `font_info_type == 0x00`: style(3) + weight(4) + reserved(1).
    StyleWeight {
        /// `font_style` (§5.3.2.3.2.2 Table 24): 0 undefined, 1 normal, 2 italic, 3 oblique.
        style: u8,
        /// `font_weight` (Table 25): 0 undefined, 1 normal, 2 bold.
        weight: u8,
    },
    /// `font_info_type == 0x01`: reserved(4) + font_file_format(4) + uri_length(8) + uri.
    FileUri {
        /// `font_file_format` (§5.3.2.3.2.3 Table 26): 0 = OFF, 1 = WOFF.
        format: u8,
        /// DVB URI string (UTF-8), `uri_length` bytes.
        uri: &'a [u8],
    },
    /// `font_info_type == 0x02`: font_size(16) followed by the length-delimited block.
    FontSize {
        /// `font_size` — font height in pixels.
        size: u16,
        /// `text_char` block following `font_info_length` (UTF-8).
        info: &'a [u8],
    },
    /// `font_info_type >= 0x03` (incl. reserved): font_info_length(8) + text_char block.
    LengthDelimited {
        /// The `font_info_type` byte as parsed (0x03 = font_family, else reserved).
        font_info_type: u8,
        /// `text_char` block, `font_info_length` bytes (UTF-8 for defined types).
        info: &'a [u8],
    },
}

/// Downloadable Font Information Section (EN 303 560 §5.3.2.3.1, Table 22).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct DownloadableFontInfoSection<'a> {
    /// 9-bit `font_id_extension` — spec-mandated all-zero; together with
    /// `font_id` forms the 16-bit table_id_extension.
    pub font_id_extension: u16,
    /// 7-bit `font_id` identifying the sub_table (one font/family).
    pub font_id: u8,
    /// 5-bit version_number.
    pub version_number: u8,
    /// current_next_indicator bit.
    pub current_next_indicator: bool,
    /// section_number.
    pub section_number: u8,
    /// last_section_number.
    pub last_section_number: u8,
    /// font_info loop entries in wire order.
    pub font_info: Vec<FontInfo<'a>>,
}

impl<'a> Parse<'a> for DownloadableFontInfoSection<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let min_len = HEADER_LEN + CRC_LEN;
        if bytes.len() < min_len {
            return Err(Error::BufferTooShort {
                need: min_len,
                have: bytes.len(),
                what: "DownloadableFontInfoSection",
            });
        }
        if bytes[0] != TABLE_ID {
            return Err(Error::UnexpectedTableId {
                table_id: bytes[0],
                what: "DownloadableFontInfoSection",
                expected: &[TABLE_ID],
            });
        }
        let section_length = (((bytes[1] & 0x0F) as usize) << 8) | bytes[2] as usize;
        let total = super::check_section_length(
            bytes.len(),
            SECTION_LENGTH_PREFIX,
            section_length,
            HEADER_LEN + CRC_LEN,
        )?;

        // bytes[3..5] = font_id_extension(9) | font_id(7).
        let id_word = u16::from_be_bytes([bytes[3], bytes[4]]);
        let font_id_extension = id_word >> 7;
        let font_id = (id_word & 0x7F) as u8;
        let version_number = (bytes[5] >> 1) & 0x1F;
        let current_next_indicator = bytes[5] & 0x01 != 0;
        let section_number = bytes[6];
        let last_section_number = bytes[7];

        let loop_end = total - CRC_LEN;
        let mut font_info = Vec::new();
        let mut pos = HEADER_LEN;
        while pos < loop_end {
            let font_info_type = bytes[pos];
            pos += 1;
            match font_info_type {
                FONT_INFO_TYPE_STYLE_WEIGHT => {
                    if pos + 1 > loop_end {
                        return Err(Error::SectionLengthOverflow {
                            declared: 1,
                            available: loop_end - pos,
                        });
                    }
                    let b = bytes[pos];
                    pos += 1;
                    font_info.push(FontInfo::StyleWeight {
                        style: b >> 5,
                        weight: (b >> 1) & 0x0F,
                    });
                }
                FONT_INFO_TYPE_FILE_URI => {
                    if pos + 2 > loop_end {
                        return Err(Error::SectionLengthOverflow {
                            declared: 2,
                            available: loop_end - pos,
                        });
                    }
                    let format = bytes[pos] & 0x0F;
                    let uri_length = bytes[pos + 1] as usize;
                    let uri_start = pos + 2;
                    let uri_end = uri_start + uri_length;
                    if uri_end > loop_end {
                        return Err(Error::SectionLengthOverflow {
                            declared: uri_length,
                            available: loop_end - uri_start,
                        });
                    }
                    font_info.push(FontInfo::FileUri {
                        format,
                        uri: &bytes[uri_start..uri_end],
                    });
                    pos = uri_end;
                }
                FONT_INFO_TYPE_FONT_SIZE => {
                    // font_size(16) then font_info_length(8) + block (Table 22, type >= 0x02).
                    if pos + 3 > loop_end {
                        return Err(Error::SectionLengthOverflow {
                            declared: 3,
                            available: loop_end - pos,
                        });
                    }
                    let size = u16::from_be_bytes([bytes[pos], bytes[pos + 1]]);
                    let info_length = bytes[pos + 2] as usize;
                    let info_start = pos + 3;
                    let info_end = info_start + info_length;
                    if info_end > loop_end {
                        return Err(Error::SectionLengthOverflow {
                            declared: info_length,
                            available: loop_end - info_start,
                        });
                    }
                    font_info.push(FontInfo::FontSize {
                        size,
                        info: &bytes[info_start..info_end],
                    });
                    pos = info_end;
                }
                _ => {
                    // font_info_type >= 0x03: font_info_length(8) + text_char block.
                    if pos + 1 > loop_end {
                        return Err(Error::SectionLengthOverflow {
                            declared: 1,
                            available: loop_end - pos,
                        });
                    }
                    let info_length = bytes[pos] as usize;
                    let info_start = pos + 1;
                    let info_end = info_start + info_length;
                    if info_end > loop_end {
                        return Err(Error::SectionLengthOverflow {
                            declared: info_length,
                            available: loop_end - info_start,
                        });
                    }
                    font_info.push(FontInfo::LengthDelimited {
                        font_info_type,
                        info: &bytes[info_start..info_end],
                    });
                    pos = info_end;
                }
            }
        }

        Ok(DownloadableFontInfoSection {
            font_id_extension,
            font_id,
            version_number,
            current_next_indicator,
            section_number,
            last_section_number,
            font_info,
        })
    }
}

impl Serialize for DownloadableFontInfoSection<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        let loop_bytes: usize = self
            .font_info
            .iter()
            .map(|f| match f {
                FontInfo::StyleWeight { .. } => 2, // type + 1 packed byte
                FontInfo::FileUri { uri, .. } => 1 + 2 + uri.len(), // type + (fmt|len) + uri
                FontInfo::FontSize { info, .. } => 1 + 2 + 1 + info.len(), // type + size + len + info
                FontInfo::LengthDelimited { info, .. } => 1 + 1 + info.len(), // type + len + info
            })
            .sum();
        HEADER_LEN + loop_bytes + CRC_LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        let section_length = (len - SECTION_LENGTH_PREFIX) as u16;
        buf[0] = TABLE_ID;
        buf[1] = super::SECTION_B1_FLAGS_DVB | ((section_length >> 8) as u8 & 0x0F);
        buf[2] = (section_length & 0xFF) as u8;
        // font_id_extension(9) | font_id(7); spec mandates extension all-zero.
        let id_word = ((self.font_id_extension & 0x01FF) << 7) | (self.font_id as u16 & 0x7F);
        buf[3..5].copy_from_slice(&id_word.to_be_bytes());
        // reserved(2)=11, version_number(5), current_next_indicator(1).
        buf[5] = 0xC0 | ((self.version_number & 0x1F) << 1) | u8::from(self.current_next_indicator);
        buf[6] = self.section_number;
        buf[7] = self.last_section_number;

        // 8-bit length prefixes error on over-range payloads rather than
        // silently truncating (the crate's strict serialize idiom).
        let guard_u8 = |len: usize| -> Result<()> {
            if len > u8::MAX as usize {
                return Err(Error::SectionLengthOverflow {
                    declared: len,
                    available: u8::MAX as usize,
                });
            }
            Ok(())
        };

        let mut pos = HEADER_LEN;
        for f in &self.font_info {
            match f {
                FontInfo::StyleWeight { style, weight } => {
                    buf[pos] = FONT_INFO_TYPE_STYLE_WEIGHT;
                    // style(3) | weight(4) | reserved_zero_future_use(1)=0.
                    buf[pos + 1] = ((style & 0x07) << 5) | ((weight & 0x0F) << 1);
                    pos += 2;
                }
                FontInfo::FileUri { format, uri } => {
                    guard_u8(uri.len())?;
                    buf[pos] = FONT_INFO_TYPE_FILE_URI;
                    // reserved_zero_future_use(4)=0 | font_file_format(4).
                    buf[pos + 1] = format & 0x0F;
                    buf[pos + 2] = uri.len() as u8;
                    let s = pos + 3;
                    buf[s..s + uri.len()].copy_from_slice(uri);
                    pos = s + uri.len();
                }
                FontInfo::FontSize { size, info } => {
                    guard_u8(info.len())?;
                    buf[pos] = FONT_INFO_TYPE_FONT_SIZE;
                    buf[pos + 1..pos + 3].copy_from_slice(&size.to_be_bytes());
                    buf[pos + 3] = info.len() as u8;
                    let s = pos + 4;
                    buf[s..s + info.len()].copy_from_slice(info);
                    pos = s + info.len();
                }
                FontInfo::LengthDelimited {
                    font_info_type,
                    info,
                } => {
                    guard_u8(info.len())?;
                    buf[pos] = *font_info_type;
                    buf[pos + 1] = info.len() as u8;
                    let s = pos + 2;
                    buf[s..s + info.len()].copy_from_slice(info);
                    pos = s + info.len();
                }
            }
        }

        let crc = dvb_common::crc32_mpeg2::compute(&buf[..pos]);
        buf[pos..len].copy_from_slice(&crc.to_be_bytes());
        Ok(len)
    }
}
impl<'a> crate::traits::TableDef<'a> for DownloadableFontInfoSection<'a> {
    const TABLE_ID_RANGES: &'static [(u8, u8)] = &[(TABLE_ID, TABLE_ID)];
    const NAME: &'static str = "DOWNLOADABLE_FONT_INFO";
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wrap a font_info loop body in the 8-byte common header + placeholder CRC.
    fn build_section(font_id: u8, version: u8, loop_body: &[u8]) -> Vec<u8> {
        let section_length =
            (HEADER_LEN - SECTION_LENGTH_PREFIX + loop_body.len() + CRC_LEN) as u16;
        // font_id_extension = 0 (spec-mandated), font_id in low 7 bits.
        let id_word = (font_id as u16) & 0x7F;
        let mut v = vec![
            TABLE_ID,
            super::super::SECTION_B1_FLAGS_DVB | ((section_length >> 8) as u8 & 0x0F),
            (section_length & 0xFF) as u8,
            (id_word >> 8) as u8,
            (id_word & 0xFF) as u8,
            0xC0 | (version << 1) | 0x01,
            0x00,
            0x00,
        ];
        v.extend_from_slice(loop_body);
        v.extend_from_slice(&[0, 0, 0, 0]);
        v
    }

    /// Build a font_info loop with one of each variant.
    fn mixed_loop() -> Vec<u8> {
        let uri = b"https://f.example/Droid.otf";
        let family = b"Droid Sans";
        let mut b = vec![
            FONT_INFO_TYPE_STYLE_WEIGHT, // type 0x00
            (2u8 << 5) | (2u8 << 1),     // style=2 (italic), weight=2 (bold)
            FONT_INFO_TYPE_FILE_URI,     // type 0x01
            0x01,                        // format=1 (WOFF)
            uri.len() as u8,             // uri_length
        ];
        b.extend_from_slice(uri);
        // type 0x02 — font_size=24, info block "px"
        b.push(FONT_INFO_TYPE_FONT_SIZE);
        b.extend_from_slice(&24u16.to_be_bytes());
        b.push(2);
        b.extend_from_slice(b"px");
        // type 0x03 — font_family
        b.push(0x03);
        b.push(family.len() as u8);
        b.extend_from_slice(family);
        b
    }

    #[test]
    fn parse_header_fields() {
        let bytes = build_section(0x42, 9, &[]);
        let sec = DownloadableFontInfoSection::parse(&bytes).unwrap();
        assert_eq!(sec.font_id, 0x42);
        assert_eq!(sec.font_id_extension, 0);
        assert_eq!(sec.version_number, 9);
        assert!(sec.current_next_indicator);
        assert!(sec.font_info.is_empty());
    }

    #[test]
    fn parse_all_variants() {
        let bytes = build_section(1, 0, &mixed_loop());
        let sec = DownloadableFontInfoSection::parse(&bytes).unwrap();
        assert_eq!(sec.font_info.len(), 4);
        assert_eq!(
            sec.font_info[0],
            FontInfo::StyleWeight {
                style: 2,
                weight: 2
            }
        );
        match &sec.font_info[1] {
            FontInfo::FileUri { format, uri } => {
                assert_eq!(*format, 1);
                assert_eq!(*uri, b"https://f.example/Droid.otf");
            }
            other => panic!("expected FileUri, got {other:?}"),
        }
        match &sec.font_info[2] {
            FontInfo::FontSize { size, info } => {
                assert_eq!(*size, 24);
                assert_eq!(*info, b"px");
            }
            other => panic!("expected FontSize, got {other:?}"),
        }
        match &sec.font_info[3] {
            FontInfo::LengthDelimited {
                font_info_type,
                info,
            } => {
                assert_eq!(*font_info_type, 0x03);
                assert_eq!(*info, b"Droid Sans");
            }
            other => panic!("expected LengthDelimited, got {other:?}"),
        }
    }

    #[test]
    fn reserved_type_round_trips_as_length_delimited() {
        // type 0x77 (reserved) is length-delimited and skippable.
        let mut body = vec![0x77u8, 0x03];
        body.extend_from_slice(&[0xAA, 0xBB, 0xCC]);
        let bytes = build_section(1, 0, &body);
        let sec = DownloadableFontInfoSection::parse(&bytes).unwrap();
        assert_eq!(
            sec.font_info[0],
            FontInfo::LengthDelimited {
                font_info_type: 0x77,
                info: &[0xAA, 0xBB, 0xCC]
            }
        );
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let mut bytes = build_section(1, 0, &mixed_loop());
        bytes[0] = 0x4C; // INT table_id
        assert!(matches!(
            DownloadableFontInfoSection::parse(&bytes).unwrap_err(),
            Error::UnexpectedTableId { table_id: 0x4C, .. }
        ));
    }

    #[test]
    fn rejects_short_buffer() {
        assert!(matches!(
            DownloadableFontInfoSection::parse(&[0x7C, 0xB0]).unwrap_err(),
            Error::BufferTooShort {
                what: "DownloadableFontInfoSection",
                ..
            }
        ));
    }

    #[test]
    fn uri_length_overflow_rejected() {
        // type 0x01, uri_length 0x20 but no uri bytes present.
        let body = vec![FONT_INFO_TYPE_FILE_URI, 0x01, 0x20];
        let bytes = build_section(1, 0, &body);
        assert!(matches!(
            DownloadableFontInfoSection::parse(&bytes).unwrap_err(),
            Error::SectionLengthOverflow { .. }
        ));
    }

    #[test]
    fn round_trip_all_variants() {
        let bytes = build_section(0x33, 4, &mixed_loop());
        let sec = DownloadableFontInfoSection::parse(&bytes).unwrap();
        let mut buf = vec![0u8; sec.serialized_len()];
        sec.serialize_into(&mut buf).unwrap();
        let re = DownloadableFontInfoSection::parse(&buf).unwrap();
        assert_eq!(sec, re);
    }

    #[test]
    fn table_trait_constants() {
        assert_eq!(TABLE_ID, 0x7C);
        assert_eq!(PID, 0x0000);
    }

    #[test]
    #[cfg(feature = "serde")]
    fn serde_json_round_trip() {
        let bytes = build_section(1, 0, &mixed_loop());
        let sec = DownloadableFontInfoSection::parse(&bytes).unwrap();
        let j = serde_json::to_string(&sec).unwrap();
        // The borrowed `uri`/`info` `&[u8]` fields cannot be JSON-deserialized
        // zero-copy (serde_json renders them as number sequences, not borrowed
        // byte arrays) — the crate-wide constraint affecting every
        // borrowed-slice table (cf. mpe.rs). Exercise the derive through the
        // WIRE form: a re-parse must serialize to byte-identical JSON.
        let reparsed = DownloadableFontInfoSection::parse(&bytes).unwrap();
        assert_eq!(serde_json::to_string(&reparsed).unwrap(), j);
        assert!(j.contains("\"font_id\":1"));
    }

    #[test]
    fn parse_rejects_zero_section_length() {
        let mut buf = vec![0u8; 64];
        buf[0] = TABLE_ID;
        buf[1] = 0xF0;
        buf[2] = 0x00;
        for b in &mut buf[3..] {
            *b = 0xFF;
        }
        assert!(matches!(
            DownloadableFontInfoSection::parse(&buf).unwrap_err(),
            Error::SectionLengthOverflow { .. }
        ));
    }
}
