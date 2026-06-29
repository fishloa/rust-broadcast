//! segmentation_descriptor() — ANSI/SCTE 35 2023r1 §10.3.3, Table 20 (tag 0x02).
//!
//! The richest splice descriptor: carries a segmentation event, its
//! restriction flags, an optional 40-bit `segmentation_duration`, a
//! type/length-prefixed `segmentation_upid()`, the
//! [`SegmentationTypeId`], and an
//! optional `sub_segment_num`/`sub_segments_expected` appendix whose presence
//! is determined by `descriptor_length` (§10.3.3.1).
//!
//! Component Segmentation Mode (`program_segmentation_flag == 0`) is deprecated
//! but parsed/serialized losslessly via [`SegmentationDescriptor::components`].
//!
//! Two UPID types carry sub-structure that is decoded on demand:
//!
//! - **MPU()** — §10.3.3.3, Table 24: `format_identifier` (32-bit) + `private_data`.
//!   Access via [`SegmentationDescriptor::mpu`].
//! - **MID()** — §10.3.3.4, Table 25: a sequence of `{ type, length, upid }` entries.
//!   Access via [`SegmentationDescriptor::mid`].

use alloc::vec::Vec;

use super::header::{self, CUEI, HEADER_LEN};
use super::segmentation_enums::{DeviceRestrictions, SegmentationTypeId, SegmentationUpidType};
use crate::error::{Error, Result};
use crate::traits::SpliceDescriptorDef;
use broadcast_common::{Parse, Serialize};

/// Width in bits (and bytes) of the `format_identifier` field in MPU() (§10.3.3.3, Table 24).
const MPU_FORMAT_IDENTIFIER_LEN: usize = 4; // 32 bits / 8

/// Minimum byte length of an MID() sub-entry header: `segmentation_upid_type` (1) + `length` (1).
const MID_ENTRY_HEADER_LEN: usize = 2;

/// Decoded view of a Managed Private UPID (MPU()) — §10.3.3.3, Table 24.
///
/// `segmentation_upid_type` 0x0C. The `segmentation_upid()` bytes begin with a
/// 32-bit `format_identifier` (uimsbf, registered with SMPTE) followed by
/// `private_data` whose length is `segmentation_upid_length − 4` (the
/// `format_identifier` is counted in the declared length).
///
/// This is a **derived view** of the raw `segmentation_upid` bytes already held
/// by the parent [`SegmentationDescriptor`]; it borrows from the same slice and
/// does not affect round-trip serialization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Mpu<'a> {
    /// 32-bit `format_identifier` (uimsbf), registered with SMPTE
    /// (ISO/IEC 13818-1 §2.6.8).
    pub format_identifier: u32,
    /// Remaining UPID bytes after the `format_identifier`; length =
    /// `segmentation_upid_length − 4`.
    pub private_data: &'a [u8],
}

/// One entry in a Multiple UPID (MID()) structure — §10.3.3.4, Table 25.
///
/// Each entry carries its own `segmentation_upid_type` (from Table 22) and the
/// corresponding raw UPID bytes. A MID sub-entry must not itself be of type MID.
///
/// This is a **derived view** borrowed from the parent descriptor's
/// `segmentation_upid` slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MidUpid<'a> {
    /// `segmentation_upid_type` for this sub-entry (Table 22).
    pub upid_type: SegmentationUpidType,
    /// Raw UPID bytes for this sub-entry (length = the sub-entry's `length` field).
    pub upid: &'a [u8],
}

/// `splice_descriptor_tag` for segmentation_descriptor (§10.1, Table 16).
pub const TAG: u8 = 0x02;

/// Delivery restriction flags, present when `delivery_not_restricted_flag == 0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DeliveryRestrictions {
    /// `web_delivery_allowed_flag`.
    pub web_delivery_allowed: bool,
    /// `no_regional_blackout_flag`.
    pub no_regional_blackout: bool,
    /// `archive_allowed_flag`.
    pub archive_allowed: bool,
    /// `device_restrictions` (Table 21).
    pub device_restrictions: DeviceRestrictions,
}

/// One component entry in the deprecated Component Segmentation Mode loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SegmentationComponent {
    /// 8-bit `component_tag`.
    pub component_tag: u8,
    /// 33-bit `pts_offset` (90 kHz ticks).
    pub pts_offset: u64,
}

/// segmentation_descriptor() — §10.3.3, Table 20.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SegmentationDescriptor<'a> {
    /// 32-bit `identifier` (shall be "CUEI").
    pub identifier: u32,
    /// 32-bit `segmentation_event_id` (§9.9.3).
    pub segmentation_event_id: u32,
    /// When `true`, the named event has been cancelled and no further fields
    /// are present.
    pub segmentation_event_cancel_indicator: bool,
    /// `segmentation_event_id_compliance_indicator`: `false` = compliant.
    pub segmentation_event_id_compliance_indicator: bool,
    /// `program_segmentation_flag`: `true` = Program mode (supported);
    /// `false` = Component mode (deprecated).
    pub program_segmentation_flag: bool,
    /// Delivery restrictions, present when `delivery_not_restricted_flag == 0`;
    /// `None` means delivery is not restricted.
    pub delivery_restrictions: Option<DeliveryRestrictions>,
    /// Component-mode entries, present when `program_segmentation_flag == 0`.
    pub components: Vec<SegmentationComponent>,
    /// 40-bit `segmentation_duration` (90 kHz ticks), present when
    /// `segmentation_duration_flag == 1`.
    pub segmentation_duration: Option<u64>,
    /// `segmentation_upid_type` (Table 22).
    pub segmentation_upid_type: SegmentationUpidType,
    /// `segmentation_upid()` payload bytes (length = `segmentation_upid_length`).
    pub segmentation_upid: &'a [u8],
    /// `segmentation_type_id` (Table 23).
    pub segmentation_type_id: SegmentationTypeId,
    /// `segment_num`.
    pub segment_num: u8,
    /// `segments_expected`.
    pub segments_expected: u8,
    /// `(sub_segment_num, sub_segments_expected)`, present when the
    /// `descriptor_length` includes the optional appendix (§10.3.3.1).
    pub sub_segments: Option<(u8, u8)>,
}

impl<'a> Default for SegmentationDescriptor<'a> {
    fn default() -> Self {
        Self {
            identifier: CUEI,
            segmentation_event_id: 0,
            segmentation_event_cancel_indicator: false,
            segmentation_event_id_compliance_indicator: true,
            program_segmentation_flag: true,
            delivery_restrictions: None,
            components: Vec::new(),
            segmentation_duration: None,
            segmentation_upid_type: SegmentationUpidType::NotUsed,
            segmentation_upid: &[],
            segmentation_type_id: SegmentationTypeId::NotIndicated,
            segment_num: 0,
            segments_expected: 0,
            sub_segments: None,
        }
    }
}

impl<'a> Parse<'a> for SegmentationDescriptor<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (identifier, body) = header::descriptor_body(bytes, TAG, "segmentation_descriptor")?;
        // segmentation_event_id (4) + flags byte (1).
        if body.len() < 5 {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN + 5,
                have: bytes.len(),
                what: "segmentation_descriptor header",
            });
        }
        let (eid_bytes, rest) = body.split_first_chunk::<4>().ok_or(Error::BufferTooShort {
            need: HEADER_LEN + 5,
            have: bytes.len(),
            what: "segmentation_descriptor header",
        })?;
        let segmentation_event_id = u32::from_be_bytes(*eid_bytes);
        let b = rest[0];
        let cancel = b & 0x80 != 0;
        let compliance = b & 0x40 != 0;

        let mut out = Self {
            identifier,
            segmentation_event_id,
            segmentation_event_cancel_indicator: cancel,
            segmentation_event_id_compliance_indicator: compliance,
            ..Self::default()
        };
        if cancel {
            return Ok(out);
        }

        if body.len() < 6 {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN + 6,
                have: bytes.len(),
                what: "segmentation_descriptor flags",
            });
        }
        let flags = body[5];
        out.program_segmentation_flag = flags & 0x80 != 0;
        let duration_flag = flags & 0x40 != 0;
        let delivery_not_restricted = flags & 0x20 != 0;
        if !delivery_not_restricted {
            out.delivery_restrictions = Some(DeliveryRestrictions {
                web_delivery_allowed: flags & 0x10 != 0,
                no_regional_blackout: flags & 0x08 != 0,
                archive_allowed: flags & 0x04 != 0,
                device_restrictions: DeviceRestrictions::from_bits(flags & 0x03),
            });
        }
        let mut pos = 6;

        if !out.program_segmentation_flag {
            if body.len() < pos + 1 {
                return Err(Error::BufferTooShort {
                    need: HEADER_LEN + pos + 1,
                    have: bytes.len(),
                    what: "segmentation_descriptor component_count",
                });
            }
            let count = body[pos] as usize;
            pos += 1;
            for _ in 0..count {
                if body.len() < pos + 6 {
                    return Err(Error::BufferTooShort {
                        need: HEADER_LEN + pos + 6,
                        have: bytes.len(),
                        what: "segmentation_descriptor component",
                    });
                }
                let component_tag = body[pos];
                // 7 reserved bits, then 33-bit pts_offset.
                let pts_offset = ((u64::from(body[pos + 1] & 0x01)) << 32)
                    | (u64::from(body[pos + 2]) << 24)
                    | (u64::from(body[pos + 3]) << 16)
                    | (u64::from(body[pos + 4]) << 8)
                    | u64::from(body[pos + 5]);
                out.components.push(SegmentationComponent {
                    component_tag,
                    pts_offset,
                });
                pos += 6;
            }
        }

        if duration_flag {
            if body.len() < pos + 5 {
                return Err(Error::BufferTooShort {
                    need: HEADER_LEN + pos + 5,
                    have: bytes.len(),
                    what: "segmentation_descriptor segmentation_duration",
                });
            }
            let d = (u64::from(body[pos]) << 32)
                | (u64::from(body[pos + 1]) << 24)
                | (u64::from(body[pos + 2]) << 16)
                | (u64::from(body[pos + 3]) << 8)
                | u64::from(body[pos + 4]);
            out.segmentation_duration = Some(d);
            pos += 5;
        }

        // segmentation_upid_type (1) + segmentation_upid_length (1).
        if body.len() < pos + 2 {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN + pos + 2,
                have: bytes.len(),
                what: "segmentation_descriptor upid header",
            });
        }
        out.segmentation_upid_type = SegmentationUpidType::from_u8(body[pos]);
        let upid_len = body[pos + 1] as usize;
        pos += 2;
        if body.len() < pos + upid_len {
            return Err(Error::LengthOverflow {
                declared: upid_len,
                available: body.len().saturating_sub(pos),
                what: "segmentation_descriptor segmentation_upid",
            });
        }
        out.segmentation_upid = &body[pos..pos + upid_len];
        pos += upid_len;

        // segmentation_type_id (1) + segment_num (1) + segments_expected (1).
        if body.len() < pos + 3 {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN + pos + 3,
                have: bytes.len(),
                what: "segmentation_descriptor type/segment",
            });
        }
        out.segmentation_type_id = SegmentationTypeId::from_u8(body[pos]);
        out.segment_num = body[pos + 1];
        out.segments_expected = body[pos + 2];
        pos += 3;

        // Optional sub_segment appendix — present iff descriptor_length left
        // room for two more bytes (§10.3.3.1).
        if body.len() >= pos + 2 {
            out.sub_segments = Some((body[pos], body[pos + 1]));
            pos += 2;
        }
        // Any trailing bytes within descriptor_length are tolerated but unused.
        let _ = pos;
        Ok(out)
    }
}

impl<'a> SegmentationDescriptor<'a> {
    fn body_len(&self) -> usize {
        if self.segmentation_event_cancel_indicator {
            return 5; // event_id (4) + cancel byte (1)
        }
        let mut len = 6; // event_id (4) + cancel byte (1) + flags byte (1)
        if !self.program_segmentation_flag {
            len += 1; // component_count
            len += self.components.len() * 6;
        }
        if self.segmentation_duration.is_some() {
            len += 5;
        }
        len += 2 + self.segmentation_upid.len(); // upid_type + upid_length + upid
        len += 3; // type_id + segment_num + segments_expected
        if self.sub_segments.is_some() {
            len += 2;
        }
        len
    }

    /// Decode the `segmentation_upid` as an MPU() structure (§10.3.3.3, Table 24).
    ///
    /// Returns `Some(Ok(Mpu { .. }))` when `segmentation_upid_type == Mpu` and the
    /// UPID bytes contain at least the 4-byte `format_identifier`.
    /// Returns `Some(Err(..))` if the UPID is shorter than 4 bytes (truncated).
    /// Returns `None` for any other UPID type.
    #[must_use]
    pub fn mpu(&self) -> Option<Result<Mpu<'a>>> {
        if self.segmentation_upid_type != SegmentationUpidType::Mpu {
            return None;
        }
        let bytes = self.segmentation_upid;
        Some(
            bytes
                .split_first_chunk::<MPU_FORMAT_IDENTIFIER_LEN>()
                .ok_or(Error::BufferTooShort {
                    need: MPU_FORMAT_IDENTIFIER_LEN,
                    have: bytes.len(),
                    what: "MPU() format_identifier",
                })
                .map(|(fi_bytes, private_data)| Mpu {
                    format_identifier: u32::from_be_bytes(*fi_bytes),
                    private_data,
                }),
        )
    }

    /// Decode the `segmentation_upid` as a MID() structure (§10.3.3.4, Table 25).
    ///
    /// Returns `Some(Ok(Vec<MidUpid>))` when `segmentation_upid_type == Mid` and
    /// every entry in the byte sequence parses cleanly.
    /// Returns `Some(Err(..))` if any entry header or payload is truncated.
    /// Returns `None` for any other UPID type.
    #[must_use]
    pub fn mid(&self) -> Option<Result<Vec<MidUpid<'a>>>> {
        if self.segmentation_upid_type != SegmentationUpidType::Mid {
            return None;
        }
        let mut entries = Vec::new();
        let mut pos = 0;
        let bytes = self.segmentation_upid;
        while pos < bytes.len() {
            if bytes.len() - pos < MID_ENTRY_HEADER_LEN {
                return Some(Err(Error::BufferTooShort {
                    need: pos + MID_ENTRY_HEADER_LEN,
                    have: bytes.len(),
                    what: "MID() entry header",
                }));
            }
            let upid_type = SegmentationUpidType::from_u8(bytes[pos]);
            let entry_len = bytes[pos + 1] as usize;
            pos += MID_ENTRY_HEADER_LEN;
            if bytes.len() - pos < entry_len {
                return Some(Err(Error::LengthOverflow {
                    declared: entry_len,
                    available: bytes.len() - pos,
                    what: "MID() entry segmentation_upid",
                }));
            }
            let upid = &bytes[pos..pos + entry_len];
            pos += entry_len;
            entries.push(MidUpid { upid_type, upid });
        }
        Some(Ok(entries))
    }
}

impl Serialize for SegmentationDescriptor<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + self.body_len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let body_len = self.body_len();
        if body_len + 4 > u8::MAX as usize {
            return Err(Error::InvalidValue {
                field: "segmentation_descriptor.descriptor_length",
                reason: "descriptor body exceeds 8-bit descriptor_length",
            });
        }
        header::write_header(buf, TAG, self.identifier, body_len);
        let mut pos = HEADER_LEN;

        buf[pos..pos + 4].copy_from_slice(&self.segmentation_event_id.to_be_bytes());
        // cancel (1) + compliance (1) + 6 reserved bits = 1.
        buf[pos + 4] = (u8::from(self.segmentation_event_cancel_indicator) << 7)
            | (u8::from(self.segmentation_event_id_compliance_indicator) << 6)
            | 0x3F;
        pos += 5;
        if self.segmentation_event_cancel_indicator {
            return Ok(need);
        }

        let duration_flag = self.segmentation_duration.is_some();
        let mut flags =
            (u8::from(self.program_segmentation_flag) << 7) | (u8::from(duration_flag) << 6);
        match &self.delivery_restrictions {
            Some(dr) => {
                // delivery_not_restricted_flag = 0
                flags |= u8::from(dr.web_delivery_allowed) << 4;
                flags |= u8::from(dr.no_regional_blackout) << 3;
                flags |= u8::from(dr.archive_allowed) << 2;
                flags |= dr.device_restrictions.bits() & 0x03;
            }
            None => {
                // delivery_not_restricted_flag = 1, 5 reserved bits = 1.
                flags |= 0x20 | 0x1F;
            }
        }
        buf[pos] = flags;
        pos += 1;

        if !self.program_segmentation_flag {
            buf[pos] = self.components.len() as u8;
            pos += 1;
            for c in &self.components {
                buf[pos] = c.component_tag;
                let o = c.pts_offset & ((1u64 << 33) - 1);
                // 7 reserved bits = 1, then top pts_offset bit.
                buf[pos + 1] = 0xFE | ((o >> 32) as u8 & 0x01);
                buf[pos + 2] = (o >> 24) as u8;
                buf[pos + 3] = (o >> 16) as u8;
                buf[pos + 4] = (o >> 8) as u8;
                buf[pos + 5] = o as u8;
                pos += 6;
            }
        }

        if let Some(d) = self.segmentation_duration {
            let d = d & ((1u64 << 40) - 1);
            buf[pos] = (d >> 32) as u8;
            buf[pos + 1] = (d >> 24) as u8;
            buf[pos + 2] = (d >> 16) as u8;
            buf[pos + 3] = (d >> 8) as u8;
            buf[pos + 4] = d as u8;
            pos += 5;
        }

        buf[pos] = self.segmentation_upid_type.to_u8();
        buf[pos + 1] = self.segmentation_upid.len() as u8;
        pos += 2;
        buf[pos..pos + self.segmentation_upid.len()].copy_from_slice(self.segmentation_upid);
        pos += self.segmentation_upid.len();

        buf[pos] = self.segmentation_type_id.to_u8();
        buf[pos + 1] = self.segment_num;
        buf[pos + 2] = self.segments_expected;
        pos += 3;

        if let Some((sn, se)) = self.sub_segments {
            buf[pos] = sn;
            buf[pos + 1] = se;
            pos += 2;
        }

        debug_assert_eq!(pos, need);
        Ok(need)
    }
}

impl<'a> SpliceDescriptorDef<'a> for SegmentationDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "SEGMENTATION";
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt(d: &SegmentationDescriptor) {
        let bytes = d.to_bytes();
        assert_eq!(bytes.len(), d.serialized_len());
        assert_eq!(bytes[0], TAG);
        // descriptor_length counts the bytes after it: identifier (4) + body.
        assert_eq!(bytes[1] as usize, 4 + d.body_len());
        let back = SegmentationDescriptor::parse(&bytes).unwrap();
        assert_eq!(*d, back);
        assert_eq!(back.to_bytes(), bytes);
    }

    #[test]
    fn round_trip_cancel() {
        rt(&SegmentationDescriptor {
            segmentation_event_id: 0x1234,
            segmentation_event_cancel_indicator: true,
            ..Default::default()
        });
    }

    #[test]
    fn round_trip_program_with_duration_and_upid() {
        rt(&SegmentationDescriptor {
            segmentation_event_id: 0x4800_000A,
            segmentation_event_id_compliance_indicator: false,
            program_segmentation_flag: true,
            delivery_restrictions: Some(DeliveryRestrictions {
                web_delivery_allowed: false,
                no_regional_blackout: true,
                archive_allowed: true,
                device_restrictions: DeviceRestrictions::RestrictGroup1,
            }),
            segmentation_duration: Some(90_000 * 30),
            segmentation_upid_type: SegmentationUpidType::AdId,
            segmentation_upid: b"ABCD12345678",
            segmentation_type_id: SegmentationTypeId::ProviderPlacementOpportunityStart,
            segment_num: 1,
            segments_expected: 1,
            sub_segments: Some((1, 2)),
            ..Default::default()
        });
    }

    #[test]
    fn round_trip_no_restrictions_no_subsegments() {
        rt(&SegmentationDescriptor {
            segmentation_event_id: 7,
            delivery_restrictions: None,
            segmentation_type_id: SegmentationTypeId::ProgramStart,
            segment_num: 1,
            segments_expected: 1,
            ..Default::default()
        });
    }

    #[test]
    fn round_trip_component_mode() {
        rt(&SegmentationDescriptor {
            segmentation_event_id: 9,
            program_segmentation_flag: false,
            components: vec![
                SegmentationComponent {
                    component_tag: 1,
                    pts_offset: 0x1_0000,
                },
                SegmentationComponent {
                    component_tag: 2,
                    pts_offset: 0,
                },
            ],
            segmentation_upid_type: SegmentationUpidType::NotUsed,
            segmentation_type_id: SegmentationTypeId::BreakStart,
            ..Default::default()
        });
    }

    // ── MPU() tests — §10.3.3.3, Table 24 ────────────────────────────────────

    /// Build an MPU() UPID field-by-field from Table 24:
    ///   format_identifier (32 bits uimsbf) | private_data (remaining bytes).
    /// The `segmentation_upid_length` includes the 4-byte format_identifier.
    #[test]
    fn mpu_accessor_decodes_correctly() {
        // format_identifier = 0x41424344 ("ABCD"), private_data = [0x01, 0x02, 0x03].
        // Wire MPU() bytes: [0x41, 0x42, 0x43, 0x44, 0x01, 0x02, 0x03].
        const FORMAT_ID: u32 = 0x4142_4344;
        let upid_bytes: &[u8] = &[0x41, 0x42, 0x43, 0x44, 0x01, 0x02, 0x03];

        let d = SegmentationDescriptor {
            segmentation_event_id: 0x1,
            segmentation_upid_type: SegmentationUpidType::Mpu,
            segmentation_upid: upid_bytes,
            segmentation_type_id: SegmentationTypeId::ContentIdentification,
            segment_num: 1,
            segments_expected: 1,
            ..Default::default()
        };

        // Accessor returns Some(Ok(..)).
        let mpu = d.mpu().expect("Some").expect("Ok");
        assert_eq!(mpu.format_identifier, FORMAT_ID);
        assert_eq!(mpu.private_data, &[0x01u8, 0x02, 0x03]);

        // Non-MPU UPID type returns None.
        let d_ti = SegmentationDescriptor {
            segmentation_upid_type: SegmentationUpidType::Ti,
            segmentation_upid: &[0u8; 8],
            ..d.clone()
        };
        assert!(d_ti.mpu().is_none());

        // Whole descriptor still round-trips byte-identical.
        rt(&d);
    }

    /// Truncated MPU() UPID (< 4 bytes) returns Some(Err(..)).
    #[test]
    fn mpu_accessor_truncated_returns_err() {
        let d = SegmentationDescriptor {
            segmentation_event_id: 0x2,
            segmentation_upid_type: SegmentationUpidType::Mpu,
            segmentation_upid: &[0xAA, 0xBB], // only 2 bytes — truncated
            segmentation_type_id: SegmentationTypeId::NotIndicated,
            ..Default::default()
        };
        assert!(matches!(
            d.mpu(),
            Some(Err(Error::BufferTooShort {
                what: "MPU() format_identifier",
                ..
            }))
        ));
    }

    // ── MID() tests — §10.3.3.4, Table 25 ───────────────────────────────────

    /// Build a 2-entry MID() UPID field-by-field from Table 25:
    ///   for each entry: segmentation_upid_type (8) | length (8) | segmentation_upid (length * 8).
    /// Number of entries is implicit — parsing ends when the outer UPID bytes are exhausted.
    #[test]
    fn mid_accessor_decodes_two_entries() {
        // Entry 1: type=0x03 (AdId), length=12, upid = b"ABCD12345678"
        // Entry 2: type=0x08 (Ti), length=8, upid = 8 zero bytes
        let mut upid_bytes = Vec::new();
        // Entry 1: AdId
        upid_bytes.push(0x03u8); // segmentation_upid_type = AdId
        upid_bytes.push(12u8); // length
        upid_bytes.extend_from_slice(b"ABCD12345678");
        // Entry 2: Ti
        upid_bytes.push(0x08u8); // segmentation_upid_type = Ti
        upid_bytes.push(8u8); // length
        upid_bytes.extend_from_slice(&[0u8; 8]);

        let d = SegmentationDescriptor {
            segmentation_event_id: 0x3,
            segmentation_upid_type: SegmentationUpidType::Mid,
            segmentation_upid: &upid_bytes,
            segmentation_type_id: SegmentationTypeId::ProgramStart,
            segment_num: 1,
            segments_expected: 1,
            ..Default::default()
        };

        let entries = d.mid().expect("Some").expect("Ok");
        assert_eq!(entries.len(), 2);

        assert_eq!(entries[0].upid_type, SegmentationUpidType::AdId);
        assert_eq!(entries[0].upid, b"ABCD12345678");

        assert_eq!(entries[1].upid_type, SegmentationUpidType::Ti);
        assert_eq!(entries[1].upid, &[0u8; 8]);

        // Non-MID UPID type returns None.
        let d_adid = SegmentationDescriptor {
            segmentation_upid_type: SegmentationUpidType::AdId,
            ..d.clone()
        };
        assert!(d_adid.mid().is_none());

        // Whole descriptor still round-trips byte-identical.
        rt(&d);
    }

    /// Truncated MID() entry (entry header present but payload truncated) returns Some(Err(..)).
    #[test]
    fn mid_accessor_truncated_entry_returns_err() {
        // Entry declares length=5 but only 2 bytes of upid follow.
        let upid_bytes: &[u8] = &[
            0x03u8, // segmentation_upid_type = AdId
            5u8,    // declared length = 5
            0xAA, 0xBB, // only 2 bytes — truncated
        ];
        let d = SegmentationDescriptor {
            segmentation_event_id: 0x4,
            segmentation_upid_type: SegmentationUpidType::Mid,
            segmentation_upid: upid_bytes,
            segmentation_type_id: SegmentationTypeId::NotIndicated,
            ..Default::default()
        };
        assert!(matches!(
            d.mid(),
            Some(Err(Error::LengthOverflow {
                what: "MID() entry segmentation_upid",
                ..
            }))
        ));
    }
}
