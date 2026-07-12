//! MXF simple/compound data types — SMPTE ST 377-1:2019 §4.2/§4.3
//! (`docs/st377-1.md`).

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use crate::error::{Error, Result};

/// A 16-byte SMPTE Universal Label or UUID.
pub type UlBytes = [u8; 16];

/// A "Package ID" (§4.2): a 32-byte Basic UMID (SMPTE ST 330) or 32 zero
/// bytes ("terminate a reference chain"). This crate treats the UMID's own
/// internal bit layout as out of scope (ST 330 is a separate normative
/// reference) — see `docs/st377-1.md`'s Scope section — and exposes it only
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
pub struct Auid(pub UlBytes);

impl Auid {
    /// True if this AUID holds a UL (byte 0's top bit clear).
    #[must_use]
    pub fn is_ul(&self) -> bool {
        self.0[0] & 0x80 == 0
    }

    /// This AUID's bytes, interpreted as a UL (no transform — only
    /// meaningful when [`Self::is_ul`]).
    #[must_use]
    pub fn as_ul_bytes(&self) -> UlBytes {
        self.0
    }

    /// This AUID's bytes, interpreted as a UUID: swaps the top/bottom 8
    /// bytes back to natural UUID storage order (§4.2.1, Table 2) — only
    /// meaningful when [`Self::is_ul`] is false.
    #[must_use]
    pub fn as_uuid_bytes(&self) -> UlBytes {
        let mut out = [0u8; 16];
        out[..8].copy_from_slice(&self.0[8..]);
        out[8..].copy_from_slice(&self.0[..8]);
        out
    }

    /// Build an AUID from a UL (stored as-is).
    #[must_use]
    pub fn from_ul(ul: UlBytes) -> Self {
        Auid(ul)
    }

    /// Build an AUID from a natural-order UUID (top/bottom 8 bytes swapped
    /// on storage per §4.2.1).
    #[must_use]
    pub fn from_uuid(uuid: UlBytes) -> Self {
        let mut out = [0u8; 16];
        out[..8].copy_from_slice(&uuid[8..]);
        out[8..].copy_from_slice(&uuid[..8]);
        Auid(out)
    }
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
        if buf.len() < TIMESTAMP_LEN {
            return Err(Error::BufferTooShort {
                need: TIMESTAMP_LEN,
                have: buf.len(),
                what: "Timestamp",
            });
        }
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
        if buf.len() < PRODUCT_VERSION_LEN {
            return Err(Error::BufferTooShort {
                need: PRODUCT_VERSION_LEN,
                have: buf.len(),
                what: "ProductVersion",
            });
        }
        let put =
            |buf: &mut [u8], i: usize, v: u16| buf[i..i + 2].copy_from_slice(&v.to_be_bytes());
        put(buf, 0, self.major);
        put(buf, 2, self.minor);
        put(buf, 4, self.tertiary);
        put(buf, 6, self.patch);
        put(buf, 8, self.release.to_u16());
        Ok(PRODUCT_VERSION_LEN)
    }
}

/// Decode a big-endian UTF-16 string (§4.3 "String") into an owned `String`.
pub fn decode_utf16_be(bytes: &[u8]) -> Result<String> {
    if bytes.len() % 2 != 0 {
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

/// Parse a Batch/Array of 16-byte elements (§4.3): 8-byte header
/// (`count: u32`, `item_len: u32`, both big-endian) followed by `count`
/// 16-byte elements. Used for every UL/StrongRef Batch or Array in the
/// Root Metadata Sets (`EssenceContainers`, `DMSchemes`, `Identifications`,
/// `Packages`, `EssenceContainerData`).
pub fn parse_uid_batch(bytes: &[u8]) -> Result<Vec<UlBytes>> {
    if bytes.is_empty() {
        // A zero-length property (no header at all) is treated as an empty
        // batch — some encoders omit the property entirely rather than
        // emit an empty 8-byte header; parse of an explicit empty header
        // is handled by the `count == 0` branch below.
        return Ok(Vec::new());
    }
    if bytes.len() < 8 {
        return Err(Error::InvalidBatchHeader {
            count: 0,
            item_len: 0,
            buffer_len: bytes.len(),
        });
    }
    let count = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let item_len = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    let body = &bytes[8..];
    if item_len != 16 || body.len() != count as usize * 16 {
        return Err(Error::InvalidBatchHeader {
            count,
            item_len,
            buffer_len: body.len(),
        });
    }
    let mut out = Vec::with_capacity(count as usize);
    for chunk in body.chunks_exact(16) {
        out.push(<UlBytes>::try_from(chunk).expect("chunks_exact(16)"));
    }
    Ok(out)
}

/// Serialize a Batch/Array of 16-byte elements (§4.3) — see
/// [`parse_uid_batch`].
#[must_use]
pub fn serialize_uid_batch(items: &[UlBytes]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + items.len() * 16);
    out.extend_from_slice(&(items.len() as u32).to_be_bytes());
    out.extend_from_slice(&16u32.to_be_bytes());
    for item in items {
        out.extend_from_slice(item);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn auid_ul_round_trip() {
        let ul: UlBytes = [
            0x06, 0x0E, 0x2B, 0x34, 0x01, 0x01, 0x01, 0x0E, 0x04, 0x04, 0x05, 0x03, 0, 0, 0, 0,
        ];
        let auid = Auid::from_ul(ul);
        assert!(auid.is_ul());
        assert_eq!(auid.as_ul_bytes(), ul);
    }

    #[test]
    fn auid_uuid_round_trip() {
        let uuid: UlBytes = [
            0x07, 0x72, 0x26, 0x2E, 0x76, 0x55, 0x43, 0x6F, 0x8F, 0xF3, 0x8A, 0xC5, 0x1B, 0x77,
            0x1E, 0x02,
        ];
        let auid = Auid::from_uuid(uuid);
        assert!(!auid.is_ul());
        // Spec Table 2 worked example: AUID storage order is
        // 8F.F3.8A.C5.1B.77.1E.02.07.72.26.2E.76.55.43.6F
        assert_eq!(
            auid.0,
            [
                0x8F, 0xF3, 0x8A, 0xC5, 0x1B, 0x77, 0x1E, 0x02, 0x07, 0x72, 0x26, 0x2E, 0x76, 0x55,
                0x43, 0x6F
            ]
        );
        assert_eq!(auid.as_uuid_bytes(), uuid);
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
    fn uid_batch_round_trip() {
        let items = alloc::vec![[1u8; 16], [2u8; 16], [3u8; 16]];
        let bytes = serialize_uid_batch(&items);
        assert_eq!(parse_uid_batch(&bytes).unwrap(), items);
    }

    #[test]
    fn empty_uid_batch_round_trip() {
        let items: Vec<UlBytes> = Vec::new();
        let bytes = serialize_uid_batch(&items);
        assert_eq!(bytes.len(), 8);
        assert_eq!(parse_uid_batch(&bytes).unwrap(), items);
    }
}
