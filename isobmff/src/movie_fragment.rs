//! Movie Fragment boxes - ISO/IEC 14496-12:2026 §8.8.
//!
//! Typed containers for fMP4 segment boxes: `moof` (Movie Fragment | §8.8.2)
//! containing `mfhd` (Movie Fragment Header | §8.8.2) and one or more `traf`
//! (Track Fragment | §8.8.3). Each `traf` contains `tfhd` (Track Fragment Header |
//! §8.8.7), optional `tfdt` (Base Media Decode Time | §8.8.12),
//! and one or more `trun` (Track Fragment Run | §8.8.8).
//!
//! Field presence in `tfhd` and `trun` is **flag-driven** (§8.8.7, §8.8.8):
//! the `flags` field of the FullBox header determines which optional fields appear
//! on the wire. The serializer recomputes every size from the logical fields
//! (no `self.raw`).

use crate::TrackId;
use crate::box_types::{
    BOX_HEADER_MIN_SIZE, BOX_TYPE_SIZE, FULLBOX_EXTRA_SIZE, FULLBOX_FLAGS_MASK, LARGESIZE_SIZE,
    box_len, write_full_box_header,
};
use crate::error::{Error, Result};
use crate::init_segment::TrackExtendsBox;
use alloc::vec::Vec;
use broadcast_core::error::BufferTooShort;
use broadcast_core::len::{need_in, need_out};
use broadcast_core::wire::{be_u32, be_u64, read_u32, u24_at};
use broadcast_core::{Parse, Serialize};

const TRUN_SAMPLE_FIELD_COUNT: usize = 4;
const CENC_AUX_INFO_TYPE_COUNT: usize = 4;
const TRAF_FIXED_CHILD_COUNT: usize = 2;
const AUX_INFO_TYPE_PRESENT_FLAG: u32 = 0x01;

// tfhd flags — ISO/IEC 14496-12:2026 §8.8.7.1 (public: builders set these).
/// `tfhd` flag: `base_data_offset` field present.
pub const TFHD_BASE_DATA_OFFSET_PRESENT: u32 = 0x000001;
/// `tfhd` flag: `sample_description_index` field present.
pub const TFHD_SAMPLE_DESCRIPTION_INDEX_PRESENT: u32 = 0x000002;
/// `tfhd` flag: `default_sample_duration` field present.
pub const TFHD_DEFAULT_SAMPLE_DURATION_PRESENT: u32 = 0x000008;
/// `tfhd` flag: `default_sample_size` field present.
pub const TFHD_DEFAULT_SAMPLE_SIZE_PRESENT: u32 = 0x000010;
/// `tfhd` flag: `default_sample_flags` field present.
pub const TFHD_DEFAULT_SAMPLE_FLAGS_PRESENT: u32 = 0x000020;

/// `tfhd` flag: duration-is-empty (no samples in this fragment for the track).
pub const TFHD_DURATION_IS_EMPTY: u32 = 0x010000;
/// `tfhd` flag: base offset is the containing `moof` (CMAF default).
pub const TFHD_DEFAULT_BASE_IS_MOOF: u32 = 0x020000;

// trun flags — ISO/IEC 14496-12:2026 §8.8.8.1 (public: builders set these).
/// `trun` flag: `data_offset` field present.
pub const TRUN_DATA_OFFSET_PRESENT: u32 = 0x000001;
/// `trun` flag: `first_sample_flags` field present.
pub const TRUN_FIRST_SAMPLE_FLAGS_PRESENT: u32 = 0x000004;
/// `trun` flag: per-sample `sample_duration` present.
pub const TRUN_SAMPLE_DURATION_PRESENT: u32 = 0x000100;
/// `trun` flag: per-sample `sample_size` present.
pub const TRUN_SAMPLE_SIZE_PRESENT: u32 = 0x000200;
/// `trun` flag: per-sample `sample_flags` present.
pub const TRUN_SAMPLE_FLAGS_PRESENT: u32 = 0x000400;
/// `trun` flag: per-sample `sample_composition_time_offset` present.
pub const TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT: u32 = 0x000800;

// `sample_flags` words (ISO/IEC 14496-12:2026 §8.8.3.1) shared by the fragment writers.
/// `sample_flags` for a sync sample (I-frame): `sample_depends_on = 2` (does not
/// depend on others), `sample_is_non_sync_sample = 0` — ISO/IEC 14496-12:2026
/// §8.8.3.1. The one copy every fragment builder writes (audit r05-O6).
pub const SAMPLE_FLAGS_SYNC: u32 = 0x0200_0000;
/// `sample_flags` for a non-sync sample: `sample_depends_on = 1`,
/// `sample_is_non_sync_sample = 1` (§8.8.3.1).
pub const SAMPLE_FLAGS_NON_SYNC: u32 = 0x0101_0000;
/// `sample_is_non_sync_sample` bit within a 32-bit `sample_flags` word
/// (§8.8.3.1, bit `[16]`). Set = the sample is **not** a sync sample.
pub const SAMPLE_FLAG_IS_NON_SYNC: u32 = 0x0001_0000;

/// Read version(8) and flags(24) from the body bytes (first 4 bytes of a FullBox payload).
fn read_ver_flags(body: &[u8]) -> Result<(u8, u32)> {
    need_in(body, core::mem::size_of::<u32>(), "FullBox version/flags")?;
    let ver = body[0];
    let flags = be_u32(body, 0, "FullBox version/flags")? & FULLBOX_FLAGS_MASK;
    Ok((ver, flags))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MovieFragmentHeaderBox {
    pub version: u8,
    pub flags: u32,
    pub sequence_number: u32,
}

impl MovieFragmentHeaderBox {
    pub fn new(sequence_number: u32) -> Self {
        Self {
            version: 0,
            flags: 0,
            sequence_number,
        }
    }
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        let (version, flags) = read_ver_flags(body)?;
        let payload = &body[FULLBOX_EXTRA_SIZE..];
        need_in(payload, core::mem::size_of::<u32>(), "mfhd.seq")?;
        Ok(Self {
            version,
            flags,
            sequence_number: be_u32(payload, 0, "mfhd.seq")?,
        })
    }
}

impl Serialize for MovieFragmentHeaderBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let c = crate::box_types::write_full_box_header(
            buf,
            b"mfhd",
            need,
            self.version,
            self.flags,
            true,
        )?;
        buf[c..c + core::mem::size_of::<u32>()]
            .copy_from_slice(&self.sequence_number.to_be_bytes());
        Ok(c + core::mem::size_of::<u32>())
    }
}

/// Four-CC of the Track Fragment Header Box (`tfhd`, ISO/IEC 14496-12:2026 §8.8.7).
pub const TFHD: [u8; 4] = *b"tfhd";

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TrackFragmentHeaderBox {
    pub version: u8,
    pub flags: u32,
    pub track_id: TrackId,
    pub base_data_offset: Option<u64>,
    pub sample_description_index: Option<u32>,
    pub default_sample_duration: Option<u32>,
    pub default_sample_size: Option<u32>,
    pub default_sample_flags: Option<u32>,
    /// Bytes after the last field the flags name (§8.8.6.2), carried verbatim
    /// so a rewrite is exact. Not a field of the box.
    trailing: Vec<u8>,
}

impl TrackFragmentHeaderBox {
    /// A version-0 `tfhd` with no bytes past its declared fields.
    #[must_use]
    pub fn new(
        flags: u32,
        track_id: TrackId,
        base_data_offset: Option<u64>,
        sample_description_index: Option<u32>,
        default_sample_duration: Option<u32>,
        default_sample_size: Option<u32>,
        default_sample_flags: Option<u32>,
    ) -> Self {
        Self {
            version: 0,
            flags,
            track_id,
            base_data_offset,
            sample_description_index,
            default_sample_duration,
            default_sample_size,
            default_sample_flags,
            trailing: Vec::new(),
        }
    }

    /// The bytes after the last field the flags name, in wire order.
    #[must_use]
    pub fn trailing(&self) -> &[u8] {
        &self.trailing
    }

    /// Keep bytes the declared fields do not account for.
    #[must_use]
    pub fn with_trailing(mut self, trailing: Vec<u8>) -> Self {
        self.trailing = trailing;
        self
    }

    pub fn parse_body(body: &[u8]) -> Result<Self> {
        let (version, flags) = read_ver_flags(body)?;
        let mut c = FULLBOX_EXTRA_SIZE;
        need_in(body, c + core::mem::size_of::<u32>(), "tfhd.track_id")?;
        let tid = read_u32(body, &mut c, "tfhd.track_id")?;

        let v_bdo = if flags & TFHD_BASE_DATA_OFFSET_PRESENT != 0 {
            need_in(body, c + core::mem::size_of::<u64>(), "tfhd.bdo")?;
            let v = be_u64(body, c, "tfhd.bdo")?;
            c += core::mem::size_of::<u64>();
            Some(v)
        } else {
            None
        };
        let v_sdi = if flags & TFHD_SAMPLE_DESCRIPTION_INDEX_PRESENT != 0 {
            need_in(body, c + core::mem::size_of::<u32>(), "tfhd.sdi")?;
            let v = read_u32(body, &mut c, "tfhd.sdi")?;
            Some(v)
        } else {
            None
        };
        let v_dsd = if flags & TFHD_DEFAULT_SAMPLE_DURATION_PRESENT != 0 {
            need_in(body, c + core::mem::size_of::<u32>(), "tfhd.dsd")?;
            let v = read_u32(body, &mut c, "tfhd.dsd")?;
            Some(v)
        } else {
            None
        };
        let v_dss = if flags & TFHD_DEFAULT_SAMPLE_SIZE_PRESENT != 0 {
            need_in(body, c + core::mem::size_of::<u32>(), "tfhd.dss")?;
            let v = read_u32(body, &mut c, "tfhd.dss")?;
            Some(v)
        } else {
            None
        };
        let v_dsf = if flags & TFHD_DEFAULT_SAMPLE_FLAGS_PRESENT != 0 {
            need_in(body, c + core::mem::size_of::<u32>(), "tfhd.dsf")?;
            Some(read_u32(body, &mut c, "tfhd.dsf")?)
        } else {
            None
        };
        Ok(TrackFragmentHeaderBox {
            version,
            flags,
            track_id: TrackId::new(tid),
            base_data_offset: v_bdo,
            sample_description_index: v_sdi,
            default_sample_duration: v_dsd,
            default_sample_size: v_dss,
            default_sample_flags: v_dsf,
            trailing: body[c..].to_vec(),
        })
    }
}

impl TrackFragmentHeaderBox {
    /// The `flags` this box actually serializes: the presence bits are derived
    /// from the `Option`s, and every other bit is taken from
    /// [`TrackFragmentHeaderBox::flags`].
    ///
    /// Field presence in `tfhd` is flag-driven (§8.8.7.1), but the values live
    /// in `Option`s. Serializing on the *stored* flags with `unwrap_or(0)`
    /// means a builder that set a presence bit without the value emitted `0`
    /// — a `base_data_offset` of 0, or a `sample_description_index` of 0 where
    /// §8.8.7.1 defines 1 as "the first entry". Setting the value without the
    /// bit silently dropped it instead. Deriving the bits removes both
    /// mismatch directions (audit r05-W16).
    ///
    /// `default-base-is-moof` may legally appear *alongside* an explicit
    /// `base_data_offset`: §8.8.7.1 says of the flag "if base-data-offset-present
    /// is 1, this flag is ignored", so such a box is well-formed (just
    /// redundantly specified) and is preserved rather than rejected. A parser
    /// must be liberal here — the flag's presence carries no information when
    /// the explicit base is set, and rewriting the file must not silently
    /// change it either.
    pub fn effective_flags(&self) -> u32 {
        let non_presence = self.flags
            & !(TFHD_BASE_DATA_OFFSET_PRESENT
                | TFHD_SAMPLE_DESCRIPTION_INDEX_PRESENT
                | TFHD_DEFAULT_SAMPLE_DURATION_PRESENT
                | TFHD_DEFAULT_SAMPLE_SIZE_PRESENT
                | TFHD_DEFAULT_SAMPLE_FLAGS_PRESENT);
        let mut bits = non_presence;
        if self.base_data_offset.is_some() {
            bits |= TFHD_BASE_DATA_OFFSET_PRESENT;
        }
        if self.sample_description_index.is_some() {
            bits |= TFHD_SAMPLE_DESCRIPTION_INDEX_PRESENT;
        }
        if self.default_sample_duration.is_some() {
            bits |= TFHD_DEFAULT_SAMPLE_DURATION_PRESENT;
        }
        if self.default_sample_size.is_some() {
            bits |= TFHD_DEFAULT_SAMPLE_SIZE_PRESENT;
        }
        if self.default_sample_flags.is_some() {
            bits |= TFHD_DEFAULT_SAMPLE_FLAGS_PRESENT;
        }
        bits
    }
}

impl Serialize for TrackFragmentHeaderBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>();
        if self.base_data_offset.is_some() {
            n += core::mem::size_of::<u64>();
        }
        if self.sample_description_index.is_some() {
            n += core::mem::size_of::<u32>();
        }
        if self.default_sample_duration.is_some() {
            n += core::mem::size_of::<u32>();
        }
        if self.default_sample_size.is_some() {
            n += core::mem::size_of::<u32>();
        }
        if self.default_sample_flags.is_some() {
            n += core::mem::size_of::<u32>();
        }
        n + self.trailing.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let flags = self.effective_flags();
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c =
            crate::box_types::write_full_box_header(buf, b"tfhd", need, self.version, flags, true)?;
        buf[c..c + core::mem::size_of::<u32>()]
            .copy_from_slice(&self.track_id.value().to_be_bytes());
        c += core::mem::size_of::<u32>();
        if let Some(v) = self.base_data_offset {
            buf[c..c + core::mem::size_of::<u64>()].copy_from_slice(&v.to_be_bytes());
            c += core::mem::size_of::<u64>();
        }
        if let Some(v) = self.sample_description_index {
            buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&v.to_be_bytes());
            c += core::mem::size_of::<u32>();
        }
        if let Some(v) = self.default_sample_duration {
            buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&v.to_be_bytes());
            c += core::mem::size_of::<u32>();
        }
        if let Some(v) = self.default_sample_size {
            buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&v.to_be_bytes());
            c += core::mem::size_of::<u32>();
        }
        if let Some(v) = self.default_sample_flags {
            buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&v.to_be_bytes());
            c += core::mem::size_of::<u32>();
        }
        buf[c..c + self.trailing.len()].copy_from_slice(&self.trailing);
        Ok(c + self.trailing.len())
    }
}

/// Four-CC of the Track Fragment Base Media Decode Time Box (`tfdt`, ISO/IEC 14496-12:2026 §8.8.12).
pub const TFDT: [u8; 4] = *b"tfdt";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TrackFragmentBaseMediaDecodeTimeBox {
    version: u8,
    flags: u32,
    v0: u32,
    v1: u64,
}

impl TrackFragmentBaseMediaDecodeTimeBox {
    pub fn new_v0(t: u32) -> Self {
        Self {
            version: 0,
            flags: 0,
            v0: t,
            v1: t as u64,
        }
    }
    pub fn new_v1(t: u64) -> Self {
        Self {
            version: 1,
            flags: 0,
            v0: t as u32,
            v1: t,
        }
    }
    pub fn base_media_decode_time(&self) -> u64 {
        self.v1
    }
    pub fn version(&self) -> u8 {
        self.version
    }

    pub fn parse_body(body: &[u8]) -> Result<Self> {
        let (ver, flags) = read_ver_flags(body)?;
        let payload = &body[FULLBOX_EXTRA_SIZE..];
        if ver == 0 {
            need_in(payload, core::mem::size_of::<u32>(), "tfdt.v0")?;
            let v = be_u32(payload, 0, "tfdt.v0")?;
            Ok(Self {
                version: ver,
                flags,
                v0: v,
                v1: v as u64,
            })
        } else {
            need_in(payload, core::mem::size_of::<u64>(), "tfdt.v1")?;
            let v = be_u64(payload, 0, "tfdt.v1")?;
            Ok(Self {
                version: ver,
                flags,
                v0: v as u32,
                v1: v,
            })
        }
    }
}

/// Ceiling on a `trun`'s `sample_count` when it carries no per-sample fields
/// (every entry is then zero bytes wide, so the buffer length cannot bound
/// it). 2^20 samples is ~10x any real single-fragment sample count (a 2-hour
/// 60 fps movie in one `moof` is ~432 000) and costs at most ~40 MiB.
pub const MAX_ZERO_WIDTH_TRUN_SAMPLES: usize = 1 << 20;

impl Serialize for TrackFragmentBaseMediaDecodeTimeBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        BOX_HEADER_MIN_SIZE
            + FULLBOX_EXTRA_SIZE
            + if self.version == 0 {
                core::mem::size_of::<u32>()
            } else {
                core::mem::size_of::<u64>()
            }
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = 0;
        c += crate::box_types::write_full_box_header(
            buf,
            b"tfdt",
            need,
            self.version,
            self.flags,
            true,
        )?;
        if self.version == 0 {
            buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&self.v0.to_be_bytes());
            c += core::mem::size_of::<u32>();
        } else {
            buf[c..c + core::mem::size_of::<u64>()].copy_from_slice(&self.v1.to_be_bytes());
            c += core::mem::size_of::<u64>();
        }
        Ok(c)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TrunSample {
    pub sample_duration: Option<u32>,
    pub sample_size: Option<u32>,
    pub sample_flags: Option<u32>,
    /// `trun.sample_composition_time_offset` (ISO/IEC 14496-12:2026 §8.8.8.2):
    /// an *unsigned* 32-bit value under version 0, a *signed* one under
    /// version 1. Held as `i64` so a legal v0 offset of 2^31 or more survives
    /// instead of wrapping negative — which also wrongly forced such a box to
    /// be re-serialized as version 1, changing both its bytes and its meaning
    /// (audit item 4, round 3).
    pub sample_composition_time_offset: Option<i64>,
}
impl TrunSample {
    pub const fn new() -> Self {
        Self {
            sample_duration: None,
            sample_size: None,
            sample_flags: None,
            sample_composition_time_offset: None,
        }
    }
}
impl Default for TrunSample {
    fn default() -> Self {
        Self::new()
    }
}

/// Four-CC of the Track Fragment Run Box (`trun`, ISO/IEC 14496-12:2026 §8.8.8).
pub const TRUN: [u8; 4] = *b"trun";

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TrackFragmentRunBox {
    pub version: u8,
    pub tr_flags: u32,
    pub data_offset: Option<i32>,
    pub first_sample_flags: Option<u32>,
    pub samples: Vec<TrunSample>,
    /// Bytes after the last sample record the count declares (§8.8.8.2),
    /// carried verbatim so a rewrite is exact. Not a field of the box.
    trailing: Vec<u8>,
}

impl TrackFragmentRunBox {
    /// A `trun` of `samples` with no bytes past the declared table.
    #[must_use]
    pub fn new(
        version: u8,
        tr_flags: u32,
        data_offset: Option<i32>,
        first_sample_flags: Option<u32>,
        samples: Vec<TrunSample>,
    ) -> Self {
        Self {
            version,
            tr_flags,
            data_offset,
            first_sample_flags,
            samples,
            trailing: Vec::new(),
        }
    }

    /// The bytes after the last declared sample record, in wire order.
    #[must_use]
    pub fn trailing(&self) -> &[u8] {
        &self.trailing
    }

    /// Keep bytes the declared table does not account for.
    #[must_use]
    pub fn with_trailing(mut self, trailing: Vec<u8>) -> Self {
        self.trailing = trailing;
        self
    }

    pub fn parse_body(body: &[u8]) -> Result<Self> {
        Self::parse_body_budgeted(body, &mut { MAX_ZERO_WIDTH_TRUN_SAMPLES })
    }

    /// [`TrackFragmentRunBox::parse_body`] drawing every zero-width sample
    /// (a `trun` with no per-sample fields, which the buffer length cannot
    /// bound) from `zero_width_budget`, so a caller parsing a whole `moof` (or
    /// a run of `moof`s) shares ONE [`MAX_ZERO_WIDTH_TRUN_SAMPLES`] allowance
    /// rather than getting a fresh one per `trun` (16-byte `trun`s x 2^20
    /// samples x 40 bytes each is a ~40 MiB-per-16-bytes amplification).
    pub fn parse_body_budgeted(body: &[u8], zero_width_budget: &mut usize) -> Result<Self> {
        let (ver, tr_flags) = read_ver_flags(body)?;
        let payload = &body[FULLBOX_EXTRA_SIZE..];
        need_in(payload, core::mem::size_of::<u32>(), "trun.sc")?;
        let mut c = 0usize;
        let sc = be_u32(payload, 0, "trun.sc")? as usize;
        c += core::mem::size_of::<u32>();

        let data_offset = if tr_flags & TRUN_DATA_OFFSET_PRESENT != 0 {
            need_in(payload, c + core::mem::size_of::<u32>(), "trun.do")?;
            let v = read_u32(payload, &mut c, "trun.do")? as i32;
            Some(v)
        } else {
            None
        };
        let fsf = if tr_flags & TRUN_FIRST_SAMPLE_FLAGS_PRESENT != 0 {
            need_in(payload, c + core::mem::size_of::<u32>(), "trun.fsf")?;
            let v = read_u32(payload, &mut c, "trun.fsf")?;
            Some(v)
        } else {
            None
        };

        let has_dur = tr_flags & TRUN_SAMPLE_DURATION_PRESENT != 0;
        let has_sz = tr_flags & TRUN_SAMPLE_SIZE_PRESENT != 0;
        let has_flg = tr_flags & TRUN_SAMPLE_FLAGS_PRESENT != 0;
        let has_cto = tr_flags & TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT != 0;

        // Each present per-sample field is a 32-bit word (ISO/IEC 14496-12:2026
        // §8.8.8.2); bound the wire-declared `sample_count` against the
        // remaining buffer before trusting it for `Vec::with_capacity` (#983).
        // The loop below still walks the raw (unbounded) `sc` — each iteration
        // re-checks its own bounds and errors out on truncated input, exactly
        // as the sibling `stts`/`stsc`/etc. parsers do.
        let entry_size = [has_dur, has_sz, has_flg, has_cto]
            .iter()
            .filter(|&&present| present)
            .count()
            * core::mem::size_of::<u32>();
        // With no per-sample field present every entry is ZERO bytes wide, so
        // the remaining-bytes bound above cannot limit `sc` at all: a 20-byte
        // `trun` declaring 2^32-1 samples would push ~170 GB of `TrunSample`s
        // (#1130). Such a `trun` is legal (sizes/durations come from `tfhd`),
        // so cap the count at a generous real-world ceiling instead.
        if entry_size == 0 {
            if sc > *zero_width_budget {
                return Err(Error::ZeroWidthTrunBudgetExceeded {
                    limit: MAX_ZERO_WIDTH_TRUN_SAMPLES,
                });
            }
            *zero_width_budget -= sc;
        }
        let mut samples = Vec::with_capacity(crate::box_support::bounded_entry_count(
            payload.len().saturating_sub(c),
            entry_size,
            sc,
        ));
        for _ in 0..sc {
            let mut s = TrunSample::new();
            if has_dur {
                need_in(payload, c + core::mem::size_of::<u32>(), "trun.dur")?;
                s.sample_duration = Some(read_u32(payload, &mut c, "trun.dur")?);
            }
            if has_sz {
                need_in(payload, c + core::mem::size_of::<u32>(), "trun.sz")?;
                s.sample_size = Some(read_u32(payload, &mut c, "trun.sz")?);
            }
            if has_flg {
                need_in(payload, c + core::mem::size_of::<u32>(), "trun.flg")?;
                s.sample_flags = Some(read_u32(payload, &mut c, "trun.flg")?);
            }
            if has_cto {
                need_in(payload, c + core::mem::size_of::<u32>(), "trun.cto")?;
                let raw = read_u32(payload, &mut c, "trun.cto")?;
                s.sample_composition_time_offset = Some(if ver == 1 {
                    i64::from(raw as i32)
                } else {
                    i64::from(raw)
                });
            }
            samples.push(s);
        }
        Ok(TrackFragmentRunBox {
            version: ver,
            tr_flags,
            data_offset,
            first_sample_flags: fsf,
            samples,
            trailing: payload[c..].to_vec(),
        })
    }

    pub fn record_stride(flags: u32) -> usize {
        let mut n = 0u32;
        if flags & TRUN_SAMPLE_DURATION_PRESENT != 0 {
            n += 1;
        }
        if flags & TRUN_SAMPLE_SIZE_PRESENT != 0 {
            n += 1;
        }
        if flags & TRUN_SAMPLE_FLAGS_PRESENT != 0 {
            n += 1;
        }
        if flags & TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT != 0 {
            n += 1;
        }
        n as usize * core::mem::size_of::<u32>()
    }
}

impl TrackFragmentRunBox {
    /// The `flags` this box actually serializes: the presence bits are derived
    /// from the `Option`s and from the samples' own fields, and every other bit
    /// is taken from [`TrackFragmentRunBox::tr_flags`].
    ///
    /// See [`TrackFragmentHeaderBox::effective_flags`] for why (audit
    /// r05-W16). A per-sample field is present only when *every* sample
    /// carries it — `trun` has no way to write the field for some records and
    /// not others, so a partially-filled sample list is a hard error in
    /// `serialize_into` rather than a `0` for the missing ones.
    pub fn effective_flags(&self) -> u32 {
        let non_presence = self.tr_flags
            & !(TRUN_DATA_OFFSET_PRESENT
                | TRUN_FIRST_SAMPLE_FLAGS_PRESENT
                | TRUN_SAMPLE_DURATION_PRESENT
                | TRUN_SAMPLE_SIZE_PRESENT
                | TRUN_SAMPLE_FLAGS_PRESENT
                | TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT);
        let mut bits = non_presence;
        if self.data_offset.is_some() {
            bits |= TRUN_DATA_OFFSET_PRESENT;
        }
        if self.first_sample_flags.is_some() {
            bits |= TRUN_FIRST_SAMPLE_FLAGS_PRESENT;
        }
        // With no samples the per-sample bits are pure metadata (the box
        // declares which fields a *hypothetical* record would carry), so the
        // stored `tr_flags` are reproduced verbatim: a zero-sample `trun` must
        // round-trip its own flags, not have them cleared.
        if self.samples.is_empty() {
            return bits | self.tr_flags;
        }
        let all = |f: fn(&TrunSample) -> bool| self.samples.iter().all(f);

        if all(|s| s.sample_duration.is_some()) {
            bits |= TRUN_SAMPLE_DURATION_PRESENT;
        }
        if all(|s| s.sample_size.is_some()) {
            bits |= TRUN_SAMPLE_SIZE_PRESENT;
        }
        if all(|s| s.sample_flags.is_some()) {
            bits |= TRUN_SAMPLE_FLAGS_PRESENT;
        }
        if all(|s| s.sample_composition_time_offset.is_some()) {
            bits |= TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT;
        }
        bits
    }

    /// Reject a sample list where some samples carry a per-sample field and
    /// others do not.
    ///
    /// A `trun` record is fixed-width with no per-record presence bit, so the
    /// field is either there for every sample or for none. Quietly dropping it
    /// (`effective_flags` would clear the bit) would discard the values the
    /// caller did supply, and writing `0` for the gaps would fabricate sample
    /// durations and sizes — so neither: a partial list is an error (audit
    /// r05-W16).
    fn check_sample_fields(&self, flags: u32) -> Result<()> {
        type FieldCheck = (u32, fn(&TrunSample) -> bool);
        let checks: [FieldCheck; TRUN_SAMPLE_FIELD_COUNT] = [
            (TRUN_SAMPLE_DURATION_PRESENT, |s: &TrunSample| {
                s.sample_duration.is_some()
            }),
            (TRUN_SAMPLE_SIZE_PRESENT, |s: &TrunSample| {
                s.sample_size.is_some()
            }),
            (TRUN_SAMPLE_FLAGS_PRESENT, |s: &TrunSample| {
                s.sample_flags.is_some()
            }),
            (
                TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT,
                |s: &TrunSample| s.sample_composition_time_offset.is_some(),
            ),
        ];
        for (bit, present) in checks {
            // A field some samples carry and others do not cannot be
            // represented at all: a record is fixed-width, so the derived
            // flags would drop the values the caller did supply, and writing
            // `0` for the gaps would fabricate them. The first branch is
            // therefore subsumed by the second: a flag-driven gap is itself a
            // disagreement when any sample carries the value.
            if self.samples.iter().any(present) && !self.samples.iter().all(present) {
                return Err(Error::InvalidInput(
                    "trun samples disagree on a per-sample field; a trun record is fixed-width",
                ));
            }
            if flags & bit != 0 && !self.samples.iter().all(present) {
                return Err(Error::InvalidInput(
                    "trun flags declare a per-sample field that a sample does not carry",
                ));
            }
        }
        Ok(())
    }
}

impl Serialize for TrackFragmentRunBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let flags = self.effective_flags();
        let mut n = BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + core::mem::size_of::<u32>();
        if flags & TRUN_DATA_OFFSET_PRESENT != 0 {
            n += core::mem::size_of::<u32>();
        }
        if flags & TRUN_FIRST_SAMPLE_FLAGS_PRESENT != 0 {
            n += core::mem::size_of::<u32>();
        }
        crate::box_types::box_len(
            n + self.samples.len() * Self::record_stride(flags) + self.trailing.len(),
        )
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let flags = self.effective_flags();
        self.check_sample_fields(flags)?;
        let need = self.serialized_len();
        need_out(buf, need)?;
        // §8.8.8.2: version 1 makes `sample_composition_time_offset` *signed*.
        // Writing version 0 with a negative offset would reinterpret it as a
        // huge positive one (a two's-complement wrap in most readers), so any
        // negative offset forces version 1. A *non-negative* offset that fits
        // version 0's unsigned field keeps the requested version — including
        // one above `i32::MAX`, which v0 can hold but v1's signed field cannot.
        let needs_v1 = self
            .samples
            .iter()
            .any(|s| s.sample_composition_time_offset.is_some_and(|v| v < 0));
        let version = if needs_v1 { 1 } else { self.version };
        let mut c =
            crate::box_types::write_full_box_header(buf, b"trun", need, version, flags, false)?;
        let sample_count = broadcast_core::len::fit_u32(self.samples.len(), "sample_count")?;
        buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&sample_count.to_be_bytes());
        c += core::mem::size_of::<u32>();
        if let Some(v) = self.data_offset {
            buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&v.to_be_bytes());
            c += core::mem::size_of::<u32>();
        }
        if let Some(v) = self.first_sample_flags {
            buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&v.to_be_bytes());
            c += core::mem::size_of::<u32>();
        }
        let has_dur = flags & TRUN_SAMPLE_DURATION_PRESENT != 0;
        let has_sz = flags & TRUN_SAMPLE_SIZE_PRESENT != 0;
        let has_flg = flags & TRUN_SAMPLE_FLAGS_PRESENT != 0;
        let has_cto = flags & TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT != 0;
        for s in &self.samples {
            if has_dur {
                buf[c..c + core::mem::size_of::<u32>()]
                    .copy_from_slice(&s.sample_duration.unwrap_or(0).to_be_bytes());
                c += core::mem::size_of::<u32>();
            }
            if has_sz {
                buf[c..c + core::mem::size_of::<u32>()]
                    .copy_from_slice(&s.sample_size.unwrap_or(0).to_be_bytes());
                c += core::mem::size_of::<u32>();
            }
            if has_flg {
                buf[c..c + core::mem::size_of::<u32>()]
                    .copy_from_slice(&s.sample_flags.unwrap_or(0).to_be_bytes());
                c += core::mem::size_of::<u32>();
            }
            if has_cto {
                let offset = s.sample_composition_time_offset.unwrap_or(0);
                if version == 1 {
                    let signed = i32::try_from(offset).map_err(|_| {
                        Error::InvalidInput(
                            "trun v1 sample_composition_time_offset leaves the signed 32-bit range",
                        )
                    })?;
                    buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&signed.to_be_bytes());
                } else {
                    let unsigned = u32::try_from(offset).map_err(|_| {
                        Error::InvalidInput(
                            "trun v0 sample_composition_time_offset leaves the unsigned 32-bit range",
                        )
                    })?;
                    buf[c..c + core::mem::size_of::<u32>()]
                        .copy_from_slice(&unsigned.to_be_bytes());
                }
                c += core::mem::size_of::<u32>();
            }
        }
        buf[c..c + self.trailing.len()].copy_from_slice(&self.trailing);
        Ok(c + self.trailing.len())
    }
}
/// A `traf` or `moof` child this crate does not model as a typed field,
/// round-tripped verbatim so a parse → serialize cannot drop it (audit
/// r05-W11).
///
/// `box_type` is the wire four-CC and `data` the payload after the 8-byte
/// `size`+`type` header — which includes the 16-byte `usertype` of a `uuid`
/// child (ISO/IEC 14496-12:2026 §4.2): an ISMV `moov`/`trak` uuid, a Smooth
/// `tfxd`/`tfrf`, or a `uuid`-form `pssh`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct OpaqueChild {
    /// The child's four-CC.
    pub box_type: [u8; BOX_TYPE_SIZE],
    /// The child's payload after `size`/`type`: the `usertype` of a `uuid`
    /// included, an 8-byte `largesize` field excluded (its form is
    /// [`OpaqueChild::largesize`]).
    pub data: Vec<u8>,
    /// Whether the wire used the `size == 1` + 64-bit `largesize` form
    /// (ISO/IEC 14496-12:2026 §4.2), so the round trip re-emits it.
    pub largesize: bool,
    /// Whether the wire used the `size == 0` form ("extends to the end of the
    /// enclosing container", §4.2). Only written back when this child is the
    /// container's last, so a sibling appended later is not swallowed.
    pub to_end: bool,
}

impl OpaqueChild {
    /// An opaque child preserving everything the wire carried: the payload
    /// after `size`/`type` (usertype of a `uuid` included, `largesize` bytes
    /// excluded) plus the `largesize` / `size == 0` forms.
    pub fn from_child(child: &crate::box_types::BoxItem<'_>) -> Self {
        Self {
            box_type: child.four_cc(),
            data: child.payload().to_vec(),
            largesize: child.has_largesize(),
            to_end: child.to_end(),
        }
    }

    /// Build one from a child's four-CC and payload (everything after
    /// `size`/`type` — the usertype of a `uuid` included; the compact header
    /// form is used).
    pub fn new(box_type: [u8; BOX_TYPE_SIZE], data: Vec<u8>) -> Self {
        Self {
            box_type,
            data,
            largesize: false,
            to_end: false,
        }
    }
}

impl Serialize for OpaqueChild {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        self.header_len() + self.data.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        self.serialize_child_into(buf, true)
    }
}

impl OpaqueChild {
    /// Serialize as a child of a `traf`/`moof`, telling the writer whether it
    /// is the container's last child: a `size == 0` box may only use that form
    /// when nothing follows it (§4.2), or it would swallow the sibling after
    /// it.
    pub fn serialize_child_into(&self, buf: &mut [u8], is_last: bool) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        // See `box_support::OpaqueBox`: the header is written here, not
        // through `BoxHeader`, because the payload already carries a `uuid`'s
        // usertype.
        let mut c = 0usize;
        if self.to_end && is_last {
            buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&0u32.to_be_bytes());
            c += core::mem::size_of::<u32>();
            buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&self.box_type);
            c += core::mem::size_of::<u32>();
        } else if self.wide() {
            buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&1u32.to_be_bytes());
            c += core::mem::size_of::<u32>();
            buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&self.box_type);
            c += core::mem::size_of::<u32>();
            buf[c..c + core::mem::size_of::<u64>()].copy_from_slice(&(need as u64).to_be_bytes());
            c += core::mem::size_of::<u64>();
        } else {
            let len = broadcast_core::len::fit_u32(need, "opaque child size")?;
            buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&len.to_be_bytes());
            c += core::mem::size_of::<u32>();
            buf[c..c + core::mem::size_of::<u32>()].copy_from_slice(&self.box_type);
            c += core::mem::size_of::<u32>();
        }
        buf[c..c + self.data.len()].copy_from_slice(&self.data);
        Ok(c + self.data.len())
    }
}

impl OpaqueChild {
    /// The header this child writes: 8 bytes, or 16 under
    /// [`OpaqueChild::largesize`] (§4.2).
    fn header_len(&self) -> usize {
        BOX_HEADER_MIN_SIZE + if self.wide() { LARGESIZE_SIZE } else { 0 }
    }

    /// Whether the header uses the 64-bit `largesize` form: the wire had it
    /// ([`OpaqueChild::largesize`]) or the box is too big for the 32-bit
    /// `size` field (§4.2, #1147) — never a silently wrapped size.
    fn wide(&self) -> bool {
        self.largesize
            || (BOX_HEADER_MIN_SIZE as u64 + self.data.len() as u64) > u64::from(u32::MAX)
    }
}

/// One child of a [`TrackFragmentBox`], in wire order.
///
/// A `traf` carries `tfhd`/`tfdt`/`trun` *and* (per ISO/IEC 14496-12:2026
/// §6.2, Table 2) optional `sbgp`/`sgpd`/`subs`/`saiz`/`saio`/`meta`. This crate
/// models only the first three; [`TrackFragmentBox::order`] keeps every child
/// in the sequence it appeared on the wire, so a parse → serialize — and any
/// rewrite such as a CENC media-segment protection pass — reproduces the file's own
/// ordering and preserves the children it does not understand.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum TrafChild {
    /// The `tfhd` at its wire position.
    Tfhd,
    /// The `tfdt` at its wire position.
    Tfdt,
    /// The `trun` at its wire position; the value is the index into
    /// [`TrackFragmentBox::trun`].
    Trun(usize),
    /// The `rsot` at its wire position (§8.8.18).
    Rsot,
    /// Any other child, verbatim.
    Opaque(OpaqueChild),
}

/// One child of a [`MovieFragmentBox`], in wire order.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum MoofChild {
    /// The `mfhd` at its wire position.
    Mfhd,
    /// The `traf` at its wire position; the value is the index into
    /// [`MovieFragmentBox::traf`].
    Traf(usize),
    /// Any other child, verbatim.
    Opaque(OpaqueChild),
}

/// Four-CC of the Track Fragment Box (`traf`, ISO/IEC 14496-12:2026 §8.8.3).
pub const TRAF: [u8; 4] = *b"traf";

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TrackFragmentBox {
    pub tfhd: TrackFragmentHeaderBox,
    pub tfdt: Option<TrackFragmentBaseMediaDecodeTimeBox>,
    pub trun: Vec<TrackFragmentRunBox>,
    /// The redundant-sample original timing box (`rsot`, §8.8.18), when the
    /// fragment states one.
    pub rsot: Option<RedundantSampleOriginalTimingBox>,
    /// Every child of this `traf` in the order it appeared on the wire.
    ///
    /// A `traf` parsed from bytes therefore round-trips byte-for-byte, and any
    /// child this crate does not model (`sgpd`/`sbgp` roll recovery and
    /// `seig`, `subs`, `sdtp`, an existing `senc`/`saiz`/`saio`, `meta`)
    /// survives a rewrite instead of being silently stripped. An empty `order`
    /// means "one of each typed child, in the conventional sequence" — the
    /// state every builder in this workspace produces.
    pub order: Vec<TrafChild>,
}

impl TrackFragmentBox {
    /// Build a `traf` from its typed parts, with the children emitted in the
    /// conventional `tfhd`, `tfdt`, `trun`… sequence and no opaque children.
    pub fn new(
        tfhd: TrackFragmentHeaderBox,
        tfdt: Option<TrackFragmentBaseMediaDecodeTimeBox>,
        trun: Vec<TrackFragmentRunBox>,
    ) -> Self {
        Self {
            tfhd,
            tfdt,
            trun,
            rsot: None,
            order: Vec::new(),
        }
    }

    pub fn parse_body(body: &[u8]) -> Result<Self> {
        Self::parse_body_budgeted(body, &mut { MAX_ZERO_WIDTH_TRUN_SAMPLES })
    }

    /// [`TrackFragmentBox::parse_body`] sharing `zero_width_budget` across its
    /// `trun`s (see [`TrackFragmentRunBox::parse_body_budgeted`]).
    pub fn parse_body_budgeted(body: &[u8], zero_width_budget: &mut usize) -> Result<Self> {
        Self::parse_body_budgeted_notes(body, zero_width_budget, None)
    }

    /// [`TrackFragmentBox::parse_body_budgeted`], reporting the bytes a `tfhd`
    /// or `trun` carried past its declared fields.
    pub(crate) fn parse_body_budgeted_notes(
        body: &[u8],
        zero_width_budget: &mut usize,
        mut notes: Option<&mut crate::box_support::Notes<'_>>,
    ) -> Result<Self> {
        let mut tfhd: Option<TrackFragmentHeaderBox> = None;
        let mut tfdt: Option<TrackFragmentBaseMediaDecodeTimeBox> = None;
        let mut trun: Vec<TrackFragmentRunBox> = Vec::new();
        let mut rsot = None;
        let mut order: Vec<TrafChild> = Vec::new();
        // The crate's one container-child walker (audit r05-O3 / #1141): this
        // loop used to be a private copy of it.
        let degrade = notes.as_deref().is_some_and(|notes| notes.degrades());
        let (children, _) = crate::box_support::container_children(body, degrade)?;
        for child in children {
            if child.header.box_type.is(b"tfhd") {
                let parsed = TrackFragmentHeaderBox::parse_body(child.payload())?;
                if let Some(notes) = notes.as_deref_mut() {
                    notes.trailing(child.bytes, parsed.trailing().len());
                }
                tfhd = Some(parsed);
                order.push(TrafChild::Tfhd);
            } else if child.header.box_type.is(b"tfdt") {
                tfdt = Some(TrackFragmentBaseMediaDecodeTimeBox::parse_body(
                    child.payload(),
                )?);
                order.push(TrafChild::Tfdt);
            } else if child.header.box_type.is(b"trun") {
                let parsed =
                    TrackFragmentRunBox::parse_body_budgeted(child.payload(), zero_width_budget)?;
                if let Some(notes) = notes.as_deref_mut() {
                    notes.trailing(child.bytes, parsed.trailing().len());
                }
                order.push(TrafChild::Trun(trun.len()));
                trun.push(parsed);
            } else if child.header.box_type.is(b"rsot") {
                rsot = Some(RedundantSampleOriginalTimingBox::parse_body(
                    child.payload(),
                )?);
                order.push(TrafChild::Rsot);
            } else {
                // `payload` is everything after `size`+`type` (+ `largesize`),
                // so a `uuid` child (Smooth `tfxd`, a PlayReady `tfrf`) keeps
                // its 16-byte usertype on the way back out (audit item 1).
                order.push(TrafChild::Opaque(OpaqueChild::from_child(&child)));
            }
        }
        let tfhd = tfhd.ok_or(Error::BufferTooShort(BufferTooShort {
            need: 1,
            have: 0,
            what: "traf missing tfhd",
        }))?;
        // A `traf` with zero `trun`s is legal: §8.8.3/§8.8.8 require none, and
        // the `tfhd` `duration-is-empty` flag (`TFHD_DURATION_IS_EMPTY`) exists
        // precisely to declare a fragment that carries no samples for this
        // track (a sparse track inside an otherwise-populated `moof`). The old
        // "traf missing trun" rejection failed the whole `moof` — and with it
        // `Mp4Demux`, `cenc_decrypt` and `protect_media_segment` — on
        // conformant input (audit r05-W11).
        Ok(TrackFragmentBox {
            tfhd,
            tfdt,
            trun,
            rsot,
            order,
        })
    }

    /// The children to serialize, in order: the recorded wire order when this
    /// `traf` was parsed, else the conventional `tfhd`, `tfdt`, `trun`…
    /// sequence of a built one.
    fn child_order(&self) -> Vec<TrafChild> {
        if !self.order.is_empty() {
            return self.order.clone();
        }
        let mut out = Vec::with_capacity(TRAF_FIXED_CHILD_COUNT + self.trun.len());
        out.push(TrafChild::Tfhd);
        if self.tfdt.is_some() {
            out.push(TrafChild::Tfdt);
        }
        for i in 0..self.trun.len() {
            out.push(TrafChild::Trun(i));
        }
        if self.rsot.is_some() {
            out.push(TrafChild::Rsot);
        }
        out
    }
}

impl Serialize for TrackFragmentBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HEADER_MIN_SIZE;
        for child in self.child_order() {
            n += match child {
                TrafChild::Tfhd => self.tfhd.serialized_len(),
                TrafChild::Tfdt => self.tfdt.as_ref().map_or(0, Serialize::serialized_len),
                TrafChild::Trun(i) => self.trun.get(i).map_or(0, Serialize::serialized_len),
                TrafChild::Rsot => self.rsot.as_ref().map_or(0, Serialize::serialized_len),
                TrafChild::Opaque(ref o) => o.serialized_len(),
            };
        }
        crate::box_types::box_len(n)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let hdr_len = crate::box_types::write_box_header(buf, b"traf", need)?;
        let mut c = hdr_len;
        let order = self.child_order();
        for (i, child) in order.iter().enumerate() {
            let is_last = i + 1 == order.len();
            match child {
                TrafChild::Tfhd => c += self.tfhd.serialize_into(&mut buf[c..])?,
                TrafChild::Tfdt => {
                    if let Some(ref t) = self.tfdt {
                        c += t.serialize_into(&mut buf[c..])?;
                    }
                }
                TrafChild::Trun(i) => {
                    if let Some(r) = self.trun.get(*i) {
                        c += r.serialize_into(&mut buf[c..])?;
                    }
                }
                TrafChild::Rsot => {
                    if let Some(r) = &self.rsot {
                        c += r.serialize_into(&mut buf[c..])?;
                    }
                }
                TrafChild::Opaque(o) => c += o.serialize_child_into(&mut buf[c..], is_last)?,
            }
        }
        if c != need {
            return Err(Error::InvalidInput(
                "traf child order does not account for the whole box",
            ));
        }
        Ok(c)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MovieFragmentBox {
    pub mfhd: MovieFragmentHeaderBox,
    pub traf: Vec<TrackFragmentBox>,
    /// Every child of this `moof` in wire order — see
    /// [`TrackFragmentBox::order`] for why (a `moof` may carry `meta`, and
    /// `pssh` is not unknown to a strict reader).
    pub order: Vec<MoofChild>,
}

impl MovieFragmentBox {
    /// Build a `moof` from its typed parts, children in the conventional
    /// `mfhd`, `traf`… sequence.
    pub fn new(mfhd: MovieFragmentHeaderBox, traf: Vec<TrackFragmentBox>) -> Self {
        Self {
            mfhd,
            traf,
            order: Vec::new(),
        }
    }

    pub fn parse_body(body: &[u8]) -> Result<Self> {
        Self::parse_body_budgeted(body, &mut { MAX_ZERO_WIDTH_TRUN_SAMPLES })
    }

    /// [`MovieFragmentBox::parse_body`] drawing zero-width `trun` samples from
    /// the caller's `zero_width_budget`: the allowance is shared by every
    /// `trun` of every `traf` of this `moof`, and a demuxer holding several
    /// `moof`s before their `mdat` passes the same budget to each.
    pub fn parse_body_budgeted(body: &[u8], zero_width_budget: &mut usize) -> Result<Self> {
        Self::parse_body_budgeted_notes(body, zero_width_budget, None)
    }

    /// [`MovieFragmentBox::parse_body_budgeted`], reporting the bytes a `tfhd`
    /// or `trun` carried past its declared fields.
    pub(crate) fn parse_body_budgeted_notes(
        body: &[u8],
        zero_width_budget: &mut usize,
        mut notes: Option<&mut crate::box_support::Notes<'_>>,
    ) -> Result<Self> {
        let mut mfhd: Option<MovieFragmentHeaderBox> = None;
        let mut traf: Vec<TrackFragmentBox> = Vec::new();
        let mut order: Vec<MoofChild> = Vec::new();
        let degrade = notes.as_deref().is_some_and(|notes| notes.degrades());
        let (children, _) = crate::box_support::container_children(body, degrade)?;
        for child in children {
            if child.header.box_type.is(b"mfhd") {
                mfhd = Some(MovieFragmentHeaderBox::parse_body(child.payload())?);
                order.push(MoofChild::Mfhd);
            } else if child.header.box_type.is(b"traf") {
                let parsed = TrackFragmentBox::parse_body_budgeted_notes(
                    child.payload(),
                    zero_width_budget,
                    notes.as_deref_mut(),
                )?;
                order.push(MoofChild::Traf(traf.len()));
                traf.push(parsed);
            } else {
                order.push(MoofChild::Opaque(OpaqueChild::from_child(&child)));
            }
        }
        let mfhd = mfhd.ok_or(Error::BufferTooShort(BufferTooShort {
            need: 1,
            have: 0,
            what: "moof missing mfhd",
        }))?;
        if traf.is_empty() {
            return Err(Error::BufferTooShort(BufferTooShort {
                need: 1,
                have: 0,
                what: "moof missing traf",
            }));
        }
        Ok(MovieFragmentBox { mfhd, traf, order })
    }

    fn child_order(&self) -> Vec<MoofChild> {
        if !self.order.is_empty() {
            return self.order.clone();
        }
        let mut out = Vec::with_capacity(1 + self.traf.len());
        out.push(MoofChild::Mfhd);
        for i in 0..self.traf.len() {
            out.push(MoofChild::Traf(i));
        }
        out
    }

    /// The `child_order` of a rewriting caller (it has to be
    /// visible to the CENC media-segment protection pass, which lives outside this impl).
    pub fn child_order_for_rewrite(&self) -> Vec<MoofChild> {
        self.child_order()
    }
}

impl Serialize for MovieFragmentBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HEADER_MIN_SIZE;
        for child in self.child_order() {
            n += match child {
                MoofChild::Mfhd => self.mfhd.serialized_len(),
                MoofChild::Traf(i) => self.traf.get(i).map_or(0, Serialize::serialized_len),
                MoofChild::Opaque(ref o) => o.serialized_len(),
            };
        }
        crate::box_types::box_len(n)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let hdr_len = crate::box_types::write_box_header(buf, b"moof", need)?;
        let mut c = hdr_len;
        let order = self.child_order();
        for (i, child) in order.iter().enumerate() {
            let is_last = i + 1 == order.len();
            match child {
                MoofChild::Mfhd => c += self.mfhd.serialize_into(&mut buf[c..])?,
                MoofChild::Traf(i) => {
                    if let Some(t) = self.traf.get(*i) {
                        c += t.serialize_into(&mut buf[c..])?;
                    }
                }
                MoofChild::Opaque(o) => c += o.serialize_child_into(&mut buf[c..], is_last)?,
            }
        }
        if c != need {
            return Err(Error::InvalidInput(
                "moof child order does not account for the whole box",
            ));
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// CENC movie-fragment protection (`senc`/`saiz`/`saio`) — ISO/IEC 23001-7
// §12.3, ISO/IEC 14496-12:2026 §8.7.8-9 — issue #564 Task 3 (muxer emission).
//
// `senc`/`saiz`/`saio` are deliberately **not** modelled as fields on
// [`TrackFragmentBox`]: every call site that builds one (e.g.
// `pipeline::build_media_segment_with_events`) goes through
// [`TrackFragmentBox::new`], and the boxes are only known to the caller that
// protects the track. Instead this is a *post-processing* pass over an
// already-built movie fragment (`moof`), driven by the caller's
// [`crate::cenc::SampleEncryptionEntry`] list (from
// the IR crypto carrier's per-sample entry list) — it composes with any CMAF
// muxer output without the crypto metadata needing to flow through the
// `trun`/`traf` builder plumbing itself.
//
// # `saio` anchor
//
// This crate's fragment builder (`pipeline::build_media_segment_with_events`)
// always sets `tfhd`'s `default-base-is-moof` flag and computes
// `trun.data_offset` **relative to the first byte of the enclosing `moof`
// box** (not an absolute file offset — see that function's own doc comment).
// `saio.offset[0]` is computed on exactly that same moof-relative basis for
// consistency with the trun convention already in this pipeline, and because
// it matches how CMAF encoders (e.g. Shaka Packager, Bento4 `mp4encrypt`)
// anchor `saio` under `default-base-is-moof`. Task 4's `mp4decrypt` interop
// test is the authoritative check of this choice against a real external
// decryptor.
// ---------------------------------------------------------------------------

/// Number of bytes from the start of a `senc` box (ISO/IEC 23001-7 §12.3) to
/// its first sample's IV: 8-byte box header + 4-byte FullBox version/flags +
/// 4-byte `sample_count`.
pub const SENC_ENTRIES_OFFSET: u64 = 16;

/// Per-`subsample` fixed fields written by `senc` when
/// [`crate::cenc::SENC_FLAG_USE_SUBSAMPLE_ENCRYPTION`] is set — ISO/IEC
/// 23001-7 §12.3.2: `subsample_count` (2 bytes).
pub const SAIZ_SUBSAMPLE_COUNT_SIZE: usize = 2;
/// One `(bytes_of_clear_data, bytes_of_protected_data)` subsample entry:
/// 2-byte clear count + 4-byte protected count — ISO/IEC 23001-7 §12.3.2.
pub const SAIZ_SUBSAMPLE_ENTRY_SIZE: usize = 6;

/// The `aux_info_type` values that name a CENC scheme. ISO/IEC 23001-7 §12.3's
/// `aux_info_type` defaults to `cenc`, and the four-CCs below are the schemes
/// that specification defines.
pub const CENC_AUX_INFO_TYPES: [[u8; BOX_TYPE_SIZE]; CENC_AUX_INFO_TYPE_COUNT] =
    [*b"cenc", *b"cens", *b"cbc1", *b"cbcs"];

/// The `aux_info_type` of a `saiz`/`saio` box's opaque payload (the bytes
/// after its 8-byte header), or `None` when absent.
///
/// Both are FullBoxes: a version-0 box carries the field only when flag bit 0
/// is set (§8.7.8.2, §8.7.9.2); version 1 always carries it.
pub fn aux_info_type_of(
    payload: &[u8],
    box_type: &[u8; BOX_TYPE_SIZE],
) -> Option<[u8; BOX_TYPE_SIZE]> {
    let _ = box_type;
    if payload.len() < BOX_TYPE_SIZE {
        return None;
    }
    let version = payload[0];
    let flags = be_u32(payload, 0, "aux_info_type").ok()? & FULLBOX_FLAGS_MASK;
    if version == 0 && flags & AUX_INFO_TYPE_PRESENT_FLAG == 0 {
        return None;
    }
    let ty = payload.get(BOX_TYPE_SIZE..BOX_HEADER_MIN_SIZE)?;
    let mut aux_info_type = [0u8; BOX_TYPE_SIZE];
    aux_info_type.copy_from_slice(ty);
    Some(aux_info_type)
}

/// Fill what a `tfhd` leaves out from the track's `trex` (ISO/IEC
/// 14496-12:2026 §8.8.3, §8.8.7): a sample's duration, size and flags default
/// to the `tfhd` value, and when that is absent too, to the `trex` of the init
/// movie.
///
/// A packager that declares its durations once in `trex` (GPAC does for a
/// fragment whose samples all last the same) writes `trun` entries with sizes
/// only; without the `trex` fallback every such sample comes out with duration
/// 0 and the whole fragment at one decode time. A `trex` size of zero means
/// "not declared" in practice and is not applied.
pub fn apply_trex_defaults(moof: &mut MovieFragmentBox, trex: &[TrackExtendsBox]) {
    for traf in &mut moof.traf {
        let Some(defaults) = trex
            .iter()
            .find(|t| t.track_id.value() == traf.tfhd.track_id.value())
        else {
            continue;
        };
        let tfhd = &mut traf.tfhd;
        tfhd.default_sample_duration
            .get_or_insert(defaults.default_sample_duration);
        tfhd.default_sample_flags
            .get_or_insert(defaults.default_sample_flags);
        if defaults.default_sample_size != 0 {
            tfhd.default_sample_size
                .get_or_insert(defaults.default_sample_size);
        }
    }
}

/// Four-CC of the Redundant Sample Original Timing Box
/// (`rsot`, ISO/IEC 14496-12:2026 §8.8.18).
pub const RSOT: [u8; BOX_TYPE_SIZE] = *b"rsot";
/// `rsot` flag: the last sample's original duration is signalled.
pub const RSOT_ORIGINAL_DURATION: u32 = 0x000001;
/// `rsot` flag: the first sample's elapsed duration is signalled.
pub const RSOT_ELAPSED_DURATION: u32 = 0x000002;
const RSOT_KNOWN_FLAGS: u32 = RSOT_ORIGINAL_DURATION | RSOT_ELAPSED_DURATION;

/// The Redundant Sample Original Timing Box (`rsot`, §8.8.18).
///
/// The version is fixed at 0 and only the two defined flags may be set;
/// anything else is a typed error rather than a silently dropped field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct RedundantSampleOriginalTimingBox {
    flags: u32,
    original_duration: Option<u32>,
    elapsed_duration: Option<u32>,
}

impl RedundantSampleOriginalTimingBox {
    /// Build one with no durations (no flags set).
    #[must_use]
    pub const fn new() -> Self {
        Self {
            flags: 0,
            original_duration: None,
            elapsed_duration: None,
        }
    }

    /// The FullBox flags.
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// The last sample's original duration, when signalled.
    #[must_use]
    pub const fn original_duration(&self) -> Option<u32> {
        self.original_duration
    }

    /// The first sample's elapsed duration, when signalled.
    #[must_use]
    pub const fn elapsed_duration(&self) -> Option<u32> {
        self.elapsed_duration
    }

    /// Sign the last sample's original duration.
    #[must_use]
    pub const fn with_original_duration(mut self, duration: u32) -> Self {
        self.flags |= RSOT_ORIGINAL_DURATION;
        self.original_duration = Some(duration);
        self
    }

    /// Sign the first sample's elapsed duration.
    #[must_use]
    pub const fn with_elapsed_duration(mut self, duration: u32) -> Self {
        self.flags |= RSOT_ELAPSED_DURATION;
        self.elapsed_duration = Some(duration);
        self
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, FULLBOX_EXTRA_SIZE, "rsot")?;
        if body[0] != 0 {
            return Err(Error::InvalidInput("rsot version is 0"));
        }
        let flags = u24_at(body, 1) & FULLBOX_FLAGS_MASK;
        if flags & !RSOT_KNOWN_FLAGS != 0 {
            return Err(Error::InvalidInput("rsot reserved flag bits are set"));
        }
        let mut cursor = FULLBOX_EXTRA_SIZE;
        let original_duration = if flags & RSOT_ORIGINAL_DURATION != 0 {
            let value = be_u32(body, cursor, "rsot original_duration")?;
            cursor += core::mem::size_of::<u32>();
            Some(value)
        } else {
            None
        };
        let elapsed_duration = if flags & RSOT_ELAPSED_DURATION != 0 {
            Some(be_u32(body, cursor, "rsot elapsed_duration")?)
        } else {
            None
        };
        Ok(Self {
            flags,
            original_duration,
            elapsed_duration,
        })
    }
}

impl<'a> Parse<'a> for RedundantSampleOriginalTimingBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let item = crate::box_types::parse_box(bytes)?.0;
        if item.four_cc() != RSOT {
            return Err(Error::InvalidInput("expected rsot"));
        }
        Self::parse_body(item.payload())
    }
}

impl Serialize for RedundantSampleOriginalTimingBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let mut len = BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE;
        if self.original_duration.is_some() {
            len += core::mem::size_of::<u32>();
        }
        if self.elapsed_duration.is_some() {
            len += core::mem::size_of::<u32>();
        }
        box_len(len)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let mut flags = self.flags;
        if self.original_duration.is_some() {
            flags |= RSOT_ORIGINAL_DURATION;
        }
        if self.elapsed_duration.is_some() {
            flags |= RSOT_ELAPSED_DURATION;
        }
        if flags & !RSOT_KNOWN_FLAGS != 0 {
            return Err(Error::InvalidInput("rsot reserved flag bits are set"));
        }
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_full_box_header(buf, &RSOT, need, 0, flags, false)?;
        if let Some(duration) = self.original_duration {
            buf[cursor..cursor + core::mem::size_of::<u32>()]
                .copy_from_slice(&duration.to_be_bytes());
            cursor += core::mem::size_of::<u32>();
        }
        if let Some(duration) = self.elapsed_duration {
            buf[cursor..cursor + core::mem::size_of::<u32>()]
                .copy_from_slice(&duration.to_be_bytes());
            cursor += core::mem::size_of::<u32>();
        }
        Ok(cursor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn mfhd_round_trip() {
        let m = MovieFragmentHeaderBox::new(42);
        let b = m.try_to_bytes().unwrap();
        let p = MovieFragmentHeaderBox::parse_body(&b[8..]).unwrap();
        assert_eq!(p.sequence_number, 42);
        assert_eq!(b, p.try_to_bytes().unwrap());
    }
    #[test]
    fn mfhd_len() {
        assert_eq!(MovieFragmentHeaderBox::new(1).serialized_len(), 16);
    }

    #[test]
    fn tfhd_minimal() {
        let t = TrackFragmentHeaderBox {
            flags: 0,
            track_id: TrackId::new(1),
            version: Default::default(),
            base_data_offset: None,
            sample_description_index: None,
            default_sample_duration: None,
            default_sample_size: None,
            default_sample_flags: None,
            trailing: Vec::new(),
        };
        let b = t.try_to_bytes().unwrap();
        let p = TrackFragmentHeaderBox::parse_body(&b[8..]).unwrap();
        assert_eq!(p.track_id, TrackId::new(1));
        assert_eq!(b, p.try_to_bytes().unwrap());
    }
    #[test]
    fn tfhd_subset() {
        let t = TrackFragmentHeaderBox {
            flags: TFHD_DEFAULT_SAMPLE_DURATION_PRESENT | TFHD_DEFAULT_SAMPLE_SIZE_PRESENT,
            track_id: TrackId::new(7),
            version: Default::default(),
            base_data_offset: None,
            sample_description_index: None,
            default_sample_duration: Some(512),
            default_sample_size: Some(3933),
            default_sample_flags: None,
            trailing: Vec::new(),
        };
        let b = t.try_to_bytes().unwrap();
        let p = TrackFragmentHeaderBox::parse_body(&b[8..]).unwrap();
        assert_eq!(p.default_sample_duration, Some(512));
        assert_eq!(p.default_sample_size, Some(3933));
        assert_eq!(b, p.try_to_bytes().unwrap());
    }
    #[test]
    fn tfhd_full_flags() {
        // An explicit `base_data_offset` excludes `default-base-is-moof`
        // (§8.8.7.1), so this is every *other* presence bit.
        let flags = TFHD_BASE_DATA_OFFSET_PRESENT
            | TFHD_SAMPLE_DESCRIPTION_INDEX_PRESENT
            | TFHD_DEFAULT_SAMPLE_DURATION_PRESENT
            | TFHD_DEFAULT_SAMPLE_SIZE_PRESENT
            | TFHD_DEFAULT_SAMPLE_FLAGS_PRESENT;
        let t = TrackFragmentHeaderBox {
            flags,
            track_id: TrackId::new(2),
            version: Default::default(),
            base_data_offset: Some(0x1234567890ABCDEF),
            sample_description_index: Some(1),
            default_sample_duration: Some(1024),
            default_sample_size: Some(256),
            default_sample_flags: Some(0x01010000),
            trailing: Vec::new(),
        };
        let b = t.try_to_bytes().unwrap();
        let p = TrackFragmentHeaderBox::parse_body(&b[8..]).unwrap();
        assert_eq!(p.base_data_offset, t.base_data_offset);
        assert_eq!(p.flags, flags);
        assert_eq!(b, p.try_to_bytes().unwrap());
    }

    /// Every optional `tfhd` field is read from its own four bytes
    /// (§8.8.7.1). Each field here carries four distinct non-zero bytes, so a
    /// wrong byte index (`body[c]`, re-reading the first byte, or a backwards
    /// `c - 3`) changes the parsed value instead of happening to agree.
    #[test]
    fn tfhd_optional_fields_read_their_own_bytes() {
        let flags = TFHD_BASE_DATA_OFFSET_PRESENT
            | TFHD_SAMPLE_DESCRIPTION_INDEX_PRESENT
            | TFHD_DEFAULT_SAMPLE_DURATION_PRESENT
            | TFHD_DEFAULT_SAMPLE_SIZE_PRESENT
            | TFHD_DEFAULT_SAMPLE_FLAGS_PRESENT;
        let mut body = Vec::new();
        body.extend_from_slice(&flags.to_be_bytes()); // version 0, flags
        body.extend_from_slice(&7u32.to_be_bytes()); // track_ID
        body.extend_from_slice(&0x1112_1314_1516_1718u64.to_be_bytes()); // base_data_offset
        body.extend_from_slice(&0x2122_2324u32.to_be_bytes()); // sample_description_index
        body.extend_from_slice(&0x3132_3334u32.to_be_bytes()); // default_sample_duration
        body.extend_from_slice(&0x0102_0304u32.to_be_bytes()); // default_sample_size
        body.extend_from_slice(&0x1112_1314u32.to_be_bytes()); // default_sample_flags

        let t = TrackFragmentHeaderBox::parse_body(&body).expect("five-field tfhd body");
        assert_eq!(t.flags, flags);
        assert_eq!(t.track_id, TrackId::new(7));
        assert_eq!(t.base_data_offset, Some(0x1112_1314_1516_1718));
        assert_eq!(t.sample_description_index, Some(0x2122_2324));
        assert_eq!(t.default_sample_duration, Some(0x3132_3334));
        assert_eq!(t.default_sample_size, Some(0x0102_0304));
        assert_eq!(t.default_sample_flags, Some(0x1112_1314));
    }

    /// r05-W16: `tfhd` field presence is flag-driven (§8.8.7.1) while the
    /// values live in `Option`s. The serializer used the *stored* flags and
    /// `unwrap_or(0)` for the values, so a builder that set a presence bit
    /// without the value wrote a `0` — a `base_data_offset` of 0 (the start of
    /// the file, not the fragment) or a `sample_description_index` of 0 where
    /// §8.8.7.1 defines 1 as the first entry. Setting the value without the
    /// bit dropped it silently instead. The bits are now derived from the
    /// values.
    #[test]
    fn tfhd_flags_are_derived_from_the_options() {
        let t = TrackFragmentHeaderBox {
            // No presence bits at all, but every value is present.
            flags: 0,
            track_id: TrackId::new(4),
            version: Default::default(),
            base_data_offset: Some(0x1000),
            sample_description_index: Some(1),
            default_sample_duration: Some(512),
            default_sample_size: Some(99),
            default_sample_flags: Some(0x01010000),
            trailing: Vec::new(),
        };
        let b = t.try_to_bytes().unwrap();
        let p = TrackFragmentHeaderBox::parse_body(&b[8..]).unwrap();
        assert_eq!(p.base_data_offset, Some(0x1000), "base must not become 0");
        assert_eq!(p.sample_description_index, Some(1));
        assert_eq!(p.default_sample_duration, Some(512));
        assert_eq!(p.default_sample_size, Some(99));
        assert_eq!(p.default_sample_flags, Some(0x01010000));
        assert_eq!(
            p.flags,
            TFHD_BASE_DATA_OFFSET_PRESENT
                | TFHD_SAMPLE_DESCRIPTION_INDEX_PRESENT
                | TFHD_DEFAULT_SAMPLE_DURATION_PRESENT
                | TFHD_DEFAULT_SAMPLE_SIZE_PRESENT
                | TFHD_DEFAULT_SAMPLE_FLAGS_PRESENT
        );

        // The reverse direction: a presence bit with no value emits nothing.
        let t = TrackFragmentHeaderBox {
            flags: TFHD_SAMPLE_DESCRIPTION_INDEX_PRESENT,
            track_id: TrackId::new(4),
            version: Default::default(),
            base_data_offset: None,
            sample_description_index: None,
            default_sample_duration: None,
            default_sample_size: None,
            default_sample_flags: None,
            trailing: Vec::new(),
        };
        let b = t.try_to_bytes().unwrap();
        assert_eq!(b.len(), 16, "no value means no field on the wire");
        let p = TrackFragmentHeaderBox::parse_body(&b[8..]).unwrap();
        assert_eq!(p.flags, 0, "the unmatched presence bit is not written");
        assert_eq!(p.sample_description_index, None);
    }

    /// The clearing mask of `effective_flags` must drop every presence bit the
    /// values do not back (§8.8.7.1): a stored bit left behind writes a header
    /// that promises a field the body does not carry, misframing the box and
    /// everything after it. Any other flag bit is metadata and survives.
    #[test]
    fn tfhd_effective_flags_clears_stale_presence_bits() {
        let t = TrackFragmentHeaderBox {
            flags: TFHD_BASE_DATA_OFFSET_PRESENT
                | TFHD_SAMPLE_DESCRIPTION_INDEX_PRESENT
                | TFHD_DEFAULT_SAMPLE_DURATION_PRESENT
                | TFHD_DEFAULT_SAMPLE_SIZE_PRESENT
                | TFHD_DEFAULT_SAMPLE_FLAGS_PRESENT,
            track_id: TrackId::new(9),
            version: Default::default(),
            base_data_offset: None,
            sample_description_index: None,
            default_sample_duration: None,
            default_sample_size: None,
            default_sample_flags: None,
            trailing: Vec::new(),
        };
        assert_eq!(t.effective_flags(), 0, "no value backs any presence bit");
        let b = t.try_to_bytes().expect("serialize");
        assert_eq!(b.len(), 16, "only the box header, FullBox and track_ID");
        let p = TrackFragmentHeaderBox::parse_body(&b[8..]).expect("parse");
        assert_eq!(p.flags, 0, "no presence bit reaches the wire");
        assert_eq!(p.track_id, TrackId::new(9));

        // A non-presence bit is not a field promise: it is preserved.
        let t = TrackFragmentHeaderBox {
            flags: TFHD_DEFAULT_BASE_IS_MOOF
                | TFHD_DEFAULT_SAMPLE_DURATION_PRESENT
                | TFHD_DEFAULT_SAMPLE_SIZE_PRESENT
                | TFHD_DEFAULT_SAMPLE_FLAGS_PRESENT,
            track_id: TrackId::new(9),
            version: Default::default(),
            base_data_offset: None,
            sample_description_index: None,
            default_sample_duration: None,
            default_sample_size: None,
            default_sample_flags: None,
            trailing: Vec::new(),
        };
        assert_eq!(t.effective_flags(), TFHD_DEFAULT_BASE_IS_MOOF);
        let b = t.try_to_bytes().expect("serialize");
        assert_eq!(b.len(), 16, "the preserved bit adds no field");
        let p = TrackFragmentHeaderBox::parse_body(&b[8..]).expect("parse");
        assert_eq!(p.flags, TFHD_DEFAULT_BASE_IS_MOOF);
    }

    /// The `trun` half of r05-W16: presence bits derived from the values, and
    /// a gap in a per-sample field rejected rather than written as `0`.
    #[test]
    fn trun_flags_are_derived_from_the_values() {
        // No `TRUN_DATA_OFFSET_PRESENT`, but `data_offset` is set: the flag
        // used to win and the offset was lost, so the samples resolved against
        // the traf base instead of where the caller pointed them.
        let trun = TrackFragmentRunBox {
            version: 0,
            tr_flags: 0,
            data_offset: Some(716),
            first_sample_flags: None,
            samples: vec![
                TrunSample {
                    sample_duration: Some(512),
                    sample_size: None,
                    sample_flags: None,
                    sample_composition_time_offset: None,
                },
                TrunSample {
                    sample_duration: Some(512),
                    sample_size: None,
                    sample_flags: None,
                    sample_composition_time_offset: None,
                },
            ],
            trailing: Vec::new(),
        };
        let b = trun.try_to_bytes().unwrap();
        let p = TrackFragmentRunBox::parse_body(&b[8..]).unwrap();
        assert_eq!(p.data_offset, Some(716), "data_offset must survive");
        assert_eq!(
            TrackFragmentRunBox::record_stride(p.tr_flags),
            4,
            "only sample_duration"
        );
        assert_eq!(p.samples[0].sample_duration, Some(512));
        assert_eq!(b, p.try_to_bytes().unwrap());

        // A gap in a per-sample field cannot be represented — a `trun` record
        // is fixed-width with no per-record presence bit.
        let gapped = TrackFragmentRunBox {
            version: 0,
            tr_flags: TRUN_SAMPLE_DURATION_PRESENT,
            data_offset: None,
            first_sample_flags: None,
            samples: vec![
                TrunSample {
                    sample_duration: Some(512),
                    sample_size: None,
                    sample_flags: None,
                    sample_composition_time_offset: None,
                },
                TrunSample {
                    sample_duration: None,
                    sample_size: None,
                    sample_flags: None,
                    sample_composition_time_offset: None,
                },
            ],
            trailing: Vec::new(),
        };
        let err = gapped.try_to_bytes().unwrap_err();
        assert!(
            matches!(err, Error::InvalidInput(_)),
            "expected InvalidInput, got {err:?}"
        );
    }

    /// `effective_flags` must clear every per-sample presence bit a non-empty
    /// sample list does not back (§8.8.8.2) and keep any other flag bit: a
    /// stale bit writes a header promising per-sample words the body does not
    /// carry, so every record after it is offset wrongly.
    #[test]
    fn trun_effective_flags_clears_stale_presence_bits() {
        let reserved = 0x0000_1000u32;
        let trun = TrackFragmentRunBox {
            version: 0,
            tr_flags: TRUN_DATA_OFFSET_PRESENT
                | TRUN_FIRST_SAMPLE_FLAGS_PRESENT
                | TRUN_SAMPLE_DURATION_PRESENT
                | TRUN_SAMPLE_SIZE_PRESENT
                | TRUN_SAMPLE_FLAGS_PRESENT
                | TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT
                | reserved,
            data_offset: None,
            first_sample_flags: None,
            samples: vec![TrunSample::new()],
            trailing: Vec::new(),
        };
        assert_eq!(trun.effective_flags(), reserved);
        let b = trun.try_to_bytes().expect("serialize");
        assert_eq!(b.len(), 16, "header, FullBox and sample_count only");
        let p = TrackFragmentRunBox::parse_body(&b[8..]).expect("parse");
        assert_eq!(p.tr_flags, reserved, "no stale presence bit is written");
        assert_eq!(TrackFragmentRunBox::record_stride(p.tr_flags), 0);
        assert_eq!(p.samples.len(), 1);
        assert_eq!(p.samples[0], TrunSample::new());
    }

    /// A v0 `trun`'s `sample_composition_time_offset` is *unsigned*
    /// (§8.8.8.2), so a legal offset of `0x8000_0001` must round-trip as
    /// version 0 — the old model wrapped it to a negative `i32` and then
    /// promoted the box to version 1, changing both its bytes and its meaning
    /// (audit item 4, round 3).
    #[test]
    fn trun_v0_large_unsigned_composition_offset_round_trips() {
        let offset: i64 = 0x8000_0001;
        let trun = TrackFragmentRunBox {
            version: 0,
            tr_flags: 0,
            data_offset: None,
            first_sample_flags: None,
            samples: vec![TrunSample {
                sample_duration: None,
                sample_size: None,
                sample_flags: None,
                sample_composition_time_offset: Some(offset),
            }],
            trailing: Vec::new(),
        };
        let b = trun.try_to_bytes().expect("serialize");
        // v0 + sample_count(4) + the 4-byte offset = 20 bytes.
        assert_eq!(b.len(), 20, "stays version 0");
        assert_eq!(b[8], 0, "version 0");
        assert_eq!(
            &b[16..20],
            &[0x80, 0x00, 0x00, 0x01],
            "the raw unsigned bytes are preserved"
        );
        let p = TrackFragmentRunBox::parse_body(&b[8..]).expect("parse");
        assert_eq!(p.version, 0, "a non-negative offset keeps version 0");
        assert_eq!(p.samples[0].sample_composition_time_offset, Some(offset));
        assert_eq!(b, p.try_to_bytes().unwrap(), "byte-identical round trip");
    }

    /// A negative offset still forces (or keeps) version 1, and a v1 offset
    /// outside the signed 32-bit range is an error rather than a wrap.
    #[test]
    fn trun_v1_negative_offsets_stay_v1_and_out_of_range_errors() {
        let neg = TrackFragmentRunBox {
            version: 1,
            tr_flags: 0,
            data_offset: None,
            first_sample_flags: None,
            samples: vec![TrunSample {
                sample_duration: None,
                sample_size: None,
                sample_flags: None,
                sample_composition_time_offset: Some(-1),
            }],
            trailing: Vec::new(),
        };
        let b = neg.try_to_bytes().expect("serialize v1");
        assert_eq!(b[8], 1, "stays version 1");
        let p = TrackFragmentRunBox::parse_body(&b[8..]).expect("parse");
        assert_eq!(p.samples[0].sample_composition_time_offset, Some(-1));

        // A v1 request with an offset past `i32::MAX` cannot be written.
        let too_big = TrackFragmentRunBox {
            version: 1,
            tr_flags: 0,
            data_offset: None,
            first_sample_flags: None,
            samples: vec![TrunSample {
                sample_duration: None,
                sample_size: None,
                sample_flags: None,
                sample_composition_time_offset: Some(0x8000_0001),
            }],
            trailing: Vec::new(),
        };
        let err = too_big.try_to_bytes().unwrap_err();
        assert!(
            matches!(err, Error::InvalidInput(_)),
            "expected InvalidInput, got {err:?}"
        );
    }

    /// `trun` version 1 makes `sample_composition_time_offset` signed
    /// (§8.8.8.2); a negative offset must never be written under version 0,
    /// where a reader would take the two's-complement bit pattern as a huge
    /// positive offset.
    #[test]
    fn trun_version_is_promoted_for_a_negative_composition_offset() {
        let trun = TrackFragmentRunBox {
            version: 0,
            tr_flags: 0,
            data_offset: None,
            first_sample_flags: None,
            samples: vec![TrunSample {
                sample_duration: None,
                sample_size: None,
                sample_flags: None,
                sample_composition_time_offset: Some(-1),
            }],
            trailing: Vec::new(),
        };
        let b = trun.try_to_bytes().expect("serialize");
        let p = TrackFragmentRunBox::parse_body(&b[8..]).expect("parse");
        assert_eq!(p.version, 1, "a negative offset forces version 1");
        assert_eq!(p.samples[0].sample_composition_time_offset, Some(-1));

        let trun = TrackFragmentRunBox {
            version: 0,
            tr_flags: 0,
            data_offset: None,
            first_sample_flags: None,
            samples: vec![TrunSample {
                sample_duration: None,
                sample_size: None,
                sample_flags: None,
                sample_composition_time_offset: Some(0),
            }],
            trailing: Vec::new(),
        };
        let p = TrackFragmentRunBox::parse_body(&trun.try_to_bytes().unwrap()[8..]).unwrap();
        assert_eq!(p.version, 0, "a non-negative offset keeps version 0");
    }

    /// A `trun` with `sample_count == 0` still carries its flag bits: the box
    /// declares which per-sample fields *would* be present, and a parse ->
    /// serialize must preserve them (a zero-sample run is legal, and a muxer
    /// writes one for a track with nothing in this fragment).
    #[test]
    fn empty_trun_round_trips_its_flag_bits() {
        let tr_flags =
            TRUN_DATA_OFFSET_PRESENT | TRUN_SAMPLE_DURATION_PRESENT | TRUN_SAMPLE_SIZE_PRESENT;
        let trun = TrackFragmentRunBox {
            version: 0,
            tr_flags,
            data_offset: Some(64),
            first_sample_flags: None,
            samples: Vec::new(),
            trailing: Vec::new(),
        };
        let b = trun.try_to_bytes().expect("serialize");
        let p = TrackFragmentRunBox::parse_body(&b[8..]).expect("parse");
        assert!(p.samples.is_empty());
        assert_eq!(
            p.tr_flags, tr_flags,
            "the flag bits survive with no samples"
        );
        assert_eq!(p.data_offset, Some(64));
        assert_eq!(b, p.try_to_bytes().unwrap(), "byte-identical round trip");
    }

    /// §8.8.7.1 says of `default-base-is-moof`: "if base-data-offset-present
    /// is 1, this flag is ignored". A `tfhd` carrying both is therefore
    /// well-formed (merely redundant), so it must parse and re-serialise
    /// byte-identically — a parser is liberal here, and a rewrite must not
    /// quietly drop the redundant bit.
    #[test]
    fn tfhd_with_both_bases_round_trips() {
        let flags = TFHD_DEFAULT_BASE_IS_MOOF | TFHD_BASE_DATA_OFFSET_PRESENT;
        let t = TrackFragmentHeaderBox {
            flags,
            track_id: TrackId::new(1),
            version: Default::default(),
            base_data_offset: Some(0x40),
            sample_description_index: None,
            default_sample_duration: None,
            default_sample_size: None,
            default_sample_flags: None,
            trailing: Vec::new(),
        };
        let b = t
            .try_to_bytes()
            .expect("a redundant default-base-is-moof is legal");
        let p = TrackFragmentHeaderBox::parse_body(&b[8..]).expect("parses");
        assert_eq!(p.flags, flags, "the redundant bit is preserved");
        assert_eq!(p.base_data_offset, Some(0x40));
        assert_eq!(b, p.try_to_bytes().unwrap(), "byte-identical round trip");
    }

    #[test]
    fn tfdt_v0() {
        let t = TrackFragmentBaseMediaDecodeTimeBox::new_v0(12345);
        let b = t.try_to_bytes().unwrap();
        assert_eq!(b.len(), 16);
        let p = TrackFragmentBaseMediaDecodeTimeBox::parse_body(&b[8..]).unwrap();
        assert_eq!(p.base_media_decode_time(), 12345);
        assert!(p.version() == 0);
        assert_eq!(b, p.try_to_bytes().unwrap());
    }
    #[test]
    fn tfdt_v1() {
        let t = TrackFragmentBaseMediaDecodeTimeBox::new_v1(0x123456789AB);
        let b = t.try_to_bytes().unwrap();
        assert_eq!(b.len(), 20);
        let p = TrackFragmentBaseMediaDecodeTimeBox::parse_body(&b[8..]).unwrap();
        assert_eq!(p.base_media_decode_time(), 0x123456789AB);
        assert!(p.version() == 1);
        assert_eq!(b, p.try_to_bytes().unwrap());
    }

    #[test]
    fn trun_subset_flags() {
        let tr = TrackFragmentRunBox {
            version: 0,
            tr_flags: TRUN_DATA_OFFSET_PRESENT
                | TRUN_FIRST_SAMPLE_FLAGS_PRESENT
                | TRUN_SAMPLE_SIZE_PRESENT
                | TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT,
            data_offset: Some(716),
            first_sample_flags: Some(0x02000000),
            samples: vec![
                TrunSample {
                    sample_duration: None,
                    sample_size: Some(3933),
                    sample_flags: None,
                    sample_composition_time_offset: Some(1024),
                },
                TrunSample {
                    sample_duration: None,
                    sample_size: Some(509),
                    sample_flags: None,
                    sample_composition_time_offset: Some(2560),
                },
            ],
            trailing: Vec::new(),
        };
        let b = tr.try_to_bytes().unwrap();
        let p = TrackFragmentRunBox::parse_body(&b[8..]).unwrap();
        assert_eq!(p.samples.len(), 2);
        assert_eq!(p.samples[0].sample_size, Some(3933));
        assert_eq!(p.samples[0].sample_composition_time_offset, Some(1024));
        assert_eq!(b, p.try_to_bytes().unwrap());
    }
    /// `first_sample_flags`, a per-sample `sample_flags` and the version-1
    /// composition offset each read from their own four bytes (§8.8.8.2).
    /// Existing round trips used values whose neighbours coincided (`-1` is
    /// `0xFF` in every byte), so these bodies carry four distinct non-zero
    /// bytes per field.
    #[test]
    fn trun_optional_words_read_their_own_bytes() {
        let flags = TRUN_DATA_OFFSET_PRESENT
            | TRUN_FIRST_SAMPLE_FLAGS_PRESENT
            | TRUN_SAMPLE_DURATION_PRESENT
            | TRUN_SAMPLE_SIZE_PRESENT
            | TRUN_SAMPLE_FLAGS_PRESENT
            | TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT;
        let mut body = Vec::new();
        body.push(1); // version 1
        body.extend_from_slice(&flags.to_be_bytes()[1..]); // flags
        body.extend_from_slice(&1u32.to_be_bytes()); // sample_count
        body.extend_from_slice(&0x1122_3344u32.to_be_bytes()); // data_offset
        body.extend_from_slice(&0x0203_0405u32.to_be_bytes()); // first_sample_flags
        body.extend_from_slice(&0x5152_5354u32.to_be_bytes()); // sample_duration
        body.extend_from_slice(&0x6162_6364u32.to_be_bytes()); // sample_size
        body.extend_from_slice(&0x0A0B_0C0Du32.to_be_bytes()); // sample_flags
        body.extend_from_slice(&0x0001_0203u32.to_be_bytes()); // composition offset

        let run = TrackFragmentRunBox::parse_body(&body).expect("six-field v1 trun");
        assert_eq!(run.version, 1);
        assert_eq!(run.tr_flags, flags);
        assert_eq!(run.data_offset, Some(0x1122_3344));
        assert_eq!(run.first_sample_flags, Some(0x0203_0405));
        assert_eq!(run.samples.len(), 1);
        assert_eq!(run.samples[0].sample_duration, Some(0x5152_5354));
        assert_eq!(run.samples[0].sample_size, Some(0x6162_6364));
        assert_eq!(run.samples[0].sample_flags, Some(0x0A0B_0C0D));
        assert_eq!(
            run.samples[0].sample_composition_time_offset,
            Some(0x0001_0203)
        );

        // Version 0's composition offset is unsigned, from its own bytes.
        let mut body = Vec::new();
        body.push(0); // version 0
        body.extend_from_slice(&TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT.to_be_bytes()[1..]);
        body.extend_from_slice(&1u32.to_be_bytes()); // sample_count
        body.extend_from_slice(&0x0102_0304u32.to_be_bytes()); // composition offset
        let run = TrackFragmentRunBox::parse_body(&body).expect("v0 composition offset");
        assert_eq!(run.version, 0);
        assert_eq!(
            run.samples[0].sample_composition_time_offset,
            Some(0x0102_0304)
        );
    }

    /// A `trun` that declares a per-sample field spends no zero-width
    /// allowance: its entries are four bytes wide each, so the buffer bounds
    /// the count and the caller's shared budget is untouched.
    #[test]
    fn a_four_byte_entry_trun_spends_no_zero_width_budget() {
        let mut body = Vec::new();
        body.push(0); // version 0
        body.extend_from_slice(&TRUN_SAMPLE_DURATION_PRESENT.to_be_bytes()[1..]);
        body.extend_from_slice(&5u32.to_be_bytes()); // sample_count
        for duration in 1..=5u32 {
            body.extend_from_slice(&(duration * 1_000).to_be_bytes());
        }
        let mut budget = 10usize;
        let run = TrackFragmentRunBox::parse_body_budgeted(&body, &mut budget)
            .expect("a payload-bounded trun parses");
        assert_eq!(run.samples.len(), 5);
        assert_eq!(run.samples[4].sample_duration, Some(5_000));
        assert_eq!(budget, 10, "four-byte entries are bounded by the payload");
    }

    #[test]
    fn trun_record_stride() {
        assert_eq!(
            TrackFragmentRunBox::record_stride(
                TRUN_SAMPLE_DURATION_PRESENT | TRUN_SAMPLE_SIZE_PRESENT
            ),
            8
        );
        assert_eq!(
            TrackFragmentRunBox::record_stride(
                TRUN_SAMPLE_DURATION_PRESENT
                    | TRUN_SAMPLE_SIZE_PRESENT
                    | TRUN_SAMPLE_FLAGS_PRESENT
                    | TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT
            ),
            16
        );
    }

    /// A `trun` declaring an optional field whose bytes are not all there must
    /// be a typed `BufferTooShort` naming that field, never a panic from the
    /// direct index behind the guard (§8.8.8.2).
    #[test]
    fn truncated_trun_optional_fields_report_their_own_name() {
        for (flags, what) in [
            (TRUN_DATA_OFFSET_PRESENT, "trun.do"),
            (TRUN_FIRST_SAMPLE_FLAGS_PRESENT, "trun.fsf"),
            (TRUN_SAMPLE_DURATION_PRESENT, "trun.dur"),
            (TRUN_SAMPLE_SIZE_PRESENT, "trun.sz"),
            (TRUN_SAMPLE_FLAGS_PRESENT, "trun.flg"),
            (TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT, "trun.cto"),
        ] {
            for extra in 0..=3usize {
                let mut body = Vec::new();
                body.extend_from_slice(&flags.to_be_bytes()); // version 0, flags
                body.extend_from_slice(&1u32.to_be_bytes()); // sample_count
                body.extend_from_slice(&[0x5A; 3][..extra]);
                let err = TrackFragmentRunBox::parse_body(&body)
                    .expect_err("a field cut short must be rejected");
                assert_eq!(
                    err,
                    Error::BufferTooShort(BufferTooShort {
                        need: 8,
                        have: 4 + extra,
                        what,
                    }),
                    "flags {flags:#x} with {extra} field bytes"
                );
            }
        }
    }
    #[test]
    fn trun_mutation_changes_bytes() {
        let t = TrackFragmentRunBox {
            version: 0,
            tr_flags: TRUN_SAMPLE_DURATION_PRESENT | TRUN_SAMPLE_SIZE_PRESENT,
            data_offset: None,
            first_sample_flags: None,
            samples: vec![TrunSample {
                sample_duration: Some(1024),
                sample_size: Some(256),
                sample_flags: None,
                sample_composition_time_offset: None,
            }],
            trailing: Vec::new(),
        };
        let orig = t.try_to_bytes().unwrap();
        let m = TrackFragmentRunBox {
            version: 0,
            tr_flags: TRUN_SAMPLE_DURATION_PRESENT
                | TRUN_SAMPLE_SIZE_PRESENT
                | TRUN_SAMPLE_FLAGS_PRESENT,
            data_offset: None,
            first_sample_flags: None,
            samples: vec![TrunSample {
                sample_duration: Some(1024),
                sample_size: Some(256),
                sample_flags: Some(0x01000000),
                sample_composition_time_offset: None,
            }],
            trailing: Vec::new(),
        };
        let mb = m.try_to_bytes().unwrap();
        assert_ne!(mb, orig);
        // flags byte at position 10 (0-based: 0-3 size, 4-7 type, 8 ver, 9-11 flags)
        assert_eq!(mb[10] & 0x04, 0x04);
        assert_eq!(orig[10] & 0x04, 0x00);
        // Change tr_flags back, get original bytes
        let m2 = TrackFragmentRunBox {
            tr_flags: t.tr_flags,
            samples: t.samples.clone(),
            ..m
        };
        assert_eq!(m2.try_to_bytes().unwrap(), orig);
    }

    #[test]
    fn traf_round_trip() {
        let tfhd = TrackFragmentHeaderBox {
            flags: TFHD_DEFAULT_SAMPLE_DURATION_PRESENT
                | TFHD_DEFAULT_SAMPLE_SIZE_PRESENT
                | TFHD_DEFAULT_SAMPLE_FLAGS_PRESENT
                | TFHD_DEFAULT_BASE_IS_MOOF,
            track_id: TrackId::new(1),
            version: Default::default(),
            base_data_offset: None,
            sample_description_index: None,
            default_sample_duration: Some(512),
            default_sample_size: Some(3933),
            default_sample_flags: Some(0x01010000),
            trailing: Vec::new(),
        };
        let tfdt = TrackFragmentBaseMediaDecodeTimeBox::new_v1(0);
        let trun = TrackFragmentRunBox {
            version: 0,
            tr_flags: TRUN_DATA_OFFSET_PRESENT
                | TRUN_FIRST_SAMPLE_FLAGS_PRESENT
                | TRUN_SAMPLE_SIZE_PRESENT
                | TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT,
            data_offset: Some(716),
            first_sample_flags: Some(0x02000000),
            samples: vec![TrunSample {
                sample_duration: None,
                sample_size: Some(3933),
                sample_flags: None,
                sample_composition_time_offset: Some(1024),
            }],
            trailing: Vec::new(),
        };
        let traf = TrackFragmentBox::new(tfhd, Some(tfdt), vec![trun]);
        let b = traf.try_to_bytes().unwrap();
        let p = TrackFragmentBox::parse_body(&b[8..]).unwrap();
        assert_eq!(p.tfhd.track_id, TrackId::new(1));
        assert!(p.tfdt.is_some());
        assert_eq!(p.trun.len(), 1);
        assert_eq!(b, p.try_to_bytes().unwrap());
    }

    #[test]
    fn moof_round_trip() {
        let mfhd = MovieFragmentHeaderBox::new(1);
        let tfhd = TrackFragmentHeaderBox {
            flags: TFHD_DEFAULT_SAMPLE_DURATION_PRESENT
                | TFHD_DEFAULT_SAMPLE_SIZE_PRESENT
                | TFHD_DEFAULT_SAMPLE_FLAGS_PRESENT
                | TFHD_DEFAULT_BASE_IS_MOOF,
            track_id: TrackId::new(1),
            version: Default::default(),
            base_data_offset: None,
            sample_description_index: None,
            default_sample_duration: Some(512),
            default_sample_size: Some(3933),
            default_sample_flags: Some(0x01010000),
            trailing: Vec::new(),
        };
        let tfdt = TrackFragmentBaseMediaDecodeTimeBox::new_v1(0);
        let trun = TrackFragmentRunBox {
            version: 0,
            tr_flags: TRUN_DATA_OFFSET_PRESENT
                | TRUN_FIRST_SAMPLE_FLAGS_PRESENT
                | TRUN_SAMPLE_SIZE_PRESENT
                | TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT,
            data_offset: Some(716),
            first_sample_flags: Some(0x02000000),
            samples: vec![TrunSample {
                sample_duration: None,
                sample_size: Some(3933),
                sample_flags: None,
                sample_composition_time_offset: Some(1024),
            }],
            trailing: Vec::new(),
        };
        let traf = TrackFragmentBox::new(tfhd, Some(tfdt), vec![trun]);
        let moof = MovieFragmentBox::new(mfhd, vec![traf]);
        let b = moof.try_to_bytes().unwrap();
        let p = MovieFragmentBox::parse_body(&b[8..]).unwrap();
        assert_eq!(p.mfhd.sequence_number, 1);
        assert_eq!(p.traf.len(), 1);
        assert_eq!(b, p.try_to_bytes().unwrap());
    }

    // ── r05-W11: a traf with no trun, and unknown children preserved ──────

    /// ISO/IEC 14496-12:2026 `traf` declarations — `trun` is optional (the
    /// syntax is `tfhd` then a `trun` *loop*), and §8.8.7.1's
    /// `duration-is-empty` flag (`0x010000`) exists to declare a fragment
    /// with no samples for one track of a multi-track `moof`. The parser used
    /// to reject such a `traf` with "traf missing trun", which failed the
    /// whole `moof` — and with it `Mp4Demux`, `cenc_decrypt` and
    /// `protect_media_segment` (audit r05-W11).
    #[test]
    fn traf_without_trun_parses() {
        // tfhd only: track 7, duration-is-empty.
        let body: &[u8] = &[
            0, 0, 0, 16, b't', b'f', b'h', b'd', // size, type
            0, 0x01, 0x00, 0x00, // version 0, flags = duration-is-empty
            0, 0, 0, 7, // track_ID
        ];
        let traf = TrackFragmentBox::parse_body(body).expect("trun is optional");
        assert_eq!(traf.tfhd.track_id, TrackId::new(7));
        assert_eq!(traf.tfhd.flags, TFHD_DURATION_IS_EMPTY);
        assert!(traf.trun.is_empty());
        let out = traf.try_to_bytes().unwrap();
        assert_eq!(&out[8..], body, "traf body round-trip");
        assert_eq!(&out[..8], &[0, 0, 0, 24, b't', b'r', b'a', b'f']);
    }

    /// `sbgp`/`sgpd` in a `traf` carry roll-recovery and sample-encryption
    /// (`seig`) signalling; `protect_media_segment` re-serializes any `moof`
    /// it touches, so an unmodelled child used to be silently stripped from a
    /// segment that merely had a track encrypted (audit r05-W11). The wire
    /// order is preserved too, so the round-trip is byte-exact.
    #[test]
    fn traf_preserves_unmodelled_children_in_wire_order() {
        // tfhd(16) + sgpd(20) + trun(20): sgpd sits between them, which is not
        // the order a typed-field serializer would emit.
        let body: &[u8] = &[
            0, 0, 0, 16, b't', b'f', b'h', b'd', 0, 0, 0, 0, 0, 0, 0, 1, // tfhd track 1
            0, 0, 0, 20, b's', b'g', b'p', b'd', 0, 0, 0, 0, b'r', b'o', b'l', b'l', 0, 0, 0, 0, 0,
            0, 0, 16, b't', b'r', b'u', b'n', 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        let traf = TrackFragmentBox::parse_body(body).expect("parses");
        assert_eq!(traf.tfhd.track_id, TrackId::new(1));
        assert_eq!(traf.trun.len(), 1);
        assert_eq!(traf.order.len(), 3);
        let out = traf.try_to_bytes().unwrap();
        assert_eq!(&out[8..], body, "children must round-trip in order");
    }

    /// The `moof` half of the same finding: a `moof` may carry `meta`, and the
    /// child order must survive a rewrite.
    #[test]
    fn moof_preserves_unmodelled_children_in_wire_order() {
        let body: &[u8] = &[
            0, 0, 0, 16, b'm', b'f', b'h', b'd', 0, 0, 0, 0, 0, 0, 0, 3, // mfhd = 3
            0, 0, 0, 12, b'm', b'e', b't', b'a', 0, 0, 0, 0, // meta
            0, 0, 0, 24, b't', b'r', b'a', b'f', // traf
            0, 0, 0, 16, b't', b'f', b'h', b'd', 0, 0, 0, 0, 0, 0, 0, 1,
        ];
        let moof = MovieFragmentBox::parse_body(body).expect("parses");
        assert_eq!(moof.mfhd.sequence_number, 3);
        assert_eq!(moof.traf.len(), 1);
        assert_eq!(moof.order.len(), 3);
        let out = moof.try_to_bytes().unwrap();
        assert_eq!(&out[8..], body, "moof must round-trip byte-identically");
    }

    #[test]
    fn real_fixture_first_moof_byte_identical() {
        let data = std::fs::read(test_fixtures::path("mp4/h264-aac-frag.mp4")).unwrap();
        use crate::box_types::parse_box;
        let mut remaining: &[u8] = &data;
        while !remaining.is_empty() {
            let (bx, consumed) = parse_box(remaining).unwrap();
            if bx.header.box_type.is(b"moof") {
                let moof_bytes = &remaining[..consumed];
                let parsed = MovieFragmentBox::parse_body(bx.body).unwrap();
                assert_eq!(parsed.mfhd.sequence_number, 1);
                assert_eq!(parsed.traf.len(), 2);
                assert_eq!(parsed.traf[0].tfhd.track_id, TrackId::new(1));
                assert_eq!(parsed.traf[1].tfhd.track_id, TrackId::new(2));
                assert!(parsed.traf[0].tfdt.is_some());
                assert_eq!(parsed.traf[0].tfdt.unwrap().base_media_decode_time(), 0);
                assert_eq!(parsed.traf[0].trun.len(), 1);
                assert_eq!(parsed.traf[0].trun[0].samples.len(), 25);
                let serialized = parsed.try_to_bytes().unwrap();
                assert_eq!(serialized.len(), moof_bytes.len());
                assert_eq!(
                    serialized, moof_bytes,
                    "moof round-trip must be byte-identical"
                );
                return;
            }
            if consumed == 0 || consumed >= remaining.len() {
                break;
            }
            remaining = &remaining[consumed..];
        }
        panic!("moof box not found");
    }

    /// `aux_info_type_of` reads a `saiz`/`saio`'s optional `aux_info_type`:
    /// version 1 always carries it, a version-0 box only when flag bit 0 is
    /// set (§8.7.8.2, §8.7.9.2), and a payload too short to hold either is
    /// absent rather than an index past the end.
    #[test]
    fn aux_info_type_of_gates_on_version_and_flag_bit_zero() {
        let zeros = [0u8; 3];
        for len in 0..4usize {
            assert_eq!(
                aux_info_type_of(&zeros[..len], b"saiz"),
                None,
                "{len} bytes is short of the FullBox word"
            );
        }
        assert_eq!(
            aux_info_type_of(&[0, 0, 0, 0, b'c', b'e', b'n', b'c'], b"saiz"),
            None,
            "version 0 without flag bit 0 carries no type"
        );
        assert_eq!(
            aux_info_type_of(&[0, 0, 0, 1, b'c', b'e', b'n', b'c'], b"saiz"),
            Some(*b"cenc"),
            "version 0 with flag bit 0 carries it"
        );
        assert_eq!(
            aux_info_type_of(&[0, 0, 0, 2, b'c', b'e', b'n', b'c'], b"saio"),
            None,
            "another flag bit alone does not"
        );
        assert_eq!(
            aux_info_type_of(&[1, 0, 0, 0, b'c', b'b', b'c', b's'], b"saio"),
            Some(*b"cbcs"),
            "version 1 always carries it"
        );
        assert_eq!(
            aux_info_type_of(&[1, 0, 0, 0], b"saio"),
            None,
            "version 1 with no type bytes is absent"
        );
    }
}
