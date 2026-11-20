//! Older AVS3 video descriptor of T/UWA 012.2-2023 clause 5.4.1 (Table 1), tag
//! `62` (`0x3E`) - the "Bai Cheng Qian Ping" UHD system descriptor that predates
//! the `0xD1` AVS3_video_descriptor of T/AI 109.6 / GY/T 420
//! ([`crate::descriptors::avs3_video::Avs3VideoDescriptor`]).
//!
//! It adds the library-stream references of a main AVS3 bitstream. The standard
//! gives it no private_data_specifier, so tag 62 is not auto-dispatched; enable
//! it with [`crate::descriptors::DescriptorRegistry::with_tuwa_avs3_video`]
//! (a PDS-agnostic registration of tag 0x3E).
//!
//! ```text
//! descriptor_tag(8)=62 descriptor_length(8)
//! profile_id(8) level_id(8)
//! multiple_frame_rate_flag(1) frame_rate_code(4) sample_precision(3)
//! chroma_format(2) temporal_id_flag(1) td_mode_flag(1) library_stream_flag(1) reserved(3)
//! colour_primaries(8) transfer_characteristics(8) matrix_coefficients(8)
//! if !library_stream_flag:
//!   num_ref_library_stream(7) id_type_flag(1)
//!   for each reference (2 bytes):
//!     if id_type_flag: ref_library_stream_PEID(13) reserved(3)
//!     else:            ref_library_stream_id(8) reserved(5) reserved(3)
//! ```
//!
//! The vendored text (an English machine translation) prints the closing
//! `reserved(3)` inside the reference loop for both branches (verified on the
//! rendered page), which makes every reference exactly 16 bits. The
//! standard states no value for `reserved`; captured on parse, all ones when
//! hand-built.

use super::avs3_video::Avs3FrameRateCode;
use super::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::len::{fit_bits, need_in, need_out};
use broadcast_common::reserved::ReservedOnes;
use broadcast_common::{Parse, Serialize};
use mpeg_ts::pid::Pid;

/// Descriptor tag of the T/UWA 012.2 AVS3 video descriptor (62).
pub const TAG: u8 = 62;
const HEADER_LEN: usize = 2;
const FIXED_BODY_LEN: usize = 7;
const REF_LEN: usize = 2;
const MAX_REFS: usize = 127;
/// Width of a `reserved(3)` field.
const RESERVED_3_BITS: u32 = 3;
/// Width of a `reserved(5)` field.
const RESERVED_5_BITS: u32 = 5;

/// One library-stream reference (the form is selected by `id_type_flag`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum TuwaLibraryRef {
    /// `id_type_flag == 1`: `ref_library_stream_PEID` (13 bits) + `reserved` (3).
    Peid {
        /// PID of the TS packets carrying the library bitstream.
        pid: Pid,
        /// `reserved` (3 bits).
        reserved: ReservedOnes<RESERVED_3_BITS>,
    },
    /// `id_type_flag == 0`: `ref_library_stream_id` (8) + `reserved` (5) + `reserved` (3).
    StreamId {
        /// `stream_id` of the PES carrying the library bitstream.
        stream_id: u8,
        /// `reserved` (5 bits).
        reserved_5: ReservedOnes<RESERVED_5_BITS>,
        /// `reserved` (3 bits).
        reserved_3: ReservedOnes<RESERVED_3_BITS>,
    },
}

impl TuwaLibraryRef {
    /// A stream_id reference with the all-ones reserved defaults.
    #[must_use]
    pub const fn stream_id(stream_id: u8) -> Self {
        Self::StreamId {
            stream_id,
            reserved_5: ReservedOnes::<RESERVED_5_BITS>::FILL,
            reserved_3: ReservedOnes::<RESERVED_3_BITS>::FILL,
        }
    }
}

impl TuwaLibraryRef {
    /// Label of the reference form (`id_type_flag`).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Peid { .. } => "PEID",
            Self::StreamId { .. } => "stream_id",
        }
    }
}
broadcast_common::impl_spec_display!(TuwaLibraryRef);

/// The references of a main (non-library) stream. All entries share one form.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct TuwaLibraryRefs {
    /// The references; `num_ref_library_stream` is their count, `id_type_flag`
    /// is derived from the entries' form (all must match).
    pub refs: Vec<TuwaLibraryRef>,
    /// `id_type_flag` to use when `refs` is empty (when non-empty it is derived).
    pub id_type_flag_if_empty: bool,
}

/// T/UWA 012.2-2023 AVS3 video descriptor (Table 1).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct TuwaAvs3VideoDescriptor {
    /// `profile_id` (8 bits).
    pub profile_id: u8,
    /// `level_id` (8 bits).
    pub level_id: u8,
    /// `multiple_frame_rate_flag` (1 bit).
    pub multiple_frame_rate_flag: bool,
    /// `frame_rate_code` (4 bits, same table as T/AI 109.2).
    pub frame_rate_code: Avs3FrameRateCode,
    /// `sample_precision` (3 bits).
    pub sample_precision: u8,
    /// `chroma_format` (2 bits).
    pub chroma_format: u8,
    /// `temporal_id_flag` (1 bit).
    pub temporal_id_flag: bool,
    /// `td_mode_flag` (1 bit).
    pub td_mode_flag: bool,
    /// `library_stream_flag` (1 bit): this ES is a library stream (then no references follow).
    pub library_stream_flag: bool,
    /// `reserved` (3 bits) after `library_stream_flag`.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved: ReservedOnes<RESERVED_3_BITS>,
    /// `colour_primaries` (8 bits).
    pub colour_primaries: u8,
    /// `transfer_characteristics` (8 bits).
    pub transfer_characteristics: u8,
    /// `matrix_coefficients` (8 bits).
    pub matrix_coefficients: u8,
    /// Library references; `Some` iff `library_stream_flag` is false.
    pub library_refs: Option<TuwaLibraryRefs>,
}

fn bit(b: u8, shift: u32) -> bool {
    (b >> shift) & 1 != 0
}

impl<'a> Parse<'a> for TuwaAvs3VideoDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "TuwaAvs3VideoDescriptor",
            "unexpected tag for T/UWA 012.2 AVS3 video descriptor",
        )?;
        need_in(body, FIXED_BODY_LEN, "TuwaAvs3VideoDescriptor")?;
        let (b2, b3) = (body[2], body[3]);
        let library_stream_flag = bit(b3, 3);
        let library_refs = if library_stream_flag {
            if body.len() != FIXED_BODY_LEN {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "library stream descriptor has trailing bytes",
                });
            }
            None
        } else {
            let head = *body.get(FIXED_BODY_LEN).ok_or(Error::BufferTooShort(
                broadcast_common::error::BufferTooShort {
                    need: FIXED_BODY_LEN + 1,
                    have: body.len(),
                    what: "TuwaAvs3VideoDescriptor",
                },
            ))?;
            let n = usize::from(head >> 1);
            let id_type = head & 1 != 0;
            let rest = &body[FIXED_BODY_LEN + 1..];
            if rest.len() != n * REF_LEN {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "num_ref_library_stream does not match descriptor_length",
                });
            }
            let refs = rest
                .chunks_exact(REF_LEN)
                .map(|c| {
                    if id_type {
                        TuwaLibraryRef::Peid {
                            pid: Pid::from_wire_masked(u16::from_be_bytes([c[0], c[1]]) >> 3),
                            reserved: ReservedOnes::from_bits(u64::from(c[1] & 0x07)),
                        }
                    } else {
                        TuwaLibraryRef::StreamId {
                            stream_id: c[0],
                            reserved_5: ReservedOnes::from_bits(u64::from(c[1] >> 3)),
                            reserved_3: ReservedOnes::from_bits(u64::from(c[1] & 0x07)),
                        }
                    }
                })
                .collect();
            Some(TuwaLibraryRefs {
                refs,
                id_type_flag_if_empty: id_type,
            })
        };
        Ok(Self {
            profile_id: body[0],
            level_id: body[1],
            multiple_frame_rate_flag: bit(b2, 7),
            frame_rate_code: Avs3FrameRateCode::from_u8((b2 >> 3) & 0x0F),
            sample_precision: b2 & 0x07,
            chroma_format: b3 >> 6,
            temporal_id_flag: bit(b3, 5),
            td_mode_flag: bit(b3, 4),
            library_stream_flag,
            reserved: ReservedOnes::from_bits(u64::from(b3 & 0x07)),
            colour_primaries: body[4],
            transfer_characteristics: body[5],
            matrix_coefficients: body[6],
            library_refs,
        })
    }
}

impl TuwaAvs3VideoDescriptor {
    fn body_len(&self) -> usize {
        match &self.library_refs {
            Some(r) => FIXED_BODY_LEN + 1 + r.refs.len() * REF_LEN,
            None => FIXED_BODY_LEN,
        }
    }
}

impl Serialize for TuwaAvs3VideoDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + self.body_len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        need_out(buf, len)?;
        if self.library_stream_flag != self.library_refs.is_none() {
            return Err(Error::ValueOutOfRange {
                field: "library_refs",
                reason: "present iff library_stream_flag is false",
            });
        }
        let frc = fit_bits(
            u64::from(self.frame_rate_code.to_u8()),
            4,
            "frame_rate_code",
        )? as u8;
        let sp = fit_bits(u64::from(self.sample_precision), 3, "sample_precision")? as u8;
        let cf = fit_bits(u64::from(self.chroma_format), 2, "chroma_format")? as u8;
        let rsv = self.reserved.get() as u8;
        write_descriptor_header(buf, TAG, self.body_len())?;
        buf[2] = self.profile_id;
        buf[3] = self.level_id;
        buf[4] = (u8::from(self.multiple_frame_rate_flag) << 7) | (frc << 3) | sp;
        buf[5] = (cf << 6)
            | (u8::from(self.temporal_id_flag) << 5)
            | (u8::from(self.td_mode_flag) << 4)
            | (u8::from(self.library_stream_flag) << 3)
            | rsv;
        buf[6] = self.colour_primaries;
        buf[7] = self.transfer_characteristics;
        buf[8] = self.matrix_coefficients;
        let mut i = HEADER_LEN + FIXED_BODY_LEN;
        if let Some(r) = &self.library_refs {
            if r.refs.len() > MAX_REFS {
                return Err(Error::ValueOutOfRange {
                    field: "num_ref_library_stream",
                    reason: "exceeds 7-bit field",
                });
            }
            let id_type = match r.refs.first() {
                Some(TuwaLibraryRef::Peid { .. }) => true,
                Some(TuwaLibraryRef::StreamId { .. }) => false,
                None => r.id_type_flag_if_empty,
            };
            let n = fit_bits(r.refs.len() as u64, 7, "num_ref_library_stream")? as u8;
            buf[i] = (n << 1) | u8::from(id_type);
            i += 1;
            for entry in &r.refs {
                match (*entry, id_type) {
                    (TuwaLibraryRef::Peid { pid, reserved }, true) => {
                        let rs = reserved.get() as u16;
                        let v = (pid.value() << 3) | rs;
                        buf[i..i + 2].copy_from_slice(&v.to_be_bytes());
                    }
                    (
                        TuwaLibraryRef::StreamId {
                            stream_id,
                            reserved_5,
                            reserved_3,
                        },
                        false,
                    ) => {
                        let r5 = reserved_5.get() as u8;
                        let r3 = reserved_3.get() as u8;
                        buf[i] = stream_id;
                        buf[i + 1] = (r5 << 3) | r3;
                    }
                    _ => {
                        return Err(Error::ValueOutOfRange {
                            field: "library_refs",
                            reason: "all references must use the same id_type_flag form",
                        });
                    }
                }
                i += REF_LEN;
            }
        }
        Ok(i)
    }
}

impl crate::traits::DescriptorDef<'_> for TuwaAvs3VideoDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "TUWA_AVS3_VIDEO";
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn round_trip(bytes: &[u8]) -> TuwaAvs3VideoDescriptor {
        let d = TuwaAvs3VideoDescriptor::parse(bytes).unwrap();
        let mut out = vec![0u8; d.serialized_len()];
        let n = d.serialize_into(&mut out).unwrap();
        assert_eq!(&out[..n], bytes, "byte-exact round trip");
        d
    }

    /// T/UWA 012.2-2023 Table 1, library stream (`library_stream_flag` = 1): no
    /// reference block; profile 0x22, level 0x42, 30 Hz with multiple rates,
    /// sample_precision 2, chroma 1, library_stream 1, colour/transfer/matrix 9/14/9.
    #[test]
    fn library_stream_has_no_references() {
        let bytes = [TAG, 7, 0x22, 0x42, 0xAA, 0x4F, 9, 14, 9];
        let d = round_trip(&bytes);
        assert!(d.library_stream_flag && d.library_refs.is_none());
        assert_eq!(d.frame_rate_code, Avs3FrameRateCode::Fps30);
        assert_eq!(
            (
                d.colour_primaries,
                d.transfer_characteristics,
                d.matrix_coefficients
            ),
            (9, 14, 9)
        );
        assert_eq!(d.reserved, ReservedOnes::FILL);
    }

    /// Main stream, `id_type_flag` = 1: two PEID references (0x0101, 0x1FFF).
    #[test]
    fn main_stream_with_peid_references() {
        // num_ref=2 (7 bits) id_type=1 -> 0b0000_0101; PEID 0x0101<<3|7 = 0x080F; 0x1FFF<<3|7 = 0xFFFF
        let bytes = [
            TAG, 12, 0x22, 0x42, 0xAA, 0x47, 9, 14, 9, 0x05, 0x08, 0x0F, 0xFF, 0xFF,
        ];
        let d = round_trip(&bytes);
        let refs = d.library_refs.as_ref().unwrap();
        assert_eq!(refs.refs.len(), 2);
        assert_eq!(
            refs.refs[0],
            TuwaLibraryRef::Peid {
                pid: Pid::from_const(0x0101),
                reserved: ReservedOnes::from_bits(7)
            }
        );
        assert_eq!(
            refs.refs[1],
            TuwaLibraryRef::Peid {
                pid: Pid::from_const(0x1FFF),
                reserved: ReservedOnes::from_bits(7)
            }
        );
    }

    /// Main stream, `id_type_flag` = 0: one stream_id reference (0xFD ext not
    /// part of this field; the PES stream_id 0xE1), reserved 5 + 3 all ones.
    #[test]
    fn main_stream_with_stream_id_reference() {
        let bytes = [TAG, 10, 0x22, 0x42, 0xAA, 0x47, 9, 14, 9, 0x02, 0xE1, 0xFF];
        let d = round_trip(&bytes);
        let refs = d.library_refs.as_ref().unwrap();
        assert_eq!(refs.refs, vec![TuwaLibraryRef::stream_id(0xE1)]);
        assert!(!refs.id_type_flag_if_empty);
    }

    #[test]
    fn rejects_bad_tag_length_and_count_mismatch() {
        assert!(matches!(
            TuwaAvs3VideoDescriptor::parse(&[0x3F, 7, 0, 0, 0, 0, 0, 0, 0]),
            Err(Error::InvalidDescriptor { tag: 0x3F, .. })
        ));
        assert!(matches!(
            TuwaAvs3VideoDescriptor::parse(&[TAG, 6, 0, 0, 0, 0, 0, 0]),
            Err(Error::BufferTooShort(..))
        ));
        // main stream with no reference header byte
        assert!(matches!(
            TuwaAvs3VideoDescriptor::parse(&[TAG, 7, 0, 0, 0, 0x40, 0, 0, 0]),
            Err(Error::BufferTooShort(..))
        ));
        // num_ref=2 but only one reference present
        assert!(matches!(
            TuwaAvs3VideoDescriptor::parse(&[TAG, 10, 0, 0, 0, 0x40, 0, 0, 0, 0x05, 0, 0]),
            Err(Error::InvalidDescriptor { tag: TAG, .. })
        ));
        // library stream with trailing bytes
        assert!(matches!(
            TuwaAvs3VideoDescriptor::parse(&[TAG, 8, 0, 0, 0, 0x08, 0, 0, 0, 0]),
            Err(Error::InvalidDescriptor { tag: TAG, .. })
        ));
    }

    #[test]
    fn serialize_rejects_inconsistent_descriptors() {
        let bytes = [
            TAG, 12, 0x22, 0x42, 0xAA, 0x47, 9, 14, 9, 0x05, 0x08, 0x0F, 0xFF, 0xFF,
        ];
        let mut d = TuwaAvs3VideoDescriptor::parse(&bytes).unwrap();
        // mixed reference forms
        d.library_refs.as_mut().unwrap().refs[1] = TuwaLibraryRef::stream_id(1);
        assert!(matches!(
            &(d.serialize_into(&mut [0u8; 32])),
            Err(crate::error::Error::ValueOutOfRange { .. })
        ));
        // flag/refs disagreement
        let mut d = TuwaAvs3VideoDescriptor::parse(&bytes).unwrap();
        d.library_stream_flag = true;
        assert!(matches!(
            d.serialize_into(&mut [0u8; 32]),
            Err(Error::ValueOutOfRange { .. })
        ));
        // the reserved field is now a 3-bit ReservedOnes, so an over-wide value
        // is unrepresentable; from_bits keeps the low three bits.
        let d = TuwaAvs3VideoDescriptor::parse(&bytes).unwrap();
        assert_eq!(
            TuwaAvs3VideoDescriptor {
                reserved: ReservedOnes::from_bits(8),
                ..d
            }
            .reserved
            .get(),
            0
        );
        // too many references
        let mut d = TuwaAvs3VideoDescriptor::parse(&bytes).unwrap();
        d.library_refs.as_mut().unwrap().refs = vec![
            TuwaLibraryRef::Peid {
                pid: Pid::from_const(1),
                reserved: ReservedOnes::from_bits(7)
            };
            128
        ];
        assert!(matches!(
            &(d.serialize_into(&mut [0u8; 300])),
            Err(crate::error::Error::FieldOverflow(_))
        ));
    }

    /// Each reserved field is width-limited by its type, so a wider value is
    /// unrepresentable and cannot spill into the adjacent syntax bits.
    #[test]
    fn reserved_fields_cannot_hold_more_than_their_width() {
        let peid = TuwaLibraryRef::Peid {
            pid: Pid::from_const(1),
            reserved: ReservedOnes::from_bits(0xFF),
        };
        let id = TuwaLibraryRef::StreamId {
            stream_id: 0,
            reserved_5: ReservedOnes::from_bits(0xFF),
            reserved_3: ReservedOnes::from_bits(0xFF),
        };
        if let TuwaLibraryRef::Peid { reserved, .. } = peid {
            assert_eq!(reserved.get(), 0x07);
        }
        if let TuwaLibraryRef::StreamId {
            reserved_5,
            reserved_3,
            ..
        } = id
        {
            assert_eq!(reserved_5.get(), 0x1F);
            assert_eq!(reserved_3.get(), 0x07);
        }
    }

    #[test]
    fn library_ref_labels_name_the_id_type() {
        let peid = TuwaLibraryRef::Peid {
            pid: Pid::from_const(0x0101),
            reserved: ReservedOnes::<RESERVED_3_BITS>::FILL,
        };
        assert_eq!(peid.name(), "PEID");
        assert_eq!(peid.to_string(), "PEID");
        assert_eq!(TuwaLibraryRef::stream_id(0xE1).name(), "stream_id");
        assert_eq!(TuwaLibraryRef::stream_id(0xE1).to_string(), "stream_id");
    }

    /// A main-stream body of exactly the fixed length is missing its reference
    /// count byte, and the error names that byte.
    #[test]
    fn missing_reference_count_reports_the_next_byte_needed() {
        // b3 bit 3 clear: not a library stream, so a count byte must follow.
        let bytes = [TAG, 7, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00];
        match TuwaAvs3VideoDescriptor::parse(&bytes) {
            Err(Error::BufferTooShort(b)) => {
                assert_eq!(b.need, FIXED_BODY_LEN + 1);
                assert_eq!(b.need, 8);
                assert_eq!(b.have, FIXED_BODY_LEN);
            }
            other => panic!("expected BufferTooShort, got {other:?}"),
        }
    }

    /// The reference count is 7 bits but the 8-bit descriptor_length caps the
    /// reachable count first: 123 refs (254-byte body) is the widest that
    /// serializes, and 124 is already rejected by the length field.
    #[test]
    fn reference_count_is_capped_by_the_descriptor_length() {
        let descriptor = |refs: usize| TuwaAvs3VideoDescriptor {
            profile_id: 0x22,
            level_id: 0x42,
            multiple_frame_rate_flag: false,
            frame_rate_code: Avs3FrameRateCode::Fps30,
            sample_precision: 2,
            chroma_format: 1,
            temporal_id_flag: false,
            td_mode_flag: false,
            library_stream_flag: false,
            reserved: ReservedOnes::<RESERVED_3_BITS>::FILL,
            colour_primaries: 9,
            transfer_characteristics: 14,
            matrix_coefficients: 9,
            library_refs: Some(TuwaLibraryRefs {
                refs: (0..refs)
                    .map(|i| TuwaLibraryRef::stream_id(i as u8))
                    .collect(),
                id_type_flag_if_empty: false,
            }),
        };

        let ok = descriptor(123);
        let mut out = vec![0u8; ok.serialized_len()];
        ok.serialize_into(&mut out).unwrap();
        assert_eq!(out[1], 254);
        assert_eq!(TuwaAvs3VideoDescriptor::parse(&out).unwrap(), ok);

        for count in [124usize, 127] {
            let bad = descriptor(count);
            assert!(matches!(
                bad.serialize_into(&mut vec![0u8; bad.serialized_len()]),
                Err(Error::FieldOverflow(_))
            ));
        }
    }
}
