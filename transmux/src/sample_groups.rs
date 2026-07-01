//! ProducerReferenceTimeBox, SampleGroupDescriptionBox, SampleToGroupBox, and
//! SubSampleInformationBox — ISO/IEC 14496-12:2015 §8.16.5, §8.9.2, §8.9.3,
//! §8.7.7.
//!
//! Typed containers for:
//!
//! | Box   | Name                           | Section   | Description                                       |
//! |-------|--------------------------------|-----------|---------------------------------------------------|
//! | `prft`| ProducerReferenceTimeBox       | §8.16.5   | NTP wall-clock anchor for the reference track     |
//! | `sgpd`| SampleGroupDescriptionBox      | §8.9.3    | Per-grouping-type sample group description table  |
//! | `sbgp`| SampleToGroupBox               | §8.9.2    | Maps samples to sample group description indices  |
//! | `subs`| SubSampleInformationBox        | §8.7.7    | Per-sample sub-sample size/priority table         |
//!
//! All boxes are FullBoxes. Sizes are computed from fields; no `self.raw`
//! passthrough in any serializer.
//!
//! # Spec citations
//!
//! - **prft**: ISO/IEC 14496-12:2015 §8.16.5.2 — version 0: `media_time` is u32;
//!   version 1: `media_time` is u64.
//! - **sgpd**: ISO/IEC 14496-12:2015 §8.9.3.2 — version 1: `default_length` field
//!   present; version 0 is deprecated. Only `'roll'` (RollRecoveryEntry, i16
//!   `roll_distance`) is typed; all other grouping types are stored as raw bytes
//!   via [`SgpdEntry::Unknown`].
//! - **sbgp**: ISO/IEC 14496-12:2015 §8.9.2.2 — version 0: no
//!   `grouping_type_parameter`; version 1: `grouping_type_parameter` present.
//! - **subs**: ISO/IEC 14496-12:2015 §8.7.7.2 — version 0: `subsample_size` is
//!   u16; version 1: `subsample_size` is u32.

use crate::error::{Error, Result};
use alloc::vec::Vec;

use broadcast_common::{Parse, Serialize};

// ---------------------------------------------------------------------------
// Wire-layout constants
// ---------------------------------------------------------------------------

const BOX_HEADER_SIZE: usize = 8;
const FULLBOX_EXTRA_SIZE: usize = 4;

const PRFT_TYPE: u32 = u32::from_be_bytes(*b"prft");
const SGPD_TYPE: u32 = u32::from_be_bytes(*b"sgpd");
const SBGP_TYPE: u32 = u32::from_be_bytes(*b"sbgp");
const SUBS_TYPE: u32 = u32::from_be_bytes(*b"subs");

/// Grouping type `'roll'` (RollRecoveryEntry) — ISO/IEC 14496-12:2015 §10.6.
pub const GROUPING_TYPE_ROLL: u32 = u32::from_be_bytes(*b"roll");

// ---------------------------------------------------------------------------
// ProducerReferenceTimeBox — prft (ISO/IEC 14496-12:2015 §8.16.5)
// ---------------------------------------------------------------------------

/// Producer Reference Time Box (`prft`) — ISO/IEC 14496-12:2015 §8.16.5.2.
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
    /// Track ID of the reference track (§8.16.5.3).
    pub reference_track_id: u32,
    /// UTC time in NTP format (§8.16.5.3).
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
        let min = FULLBOX_EXTRA_SIZE + 4 + 8;
        if body.len() < min {
            return Err(Error::BufferTooShort {
                need: min,
                have: body.len(),
                what: "prft body",
            });
        }
        let version = body[0];
        let flags = u32::from_be_bytes([0, body[1], body[2], body[3]]);
        let mut c = FULLBOX_EXTRA_SIZE;
        let reference_track_id =
            u32::from_be_bytes([body[c], body[c + 1], body[c + 2], body[c + 3]]);
        c += 4;
        let ntp_timestamp = u64::from_be_bytes([
            body[c],
            body[c + 1],
            body[c + 2],
            body[c + 3],
            body[c + 4],
            body[c + 5],
            body[c + 6],
            body[c + 7],
        ]);
        c += 8;
        let media_time = if version == 0 {
            if body.len() < c + 4 {
                return Err(Error::BufferTooShort {
                    need: c + 4,
                    have: body.len(),
                    what: "prft media_time v0",
                });
            }
            u32::from_be_bytes([body[c], body[c + 1], body[c + 2], body[c + 3]]) as u64
        } else {
            if body.len() < c + 8 {
                return Err(Error::BufferTooShort {
                    need: c + 8,
                    have: body.len(),
                    what: "prft media_time v1",
                });
            }
            u64::from_be_bytes([
                body[c],
                body[c + 1],
                body[c + 2],
                body[c + 3],
                body[c + 4],
                body[c + 5],
                body[c + 6],
                body[c + 7],
            ])
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
        if bytes.len() < BOX_HEADER_SIZE + FULLBOX_EXTRA_SIZE + 4 + 8 + 4 {
            return Err(Error::BufferTooShort {
                need: BOX_HEADER_SIZE + FULLBOX_EXTRA_SIZE + 4 + 8 + 4,
                have: bytes.len(),
                what: "prft box",
            });
        }
        let ty = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        if ty != PRFT_TYPE {
            return Err(Error::InvalidValue {
                field: "box_type",
                value: ty as u64,
                reason: "expected prft",
            });
        }
        Self::parse_body(&bytes[BOX_HEADER_SIZE..])
    }
}

impl Serialize for ProducerReferenceTimeBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mt_size = if self.version == 0 { 4 } else { 8 };
        BOX_HEADER_SIZE + FULLBOX_EXTRA_SIZE + 4 + 8 + mt_size
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"prft");
        c += 4;
        buf[c] = self.version;
        let fb = self.flags.to_be_bytes();
        buf[c + 1] = fb[1];
        buf[c + 2] = fb[2];
        buf[c + 3] = fb[3];
        c += 4;
        buf[c..c + 4].copy_from_slice(&self.reference_track_id.to_be_bytes());
        c += 4;
        buf[c..c + 8].copy_from_slice(&self.ntp_timestamp.to_be_bytes());
        c += 8;
        if self.version == 0 {
            buf[c..c + 4].copy_from_slice(&(self.media_time as u32).to_be_bytes());
            c += 4;
        } else {
            buf[c..c + 8].copy_from_slice(&self.media_time.to_be_bytes());
            c += 8;
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// SampleGroupDescriptionBox — sgpd (ISO/IEC 14496-12:2015 §8.9.3)
// ---------------------------------------------------------------------------

/// A parsed entry in the sgpd sample group description table.
///
/// Only `'roll'` (§10.6 RollRecoveryEntry) is fully typed; all other grouping
/// types carry their body as raw bytes in [`SgpdEntry::Unknown`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum SgpdEntry {
    /// RollRecoveryEntry for grouping type `'roll'` (§10.6).
    ///
    /// `roll_distance` is the number of samples that must be decoded before the
    /// stream is usable (negative = pre-roll, positive = post-roll).
    Roll {
        /// Roll distance in samples. Negative = pre-roll.
        roll_distance: i16,
    },
    /// Raw bytes for any grouping type not specifically handled.
    Unknown(Vec<u8>),
}

impl SgpdEntry {
    /// Serialized size of this entry on the wire (body bytes only, no length prefix).
    pub fn wire_len(&self) -> usize {
        match self {
            Self::Roll { .. } => 2,
            Self::Unknown(v) => v.len(),
        }
    }
}

/// Sample Group Description Box (`sgpd`) — ISO/IEC 14496-12:2015 §8.9.3.2.
///
/// Version 1 is the current (non-deprecated) form and is the only version
/// emitted by this serializer. Version 0 can be parsed.
///
/// Wire layout (FullBox header omitted):
///
/// ```text
/// grouping_type          u(32)
/// default_length         u(32)  — version == 1 only
/// entry_count            u(32)
/// for each entry:
///   [description_length  u(32)] — only if version == 1 && default_length == 0
///   SampleGroupEntry (grouping_type)
/// ```
///
/// When `version == 1` and `default_length != 0`, every entry has the same
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
    /// `default_length` from the v1 syntax; 0 means variable-length entries.
    ///
    /// On serialization this is recomputed from entries: if all entries have the
    /// same `wire_len`, `default_length` is set to that value; otherwise 0
    /// (each entry gets an explicit `description_length` prefix).
    pub default_length: u32,
    /// Parsed entries.
    pub entries: Vec<SgpdEntry>,
}

impl SampleGroupDescriptionBox {
    /// Parse the body of an `sgpd` box (after the 8-byte BoxHeader).
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        if body.len() < FULLBOX_EXTRA_SIZE + 4 + 4 {
            return Err(Error::BufferTooShort {
                need: FULLBOX_EXTRA_SIZE + 4 + 4,
                have: body.len(),
                what: "sgpd body",
            });
        }
        let version = body[0];
        let flags = u32::from_be_bytes([0, body[1], body[2], body[3]]);
        let mut c = FULLBOX_EXTRA_SIZE;
        let grouping_type = u32::from_be_bytes([body[c], body[c + 1], body[c + 2], body[c + 3]]);
        c += 4;

        let default_length = if version == 1 {
            if body.len() < c + 4 {
                return Err(Error::BufferTooShort {
                    need: c + 4,
                    have: body.len(),
                    what: "sgpd default_length",
                });
            }
            let dl = u32::from_be_bytes([body[c], body[c + 1], body[c + 2], body[c + 3]]);
            c += 4;
            dl
        } else if version >= 2 {
            // version 2 has default_sample_description_index instead
            if body.len() < c + 4 {
                return Err(Error::BufferTooShort {
                    need: c + 4,
                    have: body.len(),
                    what: "sgpd default_sample_description_index",
                });
            }
            c += 4; // skip default_sample_description_index
            0
        } else {
            0 // version 0: no default_length field
        };

        if body.len() < c + 4 {
            return Err(Error::BufferTooShort {
                need: c + 4,
                have: body.len(),
                what: "sgpd entry_count",
            });
        }
        let entry_count =
            u32::from_be_bytes([body[c], body[c + 1], body[c + 2], body[c + 3]]) as usize;
        c += 4;

        let mut entries = Vec::with_capacity(entry_count);
        for _ in 0..entry_count {
            // Determine entry length
            let entry_len: usize = if version == 1 && default_length == 0 {
                if body.len() < c + 4 {
                    return Err(Error::BufferTooShort {
                        need: c + 4,
                        have: body.len(),
                        what: "sgpd description_length",
                    });
                }
                let dl =
                    u32::from_be_bytes([body[c], body[c + 1], body[c + 2], body[c + 3]]) as usize;
                c += 4;
                dl
            } else if version == 1 {
                default_length as usize
            } else {
                // version 0: entry size is implied by the grouping_type; we
                // parse until we hit the known size for 'roll' (2 bytes), else
                // we consume remaining bytes as one blob (rare / deprecated).
                if grouping_type == GROUPING_TYPE_ROLL {
                    2
                } else {
                    // unknown v0: consume all remaining as one entry
                    body.len() - c
                }
            };
            if body.len() < c + entry_len {
                return Err(Error::BufferTooShort {
                    need: c + entry_len,
                    have: body.len(),
                    what: "sgpd entry body",
                });
            }
            let entry_bytes = &body[c..c + entry_len];
            let entry = if grouping_type == GROUPING_TYPE_ROLL && entry_len >= 2 {
                let rd = i16::from_be_bytes([entry_bytes[0], entry_bytes[1]]);
                SgpdEntry::Roll { roll_distance: rd }
            } else {
                SgpdEntry::Unknown(entry_bytes.to_vec())
            };
            entries.push(entry);
            c += entry_len;
        }

        Ok(Self {
            version,
            flags,
            grouping_type,
            default_length,
            entries,
        })
    }
}

impl<'a> Parse<'a> for SampleGroupDescriptionBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < BOX_HEADER_SIZE + FULLBOX_EXTRA_SIZE + 4 {
            return Err(Error::BufferTooShort {
                need: BOX_HEADER_SIZE + FULLBOX_EXTRA_SIZE + 4,
                have: bytes.len(),
                what: "sgpd box",
            });
        }
        let ty = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        if ty != SGPD_TYPE {
            return Err(Error::InvalidValue {
                field: "box_type",
                value: ty as u64,
                reason: "expected sgpd",
            });
        }
        Self::parse_body(&bytes[BOX_HEADER_SIZE..])
    }
}

impl Serialize for SampleGroupDescriptionBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        // Compute whether we will use a uniform default_length.
        let (use_default_len, per_entry_prefix) = self.effective_default_length();
        let entry_overhead = if per_entry_prefix { 4 } else { 0 };
        let entries_size: usize = self
            .entries
            .iter()
            .map(|e| entry_overhead + e.wire_len())
            .sum();
        // header + fullbox + grouping_type + [default_length] + entry_count + entries
        let dl_field = if self.version == 1 { 4 } else { 0 };
        let _ = use_default_len;
        BOX_HEADER_SIZE + FULLBOX_EXTRA_SIZE + 4 + dl_field + 4 + entries_size
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let (effective_dl, per_entry_prefix) = self.effective_default_length();
        let mut c = 0;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"sgpd");
        c += 4;
        buf[c] = self.version;
        let fb = self.flags.to_be_bytes();
        buf[c + 1] = fb[1];
        buf[c + 2] = fb[2];
        buf[c + 3] = fb[3];
        c += 4;
        buf[c..c + 4].copy_from_slice(&self.grouping_type.to_be_bytes());
        c += 4;
        if self.version == 1 {
            buf[c..c + 4].copy_from_slice(&effective_dl.to_be_bytes());
            c += 4;
        }
        buf[c..c + 4].copy_from_slice(&(self.entries.len() as u32).to_be_bytes());
        c += 4;
        for entry in &self.entries {
            if per_entry_prefix {
                buf[c..c + 4].copy_from_slice(&(entry.wire_len() as u32).to_be_bytes());
                c += 4;
            }
            match entry {
                SgpdEntry::Roll { roll_distance } => {
                    buf[c..c + 2].copy_from_slice(&roll_distance.to_be_bytes());
                    c += 2;
                }
                SgpdEntry::Unknown(v) => {
                    buf[c..c + v.len()].copy_from_slice(v);
                    c += v.len();
                }
            }
        }
        Ok(c)
    }
}

impl SampleGroupDescriptionBox {
    /// Compute the `default_length` to write and whether per-entry length
    /// prefixes are needed.
    ///
    /// Returns `(effective_default_length, per_entry_prefix_needed)`.
    fn effective_default_length(&self) -> (u32, bool) {
        if self.version != 1 || self.entries.is_empty() {
            return (0, false);
        }
        let first = self.entries[0].wire_len();
        let uniform = self.entries.iter().all(|e| e.wire_len() == first);
        if uniform {
            (first as u32, false)
        } else {
            (0, true)
        }
    }
}

// ---------------------------------------------------------------------------
// SampleToGroupBox — sbgp (ISO/IEC 14496-12:2015 §8.9.2)
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

/// Sample To Group Box (`sbgp`) — ISO/IEC 14496-12:2015 §8.9.2.2.
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
    /// Parse the body of an `sbgp` box (after the 8-byte BoxHeader).
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        if body.len() < FULLBOX_EXTRA_SIZE + 4 + 4 {
            return Err(Error::BufferTooShort {
                need: FULLBOX_EXTRA_SIZE + 4 + 4,
                have: body.len(),
                what: "sbgp body",
            });
        }
        let version = body[0];
        let flags = u32::from_be_bytes([0, body[1], body[2], body[3]]);
        let mut c = FULLBOX_EXTRA_SIZE;
        let grouping_type = u32::from_be_bytes([body[c], body[c + 1], body[c + 2], body[c + 3]]);
        c += 4;
        let grouping_type_parameter = if version == 1 {
            if body.len() < c + 4 {
                return Err(Error::BufferTooShort {
                    need: c + 4,
                    have: body.len(),
                    what: "sbgp grouping_type_parameter",
                });
            }
            let v = u32::from_be_bytes([body[c], body[c + 1], body[c + 2], body[c + 3]]);
            c += 4;
            Some(v)
        } else {
            None
        };
        if body.len() < c + 4 {
            return Err(Error::BufferTooShort {
                need: c + 4,
                have: body.len(),
                what: "sbgp entry_count",
            });
        }
        let entry_count =
            u32::from_be_bytes([body[c], body[c + 1], body[c + 2], body[c + 3]]) as usize;
        c += 4;
        let mut entries = Vec::with_capacity(entry_count);
        for _ in 0..entry_count {
            if body.len() < c + 8 {
                return Err(Error::BufferTooShort {
                    need: c + 8,
                    have: body.len(),
                    what: "sbgp entry",
                });
            }
            let sample_count = u32::from_be_bytes([body[c], body[c + 1], body[c + 2], body[c + 3]]);
            let group_description_index =
                u32::from_be_bytes([body[c + 4], body[c + 5], body[c + 6], body[c + 7]]);
            entries.push(SbgpEntry {
                sample_count,
                group_description_index,
            });
            c += 8;
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
        if bytes.len() < BOX_HEADER_SIZE + FULLBOX_EXTRA_SIZE + 4 {
            return Err(Error::BufferTooShort {
                need: BOX_HEADER_SIZE + FULLBOX_EXTRA_SIZE + 4,
                have: bytes.len(),
                what: "sbgp box",
            });
        }
        let ty = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        if ty != SBGP_TYPE {
            return Err(Error::InvalidValue {
                field: "box_type",
                value: ty as u64,
                reason: "expected sbgp",
            });
        }
        Self::parse_body(&bytes[BOX_HEADER_SIZE..])
    }
}

impl Serialize for SampleToGroupBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let gtp_size = if self.version == 1 { 4 } else { 0 };
        BOX_HEADER_SIZE + FULLBOX_EXTRA_SIZE + 4 + gtp_size + 4 + self.entries.len() * 8
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"sbgp");
        c += 4;
        buf[c] = self.version;
        let fb = self.flags.to_be_bytes();
        buf[c + 1] = fb[1];
        buf[c + 2] = fb[2];
        buf[c + 3] = fb[3];
        c += 4;
        buf[c..c + 4].copy_from_slice(&self.grouping_type.to_be_bytes());
        c += 4;
        if self.version == 1 {
            let gtp = self.grouping_type_parameter.unwrap_or(0);
            buf[c..c + 4].copy_from_slice(&gtp.to_be_bytes());
            c += 4;
        }
        buf[c..c + 4].copy_from_slice(&(self.entries.len() as u32).to_be_bytes());
        c += 4;
        for entry in &self.entries {
            buf[c..c + 4].copy_from_slice(&entry.sample_count.to_be_bytes());
            buf[c + 4..c + 8].copy_from_slice(&entry.group_description_index.to_be_bytes());
            c += 8;
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// SubSampleInformationBox — subs (ISO/IEC 14496-12:2015 §8.7.7)
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
        let size_field = if version == 1 { 4 } else { 2 };
        size_field + 1 + 1 + 4
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
        4 + 2 + self.subsamples.len() * SubSampleDescriptor::wire_len(version)
    }
}

/// Sub-Sample Information Box (`subs`) — ISO/IEC 14496-12:2015 §8.7.7.2.
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
        if body.len() < FULLBOX_EXTRA_SIZE + 4 {
            return Err(Error::BufferTooShort {
                need: FULLBOX_EXTRA_SIZE + 4,
                have: body.len(),
                what: "subs body",
            });
        }
        let version = body[0];
        let flags = u32::from_be_bytes([0, body[1], body[2], body[3]]);
        let mut c = FULLBOX_EXTRA_SIZE;
        let entry_count =
            u32::from_be_bytes([body[c], body[c + 1], body[c + 2], body[c + 3]]) as usize;
        c += 4;
        let ss_size_field = if version == 1 { 4usize } else { 2usize };
        let mut entries = Vec::with_capacity(entry_count);
        for _ in 0..entry_count {
            if body.len() < c + 4 + 2 {
                return Err(Error::BufferTooShort {
                    need: c + 6,
                    have: body.len(),
                    what: "subs entry header",
                });
            }
            let sample_delta = u32::from_be_bytes([body[c], body[c + 1], body[c + 2], body[c + 3]]);
            c += 4;
            let subsample_count = u16::from_be_bytes([body[c], body[c + 1]]) as usize;
            c += 2;
            let ss_wire = ss_size_field + 1 + 1 + 4;
            let mut subsamples = Vec::with_capacity(subsample_count);
            for _ in 0..subsample_count {
                if body.len() < c + ss_wire {
                    return Err(Error::BufferTooShort {
                        need: c + ss_wire,
                        have: body.len(),
                        what: "subs subsample",
                    });
                }
                let subsample_size = if version == 1 {
                    let v = u32::from_be_bytes([body[c], body[c + 1], body[c + 2], body[c + 3]]);
                    c += 4;
                    v
                } else {
                    let v = u16::from_be_bytes([body[c], body[c + 1]]) as u32;
                    c += 2;
                    v
                };
                let subsample_priority = body[c];
                let discardable = body[c + 1];
                c += 2;
                let codec_specific_parameters =
                    u32::from_be_bytes([body[c], body[c + 1], body[c + 2], body[c + 3]]);
                c += 4;
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
        if bytes.len() < BOX_HEADER_SIZE + FULLBOX_EXTRA_SIZE + 4 {
            return Err(Error::BufferTooShort {
                need: BOX_HEADER_SIZE + FULLBOX_EXTRA_SIZE + 4,
                have: bytes.len(),
                what: "subs box",
            });
        }
        let ty = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        if ty != SUBS_TYPE {
            return Err(Error::InvalidValue {
                field: "box_type",
                value: ty as u64,
                reason: "expected subs",
            });
        }
        Self::parse_body(&bytes[BOX_HEADER_SIZE..])
    }
}

impl Serialize for SubSampleInformationBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let entries_size: usize = self.entries.iter().map(|e| e.wire_len(self.version)).sum();
        BOX_HEADER_SIZE + FULLBOX_EXTRA_SIZE + 4 + entries_size
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut c = 0;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"subs");
        c += 4;
        buf[c] = self.version;
        let fb = self.flags.to_be_bytes();
        buf[c + 1] = fb[1];
        buf[c + 2] = fb[2];
        buf[c + 3] = fb[3];
        c += 4;
        buf[c..c + 4].copy_from_slice(&(self.entries.len() as u32).to_be_bytes());
        c += 4;
        for entry in &self.entries {
            buf[c..c + 4].copy_from_slice(&entry.sample_delta.to_be_bytes());
            c += 4;
            buf[c..c + 2].copy_from_slice(&(entry.subsamples.len() as u16).to_be_bytes());
            c += 2;
            for ss in &entry.subsamples {
                if self.version == 1 {
                    buf[c..c + 4].copy_from_slice(&ss.subsample_size.to_be_bytes());
                    c += 4;
                } else {
                    buf[c..c + 2].copy_from_slice(&(ss.subsample_size as u16).to_be_bytes());
                    c += 2;
                }
                buf[c] = ss.subsample_priority;
                buf[c + 1] = ss.discardable;
                c += 2;
                buf[c..c + 4].copy_from_slice(&ss.codec_specific_parameters.to_be_bytes());
                c += 4;
            }
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_common::Serialize;

    // -----------------------------------------------------------------------
    // prft
    // -----------------------------------------------------------------------

    #[test]
    fn prft_round_trip_v0() {
        let b = ProducerReferenceTimeBox {
            version: 0,
            flags: 0,
            reference_track_id: 1,
            ntp_timestamp: 0x1234_5678_9abc_def0,
            media_time: 0x0000_0000_0000_1234,
        };
        let bytes = b.to_bytes();
        assert_eq!(bytes.len(), 8 + 4 + 4 + 8 + 4);
        let parsed = ProducerReferenceTimeBox::parse(&bytes).unwrap();
        assert_eq!(parsed, b);
    }

    #[test]
    fn prft_round_trip_v1() {
        let b = ProducerReferenceTimeBox {
            version: 1,
            flags: 0x000018,
            reference_track_id: 1,
            ntp_timestamp: 0xedefe3e3_a7ae147a,
            media_time: 0x0000_0000_0000_1c20,
        };
        let bytes = b.to_bytes();
        assert_eq!(bytes.len(), 8 + 4 + 4 + 8 + 8);
        let parsed = ProducerReferenceTimeBox::parse(&bytes).unwrap();
        assert_eq!(parsed, b);
    }

    // -----------------------------------------------------------------------
    // sgpd
    // -----------------------------------------------------------------------

    #[test]
    fn sgpd_round_trip_roll_v1() {
        let b = SampleGroupDescriptionBox {
            version: 1,
            flags: 0,
            grouping_type: GROUPING_TYPE_ROLL,
            default_length: 2,
            entries: vec![SgpdEntry::Roll { roll_distance: -1 }],
        };
        let bytes = b.to_bytes();
        let parsed = SampleGroupDescriptionBox::parse(&bytes).unwrap();
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0], SgpdEntry::Roll { roll_distance: -1 });
        assert_eq!(parsed.to_bytes(), bytes);
    }

    #[test]
    fn sgpd_round_trip_two_roll_entries() {
        let b = SampleGroupDescriptionBox {
            version: 1,
            flags: 0,
            grouping_type: GROUPING_TYPE_ROLL,
            default_length: 2,
            entries: vec![
                SgpdEntry::Roll { roll_distance: -4 },
                SgpdEntry::Roll { roll_distance: -1 },
            ],
        };
        let bytes = b.to_bytes();
        let parsed = SampleGroupDescriptionBox::parse(&bytes).unwrap();
        assert_eq!(parsed.entries.len(), 2);
        assert_eq!(parsed.to_bytes(), bytes);
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
        let bytes = b.to_bytes();
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
        let bytes = b.to_bytes();
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
        let bytes = b.to_bytes();
        let parsed = SubSampleInformationBox::parse(&bytes).unwrap();
        assert_eq!(parsed, b);
        assert_eq!(parsed.to_bytes(), bytes);
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
        let bytes = b.to_bytes();
        let parsed = SubSampleInformationBox::parse(&bytes).unwrap();
        assert_eq!(parsed, b);
        assert_eq!(parsed.to_bytes(), bytes);
    }

    #[test]
    fn subs_empty_round_trip() {
        let b = SubSampleInformationBox {
            version: 0,
            flags: 0,
            entries: vec![],
        };
        let bytes = b.to_bytes();
        let parsed = SubSampleInformationBox::parse(&bytes).unwrap();
        assert_eq!(parsed, b);
    }
}
