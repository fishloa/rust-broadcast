//! ProducerReferenceTimeBox, SampleGroupDescriptionBox, SampleToGroupBox, and
//! SubSampleInformationBox — ISO/IEC 14496-12:2026 §8.14.5, §8.9.2, §8.9.3,
//! §8.7.7.
//!
//! Typed containers for:
//!
//! | Box   | Name                           | Section   | Description                                       |
//! |-------|--------------------------------|-----------|---------------------------------------------------|
//! | `prft`| ProducerReferenceTimeBox       | §8.14.5   | NTP wall-clock anchor for the reference track     |
//! | `sgpd`| SampleGroupDescriptionBox      | §8.9.3    | Per-grouping-type sample group description table  |
//! | `sbgp`| SampleToGroupBox               | §8.9.2    | Maps samples to sample group description indices  |
//! | `subs`| SubSampleInformationBox        | §8.7.7    | Per-sample sub-sample size/priority table         |
//!
//! All boxes are FullBoxes. Sizes are computed from fields; no `self.raw`
//! passthrough in any serializer.
//!
//! # Spec citations
//!
//! - **prft**: ISO/IEC 14496-12:2026 §8.14.5.2 — version 0: `media_time` is u32;
//!   version 1: `media_time` is u64.
//! - **sgpd**: ISO/IEC 14496-12:2026 §8.9.3.2 — version >= 1: `default_length`
//!   present; version >= 2: `default_group_description_index` also present;
//!   version 3 is an essential group ([`SampleGroupDescriptionBox::is_essential`]);
//!   version 0 is deprecated. `'roll'` (RollRecoveryEntry) and the
//!   ISO/IEC 23008-12:2025 image-sequence and region entries (`'eqiv'`, the
//!   five bracketing types, `'pano'`, `'refs'`, `'regm'`, `'udes'`) are typed;
//!   every other grouping type is stored as raw bytes via
//!   [`SgpdEntry::Unknown`].
//! - **sbgp**: ISO/IEC 14496-12:2026 §8.9.2.2 — version 0: no
//!   `grouping_type_parameter`; version 1: `grouping_type_parameter` present.
//! - **subs**: ISO/IEC 14496-12:2026 §8.7.7.2 — version 0: `subsample_size` is
//!   u16; version 1: `subsample_size` is u32; version 2 (the reference form,
//!   [`SubSampleInformationBoxV2`]) pairs with the `ssrt` box (§8.7.10,
//!   [`SubSampleReferenceTableBox`]).

use crate::TrackId;
use crate::box_support::bounded_entry_count;
use crate::box_types::{
    BOX_HEADER_MIN_SIZE, BOX_TYPE_OFFSET, FULLBOX_EXTRA_SIZE, FULLBOX_FLAGS_MASK,
};
use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_core::error::{BufferTooShort, InvalidValue};
use broadcast_core::len::{need_in, need_out};
use broadcast_core::wire::{be_u16, be_u32, be_u64, i16_at};

use broadcast_core::{Parse, Serialize};

// ---------------------------------------------------------------------------
// Wire-layout constants
// ---------------------------------------------------------------------------

const U32_PAIR_LEN: usize = core::mem::size_of::<u64>();
const SGPD_ENTRY_PREFIX_LEN: usize = core::mem::size_of::<u32>();
const SUBS_ENTRY_HEADER_LEN: usize = core::mem::size_of::<u32>() + core::mem::size_of::<u16>();
const SUBS_V2_ENTRY_HEADER_LEN: usize = core::mem::size_of::<u32>() + core::mem::size_of::<u16>();

/// Four-CC of the Producer Reference Time Box (`prft`, ISO/IEC 14496-12:2026 §8.14.5).
pub const PRFT: [u8; 4] = *b"prft";
const PRFT_TYPE: u32 = u32::from_be_bytes(PRFT);
const SGPD_TYPE: u32 = u32::from_be_bytes(*b"sgpd");
const SBGP_TYPE: u32 = u32::from_be_bytes(*b"sbgp");
const SUBS_TYPE: u32 = u32::from_be_bytes(*b"subs");
const SSRT_TYPE: u32 = u32::from_be_bytes(*b"ssrt");

/// First `sgpd` version with `default_length` and per-entry `description_length` — ISO/IEC 14496-12:2026 §8.9.3.2.
pub const SGPD_VERSION_DEFAULT_LENGTH: u8 = 1;
/// First `sgpd` version with `default_group_description_index` — §8.9.3.2.
pub const SGPD_VERSION_GROUP_INDEX: u8 = 2;
/// `sgpd` version marking an essential sample group description — §8.9.3.1.
pub const SGPD_VERSION_ESSENTIAL: u8 = 3;

/// First `subs` version that uses the reference form — ISO/IEC 14496-12:2026 §8.7.7.2.
pub const SUBS_VERSION_REFERENCE: u8 = 2;

const HAS_REFERENCE_BIT: u8 = 0x80;
const SUBS_SIZE16_BIT: u8 = 0x80;
const SUBS_SHORT_REF_BIT: u8 = 0x40;
const PROP_DISCARDABLE_BIT: u8 = 0x40;
const PROP_HAS_PRIORITY_BIT: u8 = 0x20;
const PROP_HAS_CSP_BIT: u8 = 0x10;
const PROP_CSP_LEN_SHIFT: u32 = 2;
const PROP_CSP_LEN_MASK: u8 = 0x03;
/// The six reserved bits of a version-2 `subs` byte after `entry_count` —
/// ISO/IEC 14496-12:2026 §8.7.7.3.
const SUBS_CFG_RESERVED_MASK: u8 = 0x3F;
/// The two reserved bits closing a version-2 `subs` property byte — §8.7.7.3.
const PROP_RESERVED_MASK: u8 = 0x03;
const SSRT_PROP_DISCARDABLE_BIT: u8 = 0x80;
const SSRT_PROP_HAS_PRIORITY_BIT: u8 = 0x40;
const SSRT_PROP_HAS_CSP_BIT: u8 = 0x20;
const SSRT_PROP_CSP_LEN_SHIFT: u32 = 3;
/// The three reserved bits closing an `ssrt` entry byte — §8.7.10.
const SSRT_PROP_RESERVED_MASK: u8 = 0x07;
const SHORT_REF_BITS: u32 = 7;
const LONG_REF_BITS: u32 = 15;
const MAX_CSP_LEN: usize = 4;

/// Grouping type `'roll'` (RollRecoveryEntry) — ISO/IEC 14496-12:2026 §10.1.
pub const GROUPING_TYPE_ROLL: u32 = u32::from_be_bytes(*b"roll");

/// Grouping type `'prol'` (AudioPreRollEntry) — ISO/IEC 14496-12:2026 §10.1.
pub const GROUPING_TYPE_PROL: u32 = u32::from_be_bytes(*b"prol");

/// Grouping type `'seig'` (CencSampleEncryptionInformationGroupEntry) —
/// ISO/IEC 23001-7 (CENC). Maps a run of samples to a KID/IV-size/pattern
/// override distinct from the track's `tenc` default, i.e. per-sample-group
/// key rotation within a single track. Not parsed as a typed [`SgpdEntry`]
/// variant by this module (it carries CENC-specific fields `tenc.rs` would
/// need to interpret, not just a bare distance/count); exposed here purely
/// as the FourCC constant a decrypt/encrypt path can test `grouping_type`
/// against to detect the presence of key-rotation content it does not yet
/// implement, rather than silently decrypting every sample with only the
/// track default key (see `transmux::cenc_decrypt`, issue #990).
pub const GROUPING_TYPE_SEIG: u32 = u32::from_be_bytes(*b"seig");

/// Grouping type `'eqiv'` (VisualEquivalenceEntry) —
/// ISO/IEC 23008-12:2025 6.8.1.2.
pub const GROUPING_TYPE_EQIV: u32 = u32::from_be_bytes(*b"eqiv");
/// Grouping type `'aebr'` (AutoExposureBracketingEntry) —
/// ISO/IEC 23008-12:2025 6.8.6.2.2.
pub const GROUPING_TYPE_AEBR: u32 = u32::from_be_bytes(*b"aebr");
/// Grouping type `'wbbr'` (WhiteBalanceBracketingEntry) — 6.8.6.3.2.
pub const GROUPING_TYPE_WBBR: u32 = u32::from_be_bytes(*b"wbbr");
/// Grouping type `'fobr'` (FocusBracketingEntry) — 6.8.6.4.2.
pub const GROUPING_TYPE_FOBR: u32 = u32::from_be_bytes(*b"fobr");
/// Grouping type `'afbr'` (FlashExposureBracketingEntry) — 6.8.6.5.2.
pub const GROUPING_TYPE_AFBR: u32 = u32::from_be_bytes(*b"afbr");
/// Grouping type `'dobr'` (DepthOfFieldBracketingEntry) — 6.8.6.6.2.
pub const GROUPING_TYPE_DOBR: u32 = u32::from_be_bytes(*b"dobr");
/// Grouping type `'pano'` (PanoramaEntry) — 6.8.8.2.
pub const GROUPING_TYPE_PANO: u32 = u32::from_be_bytes(*b"pano");
/// Grouping type `'refs'` (DirectReferenceSamplesList) — 7.4.1.
pub const GROUPING_TYPE_REFS: u32 = u32::from_be_bytes(*b"refs");
/// Grouping type `'regm'` (region-to-annotation mapping) — 11.4.3.1.
pub const GROUPING_TYPE_REGM: u32 = u32::from_be_bytes(*b"regm");
/// Grouping type `'udes'` (UserDescriptionSampleGroupEntry) — 11.4.3.2.
pub const GROUPING_TYPE_UDES: u32 = u32::from_be_bytes(*b"udes");

/// Bytes of an `'eqiv'` entry: `time_offset` + `timescale_multiplier`.
const EQIV_ENTRY_LEN: usize = 4;
/// Bytes of an `'aebr'` entry: `exposure_step` + `exposure_numerator`.
const AEBR_ENTRY_LEN: usize = 2;
/// Bytes of a `'wbbr'` entry: `blue_amber` + `green_magenta`.
const WBBR_ENTRY_LEN: usize = 3;
/// Bytes of a `'fobr'` entry: `focus_distance_numerator` + denominator.
const FOBR_ENTRY_LEN: usize = 4;
/// Bytes of an `'afbr'` entry: `flash_exposure_numerator` + denominator.
const AFBR_ENTRY_LEN: usize = 2;
/// Bytes of a `'dobr'` entry: `f_stop_numerator` + denominator.
const DOBR_ENTRY_LEN: usize = 2;
/// Bytes of a `'pano'` entry: `frame_number`.
const PANO_ENTRY_LEN: usize = 2;
/// Offset of a second 16-bit field in an entry.
const SECOND_U16_OFFSET: usize = core::mem::size_of::<u16>();

// ---------------------------------------------------------------------------
// ProducerReferenceTimeBox — prft (ISO/IEC 14496-12:2026 §8.14.5)
// ---------------------------------------------------------------------------

/// Producer Reference Time Box (`prft`) — ISO/IEC 14496-12:2026 §8.14.5.2.
///
/// Provides a UTC wall-clock anchor for the reference track.
///
/// Wire layout (FullBox header omitted):
///
/// ```text
/// reference_track_ID   u(32)
/// ntp_timestamp        u(64)  — UTC time in NTP format
/// media_time           u(32) if version == 0
///                      u(64) if version == 1
/// ```
///
/// `reference_track_ID` identifies the track whose decoding timeline is anchored.
/// `ntp_timestamp` is the wall-clock time in NTP format corresponding to
/// `media_time`. `media_time` is in the timescale of the reference track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ProducerReferenceTimeBox {
    /// FullBox version: 0 = `media_time` is u32; 1 = `media_time` is u64.
    pub version: u8,
    /// FullBox flags `[23:0]`.
    pub flags: u32,
    /// Track ID of the reference track (§8.14.5.3).
    pub reference_track_id: TrackId,
    /// UTC time in NTP format (§8.14.5.3).
    pub ntp_timestamp: u64,
    /// Media time in the reference track's timescale.
    ///
    /// Stored as u64; for version 0 the upper 32 bits are zero on the wire.
    pub media_time: u64,
}

impl ProducerReferenceTimeBox {
    /// Parse the body of a `prft` box (after the 8-byte BoxHeader).
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        // version(1) + flags(3) + ref_track_id(4) + ntp(8) = 16 minimum
        let min = FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>() + core::mem::size_of::<u64>();
        need_in(body, min, "prft body")?;
        let version = body[0];
        let flags = be_u32(body, 0, "full box header")? & FULLBOX_FLAGS_MASK;
        let mut c = FULLBOX_EXTRA_SIZE;
        let reference_track_id = TrackId::new(be_u32(body, c, "prft reference_track_id")?);
        c += core::mem::size_of::<u32>();
        let ntp_timestamp = be_u64(body, c, "prft ntp_timestamp")?;
        c += core::mem::size_of::<u64>();
        let media_time = if version == 0 {
            need_in(body, c + core::mem::size_of::<u32>(), "prft media_time v0")?;
            u64::from(be_u32(body, c, "prft media_time v0")?)
        } else {
            need_in(body, c + core::mem::size_of::<u64>(), "prft media_time v1")?;
            be_u64(body, c, "prft media_time v1")?
        };
        Ok(Self {
            version,
            flags,
            reference_track_id,
            ntp_timestamp,
            media_time,
        })
    }
}

impl<'a> Parse<'a> for ProducerReferenceTimeBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(
            bytes,
            BOX_HEADER_MIN_SIZE
                + FULLBOX_EXTRA_SIZE
                + core::mem::size_of::<u32>()
                + core::mem::size_of::<u64>()
                + core::mem::size_of::<u32>(),
            "prft box",
        )?;
        let ty = be_u32(bytes, BOX_TYPE_OFFSET, "box header")?;
        if ty != PRFT_TYPE {
            return Err(Error::InvalidValue(InvalidValue {
                field: "box_type",
                value: ty as u64,
                reason: "expected prft",
            }));
        }
        Self::parse_body(crate::box_types::parse_box(bytes)?.0.body)
    }
}

impl Serialize for ProducerReferenceTimeBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mt_size = if self.version == 0 {
            core::mem::size_of::<u32>()
        } else {
            core::mem::size_of::<u64>()
        };
        BOX_HEADER_MIN_SIZE
            + FULLBOX_EXTRA_SIZE
            + core::mem::size_of::<u32>()
            + core::mem::size_of::<u64>()
            + mt_size
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = 0;
        c += crate::box_types::write_full_box_header(
            buf,
            b"prft",
            need,
            self.version,
            self.flags,
            true,
        )?;
        buf[c..c + core::mem::size_of::<u32>()]
            .copy_from_slice(&self.reference_track_id.value().to_be_bytes());
        c += core::mem::size_of::<u32>();
        buf[c..c + core::mem::size_of::<u64>()].copy_from_slice(&self.ntp_timestamp.to_be_bytes());
        c += core::mem::size_of::<u64>();
        if self.version == 0 {
            buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(
                &(broadcast_core::len::fit_bits(self.media_time, u32::BITS, "prft media_time")?
                    as u32)
                    .to_be_bytes(),
            );
            c += core::mem::size_of::<u32>();
        } else {
            buf[c..c + core::mem::size_of::<u64>()].copy_from_slice(&self.media_time.to_be_bytes());
            c += core::mem::size_of::<u64>();
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// SampleGroupDescriptionBox — sgpd (ISO/IEC 14496-12:2026 §8.9.3)
// ---------------------------------------------------------------------------

/// A parsed entry in the sgpd sample group description table.
///
/// Only `'roll'` (§10.1 RollRecoveryEntry) is fully typed; all other grouping
/// types carry their body as raw bytes in [`SgpdEntry::Unknown`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum SgpdEntry {
    /// RollRecoveryEntry for grouping type `'roll'` (§10.1).
    ///
    /// `roll_distance` is the number of samples that must be decoded before the
    /// stream is usable (negative = pre-roll, positive = post-roll).
    Roll {
        /// Roll distance in samples. Negative = pre-roll.
        roll_distance: i16,
    },
    /// AudioPreRollEntry for grouping type `'prol'` (§10.1): samples that are
    /// independently decodable, whose decoder output is correct only after
    /// decoding the stated number of samples.
    Prol {
        /// Roll distance in samples, positive for the pre-roll count.
        roll_distance: i16,
    },
    /// VisualEquivalenceEntry for grouping type `'eqiv'` (23008-12 6.8.1.2).
    Eqiv {
        /// `time_offset`, in the timescale `timescale_multiplier` states.
        time_offset: i16,
        /// `timescale_multiplier`, an 8.8 fixed-point value.
        timescale_multiplier: u16,
    },
    /// AutoExposureBracketingEntry for `'aebr'` (6.8.6.2.2).
    Aebr {
        /// `exposure_step`.
        exposure_step: i8,
        /// `exposure_numerator`.
        exposure_numerator: i8,
    },
    /// WhiteBalanceBracketingEntry for `'wbbr'` (6.8.6.3.2).
    Wbbr {
        /// `blue_amber`, in Kelvin.
        blue_amber: u16,
        /// `green_magenta`, in 1/100 Duv.
        green_magenta: i8,
    },
    /// FocusBracketingEntry for `'fobr'` (6.8.6.4.2).
    Fobr {
        /// `focus_distance_numerator`.
        focus_distance_numerator: u16,
        /// `focus_distance_denominator`.
        focus_distance_denominator: u16,
    },
    /// FlashExposureBracketingEntry for `'afbr'` (6.8.6.5.2).
    Afbr {
        /// `flash_exposure_numerator`.
        flash_exposure_numerator: i8,
        /// `flash_exposure_denominator`.
        flash_exposure_denominator: i8,
    },
    /// DepthOfFieldBracketingEntry for `'dobr'` (6.8.6.6.2).
    Dobr {
        /// `f_stop_numerator`.
        f_stop_numerator: i8,
        /// `f_stop_denominator`.
        f_stop_denominator: i8,
    },
    /// PanoramaEntry for `'pano'` (6.8.8.2).
    Pano {
        /// `frame_number`.
        frame_number: u16,
    },
    /// DirectReferenceSamplesList for `'refs'` (7.4.1).
    Refs(crate::image_sequence::DirectReferenceSamplesList),
    /// Region-to-annotation mapping for `'regm'` (11.4.3.1).
    Regm(crate::region::RegionAnnotationMap),
    /// UserDescriptionSampleGroupEntry for `'udes'` (11.4.3.2).
    Udes(crate::region::UserDescriptionSampleGroupEntry),
    /// Raw bytes for any grouping type not specifically handled.
    Unknown(Vec<u8>),
}

/// A `roll` sample-group description: `roll_distance` (`signed int(16)`),
/// ISO/IEC 14496-12:2026 §10.1.2.
const ROLL_ENTRY_LEN: usize = 2;
/// An `prol` sample-group description: the same signed 16-bit distance.
const PROL_ENTRY_LEN: usize = 2;

impl SgpdEntry {
    /// The samples a decoder must decode before its output is correct, when
    /// this entry states a pre-roll (§10.1.3): a negative `roll` distance or a
    /// positive `prol` distance. A positive `roll` distance is post-roll
    /// (gradual refresh), not a pre-roll requirement, and yields `None`.
    #[must_use]
    pub fn pre_roll_samples(&self) -> Option<u32> {
        match self {
            Self::Roll { roll_distance } if *roll_distance < 0 => {
                u32::try_from(-i32::from(*roll_distance)).ok()
            }
            Self::Prol { roll_distance } if *roll_distance > 0 => {
                u32::try_from(*roll_distance).ok()
            }
            _ => None,
        }
    }

    /// Serialized size of this entry on the wire (body bytes only, no length prefix).
    pub fn wire_len(&self) -> usize {
        match self {
            Self::Roll { .. } => ROLL_ENTRY_LEN,
            Self::Prol { .. } => PROL_ENTRY_LEN,
            Self::Eqiv { .. } => EQIV_ENTRY_LEN,
            Self::Aebr { .. } => AEBR_ENTRY_LEN,
            Self::Wbbr { .. } => WBBR_ENTRY_LEN,
            Self::Fobr { .. } => FOBR_ENTRY_LEN,
            Self::Afbr { .. } => AFBR_ENTRY_LEN,
            Self::Dobr { .. } => DOBR_ENTRY_LEN,
            Self::Pano { .. } => PANO_ENTRY_LEN,
            Self::Refs(value) => value.serialized_len(),
            Self::Regm(value) => value.body_len(),
            Self::Udes(value) => value.body_len(),
            Self::Unknown(v) => v.len(),
        }
    }

    /// Write this entry's body (no length prefix) into `buf`, returning the
    /// bytes written.
    pub fn encode(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.wire_len();
        need_out(buf, need)?;
        match self {
            Self::Roll { roll_distance } => {
                buf[..ROLL_ENTRY_LEN].copy_from_slice(&roll_distance.to_be_bytes());
            }
            Self::Prol { roll_distance } => {
                buf[..PROL_ENTRY_LEN].copy_from_slice(&roll_distance.to_be_bytes());
            }
            Self::Eqiv {
                time_offset,
                timescale_multiplier,
            } => {
                buf[..core::mem::size_of::<u16>()].copy_from_slice(&time_offset.to_be_bytes());
                buf[core::mem::size_of::<u16>()..EQIV_ENTRY_LEN]
                    .copy_from_slice(&timescale_multiplier.to_be_bytes());
            }
            Self::Aebr {
                exposure_step,
                exposure_numerator,
            } => {
                buf[0] = *exposure_step as u8;
                buf[1] = *exposure_numerator as u8;
            }
            Self::Wbbr {
                blue_amber,
                green_magenta,
            } => {
                buf[..core::mem::size_of::<u16>()].copy_from_slice(&blue_amber.to_be_bytes());
                buf[core::mem::size_of::<u16>()] = *green_magenta as u8;
            }
            Self::Fobr {
                focus_distance_numerator,
                focus_distance_denominator,
            } => {
                buf[..core::mem::size_of::<u16>()]
                    .copy_from_slice(&focus_distance_numerator.to_be_bytes());
                buf[core::mem::size_of::<u16>()..FOBR_ENTRY_LEN]
                    .copy_from_slice(&focus_distance_denominator.to_be_bytes());
            }
            Self::Afbr {
                flash_exposure_numerator,
                flash_exposure_denominator,
            } => {
                buf[0] = *flash_exposure_numerator as u8;
                buf[1] = *flash_exposure_denominator as u8;
            }
            Self::Dobr {
                f_stop_numerator,
                f_stop_denominator,
            } => {
                buf[0] = *f_stop_numerator as u8;
                buf[1] = *f_stop_denominator as u8;
            }
            Self::Pano { frame_number } => {
                buf[..PANO_ENTRY_LEN].copy_from_slice(&frame_number.to_be_bytes());
            }
            Self::Refs(value) => {
                value.serialize_into(buf)?;
            }
            Self::Regm(value) => {
                let bytes = value.to_bytes()?;
                buf[..bytes.len()].copy_from_slice(&bytes);
            }
            Self::Udes(value) => {
                let bytes = value.to_bytes()?;
                buf[..bytes.len()].copy_from_slice(&bytes);
            }
            Self::Unknown(v) => buf[..v.len()].copy_from_slice(v),
        }
        Ok(need)
    }
}

/// The typed form of one sample group entry body, when this module models the
/// grouping type and the body is exactly the length (or, for a variable-length
/// entry, the layout) it states.
fn typed_entry(grouping_type: u32, bytes: &[u8]) -> Option<SgpdEntry> {
    match grouping_type {
        GROUPING_TYPE_ROLL if bytes.len() == ROLL_ENTRY_LEN => Some(SgpdEntry::Roll {
            roll_distance: i16_at(bytes, 0),
        }),
        GROUPING_TYPE_PROL if bytes.len() == PROL_ENTRY_LEN => Some(SgpdEntry::Prol {
            roll_distance: i16_at(bytes, 0),
        }),
        GROUPING_TYPE_EQIV if bytes.len() == EQIV_ENTRY_LEN => Some(SgpdEntry::Eqiv {
            time_offset: i16_at(bytes, 0),
            timescale_multiplier: u16::from_be_bytes([
                bytes[SECOND_U16_OFFSET],
                bytes[SECOND_U16_OFFSET + 1],
            ]),
        }),
        GROUPING_TYPE_AEBR if bytes.len() == AEBR_ENTRY_LEN => Some(SgpdEntry::Aebr {
            exposure_step: bytes[0] as i8,
            exposure_numerator: bytes[1] as i8,
        }),
        GROUPING_TYPE_WBBR if bytes.len() == WBBR_ENTRY_LEN => Some(SgpdEntry::Wbbr {
            blue_amber: u16::from_be_bytes([bytes[0], bytes[1]]),
            green_magenta: bytes[SECOND_U16_OFFSET] as i8,
        }),
        GROUPING_TYPE_FOBR if bytes.len() == FOBR_ENTRY_LEN => Some(SgpdEntry::Fobr {
            focus_distance_numerator: u16::from_be_bytes([bytes[0], bytes[1]]),
            focus_distance_denominator: u16::from_be_bytes([
                bytes[SECOND_U16_OFFSET],
                bytes[SECOND_U16_OFFSET + 1],
            ]),
        }),
        GROUPING_TYPE_AFBR if bytes.len() == AFBR_ENTRY_LEN => Some(SgpdEntry::Afbr {
            flash_exposure_numerator: bytes[0] as i8,
            flash_exposure_denominator: bytes[1] as i8,
        }),
        GROUPING_TYPE_DOBR if bytes.len() == DOBR_ENTRY_LEN => Some(SgpdEntry::Dobr {
            f_stop_numerator: bytes[0] as i8,
            f_stop_denominator: bytes[1] as i8,
        }),
        GROUPING_TYPE_PANO if bytes.len() == PANO_ENTRY_LEN => Some(SgpdEntry::Pano {
            frame_number: u16::from_be_bytes([bytes[0], bytes[1]]),
        }),
        GROUPING_TYPE_REFS => crate::image_sequence::DirectReferenceSamplesList::parse(bytes)
            .ok()
            .map(SgpdEntry::Refs),
        GROUPING_TYPE_REGM => crate::region::RegionAnnotationMap::parse(bytes)
            .ok()
            .map(SgpdEntry::Regm),
        GROUPING_TYPE_UDES => crate::region::UserDescriptionSampleGroupEntry::parse(bytes)
            .ok()
            .map(SgpdEntry::Udes),
        _ => None,
    }
}

/// Sample Group Description Box (`sgpd`) — ISO/IEC 14496-12:2026 §8.9.3.2.
///
/// Versions 0 to 3 parse and serialize at their own version.
///
/// Wire layout (FullBox header omitted):
///
/// ```text
/// grouping_type          u(32)
/// default_length         u(32)  — version >= 1
/// default_group_description_index u(32) — version >= 2
/// entry_count            u(32)
/// for each entry:
///   [description_length  u(32)] — only if version >= 1 && default_length == 0
///   SampleGroupEntry (grouping_type)
/// ```
///
/// When `version >= 1` and `default_length != 0`, every entry has the same
/// length (`default_length` bytes). When `default_length == 0`, each entry is
/// preceded by a 4-byte `description_length`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SampleGroupDescriptionBox {
    /// FullBox version.
    pub version: u8,
    /// FullBox flags `[23:0]`.
    pub flags: u32,
    /// Four-CC grouping type (e.g. `GROUPING_TYPE_ROLL`).
    pub grouping_type: u32,
    /// `default_length` (version >= 1). Zero means every entry carries its own
    /// 4-byte `description_length` prefix; a non-zero value is the shared
    /// length of every entry. Written back as the file stated it.
    pub default_length: u32,
    /// `default_group_description_index` (version >= 2, §8.9.3.2; the 2015
    /// edition called it `default_sample_description_index`): `None` for
    /// version < 2, where it is not coded.
    pub default_group_description_index: Option<u32>,
    /// Parsed entries.
    pub entries: Vec<SgpdEntry>,
}

impl SampleGroupDescriptionBox {
    /// The registered sample-grouping type this box describes.
    #[must_use]
    pub fn group_type(&self) -> crate::registered::SampleGroupType {
        crate::registered::SampleGroupType::from_code(self.grouping_type)
    }

    /// Version 3: an essential sample group description (§8.9.3.1). A reader
    /// that does not recognise `grouping_type` must not process the track.
    pub fn is_essential(&self) -> bool {
        self.version >= SGPD_VERSION_ESSENTIAL
    }

    /// Parse the body of an `sgpd` box (after the 8-byte BoxHeader).
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(
            body,
            FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>() + core::mem::size_of::<u32>(),
            "sgpd body",
        )?;
        let version = body[0];
        let flags = be_u32(body, 0, "full box header")? & FULLBOX_FLAGS_MASK;
        let mut c = FULLBOX_EXTRA_SIZE;
        let grouping_type = be_u32(body, c, "sample group field")?;
        c += core::mem::size_of::<u32>();
        let mut default_group_description_index = None;

        let default_length = if version >= SGPD_VERSION_DEFAULT_LENGTH {
            need_in(body, c + core::mem::size_of::<u32>(), "sgpd default_length")?;
            let dl = be_u32(body, c, "sample group field")?;
            c += core::mem::size_of::<u32>();
            dl
        } else {
            0
        };
        if version >= SGPD_VERSION_GROUP_INDEX {
            need_in(
                body,
                c + core::mem::size_of::<u32>(),
                "sgpd default_group_description_index",
            )?;
            let idx = be_u32(body, c, "sample group field")?;
            c += core::mem::size_of::<u32>();
            default_group_description_index = Some(idx);
        }

        need_in(body, c + core::mem::size_of::<u32>(), "sgpd entry_count")?;
        let entry_count = be_u32(body, c, "sample group field")? as usize;
        c += core::mem::size_of::<u32>();

        // Minimum bytes each entry can possibly consume, for bounding the
        // up-front allocation against a hostile `entry_count` (#988): a v1
        // per-entry `description_length` prefix (4 bytes) when
        // `default_length` is 0, else `default_length` itself; 2 bytes for a
        // v0 'roll' entry; 1 byte as a floor for the "consume the rest as one
        // blob" v0 fallback.
        let sgpd_min_entry_len: usize = if version >= SGPD_VERSION_DEFAULT_LENGTH {
            if default_length == 0 {
                SGPD_ENTRY_PREFIX_LEN
            } else {
                default_length as usize
            }
        } else if grouping_type == GROUPING_TYPE_ROLL || grouping_type == GROUPING_TYPE_PROL {
            ROLL_ENTRY_LEN
        } else {
            1
        };
        // Bound `entry_count` by the bytes actually present: every entry
        // consumes at least `sgpd_min_entry_len`, so a count whose minimum
        // footprint exceeds the body is wire-hostile, not something to
        // iterate over (r05-C10 — `bounded_entry_count` below only capped
        // the initial *capacity*, while the loop still ran `entry_count`
        // times pushing empty entries).
        let entry_min_footprint = entry_count.saturating_mul(sgpd_min_entry_len);
        if entry_min_footprint > body.len() - c {
            return Err(Error::BufferTooShort(BufferTooShort {
                need: c.saturating_add(entry_min_footprint),
                have: body.len(),
                what: "sgpd entries",
            }));
        }
        let mut entries = Vec::with_capacity(bounded_entry_count(
            body.len().saturating_sub(c),
            sgpd_min_entry_len,
            entry_count,
        ));
        for _ in 0..entry_count {
            let entry_len: usize = if version >= SGPD_VERSION_DEFAULT_LENGTH && default_length == 0
            {
                need_in(body, c + SGPD_ENTRY_PREFIX_LEN, "sgpd description_length")?;
                let dl = be_u32(body, c, "sample group field")? as usize;
                c += SGPD_ENTRY_PREFIX_LEN;
                dl
            } else if version >= SGPD_VERSION_DEFAULT_LENGTH {
                default_length as usize
            } else {
                // version 0: entry size is implied by the grouping_type; we
                // parse until we hit the known size for 'roll'/'prol' (2 bytes), else
                // we consume remaining bytes as one blob (rare / deprecated).
                if grouping_type == GROUPING_TYPE_ROLL || grouping_type == GROUPING_TYPE_PROL {
                    ROLL_ENTRY_LEN
                } else {
                    // unknown v0: consume all remaining as one entry. A
                    // zero-length blob would not advance `c`, so every later
                    // iteration of a hostile `entry_count` pushed another
                    // empty entry — ~4.3 G `Vec` headers from a 24-byte box
                    // (r05-C10). No legitimate group description is empty,
                    // and the bounds check below reports this same error for
                    // a truncated entry.
                    let rest = body.len() - c;
                    if rest == 0 {
                        return Err(Error::BufferTooShort(BufferTooShort {
                            need: c + 1,
                            have: body.len(),
                            what: "sgpd entry body",
                        }));
                    }
                    rest
                }
            };
            need_in(body, c + entry_len, "sgpd entry body")?;
            let entry_bytes = &body[c..c + entry_len];
            // A `roll` description is exactly the 2-byte `roll_distance`
            // (§10.1.2). A box whose v1 `default_length` or per-entry
            // `description_length` is larger therefore carries something this
            // crate does not model after the distance; keeping only the first
            // 2 bytes would re-serialize it 2 bytes wide and shrink the whole
            // entry list (audit r05-W15), so anything else stays opaque and
            // byte-exact.
            let entry = typed_entry(grouping_type, entry_bytes)
                .unwrap_or_else(|| SgpdEntry::Unknown(entry_bytes.to_vec()));
            entries.push(entry);
            c += entry_len;
        }

        Ok(Self {
            version,
            flags,
            grouping_type,
            default_length,
            default_group_description_index,
            entries,
        })
    }
}

impl<'a> Parse<'a> for SampleGroupDescriptionBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(
            bytes,
            BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>(),
            "sgpd box",
        )?;
        let ty = be_u32(bytes, BOX_TYPE_OFFSET, "box header")?;
        if ty != SGPD_TYPE {
            return Err(Error::InvalidValue(InvalidValue {
                field: "box_type",
                value: ty as u64,
                reason: "expected sgpd",
            }));
        }
        Self::parse_body(crate::box_types::parse_box(bytes)?.0.body)
    }
}

impl Serialize for SampleGroupDescriptionBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let entry_overhead = if self.per_entry_length() {
            SGPD_ENTRY_PREFIX_LEN
        } else {
            0
        };
        let entries_size: usize = self
            .entries
            .iter()
            .map(|e| entry_overhead + e.wire_len())
            .sum();
        let dl_field = if self.version >= SGPD_VERSION_DEFAULT_LENGTH {
            core::mem::size_of::<u32>()
        } else {
            0
        };
        let index_field = if self.version >= SGPD_VERSION_GROUP_INDEX {
            core::mem::size_of::<u32>()
        } else {
            0
        };
        crate::box_types::box_len(
            BOX_HEADER_MIN_SIZE
                + FULLBOX_EXTRA_SIZE
                + core::mem::size_of::<u32>()
                + dl_field
                + index_field
                + core::mem::size_of::<u32>()
                + entries_size,
        )
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let per_entry_prefix = self.per_entry_length();
        self.validate_default_length()?;
        let mut c = 0;
        c += crate::box_types::write_full_box_header(
            buf,
            b"sgpd",
            need,
            self.version,
            self.flags,
            false,
        )?;
        buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&self.grouping_type.to_be_bytes());
        c += core::mem::size_of::<u32>();
        if self.version >= SGPD_VERSION_DEFAULT_LENGTH {
            buf[c..c + core::mem::size_of::<u32>()]
                .copy_from_slice(&self.default_length.to_be_bytes());
            c += core::mem::size_of::<u32>();
        }
        if self.version >= SGPD_VERSION_GROUP_INDEX {
            let index = self.default_group_description_index.ok_or(Error::InvalidInput(
                "sgpd version 2 and later require default_group_description_index (ISO/IEC 14496-12:2026 §8.9.3.2)",
            ))?;
            buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&index.to_be_bytes());
            c += core::mem::size_of::<u32>();
        }
        let entry_count = broadcast_core::len::fit_u32(self.entries.len(), "entry_count")?;
        buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&entry_count.to_be_bytes());
        c += core::mem::size_of::<u32>();
        for entry in &self.entries {
            if per_entry_prefix {
                let description_length =
                    broadcast_core::len::fit_u32(entry.wire_len(), "description_length")?;
                buf[c..c + core::mem::size_of::<u32>()]
                    .copy_from_slice(&description_length.to_be_bytes());
                c += core::mem::size_of::<u32>();
            }
            c += entry.encode(&mut buf[c..])?;
        }
        Ok(c)
    }
}

impl SampleGroupDescriptionBox {
    /// Whether every entry carries its own `description_length` prefix:
    /// version 1 and later only, and only when the box states
    /// `default_length == 0` (ISO/IEC 14496-12:2026 §8.9.3.2).
    fn per_entry_length(&self) -> bool {
        self.version >= SGPD_VERSION_DEFAULT_LENGTH && self.default_length == 0
    }

    /// A shared `default_length` is the width of every entry (§8.9.3.2), so a
    /// box whose entries disagree with it cannot be written.
    fn validate_default_length(&self) -> Result<()> {
        if self.version < SGPD_VERSION_DEFAULT_LENGTH
            || self.default_length == 0
            || self
                .entries
                .iter()
                .all(|e| e.wire_len() == self.default_length as usize)
        {
            return Ok(());
        }
        Err(Error::InvalidInput(
            "sgpd default_length must equal every entry's width (ISO/IEC 14496-12:2026 §8.9.3.2)",
        ))
    }
}

// ---------------------------------------------------------------------------
// SampleToGroupBox — sbgp (ISO/IEC 14496-12:2026 §8.9.2)
// ---------------------------------------------------------------------------

/// Entry in the sbgp sample-to-group mapping table (§8.9.2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SbgpEntry {
    /// Number of consecutive samples belonging to this group.
    pub sample_count: u32,
    /// Index into the sgpd table (1-based, or 0 = no group).
    pub group_description_index: u32,
}

/// Sample To Group Box (`sbgp`) — ISO/IEC 14496-12:2026 §8.9.2.2.
///
/// Wire layout (FullBox header omitted):
///
/// ```text
/// grouping_type            u(32)
/// [grouping_type_parameter u(32)] — version == 1 only
/// entry_count              u(32)
/// for each entry:
///   sample_count             u(32)
///   group_description_index  u(32)
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SampleToGroupBox {
    /// FullBox version: 0 = no `grouping_type_parameter`; 1 = has it.
    pub version: u8,
    /// FullBox flags `[23:0]`.
    pub flags: u32,
    /// Four-CC grouping type linking this box to its sgpd.
    pub grouping_type: u32,
    /// Optional sub-type parameter (version 1 only).
    pub grouping_type_parameter: Option<u32>,
    /// Mapping entries.
    pub entries: Vec<SbgpEntry>,
}

impl SampleToGroupBox {
    /// The registered sample-grouping type this box maps samples to.
    #[must_use]
    pub fn group_type(&self) -> crate::registered::SampleGroupType {
        crate::registered::SampleGroupType::from_code(self.grouping_type)
    }

    /// Parse the body of an `sbgp` box (after the 8-byte BoxHeader).
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(
            body,
            FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>() + core::mem::size_of::<u32>(),
            "sbgp body",
        )?;
        let version = body[0];
        let flags = be_u32(body, 0, "full box header")? & FULLBOX_FLAGS_MASK;
        let mut c = FULLBOX_EXTRA_SIZE;
        let grouping_type = be_u32(body, c, "sample group field")?;
        c += core::mem::size_of::<u32>();
        let grouping_type_parameter = if version == 1 {
            need_in(
                body,
                c + core::mem::size_of::<u32>(),
                "sbgp grouping_type_parameter",
            )?;
            let v = be_u32(body, c, "sample group field")?;
            c += core::mem::size_of::<u32>();
            Some(v)
        } else {
            None
        };
        need_in(body, c + core::mem::size_of::<u32>(), "sbgp entry_count")?;
        let entry_count = be_u32(body, c, "sample group field")? as usize;
        c += core::mem::size_of::<u32>();
        // Each entry is a fixed 8 bytes (sample_count + group_description_index).
        let mut entries = Vec::with_capacity(bounded_entry_count(
            body.len().saturating_sub(c),
            U32_PAIR_LEN,
            entry_count,
        ));
        for _ in 0..entry_count {
            need_in(body, c + U32_PAIR_LEN, "sbgp entry")?;
            let sample_count = be_u32(body, c, "sample group field")?;
            let group_description_index =
                be_u32(body, c + core::mem::size_of::<u32>(), "sample group field")?;
            entries.push(SbgpEntry {
                sample_count,
                group_description_index,
            });
            c += U32_PAIR_LEN;
        }
        Ok(Self {
            version,
            flags,
            grouping_type,
            grouping_type_parameter,
            entries,
        })
    }
}

impl<'a> Parse<'a> for SampleToGroupBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(
            bytes,
            BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>(),
            "sbgp box",
        )?;
        let ty = be_u32(bytes, BOX_TYPE_OFFSET, "box header")?;
        if ty != SBGP_TYPE {
            return Err(Error::InvalidValue(InvalidValue {
                field: "box_type",
                value: ty as u64,
                reason: "expected sbgp",
            }));
        }
        Self::parse_body(crate::box_types::parse_box(bytes)?.0.body)
    }
}

impl Serialize for SampleToGroupBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let gtp_size = if self.version == 1 {
            core::mem::size_of::<u32>()
        } else {
            0
        };
        crate::box_types::box_len(
            BOX_HEADER_MIN_SIZE
                + FULLBOX_EXTRA_SIZE
                + core::mem::size_of::<u32>()
                + gtp_size
                + core::mem::size_of::<u32>()
                + self.entries.len() * U32_PAIR_LEN,
        )
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = 0;
        c += crate::box_types::write_full_box_header(
            buf,
            b"sbgp",
            need,
            self.version,
            self.flags,
            false,
        )?;
        buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&self.grouping_type.to_be_bytes());
        c += core::mem::size_of::<u32>();
        if self.version == 1 {
            let gtp = self.grouping_type_parameter.unwrap_or(0);
            buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&gtp.to_be_bytes());
            c += core::mem::size_of::<u32>();
        }
        let entry_count = broadcast_core::len::fit_u32(self.entries.len(), "entry_count")?;
        buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&entry_count.to_be_bytes());
        c += core::mem::size_of::<u32>();
        for entry in &self.entries {
            buf[c..c + core::mem::size_of::<u32>()]
                .copy_from_slice(&entry.sample_count.to_be_bytes());
            buf[c + core::mem::size_of::<u32>()..c + U32_PAIR_LEN]
                .copy_from_slice(&entry.group_description_index.to_be_bytes());
            c += U32_PAIR_LEN;
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// SubSampleInformationBox — subs (ISO/IEC 14496-12:2026 §8.7.7)
// ---------------------------------------------------------------------------

/// A single sub-sample descriptor within a [`SubsEntry`].
///
/// `subsample_size` width depends on the `subs` box version:
/// version 0 → u16; version 1 → u32. Stored as u32 in both cases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SubSampleDescriptor {
    /// Size in bytes (u16 on wire for v0, u32 for v1).
    pub subsample_size: u32,
    /// Degradation priority (higher = more important, §8.7.7.3).
    pub subsample_priority: u8,
    /// 0 = required; 1 = discardable (§8.7.7.3).
    pub discardable: u8,
    /// Codec-specific parameters (§8.7.7.3); 0 if not defined.
    pub codec_specific_parameters: u32,
}

impl SubSampleDescriptor {
    fn wire_len(version: u8) -> usize {
        let size_field = if version == 1 {
            core::mem::size_of::<u32>()
        } else {
            core::mem::size_of::<u16>()
        };
        size_field + core::mem::size_of::<u16>() + core::mem::size_of::<u32>()
    }
}

/// Per-sample entry in the subs table (§8.7.7.2).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SubsEntry {
    /// Delta from the previous entry's sample number (or from 0 for the first
    /// entry), giving the sample number of this entry's first described sample.
    pub sample_delta: u32,
    /// Sub-sample descriptors for this sample (may be empty).
    pub subsamples: Vec<SubSampleDescriptor>,
}

impl SubsEntry {
    fn wire_len(&self, version: u8) -> usize {
        SUBS_ENTRY_HEADER_LEN + self.subsamples.len() * SubSampleDescriptor::wire_len(version)
    }
}

/// Sub-Sample Information Box (`subs`) — ISO/IEC 14496-12:2026 §8.7.7.2.
///
/// Wire layout (FullBox header omitted):
///
/// ```text
/// entry_count     u(32)
/// for each entry:
///   sample_delta    u(32)
///   subsample_count u(16)
///   for each subsample:
///     subsample_size      u(16) if version == 0, u(32) if version == 1
///     subsample_priority  u(8)
///     discardable         u(8)
///     codec_specific_parameters u(32)
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SubSampleInformationBox {
    /// FullBox version: 0 = `subsample_size` is u16; 1 = u32.
    pub version: u8,
    /// FullBox flags `[23:0]`.
    pub flags: u32,
    /// Sample entries (each covering one sample with sub-sample structure).
    pub entries: Vec<SubsEntry>,
}

impl SubSampleInformationBox {
    /// Parse the body of a `subs` box (after the 8-byte BoxHeader).
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(
            body,
            FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>(),
            "subs body",
        )?;
        let version = body[0];
        if version >= SUBS_VERSION_REFERENCE {
            return Err(Error::InvalidValue(InvalidValue {
                field: "subs version",
                value: u64::from(version),
                reason: "version 2 and later use SubSampleInformationBoxV2 (ISO/IEC 14496-12:2026 §8.7.7.2)",
            }));
        }
        let flags = be_u32(body, 0, "full box header")? & FULLBOX_FLAGS_MASK;
        let mut c = FULLBOX_EXTRA_SIZE;
        let entry_count = be_u32(body, c, "sample group field")? as usize;
        c += core::mem::size_of::<u32>();
        let ss_size_field = if version == 1 {
            core::mem::size_of::<u32>()
        } else {
            core::mem::size_of::<u16>()
        };
        // Each entry's fixed header (sample_delta + subsample_count) is 6
        // bytes; the variable-length subsample list is bounded separately
        // below, per entry.
        let mut entries = Vec::with_capacity(bounded_entry_count(
            body.len().saturating_sub(c),
            SUBS_ENTRY_HEADER_LEN,
            entry_count,
        ));
        for _ in 0..entry_count {
            if body.len() < c + SUBS_ENTRY_HEADER_LEN {
                return Err(Error::BufferTooShort(BufferTooShort {
                    need: c + SUBS_ENTRY_HEADER_LEN,
                    have: body.len(),
                    what: "subs entry header",
                }));
            }
            let sample_delta = be_u32(body, c, "sample group field")?;
            c += core::mem::size_of::<u32>();
            let subsample_count = be_u16(body, c, "sample group field")? as usize;
            c += core::mem::size_of::<u16>();
            let ss_wire = ss_size_field + core::mem::size_of::<u16>() + core::mem::size_of::<u32>();
            let mut subsamples = Vec::with_capacity(bounded_entry_count(
                body.len().saturating_sub(c),
                ss_wire,
                subsample_count,
            ));
            for _ in 0..subsample_count {
                need_in(body, c + ss_wire, "subs subsample")?;
                let subsample_size = if version == 1 {
                    let v = be_u32(body, c, "sample group field")?;
                    c += core::mem::size_of::<u32>();
                    v
                } else {
                    let v = be_u16(body, c, "sample group field")? as u32;
                    c += core::mem::size_of::<u16>();
                    v
                };
                let subsample_priority = body[c];
                let discardable = body[c + 1];
                c += core::mem::size_of::<u16>();
                let codec_specific_parameters = be_u32(body, c, "sample group field")?;
                c += core::mem::size_of::<u32>();
                subsamples.push(SubSampleDescriptor {
                    subsample_size,
                    subsample_priority,
                    discardable,
                    codec_specific_parameters,
                });
            }
            entries.push(SubsEntry {
                sample_delta,
                subsamples,
            });
        }
        Ok(Self {
            version,
            flags,
            entries,
        })
    }
}

impl<'a> Parse<'a> for SubSampleInformationBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(
            bytes,
            BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>(),
            "subs box",
        )?;
        let ty = be_u32(bytes, BOX_TYPE_OFFSET, "box header")?;
        if ty != SUBS_TYPE {
            return Err(Error::InvalidValue(InvalidValue {
                field: "box_type",
                value: ty as u64,
                reason: "expected subs",
            }));
        }
        Self::parse_body(crate::box_types::parse_box(bytes)?.0.body)
    }
}

impl Serialize for SubSampleInformationBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let entries_size: usize = self.entries.iter().map(|e| e.wire_len(self.version)).sum();
        crate::box_types::box_len(
            BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>() + entries_size,
        )
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = 0;
        c += crate::box_types::write_full_box_header(
            buf,
            b"subs",
            need,
            self.version,
            self.flags,
            false,
        )?;
        let entry_count = broadcast_core::len::fit_u32(self.entries.len(), "entry_count")?;
        buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&entry_count.to_be_bytes());
        c += core::mem::size_of::<u32>();
        for entry in &self.entries {
            buf[c..c + core::mem::size_of::<u32>()]
                .copy_from_slice(&entry.sample_delta.to_be_bytes());
            c += core::mem::size_of::<u32>();
            let subsample_count =
                broadcast_core::len::fit_u16(entry.subsamples.len(), "subsample_count")?;
            buf[c..c + core::mem::size_of::<u16>()].copy_from_slice(&subsample_count.to_be_bytes());
            c += core::mem::size_of::<u16>();
            for ss in &entry.subsamples {
                if self.version == 1 {
                    buf[c..c + core::mem::size_of::<u32>()]
                        .copy_from_slice(&ss.subsample_size.to_be_bytes());
                    c += core::mem::size_of::<u32>();
                } else {
                    let subsample_size =
                        broadcast_core::len::fit_u16(ss.subsample_size as usize, "subsample_size")?;
                    buf[c..c + core::mem::size_of::<u16>()]
                        .copy_from_slice(&subsample_size.to_be_bytes());
                    c += core::mem::size_of::<u16>();
                }
                buf[c] = ss.subsample_priority;
                buf[c + 1] = ss.discardable;
                c += core::mem::size_of::<u16>();
                buf[c..c + core::mem::size_of::<u32>()]
                    .copy_from_slice(&ss.codec_specific_parameters.to_be_bytes());
                c += core::mem::size_of::<u32>();
            }
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// subs version 2 and ssrt (ISO/IEC 14496-12:2026 §8.7.7.2, §8.7.10)
// ---------------------------------------------------------------------------

/// `codec_specific_parameters` of a version-2 sub-sample property set: 1 to 4
/// bytes (`csp_length_minus_1` + 1), big-endian in `value`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CodecSpecificParameters {
    /// Wire length in bytes, 1..=4.
    pub len: u8,
    /// Value; must fit in `len` bytes.
    pub value: u32,
}

/// Explicit sub-sample properties of a `subs` version-2 sub-sample or an
/// `ssrt` entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SubsampleProperties {
    /// `discardable`.
    pub discardable: bool,
    /// `subsample_priority` when `has_subsample_priority` is set, else `None`
    /// (inferred 0).
    pub subsample_priority: Option<u8>,
    /// `codec_specific_parameters` when `is_csp_present` is set, else `None`
    /// (inferred 0).
    pub codec_specific_parameters: Option<CodecSpecificParameters>,
    /// The reserved bits closing the property byte: two in a version-2 `subs`
    /// (§8.7.7.3), three in an `ssrt` entry (§8.7.10). Carried verbatim.
    pub reserved: u8,
}

impl SubsampleProperties {
    fn wire_len_after_flags(&self) -> usize {
        usize::from(self.subsample_priority.is_some())
            + self
                .codec_specific_parameters
                .map_or(0, |c| usize::from(c.len))
    }

    fn flags_byte(&self, layout: PropertyLayout) -> Result<u8> {
        let (disc, prio, csp, shift) = match layout {
            PropertyLayout::Subs => (
                PROP_DISCARDABLE_BIT,
                PROP_HAS_PRIORITY_BIT,
                PROP_HAS_CSP_BIT,
                PROP_CSP_LEN_SHIFT,
            ),
            PropertyLayout::Ssrt => (
                SSRT_PROP_DISCARDABLE_BIT,
                SSRT_PROP_HAS_PRIORITY_BIT,
                SSRT_PROP_HAS_CSP_BIT,
                SSRT_PROP_CSP_LEN_SHIFT,
            ),
        };
        let mut b = 0u8;
        if self.discardable {
            b |= disc;
        }
        if self.subsample_priority.is_some() {
            b |= prio;
        }
        if let Some(c) = self.codec_specific_parameters {
            if c.len == 0 || usize::from(c.len) > MAX_CSP_LEN {
                return Err(Error::InvalidInput(
                    "codec_specific_parameters length must be 1..=4 bytes (ISO/IEC 14496-12:2026 §8.7.7.2)",
                ));
            }
            b |= csp | ((c.len - 1) << shift);
        }
        let reserved = match layout {
            PropertyLayout::Subs => PROP_RESERVED_MASK,
            PropertyLayout::Ssrt => SSRT_PROP_RESERVED_MASK,
        };
        b |= self.reserved & reserved;
        Ok(b)
    }

    fn write_after_flags(&self, buf: &mut [u8]) -> Result<usize> {
        let mut c = 0;
        if let Some(p) = self.subsample_priority {
            buf[c] = p;
            c += 1;
        }
        if let Some(csp) = self.codec_specific_parameters {
            let n = usize::from(csp.len);
            broadcast_core::len::fit_bits(
                u64::from(csp.value),
                u8::BITS * n as u32,
                "codec_specific_parameters",
            )?;
            buf[c..c + n].copy_from_slice(&csp.value.to_be_bytes()[MAX_CSP_LEN - n..]);
            c += n;
        }
        Ok(c)
    }

    fn parse(body: &[u8], c: &mut usize, flags: u8, layout: PropertyLayout) -> Result<Self> {
        let (disc, prio, csp, shift) = match layout {
            PropertyLayout::Subs => (
                PROP_DISCARDABLE_BIT,
                PROP_HAS_PRIORITY_BIT,
                PROP_HAS_CSP_BIT,
                PROP_CSP_LEN_SHIFT,
            ),
            PropertyLayout::Ssrt => (
                SSRT_PROP_DISCARDABLE_BIT,
                SSRT_PROP_HAS_PRIORITY_BIT,
                SSRT_PROP_HAS_CSP_BIT,
                SSRT_PROP_CSP_LEN_SHIFT,
            ),
        };
        let subsample_priority = if flags & prio != 0 {
            need_in(body, *c + 1, "subsample_priority")?;
            let p = body[*c];
            *c += 1;
            Some(p)
        } else {
            None
        };
        let codec_specific_parameters = if flags & csp != 0 {
            let n = usize::from((flags >> shift) & PROP_CSP_LEN_MASK) + 1;
            need_in(body, *c + n, "codec_specific_parameters")?;
            let mut v = [0u8; MAX_CSP_LEN];
            v[MAX_CSP_LEN - n..].copy_from_slice(&body[*c..*c + n]);
            *c += n;
            Some(CodecSpecificParameters {
                len: n as u8,
                value: u32::from_be_bytes(v),
            })
        } else {
            None
        };
        let reserved = match layout {
            PropertyLayout::Subs => PROP_RESERVED_MASK,
            PropertyLayout::Ssrt => SSRT_PROP_RESERVED_MASK,
        };
        Ok(Self {
            discardable: flags & disc != 0,
            subsample_priority,
            codec_specific_parameters,
            reserved: flags & reserved,
        })
    }
}

#[derive(Clone, Copy)]
enum PropertyLayout {
    Subs,
    Ssrt,
}

/// How one version-2 sub-sample gets its properties.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum SubsV2Source {
    /// `has_reference` = 1: the properties are entry `reference_idx` (0-based)
    /// of the matching [`SubSampleReferenceTableBox`].
    Reference {
        /// 0-based index into the `ssrt` entries.
        reference_idx: u16,
    },
    /// `has_reference` = 0: the properties are carried here.
    Explicit(SubsampleProperties),
}

/// One sub-sample of a version-2 `subs` entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SubsV2Subsample {
    /// `subsample_size` in bytes.
    pub subsample_size: u32,
    /// Where the properties come from.
    pub source: SubsV2Source,
}

/// Per-sample entry of a version-2 `subs`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SubsV2Entry {
    /// Sample-number delta, as in version 0/1.
    pub sample_delta: u32,
    /// Sub-samples of this sample.
    pub subsamples: Vec<SubsV2Subsample>,
}

/// Sub-Sample Information Box (`subs`) version 2 — ISO/IEC 14496-12:2026
/// §8.7.7.2 (private/specs-verified/mpeg/isobmff/edit-and-sampletable.md).
///
/// Versions 0 and 1 stay in [`SubSampleInformationBox`], which rejects
/// version 2.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SubSampleInformationBoxV2 {
    /// FullBox flags `[23:0]`; pairs with the `ssrt` of equal flags.
    pub flags: u32,
    /// `is_size_16_bits`: `subsample_size` is 16 bits when set, else 32.
    pub is_size_16_bits: bool,
    /// `is_short_ref`: `reference_idx` is 7 bits when set, else 15.
    pub is_short_ref: bool,
    /// The six reserved bits after `is_short_ref` (§8.7.7.3).
    pub reserved: u8,
    /// Entries.
    pub entries: Vec<SubsV2Entry>,
}

impl SubSampleInformationBoxV2 {
    fn size_len(&self) -> usize {
        if self.is_size_16_bits {
            core::mem::size_of::<u16>()
        } else {
            core::mem::size_of::<u32>()
        }
    }

    fn ref_len(&self) -> usize {
        if self.is_short_ref {
            1
        } else {
            core::mem::size_of::<u16>()
        }
    }

    fn subsample_len(&self, s: &SubsV2Subsample) -> usize {
        self.size_len()
            + match s.source {
                SubsV2Source::Reference { .. } => self.ref_len(),
                SubsV2Source::Explicit(p) => 1 + p.wire_len_after_flags(),
            }
    }

    /// Parse the body of a `subs` box (after the 8-byte BoxHeader).
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(
            body,
            FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>() + 1,
            "subs v2 body",
        )?;
        if body[0] != SUBS_VERSION_REFERENCE {
            return Err(Error::InvalidValue(InvalidValue {
                field: "subs version",
                value: u64::from(body[0]),
                reason: "expected version 2",
            }));
        }
        let flags = be_u32(body, 0, "full box header")? & FULLBOX_FLAGS_MASK;
        let mut c = FULLBOX_EXTRA_SIZE;
        let entry_count = be_u32(body, c, "sample group field")? as usize;
        c += core::mem::size_of::<u32>();
        let config = body[c];
        let is_size_16_bits = config & SUBS_SIZE16_BIT != 0;
        let is_short_ref = config & SUBS_SHORT_REF_BIT != 0;
        let reserved = config & SUBS_CFG_RESERVED_MASK;
        c += 1;
        let mut out = Self {
            flags,
            is_size_16_bits,
            is_short_ref,
            reserved,
            entries: Vec::new(),
        };
        let size_len = out.size_len();
        let min_subsample = size_len + 1;
        out.entries = Vec::with_capacity(bounded_entry_count(
            body.len().saturating_sub(c),
            SUBS_ENTRY_HEADER_LEN,
            entry_count,
        ));
        for _ in 0..entry_count {
            need_in(body, c + SUBS_V2_ENTRY_HEADER_LEN, "subs v2 entry header")?;
            let sample_delta = be_u32(body, c, "sample group field")?;
            let count = usize::from(be_u16(
                body,
                c + core::mem::size_of::<u32>(),
                "sample group field",
            )?);
            c += SUBS_ENTRY_HEADER_LEN;
            let mut subsamples = Vec::with_capacity(bounded_entry_count(
                body.len().saturating_sub(c),
                min_subsample,
                count,
            ));
            for _ in 0..count {
                need_in(body, c + min_subsample, "subs v2 subsample")?;
                let subsample_size = if is_size_16_bits {
                    u32::from(be_u16(body, c, "sample group field")?)
                } else {
                    be_u32(body, c, "sample group field")?
                };
                c += size_len;
                let first = body[c];
                let source = if first & HAS_REFERENCE_BIT != 0 {
                    let reference_idx = if is_short_ref {
                        c += 1;
                        u16::from(first & !HAS_REFERENCE_BIT)
                    } else {
                        need_in(
                            body,
                            c + core::mem::size_of::<u16>(),
                            "subs v2 reference_idx",
                        )?;
                        let v = u16::from_be_bytes([first & !HAS_REFERENCE_BIT, body[c + 1]]);
                        c += core::mem::size_of::<u16>();
                        v
                    };
                    SubsV2Source::Reference { reference_idx }
                } else {
                    c += 1;
                    SubsV2Source::Explicit(SubsampleProperties::parse(
                        body,
                        &mut c,
                        first,
                        PropertyLayout::Subs,
                    )?)
                };
                subsamples.push(SubsV2Subsample {
                    subsample_size,
                    source,
                });
            }
            out.entries.push(SubsV2Entry {
                sample_delta,
                subsamples,
            });
        }
        Ok(out)
    }
}

impl<'a> Parse<'a> for SubSampleInformationBoxV2 {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(
            bytes,
            BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>(),
            "subs box",
        )?;
        let ty = be_u32(bytes, BOX_TYPE_OFFSET, "box header")?;
        if ty != SUBS_TYPE {
            return Err(Error::InvalidValue(InvalidValue {
                field: "box_type",
                value: ty as u64,
                reason: "expected subs",
            }));
        }
        Self::parse_body(crate::box_types::parse_box(bytes)?.0.body)
    }
}

impl Serialize for SubSampleInformationBoxV2 {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let entries: usize = self
            .entries
            .iter()
            .map(|e| {
                SUBS_V2_ENTRY_HEADER_LEN
                    + e.subsamples
                        .iter()
                        .map(|s| self.subsample_len(s))
                        .sum::<usize>()
            })
            .sum();
        crate::box_types::box_len(
            BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>() + 1 + entries,
        )
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = crate::box_types::write_full_box_header(
            buf,
            b"subs",
            need,
            SUBS_VERSION_REFERENCE,
            self.flags,
            false,
        )?;
        let entry_count = broadcast_core::len::fit_u32(self.entries.len(), "entry_count")?;
        buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&entry_count.to_be_bytes());
        c += core::mem::size_of::<u32>();
        let mut cfg = self.reserved & SUBS_CFG_RESERVED_MASK;
        if self.is_size_16_bits {
            cfg |= SUBS_SIZE16_BIT;
        }
        if self.is_short_ref {
            cfg |= SUBS_SHORT_REF_BIT;
        }
        buf[c] = cfg;
        c += 1;
        let ref_bits = if self.is_short_ref {
            SHORT_REF_BITS
        } else {
            LONG_REF_BITS
        };
        for e in &self.entries {
            buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&e.sample_delta.to_be_bytes());
            let count = broadcast_core::len::fit_u16(e.subsamples.len(), "subsample_count")?;
            buf[c + core::mem::size_of::<u32>()..c + SUBS_ENTRY_HEADER_LEN]
                .copy_from_slice(&count.to_be_bytes());
            c += SUBS_ENTRY_HEADER_LEN;
            for s in &e.subsamples {
                if self.is_size_16_bits {
                    let v =
                        broadcast_core::len::fit_u16(s.subsample_size as usize, "subsample_size")?;
                    buf[c..c + core::mem::size_of::<u16>()].copy_from_slice(&v.to_be_bytes());
                } else {
                    buf[c..c + core::mem::size_of::<u32>()]
                        .copy_from_slice(&s.subsample_size.to_be_bytes());
                }
                c += self.size_len();
                match s.source {
                    SubsV2Source::Reference { reference_idx } => {
                        let idx = broadcast_core::len::fit_bits(
                            u64::from(reference_idx),
                            ref_bits,
                            "reference_idx",
                        )? as u16;
                        if self.is_short_ref {
                            buf[c] = HAS_REFERENCE_BIT | idx as u8;
                        } else {
                            let w = u16::from(HAS_REFERENCE_BIT) << u8::BITS | idx;
                            buf[c..c + core::mem::size_of::<u16>()]
                                .copy_from_slice(&w.to_be_bytes());
                        }
                        c += self.ref_len();
                    }
                    SubsV2Source::Explicit(p) => {
                        buf[c] = p.flags_byte(PropertyLayout::Subs)?;
                        c += 1;
                        c += p.write_after_flags(&mut buf[c..])?;
                    }
                }
            }
        }
        Ok(c)
    }
}

/// Sub-sample Reference Table Box (`ssrt`) — ISO/IEC 14496-12:2026 §8.7.10
/// (private/specs-verified/mpeg/isobmff/edit-and-sampletable.md). Referenced
/// by a version-2 [`SubSampleInformationBoxV2`] with equal `flags`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SubSampleReferenceTableBox {
    /// FullBox version.
    pub version: u8,
    /// FullBox flags `[23:0]`.
    pub flags: u32,
    /// `is_short_ref`: `entry_count` is 7 bits when set, else 15.
    pub is_short_ref: bool,
    /// Referenced property sets.
    pub entries: Vec<SubsampleProperties>,
}

impl<'a> Parse<'a> for SubSampleReferenceTableBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(
            bytes,
            BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + 1,
            "ssrt box",
        )?;
        let ty = be_u32(bytes, BOX_TYPE_OFFSET, "box header")?;
        if ty != SSRT_TYPE {
            return Err(Error::InvalidValue(InvalidValue {
                field: "box_type",
                value: ty as u64,
                reason: "expected ssrt",
            }));
        }
        let body = crate::box_types::parse_box(bytes)?.0.body;
        need_in(body, FULLBOX_EXTRA_SIZE + 1, "ssrt body")?;
        let version = body[0];
        let flags = be_u32(body, 0, "full box header")? & FULLBOX_FLAGS_MASK;
        let mut c = FULLBOX_EXTRA_SIZE;
        let is_short_ref = body[c] & HAS_REFERENCE_BIT != 0;
        let entry_count = if is_short_ref {
            let v = usize::from(body[c] & !HAS_REFERENCE_BIT);
            c += 1;
            v
        } else {
            need_in(body, c + core::mem::size_of::<u16>(), "ssrt entry_count")?;
            let v = usize::from(u16::from_be_bytes([
                body[c] & !HAS_REFERENCE_BIT,
                body[c + 1],
            ]));
            c += core::mem::size_of::<u16>();
            v
        };
        let mut entries = Vec::with_capacity(bounded_entry_count(
            body.len().saturating_sub(c),
            1,
            entry_count,
        ));
        for _ in 0..entry_count {
            need_in(body, c + 1, "ssrt entry")?;
            let first = body[c];
            c += 1;
            entries.push(SubsampleProperties::parse(
                body,
                &mut c,
                first,
                PropertyLayout::Ssrt,
            )?);
        }
        Ok(Self {
            version,
            flags,
            is_short_ref,
            entries,
        })
    }
}

impl Serialize for SubSampleReferenceTableBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let count = if self.is_short_ref {
            1
        } else {
            core::mem::size_of::<u16>()
        };
        let entries: usize = self
            .entries
            .iter()
            .map(|p| 1 + p.wire_len_after_flags())
            .sum();
        crate::box_types::box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + count + entries)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = crate::box_types::write_full_box_header(
            buf,
            b"ssrt",
            need,
            self.version,
            self.flags,
            false,
        )?;
        let bits = if self.is_short_ref {
            SHORT_REF_BITS
        } else {
            LONG_REF_BITS
        };
        let count =
            broadcast_core::len::fit_bits(self.entries.len() as u64, bits, "entry_count")? as u16;
        if self.is_short_ref {
            buf[c] = HAS_REFERENCE_BIT | count as u8;
            c += 1;
        } else {
            buf[c..c + core::mem::size_of::<u16>()].copy_from_slice(&count.to_be_bytes());
            c += core::mem::size_of::<u16>();
        }
        for p in &self.entries {
            buf[c] = p.flags_byte(PropertyLayout::Ssrt)?;
            c += 1;
            c += p.write_after_flags(&mut buf[c..])?;
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Sample-group mapping — the one lookup both `sbgp` and `csgp` feed
// ---------------------------------------------------------------------------

/// Four-CC of the compact sample-to-group box (`csgp`, ISO/IEC 14496-12:2026
/// §8.9.5).
pub const CSGP: [u8; 4] = *b"csgp";

/// One run of samples mapped to the same group description index, produced by
/// either an `sbgp` or a `csgp` box.
///
/// This is the one sample-group lookup: both the plain
/// [`SampleToGroupBox::mapping`] and the compact
/// [`CompactSampleToGroupBox::mapping`] return it, so a consumer never reads
/// two different shapes for the same fact.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SampleGroupMapping {
    grouping_type: u32,
    grouping_type_parameter: Option<u32>,
    runs: Vec<SbgpEntry>,
}

impl SampleGroupMapping {
    /// Build an empty mapping for one grouping type.
    #[must_use]
    pub fn new(grouping_type: u32, grouping_type_parameter: Option<u32>) -> Self {
        Self {
            grouping_type,
            grouping_type_parameter,
            runs: Vec::new(),
        }
    }

    /// The grouping type this mapping describes.
    #[must_use]
    pub const fn grouping_type(&self) -> u32 {
        self.grouping_type
    }

    /// The grouping type parameter, when the source box carried one.
    #[must_use]
    pub const fn grouping_type_parameter(&self) -> Option<u32> {
        self.grouping_type_parameter
    }

    /// The runs of samples, in sample order.
    #[must_use]
    pub fn runs(&self) -> &[SbgpEntry] {
        &self.runs
    }

    /// The group description index for the `sample_index`-th sample (zero
    /// based), or `0` when the source box does not map that sample.
    #[must_use]
    pub fn description_index(&self, sample_index: usize) -> u32 {
        let mut base = 0usize;
        for run in &self.runs {
            let next = base.saturating_add(run.sample_count as usize);
            if sample_index >= base && sample_index < next {
                return run.group_description_index;
            }
            base = next;
        }
        0
    }

    /// The total number of samples this mapping covers.
    #[must_use]
    pub fn sample_count(&self) -> usize {
        self.runs.iter().map(|run| run.sample_count as usize).sum()
    }

    fn push_run(&mut self, sample_count: u32, index: u32) {
        if sample_count == 0 {
            return;
        }
        if let Some(last) = self.runs.last_mut()
            && last.group_description_index == index
        {
            last.sample_count = last.sample_count.saturating_add(sample_count);
            return;
        }
        self.runs.push(SbgpEntry {
            sample_count,
            group_description_index: index,
        });
    }
}

impl SampleToGroupBox {
    /// This box as the shared [`SampleGroupMapping`] lookup.
    #[must_use]
    pub fn mapping(&self) -> SampleGroupMapping {
        SampleGroupMapping {
            grouping_type: self.grouping_type,
            grouping_type_parameter: self.grouping_type_parameter,
            runs: self.entries.clone(),
        }
    }
}

/// One pattern of a [`CompactSampleToGroupBox`] (§8.9.5.2).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct CsgpPattern {
    sample_count: u32,
    indices: Vec<u32>,
}

impl CsgpPattern {
    /// Build one pattern from its sample count and cycled index list.
    #[must_use]
    pub fn new(sample_count: u32, indices: Vec<u32>) -> Self {
        Self {
            sample_count,
            indices,
        }
    }

    /// The number of samples this pattern covers.
    #[must_use]
    pub const fn sample_count(&self) -> u32 {
        self.sample_count
    }

    /// The pattern's group description indices, cycled across `sample_count`.
    #[must_use]
    pub fn indices(&self) -> &[u32] {
        &self.indices
    }
}

/// Bits of the reserved half of a `csgp` flag field.
const CSGP_RESERVED_BITS: u32 = 16;
/// Bit of `index_msb_indicates_fragment_local_description`.
const CSGP_INDEX_MSB_BIT: u32 = CSGP_RESERVED_BITS;
/// Bit of `grouping_type_parameter_present`.
const CSGP_PARAMETER_BIT: u32 = CSGP_INDEX_MSB_BIT - 1;
/// Lowest bit of `pattern_size_code`.
const CSGP_PATTERN_CODE_SHIFT: u32 = CSGP_PARAMETER_BIT - 2;
/// Lowest bit of `count_size_code`.
const CSGP_COUNT_CODE_SHIFT: u32 = CSGP_PATTERN_CODE_SHIFT - 2;
/// Lowest bit of `index_size_code`.
const CSGP_INDEX_CODE_SHIFT: u32 = CSGP_COUNT_CODE_SHIFT - 2;
/// Two-bit mask selecting one of the four field widths.
const CSGP_CODE_MASK: u32 = 0b11;
/// Code 0 selects a 4-bit field.
const CSGP_CODE_4: u8 = 0;
/// Code 1 selects an 8-bit field.
const CSGP_CODE_8: u8 = 1;
/// Code 2 selects a 16-bit field.
const CSGP_CODE_16: u8 = 2;
/// Largest up-front index allocation a single pattern may request.
const CSGP_PATTERN_PREALLOC: usize = 1 << CSGP_RESERVED_BITS;

fn csgp_width(code: u8) -> u32 {
    match code {
        CSGP_CODE_4 => 4,
        CSGP_CODE_8 => 8,
        CSGP_CODE_16 => 16,
        _ => 32,
    }
}

/// Compact Sample-to-Group Box (`csgp`) — ISO/IEC 14496-12:2026 §8.9.5.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct CompactSampleToGroupBox {
    version: u8,
    flags: u32,
    grouping_type: u32,
    grouping_type_parameter: Option<u32>,
    patterns: Vec<CsgpPattern>,
}

impl CompactSampleToGroupBox {
    /// Build one from its patterns with the four-bit field widths the standard
    /// recommends for compact mappings.
    #[must_use]
    pub fn new(
        grouping_type: u32,
        grouping_type_parameter: Option<u32>,
        patterns: Vec<CsgpPattern>,
    ) -> Self {
        let mut flags: u32 = u32::from(CSGP_CODE_4) << CSGP_PATTERN_CODE_SHIFT
            | u32::from(CSGP_CODE_4) << CSGP_COUNT_CODE_SHIFT
            | u32::from(CSGP_CODE_4) << CSGP_INDEX_CODE_SHIFT;
        if grouping_type_parameter.is_some() {
            flags |= 1 << CSGP_PARAMETER_BIT;
        }
        Self {
            version: 0,
            flags,
            grouping_type,
            grouping_type_parameter,
            patterns,
        }
    }

    /// The FullBox version.
    #[must_use]
    pub const fn version(&self) -> u8 {
        self.version
    }

    /// The FullBox flags.
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// The grouping type.
    #[must_use]
    pub const fn grouping_type(&self) -> u32 {
        self.grouping_type
    }

    /// The grouping type parameter, when present.
    #[must_use]
    pub const fn grouping_type_parameter(&self) -> Option<u32> {
        self.grouping_type_parameter
    }

    /// Whether the index's most significant bit selects the fragment-local
    /// `sgpd` (`index_msb_indicates_fragment_local_description`).
    #[must_use]
    pub const fn index_msb_indicates_fragment_local_description(&self) -> bool {
        self.flags & (1 << CSGP_INDEX_MSB_BIT) != 0
    }

    /// The patterns, in wire order.
    #[must_use]
    pub fn patterns(&self) -> &[CsgpPattern] {
        &self.patterns
    }

    /// This box as the shared [`SampleGroupMapping`] lookup, expanding each
    /// pattern's index list across its `sample_count` samples.
    ///
    /// The expansion is unbounded; use
    /// [`Self::mapping_with_limit`] when the caller must cap it.
    #[must_use]
    pub fn mapping(&self) -> SampleGroupMapping {
        self.mapping_with_limit(usize::MAX).unwrap_or_else(|_| {
            SampleGroupMapping::new(self.grouping_type, self.grouping_type_parameter)
        })
    }

    /// As [`Self::mapping`], refusing a mapping larger than `max_samples`.
    pub fn mapping_with_limit(&self, max_samples: usize) -> Result<SampleGroupMapping> {
        let mut mapping = SampleGroupMapping::new(self.grouping_type, self.grouping_type_parameter);
        let mut total = 0usize;
        for pattern in &self.patterns {
            total = total.saturating_add(pattern.sample_count as usize);
            if total > max_samples {
                return Err(Error::InvalidInput("csgp mapping exceeds the caller's cap"));
            }
            if pattern.indices.is_empty() {
                continue;
            }
            let mut remaining = pattern.sample_count as usize;
            while remaining > 0 {
                let take = remaining.min(pattern.indices.len());
                for &index in &pattern.indices[..take] {
                    mapping.push_run(1, index);
                }
                remaining -= take;
            }
        }
        Ok(mapping)
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(
            body,
            FULLBOX_EXTRA_SIZE + core::mem::size_of::<u64>(),
            "csgp body",
        )?;
        let version = body[0];
        let flags = be_u32(body, 0, "csgp flags")? & FULLBOX_FLAGS_MASK;
        let mut c = FULLBOX_EXTRA_SIZE;
        let grouping_type = be_u32(body, c, "csgp grouping_type")?;
        c += core::mem::size_of::<u32>();
        let grouping_type_parameter = if flags & (1 << CSGP_PARAMETER_BIT) != 0 {
            let value = be_u32(body, c, "csgp grouping_type_parameter")?;
            c += core::mem::size_of::<u32>();
            Some(value)
        } else {
            None
        };
        let pattern_count = be_u32(body, c, "csgp pattern_count")? as usize;
        c += core::mem::size_of::<u32>();
        let pattern_code = ((flags >> CSGP_PATTERN_CODE_SHIFT) & CSGP_CODE_MASK) as u8;
        let count_code = ((flags >> CSGP_COUNT_CODE_SHIFT) & CSGP_CODE_MASK) as u8;
        let index_code = ((flags >> CSGP_INDEX_CODE_SHIFT) & CSGP_CODE_MASK) as u8;
        let mut reader = broadcast_core::bits::BitReader::new(&body[c..]);
        let pattern_bits = (csgp_width(pattern_code) + csgp_width(count_code)) as usize;
        let mut lengths = Vec::with_capacity(bounded_entry_count(
            body.len().saturating_sub(c),
            pattern_bits.div_ceil(u8::BITS as usize),
            pattern_count,
        ));
        let mut counts = lengths.clone();
        for _ in 0..pattern_count {
            let length =
                read_csgp_bits(&mut reader, csgp_width(pattern_code), "csgp pattern_length")?;
            let count = read_csgp_bits(&mut reader, csgp_width(count_code), "csgp sample_count")?;
            lengths.push(length);
            counts.push(count);
        }
        let mut patterns = Vec::with_capacity(lengths.len());
        for (length, count) in lengths.into_iter().zip(counts) {
            let mut indices = Vec::with_capacity(
                usize::try_from(length)
                    .unwrap_or(usize::MAX)
                    .min(CSGP_PATTERN_PREALLOC),
            );
            for _ in 0..length {
                indices.push(read_csgp_bits(
                    &mut reader,
                    csgp_width(index_code),
                    "csgp index",
                )?);
            }
            patterns.push(CsgpPattern {
                sample_count: count,
                indices,
            });
        }
        Ok(Self {
            version,
            flags,
            grouping_type,
            grouping_type_parameter,
            patterns,
        })
    }

    fn size_codes(&self) -> (u8, u8, u8) {
        (
            ((self.flags >> CSGP_PATTERN_CODE_SHIFT) & CSGP_CODE_MASK) as u8,
            ((self.flags >> CSGP_COUNT_CODE_SHIFT) & CSGP_CODE_MASK) as u8,
            ((self.flags >> CSGP_INDEX_CODE_SHIFT) & CSGP_CODE_MASK) as u8,
        )
    }
}

fn read_csgp_bits(
    reader: &mut broadcast_core::bits::BitReader<'_>,
    width: u32,
    what: &'static str,
) -> Result<u32> {
    reader
        .read_bits(width)
        .map(|value| value as u32)
        .map_err(|_| Error::InvalidInput(what))
}

impl<'a> Parse<'a> for CompactSampleToGroupBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(
            bytes,
            BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + core::mem::size_of::<u64>(),
            "csgp box",
        )?;
        if be_u32(bytes, BOX_TYPE_OFFSET, "csgp box")? != u32::from_be_bytes(CSGP) {
            return Err(Error::InvalidValue(InvalidValue {
                field: "box_type",
                value: u32::from_be_bytes(CSGP) as u64,
                reason: "expected csgp",
            }));
        }
        Self::parse_body(crate::box_types::parse_box(bytes)?.0.body)
    }
}

impl Serialize for CompactSampleToGroupBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let (pattern_code, count_code, index_code) = self.size_codes();
        let mut bits =
            (csgp_width(pattern_code) + csgp_width(count_code)) as usize * self.patterns.len();
        for pattern in &self.patterns {
            bits += csgp_width(index_code) as usize * pattern.indices.len();
        }
        let parameter = if self.grouping_type_parameter.is_some() {
            core::mem::size_of::<u32>()
        } else {
            0
        };
        crate::box_types::box_len(
            BOX_HEADER_MIN_SIZE
                + FULLBOX_EXTRA_SIZE
                + core::mem::size_of::<u32>()
                + parameter
                + core::mem::size_of::<u32>()
                + bits.div_ceil(u8::BITS as usize),
        )
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = crate::box_types::write_full_box_header(
            buf,
            &CSGP,
            need,
            self.version,
            self.flags,
            false,
        )?;
        buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&self.grouping_type.to_be_bytes());
        c += core::mem::size_of::<u32>();
        if let Some(parameter) = self.grouping_type_parameter {
            buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&parameter.to_be_bytes());
            c += core::mem::size_of::<u32>();
        }
        let pattern_count = broadcast_core::len::fit_u32(self.patterns.len(), "pattern_count")?;
        buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&pattern_count.to_be_bytes());
        c += core::mem::size_of::<u32>();
        let (pattern_code, count_code, index_code) = self.size_codes();
        let mut writer = broadcast_core::bits::BitBuf::new();
        for pattern in &self.patterns {
            write_csgp_bits(
                &mut writer,
                pattern.indices.len() as u64,
                csgp_width(pattern_code),
                "csgp pattern_length",
            )?;
            write_csgp_bits(
                &mut writer,
                u64::from(pattern.sample_count),
                csgp_width(count_code),
                "csgp sample_count",
            )?;
        }
        for pattern in &self.patterns {
            for &index in &pattern.indices {
                write_csgp_bits(
                    &mut writer,
                    u64::from(index),
                    csgp_width(index_code),
                    "csgp index",
                )?;
            }
        }
        writer.align_to_byte();
        let body = writer.as_bytes();
        buf[c..c + body.len()].copy_from_slice(body);
        c += body.len();
        Ok(c)
    }
}

fn write_csgp_bits(
    writer: &mut broadcast_core::bits::BitBuf,
    value: u64,
    width: u32,
    what: &'static str,
) -> Result<()> {
    writer
        .write_bits(value, width)
        .map_err(|_| Error::InvalidInput(what))
}

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_core::Serialize;

    // -----------------------------------------------------------------------
    // prft
    // -----------------------------------------------------------------------

    #[test]
    fn prft_round_trip_v0() {
        let b = ProducerReferenceTimeBox {
            version: 0,
            flags: 0,
            reference_track_id: TrackId::new(1),
            ntp_timestamp: 0x1234_5678_9abc_def0,
            media_time: 0x0000_0000_0000_1234,
        };
        let bytes = b.try_to_bytes().unwrap();
        assert_eq!(bytes.len(), 8 + 4 + 4 + 8 + 4);
        let parsed = ProducerReferenceTimeBox::parse(&bytes).unwrap();
        assert_eq!(parsed, b);
    }

    #[test]
    fn prft_round_trip_v1() {
        let b = ProducerReferenceTimeBox {
            version: 1,
            flags: 0x000018,
            reference_track_id: TrackId::new(1),
            ntp_timestamp: 0xedefe3e3_a7ae147a,
            media_time: 0x0000_0000_0000_1c20,
        };
        let bytes = b.try_to_bytes().unwrap();
        assert_eq!(bytes.len(), 8 + 4 + 4 + 8 + 8);
        let parsed = ProducerReferenceTimeBox::parse(&bytes).unwrap();
        assert_eq!(parsed, b);
    }

    /// Every `prft` field with a non-zero upper byte in both versions, and the
    /// exact byte count `serialize_into` reports (ISO/IEC 14496-12:2026
    /// §8.14.5.2). Truncating anywhere must be a reported shortfall.
    #[test]
    fn prft_fields_and_lengths_are_pinned_in_both_versions() {
        for (version, media_time, len) in [
            (0u8, 0x3132_3334u64, 28usize),
            (1, 0x4142_4344_4546_4748, 32),
        ] {
            let b = ProducerReferenceTimeBox {
                version,
                flags: 0x01_0203,
                reference_track_id: TrackId::new(0x1112_1314),
                ntp_timestamp: 0x2122_2324_2526_2728,
                media_time,
            };
            assert_eq!(b.serialized_len(), len, "v{version} serialized_len");
            let mut buf = alloc::vec![0u8; b.serialized_len()];
            let n = b.serialize_into(&mut buf).unwrap();
            assert_eq!(n, len, "v{version} serialize_into return value");

            let parsed = ProducerReferenceTimeBox::parse(&buf).unwrap();
            assert_eq!(parsed.version, version);
            assert_eq!(parsed.flags, 0x01_0203);
            assert_eq!(parsed.reference_track_id, TrackId::new(0x1112_1314));
            assert_eq!(parsed.ntp_timestamp, 0x2122_2324_2526_2728);
            assert_eq!(parsed.media_time, media_time);

            for cut in 0..buf.len() {
                match ProducerReferenceTimeBox::parse(&buf[..cut]) {
                    Err(Error::BufferTooShort(_)) => {}
                    Err(other) => {
                        panic!("v{version} {cut}-byte prft: expected BufferTooShort, got {other:?}")
                    }
                    Ok(_) => panic!("v{version} {cut}-byte prft parsed"),
                }
            }
            for cut in 0..buf.len() - 8 {
                match ProducerReferenceTimeBox::parse_body(&buf[8..8 + cut]) {
                    Err(Error::BufferTooShort(_)) => {}
                    Err(other) => panic!(
                        "v{version} {cut}-byte prft body: expected BufferTooShort, got {other:?}"
                    ),
                    Ok(_) => panic!("v{version} {cut}-byte prft body parsed"),
                }
            }
            match ProducerReferenceTimeBox::parse(&buf[..27]) {
                Err(Error::BufferTooShort(b)) => {
                    assert_eq!(
                        (b.need, b.have, b.what),
                        (28, 27, "prft box"),
                        "v{version} box-level minimum"
                    );
                }
                other => panic!("v{version}: expected BufferTooShort, got {other:?}"),
            }
        }
    }

    // -----------------------------------------------------------------------
    // sgpd
    // -----------------------------------------------------------------------

    /// `sgpd` version 2 keeps `default_length` and adds
    /// `default_group_description_index` (ISO/IEC 14496-12:2026 §8.9.3.2).
    #[test]
    fn sgpd_v2_round_trips_with_the_group_index() {
        let b = SampleGroupDescriptionBox {
            version: 2,
            flags: 0,
            grouping_type: GROUPING_TYPE_ROLL,
            default_length: 2,
            default_group_description_index: Some(3),
            entries: alloc::vec![SgpdEntry::Roll { roll_distance: -1 }],
        };
        let bytes = b.try_to_bytes().unwrap();
        // 8 header + 4 FullBox + 4 grouping_type + 4 default_length + 4 index
        // + 4 entry_count + 2 roll_distance (default_length 2).
        assert_eq!(bytes.len(), 30, "v2 carries default_length and the index");
        let parsed = SampleGroupDescriptionBox::parse(&bytes).unwrap();
        assert_eq!(parsed.version, 2);
        assert_eq!(parsed.default_group_description_index, Some(3));
        assert_eq!(parsed.entries, b.entries);
        assert_eq!(parsed.try_to_bytes().unwrap(), bytes, "v2 round-trip");
    }

    /// ISO/IEC 14496-12:2026 §8.9.3.2: `default_length` is present for every
    /// version >= 1 and `default_group_description_index` is *additionally*
    /// present for version >= 2, so a version 2 or 3 body carries both before
    /// `entry_count`.
    fn sgpd_v2_plus_body(version: u8) -> [u8; 30] {
        [
            0, 0, 0, 30, b's', b'g', b'p', b'd', version, 0, 0, 0, b'r', b'o', b'l', b'l', 0, 0, 0,
            2, 0, 0, 0, 1, 0, 0, 0, 1, 0xFF, 0xFF,
        ]
    }

    #[test]
    fn sgpd_v2_and_v3_carry_default_length_and_group_index() {
        for version in [2u8, 3] {
            let bytes = sgpd_v2_plus_body(version);
            let parsed = SampleGroupDescriptionBox::parse(&bytes).unwrap();
            assert_eq!(parsed.default_length, 2, "v{version} default_length");
            assert_eq!(parsed.default_group_description_index, Some(1));
            assert_eq!(
                parsed.entries,
                alloc::vec![SgpdEntry::Roll { roll_distance: -1 }]
            );
            assert_eq!(parsed.is_essential(), version == SGPD_VERSION_ESSENTIAL);
            assert_eq!(
                parsed.try_to_bytes().unwrap(),
                bytes,
                "v{version} round trip"
            );
        }
    }

    #[test]
    fn sgpd_v2_per_entry_description_length_when_default_length_is_zero() {
        let bytes: [u8; 39] = [
            0, 0, 0, 39, b's', b'g', b'p', b'd', 2, 0, 0, 0, b'x', b'y', b'z', b'w', 0, 0, 0, 0, 0,
            0, 0, 2, 0, 0, 0, 2, 0, 0, 0, 1, 0xAA, 0, 0, 0, 2, 0xBB, 0xCC,
        ];
        let parsed = SampleGroupDescriptionBox::parse(&bytes).unwrap();
        assert_eq!(parsed.default_group_description_index, Some(2));
        assert_eq!(
            parsed.entries,
            alloc::vec![
                SgpdEntry::Unknown(alloc::vec![0xAA]),
                SgpdEntry::Unknown(alloc::vec![0xBB, 0xCC])
            ]
        );
        assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
    }

    /// A v2 box whose index field is absent from the struct cannot be written:
    /// guessing a value would mislabel the box's own syntax.
    #[test]
    fn sgpd_v2_without_the_group_index_errors() {
        let b = SampleGroupDescriptionBox {
            version: 2,
            flags: 0,
            grouping_type: GROUPING_TYPE_ROLL,
            default_length: 0,
            default_group_description_index: None,
            entries: alloc::vec![SgpdEntry::Roll { roll_distance: -1 }],
        };
        let err = b.try_to_bytes().unwrap_err();
        assert!(
            matches!(err, Error::InvalidInput(_)),
            "expected InvalidInput, got {err:?}"
        );
    }

    /// A `roll` description is exactly 2 bytes (§10.1.2). A box whose v1
    /// `default_length` is larger carries something this crate does not model
    /// after the distance; keeping only the first 2 bytes re-serialized the
    /// entry 2 bytes wide and shrank the whole entry list (audit r05-W15).
    #[test]
    fn sgpd_roll_wider_than_two_bytes_stays_opaque_and_round_trips() {
        // v1, default_length = 4, one 4-byte `roll` description.
        let body: &[u8] = &[
            1, 0, 0, 0, // version 1, flags 0
            b'r', b'o', b'l', b'l', // grouping_type
            0, 0, 0, 4, // default_length = 4
            0, 0, 0, 1, // entry_count = 1
            0xFF, 0xFE, 0xAB, 0xCD, // a 4-byte description
        ];
        let parsed = SampleGroupDescriptionBox::parse_body(body).unwrap();
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(
            parsed.entries[0],
            SgpdEntry::Unknown(alloc::vec![0xFF, 0xFE, 0xAB, 0xCD]),
            "a roll entry wider than 2 bytes must stay opaque, not be narrowed"
        );
        let mut out = alloc::vec![0u8; parsed.serialized_len()];
        let n = parsed.serialize_into(&mut out).unwrap();
        assert_eq!(&out[8..n], body, "the wider entry must round-trip");
    }

    #[test]
    fn sgpd_round_trip_roll_v1() {
        let b = SampleGroupDescriptionBox {
            version: 1,
            flags: 0,
            grouping_type: GROUPING_TYPE_ROLL,
            default_length: 2,
            default_group_description_index: None,
            entries: vec![SgpdEntry::Roll { roll_distance: -1 }],
        };
        let bytes = b.try_to_bytes().unwrap();
        let parsed = SampleGroupDescriptionBox::parse(&bytes).unwrap();
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0], SgpdEntry::Roll { roll_distance: -1 });
        assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
    }

    #[test]
    fn sgpd_round_trip_prol_v1() {
        let b = SampleGroupDescriptionBox {
            version: 1,
            flags: 0,
            grouping_type: GROUPING_TYPE_PROL,
            default_length: 2,
            default_group_description_index: None,
            entries: vec![SgpdEntry::Prol { roll_distance: 80 }],
        };
        let bytes = b.try_to_bytes().unwrap();
        let parsed = SampleGroupDescriptionBox::parse(&bytes).unwrap();
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0], SgpdEntry::Prol { roll_distance: 80 });
        assert_eq!(parsed.entries[0].pre_roll_samples(), Some(80));
        assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
    }

    #[test]
    fn only_a_pre_roll_statement_reports_a_pre_roll() {
        assert_eq!(
            SgpdEntry::Roll { roll_distance: -4 }.pre_roll_samples(),
            Some(4)
        );
        assert_eq!(
            SgpdEntry::Prol { roll_distance: 80 }.pre_roll_samples(),
            Some(80)
        );
        assert_eq!(
            SgpdEntry::Roll { roll_distance: 4 }.pre_roll_samples(),
            None,
            "a positive roll distance is post-roll, not a pre-roll requirement"
        );
        assert_eq!(
            SgpdEntry::Prol { roll_distance: -1 }.pre_roll_samples(),
            None
        );
    }

    #[test]
    fn sgpd_round_trip_two_roll_entries() {
        let b = SampleGroupDescriptionBox {
            version: 1,
            flags: 0,
            grouping_type: GROUPING_TYPE_ROLL,
            default_length: 2,
            default_group_description_index: None,
            entries: vec![
                SgpdEntry::Roll { roll_distance: -4 },
                SgpdEntry::Roll { roll_distance: -1 },
            ],
        };
        let bytes = b.try_to_bytes().unwrap();
        let parsed = SampleGroupDescriptionBox::parse(&bytes).unwrap();
        assert_eq!(parsed.entries.len(), 2);
        assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
    }

    /// A version-1 `sgpd` may declare zero entries (§8.9.3.2); the
    /// `default_length` field is still present and is written as 0.
    #[test]
    fn sgpd_v1_empty_writes_default_length_zero() {
        let b = SampleGroupDescriptionBox {
            version: 1,
            flags: 0,
            grouping_type: GROUPING_TYPE_ROLL,
            default_length: 0,
            default_group_description_index: None,
            entries: alloc::vec![],
        };
        let bytes = b.try_to_bytes().unwrap();
        assert_eq!(
            bytes,
            [
                0, 0, 0, 24, b's', b'g', b'p', b'd', 1, 0, 0, 0, b'r', b'o', b'l', b'l', 0, 0, 0,
                0, 0, 0, 0, 0
            ]
        );
        assert_eq!(b.serialized_len(), 24);
        let parsed = SampleGroupDescriptionBox::parse(&bytes).unwrap();
        assert!(parsed.entries.is_empty(), "entry_count is zero");
        assert_eq!(parsed.default_length, 0);
    }

    /// Version 0 has no per-entry `description_length` (§8.9.3.2), so entries
    /// of different widths are written back to back with no 4-byte prefixes.
    #[test]
    fn sgpd_v0_non_uniform_entries_have_no_length_prefixes() {
        let b = SampleGroupDescriptionBox {
            version: 0,
            flags: 0,
            grouping_type: GROUPING_TYPE_ROLL,
            default_length: 0,
            default_group_description_index: None,
            entries: alloc::vec![
                SgpdEntry::Roll { roll_distance: -1 },
                SgpdEntry::Unknown(alloc::vec![1, 2, 3]),
            ],
        };
        let bytes = b.try_to_bytes().unwrap();
        assert_eq!(
            bytes,
            [
                0, 0, 0, 25, b's', b'g', b'p', b'd', 0, 0, 0, 0, b'r', b'o', b'l', b'l', 0, 0, 0,
                2, 0xFF, 0xFF, 1, 2, 3
            ]
        );
        assert_eq!(b.serialized_len(), 25);
        let parsed = SampleGroupDescriptionBox::parse(&bytes).unwrap();
        assert_eq!(parsed.entries.len(), 2);
        assert_eq!(parsed.entries[0], SgpdEntry::Roll { roll_distance: -1 });
    }

    /// Version 1 with `default_length == 0`: every entry is preceded by its own
    /// `description_length` (§8.9.3.2). Values here have non-zero upper bytes so
    /// a read from the wrong byte inside the u32 changes the answer.
    fn sgpd_v1_variable_body() -> &'static [u8] {
        &[
            1, 0, 0, 0, // version 1, flags 0
            b'r', b'o', b'l', b'l', // grouping_type
            0, 0, 0, 0, // default_length = 0 (per-entry lengths)
            0, 0, 0, 2, // entry_count = 2
            0, 0, 0, 2, 0xFF, 0xFE, // entry 0: length 2, roll_distance -2
            0, 0, 0, 3, 0x01, 0x02, 0x03, // entry 1: length 3, opaque
        ]
    }

    #[test]
    fn sgpd_v1_zero_default_length_reads_each_entry_length() {
        let body = sgpd_v1_variable_body();
        let parsed = SampleGroupDescriptionBox::parse_body(body).unwrap();
        assert_eq!(parsed.version, 1);
        assert_eq!(parsed.default_length, 0);
        assert_eq!(parsed.default_group_description_index, None);
        assert_eq!(
            parsed.entries,
            alloc::vec![
                SgpdEntry::Roll { roll_distance: -2 },
                SgpdEntry::Unknown(alloc::vec![0x01, 0x02, 0x03]),
            ]
        );
        let mut out = alloc::vec![0u8; parsed.serialized_len()];
        let n = parsed.serialize_into(&mut out).unwrap();
        assert_eq!(&out[8..n], body, "byte-exact round trip");
    }

    #[test]
    fn sgpd_v2_group_index_keeps_every_non_zero_byte() {
        let body: &[u8] = &[
            2, 0, 0, 0, // version 2, flags 0
            b'r', b'o', b'l', b'l', // grouping_type
            0, 0, 0, 2, // default_length = 2
            0x01, 0x02, 0x03, 0x04, // default_group_description_index
            0, 0, 0, 1, // entry_count = 1
            0xFF, 0xFF, // roll_distance -1
        ];
        let parsed = SampleGroupDescriptionBox::parse_body(body).unwrap();
        assert_eq!(parsed.default_length, 2);
        assert_eq!(parsed.default_group_description_index, Some(0x0102_0304));
        assert_eq!(
            parsed.entries,
            alloc::vec![SgpdEntry::Roll { roll_distance: -1 }]
        );
        let mut out = alloc::vec![0u8; parsed.serialized_len()];
        let n = parsed.serialize_into(&mut out).unwrap();
        assert_eq!(&out[8..n], body, "byte-exact round trip");
    }

    /// A `default_length` above 255: its second byte is what a shifted read
    /// loses, and 65536 is small enough to build the one entry it declares.
    #[test]
    fn sgpd_v1_default_length_above_255_parses_its_entry() {
        let mut body = alloc::vec![
            1, 0, 0, 0, // version 1, flags 0
            b'r', b'o', b'l', b'l', // grouping_type
            0x00, 0x01, 0x00, 0x00, // default_length = 65536
            0, 0, 0, 1, // entry_count = 1
        ];
        body.extend(core::iter::repeat_n(0xABu8, 0x0001_0000));
        let parsed = SampleGroupDescriptionBox::parse_body(&body).unwrap();
        assert_eq!(parsed.default_length, 0x0001_0000);
        assert_eq!(parsed.entries.len(), 1);
        let SgpdEntry::Unknown(ref entry) = parsed.entries[0] else {
            panic!("a roll entry wider than 2 bytes stays opaque");
        };
        assert_eq!(entry.len(), 0x0001_0000);
        assert_eq!(entry[0], 0xAB);
        assert_eq!(entry[0xFFFF], 0xAB);
        let mut out = alloc::vec![0u8; parsed.serialized_len()];
        let n = parsed.serialize_into(&mut out).unwrap();
        assert_eq!(&out[8..n], body, "byte-exact round trip");
    }

    fn assert_sgpd_prefixes_are_buffer_too_short(body: &[u8]) {
        for cut in 0..body.len() {
            match SampleGroupDescriptionBox::parse_body(&body[..cut]) {
                Err(Error::BufferTooShort(_)) => {}
                Err(other) => {
                    panic!("{cut}-byte sgpd body: expected BufferTooShort, got {other:?}")
                }
                Ok(parsed) => panic!(
                    "{cut}-byte sgpd body of {} parsed to {} entries",
                    body.len(),
                    parsed.entries.len()
                ),
            }
        }
        SampleGroupDescriptionBox::parse_body(body).expect("the whole body parses");
    }

    #[test]
    fn sgpd_truncations_are_buffer_too_short_with_the_exact_shortfall() {
        assert_sgpd_prefixes_are_buffer_too_short(&[
            0, 0, 0, 0, b'r', b'o', b'l', b'l', 0, 0, 0, 2, 0xFF, 0xFF, 0x00, 0x02,
        ]);
        assert_sgpd_prefixes_are_buffer_too_short(sgpd_v1_variable_body());
        assert_sgpd_prefixes_are_buffer_too_short(&[
            1, 0, 0, 0, b'r', b'o', b'l', b'l', 0, 0, 0, 2, 0, 0, 0, 2, 0xFF, 0xFF, 0x00, 0x02,
        ]);
        assert_sgpd_prefixes_are_buffer_too_short(&[
            2, 0, 0, 0, b'r', b'o', b'l', b'l', 0, 0, 0, 2, 0x01, 0x02, 0x03, 0x04, 0, 0, 0, 1,
            0xFF, 0xFF,
        ]);

        match SampleGroupDescriptionBox::parse(&[0u8; 12]) {
            Err(Error::BufferTooShort(b)) => {
                assert_eq!((b.need, b.have, b.what), (16, 12, "sgpd box"));
            }
            other => panic!("expected BufferTooShort, got {other:?}"),
        }
        match SampleGroupDescriptionBox::parse_body(&[0u8; 4]) {
            Err(Error::BufferTooShort(b)) => {
                assert_eq!((b.need, b.have, b.what), (12, 4, "sgpd body"));
            }
            other => panic!("expected BufferTooShort, got {other:?}"),
        }
        match SampleGroupDescriptionBox::parse_body(&[0u8; 9]) {
            Err(Error::BufferTooShort(b)) => {
                assert_eq!((b.need, b.have, b.what), (12, 9, "sgpd body"));
            }
            other => panic!("expected BufferTooShort, got {other:?}"),
        }
        let mut v1 = alloc::vec![1u8, 0, 0, 0];
        v1.extend_from_slice(b"roll");
        match SampleGroupDescriptionBox::parse_body(&v1) {
            Err(Error::BufferTooShort(b)) => {
                assert_eq!((b.need, b.have, b.what), (12, 8, "sgpd body"));
            }
            other => panic!("expected BufferTooShort, got {other:?}"),
        }
        let mut v2 = alloc::vec![2u8, 0, 0, 0];
        v2.extend_from_slice(b"roll");
        v2.extend_from_slice(&[0, 0, 0, 2]);
        v2.extend_from_slice(&[0x01, 0x02, 0x03]);
        match SampleGroupDescriptionBox::parse_body(&v2) {
            Err(Error::BufferTooShort(b)) => {
                assert_eq!(
                    (b.need, b.have, b.what),
                    (16, 15, "sgpd default_group_description_index")
                );
            }
            other => panic!("expected BufferTooShort, got {other:?}"),
        }
        let mut two_entries = alloc::vec![1u8, 0, 0, 0];
        two_entries.extend_from_slice(b"roll");
        two_entries.extend_from_slice(&[0, 0, 0, 0]);
        two_entries.extend_from_slice(&[0, 0, 0, 2]);
        two_entries.extend_from_slice(&[0, 0, 0, 100]);
        two_entries.extend(core::iter::repeat_n(0xABu8, 100));
        two_entries.extend_from_slice(&[0xAA, 0xBB]);
        match SampleGroupDescriptionBox::parse_body(&two_entries) {
            Err(Error::BufferTooShort(b)) => {
                assert_eq!(
                    (b.need, b.have, b.what),
                    (124, 122, "sgpd description_length")
                );
            }
            other => panic!("expected BufferTooShort, got {other:?}"),
        }
    }

    #[test]
    fn sgpd_hostile_entry_count_is_rejected_with_the_exact_need() {
        let mut v0_roll = alloc::vec![0u8, 0, 0, 0];
        v0_roll.extend_from_slice(b"roll");
        v0_roll.extend_from_slice(&0x7FFF_FFFFu32.to_be_bytes());
        match SampleGroupDescriptionBox::parse_body(&v0_roll) {
            Err(Error::BufferTooShort(b)) => {
                assert_eq!(
                    (b.need, b.have, b.what),
                    (4_294_967_306, 12, "sgpd entries")
                );
            }
            other => panic!("expected BufferTooShort, got {other:?}"),
        }

        let mut v1_variable = alloc::vec![1u8, 0, 0, 0];
        v1_variable.extend_from_slice(b"roll");
        v1_variable.extend_from_slice(&[0, 0, 0, 0]);
        v1_variable.extend_from_slice(&0x7FFF_FFFFu32.to_be_bytes());
        match SampleGroupDescriptionBox::parse_body(&v1_variable) {
            Err(Error::BufferTooShort(b)) => {
                assert_eq!(
                    (b.need, b.have, b.what),
                    (8_589_934_604, 16, "sgpd entries")
                );
            }
            other => panic!("expected BufferTooShort, got {other:?}"),
        }

        let mut v1_fixed = alloc::vec![1u8, 0, 0, 0];
        v1_fixed.extend_from_slice(b"roll");
        v1_fixed.extend_from_slice(&[0, 0, 0, 8]);
        v1_fixed.extend_from_slice(&[0, 0, 0, 3]);
        v1_fixed.extend(core::iter::repeat_n(0xABu8, 20));
        match SampleGroupDescriptionBox::parse_body(&v1_fixed) {
            Err(Error::BufferTooShort(b)) => {
                assert_eq!((b.need, b.have, b.what), (40, 36, "sgpd entries"));
            }
            other => panic!("expected BufferTooShort, got {other:?}"),
        }

        let mut hostile_len = alloc::vec![1u8, 0, 0, 0];
        hostile_len.extend_from_slice(b"roll");
        hostile_len.extend_from_slice(&[0, 0, 0, 0]);
        hostile_len.extend_from_slice(&[0, 0, 0, 1]);
        hostile_len.extend_from_slice(&0x7FFF_FFFFu32.to_be_bytes());
        match SampleGroupDescriptionBox::parse_body(&hostile_len) {
            Err(Error::BufferTooShort(b)) => {
                assert_eq!(
                    (b.need, b.have, b.what),
                    (2_147_483_667, 20, "sgpd entry body")
                );
            }
            other => panic!("expected BufferTooShort, got {other:?}"),
        }

        let mut v0_unknown = alloc::vec![0u8, 0, 0, 0];
        v0_unknown.extend_from_slice(b"abcd");
        v0_unknown.extend_from_slice(&[0, 0, 0, 2]);
        v0_unknown.push(0xAB);
        match SampleGroupDescriptionBox::parse_body(&v0_unknown) {
            Err(Error::BufferTooShort(b)) => {
                assert_eq!((b.need, b.have, b.what), (14, 13, "sgpd entries"));
            }
            other => panic!("expected BufferTooShort, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // sbgp
    // -----------------------------------------------------------------------

    #[test]
    fn sbgp_round_trip_v0() {
        let b = SampleToGroupBox {
            version: 0,
            flags: 0,
            grouping_type: GROUPING_TYPE_ROLL,
            grouping_type_parameter: None,
            entries: vec![
                SbgpEntry {
                    sample_count: 1,
                    group_description_index: 1,
                },
                SbgpEntry {
                    sample_count: 10,
                    group_description_index: 0,
                },
            ],
        };
        let bytes = b.try_to_bytes().unwrap();
        let parsed = SampleToGroupBox::parse(&bytes).unwrap();
        assert_eq!(parsed, b);
    }

    #[test]
    fn sbgp_round_trip_v1() {
        let b = SampleToGroupBox {
            version: 1,
            flags: 0,
            grouping_type: GROUPING_TYPE_ROLL,
            grouping_type_parameter: Some(0xDEAD_BEEF),
            entries: vec![SbgpEntry {
                sample_count: 5,
                group_description_index: 1,
            }],
        };
        let bytes = b.try_to_bytes().unwrap();
        let parsed = SampleToGroupBox::parse(&bytes).unwrap();
        assert_eq!(parsed, b);
    }

    // -----------------------------------------------------------------------
    // subs
    // -----------------------------------------------------------------------

    #[test]
    fn subs_round_trip_v0() {
        let b = SubSampleInformationBox {
            version: 0,
            flags: 0,
            entries: vec![SubsEntry {
                sample_delta: 1,
                subsamples: vec![
                    SubSampleDescriptor {
                        subsample_size: 100,
                        subsample_priority: 255,
                        discardable: 0,
                        codec_specific_parameters: 0,
                    },
                    SubSampleDescriptor {
                        subsample_size: 200,
                        subsample_priority: 128,
                        discardable: 1,
                        codec_specific_parameters: 0,
                    },
                ],
            }],
        };
        let bytes = b.try_to_bytes().unwrap();
        let parsed = SubSampleInformationBox::parse(&bytes).unwrap();
        assert_eq!(parsed, b);
        assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
    }

    #[test]
    fn subs_round_trip_v1() {
        let b = SubSampleInformationBox {
            version: 1,
            flags: 0,
            entries: vec![SubsEntry {
                sample_delta: 5,
                subsamples: vec![SubSampleDescriptor {
                    subsample_size: 0x0001_2345,
                    subsample_priority: 200,
                    discardable: 0,
                    codec_specific_parameters: 0xABCD_EF01,
                }],
            }],
        };
        let bytes = b.try_to_bytes().unwrap();
        let parsed = SubSampleInformationBox::parse(&bytes).unwrap();
        assert_eq!(parsed, b);
        assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
    }

    #[test]
    fn subs_empty_round_trip() {
        let b = SubSampleInformationBox {
            version: 0,
            flags: 0,
            entries: vec![],
        };
        let bytes = b.try_to_bytes().unwrap();
        let parsed = SubSampleInformationBox::parse(&bytes).unwrap();
        assert_eq!(parsed, b);
    }

    /// A `subs` version 2 box (ISO/IEC 14496-12:2026 §8.7.7.2): one entry
    /// (`sample_delta` 256) with one explicit 100-byte sub-sample whose
    /// priority and 4-byte `codec_specific_parameters` are present.
    const SUBS_V2_EXPLICIT: [u8; 31] = [
        0, 0, 0, 31, b's', b'u', b'b', b's', 2, 0, 0, 0, 0, 0, 0, 1, 0xC0, 0, 0, 1, 0, 0, 1, 0,
        100, 0x7C, 0x80, 0xAA, 0xBB, 0xCC, 0xDD,
    ];

    #[test]
    fn subs_v2_is_not_misparsed_as_v0() {
        assert!(
            SubSampleInformationBox::parse(&SUBS_V2_EXPLICIT).is_err(),
            "a version 2 subs must not be read with the version 0 layout"
        );
    }

    #[test]
    fn subs_v2_explicit_round_trips() {
        let parsed = SubSampleInformationBoxV2::parse(&SUBS_V2_EXPLICIT).unwrap();
        assert!(parsed.is_size_16_bits && parsed.is_short_ref);
        assert_eq!(parsed.entries[0].sample_delta, 256);
        assert_eq!(
            parsed.entries[0].subsamples,
            alloc::vec![SubsV2Subsample {
                subsample_size: 100,
                source: SubsV2Source::Explicit(SubsampleProperties {
                    discardable: true,
                    subsample_priority: Some(0x80),
                    codec_specific_parameters: Some(CodecSpecificParameters {
                        len: 4,
                        value: 0xAABB_CCDD
                    }),
                    reserved: 0,
                }),
            }]
        );
        assert_eq!(parsed.try_to_bytes().unwrap(), SUBS_V2_EXPLICIT);
    }

    #[test]
    fn subs_v2_reference_forms_round_trip() {
        // 32-bit sizes, 15-bit references, a sample with two sub-samples
        // (one reference, one explicit with a 1-byte csp and no priority),
        // then a 16-bit-size / 7-bit-reference box.
        let wide = SubSampleInformationBoxV2 {
            flags: 5,
            is_size_16_bits: false,
            is_short_ref: false,
            reserved: 0,
            entries: alloc::vec![SubsV2Entry {
                sample_delta: 3,
                subsamples: alloc::vec![
                    SubsV2Subsample {
                        subsample_size: 0x0001_0000,
                        source: SubsV2Source::Reference {
                            reference_idx: 0x7ABC
                        },
                    },
                    SubsV2Subsample {
                        subsample_size: 7,
                        source: SubsV2Source::Explicit(SubsampleProperties {
                            discardable: false,
                            subsample_priority: None,
                            codec_specific_parameters: Some(CodecSpecificParameters {
                                len: 1,
                                value: 0x42
                            }),
                            reserved: 0,
                        }),
                    },
                ],
            }],
        };
        let bytes = wide.try_to_bytes().unwrap();
        assert_eq!(SubSampleInformationBoxV2::parse(&bytes).unwrap(), wide);
        let mut narrow = wide.clone();
        narrow.is_size_16_bits = true;
        narrow.is_short_ref = true;
        narrow.entries[0].subsamples[0].subsample_size = 9;
        narrow.entries[0].subsamples[0].source = SubsV2Source::Reference { reference_idx: 127 };
        let bytes = narrow.try_to_bytes().unwrap();
        assert_eq!(SubSampleInformationBoxV2::parse(&bytes).unwrap(), narrow);
    }

    #[test]
    fn subs_v2_out_of_range_fields_error() {
        let mut b = SubSampleInformationBoxV2::parse(&SUBS_V2_EXPLICIT).unwrap();
        b.entries[0].subsamples[0].subsample_size = 0x1_0000;
        assert!(b.try_to_bytes().is_err(), "size exceeds 16 bits");
        b.entries[0].subsamples[0].source = SubsV2Source::Reference { reference_idx: 128 };
        b.entries[0].subsamples[0].subsample_size = 1;
        assert!(b.try_to_bytes().is_err(), "reference_idx exceeds 7 bits");
    }

    #[test]
    fn ssrt_round_trips_short_and_long_counts() {
        let entries = alloc::vec![
            SubsampleProperties {
                discardable: true,
                subsample_priority: Some(9),
                codec_specific_parameters: None,
                reserved: 0,
            },
            SubsampleProperties {
                discardable: false,
                subsample_priority: None,
                codec_specific_parameters: Some(CodecSpecificParameters {
                    len: 3,
                    value: 0x0012_3456
                }),
                reserved: 0,
            },
        ];
        for is_short_ref in [true, false] {
            let b = SubSampleReferenceTableBox {
                version: 0,
                flags: 1,
                is_short_ref,
                entries: entries.clone(),
            };
            let bytes = b.try_to_bytes().unwrap();
            assert_eq!(SubSampleReferenceTableBox::parse(&bytes).unwrap(), b);
        }
        let short = SubSampleReferenceTableBox {
            version: 0,
            flags: 0,
            is_short_ref: true,
            entries: entries.clone(),
        }
        .try_to_bytes()
        .unwrap();
        assert_eq!(
            short,
            [
                0, 0, 0, 19, b's', b's', b'r', b't', 0, 0, 0, 0, 0x82, 0xC0, 9, 0x30, 0x12, 0x34,
                0x56
            ]
        );
    }

    fn subsample(size: u32) -> SubSampleDescriptor {
        SubSampleDescriptor {
            subsample_size: size,
            subsample_priority: 0,
            discardable: 0,
            codec_specific_parameters: 0,
        }
    }

    /// A version-0 `subsample_size` past `u16::MAX` cannot fit its wire field
    /// (#1129): unfixed, `(ss.subsample_size as u16)` silently truncated it.
    #[test]
    fn subs_v0_oversized_subsample_size_errors() {
        let subs = SubSampleInformationBox {
            version: 0,
            flags: 0,
            entries: alloc::vec![SubsEntry {
                sample_delta: 1,
                subsamples: alloc::vec![subsample(65536)],
            }],
        };
        let err = subs.try_to_bytes().unwrap_err();
        assert!(
            matches!(
                err,
                Error::FieldOverflow(broadcast_core::len::FieldOverflow {
                    field: "subsample_size",
                    ..
                })
            ),
            "expected FieldOverflow for subsample_size, got {err:?}"
        );
    }

    /// The boundary: exactly `u16::MAX` still round-trips in version 0.
    #[test]
    fn subs_v0_max_subsample_size_round_trips() {
        let subs = SubSampleInformationBox {
            version: 0,
            flags: 0,
            entries: alloc::vec![SubsEntry {
                sample_delta: 1,
                subsamples: alloc::vec![subsample(65535)],
            }],
        };
        let bytes = subs.try_to_bytes().unwrap();
        let parsed = SubSampleInformationBox::parse(&bytes).unwrap();
        assert_eq!(parsed.entries[0].subsamples[0].subsample_size, 65535);
    }

    /// An entry with more than 65 535 subsamples cannot fit the 16-bit
    /// `subsample_count` field (#1129): unfixed, `.len() as u16` wrapped.
    #[test]
    fn subs_oversized_subsample_count_errors() {
        let subs = SubSampleInformationBox {
            version: 1,
            flags: 0,
            entries: alloc::vec![SubsEntry {
                sample_delta: 1,
                subsamples: (0..65536).map(|_| subsample(1)).collect(),
            }],
        };
        let err = subs.try_to_bytes().unwrap_err();
        assert!(
            matches!(
                err,
                Error::FieldOverflow(broadcast_core::len::FieldOverflow {
                    field: "subsample_count",
                    ..
                })
            ),
            "expected FieldOverflow for subsample_count, got {err:?}"
        );
    }

    /// The boundary: exactly 65 535 subsamples still round-trips.
    #[test]
    fn subs_max_subsample_count_round_trips() {
        let subs = SubSampleInformationBox {
            version: 1,
            flags: 0,
            entries: alloc::vec![SubsEntry {
                sample_delta: 1,
                subsamples: (0..65535).map(|_| subsample(1)).collect(),
            }],
        };
        let bytes = subs.try_to_bytes().unwrap();
        let parsed = SubSampleInformationBox::parse(&bytes).unwrap();
        assert_eq!(parsed.entries[0].subsamples.len(), 65535);
    }

    fn subs_v2_with_csp(len: u8) -> SubSampleInformationBoxV2 {
        SubSampleInformationBoxV2 {
            flags: 0,
            is_size_16_bits: true,
            is_short_ref: true,
            reserved: 0,
            entries: alloc::vec![SubsV2Entry {
                sample_delta: 1,
                subsamples: alloc::vec![SubsV2Subsample {
                    subsample_size: 10,
                    source: SubsV2Source::Explicit(SubsampleProperties {
                        discardable: false,
                        subsample_priority: None,
                        codec_specific_parameters: Some(CodecSpecificParameters { len, value: 0 }),
                        reserved: 0,
                    }),
                }],
            }],
        }
    }

    /// `codec_specific_parameters` is 1..=4 bytes (§8.7.7.2): a zero or
    /// over-long length cannot produce a valid flags byte in either box.
    #[test]
    fn subs_v2_codec_specific_parameters_length_is_validated() {
        for len in [0u8, 5] {
            let err = subs_v2_with_csp(len).try_to_bytes().unwrap_err();
            assert!(
                matches!(err, Error::InvalidInput(_)),
                "subs csp len {len}: expected InvalidInput, got {err:?}"
            );
            let ssrt = SubSampleReferenceTableBox {
                version: 0,
                flags: 0,
                is_short_ref: true,
                entries: alloc::vec![SubsampleProperties {
                    discardable: false,
                    subsample_priority: None,
                    codec_specific_parameters: Some(CodecSpecificParameters { len, value: 0 }),
                    reserved: 0,
                }],
            };
            let err = ssrt.try_to_bytes().unwrap_err();
            assert!(
                matches!(err, Error::InvalidInput(_)),
                "ssrt csp len {len}: expected InvalidInput, got {err:?}"
            );
        }
        for len in [1u8, 4] {
            assert!(
                subs_v2_with_csp(len).try_to_bytes().is_ok(),
                "subs csp len {len} is legal"
            );
        }
    }
}
