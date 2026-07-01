//! Segment-level ISOBMFF boxes — ISO/IEC 14496-12:2015.
//!
//! These are the boxes that wrap a CMAF/fMP4 stream but sit *outside* the
//! `moov`/`moof` trees:
//!
//! - [`FileTypeBox`] (`ftyp`, §4.3) — brand declaration at the head of an init segment.
//! - [`SegmentTypeBox`] (`styp`, §8.16.2) — brand declaration at the head of a media segment.
//! - [`MediaDataBox`] (`mdat`, §8.1.1) — the container carrying coded sample data.
//!
//! `ftyp` and `styp` are byte-identical in layout (major brand + minor version +
//! a list of compatible brands); only the four-CC differs.

use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::Serialize;

/// Four-CC of the File Type Box.
pub const FTYP: [u8; 4] = *b"ftyp";
/// Four-CC of the Segment Type Box.
pub const STYP: [u8; 4] = *b"styp";
/// Four-CC of the Media Data Box.
pub const MDAT: [u8; 4] = *b"mdat";

/// A brand / compatible-brand list, shared by `ftyp` and `styp`.
///
/// Layout (ISO/IEC 14496-12:2015 §4.3): `major_brand(32) minor_version(32)
/// compatible_brands[](32 each, filling the box)`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
struct BrandBox {
    major_brand: [u8; 4],
    minor_version: u32,
    compatible_brands: Vec<[u8; 4]>,
}

impl BrandBox {
    fn body_len(&self) -> usize {
        // major_brand(4) + minor_version(4) + N compatible brands (4 each)
        4 + 4 + self.compatible_brands.len() * 4
    }

    fn parse_body(body: &[u8]) -> Result<Self> {
        if body.len() < 8 {
            return Err(Error::BufferTooShort {
                need: 8,
                have: body.len(),
                what: "ftyp/styp body",
            });
        }
        let major_brand = [body[0], body[1], body[2], body[3]];
        let minor_version = u32::from_be_bytes([body[4], body[5], body[6], body[7]]);
        let mut compatible_brands = Vec::new();
        let mut off = 8;
        while off + 4 <= body.len() {
            compatible_brands.push([body[off], body[off + 1], body[off + 2], body[off + 3]]);
            off += 4;
        }
        Ok(Self {
            major_brand,
            minor_version,
            compatible_brands,
        })
    }

    fn serialize_into(&self, four_cc: &[u8; 4], buf: &mut [u8]) -> Result<usize> {
        let need = 8 + self.body_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        buf[0..4].copy_from_slice(&(need as u32).to_be_bytes());
        buf[4..8].copy_from_slice(four_cc);
        buf[8..12].copy_from_slice(&self.major_brand);
        buf[12..16].copy_from_slice(&self.minor_version.to_be_bytes());
        let mut off = 16;
        for brand in &self.compatible_brands {
            buf[off..off + 4].copy_from_slice(brand);
            off += 4;
        }
        Ok(need)
    }
}

/// File Type Box (`ftyp`) — ISO/IEC 14496-12:2015 §4.3.
///
/// The first box of an initialization segment; declares the specification the
/// file conforms to via a major brand plus a list of compatible brands.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct FileTypeBox {
    /// Best-use brand (e.g. `iso5`).
    pub major_brand: [u8; 4],
    /// Minor version of the major brand.
    pub minor_version: u32,
    /// Brands the file is compatible with.
    pub compatible_brands: Vec<[u8; 4]>,
}

impl FileTypeBox {
    /// Parse a whole `ftyp` box (including the 8-byte header).
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        let b = BrandBox::parse_body(box_body(bytes, &FTYP)?)?;
        Ok(Self {
            major_brand: b.major_brand,
            minor_version: b.minor_version,
            compatible_brands: b.compatible_brands,
        })
    }

    fn brand(&self) -> BrandBox {
        BrandBox {
            major_brand: self.major_brand,
            minor_version: self.minor_version,
            compatible_brands: self.compatible_brands.clone(),
        }
    }
}

impl Serialize for FileTypeBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        8 + self.brand().body_len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        self.brand().serialize_into(&FTYP, buf)
    }
}

/// Segment Type Box (`styp`) — ISO/IEC 14496-12:2015 §8.16.2.
///
/// The first box of a media segment; same layout as [`FileTypeBox`] but with
/// the `styp` four-CC.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SegmentTypeBox {
    /// Best-use brand (e.g. `msdh`).
    pub major_brand: [u8; 4],
    /// Minor version of the major brand.
    pub minor_version: u32,
    /// Brands the segment is compatible with.
    pub compatible_brands: Vec<[u8; 4]>,
}

impl SegmentTypeBox {
    /// Parse a whole `styp` box (including the 8-byte header).
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        let b = BrandBox::parse_body(box_body(bytes, &STYP)?)?;
        Ok(Self {
            major_brand: b.major_brand,
            minor_version: b.minor_version,
            compatible_brands: b.compatible_brands,
        })
    }

    fn brand(&self) -> BrandBox {
        BrandBox {
            major_brand: self.major_brand,
            minor_version: self.minor_version,
            compatible_brands: self.compatible_brands.clone(),
        }
    }
}

impl Serialize for SegmentTypeBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        8 + self.brand().body_len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        self.brand().serialize_into(&STYP, buf)
    }
}

/// Media Data Box (`mdat`) — ISO/IEC 14496-12:2015 §8.1.1.
///
/// Opaque container for coded sample data. Uses the 32-bit compact size form
/// when the box fits in `u32`, otherwise the 64-bit `largesize` form (`size==1`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MediaDataBox {
    /// The coded sample payload.
    pub data: Vec<u8>,
}

impl MediaDataBox {
    /// Whether this box must use the 64-bit `largesize` form.
    fn needs_large(&self) -> bool {
        8 + self.data.len() > u32::MAX as usize
    }

    /// Parse a whole `mdat` box (including the header), handling both the
    /// compact 32-bit and the 64-bit `largesize` forms.
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 8 {
            return Err(Error::BufferTooShort {
                need: 8,
                have: bytes.len(),
                what: "mdat",
            });
        }
        if bytes[4..8] != MDAT {
            return Err(Error::UnexpectedBox { expected: "mdat" });
        }
        let size32 = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let (size, hdr) = if size32 == 1 {
            if bytes.len() < 16 {
                return Err(Error::BufferTooShort {
                    need: 16,
                    have: bytes.len(),
                    what: "mdat largesize",
                });
            }
            (
                u64::from_be_bytes([
                    bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14],
                    bytes[15],
                ]) as usize,
                16usize,
            )
        } else if size32 == 0 {
            (bytes.len(), 8usize)
        } else {
            (size32 as usize, 8usize)
        };
        if size < hdr || size > bytes.len() {
            return Err(Error::BufferTooShort {
                need: size,
                have: bytes.len(),
                what: "mdat payload",
            });
        }
        Ok(Self {
            data: bytes[hdr..size].to_vec(),
        })
    }
}

impl Serialize for MediaDataBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        if self.needs_large() {
            16 + self.data.len()
        } else {
            8 + self.data.len()
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
        let hdr = if self.needs_large() {
            buf[0..4].copy_from_slice(&1u32.to_be_bytes());
            buf[4..8].copy_from_slice(&MDAT);
            buf[8..16].copy_from_slice(&(need as u64).to_be_bytes());
            16
        } else {
            buf[0..4].copy_from_slice(&(need as u32).to_be_bytes());
            buf[4..8].copy_from_slice(&MDAT);
            8
        };
        buf[hdr..hdr + self.data.len()].copy_from_slice(&self.data);
        Ok(need)
    }
}

/// Validate a whole box's four-CC and return its body (bytes after the 8-byte
/// header). Only the compact 32-bit size form is supported here (`ftyp`/`styp`
/// are always small).
fn box_body<'a>(bytes: &'a [u8], four_cc: &[u8; 4]) -> Result<&'a [u8]> {
    if bytes.len() < 8 {
        return Err(Error::BufferTooShort {
            need: 8,
            have: bytes.len(),
            what: "box header",
        });
    }
    if bytes[4..8] != *four_cc {
        return Err(Error::UnexpectedBox {
            expected: "ftyp/styp",
        });
    }
    let size = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
    if size < 8 || size > bytes.len() {
        return Err(Error::BufferTooShort {
            need: size,
            have: bytes.len(),
            what: "box size",
        });
    }
    Ok(&bytes[8..size])
}

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_common::Serialize;

    #[test]
    fn ftyp_round_trip() {
        // From h264_aac_frag.mp4: iso5 / minor 512 / [iso5, iso6, mp41]
        let bytes = [
            0x00, 0x00, 0x00, 0x1c, b'f', b't', b'y', b'p', b'i', b's', b'o', b'5', 0x00, 0x00,
            0x02, 0x00, b'i', b's', b'o', b'5', b'i', b's', b'o', b'6', b'm', b'p', b'4', b'1',
        ];
        let f = FileTypeBox::parse_box(&bytes).unwrap();
        assert_eq!(&f.major_brand, b"iso5");
        assert_eq!(f.minor_version, 512);
        assert_eq!(f.compatible_brands.len(), 3);
        assert_eq!(&f.compatible_brands[2], b"mp41");
        let mut out = alloc::vec![0u8; f.serialized_len()];
        let n = f.serialize_into(&mut out).unwrap();
        assert_eq!(&out[..n], &bytes, "ftyp round-trip byte-identical");
    }

    #[test]
    fn styp_round_trip() {
        let f = SegmentTypeBox {
            major_brand: *b"msdh",
            minor_version: 0,
            compatible_brands: alloc::vec![*b"msdh", *b"msix"],
        };
        let mut out = alloc::vec![0u8; f.serialized_len()];
        let n = f.serialize_into(&mut out).unwrap();
        assert_eq!(&out[4..8], b"styp");
        let g = SegmentTypeBox::parse_box(&out[..n]).unwrap();
        assert_eq!(f, g);
    }

    #[test]
    fn mdat_round_trip() {
        let m = MediaDataBox {
            data: alloc::vec![1, 2, 3, 4, 5],
        };
        let mut out = alloc::vec![0u8; m.serialized_len()];
        let n = m.serialize_into(&mut out).unwrap();
        assert_eq!(n, 13);
        assert_eq!(&out[4..8], b"mdat");
        let g = MediaDataBox::parse_box(&out[..n]).unwrap();
        assert_eq!(m, g);
    }

    #[test]
    fn mdat_mutation_changes_bytes() {
        let m = MediaDataBox {
            data: alloc::vec![1, 2, 3],
        };
        let mut a = alloc::vec![0u8; m.serialized_len()];
        m.serialize_into(&mut a).unwrap();
        let m2 = MediaDataBox {
            data: alloc::vec![1, 2, 4],
        };
        let mut b = alloc::vec![0u8; m2.serialized_len()];
        m2.serialize_into(&mut b).unwrap();
        assert_ne!(a, b);
    }
}
