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
use crate::init_segment::bounded_entry_count;
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
#[non_exhaustive]
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

/// A `roll` sample-group description: `roll_distance` (`signed int(16)`),
/// ISO/IEC 14496-12:2015 §10.6.1.
const ROLL_ENTRY_LEN: usize = 2;

impl SgpdEntry {
    /// Serialized size of this entry on the wire (body bytes only, no length prefix).
    pub fn wire_len(&self) -> usize {
        match self {
            Self::Roll { .. } => ROLL_ENTRY_LEN,
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
    /// `default_sample_description_index` from the v2 syntax (§8.9.3.2),
    /// preserved so a v2 box round-trips; `None` for any other version.
    pub default_sample_description_index: Option<u32>,
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
        let mut default_sample_description_index = None;

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
            // Version 2 replaces `default_length` with
            // `default_sample_description_index` (§8.9.3.2). It has to be
            // stored, not skipped: the serializer used to write that field
            // only for `version == 1` while keeping `version = 2`, so a parsed
            // v2 `sgpd` re-serialized 4 bytes short of its own declared syntax
            // and every reader took `entry_count` from the wrong offset
            // (audit r05-W15).
            if body.len() < c + 4 {
                return Err(Error::BufferTooShort {
                    need: c + 4,
                    have: body.len(),
                    what: "sgpd default_sample_description_index",
                });
            }
            let idx = u32::from_be_bytes([body[c], body[c + 1], body[c + 2], body[c + 3]]);
            c += 4;
            default_sample_description_index = Some(idx);
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

        // Minimum bytes each entry can possibly consume, for bounding the
        // up-front allocation against a hostile `entry_count` (#988): a v1
        // per-entry `description_length` prefix (4 bytes) when
        // `default_length` is 0, else `default_length` itself; 2 bytes for a
        // v0 'roll' entry; 1 byte as a floor for the "consume the rest as one
        // blob" v0 fallback.
        let sgpd_min_entry_len: usize = if version == 1 {
            if default_length == 0 {
                4
            } else {
                default_length as usize
            }
        } else if grouping_type == GROUPING_TYPE_ROLL {
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
            return Err(Error::BufferTooShort {
                need: c.saturating_add(entry_min_footprint),
                have: body.len(),
                what: "sgpd entries",
            });
        }
        let mut entries = Vec::with_capacity(bounded_entry_count(
            body.len().saturating_sub(c),
            sgpd_min_entry_len,
            entry_count,
        ));
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
                        return Err(Error::BufferTooShort {
                            need: c + 1,
                            have: body.len(),
                            what: "sgpd entry body",
                        });
                    }
                    rest
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
            // A `roll` description is exactly the 2-byte `roll_distance`
            // (§10.6.1). A box whose v1 `default_length` or per-entry
            // `description_length` is larger therefore carries something this
            // crate does not model after the distance; keeping only the first
            // 2 bytes would re-serialize it 2 bytes wide and shrink the whole
            // entry list (audit r05-W15), so anything else stays opaque and
            // byte-exact.
            let entry = if grouping_type == GROUPING_TYPE_ROLL && entry_len == ROLL_ENTRY_LEN {
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
            default_sample_description_index,
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
        // header + fullbox + grouping_type + [default_length | index] +
        // entry_count + entries. Version 2 replaces `default_length` with
        // `default_sample_description_index` (§8.9.3.2) — same 4 bytes, and
        // omitting it re-framed the whole box (audit r05-W15).
        let dl_field = if self.version >= 1 { 4 } else { 0 };
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
        } else if self.version >= 2 {
            let index = self.default_sample_description_index.ok_or(Error::InvalidInput(
                "sgpd version 2 requires default_sample_description_index (ISO/IEC 14496-12 §8.9.3.2)",
            ))?;
            buf[c..c + 4].copy_from_slice(&index.to_be_bytes());
            c += 4;
        }
        let entry_count = broadcast_common::len::fit_u32(self.entries.len(), "entry_count")?;
        buf[c..c + 4].copy_from_slice(&entry_count.to_be_bytes());
        c += 4;
        for entry in &self.entries {
            if per_entry_prefix {
                let description_length =
                    broadcast_common::len::fit_u32(entry.wire_len(), "description_length")?;
                buf[c..c + 4].copy_from_slice(&description_length.to_be_bytes());
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
        // Only v1's syntax has the `default_length`/per-entry-description_length
        // pair; v2 carries a sample-description index instead and every entry
        // is self-describing (§8.9.3.2).
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
        // Each entry is a fixed 8 bytes (sample_count + group_description_index).
        let mut entries = Vec::with_capacity(bounded_entry_count(
            body.len().saturating_sub(c),
            8,
            entry_count,
        ));
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
        let entry_count = broadcast_common::len::fit_u32(self.entries.len(), "entry_count")?;
        buf[c..c + 4].copy_from_slice(&entry_count.to_be_bytes());
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
        // Each entry's fixed header (sample_delta + subsample_count) is 6
        // bytes; the variable-length subsample list is bounded separately
        // below, per entry.
        let mut entries = Vec::with_capacity(bounded_entry_count(
            body.len().saturating_sub(c),
            4 + 2,
            entry_count,
        ));
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
            let mut subsamples = Vec::with_capacity(bounded_entry_count(
                body.len().saturating_sub(c),
                ss_wire,
                subsample_count,
            ));
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
        let entry_count = broadcast_common::len::fit_u32(self.entries.len(), "entry_count")?;
        buf[c..c + 4].copy_from_slice(&entry_count.to_be_bytes());
        c += 4;
        for entry in &self.entries {
            buf[c..c + 4].copy_from_slice(&entry.sample_delta.to_be_bytes());
            c += 4;
            let subsample_count =
                broadcast_common::len::fit_u16(entry.subsamples.len(), "subsample_count")?;
            buf[c..c + 2].copy_from_slice(&subsample_count.to_be_bytes());
            c += 2;
            for ss in &entry.subsamples {
                if self.version == 1 {
                    buf[c..c + 4].copy_from_slice(&ss.subsample_size.to_be_bytes());
                    c += 4;
                } else {
                    let subsample_size = broadcast_common::len::fit_u16(
                        ss.subsample_size as usize,
                        "subsample_size",
                    )?;
                    buf[c..c + 2].copy_from_slice(&subsample_size.to_be_bytes());
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

    /// `sgpd` version 2 replaces `default_length` with
    /// `default_sample_description_index` (ISO/IEC 14496-12:2015 §8.9.3.2) —
    /// the same 4 bytes, in the same position. The parser skipped the field
    /// and the serializer wrote it only for `version == 1`, so a parsed v2 box
    /// was re-emitted 4 bytes short of its own syntax and every reader took
    /// `entry_count` from the wrong offset (audit r05-W15).
    #[test]
    fn sgpd_v2_round_trips_with_the_description_index() {
        let b = SampleGroupDescriptionBox {
            version: 2,
            flags: 0,
            grouping_type: GROUPING_TYPE_ROLL,
            default_length: 0,
            default_sample_description_index: Some(3),
            entries: alloc::vec![SgpdEntry::Roll { roll_distance: -1 }],
        };
        let bytes = b.try_to_bytes().unwrap();
        // 8 header + 4 FullBox + 4 grouping_type + 4 index + 4 entry_count
        // + 2 roll_distance (v2 entries are self-describing, §8.9.3.2).
        assert_eq!(bytes.len(), 26, "v2 must carry the index field");
        let parsed = SampleGroupDescriptionBox::parse(&bytes).unwrap();
        assert_eq!(parsed.version, 2);
        assert_eq!(parsed.default_sample_description_index, Some(3));
        assert_eq!(parsed.entries, b.entries);
        assert_eq!(parsed.try_to_bytes().unwrap(), bytes, "v2 round-trip");
    }

    /// A v2 box whose index field is absent from the struct cannot be written:
    /// guessing a value would mislabel the box's own syntax.
    #[test]
    fn sgpd_v2_without_the_description_index_errors() {
        let b = SampleGroupDescriptionBox {
            version: 2,
            flags: 0,
            grouping_type: GROUPING_TYPE_ROLL,
            default_length: 0,
            default_sample_description_index: None,
            entries: alloc::vec![SgpdEntry::Roll { roll_distance: -1 }],
        };
        let err = b.try_to_bytes().unwrap_err();
        assert!(
            matches!(err, Error::InvalidInput(_)),
            "expected InvalidInput, got {err:?}"
        );
    }

    /// A `roll` description is exactly 2 bytes (§10.6.1). A box whose v1
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
            default_sample_description_index: None,
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
            default_sample_description_index: None,
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
                Error::FieldOverflow(broadcast_common::len::FieldOverflow {
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
                Error::FieldOverflow(broadcast_common::len::FieldOverflow {
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
}
