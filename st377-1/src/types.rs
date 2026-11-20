//! MXF simple/compound data types — SMPTE ST 377-1:2019 §4.2/§4.3
//! (`private/specs-verified/smpte/st377-1/st377-1.md`).

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use broadcast_common::len::{fit_u32, need_out};

use crate::error::{Error, Result};

/// A 16-byte SMPTE Universal Label (§4.2) — an object identification key,
/// distinct by type from the [`StrongRef`] and [`Auid`] values that share its
/// wire form.
///
/// The inner `[u8; 16]` is the wire octets in order. `From<[u8; 16]>`,
/// `From<UlBytes> for [u8; 16]` and [`UlBytes::as_bytes`] convert.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct UlBytes(pub [u8; 16]);

impl UlBytes {
    /// The all-zero value (a 32-zero-byte `PackageId` chain terminator's
    /// leading half, and the "unused" entry of an index table).
    pub const NULL: Self = Self([0u8; 16]);

    /// The wire octets, in order.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

impl Default for UlBytes {
    fn default() -> Self {
        Self::NULL
    }
}

impl From<[u8; 16]> for UlBytes {
    fn from(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }
}

impl From<UlBytes> for [u8; 16] {
    fn from(ul: UlBytes) -> Self {
        ul.0
    }
}

impl AsRef<[u8]> for UlBytes {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl core::fmt::Display for UlBytes {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for byte in &self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// A Strong Reference (§5.4.4): a 16-byte UUID referencing another Set in the
/// same file. Identical on the wire to [`UlBytes`], but a distinct type so a
/// reference target and a UL key cannot be swapped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct StrongRef(pub [u8; 16]);

impl StrongRef {
    /// The all-zero UUID (references no Set).
    pub const NULL: Self = Self([0u8; 16]);

    /// The wire octets, in order.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

impl From<[u8; 16]> for StrongRef {
    fn from(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }
}

impl From<StrongRef> for [u8; 16] {
    fn from(reference: StrongRef) -> Self {
        reference.0
    }
}

/// Reinterpret a 16-byte UUID as a Strong Reference (§5.4.4).
impl From<UlBytes> for StrongRef {
    fn from(ul: UlBytes) -> Self {
        Self(ul.0)
    }
}

impl AsRef<[u8]> for StrongRef {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl core::fmt::Display for StrongRef {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for byte in &self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// A Rational number (§4.3) — two big-endian `Int32` values, 8 bytes total.
/// Used for Edit Rate, Sample Rate, and similar time-base properties.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Rational {
    /// Numerator (signed 32-bit, big-endian on wire).
    pub numerator: i32,
    /// Denominator (signed 32-bit, big-endian on wire).
    pub denominator: i32,
}

/// Wire size of [`Rational`] — always 8 bytes.
pub const RATIONAL_LEN: usize = 8;

impl Rational {
    /// Parse 8 bytes as a Rational (two big-endian `Int32`).
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != RATIONAL_LEN {
            return Err(Error::InvalidPropertyLength {
                tag: 0,
                name: "Rational",
                found: bytes.len(),
                expected: RATIONAL_LEN,
            });
        }
        Ok(Rational {
            numerator: i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            denominator: i32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
        })
    }

    /// Serialize into an 8-byte buffer.
    pub fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        need_out(buf, RATIONAL_LEN)?;
        buf[0..4].copy_from_slice(&self.numerator.to_be_bytes());
        buf[4..8].copy_from_slice(&self.denominator.to_be_bytes());
        Ok(RATIONAL_LEN)
    }

    /// The 8-byte wire form, with no fallible length check.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; RATIONAL_LEN] {
        let mut buf = [0u8; RATIONAL_LEN];
        buf[0..4].copy_from_slice(&self.numerator.to_be_bytes());
        buf[4..8].copy_from_slice(&self.denominator.to_be_bytes());
        buf
    }
}

/// Copy the first 16 bytes of `bytes` into an owned [`UlBytes`].
///
/// Callers must have already verified `bytes.len() >= 16` (every call site
/// does, immediately before calling this) — rather than a length-checked
/// `try_into().expect(...)` (a panic-capable API call, even though
/// unreachable here), this reads byte-by-byte via plain indexing, the
/// established idiom throughout this workspace for a bounds-proven-safe
/// fixed-size extraction.
pub(crate) fn ul_bytes_from_prefix(bytes: &[u8]) -> UlBytes {
    let mut out = [0u8; 16];
    let mut i = 0;
    while i < 16 {
        out[i] = bytes[i];
        i += 1;
    }
    UlBytes(out)
}

/// A "Package ID" (§4.2): a 32-byte Basic UMID (SMPTE ST 330) or 32 zero
/// bytes ("terminate a reference chain"). This crate treats the UMID's own
/// internal bit layout as out of scope (ST 330 is a separate normative
/// reference) — see `private/specs-verified/smpte/st377-1/st377-1.md`'s Scope section — and exposes it only
/// as an opaque, fixed-size value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PackageId(#[cfg_attr(feature = "serde", serde(with = "serde_bytes32"))] pub [u8; 32]);

impl PackageId {
    /// The all-zero "reference chain terminator" value (§4.2).
    pub const NULL: PackageId = PackageId([0u8; 32]);

    /// True if this is the all-zero terminator value.
    #[must_use]
    pub fn is_null(&self) -> bool {
        self.0 == [0u8; 32]
    }
}

#[cfg(feature = "serde")]
mod serde_bytes32 {
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &[u8; 32], s: S) -> core::result::Result<S::Ok, S::Error> {
        s.serialize_bytes(v)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> core::result::Result<[u8; 32], D::Error> {
        let bytes = serde_bytes_vec::deserialize(d)?;
        <[u8; 32]>::try_from(bytes.as_slice())
            .map_err(|_| serde::de::Error::custom("PackageId must be 32 bytes"))
    }

    mod serde_bytes_vec {
        use alloc::vec::Vec;
        use serde::Deserialize;
        pub fn deserialize<'de, D: serde::Deserializer<'de>>(
            d: D,
        ) -> core::result::Result<Vec<u8>, D::Error> {
            Vec::<u8>::deserialize(d)
        }
    }
}

/// An AUID (§4.2.1): a 16-byte field holding either a UL or a UUID,
/// distinguished by the top bit of byte 0 (`0` = UL, stored value-order;
/// `1` = UUID, stored with its top/bottom 8 bytes swapped from natural UUID
/// order).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct Auid(pub UlBytes);

impl Auid {
    /// True if this AUID holds a UL (byte 0's top bit clear).
    #[must_use]
    pub fn is_ul(&self) -> bool {
        self.0.0[0] & 0x80 == 0
    }

    /// The UUID held in this AUID, in natural UUID storage order: the top and
    /// bottom 8 bytes are swapped back (§4.2.1). Only meaningful when
    /// [`is_ul`](Self::is_ul) is false.
    #[must_use]
    pub fn as_uuid_bytes(&self) -> UlBytes {
        swap_halves(self.0)
    }

    /// Build an AUID from a UUID in natural storage order, swapping its top and
    /// bottom 8 bytes as §4.2.1 requires; the inverse of
    /// [`as_uuid_bytes`](Self::as_uuid_bytes).
    #[must_use]
    pub fn from_uuid(uuid: UlBytes) -> Self {
        Auid(swap_halves(uuid))
    }
}

fn swap_halves(bytes: UlBytes) -> UlBytes {
    let mut out = [0u8; 16];
    out[..8].copy_from_slice(&bytes.0[8..]);
    out[8..].copy_from_slice(&bytes.0[..8]);
    UlBytes(out)
}

/// A Gregorian timestamp (§4.3): `year: Int16, month/day/hour/minute/second/
/// msec_div4: UInt8`, big-endian, 8 bytes total. All-zero means "unknown".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MxfTimestamp {
    /// Year (may be negative per the `Int16` wire type, though the
    /// Gregorian calendar this represents does not use negative years).
    pub year: i16,
    /// Month, 1-12 (0 in the all-zero "unknown" sentinel).
    pub month: u8,
    /// Day of month, 1-31 (0 in the all-zero "unknown" sentinel).
    pub day: u8,
    /// Hour, 0-23.
    pub hour: u8,
    /// Minute, 0-59.
    pub minute: u8,
    /// Second, 0-59.
    pub second: u8,
    /// Quarter-milliseconds (`msec / 4`), 0-249.
    pub msec_div4: u8,
}

/// Wire size of [`MxfTimestamp`] — always 8 bytes.
pub const TIMESTAMP_LEN: usize = 8;

impl MxfTimestamp {
    /// The all-zero "unknown" sentinel (§4.3 — "should not be used unless
    /// unavoidable").
    pub const UNKNOWN: MxfTimestamp = MxfTimestamp {
        year: 0,
        month: 0,
        day: 0,
        hour: 0,
        minute: 0,
        second: 0,
        msec_div4: 0,
    };

    /// Parse 8 bytes as a Timestamp.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != TIMESTAMP_LEN {
            return Err(Error::InvalidPropertyLength {
                tag: 0,
                name: "Timestamp",
                found: bytes.len(),
                expected: TIMESTAMP_LEN,
            });
        }
        Ok(MxfTimestamp {
            year: i16::from_be_bytes([bytes[0], bytes[1]]),
            month: bytes[2],
            day: bytes[3],
            hour: bytes[4],
            minute: bytes[5],
            second: bytes[6],
            msec_div4: bytes[7],
        })
    }

    /// Serialize into an 8-byte buffer.
    pub fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        need_out(buf, TIMESTAMP_LEN)?;
        let yb = self.year.to_be_bytes();
        buf[0] = yb[0];
        buf[1] = yb[1];
        buf[2] = self.month;
        buf[3] = self.day;
        buf[4] = self.hour;
        buf[5] = self.minute;
        buf[6] = self.second;
        buf[7] = self.msec_div4;
        Ok(TIMESTAMP_LEN)
    }

    /// The 8-byte wire form, with no fallible length check.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; TIMESTAMP_LEN] {
        let yb = self.year.to_be_bytes();
        [
            yb[0],
            yb[1],
            self.month,
            self.day,
            self.hour,
            self.minute,
            self.second,
            self.msec_div4,
        ]
    }
}

/// ProductVersion's `release` field enumeration (§4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum ReleaseType {
    /// `0` — Unknown version.
    Unknown,
    /// `1` — Released version.
    Released,
    /// `2` — Development version.
    Development,
    /// `3` — Released version with patches.
    ReleasedWithPatches,
    /// `4` — Pre-release beta version.
    PreReleaseBeta,
    /// `5` — Private version, not intended for general release.
    Private,
    /// Any other value — not defined by §4.3.
    Reserved(u16),
}

impl ReleaseType {
    /// The spec's own label (§4.3's enumeration list).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Unknown => "unknown version",
            Self::Released => "released version",
            Self::Development => "development version",
            Self::ReleasedWithPatches => "released version with patches",
            Self::PreReleaseBeta => "pre-release beta version",
            Self::Private => "private version",
            Self::Reserved(_) => "reserved",
        }
    }

    /// Decode from the wire `UInt16` value.
    #[must_use]
    pub fn from_u16(v: u16) -> Self {
        match v {
            0 => Self::Unknown,
            1 => Self::Released,
            2 => Self::Development,
            3 => Self::ReleasedWithPatches,
            4 => Self::PreReleaseBeta,
            5 => Self::Private,
            other => Self::Reserved(other),
        }
    }

    /// Encode to the wire `UInt16` value.
    #[must_use]
    pub fn to_u16(self) -> u16 {
        match self {
            Self::Unknown => 0,
            Self::Released => 1,
            Self::Development => 2,
            Self::ReleasedWithPatches => 3,
            Self::PreReleaseBeta => 4,
            Self::Private => 5,
            Self::Reserved(v) => v,
        }
    }
}

broadcast_common::impl_spec_display!(ReleaseType, Reserved);

/// Wire size of [`ProductVersion`] — always 10 bytes.
pub const PRODUCT_VERSION_LEN: usize = 10;

/// A tool/product version number (§4.3): 5 big-endian `UInt16` fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ProductVersion {
    /// Major version.
    pub major: u16,
    /// Minor version.
    pub minor: u16,
    /// Tertiary version.
    pub tertiary: u16,
    /// Patch version.
    pub patch: u16,
    /// Release kind (§4.3's 0-5 enumeration).
    pub release: ReleaseType,
}

impl ProductVersion {
    /// Parse 10 bytes as a ProductVersion.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != PRODUCT_VERSION_LEN {
            return Err(Error::InvalidPropertyLength {
                tag: 0,
                name: "ProductVersion",
                found: bytes.len(),
                expected: PRODUCT_VERSION_LEN,
            });
        }
        let u16_at = |i: usize| u16::from_be_bytes([bytes[i], bytes[i + 1]]);
        Ok(ProductVersion {
            major: u16_at(0),
            minor: u16_at(2),
            tertiary: u16_at(4),
            patch: u16_at(6),
            release: ReleaseType::from_u16(u16_at(8)),
        })
    }

    /// Serialize into a 10-byte buffer.
    pub fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        need_out(buf, PRODUCT_VERSION_LEN)?;
        let put =
            |buf: &mut [u8], i: usize, v: u16| buf[i..i + 2].copy_from_slice(&v.to_be_bytes());
        put(buf, 0, self.major);
        put(buf, 2, self.minor);
        put(buf, 4, self.tertiary);
        put(buf, 6, self.patch);
        put(buf, 8, self.release.to_u16());
        Ok(PRODUCT_VERSION_LEN)
    }

    /// The 10-byte wire form, with no fallible length check.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; PRODUCT_VERSION_LEN] {
        let mut buf = [0u8; PRODUCT_VERSION_LEN];
        let put =
            |buf: &mut [u8], i: usize, v: u16| buf[i..i + 2].copy_from_slice(&v.to_be_bytes());
        put(&mut buf, 0, self.major);
        put(&mut buf, 2, self.minor);
        put(&mut buf, 4, self.tertiary);
        put(&mut buf, 6, self.patch);
        put(&mut buf, 8, self.release.to_u16());
        buf
    }
}

/// Decode a big-endian UTF-16 string (§4.3 "String") into an owned `String`.
pub fn decode_utf16_be(bytes: &[u8]) -> Result<String> {
    if !bytes.len().is_multiple_of(2) {
        return Err(Error::InvalidUtf16 {
            tag: 0,
            name: "UTF-16 string",
        });
    }
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .collect();
    char::decode_utf16(units)
        .collect::<core::result::Result<String, _>>()
        .map_err(|_| Error::InvalidUtf16 {
            tag: 0,
            name: "UTF-16 string",
        })
}

/// Encode a string as big-endian UTF-16 (§4.3 "String").
#[must_use]
pub fn encode_utf16_be(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() * 2);
    for unit in s.encode_utf16() {
        out.extend_from_slice(&unit.to_be_bytes());
    }
    out
}

/// A Batch/Array of 16-byte ULs (§4.3) exactly as written on the wire.
///
/// Some encoders write a zero-length property (no header at all) for an
/// empty Batch, and some write an explicit header with `item_len` other
/// than 16; keeping the header bytes lets a parse → serialize → parse cycle
/// reproduce them instead of silently replacing them with this crate's
/// canonical 8-byte header ([`UidBatch::serialize`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct UidBatch {
    /// The elements (`count` of them, each 16 bytes).
    pub items: Vec<UlBytes>,
    header: Option<[u8; UID_BATCH_HEADER_LEN]>,
}

/// The 8-byte Batch/Array header (`count: u32`, `item_len: u32`).
const UID_BATCH_HEADER_LEN: usize = 8;

impl UidBatch {
    /// The elements.
    #[must_use]
    pub fn items(&self) -> &[UlBytes] {
        self.items.as_slice()
    }

    /// The 8-byte header exactly as parsed, or `None` when the property was
    /// zero-length (an absent header, i.e. an empty Batch with no elements).
    #[must_use]
    pub fn header(&self) -> Option<[u8; UID_BATCH_HEADER_LEN]> {
        self.header
    }

    /// The elements, retaining an 8-byte header exactly as given (`None`
    /// writes the canonical one).
    #[must_use]
    pub fn with_header(items: Vec<UlBytes>, header: Option<[u8; UID_BATCH_HEADER_LEN]>) -> Self {
        Self { items, header }
    }

    /// Parse a Batch/Array of 16-byte elements (§4.3): 8-byte header
    /// (`count: u32`, `item_len: u32`, both big-endian) followed by `count`
    /// 16-byte elements. Used for every UL/StrongRef Batch or Array in the
    /// Root Metadata Sets (`EssenceContainers`, `DMSchemes`, `Identifications`,
    /// `Packages`, `EssenceContainerData`).
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.is_empty() {
            // A zero-length property (no header at all) is treated as an empty
            // batch — some encoders omit the property entirely rather than
            // emit an empty 8-byte header; parse of an explicit empty header
            // is handled by the `count == 0` branch below.
            return Ok(Self::default());
        }
        if bytes.len() < UID_BATCH_HEADER_LEN {
            return Err(Error::InvalidBatchHeader {
                count: 0,
                item_len: 0,
                buffer_len: bytes.len(),
            });
        }
        let count = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let item_len = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let header = [
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ];
        let body = &bytes[8..];
        // `item_len` only describes the size of an actual element, so it is only
        // meaningful (and validated against 16) when `count > 0`. A real
        // encoder-written empty Batch (`count == 0`) has been observed with
        // `item_len == 0` rather than 16 (ffmpeg's OP1a Preface `DMSchemes`,
        // `../fixtures/mxf/smpte-st377/op1a-mpeg2-pcm.mxf`) — this crate previously rejected
        // that as `InvalidBatchHeader`, even though `count == 0` unambiguously
        // means "no elements" regardless of the stated element size.
        // #1108/MX-W1: `count as usize * 16` wraps on a 32-bit target for a
        // large enough `count`, which could make an attacker-chosen count
        // spuriously equal `body.len()` and then abort the process at
        // `Vec::with_capacity(count as usize)`. `checked_mul` rejects the
        // overflow instead, and the allocation below is sized from the
        // already-bounded `body.len()`, never from the untrusted `count`.
        let expected_len = (count as usize).checked_mul(16);
        if (count > 0 && item_len != 16) || expected_len != Some(body.len()) {
            return Err(Error::InvalidBatchHeader {
                count,
                item_len,
                buffer_len: body.len(),
            });
        }
        let mut out = Vec::with_capacity(body.len() / 16);
        for chunk in body.chunks_exact(16) {
            out.push(ul_bytes_from_prefix(chunk));
        }
        Ok(Self {
            items: out,
            header: Some(header),
        })
    }

    /// Serialize this Batch/Array (§4.3) back to its wire form: the header
    /// exactly as parsed when there is one, else the canonical 8-byte header
    /// with `item_len = 16` (the size of each *element type*, kept constant
    /// even when `count == 0`).
    ///
    /// # Errors
    /// [`Error::FieldOverflow`] if `items.len()` does not fit the 32-bit
    /// count; [`Error::InvalidBatchHeader`] if the retained header's `count`
    /// field disagrees with `items.len()`.
    pub fn serialize(&self) -> Result<Vec<u8>> {
        let count = fit_u32(self.items.len(), "batch count")?;
        let mut out = Vec::with_capacity(UID_BATCH_HEADER_LEN + self.items.len() * 16);
        match self.header {
            None => {
                out.extend_from_slice(&count.to_be_bytes());
                out.extend_from_slice(&16u32.to_be_bytes());
            }
            Some(header) => {
                let declared = u32::from_be_bytes([header[0], header[1], header[2], header[3]]);
                let item_len = u32::from_be_bytes([header[4], header[5], header[6], header[7]]);
                if declared != count {
                    return Err(Error::InvalidBatchHeader {
                        count: declared,
                        item_len,
                        buffer_len: self.items.len() * 16,
                    });
                }
                out.extend_from_slice(&header);
            }
        }
        for item in &self.items {
            out.extend_from_slice(item.as_ref());
        }
        Ok(out)
    }
}

/// Serialize a freshly built Batch/Array of 16-byte elements (§4.3) — the
/// canonical 8-byte header with `item_len = 16` (it names the size of each
/// *element type* in the batch, not the instance count, so it stays constant
/// even when `count == 0`). A Batch parsed from a real encoder keeps that
/// encoder's own header through [`UidBatch::serialize`] instead.
///
/// # Errors
/// [`Error::FieldOverflow`] if `items.len()` does not fit the 32-bit count.
pub fn serialize_uid_batch(items: &[UlBytes]) -> Result<Vec<u8>> {
    UidBatch {
        items: items.to_vec(),
        header: None,
    }
    .serialize()
}

/// [`serialize_uid_batch`] for callers whose infallible contract has already
/// bounded the count: a batch is written as an empty one (`count = 0`) if
/// `items.len()` does not fit the 32-bit count, which needs 4 294 967 296
/// 16-byte ULs (a 64 GiB batch) to reach.
pub(crate) fn serialize_uid_batch_owned(items: &[UlBytes]) -> Vec<u8> {
    serialize_uid_batch(items).unwrap_or_else(|_| serialize_uid_batch(&[]).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    /// A UL key, a Strong Reference and an AUID share one 16-byte wire form
    /// but are three different values: as bare `[u8; 16]` aliases they were
    /// interchangeable, so a reference target could be passed where a UL key
    /// belongs and still compile. This test only compiles while they are
    /// distinct types.
    #[test]
    fn ul_key_and_strong_reference_are_distinct_types() {
        fn key_length(key: &UlBytes) -> usize {
            key.as_bytes().len()
        }
        fn reference_length(reference: &StrongRef) -> usize {
            reference.as_bytes().len()
        }
        let ul = UlBytes([0x11; 16]);
        let reference = StrongRef([0x11; 16]);
        assert_eq!((key_length(&ul), reference_length(&reference)), (16, 16));
        assert_eq!(reference, StrongRef::from(ul));
        assert_eq!(<[u8; 16]>::from(reference), <[u8; 16]>::from(ul));
        assert_eq!(
            Auid(ul).as_uuid_bytes(),
            UlBytes([
                0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11,
                0x11, 0x11
            ])
        );
    }

    #[test]
    fn auid_uuid_swaps_the_top_and_bottom_eight_bytes() {
        let uuid = UlBytes([
            0x07, 0x72, 0x26, 0x2E, 0x76, 0x55, 0x43, 0x6F, 0x8F, 0xF3, 0x8A, 0xC5, 0x1B, 0x77,
            0x1E, 0x02,
        ]);
        let auid = Auid::from_uuid(uuid);
        assert_eq!(
            auid.0,
            UlBytes([
                0x8F, 0xF3, 0x8A, 0xC5, 0x1B, 0x77, 0x1E, 0x02, 0x07, 0x72, 0x26, 0x2E, 0x76, 0x55,
                0x43, 0x6F
            ])
        );
        assert!(!auid.is_ul(), "a UUID's stored top bit is always 1");
        assert_eq!(auid.as_uuid_bytes(), uuid);
        assert_eq!(Auid::from_uuid(auid.as_uuid_bytes()), auid);
    }

    #[test]
    fn timestamp_round_trip() {
        let ts = MxfTimestamp {
            year: 2019,
            month: 11,
            day: 28,
            hour: 12,
            minute: 34,
            second: 56,
            msec_div4: 10,
        };
        let mut buf = [0u8; TIMESTAMP_LEN];
        ts.serialize_into(&mut buf).unwrap();
        assert_eq!(MxfTimestamp::parse(&buf).unwrap(), ts);
    }

    #[test]
    fn product_version_round_trip() {
        let pv = ProductVersion {
            major: 1,
            minor: 2,
            tertiary: 3,
            patch: 4,
            release: ReleaseType::Released,
        };
        let mut buf = [0u8; PRODUCT_VERSION_LEN];
        pv.serialize_into(&mut buf).unwrap();
        assert_eq!(ProductVersion::parse(&buf).unwrap(), pv);
    }

    #[test]
    fn to_bytes_matches_the_serializer() {
        let ts = MxfTimestamp {
            year: -7,
            month: 12,
            day: 31,
            hour: 23,
            minute: 59,
            second: 58,
            msec_div4: 249,
        };
        let mut ts_buf = [0u8; TIMESTAMP_LEN];
        ts.serialize_into(&mut ts_buf).unwrap();
        assert_eq!(ts.to_bytes(), ts_buf);

        let pv = ProductVersion {
            major: 1,
            minor: 2,
            tertiary: 3,
            patch: 4,
            release: ReleaseType::Reserved(9),
        };
        let mut pv_buf = [0u8; PRODUCT_VERSION_LEN];
        pv.serialize_into(&mut pv_buf).unwrap();
        assert_eq!(pv.to_bytes(), pv_buf);

        let r = Rational {
            numerator: -30000,
            denominator: 1001,
        };
        let mut r_buf = [0u8; RATIONAL_LEN];
        r.serialize_into(&mut r_buf).unwrap();
        assert_eq!(r.to_bytes(), r_buf);
    }

    #[test]
    fn reserved_release_type_round_trips_value() {
        let pv = ProductVersion {
            major: 0,
            minor: 0,
            tertiary: 0,
            patch: 0,
            release: ReleaseType::from_u16(42),
        };
        assert_eq!(pv.release, ReleaseType::Reserved(42));
        assert_eq!(pv.release.to_u16(), 42);
        assert_eq!(pv.release.to_string(), "reserved(0x2A)");
    }

    #[test]
    fn utf16_round_trip() {
        let s = "MXF \u{1F3AC}"; // includes a surrogate-pair codepoint
        let bytes = encode_utf16_be(s);
        assert_eq!(decode_utf16_be(&bytes).unwrap(), s);
    }

    #[test]
    fn rational_round_trip() {
        let r = Rational {
            numerator: 25,
            denominator: 1,
        };
        let mut buf = [0u8; RATIONAL_LEN];
        r.serialize_into(&mut buf).unwrap();
        assert_eq!(Rational::parse(&buf).unwrap(), r);
    }

    #[test]
    fn rational_negative_values_round_trip() {
        let r = Rational {
            numerator: -30000,
            denominator: 1001,
        };
        let mut buf = [0u8; RATIONAL_LEN];
        r.serialize_into(&mut buf).unwrap();
        assert_eq!(Rational::parse(&buf).unwrap(), r);
    }

    #[test]
    fn uid_batch_round_trip() {
        let items = alloc::vec![UlBytes([1u8; 16]), UlBytes([2u8; 16]), UlBytes([3u8; 16])];
        let bytes = serialize_uid_batch(&items).unwrap();
        assert_eq!(UidBatch::parse(&bytes).unwrap().items(), items);
    }

    #[test]
    fn empty_uid_batch_round_trip() {
        let items: Vec<UlBytes> = Vec::new();
        let bytes = serialize_uid_batch(&items).unwrap();
        assert_eq!(bytes.len(), 8);
        assert_eq!(UidBatch::parse(&bytes).unwrap().items(), items);
    }

    #[test]
    fn a_short_uid_batch_has_an_invalid_header() {
        assert_eq!(
            UidBatch::parse(&[0u8; 4]),
            Err(Error::InvalidBatchHeader {
                count: 0,
                item_len: 0,
                buffer_len: 4
            })
        );
    }

    /// The 32-bit count is range-checked rather than narrowed with `as u32`
    /// (#2758): `u32::MAX + 1` items would otherwise be written with
    /// `count = 0`, i.e. as an empty batch.
    #[test]
    fn a_uid_batch_count_beyond_u32_is_rejected_not_wrapped() {
        let max = usize::try_from(u32::MAX).unwrap();
        assert_eq!(fit_u32(max, "batch count"), Ok(u32::MAX));
        assert!(matches!(
            fit_u32(max + 1, "batch count"),
            Err(broadcast_common::len::FieldOverflow {
                field: "batch count",
                ..
            })
        ));
    }

    #[test]
    fn a_uid_batch_with_a_wrong_item_length_is_rejected() {
        let mut bytes = alloc::vec![0u8; 8 + 16];
        bytes[3] = 1;
        bytes[7] = 8;
        assert_eq!(
            UidBatch::parse(&bytes),
            Err(Error::InvalidBatchHeader {
                count: 1,
                item_len: 8,
                buffer_len: 16
            })
        );
    }

    /// #2759: an empty Batch written as its 8-byte header is reserialized to
    /// those same bytes. Round-tripping only the elements would replace the
    /// encoder's `item_len` with 16 (`..fixtures/mxf/smpte-st377/
    /// op1a-mpeg2-pcm.mxf`'s `EssenceContainers` Batch has `item_len = 0`),
    /// so parse -> serialize was not byte-identical.
    #[test]
    fn an_empty_uid_batch_round_trips_its_absent_header_byte_identically() {
        let bytes = alloc::vec![0u8; 8];
        assert_eq!(UidBatch::parse(&bytes).unwrap().serialize().unwrap(), bytes);
    }

    /// #2759: an explicit header with `item_len == 0` (ffmpeg's style) is
    /// reproduced verbatim, not replaced by the canonical `item_len = 16`.
    #[test]
    fn an_uid_batch_keeps_the_encoders_item_len_byte_identically() {
        let mut bytes = alloc::vec![0u8; 8];
        bytes[4..8].copy_from_slice(&0u32.to_be_bytes());
        let batch = UidBatch::parse(&bytes).unwrap();
        assert!(batch.items().is_empty());
        assert_eq!(
            batch.header().map(|h| h[4..8].to_vec()),
            Some(alloc::vec![0, 0, 0, 0])
        );
        assert_eq!(batch.serialize().unwrap(), bytes);

        let mut one = alloc::vec![0u8; 8 + 16];
        one[3] = 1; // count = 1
        one[7] = 16; // item_len = 16
        assert_eq!(UidBatch::parse(&one).unwrap().serialize().unwrap(), one);
    }

    #[test]
    fn odd_length_and_lone_surrogate_utf16_are_invalid() {
        let invalid = Error::InvalidUtf16 {
            tag: 0,
            name: "UTF-16 string",
        };
        assert_eq!(decode_utf16_be(&[0x00]), Err(invalid));
        assert_eq!(decode_utf16_be(&[0xD8, 0x00]), Err(invalid));
    }

    #[test]
    fn ul_bytes_from_array_and_display_are_the_wire_form() {
        let bytes = [
            0x06, 0x0e, 0x2b, 0x34, 0x02, 0x05, 0x01, 0x01, 0x0d, 0x01, 0x02, 0x01, 0x01, 0x11,
            0x01, 0x00,
        ];
        let ul = UlBytes::from(bytes);
        assert_eq!(ul.as_ref(), &bytes);
        assert_eq!(ul.to_string(), "060e2b34020501010d01020101110100");
        assert_eq!(<[u8; 16]>::from(ul), bytes);
    }

    #[test]
    fn strong_ref_bytes_as_ref_and_display_are_the_wire_form() {
        let bytes = [
            0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54,
            0x32, 0x10,
        ];
        let reference = StrongRef(bytes);
        assert_eq!(reference.as_bytes(), &bytes);
        assert_eq!(reference.as_ref(), &bytes[..]);
        assert_eq!(reference.to_string(), "0123456789abcdeffedcba9876543210");
    }

    #[test]
    fn auid_is_ul_discriminates_on_byte_0_top_bit() {
        assert!(Auid(UlBytes([0x00; 16])).is_ul());
        assert!(Auid(UlBytes([0x06; 16])).is_ul());
        assert!(!Auid(UlBytes([0x80; 16])).is_ul());
        assert!(!Auid(UlBytes([0xFF; 16])).is_ul());
    }

    #[test]
    fn uid_batch_with_header_retains_both_parts() {
        let items = alloc::vec![UlBytes([0xAB; 16]), UlBytes([0xCD; 16])];
        let header = [0, 0, 0, 2, 0, 0, 0, 16];
        let batch = UidBatch::with_header(items.clone(), Some(header));
        assert_eq!(batch.items(), items.as_slice());
        assert_eq!(batch.header(), Some(header));
        assert_eq!(
            UidBatch::with_header(items.clone(), None).header(),
            None,
            "a freshly built Batch has no header"
        );

        // A retained header that disagrees with the element count cannot be
        // written; the error reports the body length (2 x 16 bytes).
        let mismatched = UidBatch::with_header(items, Some([0, 0, 0, 3, 0, 0, 0, 16]));
        match mismatched.serialize() {
            Err(Error::InvalidBatchHeader {
                count,
                item_len,
                buffer_len,
            }) => {
                assert_eq!(count, 3);
                assert_eq!(item_len, 16);
                assert_eq!(buffer_len, 32);
            }
            other => panic!("expected InvalidBatchHeader, got {other:?}"),
        }
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_bytes32_deserializes_every_byte() {
        let bytes: alloc::vec::Vec<u8> = (0u8..32).collect();
        let de = serde::de::value::SeqDeserializer::<_, serde::de::value::Error>::new(
            bytes.clone().into_iter(),
        );
        assert_eq!(
            serde_bytes32::deserialize(de).unwrap(),
            <[u8; 32]>::try_from(bytes.as_slice()).unwrap()
        );
    }
}
