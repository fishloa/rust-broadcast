//! The QuickTime track aperture mode dimensions atom (`tapt`) — read only.
//!
//! Source: `private/specs-verified/apple/isobmff/quicktime-file-format-online.md`,
//! "Track aperture mode dimensions atom ('tapt')", "Track clean aperture
//! dimensions atom ('clef')", "Track production aperture dimensions atom
//! ('prof')" and "Track encoded pixels dimensions atom ('enof')".

use broadcast_common::Parse;
use broadcast_common::len::need_in;
use broadcast_common::wire::{u24_at, u32_at};

use crate::box_types::{BOX_HEADER_MIN_SIZE, BOX_TYPE_SIZE, BoxIter, FULLBOX_EXTRA_SIZE};
use crate::error::{Error, Result};

/// Four-CC of the track aperture mode dimensions container.
pub const TAPT: [u8; BOX_TYPE_SIZE] = *b"tapt";
/// Four-CC of the clean aperture dimensions atom.
pub const CLEF: [u8; BOX_TYPE_SIZE] = *b"clef";
/// Four-CC of the production aperture dimensions atom.
pub const PROF: [u8; BOX_TYPE_SIZE] = *b"prof";
/// Four-CC of the encoded pixels dimensions atom.
pub const ENOF: [u8; BOX_TYPE_SIZE] = *b"enof";

/// Bytes of an aperture dimensions body after the FullBox fields.
const DIMENSIONS_BODY_LEN: usize = 8;
/// Offset of the height field after the width field.
const HEIGHT_OFFSET: usize = 4;

/// One aperture's 16.16 width and height.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ApertureDimensions {
    /// FullBox version.
    pub version: u8,
    /// FullBox flags.
    pub flags: u32,
    /// Width in 16.16 fixed point.
    pub width: u32,
    /// Height in 16.16 fixed point.
    pub height: u32,
}

impl<'a> Parse<'a> for ApertureDimensions {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(
            bytes,
            BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + DIMENSIONS_BODY_LEN,
            "aperture dimensions",
        )?;
        let body = crate::box_types::box_body(
            bytes,
            FULLBOX_EXTRA_SIZE + DIMENSIONS_BODY_LEN,
            "aperture dimensions",
        )?;
        Ok(Self {
            version: body[0],
            flags: u24_at(body, 1),
            width: u32_at(body, FULLBOX_EXTRA_SIZE),
            height: u32_at(body, FULLBOX_EXTRA_SIZE + HEIGHT_OFFSET),
        })
    }
}

/// The three aperture mode dimensions of one track.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TrackApertureModeDimensionsBox {
    /// Clean aperture dimensions.
    pub clef: ApertureDimensions,
    /// Production aperture dimensions.
    pub prof: ApertureDimensions,
    /// Encoded pixels dimensions.
    pub enof: ApertureDimensions,
}

impl TrackApertureModeDimensionsBox {
    /// Parse a whole `tapt` box, header included.
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        let body = crate::box_types::box_body(bytes, 0, "tapt")?;
        Self::parse_body(body)
    }

    /// Parse a `tapt` body (after the box header).
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        let mut clef = None;
        let mut prof = None;
        let mut enof = None;
        for item in BoxIter::clamped(body) {
            let Ok(item) = item else {
                break;
            };
            match item.four_cc() {
                CLEF => clef = Some(ApertureDimensions::parse(item.bytes)?),
                PROF => prof = Some(ApertureDimensions::parse(item.bytes)?),
                ENOF => enof = Some(ApertureDimensions::parse(item.bytes)?),
                _ => {}
            }
        }
        match (clef, prof, enof) {
            (Some(clef), Some(prof), Some(enof)) => Ok(Self { clef, prof, enof }),
            _ => Err(Error::UnexpectedBox {
                expected: "tapt with clef, prof and enof",
            }),
        }
    }
}
