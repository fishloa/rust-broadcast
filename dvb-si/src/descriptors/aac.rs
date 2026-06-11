//! AAC Descriptor — ETSI EN 300 468 Annex H, Table H.1 (tag 0x7C).
//!
//! Carried in the PMT ES_info loop to identify MPEG-4 AAC / HE-AAC / HE-AAC v2
//! audio. Per the SI PDF (etsi_en_300_468_v01.19.01, Annex H §H.2.1, Table H.1,
//! PDF pp. 196-197) the body is:
//!   profile_and_level(8)
//!   if (descriptor_length > 1) {
//!     AAC_type_flag(1) + SAOC_DE_flag(1) + reserved_zero_future_use(6)
//!     if (AAC_type_flag == 1) AAC_type(8)
//!     additional_info_byte(8*N)
//!   }
//! The optional block (everything after profile_and_level) is modelled as an
//! `Option<AacExtension>`: `None` means descriptor_length == 1.

use super::descriptor_body;
use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// Descriptor tag for AAC_descriptor.
pub const TAG: u8 = 0x7C;
const HEADER_LEN: usize = 2;

const FLAG_AAC_TYPE: u8 = 0x80;
const FLAG_SAOC_DE: u8 = 0x40;
/// reserved_zero_future_use(6) — the spec mandates these are zero.
const RESERVED_ZERO_MASK: u8 = 0x3F;

/// Decoded AAC component_type — ETSI EN 300 468 Table 26 / Annex H.
///
/// Returned by [`AacExtension::decoded_component_type`] when `aac_type` is
/// `Some`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AacComponentType {
    /// Single mono channel.
    pub mono: bool,
    /// Stereo.
    pub stereo: bool,
    /// Surround sound (> 2 channels).
    pub surround: bool,
    /// Audio description for the visually impaired.
    pub visually_impaired: bool,
    /// Audio for the hard of hearing.
    pub hard_of_hearing: bool,
    /// Supplementary audio (receiver-mix or broadcast-mix).
    pub supplementary: bool,
    /// HE-AAC v2 (SBR + PS) rather than HE-AAC v1.
    pub v2: bool,
    /// SAOC-DE ancillary data present.
    pub saoc_de: bool,
}

impl AacComponentType {
    /// Returns a human-readable description of the AAC component type.
    #[must_use]
    /// Returns a human-readable name.
    pub fn name(&self) -> &'static str {
        match (
            self.mono,
            self.stereo,
            self.surround,
            self.visually_impaired,
            self.hard_of_hearing,
            self.supplementary,
            self.v2,
            self.saoc_de,
        ) {
            (true, false, false, false, false, false, false, false) => {
                "HE-AAC audio, single mono channel"
            }
            (false, true, false, false, false, false, false, false) => "HE-AAC audio, stereo",
            (false, false, true, false, false, false, false, false) => {
                "HE-AAC audio, surround sound"
            }
            (_, _, _, true, false, false, false, false) => {
                "HE-AAC audio description for the visually impaired"
            }
            (_, _, _, false, true, false, false, false) => "HE-AAC audio for the hard of hearing",
            (_, _, _, false, false, true, false, false) => {
                "HE-AAC receiver-mix supplementary audio"
            }
            (false, true, false, false, false, false, true, false) => "HE-AAC v2 audio, stereo",
            (_, _, _, true, false, false, true, false) => {
                "HE-AAC v2 audio description for the visually impaired"
            }
            (_, _, _, false, true, false, true, false) => "HE-AAC v2 audio for the hard of hearing",
            (_, _, _, false, false, true, true, false) => {
                "HE-AAC v2 receiver-mix supplementary audio"
            }
            (_, _, _, true, false, true, false, false) => {
                "HE-AAC receiver-mix audio description for the visually impaired"
            }
            (_, _, _, true, false, true, true, false) => {
                "HE-AAC v2 receiver-mix audio description for the visually impaired"
            }
            (_, _, _, _, _, _, _, true) => "HE-AAC or HE-AAC v2 with SAOC-DE ancillary data",
            _ => "unknown HE-AAC component type",
        }
    }
}

/// Optional extension carried when descriptor_length > 1.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct AacExtension<'a> {
    /// SAOC_DE_flag — embedded SAOC-DE parametric data present (Table H.2).
    pub saoc_de_flag: bool,
    /// AAC_type — component_type when stream_content is 0x06 (Table 26).
    /// `Some` iff AAC_type_flag was set.
    pub aac_type: Option<u8>,
    /// Trailing additional_info bytes.
    pub additional_info: &'a [u8],
}

impl AacExtension<'_> {
    /// Decodes the `aac_type` field (when present) into a structured
    /// [`AacComponentType`] per ETSI EN 300 468 Table 26 / Annex H.
    ///
    /// Returns `None` when `aac_type` is `None` or when the value is not
    /// a recognised Table 26 component_type for AAC (stream_content 0x06).
    #[must_use]
    pub fn decoded_component_type(&self) -> Option<AacComponentType> {
        let ct = self.aac_type?;
        let saoc_de = self.saoc_de_flag;
        if ct == 0xA0 {
            return Some(AacComponentType {
                mono: false,
                stereo: false,
                surround: false,
                visually_impaired: false,
                hard_of_hearing: false,
                supplementary: false,
                v2: false,
                saoc_de: true,
            });
        }
        let v2 = matches!(ct, 0x43..=0x46 | 0x49..=0x4A);
        let result = match ct {
            0x01 => AacComponentType {
                mono: true,
                stereo: false,
                surround: false,
                visually_impaired: false,
                hard_of_hearing: false,
                supplementary: false,
                v2: false,
                saoc_de,
            },
            0x03 => AacComponentType {
                mono: false,
                stereo: true,
                surround: false,
                visually_impaired: false,
                hard_of_hearing: false,
                supplementary: false,
                v2: false,
                saoc_de,
            },
            0x05 => AacComponentType {
                mono: false,
                stereo: false,
                surround: true,
                visually_impaired: false,
                hard_of_hearing: false,
                supplementary: false,
                v2: false,
                saoc_de,
            },
            0x40 | 0x47 | 0x48 => AacComponentType {
                mono: false,
                stereo: false,
                surround: false,
                visually_impaired: true,
                hard_of_hearing: false,
                supplementary: matches!(ct, 0x47 | 0x48),
                v2: false,
                saoc_de,
            },
            0x41 => AacComponentType {
                mono: false,
                stereo: false,
                surround: false,
                visually_impaired: false,
                hard_of_hearing: true,
                supplementary: false,
                v2: false,
                saoc_de,
            },
            0x42 => AacComponentType {
                mono: false,
                stereo: false,
                surround: false,
                visually_impaired: false,
                hard_of_hearing: false,
                supplementary: true,
                v2: false,
                saoc_de,
            },
            0x43 => AacComponentType {
                mono: false,
                stereo: true,
                surround: false,
                visually_impaired: false,
                hard_of_hearing: false,
                supplementary: false,
                v2: true,
                saoc_de,
            },
            0x44 | 0x49 | 0x4A => AacComponentType {
                mono: false,
                stereo: false,
                surround: false,
                visually_impaired: true,
                hard_of_hearing: false,
                supplementary: matches!(ct, 0x49 | 0x4A),
                v2,
                saoc_de,
            },
            0x45 => AacComponentType {
                mono: false,
                stereo: false,
                surround: false,
                visually_impaired: false,
                hard_of_hearing: true,
                supplementary: false,
                v2: true,
                saoc_de,
            },
            0x46 => AacComponentType {
                mono: false,
                stereo: false,
                surround: false,
                visually_impaired: false,
                hard_of_hearing: false,
                supplementary: true,
                v2: true,
                saoc_de,
            },
            _ => return None,
        };
        Some(result)
    }
}

/// AAC Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct AacDescriptor<'a> {
    /// 8-bit profile_and_level (MPEG-4_audio_profile_and_level).
    pub profile_and_level: u8,
    /// Optional extension; `None` means the body was a single byte.
    pub extension: Option<AacExtension<'a>>,
}

impl<'a> Parse<'a> for AacDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN + 1 {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN + 1,
                have: bytes.len(),
                what: "AacDescriptor header+profile",
            });
        }
        let body = descriptor_body(
            bytes,
            TAG,
            "AacDescriptor",
            "unexpected tag for AAC_descriptor",
        )?;
        if body.is_empty() {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "AAC_descriptor body shorter than 1 byte",
            });
        }
        let profile_and_level = body[0];
        let extension = if body.len() > 1 {
            let flags = body[1];
            let aac_type_flag = (flags & FLAG_AAC_TYPE) != 0;
            let saoc_de_flag = (flags & FLAG_SAOC_DE) != 0;
            let mut pos = 2;
            let aac_type = if aac_type_flag {
                if pos >= body.len() {
                    return Err(Error::InvalidDescriptor {
                        tag: TAG,
                        reason: "AAC_type_flag set but AAC_type byte missing",
                    });
                }
                let t = body[pos];
                pos += 1;
                Some(t)
            } else {
                None
            };
            let additional_info = &body[pos..];
            Some(AacExtension {
                saoc_de_flag,
                aac_type,
                additional_info,
            })
        } else {
            None
        };
        Ok(Self {
            profile_and_level,
            extension,
        })
    }
}

impl Serialize for AacDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        let body = 1 + match &self.extension {
            None => 0,
            Some(ext) => 1 + usize::from(ext.aac_type.is_some()) + ext.additional_info.len(),
        };
        HEADER_LEN + body
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body_len = self.serialized_len() - HEADER_LEN;
        if body_len > u8::MAX as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "AAC_descriptor body exceeds 255 bytes",
            });
        }
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = TAG;
        buf[1] = body_len as u8;
        buf[2] = self.profile_and_level;
        let mut pos = 3;
        if let Some(ext) = &self.extension {
            let mut flags = 0u8;
            if ext.aac_type.is_some() {
                flags |= FLAG_AAC_TYPE;
            }
            if ext.saoc_de_flag {
                flags |= FLAG_SAOC_DE;
            }
            // reserved_zero_future_use(6) emitted as zeros per spec.
            buf[pos] = flags & !RESERVED_ZERO_MASK;
            pos += 1;
            if let Some(t) = ext.aac_type {
                buf[pos] = t;
                pos += 1;
            }
            buf[pos..pos + ext.additional_info.len()].copy_from_slice(ext.additional_info);
        }
        Ok(len)
    }
}
impl<'a> crate::traits::DescriptorDef<'a> for AacDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "AAC";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_profile_only() {
        let bytes = [TAG, 1, 0x50];
        let d = AacDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.profile_and_level, 0x50);
        assert!(d.extension.is_none());
    }

    #[test]
    fn parse_with_flags_no_aac_type() {
        // flags: SAOC_DE set, AAC_type clear.
        let bytes = [TAG, 2, 0x51, FLAG_SAOC_DE];
        let d = AacDescriptor::parse(&bytes).unwrap();
        let ext = d.extension.unwrap();
        assert!(ext.saoc_de_flag);
        assert!(ext.aac_type.is_none());
        assert!(ext.additional_info.is_empty());
    }

    #[test]
    fn parse_with_aac_type() {
        let bytes = [TAG, 3, 0x52, FLAG_AAC_TYPE, 0x03];
        let d = AacDescriptor::parse(&bytes).unwrap();
        let ext = d.extension.unwrap();
        assert!(!ext.saoc_de_flag);
        assert_eq!(ext.aac_type, Some(0x03));
        assert!(ext.additional_info.is_empty());
    }

    #[test]
    fn parse_with_aac_type_and_additional_info() {
        let bytes = [TAG, 5, 0x52, FLAG_AAC_TYPE | FLAG_SAOC_DE, 0x05, 0xAA, 0xBB];
        let d = AacDescriptor::parse(&bytes).unwrap();
        let ext = d.extension.unwrap();
        assert!(ext.saoc_de_flag);
        assert_eq!(ext.aac_type, Some(0x05));
        assert_eq!(ext.additional_info, &[0xAA, 0xBB]);
    }

    #[test]
    fn decode_aac_component_type_he_aac_stereo() {
        let d = AacDescriptor {
            profile_and_level: 0x50,
            extension: Some(AacExtension {
                saoc_de_flag: false,
                aac_type: Some(0x03),
                additional_info: &[],
            }),
        };
        let ct = d.extension.unwrap().decoded_component_type().unwrap();
        assert!(!ct.mono);
        assert!(ct.stereo);
        assert!(!ct.surround);
        assert_eq!(ct.name(), "HE-AAC audio, stereo");
    }

    #[test]
    fn decode_aac_component_type_saoc_de() {
        let d = AacDescriptor {
            profile_and_level: 0x50,
            extension: Some(AacExtension {
                saoc_de_flag: true,
                aac_type: Some(0xA0),
                additional_info: &[],
            }),
        };
        let ct = d.extension.unwrap().decoded_component_type().unwrap();
        assert!(ct.saoc_de);
        assert_eq!(ct.name(), "HE-AAC or HE-AAC v2 with SAOC-DE ancillary data");
    }

    #[test]
    fn decode_aac_component_type_unknown() {
        let d = AacDescriptor {
            profile_and_level: 0x50,
            extension: Some(AacExtension {
                saoc_de_flag: false,
                aac_type: Some(0xFF),
                additional_info: &[],
            }),
        };
        assert!(d.extension.unwrap().decoded_component_type().is_none());
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let bytes = [0x7B, 1, 0x50];
        assert!(matches!(
            AacDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x7B, .. }
        ));
    }

    #[test]
    fn parse_rejects_aac_type_flag_without_byte() {
        // length=2 covers profile + flags only, but AAC_type_flag claims a byte.
        let bytes = [TAG, 2, 0x50, FLAG_AAC_TYPE];
        assert!(matches!(
            AacDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { .. }
        ));
    }

    #[test]
    fn parse_rejects_length_overrunning_buffer() {
        let bytes = [TAG, 4, 0x50];
        assert!(matches!(
            AacDescriptor::parse(&bytes).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn serialize_round_trip_profile_only() {
        let d = AacDescriptor {
            profile_and_level: 0x58,
            extension: None,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, vec![TAG, 1, 0x58]);
        assert_eq!(AacDescriptor::parse(&buf).unwrap(), d);
    }

    #[test]
    fn serialize_round_trip_full() {
        let d = AacDescriptor {
            profile_and_level: 0x52,
            extension: Some(AacExtension {
                saoc_de_flag: true,
                aac_type: Some(0x40),
                additional_info: &[0xFE, 0xED],
            }),
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(AacDescriptor::parse(&buf).unwrap(), d);
    }

    #[test]
    fn serialize_emits_reserved_bits_zero() {
        let d = AacDescriptor {
            profile_and_level: 0x50,
            extension: Some(AacExtension {
                saoc_de_flag: false,
                aac_type: None,
                additional_info: &[],
            }),
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        // flags byte: no flags set, reserved zeros => 0x00.
        assert_eq!(buf[3] & RESERVED_ZERO_MASK, 0);
        assert_eq!(buf[3], 0x00);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_serializes_to_stable_json() {
        // Borrowed `&[u8]` cannot deserialize from a JSON number array, so we
        // assert the Serialize impl is wired and emits stable JSON.
        let d = AacDescriptor {
            profile_and_level: 0x52,
            extension: Some(AacExtension {
                saoc_de_flag: true,
                aac_type: Some(0x03),
                additional_info: &[0x11],
            }),
        };
        let j = serde_json::to_string(&d).unwrap();
        // Valid, re-parseable JSON (key order is map-defined, so we do not
        // assert byte-for-byte string stability).
        let _v: serde_json::Value = serde_json::from_str(&j).unwrap();
        assert!(j.contains("profile_and_level"));
    }
}
