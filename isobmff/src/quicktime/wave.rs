//! The QuickTime `siDecompressionParam` atom (`wave`) — read and written.
//!
//! A version 1/2 QuickTime sound description may put the codec configuration
//! inside a `wave` child rather than beside the fixed fields. Audio tools that
//! read only the ISO layout never see the `esds` of such a file.
//!
//! Source: `private/specs-verified/apple/isobmff/quicktime-file-format-online.md`,
//! "siDecompressionParam atom ('wave')".

use alloc::vec::Vec;
use broadcast_core::Serialize;
use broadcast_core::len::{need_in, need_out};

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
    /// The children's wire order (`frma`, `esds`, then each `others` entry).
    /// Empty means write `frma`, `esds`, then `others`.
    pub order: Vec<[u8; BOX_TYPE_SIZE]>,
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
        let mut order = Vec::new();
        for item in BoxIter::clamped(body) {
            let Ok(item) = item else {
                break;
            };
            order.push(item.four_cc());
            match item.four_cc() {
                FRMA => {
                    let payload = item.payload();
                    if payload.len() < BOX_TYPE_SIZE {
                        return Err(Error::BufferTooShort(
                            broadcast_core::error::BufferTooShort {
                                need: BOX_TYPE_SIZE,
                                have: payload.len(),
                                what: "wave frma format",
                            },
                        ));
                    }
                    format.copy_from_slice(&payload[..BOX_TYPE_SIZE]);
                }
                ESDS => esds = Some(EsdsBox::parse_body(item.payload())?),
                other => others.push(OpaqueBox {
                    box_type: other,
                    data: item.payload().to_vec(),
                    to_end: item.to_end(),
                    largesize: item.has_largesize(),
                }),
            }
        }
        Ok(Self {
            format,
            esds,
            others,
            order,
        })
    }
}

/// Bytes of the `frma` child that carries the format.
const FRMA_BOX_LEN: usize = BOX_HEADER_MIN_SIZE + BOX_TYPE_SIZE;

impl Serialize for WaveAtom {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let mut n = BOX_HEADER_MIN_SIZE;
        if self.order.is_empty() {
            n += FRMA_BOX_LEN;
            if let Some(esds) = &self.esds {
                n += esds.serialized_len();
            }
            for other in &self.others {
                n += other.serialized_len();
            }
        } else {
            let mut next_other = 0usize;
            for four_cc in &self.order {
                if *four_cc == FRMA {
                    n += FRMA_BOX_LEN;
                } else if *four_cc == ESDS {
                    if let Some(esds) = &self.esds {
                        n += esds.serialized_len();
                    }
                } else if let Some(other) = self.others.get(next_other) {
                    n += other.serialized_len();
                    next_other += 1;
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
        let mut c = crate::box_types::write_box_header(buf, &WAVE, need)?;
        if self.order.is_empty() {
            c += write_frma(buf, c, &self.format)?;
            if let Some(esds) = &self.esds {
                c += esds.serialize_into(&mut buf[c..])?;
            }
            for other in &self.others {
                c += other.serialize_into(&mut buf[c..])?;
            }
        } else {
            let mut next_other = 0usize;
            for four_cc in &self.order {
                if *four_cc == FRMA {
                    c += write_frma(buf, c, &self.format)?;
                } else if *four_cc == ESDS {
                    if let Some(esds) = &self.esds {
                        c += esds.serialize_into(&mut buf[c..])?;
                    }
                } else if let Some(other) = self.others.get(next_other) {
                    c += other.serialize_into(&mut buf[c..])?;
                    next_other += 1;
                }
            }
            for other in &self.others[next_other..] {
                c += other.serialize_into(&mut buf[c..])?;
            }
        }
        Ok(c)
    }
}

fn write_frma(buf: &mut [u8], c: usize, format: &[u8; BOX_TYPE_SIZE]) -> Result<usize> {
    let mut written = crate::box_types::write_box_header(&mut buf[c..], &FRMA, FRMA_BOX_LEN)?;
    buf[c + written..c + written + BOX_TYPE_SIZE].copy_from_slice(format);
    written += BOX_TYPE_SIZE;
    Ok(written)
}
