//! ISOBMFF Box/FullBox layer — ISO/IEC 14496-12:2026 §4.2.
//!
//! Types for parsing and serialising ISO Base Media File Format (ISOBMFF) box headers.
//!
//! - [`BoxHeader`] — the 8-byte-or-larger header common to every box:
//!   `size`(32), `type`(32 four-CC), optional `largesize`(64 when size==1),
//!   optional `usertype`(128 when type==`uuid`), and `size==0` = to end of file.
//! - [`FullBoxHeader`] — extends [`BoxHeader`] with `version`(8) + `flags`(24).
//! - [`BoxItem`] — one box: header, offset, whole bytes and body (a slice),
//!   for generic size-driven walking.
//! - [`BoxIter`] — yield every box in an ISOBMFF buffer by advancing via
//!   `size`; unknown `type` values are skipped (ISO/IEC 14496-12:2026 §4.2).
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
//! See ISO/IEC 14496-12:2026 §4.2.

use crate::error::{Error, Result};
use broadcast_common::error::{BufferTooShort, InvalidValue};
use broadcast_common::len::{need_in, need_out};
use broadcast_common::{Parse, Serialize};
use core::fmt;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// The special size value `1` indicating a 64-bit `largesize` follows (ISO/IEC 14496-12:2026 §4.2).
pub const SIZE_INDICATES_LARGESIZE: u32 = 1;

/// The special size value `0` indicating the box runs to the end of the file (§4.2).
pub const SIZE_TO_EOF: u32 = 0;

/// Size of the fixed-size Box header fields: `size`(32) + `type`(32) = 8 bytes.
pub const BOX_HEADER_MIN_SIZE: usize = 8;

/// Byte range of the four-CC within a box's header (ISO/IEC 14496-12:2026 §4.2).
const BOX_TYPE_RANGE: core::ops::Range<usize> = BOX_TYPE_OFFSET..BOX_TYPE_OFFSET + BOX_TYPE_SIZE;

/// Size of the 64-bit `largesize` field (ISO/IEC 14496-12:2026 §4.2).
pub const LARGESIZE_SIZE: usize = 8;

/// Size of the 128-bit `usertype` field for `uuid` boxes (§4.2).
pub const UUID_TYPE_SIZE: usize = 16;

/// Size of the FullBox extension fields: `version`(8) + `flags`(24) = 4 bytes.
pub const FULLBOX_EXTRA_SIZE: usize = 4;

/// Size of a box type / four-CC field in bytes (§4.2).
pub const BOX_TYPE_SIZE: usize = 4;

/// Width of a CENC key ID (KID) in bytes (AES-128, ISO/IEC 23001-7); also the
/// width of the AES-128 content key itself.
pub const KEY_ID_LEN: usize = 16;

/// Size of the widest fixed header: `size` + `type` + 64-bit `largesize`.
pub const BOX_HEADER_LARGESIZE_SIZE: usize = BOX_HEADER_MIN_SIZE + LARGESIZE_SIZE;

/// Byte offset of the `size` field, which starts every box (ISO/IEC 14496-12:2026 §4.2).
pub const BOX_SIZE_OFFSET: usize = 0;

/// Byte offset of the four-CC `type` field inside a box header (§4.2).
pub const BOX_TYPE_OFFSET: usize = core::mem::size_of::<u32>();

/// Byte offset of the `largesize` field after the fixed header (§4.2).
const LARGESIZE_OFFSET: usize = BOX_HEADER_MIN_SIZE;

/// Byte offset of a FullBox's `version` byte, which follows the 8-byte box
/// header (§4.2).
pub const FULLBOX_VERSION_OFFSET: usize = BOX_HEADER_MIN_SIZE;

/// Width of the FullBox `flags` field (§4.2).
const FULLBOX_FLAGS_BITS: u32 = 24;

/// Mask of the FullBox `flags` field.
pub const FULLBOX_FLAGS_MASK: u32 = (1 << FULLBOX_FLAGS_BITS) - 1;

/// Byte offset of the first of the three `flags` bytes inside the FullBox
/// extension, i.e. just past its `version` byte (§4.2).
pub const FULLBOX_FLAGS_OFFSET: usize = 1;

/// Bytes the 24-bit `flags` field occupies.
const FULLBOX_FLAGS_LEN: usize = FULLBOX_EXTRA_SIZE - FULLBOX_FLAGS_OFFSET;

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
pub struct BoxType(pub [u8; BOX_TYPE_SIZE]);

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
    /// # use isobmff::box_types::BoxType;
    /// assert!(BoxType(*b"ftyp").is(b"ftyp"));
    /// assert!(!BoxType(*b"moov").is(b"mdat"));
    /// ```
    pub fn is(&self, literal: &[u8; 4]) -> bool {
        self.0 == *literal
    }

    /// Label for the box type, per the #204 convention.
    pub fn name(&self) -> &'static str {
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
            b"avcC" => "avcC",
            b"hvcC" => "hvcC",
            b"avc2" => "avc2",
            b"avc3" => "avc3",
            b"avc4" => "avc4",
            b"hev1" => "hev1",
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

/// Parsed header of an ISOBMFF Box (ISO/IEC 14496-12:2026 §4.2).
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
    /// Whether the wire encoding used the 64-bit `largesize` form
    /// (`size == 1` + an 8-byte length, ISO/IEC 14496-12:2026 §4.2).
    pub fn has_largesize(&self) -> bool {
        self.has_largesize
    }

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

    /// The header for a box carrying `payload_len` bytes after its header
    /// (ISO/IEC 14496-12:2026 §4.2): the compact 32-bit `size` when the whole
    /// box fits, else `size = 1` + 64-bit `largesize`, whose extra 8 header
    /// bytes count toward the size. The single place a box writer should
    /// derive its header length and `size` from.
    pub fn for_payload(
        box_type: BoxType,
        usertype: Option<[u8; UUID_TYPE_SIZE]>,
        payload_len: u64,
    ) -> Self {
        let uuid = if box_type.is(b"uuid") {
            UUID_TYPE_SIZE as u64
        } else {
            0
        };
        let compact = (BOX_HEADER_MIN_SIZE as u64 + uuid).saturating_add(payload_len);
        let size = if compact > u32::MAX as u64 {
            compact.saturating_add(LARGESIZE_SIZE as u64)
        } else {
            compact
        };
        Self::new(size, box_type, usertype)
    }
}

// Test seam (#1147): while a [`ForceLargesize`] guard is alive on this
// thread, every box writer behind [`box_len`] / [`write_box_header`] emits
// the 16-byte `size = 1` + 64-bit `largesize` header even for a small box.
// A real >4 GiB box is not allocatable in a test; this drives the identical
// code path with a small real payload. Compiled out of non-test builds.
#[cfg(test)]
std::thread_local! {
    static FORCE_LARGESIZE: core::cell::Cell<bool> = const { core::cell::Cell::new(false) };
}

/// RAII guard turning the largesize test seam on for this thread.
#[cfg(test)]
pub struct ForceLargesize;

#[cfg(test)]
impl ForceLargesize {
    pub fn on() -> Self {
        FORCE_LARGESIZE.with(|f| f.set(true));
        ForceLargesize
    }
}

#[cfg(test)]
impl Drop for ForceLargesize {
    fn drop(&mut self) {
        FORCE_LARGESIZE.with(|f| f.set(false));
    }
}

/// Whether the test seam currently forces the `largesize` form.
#[cfg(test)]
fn force_largesize() -> bool {
    FORCE_LARGESIZE.with(core::cell::Cell::get)
}

/// Whether the test seam currently forces the `largesize` form (never, in a
/// non-test build).
#[cfg(not(test))]
fn force_largesize() -> bool {
    false
}

/// Total on-wire length of a box whose compact-form (8-byte header) length is
/// `compact_len` (ISO/IEC 14496-12:2026 §4.2): `compact_len` itself while it
/// fits the 32-bit `size` field, else `compact_len` plus the 8-byte
/// `largesize` the `size = 1` form adds. Every variable-size box writer's
/// `serialized_len` ends with this, so it agrees with
/// [`write_box_header`]'s choice of form (#1147).
pub fn box_len(compact_len: usize) -> usize {
    if force_largesize() || compact_len as u64 > u32::MAX as u64 {
        compact_len.saturating_add(LARGESIZE_SIZE)
    } else {
        compact_len
    }
}

/// Write a box header for a box of `need` total bytes at the front of `buf`,
/// returning the header length (8, or 16 for the `largesize` form): the compact
/// 8-byte `size`+`type` when `need` fits `u32`, else the 16-byte `size = 1` +
/// `type` + 64-bit `largesize` form (ISO/IEC 14496-12:2026 §4.2). `need` must
/// come from [`box_len`] (it counts the 8 `largesize` bytes). The single place
/// a box writer's `size` is derived — replaces the per-box `need as u32` that
/// silently wrapped a >4 GiB box (#1147).
pub fn write_box_header(
    buf: &mut [u8],
    fourcc: &[u8; BOX_TYPE_SIZE],
    need: usize,
) -> Result<usize> {
    if force_largesize() || (need as u64) > u32::MAX as u64 {
        need_out(buf, BOX_HEADER_LARGESIZE_SIZE)?;
        buf[0..BOX_TYPE_SIZE].copy_from_slice(&SIZE_INDICATES_LARGESIZE.to_be_bytes());
        buf[BOX_TYPE_RANGE].copy_from_slice(fourcc);
        buf[LARGESIZE_OFFSET..BOX_HEADER_LARGESIZE_SIZE]
            .copy_from_slice(&(need as u64).to_be_bytes());
        Ok(BOX_HEADER_LARGESIZE_SIZE)
    } else {
        need_out(buf, BOX_HEADER_MIN_SIZE)?;
        let size = broadcast_common::len::fit_u32(need, "box size")?;
        buf[0..BOX_TYPE_SIZE].copy_from_slice(&size.to_be_bytes());
        buf[BOX_TYPE_RANGE].copy_from_slice(fourcc);
        Ok(BOX_HEADER_MIN_SIZE)
    }
}

/// Write a FullBox header: box header (size + type) followed by the 4-byte
/// FullBox extension (`version`(8) + `flags`(24)), returning the number of
/// bytes written. Replaces the per-box copy of this sequence (#2004).
pub fn write_full_box_header(
    buf: &mut [u8],
    fourcc: &[u8; BOX_TYPE_SIZE],
    need: usize,
    version: u8,
    flags: u32,
    compact: bool,
) -> Result<usize> {
    let mut c = if compact {
        let size = broadcast_common::len::fit_u32(need, "box size")?;
        need_out(buf, BOX_HEADER_MIN_SIZE)?;
        buf[0..BOX_TYPE_SIZE].copy_from_slice(&size.to_be_bytes());
        buf[BOX_TYPE_RANGE].copy_from_slice(fourcc);
        BOX_HEADER_MIN_SIZE
    } else {
        write_box_header(buf, fourcc, need)?
    };
    need_out(buf, c + FULLBOX_EXTRA_SIZE)?;
    buf[c] = version;
    c += 1;
    let fb = flags.to_be_bytes();
    buf[c..c + FULLBOX_FLAGS_LEN].copy_from_slice(&fb[FULLBOX_FLAGS_OFFSET..]);
    c += FULLBOX_FLAGS_LEN;
    Ok(c)
}

/// The whole box `bytes` in the compact 8-byte-header form.
///
/// Typed box readers index a box's fields at fixed offsets from the start of
/// its 8-byte header. A box written in the `size = 1` + `largesize` form (§4.2)
/// has a 16-byte header, which shifts every field; this returns the input
/// unchanged for the compact form and, for `largesize`, a copy with the extra
/// 8 bytes dropped (the `size` field rewritten to the compact length, saturating
/// at `u32::MAX` for a box too big to express compactly — no reader consults
/// it). Borrowed in the overwhelmingly common compact case (#1147).
pub fn compact_form(bytes: &[u8]) -> Result<alloc::borrow::Cow<'_, [u8]>> {
    need_in(bytes, BOX_HEADER_MIN_SIZE, "box header")?;
    if broadcast_common::wire::u32_at(bytes, BOX_SIZE_OFFSET) != SIZE_INDICATES_LARGESIZE {
        return Ok(alloc::borrow::Cow::Borrowed(bytes));
    }
    if bytes.len() < BOX_HEADER_LARGESIZE_SIZE {
        return Err(Error::LargesizeBufferTooShort {
            need: BOX_HEADER_LARGESIZE_SIZE,
            have: bytes.len(),
        });
    }
    let mut out = alloc::vec::Vec::with_capacity(bytes.len() - LARGESIZE_SIZE);
    let compact_len = u32::try_from(bytes.len() - LARGESIZE_SIZE).unwrap_or(u32::MAX);
    out.extend_from_slice(&compact_len.to_be_bytes());
    out.extend_from_slice(&bytes[BOX_TYPE_RANGE]);
    out.extend_from_slice(&bytes[BOX_HEADER_LARGESIZE_SIZE..]);
    Ok(alloc::borrow::Cow::Owned(out))
}

/// The body of the whole box `bytes` (everything after its 8- or 16-byte
/// header), checked to hold at least `min_body` bytes.
///
/// Every sample-entry / fixed-layout box reader derives its minimum length from
/// the box *body*, never from `8 + body`: a `largesize` box has a 16-byte
/// header, so a guard written against an 8-byte header lets a short
/// `largesize` box through to an out-of-bounds index (#1147 follow-up). The
/// error reports the whole-box length needed (`header + min_body`).
pub fn box_body<'a>(bytes: &'a [u8], min_body: usize, what: &'static str) -> Result<&'a [u8]> {
    let hdr = BoxHeader::parse(bytes)?.header_size();
    let body = &bytes[hdr..];
    if body.len() < min_body {
        return Err(Error::BufferTooShort(BufferTooShort {
            need: hdr + min_body,
            have: bytes.len(),
            what,
        }));
    }
    Ok(body)
}

impl<'a> Parse<'a> for BoxHeader {
    type Error = Error;

    /// Parse a `BoxHeader` from the front of `bytes`.
    ///
    /// Returns the header and the number of bytes consumed. The caller should
    /// verify `header.size` against the full buffer.
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(bytes, BOX_HEADER_MIN_SIZE, "BoxHeader")?;

        let raw_size = broadcast_common::wire::u32_at(bytes, BOX_SIZE_OFFSET);
        let box_type =
            BoxType::from_bytes(broadcast_common::wire::fourcc_at(bytes, BOX_TYPE_OFFSET));

        let mut cursor = BOX_HEADER_MIN_SIZE;

        let size: u64 = if raw_size == SIZE_INDICATES_LARGESIZE {
            if bytes.len() < cursor + LARGESIZE_SIZE {
                return Err(Error::LargesizeBufferTooShort {
                    need: cursor + LARGESIZE_SIZE,
                    have: bytes.len(),
                });
            }
            let v = broadcast_common::wire::u64_at(bytes, cursor);
            cursor += LARGESIZE_SIZE;
            v
        } else {
            raw_size as u64
        };

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
        need_out(buf, need)?;

        // Write size in the form the header was parsed/constructed with, never
        // re-derived from `size`: a header that came off the wire as
        // `size == 1` + largesize has `header_size() == 16`, and writing the
        // compact 8-byte form here would leave the caller's 8 layout bytes
        // stale (r04-W8).
        let mut cursor = 0usize;
        if self.has_largesize {
            buf[0..BOX_TYPE_SIZE].copy_from_slice(&SIZE_INDICATES_LARGESIZE.to_be_bytes());
            cursor += BOX_TYPE_SIZE;
            buf[cursor..cursor + BOX_TYPE_SIZE]
                .copy_from_slice(&self.box_type.to_u32().to_be_bytes());
            cursor += BOX_TYPE_SIZE;
            buf[cursor..cursor + LARGESIZE_SIZE].copy_from_slice(&self.size.to_be_bytes());
            cursor += LARGESIZE_SIZE;
        } else {
            // A `size` of 1 without `has_largesize` would be written as the
            // largesize marker with no largesize following, so a header that
            // declares no largesize must not claim that size.
            if self.size == SIZE_INDICATES_LARGESIZE as u64 {
                return Err(Error::InvalidValue(InvalidValue {
                    field: "BoxHeader.size",
                    value: self.size,
                    reason: "size 1 signals a 64-bit largesize, but has_largesize is false",
                }));
            }
            // The compact form's 32-bit field cannot hold a larger size; a
            // header that claims one without setting `has_largesize` would
            // silently wrap (r04-W8).
            let size32 = u32::try_from(self.size).map_err(|_| {
                Error::InvalidValue(InvalidValue {
                    field: "BoxHeader.size",
                    value: self.size,
                    reason: "size exceeds the 32-bit field and has_largesize is false",
                })
            })?;
            buf[0..BOX_TYPE_SIZE].copy_from_slice(&size32.to_be_bytes());
            cursor += BOX_TYPE_SIZE;
            buf[cursor..cursor + BOX_TYPE_SIZE]
                .copy_from_slice(&self.box_type.to_u32().to_be_bytes());
            cursor += BOX_TYPE_SIZE;
        }

        // usertype for uuid. `header_size()` counts these 16 bytes, so a `uuid`
        // header without them would under-report the buffer the caller reserves
        // and write nothing into it.
        if self.box_type.is(b"uuid") {
            let Some(ut) = &self.usertype else {
                return Err(Error::InvalidValue(InvalidValue {
                    field: "BoxHeader.usertype",
                    value: 0,
                    reason: "a uuid box header requires a 16-byte usertype",
                }));
            };
            buf[cursor..cursor + UUID_TYPE_SIZE].copy_from_slice(ut);
            cursor += UUID_TYPE_SIZE;
        }

        Ok(cursor)
    }
}

// ---------------------------------------------------------------------------
// FullBoxHeader
// ---------------------------------------------------------------------------

/// Extended header for a FullBox (ISO/IEC 14496-12:2026 §4.2): adds
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
        debug_assert!(flags <= FULLBOX_FLAGS_MASK, "flags must fit in 24 bits");
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

        need_in(bytes, hdr_sz + FULLBOX_EXTRA_SIZE, "FullBoxHeader")?;

        let version = bytes[hdr_sz];
        let mut flag_word = [0u8; BOX_TYPE_SIZE];
        flag_word[FULLBOX_FLAGS_OFFSET..]
            .copy_from_slice(&bytes[hdr_sz + FULLBOX_FLAGS_OFFSET..hdr_sz + FULLBOX_EXTRA_SIZE]);
        let flags = u32::from_be_bytes(flag_word);

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
        need_out(buf, need)?;

        let cursor = self.box_header.serialize_into(buf)?;
        buf[cursor] = self.version;
        let flag_bytes = self.flags.to_be_bytes();
        buf[cursor + FULLBOX_FLAGS_OFFSET..cursor + FULLBOX_EXTRA_SIZE]
            .copy_from_slice(&flag_bytes[FULLBOX_FLAGS_OFFSET..]);
        Ok(cursor + FULLBOX_EXTRA_SIZE)
    }
}

// ---------------------------------------------------------------------------
// BoxItem / BoxIter — the one box walker
// ---------------------------------------------------------------------------

/// One box located by [`BoxIter`]: its header, its offset in the walked
/// buffer, and its (possibly clamped) bytes.
#[derive(Debug, Clone, Copy)]
pub struct BoxItem<'a> {
    /// The parsed header.
    pub header: BoxHeader,
    /// Offset of the first header byte in the buffer that was walked.
    pub start: usize,
    /// Offset one past this box's last byte in that buffer.
    pub end: usize,
    /// `buffer[start..end]`, the whole box bytes.
    pub bytes: &'a [u8],
    /// The body: everything after the whole header (`usertype` included in
    /// the header).
    pub body: &'a [u8],
}

impl<'a> BoxItem<'a> {
    /// The box's four-CC (`type`).
    pub fn four_cc(&self) -> [u8; BOX_TYPE_SIZE] {
        self.header.box_type.0
    }

    /// Whether the wire used the `size == 0` to-end form
    /// (ISO/IEC 14496-12:2026 §4.2).
    pub fn to_end(&self) -> bool {
        self.header.size == 0
    }

    /// Whether the wire used the `size == 1` + 64-bit `largesize` form
    /// (ISO/IEC 14496-12:2026 §4.2).
    pub fn has_largesize(&self) -> bool {
        self.header.has_largesize()
    }

    /// Whether the declared size ran past the walked buffer, so `end` and
    /// `body` were clamped to it. Always `false` in strict mode.
    pub fn is_truncated(&self) -> bool {
        self.header.size != 0 && u64::try_from(self.bytes.len()).ok() != Some(self.header.size)
    }

    /// The payload after `size`/`type` (`usertype` of a `uuid` included,
    /// `largesize` bytes excluded): the bytes an opaque child keeps so a
    /// round trip re-emits them verbatim.
    pub fn payload(&self) -> &'a [u8] {
        let start = BOX_HEADER_MIN_SIZE
            + if self.header.has_largesize() {
                LARGESIZE_SIZE
            } else {
                0
            };
        self.bytes.get(start..).unwrap_or(&[])
    }

    /// Offset of the first body byte in the walked buffer.
    pub fn payload_start(&self) -> usize {
        self.start + self.header.header_size()
    }

    /// Iterate this container's children in wire order.
    pub fn children(&self) -> BoxIter<'a> {
        BoxIter::new(self.body)
    }
}

/// The walk's short-buffer policy: strict stops on a box that overruns its
/// buffer, clamped shortens it to the buffer end instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WalkMode {
    Strict,
    Clamped,
}

/// An iterator that yields every box in an ISOBMFF byte buffer in wire order.
///
/// The walk is **size-driven**: each step advances by the box's reported
/// `size` (or the remaining buffer for to-EOF boxes, ISO/IEC 14496-12:2026
/// §4.2). [`BoxIter::new`] is strict: a box that overruns the buffer yields
/// `Err`. [`BoxIter::clamped`] shortens such a box to the buffer end and ends
/// the walk at an unparseable header, a size below the header size or a zero
/// step, so a truncated capture still yields its intact leading boxes.
///
/// # Examples
///
/// ```
/// use isobmff::BoxIter;
///
/// # let bytes = &[];
/// // Walk all top-level boxes:
/// for result in BoxIter::new(bytes) {
///     let bx = result.unwrap();
///     println!("{} at offset {}", bx.header.box_type, bx.start);
/// }
/// ```
#[derive(Debug, Clone)]
pub struct BoxIter<'a> {
    remaining: &'a [u8],
    offset: usize,
    mode: WalkMode,
}

impl<'a> BoxIter<'a> {
    /// Create a strict iterator over `data`.
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            remaining: data,
            offset: 0,
            mode: WalkMode::Strict,
        }
    }

    /// Create an iterator that clamps a box overrunning `data` to the buffer
    /// end instead of reporting it.
    pub fn clamped(data: &'a [u8]) -> Self {
        Self {
            remaining: data,
            offset: 0,
            mode: WalkMode::Clamped,
        }
    }
}

impl<'a> Iterator for BoxIter<'a> {
    type Item = Result<BoxItem<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining.is_empty() {
            return None;
        }
        match self.mode {
            WalkMode::Strict => {
                if self.remaining.len() < BOX_HEADER_MIN_SIZE {
                    let err = Error::BufferTooShort(BufferTooShort {
                        need: self.offset + BOX_HEADER_MIN_SIZE,
                        have: self.offset + self.remaining.len(),
                        what: "container child header",
                    });
                    self.remaining = &[];
                    return Some(Err(err));
                }
                match parse_box_at(self.remaining, self.offset) {
                    Ok((item, consumed)) => {
                        self.remaining = self.remaining.get(consumed..).unwrap_or(&[]);
                        self.offset += consumed;
                        Some(Ok(item))
                    }
                    Err(err) => {
                        self.remaining = &[];
                        Some(Err(err))
                    }
                }
            }
            WalkMode::Clamped => {
                let (item, consumed) = clamped_item(self.remaining, self.offset)?;
                self.remaining = self.remaining.get(consumed..).unwrap_or(&[]);
                self.offset += consumed;
                Some(Ok(item))
            }
        }
    }
}

/// Locate the next clamped box of `data` at `offset`.
fn clamped_item(data: &[u8], offset: usize) -> Option<(BoxItem<'_>, usize)> {
    if data.len() < BOX_HEADER_MIN_SIZE {
        return None;
    }
    let header = BoxHeader::parse(data).ok()?;
    let hdr_sz = header.header_size();
    if hdr_sz > data.len() {
        return None;
    }
    let declared = if header.size == 0 {
        data.len()
    } else {
        match usize::try_from(header.size) {
            Ok(total) if total >= hdr_sz => total,
            Ok(_) => return None,
            Err(_) => data.len(),
        }
    };
    let step = declared.min(data.len());
    if step == 0 {
        return None;
    }
    let bytes = &data[..step];
    Some((
        BoxItem {
            header,
            start: offset,
            end: offset + step,
            bytes,
            body: bytes.get(hdr_sz..).unwrap_or(&[]),
        },
        step,
    ))
}

/// Decode one strict box from the front of `bytes` at `offset`.
fn parse_box_at(bytes: &[u8], offset: usize) -> Result<(BoxItem<'_>, usize)> {
    let header = BoxHeader::parse(bytes)?;
    let hdr_sz = header.header_size();
    let total = if header.size == 0 {
        bytes.len()
    } else {
        let declared = usize::try_from(header.size).map_err(|_| {
            Error::BufferTooShort(BufferTooShort {
                need: usize::MAX,
                have: bytes.len(),
                what: "BoxRef body",
            })
        })?;
        if declared < hdr_sz {
            return Err(Error::BoxSizeUnderflow {
                size: header.size,
                header_size: hdr_sz,
            });
        }
        need_in(bytes, declared, "BoxRef body")?;
        declared
    };
    let bytes = &bytes[..total];
    Ok((
        BoxItem {
            header,
            start: offset,
            end: offset + total,
            bytes,
            body: bytes.get(hdr_sz..).unwrap_or(&[]),
        },
        total,
    ))
}

/// Decode one box from the front of a byte buffer.
///
/// Returns the box and the number of bytes it occupies. Strict: a box that
/// overruns `bytes` is a typed short read. A `size == 0` box consumes the
/// rest of the buffer (ISO/IEC 14496-12:2026 §4.2).
pub fn parse_box(bytes: &[u8]) -> Result<(BoxItem<'_>, usize)> {
    parse_box_at(bytes, 0)
}

/// Find a box by four-CC at one level, returning its full bytes (header +
/// body). Call it on a file buffer for a top-level box, or on a container's
/// [`BoxItem::body`] for one of its children.
pub fn find_box<'a>(data: &'a [u8], fourcc: &[u8; BOX_TYPE_SIZE]) -> Option<&'a [u8]> {
    BoxIter::new(data)
        .filter_map(|step| step.ok())
        .find(|bx| bx.header.box_type.is(fourcc))
        .map(|bx| bx.bytes)
}

/// Descend a chain of container four-CCs from `start`, returning the innermost
/// box's full bytes (or `None` if any link is missing).
pub fn descend_boxes<'a>(start: &'a [u8], path: &[&[u8; BOX_TYPE_SIZE]]) -> Option<&'a [u8]> {
    let mut cur = start;
    for fourcc in path {
        cur = find_box(box_body(cur, 0, "container body").ok()?, fourcc)?;
    }
    Some(cur)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use broadcast_common::error::{BufferTooShort, InvalidValue};

    use super::*;
    use broadcast_common::{Parse, Serialize};

    // -- BoxHeader tests ---------------------------------------------------

    /// The #204 label table, one literal per arm of `BoxType::name`. Written
    /// out rather than derived from the arms it checks, so a deleted arm
    /// (which silently becomes `"<unknown>"`) or a copy-paste typo fails here.
    #[test]
    fn box_type_names_match_their_four_cc() {
        let table: &[(&[u8; 4], &str)] = &[
            (b"ftyp", "ftyp"),
            (b"moov", "moov"),
            (b"moof", "moof"),
            (b"trak", "trak"),
            (b"mdia", "mdia"),
            (b"minf", "minf"),
            (b"stbl", "stbl"),
            (b"dinf", "dinf"),
            (b"edts", "edts"),
            (b"mvex", "mvex"),
            (b"mvhd", "mvhd"),
            (b"tkhd", "tkhd"),
            (b"mdhd", "mdhd"),
            (b"hdlr", "hdlr"),
            (b"vmhd", "vmhd"),
            (b"smhd", "smhd"),
            (b"stsd", "stsd"),
            (b"stts", "stts"),
            (b"stsc", "stsc"),
            (b"stsz", "stsz"),
            (b"stco", "stco"),
            (b"co64", "co64"),
            (b"ctts", "ctts"),
            (b"stss", "stss"),
            (b"stsh", "stsh"),
            (b"elst", "elst"),
            (b"dref", "dref"),
            (b"tref", "tref"),
            (b"mdat", "mdat"),
            (b"free", "free"),
            (b"skip", "skip"),
            (b"uuid", "uuid"),
            (b"mfhd", "mfhd"),
            (b"traf", "traf"),
            (b"tfhd", "tfhd"),
            (b"trun", "trun"),
            (b"tfdt", "tfdt"),
            (b"sidx", "sidx"),
            (b"styp", "styp"),
            (b"mfra", "mfra"),
            (b"tfra", "tfra"),
            (b"mfro", "mfro"),
            (b"emsg", "emsg"),
            (b"avc1", "avc1"),
            (b"mp4a", "mp4a"),
            (b"enca", "enca"),
            (b"encv", "encv"),
            (b"hvc1", "hvc1"),
            (b"avcC", "avcC"),
            (b"hvcC", "hvcC"),
            (b"avc2", "avc2"),
            (b"avc3", "avc3"),
            (b"avc4", "avc4"),
            (b"hev1", "hev1"),
        ];
        assert_eq!(table.len(), 54, "every named arm is in the table");
        for (four_cc, label) in table {
            assert_eq!(
                BoxType(**four_cc).name(),
                *label,
                "BoxType::name for {:?}",
                core::str::from_utf8(*four_cc)
            );
        }
        assert_eq!(BoxType(*b"zzzz").name(), "<unknown>");
    }

    /// r04-W8: a wire header that used `size == 1` + largesize must serialize
    /// back to that same 16-byte form. Unfixed, the serializer re-derived the
    /// form from `size` (`<= u32::MAX` here), so it wrote the compact 8-byte
    /// header and returned 8 while `serialized_len()` said 16 — callers that
    /// lay out `header_size()` bytes were left with 8 stale bytes.
    #[test]
    fn largesize_form_round_trips_even_when_size_fits_u32() {
        // size == 1 (marker) + largesize == 16 (fits in u32 on purpose; 16 is
        // the header's own length, the minimum a valid size can be).
        let wire = [
            0, 0, 0, 1, // size = 1 → largesize follows
            b'f', b't', b'y', b'p', // type
            0, 0, 0, 0, 0, 0, 0, 16, // largesize = 16
        ];
        let header = BoxHeader::parse(&wire).unwrap();
        assert_eq!(header.size, 16);
        assert!(header.has_largesize);
        assert_eq!(header.header_size(), 16);
        assert_eq!(header.serialized_len(), 16);
        let mut out = [0u8; 16];
        assert_eq!(header.serialize_into(&mut out).unwrap(), 16);
        assert_eq!(out, wire, "largesize form must re-serialize identically");
        // And it parses back to the same header.
        assert_eq!(BoxHeader::parse(&out).unwrap(), header);
    }

    /// A compact header whose `size` does not fit the 32-bit field is rejected
    /// rather than silently wrapped (r04-W8).
    #[test]
    fn compact_size_beyond_u32_errors() {
        let mut header = BoxHeader::new(16, BoxType::from_bytes(*b"ftyp"), None);
        assert!(!header.has_largesize);
        header.size = u32::MAX as u64 + 1;
        let mut out = [0u8; BOX_HEADER_MIN_SIZE];
        let err = header.serialize_into(&mut out).unwrap_err();
        assert!(
            matches!(
                err,
                Error::InvalidValue(InvalidValue {
                    field: "BoxHeader.size",
                    ..
                })
            ),
            "expected InvalidValue for an oversized compact size, got {err:?}"
        );
        // At the boundary it still writes the compact form.
        header.size = u32::MAX as u64;
        assert_eq!(
            header.serialize_into(&mut out).unwrap(),
            BOX_HEADER_MIN_SIZE
        );
        assert_eq!(
            u32::from_be_bytes([out[0], out[1], out[2], out[3]]),
            u32::MAX
        );
    }

    /// The compact form is unaffected: a header built from a small size writes
    /// and round-trips the 8-byte header.
    #[test]
    fn compact_form_still_round_trips() {
        let wire = [0, 0, 0, 16, b'm', b'o', b'o', b'f'];
        let header = BoxHeader::parse(&wire).unwrap();
        assert!(!header.has_largesize);
        assert_eq!(header.header_size(), BOX_HEADER_MIN_SIZE);
        let mut out = [0u8; BOX_HEADER_MIN_SIZE];
        assert_eq!(
            header.serialize_into(&mut out).unwrap(),
            BOX_HEADER_MIN_SIZE
        );
        assert_eq!(out, wire);
    }

    /// §4.2 boundaries: an empty box (`size ==` its own header) parses, and a
    /// `size == 1` header needs exactly 16 bytes, reporting the shortfall.
    #[test]
    fn header_boundaries_pin_empty_and_largesize_boxes() {
        let empty = [0, 0, 0, 8, b'f', b'r', b'e', b'e'];
        let header = BoxHeader::parse(&empty).unwrap();
        assert_eq!(header.size, 8);
        assert_eq!(header.header_size(), 8);
        assert!(!header.has_largesize);

        let mut large = [0u8; 16];
        large[3] = 1;
        large[4..8].copy_from_slice(b"free");
        large[8..16].copy_from_slice(&0x110u64.to_be_bytes());
        let header = BoxHeader::parse(&large).unwrap();
        assert_eq!(header.size, 0x110);
        assert!(header.has_largesize);
        assert_eq!(header.header_size(), 16);

        // A largesize whose every byte differs: a 64-bit length is a promise
        // about the box, and a read from the wrong byte of it changes it.
        let mut huge = [0u8; 16];
        huge[3] = 1;
        huge[4..8].copy_from_slice(b"free");
        huge[8..16].copy_from_slice(&0x0102_0304_0506_0708u64.to_be_bytes());
        let header = BoxHeader::parse(&huge).unwrap();
        assert_eq!(header.size, 0x0102_0304_0506_0708);

        for have in 8..16usize {
            match BoxHeader::parse(&large[..have]) {
                Err(Error::LargesizeBufferTooShort {
                    need,
                    have: reported,
                }) => assert_eq!((need, reported), (16, have)),
                other => panic!(
                    "BoxHeader::parse on {have} bytes: expected LargesizeBufferTooShort, got {other:?}"
                ),
            }
        }
    }

    /// A `uuid` box's 16-byte usertype follows the (optional) 8-byte
    /// `largesize`, and the usertype guard needs exactly its 16 bytes.
    #[test]
    fn uuid_header_usertype_follows_the_largesize() {
        let usertype = [
            0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xAB, 0xAC, 0xAD,
            0xAE, 0xAF,
        ];

        let mut compact = alloc::vec![0u8; 24];
        compact[0..4].copy_from_slice(&24u32.to_be_bytes());
        compact[4..8].copy_from_slice(b"uuid");
        compact[8..24].copy_from_slice(&usertype);
        let header = BoxHeader::parse(&compact).unwrap();
        assert_eq!(header.size, 24);
        assert_eq!(header.usertype, Some(usertype));
        assert_eq!(header.header_size(), 24);

        match BoxHeader::parse(&compact[..23]) {
            Err(Error::UuidBufferTooShort {
                need,
                have: reported,
            }) => assert_eq!((need, reported), (24, 23)),
            other => panic!("expected UuidBufferTooShort, got {other:?}"),
        }

        let mut large = alloc::vec![0u8; 32];
        large[3] = 1;
        large[4..8].copy_from_slice(b"uuid");
        large[8..16].copy_from_slice(&32u64.to_be_bytes());
        large[16..32].copy_from_slice(&usertype);
        let header = BoxHeader::parse(&large).unwrap();
        assert_eq!(header.size, 32);
        assert!(header.has_largesize);
        assert_eq!(header.usertype, Some(usertype));
        assert_eq!(header.header_size(), 32);
    }

    /// `FullBoxHeader`, `box_body`, `for_payload` and `Display` lengths.
    #[test]
    fn full_box_header_lengths_display_and_clamped_guard() {
        let free = BoxType::from_bytes(*b"free");
        let compact = FullBoxHeader::new(BoxHeader::new(12, free, None), 1, 0x12_3456);
        assert_eq!(compact.serialized_len(), 12);
        let mut buf = [0u8; 12];
        assert_eq!(compact.serialize_into(&mut buf).unwrap(), 12);
        assert_eq!(buf[8], 1);
        assert_eq!(&buf[9..12], &[0x12, 0x34, 0x56]);

        match write_full_box_header(&mut [0u8; 11], b"free", 12, 0, 0, true) {
            Err(Error::OutputBufferTooSmall(_)) => {}
            other => panic!("expected OutputBufferTooSmall, got {other:?}"),
        }

        let large = BoxHeader {
            size: 20,
            box_type: free,
            usertype: None,
            has_largesize: true,
        };
        let full = FullBoxHeader::new(large, 0, 0);
        assert_eq!(full.serialized_len(), 20);
        let mut buf = [0u8; 20];
        assert_eq!(full.serialize_into(&mut buf).unwrap(), 20);
        match full.serialize_into(&mut [0u8; 19]) {
            Err(Error::OutputBufferTooSmall(_)) => {}
            other => panic!("expected OutputBufferTooSmall, got {other:?}"),
        }
        match FullBoxHeader::parse(&[0u8; 11]) {
            Err(Error::BufferTooShort(b)) => {
                assert_eq!((b.need, b.have, b.what), (12, 11, "FullBoxHeader"));
            }
            other => panic!("expected BufferTooShort, got {other:?}"),
        }

        match box_body(&[0, 0, 0, 20, b'f', b'r', b'e', b'e', 0, 0], 8, "body") {
            Err(Error::BufferTooShort(b)) => {
                assert_eq!((b.need, b.have, b.what), (16, 10, "body"));
            }
            other => panic!("expected BufferTooShort, got {other:?}"),
        }

        let uuid = BoxHeader::for_payload(
            BoxType::from_bytes(*b"uuid"),
            Some([0u8; UUID_TYPE_SIZE]),
            10,
        );
        assert_eq!(uuid.size, 34);
        assert_eq!(uuid.serialized_len(), 24);
        let plain = BoxHeader::for_payload(free, None, 10);
        assert_eq!(plain.size, 18);
        assert_eq!(plain.serialized_len(), 8);

        assert_eq!(alloc::format!("{}", BoxType::from_bytes(*b"url ")), "url ");
        assert_eq!(alloc::format!("{}", BoxType::from_bytes(*b"mp4v")), "mp4v");
        assert_eq!(
            alloc::format!("{}", BoxType::from_bytes([0x00, b'a', b'b', 0xFF])),
            "\\x00ab\\xff"
        );

        let mut uuid_box = alloc::vec![0u8; 24];
        uuid_box[0..4].copy_from_slice(&12u32.to_be_bytes());
        uuid_box[4..8].copy_from_slice(b"uuid");
        assert!(BoxIter::clamped(&uuid_box).next().is_none());
    }

    /// r04-W8: `header_size()` counts 16 bytes for a `uuid` header, so one
    /// without a `usertype` would under-report the buffer the caller reserves
    /// and write nothing into it. Serializing must fail instead.
    #[test]
    fn uuid_header_without_usertype_errors() {
        let header = BoxHeader::new(16 + 16, BoxType::from_bytes(*b"uuid"), None);
        assert_eq!(header.header_size(), BOX_HEADER_MIN_SIZE + UUID_TYPE_SIZE);
        let mut out = [0u8; BOX_HEADER_MIN_SIZE + UUID_TYPE_SIZE];
        let err = header.serialize_into(&mut out).unwrap_err();
        assert!(
            matches!(
                err,
                Error::InvalidValue(InvalidValue {
                    field: "BoxHeader.usertype",
                    ..
                })
            ),
            "expected InvalidValue for the missing usertype, got {err:?}"
        );
    }

    /// A `size` of 1 without `has_largesize` would write the largesize marker
    /// with no largesize after it, so it is rejected rather than emitted.
    #[test]
    fn size_one_without_largesize_errors() {
        let mut header = BoxHeader::new(
            SIZE_INDICATES_LARGESIZE as u64,
            BoxType::from_bytes(*b"ftyp"),
            None,
        );
        assert!(!header.has_largesize);
        let mut out = [0u8; BOX_HEADER_MIN_SIZE];
        let err = header.serialize_into(&mut out).unwrap_err();
        assert!(
            matches!(
                err,
                Error::InvalidValue(InvalidValue {
                    field: "BoxHeader.size",
                    ..
                })
            ),
            "{err:?}"
        );
        // The 16-byte form is fine for that size.
        header.size = 16;
        assert_eq!(
            header.serialize_into(&mut out).unwrap(),
            BOX_HEADER_MIN_SIZE
        );
    }

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
        assert!(matches!(
            err,
            Error::BufferTooShort(BufferTooShort { need: 8, .. })
        ));
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

    // -- parse_box tests ------------------------------------------------------

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
    fn box_walker_is_empty_for_an_empty_buffer() {
        let result: Vec<_> = BoxIter::new(&[]).collect();
        assert!(result.is_empty());
    }

    #[test]
    fn box_walker_yields_every_top_level_box() {
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

        let boxes: Vec<_> = BoxIter::new(&data).collect::<Result<Vec<_>>>().unwrap();
        assert_eq!(boxes.len(), 3);
        assert!(boxes[0].header.box_type.is(b"ftyp"));
        assert!(boxes[1].header.box_type.is(b"moov"));
        assert!(boxes[2].header.box_type.is(b"mdat"));
    }

    // -- Real-fixture round-trip test --------------------------------------

    #[test]
    fn test_round_trip_known_boxes() {
        let path = test_fixtures::path("mp4/h264-aac-frag.mp4");
        let data = std::fs::read(path).expect("fixture file must exist");
        let original_len = data.len();

        // Re-serialize every top-level box into a flat buffer.
        let mut output = Vec::new();
        for result in BoxIter::new(&data) {
            let bx = result.unwrap();
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

        let boxes: Vec<_> = BoxIter::new(&data).collect::<Result<Vec<_>>>().unwrap();
        assert_eq!(
            boxes.len(),
            3,
            "should parse all three boxes including unknown"
        );
        assert!(boxes[0].header.box_type.is(b"ftyp"));
        assert!(
            boxes[1].header.box_type.is(b"XXXX"),
            "unknown box should parse by size"
        );
        assert!(boxes[2].header.box_type.is(b"moov"));
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

        let boxes: Vec<_> = BoxIter::new(&data).collect::<Result<Vec<_>>>().unwrap();
        assert_eq!(boxes.len(), 3);
        assert!(boxes[0].header.box_type.is(b"ftyp"));
        assert_eq!(boxes[0].header.size, 16);
        assert!(boxes[1].header.box_type.is(b"free"));
        assert_eq!(boxes[1].header.size, 100);
        assert_eq!(boxes[1].body.len(), 92); // 100 - 8 header
        assert!(boxes[2].header.box_type.is(b"mdat"));
        assert_eq!(boxes[2].header.size, 32);
    }
}

/// #1147: box sizes past `u32::MAX` use the `size = 1` + 64-bit `largesize`
/// form instead of silently wrapping. A real >4 GiB box is not allocatable in
/// a test, so the threshold logic is driven with a synthetic `total`, and the
/// full writers + typed readers are driven under the [`ForceLargesize`] seam
/// with a small real payload from a committed fixture.
#[cfg(test)]
mod largesize_tests {
    use broadcast_common::error::OutputBufferTooSmall;

    use super::*;
    use crate::init_segment::MovieBox;
    use crate::movie_fragment::MovieFragmentBox;
    use broadcast_common::{Parse, Serialize};

    const FOUR_GIB: u64 = 1 << 32;

    #[test]
    fn header_threshold_is_u32_max() {
        let mut hdr = [0u8; BOX_HEADER_LARGESIZE_SIZE];
        let len = write_box_header(&mut hdr, b"stts", u32::MAX as usize).unwrap();
        assert_eq!(len, 8, "u32::MAX still fits the compact form");
        assert_eq!(&hdr[..4], &u32::MAX.to_be_bytes());
        assert_eq!(&hdr[4..8], b"stts");

        let len = write_box_header(&mut hdr, b"stts", FOUR_GIB as usize).unwrap();
        assert_eq!(len, 16, "one past u32::MAX needs largesize");
        assert_eq!(&hdr[..4], &1u32.to_be_bytes(), "size = 1 marker");
        assert_eq!(&hdr[4..8], b"stts");
        assert_eq!(&hdr[8..16], &FOUR_GIB.to_be_bytes(), "largesize = total");

        // 5 GiB: the pre-fix `as u32` wrote 1 GiB (0x4000_0000) here.
        let five = 5 * (1u64 << 30);
        let len = write_box_header(&mut hdr, b"moov", five as usize).unwrap();
        assert_eq!(len, 16);
        assert_eq!(&hdr[8..16], &five.to_be_bytes());
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn box_len_adds_the_largesize_field_only_past_u32_max() {
        let max = u32::MAX as usize;
        assert_eq!(box_len(max), max, "fits compact: unchanged");
        assert_eq!(
            box_len(max + 1),
            max + 1 + LARGESIZE_SIZE,
            "the 8 largesize bytes count toward the box"
        );
        // Agrees with the header the progressive mdat path derives.
        for payload in [
            0u64,
            1000,
            u64::from(u32::MAX) - 8,
            u64::from(u32::MAX) - 7,
            6 << 30,
        ] {
            let h = BoxHeader::for_payload(BoxType::from_bytes(*b"mdat"), None, payload);
            assert_eq!(
                h.size,
                box_len(BOX_HEADER_MIN_SIZE + payload as usize) as u64
            );
        }
    }

    /// The header-only writer takes a synthetic multi-GiB `need` and touches no
    /// payload, so the >4 GiB form is checked without allocating 4 GiB.
    #[cfg(target_pointer_width = "64")]
    #[test]
    fn write_box_header_emits_largesize_for_a_synthetic_5_gib_box() {
        let need = 5usize << 30;
        let mut buf = [0u8; 16];
        let n = write_box_header(&mut buf, b"mdat", need).unwrap();
        assert_eq!(n, 16);
        assert_eq!(&buf[..4], &[0, 0, 0, 1]);
        assert_eq!(&buf[4..8], b"mdat");
        assert_eq!(
            u64::from_be_bytes(buf[8..16].try_into().unwrap()),
            need as u64
        );
        // The writer's output parses back to that size.
        let parsed = BoxHeader::parse(&buf).unwrap();
        assert_eq!(parsed.size, need as u64);
        assert!(parsed.has_largesize());
        assert_eq!(parsed.header_size(), n);
    }

    #[test]
    fn write_box_header_rejects_a_too_small_buffer() {
        let mut buf = [0u8; 12];
        let err = write_box_header(&mut buf, b"stts", 1usize << 32).unwrap_err();
        assert!(matches!(
            err,
            Error::OutputBufferTooSmall(OutputBufferTooSmall { need: 16, have: 12 })
        ));
    }

    fn fixture(name: &str) -> Vec<u8> {
        test_fixtures::read(&format!("mp4/{name}"))
    }

    fn top_box<'a>(data: &'a [u8], fourcc: &[u8; 4]) -> &'a [u8] {
        find_box(data, fourcc).expect("box present")
    }

    /// Count how many boxes named `fourcc` appear, and how many of those have
    /// the largesize marker right before the type (so `size == 1`).
    fn count_large(bytes: &[u8], fourcc: &[u8; 4]) -> (usize, usize) {
        let (mut large, mut all) = (0, 0);
        for i in 4..bytes.len().saturating_sub(4) {
            if &bytes[i..i + 4] == fourcc {
                all += 1;
                if bytes[i - 4..i] == [0, 0, 0, 1] {
                    large += 1;
                }
            }
        }
        (large, all)
    }

    /// A real `moov` (every variable-size sample-table box + every container)
    /// serialized in the forced largesize form re-parses to an identical tree,
    /// and re-serializes in the compact form to the original bytes.
    #[test]
    fn moov_round_trips_through_the_largesize_form() {
        let data = fixture("h264-aac-prog.mp4");
        let orig = top_box(&data, b"moov");
        let moov = MovieBox::parse(orig).unwrap();

        let forced = {
            let _g = ForceLargesize::on();
            let len = moov.serialized_len();
            let bytes = moov.try_to_bytes().unwrap();
            assert_eq!(bytes.len(), len, "serialized_len agrees with the writer");
            bytes
        };
        assert!(forced.len() > orig.len());
        assert_eq!(&forced[..4], &[0, 0, 0, 1], "moov carries the marker");
        assert_eq!(&forced[4..8], b"moov");
        assert_eq!(
            u64::from_be_bytes(forced[8..16].try_into().unwrap()),
            forced.len() as u64,
            "largesize is the whole box"
        );
        for cc in [
            b"trak", b"mdia", b"minf", b"stbl", b"stsd", b"stts", b"stsc", b"stsz", b"stco",
            b"dinf", b"dref", b"url ",
        ] {
            let (large, all) = count_large(&forced, cc);
            assert!(
                all > 0 && large == all,
                "{cc:?}: {large}/{all} use largesize"
            );
        }

        // `hdlr` also occurs inside a verbatim opaque child (kept compact), so
        // only require that the typed ones switched.
        let (hdlr_large, _) = count_large(&forced, b"hdlr");
        assert!(hdlr_large >= 2, "typed hdlr boxes use largesize");

        let reparsed = MovieBox::parse(&forced).expect("largesize moov parses");
        assert_eq!(reparsed, moov, "typed tree identical after largesize trip");
        assert_eq!(
            reparsed.try_to_bytes().unwrap(),
            orig,
            "compact re-serialize is byte-identical"
        );
    }

    /// Same for a fragment's `moof` (mfhd/traf/trun).
    #[test]
    fn moof_round_trips_through_the_largesize_form() {
        let data = fixture("h264-aac-frag.mp4");
        let orig = top_box(&data, b"moof");
        let moof = MovieFragmentBox::parse_body(parse_box(orig).unwrap().0.body).unwrap();
        let forced = {
            let _g = ForceLargesize::on();
            moof.try_to_bytes().unwrap()
        };
        assert_eq!(&forced[..4], &[0, 0, 0, 1]);
        for cc in [b"traf", b"trun"] {
            let (large, all) = count_large(&forced, cc);
            assert!(all > 0 && large == all, "{cc:?}: {large}/{all}");
        }
        let reparsed = MovieFragmentBox::parse_body(parse_box(&forced).unwrap().0.body)
            .expect("largesize moof parses");
        assert_eq!(reparsed, moof);
        assert_eq!(reparsed.try_to_bytes().unwrap(), orig);
    }

    fn read_path(rel: &str) -> Vec<u8> {
        let rel = rel.trim_start_matches("../fixtures/");
        let path = test_fixtures::path(rel);
        std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    /// Every committed fixture's `moov` (AVC/HEVC/AAC entries, CENC `sinf`/
    /// `schm`/`schi`/`tenc` wrappers, subtitle entries, v1 `mvhd`) survives the
    /// largesize form: tree-identical on re-parse and byte-identical when
    /// written compact again.
    #[test]
    fn every_fixture_moov_round_trips_through_the_largesize_form() {
        let mut checked = 0;
        for rel in [
            "../fixtures/mp4/h264-aac-prog.mp4",
            "../fixtures/mp4/h264-aac-frag.mp4",
            "../fixtures/mp4/h264-cenc.mp4",
            "../fixtures/mp4/h264-cbcs.mp4",
            "../fixtures/mp4/hevc-frag.mp4",
            "../fixtures/mp4/cmaf-av-subtitle/av-frag.mp4",
            "../fixtures/mp4/cmaf-av-subtitle/av-subtitle-frag.mp4",
            "../fixtures/mp4/cmaf-av-subtitle/av-aac-subtitle-frag.mp4",
            "../fixtures/mp4/cenc-boxes/aac-cenc.mp4",
            "../fixtures/mp4/cenc-boxes/pcm-clear.mp4",
            "../fixtures/mp4/cenc-boxes/v1-mvhd.mp4",
        ] {
            let data = read_path(rel);
            let Some(orig) = find_box(&data, b"moov") else {
                continue;
            };
            checked += 1;
            let moov = MovieBox::parse(orig).unwrap_or_else(|e| panic!("{rel}: {e}"));
            let forced = {
                let _g = ForceLargesize::on();
                moov.try_to_bytes().unwrap()
            };
            assert_eq!(&forced[..4], &[0, 0, 0, 1], "{rel}");
            let reparsed =
                MovieBox::parse(&forced).unwrap_or_else(|e| panic!("{rel}: largesize: {e}"));
            assert_eq!(reparsed, moov, "{rel}: tree changed through largesize");
            assert_eq!(
                reparsed.try_to_bytes().unwrap(),
                orig,
                "{rel}: not byte-identical"
            );
        }
        assert!(
            checked >= 8,
            "fixtures went missing: only {checked} checked"
        );
    }

    /// Every fixture fragment's `moof` too (traf with opaque `senc`/`saiz`/`saio`
    /// and typed `tfhd`/`tfdt`/`trun`).
    #[test]
    fn every_fixture_moof_round_trips_through_the_largesize_form() {
        let mut checked = 0;
        for rel in [
            "../fixtures/mp4/h264-aac-frag.mp4",
            "../fixtures/mp4/h264-cenc.mp4",
            "../fixtures/mp4/h264-cbcs.mp4",
            "../fixtures/mp4/hevc-frag.mp4",
            "../fixtures/mp4/cmaf-av-subtitle/av-frag.mp4",
            "../fixtures/mp4/cmaf-av-subtitle/av-subtitle-frag.mp4",
        ] {
            let data = read_path(rel);
            let Some(orig) = find_box(&data, b"moof") else {
                continue;
            };
            checked += 1;
            let moof = MovieFragmentBox::parse_body(parse_box(orig).unwrap().0.body).unwrap();
            let forced = {
                let _g = ForceLargesize::on();
                moof.try_to_bytes().unwrap()
            };
            let body = parse_box(&forced).unwrap().0.body;
            let reparsed = MovieFragmentBox::parse_body(body)
                .unwrap_or_else(|e| panic!("{rel}: largesize: {e}"));
            assert_eq!(reparsed, moof, "{rel}");
            assert_eq!(reparsed.try_to_bytes().unwrap(), orig, "{rel}");
        }
        assert!(
            checked >= 4,
            "fixtures went missing: only {checked} checked"
        );
    }
}
