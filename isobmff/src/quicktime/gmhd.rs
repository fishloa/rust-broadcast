//! The QuickTime base media information header (`gmhd`) — read only.
//!
//! Source: `private/specs-verified/apple/isobmff/quicktime-file-format-online.md`,
//! "Base media information header atom ('gmhd')", "Base media info atom" and
//! "Timecode media information atom ('tcmi')"; the 2001 transcription carries
//! the same field tables.

use alloc::vec::Vec;
use broadcast_common::Parse;
use broadcast_common::len::need_in;
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
/// Bytes of a `tcmi` body after its FullBox header, before the font name.
const TCMI_BODY_LEN: usize = 18;
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
    pub opcolor: [u16; 3],
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
            graphics_mode: u16_at(body, FULLBOX_EXTRA_SIZE),
            opcolor: [
                u16_at(body, FULLBOX_EXTRA_SIZE + 2),
                u16_at(body, FULLBOX_EXTRA_SIZE + 4),
                u16_at(body, FULLBOX_EXTRA_SIZE + 6),
            ],
            balance: i16_at(body, FULLBOX_EXTRA_SIZE + 8),
            reserved: u16_at(body, FULLBOX_EXTRA_SIZE + 10),
        })
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
    pub text_color: [u16; 3],
    /// Background colour, red/green/blue.
    pub background_color: [u16; 3],
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
        let name_len = usize::from(body[base + TCMI_BODY_LEN]);
        let name_at = base + TCMI_BODY_LEN + 1;
        let font_name = body
            .get(name_at..name_at.saturating_add(name_len))
            .map(<[u8]>::to_vec)
            .unwrap_or_default();
        Ok(Self {
            version,
            flags,
            text_font: u16_at(body, base),
            text_face: u16_at(body, base + 2),
            text_size: u16_at(body, base + 4),
            text_color: [
                u16_at(body, base + 6),
                u16_at(body, base + 8),
                u16_at(body, base + 10),
            ],
            background_color: [
                u16_at(body, base + 12),
                u16_at(body, base + 14),
                u16_at(body, base + 16),
            ],
            font_name,
        })
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
    /// Every other child, verbatim.
    pub others: Vec<OpaqueBox>,
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
        let mut others = Vec::new();
        for item in BoxIter::clamped(body) {
            let Ok(item) = item else {
                break;
            };
            match item.four_cc() {
                GMIN => gmin = Some(GminAtom::parse(item.bytes)?),
                TEXT => text = Some(OpaqueBox::new(item.four_cc(), item.payload().to_vec())),
                TCMI => timecode = Some(TcmiAtom::parse(item.bytes)?),
                TMCD => {
                    if let Some(inner) = inner_tcmi(item.payload())? {
                        timecode = Some(inner);
                    } else {
                        others.push(OpaqueBox::new(item.four_cc(), item.payload().to_vec()));
                    }
                }
                other => others.push(OpaqueBox::new(other, item.payload().to_vec())),
            }
        }
        Ok(Self {
            gmin,
            text,
            timecode,
            others,
        })
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
