//! Segment-level ISOBMFF boxes — ISO/IEC 14496-12:2026.
//!
//! These are the boxes that wrap a CMAF/fMP4 stream but sit *outside* the
//! `moov`/`moof` trees:
//!
//! - [`FileTypeBox`] (`ftyp`, §5.2) — brand declaration at the head of an init segment.
//! - [`SegmentTypeBox`] (`styp`, §8.14.2) — brand declaration at the head of a media segment.
//! - [`MediaDataBox`] (`mdat`, §8.1.1) — the container carrying coded sample data.
//!
//! `ftyp` and `styp` are byte-identical in layout (major brand + minor version +
//! a list of compatible brands); only the four-CC differs.

use crate::box_types::{BOX_HEADER_MIN_SIZE, BOX_TYPE_SIZE, LARGESIZE_SIZE};
use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::Serialize;
use broadcast_common::error::BufferTooShort;
use broadcast_common::len::{need_in, need_out};

const BRAND_BOX_FIXED_LEN: usize = BOX_TYPE_SIZE + core::mem::size_of::<u32>();
const MINOR_VERSION_OFFSET: usize = BOX_TYPE_SIZE;
const BRANDS_START_OFFSET: usize = BRAND_BOX_FIXED_LEN;

/// Four-CC of the File Type Box.
pub const FTYP: [u8; 4] = *b"ftyp";
/// Four-CC of the Segment Type Box.
pub const STYP: [u8; 4] = *b"styp";
/// Four-CC of the Media Data Box.
pub const MDAT: [u8; 4] = *b"mdat";
/// Four-CC of the Free Space Box (ISO/IEC 14496-12:2026 §8.1.2).
pub const FREE: [u8; 4] = *b"free";
/// Four-CC of the Skip Box (ISO/IEC 14496-12:2026 §8.1.2).
pub const SKIP: [u8; 4] = *b"skip";

/// A brand / compatible-brand list, shared by `ftyp` and `styp`.
///
/// Layout (ISO/IEC 14496-12:2026 §5.2): `major_brand(32) minor_version(32)
/// compatible_brands[](32 each, filling the box)`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
struct BrandBox {
    major_brand: [u8; BOX_TYPE_SIZE],
    minor_version: u32,
    compatible_brands: Vec<[u8; BOX_TYPE_SIZE]>,
}

impl BrandBox {
    fn body_len(&self) -> usize {
        // major_brand(4) + minor_version(4) + N compatible brands (4 each)
        BRAND_BOX_FIXED_LEN + self.compatible_brands.len() * BOX_TYPE_SIZE
    }

    fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, BRAND_BOX_FIXED_LEN, "ftyp/styp body")?;
        let mut major_brand = [0u8; BOX_TYPE_SIZE];
        major_brand.copy_from_slice(&body[..BOX_TYPE_SIZE]);
        let minor_version =
            broadcast_common::wire::be_u32(body, MINOR_VERSION_OFFSET, "ftyp/styp body")?;
        let mut compatible_brands = Vec::new();
        let mut off = BRANDS_START_OFFSET;
        while off + BOX_TYPE_SIZE <= body.len() {
            let mut brand = [0u8; BOX_TYPE_SIZE];
            brand.copy_from_slice(&body[off..off + BOX_TYPE_SIZE]);
            compatible_brands.push(brand);
            off += BOX_TYPE_SIZE;
        }
        Ok(Self {
            major_brand,
            minor_version,
            compatible_brands,
        })
    }

    fn serialize_into(&self, four_cc: &[u8; BOX_TYPE_SIZE], buf: &mut [u8]) -> Result<usize> {
        let need = BOX_HEADER_MIN_SIZE + self.body_len();
        need_out(buf, need)?;
        // `ftyp`/`styp` are brand lists and their reader takes the compact
        // form only; a size past 32 bits is a typed error, never a wrap
        // (#1147).
        let size = broadcast_common::len::fit_u32(need, "ftyp/styp size")?;
        buf[..core::mem::size_of::<u32>()].copy_from_slice(&size.to_be_bytes());
        buf[core::mem::size_of::<u32>()..BOX_HEADER_MIN_SIZE].copy_from_slice(four_cc);
        buf[BOX_HEADER_MIN_SIZE..BOX_HEADER_MIN_SIZE + BOX_TYPE_SIZE]
            .copy_from_slice(&self.major_brand);
        buf[BOX_HEADER_MIN_SIZE + BOX_TYPE_SIZE..BOX_HEADER_MIN_SIZE + BRAND_BOX_FIXED_LEN]
            .copy_from_slice(&self.minor_version.to_be_bytes());
        let mut off = BOX_HEADER_MIN_SIZE + BRANDS_START_OFFSET;
        for brand in &self.compatible_brands {
            buf[off..off + BOX_TYPE_SIZE].copy_from_slice(brand);
            off += BOX_TYPE_SIZE;
        }
        Ok(need)
    }
}

/// File Type Box (`ftyp`) — ISO/IEC 14496-12:2026 §5.2.
///
/// The first box of an initialization segment; declares the specification the
/// file conforms to via a major brand plus a list of compatible brands.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct FileTypeBox {
    /// Best-use brand (e.g. `iso5`).
    pub major_brand: [u8; BOX_TYPE_SIZE],
    /// Minor version of the major brand.
    pub minor_version: u32,
    /// Brands the file is compatible with.
    pub compatible_brands: Vec<[u8; BOX_TYPE_SIZE]>,
}

impl FileTypeBox {
    /// Parse a whole `ftyp` box (including the 8-byte header).
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        let b = BrandBox::parse_body(typed_body(bytes, &FTYP)?)?;
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
        BOX_HEADER_MIN_SIZE + self.brand().body_len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        self.brand().serialize_into(&FTYP, buf)
    }
}

/// Segment Type Box (`styp`) — ISO/IEC 14496-12:2026 §8.14.2.
///
/// The first box of a media segment; same layout as [`FileTypeBox`] but with
/// the `styp` four-CC.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SegmentTypeBox {
    /// Best-use brand (e.g. `msdh`).
    pub major_brand: [u8; BOX_TYPE_SIZE],
    /// Minor version of the major brand.
    pub minor_version: u32,
    /// Brands the segment is compatible with.
    pub compatible_brands: Vec<[u8; BOX_TYPE_SIZE]>,
}

impl SegmentTypeBox {
    /// Parse a whole `styp` box (including the 8-byte header).
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        let b = BrandBox::parse_body(typed_body(bytes, &STYP)?)?;
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
        BOX_HEADER_MIN_SIZE + self.brand().body_len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        self.brand().serialize_into(&STYP, buf)
    }
}

/// Media Data Box (`mdat`) — ISO/IEC 14496-12:2026 §8.1.1.
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
        BOX_HEADER_MIN_SIZE + self.data.len() > u32::MAX as usize
    }

    /// Parse a whole `mdat` box (including the header), handling both the
    /// compact 32-bit and the 64-bit `largesize` forms.
    pub fn parse_box(bytes: &[u8]) -> Result<Self> {
        need_in(bytes, BOX_HEADER_MIN_SIZE, "mdat")?;
        if bytes[core::mem::size_of::<u32>()..BOX_HEADER_MIN_SIZE] != MDAT {
            return Err(Error::UnexpectedBox { expected: "mdat" });
        }
        let size32 = broadcast_common::wire::be_u32(bytes, 0, "mdat")?;
        let (size, hdr) = if size32 == 1 {
            let large_header = BOX_HEADER_MIN_SIZE + LARGESIZE_SIZE;
            need_in(bytes, large_header, "mdat largesize")?;
            (
                // A `largesize` beyond `usize` can never fit `bytes`: saturate so
                // the size check below rejects it instead of truncating.
                usize::try_from(broadcast_common::wire::be_u64(
                    bytes,
                    BOX_HEADER_MIN_SIZE,
                    "mdat largesize",
                )?)
                .unwrap_or(usize::MAX),
                large_header,
            )
        } else if size32 == 0 {
            (bytes.len(), BOX_HEADER_MIN_SIZE)
        } else {
            (size32 as usize, BOX_HEADER_MIN_SIZE)
        };
        if size < hdr || size > bytes.len() {
            return Err(Error::BufferTooShort(BufferTooShort {
                need: size,
                have: bytes.len(),
                what: "mdat payload",
            }));
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
            BOX_HEADER_MIN_SIZE + LARGESIZE_SIZE + self.data.len()
        } else {
            BOX_HEADER_MIN_SIZE + self.data.len()
        }
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let hdr = if self.needs_large() {
            buf[..core::mem::size_of::<u32>()].copy_from_slice(&1u32.to_be_bytes());
            buf[core::mem::size_of::<u32>()..BOX_HEADER_MIN_SIZE].copy_from_slice(&MDAT);
            let large_header = BOX_HEADER_MIN_SIZE + LARGESIZE_SIZE;
            buf[BOX_HEADER_MIN_SIZE..large_header].copy_from_slice(&(need as u64).to_be_bytes());
            large_header
        } else {
            buf[..core::mem::size_of::<u32>()].copy_from_slice(&(need as u32).to_be_bytes());
            buf[core::mem::size_of::<u32>()..BOX_HEADER_MIN_SIZE].copy_from_slice(&MDAT);
            BOX_HEADER_MIN_SIZE
        };
        buf[hdr..hdr + self.data.len()].copy_from_slice(&self.data);
        Ok(need)
    }
}

/// The body of a whole `ftyp`/`styp` box, after validating its four-CC.
fn typed_body<'a>(bytes: &'a [u8], four_cc: &[u8; BOX_TYPE_SIZE]) -> Result<&'a [u8]> {
    let item = crate::box_types::parse_box(bytes)?.0;
    if !item.header.box_type.is(four_cc) {
        return Err(Error::UnexpectedBox {
            expected: "ftyp/styp",
        });
    }
    Ok(item.body)
}

/// Four-CC of the Movie Fragment Box.
pub const MOOF: [u8; 4] = *b"moof";

/// Whether `bytes` begin with a top-level box of type `box_type` — the one
/// place a caller asks "does this stream begin with a `moof`/`styp` box?"
/// instead of comparing four raw bytes itself.
#[must_use]
pub fn starts_with_box_type(bytes: &[u8], box_type: [u8; BOX_TYPE_SIZE]) -> bool {
    bytes.starts_with(&box_type)
}

/// Whether `bytes` begin with a top-level [`MOOF`] box.
#[must_use]
pub fn starts_with_moof(bytes: &[u8]) -> bool {
    starts_with_box_type(bytes, MOOF)
}

/// Splice serialized event message (`emsg`) boxes immediately after the first
/// top-level `styp` box of `segment` (the CMAF segment layout
/// `[styp][emsg*][moof][mdat]`, ISO/IEC 23000-19 §7.3.3). Box boundaries are
/// parsed, so a 64-bit `largesize` box before `styp` is stepped over correctly.
/// A segment with no `styp` gets the boxes prepended.
pub fn insert_event_message_boxes_after_styp(segment: &[u8], emsg_bytes: &[u8]) -> Vec<u8> {
    if emsg_bytes.is_empty() {
        return segment.to_vec();
    }
    let mut offset = 0usize;
    while offset < segment.len() {
        match crate::parse_box(&segment[offset..]) {
            Ok((box_ref, consumed)) if consumed > 0 => {
                if box_ref.header.box_type.is(&STYP) {
                    let mut out = Vec::with_capacity(segment.len() + emsg_bytes.len());
                    out.extend_from_slice(&segment[..offset + consumed]);
                    out.extend_from_slice(emsg_bytes);
                    out.extend_from_slice(&segment[offset + consumed..]);
                    return out;
                }
                offset += consumed;
            }
            _ => break,
        }
    }
    let mut out = Vec::with_capacity(segment.len() + emsg_bytes.len());
    out.extend_from_slice(emsg_bytes);
    out.extend_from_slice(segment);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain_box(four_cc: &[u8; 4], body_len: usize) -> Vec<u8> {
        let mut b = ((8 + body_len) as u32).to_be_bytes().to_vec();
        b.extend_from_slice(four_cc);
        b.extend(std::iter::repeat_n(0u8, body_len));
        b
    }

    fn large_box(four_cc: &[u8; 4], body_len: usize) -> Vec<u8> {
        let mut b = 1u32.to_be_bytes().to_vec();
        b.extend_from_slice(four_cc);
        b.extend_from_slice(&((16 + body_len) as u64).to_be_bytes());
        b.extend(std::iter::repeat_n(0u8, body_len));
        b
    }

    #[test]
    fn event_boxes_go_right_after_styp() {
        let styp = plain_box(b"styp", 8);
        let moof = plain_box(b"moof", 4);
        let emsg = plain_box(b"emsg", 3);
        let segment = [styp.clone(), moof.clone()].concat();
        let out = insert_event_message_boxes_after_styp(&segment, &emsg);
        assert_eq!(out, [styp, emsg, moof].concat());
    }

    #[test]
    fn event_boxes_are_prepended_without_styp() {
        let moof = plain_box(b"moof", 4);
        let emsg = plain_box(b"emsg", 3);
        let out = insert_event_message_boxes_after_styp(&moof, &emsg);
        assert_eq!(out, [emsg, moof].concat());
    }

    #[test]
    fn a_largesize_box_before_styp_is_stepped_over() {
        let lead = large_box(b"free", 5);
        let styp = plain_box(b"styp", 8);
        let moof = plain_box(b"moof", 4);
        let emsg = plain_box(b"emsg", 3);
        let segment = [lead.clone(), styp.clone(), moof.clone()].concat();
        let out = insert_event_message_boxes_after_styp(&segment, &emsg);
        assert_eq!(out, [lead, styp, emsg, moof].concat());
    }
    use broadcast_common::Serialize;

    #[test]
    fn ftyp_round_trip() {
        // From h264-aac-frag.mp4: iso5 / minor 512 / [iso5, iso6, mp41]
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

    #[test]
    fn mdat_parse_accepts_a_header_only_box() {
        let bytes = [0, 0, 0, 8, b'm', b'd', b'a', b't'];
        let m = MediaDataBox::parse_box(&bytes).unwrap();
        assert!(m.data.is_empty(), "size 8 leaves no payload");
    }

    #[test]
    fn mdat_parse_rejects_a_size_below_the_header() {
        let bytes = [0, 0, 0, 4, b'm', b'd', b'a', b't'];
        assert!(MediaDataBox::parse_box(&bytes).is_err());
    }

    #[test]
    fn mdat_parse_rejects_a_size_past_the_buffer() {
        let bytes = [0, 0, 0, 12, b'm', b'd', b'a', b't'];
        assert!(MediaDataBox::parse_box(&bytes).is_err());
    }

    #[test]
    fn mdat_parse_rejects_a_largesize_below_sixteen() {
        let mut bytes = 1u32.to_be_bytes().to_vec();
        bytes.extend_from_slice(b"mdat");
        bytes.extend_from_slice(&15u64.to_be_bytes());
        assert!(MediaDataBox::parse_box(&bytes).is_err());
    }

    #[test]
    fn mdat_parse_size_zero_takes_the_rest_of_the_buffer() {
        let mut bytes = 0u32.to_be_bytes().to_vec();
        bytes.extend_from_slice(b"mdat");
        bytes.extend_from_slice(&[1, 2, 3, 4, 5]);
        let m = MediaDataBox::parse_box(&bytes).unwrap();
        assert_eq!(m.data, alloc::vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn mdat_parse_compact_size_excludes_trailing_bytes() {
        let mut bytes = 12u32.to_be_bytes().to_vec();
        bytes.extend_from_slice(b"mdat");
        bytes.extend_from_slice(&[0xAA; 12]);
        assert_eq!(bytes.len(), 20);
        let m = MediaDataBox::parse_box(&bytes).unwrap();
        assert_eq!(m.data.len(), 4, "size 12 carries a 4-byte payload");
        assert_eq!(m.data, alloc::vec![0xAA; 4]);
    }

    #[test]
    fn header_only_box_parses_with_an_empty_body() {
        let bytes = [0, 0, 0, 8, b's', b't', b'y', b'p'];
        assert_eq!(typed_body(&bytes, &STYP).unwrap(), &[] as &[u8]);
    }

    #[test]
    fn typed_body_rejects_a_size_below_the_header_or_past_the_buffer() {
        let below = [0, 0, 0, 7, b's', b't', b'y', b'p'];
        assert!(typed_body(&below, &STYP).is_err());
        let past = [0, 0, 0, 16, b's', b't', b'y', b'p'];
        assert!(typed_body(&past, &STYP).is_err());
        assert!(SegmentTypeBox::parse_box(&below).is_err());
    }

    #[test]
    fn typed_body_excludes_trailing_bytes() {
        let mut bytes = plain_box(b"styp", 8);
        bytes.extend_from_slice(&[0xAA; 4]);
        let body = typed_body(&bytes, &STYP).unwrap();
        assert_eq!(body.len(), 8, "a size 16 box stops at byte 16");
        assert_eq!(body, &bytes[8..16]);
    }

    #[test]
    fn starts_with_helpers_distinguish_box_types() {
        assert!(starts_with_moof(b"moof...."));
        assert!(!starts_with_moof(b"mdat...."));
        assert!(!starts_with_moof(b"moo"));
        assert!(starts_with_box_type(b"styp..", *b"styp"));
        assert!(!starts_with_box_type(b"styp..", *b"moof"));
    }
}
