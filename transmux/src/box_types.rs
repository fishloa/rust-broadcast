//! ISOBMFF Box/FullBox layer — ISO/IEC 14496-12:2015 §4.2.
//!
//! Types for parsing and serialising ISO Base Media File Format (ISOBMFF) box headers.
//!
//! - [`BoxHeader`] — the 8-byte-or-larger header common to every box:
//!   `size`(32), `type`(32 four-CC), optional `largesize`(64 when size==1),
//!   optional `usertype`(128 when type==`uuid`), and `size==0` = to end of file.
//! - [`FullBoxHeader`] — extends [`BoxHeader`] with `version`(8) + `flags`(24).
//! - [`BoxRef`] — a borrowed view of a box: header + body bytes (a slice), for
//!   generic size-driven walking.
//! - [`box_iter`] — yield every top-level box in an ISOBMFF file by advancing
//!   via `size`; unknown `type` values are skipped (ISO/IEC 14496-12:2015 §4.2 L1294).
//!
//! # Wire layout (§4.2)
//!
//! ```text
//! Box {
//!   size        u(32)  — entire box in bytes (0 = to EOF); 1 => largesize
//!   type        u(32)  — four-CC (e.g. 'ftyp', 'moov', 'mdat')
//!   [largesize] u(64)  — only when size==1
//!   [usertype]  u(128) — only when type=='uuid'
//!   ...
//! }
//!
//! FullBox extends Box {
//!   version     u(8)
//!   flags       u(24)
//! }
//! ```
//!
//! All fields are big-endian. Sub-byte fields are packed MSB-first.
//! See ISO/IEC 14496-12:2015 §4.2 (fulltext L1254–L1304).

use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};
use core::fmt;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// The special size value `1` indicating a 64-bit `largesize` follows (ISO/IEC 14496-12:2015 §4.2).
pub const SIZE_INDICATES_LARGESIZE: u32 = 1;

/// The special size value `0` indicating the box runs to the end of the file (§4.2).
pub const SIZE_TO_EOF: u32 = 0;

/// The four-CC value for extended-type (`uuid`) boxes (§4.2).
pub const UUID_TYPE_BYTES: [u8; 4] = *b"uuid";

/// Size of the fixed-size Box header fields: `size`(32) + `type`(32) = 8 bytes.
pub const BOX_HEADER_MIN_SIZE: usize = 8;

/// Size of the 64-bit `largesize` field (ISO/IEC 14496-12:2015 §4.2).
pub const LARGESIZE_SIZE: usize = 8;

/// Size of the 128-bit `usertype` field for `uuid` boxes (§4.2).
pub const UUID_TYPE_SIZE: usize = 16;

/// Size of the FullBox extension fields: `version`(8) + `flags`(24) = 4 bytes.
pub const FULLBOX_EXTRA_SIZE: usize = 4;

// ---------------------------------------------------------------------------
// BoxType — 4-byte four-CC newtype
// ---------------------------------------------------------------------------

/// A 4-byte ISOBMFF box type (four-CC), stored as a raw byte array for simple
/// equality checks.
///
/// # Display
/// Renders the ASCII four-CC with underscores for non-printable bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct BoxType(pub [u8; 4]);

impl BoxType {
    /// Create a `BoxType` from a 4-byte array.
    pub const fn from_bytes(b: [u8; 4]) -> Self {
        Self(b)
    }

    /// Parse a 4-byte four-CC big-endian u32 into a `BoxType`.
    pub fn from_u32(v: u32) -> Self {
        Self(v.to_be_bytes())
    }

    /// Encode as a big-endian u32.
    pub fn to_u32(self) -> u32 {
        u32::from_be_bytes(self.0)
    }

    /// Check whether this type matches a static ASCII literal.
    ///
    /// ```
    /// # use transmux::box_types::BoxType;
    /// assert!(BoxType(*b"ftyp").is(b"ftyp"));
    /// assert!(!BoxType(*b"moov").is(b"mdat"));
    /// ```
    pub fn is(&self, literal: &[u8; 4]) -> bool {
        self.0 == *literal
    }

    /// Label for the box type, per the #204 convention.
    pub fn name(&self) -> &'static str {
        // Return a static ASCII representation — since four-CCs are a fixed
        // set in practice, this returns a best-effort string.
        match &self.0 {
            b"ftyp" => "ftyp",
            b"moov" => "moov",
            b"moof" => "moof",
            b"trak" => "trak",
            b"mdia" => "mdia",
            b"minf" => "minf",
            b"stbl" => "stbl",
            b"dinf" => "dinf",
            b"edts" => "edts",
            b"mvex" => "mvex",
            b"mvhd" => "mvhd",
            b"tkhd" => "tkhd",
            b"mdhd" => "mdhd",
            b"hdlr" => "hdlr",
            b"vmhd" => "vmhd",
            b"smhd" => "smhd",
            b"stsd" => "stsd",
            b"stts" => "stts",
            b"stsc" => "stsc",
            b"stsz" => "stsz",
            b"stco" => "stco",
            b"co64" => "co64",
            b"ctts" => "ctts",
            b"stss" => "stss",
            b"stsh" => "stsh",
            b"elst" => "elst",
            b"dref" => "dref",
            b"tref" => "tref",
            b"mdat" => "mdat",
            b"free" => "free",
            b"skip" => "skip",
            b"uuid" => "uuid",
            b"mfhd" => "mfhd",
            b"traf" => "traf",
            b"tfhd" => "tfhd",
            b"trun" => "trun",
            b"tfdt" => "tfdt",
            b"sidx" => "sidx",
            b"styp" => "styp",
            b"mfra" => "mfra",
            b"tfra" => "tfra",
            b"mfro" => "mfro",
            b"emsg" => "emsg",
            b"avc1" => "avc1",
            b"mp4a" => "mp4a",
            b"enca" => "enca",
            b"encv" => "encv",
            b"hvc1" => "hvc1",
            _ => "<unknown>",
        }
    }
}

impl fmt::Display for BoxType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for &b in &self.0 {
            if b.is_ascii_graphic() || b == b' ' {
                f.write_str(core::str::from_utf8(&[b]).unwrap_or("?"))?;
            } else {
                write!(f, "\\x{b:02x}")?;
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// BoxHeader
// ---------------------------------------------------------------------------

/// Parsed header of an ISOBMFF Box (ISO/IEC 14496-12:2015 §4.2).
///
/// Carries the decoded size, type, and conditional largesize/usertype fields.
/// All sizes are checked for consistency at parse time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct BoxHeader {
    /// Full size of the box in bytes (header + body), or `0` for to-EOF boxes.
    pub size: u64,
    /// Four-character code identifying the box type.
    pub box_type: BoxType,
    /// User-extended type (only present when `type == 'uuid'`).
    pub usertype: Option<[u8; UUID_TYPE_SIZE]>,
    /// Whether the wire encoding used 64-bit largesize (size field was 1).
    has_largesize: bool,
}

impl BoxHeader {
    /// Minimum header bytes this box header actually occupies on the wire.
    ///
    /// Distinguished from the constant `BOX_HEADER_MIN_SIZE` because a box with
    /// `size == 1` adds 8 more bytes, and a `uuid` type adds 16 more.
    pub fn header_size(&self) -> usize {
        let mut sz = BOX_HEADER_MIN_SIZE;
        if self.has_largesize {
            sz += LARGESIZE_SIZE;
        }
        if self.box_type.is(b"uuid") {
            sz += UUID_TYPE_SIZE;
        }
        sz
    }

    /// Build a header from its logical components.
    ///
    /// `has_largesize` is set to true when `size > u32::MAX` (the only situation
    /// that requires largesize on the wire). Callers constructing a box that fits
    /// in 32 bits do not need to worry about this: the serializer will write the
    /// compact form.
    pub fn new(size: u64, box_type: BoxType, usertype: Option<[u8; UUID_TYPE_SIZE]>) -> Self {
        let has_largesize = size > u32::MAX as u64;
        Self {
            size,
            box_type,
            usertype,
            has_largesize,
        }
    }
}

impl<'a> Parse<'a> for BoxHeader {
    type Error = Error;

    /// Parse a `BoxHeader` from the front of `bytes`.
    ///
    /// Returns the header and the number of bytes consumed. The caller should
    /// verify `header.size` against the full buffer.
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < BOX_HEADER_MIN_SIZE {
            return Err(Error::BufferTooShort {
                need: BOX_HEADER_MIN_SIZE,
                have: bytes.len(),
                what: "BoxHeader",
            });
        }

        let raw_size = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let box_type = BoxType::from_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);

        let mut cursor = BOX_HEADER_MIN_SIZE;

        // Decode the effective size.
        let size: u64 = if raw_size == SIZE_INDICATES_LARGESIZE {
            if bytes.len() < cursor + LARGESIZE_SIZE {
                return Err(Error::LargesizeBufferTooShort {
                    need: cursor + LARGESIZE_SIZE,
                    have: bytes.len(),
                });
            }
            let v = u64::from_be_bytes([
                bytes[cursor],
                bytes[cursor + 1],
                bytes[cursor + 2],
                bytes[cursor + 3],
                bytes[cursor + 4],
                bytes[cursor + 5],
                bytes[cursor + 6],
                bytes[cursor + 7],
            ]);
            cursor += LARGESIZE_SIZE;
            v
        } else {
            raw_size as u64
        };

        // Validate that size >= number of bytes we've consumed so far.
        if size != 0 && size < cursor as u64 {
            return Err(Error::BoxSizeUnderflow {
                size,
                header_size: cursor,
            });
        }

        // Decode usertype if uuid.
        let usertype = if box_type.is(b"uuid") {
            if bytes.len() < cursor + UUID_TYPE_SIZE {
                return Err(Error::UuidBufferTooShort {
                    need: cursor + UUID_TYPE_SIZE,
                    have: bytes.len(),
                });
            }
            let mut ut = [0u8; UUID_TYPE_SIZE];
            ut.copy_from_slice(&bytes[cursor..cursor + UUID_TYPE_SIZE]);
            Some(ut)
        } else {
            None
        };

        // Determine if largesize was actually used on the wire.
        let has_largesize = raw_size == SIZE_INDICATES_LARGESIZE;

        Ok(Self {
            size,
            box_type,
            usertype,
            has_largesize,
        })
    }
}

impl Serialize for BoxHeader {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        self.header_size()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.header_size();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }

        // Write size. If it fits in 32 bits, store as-is; otherwise write 1 (largesize indicator).
        let mut cursor = 0usize;
        if self.size > u32::MAX as u64 {
            // largesize path
            buf[0..4].copy_from_slice(&SIZE_INDICATES_LARGESIZE.to_be_bytes());
            cursor += 4;
            buf[cursor..cursor + 4].copy_from_slice(&self.box_type.to_u32().to_be_bytes());
            cursor += 4;
            buf[cursor..cursor + 8].copy_from_slice(&self.size.to_be_bytes());
            cursor += 8;
        } else {
            let size32 = self.size as u32;
            buf[0..4].copy_from_slice(&size32.to_be_bytes());
            cursor += 4;
            buf[cursor..cursor + 4].copy_from_slice(&self.box_type.to_u32().to_be_bytes());
            cursor += 4;
            if size32 == SIZE_INDICATES_LARGESIZE {
                // size==1 but fits in u32? That's 1, not largesize-worthy — fine.
            }
        }

        // usertype for uuid
        if self.box_type.is(b"uuid") {
            if let Some(ut) = &self.usertype {
                buf[cursor..cursor + UUID_TYPE_SIZE].copy_from_slice(ut);
                cursor += UUID_TYPE_SIZE;
            }
        }

        Ok(cursor)
    }
}

// ---------------------------------------------------------------------------
// FullBoxHeader
// ---------------------------------------------------------------------------

/// Extended header for a FullBox (ISO/IEC 14496-12:2015 §4.2 L1298): adds
/// `version`(8) + `flags`(24) after the base [`BoxHeader`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct FullBoxHeader {
    /// Base box header.
    pub box_header: BoxHeader,
    /// 8-bit version number.
    pub version: u8,
    /// 24-bit flags.
    pub flags: u32,
}

impl FullBoxHeader {
    /// Build a FullBoxHeader from a BoxHeader + version/flags.
    pub fn new(box_header: BoxHeader, version: u8, flags: u32) -> Self {
        debug_assert!(flags <= 0xFFFFFF, "flags must fit in 24 bits");
        Self {
            box_header,
            version,
            flags,
        }
    }
}

impl<'a> Parse<'a> for FullBoxHeader {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let box_header = BoxHeader::parse(bytes)?;
        let hdr_sz = box_header.header_size();

        if bytes.len() < hdr_sz + FULLBOX_EXTRA_SIZE {
            return Err(Error::BufferTooShort {
                need: hdr_sz + FULLBOX_EXTRA_SIZE,
                have: bytes.len(),
                what: "FullBoxHeader",
            });
        }

        let version = bytes[hdr_sz];
        let flags =
            u32::from_be_bytes([0, bytes[hdr_sz + 1], bytes[hdr_sz + 2], bytes[hdr_sz + 3]]);

        Ok(Self {
            box_header,
            version,
            flags,
        })
    }
}

impl Serialize for FullBoxHeader {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        self.box_header.serialized_len() + FULLBOX_EXTRA_SIZE
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let base_len = self.box_header.serialized_len();
        let need = base_len + FULLBOX_EXTRA_SIZE;
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }

        let cursor = self.box_header.serialize_into(buf)?;
        buf[cursor] = self.version;
        let flag_bytes = self.flags.to_be_bytes();
        buf[cursor + 1] = flag_bytes[1];
        buf[cursor + 2] = flag_bytes[2];
        buf[cursor + 3] = flag_bytes[3];
        Ok(cursor + FULLBOX_EXTRA_SIZE)
    }
}

// ---------------------------------------------------------------------------
// BoxRef — borrowed box view
// ---------------------------------------------------------------------------

/// A borrowed view of an entire ISOBMFF box: its header and its body bytes.
///
/// The body starts after the entire header (including optional largesize/usertype
/// and FullBox extension). The body length is `header.size - header.header_size()`
/// (for non-to-EOF boxes).
#[derive(Debug, Clone, Copy)]
pub struct BoxRef<'a> {
    /// The parsed header.
    pub header: BoxHeader,
    /// Body bytes (after all header + FullBox extras).
    pub body: &'a [u8],
}

/// Decode a `BoxRef` from the front of a byte buffer.
///
/// Returns the box and the number of bytes consumed. Parses only the header;
/// body is a slice of the remaining bytes up to `header.size` (or to EOF if
/// size == 0).
pub fn parse_box<'a>(bytes: &'a [u8]) -> Result<(BoxRef<'a>, usize)> {
    let header = BoxHeader::parse(bytes)?;
    let hdr_sz = header.header_size();

    let body: &'a [u8] = if header.size == SIZE_TO_EOF as u64 {
        // To end of buffer.
        &bytes[hdr_sz..]
    } else {
        let total = header.size as usize;
        if total < hdr_sz {
            return Err(Error::BoxSizeUnderflow {
                size: header.size,
                header_size: hdr_sz,
            });
        }
        if bytes.len() < total {
            return Err(Error::BufferTooShort {
                need: total,
                have: bytes.len(),
                what: "BoxRef body",
            });
        }
        &bytes[hdr_sz..total]
    };

    let consumed = if header.size == 0 {
        bytes.len()
    } else {
        header.size as usize
    };

    Ok((BoxRef { header, body }, consumed))
}

// ---------------------------------------------------------------------------
// BoxIter — size-driven box walker
// ---------------------------------------------------------------------------

/// An iterator that yields every top-level box in an ISOBMFF byte buffer.
///
/// The walk is **size-driven**: each step advances by the box's reported `size`
/// (or the remaining buffer for to-EOF boxes). Unknown box types are skipped —
/// the iterator yields only parseable boxes. This matches the spec requirement
/// (ISO/IEC 14496-12:2015 §4.2 L1294): "Unknown `type` → ignore + skip".
///
/// # Examples
///
/// ```
/// use transmux::box_iter;
///
/// # let bytes = &[];
/// // Walk all top-level boxes:
/// for result in box_iter(bytes) {
///     let (box_ref, consumed) = result.unwrap();
///     println!("{} at offset {}", box_ref.header.box_type, consumed);
/// }
/// ```
#[derive(Debug, Clone)]
pub struct BoxIter<'a> {
    remaining: &'a [u8],
}

impl<'a> BoxIter<'a> {
    /// Create a new iterator over boxes in `data`.
    pub fn new(data: &'a [u8]) -> Self {
        Self { remaining: data }
    }
}

impl<'a> Iterator for BoxIter<'a> {
    type Item = Result<(BoxRef<'a>, usize)>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining.is_empty() {
            return None;
        }

        match parse_box(self.remaining) {
            Ok((box_ref, consumed)) => {
                self.remaining = &self.remaining[consumed.min(self.remaining.len())..];
                Some(Ok((box_ref, consumed)))
            }
            Err(e) => {
                // Advance past the error point to avoid infinite loop.
                self.remaining = &[];
                Some(Err(e))
            }
        }
    }
}

/// Convenience function to create a `BoxIter`.
pub fn box_iter(data: &[u8]) -> BoxIter<'_> {
    BoxIter::new(data)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_common::{Parse, Serialize};

    // -- BoxHeader tests ---------------------------------------------------

    #[test]
    fn test_box_header_minimal() {
        // size=12, type="ftyp", no largesize, no uuid
        let bytes = [
            0, 0, 0, 12, // size = 12
            b'f', b't', b'y', b'p', // type = "ftyp"
            0, 0, 0, 0, // body padding
        ];
        let header = BoxHeader::parse(&bytes).unwrap();
        assert_eq!(header.size, 12);
        assert!(header.box_type.is(b"ftyp"));
        assert!(header.usertype.is_none());
        assert_eq!(header.header_size(), BOX_HEADER_MIN_SIZE);

        // Round-trip serialize
        let mut out = vec![0u8; header.serialized_len()];
        header.serialize_into(&mut out).unwrap();
        assert_eq!(&out, &bytes[..header.serialized_len()]);
    }

    #[test]
    fn test_box_header_largesize() {
        // size=1 indicator, type="mdat", largesize=1234567890123
        // parse_box validates that size >= cursor; we need a buffer large enough
        // for the header (8 + 8 = 16 bytes).
        let largesize: u64 = 1_234_567_890_123;
        let mut bytes = vec![0u8; 24]; // 8 header + 8 largesize + 8 body
        bytes[0..4].copy_from_slice(&SIZE_INDICATES_LARGESIZE.to_be_bytes());
        bytes[4..8].copy_from_slice(b"mdat");
        bytes[8..16].copy_from_slice(&largesize.to_be_bytes());

        let header = BoxHeader::parse(&bytes).unwrap();
        assert_eq!(header.size, largesize);
        assert!(header.box_type.is(b"mdat"));
        assert!(header.usertype.is_none());
        assert_eq!(header.header_size(), BOX_HEADER_MIN_SIZE + LARGESIZE_SIZE);

        // Round-trip serialize
        let mut out = vec![0u8; header.serialized_len()];
        header.serialize_into(&mut out).unwrap();
        assert_eq!(&out, &bytes[..header.serialized_len()]);
    }

    #[test]
    fn test_box_header_uuid() {
        // size=36 (8 header + 16 usertype + 12 body), type="uuid", usertype=16 bytes
        let usertype: [u8; 16] = [
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
            0x0f, 0x10,
        ];
        let mut bytes = Vec::new();
        let total_size: u32 = BOX_HEADER_MIN_SIZE as u32 + UUID_TYPE_SIZE as u32 + 12;
        bytes.extend_from_slice(&total_size.to_be_bytes());
        bytes.extend_from_slice(b"uuid");
        bytes.extend_from_slice(&usertype);
        bytes.resize(total_size as usize, 0xAB);

        let header = BoxHeader::parse(&bytes).unwrap();
        assert_eq!(header.size, total_size as u64);
        assert!(header.box_type.is(b"uuid"));
        assert_eq!(header.usertype, Some(usertype));
        assert_eq!(header.header_size(), BOX_HEADER_MIN_SIZE + UUID_TYPE_SIZE);
    }

    #[test]
    fn test_box_header_too_short() {
        let too_short = [0u8; 4];
        let err = BoxHeader::parse(&too_short).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { need: 8, .. }));
    }

    #[test]
    fn test_box_size_underflow() {
        // size=4 but header minimum is 8.
        let bytes = [0, 0, 0, 4, b'f', b't', b'y', b'p'];
        let err = BoxHeader::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::BoxSizeUnderflow { size: 4, .. }));
    }

    // -- FullBoxHeader tests -----------------------------------------------

    #[test]
    fn test_full_box_header() {
        // size=16 (8 header + 4 fullbox + 4 body), type="mvhd", version=1, flags=0x000003
        let mut bytes = vec![0u8; 16];
        bytes[0..4].copy_from_slice(&16u32.to_be_bytes());
        bytes[4..8].copy_from_slice(b"mvhd");
        bytes[8] = 1; // version
        bytes[9..12].copy_from_slice(&[0, 0, 3]); // flags

        let fh = FullBoxHeader::parse(&bytes).unwrap();
        assert!(fh.box_header.box_type.is(b"mvhd"));
        assert_eq!(fh.version, 1);
        assert_eq!(fh.flags, 3);

        // Round-trip serialize
        let mut out = vec![0u8; fh.serialized_len()];
        fh.serialize_into(&mut out).unwrap();
        assert_eq!(out, bytes[..fh.serialized_len()]);
    }

    // -- BoxRef tests ------------------------------------------------------

    #[test]
    fn test_parse_box() {
        let mut bytes = vec![0u8; 40];
        bytes[0..4].copy_from_slice(&20u32.to_be_bytes());
        bytes[4..8].copy_from_slice(b"ftyp");
        // body = bytes[8..20]
        for (i, b) in bytes.iter_mut().enumerate().take(20).skip(8) {
            *b = i as u8;
        }

        let (bx, consumed) = parse_box(&bytes).unwrap();
        assert_eq!(bx.header.size, 20);
        assert_eq!(bx.header.box_type.to_string(), "ftyp");
        assert_eq!(bx.body.len(), 12);
        assert_eq!(consumed, 20);
    }

    // -- BoxIter tests -----------------------------------------------------

    #[test]
    fn test_box_iter_empty() {
        let result: Vec<_> = box_iter(&[]).collect();
        assert!(result.is_empty());
    }

    #[test]
    fn test_box_iter_multiple() {
        // Three boxes: ftyp (16 bytes), moov (24 bytes), mdat (32 bytes)
        let mut data = Vec::new();
        // Box 1: ftyp (16)
        let b1_size = 16u32;
        data.extend_from_slice(&b1_size.to_be_bytes());
        data.extend_from_slice(b"ftyp");
        data.resize(16, 0);
        // Box 2: moov (24)
        let b2_size = 24u32;
        data.extend_from_slice(&b2_size.to_be_bytes());
        data.extend_from_slice(b"moov");
        data.resize(16 + 24, 0);
        // Box 3: mdat (32)
        let b3_size = 32u32;
        data.extend_from_slice(&b3_size.to_be_bytes());
        data.extend_from_slice(b"mdat");
        data.resize(16 + 24 + 32, 0);

        let boxes: Vec<_> = box_iter(&data).collect::<Result<Vec<_>>>().unwrap();
        assert_eq!(boxes.len(), 3);
        assert!(boxes[0].0.header.box_type.is(b"ftyp"));
        assert!(boxes[1].0.header.box_type.is(b"moov"));
        assert!(boxes[2].0.header.box_type.is(b"mdat"));
    }

    // -- Real-fixture round-trip test --------------------------------------

    #[test]
    fn test_round_trip_known_boxes() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../fixtures/transmux/h264_aac_frag.mp4"
        );
        let data = std::fs::read(path).expect("fixture file must exist");
        let original_len = data.len();

        // Re-serialize every top-level box into a flat buffer.
        let mut output = Vec::new();
        for result in box_iter(&data) {
            let (bx, _consumed) = result.unwrap();
            // Recompute the box header + body.
            let hdr_sz = bx.header.header_size();
            let total = hdr_sz + bx.body.len();
            let mut buf = vec![0u8; total];
            bx.header.serialize_into(&mut buf).unwrap();
            buf[hdr_sz..].copy_from_slice(bx.body);
            output.extend_from_slice(&buf);
        }

        assert_eq!(
            output.len(),
            original_len,
            "round-trip length mismatch: got {}, expected {}",
            output.len(),
            original_len
        );
        assert_eq!(output, data, "round-trip bytes differ from original");
    }

    // -- Skip-unknown box test ---------------------------------------------

    #[test]
    fn test_skip_unknown_box() {
        // Three boxes: ftyp (16), XXXX (24, unknown), moov (20, at offset 16+24=40)
        let mut data = Vec::new();
        // ftyp: 16 bytes
        data.extend_from_slice(&16u32.to_be_bytes());
        data.extend_from_slice(b"ftyp");
        data.resize(16, 0);
        // XXXX (unknown): 24 bytes
        data.extend_from_slice(&24u32.to_be_bytes());
        data.extend_from_slice(b"XXXX");
        data.resize(16 + 24, 0xFF);
        // moov: 20 bytes
        data.extend_from_slice(&20u32.to_be_bytes());
        data.extend_from_slice(b"moov");
        data.resize(16 + 24 + 20, 0xEE);

        let boxes: Vec<_> = box_iter(&data).collect::<Result<Vec<_>>>().unwrap();
        assert_eq!(
            boxes.len(),
            3,
            "should parse all three boxes including unknown"
        );
        assert!(boxes[0].0.header.box_type.is(b"ftyp"));
        assert!(
            boxes[1].0.header.box_type.is(b"XXXX"),
            "unknown box should parse by size"
        );
        assert!(boxes[2].0.header.box_type.is(b"moov"));
    }

    // -- Field mutation test (no raw passthrough) --------------------------

    #[test]
    fn test_field_mutation_changes_bytes() {
        // Build an moov box with body content, then mutate header size and verify.
        let body_content = [0xAA, 0xBB, 0xCC];

        let correct_size = (BOX_HEADER_MIN_SIZE + body_content.len()) as u64;

        let mut header = BoxHeader::new(correct_size, BoxType(*b"moov"), None);

        // Serialize with correct size.
        let mut buf = vec![0u8; header.serialized_len() + body_content.len()];
        let cursor = header.serialize_into(&mut buf).unwrap();
        buf[cursor..cursor + body_content.len()].copy_from_slice(&body_content);

        // Grab the serialized size bytes.
        let orig_size_bytes = [buf[0], buf[1], buf[2], buf[3]];

        // Mutate the size and serialize into a larger buffer.
        header.size = correct_size + 100;
        let new_len = header.serialized_len() + body_content.len() + 100;
        let mut buf2 = vec![0u8; new_len];
        let cursor2 = header.serialize_into(&mut buf2).unwrap();
        buf2[cursor2..cursor2 + body_content.len()].copy_from_slice(&body_content);

        let new_size_bytes = [buf2[0], buf2[1], buf2[2], buf2[3]];
        assert_ne!(
            orig_size_bytes, new_size_bytes,
            "mutating size must change serialized bytes"
        );
    }

    // -- To-EOF box test ---------------------------------------------------

    #[test]
    fn test_box_size_zero_to_eof() {
        // size=0, type="mdat" — body runs through the rest of the file.
        let body = [0u8; 100];
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0u32.to_be_bytes()); // size = 0
        bytes.extend_from_slice(b"mdat");
        bytes.extend_from_slice(&body);

        let header = BoxHeader::parse(&bytes).unwrap();
        assert_eq!(header.size, 0);
        assert!(header.box_type.is(b"mdat"));
        assert_eq!(header.header_size(), BOX_HEADER_MIN_SIZE);

        let (bx, _consumed) = parse_box(&bytes).unwrap();
        assert_eq!(bx.body.len(), 100);
    }

    // -- Variable-length items (container) ---------------------------------

    #[test]
    fn test_container_with_variable_items() {
        // Simulate a container (moov) with: ftyp(16), free(100, variable), mdat(32)
        // This tests that the size-driven walker can handle a larger variable-length
        // box NOT at the end.
        let mut data = Vec::new();
        // ftyp: 16
        data.extend_from_slice(&16u32.to_be_bytes());
        data.extend_from_slice(b"ftyp");
        data.resize(16, 0);
        // free: 100 bytes
        data.extend_from_slice(&100u32.to_be_bytes());
        data.extend_from_slice(b"free");
        data.resize(16 + 100, 0xAB);
        // mdat: 32
        data.extend_from_slice(&32u32.to_be_bytes());
        data.extend_from_slice(b"mdat");
        data.resize(16 + 100 + 32, 0xCD);

        let boxes: Vec<_> = box_iter(&data).collect::<Result<Vec<_>>>().unwrap();
        assert_eq!(boxes.len(), 3);
        assert!(boxes[0].0.header.box_type.is(b"ftyp"));
        assert_eq!(boxes[0].0.header.size, 16);
        assert!(boxes[1].0.header.box_type.is(b"free"));
        assert_eq!(boxes[1].0.header.size, 100);
        assert_eq!(boxes[1].0.body.len(), 92); // 100 - 8 header
        assert!(boxes[2].0.header.box_type.is(b"mdat"));
        assert_eq!(boxes[2].0.header.size, 32);
    }
}
