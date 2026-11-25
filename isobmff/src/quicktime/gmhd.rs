//! The QuickTime base media information header (`gmhd`) — read and written.
//!
//! Source: `private/specs-verified/apple/isobmff/quicktime-file-format-online.md`,
//! "Base media information header atom ('gmhd')", "Base media info atom" and
//! "Timecode media information atom ('tcmi')"; the 2001 transcription carries
//! the same field tables.

use alloc::vec::Vec;
use broadcast_common::Parse;
use broadcast_common::Serialize;
use broadcast_common::len::{fit_u8, need_in, need_out};
use broadcast_common::wire::{i16_at, u16_at, u24_at};

use crate::box_support::OpaqueBox;
use crate::box_types::{BOX_HEADER_MIN_SIZE, BOX_TYPE_SIZE, BoxIter, FULLBOX_EXTRA_SIZE};
use crate::error::{Error, Result};

/// Four-CC of the base media information header.
pub const GMHD: [u8; BOX_TYPE_SIZE] = *b"gmhd";
/// Four-CC of the base media info atom.
pub const GMIN: [u8; BOX_TYPE_SIZE] = *b"gmin";
/// Four-CC of the timecode media information atom.
pub const TCMI: [u8; BOX_TYPE_SIZE] = *b"tcmi";
/// Four-CC of the wrapper ffmpeg writes around `tcmi`.
pub const TMCD: [u8; BOX_TYPE_SIZE] = *b"tmcd";
/// Four-CC of the text media information atom, kept opaque.
pub const TEXT: [u8; BOX_TYPE_SIZE] = *b"text";

/// Bytes of a `gmin` body after its FullBox header.
const GMIN_BODY_LEN: usize = 12;
/// Bytes of a `tcmi` body after its FullBox header through the length byte.
const TCMI_BODY_LEN: usize = 21;
/// An RGB triple's element count.
const COLOR_LEN: usize = 3;
/// Offset of the graphics mode.
const GMIN_GRAPHICS_MODE_OFFSET: usize = 0;
/// Offset of the red opcolour.
const GMIN_OPCOLOR_RED_OFFSET: usize = 2;
/// Offset of the green opcolour.
const GMIN_OPCOLOR_GREEN_OFFSET: usize = 4;
/// Offset of the blue opcolour.
const GMIN_OPCOLOR_BLUE_OFFSET: usize = 6;
/// Offset of the sound balance.
const GMIN_BALANCE_OFFSET: usize = 8;
/// Offset of the reserved field.
const GMIN_RESERVED_OFFSET: usize = 10;
/// Offset of the text font.
const TCMI_FONT_OFFSET: usize = 0;
/// Offset of the text face.
const TCMI_FACE_OFFSET: usize = 2;
/// Offset of the text size.
const TCMI_SIZE_OFFSET: usize = 4;
/// Offset of the red text colour.
const TCMI_TEXT_RED_OFFSET: usize = 6;
/// Offset of the green text colour.
const TCMI_TEXT_GREEN_OFFSET: usize = 8;
/// Offset of the blue text colour.
const TCMI_TEXT_BLUE_OFFSET: usize = 10;
/// Offset of the red background colour.
const TCMI_BACKGROUND_RED_OFFSET: usize = 12;
/// Offset of the green background colour.
const TCMI_BACKGROUND_GREEN_OFFSET: usize = 14;
/// Offset of the blue background colour.
const TCMI_BACKGROUND_BLUE_OFFSET: usize = 16;
/// Offset of the reserved 16-bit field.
const TCMI_RESERVED_OFFSET: usize = 18;
/// Offset of the font name's length byte.
const TCMI_NAME_LEN_OFFSET: usize = 20;
/// Bytes of the fields after a `tcmi` FullBox header, before the name length.
const TCMI_FIELDS_LEN: usize = 20;
/// Bytes of a `tcmi` header, FullBox fields and name length, before the name.
const TCMI_MIN_BOX_LEN: usize = BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + TCMI_FIELDS_LEN + 1;
/// Bytes of a `tmcd` wrapper around a `tcmi` box.
const TMCD_WRAPPER_LEN: usize = BOX_HEADER_MIN_SIZE;
/// Bytes a 16-bit field occupies.
const U16_LEN: usize = core::mem::size_of::<u16>();
/// The `gmin` base media info atom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct GminAtom {
    /// FullBox version.
    pub version: u8,
    /// FullBox flags.
    pub flags: u32,
    /// QuickDraw transfer mode.
    pub graphics_mode: u16,
    /// Red, green and blue opcolour values.
    pub opcolor: [u16; COLOR_LEN],
    /// Sound balance.
    pub balance: i16,
    /// Reserved.
    pub reserved: u16,
}

impl<'a> Parse<'a> for GminAtom {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(
            bytes,
            BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + GMIN_BODY_LEN,
            "gmin",
        )?;
        let body = crate::box_types::box_body(bytes, FULLBOX_EXTRA_SIZE + GMIN_BODY_LEN, "gmin")?;
        Ok(Self {
            version: body[0],
            flags: u24_at(body, 1),
            graphics_mode: u16_at(body, FULLBOX_EXTRA_SIZE + GMIN_GRAPHICS_MODE_OFFSET),
            opcolor: [
                u16_at(body, FULLBOX_EXTRA_SIZE + GMIN_OPCOLOR_RED_OFFSET),
                u16_at(body, FULLBOX_EXTRA_SIZE + GMIN_OPCOLOR_GREEN_OFFSET),
                u16_at(body, FULLBOX_EXTRA_SIZE + GMIN_OPCOLOR_BLUE_OFFSET),
            ],
            balance: i16_at(body, FULLBOX_EXTRA_SIZE + GMIN_BALANCE_OFFSET),
            reserved: u16_at(body, FULLBOX_EXTRA_SIZE + GMIN_RESERVED_OFFSET),
        })
    }
}

impl Serialize for GminAtom {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let n = BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + GMIN_BODY_LEN;
        crate::box_types::box_len(n)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = crate::box_types::write_full_box_header(
            buf,
            &GMIN,
            need,
            self.version,
            self.flags,
            false,
        )?;
        buf[c..c + U16_LEN].copy_from_slice(&self.graphics_mode.to_be_bytes());
        c += U16_LEN;
        for value in self.opcolor {
            buf[c..c + U16_LEN].copy_from_slice(&value.to_be_bytes());
            c += U16_LEN;
        }
        buf[c..c + U16_LEN].copy_from_slice(&self.balance.to_be_bytes());
        c += U16_LEN;
        buf[c..c + U16_LEN].copy_from_slice(&self.reserved.to_be_bytes());
        c += U16_LEN;
        Ok(c)
    }
}

/// The `tcmi` timecode media information atom.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TcmiAtom {
    /// FullBox version.
    pub version: u8,
    /// FullBox flags.
    pub flags: u32,
    /// Text font identifier.
    pub text_font: u16,
    /// Text style bits.
    pub text_face: u16,
    /// Text point size.
    pub text_size: u16,
    /// Text colour, red/green/blue.
    pub text_color: [u16; COLOR_LEN],
    /// Background colour, red/green/blue.
    pub background_color: [u16; COLOR_LEN],
    /// Reserved 16-bit field.
    pub reserved: u16,
    /// Font name bytes (a Pascal string's payload).
    pub font_name: Vec<u8>,
}

impl<'a> Parse<'a> for TcmiAtom {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(
            bytes,
            BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + TCMI_BODY_LEN,
            "tcmi",
        )?;
        let body = crate::box_types::box_body(bytes, FULLBOX_EXTRA_SIZE + TCMI_BODY_LEN, "tcmi")?;
        let version = body[0];
        let flags = u24_at(body, 1);
        let base = FULLBOX_EXTRA_SIZE;
        let name_len = usize::from(body[base + TCMI_NAME_LEN_OFFSET]);
        let name_at = base + TCMI_NAME_LEN_OFFSET + 1;
        let font_name = body
            .get(name_at..name_at.saturating_add(name_len))
            .map(<[u8]>::to_vec)
            .unwrap_or_default();
        Ok(Self {
            version,
            flags,
            text_font: u16_at(body, base + TCMI_FONT_OFFSET),
            text_face: u16_at(body, base + TCMI_FACE_OFFSET),
            text_size: u16_at(body, base + TCMI_SIZE_OFFSET),
            text_color: [
                u16_at(body, base + TCMI_TEXT_RED_OFFSET),
                u16_at(body, base + TCMI_TEXT_GREEN_OFFSET),
                u16_at(body, base + TCMI_TEXT_BLUE_OFFSET),
            ],
            background_color: [
                u16_at(body, base + TCMI_BACKGROUND_RED_OFFSET),
                u16_at(body, base + TCMI_BACKGROUND_GREEN_OFFSET),
                u16_at(body, base + TCMI_BACKGROUND_BLUE_OFFSET),
            ],
            reserved: u16_at(body, base + TCMI_RESERVED_OFFSET),
            font_name,
        })
    }
}

impl Serialize for TcmiAtom {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        crate::box_types::box_len(TCMI_MIN_BOX_LEN + self.font_name.len())
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = crate::box_types::write_full_box_header(
            buf,
            &TCMI,
            need,
            self.version,
            self.flags,
            false,
        )?;
        for value in [self.text_font, self.text_face, self.text_size] {
            buf[c..c + U16_LEN].copy_from_slice(&value.to_be_bytes());
            c += U16_LEN;
        }
        for value in self.text_color.iter().chain(self.background_color.iter()) {
            buf[c..c + U16_LEN].copy_from_slice(&value.to_be_bytes());
            c += U16_LEN;
        }
        buf[c..c + U16_LEN].copy_from_slice(&self.reserved.to_be_bytes());
        c += U16_LEN;
        buf[c] = fit_u8(self.font_name.len(), "tcmi font name length")?;
        c += 1;
        buf[c..c + self.font_name.len()].copy_from_slice(&self.font_name);
        c += self.font_name.len();
        Ok(c)
    }
}

/// A QuickTime base media information header.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct GmhdAtom {
    /// The required `gmin` child.
    pub gmin: Option<GminAtom>,
    /// The `text` child, kept opaque.
    pub text: Option<OpaqueBox>,
    /// The `tcmi` timecode fields, directly or inside ffmpeg's `tmcd`.
    pub timecode: Option<TcmiAtom>,
    /// Whether the wire wrapped `timecode` in ffmpeg's `tmcd` box.
    pub timecode_wrapped: bool,
    /// Every other child, verbatim.
    pub others: Vec<OpaqueBox>,
    /// The children's wire order. Empty means `gmin`, `text`, `timecode`,
    /// then `others`.
    pub order: Vec<[u8; BOX_TYPE_SIZE]>,
}

impl GmhdAtom {
    /// Parse a whole `gmhd` box, header included.
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        let body = crate::box_types::box_body(bytes, 0, "gmhd")?;
        Self::parse_body(body)
    }

    /// Parse a `gmhd` body (after the box header).
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        let mut gmin = None;
        let mut text = None;
        let mut timecode = None;
        let mut timecode_wrapped = false;
        let mut others = Vec::new();
        let mut order = Vec::new();
        for item in BoxIter::clamped(body) {
            let Ok(item) = item else {
                break;
            };
            order.push(item.four_cc());
            match item.four_cc() {
                GMIN => gmin = Some(GminAtom::parse(item.bytes)?),
                TEXT => {
                    text = Some(OpaqueBox {
                        box_type: item.four_cc(),
                        data: item.payload().to_vec(),
                        to_end: item.to_end(),
                        largesize: item.has_largesize(),
                    });
                }
                TCMI => {
                    timecode = Some(TcmiAtom::parse(item.bytes)?);
                    timecode_wrapped = false;
                }
                TMCD => {
                    if let Some(inner) = inner_tcmi(item.payload())? {
                        timecode = Some(inner);
                        timecode_wrapped = true;
                    } else {
                        others.push(OpaqueBox {
                            box_type: item.four_cc(),
                            data: item.payload().to_vec(),
                            to_end: item.to_end(),
                            largesize: item.has_largesize(),
                        });
                    }
                }
                other => others.push(OpaqueBox {
                    box_type: other,
                    data: item.payload().to_vec(),
                    to_end: item.to_end(),
                    largesize: item.has_largesize(),
                }),
            }
        }
        Ok(Self {
            gmin,
            text,
            timecode,
            timecode_wrapped,
            others,
            order,
        })
    }
}

impl Serialize for GmhdAtom {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let mut n = BOX_HEADER_MIN_SIZE;
        if self.order.is_empty() {
            if let Some(gmin) = &self.gmin {
                n += gmin.serialized_len();
            }
            if let Some(text) = &self.text {
                n += text.serialized_len();
            }
            n += self.timecode_len();
            for other in &self.others {
                n += other.serialized_len();
            }
        } else {
            let mut next_other = 0usize;
            for four_cc in &self.order {
                match *four_cc {
                    GMIN => {
                        if let Some(gmin) = &self.gmin {
                            n += gmin.serialized_len();
                        }
                    }
                    TEXT => {
                        if let Some(text) = &self.text {
                            n += text.serialized_len();
                        }
                    }
                    TCMI | TMCD if self.timecode.is_some() => n += self.timecode_len(),
                    _ => {
                        if let Some(other) = self.others.get(next_other) {
                            n += other.serialized_len();
                            next_other += 1;
                        }
                    }
                }
            }
            for other in &self.others[next_other..] {
                n += other.serialized_len();
            }
        }
        crate::box_types::box_len(n)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = crate::box_types::write_box_header(buf, &GMHD, need)?;
        if self.order.is_empty() {
            if let Some(gmin) = &self.gmin {
                c += gmin.serialize_into(&mut buf[c..])?;
            }
            if let Some(text) = &self.text {
                c += text.serialize_into(&mut buf[c..])?;
            }
            c += self.write_timecode(buf, c)?;
            for other in &self.others {
                c += other.serialize_into(&mut buf[c..])?;
            }
        } else {
            let mut next_other = 0usize;
            for four_cc in &self.order {
                match *four_cc {
                    GMIN => {
                        if let Some(gmin) = &self.gmin {
                            c += gmin.serialize_into(&mut buf[c..])?;
                        }
                    }
                    TEXT => {
                        if let Some(text) = &self.text {
                            c += text.serialize_into(&mut buf[c..])?;
                        }
                    }
                    TCMI | TMCD if self.timecode.is_some() => {
                        c += self.write_timecode(buf, c)?;
                    }
                    _ => {
                        if let Some(other) = self.others.get(next_other) {
                            c += other.serialize_into(&mut buf[c..])?;
                            next_other += 1;
                        }
                    }
                }
            }
            for other in &self.others[next_other..] {
                c += other.serialize_into(&mut buf[c..])?;
            }
        }
        Ok(c)
    }
}

impl GmhdAtom {
    fn timecode_len(&self) -> usize {
        match &self.timecode {
            None => 0,
            Some(tcmi) => {
                if self.timecode_wrapped {
                    TMCD_WRAPPER_LEN + tcmi.serialized_len()
                } else {
                    tcmi.serialized_len()
                }
            }
        }
    }

    fn write_timecode(&self, buf: &mut [u8], c: usize) -> Result<usize> {
        match &self.timecode {
            None => Ok(0),
            Some(tcmi) => {
                if self.timecode_wrapped {
                    let need = TMCD_WRAPPER_LEN + tcmi.serialized_len();
                    let mut written =
                        crate::box_types::write_box_header(&mut buf[c..], &TMCD, need)?;
                    written += tcmi.serialize_into(&mut buf[c + written..])?;
                    Ok(written)
                } else {
                    tcmi.serialize_into(&mut buf[c..])
                }
            }
        }
    }
}

/// The `tcmi` child of ffmpeg's `tmcd` wrapper.
fn inner_tcmi(body: &[u8]) -> Result<Option<TcmiAtom>> {
    for item in BoxIter::clamped(body) {
        let Ok(item) = item else {
            return Ok(None);
        };
        if item.four_cc() == TCMI {
            return TcmiAtom::parse(item.bytes).map(Some);
        }
    }
    Ok(None)
}

/// Read a `gmhd` out of a `minf` child box, when the box is one.
#[must_use]
pub fn gmhd_from_box(box_type: [u8; BOX_TYPE_SIZE], body: &[u8]) -> Option<GmhdAtom> {
    if box_type != GMHD {
        return None;
    }
    GmhdAtom::parse_body(body).ok()
}
