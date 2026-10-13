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

/// Byte range of the four-CC within a box's header (ISO/IEC 14496-12 §4.2).
const BOX_TYPE_RANGE: core::ops::Range<usize> = 4..8;

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
    /// Whether the wire encoding used the 64-bit `largesize` form
    /// (`size == 1` + an 8-byte length, ISO/IEC 14496-12:2015 §4.2).
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
    /// (ISO/IEC 14496-12:2015 §4.2): the compact 32-bit `size` when the whole
    /// box fits, else `size = 1` + 64-bit `largesize`, whose extra 8 header
    /// bytes count toward the size. The single place a box writer should
    /// derive its header length and `size` from.
    pub(crate) fn for_payload(
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
// thread, every box writer behind [`box_len`] / [`encode_box_header`] emits
// the 16-byte `size = 1` + 64-bit `largesize` header even for a small box.
// A real >4 GiB box is not allocatable in a test; this drives the identical
// code path with a small real payload. Compiled out of non-test builds.
#[cfg(test)]
std::thread_local! {
    static FORCE_LARGESIZE: core::cell::Cell<bool> = const { core::cell::Cell::new(false) };
}

/// RAII guard turning the largesize test seam on for this thread.
#[cfg(test)]
pub(crate) struct ForceLargesize;

#[cfg(test)]
impl ForceLargesize {
    pub(crate) fn on() -> Self {
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
/// `compact_len` (ISO/IEC 14496-12:2015 §4.2): `compact_len` itself while it
/// fits the 32-bit `size` field, else `compact_len` plus the 8-byte
/// `largesize` the `size = 1` form adds. Every variable-size box writer's
/// `serialized_len` ends with this, so it agrees with
/// [`write_box_header`]'s choice of form (#1147).
pub(crate) fn box_len(compact_len: usize) -> usize {
    if force_largesize() || compact_len as u64 > u32::MAX as u64 {
        compact_len.saturating_add(LARGESIZE_SIZE)
    } else {
        compact_len
    }
}

/// Encode a box header for a box of `total` bytes (header included) and
/// return its length: the compact 8-byte `size`+`type` when `total` fits
/// `u32`, else the 16-byte `size = 1` + `type` + 64-bit `largesize` form
/// (ISO/IEC 14496-12:2015 §4.2). `total` must come from [`box_len`] (it
/// counts the 8 `largesize` bytes). Pure: touches no payload, so tests can
/// exercise a multi-GiB `total` without allocating it.
pub(crate) fn encode_box_header(fourcc: &[u8; 4], total: u64) -> ([u8; 16], usize) {
    let mut out = [0u8; 16];
    out[4..8].copy_from_slice(fourcc);
    if force_largesize() || total > u32::MAX as u64 {
        out[0..4].copy_from_slice(&SIZE_INDICATES_LARGESIZE.to_be_bytes());
        out[8..16].copy_from_slice(&total.to_be_bytes());
        (out, BOX_HEADER_MIN_SIZE + LARGESIZE_SIZE)
    } else {
        // `total <= u32::MAX` was just checked.
        out[0..4].copy_from_slice(&(total as u32).to_be_bytes());
        (out, BOX_HEADER_MIN_SIZE)
    }
}

/// Write a box header for a box of `need` total bytes at the front of `buf`
/// via [`encode_box_header`], returning the header length (8, or 16 for the
/// `largesize` form). The single place a box writer's `size` is derived —
/// replaces the per-box `need as u32` that silently wrapped a >4 GiB box
/// (#1147).
pub(crate) fn write_box_header(buf: &mut [u8], fourcc: &[u8; 4], need: usize) -> Result<usize> {
    let (hdr, len) = encode_box_header(fourcc, need as u64);
    if buf.len() < len {
        return Err(Error::OutputBufferTooSmall {
            need: len,
            have: buf.len(),
        });
    }
    buf[..len].copy_from_slice(&hdr[..len]);
    Ok(len)
}

/// Write the compact 8-byte header of a box whose size is bounded by its own
/// fixed layout (`mvhd`, `tkhd`, `mfhd`, ...; no payload that can grow toward
/// 4 GiB), returning 8. A `need` that nevertheless exceeds the 32-bit field is
/// a typed error, never a wrap (#1147). These boxes' readers take fixed
/// offsets, so unlike [`write_box_header`] this never switches to `largesize`.
pub(crate) fn write_compact_box_header(
    buf: &mut [u8],
    fourcc: &[u8; 4],
    need: usize,
) -> Result<usize> {
    let size = broadcast_common::len::fit_u32(need, "box size")?;
    if buf.len() < BOX_HEADER_MIN_SIZE {
        return Err(Error::OutputBufferTooSmall {
            need: BOX_HEADER_MIN_SIZE,
            have: buf.len(),
        });
    }
    buf[0..4].copy_from_slice(&size.to_be_bytes());
    buf[4..8].copy_from_slice(fourcc);
    Ok(BOX_HEADER_MIN_SIZE)
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
pub(crate) fn compact_form(bytes: &[u8]) -> Result<alloc::borrow::Cow<'_, [u8]>> {
    let hdr = box_header_len(bytes)?;
    if hdr == BOX_HEADER_MIN_SIZE {
        return Ok(alloc::borrow::Cow::Borrowed(bytes));
    }
    let mut out = alloc::vec::Vec::with_capacity(bytes.len() - LARGESIZE_SIZE);
    let compact_len = u32::try_from(bytes.len() - LARGESIZE_SIZE).unwrap_or(u32::MAX);
    out.extend_from_slice(&compact_len.to_be_bytes());
    out.extend_from_slice(&bytes[BOX_TYPE_RANGE]);
    out.extend_from_slice(&bytes[hdr..]);
    Ok(alloc::borrow::Cow::Owned(out))
}

/// The total length of the child box at the front of `region`, header
/// included: its `size`, the 64-bit `largesize` when `size == 1`, or the rest
/// of `region` when `size == 0` (ISO/IEC 14496-12:2015 §4.2). `None` when the
/// header is truncated or declares a length shorter than itself — the
/// tolerant "stop walking here" signal the lenient child scanners use. Not
/// clamped to `region`.
pub(crate) fn child_extent(region: &[u8]) -> Option<usize> {
    let hdr = box_header_len(region).ok()?;
    let size = u32::from_be_bytes([region[0], region[1], region[2], region[3]]);
    let total = if size == SIZE_INDICATES_LARGESIZE {
        usize::try_from(u64::from_be_bytes(region[8..16].try_into().ok()?)).ok()?
    } else if size == SIZE_TO_EOF {
        region.len()
    } else {
        size as usize
    };
    (total >= hdr).then_some(total)
}

/// Length of the `size`+`type` (+ `largesize`) header at the front of the
/// whole box `bytes`: 8, or 16 when `size == 1` announces the 64-bit
/// `largesize` (ISO/IEC 14496-12:2015 §4.2). Typed box readers slice their
/// body at this offset instead of a hardcoded 8, so a box written in the
/// `largesize` form (see [`write_box_header`]) parses back (#1147). The usual
/// `uuid` `usertype` is left to the callers that handle `uuid` boxes.
pub(crate) fn box_header_len(bytes: &[u8]) -> Result<usize> {
    if bytes.len() < BOX_HEADER_MIN_SIZE {
        return Err(Error::BufferTooShort {
            need: BOX_HEADER_MIN_SIZE,
            have: bytes.len(),
            what: "box header",
        });
    }
    let size = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    if size == SIZE_INDICATES_LARGESIZE {
        let need = BOX_HEADER_MIN_SIZE + LARGESIZE_SIZE;
        if bytes.len() < need {
            return Err(Error::LargesizeBufferTooShort {
                need,
                have: bytes.len(),
            });
        }
        Ok(need)
    } else {
        Ok(BOX_HEADER_MIN_SIZE)
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

        // Write size in the form the header was parsed/constructed with, never
        // re-derived from `size`: a header that came off the wire as
        // `size == 1` + largesize has `header_size() == 16`, and writing the
        // compact 8-byte form here would leave the caller's 8 layout bytes
        // stale (r04-W8).
        let mut cursor = 0usize;
        if self.has_largesize {
            buf[0..4].copy_from_slice(&SIZE_INDICATES_LARGESIZE.to_be_bytes());
            cursor += 4;
            buf[cursor..cursor + 4].copy_from_slice(&self.box_type.to_u32().to_be_bytes());
            cursor += 4;
            buf[cursor..cursor + 8].copy_from_slice(&self.size.to_be_bytes());
            cursor += 8;
        } else {
            // A `size` of 1 without `has_largesize` would be written as the
            // largesize marker with no largesize following, so a header that
            // declares no largesize must not claim that size.
            if self.size == SIZE_INDICATES_LARGESIZE as u64 {
                return Err(Error::InvalidValue {
                    field: "BoxHeader.size",
                    value: self.size,
                    reason: "size 1 signals a 64-bit largesize, but has_largesize is false",
                });
            }
            // The compact form's 32-bit field cannot hold a larger size; a
            // header that claims one without setting `has_largesize` would
            // silently wrap (r04-W8).
            let size32 = u32::try_from(self.size).map_err(|_| Error::InvalidValue {
                field: "BoxHeader.size",
                value: self.size,
                reason: "size exceeds the 32-bit field and has_largesize is false",
            })?;
            buf[0..4].copy_from_slice(&size32.to_be_bytes());
            cursor += 4;
            buf[cursor..cursor + 4].copy_from_slice(&self.box_type.to_u32().to_be_bytes());
            cursor += 4;
        }

        // usertype for uuid. `header_size()` counts these 16 bytes, so a `uuid`
        // header without them would under-report the buffer the caller reserves
        // and write nothing into it.
        if self.box_type.is(b"uuid") {
            let Some(ut) = &self.usertype else {
                return Err(Error::InvalidValue {
                    field: "BoxHeader.usertype",
                    value: 0,
                    reason: "a uuid box header requires a 16-byte usertype",
                });
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

/// Iterate the top-level boxes of `data`, yielding each box's *full* bytes
/// (header + body). Best-effort: the first malformed or truncated box ends the
/// walk silently — for navigation helpers that hand the slices to a strict
/// typed parser. The single slice-yielding adapter over [`box_iter`]
/// (audit r05-O1 / #1141; `cenc_decrypt`, `media`, `progressive` each carried
/// their own walker).
pub(crate) fn box_slices(data: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut offset = 0usize;
    box_iter(data).map_while(move |step| {
        let (_, consumed) = step.ok()?;
        let start = offset;
        offset += consumed;
        Some(&data[start..start + consumed])
    })
}

/// Find a top-level box by four-CC, returning its full bytes (header + body).
pub(crate) fn find_top_box<'a>(data: &'a [u8], fourcc: &[u8; 4]) -> Option<&'a [u8]> {
    box_slices(data).find(|b| b[BOX_TYPE_RANGE] == *fourcc)
}

/// The body of the whole container box `container` (everything after its
/// `size`+`type` header, which is 16 bytes when the `largesize` form is used,
/// so a `largesize` container's children are found — a hardcoded 8-byte skip
/// would start mid-`largesize`). A truncated header yields an empty body: the
/// lenient navigation contract of [`box_slices`].
fn container_body(container: &[u8]) -> &[u8] {
    match box_header_len(container) {
        Ok(hdr) => &container[hdr..],
        Err(_) => &[],
    }
}

/// Iterate the children of the container box `container` (full bytes) whose
/// four-CC is `fourcc`, each as full box bytes. Best-effort like
/// [`box_slices`]. The shared child-navigation helper (issue #982:
/// `cenc_decrypt` carried private copies).
pub(crate) fn child_boxes<'a>(
    container: &'a [u8],
    fourcc: &'a [u8; 4],
) -> impl Iterator<Item = &'a [u8]> {
    box_slices(container_body(container)).filter(move |b| b[BOX_TYPE_RANGE] == *fourcc)
}

/// The first child of `container` with four-CC `fourcc` (full box bytes).
pub(crate) fn find_child_box<'a>(container: &'a [u8], fourcc: &[u8; 4]) -> Option<&'a [u8]> {
    box_slices(container_body(container)).find(|b| b[BOX_TYPE_RANGE] == *fourcc)
}

/// Descend a chain of container four-CCs from `start`, returning the innermost
/// box's full bytes (or `None` if any link is missing).
pub(crate) fn descend_boxes<'a>(start: &'a [u8], path: &[&[u8; 4]]) -> Option<&'a [u8]> {
    let mut cur = start;
    for fourcc in path {
        cur = find_child_box(cur, fourcc)?;
    }
    Some(cur)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_common::{Parse, Serialize};

    // -- BoxHeader tests ---------------------------------------------------

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
                Error::InvalidValue {
                    field: "BoxHeader.size",
                    ..
                }
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
                Error::InvalidValue {
                    field: "BoxHeader.usertype",
                    ..
                }
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
                Error::InvalidValue {
                    field: "BoxHeader.size",
                    ..
                }
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

/// #1147: box sizes past `u32::MAX` use the `size = 1` + 64-bit `largesize`
/// form instead of silently wrapping. A real >4 GiB box is not allocatable in
/// a test, so the threshold logic is driven with a synthetic `total`, and the
/// full writers + typed readers are driven under the [`ForceLargesize`] seam
/// with a small real payload from a committed fixture.
#[cfg(test)]
mod largesize_tests {
    use super::*;
    use crate::{MovieBox, MovieFragmentBox};
    use broadcast_common::{Parse, Serialize};

    const FOUR_GIB: u64 = 1 << 32;

    #[test]
    fn encode_header_threshold_is_u32_max() {
        let (hdr, len) = encode_box_header(b"stts", u64::from(u32::MAX));
        assert_eq!(len, 8, "u32::MAX still fits the compact form");
        assert_eq!(&hdr[..4], &u32::MAX.to_be_bytes());
        assert_eq!(&hdr[4..8], b"stts");

        let (hdr, len) = encode_box_header(b"stts", FOUR_GIB);
        assert_eq!(len, 16, "one past u32::MAX needs largesize");
        assert_eq!(&hdr[..4], &1u32.to_be_bytes(), "size = 1 marker");
        assert_eq!(&hdr[4..8], b"stts");
        assert_eq!(&hdr[8..16], &FOUR_GIB.to_be_bytes(), "largesize = total");

        // 5 GiB: the pre-fix `as u32` wrote 1 GiB (0x4000_0000) here.
        let five = 5 * (1u64 << 30);
        let (hdr, len) = encode_box_header(b"moov", five);
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
            Error::OutputBufferTooSmall { need: 16, have: 12 }
        ));
    }

    fn fixture(name: &str) -> Vec<u8> {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/transmux/");
        std::fs::read([path, name].concat()).expect("fixture must exist")
    }

    fn top_box<'a>(data: &'a [u8], fourcc: &[u8; 4]) -> &'a [u8] {
        find_top_box(data, fourcc).expect("box present")
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
        let data = fixture("h264_aac_prog.mp4");
        let orig = top_box(&data, b"moov");
        let moov = MovieBox::parse(orig).unwrap();

        let forced = {
            let _g = ForceLargesize::on();
            let len = moov.serialized_len();
            let bytes = moov.to_bytes();
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
            reparsed.to_bytes(),
            orig,
            "compact re-serialize is byte-identical"
        );
    }

    /// Same for a fragment's `moof` (mfhd/traf/trun).
    #[test]
    fn moof_round_trips_through_the_largesize_form() {
        let data = fixture("h264_aac_frag.mp4");
        let orig = top_box(&data, b"moof");
        let moof = MovieFragmentBox::parse_body(&orig[box_header_len(orig).unwrap()..]).unwrap();
        let forced = {
            let _g = ForceLargesize::on();
            moof.to_bytes()
        };
        assert_eq!(&forced[..4], &[0, 0, 0, 1]);
        for cc in [b"traf", b"trun"] {
            let (large, all) = count_large(&forced, cc);
            assert!(all > 0 && large == all, "{cc:?}: {large}/{all}");
        }
        let reparsed = MovieFragmentBox::parse_body(&forced[box_header_len(&forced).unwrap()..])
            .expect("largesize moof parses");
        assert_eq!(reparsed, moof);
        assert_eq!(reparsed.to_bytes(), orig);
    }

    fn read_path(rel: &str) -> Vec<u8> {
        let path = [env!("CARGO_MANIFEST_DIR"), "/", rel].concat();
        std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
    }

    /// Every committed fixture's `moov` (AVC/HEVC/AAC entries, CENC `sinf`/
    /// `schm`/`schi`/`tenc` wrappers, subtitle entries, v1 `mvhd`) survives the
    /// largesize form: tree-identical on re-parse and byte-identical when
    /// written compact again.
    #[test]
    fn every_fixture_moov_round_trips_through_the_largesize_form() {
        let mut checked = 0;
        for rel in [
            "../fixtures/transmux/h264_aac_prog.mp4",
            "../fixtures/transmux/h264_aac_frag.mp4",
            "../fixtures/transmux/h264_cenc.mp4",
            "../fixtures/transmux/h264_cbcs.mp4",
            "../fixtures/transmux/hevc_frag.mp4",
            "tests/fixtures/mp4/cmaf/av_frag.mp4",
            "tests/fixtures/mp4/cmaf/av_subtitle_frag.mp4",
            "tests/fixtures/mp4/cmaf/av_aac_subtitle_frag.mp4",
            "tests/fixtures/mp4/cenc_boxes/aac_cenc.mp4",
            "tests/fixtures/mp4/cenc_boxes/pcm_clear.mp4",
            "tests/fixtures/mp4/cenc_boxes/v1_mvhd.mp4",
        ] {
            let data = read_path(rel);
            let Some(orig) = find_top_box(&data, b"moov") else {
                continue;
            };
            checked += 1;
            let moov = MovieBox::parse(orig).unwrap_or_else(|e| panic!("{rel}: {e}"));
            let forced = {
                let _g = ForceLargesize::on();
                moov.to_bytes()
            };
            assert_eq!(&forced[..4], &[0, 0, 0, 1], "{rel}");
            let reparsed =
                MovieBox::parse(&forced).unwrap_or_else(|e| panic!("{rel}: largesize: {e}"));
            assert_eq!(reparsed, moov, "{rel}: tree changed through largesize");
            assert_eq!(reparsed.to_bytes(), orig, "{rel}: not byte-identical");
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
            "../fixtures/transmux/h264_aac_frag.mp4",
            "../fixtures/transmux/h264_cenc.mp4",
            "../fixtures/transmux/h264_cbcs.mp4",
            "../fixtures/transmux/hevc_frag.mp4",
            "tests/fixtures/mp4/cmaf/av_frag.mp4",
            "tests/fixtures/mp4/cmaf/av_subtitle_frag.mp4",
        ] {
            let data = read_path(rel);
            let Some(orig) = find_top_box(&data, b"moof") else {
                continue;
            };
            checked += 1;
            let hdr = box_header_len(orig).unwrap();
            let moof = MovieFragmentBox::parse_body(&orig[hdr..]).unwrap();
            let forced = {
                let _g = ForceLargesize::on();
                moof.to_bytes()
            };
            let body = &forced[box_header_len(&forced).unwrap()..];
            let reparsed = MovieFragmentBox::parse_body(body)
                .unwrap_or_else(|e| panic!("{rel}: largesize: {e}"));
            assert_eq!(reparsed, moof, "{rel}");
            assert_eq!(reparsed.to_bytes(), orig, "{rel}");
        }
        assert!(
            checked >= 4,
            "fixtures went missing: only {checked} checked"
        );
    }
}
