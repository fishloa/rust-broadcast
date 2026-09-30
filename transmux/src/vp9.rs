//! VP9 in ISOBMFF — `vp09` VisualSampleEntry + `vpcC` config box.
//!
//! WebM Project "VP Codec ISO Media File Format Binding"
//! (<https://www.webmproject.org/vp9/mp4/>, VPCodecConfigurationBox).
//!
//! # Types
//!
//! | Box | FourCC | Description |
//! |-----|--------|-------------|
//! | [`Vp9ConfigurationBox`] | `vpcC` | `FullBox` v1 VPCodecConfigurationRecord |
//! | [`Vp9SampleEntry`] | `vp09` | VisualSampleEntry carrying `vpcC` |

use crate::error::{Error, Result};
use crate::sample_entries::VisualSampleEntryFields;
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

/// FourCC of the VP9 config box.
pub const VPCC_FOURCC: [u8; 4] = *b"vpcC";
/// FourCC of the VP9 sample entry.
pub const VP09_FOURCC: [u8; 4] = *b"vp09";
/// 8-byte box header.
const BOX_HDR: usize = 8;
/// FullBox extension: version(1) + flags(3).
const FULL_HDR: usize = 4;
/// Fixed length of the VPCodecConfigurationRecord (before `codecInitializationData`).
const VPCC_RECORD_FIXED: usize = 8;
/// The only FullBox version the binding defines. The VP Codec ISO Media File
/// Format Binding v1.0 (2017-03-31) declares the box as
/// `FullBox('vpcC', version = 1, 0)` and states "Version 0 is deprecated and
/// should not be used" — it gives no syntax for version 0 at all, so no other
/// version can be decoded or emitted (see `docs/codec/vp9-isobmff.md`).
const VERSION_1: u8 = 1;
/// A 4-bit field occupies the high nibble of its byte.
const BITS_4: u8 = 4;
/// `chromaSubsampling` is 3 bits, so 0..=7 (values 4..7 reserved); the record
/// packs `bitDepth(4)` then `chromaSubsampling(3)` in one byte.
const CHROMA_SUBSAMPLING_MASK: u8 = 0x07;

/// VPCodecConfigurationBox (`vpcC`) — a `FullBox(version=1, 0)` VP9 config record.
///
/// Record layout after the FullBox header — VP Codec ISO Media File Format
/// Binding v1.0 `VPCodecConfigurationRecord`:
/// `profile(8)` | `level(8)` | `bitDepth(4)` | `chromaSubsampling(3)` |
/// `videoFullRangeFlag(1)` | `colourPrimaries(8)` | `transferCharacteristics(8)` |
/// `matrixCoefficients(8)` | `codecInitializationDataSize(16)` | `codecInitializationData[]`.
///
/// Only version 1 exists in the binding; a version other than 1 is
/// [`Error::InvalidValue`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Vp9ConfigurationBox {
    /// FullBox version (1).
    pub version: u8,
    /// FullBox flags (0).
    pub flags: u32,
    /// VP9 profile (0-3).
    pub profile: u8,
    /// VP9 level.
    pub level: u8,
    /// Bit depth in bits (8, 10, or 12).
    pub bit_depth: u8,
    /// `chromaSubsampling` (3 bits; 0..=3 defined, 4..7 reserved).
    pub chroma_subsampling: u8,
    /// `videoFullRangeFlag`.
    pub video_full_range_flag: bool,
    /// `colourPrimaries` (ISO/IEC 23001-8).
    pub colour_primaries: u8,
    /// `transferCharacteristics` (ISO/IEC 23001-8).
    pub transfer_characteristics: u8,
    /// `matrixCoefficients` (ISO/IEC 23001-8).
    pub matrix_coefficients: u8,
    /// `codecInitializationData` (MUST be empty for VP8/VP9).
    pub codec_initialization_data: Vec<u8>,
}

impl Vp9ConfigurationBox {
    /// RFC 6381 codec string `vp09.PP.LL.DD` (profile, level, bit depth).
    pub fn rfc6381(&self) -> alloc::string::String {
        use alloc::format;
        format!(
            "vp09.{:02}.{:02}.{:02}",
            self.profile, self.level, self.bit_depth
        )
    }
}

impl<'a> Parse<'a> for Vp9ConfigurationBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < FULL_HDR + VPCC_RECORD_FIXED {
            return Err(Error::BufferTooShort {
                need: FULL_HDR + VPCC_RECORD_FIXED,
                have: bytes.len(),
                what: "vpcC body",
            });
        }
        let version = bytes[0];
        // Only version 1 is defined by the binding; version 0 is deprecated with
        // no published syntax, so guessing at a layout for it would be
        // fabrication. Reject anything else rather than mis-reading it.
        if version != VERSION_1 {
            return Err(Error::InvalidValue {
                field: "vpcC FullBox version",
                value: u64::from(version),
                reason: "VP Codec ISO Media File Format Binding defines only version 1 \
                         (version 0 is deprecated with no published syntax)",
            });
        }
        let flags = u32::from_be_bytes([0, bytes[1], bytes[2], bytes[3]]);
        let r = &bytes[FULL_HDR..];
        let profile = r[0];
        let level = r[1];
        // `bitDepth(4) chromaSubsampling(3) videoFullRangeFlag(1)`, then the
        // three 8-bit CICP fields (v1 `VPCodecConfigurationRecord`).
        let bit_depth = r[2] >> BITS_4;
        let chroma_subsampling = (r[2] >> 1) & CHROMA_SUBSAMPLING_MASK;
        let video_full_range_flag = (r[2] & 0x01) != 0;
        let colour_primaries = r[3];
        let transfer_characteristics = r[4];
        let matrix_coefficients = r[5];
        let init_size = u16::from_be_bytes([r[6], r[7]]) as usize;
        let init_start = FULL_HDR + VPCC_RECORD_FIXED;
        // A `codecInitializationDataSize` that overruns the box is an error, not
        // a silent truncation: the size is the only length the record carries,
        // so a short read means the box is malformed.
        let init_end = init_start
            .checked_add(init_size)
            .ok_or(Error::BufferTooShort {
                need: usize::MAX,
                have: bytes.len(),
                what: "codecInitializationDataSize overflows the offset",
            })?;
        if init_end > bytes.len() {
            return Err(Error::BufferTooShort {
                need: init_end,
                have: bytes.len(),
                what: "codecInitializationData",
            });
        }
        let codec_initialization_data = bytes[init_start..init_end].to_vec();
        Ok(Self {
            version,
            flags,
            profile,
            level,
            bit_depth,
            chroma_subsampling,
            video_full_range_flag,
            colour_primaries,
            transfer_characteristics,
            matrix_coefficients,
            codec_initialization_data,
        })
    }
}

impl Serialize for Vp9ConfigurationBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        FULL_HDR + VPCC_RECORD_FIXED + self.codec_initialization_data.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        // Only version 1 can be written; emitting any other version with this
        // layout would claim a syntax that is not the one being written.
        if self.version != VERSION_1 {
            return Err(Error::InvalidValue {
                field: "vpcC FullBox version",
                value: u64::from(self.version),
                reason: "VP Codec ISO Media File Format Binding defines only version 1",
            });
        }
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
        let r = &mut buf[FULL_HDR..];
        r[0] = self.profile;
        r[1] = self.level;
        // Both 4-bit/3-bit fields are written through the checked helper:
        // `bitDepth` outside 0..=15 would shift its high bits out of the byte,
        // and `chromaSubsampling` outside 0..=7 would fold into
        // `videoFullRangeFlag` — either way the wire would describe a different
        // stream than the struct.
        let bit_depth = broadcast_common::len::fit_bits(u64::from(self.bit_depth), 4, "bitDepth")?;
        let chroma = broadcast_common::len::fit_bits(
            u64::from(self.chroma_subsampling),
            3,
            "chromaSubsampling",
        )?;
        r[2] = ((bit_depth as u8) << BITS_4)
            | ((chroma as u8) << 1)
            | (self.video_full_range_flag as u8);
        r[3] = self.colour_primaries;
        r[4] = self.transfer_characteristics;
        r[5] = self.matrix_coefficients;
        let size = broadcast_common::len::fit_u16(
            self.codec_initialization_data.len(),
            "codecIntializationDataSize",
        )?;
        r[6..8].copy_from_slice(&size.to_be_bytes());
        r[VPCC_RECORD_FIXED..VPCC_RECORD_FIXED + self.codec_initialization_data.len()]
            .copy_from_slice(&self.codec_initialization_data);
        Ok(need)
    }
}

/// VP9 sample entry (`vp09`) — a `VisualSampleEntry` carrying a `vpcC` box.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Vp9SampleEntry {
    /// Fixed VisualSampleEntry fields.
    pub visual: VisualSampleEntryFields,
    /// The `vpcC` configuration box.
    pub config: Vp9ConfigurationBox,
}

impl Vp9SampleEntry {
    /// Parse from full box bytes (including the 8-byte header).
    pub fn parse_entry(bytes: &[u8]) -> Result<Self> {
        let visual = VisualSampleEntryFields::parse_body(bytes, "vp09")?;
        let region = &bytes[BOX_HDR + VisualSampleEntryFields::serialized_len()..];
        let vpcc = crate::sample_entries::find_config_box(region, &VPCC_FOURCC).ok_or(
            Error::BufferTooShort {
                need: 0,
                have: 0,
                what: "vp09 missing vpcC",
            },
        )?;
        let config = Vp9ConfigurationBox::parse(&vpcc[BOX_HDR..])?;
        Ok(Self { visual, config })
    }
}

impl Serialize for Vp9SampleEntry {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        BOX_HDR + VisualSampleEntryFields::serialized_len() + BOX_HDR + self.config.serialized_len()
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
        buf[c..c + 4].copy_from_slice(&VP09_FOURCC);
        c += 4;
        c += self.visual.serialize_body_into(&mut buf[c..])?;
        let vpcc_len = BOX_HDR + self.config.serialized_len();
        buf[c..c + 4].copy_from_slice(&(vpcc_len as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(&VPCC_FOURCC);
        c += 4;
        c += self.config.serialize_into(&mut buf[c..])?;
        Ok(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(codec_initialization_data: Vec<u8>) -> Vp9ConfigurationBox {
        Vp9ConfigurationBox {
            version: 1,
            flags: 0,
            profile: 0,
            level: 10,
            bit_depth: 8,
            chroma_subsampling: 1,
            video_full_range_flag: false,
            colour_primaries: 2,
            transfer_characteristics: 2,
            matrix_coefficients: 2,
            codec_initialization_data,
        }
    }

    /// A real `vpcC` body produced by ffmpeg (independent oracle), FullBox v1.
    ///
    /// Generated with:
    /// `ffmpeg -f lavfi -i testsrc2=size=160x120:rate=25 -t 0.4 -c:v libvpx-vp9 -b:v 200k out.mp4`
    /// then reading the 20-byte `vpcC` box body (`01000000...` — FullBox
    /// version 1, flags 0). Fields: profile 0, level 10, byte 2 = `0x82`
    /// (bitDepth 8, chromaSubsampling 1, videoFullRangeFlag 0), CICP 2/2/2,
    /// `codecInitializationDataSize` 0.
    const FFMPEG_VPCC_BODY: [u8; 12] = [
        0x01, 0x00, 0x00, 0x00, // FullBox version 1, flags 0
        0x00, 0x0A, // profile 0, level 10
        0x82, // bitDepth 8 | chromaSubsampling 1 | videoFullRangeFlag 0
        0x02, // colourPrimaries
        0x02, // transferCharacteristics
        0x02, // matrixCoefficients
        0x00, 0x00, // codecInitializationDataSize
    ];

    /// The v1 record parses the real ffmpeg bytes and round-trips byte-exactly.
    #[test]
    fn version_1_ffmpeg_oracle_parses_and_round_trips() {
        let cfg = Vp9ConfigurationBox::parse(&FFMPEG_VPCC_BODY).expect("parse real vpcC");
        assert_eq!(cfg.version, 1);
        assert_eq!(cfg.flags, 0);
        assert_eq!(cfg.profile, 0);
        assert_eq!(cfg.level, 10);
        assert_eq!(cfg.bit_depth, 8);
        assert_eq!(cfg.chroma_subsampling, 1);
        assert!(!cfg.video_full_range_flag);
        assert_eq!(cfg.colour_primaries, 2);
        assert_eq!(cfg.transfer_characteristics, 2);
        assert_eq!(cfg.matrix_coefficients, 2);
        assert!(cfg.codec_initialization_data.is_empty());
        assert_eq!(cfg.rfc6381(), "vp09.00.10.08");
        assert_eq!(
            cfg.try_to_bytes().unwrap(),
            FFMPEG_VPCC_BODY,
            "the real vpcC must round-trip byte-identically"
        );
    }

    /// `codec_initialization_data` of 65 536 bytes cannot fit the 16-bit
    /// `codecIntializationDataSize` field (#1129): unfixed, `(len as u16)`
    /// silently wrapped 65536 to 0.
    #[test]
    fn oversized_init_data_length_errors() {
        let cfg = config(alloc::vec![0u8; 65536]);
        let err = cfg.try_to_bytes().unwrap_err();
        assert!(
            matches!(
                err,
                Error::FieldOverflow(broadcast_common::len::FieldOverflow {
                    field: "codecIntializationDataSize",
                    ..
                })
            ),
            "expected FieldOverflow for codecIntializationDataSize, got {err:?}"
        );
    }

    /// The boundary: exactly 65 535 bytes (u16::MAX) still round-trips.
    #[test]
    fn max_init_data_length_round_trips() {
        let cfg = config(alloc::vec![0xABu8; 65535]);
        let bytes = cfg.try_to_bytes().unwrap();
        let parsed = Vp9ConfigurationBox::parse(&bytes).unwrap();
        assert_eq!(parsed.codec_initialization_data.len(), 65535);
    }

    /// The binding (v1.0, 2017-03-31) defines only `version = 1`; it states
    /// "Version 0 is deprecated and should not be used" and publishes no v0
    /// syntax. A version other than 1 must be rejected — guessing at an
    /// undocumented layout would report wrong depth/chroma/colour values.
    #[test]
    fn non_version_1_records_are_rejected() {
        for version in [0u8, 2, 0xFF] {
            let mut body = FFMPEG_VPCC_BODY;
            body[0] = version;
            assert!(
                matches!(
                    Vp9ConfigurationBox::parse(&body),
                    Err(Error::InvalidValue {
                        field: "vpcC FullBox version",
                        ..
                    })
                ),
                "version {version} must be rejected"
            );
        }
    }

    /// Serialize must not emit a version it cannot lay out: a struct whose
    /// `version` is not 1 is an error rather than silently-written v1 bytes.
    #[test]
    fn serialize_rejects_a_non_version_1_record() {
        let mut cfg = config(Vec::new());
        cfg.version = 0;
        assert!(matches!(
            cfg.try_to_bytes(),
            Err(Error::InvalidValue {
                field: "vpcC FullBox version",
                ..
            })
        ));
    }

    /// `bitDepth` is a 4-bit field: a value above 15 is an error rather than a
    /// shifted-out byte.
    #[test]
    fn bit_depth_over_4_bits_errors() {
        let mut cfg = config(Vec::new());
        cfg.bit_depth = 16;
        assert!(
            matches!(
                cfg.try_to_bytes(),
                Err(Error::FieldOverflow(broadcast_common::len::FieldOverflow {
                    field: "bitDepth",
                    ..
                }))
            ),
            "bitDepth = 16 must be a FieldOverflow"
        );
        // 15 (the largest 4-bit value) still serializes.
        cfg.bit_depth = 15;
        assert!(cfg.try_to_bytes().is_ok());
    }

    /// `chromaSubsampling` is a 3-bit field: an out-of-range value is an error,
    /// not a silently-folded byte (it would otherwise corrupt
    /// `videoFullRangeFlag`).
    #[test]
    fn chroma_subsampling_over_3_bits_errors() {
        let mut cfg = config(Vec::new());
        cfg.chroma_subsampling = 8; // needs 4 bits
        assert!(
            matches!(
                cfg.try_to_bytes(),
                Err(Error::FieldOverflow(broadcast_common::len::FieldOverflow {
                    field: "chromaSubsampling",
                    ..
                }))
            ),
            "chromaSubsampling = 8 must be a FieldOverflow"
        );
        // 7 (the largest 3-bit value) still serializes.
        cfg.chroma_subsampling = 7;
        assert!(cfg.try_to_bytes().is_ok());
    }

    /// Hostile input: a `codecInitializationDataSize` past the box is an error
    /// rather than a silent truncation.
    #[test]
    fn short_init_data_and_truncated_body_are_errors() {
        // v1 record declaring 4 bytes of init data but carrying 2.
        let short: &[u8] = &[
            0x01, 0x00, 0x00, 0x00, 0x00, 0x14, 0x82, 0x02, 0x02, 0x02, 0x00, 0x04, 0xAA, 0xBB,
        ];
        assert!(matches!(
            Vp9ConfigurationBox::parse(short),
            Err(Error::BufferTooShort { .. })
        ));
        // A body too short for the fixed record.
        assert!(Vp9ConfigurationBox::parse(&[0x01, 0x00, 0x00, 0x00, 0x00]).is_err());
    }
}
