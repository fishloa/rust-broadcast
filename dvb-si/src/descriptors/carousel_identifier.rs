//! Carousel Identifier Descriptor — ISO/IEC 13818-6 / ETSI TR 101 202 §4.7.7.1 (tag 0x13).
//!
//! Table 4.17 / 4.17a (TR 101 202 v1.2.1). Carried in the PMT `ES_info` loop
//! to bind an elementary stream to a DVB object carousel.
//!
//! Wire layout:
//!
//! ```text
//! carousel_identifier_descriptor() {
//!   descriptor_tag       8   = 0x13
//!   descriptor_length    8
//!   carousel_id         32   uimsbf
//!   FormatId             8   uimsbf   (selects the FormatSpecifier; Table 4.17a)
//!   FormatSpecifier()    8×N2         (depends on FormatId; 0 bytes if 0x00)
//!   private_data_byte    8×N1         (remainder of the descriptor body)
//! }
//! ```
//!
//! `FormatId` values: `0x00` → no specifier; `0x01` → aggregated specifier
//! (full field set from Table 4.17a); `0x02`–`0xFF` → reserved/private
//! (carried as opaque bytes in [`FormatSpecifier::Other`]).

use super::descriptor_body;
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for carousel_identifier_descriptor.
pub const TAG: u8 = 0x13;

// ── field-width constants ─────────────────────────────────────────────────────
/// 2-byte descriptor outer header (tag + length).
const HEADER_LEN: usize = 2;
/// `carousel_id` field width in bytes (32-bit uimsbf).
const CAROUSEL_ID_LEN: usize = 4;
/// `FormatId` field width in bytes (8-bit uimsbf).
const FORMAT_ID_LEN: usize = 1;
/// Fixed prefix of every descriptor body: carousel_id + FormatId.
const BODY_PREFIX_LEN: usize = CAROUSEL_ID_LEN + FORMAT_ID_LEN;

// ── FormatId = 0x01 FormatSpecifier field widths ──────────────────────────────
/// `ModuleVersion` field: 8-bit uimsbf.
const FS1_MODULE_VERSION_LEN: usize = 1;
/// `ModuleId` field: 16-bit uimsbf.
const FS1_MODULE_ID_LEN: usize = 2;
/// `BlockSize` field: 16-bit uimsbf.
const FS1_BLOCK_SIZE_LEN: usize = 2;
/// `ModuleSize` field: 32-bit uimsbf.
const FS1_MODULE_SIZE_LEN: usize = 4;
/// `CompressionMethod` field: 8-bit uimsbf.
const FS1_COMPRESSION_METHOD_LEN: usize = 1;
/// `OriginalSize` field: 32-bit uimsbf.
const FS1_ORIGINAL_SIZE_LEN: usize = 4;
/// `TimeOut` field: 8-bit uimsbf (TR 101 202 v1.2.1; 8-bit, not the 32-bit of
/// later-edition documents).
const FS1_TIMEOUT_LEN: usize = 1;
/// `ObjectKeyLength` field: 8-bit uimsbf.
const FS1_OBJECT_KEY_LENGTH_LEN: usize = 1;
/// Total fixed-size bytes of the FormatId=0x01 specifier before `ObjectKeyData`.
const FS1_FIXED_LEN: usize = FS1_MODULE_VERSION_LEN
    + FS1_MODULE_ID_LEN
    + FS1_BLOCK_SIZE_LEN
    + FS1_MODULE_SIZE_LEN
    + FS1_COMPRESSION_METHOD_LEN
    + FS1_ORIGINAL_SIZE_LEN
    + FS1_TIMEOUT_LEN
    + FS1_OBJECT_KEY_LENGTH_LEN; // = 16

// ── FormatId values ───────────────────────────────────────────────────────────
/// `FormatId = 0x00`: no `FormatSpecifier` bytes present.
const FORMAT_ID_NONE: u8 = 0x00;
/// `FormatId = 0x01`: aggregated FormatSpecifier (Table 4.17a).
const FORMAT_ID_AGGREGATED: u8 = 0x01;

/// `FormatSpecifier` — the optional aggregated block in a
/// `carousel_identifier_descriptor` (ISO/IEC 13818-6 / TR 101 202 Table 4.17a).
///
/// Dispatch on `FormatId`:
///
/// | FormatId | Variant | Meaning |
/// |----------|---------|---------|
/// | `0x00` | [`Absent`][FormatSpecifier::Absent] | No specifier bytes |
/// | `0x01` | [`Aggregated`][FormatSpecifier::Aggregated] | Full field set (Table 4.17a) |
/// | `0x02`–`0xFF` | [`Other`][FormatSpecifier::Other] | Reserved/private; raw bytes |
///
/// This is a data-carrying ADT — the variants hold payloads, not just labels.
/// Use [`FormatSpecifier::format_id`] to retrieve the wire `FormatId` value.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum FormatSpecifier<'a> {
    /// `FormatId = 0x00`: no `FormatSpecifier` bytes.
    Absent,
    /// `FormatId = 0x01`: aggregated FormatSpecifier (TR 101 202 Table 4.17a).
    ///
    /// Wire layout of the specifier block (all uimsbf):
    ///
    /// ```text
    /// ModuleVersion      8
    /// ModuleId          16
    /// BlockSize         16
    /// ModuleSize        32
    /// CompressionMethod  8
    /// OriginalSize      32
    /// TimeOut            8   (8-bit per TR 101 202 v1.2.1)
    /// ObjectKeyLength    8   = N1
    /// ObjectKeyData      8×N1
    /// ```
    Aggregated {
        /// `ModuleVersion` `[7:0]` — version of the DSM-CC module.
        module_version: u8,
        /// `ModuleId` `[15:0]` — identity of the DSM-CC module.
        module_id: u16,
        /// `BlockSize` `[15:0]` — download block size in bytes.
        block_size: u16,
        /// `ModuleSize` `[31:0]` — total module size in bytes.
        module_size: u32,
        /// `CompressionMethod` `[7:0]` — 0x00 = none.
        compression_method: u8,
        /// `OriginalSize` `[31:0]` — uncompressed module size in bytes; only
        /// meaningful when `compression_method != 0x00`.
        original_size: u32,
        /// `TimeOut` `[7:0]` — acquisition timeout (8-bit; TR 101 202 v1.2.1).
        timeout: u8,
        /// `ObjectKeyData` — the object key bytes (`ObjectKeyLength` bytes).
        #[cfg_attr(feature = "serde", serde(borrow))]
        object_key: &'a [u8],
    },
    /// `FormatId = 0x02`–`0xFF`: reserved or private format; carried opaque.
    Other {
        /// The raw `FormatId` byte.
        format_id: u8,
        /// Raw specifier bytes (remainder after the carousel_id + FormatId
        /// prefix, before `private_data_byte`).
        ///
        /// For unknown `FormatId` values the caller cannot know where the
        /// specifier ends and private data begins, so the entire body remainder
        /// is treated as specifier bytes and `private_data` is empty.
        #[cfg_attr(feature = "serde", serde(borrow))]
        bytes: &'a [u8],
    },
}

impl<'a> FormatSpecifier<'a> {
    /// The wire `FormatId` byte for this variant.
    #[must_use]
    pub fn format_id(&self) -> u8 {
        match self {
            FormatSpecifier::Absent => FORMAT_ID_NONE,
            FormatSpecifier::Aggregated { .. } => FORMAT_ID_AGGREGATED,
            FormatSpecifier::Other { format_id, .. } => *format_id,
        }
    }

    /// Byte length of the serialized specifier block (excluding the outer
    /// descriptor tag/length/carousel_id/FormatId fields).
    fn serialized_len(&self) -> usize {
        match self {
            FormatSpecifier::Absent => 0,
            FormatSpecifier::Aggregated { object_key, .. } => FS1_FIXED_LEN + object_key.len(),
            FormatSpecifier::Other { bytes, .. } => bytes.len(),
        }
    }
}

/// Carousel Identifier Descriptor (tag 0x13) —
/// ISO/IEC 13818-6 / ETSI TR 101 202 §4.7.7.1.
///
/// Carried in the PMT `ES_info` loop to bind an elementary stream to a DVB
/// object carousel. The `format` field selects the typed
/// [`FormatSpecifier`] variant; the `private_data` tail follows.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct CarouselIdentifierDescriptor<'a> {
    /// `carousel_id` `[31:0]` — 32-bit carousel identity, unique per TS.
    pub carousel_id: u32,
    /// Parsed `FormatSpecifier` (dispatch on `FormatId`).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub format: FormatSpecifier<'a>,
    /// `private_data_byte` tail — zero or more bytes after the `FormatSpecifier`.
    ///
    /// Empty for [`FormatSpecifier::Other`] (the whole remainder is in
    /// [`FormatSpecifier::Other::bytes`] since the boundary is unknown).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub private_data: &'a [u8],
}

impl<'a> Parse<'a> for CarouselIdentifierDescriptor<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "CarouselIdentifierDescriptor",
            "unexpected tag for carousel_identifier_descriptor",
        )?;
        let (prefix, after_prefix) =
            body.split_first_chunk::<BODY_PREFIX_LEN>()
                .ok_or(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "carousel_identifier_descriptor body shorter than 5 bytes",
                })?;
        let carousel_id = u32::from_be_bytes([prefix[0], prefix[1], prefix[2], prefix[3]]);
        let format_id = prefix[CAROUSEL_ID_LEN];

        let (format, private_data) = match format_id {
            FORMAT_ID_NONE => (FormatSpecifier::Absent, after_prefix),
            FORMAT_ID_AGGREGATED => {
                let (fs1, after_fixed) = after_prefix.split_first_chunk::<FS1_FIXED_LEN>().ok_or(
                    Error::InvalidDescriptor {
                        tag: TAG,
                        reason: "FormatId=0x01 specifier truncated (insufficient fixed fields)",
                    },
                )?;
                // fs1 layout: [0]=module_version, [1..3]=module_id, [3..5]=block_size,
                // [5..9]=module_size, [9]=compression_method, [10..14]=original_size,
                // [14]=timeout, [15]=object_key_length
                let module_version = fs1[0];
                let module_id = u16::from_be_bytes([fs1[1], fs1[2]]);
                let block_size = u16::from_be_bytes([fs1[3], fs1[4]]);
                let module_size = u32::from_be_bytes([fs1[5], fs1[6], fs1[7], fs1[8]]);
                let compression_method = fs1[9];
                let original_size = u32::from_be_bytes([fs1[10], fs1[11], fs1[12], fs1[13]]);
                let timeout = fs1[14];
                let object_key_length = fs1[15] as usize;
                if object_key_length > after_fixed.len() {
                    return Err(Error::InvalidDescriptor {
                        tag: TAG,
                        reason: "FormatId=0x01 ObjectKeyData exceeds descriptor body",
                    });
                }
                let object_key = &after_fixed[..object_key_length];
                let private_data = &after_fixed[object_key_length..];
                (
                    FormatSpecifier::Aggregated {
                        module_version,
                        module_id,
                        block_size,
                        module_size,
                        compression_method,
                        original_size,
                        timeout,
                        object_key,
                    },
                    private_data,
                )
            }
            other => {
                // Unknown/private: the boundary between specifier and
                // private_data is unknowable, so absorb the whole remainder
                // into the specifier bytes.
                (
                    FormatSpecifier::Other {
                        format_id: other,
                        bytes: after_prefix,
                    },
                    &[][..],
                )
            }
        };
        Ok(Self {
            carousel_id,
            format,
            private_data,
        })
    }
}

impl Serialize for CarouselIdentifierDescriptor<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        let body = BODY_PREFIX_LEN + self.format.serialized_len() + self.private_data.len();
        HEADER_LEN + body
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let total = self.serialized_len();
        let body_len = BODY_PREFIX_LEN + self.format.serialized_len() + self.private_data.len();
        if body_len > u8::MAX as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "carousel_identifier_descriptor body exceeds 255 bytes",
            });
        }
        if buf.len() < total {
            return Err(Error::OutputBufferTooSmall {
                need: total,
                have: buf.len(),
            });
        }

        buf[0] = TAG;
        buf[1] = body_len as u8;
        buf[HEADER_LEN..HEADER_LEN + CAROUSEL_ID_LEN]
            .copy_from_slice(&self.carousel_id.to_be_bytes());
        buf[HEADER_LEN + CAROUSEL_ID_LEN] = self.format.format_id();

        let mut pos = HEADER_LEN + BODY_PREFIX_LEN;
        match &self.format {
            FormatSpecifier::Absent => {}
            FormatSpecifier::Aggregated {
                module_version,
                module_id,
                block_size,
                module_size,
                compression_method,
                original_size,
                timeout,
                object_key,
            } => {
                if object_key.len() > u8::MAX as usize {
                    return Err(Error::InvalidDescriptor {
                        tag: TAG,
                        reason: "FormatId=0x01 ObjectKeyData exceeds 255 bytes",
                    });
                }
                buf[pos] = *module_version;
                pos += FS1_MODULE_VERSION_LEN;
                buf[pos..pos + FS1_MODULE_ID_LEN].copy_from_slice(&module_id.to_be_bytes());
                pos += FS1_MODULE_ID_LEN;
                buf[pos..pos + FS1_BLOCK_SIZE_LEN].copy_from_slice(&block_size.to_be_bytes());
                pos += FS1_BLOCK_SIZE_LEN;
                buf[pos..pos + FS1_MODULE_SIZE_LEN].copy_from_slice(&module_size.to_be_bytes());
                pos += FS1_MODULE_SIZE_LEN;
                buf[pos] = *compression_method;
                pos += FS1_COMPRESSION_METHOD_LEN;
                buf[pos..pos + FS1_ORIGINAL_SIZE_LEN].copy_from_slice(&original_size.to_be_bytes());
                pos += FS1_ORIGINAL_SIZE_LEN;
                buf[pos] = *timeout;
                pos += FS1_TIMEOUT_LEN;
                buf[pos] = object_key.len() as u8;
                pos += FS1_OBJECT_KEY_LENGTH_LEN;
                buf[pos..pos + object_key.len()].copy_from_slice(object_key);
                pos += object_key.len();
            }
            FormatSpecifier::Other { bytes, .. } => {
                buf[pos..pos + bytes.len()].copy_from_slice(bytes);
                pos += bytes.len();
            }
        }
        buf[pos..pos + self.private_data.len()].copy_from_slice(self.private_data);
        Ok(total)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for CarouselIdentifierDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "CAROUSEL_IDENTIFIER";
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── byte-anchor constants for hand-built test vector ─────────────────────
    //
    // Descriptor wire layout (FormatId=0x01, ObjectKeyLength=1, 2 private bytes):
    //
    // Offset  Field                 Value      Comment
    //   [0]   descriptor_tag        0x13
    //   [1]   descriptor_length     0x18       = 24 bytes body
    //   [2]   carousel_id[31:24]    0x00
    //   [3]   carousel_id[23:16]    0x00
    //   [4]   carousel_id[15:8]     0x00
    //   [5]   carousel_id[7:0]      0x01       carousel_id = 1
    //   [6]   FormatId              0x01       = Aggregated
    //   [7]   ModuleVersion         0x03
    //   [8]   ModuleId[15:8]        0x00
    //   [9]   ModuleId[7:0]         0x05       module_id = 5
    //  [10]   BlockSize[15:8]       0x04
    //  [11]   BlockSize[7:0]        0x00       block_size = 1024
    //  [12]   ModuleSize[31:24]     0x00
    //  [13]   ModuleSize[23:16]     0x00
    //  [14]   ModuleSize[15:8]      0x10
    //  [15]   ModuleSize[7:0]       0x00       module_size = 0x1000 = 4096
    //  [16]   CompressionMethod     0x00       no compression
    //  [17]   OriginalSize[31:24]   0x00
    //  [18]   OriginalSize[23:16]   0x00
    //  [19]   OriginalSize[15:8]    0x10
    //  [20]   OriginalSize[7:0]     0x00       original_size = 4096
    //  [21]   TimeOut               0x1E       = 30
    //  [22]   ObjectKeyLength       0x01       N1 = 1
    //  [23]   ObjectKeyData[0]      0xAB
    //  [24]   private_data_byte[0]  0xDE
    //  [25]   private_data_byte[1]  0xAD
    //
    // Total: 26 bytes; body = 24 (0x18).

    #[rustfmt::skip]
    const ANCHOR_BYTES: &[u8] = &[
        0x13, 0x18,                         // tag, length=24
        0x00, 0x00, 0x00, 0x01,             // carousel_id = 1
        0x01,                               // FormatId = Aggregated
        0x03,                               // ModuleVersion = 3
        0x00, 0x05,                         // ModuleId = 5
        0x04, 0x00,                         // BlockSize = 1024
        0x00, 0x00, 0x10, 0x00,             // ModuleSize = 4096
        0x00,                               // CompressionMethod = 0
        0x00, 0x00, 0x10, 0x00,             // OriginalSize = 4096
        0x1E,                               // TimeOut = 30
        0x01,                               // ObjectKeyLength = 1
        0xAB,                               // ObjectKeyData
        0xDE, 0xAD,                         // private_data
    ];

    fn anchor_descriptor() -> CarouselIdentifierDescriptor<'static> {
        CarouselIdentifierDescriptor {
            carousel_id: 1,
            format: FormatSpecifier::Aggregated {
                module_version: 3,
                module_id: 5,
                block_size: 1024,
                module_size: 4096,
                compression_method: 0,
                original_size: 4096,
                timeout: 30,
                object_key: &[0xAB],
            },
            private_data: &[0xDE, 0xAD],
        }
    }

    // ── byte-anchor test ──────────────────────────────────────────────────────

    #[test]
    fn byte_anchor_serialize_matches_hand_built() {
        let d = anchor_descriptor();
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(
            buf.as_slice(),
            ANCHOR_BYTES,
            "serialized bytes differ from anchor"
        );
    }

    #[test]
    fn byte_anchor_parse_extracts_correct_fields() {
        let d = CarouselIdentifierDescriptor::parse(ANCHOR_BYTES).unwrap();
        assert_eq!(d.carousel_id, 1);
        assert_eq!(d.private_data, &[0xDE, 0xAD]);
        match &d.format {
            FormatSpecifier::Aggregated {
                module_version,
                module_id,
                block_size,
                module_size,
                compression_method,
                original_size,
                timeout,
                object_key,
            } => {
                assert_eq!(*module_version, 3);
                assert_eq!(*module_id, 5);
                assert_eq!(*block_size, 1024);
                assert_eq!(*module_size, 4096);
                assert_eq!(*compression_method, 0);
                assert_eq!(*original_size, 4096);
                assert_eq!(*timeout, 30);
                assert_eq!(*object_key, &[0xAB]);
            }
            other => panic!("expected Aggregated, got {other:?}"),
        }
    }

    #[test]
    fn byte_anchor_round_trip_byte_identical() {
        // parse → serialize → byte-identical with the original anchor bytes.
        let d = CarouselIdentifierDescriptor::parse(ANCHOR_BYTES).unwrap();
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(
            buf.as_slice(),
            ANCHOR_BYTES,
            "round-trip not byte-identical"
        );
        // Second pass: serialize the struct literal, parse back, re-serialize.
        let d2 = anchor_descriptor();
        let mut buf2 = vec![0u8; d2.serialized_len()];
        d2.serialize_into(&mut buf2).unwrap();
        let d3 = CarouselIdentifierDescriptor::parse(&buf2).unwrap();
        let mut buf3 = vec![0u8; d3.serialized_len()];
        d3.serialize_into(&mut buf3).unwrap();
        assert_eq!(buf2, buf3, "struct literal round-trip not byte-identical");
    }

    // ── FormatId=0x00 (Absent) ────────────────────────────────────────────────

    #[test]
    fn parse_format_absent_extracts_fields() {
        // carousel_id=0xDEADBEEF, FormatId=0x00, 3 private bytes.
        let bytes = [TAG, 0x08, 0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0xAA, 0xBB, 0xCC];
        let d = CarouselIdentifierDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.carousel_id, 0xDEAD_BEEF);
        assert_eq!(d.format, FormatSpecifier::Absent);
        assert_eq!(d.format.format_id(), 0x00);
        assert_eq!(d.private_data, &[0xAA, 0xBB, 0xCC]);
    }

    #[test]
    fn round_trip_format_absent() {
        let d = CarouselIdentifierDescriptor {
            carousel_id: 0x0000_0042,
            format: FormatSpecifier::Absent,
            private_data: &[0x01, 0x02, 0x03],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = CarouselIdentifierDescriptor::parse(&buf).unwrap();
        assert_eq!(re, d);
    }

    #[test]
    fn format_absent_no_private_data() {
        // Minimum body: 5 bytes (carousel_id + FormatId only).
        let bytes = [TAG, 0x05, 0x00, 0x00, 0x00, 0x00, 0x00];
        let d = CarouselIdentifierDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.carousel_id, 0);
        assert_eq!(d.format, FormatSpecifier::Absent);
        assert!(d.private_data.is_empty());
    }

    // ── FormatId=0x02 (Other / private) ──────────────────────────────────────

    #[test]
    fn parse_format_other_carries_raw_bytes() {
        // FormatId=0x42 (private), 3 opaque bytes.
        let bytes = [TAG, 0x08, 0x00, 0x00, 0x00, 0x7F, 0x42, 0xAA, 0xBB, 0xCC];
        let d = CarouselIdentifierDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.carousel_id, 0x7F);
        assert_eq!(d.format.format_id(), 0x42);
        match &d.format {
            FormatSpecifier::Other { format_id, bytes } => {
                assert_eq!(*format_id, 0x42);
                assert_eq!(*bytes, &[0xAA, 0xBB, 0xCC]);
            }
            other => panic!("expected Other, got {other:?}"),
        }
        // private_data is always empty for Other (whole remainder in bytes).
        assert!(d.private_data.is_empty());
    }

    #[test]
    fn round_trip_format_other() {
        let d = CarouselIdentifierDescriptor {
            carousel_id: 0x0000_00FF,
            format: FormatSpecifier::Other {
                format_id: 0x05,
                bytes: &[0x11, 0x22],
            },
            private_data: &[],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = CarouselIdentifierDescriptor::parse(&buf).unwrap();
        assert_eq!(re, d);
    }

    // ── error cases ──────────────────────────────────────────────────────────

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = CarouselIdentifierDescriptor::parse(&[0x14, 0x05, 0x00, 0x00, 0x00, 0x01, 0x00])
            .unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x14, .. }));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        // Only the tag byte — not even the length field present.
        let err = CarouselIdentifierDescriptor::parse(&[TAG]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn parse_rejects_body_too_short() {
        // length=4: 4-byte carousel_id but no FormatId.
        let err =
            CarouselIdentifierDescriptor::parse(&[TAG, 0x04, 0x00, 0x00, 0x00, 0x01]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { .. }));
    }

    #[test]
    fn parse_rejects_format1_fixed_truncated() {
        // FormatId=0x01 but only 3 bytes after the prefix (need FS1_FIXED_LEN=16).
        let mut bytes = vec![TAG, 0x08, 0x00, 0x00, 0x00, 0x01, 0x01, 0xAA, 0xBB, 0xCC];
        bytes[1] = (bytes.len() - 2) as u8;
        let err = CarouselIdentifierDescriptor::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { .. }));
    }

    #[test]
    fn parse_rejects_format1_objectkey_overflow() {
        // FormatId=0x01 with ObjectKeyLength claiming more bytes than available.
        // Fixed fields (16 bytes) + ObjectKeyLength=0x0A (10), but no key bytes follow.
        let body = vec![
            0x00, 0x00, 0x00, 0x02, // carousel_id
            0x01, // FormatId
            0x01, // ModuleVersion
            0x00, 0x01, // ModuleId
            0x00, 0x80, // BlockSize
            0x00, 0x00, 0x20, 0x00, // ModuleSize
            0x00, // CompressionMethod
            0x00, 0x00, 0x20, 0x00, // OriginalSize
            0x05, // TimeOut
            0x0A, // ObjectKeyLength=10, but no bytes follow
        ];
        let mut full = vec![TAG, body.len() as u8];
        full.extend_from_slice(&body);
        // Trim to only 1 key byte when 10 were declared.
        full.push(0xEE);
        full[1] = (full.len() - 2) as u8;
        // Re-fix body length in the full buffer.
        // Actually: full = [tag, len, ...body..., 0xEE] where len was set
        // for body (22 bytes) but body claims 10 key bytes.  Parse should reject.
        let _ = body; // already consumed
        let err = CarouselIdentifierDescriptor::parse(&full).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { .. }));
    }

    #[test]
    fn parse_rejects_length_overrun() {
        // Declared length=10 but only 4 bytes of body present.
        let err =
            CarouselIdentifierDescriptor::parse(&[TAG, 0x0A, 0x00, 0x00, 0x00, 0x01]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn serialize_rejects_too_small_buffer() {
        let d = anchor_descriptor();
        let mut tiny = [0u8; 2];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }

    // ── serde tests ──────────────────────────────────────────────────────────

    #[cfg(feature = "serde")]
    #[test]
    fn serde_serialize_aggregated_fields_present() {
        let d = anchor_descriptor();
        let json = serde_json::to_string(&d).unwrap();
        assert!(json.contains("\"carousel_id\""));
        assert!(json.contains("\"format\""));
        // Aggregated variant key.
        assert!(json.contains("Aggregated") || json.contains("aggregated"));
        assert!(json.contains("\"module_version\""));
        assert!(json.contains("\"timeout\""));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_serialize_absent_format() {
        let d = CarouselIdentifierDescriptor {
            carousel_id: 0xABCD,
            format: FormatSpecifier::Absent,
            private_data: &[],
        };
        let json = serde_json::to_string(&d).unwrap();
        assert!(json.contains("\"carousel_id\""));
        assert!(json.contains("Absent") || json.contains("absent"));
    }
}
