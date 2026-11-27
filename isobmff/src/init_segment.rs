//! Typed init-segment (moov) box tree — ISO/IEC 14496-12:2026 §8.2–8.7.
//!
//! Complete typed representation of the `moov` hierarchy found in ISOBMFF
//! initialisation segments. Container sizes are **computed** from children
//! (no `self.raw` passthrough). Unknown/opaque child boxes are preserved as
//! [`OpaqueBox`] for byte-exact round-trip.
//!
//! Reuses `TimeToSampleBox`, `CompositionOffsetBox`, `EditListBox` from
//! the `timing` module and `AVCSampleEntry` etc. from
//! `sample_entries`.

use crate::TrackId;
use crate::box_types::{
    BOX_HEADER_MIN_SIZE, BOX_TYPE_OFFSET, BOX_TYPE_SIZE, BoxIter, FULLBOX_EXTRA_SIZE,
    FULLBOX_FLAGS_MASK, FULLBOX_VERSION_OFFSET, parse_box, write_full_box_header,
};
use crate::error::{Error, Result};
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use broadcast_core::error::BufferTooShort;
use broadcast_core::len::{need_in, need_out};
use broadcast_core::timescale::Timescale;
use broadcast_core::wire::{fourcc_at, i16_at, u16_at, u24_at, u32_at, u64_at};
use broadcast_core::{Parse, Serialize};

use crate::box_support::{
    ChildOrder, FLAT_TABLE_COUNT_OFFSET, FLAT_TABLE_FLAGS_OFFSET, FLAT_TABLE_HEADER_LEN,
    FLAT_TABLE_VERSION_OFFSET, FULLBOX_FLAGS_WIDTH, FULLBOX_FLAGS_WIRE_OFFSET, FULLBOX_VERSION_LEN,
    OpaqueBox, SerializeBox, bounded_entry_count, check_entry_count, flat_table_len,
    parse_flat_table, serialize_container, serialize_flat_table,
};
use crate::limits::Limits;
use crate::quicktime::QuickTimeSound;

/// Wire width of one transformation-matrix value.
const MATRIX_VALUE_LEN: usize = core::mem::size_of::<i32>();
/// Number of values in a transformation matrix.
const MATRIX_VALUES: usize = 9;
/// Wire length of a transformation matrix.
const MATRIX_WIRE_LEN: usize = MATRIX_VALUES * MATRIX_VALUE_LEN;

// ---------------------------------------------------------------------------
// MovieHeaderBox — mvhd (ISO/IEC 14496-12:2026 §8.2.2)
// ---------------------------------------------------------------------------

/// Byte offset just past the FullBox `version`/`flags` fields.
const FULLBOX_BODY_OFFSET: usize = BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE;
/// Shortest `mvhd` prefix before its version-dependent fields.
const MVHD_MIN_LEN: usize = FULLBOX_BODY_OFFSET;
/// `mvhd` v0 creation_time offset (§8.2.2.2).
const MVHD_V0_CREATION_TIME_OFFSET: usize = 12;
/// `mvhd` v0 modification_time offset (§8.2.2.2).
const MVHD_V0_MODIFICATION_TIME_OFFSET: usize = 16;
/// `mvhd` v0 timescale offset (§8.2.2.2).
const MVHD_V0_TIMESCALE_OFFSET: usize = 20;
/// `mvhd` v0 duration offset (§8.2.2.2).
const MVHD_V0_DURATION_OFFSET: usize = 24;
/// `mvhd` v0 rate offset (§8.2.2.2).
const MVHD_V0_RATE_OFFSET: usize = 28;
/// `mvhd` v0 volume offset (§8.2.2.2).
const MVHD_V0_VOLUME_OFFSET: usize = 32;
/// `mvhd` v0 matrix offset (§8.2.2.2).
const MVHD_V0_MATRIX_OFFSET: usize = 44;
/// `mvhd` v0 next_track_ID offset (§8.2.2.2).
const MVHD_V0_NEXT_TRACK_ID_OFFSET: usize = 104;
/// `mvhd` v0 box length (§8.2.2.2).
const MVHD_V0_LEN: usize = 108;
/// `mvhd` v1 creation_time offset (§8.2.2.2).
const MVHD_V1_CREATION_TIME_OFFSET: usize = 12;
/// `mvhd` v1 modification_time offset (§8.2.2.2).
const MVHD_V1_MODIFICATION_TIME_OFFSET: usize = 20;
/// `mvhd` v1 timescale offset (§8.2.2.2).
const MVHD_V1_TIMESCALE_OFFSET: usize = 28;
/// `mvhd` v1 duration offset (§8.2.2.2).
const MVHD_V1_DURATION_OFFSET: usize = 32;
/// `mvhd` v1 rate offset (§8.2.2.2).
const MVHD_V1_RATE_OFFSET: usize = 40;
/// `mvhd` v1 volume offset (§8.2.2.2).
const MVHD_V1_VOLUME_OFFSET: usize = 44;
/// `mvhd` v1 matrix offset (§8.2.2.2).
const MVHD_V1_MATRIX_OFFSET: usize = 56;
/// `mvhd` v1 next_track_ID offset (§8.2.2.2).
const MVHD_V1_NEXT_TRACK_ID_OFFSET: usize = 116;
/// `mvhd` v1 box length (§8.2.2.2).
const MVHD_V1_LEN: usize = 120;
/// Reserved bytes between `volume` and `matrix` (§8.2.2.2).
const MVHD_RESERVED_LEN: usize = 10;
/// `pre_defined` bytes at the end of `mvhd` (§8.2.2.2).
const MVHD_PRE_DEFINED_LEN: usize = 24;

/// Four-CC of the Movie Header Box (`mvhd`, ISO/IEC 14496-12:2026 §8.2.2).
pub const MVHD: [u8; 4] = *b"mvhd";

/// Movie Header Box (`mvhd`) — §8.2.2.
/// v0: 32-bit creation_time, modification_time, duration.
/// v1: 64-bit equivalents.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MovieHeaderBox {
    pub version: u8,
    pub flags: u32,
    pub creation_time: u64,
    pub modification_time: u64,
    #[cfg_attr(feature = "serde", serde(serialize_with = "crate::ser_timescale"))]
    pub timescale: Timescale,
    pub duration: u64,
    pub rate: u32,
    pub volume: u16,
    pub matrix: [i32; MATRIX_VALUES],
    pub next_track_id: TrackId,
}

impl<'a> Parse<'a> for MovieHeaderBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(bytes, MVHD_MIN_LEN, "mvhd")?;
        let ver = bytes[FULLBOX_VERSION_OFFSET];
        let flags = u24_at(bytes, FULLBOX_FLAGS_WIRE_OFFSET);
        if ver == 0 {
            need_in(bytes, MVHD_V0_LEN, "mvhd v0")?;
            Ok(Self {
                version: 0,
                flags,
                creation_time: u64::from(u32_at(bytes, MVHD_V0_CREATION_TIME_OFFSET)),
                modification_time: u64::from(u32_at(bytes, MVHD_V0_MODIFICATION_TIME_OFFSET)),
                timescale: Timescale::try_new(u32_at(bytes, MVHD_V0_TIMESCALE_OFFSET))?,
                duration: u64::from(u32_at(bytes, MVHD_V0_DURATION_OFFSET)),
                rate: u32_at(bytes, MVHD_V0_RATE_OFFSET),
                volume: u16_at(bytes, MVHD_V0_VOLUME_OFFSET),
                matrix: matrix_from_bytes(
                    &bytes[MVHD_V0_MATRIX_OFFSET..MVHD_V0_MATRIX_OFFSET + MATRIX_WIRE_LEN],
                ),
                next_track_id: TrackId::new(u32_at(bytes, MVHD_V0_NEXT_TRACK_ID_OFFSET)),
            })
        } else {
            // v1 body (ISO/IEC 14496-12:2026 §8.2.2.2): the 8-byte creation/
            // modification_time + duration widening (+12 bytes over v0) is
            // the ONLY size change — `pre_defined[6]` still ends at byte 116,
            // so `next_track_id` sits at `116..120` and the whole box is
            // **120** bytes, not 124 (issue #1015).
            need_in(bytes, MVHD_V1_LEN, "mvhd v1")?;
            Ok(Self {
                version: 1,
                flags,
                creation_time: broadcast_core::wire::be_u64(
                    bytes,
                    MVHD_V1_CREATION_TIME_OFFSET,
                    "mvhd v1",
                )?,
                modification_time: broadcast_core::wire::be_u64(
                    bytes,
                    MVHD_V1_MODIFICATION_TIME_OFFSET,
                    "mvhd v1",
                )?,
                timescale: Timescale::try_new(u32_at(bytes, MVHD_V1_TIMESCALE_OFFSET))?,
                duration: broadcast_core::wire::be_u64(bytes, MVHD_V1_DURATION_OFFSET, "mvhd v1")?,
                rate: u32_at(bytes, MVHD_V1_RATE_OFFSET),
                volume: u16_at(bytes, MVHD_V1_VOLUME_OFFSET),
                matrix: matrix_from_bytes(
                    &bytes[MVHD_V1_MATRIX_OFFSET..MVHD_V1_MATRIX_OFFSET + MATRIX_WIRE_LEN],
                ),
                next_track_id: TrackId::new(u32_at(bytes, MVHD_V1_NEXT_TRACK_ID_OFFSET)),
            })
        }
    }
}

impl Serialize for MovieHeaderBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        // v1 is 120 bytes, not 124 — see the parse-side comment above (#1015).
        if self.version == 0 {
            MVHD_V0_LEN
        } else {
            MVHD_V1_LEN
        }
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = 0usize;
        c += crate::box_types::write_full_box_header(
            buf,
            b"mvhd",
            need,
            self.version,
            self.flags,
            true,
        )?;
        let u32_len = core::mem::size_of::<u32>();
        let u64_len = core::mem::size_of::<u64>();
        let (ct_sz, mt_sz, dur_sz) = if self.version == 0 {
            (u32_len, u32_len, u32_len)
        } else {
            (u64_len, u64_len, u64_len)
        };
        // Version 0 stores these as 32-bit: an over-range value is an error,
        // never a silent wrap (#1129).
        let write_u64 = |buf: &mut [u8], off: usize, sz: usize, v: u64, field: &'static str| {
            if sz == u32_len {
                let v32 = broadcast_core::len::fit_bits(v, u32::BITS, field)? as u32;
                buf[off..off + sz].copy_from_slice(&v32.to_be_bytes());
            } else {
                buf[off..off + sz].copy_from_slice(&v.to_be_bytes());
            }
            Ok::<(), Error>(())
        };
        write_u64(buf, c, ct_sz, self.creation_time, "mvhd creation_time")?;
        c += ct_sz;
        write_u64(
            buf,
            c,
            mt_sz,
            self.modification_time,
            "mvhd modification_time",
        )?;
        c += mt_sz;
        buf[c..c + u32_len].copy_from_slice(&self.timescale.get().to_be_bytes());
        c += u32_len;
        write_u64(buf, c, dur_sz, self.duration, "mvhd duration")?;
        c += dur_sz;
        buf[c..c + u32_len].copy_from_slice(&self.rate.to_be_bytes());
        c += u32_len;
        let u16_len = core::mem::size_of::<u16>();
        buf[c..c + u16_len].copy_from_slice(&self.volume.to_be_bytes());
        c += u16_len;
        buf[c..c + MVHD_RESERVED_LEN].fill(0);
        c += MVHD_RESERVED_LEN; // reserved
        for &m in &self.matrix {
            buf[c..c + MATRIX_VALUE_LEN].copy_from_slice(&m.to_be_bytes());
            c += MATRIX_VALUE_LEN;
        }
        buf[c..c + MVHD_PRE_DEFINED_LEN].fill(0);
        c += MVHD_PRE_DEFINED_LEN; // pre_defined
        buf[c..c + u32_len].copy_from_slice(&self.next_track_id.value().to_be_bytes());
        c += u32_len;
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// TrackHeaderBox — tkhd (ISO/IEC 14496-12:2026 §8.3.2)
// ---------------------------------------------------------------------------

/// Shortest `tkhd` prefix before its version-dependent fields.
const TKHD_MIN_LEN: usize = FULLBOX_BODY_OFFSET;
/// `tkhd` v0 creation_time offset (§8.3.2.2).
const TKHD_V0_CREATION_TIME_OFFSET: usize = 12;
/// `tkhd` v0 modification_time offset (§8.3.2.2).
const TKHD_V0_MODIFICATION_TIME_OFFSET: usize = 16;
/// `tkhd` v0 track_ID offset (§8.3.2.2).
const TKHD_V0_TRACK_ID_OFFSET: usize = 20;
/// `tkhd` v0 duration offset (§8.3.2.2).
const TKHD_V0_DURATION_OFFSET: usize = 28;
/// `tkhd` v0 layer offset (§8.3.2.2).
const TKHD_V0_LAYER_OFFSET: usize = 40;
/// `tkhd` v0 alternate_group offset (§8.3.2.2).
const TKHD_V0_ALTERNATE_GROUP_OFFSET: usize = 42;
/// `tkhd` v0 volume offset (§8.3.2.2).
const TKHD_V0_VOLUME_OFFSET: usize = 44;
/// `tkhd` v0 matrix offset (§8.3.2.2).
const TKHD_V0_MATRIX_OFFSET: usize = 48;
/// `tkhd` v0 width offset (§8.3.2.2).
const TKHD_V0_WIDTH_OFFSET: usize = 84;
/// `tkhd` v0 height offset (§8.3.2.2).
const TKHD_V0_HEIGHT_OFFSET: usize = 88;
/// `tkhd` v0 box length (§8.3.2.2).
const TKHD_V0_LEN: usize = 92;
/// `tkhd` v1 creation_time offset (§8.3.2.2).
const TKHD_V1_CREATION_TIME_OFFSET: usize = 12;
/// `tkhd` v1 modification_time offset (§8.3.2.2).
const TKHD_V1_MODIFICATION_TIME_OFFSET: usize = 20;
/// `tkhd` v1 track_ID offset (§8.3.2.2).
const TKHD_V1_TRACK_ID_OFFSET: usize = 28;
/// `tkhd` v1 duration offset (§8.3.2.2).
const TKHD_V1_DURATION_OFFSET: usize = 36;
/// `tkhd` v1 layer offset (§8.3.2.2).
const TKHD_V1_LAYER_OFFSET: usize = 52;
/// `tkhd` v1 alternate_group offset (§8.3.2.2).
const TKHD_V1_ALTERNATE_GROUP_OFFSET: usize = 54;
/// `tkhd` v1 volume offset (§8.3.2.2).
const TKHD_V1_VOLUME_OFFSET: usize = 56;
/// `tkhd` v1 matrix offset (§8.3.2.2).
const TKHD_V1_MATRIX_OFFSET: usize = 60;
/// `tkhd` v1 width offset (§8.3.2.2).
const TKHD_V1_WIDTH_OFFSET: usize = 96;
/// `tkhd` v1 height offset (§8.3.2.2).
const TKHD_V1_HEIGHT_OFFSET: usize = 100;
/// `tkhd` v1 box length (§8.3.2.2).
const TKHD_V1_LEN: usize = 104;
/// Single reserved word in `tkhd` (§8.3.2.2).
const TKHD_RESERVED_LEN: usize = 4;
/// The two reserved words after `duration` in `tkhd` (§8.3.2.2).
const TKHD_RESERVED_PAIR_LEN: usize = 8;
/// The reserved field after `volume` in `tkhd` (§8.3.2.2).
const TKHD_TRAILING_RESERVED_LEN: usize = 2;

/// Four-CC of the Track Header Box (`tkhd`, ISO/IEC 14496-12:2026 §8.3.2).
pub const TKHD: [u8; 4] = *b"tkhd";

/// Track Header Box (`tkhd`) — §8.3.2.
/// v0: 32-bit creation_time, modification_time, duration.
/// v1: 64-bit equivalents.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TrackHeaderBox {
    pub version: u8,
    pub flags: u32,
    pub creation_time: u64,
    pub modification_time: u64,
    pub track_id: TrackId,
    pub duration: u64,
    pub layer: i16,
    pub alternate_group: i16,
    pub volume: i16,
    pub matrix: [i32; MATRIX_VALUES],
    pub width: u32,
    pub height: u32,
}

impl<'a> Parse<'a> for TrackHeaderBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(bytes, TKHD_MIN_LEN, "tkhd")?;
        let ver = bytes[FULLBOX_VERSION_OFFSET];
        let flags = u24_at(bytes, FULLBOX_FLAGS_WIRE_OFFSET);
        if ver == 0 {
            need_in(bytes, TKHD_V0_LEN, "tkhd v0")?;
            let ct = u64::from(u32_at(bytes, TKHD_V0_CREATION_TIME_OFFSET));
            let mt = u64::from(u32_at(bytes, TKHD_V0_MODIFICATION_TIME_OFFSET));
            let tid = u32_at(bytes, TKHD_V0_TRACK_ID_OFFSET);
            let dur = u64::from(u32_at(bytes, TKHD_V0_DURATION_OFFSET));
            Ok(Self {
                version: 0,
                flags,
                creation_time: ct,
                modification_time: mt,
                track_id: TrackId::new(tid),
                duration: dur,
                layer: i16_at(bytes, TKHD_V0_LAYER_OFFSET),
                alternate_group: i16_at(bytes, TKHD_V0_ALTERNATE_GROUP_OFFSET),
                volume: i16_at(bytes, TKHD_V0_VOLUME_OFFSET),
                matrix: matrix_from_bytes(
                    &bytes[TKHD_V0_MATRIX_OFFSET..TKHD_V0_MATRIX_OFFSET + MATRIX_WIRE_LEN],
                ),
                width: u32_at(bytes, TKHD_V0_WIDTH_OFFSET),
                height: u32_at(bytes, TKHD_V0_HEIGHT_OFFSET),
            })
        } else {
            need_in(bytes, TKHD_V1_LEN, "tkhd v1")?;
            // v1 body (ISO/IEC 14496-12:2026 §8.3.2): track_ID(32) + a single
            // 32-bit `reserved` (not 8) sit between modification_time and
            // duration, so duration starts at byte 36, not 40 — every field
            // from `duration` on was previously read 4 bytes early
            // (issue #1016).
            let ct = broadcast_core::wire::be_u64(bytes, TKHD_V1_CREATION_TIME_OFFSET, "tkhd v1")?;
            let mt =
                broadcast_core::wire::be_u64(bytes, TKHD_V1_MODIFICATION_TIME_OFFSET, "tkhd v1")?;
            let tid = u32_at(bytes, TKHD_V1_TRACK_ID_OFFSET);
            let dur = broadcast_core::wire::be_u64(bytes, TKHD_V1_DURATION_OFFSET, "tkhd v1")?;
            Ok(Self {
                version: 1,
                flags,
                creation_time: ct,
                modification_time: mt,
                track_id: TrackId::new(tid),
                duration: dur,
                layer: i16_at(bytes, TKHD_V1_LAYER_OFFSET),
                alternate_group: i16_at(bytes, TKHD_V1_ALTERNATE_GROUP_OFFSET),
                volume: i16_at(bytes, TKHD_V1_VOLUME_OFFSET),
                matrix: matrix_from_bytes(
                    &bytes[TKHD_V1_MATRIX_OFFSET..TKHD_V1_MATRIX_OFFSET + MATRIX_WIRE_LEN],
                ),
                width: u32_at(bytes, TKHD_V1_WIDTH_OFFSET),
                height: u32_at(bytes, TKHD_V1_HEIGHT_OFFSET),
            })
        }
    }
}

fn matrix_from_bytes(b: &[u8]) -> [i32; MATRIX_VALUES] {
    let mut m = [0i32; MATRIX_VALUES];
    for (i, slot) in m.iter_mut().enumerate() {
        *slot = broadcast_core::wire::i32_at(b, i * MATRIX_VALUE_LEN);
    }
    m
}

impl Serialize for TrackHeaderBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        if self.version == 0 {
            TKHD_V0_LEN
        } else {
            TKHD_V1_LEN
        }
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = 0usize;
        c += crate::box_types::write_full_box_header(
            buf,
            b"tkhd",
            need,
            self.version,
            self.flags,
            true,
        )?;
        let u32_len = core::mem::size_of::<u32>();
        let u64_len = core::mem::size_of::<u64>();
        let i16_len = core::mem::size_of::<i16>();
        if self.version == 0 {
            buf[c..c + u32_len].copy_from_slice(
                &(broadcast_core::len::fit_bits(
                    self.creation_time,
                    u32::BITS,
                    "tkhd creation_time",
                )? as u32)
                    .to_be_bytes(),
            );
            c += u32_len;
            buf[c..c + u32_len].copy_from_slice(
                &(broadcast_core::len::fit_bits(
                    self.modification_time,
                    u32::BITS,
                    "tkhd modification_time",
                )? as u32)
                    .to_be_bytes(),
            );
            c += u32_len;
            buf[c..c + u32_len].copy_from_slice(&self.track_id.value().to_be_bytes());
            c += u32_len;
            buf[c..c + TKHD_RESERVED_LEN].fill(0);
            c += TKHD_RESERVED_LEN; // reserved
            buf[c..c + u32_len].copy_from_slice(
                &(broadcast_core::len::fit_bits(self.duration, u32::BITS, "tkhd duration")? as u32)
                    .to_be_bytes(),
            );
            c += u32_len;
            buf[c..c + TKHD_RESERVED_PAIR_LEN].fill(0);
            c += TKHD_RESERVED_PAIR_LEN; // reserved * 2
            buf[c..c + i16_len].copy_from_slice(&self.layer.to_be_bytes());
            c += i16_len;
            buf[c..c + i16_len].copy_from_slice(&self.alternate_group.to_be_bytes());
            c += i16_len;
            buf[c..c + i16_len].copy_from_slice(&self.volume.to_be_bytes());
            c += i16_len;
            buf[c..c + TKHD_TRAILING_RESERVED_LEN].fill(0);
            c += TKHD_TRAILING_RESERVED_LEN; // reserved
        } else {
            buf[c..c + u64_len].copy_from_slice(&self.creation_time.to_be_bytes());
            c += u64_len;
            buf[c..c + u64_len].copy_from_slice(&self.modification_time.to_be_bytes());
            c += u64_len;
            buf[c..c + u32_len].copy_from_slice(&self.track_id.value().to_be_bytes());
            c += u32_len;
            buf[c..c + TKHD_RESERVED_LEN].fill(0);
            c += TKHD_RESERVED_LEN; // reserved
            buf[c..c + u64_len].copy_from_slice(&self.duration.to_be_bytes());
            c += u64_len;
            buf[c..c + TKHD_RESERVED_PAIR_LEN].fill(0);
            c += TKHD_RESERVED_PAIR_LEN; // reserved * 2
            buf[c..c + i16_len].copy_from_slice(&self.layer.to_be_bytes());
            c += i16_len;
            buf[c..c + i16_len].copy_from_slice(&self.alternate_group.to_be_bytes());
            c += i16_len;
            buf[c..c + i16_len].copy_from_slice(&self.volume.to_be_bytes());
            c += i16_len;
            buf[c..c + TKHD_TRAILING_RESERVED_LEN].fill(0);
            c += TKHD_TRAILING_RESERVED_LEN; // reserved
        }
        for &m in &self.matrix {
            buf[c..c + MATRIX_VALUE_LEN].copy_from_slice(&m.to_be_bytes());
            c += MATRIX_VALUE_LEN;
        }
        buf[c..c + u32_len].copy_from_slice(&self.width.to_be_bytes());
        c += u32_len;
        buf[c..c + u32_len].copy_from_slice(&self.height.to_be_bytes());
        c += u32_len;
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// MediaHeaderBox — mdhd (ISO/IEC 14496-12:2026 §8.4.2)
// ---------------------------------------------------------------------------

/// Shortest `mdhd` prefix before its version-dependent fields.
const MDHD_MIN_LEN: usize = FULLBOX_BODY_OFFSET;
/// `mdhd` v0 creation_time offset (§8.4.2.2).
const MDHD_V0_CREATION_TIME_OFFSET: usize = 12;
/// `mdhd` v0 modification_time offset (§8.4.2.2).
const MDHD_V0_MODIFICATION_TIME_OFFSET: usize = 16;
/// `mdhd` v0 timescale offset (§8.4.2.2).
const MDHD_V0_TIMESCALE_OFFSET: usize = 20;
/// `mdhd` v0 duration offset (§8.4.2.2).
const MDHD_V0_DURATION_OFFSET: usize = 24;
/// `mdhd` v0 language offset (§8.4.2.2).
const MDHD_V0_LANGUAGE_OFFSET: usize = 28;
/// `mdhd` v0 box length (§8.4.2.2).
const MDHD_V0_LEN: usize = 32;
/// `mdhd` v1 creation_time offset (§8.4.2.2).
const MDHD_V1_CREATION_TIME_OFFSET: usize = 12;
/// `mdhd` v1 modification_time offset (§8.4.2.2).
const MDHD_V1_MODIFICATION_TIME_OFFSET: usize = 20;
/// `mdhd` v1 timescale offset (§8.4.2.2).
const MDHD_V1_TIMESCALE_OFFSET: usize = 28;
/// `mdhd` v1 duration offset (§8.4.2.2).
const MDHD_V1_DURATION_OFFSET: usize = 32;
/// `mdhd` v1 language offset (§8.4.2.2).
const MDHD_V1_LANGUAGE_OFFSET: usize = 40;
/// `mdhd` v1 box length (§8.4.2.2).
const MDHD_V1_LEN: usize = 44;
/// `quality` and reserved fields after `language` (§8.4.2.2).
const MDHD_QUALITY_RESERVED_LEN: usize = 2;

/// Media Header Box (`mdhd`) — §8.4.2.
/// v0: 32-bit creation/modification/duration; v1: 64-bit.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MediaHeaderBox {
    pub version: u8,
    pub flags: u32,
    pub creation_time: u64,
    pub modification_time: u64,
    #[cfg_attr(feature = "serde", serde(serialize_with = "crate::ser_timescale"))]
    pub timescale: Timescale,
    pub duration: u64,
    pub language: u16,
}

impl<'a> Parse<'a> for MediaHeaderBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(bytes, MDHD_MIN_LEN, "mdhd")?;
        let ver = bytes[FULLBOX_VERSION_OFFSET];
        let flags = u24_at(bytes, FULLBOX_FLAGS_WIRE_OFFSET);
        if ver == 0 {
            need_in(bytes, MDHD_V0_LEN, "mdhd v0")?;
            Ok(Self {
                version: 0,
                flags,
                creation_time: u64::from(u32_at(bytes, MDHD_V0_CREATION_TIME_OFFSET)),
                modification_time: u64::from(u32_at(bytes, MDHD_V0_MODIFICATION_TIME_OFFSET)),
                timescale: Timescale::try_new(u32_at(bytes, MDHD_V0_TIMESCALE_OFFSET))?,
                duration: u64::from(u32_at(bytes, MDHD_V0_DURATION_OFFSET)),
                language: u16_at(bytes, MDHD_V0_LANGUAGE_OFFSET),
            })
        } else {
            need_in(bytes, MDHD_V1_LEN, "mdhd v1")?;
            Ok(Self {
                version: 1,
                flags,
                creation_time: broadcast_core::wire::be_u64(
                    bytes,
                    MDHD_V1_CREATION_TIME_OFFSET,
                    "mdhd v1",
                )?,
                modification_time: broadcast_core::wire::be_u64(
                    bytes,
                    MDHD_V1_MODIFICATION_TIME_OFFSET,
                    "mdhd v1",
                )?,
                timescale: Timescale::try_new(u32_at(bytes, MDHD_V1_TIMESCALE_OFFSET))?,
                duration: broadcast_core::wire::be_u64(bytes, MDHD_V1_DURATION_OFFSET, "mdhd v1")?,
                language: u16_at(bytes, MDHD_V1_LANGUAGE_OFFSET),
            })
        }
    }
}

impl Serialize for MediaHeaderBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        if self.version == 0 {
            MDHD_V0_LEN
        } else {
            MDHD_V1_LEN
        }
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = 0usize;
        c += crate::box_types::write_full_box_header(
            buf,
            b"mdhd",
            need,
            self.version,
            self.flags,
            true,
        )?;
        let u32_len = core::mem::size_of::<u32>();
        let u64_len = core::mem::size_of::<u64>();
        let u16_len = core::mem::size_of::<u16>();
        if self.version == 0 {
            buf[c..c + u32_len].copy_from_slice(
                &(broadcast_core::len::fit_bits(
                    self.creation_time,
                    u32::BITS,
                    "mdhd creation_time",
                )? as u32)
                    .to_be_bytes(),
            );
            c += u32_len;
            buf[c..c + u32_len].copy_from_slice(
                &(broadcast_core::len::fit_bits(
                    self.modification_time,
                    u32::BITS,
                    "mdhd modification_time",
                )? as u32)
                    .to_be_bytes(),
            );
            c += u32_len;
            buf[c..c + u32_len].copy_from_slice(&self.timescale.get().to_be_bytes());
            c += u32_len;
            buf[c..c + u32_len].copy_from_slice(
                &(broadcast_core::len::fit_bits(self.duration, u32::BITS, "mdhd duration")? as u32)
                    .to_be_bytes(),
            );
            c += u32_len;
            buf[c..c + u16_len].copy_from_slice(&self.language.to_be_bytes());
            c += u16_len;
        } else {
            buf[c..c + u64_len].copy_from_slice(&self.creation_time.to_be_bytes());
            c += u64_len;
            buf[c..c + u64_len].copy_from_slice(&self.modification_time.to_be_bytes());
            c += u64_len;
            buf[c..c + u32_len].copy_from_slice(&self.timescale.get().to_be_bytes());
            c += u32_len;
            buf[c..c + u64_len].copy_from_slice(&self.duration.to_be_bytes());
            c += u64_len;
            buf[c..c + u16_len].copy_from_slice(&self.language.to_be_bytes());
            c += u16_len;
        }
        // quality(16), reserved = 0 — written, not just counted, so a reused
        // caller buffer cannot keep garbage there (r04-W45).
        buf[c..c + MDHD_QUALITY_RESERVED_LEN].fill(0);
        Ok(c + MDHD_QUALITY_RESERVED_LEN)
    }
}

// ---------------------------------------------------------------------------
// HandlerBox — hdlr (ISO/IEC 14496-12:2026 §8.4.3)
// ---------------------------------------------------------------------------

/// Shortest `hdlr` prefix ending at `handler_type` (§8.4.3.2).
const HDLR_MIN_LEN: usize = 24;
/// Byte offset of `handler_type` (§8.4.3.2).
const HDLR_HANDLER_TYPE_OFFSET: usize = 16;
/// Byte offset of the handler `name` string (§8.4.3.2).
const HDLR_NAME_OFFSET: usize = 32;
/// Fixed `hdlr` body after the FullBox fields (§8.4.3.2).
const HDLR_FIXED_BODY_LEN: usize = 20;
/// Width of the `pre_defined` field (§8.4.3.2).
const HDLR_PRE_DEFINED_LEN: usize = 4;
/// Byte offset of `pre_defined` (§8.4.3.2).
const HDLR_PRE_DEFINED_OFFSET: usize = 12;
/// Reserved bytes after `handler_type` (§8.4.3.2).
pub const HDLR_RESERVED_LEN: usize = 12;

/// Handler Box (`hdlr`) — §8.4.3.
/// Declares the media handler type (`vide`, `soun`, etc.) and an optional name.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct HandlerBox {
    pub version: u8,
    pub flags: u32,
    /// The `pre_defined` field (§8.4.3.2). ISO files write 0; a QuickTime
    /// file's own convention puts the component type there (`mhlr`/`dhlr`),
    /// which is carried verbatim so a `.mov` round-trips.
    pub pre_defined: u32,
    pub handler_type: [u8; BOX_TYPE_SIZE],
    /// The 12 reserved bytes after `handler_type` (§8.4.3.2), carried verbatim:
    /// Apple writes its manufacturer code and flags there.
    pub reserved: [u8; HDLR_RESERVED_LEN],
    pub name: Vec<u8>,
}

impl HandlerBox {
    /// The registered handler type this box declares.
    #[must_use]
    pub fn handler(&self) -> crate::registered::HandlerType {
        crate::registered::HandlerType::from_fourcc(self.handler_type)
    }
}

impl<'a> Parse<'a> for HandlerBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let compact = crate::box_types::compact_form(bytes)?;
        let bytes: &[u8] = &compact;
        need_in(bytes, HDLR_MIN_LEN, "hdlr")?;
        let ver = bytes[FULLBOX_VERSION_OFFSET];
        let flags = u24_at(bytes, FULLBOX_FLAGS_WIRE_OFFSET);
        let pre_defined = u32_at(bytes, HDLR_PRE_DEFINED_OFFSET);
        let handler_type = fourcc_at(bytes, HDLR_HANDLER_TYPE_OFFSET);
        let mut reserved = [0u8; HDLR_RESERVED_LEN];
        reserved.copy_from_slice(
            bytes
                .get(HDLR_HANDLER_TYPE_OFFSET + BOX_TYPE_SIZE..HDLR_NAME_OFFSET)
                .ok_or(Error::BufferTooShort(BufferTooShort {
                    need: HDLR_NAME_OFFSET,
                    have: bytes.len(),
                    what: "hdlr reserved",
                }))?,
        );
        let name = if bytes.len() > HDLR_NAME_OFFSET {
            bytes[HDLR_NAME_OFFSET..].to_vec()
        } else {
            Vec::new()
        };
        Ok(Self {
            version: ver,
            flags,
            pre_defined,
            handler_type,
            reserved,
            name,
        })
    }
}

impl Serialize for HandlerBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        crate::box_types::box_len(
            BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + HDLR_FIXED_BODY_LEN + self.name.len(),
        )
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = 0usize;
        c += crate::box_types::write_full_box_header(
            buf,
            b"hdlr",
            need,
            self.version,
            self.flags,
            false,
        )?;
        // pre_defined(32), carried from the source rather than skipped
        // (r04-W45): a QuickTime file keeps its component type here.
        buf[c..c + HDLR_PRE_DEFINED_LEN].copy_from_slice(&self.pre_defined.to_be_bytes());
        c += HDLR_PRE_DEFINED_LEN;
        buf[c..c + BOX_TYPE_SIZE].copy_from_slice(&self.handler_type);
        c += BOX_TYPE_SIZE;
        // reserved[3] (12 bytes), carried from the source rather than skipped
        // (r04-W45): Apple writes its manufacturer code here.
        buf[c..c + HDLR_RESERVED_LEN].copy_from_slice(&self.reserved);
        c += HDLR_RESERVED_LEN;
        if !self.name.is_empty() {
            buf[c..c + self.name.len()].copy_from_slice(&self.name);
        }
        Ok(c + self.name.len())
    }
}

// ---------------------------------------------------------------------------
// VideoMediaHeaderBox — vmhd (ISO/IEC 14496-12:2026 §12.1.2)
// ---------------------------------------------------------------------------

/// Shortest `vmhd` body (§12.1.2).
const VMHD_MIN_LEN: usize = 20;
/// Byte offset of `graphicsmode` (§12.1.2).
const VMHD_GRAPHICSMODE_OFFSET: usize = 12;
/// Byte offset of `opcolor` (§12.1.2).
const VMHD_OPCOLOR_OFFSET: usize = 14;
/// Number of `opcolor` components (§12.1.2).
pub const OPCOLOR_VALUES: usize = 3;
/// `vmhd` body length after the FullBox fields (§12.1.2).
const VMHD_BODY_LEN: usize = 8;

/// Video Media Header Box (`vmhd`) — §12.1.2.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct VideoMediaHeaderBox {
    pub version: u8,
    pub flags: u32,
    pub graphicsmode: u16,
    pub opcolor: [u16; OPCOLOR_VALUES],
}

impl<'a> Parse<'a> for VideoMediaHeaderBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(bytes, VMHD_MIN_LEN, "vmhd")?;
        let ver = bytes[FULLBOX_VERSION_OFFSET];
        let flags = u24_at(bytes, FULLBOX_FLAGS_WIRE_OFFSET);
        let mut opcolor = [0u16; OPCOLOR_VALUES];
        for (i, slot) in opcolor.iter_mut().enumerate() {
            *slot = u16_at(bytes, VMHD_OPCOLOR_OFFSET + i * core::mem::size_of::<u16>());
        }
        Ok(Self {
            version: ver,
            flags,
            graphicsmode: u16_at(bytes, VMHD_GRAPHICSMODE_OFFSET),
            opcolor,
        })
    }
}

impl Serialize for VideoMediaHeaderBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + VMHD_BODY_LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = 0usize;
        c += crate::box_types::write_full_box_header(
            buf,
            b"vmhd",
            need,
            self.version,
            self.flags,
            true,
        )?;
        let u16_len = core::mem::size_of::<u16>();
        buf[c..c + u16_len].copy_from_slice(&self.graphicsmode.to_be_bytes());
        c += u16_len;
        for color in &self.opcolor {
            buf[c..c + u16_len].copy_from_slice(&color.to_be_bytes());
            c += u16_len;
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// SoundMediaHeaderBox — smhd (ISO/IEC 14496-12:2026 §12.2.2)
// ---------------------------------------------------------------------------

/// Shortest `smhd` body (§12.2.2).
const SMHD_MIN_LEN: usize = 16;
/// Byte offset of `balance` (§12.2.2).
const SMHD_BALANCE_OFFSET: usize = 12;
/// `smhd` body length after the FullBox fields (§12.2.2).
const SMHD_BODY_LEN: usize = 4;
/// Reserved field after `balance` (§12.2.2).
const SMHD_RESERVED_LEN: usize = 2;

/// Sound Media Header Box (`smhd`) — §12.2.2.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SoundMediaHeaderBox {
    pub version: u8,
    pub flags: u32,
    pub balance: i16,
}

impl<'a> Parse<'a> for SoundMediaHeaderBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(bytes, SMHD_MIN_LEN, "smhd")?;
        let ver = bytes[FULLBOX_VERSION_OFFSET];
        let flags = u24_at(bytes, FULLBOX_FLAGS_WIRE_OFFSET);
        Ok(Self {
            version: ver,
            flags,
            balance: i16_at(bytes, SMHD_BALANCE_OFFSET),
        })
    }
}

impl Serialize for SoundMediaHeaderBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + SMHD_BODY_LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = 0usize;
        c += crate::box_types::write_full_box_header(
            buf,
            b"smhd",
            need,
            self.version,
            self.flags,
            true,
        )?;
        let i16_len = core::mem::size_of::<i16>();
        buf[c..c + i16_len].copy_from_slice(&self.balance.to_be_bytes());
        c += i16_len;
        // reserved(16) = 0, written rather than skipped (r04-W45).
        buf[c..c + SMHD_RESERVED_LEN].fill(0);
        c += SMHD_RESERVED_LEN;
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// NullMediaHeaderBox — nmhd (ISO/IEC 14496-12:2026 §8.4.5.2)
// HintMediaHeaderBox — hmhd (§12.4.3.1)
// VolumetricVisualMediaHeaderBox — vvhd (§12.9.2)
// ---------------------------------------------------------------------------

/// Four-CC of the Null Media Header Box (`nmhd`, §8.4.5.2).
pub const NMHD: [u8; BOX_TYPE_SIZE] = *b"nmhd";
/// Four-CC of the Hint Media Header Box (`hmhd`, §12.4.3.1).
pub const HMHD: [u8; BOX_TYPE_SIZE] = *b"hmhd";
/// Four-CC of the Volumetric Visual Media Header Box (`vvhd`, §12.9.2).
pub const VVHD: [u8; BOX_TYPE_SIZE] = *b"vvhd";
/// Four-CC of the Subtitle Media Header Box (`sthd`, §12.6.2).
pub const STHD: [u8; BOX_TYPE_SIZE] = *b"sthd";
/// `hmhd` body length after the FullBox fields (§12.4.3.1).
const HMHD_BODY_LEN: usize = 16;
/// Body offset of `maxbitrate`, after the two 16-bit PDU sizes (§12.4.3.1).
const HMHD_MAX_BITRATE_OFFSET: usize = 2 * core::mem::size_of::<u16>();
/// Body offset of `avgbitrate` (§12.4.3.1).
const HMHD_AVG_BITRATE_OFFSET: usize = HMHD_MAX_BITRATE_OFFSET + core::mem::size_of::<u32>();
/// Body offset of the trailing reserved word (§12.4.3.1).
const HMHD_RESERVED_OFFSET: usize = HMHD_AVG_BITRATE_OFFSET + core::mem::size_of::<u32>();

/// Null Media Header Box (`nmhd`) — ISO/IEC 14496-12:2026 §8.4.5.2.
///
/// The header a stream with no media-specific header of its own uses: only
/// the FullBox fields, whose flags the clause reserves.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct NullMediaHeaderBox {
    version: u8,
    flags: u32,
}

impl NullMediaHeaderBox {
    /// A version-0 header with every flag clear.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
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

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, FULLBOX_EXTRA_SIZE, "nmhd")?;
        Ok(Self {
            version: body[0],
            flags: u24_at(body, 1) & FULLBOX_FLAGS_MASK,
        })
    }
}

impl<'a> Parse<'a> for NullMediaHeaderBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(bytes, BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE, "nmhd")?;
        if bytes[BOX_TYPE_OFFSET..BOX_HEADER_MIN_SIZE] != NMHD {
            return Err(Error::UnexpectedBox { expected: "nmhd" });
        }
        Self::parse_body(crate::box_types::box_body(
            bytes,
            FULLBOX_EXTRA_SIZE,
            "nmhd",
        )?)
    }
}

impl Serialize for NullMediaHeaderBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        write_full_box_header(buf, &NMHD, need, self.version, self.flags, false)
    }
}

/// Subtitle Media Header Box (`sthd`) — ISO/IEC 14496-12:2026 §12.6.2.
///
/// The header every subtitle track carries in its `minf`: only the FullBox
/// fields, and the clause fixes both to zero.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct SubtitleMediaHeaderBox {
    version: u8,
    flags: u32,
}

impl SubtitleMediaHeaderBox {
    /// A version-0 header with every flag clear.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
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

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, FULLBOX_EXTRA_SIZE, "sthd")?;
        Ok(Self {
            version: body[0],
            flags: u24_at(body, 1) & FULLBOX_FLAGS_MASK,
        })
    }
}

impl<'a> Parse<'a> for SubtitleMediaHeaderBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(bytes, BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE, "sthd")?;
        if bytes[BOX_TYPE_OFFSET..BOX_HEADER_MIN_SIZE] != STHD {
            return Err(Error::UnexpectedBox { expected: "sthd" });
        }
        Self::parse_body(crate::box_types::box_body(
            bytes,
            FULLBOX_EXTRA_SIZE,
            "sthd",
        )?)
    }
}

impl Serialize for SubtitleMediaHeaderBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        crate::box_types::box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        write_full_box_header(buf, &STHD, need, self.version, self.flags, false)
    }
}

/// Hint Media Header Box (`hmhd`) — ISO/IEC 14496-12:2026 §12.4.3.1.
///
/// The protocol-independent information every hint track declares: PDU sizes
/// and bitrates.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct HintMediaHeaderBox {
    version: u8,
    flags: u32,
    max_pdu_size: u16,
    avg_pdu_size: u16,
    max_bitrate: u32,
    avg_bitrate: u32,
    reserved: u32,
}

impl HintMediaHeaderBox {
    /// A version-0 header with the given PDU sizes and bitrates.
    #[must_use]
    pub fn new(max_pdu_size: u16, avg_pdu_size: u16, max_bitrate: u32, avg_bitrate: u32) -> Self {
        Self {
            version: 0,
            flags: 0,
            max_pdu_size,
            avg_pdu_size,
            max_bitrate,
            avg_bitrate,
            reserved: 0,
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

    /// `maxPDUsize`, in bytes.
    #[must_use]
    pub const fn max_pdu_size(&self) -> u16 {
        self.max_pdu_size
    }

    /// `avgPDUsize`, in bytes.
    #[must_use]
    pub const fn avg_pdu_size(&self) -> u16 {
        self.avg_pdu_size
    }

    /// `maxbitrate`, in bits per second.
    #[must_use]
    pub const fn max_bitrate(&self) -> u32 {
        self.max_bitrate
    }

    /// `avgbitrate`, in bits per second.
    #[must_use]
    pub const fn avg_bitrate(&self) -> u32 {
        self.avg_bitrate
    }

    /// The syntax's trailing reserved word (§12.4.3.1).
    #[must_use]
    pub const fn reserved(&self) -> u32 {
        self.reserved
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, FULLBOX_EXTRA_SIZE + HMHD_BODY_LEN, "hmhd")?;
        let cursor = FULLBOX_EXTRA_SIZE;
        Ok(Self {
            version: body[0],
            flags: u24_at(body, 1) & FULLBOX_FLAGS_MASK,
            max_pdu_size: u16_at(body, cursor),
            avg_pdu_size: u16_at(body, cursor + core::mem::size_of::<u16>()),
            max_bitrate: u32_at(body, cursor + HMHD_MAX_BITRATE_OFFSET),
            avg_bitrate: u32_at(body, cursor + HMHD_AVG_BITRATE_OFFSET),
            reserved: u32_at(body, cursor + HMHD_RESERVED_OFFSET),
        })
    }
}

impl<'a> Parse<'a> for HintMediaHeaderBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(
            bytes,
            BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + HMHD_BODY_LEN,
            "hmhd",
        )?;
        if bytes[BOX_TYPE_OFFSET..BOX_HEADER_MIN_SIZE] != HMHD {
            return Err(Error::UnexpectedBox { expected: "hmhd" });
        }
        Self::parse_body(crate::box_types::box_body(
            bytes,
            FULLBOX_EXTRA_SIZE + HMHD_BODY_LEN,
            "hmhd",
        )?)
    }
}

impl Serialize for HintMediaHeaderBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + HMHD_BODY_LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_full_box_header(buf, &HMHD, need, self.version, self.flags, false)?;
        let u16_len = core::mem::size_of::<u16>();
        let u32_len = core::mem::size_of::<u32>();
        for value in [self.max_pdu_size, self.avg_pdu_size] {
            buf[cursor..cursor + u16_len].copy_from_slice(&value.to_be_bytes());
            cursor += u16_len;
        }
        for value in [self.max_bitrate, self.avg_bitrate, self.reserved] {
            buf[cursor..cursor + u32_len].copy_from_slice(&value.to_be_bytes());
            cursor += u32_len;
        }
        Ok(cursor)
    }
}

/// Volumetric Visual Media Header Box (`vvhd`) — ISO/IEC 14496-12:2026
/// §12.9.2.
///
/// The header every volumetric visual track carries: only the FullBox fields.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct VolumetricVisualMediaHeaderBox {
    version: u8,
    flags: u32,
}

impl VolumetricVisualMediaHeaderBox {
    /// A version-0 header with every flag clear.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
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

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, FULLBOX_EXTRA_SIZE, "vvhd")?;
        Ok(Self {
            version: body[0],
            flags: u24_at(body, 1) & FULLBOX_FLAGS_MASK,
        })
    }
}

impl<'a> Parse<'a> for VolumetricVisualMediaHeaderBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(bytes, BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE, "vvhd")?;
        if bytes[BOX_TYPE_OFFSET..BOX_HEADER_MIN_SIZE] != VVHD {
            return Err(Error::UnexpectedBox { expected: "vvhd" });
        }
        Self::parse_body(crate::box_types::box_body(
            bytes,
            FULLBOX_EXTRA_SIZE,
            "vvhd",
        )?)
    }
}

impl Serialize for VolumetricVisualMediaHeaderBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        write_full_box_header(buf, &VVHD, need, self.version, self.flags, false)
    }
}

/// The media information header one `minf` carries — ISO/IEC 14496-12:2026
/// §8.4.5.
///
/// Exactly one specific media header belongs in a `minf`; this is the crate's
/// typed view of the ones it models.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum MediaInformationHeader {
    /// `vmhd` — [`VideoMediaHeaderBox`] (§12.1.2).
    Video(VideoMediaHeaderBox),
    /// `smhd` — [`SoundMediaHeaderBox`] (§12.2.2).
    Sound(SoundMediaHeaderBox),
    /// `nmhd` — [`NullMediaHeaderBox`] (§8.4.5.2).
    Null(NullMediaHeaderBox),
    /// `hmhd` — [`HintMediaHeaderBox`] (§12.4.3.1).
    Hint(HintMediaHeaderBox),
    /// `vvhd` — [`VolumetricVisualMediaHeaderBox`] (§12.9.2).
    Volumetric(VolumetricVisualMediaHeaderBox),
    /// `sthd` — [`SubtitleMediaHeaderBox`] (§12.6.2).
    Subtitle(SubtitleMediaHeaderBox),
}

impl MediaInformationHeader {
    /// The header's four-CC.
    #[must_use]
    pub const fn fourcc(&self) -> [u8; BOX_TYPE_SIZE] {
        match self {
            Self::Video(_) => *b"vmhd",
            Self::Sound(_) => *b"smhd",
            Self::Null(_) => NMHD,
            Self::Hint(_) => HMHD,
            Self::Volumetric(_) => VVHD,
            Self::Subtitle(_) => STHD,
        }
    }
}

// ---------------------------------------------------------------------------
// DataReferenceBox — dref (ISO/IEC 14496-12:2026 §8.7.2)
// ---------------------------------------------------------------------------

const URL_ENTRY_TYPE: [u8; BOX_TYPE_SIZE] = *b"url ";
const URN_ENTRY_TYPE: [u8; BOX_TYPE_SIZE] = *b"urn ";
const IMDA_ENTRY_TYPE: [u8; BOX_TYPE_SIZE] = *b"imdt";
const SEQ_NUM_IMDA_ENTRY_TYPE: [u8; BOX_TYPE_SIZE] = *b"snim";
/// Length of the NUL terminator after a `urn ` entry name (§8.7.2.2).
const URN_NAME_TERMINATOR_LEN: usize = 1;

/// Data Reference Box (`dref`) — §8.7.2.
/// Contains a list of data entry boxes ([`DataEntry`]).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DataReferenceBox {
    pub version: u8,
    pub flags: u32,
    pub entries: Vec<DataEntry>,
}

/// One entry of a `dref` (§8.7.2.2): the entry four-CC is preserved, so a
/// `urn `, `imdt` or `snim` entry round-trips as itself.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum DataEntry {
    /// `url ` — [`DataEntryUrlBox`].
    Url(DataEntryUrlBox),
    /// `urn ` — [`DataEntryUrnBox`].
    Urn(DataEntryUrnBox),
    /// `imdt` (2026) — [`DataEntryImdaBox`].
    Imda(DataEntryImdaBox),
    /// `snim` (2026) — [`DataEntrySeqNumImdaBox`].
    SeqNumImda(DataEntrySeqNumImdaBox),
    /// An entry type this crate does not model (for example QuickTime
    /// `alis`), kept with its body.
    Other(OpaqueBox),
}

impl From<DataEntryUrlBox> for DataEntry {
    fn from(b: DataEntryUrlBox) -> Self {
        Self::Url(b)
    }
}

impl DataEntry {
    fn parse(bytes: &[u8]) -> Result<Self> {
        need_in(bytes, BOX_HEADER_MIN_SIZE, "dref entry")?;
        let ty = fourcc_at(bytes, BOX_TYPE_OFFSET);
        match ty {
            URL_ENTRY_TYPE => Ok(Self::Url(DataEntryUrlBox::parse(bytes)?)),
            URN_ENTRY_TYPE => Ok(Self::Urn(DataEntryUrnBox::parse(bytes)?)),
            IMDA_ENTRY_TYPE => Ok(Self::Imda(DataEntryImdaBox::parse(bytes)?)),
            SEQ_NUM_IMDA_ENTRY_TYPE => Ok(Self::SeqNumImda(DataEntrySeqNumImdaBox::parse(bytes)?)),
            _ => {
                let item = crate::box_types::parse_box(bytes)?.0;
                let mut opaque = OpaqueBox::new(ty, item.payload().to_vec());
                opaque.to_end = item.to_end();
                opaque.largesize = item.has_largesize();
                Ok(Self::Other(opaque))
            }
        }
    }
}

impl Serialize for DataEntry {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        match self {
            Self::Url(b) => b.serialized_len(),
            Self::Urn(b) => b.serialized_len(),
            Self::Imda(b) => b.serialized_len(),
            Self::SeqNumImda(b) => b.serialized_len(),
            Self::Other(b) => b.serialized_len(),
        }
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Self::Url(b) => b.serialize_into(buf),
            Self::Urn(b) => b.serialize_into(buf),
            Self::Imda(b) => b.serialize_into(buf),
            Self::SeqNumImda(b) => b.serialize_into(buf),
            Self::Other(b) => Ok(b.serialize_into(buf)?),
        }
    }
}

impl<'a> Parse<'a> for DataReferenceBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let compact = crate::box_types::compact_form(bytes)?;
        let bytes: &[u8] = &compact;
        need_in(bytes, FLAT_TABLE_HEADER_LEN, "dref")?;
        let ver = bytes[FLAT_TABLE_VERSION_OFFSET];
        let flags = u24_at(bytes, FLAT_TABLE_FLAGS_OFFSET);
        let count = u32_at(bytes, FLAT_TABLE_COUNT_OFFSET) as usize;
        check_entry_count(
            bytes.len().saturating_sub(FLAT_TABLE_HEADER_LEN),
            BOX_HEADER_MIN_SIZE,
            count,
            "dref.entry_count",
        )?;
        let mut entries = Vec::with_capacity(bounded_entry_count(
            bytes.len().saturating_sub(FLAT_TABLE_HEADER_LEN),
            BOX_HEADER_MIN_SIZE,
            count,
        ));
        for item in BoxIter::clamped(&bytes[FLAT_TABLE_HEADER_LEN..]).take(count) {
            let Ok(item) = item else {
                break;
            };
            entries.push(DataEntry::parse(item.bytes)?);
        }
        Ok(Self {
            version: ver,
            flags,
            entries,
        })
    }
}

impl Serialize for DataReferenceBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>();
        for e in &self.entries {
            n += e.serialized_len();
        }
        crate::box_types::box_len(n)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = 0usize;
        c += crate::box_types::write_full_box_header(
            buf,
            b"dref",
            need,
            self.version,
            self.flags,
            false,
        )?;
        let entry_count = broadcast_core::len::fit_u32(self.entries.len(), "entry_count")?;
        let u32_len = core::mem::size_of::<u32>();
        buf[c..c + u32_len].copy_from_slice(&entry_count.to_be_bytes());
        c += u32_len;
        for entry in &self.entries {
            c += entry.serialize_into(&mut buf[c..])?;
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// DataEntryUrlBox — url  (ISO/IEC 14496-12:2026 §8.7.2)
// ---------------------------------------------------------------------------

/// Data Entry URL Box (`url `) — §8.7.2.
/// When `flags & 1` is set, the media data is in this file (self-contained).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DataEntryUrlBox {
    pub version: u8,
    pub flags: u32,
    pub location: Vec<u8>,
}

impl<'a> Parse<'a> for DataEntryUrlBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let compact = crate::box_types::compact_form(bytes)?;
        let bytes: &[u8] = &compact;
        need_in(bytes, FULLBOX_BODY_OFFSET, "url")?;
        let ver = bytes[FULLBOX_VERSION_OFFSET];
        let flags = u24_at(bytes, FULLBOX_FLAGS_WIRE_OFFSET);
        let location = if bytes.len() > FULLBOX_BODY_OFFSET {
            bytes[FULLBOX_BODY_OFFSET..].to_vec()
        } else {
            Vec::new()
        };
        Ok(Self {
            version: ver,
            flags,
            location,
        })
    }
}

impl Serialize for DataEntryUrlBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        crate::box_types::box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + self.location.len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = 0usize;
        c += crate::box_types::write_full_box_header(
            buf,
            b"url ",
            need,
            self.version,
            self.flags,
            false,
        )?;
        if !self.location.is_empty() {
            buf[c..c + self.location.len()].copy_from_slice(&self.location);
            c += self.location.len();
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// DataEntryUrnBox / DataEntryImdaBox / DataEntrySeqNumImdaBox — urn , imdt, snim
// (ISO/IEC 14496-12:2026 §8.7.2.2)
// ---------------------------------------------------------------------------

fn parse_data_entry_header(
    bytes: &[u8],
    expected: [u8; BOX_TYPE_SIZE],
    what: &'static str,
) -> Result<(u8, u32, Vec<u8>)> {
    let compact = crate::box_types::compact_form(bytes)?;
    need_in(&compact, FULLBOX_BODY_OFFSET, what)?;
    if compact[BOX_TYPE_OFFSET..BOX_HEADER_MIN_SIZE] != expected {
        return Err(Error::InvalidValue(broadcast_core::error::InvalidValue {
            field: "box_type",
            value: u64::from(u32_at(&compact, BOX_TYPE_OFFSET)),
            reason: what,
        }));
    }
    let flags = u24_at(&compact, FULLBOX_FLAGS_WIRE_OFFSET);
    Ok((
        compact[FULLBOX_VERSION_OFFSET],
        flags,
        compact[FULLBOX_BODY_OFFSET..].to_vec(),
    ))
}

fn write_data_entry_header(
    buf: &mut [u8],
    ty: &[u8; BOX_TYPE_SIZE],
    need: usize,
    version: u8,
    flags: u32,
) -> Result<usize> {
    crate::box_types::write_full_box_header(buf, ty, need, version, flags, false)
}

/// Data Entry URN Box (`urn `) — §8.7.2.2: `utf8string name; utf8string
/// location`. `name` is required; `location` is optional and kept with its
/// terminator exactly as on the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DataEntryUrnBox {
    pub version: u8,
    pub flags: u32,
    /// `name` without its NUL terminator.
    pub name: Vec<u8>,
    /// Everything after the `name` terminator (`location`, usually with its
    /// own terminator).
    pub location: Vec<u8>,
}

impl<'a> Parse<'a> for DataEntryUrnBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (version, flags, rest) =
            parse_data_entry_header(bytes, URN_ENTRY_TYPE, "expected urn ")?;
        let nul = rest
            .iter()
            .position(|&b| b == 0)
            .ok_or(Error::InvalidInput(
                "urn entry: name is not NUL-terminated (ISO/IEC 14496-12:2026 §8.7.2.2)",
            ))?;
        Ok(Self {
            version,
            flags,
            name: rest[..nul].to_vec(),
            location: rest[nul + URN_NAME_TERMINATOR_LEN..].to_vec(),
        })
    }
}

impl Serialize for DataEntryUrnBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        crate::box_types::box_len(
            BOX_HEADER_MIN_SIZE
                + FULLBOX_EXTRA_SIZE
                + self.name.len()
                + URN_NAME_TERMINATOR_LEN
                + self.location.len(),
        )
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if self.name.contains(&0) {
            return Err(Error::InvalidInput("urn entry: name contains a NUL byte"));
        }
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = write_data_entry_header(buf, &URN_ENTRY_TYPE, need, self.version, self.flags)?;
        buf[c..c + self.name.len()].copy_from_slice(&self.name);
        c += self.name.len();
        buf[c] = 0;
        c += URN_NAME_TERMINATOR_LEN;
        buf[c..c + self.location.len()].copy_from_slice(&self.location);
        c += self.location.len();
        Ok(c)
    }
}

/// Data Entry IMDA Box (`imdt`, 2026) — §8.7.2.2: references the
/// IdentifiedMediaDataBox whose `imda_identifier` equals `imda_ref_identifier`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DataEntryImdaBox {
    pub version: u8,
    pub flags: u32,
    pub imda_ref_identifier: u32,
}

impl<'a> Parse<'a> for DataEntryImdaBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (version, flags, rest) =
            parse_data_entry_header(bytes, IMDA_ENTRY_TYPE, "expected imdt")?;
        need_in(
            &rest,
            core::mem::size_of::<u32>(),
            "imdt imda_ref_identifier",
        )?;
        Ok(Self {
            version,
            flags,
            imda_ref_identifier: u32_at(&rest, 0),
        })
    }
}

impl Serialize for DataEntryImdaBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        crate::box_types::box_len(
            BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>(),
        )
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = write_data_entry_header(buf, &IMDA_ENTRY_TYPE, need, self.version, self.flags)?;
        let u32_len = core::mem::size_of::<u32>();
        buf[c..c + u32_len].copy_from_slice(&self.imda_ref_identifier.to_be_bytes());
        c += u32_len;
        Ok(c)
    }
}

/// Data Entry Sequence-Number IMDA Box (`snim`, 2026) — §8.7.2.2: no fields;
/// the IdentifiedMediaDataBox is the one whose `imda_identifier` equals the
/// enclosing movie fragment's `sequence_number`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DataEntrySeqNumImdaBox {
    pub version: u8,
    pub flags: u32,
}

impl<'a> Parse<'a> for DataEntrySeqNumImdaBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (version, flags, _) =
            parse_data_entry_header(bytes, SEQ_NUM_IMDA_ENTRY_TYPE, "expected snim")?;
        Ok(Self { version, flags })
    }
}

impl Serialize for DataEntrySeqNumImdaBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        crate::box_types::box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        write_data_entry_header(
            buf,
            &SEQ_NUM_IMDA_ENTRY_TYPE,
            need,
            self.version,
            self.flags,
        )
    }
}

// ---------------------------------------------------------------------------
// SampleToChunkBox — stsc (ISO/IEC 14496-12:2026 §8.7.4)
// ---------------------------------------------------------------------------

/// `stsc` `first_chunk` offset within an entry (§8.7.4.3).
const STSC_FIRST_CHUNK_OFFSET: usize = 0;
/// `stsc` `samples_per_chunk` offset within an entry (§8.7.4.3).
const STSC_SAMPLES_PER_CHUNK_OFFSET: usize = core::mem::size_of::<u32>();
/// `stsc` `sample_description_index` offset within an entry (§8.7.4.3).
const STSC_SAMPLE_DESCRIPTION_INDEX_OFFSET: usize = 2 * core::mem::size_of::<u32>();
/// Wire length of one `stsc` entry (§8.7.4.3).
const STSC_ENTRY_LEN: usize = 3 * core::mem::size_of::<u32>();

/// Entry in the stsc chunk-to-sample table (§8.7.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct StscEntry {
    pub first_chunk: u32,
    pub samples_per_chunk: u32,
    pub sample_description_index: u32,
}

/// Sample To Chunk Box (`stsc`) — §8.7.4.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SampleToChunkBox {
    pub version: u8,
    pub flags: u32,
    pub entries: Vec<StscEntry>,
}

impl<'a> Parse<'a> for SampleToChunkBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let compact = crate::box_types::compact_form(bytes)?;
        let bytes: &[u8] = &compact;
        need_in(bytes, FLAT_TABLE_HEADER_LEN, "stsc")?;
        let ver = bytes[FLAT_TABLE_VERSION_OFFSET];
        let flags = u24_at(bytes, FLAT_TABLE_FLAGS_OFFSET);
        let count = u32_at(bytes, FLAT_TABLE_COUNT_OFFSET) as usize;
        check_entry_count(
            bytes.len().saturating_sub(FLAT_TABLE_HEADER_LEN),
            STSC_ENTRY_LEN,
            count,
            "SampleToChunkBox.entry_count",
        )?;
        let mut entries = Vec::with_capacity(bounded_entry_count(
            bytes.len().saturating_sub(FLAT_TABLE_HEADER_LEN),
            STSC_ENTRY_LEN,
            count,
        ));
        let mut off = FLAT_TABLE_HEADER_LEN;
        for _ in 0..count {
            if off + STSC_ENTRY_LEN > bytes.len() {
                break;
            }
            entries.push(StscEntry {
                first_chunk: u32_at(bytes, off + STSC_FIRST_CHUNK_OFFSET),
                samples_per_chunk: u32_at(bytes, off + STSC_SAMPLES_PER_CHUNK_OFFSET),
                sample_description_index: u32_at(bytes, off + STSC_SAMPLE_DESCRIPTION_INDEX_OFFSET),
            });
            off += STSC_ENTRY_LEN;
        }
        Ok(Self {
            version: ver,
            flags,
            entries,
        })
    }
}

impl Serialize for SampleToChunkBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        crate::box_types::box_len(FLAT_TABLE_HEADER_LEN + self.entries.len() * STSC_ENTRY_LEN)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = 0usize;
        c += crate::box_types::write_full_box_header(
            buf,
            b"stsc",
            need,
            self.version,
            self.flags,
            false,
        )?;
        let entry_count = broadcast_core::len::fit_u32(self.entries.len(), "entry_count")?;
        let u32_len = core::mem::size_of::<u32>();
        buf[c..c + u32_len].copy_from_slice(&entry_count.to_be_bytes());
        c += u32_len;
        for entry in &self.entries {
            let entry_start = c;
            buf[entry_start..entry_start + u32_len]
                .copy_from_slice(&entry.first_chunk.to_be_bytes());
            let samples = entry_start + STSC_SAMPLES_PER_CHUNK_OFFSET;
            buf[samples..samples + u32_len].copy_from_slice(&entry.samples_per_chunk.to_be_bytes());
            let description = entry_start + STSC_SAMPLE_DESCRIPTION_INDEX_OFFSET;
            buf[description..description + u32_len]
                .copy_from_slice(&entry.sample_description_index.to_be_bytes());
            c += STSC_ENTRY_LEN;
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// SampleSizeBox — stsz (ISO/IEC 14496-12:2026 §8.7.3)
// ---------------------------------------------------------------------------

/// Byte offset of `stsz` `sample_size` (§8.7.3.2).
const STSZ_SAMPLE_SIZE_OFFSET: usize = FULLBOX_BODY_OFFSET;
/// Byte offset of `stsz` `sample_count` (§8.7.3.2).
const STSZ_SAMPLE_COUNT_OFFSET: usize = FLAT_TABLE_HEADER_LEN;
/// Length of the `stsz` header before its per-sample size entries (§8.7.3.2).
const STSZ_PREFIX_LEN: usize = FLAT_TABLE_HEADER_LEN + core::mem::size_of::<u32>();

/// Sample Size Box (`stsz`) — §8.7.3.
/// If `sample_size > 0`, all samples have that uniform size and the entries vec
/// is empty. If `sample_size == 0`, entries contains per-sample sizes.
///
/// `sample_count` (wire `unsigned int(32)`) is carried explicitly for the
/// same reason as `SampleAuxInfoSizesBox::sample_count` (issue #1013): when
/// `sample_size != 0` the wire count is real but `entries` stays empty, so
/// `entries.len()` alone cannot stand in for it without collapsing every
/// uniform-size track to "0 samples" on re-serialize (issue #1018).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SampleSizeBox {
    pub version: u8,
    pub flags: u32,
    pub sample_size: u32,
    pub sample_count: u32,
    pub entries: Vec<u32>,
}

impl<'a> Parse<'a> for SampleSizeBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let compact = crate::box_types::compact_form(bytes)?;
        let bytes: &[u8] = &compact;
        need_in(bytes, STSZ_PREFIX_LEN, "stsz")?;
        let ver = bytes[FULLBOX_VERSION_OFFSET];
        let flags = u24_at(bytes, FULLBOX_FLAGS_WIRE_OFFSET);
        let sample_size = u32_at(bytes, STSZ_SAMPLE_SIZE_OFFSET);
        let count = u32_at(bytes, STSZ_SAMPLE_COUNT_OFFSET) as usize;
        if sample_size == 0 {
            check_entry_count(
                bytes.len().saturating_sub(STSZ_PREFIX_LEN),
                core::mem::size_of::<u32>(),
                count,
                "SampleSizeBox sample_count",
            )?;
        }
        // `entries` is only ever populated below when `sample_size == 0`
        // (per-sample sizes) — a nonzero uniform `sample_size` means the loop
        // never runs, so a wire `count` in that branch mustn't drive any
        // allocation at all, not merely a bounded one.
        let entry_len = core::mem::size_of::<u32>();
        let capacity = if sample_size == 0 {
            bounded_entry_count(
                bytes.len().saturating_sub(STSZ_PREFIX_LEN),
                entry_len,
                count,
            )
        } else {
            0
        };
        let mut entries = Vec::with_capacity(capacity);
        if sample_size == 0 {
            let mut off = STSZ_PREFIX_LEN;
            for _ in 0..count {
                if off + entry_len > bytes.len() {
                    break;
                }
                entries.push(u32_at(bytes, off));
                off += entry_len;
            }
        }
        Ok(Self {
            version: ver,
            flags,
            sample_size,
            sample_count: count as u32,
            entries,
        })
    }
}

impl Serialize for SampleSizeBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let count = if self.sample_size == 0 {
            self.entries.len()
        } else {
            0
        };
        crate::box_types::box_len(STSZ_PREFIX_LEN + count * core::mem::size_of::<u32>())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if self.sample_size == 0 && self.entries.len() != self.sample_count as usize {
            return Err(Error::InvalidInput(
                "stsz: entries.len() must equal sample_count when sample_size == 0",
            ));
        }
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = 0usize;
        c += crate::box_types::write_full_box_header(
            buf,
            b"stsz",
            need,
            self.version,
            self.flags,
            false,
        )?;
        let u32_len = core::mem::size_of::<u32>();
        buf[c..c + u32_len].copy_from_slice(&self.sample_size.to_be_bytes());
        c += u32_len;
        buf[c..c + u32_len].copy_from_slice(&self.sample_count.to_be_bytes());
        c += u32_len;
        for &sz in &self.entries {
            buf[c..c + u32_len].copy_from_slice(&sz.to_be_bytes());
            c += u32_len;
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// ChunkOffsetBox — stco (ISO/IEC 14496-12:2026 §8.7.5)
// ---------------------------------------------------------------------------

/// Chunk Offset Box (`stco`) — §8.7.5 (32-bit offsets).
/// 64-bit offsets via `co64` are captured as an opaque box.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ChunkOffsetBox {
    pub version: u8,
    pub flags: u32,
    pub entries: Vec<u32>,
}

impl<'a> Parse<'a> for ChunkOffsetBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (version, flags, entries) = parse_flat_table(
            bytes,
            "stco",
            "ChunkOffsetBox.entry_count",
            core::mem::size_of::<u32>(),
            |b| u32_at(b, 0),
        )?;
        Ok(Self {
            version,
            flags,
            entries,
        })
    }
}

impl Serialize for ChunkOffsetBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        flat_table_len(self.entries.len(), core::mem::size_of::<u32>())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        serialize_flat_table(
            buf,
            b"stco",
            (self.version, self.flags),
            &self.entries,
            core::mem::size_of::<u32>(),
            |entry, out| {
                let u32_len = core::mem::size_of::<u32>();
                out[..u32_len].copy_from_slice(&entry.to_be_bytes());
                Ok(u32_len)
            },
        )
    }
}

// ---------------------------------------------------------------------------
// ChunkLargeOffsetBox — co64 (ISO/IEC 14496-12:2026 §8.7.5)
// ---------------------------------------------------------------------------

/// Chunk Large Offset Box (`co64`) — §8.7.5 (64-bit chunk offsets).
///
/// The 64-bit sibling of [`ChunkOffsetBox`], used when any chunk offset exceeds
/// [`u32::MAX`]. Same semantics; each entry is an absolute byte offset into the
/// file of the first sample in a chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ChunkLargeOffsetBox {
    pub version: u8,
    pub flags: u32,
    pub entries: Vec<u64>,
}

impl<'a> Parse<'a> for ChunkLargeOffsetBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (version, flags, entries) = parse_flat_table(
            bytes,
            "co64",
            "ChunkLargeOffsetBox.entry_count",
            core::mem::size_of::<u64>(),
            |b| u64_at(b, 0),
        )?;
        Ok(Self {
            version,
            flags,
            entries,
        })
    }
}

impl Serialize for ChunkLargeOffsetBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        flat_table_len(self.entries.len(), core::mem::size_of::<u64>())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        serialize_flat_table(
            buf,
            b"co64",
            (self.version, self.flags),
            &self.entries,
            core::mem::size_of::<u64>(),
            |entry, out| {
                let u64_len = core::mem::size_of::<u64>();
                out[..u64_len].copy_from_slice(&entry.to_be_bytes());
                Ok(u64_len)
            },
        )
    }
}

// ---------------------------------------------------------------------------
// SyncSampleBox — stss (ISO/IEC 14496-12:2026 §8.6.2)
// ---------------------------------------------------------------------------

/// Sync Sample Box (`stss`) — §8.6.2.
///
/// Lists the 1-based indices of the sync (random-access) samples. If the box is
/// absent every sample is a sync sample; when present it is the exhaustive list
/// of random-access points.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SyncSampleBox {
    pub version: u8,
    pub flags: u32,
    /// 1-based sample numbers that are sync samples.
    pub entries: Vec<u32>,
}

impl<'a> Parse<'a> for SyncSampleBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (version, flags, entries) = parse_flat_table(
            bytes,
            "stss",
            "SyncSampleBox.entry_count",
            core::mem::size_of::<u32>(),
            |b| u32_at(b, 0),
        )?;
        Ok(Self {
            version,
            flags,
            entries,
        })
    }
}

impl Serialize for SyncSampleBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        flat_table_len(self.entries.len(), core::mem::size_of::<u32>())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        serialize_flat_table(
            buf,
            b"stss",
            (self.version, self.flags),
            &self.entries,
            core::mem::size_of::<u32>(),
            |entry, out| {
                let u32_len = core::mem::size_of::<u32>();
                out[..u32_len].copy_from_slice(&entry.to_be_bytes());
                Ok(u32_len)
            },
        )
    }
}

// ---------------------------------------------------------------------------
// CompactSampleSizeBox — stz2 (ISO/IEC 14496-12:2026 §8.7.3.3)
// ---------------------------------------------------------------------------

/// `stz2` reserved-field length before `field_size` (§8.7.3.3.1).
const STZ2_RESERVED_LEN: usize = 3;
/// Byte offset of `stz2` `field_size` (§8.7.3.3.1).
const STZ2_FIELD_SIZE_OFFSET: usize = FULLBOX_BODY_OFFSET + STZ2_RESERVED_LEN;
/// Byte offset of `stz2` `sample_count` (§8.7.3.3.1).
const STZ2_SAMPLE_COUNT_OFFSET: usize = STZ2_FIELD_SIZE_OFFSET + 1;
/// Length of the `stz2` header before its size entries (§8.7.3.3.1).
const STZ2_PREFIX_LEN: usize = STZ2_SAMPLE_COUNT_OFFSET + core::mem::size_of::<u32>();
/// Bits in one byte.
const STZ2_BITS_PER_BYTE: usize = u8::BITS as usize;
/// Shift that places the first 4-bit entry in a packed byte's high nibble (§8.7.3.3.2).
const STZ2_HIGH_NIBBLE_SHIFT: u32 = 4;
/// Mask of one 4-bit packed entry.
const STZ2_NIBBLE_MASK: u8 = 0x0F;
/// Entries packed into one byte when `field_size` is 4 (§8.7.3.3.2).
const STZ2_ENTRIES_PER_NIBBLE_BYTE: usize = 2;

/// The `stz2` `field_size` values the spec allows (§8.7.3.3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompactFieldSize {
    Bits4,
    Bits8,
    Bits16,
}

impl CompactFieldSize {
    fn parse(field_size: u8) -> Result<Self> {
        match field_size {
            4 => Ok(Self::Bits4),
            8 => Ok(Self::Bits8),
            16 => Ok(Self::Bits16),
            _ => Err(Error::InvalidValue(broadcast_core::error::InvalidValue {
                field: "stz2.field_size",
                value: u64::from(field_size),
                reason: "field_size must be 4, 8 or 16 (ISO/IEC 14496-12:2026 §8.7.3.3.2)",
            })),
        }
    }

    const fn bits(self) -> u32 {
        match self {
            Self::Bits4 => 4,
            Self::Bits8 => 8,
            Self::Bits16 => 16,
        }
    }

    /// Bytes needed to pack `count` entries at this width.
    fn packed_len(self, count: usize) -> usize {
        let total_bits = count.saturating_mul(self.bits() as usize);
        total_bits.div_ceil(STZ2_BITS_PER_BYTE)
    }
}

/// Compact Sample Size Box (`stz2`) — ISO/IEC 14496-12:2026 §8.7.3.3.
///
/// The compact sibling of [`SampleSizeBox`]: every entry is `field_size`
/// (4, 8 or 16) bits wide, and two 4-bit entries share one byte with the first
/// entry in the high nibble (§8.7.3.3.2). `entries` holds each size widened to
/// a `u32`, so a caller reads the same values whichever size table the file
/// chose.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CompactSampleSizeBox {
    pub version: u8,
    pub flags: u32,
    /// Width of one size entry in bits: 4, 8 or 16 (§8.7.3.3.2).
    pub field_size: u8,
    pub sample_count: u32,
    pub entries: Vec<u32>,
}

impl<'a> Parse<'a> for CompactSampleSizeBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let compact = crate::box_types::compact_form(bytes)?;
        let bytes: &[u8] = &compact;
        need_in(bytes, STZ2_PREFIX_LEN, "stz2")?;
        let version = bytes[FLAT_TABLE_VERSION_OFFSET];
        let flags = u24_at(bytes, FULLBOX_FLAGS_WIRE_OFFSET);
        let field_size = bytes[STZ2_FIELD_SIZE_OFFSET];
        let size = CompactFieldSize::parse(field_size)?;
        let count = u32_at(bytes, STZ2_SAMPLE_COUNT_OFFSET) as usize;
        let payload = &bytes[STZ2_PREFIX_LEN..];
        let needed = size.packed_len(count);
        if needed > payload.len() {
            return Err(Error::BufferTooShort(BufferTooShort {
                need: STZ2_PREFIX_LEN.saturating_add(needed),
                have: bytes.len(),
                what: "stz2 sample sizes",
            }));
        }
        let payload = &payload[..needed];
        let entries: Vec<u32> = match size {
            CompactFieldSize::Bits4 => payload
                .iter()
                .flat_map(|&b| {
                    [
                        u32::from(b >> STZ2_HIGH_NIBBLE_SHIFT),
                        u32::from(b & STZ2_NIBBLE_MASK),
                    ]
                })
                .take(count)
                .collect(),
            CompactFieldSize::Bits8 => payload.iter().take(count).map(|&b| u32::from(b)).collect(),
            CompactFieldSize::Bits16 => payload
                .chunks_exact(core::mem::size_of::<u16>())
                .take(count)
                .map(|c| u32::from(u16::from_be_bytes([c[0], c[1]])))
                .collect(),
        };
        Ok(Self {
            version,
            flags,
            field_size,
            sample_count: count as u32,
            entries,
        })
    }
}

impl Serialize for CompactSampleSizeBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let total_bits = self
            .entries
            .len()
            .saturating_mul(usize::from(self.field_size));
        crate::box_types::box_len(STZ2_PREFIX_LEN + total_bits.div_ceil(STZ2_BITS_PER_BYTE))
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if self.entries.len() != self.sample_count as usize {
            return Err(Error::InvalidInput(
                "stz2: entries.len() must equal sample_count",
            ));
        }
        let size = CompactFieldSize::parse(self.field_size)?;
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = crate::box_types::write_full_box_header(
            buf,
            b"stz2",
            need,
            self.version,
            self.flags,
            false,
        )?;
        buf[c..c + STZ2_RESERVED_LEN].fill(0);
        c += STZ2_RESERVED_LEN;
        buf[c] = self.field_size;
        c += 1;
        let u32_len = core::mem::size_of::<u32>();
        buf[c..c + u32_len].copy_from_slice(&self.sample_count.to_be_bytes());
        c += u32_len;
        match size {
            CompactFieldSize::Bits4 => {
                for pair in self.entries.chunks(STZ2_ENTRIES_PER_NIBBLE_BYTE) {
                    let high = broadcast_core::len::fit_bits(
                        u64::from(pair[0]),
                        size.bits(),
                        "stz2.entry_size",
                    )? as u8;
                    let low = match pair.get(1) {
                        Some(&value) => broadcast_core::len::fit_bits(
                            u64::from(value),
                            size.bits(),
                            "stz2.entry_size",
                        )? as u8,
                        None => 0,
                    };
                    buf[c] = (high << STZ2_HIGH_NIBBLE_SHIFT) | low;
                    c += 1;
                }
            }
            CompactFieldSize::Bits8 => {
                for &value in &self.entries {
                    buf[c] = broadcast_core::len::fit_bits(
                        u64::from(value),
                        size.bits(),
                        "stz2.entry_size",
                    )? as u8;
                    c += 1;
                }
            }
            CompactFieldSize::Bits16 => {
                let u16_len = core::mem::size_of::<u16>();
                for &value in &self.entries {
                    let wide = broadcast_core::len::fit_bits(
                        u64::from(value),
                        size.bits(),
                        "stz2.entry_size",
                    )? as u16;
                    buf[c..c + u16_len].copy_from_slice(&wide.to_be_bytes());
                    c += u16_len;
                }
            }
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// SampleDependencyTypeBox — sdtp (ISO/IEC 14496-12:2026 §8.6.4)
// ---------------------------------------------------------------------------

/// `sdtp` shortest prefix: the Box header plus the FullBox version/flags.
const SDTP_MIN_LEN: usize = FULLBOX_BODY_OFFSET;
/// Shift of the `is_leading` field in an `sdtp` sample byte (§8.6.4.2).
const SDTP_IS_LEADING_SHIFT: u8 = 6;
/// Shift of the `sample_depends_on` field in an `sdtp` sample byte (§8.6.4.2).
const SDTP_SAMPLE_DEPENDS_ON_SHIFT: u8 = 4;
/// Shift of the `sample_is_depended_on` field in an `sdtp` sample byte (§8.6.4.2).
const SDTP_SAMPLE_IS_DEPENDED_ON_SHIFT: u8 = 2;
/// Shift of the `sample_has_redundancy` field in an `sdtp` sample byte (§8.6.4.2).
const SDTP_SAMPLE_HAS_REDUNDANCY_SHIFT: u8 = 0;
/// Mask of one 2-bit `sdtp` field (§8.6.4.2).
const SDTP_FIELD_MASK: u8 = 0x03;

/// Sample Dependency Type Box (`sdtp`) — ISO/IEC 14496-12:2026 §8.6.4.
///
/// One byte per sample, four 2-bit fields (§8.6.4.2). The table's length is the
/// size of the box, so it carries no count of its own; `entries.len()` is the
/// sample count the size table declared.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SampleDependencyTypeBox {
    pub version: u8,
    pub flags: u32,
    /// One packed dependency byte per sample (§8.6.4.2).
    pub entries: Vec<u8>,
}

impl SampleDependencyTypeBox {
    fn field(&self, sample: u32, shift: u8) -> Option<u8> {
        let byte = *self.entries.get(usize::try_from(sample).ok()?)?;
        Some((byte >> shift) & SDTP_FIELD_MASK)
    }

    /// The `is_leading` value of sample `sample` (§8.6.4.3), or `None` past the
    /// table.
    #[must_use]
    pub fn is_leading(&self, sample: u32) -> Option<u8> {
        self.field(sample, SDTP_IS_LEADING_SHIFT)
    }

    /// The `sample_depends_on` value of sample `sample` (§8.6.4.3), or `None`
    /// past the table.
    #[must_use]
    pub fn sample_depends_on(&self, sample: u32) -> Option<u8> {
        self.field(sample, SDTP_SAMPLE_DEPENDS_ON_SHIFT)
    }

    /// The `sample_is_depended_on` value of sample `sample` (§8.6.4.3), or
    /// `None` past the table.
    #[must_use]
    pub fn sample_is_depended_on(&self, sample: u32) -> Option<u8> {
        self.field(sample, SDTP_SAMPLE_IS_DEPENDED_ON_SHIFT)
    }

    /// The `sample_has_redundancy` value of sample `sample` (§8.6.4.3), or
    /// `None` past the table.
    #[must_use]
    pub fn sample_has_redundancy(&self, sample: u32) -> Option<u8> {
        self.field(sample, SDTP_SAMPLE_HAS_REDUNDANCY_SHIFT)
    }
}

impl<'a> Parse<'a> for SampleDependencyTypeBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let compact = crate::box_types::compact_form(bytes)?;
        let bytes: &[u8] = &compact;
        need_in(bytes, SDTP_MIN_LEN, "sdtp")?;
        Ok(Self {
            version: bytes[FLAT_TABLE_VERSION_OFFSET],
            flags: u24_at(bytes, FULLBOX_FLAGS_WIRE_OFFSET),
            entries: bytes[SDTP_MIN_LEN..].to_vec(),
        })
    }
}

impl Serialize for SampleDependencyTypeBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        crate::box_types::box_len(FULLBOX_BODY_OFFSET + self.entries.len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = crate::box_types::write_full_box_header(
            buf,
            b"sdtp",
            need,
            self.version,
            self.flags,
            false,
        )?;
        buf[c..c + self.entries.len()].copy_from_slice(&self.entries);
        c += self.entries.len();
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// SampleDescriptionBox — stsd (ISO/IEC 14496-12:2026 §8.5.2)
// ---------------------------------------------------------------------------

/// The `entry_version`-1 ("AudioSampleEntryV1") form of an audio sample entry —
/// ISO/IEC 14496-12:2026 §12.2.3.2.
///
/// A plain `AudioSampleEntry` carries the sampling rate as a 16.16 fixed-point
/// `u32`, so its integer part is limited to 65535 Hz. A rate above that (88.2,
/// 96, 176.4, 192 kHz, …) cannot be represented and is silently truncated:
/// `192000 << 16` keeps only `0xEE000000`, whose integer part reads back as
/// 60928 Hz (`96000` gives `0x77000000` -> 30464 Hz). The spec's answer
/// is to use the v1 entry, whose `entry_version` field is 1, whose `samplerate`
/// field is the placeholder `1 << 16`, and which carries the real rate in a
/// [`SamplingRateBox`]. Such an entry must sit in an `stsd` with
/// `version == 1`.
///
/// This type exists so the constants have one home; the flag is carried per
/// sample entry as its `entry_version` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioSampleEntryV1;

impl AudioSampleEntryV1 {
    /// The `entry_version` value that marks an `AudioSampleEntryV1`
    /// (ISO/IEC 14496-12:2026 §12.2.3.2: "must be 1").
    pub const ENTRY_VERSION: u16 = 1;
    /// The `stsd` `version` an `AudioSampleEntryV1` requires — "must be in an
    /// stsd with version == 1" (ISO/IEC 14496-12:2026 §12.2.3.2).
    pub const STSD_VERSION: u8 = 1;
    /// The placeholder written into the v1 `samplerate` field:
    /// `template unsigned int(32) samplerate = 1<<16;` (§12.2.3.2).
    pub const SAMPLERATE_PLACEHOLDER: u32 = 1 << 16;

    /// Whether an `AudioSampleEntry`'s 16.16 fixed-point `samplerate` field can
    /// carry `rate` without losing its integer part.
    ///
    /// The integer part is the top 16 bits, so any rate above `u16::MAX` needs
    /// the v1 form (ISO/IEC 14496-12:2026 §12.2.3 vs §12.2.3.2).
    pub fn rate_fits_v0(rate: u32) -> bool {
        rate <= u32::from(u16::MAX)
    }
}

/// Sampling Rate Box (`srat`) — ISO/IEC 14496-12:2026 §12.2.3.1.
///
/// Carries the *actual* sampling rate of an audio track whose rate does not fit
/// the 16.16 fixed-point `samplerate` field of an AudioSampleEntry. It is valid
/// only inside an `AudioSampleEntryV1` (whose `entry_version` is 1 and which
/// must sit in an `stsd` with `version == 1`), where it overrides the
/// `samplerate` field — that field is then written as `1 << 16` (`0x00010000`).
///
/// ```text
/// aligned(8) class SamplingRateBox extends FullBox('srat') {
///     unsigned int(32) sampling_rate;
/// }
/// ```
///
/// 88.2, 96, 176.4 and 192 kHz tracks use this box: `96000 << 16` truncated to
/// the 16.16 field's integer part reads back as 30464 Hz, and `192000 << 16` as
/// 60928 Hz.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SamplingRateBox {
    /// Version of the `FullBox` (`srat` is defined with version 0, flags 0).
    pub version: u8,
    pub flags: u32,
    /// Actual sampling rate in Hz.
    pub sampling_rate: u32,
}

impl SamplingRateBox {
    /// FourCC of this box.
    pub const FOURCC: [u8; BOX_TYPE_SIZE] = *b"srat";
    /// Box size in bytes (8-byte header + FullBox header + one `u32`).
    pub const SIZE: usize = BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>();

    /// Build a version-0/flags-0 `srat` for the given rate.
    pub fn new(sampling_rate: u32) -> Self {
        Self {
            version: 0,
            flags: 0,
            sampling_rate,
        }
    }
}

impl Serialize for SamplingRateBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        Self::SIZE
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = Self::SIZE;
        need_out(buf, need)?;
        buf[0..BOX_TYPE_SIZE].copy_from_slice(&(need as u32).to_be_bytes());
        buf[BOX_TYPE_OFFSET..BOX_HEADER_MIN_SIZE].copy_from_slice(&Self::FOURCC);
        buf[FULLBOX_VERSION_OFFSET] = self.version;
        let fb = self.flags.to_be_bytes();
        buf[FULLBOX_FLAGS_WIRE_OFFSET..FULLBOX_BODY_OFFSET]
            .copy_from_slice(&fb[FULLBOX_VERSION_LEN..]);
        buf[FULLBOX_BODY_OFFSET..FULLBOX_BODY_OFFSET + core::mem::size_of::<u32>()]
            .copy_from_slice(&self.sampling_rate.to_be_bytes());
        Ok(need)
    }
}

impl<'a> Parse<'a> for SamplingRateBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(bytes, Self::SIZE, "srat")?;
        // The four-CC is validated, not assumed: this parser is also reached
        // from a raw child-box walk, where the bytes preceding it may belong to
        // any box at all (a wrong box parsed as an `srat` would silently supply
        // a bogus rate).
        if bytes[BOX_TYPE_OFFSET..BOX_HEADER_MIN_SIZE] != Self::FOURCC {
            return Err(Error::InvalidInput("SamplingRateBox: not an 'srat' box"));
        }
        let version = bytes[FULLBOX_VERSION_OFFSET];
        let flags = u24_at(bytes, FULLBOX_FLAGS_WIRE_OFFSET);
        let sampling_rate = u32_at(bytes, FULLBOX_BODY_OFFSET);
        Ok(Self {
            version,
            flags,
            sampling_rate,
        })
    }
}

/// Read the `srat` body out of an audio sample entry's config boxes, if any.
///
/// A `SamplingRateBox` may appear at most once and takes precedence over the
/// entry's 16.16 `samplerate` field (ISO/IEC 14496-12:2026 §12.2.3.1).
///
/// Returns `None` — leaving the entry's own 16.16 field to stand — when the box
/// is absent, fails to parse, or declares `sampling_rate == 0`. A zero rate is
/// not a rate: honouring it would *replace* a perfectly good 16.16 field with
/// "0 Hz", which is strictly worse than ignoring the malformed box.
pub fn sampling_rate_override(config_boxes: &[OpaqueBox]) -> Option<u32> {
    let b = config_boxes
        .iter()
        .find(|b| b.box_type == SamplingRateBox::FOURCC)?;
    // The body was captured without its 8-byte header; rebuild the FullBox
    // bytes so the typed parser sees a complete box.
    let mut full = vec![0u8; BOX_HEADER_MIN_SIZE + b.data.len()];
    full[BOX_TYPE_OFFSET..BOX_HEADER_MIN_SIZE].copy_from_slice(&b.box_type);
    full[BOX_HEADER_MIN_SIZE..].copy_from_slice(&b.data);
    let srat = SamplingRateBox::parse(&full).ok()?;
    if srat.sampling_rate == 0 {
        return None;
    }
    Some(srat.sampling_rate)
}

/// Width of the `reserved[3]` block in an audio sample entry (§12.2.3.2).
pub const AUDIO_SAMPLE_ENTRY_RESERVED_LEN: usize = 6;
/// Width of the SampleEntry `reserved[6]` block (§8.5.2.2).
const SAMPLE_ENTRY_RESERVED_LEN: usize = AUDIO_SAMPLE_ENTRY_RESERVED_LEN;

/// AAC audio sample entry (`mp4a`) — ISO/IEC 14496-12:2026 §12.2.3.
///
/// Wire layout (32 bytes before optional config children):
/// - SampleEntry: reserved(6) + data_reference_index(16) = 8 bytes
/// - AudioSampleEntry reserved `[2]`: 8 bytes
/// - channelcount(16) + samplesize(16) + predefined(16) + reserved(16) + samplerate(32) = 16 bytes
/// - then config boxes (esds, etc.)
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Mp4aSampleEntry {
    /// The SampleEntry `reserved[6]` bytes (§8.5.2.2), written back verbatim.
    pub reserved: [u8; SAMPLE_ENTRY_RESERVED_LEN],
    /// The FourCC of this sample entry — `mp4a`, or `enca` when the track is
    /// CENC-protected (the `sinf` child then carries the real codec under
    /// `frma`). Previously hard-coded to `mp4a` on serialize, which silently
    /// re-labelled a protected audio track as clear on any parse -> serialize
    /// round trip (issue #1017).
    pub codec_type: [u8; BOX_TYPE_SIZE],
    /// `entry_version`: 0 for a plain `AudioSampleEntry` (§12.2.3), 1 for an
    /// [`AudioSampleEntryV1`] (§12.2.3.2) whose real rate lives in a
    /// [`SamplingRateBox`] among `config_boxes`, or any other value a
    /// QuickTime sound description carried.
    ///
    /// Stored and written back **verbatim** — the field used to be re-derived
    /// as 0-or-1 on serialize, which silently rewrote a QuickTime v2 (`02 00`)
    /// entry as v0 on any parse -> serialize round trip.
    pub entry_version: u16,
    /// The six bytes following `entry_version` in the fixed 8-byte block
    /// (QuickTime's `revision_level` + `vendor`, ISO/IEC 14496-12:2026's
    /// `reserved[3]`). Written back verbatim so a real file round-trips.
    pub reserved_1: [u8; AUDIO_SAMPLE_ENTRY_RESERVED_LEN],
    pub data_reference_index: u16,
    pub channelcount: u16,
    pub samplesize: u16,
    /// The `pre_defined` + `reserved` pair (ISO/IEC 14496-12:2026 §12.2.3) — or, in
    /// a QuickTime sound description, `compression_ID` + `packet_size`.
    /// QuickTime's VBR marker is `compression_ID = -2` (`ff fe`), so this is
    /// not always zero and must survive a round trip.
    pub compression_id_and_packet_size: [u8; core::mem::size_of::<u32>()],
    pub samplerate: u32,
    /// The QuickTime version 1/2 extension, when the entry carries one.
    pub quicktime: Option<QuickTimeSound>,
    pub config_boxes: Vec<OpaqueBox>,
    /// Bytes after the last complete child, kept verbatim.
    pub trailing: Vec<u8>,
}

// ---------------------------------------------------------------------------
// Ac3SampleEntry (ac-3) — ETSI TS 102 366 §F.3
// ---------------------------------------------------------------------------

/// AC-3 audio sample entry (`ac-3`) — ETSI TS 102 366 §F.3.
///
/// Same AudioSampleEntry fixed fields as [`Mp4aSampleEntry`], then a `dac3`
/// config box.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Ac3SampleEntry {
    /// The SampleEntry `reserved[6]` bytes (§8.5.2.2), written back verbatim.
    pub reserved: [u8; SAMPLE_ENTRY_RESERVED_LEN],
    /// `entry_version`: 0 for a plain `AudioSampleEntry` (§12.2.3), 1 for an
    /// [`AudioSampleEntryV1`] (§12.2.3.2) whose real rate lives in a
    /// [`SamplingRateBox`] among `config_boxes`, or any other value a
    /// QuickTime sound description carried.
    ///
    /// Stored and written back **verbatim** — the field used to be re-derived
    /// as 0-or-1 on serialize, which silently rewrote a QuickTime v2 (`02 00`)
    /// entry as v0 on any parse -> serialize round trip.
    pub entry_version: u16,
    /// The six bytes following `entry_version` in the fixed 8-byte block
    /// (QuickTime's `revision_level` + `vendor`, ISO/IEC 14496-12:2026's
    /// `reserved[3]`). Written back verbatim so a real file round-trips.
    pub reserved_1: [u8; AUDIO_SAMPLE_ENTRY_RESERVED_LEN],
    pub data_reference_index: u16,
    pub channelcount: u16,
    pub samplesize: u16,
    /// The `pre_defined` + `reserved` pair (ISO/IEC 14496-12:2026 §12.2.3) — or, in
    /// a QuickTime sound description, `compression_ID` + `packet_size`.
    /// QuickTime's VBR marker is `compression_ID = -2` (`ff fe`), so this is
    /// not always zero and must survive a round trip.
    pub compression_id_and_packet_size: [u8; core::mem::size_of::<u32>()],
    pub samplerate: u32,
    /// The QuickTime version 1/2 extension, when the entry carries one.
    pub quicktime: Option<QuickTimeSound>,
    pub config_boxes: Vec<OpaqueBox>,
    /// Bytes after the last complete child, kept verbatim.
    pub trailing: Vec<u8>,
}

impl Serialize for Ac3SampleEntry {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        audio_sample_entry_serialized_len(&self.quicktime, &self.config_boxes, &self.trailing)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        serialize_audio_sample_entry(
            buf,
            AudioSampleEntryFields {
                fourcc: b"ac-3",
                reserved: self.reserved,
                entry_version: self.entry_version,
                reserved_1: self.reserved_1,
                data_reference_index: self.data_reference_index,
                channelcount: self.channelcount,
                samplesize: self.samplesize,
                compression_id_and_packet_size: self.compression_id_and_packet_size,
                samplerate: self.samplerate,
                quicktime: &self.quicktime,
                config_boxes: &self.config_boxes,
                trailing: &self.trailing,
            },
        )
    }
}

// ---------------------------------------------------------------------------
// Ec3SampleEntry (ec-3) — ETSI TS 102 366 §F.5
// ---------------------------------------------------------------------------

/// E-AC-3 audio sample entry (`ec-3`) — ETSI TS 102 366 §F.5.
///
/// Same AudioSampleEntry fixed fields as [`Mp4aSampleEntry`], then a `dec3`
/// config box.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Ec3SampleEntry {
    /// The SampleEntry `reserved[6]` bytes (§8.5.2.2), written back verbatim.
    pub reserved: [u8; SAMPLE_ENTRY_RESERVED_LEN],
    /// `entry_version`: 0 for a plain `AudioSampleEntry` (§12.2.3), 1 for an
    /// [`AudioSampleEntryV1`] (§12.2.3.2) whose real rate lives in a
    /// [`SamplingRateBox`] among `config_boxes`, or any other value a
    /// QuickTime sound description carried.
    ///
    /// Stored and written back **verbatim** — the field used to be re-derived
    /// as 0-or-1 on serialize, which silently rewrote a QuickTime v2 (`02 00`)
    /// entry as v0 on any parse -> serialize round trip.
    pub entry_version: u16,
    /// The six bytes following `entry_version` in the fixed 8-byte block
    /// (QuickTime's `revision_level` + `vendor`, ISO/IEC 14496-12:2026's
    /// `reserved[3]`). Written back verbatim so a real file round-trips.
    pub reserved_1: [u8; AUDIO_SAMPLE_ENTRY_RESERVED_LEN],
    pub data_reference_index: u16,
    pub channelcount: u16,
    pub samplesize: u16,
    /// The `pre_defined` + `reserved` pair (ISO/IEC 14496-12:2026 §12.2.3) — or, in
    /// a QuickTime sound description, `compression_ID` + `packet_size`.
    /// QuickTime's VBR marker is `compression_ID = -2` (`ff fe`), so this is
    /// not always zero and must survive a round trip.
    pub compression_id_and_packet_size: [u8; core::mem::size_of::<u32>()],
    pub samplerate: u32,
    /// The QuickTime version 1/2 extension, when the entry carries one.
    pub quicktime: Option<QuickTimeSound>,
    pub config_boxes: Vec<OpaqueBox>,
    /// Bytes after the last complete child, kept verbatim.
    pub trailing: Vec<u8>,
}

impl Serialize for Ec3SampleEntry {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        audio_sample_entry_serialized_len(&self.quicktime, &self.config_boxes, &self.trailing)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        serialize_audio_sample_entry(
            buf,
            AudioSampleEntryFields {
                fourcc: b"ec-3",
                reserved: self.reserved,
                entry_version: self.entry_version,
                reserved_1: self.reserved_1,
                data_reference_index: self.data_reference_index,
                channelcount: self.channelcount,
                samplesize: self.samplesize,
                compression_id_and_packet_size: self.compression_id_and_packet_size,
                samplerate: self.samplerate,
                quicktime: &self.quicktime,
                config_boxes: &self.config_boxes,
                trailing: &self.trailing,
            },
        )
    }
}

// ---------------------------------------------------------------------------
// OpusSampleEntry (Opus) / FlacSampleEntry (fLaC) / Ac4SampleEntry (ac-4)
// ---------------------------------------------------------------------------

/// Opus audio sample entry (`Opus`) — Opus-in-ISOBMFF §4.3.2.
///
/// Same AudioSampleEntry fixed fields as [`Mp4aSampleEntry`], then a `dOps` box.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct OpusSampleEntry {
    /// The SampleEntry `reserved[6]` bytes (§8.5.2.2), written back verbatim.
    pub reserved: [u8; SAMPLE_ENTRY_RESERVED_LEN],
    /// `entry_version`: 0 for a plain `AudioSampleEntry` (§12.2.3), 1 for an
    /// [`AudioSampleEntryV1`] (§12.2.3.2) whose real rate lives in a
    /// [`SamplingRateBox`] among `config_boxes`, or any other value a
    /// QuickTime sound description carried.
    ///
    /// Stored and written back **verbatim** — the field used to be re-derived
    /// as 0-or-1 on serialize, which silently rewrote a QuickTime v2 (`02 00`)
    /// entry as v0 on any parse -> serialize round trip.
    pub entry_version: u16,
    /// The six bytes following `entry_version` in the fixed 8-byte block
    /// (QuickTime's `revision_level` + `vendor`, ISO/IEC 14496-12:2026's
    /// `reserved[3]`). Written back verbatim so a real file round-trips.
    pub reserved_1: [u8; AUDIO_SAMPLE_ENTRY_RESERVED_LEN],
    pub data_reference_index: u16,
    pub channelcount: u16,
    pub samplesize: u16,
    /// The `pre_defined` + `reserved` pair (ISO/IEC 14496-12:2026 §12.2.3) — or, in
    /// a QuickTime sound description, `compression_ID` + `packet_size`.
    /// QuickTime's VBR marker is `compression_ID = -2` (`ff fe`), so this is
    /// not always zero and must survive a round trip.
    pub compression_id_and_packet_size: [u8; core::mem::size_of::<u32>()],
    pub samplerate: u32,
    /// The QuickTime version 1/2 extension, when the entry carries one.
    pub quicktime: Option<QuickTimeSound>,
    pub config_boxes: Vec<OpaqueBox>,
    /// Bytes after the last complete child, kept verbatim.
    pub trailing: Vec<u8>,
}

impl Serialize for OpusSampleEntry {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        audio_sample_entry_serialized_len(&self.quicktime, &self.config_boxes, &self.trailing)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        serialize_audio_sample_entry(
            buf,
            AudioSampleEntryFields {
                fourcc: b"Opus",
                reserved: self.reserved,
                entry_version: self.entry_version,
                reserved_1: self.reserved_1,
                data_reference_index: self.data_reference_index,
                channelcount: self.channelcount,
                samplesize: self.samplesize,
                compression_id_and_packet_size: self.compression_id_and_packet_size,
                samplerate: self.samplerate,
                quicktime: &self.quicktime,
                config_boxes: &self.config_boxes,
                trailing: &self.trailing,
            },
        )
    }
}

impl<'a> Parse<'a> for OpusSampleEntry {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let e = parse_audio_sample_entry(bytes, "Opus")?;
        Ok(Self {
            reserved: e.reserved,
            entry_version: e.entry_version,
            reserved_1: e.reserved_1,
            data_reference_index: e.data_reference_index,
            channelcount: e.channelcount,
            samplesize: e.samplesize,
            compression_id_and_packet_size: e.compression_id_and_packet_size,
            samplerate: e.samplerate,
            quicktime: e.quicktime,
            config_boxes: e.config_boxes,
            trailing: e.trailing,
        })
    }
}

/// FLAC audio sample entry (`fLaC`) — FLAC-in-ISOBMFF.
///
/// Same AudioSampleEntry fixed fields as [`Mp4aSampleEntry`], then a `dfLa` box.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct FlacSampleEntry {
    /// The SampleEntry `reserved[6]` bytes (§8.5.2.2), written back verbatim.
    pub reserved: [u8; SAMPLE_ENTRY_RESERVED_LEN],
    /// `entry_version`: 0 for a plain `AudioSampleEntry` (§12.2.3), 1 for an
    /// [`AudioSampleEntryV1`] (§12.2.3.2) whose real rate lives in a
    /// [`SamplingRateBox`] among `config_boxes`, or any other value a
    /// QuickTime sound description carried.
    ///
    /// Stored and written back **verbatim** — the field used to be re-derived
    /// as 0-or-1 on serialize, which silently rewrote a QuickTime v2 (`02 00`)
    /// entry as v0 on any parse -> serialize round trip.
    pub entry_version: u16,
    /// The six bytes following `entry_version` in the fixed 8-byte block
    /// (QuickTime's `revision_level` + `vendor`, ISO/IEC 14496-12:2026's
    /// `reserved[3]`). Written back verbatim so a real file round-trips.
    pub reserved_1: [u8; AUDIO_SAMPLE_ENTRY_RESERVED_LEN],
    pub data_reference_index: u16,
    pub channelcount: u16,
    pub samplesize: u16,
    /// The `pre_defined` + `reserved` pair (ISO/IEC 14496-12:2026 §12.2.3) — or, in
    /// a QuickTime sound description, `compression_ID` + `packet_size`.
    /// QuickTime's VBR marker is `compression_ID = -2` (`ff fe`), so this is
    /// not always zero and must survive a round trip.
    pub compression_id_and_packet_size: [u8; core::mem::size_of::<u32>()],
    pub samplerate: u32,
    /// The QuickTime version 1/2 extension, when the entry carries one.
    pub quicktime: Option<QuickTimeSound>,
    pub config_boxes: Vec<OpaqueBox>,
    /// Bytes after the last complete child, kept verbatim.
    pub trailing: Vec<u8>,
}

impl Serialize for FlacSampleEntry {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        audio_sample_entry_serialized_len(&self.quicktime, &self.config_boxes, &self.trailing)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        serialize_audio_sample_entry(
            buf,
            AudioSampleEntryFields {
                fourcc: b"fLaC",
                reserved: self.reserved,
                entry_version: self.entry_version,
                reserved_1: self.reserved_1,
                data_reference_index: self.data_reference_index,
                channelcount: self.channelcount,
                samplesize: self.samplesize,
                compression_id_and_packet_size: self.compression_id_and_packet_size,
                samplerate: self.samplerate,
                quicktime: &self.quicktime,
                config_boxes: &self.config_boxes,
                trailing: &self.trailing,
            },
        )
    }
}

impl<'a> Parse<'a> for FlacSampleEntry {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let e = parse_audio_sample_entry(bytes, "fLaC")?;
        Ok(Self {
            reserved: e.reserved,
            entry_version: e.entry_version,
            reserved_1: e.reserved_1,
            data_reference_index: e.data_reference_index,
            channelcount: e.channelcount,
            samplesize: e.samplesize,
            compression_id_and_packet_size: e.compression_id_and_packet_size,
            samplerate: e.samplerate,
            quicktime: e.quicktime,
            config_boxes: e.config_boxes,
            trailing: e.trailing,
        })
    }
}

/// AC-4 audio sample entry (`ac-4`) — ETSI TS 103 190-2 §E.4.
///
/// Same AudioSampleEntry fixed fields as [`Mp4aSampleEntry`], then a `dac4` box.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Ac4SampleEntry {
    /// The SampleEntry `reserved[6]` bytes (§8.5.2.2), written back verbatim.
    pub reserved: [u8; SAMPLE_ENTRY_RESERVED_LEN],
    /// `entry_version`: 0 for a plain `AudioSampleEntry` (§12.2.3), 1 for an
    /// [`AudioSampleEntryV1`] (§12.2.3.2) whose real rate lives in a
    /// [`SamplingRateBox`] among `config_boxes`, or any other value a
    /// QuickTime sound description carried.
    ///
    /// Stored and written back **verbatim** — the field used to be re-derived
    /// as 0-or-1 on serialize, which silently rewrote a QuickTime v2 (`02 00`)
    /// entry as v0 on any parse -> serialize round trip.
    pub entry_version: u16,
    /// The six bytes following `entry_version` in the fixed 8-byte block
    /// (QuickTime's `revision_level` + `vendor`, ISO/IEC 14496-12:2026's
    /// `reserved[3]`). Written back verbatim so a real file round-trips.
    pub reserved_1: [u8; AUDIO_SAMPLE_ENTRY_RESERVED_LEN],
    pub data_reference_index: u16,
    pub channelcount: u16,
    pub samplesize: u16,
    /// The `pre_defined` + `reserved` pair (ISO/IEC 14496-12:2026 §12.2.3) — or, in
    /// a QuickTime sound description, `compression_ID` + `packet_size`.
    /// QuickTime's VBR marker is `compression_ID = -2` (`ff fe`), so this is
    /// not always zero and must survive a round trip.
    pub compression_id_and_packet_size: [u8; core::mem::size_of::<u32>()],
    pub samplerate: u32,
    /// The QuickTime version 1/2 extension, when the entry carries one.
    pub quicktime: Option<QuickTimeSound>,
    pub config_boxes: Vec<OpaqueBox>,
    /// Bytes after the last complete child, kept verbatim.
    pub trailing: Vec<u8>,
}

impl Serialize for Ac4SampleEntry {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        audio_sample_entry_serialized_len(&self.quicktime, &self.config_boxes, &self.trailing)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        serialize_audio_sample_entry(
            buf,
            AudioSampleEntryFields {
                fourcc: b"ac-4",
                reserved: self.reserved,
                entry_version: self.entry_version,
                reserved_1: self.reserved_1,
                data_reference_index: self.data_reference_index,
                channelcount: self.channelcount,
                samplesize: self.samplesize,
                compression_id_and_packet_size: self.compression_id_and_packet_size,
                samplerate: self.samplerate,
                quicktime: &self.quicktime,
                config_boxes: &self.config_boxes,
                trailing: &self.trailing,
            },
        )
    }
}

impl<'a> Parse<'a> for Ac4SampleEntry {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let e = parse_audio_sample_entry(bytes, "ac-4")?;
        Ok(Self {
            reserved: e.reserved,
            entry_version: e.entry_version,
            reserved_1: e.reserved_1,
            data_reference_index: e.data_reference_index,
            channelcount: e.channelcount,
            samplesize: e.samplesize,
            compression_id_and_packet_size: e.compression_id_and_packet_size,
            samplerate: e.samplerate,
            quicktime: e.quicktime,
            config_boxes: e.config_boxes,
            trailing: e.trailing,
        })
    }
}

// ---------------------------------------------------------------------------
// DtsSampleEntry (dtsc / dtsh / dtsl / dtse) — ETSI TS 102 114 §E.2
// ---------------------------------------------------------------------------

/// DTS audio sample entry (`dtsc`, `dtsh`, `dtsl`, `dtse`) — ETSI TS 102 114 §E.2.
///
/// Same AudioSampleEntry fixed fields as [`Mp4aSampleEntry`], then a `ddts` box
/// carrying the [`crate::dts::DtsSpecificBox`].  The `codec_type` field records
/// which of the four DTS FourCCs (`dtsc`/`dtsh`/`dtsl`/`dtse`) was parsed or built.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DtsSampleEntry {
    /// The SampleEntry `reserved[6]` bytes (§8.5.2.2), written back verbatim.
    pub reserved: [u8; SAMPLE_ENTRY_RESERVED_LEN],
    /// The FourCC of this sample entry — one of `dtsc`, `dtsh`, `dtsl`, `dtse`.
    pub codec_type: [u8; BOX_TYPE_SIZE],
    /// `entry_version`: 0 for a plain `AudioSampleEntry` (§12.2.3), 1 for an
    /// [`AudioSampleEntryV1`] (§12.2.3.2) whose real rate lives in a
    /// [`SamplingRateBox`] among `config_boxes`, or any other value a
    /// QuickTime sound description carried.
    ///
    /// Stored and written back **verbatim** — the field used to be re-derived
    /// as 0-or-1 on serialize, which silently rewrote a QuickTime v2 (`02 00`)
    /// entry as v0 on any parse -> serialize round trip.
    pub entry_version: u16,
    /// The six bytes following `entry_version` in the fixed 8-byte block
    /// (QuickTime's `revision_level` + `vendor`, ISO/IEC 14496-12:2026's
    /// `reserved[3]`). Written back verbatim so a real file round-trips.
    pub reserved_1: [u8; AUDIO_SAMPLE_ENTRY_RESERVED_LEN],
    pub data_reference_index: u16,
    pub channelcount: u16,
    pub samplesize: u16,
    /// The `pre_defined` + `reserved` pair (ISO/IEC 14496-12:2026 §12.2.3) — or, in
    /// a QuickTime sound description, `compression_ID` + `packet_size`.
    /// QuickTime's VBR marker is `compression_ID = -2` (`ff fe`), so this is
    /// not always zero and must survive a round trip.
    pub compression_id_and_packet_size: [u8; core::mem::size_of::<u32>()],
    pub samplerate: u32,
    /// Config and any extra child boxes (typically one `ddts`).
    pub quicktime: Option<QuickTimeSound>,
    pub config_boxes: Vec<OpaqueBox>,
    /// Bytes after the last complete child, kept verbatim.
    pub trailing: Vec<u8>,
}

impl Serialize for DtsSampleEntry {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        audio_sample_entry_serialized_len(&self.quicktime, &self.config_boxes, &self.trailing)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        serialize_audio_sample_entry(
            buf,
            AudioSampleEntryFields {
                fourcc: &self.codec_type,
                reserved: self.reserved,
                entry_version: self.entry_version,
                reserved_1: self.reserved_1,
                data_reference_index: self.data_reference_index,
                channelcount: self.channelcount,
                samplesize: self.samplesize,
                compression_id_and_packet_size: self.compression_id_and_packet_size,
                samplerate: self.samplerate,
                quicktime: &self.quicktime,
                config_boxes: &self.config_boxes,
                trailing: &self.trailing,
            },
        )
    }
}

impl<'a> Parse<'a> for DtsSampleEntry {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let compact = crate::box_types::compact_form(bytes)?;
        let bytes: &[u8] = &compact;
        need_in(bytes, BOX_HEADER_MIN_SIZE, "DtsSampleEntry")?;
        let mut codec_type = [0u8; BOX_TYPE_SIZE];
        codec_type.copy_from_slice(&bytes[BOX_TYPE_OFFSET..BOX_HEADER_MIN_SIZE]);
        let e = parse_audio_sample_entry(bytes, "DtsSampleEntry")?;
        Ok(Self {
            codec_type,
            reserved: e.reserved,
            entry_version: e.entry_version,
            reserved_1: e.reserved_1,
            data_reference_index: e.data_reference_index,
            channelcount: e.channelcount,
            samplesize: e.samplesize,
            compression_id_and_packet_size: e.compression_id_and_packet_size,
            samplerate: e.samplerate,
            quicktime: e.quicktime,
            config_boxes: e.config_boxes,
            trailing: e.trailing,
        })
    }
}

// ---------------------------------------------------------------------------
// MhaSampleEntry (mha1 / mha2 / mhm1 / mhm2) — ISO/IEC 23008-3 §20
// ---------------------------------------------------------------------------

/// MPEG-H 3D Audio sample entry (`mha1`, `mha2`, `mhm1`, `mhm2`) — ISO/IEC 23008-3 §20.
///
/// Same AudioSampleEntry fixed fields as [`Mp4aSampleEntry`], then an `mhaC` box
/// (mandatory for `mha1`/`mha2`; optional for `mhm1`/`mhm2`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MhaSampleEntry {
    /// The SampleEntry `reserved[6]` bytes (§8.5.2.2), written back verbatim.
    pub reserved: [u8; SAMPLE_ENTRY_RESERVED_LEN],
    /// The FourCC of this sample entry — one of `mha1`, `mha2`, `mhm1`, `mhm2`.
    pub codec_type: [u8; BOX_TYPE_SIZE],
    /// `entry_version`: 0 for a plain `AudioSampleEntry` (§12.2.3), 1 for an
    /// [`AudioSampleEntryV1`] (§12.2.3.2) whose real rate lives in a
    /// [`SamplingRateBox`] among `config_boxes`, or any other value a
    /// QuickTime sound description carried.
    ///
    /// Stored and written back **verbatim** — the field used to be re-derived
    /// as 0-or-1 on serialize, which silently rewrote a QuickTime v2 (`02 00`)
    /// entry as v0 on any parse -> serialize round trip.
    pub entry_version: u16,
    /// The six bytes following `entry_version` in the fixed 8-byte block
    /// (QuickTime's `revision_level` + `vendor`, ISO/IEC 14496-12:2026's
    /// `reserved[3]`). Written back verbatim so a real file round-trips.
    pub reserved_1: [u8; AUDIO_SAMPLE_ENTRY_RESERVED_LEN],
    pub data_reference_index: u16,
    pub channelcount: u16,
    pub samplesize: u16,
    /// The `pre_defined` + `reserved` pair (ISO/IEC 14496-12:2026 §12.2.3) — or, in
    /// a QuickTime sound description, `compression_ID` + `packet_size`.
    /// QuickTime's VBR marker is `compression_ID = -2` (`ff fe`), so this is
    /// not always zero and must survive a round trip.
    pub compression_id_and_packet_size: [u8; core::mem::size_of::<u32>()],
    pub samplerate: u32,
    /// Config and any extra child boxes (typically one `mhaC`).
    pub quicktime: Option<QuickTimeSound>,
    pub config_boxes: Vec<OpaqueBox>,
    /// Bytes after the last complete child, kept verbatim.
    pub trailing: Vec<u8>,
}

impl Serialize for MhaSampleEntry {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        audio_sample_entry_serialized_len(&self.quicktime, &self.config_boxes, &self.trailing)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        serialize_audio_sample_entry(
            buf,
            AudioSampleEntryFields {
                fourcc: &self.codec_type,
                reserved: self.reserved,
                entry_version: self.entry_version,
                reserved_1: self.reserved_1,
                data_reference_index: self.data_reference_index,
                channelcount: self.channelcount,
                samplesize: self.samplesize,
                compression_id_and_packet_size: self.compression_id_and_packet_size,
                samplerate: self.samplerate,
                quicktime: &self.quicktime,
                config_boxes: &self.config_boxes,
                trailing: &self.trailing,
            },
        )
    }
}

impl<'a> Parse<'a> for MhaSampleEntry {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let compact = crate::box_types::compact_form(bytes)?;
        let bytes: &[u8] = &compact;
        need_in(bytes, BOX_HEADER_MIN_SIZE, "MhaSampleEntry")?;
        let mut codec_type = [0u8; BOX_TYPE_SIZE];
        codec_type.copy_from_slice(&bytes[BOX_TYPE_OFFSET..BOX_HEADER_MIN_SIZE]);
        let e = parse_audio_sample_entry(bytes, "MhaSampleEntry")?;
        Ok(Self {
            codec_type,
            reserved: e.reserved,
            entry_version: e.entry_version,
            reserved_1: e.reserved_1,
            data_reference_index: e.data_reference_index,
            channelcount: e.channelcount,
            samplesize: e.samplesize,
            compression_id_and_packet_size: e.compression_id_and_packet_size,
            samplerate: e.samplerate,
            quicktime: e.quicktime,
            config_boxes: e.config_boxes,
            trailing: e.trailing,
        })
    }
}

// ---------------------------------------------------------------------------
// Shared audio sample entry parse/serialize helpers
// ---------------------------------------------------------------------------

/// Fixed bytes of an audio sample entry body after the box header (§12.2.3.2).
const AUDIO_SAMPLE_ENTRY_FIXED_LEN: usize = 28;

/// Offset of `data_reference_index` in an audio sample entry body (§12.2.3.2).
const AUDIO_SAMPLE_ENTRY_DATA_REFERENCE_INDEX_OFFSET: usize = 6;
/// Offset of `entry_version` in an audio sample entry body (§12.2.3.2).
const AUDIO_SAMPLE_ENTRY_ENTRY_VERSION_OFFSET: usize = 8;
/// Offset of `reserved[3]` in an audio sample entry body (§12.2.3.2).
const AUDIO_SAMPLE_ENTRY_RESERVED_OFFSET: usize = 10;
/// Offset of `channelcount` in an audio sample entry body (§12.2.3.2).
const AUDIO_SAMPLE_ENTRY_CHANNELCOUNT_OFFSET: usize = 16;
/// Offset of `samplesize` in an audio sample entry body (§12.2.3.2).
const AUDIO_SAMPLE_ENTRY_SAMPLESIZE_OFFSET: usize = 18;
/// Offset of `pre_defined`/`reserved` in an audio sample entry body (§12.2.3.2).
const AUDIO_SAMPLE_ENTRY_COMPRESSION_OFFSET: usize = 20;
/// Offset of `samplerate` in an audio sample entry body (§12.2.3.2).
const AUDIO_SAMPLE_ENTRY_SAMPLERATE_OFFSET: usize = 24;

/// Parse an AudioSampleEntry-derived box (28-byte fixed prefix + config boxes).
///
/// The fixed fields parsed out of an audio sample entry, plus its config
/// children.
///
/// `entry_version` is 0 for a plain `AudioSampleEntry`, 1 for an
/// `AudioSampleEntryV1` (ISO/IEC 14496-12:2026 §12.2.3.2)
/// or any other value a QuickTime sound description carried; every form shares
/// the 28-byte fixed layout. Every field is preserved verbatim, including the
/// ones this crate does not interpret (QuickTime's `revision_level`/`vendor`
/// and `compression_ID`/`packet_size`), so a real file round-trips
/// byte-exactly.
struct ParsedAudioSampleEntry {
    reserved: [u8; SAMPLE_ENTRY_RESERVED_LEN],
    entry_version: u16,
    reserved_1: [u8; AUDIO_SAMPLE_ENTRY_RESERVED_LEN],
    data_reference_index: u16,
    channelcount: u16,
    samplesize: u16,
    compression_id_and_packet_size: [u8; core::mem::size_of::<u32>()],
    samplerate: u32,
    quicktime: Option<QuickTimeSound>,
    config_boxes: Vec<OpaqueBox>,
    trailing: Vec<u8>,
}

fn parse_audio_sample_entry(bytes: &[u8], what: &'static str) -> Result<ParsedAudioSampleEntry> {
    let body = crate::box_types::box_body(bytes, AUDIO_SAMPLE_ENTRY_FIXED_LEN, what)?;
    let mut reserved = [0u8; SAMPLE_ENTRY_RESERVED_LEN];
    reserved.copy_from_slice(&body[..SAMPLE_ENTRY_RESERVED_LEN]);
    let entry_version = u16_at(body, AUDIO_SAMPLE_ENTRY_ENTRY_VERSION_OFFSET);
    let mut reserved_1 = [0u8; AUDIO_SAMPLE_ENTRY_RESERVED_LEN];
    reserved_1.copy_from_slice(
        &body[AUDIO_SAMPLE_ENTRY_RESERVED_OFFSET
            ..AUDIO_SAMPLE_ENTRY_RESERVED_OFFSET + AUDIO_SAMPLE_ENTRY_RESERVED_LEN],
    );
    let dri = u16_at(body, AUDIO_SAMPLE_ENTRY_DATA_REFERENCE_INDEX_OFFSET);
    let chan = u16_at(body, AUDIO_SAMPLE_ENTRY_CHANNELCOUNT_OFFSET);
    let samp_sz = u16_at(body, AUDIO_SAMPLE_ENTRY_SAMPLESIZE_OFFSET);
    let mut compression_id_and_packet_size = [0u8; core::mem::size_of::<u32>()];
    compression_id_and_packet_size.copy_from_slice(
        &body[AUDIO_SAMPLE_ENTRY_COMPRESSION_OFFSET
            ..AUDIO_SAMPLE_ENTRY_COMPRESSION_OFFSET + core::mem::size_of::<u32>()],
    );
    let sr = u32_at(body, AUDIO_SAMPLE_ENTRY_SAMPLERATE_OFFSET);
    let (quicktime, children_at) = crate::quicktime::sound::parse_extension(bytes, body)?;

    let (config_boxes, trailing) =
        crate::box_support::capture_children(body.get(children_at..).unwrap_or(&[])).into_parts();
    Ok(ParsedAudioSampleEntry {
        reserved,
        entry_version,
        reserved_1,
        data_reference_index: dri,
        channelcount: chan,
        samplesize: samp_sz,
        compression_id_and_packet_size,
        samplerate: sr,
        quicktime,
        config_boxes,
        trailing,
    })
}

impl<'a> Parse<'a> for Ac3SampleEntry {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let e = parse_audio_sample_entry(bytes, "ac-3")?;
        Ok(Self {
            reserved: e.reserved,
            entry_version: e.entry_version,
            reserved_1: e.reserved_1,
            data_reference_index: e.data_reference_index,
            channelcount: e.channelcount,
            samplesize: e.samplesize,
            compression_id_and_packet_size: e.compression_id_and_packet_size,
            samplerate: e.samplerate,
            quicktime: e.quicktime,
            config_boxes: e.config_boxes,
            trailing: e.trailing,
        })
    }
}

impl<'a> Parse<'a> for Ec3SampleEntry {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let e = parse_audio_sample_entry(bytes, "ec-3")?;
        Ok(Self {
            reserved: e.reserved,
            entry_version: e.entry_version,
            reserved_1: e.reserved_1,
            data_reference_index: e.data_reference_index,
            channelcount: e.channelcount,
            samplesize: e.samplesize,
            compression_id_and_packet_size: e.compression_id_and_packet_size,
            samplerate: e.samplerate,
            quicktime: e.quicktime,
            config_boxes: e.config_boxes,
            trailing: e.trailing,
        })
    }
}

fn audio_sample_entry_serialized_len(
    quicktime: &Option<QuickTimeSound>,
    config_boxes: &[OpaqueBox],
    trailing: &[u8],
) -> usize {
    let mut n = BOX_HEADER_MIN_SIZE + crate::quicktime::sound::fixed_len(quicktime);
    for c in config_boxes {
        n += c.serialized_len();
    }
    n += trailing.len();
    crate::box_types::box_len(n)
}

/// The shared fixed fields of an audio sample entry, grouped so the writer
/// stays within the argument-count lint.
struct AudioSampleEntryFields<'a> {
    fourcc: &'a [u8; BOX_TYPE_SIZE],
    reserved: [u8; SAMPLE_ENTRY_RESERVED_LEN],
    entry_version: u16,
    reserved_1: [u8; AUDIO_SAMPLE_ENTRY_RESERVED_LEN],
    data_reference_index: u16,
    channelcount: u16,
    samplesize: u16,
    compression_id_and_packet_size: [u8; core::mem::size_of::<u32>()],
    samplerate: u32,
    quicktime: &'a Option<QuickTimeSound>,
    config_boxes: &'a [OpaqueBox],
    trailing: &'a [u8],
}

fn serialize_audio_sample_entry(buf: &mut [u8], f: AudioSampleEntryFields<'_>) -> Result<usize> {
    let config_boxes = f.config_boxes;
    let need = audio_sample_entry_serialized_len(f.quicktime, config_boxes, f.trailing);
    need_out(buf, need)?;
    let mut c = 0usize;
    c += crate::box_types::write_box_header(buf, f.fourcc, need)?;
    // SampleEntry: `reserved[6]` (written from the parsed value: a QuickTime
    // writer may carry its own bytes there) + data_reference_index(2).
    let u16_len = core::mem::size_of::<u16>();
    buf[c..c + SAMPLE_ENTRY_RESERVED_LEN].copy_from_slice(&f.reserved);
    c += SAMPLE_ENTRY_RESERVED_LEN;
    buf[c..c + u16_len].copy_from_slice(&f.data_reference_index.to_be_bytes());
    c += u16_len;
    // The 8-byte block: `entry_version` (0 for a plain AudioSampleEntry
    // §12.2.3, 1 for an AudioSampleEntryV1 §12.2.3.2, or whatever a QuickTime
    // sound description carried) followed by 6 bytes that are
    // `reserved[3]`/`revision_level`+`vendor`. Both are written back verbatim:
    // re-deriving them rewrote a QuickTime entry as v0 on re-serialize.
    buf[c..c + u16_len].copy_from_slice(&f.entry_version.to_be_bytes());
    let reserved_at = c + u16_len;
    buf[reserved_at..reserved_at + AUDIO_SAMPLE_ENTRY_RESERVED_LEN].copy_from_slice(&f.reserved_1);
    c += u16_len + AUDIO_SAMPLE_ENTRY_RESERVED_LEN;
    match f.quicktime {
        Some(extension @ QuickTimeSound::V2(_)) => {
            c += crate::quicktime::sound::write_extension(&mut buf[c..], extension)?;
        }
        extension => {
            write_audio_fixed_fields(buf, &mut c, &f);
            if let Some(extension) = extension {
                c += crate::quicktime::sound::write_extension(&mut buf[c..], extension)?;
            }
        }
    }
    for cb in config_boxes {
        c += cb.serialize_into(&mut buf[c..])?;
    }
    buf[c..c + f.trailing.len()].copy_from_slice(f.trailing);
    c += f.trailing.len();
    Ok(c)
}

/// Write the 12 bytes of ISO `AudioSampleEntry` fields that precede the
/// version 1 extension or the children of a version 0 entry (§12.2.3).
fn write_audio_fixed_fields(buf: &mut [u8], c: &mut usize, f: &AudioSampleEntryFields<'_>) {
    let u16_len = core::mem::size_of::<u16>();
    let u32_len = core::mem::size_of::<u32>();
    buf[*c..*c + u16_len].copy_from_slice(&f.channelcount.to_be_bytes());
    *c += u16_len;
    buf[*c..*c + u16_len].copy_from_slice(&f.samplesize.to_be_bytes());
    *c += u16_len;
    // `pre_defined`(16) + `reserved`(16) — QuickTime's `compression_ID` +
    // `packet_size`. Written from the parsed value, not zeroed: QuickTime marks
    // VBR audio with `compression_ID = -2`, so zeroing would corrupt it.
    buf[*c..*c + u32_len].copy_from_slice(&f.compression_id_and_packet_size);
    *c += u32_len;
    buf[*c..*c + u32_len].copy_from_slice(&f.samplerate.to_be_bytes());
    *c += u32_len;
}

// ---------------------------------------------------------------------------
// Mp4aSampleEntry (existing — refactored to use helpers)
// ---------------------------------------------------------------------------

impl<'a> Parse<'a> for Mp4aSampleEntry {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let compact = crate::box_types::compact_form(bytes)?;
        let bytes: &[u8] = &compact;
        // bytes is a full box with 8-byte header; fields start at bytes[8]
        need_in(
            bytes,
            BOX_HEADER_MIN_SIZE + AUDIO_SAMPLE_ENTRY_FIXED_LEN,
            "mp4a",
        )?;
        let mut codec_type = [0u8; BOX_TYPE_SIZE];
        codec_type.copy_from_slice(&bytes[BOX_TYPE_OFFSET..BOX_HEADER_MIN_SIZE]);
        let e = parse_audio_sample_entry(bytes, "mp4a")?;
        Ok(Self {
            codec_type,
            reserved: e.reserved,
            entry_version: e.entry_version,
            reserved_1: e.reserved_1,
            data_reference_index: e.data_reference_index,
            channelcount: e.channelcount,
            samplesize: e.samplesize,
            compression_id_and_packet_size: e.compression_id_and_packet_size,
            samplerate: e.samplerate,
            quicktime: e.quicktime,
            config_boxes: e.config_boxes,
            trailing: e.trailing,
        })
    }
}

impl Serialize for Mp4aSampleEntry {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        audio_sample_entry_serialized_len(&self.quicktime, &self.config_boxes, &self.trailing)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        serialize_audio_sample_entry(
            buf,
            AudioSampleEntryFields {
                fourcc: &self.codec_type,
                reserved: self.reserved,
                entry_version: self.entry_version,
                reserved_1: self.reserved_1,
                data_reference_index: self.data_reference_index,
                channelcount: self.channelcount,
                samplesize: self.samplesize,
                compression_id_and_packet_size: self.compression_id_and_packet_size,
                samplerate: self.samplerate,
                quicktime: &self.quicktime,
                config_boxes: &self.config_boxes,
                trailing: &self.trailing,
            },
        )
    }
}

impl Mp4aSampleEntry {
    /// The `wave` atom among this entry's config children, when present.
    #[must_use]
    pub fn wave(&self) -> Option<crate::quicktime::WaveAtom> {
        crate::quicktime::sound::wave_of(&self.config_boxes)
    }

    /// The entry's elementary stream descriptor, whether it sits beside the
    /// fixed fields or inside a `wave` atom.
    #[must_use]
    pub fn esds(&self) -> Option<crate::mp4esds::EsdsBox> {
        crate::quicktime::sound::esds_of(&self.config_boxes)
    }
}

/// Describes one sample entry in an stsd box.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum SampleEntryVariant {
    Avc1(crate::sample_entries::AVCSampleEntry),
    Hevc1(crate::sample_entries::HEVCSampleEntry),
    /// AVC parameter set stream entry (`avcp`) — ISO/IEC 14496-15:2017 §5.4.3.1.
    /// It parses and serializes; the IR has no track kind for it, so a demux
    /// still reports it as an unsupported sample entry.
    AvcParameterSet(Box<crate::avc_parameter_set::AVCParameterSampleEntry>),
    /// H.266/VVC sample entry (`vvc1`/`vvi1`) — ISO/IEC 14496-15:2022 §11.3.3.
    /// The `codec_type` field on [`crate::sample_entries::VVCSampleEntry`]
    /// records which FourCC was parsed.
    Vvc(Box<crate::sample_entries::VVCSampleEntry>),
    /// MPEG visual sample entry (`mp4v`, esds-bearing) — used for MPEG-2 video
    /// (H.262). ISO/IEC 14496-14 §5.6.
    Mp4v(Box<crate::sample_entries::Mp4vSampleEntry>),
    Av01(Box<crate::av1::Av1SampleEntry>),
    Vp09(Box<crate::vp9::Vp9SampleEntry>),
    Mp4a(Box<Mp4aSampleEntry>),
    Ac3(Box<Ac3SampleEntry>),
    Ec3(Box<Ec3SampleEntry>),
    /// ISO/IEC 14496-30 TTML/IMSC XML subtitle sample entry (`stpp`).
    Stpp(Box<crate::subtitle_entries::XmlSubtitleSampleEntry>),
    /// ISO/IEC 14496-30 WebVTT subtitle sample entry (`wvtt`).
    Wvtt(Box<crate::subtitle_entries::WvttSampleEntry>),
    Ac4(Box<Ac4SampleEntry>),
    Opus(Box<OpusSampleEntry>),
    Flac(Box<FlacSampleEntry>),
    /// MPEG-H 3D Audio sample entry (`mha1`, `mha2`, `mhm1`, or `mhm2`) —
    /// ISO/IEC 23008-3 §20.  The `codec_type` field on [`MhaSampleEntry`]
    /// records which FourCC was parsed.
    Mha(Box<MhaSampleEntry>),
    /// DTS audio sample entry (`dtsc`, `dtsh`, `dtsl`, or `dtse`) —
    /// ETSI TS 102 114 §E.2.  The `codec_type` field on [`DtsSampleEntry`]
    /// records which FourCC was parsed.
    Dts(Box<DtsSampleEntry>),
    /// QuickTime timecode sample entry (`tmcd`).
    Tmcd(Box<crate::quicktime::TimecodeEntry>),
    /// QuickTime closed-caption sample entry (`c608`).
    ClosedCaption(Box<crate::subtitle_entries::ClosedCaptionSampleEntry>),
    /// XML timed-metadata sample entry (`metx`) —
    /// ISO/IEC 14496-12:2026 §12.3.3.
    Metx(Box<crate::meta_track::XmlMetaDataSampleEntry>),
    /// Text metadata (`mett`) or simple text (`stxt`) sample entry —
    /// §12.3.3, §12.5.3. The entry's `codec_type` records which.
    Text(Box<crate::meta_track::TextMetaDataSampleEntry>),
    /// URI metadata sample entry (`urim`) — §12.3.3.
    Urim(Box<crate::meta_track::UriMetaDataSampleEntry>),
    /// Boxed metadata sample entry (`mebx`) — §12.8.4.1.
    Mebx(Box<crate::meta_track::BoxedMetaDataSampleEntry>),
    /// Region-track sample entry (`rgan`) —
    /// ISO/IEC 23008-12:2025 11.4.2.2.2.
    Rgan(Box<crate::region::RegionSampleEntry>),
    /// A QuickTime-only sound sample entry (`lpcm`, `in24`, `sowt`, …).
    QuickTimeSound(Box<crate::quicktime::SoundEntry>),
    /// An RTP/SRTP/RTCP, MPEG-2 TS or FD hint sample entry (§9.1.2, §9.2.3,
    /// §9.3.3, §9.4.1–9.4.4).
    Hint(Box<crate::hint::HintSampleEntry>),
    Unknown(OpaqueBox),
}

impl SampleEntryVariant {
    /// The sample entry's unmodelled child boxes, when this is an audio entry
    /// that carries them.
    #[must_use]
    pub fn audio_config_boxes(&self) -> Option<&[OpaqueBox]> {
        match self {
            Self::Mp4a(e) => Some(&e.config_boxes),
            Self::Ac3(e) => Some(&e.config_boxes),
            Self::Ec3(e) => Some(&e.config_boxes),
            Self::Ac4(e) => Some(&e.config_boxes),
            Self::Opus(e) => Some(&e.config_boxes),
            Self::Flac(e) => Some(&e.config_boxes),
            Self::Mha(e) => Some(&e.config_boxes),
            Self::Dts(e) => Some(&e.config_boxes),
            Self::QuickTimeSound(e) => Some(&e.config_boxes),
            _ => None,
        }
    }

    /// The channel layout (`chnl`) this audio sample entry carries, when
    /// present.
    #[must_use]
    pub fn channel_layout(&self) -> Option<crate::audio_boxes::ChannelLayoutBox> {
        let child = self
            .audio_config_boxes()?
            .iter()
            .find(|child| child.box_type == crate::audio_boxes::CHNL)?;
        crate::audio_boxes::ChannelLayoutBox::parse_body(&child.data).ok()
    }

    /// The out-of-stream downmix instructions (`dmix`) this audio sample entry
    /// carries, parsed against the entry's base channel count.
    #[must_use]
    pub fn downmix_instructions(
        &self,
        base_channel_count: u8,
    ) -> Option<crate::audio_boxes::DownMixInstructionsBox> {
        let child = self
            .audio_config_boxes()?
            .iter()
            .find(|child| child.box_type == crate::audio_boxes::DMIX)?;
        crate::audio_boxes::DownMixInstructionsBox::parse_body_with_base_channel_count(
            &child.data,
            base_channel_count,
        )
        .ok()
    }

    /// The `stsd` version this entry requires.
    ///
    /// An `AudioSampleEntryV1` (`entry_version == 1`) is valid only inside an
    /// `stsd` with `version == 1` (ISO/IEC 14496-12:2026 §12.2.3.2).
    /// Every other entry uses the version-0 form (§8.5.2).
    pub fn required_stsd_version(&self) -> u8 {
        let entry_version = match self {
            SampleEntryVariant::Mp4a(e) => e.entry_version,
            SampleEntryVariant::Ac3(e) => e.entry_version,
            SampleEntryVariant::Ec3(e) => e.entry_version,
            SampleEntryVariant::Opus(e) => e.entry_version,
            SampleEntryVariant::Flac(e) => e.entry_version,
            SampleEntryVariant::Ac4(e) => e.entry_version,
            SampleEntryVariant::Mha(e) => e.entry_version,
            SampleEntryVariant::Dts(e) => e.entry_version,
            _ => 0,
        };
        if entry_version == AudioSampleEntryV1::ENTRY_VERSION {
            AudioSampleEntryV1::STSD_VERSION
        } else {
            0
        }
    }
}

impl Serialize for SampleEntryVariant {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        match self {
            SampleEntryVariant::Avc1(a) => a.serialized_len(),
            SampleEntryVariant::Hevc1(h) => h.serialized_len(),
            SampleEntryVariant::AvcParameterSet(p) => p.serialized_len(),
            SampleEntryVariant::Vvc(v) => v.serialized_len(),
            SampleEntryVariant::Mp4v(m) => m.serialized_len(),
            SampleEntryVariant::Av01(a) => a.serialized_len(),
            SampleEntryVariant::Vp09(v) => v.serialized_len(),
            SampleEntryVariant::Mp4a(m) => m.serialized_len(),
            SampleEntryVariant::Ac3(a) => a.serialized_len(),
            SampleEntryVariant::Ec3(e) => e.serialized_len(),
            SampleEntryVariant::Stpp(s) => s.serialized_len(),
            SampleEntryVariant::Wvtt(w) => w.serialized_len(),
            SampleEntryVariant::Ac4(a) => a.serialized_len(),
            SampleEntryVariant::Opus(o) => o.serialized_len(),
            SampleEntryVariant::Flac(f) => f.serialized_len(),
            SampleEntryVariant::Mha(m) => m.serialized_len(),
            SampleEntryVariant::Dts(d) => d.serialized_len(),
            SampleEntryVariant::Tmcd(e) => e.serialized_len(),
            SampleEntryVariant::ClosedCaption(e) => e.serialized_len(),
            SampleEntryVariant::Metx(e) => e.serialized_len(),
            SampleEntryVariant::Rgan(e) => e.serialized_len(),
            SampleEntryVariant::Text(e) => e.serialized_len(),
            SampleEntryVariant::Urim(e) => e.serialized_len(),
            SampleEntryVariant::Mebx(e) => e.serialized_len(),
            SampleEntryVariant::QuickTimeSound(e) => e.serialized_len(),
            SampleEntryVariant::Hint(e) => e.serialized_len(),
            SampleEntryVariant::Unknown(u) => u.serialized_len(),
        }
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            SampleEntryVariant::Avc1(a) => a.serialize_into(buf),
            SampleEntryVariant::Hevc1(h) => h.serialize_into(buf),
            SampleEntryVariant::AvcParameterSet(p) => p.serialize_into(buf),
            SampleEntryVariant::Vvc(v) => v.serialize_into(buf),
            SampleEntryVariant::Mp4v(m) => m.serialize_into(buf),
            SampleEntryVariant::Av01(a) => a.serialize_into(buf),
            SampleEntryVariant::Vp09(v) => v.serialize_into(buf),
            SampleEntryVariant::Mp4a(m) => m.serialize_into(buf),
            SampleEntryVariant::Ac3(a) => a.serialize_into(buf),
            SampleEntryVariant::Ec3(e) => e.serialize_into(buf),
            SampleEntryVariant::Stpp(s) => s.serialize_into(buf),
            SampleEntryVariant::Wvtt(w) => w.serialize_into(buf),
            SampleEntryVariant::Ac4(a) => a.serialize_into(buf),
            SampleEntryVariant::Opus(o) => o.serialize_into(buf),
            SampleEntryVariant::Flac(f) => f.serialize_into(buf),
            SampleEntryVariant::Mha(m) => m.serialize_into(buf),
            SampleEntryVariant::Dts(d) => d.serialize_into(buf),
            SampleEntryVariant::Tmcd(e) => e.serialize_into(buf),
            SampleEntryVariant::ClosedCaption(e) => e.serialize_into(buf),
            SampleEntryVariant::Metx(e) => e.serialize_into(buf),
            SampleEntryVariant::Rgan(e) => e.serialize_into(buf),
            SampleEntryVariant::Text(e) => e.serialize_into(buf),
            SampleEntryVariant::Urim(e) => e.serialize_into(buf),
            SampleEntryVariant::Mebx(e) => e.serialize_into(buf),
            SampleEntryVariant::QuickTimeSound(e) => e.serialize_into(buf),
            SampleEntryVariant::Hint(e) => e.serialize_into(buf),
            SampleEntryVariant::Unknown(u) => Ok(u.serialize_into(buf)?),
        }
    }
}

/// Four-CC of the Sample Description Box (`stsd`, ISO/IEC 14496-12:2026 §8.5.2).
pub const STSD: [u8; 4] = *b"stsd";

/// Sample Description Box (`stsd`) — §8.5.2.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SampleDescriptionBox {
    pub version: u8,
    pub flags: u32,
    pub entries: Vec<SampleEntryVariant>,
}

/// Parse one `stsd` entry's bytes into its typed variant.
///
/// The permissive reader uses this to tell a known four-CC that fails to parse
/// from one this crate has no typed form for.
/// Parse one sample entry's box bytes into its typed variant (§8.5.2,
/// ISO/IEC 14496-15:2017 sample entries).
pub fn sample_entry_variant(box_bytes: &[u8]) -> Result<SampleEntryVariant> {
    // A typed entry's serialiser writes the compact header form. An entry the
    // source stated with `size == 0` (§4.2) cannot be represented that way, so
    // it stays opaque and keeps its own header.
    if let Ok(header) = crate::box_types::BoxHeader::parse(box_bytes) {
        if header.size == 0 {
            let mut opaque = OpaqueBox::new(
                header.box_type.0,
                box_bytes
                    .get(header.header_size()..)
                    .unwrap_or_default()
                    .to_vec(),
            );
            opaque.to_end = true;
            return Ok(SampleEntryVariant::Unknown(opaque));
        }
        if usize::try_from(header.size)
            .ok()
            .is_some_and(|size| size > box_bytes.len())
        {
            return Err(Error::BufferTooShort(
                broadcast_core::error::BufferTooShort {
                    need: usize::try_from(header.size).unwrap_or(usize::MAX),
                    have: box_bytes.len(),
                    what: "sample entry",
                },
            ));
        }
    }
    let codec = &box_bytes[BOX_TYPE_OFFSET..BOX_HEADER_MIN_SIZE];
    Ok(match codec {
        b"avc1" | b"avc3" | b"avc2" | b"avc4" => SampleEntryVariant::Avc1(
            crate::sample_entries::AVCSampleEntry::bare_parse(box_bytes)?,
        ),
        b"avcp" => SampleEntryVariant::AvcParameterSet(Box::new(
            crate::avc_parameter_set::AVCParameterSampleEntry::bare_parse(box_bytes)?,
        )),
        b"hvc1" | b"hev1" | b"dvh1" | b"dvhe" => SampleEntryVariant::Hevc1(
            crate::sample_entries::HEVCSampleEntry::bare_parse(box_bytes)?,
        ),
        b"vvc1" | b"vvi1" => SampleEntryVariant::Vvc(Box::new(
            crate::sample_entries::VVCSampleEntry::bare_parse(box_bytes)?,
        )),
        b"mp4v" => SampleEntryVariant::Mp4v(Box::new(
            crate::sample_entries::Mp4vSampleEntry::bare_parse(box_bytes)?,
        )),
        b"mp4a" | b"enca" => SampleEntryVariant::Mp4a(Box::new(Mp4aSampleEntry::parse(box_bytes)?)),
        b"ac-3" => SampleEntryVariant::Ac3(Box::new(Ac3SampleEntry::parse(box_bytes)?)),
        b"ec-3" => SampleEntryVariant::Ec3(Box::new(Ec3SampleEntry::parse(box_bytes)?)),
        b"stpp" => SampleEntryVariant::Stpp(Box::new(
            crate::subtitle_entries::XmlSubtitleSampleEntry::bare_parse(box_bytes)?,
        )),
        b"wvtt" => SampleEntryVariant::Wvtt(Box::new(
            crate::subtitle_entries::WvttSampleEntry::bare_parse(box_bytes)?,
        )),
        b"av01" => SampleEntryVariant::Av01(Box::new(crate::av1::Av1SampleEntry::parse_entry(
            box_bytes,
        )?)),
        b"vp09" => SampleEntryVariant::Vp09(Box::new(crate::vp9::Vp9SampleEntry::parse_entry(
            box_bytes,
        )?)),
        b"ac-4" => SampleEntryVariant::Ac4(Box::new(Ac4SampleEntry::parse(box_bytes)?)),
        b"Opus" => SampleEntryVariant::Opus(Box::new(OpusSampleEntry::parse(box_bytes)?)),
        b"fLaC" => SampleEntryVariant::Flac(Box::new(FlacSampleEntry::parse(box_bytes)?)),
        b"mha1" | b"mha2" | b"mhm1" | b"mhm2" => {
            SampleEntryVariant::Mha(Box::new(MhaSampleEntry::parse(box_bytes)?))
        }
        b"dtsc" | b"dtsh" | b"dtsl" | b"dtse" => {
            SampleEntryVariant::Dts(Box::new(DtsSampleEntry::parse(box_bytes)?))
        }
        b"tmcd" => {
            SampleEntryVariant::Tmcd(Box::new(crate::quicktime::TimecodeEntry::parse(box_bytes)?))
        }
        b"c608" => SampleEntryVariant::ClosedCaption(Box::new(
            crate::subtitle_entries::ClosedCaptionSampleEntry::parse_box(box_bytes)?,
        )),
        b"metx" => SampleEntryVariant::Metx(Box::new(
            crate::meta_track::XmlMetaDataSampleEntry::parse_box(box_bytes)?,
        )),
        b"mett" | b"stxt" => SampleEntryVariant::Text(Box::new(
            crate::meta_track::TextMetaDataSampleEntry::parse_box(box_bytes)?,
        )),
        b"urim" => SampleEntryVariant::Urim(Box::new(
            crate::meta_track::UriMetaDataSampleEntry::parse_box(box_bytes)?,
        )),
        b"mebx" => SampleEntryVariant::Mebx(Box::new(
            crate::meta_track::BoxedMetaDataSampleEntry::parse_box(box_bytes)?,
        )),
        b"rgan" => SampleEntryVariant::Rgan(Box::new(crate::region::RegionSampleEntry::parse_box(
            box_bytes,
        )?)),
        b"lpcm" | b"in24" | b"in32" | b"sowt" | b"twos" | b"raw " | b"fl32" | b"fl64" | b"ima4"
        | b"MAC3 " | b"MAC6 " | b"ulaw" | b"alaw" | b"NONE" => SampleEntryVariant::QuickTimeSound(
            Box::new(crate::quicktime::SoundEntry::parse(box_bytes)?),
        ),
        b"rtp " | b"srtp" | b"rrtp" | b"rsrp" | b"rtcp" | b"stcp" | b"prtp" | b"rm2t" | b"sm2t"
        | b"pm2t" | b"fdp " => SampleEntryVariant::Hint(Box::new(
            crate::hint::HintSampleEntry::parse_box(box_bytes)?,
        )),
        _ => {
            let mut c4 = [0u8; BOX_TYPE_SIZE];
            c4.copy_from_slice(&codec[..BOX_TYPE_SIZE.min(codec.len())]);
            let item = crate::box_types::parse_box(box_bytes)?.0;
            let mut opaque = OpaqueBox::new(c4, item.payload().to_vec());
            opaque.to_end = item.to_end();
            opaque.largesize = item.has_largesize();
            SampleEntryVariant::Unknown(opaque)
        }
    })
}

impl<'a> Parse<'a> for SampleDescriptionBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let compact = crate::box_types::compact_form(bytes)?;
        let bytes: &[u8] = &compact;
        need_in(bytes, FLAT_TABLE_HEADER_LEN, "stsd")?;
        let ver = bytes[FLAT_TABLE_VERSION_OFFSET];
        let flags = u24_at(bytes, FLAT_TABLE_FLAGS_OFFSET);
        let count = u32_at(bytes, FLAT_TABLE_COUNT_OFFSET) as usize;
        check_entry_count(
            bytes.len().saturating_sub(FLAT_TABLE_HEADER_LEN),
            BOX_HEADER_MIN_SIZE,
            count,
            "SampleDescriptionBox.entry_count",
        )?;
        let mut entries = Vec::with_capacity(bounded_entry_count(
            bytes.len().saturating_sub(FLAT_TABLE_HEADER_LEN),
            BOX_HEADER_MIN_SIZE,
            count,
        ));
        for item in BoxIter::clamped(&bytes[FLAT_TABLE_HEADER_LEN..]).take(count) {
            let Ok(item) = item else {
                break;
            };
            let box_bytes = item.bytes;
            let entry = sample_entry_variant(box_bytes)?;
            entries.push(entry);
        }
        Ok(Self {
            version: ver,
            flags,
            entries,
        })
    }
}

impl Serialize for SampleDescriptionBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>();
        for e in &self.entries {
            n += e.serialized_len();
        }
        crate::box_types::box_len(n)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = 0usize;
        c += crate::box_types::write_full_box_header(
            buf,
            b"stsd",
            need,
            self.version,
            self.flags,
            false,
        )?;
        let entry_count = broadcast_core::len::fit_u32(self.entries.len(), "entry_count")?;
        let u32_len = core::mem::size_of::<u32>();
        buf[c..c + u32_len].copy_from_slice(&entry_count.to_be_bytes());
        c += u32_len;
        for e in &self.entries {
            c += e.serialize_into(&mut buf[c..])?;
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// stbl children that we preserve as opaque (stss, sgpd, sbgp)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Helper: parse a list of child boxes from container body and return typed
// variants via an enum.  Used by the container types below.
// ---------------------------------------------------------------------------

/// A single child within an stbl container.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum StblChild {
    Stsd(SampleDescriptionBox),
    Stts(crate::timing::TimeToSampleBox),
    Ctts(crate::timing::CompositionOffsetBox),
    Stsc(SampleToChunkBox),
    Stsz(SampleSizeBox),
    Stz2(CompactSampleSizeBox),
    Sdtp(SampleDependencyTypeBox),
    Stco(ChunkOffsetBox),
    Co64(ChunkLargeOffsetBox),
    Stss(SyncSampleBox),
    Stdp(crate::sample_tables_extra::DegradationPriorityBox),
    Padb(crate::sample_tables_extra::PaddingBitsBox),
    Csgp(crate::sample_groups::CompactSampleToGroupBox),
    Opaque(Vec<u8>),
}

fn parse_stbl_children(body: &[u8]) -> Result<Vec<StblChild>> {
    let mut children = Vec::new();
    for child in BoxIter::new(body) {
        let child = child?;
        let box_bytes = child.bytes;
        let boxtype = child.four_cc();
        children.push(match &boxtype {
            // A `stsd` that fails to parse (e.g. an `avcC` with a malformed
            // trailer) is kept as raw bytes rather than defaulted to an empty
            // box — an empty `entries: Vec::new()` would later present to
            // `track_spec_from_trak` as "no stsd entry" and discard the real
            // parse error, hiding *why* the whole track is about to be
            // dropped (issue #952). The moov as a whole must still parse
            // (media-plane "lenient but loud": one broken optional field
            // costs that field, not the file), so `track_spec_from_trak`
            // re-parses these raw bytes to recover the real error for
            // `Media::skipped`.
            b"stsd" => match SampleDescriptionBox::parse(box_bytes) {
                Ok(stsd) => StblChild::Stsd(stsd),
                Err(_) => StblChild::Opaque(box_bytes.to_vec()),
            },
            // Same treatment as the `stsd` arm above (issue #952): a box that
            // fails to parse is kept as raw bytes, not defaulted to an empty
            // typed box. `TimeToSampleBox { entries: Vec::new(), .. }` for a
            // malformed `stts` used to claim (falsely) that the track has
            // *zero* sample durations — every real duration silently
            // discarded, with nothing surfacing in `Media::skipped` — rather
            // than the truth, which is "this box didn't parse". Consumers
            // (`progressive_demux::find_stbl_child`) re-parse `Opaque` bytes
            // whose four-CC matches what they're looking for, recovering the
            // real error instead of silently treating a corrupt table as
            // absent or empty (audit finding #3).
            b"stts" => match crate::timing::TimeToSampleBox::parse(box_bytes) {
                Ok(b) => StblChild::Stts(b),
                Err(_) => StblChild::Opaque(box_bytes.to_vec()),
            },
            b"ctts" => match crate::timing::CompositionOffsetBox::parse(box_bytes) {
                Ok(b) => StblChild::Ctts(b),
                Err(_) => StblChild::Opaque(box_bytes.to_vec()),
            },
            b"stsc" => match SampleToChunkBox::parse(box_bytes) {
                Ok(b) => StblChild::Stsc(b),
                Err(_) => StblChild::Opaque(box_bytes.to_vec()),
            },
            b"stsz" => match SampleSizeBox::parse(box_bytes) {
                Ok(b) => StblChild::Stsz(b),
                Err(_) => StblChild::Opaque(box_bytes.to_vec()),
            },
            b"stz2" => match CompactSampleSizeBox::parse(box_bytes) {
                Ok(b) => StblChild::Stz2(b),
                Err(_) => StblChild::Opaque(box_bytes.to_vec()),
            },
            b"sdtp" => match SampleDependencyTypeBox::parse(box_bytes) {
                Ok(b) => StblChild::Sdtp(b),
                Err(_) => StblChild::Opaque(box_bytes.to_vec()),
            },
            b"stco" => match ChunkOffsetBox::parse(box_bytes) {
                Ok(b) => StblChild::Stco(b),
                Err(_) => StblChild::Opaque(box_bytes.to_vec()),
            },
            b"co64" => match ChunkLargeOffsetBox::parse(box_bytes) {
                Ok(b) => StblChild::Co64(b),
                Err(_) => StblChild::Opaque(box_bytes.to_vec()),
            },
            b"stss" => match SyncSampleBox::parse(box_bytes) {
                Ok(b) => StblChild::Stss(b),
                Err(_) => StblChild::Opaque(box_bytes.to_vec()),
            },
            b"stdp" => match crate::sample_tables_extra::DegradationPriorityBox::parse(box_bytes) {
                Ok(b) => StblChild::Stdp(b),
                Err(_) => StblChild::Opaque(box_bytes.to_vec()),
            },
            b"padb" => match crate::sample_tables_extra::PaddingBitsBox::parse(box_bytes) {
                Ok(b) => StblChild::Padb(b),
                Err(_) => StblChild::Opaque(box_bytes.to_vec()),
            },
            b"csgp" => match crate::sample_groups::CompactSampleToGroupBox::parse(box_bytes) {
                Ok(b) => StblChild::Csgp(b),
                Err(_) => StblChild::Opaque(box_bytes.to_vec()),
            },
            _ => StblChild::Opaque(box_bytes.to_vec()),
        });
    }
    Ok(children)
}

fn serialize_stbl_children(children: &[StblChild], buf: &mut [u8], off: &mut usize) -> Result<()> {
    for child in children {
        match child {
            StblChild::Stsd(b) => *off += b.serialize_into(&mut buf[*off..])?,
            StblChild::Stts(b) => *off += b.serialize_into(&mut buf[*off..])?,
            StblChild::Ctts(b) => *off += b.serialize_into(&mut buf[*off..])?,
            StblChild::Stsc(b) => *off += b.serialize_into(&mut buf[*off..])?,
            StblChild::Stsz(b) => *off += b.serialize_into(&mut buf[*off..])?,
            StblChild::Stz2(b) => *off += b.serialize_into(&mut buf[*off..])?,
            StblChild::Sdtp(b) => *off += b.serialize_into(&mut buf[*off..])?,
            StblChild::Stco(b) => *off += b.serialize_into(&mut buf[*off..])?,
            StblChild::Co64(b) => *off += b.serialize_into(&mut buf[*off..])?,
            StblChild::Stss(b) => *off += b.serialize_into(&mut buf[*off..])?,
            StblChild::Stdp(b) => *off += b.serialize_into(&mut buf[*off..])?,
            StblChild::Padb(b) => *off += b.serialize_into(&mut buf[*off..])?,
            StblChild::Csgp(b) => *off += b.serialize_into(&mut buf[*off..])?,
            StblChild::Opaque(d) => {
                let len = d.len();
                buf[*off..*off + len].copy_from_slice(d);
                *off += len;
            }
        }
    }
    Ok(())
}

fn stbl_children_len(children: &[StblChild]) -> usize {
    let mut n = 0;
    for child in children {
        n += match child {
            StblChild::Stsd(b) => b.serialized_len(),
            StblChild::Stts(b) => b.serialized_len(),
            StblChild::Ctts(b) => b.serialized_len(),
            StblChild::Stsc(b) => b.serialized_len(),
            StblChild::Stsz(b) => b.serialized_len(),
            StblChild::Stz2(b) => b.serialized_len(),
            StblChild::Sdtp(b) => b.serialized_len(),
            StblChild::Stco(b) => b.serialized_len(),
            StblChild::Co64(b) => b.serialized_len(),
            StblChild::Stss(b) => b.serialized_len(),
            StblChild::Stdp(b) => b.serialized_len(),
            StblChild::Padb(b) => b.serialized_len(),
            StblChild::Csgp(b) => b.serialized_len(),
            StblChild::Opaque(d) => d.len(),
        };
    }
    n
}

// ---------------------------------------------------------------------------
// SampleTableBox — stbl (container)
// ---------------------------------------------------------------------------

/// Four-CC of the Sample Table Box (`stbl`, ISO/IEC 14496-12:2026 §8.5.1).
pub const STBL: [u8; 4] = *b"stbl";

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SampleTableBox {
    pub children: Vec<StblChild>,
}

impl<'a> Parse<'a> for SampleTableBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        // Expect full box bytes (size+type header then body)

        Ok(Self {
            children: parse_stbl_children(parse_box(bytes)?.0.body)?,
        })
    }
}

impl Serialize for SampleTableBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        crate::box_types::box_len(BOX_HEADER_MIN_SIZE + stbl_children_len(&self.children))
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = 0usize;
        c += crate::box_types::write_box_header(buf, b"stbl", need)?;
        serialize_stbl_children(&self.children, buf, &mut c)?;
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// DataInformationBox — dinf (container: dref)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DataInformationBox {
    pub dref: Option<DataReferenceBox>,
    pub opaque: Vec<OpaqueBox>,
    /// The wire order of this container's children (audit r05-W12).
    pub order: ChildOrder,
}

impl DataInformationBox {
    /// Build a `dinf` from its typed parts, children in the conventional
    /// `dref`-then-opaque order.
    pub fn new(dref: Option<DataReferenceBox>, opaque: Vec<OpaqueBox>) -> Self {
        Self {
            dref,
            opaque,
            order: ChildOrder::default(),
        }
    }

    /// The QuickTime base media information header (`gmhd`), when this
    /// container reaches one.
    #[must_use]
    pub fn gmhd(&self) -> Option<crate::quicktime::gmhd::GmhdAtom> {
        self.opaque
            .iter()
            .find(|b| b.box_type == crate::quicktime::gmhd::GMHD)
            .and_then(|b| crate::quicktime::gmhd::GmhdAtom::parse_body(&b.data).ok())
    }

    /// The container's typed children, in declaration order — the input
    /// [`ChildOrder::resolve`] maps a wire order onto.
    fn typed_children(&self) -> Vec<&dyn SerializeBox<Error = crate::Error>> {
        let mut out: Vec<&dyn SerializeBox<Error = crate::Error>> = Vec::with_capacity(1);
        if let Some(ref d) = self.dref {
            out.push(d);
        }
        out
    }

    /// This container's typed children in declaration order, as four-CCs.
    fn typed_four_ccs(&self) -> Vec<&'static [u8; BOX_TYPE_SIZE]> {
        let mut out = Vec::with_capacity(1);
        if self.dref.is_some() {
            out.push(b"dref" as &'static [u8; BOX_TYPE_SIZE]);
        }
        out
    }

    /// The children to serialize, in wire order.
    fn child_order(&self) -> Vec<Option<usize>> {
        self.order.resolve(&self.typed_four_ccs())
    }
}

impl<'a> Parse<'a> for DataInformationBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let mut dref = None;
        let mut opaque = Vec::new();
        let mut order = ChildOrder::default();
        for child in parse_box(bytes)?.0.children() {
            let child = child?;
            if child.header.box_type.is(b"dref") {
                order.push(child.four_cc());
                dref = Some(DataReferenceBox::parse(child.bytes)?);
            } else {
                order.push_opaque(child.four_cc());
                opaque.push(OpaqueBox {
                    box_type: child.four_cc(),
                    data: child.payload().to_vec(),
                    to_end: child.to_end(),
                    largesize: child.has_largesize(),
                });
            }
        }
        Ok(Self {
            dref,
            opaque,
            order,
        })
    }
}

impl Serialize for DataInformationBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HEADER_MIN_SIZE;
        for b in self.typed_children() {
            n += b.box_len();
        }
        for o in &self.opaque {
            n += o.serialized_len();
        }
        crate::box_types::box_len(n)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        serialize_container(
            b"dinf",
            self.serialized_len(),
            &self.child_order(),
            &self.typed_children(),
            &self.opaque,
            buf,
        )
    }
}

// ---------------------------------------------------------------------------
// MediaInformationBox — minf (container: vmhd/smhd, dinf, stbl)
// ---------------------------------------------------------------------------

/// Four-CC of the Media Information Box (`minf`, ISO/IEC 14496-12:2026 §8.4.4).
pub const MINF: [u8; 4] = *b"minf";

/// Typed child slots in a `minf` (§8.4.4).
const MINF_TYPED_CHILDREN_MAX: usize = 9;

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MediaInformationBox {
    pub vmhd: Option<VideoMediaHeaderBox>,
    pub smhd: Option<SoundMediaHeaderBox>,
    /// The null media header (`nmhd`, §8.4.5.2), when present.
    pub nmhd: Option<NullMediaHeaderBox>,
    /// The hint media header (`hmhd`, §12.4.3.1), when present.
    pub hmhd: Option<HintMediaHeaderBox>,
    /// The volumetric visual media header (`vvhd`, §12.9.2), when present.
    pub vvhd: Option<VolumetricVisualMediaHeaderBox>,
    /// The subtitle media header (`sthd`, §12.6.2), when present.
    pub sthd: Option<SubtitleMediaHeaderBox>,
    /// The QuickTime base media information header (`gmhd`), when present.
    pub gmhd: Option<crate::quicktime::gmhd::GmhdAtom>,
    pub dinf: Option<DataInformationBox>,
    pub stbl: Option<SampleTableBox>,
    pub opaque: Vec<OpaqueBox>,
    /// The wire order of this container's children (audit r05-W12).
    pub order: ChildOrder,
}

impl MediaInformationBox {
    /// Build a `minf` from its typed parts, children in the conventional
    /// sequence (media header, `dinf`, `stbl`, then opaque).
    pub fn new(
        vmhd: Option<VideoMediaHeaderBox>,
        smhd: Option<SoundMediaHeaderBox>,
        dinf: Option<DataInformationBox>,
        stbl: Option<SampleTableBox>,
        opaque: Vec<OpaqueBox>,
    ) -> Self {
        Self {
            vmhd,
            smhd,
            nmhd: None,
            hmhd: None,
            vvhd: None,
            sthd: None,
            gmhd: None,
            dinf,
            stbl,
            opaque,
            order: ChildOrder::default(),
        }
    }
}

impl MediaInformationBox {
    /// Install an opaque media header (`sthd` / `nmhd`, which this crate does
    /// not model) as the **first** `minf` child, ahead of `dinf`/`stbl` —
    /// ISO/IEC 14496-12:2026 §7.3.4 requires the media header first, and the
    /// default "typed first, opaque last" order would put it after `stbl`.
    pub fn set_leading_opaque_header(&mut self, header: OpaqueBox) {
        let mut order = ChildOrder::default();
        order.push_opaque(header.box_type);
        if self.vmhd.is_some() {
            order.push(*b"vmhd");
        }
        if self.smhd.is_some() {
            order.push(*b"smhd");
        }
        if self.nmhd.is_some() {
            order.push(*b"nmhd");
        }
        if self.hmhd.is_some() {
            order.push(*b"hmhd");
        }
        if self.vvhd.is_some() {
            order.push(*b"vvhd");
        }
        if self.sthd.is_some() {
            order.push(*b"sthd");
        }
        if self.gmhd.is_some() {
            order.push(*b"gmhd");
        }
        if self.dinf.is_some() {
            order.push(*b"dinf");
        }
        if self.stbl.is_some() {
            order.push(*b"stbl");
        }
        self.opaque.insert(0, header);
        self.order = order;
    }

    /// The QuickTime base media information header (`gmhd`), when this `minf`
    /// carries one.
    #[must_use]
    pub fn gmhd(&self) -> Option<crate::quicktime::gmhd::GmhdAtom> {
        self.gmhd.clone()
    }

    /// The media information header this `minf` carries (§8.4.5), when the
    /// crate models its four-CC.
    #[must_use]
    pub fn media_header(&self) -> Option<MediaInformationHeader> {
        if let Some(b) = &self.vmhd {
            return Some(MediaInformationHeader::Video(b.clone()));
        }
        if let Some(b) = &self.smhd {
            return Some(MediaInformationHeader::Sound(b.clone()));
        }
        if let Some(b) = &self.nmhd {
            return Some(MediaInformationHeader::Null(b.clone()));
        }
        if let Some(b) = &self.hmhd {
            return Some(MediaInformationHeader::Hint(b.clone()));
        }
        if let Some(b) = &self.vvhd {
            return Some(MediaInformationHeader::Volumetric(b.clone()));
        }
        self.sthd
            .as_ref()
            .map(|b| MediaInformationHeader::Subtitle(b.clone()))
    }

    /// The container's typed children, in declaration order — the input
    /// [`ChildOrder::resolve`] maps a wire order onto.
    fn typed_children(&self) -> Vec<&dyn SerializeBox<Error = crate::Error>> {
        let mut out: Vec<&dyn SerializeBox<Error = crate::Error>> =
            Vec::with_capacity(MINF_TYPED_CHILDREN_MAX);
        if let Some(ref b) = self.vmhd {
            out.push(b);
        }
        if let Some(ref b) = self.smhd {
            out.push(b);
        }
        if let Some(ref b) = self.nmhd {
            out.push(b);
        }
        if let Some(ref b) = self.hmhd {
            out.push(b);
        }
        if let Some(ref b) = self.vvhd {
            out.push(b);
        }
        if let Some(ref b) = self.sthd {
            out.push(b);
        }
        if let Some(ref b) = self.gmhd {
            out.push(b);
        }
        if let Some(ref b) = self.dinf {
            out.push(b);
        }
        if let Some(ref b) = self.stbl {
            out.push(b);
        }
        out
    }

    /// This container's typed children in declaration order, as four-CCs.
    fn typed_four_ccs(&self) -> Vec<&'static [u8; BOX_TYPE_SIZE]> {
        let mut out = Vec::with_capacity(MINF_TYPED_CHILDREN_MAX);
        if self.vmhd.is_some() {
            out.push(b"vmhd" as &'static [u8; BOX_TYPE_SIZE]);
        }
        if self.smhd.is_some() {
            out.push(b"smhd" as &'static [u8; BOX_TYPE_SIZE]);
        }
        if self.nmhd.is_some() {
            out.push(b"nmhd" as &'static [u8; BOX_TYPE_SIZE]);
        }
        if self.hmhd.is_some() {
            out.push(b"hmhd" as &'static [u8; BOX_TYPE_SIZE]);
        }
        if self.vvhd.is_some() {
            out.push(b"vvhd" as &'static [u8; BOX_TYPE_SIZE]);
        }
        if self.sthd.is_some() {
            out.push(b"sthd" as &'static [u8; BOX_TYPE_SIZE]);
        }
        if self.gmhd.is_some() {
            out.push(b"gmhd" as &'static [u8; BOX_TYPE_SIZE]);
        }
        if self.dinf.is_some() {
            out.push(b"dinf" as &'static [u8; BOX_TYPE_SIZE]);
        }
        if self.stbl.is_some() {
            out.push(b"stbl" as &'static [u8; BOX_TYPE_SIZE]);
        }
        out
    }

    fn child_order(&self) -> Vec<Option<usize>> {
        self.order.resolve(&self.typed_four_ccs())
    }
}

impl<'a> Parse<'a> for MediaInformationBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let mut vmhd = None;
        let mut smhd = None;
        let mut nmhd = None;
        let mut sthd = None;
        let mut hmhd = None;
        let mut vvhd = None;
        let mut gmhd = None;
        let mut dinf = None;
        let mut stbl = None;
        let mut opaque = Vec::new();
        let mut order = ChildOrder::default();
        for child in parse_box(bytes)?.0.children() {
            let child = child?;
            match &child.four_cc() {
                b"vmhd" => {
                    order.push(child.four_cc());
                    vmhd = Some(VideoMediaHeaderBox::parse(child.bytes)?);
                }
                b"smhd" => {
                    order.push(child.four_cc());
                    smhd = Some(SoundMediaHeaderBox::parse(child.bytes)?);
                }
                b"nmhd" => {
                    order.push(child.four_cc());
                    nmhd = Some(NullMediaHeaderBox::parse(child.bytes)?);
                }
                b"hmhd" => {
                    order.push(child.four_cc());
                    hmhd = Some(HintMediaHeaderBox::parse(child.bytes)?);
                }
                b"vvhd" => {
                    order.push(child.four_cc());
                    vvhd = Some(VolumetricVisualMediaHeaderBox::parse(child.bytes)?);
                }
                b"sthd" => {
                    order.push(child.four_cc());
                    sthd = Some(SubtitleMediaHeaderBox::parse(child.bytes)?);
                }
                b"gmhd" => {
                    order.push(child.four_cc());
                    gmhd = Some(crate::quicktime::gmhd::GmhdAtom::parse_box(child.bytes)?);
                }
                b"dinf" => {
                    order.push(child.four_cc());
                    dinf = Some(DataInformationBox::parse(child.bytes)?);
                }
                b"stbl" => {
                    order.push(child.four_cc());
                    stbl = Some(SampleTableBox::parse(child.bytes)?);
                }
                // A subtitle track's `sthd` and a data track's `nmhd` are not
                // modelled (no fields to carry — §12.6.2/§8.4.5.2 are both
                // headerless FullBoxes), but they are preserved *in place*:
                // §7.3.4 puts a media header first in `minf`, and the old
                // "typed first, opaque last" serializer moved them after
                // `stbl`, where strict readers reject them (audit r05-W12).
                _ => {
                    order.push_opaque(child.four_cc());
                    opaque.push(OpaqueBox {
                        box_type: child.four_cc(),
                        data: child.payload().to_vec(),
                        to_end: child.to_end(),
                        largesize: child.has_largesize(),
                    });
                }
            }
        }
        Ok(Self {
            vmhd,
            smhd,
            nmhd,
            hmhd,
            vvhd,
            sthd,
            gmhd,
            dinf,
            stbl,
            opaque,
            order,
        })
    }
}

impl Serialize for MediaInformationBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HEADER_MIN_SIZE;
        for b in self.typed_children() {
            n += b.box_len();
        }
        for o in &self.opaque {
            n += o.serialized_len();
        }
        crate::box_types::box_len(n)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        serialize_container(
            b"minf",
            self.serialized_len(),
            &self.child_order(),
            &self.typed_children(),
            &self.opaque,
            buf,
        )
    }
}

// ---------------------------------------------------------------------------
// MediaBox — mdia (container: mdhd, hdlr, minf)
// ---------------------------------------------------------------------------

/// Four-CC of the Media Box (`mdia`, ISO/IEC 14496-12:2026 §8.4.1).
pub const MDIA: [u8; 4] = *b"mdia";

/// Typed child slots in an `mdia` (§8.4.1).
const MDIA_TYPED_CHILDREN_MAX: usize = 4;

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MediaBox {
    pub mdhd: Option<MediaHeaderBox>,
    pub hdlr: Option<HandlerBox>,
    pub minf: Option<MediaInformationBox>,
    /// The extended language tag (§8.4.6), when present.
    pub elng: Option<crate::track_desc::ExtendedLanguageBox>,
    pub opaque: Vec<OpaqueBox>,
    /// The wire order of this container's children (audit r05-W12).
    pub order: ChildOrder,
}

impl MediaBox {
    /// Build an `mdia` from its typed parts, children in the conventional
    /// `mdhd`, `hdlr`, `minf` sequence.
    pub fn new(
        mdhd: Option<MediaHeaderBox>,
        hdlr: Option<HandlerBox>,
        minf: Option<MediaInformationBox>,
        opaque: Vec<OpaqueBox>,
    ) -> Self {
        Self {
            mdhd,
            hdlr,
            minf,
            elng: None,
            opaque,
            order: ChildOrder::default(),
        }
    }
}

impl MediaBox {
    /// The QuickTime base media information header (`gmhd`), when this
    /// container reaches one.
    #[must_use]
    pub fn gmhd(&self) -> Option<crate::quicktime::gmhd::GmhdAtom> {
        self.minf.as_ref().and_then(|minf| minf.gmhd())
    }

    /// The container's typed children, in declaration order — the input
    /// [`ChildOrder::resolve`] maps a wire order onto.
    fn typed_children(&self) -> Vec<&dyn SerializeBox<Error = crate::Error>> {
        let mut out: Vec<&dyn SerializeBox<Error = crate::Error>> =
            Vec::with_capacity(MDIA_TYPED_CHILDREN_MAX);
        if let Some(ref b) = self.mdhd {
            out.push(b);
        }
        if let Some(ref b) = self.hdlr {
            out.push(b);
        }
        if let Some(ref b) = self.minf {
            out.push(b);
        }
        if let Some(ref b) = self.elng {
            out.push(b);
        }
        out
    }

    /// This container's typed children in declaration order, as four-CCs.
    fn typed_four_ccs(&self) -> Vec<&'static [u8; BOX_TYPE_SIZE]> {
        let mut out = Vec::with_capacity(MDIA_TYPED_CHILDREN_MAX);
        if self.mdhd.is_some() {
            out.push(b"mdhd" as &'static [u8; BOX_TYPE_SIZE]);
        }
        if self.hdlr.is_some() {
            out.push(b"hdlr" as &'static [u8; BOX_TYPE_SIZE]);
        }
        if self.minf.is_some() {
            out.push(b"minf" as &'static [u8; BOX_TYPE_SIZE]);
        }
        if self.elng.is_some() {
            out.push(b"elng" as &'static [u8; BOX_TYPE_SIZE]);
        }
        out
    }

    fn child_order(&self) -> Vec<Option<usize>> {
        self.order.resolve(&self.typed_four_ccs())
    }
}

impl<'a> Parse<'a> for MediaBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let mut mdhd = None;
        let mut hdlr = None;
        let mut minf = None;
        let mut elng = None;
        let mut opaque = Vec::new();
        let mut order = ChildOrder::default();
        for child in parse_box(bytes)?.0.children() {
            let child = child?;
            match &child.four_cc() {
                b"mdhd" => {
                    order.push(child.four_cc());
                    mdhd = Some(MediaHeaderBox::parse(child.bytes)?);
                }
                b"hdlr" => {
                    order.push(child.four_cc());
                    hdlr = Some(HandlerBox::parse(child.bytes)?);
                }
                b"minf" => {
                    order.push(child.four_cc());
                    minf = Some(MediaInformationBox::parse(child.bytes)?);
                }
                b"elng" => {
                    order.push(child.four_cc());
                    elng = Some(crate::track_desc::ExtendedLanguageBox::parse(child.bytes)?);
                }
                _ => {
                    order.push_opaque(child.four_cc());
                    opaque.push(OpaqueBox {
                        box_type: child.four_cc(),
                        data: child.payload().to_vec(),
                        to_end: child.to_end(),
                        largesize: child.has_largesize(),
                    });
                }
            }
        }
        Ok(Self {
            mdhd,
            hdlr,
            minf,
            elng,
            opaque,
            order,
        })
    }
}

impl Serialize for MediaBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HEADER_MIN_SIZE;
        for b in self.typed_children() {
            n += b.box_len();
        }
        for o in &self.opaque {
            n += o.serialized_len();
        }
        crate::box_types::box_len(n)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        serialize_container(
            b"mdia",
            self.serialized_len(),
            &self.child_order(),
            &self.typed_children(),
            &self.opaque,
            buf,
        )
    }
}

// ---------------------------------------------------------------------------
// EditBox — edts (container: elst)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct EditBox {
    pub elst: Option<crate::timing::EditListBox>,
    pub opaque: Vec<OpaqueBox>,
    /// The wire order of this container's children (audit r05-W12).
    pub order: ChildOrder,
}

impl EditBox {
    /// Build an `edts` from its typed parts, children in the conventional
    /// `elst`-then-opaque order.
    pub fn new(elst: Option<crate::timing::EditListBox>, opaque: Vec<OpaqueBox>) -> Self {
        Self {
            elst,
            opaque,
            order: ChildOrder::default(),
        }
    }

    /// The QuickTime base media information header (`gmhd`), when this
    /// container reaches one.
    #[must_use]
    pub fn gmhd(&self) -> Option<crate::quicktime::gmhd::GmhdAtom> {
        self.opaque
            .iter()
            .find(|b| b.box_type == crate::quicktime::gmhd::GMHD)
            .and_then(|b| crate::quicktime::gmhd::GmhdAtom::parse_body(&b.data).ok())
    }

    /// The container's typed children, in declaration order — the input
    /// [`ChildOrder::resolve`] maps a wire order onto.
    fn typed_children(&self) -> Vec<&dyn SerializeBox<Error = crate::Error>> {
        let mut out: Vec<&dyn SerializeBox<Error = crate::Error>> = Vec::with_capacity(1);
        if let Some(ref b) = self.elst {
            out.push(b);
        }
        out
    }

    /// This container's typed children in declaration order, as four-CCs.
    fn typed_four_ccs(&self) -> Vec<&'static [u8; BOX_TYPE_SIZE]> {
        let mut out = Vec::with_capacity(1);
        if self.elst.is_some() {
            out.push(b"elst" as &'static [u8; BOX_TYPE_SIZE]);
        }
        out
    }

    fn child_order(&self) -> Vec<Option<usize>> {
        self.order.resolve(&self.typed_four_ccs())
    }
}

impl<'a> Parse<'a> for EditBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let mut elst = None;
        let mut opaque = Vec::new();
        let mut order = ChildOrder::default();
        for child in parse_box(bytes)?.0.children() {
            let child = child?;
            if child.header.box_type.is(b"elst") {
                order.push(child.four_cc());
                elst = Some(crate::timing::EditListBox::parse(child.bytes)?);
            } else {
                order.push_opaque(child.four_cc());
                opaque.push(OpaqueBox {
                    box_type: child.four_cc(),
                    data: child.payload().to_vec(),
                    to_end: child.to_end(),
                    largesize: child.has_largesize(),
                });
            }
        }
        Ok(Self {
            elst,
            opaque,
            order,
        })
    }
}

impl Serialize for EditBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HEADER_MIN_SIZE;
        for b in self.typed_children() {
            n += b.box_len();
        }
        for o in &self.opaque {
            n += o.serialized_len();
        }
        crate::box_types::box_len(n)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        serialize_container(
            b"edts",
            self.serialized_len(),
            &self.child_order(),
            &self.typed_children(),
            &self.opaque,
            buf,
        )
    }
}

// ---------------------------------------------------------------------------
// TrackBox — trak (container: tkhd, edts?, mdia, …)
// ---------------------------------------------------------------------------

/// Four-CC of the Track Box (`trak`, ISO/IEC 14496-12:2026 §8.3.1).
pub const TRAK: [u8; 4] = *b"trak";

/// Typed child slots in a `trak` (§8.3.1).
const TRAK_TYPED_CHILDREN_MAX: usize = 4;

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TrackBox {
    pub tkhd: TrackHeaderBox,
    pub edts: Option<EditBox>,
    pub mdia: Option<MediaBox>,
    /// The track groups (§8.3.4), when present.
    pub trgr: Option<crate::track_desc::TrackGroupBox>,
    pub opaque: Vec<OpaqueBox>,
    /// The wire order of this container's children (audit r05-W12).
    pub order: ChildOrder,
}

impl TrackBox {
    /// The track's `tref` (ISO/IEC 14496-12:2026 §8.3.3), parsed from the
    /// opaque child that round-trips in place; `None` when the track has none.
    pub fn track_references(&self) -> Result<Option<crate::track_reference::TrackReferenceBox>> {
        self.opaque
            .iter()
            .find(|b| b.box_type == *b"tref")
            .map(|b| crate::track_reference::TrackReferenceBox::parse_body(&b.data))
            .transpose()
    }

    /// The track's sample-group mappings, from both `sbgp` and `csgp`, in
    /// wire order.
    ///
    /// Both box types feed the one [`crate::sample_groups::SampleGroupMapping`]
    /// lookup, so a caller never reads two shapes for the same fact.
    /// `max_samples` caps a compact mapping's expansion; pass
    /// [`crate::limits::DEFAULT_MAX_SAMPLES_PER_TRACK`](crate::limits)
    /// as `usize` for the crate's hostile-input default.
    pub fn sample_group_mappings(
        &self,
        max_samples: usize,
    ) -> Result<Vec<crate::sample_groups::SampleGroupMapping>> {
        let Some(stbl) = self
            .mdia
            .as_ref()
            .and_then(|mdia| mdia.minf.as_ref())
            .and_then(|minf| minf.stbl.as_ref())
        else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for child in &stbl.children {
            match child {
                StblChild::Csgp(csgp) => out.push(csgp.mapping_with_limit(max_samples)?),
                StblChild::Opaque(bytes)
                    if bytes.get(BOX_TYPE_OFFSET..BOX_HEADER_MIN_SIZE)
                        == Some(b"sbgp".as_slice()) =>
                {
                    out.push(crate::sample_groups::SampleToGroupBox::parse(bytes)?.mapping());
                }
                _ => {}
            }
        }
        Ok(out)
    }

    /// The track's typed `udta`/`meta` metadata, when either is present.
    ///
    /// The boxes themselves stay in [`TrackBox::opaque`] so every existing
    /// init-segment golden re-serialises byte for byte; this reads them on
    /// demand under the caller's [`Limits`].
    pub fn metadata(&self, limits: &Limits) -> Result<Option<crate::metadata::Metadata>> {
        metadata_of_opaque(&self.opaque, limits)
    }

    /// Build a `trak` from its typed parts, children in the conventional
    /// `tkhd`, `edts`, `mdia` sequence.
    pub fn new(
        tkhd: TrackHeaderBox,
        edts: Option<EditBox>,
        mdia: Option<MediaBox>,
        opaque: Vec<OpaqueBox>,
    ) -> Self {
        Self {
            tkhd,
            edts,
            mdia,
            trgr: None,
            opaque,
            order: ChildOrder::default(),
        }
    }

    /// The QuickTime base media information header (`gmhd`), when this
    /// container reaches one.
    #[must_use]
    pub fn gmhd(&self) -> Option<crate::quicktime::gmhd::GmhdAtom> {
        self.mdia.as_ref().and_then(|mdia| mdia.gmhd())
    }

    /// The container's typed children, in declaration order — the input
    /// [`ChildOrder::resolve`] maps a wire order onto.
    fn typed_children(&self) -> Vec<&dyn SerializeBox<Error = crate::Error>> {
        let mut out: Vec<&dyn SerializeBox<Error = crate::Error>> =
            Vec::with_capacity(TRAK_TYPED_CHILDREN_MAX);
        out.push(&self.tkhd);
        if let Some(ref b) = self.edts {
            out.push(b);
        }
        if let Some(ref b) = self.mdia {
            out.push(b);
        }
        if let Some(ref b) = self.trgr {
            out.push(b);
        }
        out
    }

    /// This container's typed children in declaration order, as four-CCs.
    fn typed_four_ccs(&self) -> Vec<&'static [u8; BOX_TYPE_SIZE]> {
        let mut out = Vec::with_capacity(TRAK_TYPED_CHILDREN_MAX);
        out.push(b"tkhd" as &'static [u8; BOX_TYPE_SIZE]);
        if self.edts.is_some() {
            out.push(b"edts");
        }
        if self.mdia.is_some() {
            out.push(b"mdia");
        }
        if self.trgr.is_some() {
            out.push(b"trgr");
        }
        out
    }

    fn child_order(&self) -> Vec<Option<usize>> {
        self.order.resolve(&self.typed_four_ccs())
    }
}

impl<'a> Parse<'a> for TrackBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let mut tkhd = None;
        let mut edts = None;
        let mut mdia = None;
        let mut trgr = None;
        let mut opaque = Vec::new();
        let mut order = ChildOrder::default();
        for child in parse_box(bytes)?.0.children() {
            let child = child?;
            match &child.four_cc() {
                b"tkhd" => {
                    order.push(child.four_cc());
                    tkhd = Some(TrackHeaderBox::parse(child.bytes)?);
                }
                b"edts" => {
                    order.push(child.four_cc());
                    edts = Some(EditBox::parse(child.bytes)?);
                }
                b"mdia" => {
                    order.push(child.four_cc());
                    mdia = Some(MediaBox::parse(child.bytes)?);
                }
                b"trgr" => {
                    order.push(child.four_cc());
                    trgr = Some(crate::track_desc::TrackGroupBox::parse(child.bytes)?);
                }
                // `tref`/`udta`/`meta` stay opaque, in place.
                _ => {
                    order.push_opaque(child.four_cc());
                    opaque.push(OpaqueBox {
                        box_type: child.four_cc(),
                        data: child.payload().to_vec(),
                        to_end: child.to_end(),
                        largesize: child.has_largesize(),
                    });
                }
            }
        }
        Ok(Self {
            tkhd: tkhd.ok_or(Error::BufferTooShort(BufferTooShort {
                need: 0,
                have: 0,
                what: "trak missing tkhd",
            }))?,
            edts,
            mdia,
            trgr,
            opaque,
            order,
        })
    }
}

impl Serialize for TrackBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HEADER_MIN_SIZE;
        for b in self.typed_children() {
            n += b.box_len();
        }
        for o in &self.opaque {
            n += o.serialized_len();
        }
        crate::box_types::box_len(n)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        serialize_container(
            b"trak",
            self.serialized_len(),
            &self.child_order(),
            &self.typed_children(),
            &self.opaque,
            buf,
        )
    }
}

/// Parse the `udta`/`meta` children of an opaque child list into the logical
/// [`Metadata`](crate::metadata::Metadata) view.
fn metadata_of_opaque(
    opaque: &[OpaqueBox],
    limits: &Limits,
) -> Result<Option<crate::metadata::Metadata>> {
    let mut metadata = crate::metadata::Metadata::default();
    let mut any = false;
    for child in opaque {
        match &child.box_type {
            b"udta" => {
                let udta = crate::metadata::Udta::parse_body(&child.data, limits)?;
                metadata
                    .items
                    .extend(crate::metadata::Metadata::from_udta(&udta).items);
                any = true;
            }
            b"meta" => {
                let meta = crate::metadata::Meta::parse_body(&child.data, limits)?;
                metadata
                    .items
                    .extend(crate::metadata::Metadata::from_meta(&meta).items);
                any = true;
            }
            _ => {}
        }
    }
    Ok(any.then_some(metadata))
}

// ---------------------------------------------------------------------------
// MovieBox — moov (container: mvhd, trak*, …) — THE TOP-LEVEL TYPE
// ---------------------------------------------------------------------------

/// Fixed typed children of a `moov` before its `trak`s (§8.2.1).
const MOOV_FIXED_TYPED_CHILDREN: usize = 2;

/// `trex` fixed body length after the box header (§8.8.3).
const TREX_BODY_LEN: usize = 24;
/// `trex` box length (§8.8.3).
const TREX_LEN: usize = BOX_HEADER_MIN_SIZE + TREX_BODY_LEN;
/// Offset of `trex` `flags` within its body (§8.8.3).
const TREX_FLAGS_OFFSET: usize = FULLBOX_VERSION_LEN;
/// Offset of `trex` `track_ID` within its body (§8.8.3).
const TREX_TRACK_ID_OFFSET: usize = 4;
/// Offset of `trex` `default_sample_description_index` within its body (§8.8.3).
const TREX_SAMPLE_DESCRIPTION_INDEX_OFFSET: usize = 8;
/// Offset of `trex` `default_sample_duration` within its body (§8.8.3).
const TREX_SAMPLE_DURATION_OFFSET: usize = 12;
/// Offset of `trex` `default_sample_size` within its body (§8.8.3).
const TREX_SAMPLE_SIZE_OFFSET: usize = 16;
/// Offset of `trex` `default_sample_flags` within its body (§8.8.3).
const TREX_SAMPLE_FLAGS_OFFSET: usize = 20;

/// Track Extends Box (`trex`) — ISO/IEC 14496-12:2026 §8.8.3.
///
/// Declares per-track defaults for the samples carried in movie fragments. A
/// fragmented-init `moov` carries one `trex` per track inside [`MovieExtendsBox`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TrackExtendsBox {
    /// FullBox version (0).
    pub version: u8,
    /// FullBox flags (0).
    pub flags: u32,
    /// The track these defaults apply to.
    pub track_id: TrackId,
    /// Default `stsd` entry index (1-based).
    pub default_sample_description_index: u32,
    /// Default sample duration (movie timescale units).
    pub default_sample_duration: u32,
    /// Default sample size in bytes.
    pub default_sample_size: u32,
    /// Default per-sample flags (§8.8.3 sample flags layout).
    pub default_sample_flags: u32,
}

impl<'a> Parse<'a> for TrackExtendsBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = crate::box_types::box_body(bytes, TREX_BODY_LEN, "trex")?;
        let version = body[0];
        let flags = u24_at(body, TREX_FLAGS_OFFSET);
        Ok(Self {
            version,
            flags,
            track_id: TrackId::new(u32_at(body, TREX_TRACK_ID_OFFSET)),
            default_sample_description_index: u32_at(body, TREX_SAMPLE_DESCRIPTION_INDEX_OFFSET),
            default_sample_duration: u32_at(body, TREX_SAMPLE_DURATION_OFFSET),
            default_sample_size: u32_at(body, TREX_SAMPLE_SIZE_OFFSET),
            default_sample_flags: u32_at(body, TREX_SAMPLE_FLAGS_OFFSET),
        })
    }
}

impl Serialize for TrackExtendsBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        TREX_LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        need_out(buf, TREX_LEN)?;
        let u32_len = core::mem::size_of::<u32>();
        let mut c = 0usize;
        buf[c..c + u32_len].copy_from_slice(&(TREX_LEN as u32).to_be_bytes());
        c += u32_len;
        buf[c..c + BOX_TYPE_SIZE].copy_from_slice(b"trex");
        c += BOX_TYPE_SIZE;
        buf[c] = self.version;
        c += FULLBOX_VERSION_LEN;
        let fb = self.flags.to_be_bytes();
        buf[c..c + FULLBOX_FLAGS_WIDTH].copy_from_slice(&fb[FULLBOX_VERSION_LEN..]);
        c += FULLBOX_FLAGS_WIDTH;
        buf[c..c + u32_len].copy_from_slice(&self.track_id.value().to_be_bytes());
        c += u32_len;
        buf[c..c + u32_len].copy_from_slice(&self.default_sample_description_index.to_be_bytes());
        c += u32_len;
        buf[c..c + u32_len].copy_from_slice(&self.default_sample_duration.to_be_bytes());
        c += u32_len;
        buf[c..c + u32_len].copy_from_slice(&self.default_sample_size.to_be_bytes());
        c += u32_len;
        buf[c..c + u32_len].copy_from_slice(&self.default_sample_flags.to_be_bytes());
        c += u32_len;
        Ok(c)
    }
}

/// Movie Extends Box (`mvex`) — ISO/IEC 14496-12:2026 §8.8.1.
///
/// Signals that the movie is fragmented and carries the per-track [`TrackExtendsBox`]
/// defaults. Any other children are preserved verbatim in `opaque`, at the
/// position they occupied on the wire — see [`MovieExtendsBox::order`].
///
/// Movie Extends Header Box (`mehd`) — ISO/IEC 14496-12:2026 §8.8.2.
///
/// The presentation's whole duration, fragments included, in the movie
/// timescale. Version 1 widens `fragment_duration` to 64 bits.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct MovieExtendsHeaderBox {
    version: u8,
    flags: u32,
    fragment_duration: u64,
}

impl MovieExtendsHeaderBox {
    /// A version-0 header.
    #[must_use]
    pub const fn new(fragment_duration: u32) -> Self {
        Self {
            version: 0,
            flags: 0,
            fragment_duration: fragment_duration as u64,
        }
    }

    /// A version-1 header with a 64-bit duration.
    #[must_use]
    pub const fn wide(fragment_duration: u64) -> Self {
        Self {
            version: 1,
            flags: 0,
            fragment_duration,
        }
    }

    /// The FullBox version (0 or 1).
    #[must_use]
    pub const fn version(&self) -> u8 {
        self.version
    }

    /// The FullBox flags.
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// `fragment_duration`, in the movie timescale.
    #[must_use]
    pub const fn fragment_duration(&self) -> u64 {
        self.fragment_duration
    }

    /// The width of `fragment_duration` this version uses.
    const fn duration_len(version: u8) -> usize {
        if version == 1 {
            core::mem::size_of::<u64>()
        } else {
            core::mem::size_of::<u32>()
        }
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, FULLBOX_EXTRA_SIZE, "mehd")?;
        let version = body[0];
        if version > 1 {
            return Err(Error::UnsupportedVersion(
                broadcast_core::error::UnsupportedVersion {
                    what: "mehd",
                    found: version,
                },
            ));
        }
        let len = Self::duration_len(version);
        need_in(body, FULLBOX_EXTRA_SIZE + len, "mehd")?;
        let fragment_duration = if version == 1 {
            broadcast_core::wire::be_u64(body, FULLBOX_EXTRA_SIZE, "mehd fragment_duration")?
        } else {
            u64::from(broadcast_core::wire::be_u32(
                body,
                FULLBOX_EXTRA_SIZE,
                "mehd fragment_duration",
            )?)
        };
        Ok(Self {
            version,
            flags: u24_at(body, 1) & FULLBOX_FLAGS_MASK,
            fragment_duration,
        })
    }
}

impl<'a> Parse<'a> for MovieExtendsHeaderBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let item = parse_box(bytes)?.0;
        if item.four_cc() != *b"mehd" {
            return Err(Error::UnexpectedBox { expected: "mehd" });
        }
        Self::parse_body(item.body)
    }
}

impl Serialize for MovieExtendsHeaderBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + Self::duration_len(self.version)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor =
            write_full_box_header(buf, b"mehd", need, self.version, self.flags, false)?;
        let len = Self::duration_len(self.version);
        if self.version == 1 {
            buf[cursor..cursor + len].copy_from_slice(&self.fragment_duration.to_be_bytes());
        } else {
            let value = broadcast_core::len::fit_u32(
                usize::try_from(self.fragment_duration)
                    .map_err(|_| Error::InvalidInput("mehd duration exceeds u32"))?,
                "mehd fragment_duration",
            )?;
            buf[cursor..cursor + len].copy_from_slice(&value.to_be_bytes());
        }
        cursor += len;
        Ok(cursor)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MovieExtendsBox {
    /// One `trex` per track.
    pub trex: Vec<TrackExtendsBox>,
    /// The level assignment (§8.8.13), when present.
    pub leva: Option<crate::index_boxes::LevelAssignmentBox>,
    /// The track extension properties (§8.8.15), in wire order.
    pub trep: Vec<crate::index_boxes::TrackExtensionPropertiesBox>,
    /// The movie extends header (`mehd`, §8.8.2), when present.
    pub mehd: Option<MovieExtendsHeaderBox>,
    /// Other `mvex` children preserved verbatim (e.g. `mehd`).
    pub opaque: Vec<OpaqueBox>,
    /// The wire order of this container's children (audit r05-W12).
    pub order: ChildOrder,
}

impl MovieExtendsBox {
    /// Build an `mvex` from its typed parts, children in the conventional
    /// `trex`…-then-opaque order.
    pub fn new(trex: Vec<TrackExtendsBox>, opaque: Vec<OpaqueBox>) -> Self {
        Self {
            trex,
            leva: None,
            trep: Vec::new(),
            mehd: None,
            opaque,
            order: ChildOrder::default(),
        }
    }

    /// The QuickTime base media information header (`gmhd`), when this
    /// container reaches one.
    #[must_use]
    pub fn gmhd(&self) -> Option<crate::quicktime::gmhd::GmhdAtom> {
        self.opaque
            .iter()
            .find(|b| b.box_type == crate::quicktime::gmhd::GMHD)
            .and_then(|b| crate::quicktime::gmhd::GmhdAtom::parse_body(&b.data).ok())
    }

    /// The container's typed children, in declaration order — the input
    /// [`ChildOrder::resolve`] maps a wire order onto.
    fn typed_children(&self) -> Vec<&dyn SerializeBox<Error = crate::Error>> {
        let mut out: Vec<&dyn SerializeBox<Error = crate::Error>> = self
            .trex
            .iter()
            .map(|t| t as &dyn SerializeBox<Error = crate::Error>)
            .collect();
        if let Some(ref leva) = self.leva {
            out.push(leva);
        }
        for trep in &self.trep {
            out.push(trep);
        }
        if let Some(ref mehd) = self.mehd {
            out.push(mehd);
        }
        out
    }

    /// This container's typed children in declaration order, as four-CCs.
    fn typed_four_ccs(&self) -> Vec<&'static [u8; BOX_TYPE_SIZE]> {
        let mut out = alloc::vec![b"trex" as &'static [u8; BOX_TYPE_SIZE]; self.trex.len()];
        if self.leva.is_some() {
            out.push(b"leva");
        }
        out.extend(core::iter::repeat_n(
            b"trep" as &'static [u8; BOX_TYPE_SIZE],
            self.trep.len(),
        ));
        if self.mehd.is_some() {
            out.push(b"mehd");
        }
        out
    }

    fn child_order(&self) -> Vec<Option<usize>> {
        self.order.resolve(&self.typed_four_ccs())
    }
}

impl<'a> Parse<'a> for MovieExtendsBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let mut trex = Vec::new();
        let mut leva = None;
        let mut trep = Vec::new();
        let mut mehd = None;
        let mut opaque = Vec::new();
        let mut order = ChildOrder::default();
        for child in parse_box(bytes)?.0.children() {
            let child = child?;
            match &child.four_cc() {
                b"trex" => {
                    order.push(child.four_cc());
                    trex.push(TrackExtendsBox::parse(child.bytes)?);
                }
                b"leva" => {
                    order.push(child.four_cc());
                    leva = Some(crate::index_boxes::LevelAssignmentBox::parse(child.bytes)?);
                }
                b"trep" => {
                    order.push(child.four_cc());
                    trep.push(crate::index_boxes::TrackExtensionPropertiesBox::parse(
                        child.bytes,
                    )?);
                }
                b"mehd" => {
                    order.push(child.four_cc());
                    mehd = Some(MovieExtendsHeaderBox::parse(child.bytes)?);
                }
                // Any unmodelled child stays opaque, in place.
                _ => {
                    order.push_opaque(child.four_cc());
                    opaque.push(OpaqueBox {
                        box_type: child.four_cc(),
                        data: child.payload().to_vec(),
                        to_end: child.to_end(),
                        largesize: child.has_largesize(),
                    });
                }
            }
        }
        Ok(Self {
            trex,
            leva,
            trep,
            mehd,
            opaque,
            order,
        })
    }
}

impl Serialize for MovieExtendsBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HEADER_MIN_SIZE;
        for b in self.typed_children() {
            n += b.box_len();
        }
        for o in &self.opaque {
            n += o.serialized_len();
        }
        crate::box_types::box_len(n)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        serialize_container(
            b"mvex",
            self.serialized_len(),
            &self.child_order(),
            &self.typed_children(),
            &self.opaque,
            buf,
        )
    }
}

/// Four-CC of the Movie Box (`moov`, ISO/IEC 14496-12:2026 §8.2.1).
pub const MOOV: [u8; 4] = *b"moov";

/// Movie Box (`moov`) — §8.2.1.  The top-level init-segment container.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MovieBox {
    pub mvhd: MovieHeaderBox,
    pub tracks: Vec<TrackBox>,
    /// Movie-extends box (`mvex`) present in fragmented-init movies.
    pub mvex: Option<MovieExtendsBox>,
    pub opaque: Vec<OpaqueBox>,
    /// The wire order of this container's children (audit r05-W12).
    pub order: ChildOrder,
}

impl MovieBox {
    /// The movie's typed `udta`/`meta` metadata, when either is present.
    ///
    /// Both boxes stay in [`MovieBox::opaque`], so the typed tree and every
    /// golden built from it are unchanged; this parses them on demand.
    pub fn metadata(&self, limits: &Limits) -> Result<Option<crate::metadata::Metadata>> {
        metadata_of_opaque(&self.opaque, limits)
    }

    /// Build a `moov` from its typed parts, children in the conventional
    /// `mvhd`, `trak`…, `mvex` sequence.
    pub fn new(
        mvhd: MovieHeaderBox,
        tracks: Vec<TrackBox>,
        mvex: Option<MovieExtendsBox>,
        opaque: Vec<OpaqueBox>,
    ) -> Self {
        Self {
            mvhd,
            tracks,
            mvex,
            opaque,
            order: ChildOrder::default(),
        }
    }

    /// The QuickTime base media information header (`gmhd`), when this
    /// container reaches one.
    #[must_use]
    pub fn gmhd(&self) -> Option<crate::quicktime::gmhd::GmhdAtom> {
        self.opaque
            .iter()
            .find(|b| b.box_type == crate::quicktime::gmhd::GMHD)
            .and_then(|b| crate::quicktime::gmhd::GmhdAtom::parse_body(&b.data).ok())
    }

    /// The container's typed children, in declaration order — the input
    /// [`ChildOrder::resolve`] maps a wire order onto.
    fn typed_children(&self) -> Vec<&dyn SerializeBox<Error = crate::Error>> {
        let mut out: Vec<&dyn SerializeBox<Error = crate::Error>> =
            Vec::with_capacity(MOOV_FIXED_TYPED_CHILDREN + self.tracks.len());
        out.push(&self.mvhd);
        for t in &self.tracks {
            out.push(t);
        }
        if let Some(ref m) = self.mvex {
            out.push(m);
        }
        out
    }

    /// This container's typed children in declaration order, as four-CCs.
    fn typed_four_ccs(&self) -> Vec<&'static [u8; BOX_TYPE_SIZE]> {
        let mut out: Vec<&'static [u8; BOX_TYPE_SIZE]> = alloc::vec![b"mvhd"; 1];
        out.resize(1 + self.tracks.len(), b"trak");
        if self.mvex.is_some() {
            out.push(b"mvex");
        }
        out
    }

    /// The children to serialize, in wire order — the `pssh` a movie may carry
    /// keeps its own position instead of moving after the `trak`s (audit
    /// r05-W12).
    pub fn child_order(&self) -> Vec<Option<usize>> {
        self.order.resolve(&self.typed_four_ccs())
    }
}

impl<'a> Parse<'a> for MovieBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let mut mvhd = None;
        let mut tracks = Vec::new();
        let mut mvex = None;
        let mut opaque = Vec::new();
        let mut order = ChildOrder::default();
        for child in parse_box(bytes)?.0.children() {
            let child = child?;
            match &child.four_cc() {
                b"mvhd" => {
                    order.push(child.four_cc());
                    mvhd = Some(MovieHeaderBox::parse(child.bytes)?);
                }
                b"trak" => {
                    order.push(child.four_cc());
                    tracks.push(TrackBox::parse(child.bytes)?);
                }
                b"mvex" => {
                    order.push(child.four_cc());
                    mvex = Some(MovieExtendsBox::parse(child.bytes)?);
                }
                // `pssh`/`udta`/`meta`/`iods` stay opaque, in place: the
                // conventional layout writes `pssh` ahead of the `trak`s, and the
                // old serializer always emitted it last.
                _ => {
                    order.push_opaque(child.four_cc());
                    opaque.push(OpaqueBox {
                        box_type: child.four_cc(),
                        data: child.payload().to_vec(),
                        to_end: child.to_end(),
                        largesize: child.has_largesize(),
                    });
                }
            }
        }
        Ok(Self {
            mvhd: mvhd.ok_or(Error::BufferTooShort(BufferTooShort {
                need: 0,
                have: 0,
                what: "moov missing mvhd",
            }))?,
            tracks,
            mvex,
            opaque,
            order,
        })
    }
}

impl Serialize for MovieBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HEADER_MIN_SIZE;
        for b in self.typed_children() {
            n += b.box_len();
        }
        for o in &self.opaque {
            n += o.serialized_len();
        }
        crate::box_types::box_len(n)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        serialize_container(
            b"moov",
            self.serialized_len(),
            &self.child_order(),
            &self.typed_children(),
            &self.opaque,
            buf,
        )
    }
}

// ---------------------------------------------------------------------------
// CENC init-segment protection (`encv`/`enca` + `sinf`) — ISO/IEC
// 14496-12:2026 §13.4 (sinf/frma/schm/schi), ISO/IEC 23001-7 §12.2 (tenc) —
// issue #564 Task 3 (muxer emission).
// ---------------------------------------------------------------------------

/// `schm.scheme_version` written for every CENC-protected sample entry:
/// version 1.0, packed as a 16.16 major.minor pair per ISO/IEC
/// 14496-12:2026 §13.4.6.
const CENC_SCHEME_VERSION: u32 = 0x0001_0000;

/// Which media kind a [`SampleEntryVariant`] carries, to choose the
/// CENC-protected wrapper four-CC (`encv` for video, `enca` for audio —
/// ISO/IEC 14496-12:2026 §13.4.1). Subtitle (`stpp`/`wvtt`) and unrecognised
/// entries have no protected-wrapper form defined in this crate.
enum SampleEntryMediaKind {
    Video,
    Audio,
    Unsupported,
}

fn sample_entry_media_kind(entry: &SampleEntryVariant) -> SampleEntryMediaKind {
    match entry {
        SampleEntryVariant::Avc1(_)
        | SampleEntryVariant::Hevc1(_)
        | SampleEntryVariant::Vvc(_)
        | SampleEntryVariant::Mp4v(_)
        | SampleEntryVariant::Av01(_)
        | SampleEntryVariant::Vp09(_) => SampleEntryMediaKind::Video,
        SampleEntryVariant::Mp4a(_)
        | SampleEntryVariant::Ac3(_)
        | SampleEntryVariant::Ec3(_)
        | SampleEntryVariant::Opus(_)
        | SampleEntryVariant::Flac(_)
        | SampleEntryVariant::Mha(_)
        | SampleEntryVariant::Dts(_)
        | SampleEntryVariant::Ac4(_) => SampleEntryMediaKind::Audio,
        SampleEntryVariant::AvcParameterSet(_)
        | SampleEntryVariant::Stpp(_)
        | SampleEntryVariant::Wvtt(_)
        | SampleEntryVariant::Tmcd(_)
        | SampleEntryVariant::Metx(_)
        | SampleEntryVariant::Text(_)
        | SampleEntryVariant::Urim(_)
        | SampleEntryVariant::Mebx(_)
        | SampleEntryVariant::Rgan(_)
        | SampleEntryVariant::QuickTimeSound(_)
        | SampleEntryVariant::ClosedCaption(_)
        | SampleEntryVariant::Hint(_)
        | SampleEntryVariant::Unknown(_) => SampleEntryMediaKind::Unsupported,
    }
}

/// Serialize a [`SampleEntryVariant`] to its own box bytes (header + body).
/// Mirrors the dispatch [`SampleDescriptionBox::serialize_into`] uses,
/// exposed standalone so [`protect_sample_entry`] can recover the original
/// entry's bytes to wrap (rather than hand-rolling a duplicate encoder).
fn sample_entry_bytes(entry: &SampleEntryVariant) -> Result<Vec<u8>> {
    let mut buf = alloc::vec![0u8; entry.serialized_len()];
    let n = entry.serialize_into(&mut buf)?;
    buf.truncate(n);
    Ok(buf)
}

/// Wrap a clear sample entry as a CENC-protected `encv`/`enca` entry —
/// ISO/IEC 14496-12:2026 §13.4.1: rename the codec four-CC to `encv`/`enca`,
/// keep the original four-CC in a child `sinf`>`frma`, and add `sinf`>`schm`
/// (`scheme_type`/`scheme_version`) + `sinf`>`schi`>`tenc` (ISO/IEC
/// 23001-7 §12.2).
///
/// Returned as [`SampleEntryVariant::Unknown`] — this crate's generic
/// passthrough case — rather than as a new enum variant: the decrypt side
/// (`crate::cenc_decrypt`) locates `sinf` by walking the raw sample-entry
/// bytes directly, not through this enum, so a generic body box is
/// sufficient, and it keeps every existing exhaustive match over
/// [`SampleEntryVariant`] (e.g. `crate::media::codec_config_from_entry`)
/// unchanged (issue #564).
pub fn protect_sample_entry(
    original: &SampleEntryVariant,
    scheme: crate::cenc::CencScheme,
    tenc: &crate::cenc::TrackEncryptionBox,
) -> Result<SampleEntryVariant> {
    let wrapper_fourcc: [u8; BOX_TYPE_SIZE] = match sample_entry_media_kind(original) {
        SampleEntryMediaKind::Video => *b"encv",
        SampleEntryMediaKind::Audio => *b"enca",
        SampleEntryMediaKind::Unsupported => {
            return Err(Error::InvalidInput(
                "protect_sample_entry: CENC protection is only defined for audio/video sample entries",
            ));
        }
    };

    let original_bytes = sample_entry_bytes(original)?;
    need_in(&original_bytes, BOX_HEADER_MIN_SIZE, "sample entry")?;
    let mut original_format = [0u8; BOX_TYPE_SIZE];
    original_format.copy_from_slice(&original_bytes[4..8]);

    let scheme_name = scheme.name().as_bytes();
    let mut scheme_type = [0u8; BOX_TYPE_SIZE];
    scheme_type.copy_from_slice(&scheme_name[..4]);

    let sinf = crate::cenc::ProtectionSchemeInfoBox {
        original_format: crate::cenc::OriginalFormatBox {
            data_format: original_format,
        },
        scheme_type: Some(crate::cenc::SchemeTypeBox {
            version: 0,
            flags: 0,
            scheme_type,
            scheme_version: CENC_SCHEME_VERSION,
            scheme_uri: None,
        }),
        scheme_info: Some(crate::cenc::SchemeInformationBox {
            tenc: Some(tenc.clone()),
            stvi: None,
            spki: None,
            extra_boxes: Vec::new(),
        }),
        extra_boxes: Vec::new(),
    };
    let mut sinf_bytes = alloc::vec![0u8; sinf.serialized_len()];
    let n = sinf.serialize_into(&mut sinf_bytes)?;
    sinf_bytes.truncate(n);

    let mut body =
        Vec::with_capacity(original_bytes.len() - BOX_HEADER_MIN_SIZE + sinf_bytes.len());
    body.extend_from_slice(crate::box_types::parse_box(&original_bytes)?.0.body);
    body.extend_from_slice(&sinf_bytes);

    Ok(SampleEntryVariant::Unknown(OpaqueBox::new(
        wrapper_fourcc,
        body,
    )))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    fn timescale(value: u32) -> Timescale {
        Timescale::try_new(value).expect("a non-zero timescale")
    }

    use super::*;
    use broadcast_core::Serialize;

    /// Build a small mvhd v0 for unit testing.
    fn sample_mvhd_v0() -> MovieHeaderBox {
        MovieHeaderBox {
            version: 0,
            flags: 0,
            creation_time: 0,
            modification_time: 0,
            timescale: timescale(1000),
            duration: 2000,
            rate: 0x00010000,
            volume: 0x0100,
            matrix: [0x00010000, 0, 0, 0, 0x00010000, 0, 0, 0, 0x40000000],
            next_track_id: TrackId::new(3),
        }
    }

    #[test]
    fn mvhd_v0_round_trip() {
        let m = sample_mvhd_v0();
        let bytes = m.try_to_bytes().unwrap();
        let parsed = MovieHeaderBox::parse(&bytes).unwrap();
        assert_eq!(parsed, m);
    }

    #[test]
    fn mvhd_v0_mutation_changes_bytes() {
        let m = sample_mvhd_v0();
        let orig = m.try_to_bytes().unwrap();
        let mut m2 = m.clone();
        m2.timescale = broadcast_core::timescale!(30000);
        let mutated = m2.try_to_bytes().unwrap();
        assert_ne!(orig, mutated);
        // Verify the right field changed
        assert_ne!(orig[20..24], mutated[20..24]);
    }

    #[test]
    fn tkhd_v0_round_trip() {
        let t = TrackHeaderBox {
            version: 0,
            flags: 0x000003, // track_enabled | track_in_movie
            creation_time: 0,
            modification_time: 0,
            track_id: TrackId::new(1),
            duration: 2000,
            layer: 0,
            alternate_group: 0,
            volume: 0,
            matrix: [0x00010000, 0, 0, 0, 0x00010000, 0, 0, 0, 0x40000000],
            width: 0,
            height: 0,
        };
        let bytes = t.try_to_bytes().unwrap();
        let parsed = TrackHeaderBox::parse(&bytes).unwrap();
        assert_eq!(parsed, t);
    }

    /// A §8.2.2/§8.3.2 transformation matrix whose nine 16.16/2.30 entries all
    /// have their low bytes non-zero: the identity matrix's low 16 bits are
    /// always zero, so a read from the wrong byte index inside the 36-byte
    /// matrix (audit 23) still picked up a zero and parsed "correctly".
    #[test]
    fn mvhd_and_tkhd_matrix_keep_every_non_zero_byte() {
        let matrix: [i32; 9] = [
            0x0001_8000,
            0x0000_00FF,
            0x1234_5678,
            0x9ABC_DEF0u32 as i32,
            0x0001_0001,
            0x0102_0304,
            0x7FFF_FFFF,
            0x0000_0001,
            0x4000_00AB,
        ];

        let mvhd = MovieHeaderBox {
            matrix,
            ..sample_mvhd_v0()
        };
        let bytes = mvhd.try_to_bytes().unwrap();
        let parsed = MovieHeaderBox::parse(&bytes).unwrap();
        assert_eq!(parsed.matrix, matrix, "mvhd matrix entries");
        assert_eq!(parsed.try_to_bytes().unwrap(), bytes);

        let tkhd = TrackHeaderBox {
            version: 0,
            flags: 0,
            creation_time: 0,
            modification_time: 0,
            track_id: TrackId::new(1),
            duration: 0,
            layer: 0,
            alternate_group: 0,
            volume: 0,
            matrix,
            width: 0,
            height: 0,
        };
        let bytes = tkhd.try_to_bytes().unwrap();
        let parsed = TrackHeaderBox::parse(&bytes).unwrap();
        assert_eq!(parsed.matrix, matrix, "tkhd matrix entries");
        assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
    }

    /// The trailing-byte shortfall the child walker reports must name the
    /// bytes actually missing: a caller feeding buffer length back from `need`
    /// reads the wrong amount otherwise (ISO/IEC 14496-12:2026 §4.2 box
    /// framing).
    #[test]
    fn child_walker_reports_the_trailing_child_header_shortfall() {
        let after_one_child = b"\x00\x00\x00\x08free\xAA\xBB\xCC";
        let b = match BoxIter::new(after_one_child).find_map(|r| r.err()) {
            Some(Error::BufferTooShort(b)) => b,
            other => panic!("expected BufferTooShort, got {other:?}"),
        };
        assert_eq!(b.need, 16);
        assert_eq!(b.have, 11);
        assert_eq!(b.what, "container child header");

        let at_start = b"\xAA\xBB\xCC";
        let b = match BoxIter::new(at_start).find_map(|r| r.err()) {
            Some(Error::BufferTooShort(b)) => b,
            other => panic!("expected BufferTooShort, got {other:?}"),
        };
        assert_eq!(b.need, 8);
        assert_eq!(b.have, 3);
        assert_eq!(b.what, "container child header");
    }

    /// `ChildOrder::is_empty` is the public way to ask whether a container
    /// recorded a wire order (hand-built) or came off the wire (parsed).
    #[test]
    fn child_order_is_empty_only_before_a_child_is_recorded() {
        let mut order = ChildOrder::default();
        assert!(order.is_empty());
        order.append_opaque(*b"free");
        assert!(!order.is_empty());

        let hand_built = MovieBox::new(sample_mvhd_v0(), alloc::vec![], None, alloc::vec![]);
        assert!(hand_built.order.is_empty(), "hand-built moov has no order");
        let bytes = hand_built.try_to_bytes().unwrap();
        let parsed = MovieBox::parse(&bytes).unwrap();
        assert!(
            !parsed.order.is_empty(),
            "a parsed moov must record the wire order it read"
        );
    }

    /// A `url ` data entry's `location` is an external reference
    /// (ISO/IEC 14496-12:2026 §8.7.2.2); dropping it rewrites the file as
    /// self-contained and loses the data reference.
    #[test]
    fn url_data_entry_keeps_its_external_location() {
        let url = DataEntryUrlBox {
            version: 0,
            flags: 0,
            location: b"http://example.com/a.mp4\0".to_vec(),
        };
        assert_eq!(url.serialized_len(), 12 + 25);
        let mut buf = alloc::vec![0u8; url.serialized_len()];
        let n = url.serialize_into(&mut buf).unwrap();
        assert_eq!(n, 37);
        assert_eq!(&buf[4..8], b"url ");
        assert_eq!(&buf[12..], b"http://example.com/a.mp4\0");
        assert_eq!(DataEntryUrlBox::parse(&buf).unwrap(), url);

        let dref = DataReferenceBox {
            version: 0,
            flags: 0,
            entries: alloc::vec![DataEntry::Url(url)],
        };
        let bytes = dref.try_to_bytes().unwrap();
        let parsed = DataReferenceBox::parse(&bytes).unwrap();
        assert_eq!(parsed.entries.len(), 1);
        let DataEntry::Url(entry) = &parsed.entries[0] else {
            panic!("expected a `url ` entry, got {:?}", parsed.entries[0]);
        };
        assert_eq!(entry.location, b"http://example.com/a.mp4\0");
        assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
    }

    /// `co64` is the chunk-offset table used past 4 GiB (§8.7.5.2): every
    /// entry is 8 bytes wide and the box length must follow them.
    #[test]
    fn co64_entries_are_eight_bytes_wide() {
        let co64 = ChunkLargeOffsetBox {
            version: 0,
            flags: 0,
            entries: alloc::vec![0x1_0000_0000, 0x1122_3344_5566_7788],
        };
        assert_eq!(co64.serialized_len(), 8 + 4 + 4 + 16);
        let mut buf = alloc::vec![0u8; co64.serialized_len()];
        let n = co64.serialize_into(&mut buf).unwrap();
        assert_eq!(n, 32);
        let mut expected = alloc::vec![0, 0, 0, 32, b'c', b'o', b'6', b'4', 0, 0, 0, 0];
        expected.extend_from_slice(&2u32.to_be_bytes());
        expected.extend_from_slice(&0x1_0000_0000u64.to_be_bytes());
        expected.extend_from_slice(&0x1122_3344_5566_7788u64.to_be_bytes());
        assert_eq!(buf, expected);
        assert_eq!(ChunkLargeOffsetBox::parse(&buf).unwrap(), co64);
    }

    /// `AudioSampleEntryV1`'s `samplerate` field is the placeholder `1 << 16`
    /// (ISO/IEC 14496-12:2026 §12.2.3.2), with the real rate in `srat`.
    #[test]
    fn audio_sample_entry_v1_writes_the_samplerate_placeholder() {
        assert_eq!(AudioSampleEntryV1::SAMPLERATE_PLACEHOLDER, 0x0001_0000);
        let entry = FlacSampleEntry {
            reserved: [0; 6],
            entry_version: AudioSampleEntryV1::ENTRY_VERSION,
            reserved_1: [0; 6],
            data_reference_index: 1,
            channelcount: 2,
            samplesize: 24,
            compression_id_and_packet_size: [0; 4],
            samplerate: AudioSampleEntryV1::SAMPLERATE_PLACEHOLDER,
            quicktime: None,
            config_boxes: alloc::vec![],
            trailing: alloc::vec![],
        };
        assert_eq!(entry.entry_version, 1);
        let mut buf = alloc::vec![0u8; entry.serialized_len()];
        let _ = entry.serialize_into(&mut buf).unwrap();
        assert_eq!(&buf[16..18], &[0x00, 0x01], "entry_version");
        assert_eq!(&buf[32..36], &[0x00, 0x01, 0x00, 0x00], "samplerate");
    }

    /// `stsc` entries are 12 bytes (§8.7.4.2) and every field must come from its
    /// own offset; the four-CCs of a declared-but-absent entry are a shortfall,
    /// not a silently shorter table.
    #[test]
    fn stsc_entries_read_their_own_fields_and_a_short_table_errors() {
        let mut body = alloc::vec![0u8, 0, 0, 0];
        body.extend_from_slice(&3u32.to_be_bytes());
        for chunk in 1..=3u32 {
            body.extend_from_slice(&(0x1100_0000 + chunk).to_be_bytes());
            body.extend_from_slice(&(0x2200_0000 + chunk).to_be_bytes());
            body.extend_from_slice(&(0x3300_0000 + chunk).to_be_bytes());
        }
        let mut bytes = alloc::vec![0u8; 8];
        bytes[0..4].copy_from_slice(&((8 + body.len()) as u32).to_be_bytes());
        bytes[4..8].copy_from_slice(b"stsc");
        bytes.extend_from_slice(&body);
        let parsed = SampleToChunkBox::parse(&bytes).unwrap();
        assert_eq!(parsed.entries.len(), 3);
        for (i, entry) in parsed.entries.iter().enumerate() {
            let chunk = i as u32 + 1;
            assert_eq!(entry.first_chunk, 0x1100_0000 + chunk);
            assert_eq!(entry.samples_per_chunk, 0x2200_0000 + chunk);
            assert_eq!(entry.sample_description_index, 0x3300_0000 + chunk);
        }
        assert_eq!(parsed.try_to_bytes().unwrap(), bytes);

        let mut short = bytes.clone();
        short.truncate(16 + 24);
        short[0..4].copy_from_slice(&((16 + 24) as u32).to_be_bytes());
        match SampleToChunkBox::parse(&short) {
            Err(Error::BufferTooShort(b)) => {
                assert_eq!(
                    (b.need, b.have, b.what),
                    (60, 24, "SampleToChunkBox.entry_count")
                );
            }
            other => panic!("expected BufferTooShort, got {other:?}"),
        }
    }

    /// `stsz` with `sample_size == 0` carries a 4-byte per-sample table
    /// (§8.7.3.2); a table one entry short of `sample_count` is a malformed
    /// box, and the last entry that does fit must be read.
    #[test]
    fn stsz_per_sample_sizes_are_read_to_the_last_entry() {
        let mut body = alloc::vec![0u8, 0, 0, 0];
        body.extend_from_slice(&0u32.to_be_bytes());
        body.extend_from_slice(&3u32.to_be_bytes());
        for size in [0x1102_0304u32, 0x2132_4354, 0x3162_7384] {
            body.extend_from_slice(&size.to_be_bytes());
        }
        let mut bytes = alloc::vec![0u8; 8];
        bytes[0..4].copy_from_slice(&((8 + body.len()) as u32).to_be_bytes());
        bytes[4..8].copy_from_slice(b"stsz");
        bytes.extend_from_slice(&body);
        let parsed = SampleSizeBox::parse(&bytes).unwrap();
        assert_eq!(parsed.sample_size, 0);
        assert_eq!(parsed.sample_count, 3);
        assert_eq!(
            parsed.entries,
            alloc::vec![0x1102_0304, 0x2132_4354, 0x3162_7384]
        );
        assert_eq!(parsed.try_to_bytes().unwrap(), bytes);

        let mut short = bytes.clone();
        short.truncate(20 + 10);
        short[0..4].copy_from_slice(&(20 + 10u32).to_be_bytes());
        match SampleSizeBox::parse(&short) {
            Err(Error::BufferTooShort(b)) => {
                assert_eq!(
                    (b.need, b.have, b.what),
                    (22, 10, "SampleSizeBox sample_count")
                );
            }
            other => panic!("expected BufferTooShort, got {other:?}"),
        }
    }

    /// A `dref` whose last child is a truncated 8-byte box is an error, not a
    /// table that silently stops one entry early (§8.7.2.1).
    #[test]
    fn dref_last_child_shorter_than_its_own_body_errors() {
        let mut body = alloc::vec![0u8, 0, 0, 0, 0, 0, 0, 2];
        body.extend_from_slice(&12u32.to_be_bytes());
        body.extend_from_slice(b"url ");
        body.extend_from_slice(&[0, 0, 0, 1]);
        body.extend_from_slice(&8u32.to_be_bytes());
        body.extend_from_slice(b"urn ");
        let mut bytes = alloc::vec![0u8; 8];
        bytes[0..4].copy_from_slice(&((8 + body.len()) as u32).to_be_bytes());
        bytes[4..8].copy_from_slice(b"dref");
        bytes.extend_from_slice(&body);
        match DataReferenceBox::parse(&bytes) {
            Err(Error::BufferTooShort(b)) => {
                assert_eq!((b.need, b.have, b.what), (12, 8, "expected urn "));
            }
            other => panic!("expected BufferTooShort, got {other:?}"),
        }

        for short_len in 8..12usize {
            match DataEntryUrnBox::parse(&[0u8; 12][..short_len]) {
                Err(Error::BufferTooShort(b)) => {
                    assert_eq!(b.need, 12, "urn at {short_len} bytes");
                    assert_eq!(b.have, short_len);
                    assert_eq!(b.what, "expected urn ");
                }
                other => panic!("urn at {short_len} bytes: expected BufferTooShort, got {other:?}"),
            }
        }
    }

    #[test]
    fn stsc_round_trip() {
        let s = SampleToChunkBox {
            version: 0,
            flags: 0,
            entries: alloc::vec![StscEntry {
                first_chunk: 1,
                samples_per_chunk: 10,
                sample_description_index: 1
            },],
        };
        let bytes = s.try_to_bytes().unwrap();
        let parsed = SampleToChunkBox::parse(&bytes).unwrap();
        assert_eq!(parsed, s);
    }

    #[test]
    fn stsz_uniform_round_trip() {
        // Uniform sample_size (constant-size samples): the wire still
        // carries a real sample_count even though `entries` stays empty
        // (issue #1018) — a real MP4Box/ffmpeg-produced fixture asserting
        // this against an independent oracle lives in
        // `tests/it/cenc_saiz_stsz_v1_boxes.rs`.
        let s = SampleSizeBox {
            version: 0,
            flags: 0,
            sample_size: 512,
            sample_count: 88,
            entries: alloc::vec![],
        };
        let bytes = s.try_to_bytes().unwrap();
        let parsed = SampleSizeBox::parse(&bytes).unwrap();
        assert_eq!(parsed, s);
        assert_eq!(
            parsed.sample_count, 88,
            "uniform stsz must keep its real sample_count"
        );
    }

    #[test]
    fn stco_round_trip() {
        let s = ChunkOffsetBox {
            version: 0,
            flags: 0,
            entries: alloc::vec![0, 1024, 2048, 4096],
        };
        let bytes = s.try_to_bytes().unwrap();
        let parsed = ChunkOffsetBox::parse(&bytes).unwrap();
        assert_eq!(parsed, s);
    }

    /// A `dref` of `urn `, `imdt` and `snim` entries (ISO/IEC 14496-12:2026
    /// §8.7.2.2): re-serializing must keep each entry's four-CC and body.
    fn dref_with_urn_imdt_snim() -> Vec<u8> {
        let mut body = alloc::vec![0, 0, 0, 0, 0, 0, 0, 3];
        body.extend_from_slice(&[0, 0, 0, 18, b'u', b'r', b'n', b' ', 0, 0, 0, 0]);
        body.extend_from_slice(b"nm\0lc\0");
        body.extend_from_slice(&[0, 0, 0, 16, b'i', b'm', b'd', b't', 0, 0, 0, 0, 0, 0, 0, 7]);
        body.extend_from_slice(&[0, 0, 0, 12, b's', b'n', b'i', b'm', 0, 0, 0, 0]);
        let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(b"dref");
        out.extend_from_slice(&body);
        out
    }

    #[test]
    fn dref_keeps_urn_imdt_snim_entry_types() {
        let bytes = dref_with_urn_imdt_snim();
        let parsed = DataReferenceBox::parse(&bytes).unwrap();
        assert_eq!(
            parsed.entries,
            alloc::vec![
                DataEntry::Urn(DataEntryUrnBox {
                    version: 0,
                    flags: 0,
                    name: b"nm".to_vec(),
                    location: b"lc\0".to_vec(),
                }),
                DataEntry::Imda(DataEntryImdaBox {
                    version: 0,
                    flags: 0,
                    imda_ref_identifier: 7,
                }),
                DataEntry::SeqNumImda(DataEntrySeqNumImdaBox {
                    version: 0,
                    flags: 0
                }),
            ]
        );
        assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
    }

    #[test]
    fn dref_unmodelled_entry_type_round_trips_and_urn_without_name_terminator_errors() {
        let mut body = alloc::vec![0, 0, 0, 0, 0, 0, 0, 1];
        body.extend_from_slice(&[0, 0, 0, 14, b'a', b'l', b'i', b's', 0, 0, 0, 1, 0xAB, 0xCD]);
        let mut bytes = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        bytes.extend_from_slice(b"dref");
        bytes.extend_from_slice(&body);
        let parsed = DataReferenceBox::parse(&bytes).unwrap();
        assert!(matches!(parsed.entries[0], DataEntry::Other(_)));
        assert_eq!(parsed.try_to_bytes().unwrap(), bytes);

        let urn = [0, 0, 0, 14, b'u', b'r', b'n', b' ', 0, 0, 0, 0, b'a', b'b'];
        assert!(DataEntryUrnBox::parse(&urn).is_err());
    }

    #[test]
    fn dref_url_round_trip() {
        let url = DataEntryUrlBox {
            version: 0,
            flags: 1,
            location: alloc::vec![],
        };
        let dref = DataReferenceBox {
            version: 0,
            flags: 0,
            entries: alloc::vec![DataEntry::Url(url)],
        };
        let bytes = dref.try_to_bytes().unwrap();
        let parsed = DataReferenceBox::parse(&bytes).unwrap();
        assert_eq!(parsed, dref);
    }

    /// Audit finding #3: a `stbl` child that fails to parse must survive as
    /// [`StblChild::Opaque`] (its real bytes, so the real error is
    /// recoverable at the point of use — see
    /// `progressive_demux::find_stbl_child`), never as a defaulted-empty
    /// typed box that falsely claims the table has zero entries.
    ///
    /// One self-contained `stbl` body: a bare 8-byte `stsc` header (well
    /// below `SampleToChunkBox::parse`'s own 16-byte minimum, so it is
    /// guaranteed to fail to parse) followed by nothing else — no sibling
    /// boxes to keep aligned, so this pins the `parse_stbl_children` behaviour
    /// in isolation from any other box's layout.
    #[test]
    fn parse_stbl_children_keeps_malformed_box_as_opaque_not_defaulted_empty() {
        let mut body = alloc::vec![0u8; 8];
        body[0..4].copy_from_slice(&8u32.to_be_bytes());
        body[4..8].copy_from_slice(b"stsc");

        let children = parse_stbl_children(&body).unwrap();
        assert_eq!(children.len(), 1);
        match &children[0] {
            StblChild::Opaque(raw) => assert_eq!(raw.as_slice(), body.as_slice()),
            StblChild::Stsc(b) => panic!(
                "a malformed stsc must survive as Opaque, not a defaulted-empty typed box \
                 (got StblChild::Stsc with {} entries)",
                b.entries.len()
            ),
            other => panic!("expected StblChild::Opaque or StblChild::Stsc, got {other:?}"),
        }
    }

    /// The same walk over a well-formed `stsc` (entry_count = 0, the smallest
    /// box that still meets the 16-byte minimum) must still produce the typed
    /// variant — the fix must not have over-tightened parsing of genuinely
    /// valid boxes.
    #[test]
    fn parse_stbl_children_well_formed_stsc_stays_typed() {
        let mut body = alloc::vec![0u8; 16];
        body[0..4].copy_from_slice(&16u32.to_be_bytes());
        body[4..8].copy_from_slice(b"stsc");
        // version/flags already zero; entry_count (bytes 12..16) already zero.

        let children = parse_stbl_children(&body).unwrap();
        assert_eq!(children.len(), 1);
        match &children[0] {
            StblChild::Stsc(b) => assert!(b.entries.is_empty()),
            other => panic!("expected a well-formed empty StblChild::Stsc, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // bounded_entry_count / Vec::with_capacity DoS (audit finding #4)
    // -----------------------------------------------------------------------

    /// The audit's own scenario: a 16-byte `co64` (`BOX_HDR` + `FULL_HDR` +
    /// the 4-byte count field — no room left for even one 8-byte entry)
    /// declaring `count = 0xFFFFFFFF`. The *naive* `Vec::with_capacity(count)`
    /// would ask the allocator for ~32 GB; the bound must refuse that
    /// request outright, not merely complete it.
    #[test]
    fn bounded_entry_count_refuses_the_audits_co64_scenario() {
        let hostile_count = 0xFFFF_FFFFusize;
        let remaining = 0usize; // nothing left in a 16-byte co64 body
        let entry_len = 8usize; // co64 entries are 8-byte u64 offsets

        let bounded = bounded_entry_count(remaining, entry_len, hostile_count);

        let naive_bytes = hostile_count as u64 * entry_len as u64;
        assert!(
            naive_bytes > 30_000_000_000,
            "sanity check on the scenario itself: the naive request really is ~32 GB \
             ({naive_bytes} bytes)"
        );
        assert_eq!(bounded, 0, "no bytes remain for even one entry");
        assert!(
            (bounded as u64) * (entry_len as u64) < 1024,
            "the fix must request a bounded, sane allocation instead of {naive_bytes} bytes"
        );
    }

    /// The bound must not be so aggressive that it under-serves a
    /// legitimately-sized body: exactly as many entries fit as the remaining
    /// bytes can hold, and a genuinely smaller count is passed through
    /// untouched.
    #[test]
    fn bounded_entry_count_never_preallocates_a_zero_width_count() {
        assert_eq!(
            bounded_entry_count(0, 0, 0xFFFF_FFFF),
            crate::box_support::ZERO_WIDTH_PREALLOC
        );
        assert_eq!(bounded_entry_count(0, 0, 7), 7);
    }

    #[test]
    fn bounded_entry_count_allows_exactly_what_fits() {
        assert_eq!(bounded_entry_count(40, 8, 0xFFFF_FFFF), 5);
        assert_eq!(
            bounded_entry_count(40, 8, 3),
            3,
            "a smaller, wire-verifiable count must pass through untouched"
        );
    }

    /// End-to-end: `ChunkLargeOffsetBox::parse` on the audit's exact `co64`
    /// scenario must reject the box rather than attempt to preallocate ~32 GB
    /// — `entry_count = 0xFFFFFFFF` cannot fit a 16-byte body (audit
    /// r05-W13: the old loop `break`ed and returned `Ok` with no entries).
    #[test]
    fn co64_hostile_count_is_rejected() {
        let mut body = alloc::vec![0u8; 16];
        body[0..4].copy_from_slice(&16u32.to_be_bytes());
        body[4..8].copy_from_slice(b"co64");
        body[12..16].copy_from_slice(&0xFFFF_FFFFu32.to_be_bytes());

        let err = ChunkLargeOffsetBox::parse(&body)
            .expect_err("a count the body cannot hold is malformed");
        assert!(
            matches!(err, Error::BufferTooShort(..)),
            "expected BufferTooShort, got {err:?}"
        );
    }

    /// `stsz` with a nonzero uniform `sample_size` never populates `entries`
    /// at all (§8.7.3) — a hostile `count` in that branch must drive *no*
    /// allocation whatsoever, not merely a bounded one.
    #[test]
    fn stsz_uniform_sample_size_ignores_hostile_count() {
        let mut body = alloc::vec![0u8; 20];
        body[0..4].copy_from_slice(&20u32.to_be_bytes());
        body[4..8].copy_from_slice(b"stsz");
        body[12..16].copy_from_slice(&64u32.to_be_bytes()); // uniform sample_size
        body[16..20].copy_from_slice(&0xFFFF_FFFFu32.to_be_bytes()); // hostile count

        let parsed = SampleSizeBox::parse(&body).expect("a well-formed-enough header parses");
        assert_eq!(parsed.sample_size, 64);
        assert!(parsed.entries.is_empty());
    }

    /// r04-W45: every serializer must write its reserved/`pre_defined` bytes as
    /// zeros rather than skipping them, because `Serialize` accepts any
    /// `&mut [u8]` — a reused or dirty caller buffer would otherwise emit
    /// garbage that a conformance validator rejects. Serializing into a
    /// 0xFF-filled buffer must be byte-identical to `to_bytes()` (which
    /// zero-fills). Unfixed, the hdlr `pre_defined`/`reserved`, the smhd
    /// `reserved`, and the `mdhd` quality field all differed.
    #[test]
    fn reserved_bytes_written_as_zeros_into_a_dirty_buffer() {
        fn dirty_matches<T: Serialize>(value: &T, what: &str)
        where
            <T as Serialize>::Error: core::fmt::Debug,
        {
            let clean = value.try_to_bytes().unwrap();
            let mut dirty = alloc::vec![0xFFu8; clean.len()];
            let written = value
                .serialize_into(&mut dirty)
                .expect("serialize into a correctly-sized buffer");
            assert_eq!(written, clean.len(), "{what}: serializer length");
            assert_eq!(
                dirty, clean,
                "{what}: reserializing into a 0xFF-filled buffer must equal to_bytes()"
            );
        }

        dirty_matches(&sample_mvhd_v0(), "mvhd");
        // mvhd v1 (the 64-bit form, issue #1015's 120-byte layout) has its own
        // reserved(10) + pre_defined(24) regions.
        let mut mvhd_v1 = sample_mvhd_v0();
        mvhd_v1.version = 1;
        dirty_matches(&mvhd_v1, "mvhd v1");
        dirty_matches(
            &MediaHeaderBox {
                version: 0,
                flags: 0,
                creation_time: 1,
                modification_time: 2,
                timescale: timescale(90_000),
                duration: 180_000,
                language: 0x55C4,
            },
            "mdhd",
        );
        dirty_matches(
            &HandlerBox {
                version: 0,
                flags: 0,
                pre_defined: 0,
                reserved: [0u8; 12],
                handler_type: *b"vide",
                name: alloc::vec![b'v', b'i', b'd', 0],
            },
            "hdlr",
        );
        dirty_matches(
            &SoundMediaHeaderBox {
                version: 0,
                flags: 0,
                balance: 0,
            },
            "smhd",
        );
        dirty_matches(
            &VideoMediaHeaderBox {
                version: 0,
                flags: 0,
                graphicsmode: 0,
                opcolor: [0, 0, 0],
            },
            "vmhd",
        );
        // tkhd v0 and v1 both carry reserved(4)+reserved[2](8)+reserved(2).
        for version in [0u8, 1] {
            dirty_matches(
                &TrackHeaderBox {
                    version,
                    flags: 0x000007,
                    creation_time: 1,
                    modification_time: 2,
                    track_id: TrackId::new(1),
                    duration: 1000,
                    layer: 0,
                    alternate_group: 0,
                    volume: 0x0100,
                    matrix: [0x00010000, 0, 0, 0, 0x00010000, 0, 0, 0, 0x40000000],
                    width: 640 << 16,
                    height: 360 << 16,
                },
                "tkhd",
            );
        }
    }

    /// The shared audio-sample-entry writer (`mp4a`/`ac-3`/`ec-3`/`Opus`/`fLaC`)
    /// skips 6 + 8 + 4 reserved/`pre_defined` bytes; each must be written as
    /// zeros (r04-W45). Serializing into a 0xFF-filled buffer must equal
    /// serializing into a fresh zeroed one.
    #[test]
    fn audio_sample_entry_reserved_bytes_written_as_zeros_into_a_dirty_buffer() {
        const NEED: usize = 36;
        let config_boxes: [OpaqueBox; 0] = [];
        for fourcc in [b"mp4a", b"ac-3", b"ec-3", b"Opus", b"fLaC"] {
            let mut clean = alloc::vec![0u8; NEED];
            let clean_len = serialize_audio_sample_entry(
                &mut clean,
                AudioSampleEntryFields {
                    fourcc,
                    reserved: [0u8; 6],
                    entry_version: 0,
                    reserved_1: [0u8; 6],
                    data_reference_index: 1,
                    channelcount: 2,
                    samplesize: 16,
                    compression_id_and_packet_size: [0u8; 4],
                    samplerate: 48_000 << 16,
                    quicktime: &None,
                    config_boxes: &config_boxes,
                    trailing: &[],
                },
            )
            .expect("serialize audio sample entry");
            assert_eq!(clean_len, NEED);

            let mut dirty = alloc::vec![0xFFu8; NEED];
            let dirty_len = serialize_audio_sample_entry(
                &mut dirty,
                AudioSampleEntryFields {
                    fourcc,
                    reserved: [0u8; 6],
                    entry_version: 0,
                    reserved_1: [0u8; 6],
                    data_reference_index: 1,
                    channelcount: 2,
                    samplesize: 16,
                    compression_id_and_packet_size: [0u8; 4],
                    samplerate: 48_000 << 16,
                    quicktime: &None,
                    config_boxes: &config_boxes,
                    trailing: &[],
                },
            )
            .expect("serialize into a dirty buffer");
            assert_eq!(dirty_len, NEED);
            assert_eq!(
                dirty, clean,
                "{fourcc:?}: a 0xFF-filled buffer must yield the same bytes"
            );
            // The reserved ranges are literally zero in the output.
            assert_eq!(&clean[8..14], &[0u8; 6], "{fourcc:?}: 6 reserved bytes");
            assert_eq!(
                &clean[16..24],
                &[0u8; 8],
                "{fourcc:?}: AudioSampleEntry reserved[2]"
            );
            assert_eq!(&clean[28..32], &[0u8; 4], "{fourcc:?}: predefined+reserved");
        }
    }
}

#[cfg(test)]
mod compact_size_and_dependency_tests {
    use super::*;
    use broadcast_core::Serialize;

    fn stz2_box(field_size: u8, sample_count: u32, payload: &[u8]) -> Vec<u8> {
        let len = STZ2_PREFIX_LEN + payload.len();
        let mut out = Vec::with_capacity(len);
        out.extend_from_slice(&(len as u32).to_be_bytes());
        out.extend_from_slice(b"stz2");
        out.extend_from_slice(&[0u8; FULLBOX_EXTRA_SIZE]);
        out.extend_from_slice(&[0u8; STZ2_RESERVED_LEN]);
        out.push(field_size);
        out.extend_from_slice(&sample_count.to_be_bytes());
        out.extend_from_slice(payload);
        out
    }

    fn assert_stz2_round_trip(field_size: u8, entries: &[u32], payload: &[u8]) {
        let bytes = stz2_box(field_size, entries.len() as u32, payload);
        let parsed = CompactSampleSizeBox::parse(&bytes).expect("a well-formed stz2");
        assert_eq!(parsed.field_size, field_size);
        assert_eq!(parsed.sample_count, entries.len() as u32);
        assert_eq!(parsed.entries, entries);
        let mut out = alloc::vec![0u8; parsed.serialized_len()];
        let written = parsed.serialize_into(&mut out).expect("serialize");
        assert_eq!(written, out.len());
        assert_eq!(out, bytes);
    }

    #[test]
    fn stz2_round_trips_at_each_field_size() {
        // Two 4-bit entries share one byte, the first in the high nibble; an
        // odd count pads the last byte with zeros (§8.7.3.3.2).
        assert_stz2_round_trip(4, &[1, 15, 7], &[0x1F, 0x70]);
        assert_stz2_round_trip(8, &[200, 3], &[200, 3]);
        assert_stz2_round_trip(16, &[1000, 65535], &[0x03, 0xE8, 0xFF, 0xFF]);
    }

    #[test]
    fn stz2_rejects_an_unsupported_field_size() {
        let bytes = stz2_box(5, 1, &[0]);
        assert!(matches!(
            CompactSampleSizeBox::parse(&bytes),
            Err(Error::InvalidValue(broadcast_core::error::InvalidValue {
                field: "stz2.field_size",
                ..
            }))
        ));
    }

    #[test]
    fn stz2_rejects_a_truncated_size_table() {
        let bytes = stz2_box(16, 2, &[0, 1]);
        assert!(matches!(
            CompactSampleSizeBox::parse(&bytes),
            Err(Error::BufferTooShort(_))
        ));
    }

    fn sdtp_box(entries: &[u8]) -> Vec<u8> {
        let len = FULLBOX_BODY_OFFSET + entries.len();
        let mut out = Vec::with_capacity(len);
        out.extend_from_slice(&(len as u32).to_be_bytes());
        out.extend_from_slice(b"sdtp");
        out.extend_from_slice(&[0u8; FULLBOX_EXTRA_SIZE]);
        out.extend_from_slice(entries);
        out
    }

    fn find_sdtp_box(file: &[u8]) -> Option<&[u8]> {
        let needle = b"sdtp";
        let mut i = core::mem::size_of::<u32>();
        while i + needle.len() <= file.len() {
            if &file[i..i + needle.len()] == needle {
                let size = u32::from_be_bytes([file[i - 4], file[i - 3], file[i - 2], file[i - 1]])
                    as usize;
                if size >= FULLBOX_BODY_OFFSET && i - 4 + size <= file.len() {
                    return Some(&file[i - 4..i - 4 + size]);
                }
            }
            i += 1;
        }
        None
    }

    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| matches!(e, "mp4" | "m4a" | "mov"))
            {
                out.push(path);
            }
        }
    }

    #[test]
    fn sdtp_round_trips_a_real_box() {
        let mut files = Vec::new();
        walk(&test_fixtures::root().join("mp4"), &mut files);
        if let Some(private) = test_fixtures::private("mp4") {
            walk(&private, &mut files);
        }
        let mut checked = 0usize;
        for path in files {
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            let Some(box_bytes) = find_sdtp_box(&bytes) else {
                continue;
            };
            let parsed = SampleDependencyTypeBox::parse(box_bytes)
                .unwrap_or_else(|e| panic!("{}: sdtp parse: {e}", path.display()));
            let mut out = alloc::vec![0u8; parsed.serialized_len()];
            let written = parsed.serialize_into(&mut out).expect("serialize");
            assert_eq!(written, out.len());
            assert_eq!(
                out,
                box_bytes,
                "{}: sdtp must serialize byte-identically",
                path.display()
            );
            checked += 1;
        }
        if checked == 0 {
            eprintln!(
                "SKIPPED: no fixture under fixtures/mp4 or private/fixtures/mp4 has an sdtp box"
            );
        }
    }

    #[test]
    fn sdtp_exposes_the_four_two_bit_fields() {
        // is_leading=1, depends_on=2, is_depended_on=0, has_redundancy=3.
        let raw = (1u8 << SDTP_IS_LEADING_SHIFT)
            | (2u8 << SDTP_SAMPLE_DEPENDS_ON_SHIFT)
            | (0u8 << SDTP_SAMPLE_IS_DEPENDED_ON_SHIFT)
            | (3u8 << SDTP_SAMPLE_HAS_REDUNDANCY_SHIFT);
        let parsed = SampleDependencyTypeBox::parse(&sdtp_box(&[raw])).expect("parse");
        assert_eq!(parsed.is_leading(0), Some(1));
        assert_eq!(parsed.sample_depends_on(0), Some(2));
        assert_eq!(parsed.sample_is_depended_on(0), Some(0));
        assert_eq!(parsed.sample_has_redundancy(0), Some(3));
        assert_eq!(parsed.is_leading(1), None);
    }

    #[test]
    fn stz2_and_sdtp_round_trip_through_an_stbl() {
        let stz2 = stz2_box(8, 2, &[4, 5]);
        let sdtp = sdtp_box(&[0x24, 0x18]);
        let mut body = Vec::new();
        body.extend_from_slice(&stz2);
        body.extend_from_slice(&sdtp);
        let mut whole = Vec::with_capacity(BOX_HEADER_MIN_SIZE + body.len());
        whole.extend_from_slice(&((BOX_HEADER_MIN_SIZE + body.len()) as u32).to_be_bytes());
        whole.extend_from_slice(b"stbl");
        whole.extend_from_slice(&body);
        let parsed = SampleTableBox::parse(&whole).expect("stbl");
        assert!(matches!(parsed.children[0], StblChild::Stz2(_)));
        assert!(matches!(parsed.children[1], StblChild::Sdtp(_)));
        let mut out = alloc::vec![0u8; parsed.serialized_len()];
        parsed.serialize_into(&mut out).expect("serialize");
        assert_eq!(out, whole);
    }
}
