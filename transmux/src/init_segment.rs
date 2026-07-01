//! Typed init-segment (moov) box tree — ISO/IEC 14496-12:2015 §8.2–8.7.
//!
//! Complete typed representation of the `moov` hierarchy found in ISOBMFF
//! initialisation segments. Container sizes are **computed** from children
//! (no `self.raw` passthrough). Unknown/opaque child boxes are preserved as
//! [`OpaqueBox`] for byte-exact round-trip.
//!
//! Reuses `TimeToSampleBox`, `CompositionOffsetBox`, `EditListBox` from
//! the `timing` module and `AVCSampleEntry` etc. from
//! `sample_entries`.

use crate::error::{Error, Result};
use alloc::boxed::Box;
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

const BOX_HDR: usize = 8;
const FULL_HDR: usize = 4;

// ---------------------------------------------------------------------------
// OpaqueBox — round-trip unknown child boxes
// ---------------------------------------------------------------------------

/// An opaque box whose contents we do not parse — round-tripped verbatim.
/// Preserves the exact bytes so the real-fixture test stays byte-identical.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct OpaqueBox {
    pub box_type: [u8; 4],
    pub data: Vec<u8>,
}

impl OpaqueBox {
    pub fn new(box_type: [u8; 4], data: Vec<u8>) -> Self {
        Self { box_type, data }
    }
}

impl Serialize for OpaqueBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        BOX_HDR + self.data.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        buf[..4].copy_from_slice(&(need as u32).to_be_bytes());
        buf[4..8].copy_from_slice(&self.box_type);
        buf[8..8 + self.data.len()].copy_from_slice(&self.data);
        Ok(need)
    }
}

// ---------------------------------------------------------------------------
// MovieHeaderBox — mvhd (ISO/IEC 14496-12:2015 §8.2.2)
// ---------------------------------------------------------------------------

/// Movie Header Box (`mvhd`) — §8.2.2.
/// v0: 32-bit creation_time, modification_time, duration.
/// v1: 64-bit equivalents.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MovieHeaderBox {
    pub version: u8,
    pub flags: u32,
    pub creation_time: u64,
    pub modification_time: u64,
    pub timescale: u32,
    pub duration: u64,
    pub rate: u32,
    pub volume: u16,
    pub matrix: [i32; 9],
    pub next_track_id: u32,
}

impl<'a> Parse<'a> for MovieHeaderBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 12 {
            return Err(Error::BufferTooShort {
                need: 12,
                have: bytes.len(),
                what: "mvhd",
            });
        }
        let ver = bytes[8];
        let flags = u32::from_be_bytes([0, bytes[9], bytes[10], bytes[11]]);
        if ver == 0 {
            let need = 108;
            if bytes.len() < need {
                return Err(Error::BufferTooShort {
                    need,
                    have: bytes.len(),
                    what: "mvhd v0",
                });
            }
            Ok(Self {
                version: 0,
                flags,
                creation_time: u32::from_be_bytes([bytes[12], bytes[13], bytes[14], bytes[15]])
                    as u64,
                modification_time: u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]])
                    as u64,
                timescale: u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]),
                duration: u32::from_be_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]) as u64,
                rate: u32::from_be_bytes([bytes[28], bytes[29], bytes[30], bytes[31]]),
                volume: u16::from_be_bytes([bytes[32], bytes[33]]),
                matrix: [
                    i32::from_be_bytes([bytes[44], bytes[45], bytes[46], bytes[47]]),
                    i32::from_be_bytes([bytes[48], bytes[49], bytes[50], bytes[51]]),
                    i32::from_be_bytes([bytes[52], bytes[53], bytes[54], bytes[55]]),
                    i32::from_be_bytes([bytes[56], bytes[57], bytes[58], bytes[59]]),
                    i32::from_be_bytes([bytes[60], bytes[61], bytes[62], bytes[63]]),
                    i32::from_be_bytes([bytes[64], bytes[65], bytes[66], bytes[67]]),
                    i32::from_be_bytes([bytes[68], bytes[69], bytes[70], bytes[71]]),
                    i32::from_be_bytes([bytes[72], bytes[73], bytes[74], bytes[75]]),
                    i32::from_be_bytes([bytes[76], bytes[77], bytes[78], bytes[79]]),
                ],
                next_track_id: u32::from_be_bytes([bytes[104], bytes[105], bytes[106], bytes[107]]),
            })
        } else {
            let need = 124;
            if bytes.len() < need {
                return Err(Error::BufferTooShort {
                    need,
                    have: bytes.len(),
                    what: "mvhd v1",
                });
            }
            Ok(Self {
                version: 1,
                flags,
                creation_time: u64::from_be_bytes([
                    bytes[12], bytes[13], bytes[14], bytes[15], bytes[16], bytes[17], bytes[18],
                    bytes[19],
                ]),
                modification_time: u64::from_be_bytes([
                    bytes[20], bytes[21], bytes[22], bytes[23], bytes[24], bytes[25], bytes[26],
                    bytes[27],
                ]),
                timescale: u32::from_be_bytes([bytes[28], bytes[29], bytes[30], bytes[31]]),
                duration: u64::from_be_bytes([
                    bytes[32], bytes[33], bytes[34], bytes[35], bytes[36], bytes[37], bytes[38],
                    bytes[39],
                ]),
                rate: u32::from_be_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]),
                volume: u16::from_be_bytes([bytes[44], bytes[45]]),
                matrix: [
                    i32::from_be_bytes([bytes[56], bytes[57], bytes[58], bytes[59]]),
                    i32::from_be_bytes([bytes[60], bytes[61], bytes[62], bytes[63]]),
                    i32::from_be_bytes([bytes[64], bytes[65], bytes[66], bytes[67]]),
                    i32::from_be_bytes([bytes[68], bytes[69], bytes[70], bytes[71]]),
                    i32::from_be_bytes([bytes[72], bytes[73], bytes[74], bytes[75]]),
                    i32::from_be_bytes([bytes[76], bytes[77], bytes[78], bytes[79]]),
                    i32::from_be_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]),
                    i32::from_be_bytes([bytes[84], bytes[85], bytes[86], bytes[87]]),
                    i32::from_be_bytes([bytes[88], bytes[89], bytes[90], bytes[91]]),
                ],
                next_track_id: u32::from_be_bytes([bytes[120], bytes[121], bytes[122], bytes[123]]),
            })
        }
    }
}

impl Serialize for MovieHeaderBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        if self.version == 0 {
            108
        } else {
            124
        }
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"mvhd");
        c += 4;
        buf[c] = self.version;
        c += 1;
        let fb = self.flags.to_be_bytes();
        buf[c..c + 3].copy_from_slice(&fb[1..]);
        c += 3;
        let (ct_sz, mt_sz, dur_sz) = if self.version == 0 {
            (4u8, 4u8, 4u8)
        } else {
            (8u8, 8u8, 8u8)
        };
        let write_u64 = |buf: &mut [u8], off: usize, sz: u8, v: u64| {
            if sz == 4 {
                buf[off..off + 4].copy_from_slice(&(v as u32).to_be_bytes());
            } else {
                buf[off..off + 8].copy_from_slice(&v.to_be_bytes());
            }
        };
        write_u64(buf, c, ct_sz, self.creation_time);
        c += ct_sz as usize;
        write_u64(buf, c, mt_sz, self.modification_time);
        c += mt_sz as usize;
        buf[c..c + 4].copy_from_slice(&self.timescale.to_be_bytes());
        c += 4;
        write_u64(buf, c, dur_sz, self.duration);
        c += dur_sz as usize;
        buf[c..c + 4].copy_from_slice(&self.rate.to_be_bytes());
        c += 4;
        buf[c..c + 2].copy_from_slice(&self.volume.to_be_bytes());
        c += 2;
        c += 10; // reserved
        for &m in &self.matrix {
            buf[c..c + 4].copy_from_slice(&m.to_be_bytes());
            c += 4;
        }
        c += 24; // pre_defined
        buf[c..c + 4].copy_from_slice(&self.next_track_id.to_be_bytes());
        c += 4;
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// TrackHeaderBox — tkhd (ISO/IEC 14496-12:2015 §8.2.3)
// ---------------------------------------------------------------------------

/// Track Header Box (`tkhd`) — §8.2.3.
/// v0: 32-bit creation_time, modification_time, duration.
/// v1: 64-bit equivalents.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TrackHeaderBox {
    pub version: u8,
    pub flags: u32,
    pub creation_time: u64,
    pub modification_time: u64,
    pub track_id: u32,
    pub duration: u64,
    pub layer: i16,
    pub alternate_group: i16,
    pub volume: i16,
    pub matrix: [i32; 9],
    pub width: u32,
    pub height: u32,
}

impl<'a> Parse<'a> for TrackHeaderBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 12 {
            return Err(Error::BufferTooShort {
                need: 12,
                have: bytes.len(),
                what: "tkhd",
            });
        }
        let ver = bytes[8];
        let flags = u32::from_be_bytes([0, bytes[9], bytes[10], bytes[11]]);
        if ver == 0 {
            let need = 92;
            if bytes.len() < need {
                return Err(Error::BufferTooShort {
                    need,
                    have: bytes.len(),
                    what: "tkhd v0",
                });
            }
            let ct = u32::from_be_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as u64;
            let mt = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]) as u64;
            let tid = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
            let dur = u32::from_be_bytes([bytes[28], bytes[29], bytes[30], bytes[31]]) as u64;
            Ok(Self {
                version: 0,
                flags,
                creation_time: ct,
                modification_time: mt,
                track_id: tid,
                duration: dur,
                layer: i16::from_be_bytes([bytes[40], bytes[41]]),
                alternate_group: i16::from_be_bytes([bytes[42], bytes[43]]),
                volume: i16::from_be_bytes([bytes[44], bytes[45]]),
                matrix: matrix_from_bytes(&bytes[48..84]),
                width: u32::from_be_bytes([bytes[84], bytes[85], bytes[86], bytes[87]]),
                height: u32::from_be_bytes([bytes[88], bytes[89], bytes[90], bytes[91]]),
            })
        } else {
            let need = 104;
            if bytes.len() < need {
                return Err(Error::BufferTooShort {
                    need,
                    have: bytes.len(),
                    what: "tkhd v1",
                });
            }
            let ct = u64::from_be_bytes(bytes[12..20].try_into().unwrap());
            let mt = u64::from_be_bytes(bytes[20..28].try_into().unwrap());
            let tid = u32::from_be_bytes([bytes[28], bytes[29], bytes[30], bytes[31]]);
            let dur = u64::from_be_bytes(bytes[40..48].try_into().unwrap());
            Ok(Self {
                version: 1,
                flags,
                creation_time: ct,
                modification_time: mt,
                track_id: tid,
                duration: dur,
                layer: i16::from_be_bytes([bytes[48], bytes[49]]),
                alternate_group: i16::from_be_bytes([bytes[50], bytes[51]]),
                volume: i16::from_be_bytes([bytes[52], bytes[53]]),
                matrix: matrix_from_bytes(&bytes[56..92]),
                width: u32::from_be_bytes([bytes[96], bytes[97], bytes[98], bytes[99]]),
                height: u32::from_be_bytes([bytes[100], bytes[101], bytes[102], bytes[103]]),
            })
        }
    }
}

fn matrix_from_bytes(b: &[u8]) -> [i32; 9] {
    let mut m = [0i32; 9];
    for i in 0..9 {
        m[i] = i32::from_be_bytes([b[i * 4], b[i * 4 + 1], b[i * 4 + 2], b[i * 4 + 3]]);
    }
    m
}

impl Serialize for TrackHeaderBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        if self.version == 0 {
            92
        } else {
            104
        }
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"tkhd");
        c += 4;
        buf[c] = self.version;
        c += 1;
        let fb = self.flags.to_be_bytes();
        buf[c..c + 3].copy_from_slice(&fb[1..]);
        c += 3;
        if self.version == 0 {
            buf[c..c + 4].copy_from_slice(&(self.creation_time as u32).to_be_bytes());
            c += 4;
            buf[c..c + 4].copy_from_slice(&(self.modification_time as u32).to_be_bytes());
            c += 4;
            buf[c..c + 4].copy_from_slice(&self.track_id.to_be_bytes());
            c += 4;
            c += 4; // reserved
            buf[c..c + 4].copy_from_slice(&(self.duration as u32).to_be_bytes());
            c += 4;
            c += 8; // reserved * 2
            buf[c..c + 2].copy_from_slice(&self.layer.to_be_bytes());
            c += 2;
            buf[c..c + 2].copy_from_slice(&self.alternate_group.to_be_bytes());
            c += 2;
            buf[c..c + 2].copy_from_slice(&self.volume.to_be_bytes());
            c += 2;
            c += 2; // reserved
        } else {
            buf[c..c + 8].copy_from_slice(&self.creation_time.to_be_bytes());
            c += 8;
            buf[c..c + 8].copy_from_slice(&self.modification_time.to_be_bytes());
            c += 8;
            buf[c..c + 4].copy_from_slice(&self.track_id.to_be_bytes());
            c += 4;
            c += 4;
            buf[c..c + 8].copy_from_slice(&self.duration.to_be_bytes());
            c += 8;
            c += 8;
            buf[c..c + 2].copy_from_slice(&self.layer.to_be_bytes());
            c += 2;
            buf[c..c + 2].copy_from_slice(&self.alternate_group.to_be_bytes());
            c += 2;
            buf[c..c + 2].copy_from_slice(&self.volume.to_be_bytes());
            c += 2;
            c += 2;
        }
        for &m in &self.matrix {
            buf[c..c + 4].copy_from_slice(&m.to_be_bytes());
            c += 4;
        }
        buf[c..c + 4].copy_from_slice(&self.width.to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(&self.height.to_be_bytes());
        c += 4;
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// MediaHeaderBox — mdhd (ISO/IEC 14496-12:2015 §8.4.2)
// ---------------------------------------------------------------------------

/// Media Header Box (`mdhd`) — §8.4.2.
/// v0: 32-bit creation/modification/duration; v1: 64-bit.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MediaHeaderBox {
    pub version: u8,
    pub flags: u32,
    pub creation_time: u64,
    pub modification_time: u64,
    pub timescale: u32,
    pub duration: u64,
    pub language: u16,
}

impl<'a> Parse<'a> for MediaHeaderBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 12 {
            return Err(Error::BufferTooShort {
                need: 12,
                have: bytes.len(),
                what: "mdhd",
            });
        }
        let ver = bytes[8];
        let flags = u32::from_be_bytes([0, bytes[9], bytes[10], bytes[11]]);
        if ver == 0 {
            if bytes.len() < 32 {
                return Err(Error::BufferTooShort {
                    need: 32,
                    have: bytes.len(),
                    what: "mdhd v0",
                });
            }
            Ok(Self {
                version: 0,
                flags,
                creation_time: u32::from_be_bytes([bytes[12], bytes[13], bytes[14], bytes[15]])
                    as u64,
                modification_time: u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]])
                    as u64,
                timescale: u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]),
                duration: u32::from_be_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]) as u64,
                language: u16::from_be_bytes([bytes[28], bytes[29]]),
            })
        } else {
            if bytes.len() < 44 {
                return Err(Error::BufferTooShort {
                    need: 44,
                    have: bytes.len(),
                    what: "mdhd v1",
                });
            }
            Ok(Self {
                version: 1,
                flags,
                creation_time: u64::from_be_bytes(bytes[12..20].try_into().unwrap()),
                modification_time: u64::from_be_bytes(bytes[20..28].try_into().unwrap()),
                timescale: u32::from_be_bytes([bytes[28], bytes[29], bytes[30], bytes[31]]),
                duration: u64::from_be_bytes(bytes[32..40].try_into().unwrap()),
                language: u16::from_be_bytes([bytes[40], bytes[41]]),
            })
        }
    }
}

impl Serialize for MediaHeaderBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        if self.version == 0 {
            32
        } else {
            44
        }
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"mdhd");
        c += 4;
        buf[c] = self.version;
        c += 1;
        let fb = self.flags.to_be_bytes();
        buf[c..c + 3].copy_from_slice(&fb[1..]);
        c += 3;
        if self.version == 0 {
            buf[c..c + 4].copy_from_slice(&(self.creation_time as u32).to_be_bytes());
            c += 4;
            buf[c..c + 4].copy_from_slice(&(self.modification_time as u32).to_be_bytes());
            c += 4;
            buf[c..c + 4].copy_from_slice(&self.timescale.to_be_bytes());
            c += 4;
            buf[c..c + 4].copy_from_slice(&(self.duration as u32).to_be_bytes());
            c += 4;
            buf[c..c + 2].copy_from_slice(&self.language.to_be_bytes());
            c += 2;
        } else {
            buf[c..c + 8].copy_from_slice(&self.creation_time.to_be_bytes());
            c += 8;
            buf[c..c + 8].copy_from_slice(&self.modification_time.to_be_bytes());
            c += 8;
            buf[c..c + 4].copy_from_slice(&self.timescale.to_be_bytes());
            c += 4;
            buf[c..c + 8].copy_from_slice(&self.duration.to_be_bytes());
            c += 8;
            buf[c..c + 2].copy_from_slice(&self.language.to_be_bytes());
            c += 2;
        }
        Ok(c + 2) // +2 for the quality field (reserved)
    }
}

// ---------------------------------------------------------------------------
// HandlerBox — hdlr (ISO/IEC 14496-12:2015 §8.4.3)
// ---------------------------------------------------------------------------

/// Handler Box (`hdlr`) — §8.4.3.
/// Declares the media handler type (`vide`, `soun`, etc.) and an optional name.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct HandlerBox {
    pub version: u8,
    pub flags: u32,
    pub handler_type: [u8; 4],
    pub name: Vec<u8>,
}

impl<'a> Parse<'a> for HandlerBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 24 {
            return Err(Error::BufferTooShort {
                need: 24,
                have: bytes.len(),
                what: "hdlr",
            });
        }
        let ver = bytes[8];
        let flags = u32::from_be_bytes([0, bytes[9], bytes[10], bytes[11]]);
        let handler_type = [bytes[16], bytes[17], bytes[18], bytes[19]];
        let name = if bytes.len() > 32 {
            bytes[32..].to_vec()
        } else {
            Vec::new()
        };
        Ok(Self {
            version: ver,
            flags,
            handler_type,
            name,
        })
    }
}

impl Serialize for HandlerBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        BOX_HDR + FULL_HDR + 20 + self.name.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"hdlr");
        c += 4;
        buf[c] = self.version;
        c += 1;
        let fb = self.flags.to_be_bytes();
        buf[c..c + 3].copy_from_slice(&fb[1..]);
        c += 3;
        c += 4; // pre_defined
        buf[c..c + 4].copy_from_slice(&self.handler_type);
        c += 4;
        c += 12; // reserved * 3
        if !self.name.is_empty() {
            buf[c..c + self.name.len()].copy_from_slice(&self.name);
        }
        Ok(c + self.name.len())
    }
}

// ---------------------------------------------------------------------------
// VideoMediaHeaderBox — vmhd (ISO/IEC 14496-12:2015 §8.4.5.2)
// ---------------------------------------------------------------------------

/// Video Media Header Box (`vmhd`) — §8.4.5.2.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct VideoMediaHeaderBox {
    pub version: u8,
    pub flags: u32,
    pub graphicsmode: u16,
    pub opcolor: [u16; 3],
}

impl<'a> Parse<'a> for VideoMediaHeaderBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 20 {
            return Err(Error::BufferTooShort {
                need: 20,
                have: bytes.len(),
                what: "vmhd",
            });
        }
        let ver = bytes[8];
        let flags = u32::from_be_bytes([0, bytes[9], bytes[10], bytes[11]]);
        Ok(Self {
            version: ver,
            flags,
            graphicsmode: u16::from_be_bytes([bytes[12], bytes[13]]),
            opcolor: [
                u16::from_be_bytes([bytes[14], bytes[15]]),
                u16::from_be_bytes([bytes[16], bytes[17]]),
                u16::from_be_bytes([bytes[18], bytes[19]]),
            ],
        })
    }
}

impl Serialize for VideoMediaHeaderBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        BOX_HDR + FULL_HDR + 8
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"vmhd");
        c += 4;
        buf[c] = self.version;
        c += 1;
        let fb = self.flags.to_be_bytes();
        buf[c..c + 3].copy_from_slice(&fb[1..]);
        c += 3;
        buf[c..c + 2].copy_from_slice(&self.graphicsmode.to_be_bytes());
        c += 2;
        buf[c..c + 2].copy_from_slice(&self.opcolor[0].to_be_bytes());
        c += 2;
        buf[c..c + 2].copy_from_slice(&self.opcolor[1].to_be_bytes());
        c += 2;
        buf[c..c + 2].copy_from_slice(&self.opcolor[2].to_be_bytes());
        Ok(c + 2)
    }
}

// ---------------------------------------------------------------------------
// SoundMediaHeaderBox — smhd (ISO/IEC 14496-12:2015 §8.4.5.3)
// ---------------------------------------------------------------------------

/// Sound Media Header Box (`smhd`) — §8.4.5.3.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SoundMediaHeaderBox {
    pub version: u8,
    pub flags: u32,
    pub balance: i16,
}

impl<'a> Parse<'a> for SoundMediaHeaderBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 16 {
            return Err(Error::BufferTooShort {
                need: 16,
                have: bytes.len(),
                what: "smhd",
            });
        }
        let ver = bytes[8];
        let flags = u32::from_be_bytes([0, bytes[9], bytes[10], bytes[11]]);
        Ok(Self {
            version: ver,
            flags,
            balance: i16::from_be_bytes([bytes[12], bytes[13]]),
        })
    }
}

impl Serialize for SoundMediaHeaderBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        BOX_HDR + FULL_HDR + 4
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"smhd");
        c += 4;
        buf[c] = self.version;
        c += 1;
        let fb = self.flags.to_be_bytes();
        buf[c..c + 3].copy_from_slice(&fb[1..]);
        c += 3;
        buf[c..c + 2].copy_from_slice(&self.balance.to_be_bytes());
        c += 2;
        c += 2; // reserved
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// DataReferenceBox — dref (ISO/IEC 14496-12:2015 §8.7.2)
// ---------------------------------------------------------------------------

/// Data Reference Box (`dref`) — §8.7.2.
/// Contains a list of DataEntryUrlBox entries.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DataReferenceBox {
    pub version: u8,
    pub flags: u32,
    pub entries: Vec<DataEntryUrlBox>,
}

impl<'a> Parse<'a> for DataReferenceBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 16 {
            return Err(Error::BufferTooShort {
                need: 16,
                have: bytes.len(),
                what: "dref",
            });
        }
        let ver = bytes[8];
        let flags = u32::from_be_bytes([0, bytes[9], bytes[10], bytes[11]]);
        let count = u32::from_be_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;
        let mut entries = Vec::with_capacity(count);
        let mut off = 16usize;
        for _ in 0..count {
            if off + 8 > bytes.len() {
                break;
            }
            let sz =
                u32::from_be_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]])
                    as usize;
            if sz < 8 {
                break;
            }
            let end = (off + sz).min(bytes.len());
            entries.push(DataEntryUrlBox::parse(&bytes[off..end])?);
            off += sz;
        }
        Ok(Self {
            version: ver,
            flags,
            entries,
        })
    }
}

impl Serialize for DataReferenceBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HDR + FULL_HDR + 4;
        for e in &self.entries {
            n += e.serialized_len();
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
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"dref");
        c += 4;
        buf[c] = self.version;
        c += 1;
        let fb = self.flags.to_be_bytes();
        buf[c..c + 3].copy_from_slice(&fb[1..]);
        c += 3;
        buf[c..c + 4].copy_from_slice(&(self.entries.len() as u32).to_be_bytes());
        c += 4;
        for entry in &self.entries {
            c += entry.serialize_into(&mut buf[c..])?;
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// DataEntryUrlBox — url  (ISO/IEC 14496-12:2015 §8.7.2)
// ---------------------------------------------------------------------------

/// Data Entry URL Box (`url `) — §8.7.2.
/// When `flags & 1` is set, the media data is in this file (self-contained).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DataEntryUrlBox {
    pub version: u8,
    pub flags: u32,
    pub location: Vec<u8>,
}

impl<'a> Parse<'a> for DataEntryUrlBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 12 {
            return Err(Error::BufferTooShort {
                need: 12,
                have: bytes.len(),
                what: "url",
            });
        }
        let ver = bytes[8];
        let flags = u32::from_be_bytes([0, bytes[9], bytes[10], bytes[11]]);
        let location = if bytes.len() > 12 {
            bytes[12..].to_vec()
        } else {
            Vec::new()
        };
        Ok(Self {
            version: ver,
            flags,
            location,
        })
    }
}

impl Serialize for DataEntryUrlBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        BOX_HDR + FULL_HDR + self.location.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"url ");
        c += 4;
        buf[c] = self.version;
        c += 1;
        let fb = self.flags.to_be_bytes();
        buf[c..c + 3].copy_from_slice(&fb[1..]);
        c += 3;
        if !self.location.is_empty() {
            buf[c..c + self.location.len()].copy_from_slice(&self.location);
            c += self.location.len();
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// SampleToChunkBox — stsc (ISO/IEC 14496-12:2015 §8.7.4)
// ---------------------------------------------------------------------------

/// Entry in the stsc chunk-to-sample table (§8.7.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct StscEntry {
    pub first_chunk: u32,
    pub samples_per_chunk: u32,
    pub sample_description_index: u32,
}

/// Sample To Chunk Box (`stsc`) — §8.7.4.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SampleToChunkBox {
    pub version: u8,
    pub flags: u32,
    pub entries: Vec<StscEntry>,
}

impl<'a> Parse<'a> for SampleToChunkBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 16 {
            return Err(Error::BufferTooShort {
                need: 16,
                have: bytes.len(),
                what: "stsc",
            });
        }
        let ver = bytes[8];
        let flags = u32::from_be_bytes([0, bytes[9], bytes[10], bytes[11]]);
        let count = u32::from_be_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;
        let mut entries = Vec::with_capacity(count);
        let mut off = 16usize;
        for _ in 0..count {
            if off + 12 > bytes.len() {
                break;
            }
            entries.push(StscEntry {
                first_chunk: u32::from_be_bytes([
                    bytes[off],
                    bytes[off + 1],
                    bytes[off + 2],
                    bytes[off + 3],
                ]),
                samples_per_chunk: u32::from_be_bytes([
                    bytes[off + 4],
                    bytes[off + 5],
                    bytes[off + 6],
                    bytes[off + 7],
                ]),
                sample_description_index: u32::from_be_bytes([
                    bytes[off + 8],
                    bytes[off + 9],
                    bytes[off + 10],
                    bytes[off + 11],
                ]),
            });
            off += 12;
        }
        Ok(Self {
            version: ver,
            flags,
            entries,
        })
    }
}

impl Serialize for SampleToChunkBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        BOX_HDR + FULL_HDR + 4 + self.entries.len() * 12
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"stsc");
        c += 4;
        buf[c] = self.version;
        c += 1;
        let fb = self.flags.to_be_bytes();
        buf[c..c + 3].copy_from_slice(&fb[1..]);
        c += 3;
        buf[c..c + 4].copy_from_slice(&(self.entries.len() as u32).to_be_bytes());
        c += 4;
        for entry in &self.entries {
            buf[c..c + 4].copy_from_slice(&entry.first_chunk.to_be_bytes());
            buf[c + 4..c + 8].copy_from_slice(&entry.samples_per_chunk.to_be_bytes());
            buf[c + 8..c + 12].copy_from_slice(&entry.sample_description_index.to_be_bytes());
            c += 12;
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// SampleSizeBox — stsz (ISO/IEC 14496-12:2015 §8.7.3)
// ---------------------------------------------------------------------------

/// Sample Size Box (`stsz`) — §8.7.3.
/// If `sample_size > 0`, all samples have that uniform size and the entries vec
/// is empty. If `sample_size == 0`, entries contains per-sample sizes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SampleSizeBox {
    pub version: u8,
    pub flags: u32,
    pub sample_size: u32,
    pub entries: Vec<u32>,
}

impl<'a> Parse<'a> for SampleSizeBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 20 {
            return Err(Error::BufferTooShort {
                need: 20,
                have: bytes.len(),
                what: "stsz",
            });
        }
        let ver = bytes[8];
        let flags = u32::from_be_bytes([0, bytes[9], bytes[10], bytes[11]]);
        let sample_size = u32::from_be_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
        let count = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]) as usize;
        let mut entries = Vec::with_capacity(count);
        if sample_size == 0 {
            let mut off = 20usize;
            for _ in 0..count {
                if off + 4 > bytes.len() {
                    break;
                }
                entries.push(u32::from_be_bytes([
                    bytes[off],
                    bytes[off + 1],
                    bytes[off + 2],
                    bytes[off + 3],
                ]));
                off += 4;
            }
        }
        Ok(Self {
            version: ver,
            flags,
            sample_size,
            entries,
        })
    }
}

impl Serialize for SampleSizeBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let count = if self.sample_size == 0 {
            self.entries.len()
        } else {
            0
        };
        BOX_HDR + FULL_HDR + 8 + count * 4
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let count = if self.sample_size == 0 {
            self.entries.len()
        } else {
            0
        };
        let need = BOX_HDR + FULL_HDR + 8 + count * 4;
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"stsz");
        c += 4;
        buf[c] = self.version;
        c += 1;
        let fb = self.flags.to_be_bytes();
        buf[c..c + 3].copy_from_slice(&fb[1..]);
        c += 3;
        buf[c..c + 4].copy_from_slice(&self.sample_size.to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(&(count as u32).to_be_bytes());
        c += 4;
        for &sz in &self.entries {
            buf[c..c + 4].copy_from_slice(&sz.to_be_bytes());
            c += 4;
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// ChunkOffsetBox — stco (ISO/IEC 14496-12:2015 §8.7.5)
// ---------------------------------------------------------------------------

/// Chunk Offset Box (`stco`) — §8.7.5 (32-bit offsets).
/// 64-bit offsets via `co64` are captured as an opaque box.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ChunkOffsetBox {
    pub version: u8,
    pub flags: u32,
    pub entries: Vec<u32>,
}

impl<'a> Parse<'a> for ChunkOffsetBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 16 {
            return Err(Error::BufferTooShort {
                need: 16,
                have: bytes.len(),
                what: "stco",
            });
        }
        let ver = bytes[8];
        let flags = u32::from_be_bytes([0, bytes[9], bytes[10], bytes[11]]);
        let count = u32::from_be_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;
        let mut entries = Vec::with_capacity(count);
        let mut off = 16usize;
        for _ in 0..count {
            if off + 4 > bytes.len() {
                break;
            }
            entries.push(u32::from_be_bytes([
                bytes[off],
                bytes[off + 1],
                bytes[off + 2],
                bytes[off + 3],
            ]));
            off += 4;
        }
        Ok(Self {
            version: ver,
            flags,
            entries,
        })
    }
}

impl Serialize for ChunkOffsetBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        BOX_HDR + FULL_HDR + 4 + self.entries.len() * 4
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"stco");
        c += 4;
        buf[c] = self.version;
        c += 1;
        let fb = self.flags.to_be_bytes();
        buf[c..c + 3].copy_from_slice(&fb[1..]);
        c += 3;
        buf[c..c + 4].copy_from_slice(&(self.entries.len() as u32).to_be_bytes());
        c += 4;
        for entry in &self.entries {
            buf[c..c + 4].copy_from_slice(&entry.to_be_bytes());
            c += 4;
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// SampleDescriptionBox — stsd (ISO/IEC 14496-12:2015 §8.5.2)
// ---------------------------------------------------------------------------

/// AAC audio sample entry (`mp4a`) — ISO/IEC 14496-12:2015 §12.2.3.
///
/// Wire layout (32 bytes before optional config children):
/// - SampleEntry: reserved(6) + data_reference_index(16) = 8 bytes
/// - AudioSampleEntry reserved `[2]`: 8 bytes
/// - channelcount(16) + samplesize(16) + predefined(16) + reserved(16) + samplerate(32) = 16 bytes
/// - then config boxes (esds, etc.)
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Mp4aSampleEntry {
    pub data_reference_index: u16,
    pub channelcount: u16,
    pub samplesize: u16,
    pub samplerate: u32,
    pub config_boxes: Vec<OpaqueBox>,
}

impl<'a> Parse<'a> for Mp4aSampleEntry {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        // bytes is a full box with 8-byte header; fields start at bytes[8]
        if bytes.len() < 8 + 28 {
            return Err(Error::BufferTooShort {
                need: 8 + 28,
                have: bytes.len(),
                what: "mp4a",
            });
        }
        let body = &bytes[8..];
        let dri = u16::from_be_bytes([body[6], body[7]]);
        let chan = u16::from_be_bytes([body[16], body[17]]);
        let samp_sz = u16::from_be_bytes([body[18], body[19]]);
        let sr = u32::from_be_bytes([body[24], body[25], body[26], body[27]]);

        let mut config_boxes = Vec::new();
        let mut off = 28usize;
        while off + 8 <= body.len() {
            let sz = u32::from_be_bytes([body[off], body[off + 1], body[off + 2], body[off + 3]])
                as usize;
            if sz < 8 {
                break;
            }
            let end = (off + sz).min(body.len());
            let boxtype = [body[off + 4], body[off + 5], body[off + 6], body[off + 7]];
            let data = body[off + 8..end].to_vec();
            config_boxes.push(OpaqueBox {
                box_type: boxtype,
                data,
            });
            off += sz;
        }
        Ok(Self {
            data_reference_index: dri,
            channelcount: chan,
            samplesize: samp_sz,
            samplerate: sr,
            config_boxes,
        })
    }
}

impl Serialize for Mp4aSampleEntry {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HDR + 28; // box header + AudioSampleEntry fixed fields
        for c in &self.config_boxes {
            n += c.serialized_len();
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
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"mp4a");
        c += 4;
        // SampleEntry: reserved(6) + data_reference_index(2)
        c += 6;
        buf[c..c + 2].copy_from_slice(&self.data_reference_index.to_be_bytes());
        c += 2;
        // AudioSampleEntry: reserved[2] (8 bytes)
        c += 8;
        // channelcount(16) + samplesize(16) + predefined(16) + reserved(16) + samplerate(32) = 12 bytes
        buf[c..c + 2].copy_from_slice(&self.channelcount.to_be_bytes());
        c += 2;
        buf[c..c + 2].copy_from_slice(&self.samplesize.to_be_bytes());
        c += 2;
        c += 4; // predefined(16) + reserved(16)
        buf[c..c + 4].copy_from_slice(&self.samplerate.to_be_bytes());
        c += 4;
        for cb in &self.config_boxes {
            c += cb.serialize_into(&mut buf[c..])?;
        }
        Ok(c)
    }
}

/// Describes one sample entry in an stsd box.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum SampleEntryVariant {
    Avc1(crate::sample_entries::AVCSampleEntry),
    Hevc1(crate::sample_entries::HEVCSampleEntry),
    Mp4a(Box<Mp4aSampleEntry>),
    Unknown(OpaqueBox),
}

/// Sample Description Box (`stsd`) — §8.5.2.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SampleDescriptionBox {
    pub version: u8,
    pub flags: u32,
    pub entries: Vec<SampleEntryVariant>,
}

impl<'a> Parse<'a> for SampleDescriptionBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 16 {
            return Err(Error::BufferTooShort {
                need: 16,
                have: bytes.len(),
                what: "stsd",
            });
        }
        let ver = bytes[8];
        let flags = u32::from_be_bytes([0, bytes[9], bytes[10], bytes[11]]);
        let count = u32::from_be_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;
        let mut entries = Vec::with_capacity(count);
        let mut off = 16usize;
        for _ in 0..count {
            if off + 8 > bytes.len() {
                break;
            }
            let sz =
                u32::from_be_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]])
                    as usize;
            if sz < 8 {
                break;
            }
            let end = (off + sz).min(bytes.len());
            let box_bytes = &bytes[off..end];
            let codec = &box_bytes[4..8];
            let entry = match codec {
                b"avc1" | b"avc3" | b"avc2" | b"avc4" => SampleEntryVariant::Avc1(
                    crate::sample_entries::AVCSampleEntry::bare_parse(box_bytes)?,
                ),
                b"hvc1" | b"hev1" => SampleEntryVariant::Hevc1(
                    crate::sample_entries::HEVCSampleEntry::bare_parse(box_bytes)?,
                ),
                b"mp4a" | b"enca" => {
                    SampleEntryVariant::Mp4a(Box::new(Mp4aSampleEntry::parse(box_bytes)?))
                }
                _ => {
                    let mut c4 = [0u8; 4];
                    c4.copy_from_slice(&codec[..4.min(codec.len())]);
                    SampleEntryVariant::Unknown(OpaqueBox::new(c4, box_bytes[8..].to_vec()))
                }
            };
            entries.push(entry);
            off += sz;
        }
        Ok(Self {
            version: ver,
            flags,
            entries,
        })
    }
}

impl Serialize for SampleDescriptionBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HDR + FULL_HDR + 4;
        for e in &self.entries {
            n += match e {
                SampleEntryVariant::Avc1(a) => a.serialized_len(),
                SampleEntryVariant::Hevc1(h) => h.serialized_len(),
                SampleEntryVariant::Mp4a(m) => m.serialized_len(),
                SampleEntryVariant::Unknown(u) => u.serialized_len(),
            };
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
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"stsd");
        c += 4;
        buf[c] = self.version;
        c += 1;
        let fb = self.flags.to_be_bytes();
        buf[c..c + 3].copy_from_slice(&fb[1..]);
        c += 3;
        buf[c..c + 4].copy_from_slice(&(self.entries.len() as u32).to_be_bytes());
        c += 4;
        for e in &self.entries {
            c += match e {
                SampleEntryVariant::Avc1(a) => a.serialize_into(&mut buf[c..])?,
                SampleEntryVariant::Hevc1(h) => h.serialize_into(&mut buf[c..])?,
                SampleEntryVariant::Mp4a(m) => m.serialize_into(&mut buf[c..])?,
                SampleEntryVariant::Unknown(u) => u.serialize_into(&mut buf[c..])?,
            };
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// stbl children that we preserve as opaque (stss, sgpd, sbgp)
// ---------------------------------------------------------------------------

/// Opaque stbl child box (stss, sgpd, sbgp, etc.)
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct StblOpaque {
    /// The full box bytes including 8-byte header.
    pub data: Vec<u8>,
}

// ---------------------------------------------------------------------------
// Helper: parse a list of child boxes from container body and return typed
// variants via an enum.  Used by the container types below.
// ---------------------------------------------------------------------------

/// A single child within an stbl container.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum StblChild {
    Stsd(SampleDescriptionBox),
    Stts(crate::timing::TimeToSampleBox),
    Ctts(crate::timing::CompositionOffsetBox),
    Stsc(SampleToChunkBox),
    Stsz(SampleSizeBox),
    Stco(ChunkOffsetBox),
    Opaque(Vec<u8>),
}

fn parse_stbl_children(body: &[u8]) -> Vec<StblChild> {
    let mut children = Vec::new();
    let mut off = 0usize;
    while off + 8 <= body.len() {
        let size =
            u32::from_be_bytes([body[off], body[off + 1], body[off + 2], body[off + 3]]) as usize;
        if size < 8 {
            break;
        }
        let boxtype = [body[off + 4], body[off + 5], body[off + 6], body[off + 7]];
        let box_bytes = &body[off..off + size.min(body.len() - off)];
        children.push(match &boxtype {
            b"stsd" => {
                StblChild::Stsd(SampleDescriptionBox::parse(box_bytes).unwrap_or_else(|_| {
                    SampleDescriptionBox {
                        version: 0,
                        flags: 0,
                        entries: Vec::new(),
                    }
                }))
            }
            b"stts" => StblChild::Stts(
                crate::timing::TimeToSampleBox::parse(box_bytes).unwrap_or_else(|_| {
                    crate::timing::TimeToSampleBox {
                        version: 0,
                        flags: 0,
                        entries: Vec::new(),
                    }
                }),
            ),
            b"ctts" => StblChild::Ctts(
                crate::timing::CompositionOffsetBox::parse(box_bytes).unwrap_or_else(|_| {
                    crate::timing::CompositionOffsetBox {
                        version: 0,
                        flags: 0,
                        entries: Vec::new(),
                    }
                }),
            ),
            b"stsc" => StblChild::Stsc(SampleToChunkBox::parse(box_bytes).unwrap_or_else(|_| {
                SampleToChunkBox {
                    version: 0,
                    flags: 0,
                    entries: Vec::new(),
                }
            })),
            b"stsz" => {
                StblChild::Stsz(
                    SampleSizeBox::parse(box_bytes).unwrap_or_else(|_| SampleSizeBox {
                        version: 0,
                        flags: 0,
                        sample_size: 0,
                        entries: Vec::new(),
                    }),
                )
            }
            b"stco" => StblChild::Stco(ChunkOffsetBox::parse(box_bytes).unwrap_or_else(|_| {
                ChunkOffsetBox {
                    version: 0,
                    flags: 0,
                    entries: Vec::new(),
                }
            })),
            _ => StblChild::Opaque(box_bytes.to_vec()),
        });
        off += size;
    }
    children
}

fn serialize_stbl_children(children: &[StblChild], buf: &mut [u8], off: &mut usize) -> Result<()> {
    for child in children {
        match child {
            StblChild::Stsd(b) => *off += b.serialize_into(&mut buf[*off..])?,
            StblChild::Stts(b) => *off += b.serialize_into(&mut buf[*off..])?,
            StblChild::Ctts(b) => *off += b.serialize_into(&mut buf[*off..])?,
            StblChild::Stsc(b) => *off += b.serialize_into(&mut buf[*off..])?,
            StblChild::Stsz(b) => *off += b.serialize_into(&mut buf[*off..])?,
            StblChild::Stco(b) => *off += b.serialize_into(&mut buf[*off..])?,
            StblChild::Opaque(d) => {
                let len = d.len();
                buf[*off..*off + len].copy_from_slice(d);
                *off += len;
            }
        }
    }
    Ok(())
}

fn stbl_children_len(children: &[StblChild]) -> usize {
    let mut n = 0;
    for child in children {
        n += match child {
            StblChild::Stsd(b) => b.serialized_len(),
            StblChild::Stts(b) => b.serialized_len(),
            StblChild::Ctts(b) => b.serialized_len(),
            StblChild::Stsc(b) => b.serialized_len(),
            StblChild::Stsz(b) => b.serialized_len(),
            StblChild::Stco(b) => b.serialized_len(),
            StblChild::Opaque(d) => d.len(),
        };
    }
    n
}

// ---------------------------------------------------------------------------
// SampleTableBox — stbl (container)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SampleTableBox {
    pub children: Vec<StblChild>,
}

impl<'a> Parse<'a> for SampleTableBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        // Expect full box bytes (size+type header then body)
        if bytes.len() < 8 {
            return Err(Error::BufferTooShort {
                need: 8,
                have: bytes.len(),
                what: "stbl",
            });
        }
        let body = &bytes[8..];
        Ok(Self {
            children: parse_stbl_children(body),
        })
    }
}

impl Serialize for SampleTableBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        BOX_HDR + stbl_children_len(&self.children)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"stbl");
        c += 4;
        serialize_stbl_children(&self.children, buf, &mut c)?;
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// DataInformationBox — dinf (container: dref)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DataInformationBox {
    pub dref: Option<DataReferenceBox>,
    pub opaque: Vec<OpaqueBox>,
}

impl<'a> Parse<'a> for DataInformationBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 8 {
            return Err(Error::BufferTooShort {
                need: 8,
                have: bytes.len(),
                what: "dinf",
            });
        }
        let body = &bytes[8..];
        let mut dref = None;
        let mut opaque = Vec::new();
        let mut off = 0usize;
        while off + 8 <= body.len() {
            let size = u32::from_be_bytes([body[off], body[off + 1], body[off + 2], body[off + 3]])
                as usize;
            if size < 8 {
                break;
            }
            let boxtype = [body[off + 4], body[off + 5], body[off + 6], body[off + 7]];
            let box_bytes = &body[off..off + size.min(body.len() - off)];
            if &boxtype == b"dref" {
                dref = Some(DataReferenceBox::parse(box_bytes)?);
            } else {
                opaque.push(OpaqueBox::new(boxtype, box_bytes[8..].to_vec()));
            }
            off += size;
        }
        Ok(Self { dref, opaque })
    }
}

impl Serialize for DataInformationBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HDR;
        if let Some(ref d) = self.dref {
            n += d.serialized_len();
        }
        for o in &self.opaque {
            n += o.serialized_len();
        }
        n
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let mut children_len = 0usize;
        if let Some(ref d) = self.dref {
            children_len += d.serialized_len();
        }
        for o in &self.opaque {
            children_len += o.serialized_len();
        }
        let need = BOX_HDR + children_len;
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"dinf");
        c += 4;
        if let Some(ref d) = self.dref {
            c += d.serialize_into(&mut buf[c..])?;
        }
        for o in &self.opaque {
            c += o.serialize_into(&mut buf[c..])?;
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// MediaInformationBox — minf (container: vmhd/smhd, dinf, stbl)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MediaInformationBox {
    pub vmhd: Option<VideoMediaHeaderBox>,
    pub smhd: Option<SoundMediaHeaderBox>,
    pub dinf: Option<DataInformationBox>,
    pub stbl: Option<SampleTableBox>,
    pub opaque: Vec<OpaqueBox>,
}

impl<'a> Parse<'a> for MediaInformationBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 8 {
            return Err(Error::BufferTooShort {
                need: 8,
                have: bytes.len(),
                what: "minf",
            });
        }
        let body = &bytes[8..];
        let mut vmhd = None;
        let mut smhd = None;
        let mut dinf = None;
        let mut stbl = None;
        let mut opaque = Vec::new();
        let mut off = 0usize;
        while off + 8 <= body.len() {
            let size = u32::from_be_bytes([body[off], body[off + 1], body[off + 2], body[off + 3]])
                as usize;
            if size < 8 {
                break;
            }
            let boxtype = [body[off + 4], body[off + 5], body[off + 6], body[off + 7]];
            let box_bytes = &body[off..off + size.min(body.len() - off)];
            match &boxtype {
                b"vmhd" => vmhd = Some(VideoMediaHeaderBox::parse(box_bytes)?),
                b"smhd" => smhd = Some(SoundMediaHeaderBox::parse(box_bytes)?),
                b"dinf" => dinf = Some(DataInformationBox::parse(box_bytes)?),
                b"stbl" => stbl = Some(SampleTableBox::parse(box_bytes)?),
                _ => {
                    opaque.push(OpaqueBox::new(boxtype, box_bytes[8..].to_vec()));
                }
            }
            off += size;
        }
        Ok(Self {
            vmhd,
            smhd,
            dinf,
            stbl,
            opaque,
        })
    }
}

impl Serialize for MediaInformationBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HDR;
        if let Some(ref b) = self.vmhd {
            n += b.serialized_len();
        }
        if let Some(ref b) = self.smhd {
            n += b.serialized_len();
        }
        if let Some(ref b) = self.dinf {
            n += b.serialized_len();
        }
        if let Some(ref b) = self.stbl {
            n += b.serialized_len();
        }
        for o in &self.opaque {
            n += o.serialized_len();
        }
        n
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let mut children_len = 0usize;
        if let Some(ref b) = self.vmhd {
            children_len += b.serialized_len();
        }
        if let Some(ref b) = self.smhd {
            children_len += b.serialized_len();
        }
        if let Some(ref b) = self.dinf {
            children_len += b.serialized_len();
        }
        if let Some(ref b) = self.stbl {
            children_len += b.serialized_len();
        }
        for o in &self.opaque {
            children_len += o.serialized_len();
        }
        let need = BOX_HDR + children_len;
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"minf");
        c += 4;
        if let Some(ref b) = self.vmhd {
            c += b.serialize_into(&mut buf[c..])?;
        }
        if let Some(ref b) = self.smhd {
            c += b.serialize_into(&mut buf[c..])?;
        }
        if let Some(ref b) = self.dinf {
            c += b.serialize_into(&mut buf[c..])?;
        }
        if let Some(ref b) = self.stbl {
            c += b.serialize_into(&mut buf[c..])?;
        }
        for o in &self.opaque {
            c += o.serialize_into(&mut buf[c..])?;
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// MediaBox — mdia (container: mdhd, hdlr, minf)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MediaBox {
    pub mdhd: Option<MediaHeaderBox>,
    pub hdlr: Option<HandlerBox>,
    pub minf: Option<MediaInformationBox>,
    pub opaque: Vec<OpaqueBox>,
}

impl<'a> Parse<'a> for MediaBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 8 {
            return Err(Error::BufferTooShort {
                need: 8,
                have: bytes.len(),
                what: "mdia",
            });
        }
        let body = &bytes[8..];
        let mut mdhd = None;
        let mut hdlr = None;
        let mut minf = None;
        let mut opaque = Vec::new();
        let mut off = 0usize;
        while off + 8 <= body.len() {
            let size = u32::from_be_bytes([body[off], body[off + 1], body[off + 2], body[off + 3]])
                as usize;
            if size < 8 {
                break;
            }
            let boxtype = [body[off + 4], body[off + 5], body[off + 6], body[off + 7]];
            let box_bytes = &body[off..off + size.min(body.len() - off)];
            match &boxtype {
                b"mdhd" => mdhd = Some(MediaHeaderBox::parse(box_bytes)?),
                b"hdlr" => hdlr = Some(HandlerBox::parse(box_bytes)?),
                b"minf" => minf = Some(MediaInformationBox::parse(box_bytes)?),
                _ => {
                    opaque.push(OpaqueBox::new(boxtype, box_bytes[8..].to_vec()));
                }
            }
            off += size;
        }
        Ok(Self {
            mdhd,
            hdlr,
            minf,
            opaque,
        })
    }
}

impl Serialize for MediaBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HDR;
        if let Some(ref b) = self.mdhd {
            n += b.serialized_len();
        }
        if let Some(ref b) = self.hdlr {
            n += b.serialized_len();
        }
        if let Some(ref b) = self.minf {
            n += b.serialized_len();
        }
        for o in &self.opaque {
            n += o.serialized_len();
        }
        n
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let mut children_len = 0usize;
        if let Some(ref b) = self.mdhd {
            children_len += b.serialized_len();
        }
        if let Some(ref b) = self.hdlr {
            children_len += b.serialized_len();
        }
        if let Some(ref b) = self.minf {
            children_len += b.serialized_len();
        }
        for o in &self.opaque {
            children_len += o.serialized_len();
        }
        let need = BOX_HDR + children_len;
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"mdia");
        c += 4;
        if let Some(ref b) = self.mdhd {
            c += b.serialize_into(&mut buf[c..])?;
        }
        if let Some(ref b) = self.hdlr {
            c += b.serialize_into(&mut buf[c..])?;
        }
        if let Some(ref b) = self.minf {
            c += b.serialize_into(&mut buf[c..])?;
        }
        for o in &self.opaque {
            c += o.serialize_into(&mut buf[c..])?;
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// EditBox — edts (container: elst)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct EditBox {
    pub elst: Option<crate::timing::EditListBox>,
    pub opaque: Vec<OpaqueBox>,
}

impl<'a> Parse<'a> for EditBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 8 {
            return Err(Error::BufferTooShort {
                need: 8,
                have: bytes.len(),
                what: "edts",
            });
        }
        let body = &bytes[8..];
        let mut elst = None;
        let mut opaque = Vec::new();
        let mut off = 0usize;
        while off + 8 <= body.len() {
            let size = u32::from_be_bytes([body[off], body[off + 1], body[off + 2], body[off + 3]])
                as usize;
            if size < 8 {
                break;
            }
            let boxtype = [body[off + 4], body[off + 5], body[off + 6], body[off + 7]];
            let box_bytes = &body[off..off + size.min(body.len() - off)];
            if &boxtype == b"elst" {
                elst = Some(crate::timing::EditListBox::parse(box_bytes)?);
            } else {
                opaque.push(OpaqueBox::new(boxtype, box_bytes[8..].to_vec()));
            }
            off += size;
        }
        Ok(Self { elst, opaque })
    }
}

impl Serialize for EditBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HDR;
        if let Some(ref b) = self.elst {
            n += b.serialized_len();
        }
        for o in &self.opaque {
            n += o.serialized_len();
        }
        n
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let mut children_len = 0usize;
        if let Some(ref b) = self.elst {
            children_len += b.serialized_len();
        }
        for o in &self.opaque {
            children_len += o.serialized_len();
        }
        let need = BOX_HDR + children_len;
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"edts");
        c += 4;
        if let Some(ref b) = self.elst {
            c += b.serialize_into(&mut buf[c..])?;
        }
        for o in &self.opaque {
            c += o.serialize_into(&mut buf[c..])?;
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// TrackBox — trak (container: tkhd, edts?, mdia, …)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TrackBox {
    pub tkhd: TrackHeaderBox,
    pub edts: Option<EditBox>,
    pub mdia: Option<MediaBox>,
    pub opaque: Vec<OpaqueBox>,
}

impl<'a> Parse<'a> for TrackBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 8 {
            return Err(Error::BufferTooShort {
                need: 8,
                have: bytes.len(),
                what: "trak",
            });
        }
        let body = &bytes[8..];
        let mut tkhd = None;
        let mut edts = None;
        let mut mdia = None;
        let mut opaque = Vec::new();
        let mut off = 0usize;
        while off + 8 <= body.len() {
            let size = u32::from_be_bytes([body[off], body[off + 1], body[off + 2], body[off + 3]])
                as usize;
            if size < 8 {
                break;
            }
            let boxtype = [body[off + 4], body[off + 5], body[off + 6], body[off + 7]];
            let box_bytes = &body[off..off + size.min(body.len() - off)];
            match &boxtype {
                b"tkhd" => tkhd = Some(TrackHeaderBox::parse(box_bytes)?),
                b"edts" => edts = Some(EditBox::parse(box_bytes)?),
                b"mdia" => mdia = Some(MediaBox::parse(box_bytes)?),
                _ => {
                    opaque.push(OpaqueBox::new(boxtype, box_bytes[8..].to_vec()));
                }
            }
            off += size;
        }
        Ok(Self {
            tkhd: tkhd.ok_or(Error::BufferTooShort {
                need: 0,
                have: 0,
                what: "trak missing tkhd",
            })?,
            edts,
            mdia,
            opaque,
        })
    }
}

impl Serialize for TrackBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HDR + self.tkhd.serialized_len();
        if let Some(ref b) = self.edts {
            n += b.serialized_len();
        }
        if let Some(ref b) = self.mdia {
            n += b.serialized_len();
        }
        for o in &self.opaque {
            n += o.serialized_len();
        }
        n
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let mut children_len = self.tkhd.serialized_len();
        if let Some(ref b) = self.edts {
            children_len += b.serialized_len();
        }
        if let Some(ref b) = self.mdia {
            children_len += b.serialized_len();
        }
        for o in &self.opaque {
            children_len += o.serialized_len();
        }
        let need = BOX_HDR + children_len;
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"trak");
        c += 4;
        c += self.tkhd.serialize_into(&mut buf[c..])?;
        if let Some(ref b) = self.edts {
            c += b.serialize_into(&mut buf[c..])?;
        }
        if let Some(ref b) = self.mdia {
            c += b.serialize_into(&mut buf[c..])?;
        }
        for o in &self.opaque {
            c += o.serialize_into(&mut buf[c..])?;
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// MovieBox — moov (container: mvhd, trak*, …) — THE TOP-LEVEL TYPE
// ---------------------------------------------------------------------------

/// Track Extends Box (`trex`) — ISO/IEC 14496-12:2015 §8.8.3.
///
/// Declares per-track defaults for the samples carried in movie fragments. A
/// fragmented-init `moov` carries one `trex` per track inside [`MovieExtendsBox`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TrackExtendsBox {
    /// FullBox version (0).
    pub version: u8,
    /// FullBox flags (0).
    pub flags: u32,
    /// The track these defaults apply to.
    pub track_id: u32,
    /// Default `stsd` entry index (1-based).
    pub default_sample_description_index: u32,
    /// Default sample duration (movie timescale units).
    pub default_sample_duration: u32,
    /// Default sample size in bytes.
    pub default_sample_size: u32,
    /// Default per-sample flags (§8.8.3 sample flags layout).
    pub default_sample_flags: u32,
}

impl<'a> Parse<'a> for TrackExtendsBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 32 {
            return Err(Error::BufferTooShort {
                need: 32,
                have: bytes.len(),
                what: "trex",
            });
        }
        let body = &bytes[8..];
        let version = body[0];
        let flags = u32::from_be_bytes([0, body[1], body[2], body[3]]);
        Ok(Self {
            version,
            flags,
            track_id: u32::from_be_bytes([body[4], body[5], body[6], body[7]]),
            default_sample_description_index: u32::from_be_bytes([
                body[8], body[9], body[10], body[11],
            ]),
            default_sample_duration: u32::from_be_bytes([body[12], body[13], body[14], body[15]]),
            default_sample_size: u32::from_be_bytes([body[16], body[17], body[18], body[19]]),
            default_sample_flags: u32::from_be_bytes([body[20], body[21], body[22], body[23]]),
        })
    }
}

impl Serialize for TrackExtendsBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        32
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.len() < 32 {
            return Err(Error::OutputBufferTooSmall {
                need: 32,
                have: buf.len(),
            });
        }
        buf[0..4].copy_from_slice(&32u32.to_be_bytes());
        buf[4..8].copy_from_slice(b"trex");
        buf[8] = self.version;
        let fb = self.flags.to_be_bytes();
        buf[9..12].copy_from_slice(&fb[1..]);
        buf[12..16].copy_from_slice(&self.track_id.to_be_bytes());
        buf[16..20].copy_from_slice(&self.default_sample_description_index.to_be_bytes());
        buf[20..24].copy_from_slice(&self.default_sample_duration.to_be_bytes());
        buf[24..28].copy_from_slice(&self.default_sample_size.to_be_bytes());
        buf[28..32].copy_from_slice(&self.default_sample_flags.to_be_bytes());
        Ok(32)
    }
}

/// Movie Extends Box (`mvex`) — ISO/IEC 14496-12:2015 §8.8.1.
///
/// Signals that the movie is fragmented and carries the per-track [`TrackExtendsBox`]
/// defaults. Any other children (e.g. `mehd`) are preserved verbatim in `opaque`;
/// note the spec orders `mehd` before the `trex` list, so `opaque` is serialized
/// last (correct for the common `trex`-only case built by the remux pipeline).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MovieExtendsBox {
    /// One `trex` per track.
    pub trex: Vec<TrackExtendsBox>,
    /// Other `mvex` children preserved verbatim (e.g. `mehd`).
    pub opaque: Vec<OpaqueBox>,
}

impl<'a> Parse<'a> for MovieExtendsBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 8 {
            return Err(Error::BufferTooShort {
                need: 8,
                have: bytes.len(),
                what: "mvex",
            });
        }
        let body = &bytes[8..];
        let mut trex = Vec::new();
        let mut opaque = Vec::new();
        let mut off = 0usize;
        while off + 8 <= body.len() {
            let size = u32::from_be_bytes([body[off], body[off + 1], body[off + 2], body[off + 3]])
                as usize;
            if size < 8 {
                break;
            }
            let boxtype = [body[off + 4], body[off + 5], body[off + 6], body[off + 7]];
            let box_bytes = &body[off..off + size.min(body.len() - off)];
            match &boxtype {
                b"trex" => trex.push(TrackExtendsBox::parse(box_bytes)?),
                _ => opaque.push(OpaqueBox::new(boxtype, box_bytes[8..].to_vec())),
            }
            off += size;
        }
        Ok(Self { trex, opaque })
    }
}

impl Serialize for MovieExtendsBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HDR;
        for t in &self.trex {
            n += t.serialized_len();
        }
        for o in &self.opaque {
            n += o.serialized_len();
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
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"mvex");
        c += 4;
        for t in &self.trex {
            c += t.serialize_into(&mut buf[c..])?;
        }
        for o in &self.opaque {
            c += o.serialize_into(&mut buf[c..])?;
        }
        Ok(c)
    }
}

/// Movie Box (`moov`) — §8.2.1.  The top-level init-segment container.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MovieBox {
    pub mvhd: MovieHeaderBox,
    pub tracks: Vec<TrackBox>,
    /// Movie-extends box (`mvex`) present in fragmented-init movies.
    pub mvex: Option<MovieExtendsBox>,
    pub opaque: Vec<OpaqueBox>,
}

impl<'a> Parse<'a> for MovieBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < 8 {
            return Err(Error::BufferTooShort {
                need: 8,
                have: bytes.len(),
                what: "moov",
            });
        }
        let body = &bytes[8..];
        let mut mvhd = None;
        let mut tracks = Vec::new();
        let mut mvex = None;
        let mut opaque = Vec::new();
        let mut off = 0usize;
        while off + 8 <= body.len() {
            let size = u32::from_be_bytes([body[off], body[off + 1], body[off + 2], body[off + 3]])
                as usize;
            if size < 8 {
                break;
            }
            let boxtype = [body[off + 4], body[off + 5], body[off + 6], body[off + 7]];
            let box_bytes = &body[off..off + size.min(body.len() - off)];
            match &boxtype {
                b"mvhd" => mvhd = Some(MovieHeaderBox::parse(box_bytes)?),
                b"trak" => tracks.push(TrackBox::parse(box_bytes)?),
                b"mvex" => mvex = Some(MovieExtendsBox::parse(box_bytes)?),
                _ => {
                    opaque.push(OpaqueBox::new(boxtype, box_bytes[8..].to_vec()));
                }
            }
            off += size;
        }
        Ok(Self {
            mvhd: mvhd.ok_or(Error::BufferTooShort {
                need: 0,
                have: 0,
                what: "moov missing mvhd",
            })?,
            tracks,
            mvex,
            opaque,
        })
    }
}

impl Serialize for MovieBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HDR + self.mvhd.serialized_len();
        for t in &self.tracks {
            n += t.serialized_len();
        }
        if let Some(mvex) = &self.mvex {
            n += mvex.serialized_len();
        }
        for o in &self.opaque {
            n += o.serialized_len();
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
        let mut c = 0usize;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"moov");
        c += 4;
        c += self.mvhd.serialize_into(&mut buf[c..])?;
        for t in &self.tracks {
            c += t.serialize_into(&mut buf[c..])?;
        }
        if let Some(mvex) = &self.mvex {
            c += mvex.serialize_into(&mut buf[c..])?;
        }
        for o in &self.opaque {
            c += o.serialize_into(&mut buf[c..])?;
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_common::Serialize;

    /// Build a small mvhd v0 for unit testing.
    fn sample_mvhd_v0() -> MovieHeaderBox {
        MovieHeaderBox {
            version: 0,
            flags: 0,
            creation_time: 0,
            modification_time: 0,
            timescale: 1000,
            duration: 2000,
            rate: 0x00010000,
            volume: 0x0100,
            matrix: [0x00010000, 0, 0, 0, 0x00010000, 0, 0, 0, 0x40000000],
            next_track_id: 3,
        }
    }

    #[test]
    fn mvhd_v0_round_trip() {
        let m = sample_mvhd_v0();
        let bytes = m.to_bytes();
        let parsed = MovieHeaderBox::parse(&bytes).unwrap();
        assert_eq!(parsed, m);
    }

    #[test]
    fn mvhd_v0_mutation_changes_bytes() {
        let m = sample_mvhd_v0();
        let orig = m.to_bytes();
        let mut m2 = m.clone();
        m2.timescale = 30000;
        let mutated = m2.to_bytes();
        assert_ne!(orig, mutated);
        // Verify the right field changed
        assert_ne!(orig[20..24], mutated[20..24]);
    }

    #[test]
    fn tkhd_v0_round_trip() {
        let t = TrackHeaderBox {
            version: 0,
            flags: 0x000003, // track_enabled | track_in_movie
            creation_time: 0,
            modification_time: 0,
            track_id: 1,
            duration: 2000,
            layer: 0,
            alternate_group: 0,
            volume: 0,
            matrix: [0x00010000, 0, 0, 0, 0x00010000, 0, 0, 0, 0x40000000],
            width: 0,
            height: 0,
        };
        let bytes = t.to_bytes();
        let parsed = TrackHeaderBox::parse(&bytes).unwrap();
        assert_eq!(parsed, t);
    }

    #[test]
    fn stsc_round_trip() {
        let s = SampleToChunkBox {
            version: 0,
            flags: 0,
            entries: alloc::vec![StscEntry {
                first_chunk: 1,
                samples_per_chunk: 10,
                sample_description_index: 1
            },],
        };
        let bytes = s.to_bytes();
        let parsed = SampleToChunkBox::parse(&bytes).unwrap();
        assert_eq!(parsed, s);
    }

    #[test]
    fn stsz_uniform_round_trip() {
        let s = SampleSizeBox {
            version: 0,
            flags: 0,
            sample_size: 512,
            entries: alloc::vec![],
        };
        let bytes = s.to_bytes();
        let parsed = SampleSizeBox::parse(&bytes).unwrap();
        assert_eq!(parsed, s);
    }

    #[test]
    fn stco_round_trip() {
        let s = ChunkOffsetBox {
            version: 0,
            flags: 0,
            entries: alloc::vec![0, 1024, 2048, 4096],
        };
        let bytes = s.to_bytes();
        let parsed = ChunkOffsetBox::parse(&bytes).unwrap();
        assert_eq!(parsed, s);
    }

    #[test]
    fn dref_url_round_trip() {
        let url = DataEntryUrlBox {
            version: 0,
            flags: 1,
            location: alloc::vec![],
        };
        let dref = DataReferenceBox {
            version: 0,
            flags: 0,
            entries: alloc::vec![url],
        };
        let bytes = dref.to_bytes();
        let parsed = DataReferenceBox::parse(&bytes).unwrap();
        assert_eq!(parsed, dref);
    }
}
