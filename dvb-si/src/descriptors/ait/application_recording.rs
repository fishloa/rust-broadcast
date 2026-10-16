//! Application Recording Descriptor — ETSI TS 102 809 V1.3.1 §5.3.5.4, Table 21
//! (AIT tag 0x06; `storage_properties` values Table 22). PDF pp.40-41,
//! transcribed in `docs/descriptors/ts_102_809/21-*.md` and
//! `docs/enums/ts_102_809/22-*.md`, verified against the rendered pages.
//!
//! Body: six flag bits + `reserved`(2); `label_count`(8) × { `label_length`(8),
//! `label_char`×N1, `storage_properties`(2) + `reserved`(6) };
//! `component_tag_list_length`(8) + tags; `private_length`(8) + private bytes;
//! then `reserved_future_use`(8)×N4 up to `descriptor_length`.

use crate::descriptors::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for application_recording_descriptor (AIT namespace).
pub const TAG: u8 = 0x06;
const HEADER_LEN: usize = 2;

const SCHEDULED_RECORDING_FLAG: u8 = 0x80;
const TRICK_MODE_AWARE_FLAG: u8 = 0x40;
const TIME_SHIFT_FLAG: u8 = 0x20;
const DYNAMIC_FLAG: u8 = 0x10;
const AV_SYNCED_FLAG: u8 = 0x08;
const INITIATING_REPLAY_FLAG: u8 = 0x04;
const RESERVED_MASK: u8 = 0x03;
const LABEL_RESERVED_MASK: u8 = 0x3F;
const STORAGE_PROPERTIES_SHIFT: u32 = 6;

/// Spec default of the 2-bit `reserved` after the flags (all ones, TS 102 809
/// V1.3.1 NOTE: reserved bits are set to "1").
pub const RESERVED_DEFAULT: u8 = 0x03;
/// Spec default of the 6-bit `reserved` after `storage_properties` (all ones).
pub const LABEL_RESERVED_DEFAULT: u8 = 0x3F;

/// `storage_properties` (TS 102 809 §5.3.5.4 Table 22): importance of storing
/// a labelled part of the application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum StorageProperties {
    /// 0 — should not be stored.
    ShouldNotBeStored,
    /// 1 — critical to store.
    CriticalToStore,
    /// 2 — optional to store.
    OptionalToStore,
    /// 3 — reserved.
    Reserved(u8),
}

impl StorageProperties {
    /// Decode from the 2-bit wire value.
    #[must_use]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::ShouldNotBeStored,
            1 => Self::CriticalToStore,
            2 => Self::OptionalToStore,
            other => Self::Reserved(other),
        }
    }

    /// Encode to the 2-bit wire value.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::ShouldNotBeStored => 0,
            Self::CriticalToStore => 1,
            Self::OptionalToStore => 2,
            Self::Reserved(v) => v,
        }
    }

    /// Spec label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::ShouldNotBeStored => "should not be stored",
            Self::CriticalToStore => "critical to store",
            Self::OptionalToStore => "optional to store",
            Self::Reserved(_) => "reserved",
        }
    }
}
broadcast_common::impl_spec_display!(StorageProperties, Reserved);

/// One labelled part of the application and its storage importance.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct RecordingLabel<'a> {
    /// `label_char` bytes — labels a part of the application within its
    /// transport protocol. The format is defined by platform specifications
    /// (Note 3), so the bytes are opaque here.
    pub label: &'a [u8],
    /// `storage_properties`.
    pub storage_properties: StorageProperties,
    /// `reserved`(6) after `storage_properties`; parsed value kept, default
    /// [`LABEL_RESERVED_DEFAULT`].
    pub reserved: u8,
}

impl<'a> RecordingLabel<'a> {
    /// A label with the spec-default reserved bits.
    #[must_use]
    pub const fn new(label: &'a [u8], storage_properties: StorageProperties) -> Self {
        Self {
            label,
            storage_properties,
            reserved: LABEL_RESERVED_DEFAULT,
        }
    }
}

/// Application Recording Descriptor (AIT tag 0x06).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ApplicationRecordingDescriptor<'a> {
    /// `scheduled_recording_flag` — appropriate to record in a scheduled recording.
    pub scheduled_recording_flag: bool,
    /// `trick_mode_aware_flag` — the application is trick-mode aware.
    pub trick_mode_aware_flag: bool,
    /// `time_shift_flag` — appropriate to record in time-shift mode.
    pub time_shift_flag: bool,
    /// `dynamic_flag` — relies on dynamic broadcast data during execution.
    pub dynamic_flag: bool,
    /// `av_synced_flag` — requires stream events.
    pub av_synced_flag: bool,
    /// `initiating_replay_flag` — the application (not the receiver) starts playback.
    pub initiating_replay_flag: bool,
    /// `reserved`(2) after the flags; parsed value kept, default [`RESERVED_DEFAULT`].
    pub reserved: u8,
    /// Labelled parts (`label_count` entries).
    pub labels: Vec<RecordingLabel<'a>>,
    /// `component_tag` list — service components to record with the application.
    pub component_tags: Vec<u8>,
    /// `private` bytes (private extensions, opaque by definition).
    pub private: &'a [u8],
    /// Trailing `reserved_future_use`(8)×N4 bytes (future DVB extensions);
    /// empty in every defined edition, preserved if present.
    pub reserved_future_use: &'a [u8],
}

impl Default for ApplicationRecordingDescriptor<'_> {
    /// All flags clear, no labels/components/private data, spec-default reserved bits.
    fn default() -> Self {
        Self {
            scheduled_recording_flag: false,
            trick_mode_aware_flag: false,
            time_shift_flag: false,
            dynamic_flag: false,
            av_synced_flag: false,
            initiating_replay_flag: false,
            reserved: RESERVED_DEFAULT,
            labels: Vec::new(),
            component_tags: Vec::new(),
            private: &[],
            reserved_future_use: &[],
        }
    }
}

/// Take a `len`-byte-prefixed run from `body[pos..]`; returns `(bytes, next_pos)`.
fn take_prefixed<'a>(body: &'a [u8], pos: usize, what: &'static str) -> Result<(&'a [u8], usize)> {
    let len = *body.get(pos).ok_or(Error::InvalidDescriptor {
        tag: TAG,
        reason: what,
    })? as usize;
    let start = pos + 1;
    let end = start + len;
    body.get(start..end)
        .map(|s| (s, end))
        .ok_or(Error::InvalidDescriptor {
            tag: TAG,
            reason: what,
        })
}

impl<'a> Parse<'a> for ApplicationRecordingDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "ApplicationRecordingDescriptor",
            "unexpected tag for application_recording_descriptor",
        )?;
        let flags = *body.first().ok_or(Error::InvalidDescriptor {
            tag: TAG,
            reason: "application_recording_descriptor missing flags byte",
        })?;
        let label_count = *body.get(1).ok_or(Error::InvalidDescriptor {
            tag: TAG,
            reason: "application_recording_descriptor missing label_count",
        })? as usize;
        let mut pos = 2;
        let mut labels = Vec::with_capacity(label_count);
        for _ in 0..label_count {
            let (label, next) = take_prefixed(
                body,
                pos,
                "application_recording_descriptor label overruns body",
            )?;
            let props = *body.get(next).ok_or(Error::InvalidDescriptor {
                tag: TAG,
                reason: "application_recording_descriptor missing storage_properties byte",
            })?;
            labels.push(RecordingLabel {
                label,
                storage_properties: StorageProperties::from_u8(props >> STORAGE_PROPERTIES_SHIFT),
                reserved: props & LABEL_RESERVED_MASK,
            });
            pos = next + 1;
        }
        let (tags, next) = take_prefixed(
            body,
            pos,
            "application_recording_descriptor component_tag list overruns body",
        )?;
        let (private, next) = take_prefixed(
            body,
            next,
            "application_recording_descriptor private bytes overrun body",
        )?;
        Ok(Self {
            scheduled_recording_flag: flags & SCHEDULED_RECORDING_FLAG != 0,
            trick_mode_aware_flag: flags & TRICK_MODE_AWARE_FLAG != 0,
            time_shift_flag: flags & TIME_SHIFT_FLAG != 0,
            dynamic_flag: flags & DYNAMIC_FLAG != 0,
            av_synced_flag: flags & AV_SYNCED_FLAG != 0,
            initiating_replay_flag: flags & INITIATING_REPLAY_FLAG != 0,
            reserved: flags & RESERVED_MASK,
            labels,
            component_tags: tags.to_vec(),
            private,
            reserved_future_use: &body[next..],
        })
    }
}

impl Serialize for ApplicationRecordingDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN
            + 2
            + self.labels.iter().map(|l| 2 + l.label.len()).sum::<usize>()
            + 1
            + self.component_tags.len()
            + 1
            + self.private.len()
            + self.reserved_future_use.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        let reserved = broadcast_common::len::fit_mask(self.reserved, RESERVED_MASK, "reserved")?;
        let label_count = broadcast_common::len::fit_u8(self.labels.len(), "label_count")?;
        let tag_list_len =
            broadcast_common::len::fit_u8(self.component_tags.len(), "component_tag_list_length")?;
        let private_len = broadcast_common::len::fit_u8(self.private.len(), "private_length")?;
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        write_descriptor_header(buf, TAG, len - HEADER_LEN)?;
        buf[2] = reserved
            | (u8::from(self.scheduled_recording_flag) * SCHEDULED_RECORDING_FLAG)
            | (u8::from(self.trick_mode_aware_flag) * TRICK_MODE_AWARE_FLAG)
            | (u8::from(self.time_shift_flag) * TIME_SHIFT_FLAG)
            | (u8::from(self.dynamic_flag) * DYNAMIC_FLAG)
            | (u8::from(self.av_synced_flag) * AV_SYNCED_FLAG)
            | (u8::from(self.initiating_replay_flag) * INITIATING_REPLAY_FLAG);
        buf[3] = label_count;
        let mut pos = 4;
        for l in &self.labels {
            buf[pos] = broadcast_common::len::fit_u8(l.label.len(), "label_length")?;
            pos += 1;
            buf[pos..pos + l.label.len()].copy_from_slice(l.label);
            pos += l.label.len();
            let props = broadcast_common::len::fit_mask(
                l.storage_properties.to_u8(),
                RESERVED_MASK,
                "storage_properties",
            )?;
            let res = broadcast_common::len::fit_mask(l.reserved, LABEL_RESERVED_MASK, "reserved")?;
            buf[pos] = (props << STORAGE_PROPERTIES_SHIFT) | res;
            pos += 1;
        }
        buf[pos] = tag_list_len;
        pos += 1;
        buf[pos..pos + self.component_tags.len()].copy_from_slice(&self.component_tags);
        pos += self.component_tags.len();
        buf[pos] = private_len;
        pos += 1;
        buf[pos..pos + self.private.len()].copy_from_slice(self.private);
        pos += self.private.len();
        buf[pos..pos + self.reserved_future_use.len()].copy_from_slice(self.reserved_future_use);
        Ok(len)
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for ApplicationRecordingDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "APPLICATION_RECORDING";
}

#[cfg(test)]
mod tests {
    use super::*;

    // Table 21: flags scheduled+dynamic (0x90) + reserved 0b11 => 0x93; one
    // label "ab" with storage_properties=1 (critical) + reserved 0x3F => 0x7F;
    // component tags [0x10, 0x11]; private [0xAA]; no trailer.
    const BYTES: [u8; 14] = [
        TAG, 12, 0x93, 0x01, 0x02, b'a', b'b', 0x7F, 0x02, 0x10, 0x11, 0x01, 0xAA, 0xFF,
    ];

    #[test]
    fn parse_fields() {
        let d = ApplicationRecordingDescriptor::parse(&BYTES).unwrap();
        assert!(d.scheduled_recording_flag && d.dynamic_flag);
        assert!(!d.trick_mode_aware_flag && !d.time_shift_flag);
        assert!(!d.av_synced_flag && !d.initiating_replay_flag);
        assert_eq!(d.reserved, 3);
        assert_eq!(d.labels.len(), 1);
        assert_eq!(d.labels[0].label, b"ab");
        assert_eq!(
            d.labels[0].storage_properties,
            StorageProperties::CriticalToStore
        );
        assert_eq!(d.labels[0].reserved, 0x3F);
        assert_eq!(d.component_tags, vec![0x10, 0x11]);
        assert_eq!(d.private, &[0xAA]);
        assert_eq!(d.reserved_future_use, &[0xFF]);
    }

    #[test]
    fn round_trip_byte_identical_incl_zero_reserved_and_trailer() {
        for bytes in [
            BYTES.to_vec(),
            vec![TAG, 4, 0x00, 0x00, 0x00, 0x00],
            vec![TAG, 6, 0xFC, 0x01, 0x00, 0xC0, 0x00, 0x00],
        ] {
            let d = ApplicationRecordingDescriptor::parse(&bytes).unwrap();
            let mut out = vec![0u8; d.serialized_len()];
            d.serialize_into(&mut out).unwrap();
            assert_eq!(out, bytes);
            assert_eq!(ApplicationRecordingDescriptor::parse(&out).unwrap(), d);
        }
    }

    #[test]
    fn default_and_builder_use_spec_reserved() {
        let d = ApplicationRecordingDescriptor {
            labels: vec![RecordingLabel::new(
                b"x",
                StorageProperties::OptionalToStore,
            )],
            ..Default::default()
        };
        let mut out = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, [TAG, 7, 0x03, 0x01, 0x01, b'x', 0xBF, 0x00, 0x00]);
    }

    #[test]
    fn malformed_bodies_rejected() {
        assert!(ApplicationRecordingDescriptor::parse(&[TAG, 0]).is_err());
        assert!(ApplicationRecordingDescriptor::parse(&[TAG, 2, 0x00, 0x01]).is_err());
        // label overruns
        assert!(ApplicationRecordingDescriptor::parse(&[TAG, 4, 0, 1, 5, b'a']).is_err());
        // private_length overruns
        assert!(ApplicationRecordingDescriptor::parse(&[TAG, 4, 0, 0, 0, 0xFF]).is_err());
        // a single trailing reserved_future_use byte is accepted
        assert!(ApplicationRecordingDescriptor::parse(&[TAG, 5, 0, 0, 0, 0, 0xFF]).is_ok());
        // missing private_length
        assert!(ApplicationRecordingDescriptor::parse(&[TAG, 3, 0, 0, 0]).is_err());
    }

    #[test]
    fn over_range_values_rejected_on_serialize() {
        let labels: Vec<_> = (0..256)
            .map(|_| RecordingLabel::new(b"", StorageProperties::ShouldNotBeStored))
            .collect();
        let d = ApplicationRecordingDescriptor {
            labels,
            ..Default::default()
        };
        let mut out = vec![0u8; d.serialized_len()];
        assert!(d.serialize_into(&mut out).is_err());
        let d = ApplicationRecordingDescriptor {
            reserved: 4,
            ..Default::default()
        };
        let mut out = vec![0u8; d.serialized_len()];
        assert!(d.serialize_into(&mut out).is_err());
    }
}
