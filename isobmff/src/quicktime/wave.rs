//! The QuickTime `siDecompressionParam` atom (`wave`) — read only.
//!
//! A version 1/2 QuickTime sound description may put the codec configuration
//! inside a `wave` child rather than beside the fixed fields. Audio tools that
//! read only the ISO layout never see the `esds` of such a file.
//!
//! Source: `private/specs-verified/apple/isobmff/quicktime-file-format-online.md`,
//! "siDecompressionParam atom ('wave')".

use alloc::vec::Vec;
use broadcast_common::len::need_in;

use crate::box_support::OpaqueBox;
use crate::box_types::{BOX_HEADER_MIN_SIZE, BOX_TYPE_SIZE, BoxIter};
use crate::error::{Error, Result};
use crate::mp4esds::EsdsBox;

/// Four-CC of the decompression parameter container.
pub const WAVE: [u8; BOX_TYPE_SIZE] = *b"wave";
/// Four-CC of the format atom inside `wave`.
pub const FRMA: [u8; BOX_TYPE_SIZE] = *b"frma";
/// Four-CC of the elementary stream descriptor inside `wave`.
pub const ESDS: [u8; BOX_TYPE_SIZE] = *b"esds";

/// A QuickTime `wave` atom: its `frma` format, the `esds` codec configuration
/// and every other child kept verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct WaveAtom {
    /// The `frma` format four-CC; all-zero when the atom carries none.
    pub format: [u8; BOX_TYPE_SIZE],
    /// The elementary stream descriptor, when present.
    pub esds: Option<EsdsBox>,
    /// Every other child, including the terminator atom, in wire order.
    pub others: Vec<OpaqueBox>,
}

impl WaveAtom {
    /// Parse a whole `wave` box, header included.
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        need_in(bytes, BOX_HEADER_MIN_SIZE, "wave")?;
        let body = crate::box_types::box_body(bytes, 0, "wave")?;
        Self::parse_body(body)
    }

    /// Parse a `wave` body (after the box header).
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        let mut format = [0u8; BOX_TYPE_SIZE];
        let mut esds = None;
        let mut others = Vec::new();
        for item in BoxIter::clamped(body) {
            let Ok(item) = item else {
                break;
            };
            match item.four_cc() {
                FRMA => {
                    let payload = item.payload();
                    if payload.len() < BOX_TYPE_SIZE {
                        return Err(Error::BufferTooShort(
                            broadcast_common::error::BufferTooShort {
                                need: BOX_TYPE_SIZE,
                                have: payload.len(),
                                what: "wave frma format",
                            },
                        ));
                    }
                    format.copy_from_slice(&payload[..BOX_TYPE_SIZE]);
                }
                ESDS => esds = Some(EsdsBox::parse_body(item.payload())?),
                other => others.push(OpaqueBox::new(other, item.payload().to_vec())),
            }
        }
        Ok(Self {
            format,
            esds,
            others,
        })
    }
}
