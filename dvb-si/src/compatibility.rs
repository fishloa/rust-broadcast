//! Compatibility Descriptor — ETSI TS 102 006 §9.4.2.2 Table 15 / ISO/IEC 13818-6.
//!
//! The `compatibilityDescriptor()` structure describes the hardware/software
//! compatibility requirements of a DSM-CC download group. It is carried in
//! DSI/DII messages (ISO/IEC 13818-6 §7.3) and UNT platform entries
//! (TS 102 006 Table 11).
//!
//! Wire layout (Table 15):
//!
//! ```text
//! compatibilityDescriptor() {
//!   compatibilityDescriptorLength  [15:0]  16 bits  — byte count of everything after this field
//!   descriptorCount                [15:0]  16 bits
//!   for (i < descriptorCount) {
//!     descriptorType                [7:0]   8 bits   — Table 16
//!     descriptorLength              [7:0]   8 bits   — byte count of the rest of THIS descriptor
//!     specifierType                 [7:0]   8 bits   — 0x01 = IEEE OUI
//!     specifierData                 [23:0]  24 bits  — 3-byte IEEE OUI
//!     model                         [15:0]  16 bits
//!     version                       [15:0]  16 bits
//!     subDescriptorCount            [7:0]   8 bits
//!     for (j < subDescriptorCount) {
//!       subDescriptorType           [7:0]   8 bits
//!       subDescriptorLength         [7:0]   8 bits
//!       subDescriptorData           (subDescriptorLength bytes)
//!     }
//!   }
//! }
//! ```
//!
//! An empty descriptor (`descriptorCount == 0`) is encoded as
//! `compatibilityDescriptorLength = 0` (2 bytes on wire: `0x00 0x00`),
//! matching the DSM-CC convention used by existing DVB broadcasts.

use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

pub(crate) const COMPAT_DESC_LEN_FIELD: usize = 2;
const DESC_COUNT_FIELD: usize = 2;
const DESC_HEADER_LEN: usize = 2;
const DESC_FIXED_LEN: usize = 9;
const SUB_DESC_HEADER_LEN: usize = 2;

/// Compatibility descriptor type — TS 102 006 Table 16 / ISO/IEC 13818-6.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum DescriptorType {
    /// 0x00 — pad descriptor.
    Pad,
    /// 0x01 — system hardware descriptor.
    SystemHardware,
    /// 0x02 — system software descriptor.
    SystemSoftware,
    /// 0x03..=0x3F — ISO/IEC 13818-6 reserved.
    IsoReserved(u8),
    /// 0x40..=0x7F — reserved for future use.
    DvbReserved(u8),
    /// 0x80..=0xFF — user defined.
    UserDefined(u8),
}

impl DescriptorType {
    #[must_use]
    /// Decode from the wire value.  Every value maps (lossless).
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x00 => Self::Pad,
            0x01 => Self::SystemHardware,
            0x02 => Self::SystemSoftware,
            v if v < 0x40 => Self::IsoReserved(v),
            v if v < 0x80 => Self::DvbReserved(v),
            v => Self::UserDefined(v),
        }
    }

    #[must_use]
    /// Encode to the wire value.  Inverse of `from_u8` / `from_u16`.
    pub fn to_u8(self) -> u8 {
        match self {
            Self::Pad => 0x00,
            Self::SystemHardware => 0x01,
            Self::SystemSoftware => 0x02,
            Self::IsoReserved(v) | Self::DvbReserved(v) | Self::UserDefined(v) => v,
        }
    }

    #[must_use]
    /// Human-readable spec display name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Pad => "Pad",
            Self::SystemHardware => "System Hardware",
            Self::SystemSoftware => "System Software",
            Self::IsoReserved(_) => "ISO Reserved",
            Self::DvbReserved(_) => "DVB Reserved",
            Self::UserDefined(_) => "User Defined",
        }
    }
}

/// Compatibility specifier type — TS 102 006 Table 15 / ISO/IEC 13818-6.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum SpecifierType {
    /// 0x01 — IEEE OUI.
    IeeeOui,
    /// Catch-all for other / reserved values.
    Unallocated(u8),
}

impl SpecifierType {
    #[must_use]
    /// Decode from the wire value.  Every value maps (lossless).
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x01 => Self::IeeeOui,
            v => Self::Unallocated(v),
        }
    }

    #[must_use]
    /// Encode to the wire value.  Inverse of `from_u8` / `from_u16`.
    pub fn to_u8(self) -> u8 {
        match self {
            Self::IeeeOui => 0x01,
            Self::Unallocated(v) => v,
        }
    }

    #[must_use]
    /// Human-readable spec display name.
    pub fn name(self) -> &'static str {
        match self {
            Self::IeeeOui => "IEEE OUI",
            Self::Unallocated(_) => "Unallocated",
        }
    }
}

/// Compatibility sub-descriptor type — ISO/IEC 13818-6.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum SubDescriptorType {
    /// Catch-all carrying the raw byte value for round-trip fidelity.
    Unallocated(u8),
}

impl SubDescriptorType {
    #[must_use]
    /// Decode from the wire value.  Every value maps (lossless).
    pub fn from_u8(v: u8) -> Self {
        Self::Unallocated(v)
    }

    #[must_use]
    /// Encode to the wire value.  Inverse of `from_u8` / `from_u16`.
    pub fn to_u8(self) -> u8 {
        match self {
            Self::Unallocated(v) => v,
        }
    }

    #[must_use]
    /// Human-readable spec display name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Unallocated(_) => "Unallocated",
        }
    }
}

/// Compatibility Descriptor — ETSI TS 102 006 §9.4.2.2 Table 15 / ISO/IEC
/// 13818-6 `compatibilityDescriptor()`.
///
/// The wire form starts with a 16-bit `compatibilityDescriptorLength` field;
/// [`Parse`] consumes the **full** block including this length prefix, and
/// [`Serialize`] emits it. An empty descriptor (no entries) serialises as the
/// 2-byte `0x00 0x00` length-only form, matching the DSM-CC convention for
/// "no compatibility information".
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CompatibilityDescriptor<'a> {
    /// Descriptor entries (may be empty).
    pub descriptors: Vec<CompatibilityDescriptorEntry<'a>>,
}

/// A single compatibility descriptor entry — TS 102 006 Table 15 / ISO/IEC
/// 13818-6.
///
/// `descriptorType` values are defined in TS 102 006 Table 16
/// (0x00 = pad, 0x01 = system hardware, 0x02 = system software,
/// 0x03–0x3F ISO reserved, 0x40–0x7F reserved, 0x80–0xFF private).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CompatibilityDescriptorEntry<'a> {
    /// `descriptorType` — TS 102 006 Table 16 / ISO/IEC 13818-6.
    pub descriptor_type: DescriptorType,
    /// `specifierType` — `0x01` = IEEE OUI (Table 15 remark).
    pub specifier_type: SpecifierType,
    /// `specifierData` — 3-byte IEEE OUI when `specifierType == 0x01`.
    pub specifier_data: [u8; 3],
    /// `model` — zero if transmitted in a manufacturer private location.
    pub model: u16,
    /// `version` — zero if transmitted in a manufacturer private location.
    pub version: u16,
    /// Sub-descriptor entries.
    pub sub_descriptors: Vec<SubDescriptor<'a>>,
}

/// A sub-descriptor within a compatibility descriptor entry — ISO/IEC 13818-6.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SubDescriptor<'a> {
    /// `subDescriptorType`.
    pub sub_descriptor_type: SubDescriptorType,
    /// `subDescriptorData`.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub data: &'a [u8],
}

fn entry_serialized_len(entry: &CompatibilityDescriptorEntry) -> usize {
    DESC_HEADER_LEN
        + DESC_FIXED_LEN
        + entry
            .sub_descriptors
            .iter()
            .map(|sd| SUB_DESC_HEADER_LEN + sd.data.len())
            .sum::<usize>()
}

impl<'a> Parse<'a> for CompatibilityDescriptor<'a> {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < COMPAT_DESC_LEN_FIELD {
            return Err(Error::BufferTooShort {
                need: COMPAT_DESC_LEN_FIELD,
                have: bytes.len(),
                what: "CompatibilityDescriptor length field",
            });
        }
        let compat_desc_len = u16::from_be_bytes([bytes[0], bytes[1]]) as usize;
        let body_end = COMPAT_DESC_LEN_FIELD + compat_desc_len;
        if body_end > bytes.len() {
            return Err(Error::SectionLengthOverflow {
                declared: compat_desc_len,
                available: bytes.len() - COMPAT_DESC_LEN_FIELD,
            });
        }
        if compat_desc_len == 0 {
            return Ok(CompatibilityDescriptor {
                descriptors: Vec::new(),
            });
        }
        if compat_desc_len < DESC_COUNT_FIELD {
            return Err(Error::BufferTooShort {
                need: COMPAT_DESC_LEN_FIELD + DESC_COUNT_FIELD,
                have: bytes.len(),
                what: "CompatibilityDescriptor descriptorCount",
            });
        }
        let body = &bytes[COMPAT_DESC_LEN_FIELD..body_end];
        let descriptor_count = u16::from_be_bytes([body[0], body[1]]) as usize;
        let mut pos = DESC_COUNT_FIELD;
        let max_entries = (body.len() - DESC_COUNT_FIELD) / (DESC_HEADER_LEN + DESC_FIXED_LEN);
        let mut descriptors = Vec::with_capacity(descriptor_count.min(max_entries));
        for _ in 0..descriptor_count {
            if pos + DESC_HEADER_LEN > body.len() {
                return Err(Error::BufferTooShort {
                    need: COMPAT_DESC_LEN_FIELD + pos + DESC_HEADER_LEN,
                    have: COMPAT_DESC_LEN_FIELD + body.len(),
                    what: "CompatibilityDescriptor entry header",
                });
            }
            let descriptor_type = DescriptorType::from_u8(body[pos]);
            let descriptor_length = body[pos + 1] as usize;
            let entry_end = pos + DESC_HEADER_LEN + descriptor_length;
            if entry_end > body.len() {
                return Err(Error::SectionLengthOverflow {
                    declared: descriptor_length,
                    available: body.len() - pos - DESC_HEADER_LEN,
                });
            }
            if descriptor_length < DESC_FIXED_LEN {
                return Err(Error::InvalidDescriptor {
                    tag: descriptor_type.to_u8(),
                    reason: "descriptorLength shorter than fixed fields",
                });
            }
            let specifier_type = SpecifierType::from_u8(body[pos + DESC_HEADER_LEN]);
            let specifier_data = [
                body[pos + DESC_HEADER_LEN + 1],
                body[pos + DESC_HEADER_LEN + 2],
                body[pos + DESC_HEADER_LEN + 3],
            ];
            let model = u16::from_be_bytes([
                body[pos + DESC_HEADER_LEN + 4],
                body[pos + DESC_HEADER_LEN + 5],
            ]);
            let version = u16::from_be_bytes([
                body[pos + DESC_HEADER_LEN + 6],
                body[pos + DESC_HEADER_LEN + 7],
            ]);
            let sub_descriptor_count = body[pos + DESC_HEADER_LEN + 8] as usize;
            let sub_desc_start = pos + DESC_HEADER_LEN + DESC_FIXED_LEN;
            let sub_desc_end = entry_end;
            let sub_desc_region_len = sub_desc_end.saturating_sub(sub_desc_start);
            let mut sub_descriptors = Vec::with_capacity(
                sub_descriptor_count.min(sub_desc_region_len / SUB_DESC_HEADER_LEN),
            );
            let mut sub_pos = sub_desc_start;
            for _ in 0..sub_descriptor_count {
                if sub_pos + SUB_DESC_HEADER_LEN > sub_desc_end {
                    return Err(Error::BufferTooShort {
                        need: COMPAT_DESC_LEN_FIELD + sub_pos + SUB_DESC_HEADER_LEN,
                        have: COMPAT_DESC_LEN_FIELD + sub_desc_end,
                        what: "CompatibilityDescriptor subDescriptor header",
                    });
                }
                let sub_descriptor_type = SubDescriptorType::from_u8(body[sub_pos]);
                let sub_descriptor_length = body[sub_pos + 1] as usize;
                sub_pos += SUB_DESC_HEADER_LEN;
                if sub_pos + sub_descriptor_length > sub_desc_end {
                    return Err(Error::SectionLengthOverflow {
                        declared: sub_descriptor_length,
                        available: sub_desc_end - sub_pos,
                    });
                }
                sub_descriptors.push(SubDescriptor {
                    sub_descriptor_type,
                    data: &body[sub_pos..sub_pos + sub_descriptor_length],
                });
                sub_pos += sub_descriptor_length;
            }
            pos = entry_end;
            descriptors.push(CompatibilityDescriptorEntry {
                descriptor_type,
                specifier_type,
                specifier_data,
                model,
                version,
                sub_descriptors,
            });
        }
        // Reject slack inside compatibilityDescriptorLength — leftover bytes
        // would be silently dropped and lost on re-serialize.
        if pos != body.len() {
            return Err(Error::InvalidDescriptor {
                tag: 0,
                reason: "trailing bytes after compatibility descriptor entries",
            });
        }
        Ok(CompatibilityDescriptor { descriptors })
    }
}

impl Serialize for CompatibilityDescriptor<'_> {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        if self.descriptors.is_empty() {
            return COMPAT_DESC_LEN_FIELD;
        }
        COMPAT_DESC_LEN_FIELD
            + DESC_COUNT_FIELD
            + self
                .descriptors
                .iter()
                .map(entry_serialized_len)
                .sum::<usize>()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        if self.descriptors.is_empty() {
            buf[0] = 0x00;
            buf[1] = 0x00;
            return Ok(COMPAT_DESC_LEN_FIELD);
        }
        let body_len = len - COMPAT_DESC_LEN_FIELD;
        if body_len > u16::MAX as usize {
            return Err(Error::SectionLengthOverflow {
                declared: body_len,
                available: u16::MAX as usize,
            });
        }
        if self.descriptors.len() > u16::MAX as usize {
            return Err(Error::SectionLengthOverflow {
                declared: self.descriptors.len(),
                available: u16::MAX as usize,
            });
        }
        buf[..COMPAT_DESC_LEN_FIELD].copy_from_slice(&(body_len as u16).to_be_bytes());
        buf[COMPAT_DESC_LEN_FIELD..COMPAT_DESC_LEN_FIELD + DESC_COUNT_FIELD]
            .copy_from_slice(&(self.descriptors.len() as u16).to_be_bytes());
        let mut pos = COMPAT_DESC_LEN_FIELD + DESC_COUNT_FIELD;
        for entry in &self.descriptors {
            let entry_body_len = entry_serialized_len(entry) - DESC_HEADER_LEN;
            if entry_body_len > u8::MAX as usize {
                return Err(Error::SectionLengthOverflow {
                    declared: entry_body_len,
                    available: u8::MAX as usize,
                });
            }
            buf[pos] = entry.descriptor_type.to_u8();
            buf[pos + 1] = entry_body_len as u8;
            buf[pos + DESC_HEADER_LEN] = entry.specifier_type.to_u8();
            buf[pos + DESC_HEADER_LEN + 1..pos + DESC_HEADER_LEN + 4]
                .copy_from_slice(&entry.specifier_data);
            buf[pos + DESC_HEADER_LEN + 4..pos + DESC_HEADER_LEN + 6]
                .copy_from_slice(&entry.model.to_be_bytes());
            buf[pos + DESC_HEADER_LEN + 6..pos + DESC_HEADER_LEN + 8]
                .copy_from_slice(&entry.version.to_be_bytes());
            if entry.sub_descriptors.len() > u8::MAX as usize {
                return Err(Error::SectionLengthOverflow {
                    declared: entry.sub_descriptors.len(),
                    available: u8::MAX as usize,
                });
            }
            buf[pos + DESC_HEADER_LEN + 8] = entry.sub_descriptors.len() as u8;
            pos += DESC_HEADER_LEN + DESC_FIXED_LEN;
            for sd in &entry.sub_descriptors {
                buf[pos] = sd.sub_descriptor_type.to_u8();
                if sd.data.len() > u8::MAX as usize {
                    return Err(Error::SectionLengthOverflow {
                        declared: sd.data.len(),
                        available: u8::MAX as usize,
                    });
                }
                buf[pos + 1] = sd.data.len() as u8;
                pos += SUB_DESC_HEADER_LEN;
                buf[pos..pos + sd.data.len()].copy_from_slice(sd.data);
                pos += sd.data.len();
            }
        }
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_round_trip() {
        let cd = CompatibilityDescriptor {
            descriptors: vec![],
        };
        let mut buf = vec![0u8; cd.serialized_len()];
        cd.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, &[0x00, 0x00]);
        let re = CompatibilityDescriptor::parse(&buf).unwrap();
        assert!(re.descriptors.is_empty());
    }

    #[test]
    fn empty_with_count_parses_to_empty() {
        let bytes: &[u8] = &[0x00, 0x02, 0x00, 0x00];
        let cd = CompatibilityDescriptor::parse(bytes).unwrap();
        assert!(cd.descriptors.is_empty());
        let mut buf = vec![0u8; cd.serialized_len()];
        cd.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, &[0x00, 0x00]);
    }

    /// Hand-built wire bytes (not serializer-derived) pinning every field
    /// position against TS 102 006 Table 15 — catches a mirrored read/write
    /// layout bug that serializer-built round-trips cannot.
    #[test]
    fn hand_built_byte_anchor() {
        // len=0x11(17), count=1; entry: type=0x01 len=0x0D(13),
        // specifierType=0x01, OUI 00:15:0A, model 0x1234, version 0x0001,
        // subCount=1, sub: type=0x05 len=0x02 data AA BB.
        let bytes: &[u8] = &[
            0x00, 0x11, 0x00, 0x01, 0x01, 0x0D, 0x01, 0x00, 0x15, 0x0A, 0x12, 0x34, 0x00, 0x01,
            0x01, 0x05, 0x02, 0xAA, 0xBB,
        ];
        let cd = CompatibilityDescriptor::parse(bytes).unwrap();
        assert_eq!(cd.descriptors.len(), 1);
        let e = &cd.descriptors[0];
        assert_eq!(e.descriptor_type, DescriptorType::SystemHardware);
        assert_eq!(e.specifier_type, SpecifierType::IeeeOui);
        assert_eq!(e.specifier_data, [0x00, 0x15, 0x0A]);
        assert_eq!(e.model, 0x1234);
        assert_eq!(e.version, 0x0001);
        assert_eq!(e.sub_descriptors.len(), 1);
        assert_eq!(
            e.sub_descriptors[0].sub_descriptor_type,
            SubDescriptorType::Unallocated(0x05)
        );
        assert_eq!(e.sub_descriptors[0].data, &[0xAA, 0xBB]);
        // Byte-identical re-serialize against the hand-built wire.
        let mut buf = vec![0u8; cd.serialized_len()];
        cd.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, bytes);
    }

    #[test]
    fn rejects_trailing_bytes() {
        // len=0x03: count=0 (2 bytes) + 1 slack byte inside the declared length.
        let bytes: &[u8] = &[0x00, 0x03, 0x00, 0x00, 0xFF];
        assert!(matches!(
            CompatibilityDescriptor::parse(bytes).unwrap_err(),
            Error::InvalidDescriptor { .. }
        ));
    }

    #[test]
    fn rejects_truncated_entry_header() {
        // count=1 but only 1 byte of the 2-byte entry header present.
        let bytes: &[u8] = &[0x00, 0x03, 0x00, 0x01, 0x01];
        assert!(CompatibilityDescriptor::parse(bytes).is_err());
    }

    #[test]
    fn rejects_truncated_sub_descriptor() {
        // descriptorLength=9 (fixed fields only) but subCount=1 → no room for
        // the sub-descriptor header inside the entry.
        let bytes: &[u8] = &[
            0x00, 0x0D, 0x00, 0x01, 0x01, 0x09, 0x01, 0x00, 0x15, 0x0A, 0x12, 0x34, 0x00, 0x01,
            0x01,
        ];
        assert!(CompatibilityDescriptor::parse(bytes).is_err());
    }

    #[test]
    fn one_descriptor_with_sub_round_trip() {
        let cd = CompatibilityDescriptor {
            descriptors: vec![CompatibilityDescriptorEntry {
                descriptor_type: DescriptorType::SystemHardware,
                specifier_type: SpecifierType::IeeeOui,
                specifier_data: [0x00, 0x15, 0x0A],
                model: 0x1234,
                version: 0x0001,
                sub_descriptors: vec![
                    SubDescriptor {
                        sub_descriptor_type: SubDescriptorType::Unallocated(0x01),
                        data: &[0xAA, 0xBB],
                    },
                    SubDescriptor {
                        sub_descriptor_type: SubDescriptorType::Unallocated(0x02),
                        data: &[0xCC],
                    },
                ],
            }],
        };
        let mut buf = vec![0u8; cd.serialized_len()];
        cd.serialize_into(&mut buf).unwrap();
        let re = CompatibilityDescriptor::parse(&buf).unwrap();
        assert_eq!(re.descriptors.len(), 1);
        let e = &re.descriptors[0];
        assert_eq!(e.descriptor_type, DescriptorType::SystemHardware);
        assert_eq!(e.specifier_type, SpecifierType::IeeeOui);
        assert_eq!(e.specifier_data, [0x00, 0x15, 0x0A]);
        assert_eq!(e.model, 0x1234);
        assert_eq!(e.version, 0x0001);
        assert_eq!(e.sub_descriptors.len(), 2);
        assert_eq!(
            e.sub_descriptors[0].sub_descriptor_type,
            SubDescriptorType::Unallocated(0x01)
        );
        assert_eq!(e.sub_descriptors[0].data, &[0xAA, 0xBB]);
        assert_eq!(
            e.sub_descriptors[1].sub_descriptor_type,
            SubDescriptorType::Unallocated(0x02)
        );
        assert_eq!(e.sub_descriptors[1].data, &[0xCC]);
        let mut buf2 = vec![0u8; cd.serialized_len()];
        cd.serialize_into(&mut buf2).unwrap();
        assert_eq!(buf, buf2, "byte-exact re-serialize");
        assert_eq!(re, cd);
    }

    #[test]
    fn two_descriptors_round_trip() {
        let cd = CompatibilityDescriptor {
            descriptors: vec![
                CompatibilityDescriptorEntry {
                    descriptor_type: DescriptorType::SystemHardware,
                    specifier_type: SpecifierType::IeeeOui,
                    specifier_data: [0x00, 0x00, 0x00],
                    model: 0x0000,
                    version: 0x0000,
                    sub_descriptors: vec![],
                },
                CompatibilityDescriptorEntry {
                    descriptor_type: DescriptorType::SystemSoftware,
                    specifier_type: SpecifierType::IeeeOui,
                    specifier_data: [0x00, 0x15, 0x5A],
                    model: 0x0100,
                    version: 0x0002,
                    sub_descriptors: vec![SubDescriptor {
                        sub_descriptor_type: SubDescriptorType::Unallocated(0x80),
                        data: &[0xDE, 0xAD, 0xBE, 0xEF],
                    }],
                },
            ],
        };
        let mut buf = vec![0u8; cd.serialized_len()];
        cd.serialize_into(&mut buf).unwrap();
        let re = CompatibilityDescriptor::parse(&buf).unwrap();
        assert_eq!(re, cd);
    }

    #[test]
    fn parse_rejects_short_buffer() {
        assert!(matches!(
            CompatibilityDescriptor::parse(&[0x00]).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn parse_rejects_truncated_body() {
        assert!(matches!(
            CompatibilityDescriptor::parse(&[0x00, 0x05, 0x00, 0x01]).unwrap_err(),
            Error::SectionLengthOverflow { .. }
        ));
    }

    #[test]
    fn parse_rejects_descriptor_length_too_short() {
        let bytes: &[u8] = &[
            0x00, 0x06, // compatibilityDescriptorLength = 6
            0x00, 0x01, // descriptorCount = 1
            0x01, 0x02, // descriptorType=1, descriptorLength=2 (too short, need 9)
            0xAA, 0xBB, // 2 bytes of entry body (to satisfy entry_end bounds)
        ];
        assert!(matches!(
            CompatibilityDescriptor::parse(bytes).unwrap_err(),
            Error::InvalidDescriptor { .. }
        ));
    }

    #[test]
    fn serialize_rejects_small_buffer() {
        let cd = CompatibilityDescriptor {
            descriptors: vec![],
        };
        assert!(matches!(
            cd.serialize_into(&mut [0u8; 1]).unwrap_err(),
            Error::OutputBufferTooSmall { .. }
        ));
    }
}
