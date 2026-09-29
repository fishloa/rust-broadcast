//! FLAC in ISOBMFF — `fLaC` AudioSampleEntry + `dfLa` config box.
//!
//! xiph "Encapsulation of FLAC in ISO Base Media File Format"
//! (<https://github.com/xiph/flac/blob/master/doc/isoflac.txt>, FLACSpecificBox).
//!
//! `dfLa` is a `FullBox(version=0, 0)` carrying one or more FLAC metadata blocks;
//! the first block MUST be STREAMINFO (block type 0).

use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

/// FourCC of the FLAC config box.
pub const DFLA_FOURCC: [u8; 4] = *b"dfLa";
/// FourCC of the FLAC sample entry.
pub const FLAC_FOURCC: [u8; 4] = *b"fLaC";
/// FullBox extension: version(1) + flags(3).
const FULL_HDR: usize = 4;
/// FLAC metadata block header: `last(1)` | `type(7)` | `length(24)` = 4 bytes.
const METADATA_BLOCK_HDR: usize = 4;
/// STREAMINFO metadata block type.
pub const BLOCK_TYPE_STREAMINFO: u8 = 0;

/// A single FLAC metadata block (header + raw block data).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct FlacMetadataBlock {
    /// `LastMetadataBlockFlag`.
    pub last: bool,
    /// `BlockType` (7 bits); `0` = STREAMINFO.
    pub block_type: u8,
    /// Raw `BlockData` (opaque FLAC metadata; STREAMINFO is 34 bytes).
    pub data: Vec<u8>,
}

/// FLACSpecificBox (`dfLa` box body) — a `FullBox(version=0, 0)` of metadata blocks.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct FlacSpecificBox {
    /// FullBox version (0).
    pub version: u8,
    /// FullBox flags (0).
    pub flags: u32,
    /// Metadata blocks; the first MUST be STREAMINFO (block type 0).
    pub blocks: Vec<FlacMetadataBlock>,
}

impl FlacSpecificBox {
    /// RFC 6381 codec string — always the literal `"fLaC"`.
    pub fn rfc6381(&self) -> &'static str {
        "fLaC"
    }

    /// The STREAMINFO block data, if the first block is STREAMINFO.
    pub fn streaminfo(&self) -> Option<&[u8]> {
        self.blocks
            .first()
            .filter(|b| b.block_type == BLOCK_TYPE_STREAMINFO)
            .map(|b| b.data.as_slice())
    }
}

impl<'a> Parse<'a> for FlacSpecificBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < FULL_HDR {
            return Err(Error::BufferTooShort {
                need: FULL_HDR,
                have: bytes.len(),
                what: "dfLa body",
            });
        }
        let version = bytes[0];
        let flags = u32::from_be_bytes([0, bytes[1], bytes[2], bytes[3]]);
        let mut blocks = Vec::new();
        let mut off = FULL_HDR;
        while off + METADATA_BLOCK_HDR <= bytes.len() {
            let hdr = bytes[off];
            let last = (hdr & 0x80) != 0;
            let block_type = hdr & 0x7F;
            let length =
                u32::from_be_bytes([0, bytes[off + 1], bytes[off + 2], bytes[off + 3]]) as usize;
            let data_start = off + METADATA_BLOCK_HDR;
            // A block whose declared `METADATA_BLOCK_LENGTH` runs past the end of
            // the box is truncated input, not a shorter valid block: accepting it
            // would make `serialize_into` re-emit the *truncated* length, so the
            // round trip would silently stop being byte-identical.
            let data_end = data_start.checked_add(length).filter(|&e| e <= bytes.len());
            let Some(data_end) = data_end else {
                return Err(Error::BufferTooShort {
                    need: data_start + length,
                    have: bytes.len(),
                    what: "dfLa metadata block",
                });
            };
            blocks.push(FlacMetadataBlock {
                last,
                block_type,
                data: bytes[data_start..data_end].to_vec(),
            });
            off = data_end;
            if last {
                break;
            }
        }
        // `isoflac.txt` FLACSpecificBox: "The first metadata block MUST be
        // STREAMINFO" — the stream geometry lives only there, so a box whose
        // first block is anything else is unusable.
        match blocks.first() {
            Some(b) if b.block_type == BLOCK_TYPE_STREAMINFO => {}
            other => {
                return Err(Error::InvalidValue {
                    field: "dfLa first metadata block",
                    value: other.map_or(u64::MAX, |b| b.block_type as u64),
                    reason: "the first FLAC metadata block must be STREAMINFO (type 0)",
                });
            }
        }
        Ok(Self {
            version,
            flags,
            blocks,
        })
    }
}

impl Serialize for FlacSpecificBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = FULL_HDR;
        for b in &self.blocks {
            n += METADATA_BLOCK_HDR + b.data.len();
        }
        n
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        buf[0] = self.version;
        let fb = self.flags.to_be_bytes();
        buf[1..4].copy_from_slice(&fb[1..]);
        let mut off = FULL_HDR;
        for b in &self.blocks {
            buf[off] = ((b.last as u8) << 7) | (b.block_type & 0x7F);
            let len = broadcast_common::len::fit_u24(b.data.len(), "METADATA_BLOCK_LENGTH")?;
            let lb = len.to_be_bytes();
            buf[off + 1..off + 4].copy_from_slice(&lb[1..]);
            off += METADATA_BLOCK_HDR;
            buf[off..off + b.data.len()].copy_from_slice(&b.data);
            off += b.data.len();
        }
        Ok(need)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A metadata block of 16 MiB (2^24) cannot fit the 24-bit
    /// `METADATA_BLOCK_LENGTH` field (#1129, W17): unfixed, `len as u32` then
    /// the UI24 writer kept only the low 24 bits, silently truncating the
    /// declared block length while the full-length payload was still copied.
    #[test]
    fn oversized_block_length_errors() {
        let flac = FlacSpecificBox {
            version: 0,
            flags: 0,
            blocks: alloc::vec![FlacMetadataBlock {
                last: true,
                block_type: BLOCK_TYPE_STREAMINFO,
                data: alloc::vec![0u8; 1 << 24],
            }],
        };
        let err = flac.try_to_bytes().unwrap_err();
        assert!(
            matches!(
                err,
                Error::FieldOverflow(broadcast_common::len::FieldOverflow {
                    field: "METADATA_BLOCK_LENGTH",
                    ..
                })
            ),
            "expected FieldOverflow for METADATA_BLOCK_LENGTH, got {err:?}"
        );
    }

    /// The boundary: exactly (2^24 - 1) bytes still round-trips.
    #[test]
    fn max_block_length_round_trips() {
        let flac = FlacSpecificBox {
            version: 0,
            flags: 0,
            blocks: alloc::vec![FlacMetadataBlock {
                last: true,
                block_type: BLOCK_TYPE_STREAMINFO,
                data: alloc::vec![0xABu8; (1 << 24) - 1],
            }],
        };
        let bytes = flac.try_to_bytes().unwrap();
        let parsed = FlacSpecificBox::parse(&bytes).unwrap();
        assert_eq!(parsed.blocks[0].data.len(), (1 << 24) - 1);
    }

    /// A metadata block whose declared 24-bit length runs past the end of the
    /// box is rejected (r04-W17). Unfixed, `.min(bytes.len())` kept the short
    /// slice, so parsing then re-serializing wrote the *truncated* length and
    /// the round trip silently stopped being byte-identical.
    #[test]
    fn truncated_metadata_block_errors() {
        // dfLa: version/flags, then one last-block STREAMINFO header declaring
        // 34 bytes while only 4 are present.
        let mut bytes = alloc::vec![0u8; FULL_HDR];
        bytes.push(0x80 | BLOCK_TYPE_STREAMINFO);
        bytes.extend_from_slice(&[0x00, 0x00, 34]);
        bytes.extend_from_slice(&[0xAA; 4]);
        let err = FlacSpecificBox::parse(&bytes).unwrap_err();
        assert!(
            matches!(
                err,
                Error::BufferTooShort {
                    need: 42,
                    have: 12,
                    ..
                }
            ),
            "expected BufferTooShort for the truncated block, got {err:?}"
        );
    }

    /// `isoflac.txt` requires the first metadata block to be STREAMINFO
    /// (r04-W17); a box starting with any other block type is rejected.
    #[test]
    fn first_block_must_be_streaminfo() {
        let mut bytes = alloc::vec![0u8; FULL_HDR];
        // PADDING (type 1), 2 bytes, marked last.
        bytes.push(0x80 | 1);
        bytes.extend_from_slice(&[0x00, 0x00, 2]);
        bytes.extend_from_slice(&[0x00, 0x00]);
        let err = FlacSpecificBox::parse(&bytes).unwrap_err();
        assert!(
            matches!(
                err,
                Error::InvalidValue {
                    field: "dfLa first metadata block",
                    value: 1,
                    ..
                }
            ),
            "expected InvalidValue naming block type 1, got {err:?}"
        );
    }

    /// A STREAMINFO-first box whose later blocks are not STREAMINFO still
    /// parses (only the *first* block carries the rule).
    #[test]
    fn streaminfo_then_other_blocks_parses() {
        let flac = FlacSpecificBox {
            version: 0,
            flags: 0,
            blocks: alloc::vec![
                FlacMetadataBlock {
                    last: false,
                    block_type: BLOCK_TYPE_STREAMINFO,
                    data: alloc::vec![0x11; 34],
                },
                FlacMetadataBlock {
                    last: true,
                    block_type: 4,
                    data: alloc::vec![0x22; 3],
                },
            ],
        };
        let bytes = flac.try_to_bytes().unwrap();
        let parsed = FlacSpecificBox::parse(&bytes).unwrap();
        assert_eq!(parsed, flac);
        assert_eq!(parsed.serialized_len(), bytes.len());
    }
}
