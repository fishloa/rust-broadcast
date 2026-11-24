//! segmentation_descriptor() — ANSI/SCTE 35 2023r1 §10.3.3, Table 20 (tag 0x02).
//!
//! The richest splice descriptor: carries a segmentation event, its
//! restriction flags, an optional 40-bit `segmentation_duration`, a
//! type/length-prefixed `segmentation_upid()`, the
//! [`SegmentationTypeId`], and an optional `sub_segment_num` /
//! `sub_segments_expected` appendix that §10.3.3.1 defines for the eight
//! Table 23 `Start` types ([`SegmentationTypeId::has_sub_segments`]) and that
//! `descriptor_length` announces on the wire.
//!
//! The tail after `segments_expected` is carried, never guessed away: parse
//! reads the appendix only for a type that carries one and keeps every other
//! byte inside `descriptor_length` in [`SegmentationDescriptor::trailing`], so
//! an over-long descriptor re-serializes byte-exactly instead of losing a
//! third byte to a skipped position.
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
use broadcast_common::len::need_out;

use super::header::{self, CUEI, HEADER_LEN};
use super::segmentation_enums::{DeviceRestrictions, SegmentationTypeId, SegmentationUpidType};
use crate::CANCEL_INDICATOR_MASK;
use crate::error::{Error, Result};
use crate::traits::SpliceDescriptorDef;
use broadcast_common::{Parse, Serialize};

/// Width in bits (and bytes) of the `format_identifier` field in MPU() (§10.3.3.3, Table 24).
const MPU_FORMAT_IDENTIFIER_LEN: usize = 4; // 32 bits / 8

/// Minimum byte length of an MID() sub-entry header: `segmentation_upid_type` (1) + `length` (1).
const MID_ENTRY_HEADER_LEN: usize = 2;

/// Byte length of the CUEI `identifier` preceding a descriptor body.
const CUEI_LEN: usize = core::mem::size_of::<u32>();
const SEGMENTATION_EVENT_ID_LEN: usize = core::mem::size_of::<u32>();
const CANCEL_AND_COMPLIANCE_LEN: usize = 1;
/// `segmentation_event_id` + the cancel/compliance flags byte.
const EVENT_ID_AND_FLAGS_LEN: usize = SEGMENTATION_EVENT_ID_LEN + CANCEL_AND_COMPLIANCE_LEN;
/// Event ID, cancel/compliance byte, and the program flags byte.
const SEGMENTATION_HEADER_LEN: usize = EVENT_ID_AND_FLAGS_LEN + 1;
const FLAGS_OFFSET: usize = EVENT_ID_AND_FLAGS_LEN;
const COMPLIANCE_INDICATOR_MASK: u8 = 0x40;
const PROGRAM_SEGMENTATION_MASK: u8 = 0x80;
/// Mask of the segmentation descriptor `duration_flag` bit.
const DURATION_FLAG_MASK: u8 = 0x40;
/// Bit position of the segmentation descriptor `duration_flag`.
const DURATION_FLAG_SHIFT: u32 = 6;
const DELIVERY_NOT_RESTRICTED_MASK: u8 = 0x20;
const WEB_DELIVERY_ALLOWED_MASK: u8 = 0x10;
const NO_REGIONAL_BLACKOUT_MASK: u8 = 0x08;
const ARCHIVE_ALLOWED_MASK: u8 = 0x04;
const DEVICE_RESTRICTIONS_MASK: u8 = 0x03;
const CANCEL_INDICATOR_SHIFT: u32 = 7;
const COMPLIANCE_INDICATOR_SHIFT: u32 = 6;
const PROGRAM_SEGMENTATION_SHIFT: u32 = 7;
const WEB_DELIVERY_ALLOWED_SHIFT: u32 = 4;
const NO_REGIONAL_BLACKOUT_SHIFT: u32 = 3;
const ARCHIVE_ALLOWED_SHIFT: u32 = 2;
const RESERVED_BITS_SET: u8 = 0x3F;
const DELIVERY_RESERVED_BITS: u8 = 0x1F;
const PTS_RESERVED_BITS: u8 = 0xFE;
const COMPONENT_LEN: usize = 6;
const COMPONENT_TAG_OFFSET: usize = 0;
const COMPONENT_PTS_TOP_OFFSET: usize = 1;
const COMPONENT_PTS_HIGH_OFFSET: usize = 2;
const COMPONENT_PTS_MID_OFFSET: usize = 3;
const COMPONENT_PTS_LOW_OFFSET: usize = 4;
const COMPONENT_PTS_BOTTOM_OFFSET: usize = 5;
const PTS_OFFSET_TOP_MASK: u8 = 0x01;
const PTS_OFFSET_TOP_SHIFT: u32 = 32;
const PTS_OFFSET_HIGH_SHIFT: u32 = 24;
const PTS_OFFSET_MID_SHIFT: u32 = 16;
const PTS_OFFSET_LOW_SHIFT: u32 = 8;
const DURATION_LEN: usize = 5;
const DURATION_TOP_OFFSET: usize = 0;
const DURATION_HIGH_OFFSET: usize = 1;
const DURATION_MID_OFFSET: usize = 2;
const DURATION_LOW_OFFSET: usize = 3;
const DURATION_BOTTOM_OFFSET: usize = 4;
const DURATION_TOP_SHIFT: u32 = 32;
const DURATION_HIGH_SHIFT: u32 = 24;
const DURATION_MID_SHIFT: u32 = 16;
const DURATION_LOW_SHIFT: u32 = 8;
const UPID_TYPE_LEN: usize = 1;
const UPID_HEADER_LEN: usize = 2;
const SEGMENTATION_TYPE_ID_OFFSET: usize = 0;
const SEGMENT_NUM_OFFSET: usize = 1;
const SEGMENTS_EXPECTED_OFFSET: usize = 2;
const TYPE_SEGMENT_LEN: usize = 3;
const SUB_SEGMENTS_LEN: usize = 2;

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

/// The optional `sub_segment_num` / `sub_segments_expected` appendix of a
/// `segmentation_descriptor()` (§10.3.3.1, Table 20).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SubSegments {
    /// `sub_segment_num` (Table 23).
    pub sub_segment_num: u8,
    /// `sub_segments_expected` (Table 23).
    pub sub_segments_expected: u8,
}

impl SubSegments {
    /// The plain `(sub_segment_num, sub_segments_expected)` pair.
    #[must_use]
    pub const fn pair(self) -> (u8, u8) {
        (self.sub_segment_num, self.sub_segments_expected)
    }
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
    /// `(sub_segment_num, sub_segments_expected)` — the optional appendix
    /// §10.3.3.1 defines only for the eight Table 23 `Start` types
    /// ([`SegmentationTypeId::has_sub_segments`]).
    ///
    /// The struct form is exact: `None` writes no appendix (for any type) and
    /// `Some(..)` writes it and requires the type to carry it. Wire input is
    /// looser by design — SCTE 35 says `descriptor_length` alone signals
    /// presence — so an appendix on a type that does not carry one, or bytes
    /// past a well-formed appendix, are carried verbatim in
    /// [`Self::trailing`] instead of being discarded.
    pub sub_segments: Option<SubSegments>,
    /// Bytes after the descriptor's last defined field, inside
    /// `descriptor_length`: bytes past the appendix (`pos + 2`), and the whole
    /// appendix `Some(s)` would otherwise have taken. `&[]` for every
    /// descriptor this crate builds.
    pub trailing: &'a [u8],
}

impl Default for SegmentationDescriptor<'_> {
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
            trailing: &[],
        }
    }
}

impl<'a> Parse<'a> for SegmentationDescriptor<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (identifier, body) = header::descriptor_body(bytes, TAG, "segmentation_descriptor")?;
        // segmentation_event_id (4) + flags byte (1).
        if body.len() < EVENT_ID_AND_FLAGS_LEN {
            return Err(Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: HEADER_LEN + EVENT_ID_AND_FLAGS_LEN,
                    have: bytes.len(),
                    what: "segmentation_descriptor header",
                },
            ));
        }
        let (eid_bytes, rest) = body
            .split_first_chunk::<SEGMENTATION_EVENT_ID_LEN>()
            .ok_or(Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: HEADER_LEN + EVENT_ID_AND_FLAGS_LEN,
                    have: bytes.len(),
                    what: "segmentation_descriptor header",
                },
            ))?;
        let segmentation_event_id = u32::from_be_bytes(*eid_bytes);
        let b = rest[0];
        let cancel = b & CANCEL_INDICATOR_MASK != 0;
        let compliance = b & COMPLIANCE_INDICATOR_MASK != 0;

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

        if body.len() < SEGMENTATION_HEADER_LEN {
            return Err(Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: HEADER_LEN + SEGMENTATION_HEADER_LEN,
                    have: bytes.len(),
                    what: "segmentation_descriptor flags",
                },
            ));
        }
        let flags = body[FLAGS_OFFSET];
        out.program_segmentation_flag = flags & PROGRAM_SEGMENTATION_MASK != 0;
        let duration_flag = flags & DURATION_FLAG_MASK != 0;
        let delivery_not_restricted = flags & DELIVERY_NOT_RESTRICTED_MASK != 0;
        if !delivery_not_restricted {
            out.delivery_restrictions = Some(DeliveryRestrictions {
                web_delivery_allowed: flags & WEB_DELIVERY_ALLOWED_MASK != 0,
                no_regional_blackout: flags & NO_REGIONAL_BLACKOUT_MASK != 0,
                archive_allowed: flags & ARCHIVE_ALLOWED_MASK != 0,
                device_restrictions: DeviceRestrictions::from_bits(
                    flags & DEVICE_RESTRICTIONS_MASK,
                ),
            });
        }
        let mut pos = SEGMENTATION_HEADER_LEN;

        if !out.program_segmentation_flag {
            if body.len() < pos + 1 {
                return Err(Error::BufferTooShort(
                    broadcast_common::error::BufferTooShort {
                        need: HEADER_LEN + pos + 1,
                        have: bytes.len(),
                        what: "segmentation_descriptor component_count",
                    },
                ));
            }
            let count = body[pos] as usize;
            pos += 1;
            for _ in 0..count {
                if body.len() < pos + COMPONENT_LEN {
                    return Err(Error::BufferTooShort(
                        broadcast_common::error::BufferTooShort {
                            need: HEADER_LEN + pos + COMPONENT_LEN,
                            have: bytes.len(),
                            what: "segmentation_descriptor component",
                        },
                    ));
                }
                let component_tag = body[pos + COMPONENT_TAG_OFFSET];
                // 7 reserved bits, then 33-bit pts_offset.
                let pts_offset =
                    (u64::from(body[pos + COMPONENT_PTS_TOP_OFFSET] & PTS_OFFSET_TOP_MASK)
                        << PTS_OFFSET_TOP_SHIFT)
                        | (u64::from(body[pos + COMPONENT_PTS_HIGH_OFFSET])
                            << PTS_OFFSET_HIGH_SHIFT)
                        | (u64::from(body[pos + COMPONENT_PTS_MID_OFFSET]) << PTS_OFFSET_MID_SHIFT)
                        | (u64::from(body[pos + COMPONENT_PTS_LOW_OFFSET]) << PTS_OFFSET_LOW_SHIFT)
                        | u64::from(body[pos + COMPONENT_PTS_BOTTOM_OFFSET]);
                out.components.push(SegmentationComponent {
                    component_tag,
                    pts_offset,
                });
                pos += COMPONENT_LEN;
            }
        }

        if duration_flag {
            if body.len() < pos + DURATION_LEN {
                return Err(Error::BufferTooShort(
                    broadcast_common::error::BufferTooShort {
                        need: HEADER_LEN + pos + DURATION_LEN,
                        have: bytes.len(),
                        what: "segmentation_descriptor segmentation_duration",
                    },
                ));
            }
            let d = (u64::from(body[pos + DURATION_TOP_OFFSET]) << DURATION_TOP_SHIFT)
                | (u64::from(body[pos + DURATION_HIGH_OFFSET]) << DURATION_HIGH_SHIFT)
                | (u64::from(body[pos + DURATION_MID_OFFSET]) << DURATION_MID_SHIFT)
                | (u64::from(body[pos + DURATION_LOW_OFFSET]) << DURATION_LOW_SHIFT)
                | u64::from(body[pos + DURATION_BOTTOM_OFFSET]);
            out.segmentation_duration = Some(d);
            pos += DURATION_LEN;
        }

        // segmentation_upid_type (1) + segmentation_upid_length (1).
        if body.len() < pos + UPID_HEADER_LEN {
            return Err(Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: HEADER_LEN + pos + UPID_HEADER_LEN,
                    have: bytes.len(),
                    what: "segmentation_descriptor upid header",
                },
            ));
        }
        out.segmentation_upid_type =
            SegmentationUpidType::from_u8(body[pos + SEGMENTATION_TYPE_ID_OFFSET]);
        let upid_len = body[pos + UPID_TYPE_LEN] as usize;
        pos += UPID_HEADER_LEN;
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
        if body.len() < pos + TYPE_SEGMENT_LEN {
            return Err(Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: HEADER_LEN + pos + TYPE_SEGMENT_LEN,
                    have: bytes.len(),
                    what: "segmentation_descriptor type/segment",
                },
            ));
        }
        out.segmentation_type_id =
            SegmentationTypeId::from_u8(body[pos + SEGMENTATION_TYPE_ID_OFFSET]);
        out.segment_num = body[pos + SEGMENT_NUM_OFFSET];
        out.segments_expected = body[pos + SEGMENTS_EXPECTED_OFFSET];
        pos += TYPE_SEGMENT_LEN;

        // The optional sub-segment appendix (§10.3.3.1). SCTE 35 says
        // `descriptor_length` alone announces it, while Table 23 defines it only
        // for the eight `Start` types; bytes the struct form cannot carry are
        // kept in `trailing`, never dropped, so a re-serialize is byte-exact.
        let rest = &body[pos..];
        out.sub_segments = if out.segmentation_type_id.has_sub_segments() {
            rest.first_chunk::<SUB_SEGMENTS_LEN>()
                .map(|pair| SubSegments {
                    sub_segment_num: pair[0],
                    sub_segments_expected: pair[1],
                })
        } else {
            None
        };
        out.trailing = &rest[if out.sub_segments.is_some() {
            SUB_SEGMENTS_LEN
        } else {
            0
        }..];
        Ok(out)
    }
}

impl<'a> SegmentationDescriptor<'a> {
    fn body_len(&self) -> usize {
        if self.segmentation_event_cancel_indicator {
            return EVENT_ID_AND_FLAGS_LEN; // event_id (4) + cancel byte (1)
        }
        let mut len = SEGMENTATION_HEADER_LEN; // event_id (4) + cancel byte (1) + flags byte (1)
        if !self.program_segmentation_flag {
            len += 1; // component_count
            len += self.components.len() * COMPONENT_LEN;
        }
        if self.segmentation_duration.is_some() {
            len += DURATION_LEN;
        }
        len += UPID_HEADER_LEN + self.segmentation_upid.len(); // upid_type + upid_length + upid
        len += TYPE_SEGMENT_LEN; // type_id + segment_num + segments_expected
        if self.sub_segments.is_some() {
            len += SUB_SEGMENTS_LEN;
        }
        len += self.trailing.len();
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
                .ok_or(Error::BufferTooShort(
                    broadcast_common::error::BufferTooShort {
                        need: MPU_FORMAT_IDENTIFIER_LEN,
                        have: bytes.len(),
                        what: "MPU() format_identifier",
                    },
                ))
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
                return Some(Err(Error::BufferTooShort(
                    broadcast_common::error::BufferTooShort {
                        need: pos + MID_ENTRY_HEADER_LEN,
                        have: bytes.len(),
                        what: "MID() entry header",
                    },
                )));
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
        need_out(buf, need)?;
        let body_len = self.body_len();
        if body_len + CUEI_LEN > u8::MAX as usize {
            return Err(Error::FieldOverflow(broadcast_common::len::FieldOverflow {
                field: "segmentation_descriptor.descriptor_length",
                value: (body_len + CUEI_LEN) as u64,
                max: u64::from(u8::MAX),
            }));
        }
        header::write_header(buf, TAG, self.identifier, body_len)?;
        let mut pos = HEADER_LEN;

        buf[pos..pos + SEGMENTATION_EVENT_ID_LEN]
            .copy_from_slice(&self.segmentation_event_id.to_be_bytes());
        // cancel (1) + compliance (1) + 6 reserved bits = 1.
        buf[pos + SEGMENTATION_EVENT_ID_LEN] = (u8::from(self.segmentation_event_cancel_indicator)
            << CANCEL_INDICATOR_SHIFT)
            | (u8::from(self.segmentation_event_id_compliance_indicator)
                << COMPLIANCE_INDICATOR_SHIFT)
            | RESERVED_BITS_SET;
        pos += EVENT_ID_AND_FLAGS_LEN;
        if self.segmentation_event_cancel_indicator {
            return Ok(need);
        }

        let duration_flag = self.segmentation_duration.is_some();
        let mut flags = (u8::from(self.program_segmentation_flag) << PROGRAM_SEGMENTATION_SHIFT)
            | (u8::from(duration_flag) << DURATION_FLAG_SHIFT);
        match &self.delivery_restrictions {
            Some(dr) => {
                // delivery_not_restricted_flag = 0
                flags |= u8::from(dr.web_delivery_allowed) << WEB_DELIVERY_ALLOWED_SHIFT;
                flags |= u8::from(dr.no_regional_blackout) << NO_REGIONAL_BLACKOUT_SHIFT;
                flags |= u8::from(dr.archive_allowed) << ARCHIVE_ALLOWED_SHIFT;
                // `DeviceRestrictions` is a closed 2-bit enum: `bits()` is always <= 0b11.
                flags |= dr.device_restrictions.bits();
            }
            None => {
                // delivery_not_restricted_flag = 1, 5 reserved bits = 1.
                flags |= DELIVERY_NOT_RESTRICTED_MASK | DELIVERY_RESERVED_BITS;
            }
        }
        buf[pos] = flags;
        pos += 1;

        if !self.program_segmentation_flag {
            buf[pos] = broadcast_common::len::fit_u8(
                self.components.len(),
                "segmentation_descriptor.component_count",
            )?;
            pos += 1;
            for c in &self.components {
                buf[pos] = c.component_tag;
                if c.pts_offset > crate::time::PTS_MAX {
                    return Err(Error::FieldOverflow(broadcast_common::len::FieldOverflow {
                        field: "segmentation_descriptor.component.pts_offset",
                        value: c.pts_offset,
                        max: crate::time::PTS_MAX,
                    }));
                }
                let o = c.pts_offset;
                // 7 reserved bits = 1, then top pts_offset bit.
                buf[pos + COMPONENT_PTS_TOP_OFFSET] =
                    PTS_RESERVED_BITS | ((o >> PTS_OFFSET_TOP_SHIFT) as u8 & PTS_OFFSET_TOP_MASK);
                buf[pos + COMPONENT_PTS_HIGH_OFFSET] = (o >> PTS_OFFSET_HIGH_SHIFT) as u8;
                buf[pos + COMPONENT_PTS_MID_OFFSET] = (o >> PTS_OFFSET_MID_SHIFT) as u8;
                buf[pos + COMPONENT_PTS_LOW_OFFSET] = (o >> PTS_OFFSET_LOW_SHIFT) as u8;
                buf[pos + COMPONENT_PTS_BOTTOM_OFFSET] = o as u8;
                pos += COMPONENT_LEN;
            }
        }

        if let Some(d) = self.segmentation_duration {
            if d > crate::time::DURATION_40_MAX {
                return Err(Error::FieldOverflow(broadcast_common::len::FieldOverflow {
                    field: "segmentation_descriptor.segmentation_duration",
                    value: d,
                    max: crate::time::DURATION_40_MAX,
                }));
            }
            buf[pos + DURATION_TOP_OFFSET] = (d >> DURATION_TOP_SHIFT) as u8;
            buf[pos + DURATION_HIGH_OFFSET] = (d >> DURATION_HIGH_SHIFT) as u8;
            buf[pos + DURATION_MID_OFFSET] = (d >> DURATION_MID_SHIFT) as u8;
            buf[pos + DURATION_LOW_OFFSET] = (d >> DURATION_LOW_SHIFT) as u8;
            buf[pos + DURATION_BOTTOM_OFFSET] = d as u8;
            pos += DURATION_LEN;
        }

        buf[pos + SEGMENTATION_TYPE_ID_OFFSET] = self.segmentation_upid_type.to_u8();
        buf[pos + UPID_TYPE_LEN] = broadcast_common::len::fit_u8(
            self.segmentation_upid.len(),
            "segmentation_descriptor.segmentation_upid_length",
        )?;
        pos += UPID_HEADER_LEN;
        buf[pos..pos + self.segmentation_upid.len()].copy_from_slice(self.segmentation_upid);
        pos += self.segmentation_upid.len();

        buf[pos + SEGMENTATION_TYPE_ID_OFFSET] = self.segmentation_type_id.to_u8();
        buf[pos + SEGMENT_NUM_OFFSET] = self.segment_num;
        buf[pos + SEGMENTS_EXPECTED_OFFSET] = self.segments_expected;
        pos += TYPE_SEGMENT_LEN;

        if let Some(sub_segments) = self.sub_segments {
            if !self.segmentation_type_id.has_sub_segments() {
                return Err(Error::InvalidValue(broadcast_common::error::InvalidValue {
                    field: "segmentation_descriptor.sub_segments",
                    value: u64::from(self.segmentation_type_id.to_u8()),
                    reason: "segmentation_type_id is not one that carries sub_segment_num and sub_segments_expected (Table 23)",
                }));
            }
            buf[pos] = sub_segments.sub_segment_num;
            buf[pos + 1] = sub_segments.sub_segments_expected;
            pos += SUB_SEGMENTS_LEN;
        }
        buf[pos..pos + self.trailing.len()].copy_from_slice(self.trailing);
        pos += self.trailing.len();

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
    use alloc::vec;

    fn rt(d: &SegmentationDescriptor) {
        let bytes = d.try_to_bytes().unwrap();
        assert_eq!(bytes.len(), d.serialized_len());
        assert_eq!(bytes[0], TAG);
        // descriptor_length counts the bytes after it: identifier (4) + body.
        assert_eq!(bytes[1] as usize, 4 + d.body_len());
        let back = SegmentationDescriptor::parse(&bytes).unwrap();
        assert_eq!(*d, back);
        assert_eq!(back.try_to_bytes().unwrap(), bytes);
    }

    /// Rewrite a parsed descriptor's `descriptor_length` and CRC-free body:
    /// returns the bytes with one extra trailing byte appended inside the
    /// declared length, which is how a sender over-long for its shape
    /// appears on the wire.
    fn with_extra_trailing(bytes: &[u8]) -> alloc::vec::Vec<u8> {
        let mut out = bytes.to_vec();
        out[1] += 1;
        out.push(0xAB);
        out
    }

    /// The type-specific shapes §10.3.3.1 leaves after the fixed fields
    /// (`sub_segments`, `trailing`) must re-serialize byte-exactly whatever
    /// the wire carried.
    fn tail_is_byte_exact(bytes: &[u8]) {
        let parsed = SegmentationDescriptor::parse(bytes).expect("descriptor parses");
        assert_eq!(
            parsed.try_to_bytes().unwrap(),
            bytes,
            "byte-exact re-serialize"
        );
    }

    fn descriptor(segmentation_type_id: SegmentationTypeId) -> Vec<u8> {
        SegmentationDescriptor {
            segmentation_event_id: 0x55,
            segmentation_type_id,
            segment_num: 1,
            segments_expected: 1,
            ..Default::default()
        }
        .try_to_bytes()
        .unwrap()
    }

    /// Audit 19b, finding 15: the old parse took any two leftover bytes as
    /// `sub_segments` regardless of `segmentation_type_id` and skipped a third
    /// without keeping it. Those bytes are carried now, so every tail shape
    /// round-trips byte-exactly.
    #[test]
    fn an_over_long_descriptor_keeps_every_byte() {
        // A type that does not carry the appendix: all leftover bytes are
        // carried, never read as a sub_segment pair.
        for type_id in [
            SegmentationTypeId::ProgramStart,
            SegmentationTypeId::ProviderAdvertisementEnd,
        ] {
            let base = descriptor(type_id);
            let parsed = SegmentationDescriptor::parse(&base).unwrap();
            assert_eq!(parsed.sub_segments, None, "{type_id:?}");
            let one = with_extra_trailing(&base);
            assert_eq!(
                SegmentationDescriptor::parse(&one).unwrap().trailing.len(),
                1
            );
            tail_is_byte_exact(&one);
            let two = with_extra_trailing(&one);
            assert_eq!(
                SegmentationDescriptor::parse(&two).unwrap().trailing.len(),
                2
            );
            tail_is_byte_exact(&two);
        }
        // A type that carries the appendix: two leftover bytes are the pair,
        // a third is carried.
        let base = descriptor(SegmentationTypeId::ProviderPlacementOpportunityStart);
        let appendix = with_extra_trailing(&with_extra_trailing(&base));
        let parsed = SegmentationDescriptor::parse(&appendix).unwrap();
        assert_eq!(
            parsed.sub_segments,
            Some(SubSegments {
                sub_segment_num: 0xAB,
                sub_segments_expected: 0xAB,
            })
        );
        assert!(parsed.trailing.is_empty());
        tail_is_byte_exact(&appendix);
        let over = with_extra_trailing(&appendix);
        let parsed = SegmentationDescriptor::parse(&over).unwrap();
        assert_eq!(parsed.trailing, &[0xAB]);
        tail_is_byte_exact(&over);
    }

    /// The struct form is exact: an appendix on a type that does not carry one
    /// is rejected instead of being written for a producer to re-read as
    /// something else (or dropped).
    #[test]
    fn sub_segments_on_a_type_that_does_not_carry_them_is_rejected() {
        let d = SegmentationDescriptor {
            segmentation_event_id: 1,
            segmentation_type_id: SegmentationTypeId::ProgramStart,
            segment_num: 1,
            segments_expected: 1,
            sub_segments: Some(SubSegments {
                sub_segment_num: 1,
                sub_segments_expected: 1,
            }),
            ..Default::default()
        };
        assert!(matches!(
            d.try_to_bytes(),
            Err(Error::InvalidValue(broadcast_common::error::InvalidValue {
                field: "segmentation_descriptor.sub_segments",
                ..
            }))
        ));
        // ...and the same type with no appendix still serializes.
        rt(&SegmentationDescriptor {
            sub_segments: None,
            ..d
        });
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
            sub_segments: Some(SubSegments {
                sub_segment_num: 1,
                sub_segments_expected: 2,
            }),
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

    /// SC-W4 (#1129): `pts_offset` used to be masked with `& ((1<<33)-1)`
    /// instead of validated, silently accepting an out-of-range value.
    #[test]
    fn component_pts_offset_over_33_bits_is_rejected_not_masked() {
        let d = SegmentationDescriptor {
            segmentation_event_id: 1,
            program_segmentation_flag: false,
            components: vec![SegmentationComponent {
                component_tag: 1,
                pts_offset: 1u64 << 33,
            }],
            segmentation_type_id: SegmentationTypeId::ProgramStart,
            segment_num: 1,
            segments_expected: 1,
            ..Default::default()
        };
        assert_eq!(
            d.try_to_bytes().err(),
            Some(crate::error::Error::FieldOverflow(
                broadcast_common::len::FieldOverflow {
                    field: "segmentation_descriptor.component.pts_offset",
                    value: 1u64 << 33,
                    max: crate::time::PTS_MAX
                }
            ))
        );
    }

    /// SC-W4 (#1129): `segmentation_duration` used to be masked with
    /// `& ((1<<40)-1)` instead of validated.
    #[test]
    fn segmentation_duration_over_40_bits_is_rejected_not_masked() {
        let d = SegmentationDescriptor {
            segmentation_event_id: 1,
            program_segmentation_flag: true,
            segmentation_duration: Some(1u64 << 40),
            segmentation_type_id: SegmentationTypeId::ProgramStart,
            segment_num: 1,
            segments_expected: 1,
            ..Default::default()
        };
        assert_eq!(
            d.try_to_bytes().err(),
            Some(crate::error::Error::FieldOverflow(
                broadcast_common::len::FieldOverflow {
                    field: "segmentation_descriptor.segmentation_duration",
                    value: 1u64 << 40,
                    max: crate::time::DURATION_40_MAX
                }
            ))
        );
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
            Some(Err(Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    what: "MPU() format_identifier",
                    ..
                }
            )))
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
