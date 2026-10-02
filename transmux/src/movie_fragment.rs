//! Movie Fragment boxes - ISO/IEC 14496-12:2015 Â§8.8.
//!
//! Typed containers for fMP4 segment boxes: `moof` (Movie Fragment | Â§8.8.2)
//! containing `mfhd` (Movie Fragment Header | Â§8.8.2) and one or more `traf`
//! (Track Fragment | Â§8.8.3). Each `traf` contains `tfhd` (Track Fragment Header |
//! Â§8.8.7), optional `tfdt` (Base Media Decode Time | Â§8.8.12),
//! and one or more `trun` (Track Fragment Run | Â§8.8.8).
//!
//! Field presence in `tfhd` and `trun` is **flag-driven** (Â§8.8.7, Â§8.8.8):
//! the `flags` field of the FullBox header determines which optional fields appear
//! on the wire. The serializer recomputes every size from the logical fields
//! (no `self.raw`).

use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::Serialize;

const BOX_HEADER_SIZE: usize = 8;

/// Size of the optional 64-bit `largesize` field that follows a `size` of 1
/// (ISO/IEC 14496-12:2015 §4.2).
const LARGESIZE_SIZE: usize = 8;
const FULLBOX_EXTRA_SIZE: usize = 4;

// tfhd flags — ISO/IEC 14496-12:2015 §8.8.7.1 (public: builders set these).
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

// trun flags — ISO/IEC 14496-12:2015 §8.8.8.1 (public: builders set these).
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

// `sample_flags` words (ISO/IEC 14496-12:2015 §8.8.3.1) shared by the fragment writers.
/// `sample_flags` for a sync sample (I-frame): `sample_depends_on = 2` (does not
/// depend on others), `sample_is_non_sync_sample = 0` — ISO/IEC 14496-12:2015
/// §8.8.3.1. The one copy every fragment builder writes (audit r05-O6).
pub(crate) const SAMPLE_FLAGS_SYNC: u32 = 0x0200_0000;
/// `sample_flags` for a non-sync sample: `sample_depends_on = 1`,
/// `sample_is_non_sync_sample = 1` (§8.8.3.1).
pub(crate) const SAMPLE_FLAGS_NON_SYNC: u32 = 0x0101_0000;
/// `sample_is_non_sync_sample` bit within a 32-bit `sample_flags` word
/// (§8.8.3.1, bit `[16]`). Set = the sample is **not** a sync sample.
pub(crate) const SAMPLE_FLAG_IS_NON_SYNC: u32 = 0x0001_0000;

/// One sample of a media fragment's `trun`, as the segment writers describe it.
pub(crate) struct MediaRunSample {
    /// `sample_duration`, in the track timescale.
    pub duration: u32,
    /// Coded size in bytes.
    pub size: usize,
    /// Sync sample (random-access point).
    pub is_sync: bool,
    /// `pts - dts` composition offset.
    pub composition_offset: i32,
}

/// The `tfhd` + `trun` every media-fragment writer (`build_media_segment`,
/// the LL-DASH chunk, the Smooth fragment) emits for one track: per-sample
/// duration/size/flags, with a v1 signed composition offset only when some
/// sample has a non-zero one (B-frames), `default-base-is-moof` addressing and a
/// placeholder `data_offset` of 0 for the caller to patch once the `moof` size
/// is known. One copy of what had been three (audit r05-O6).
pub(crate) fn media_fragment_run(
    track_id: u32,
    samples: impl Iterator<Item = MediaRunSample> + Clone,
) -> Result<(TrackFragmentHeaderBox, TrackFragmentRunBox)> {
    let any_cts = samples.clone().any(|s| s.composition_offset != 0);
    let mut run_samples = Vec::new();
    for s in samples {
        run_samples.push(TrunSample {
            sample_duration: Some(s.duration),
            sample_size: Some(u32::try_from(s.size).map_err(|_| {
                Error::InvalidInput("sample larger than the 32-bit trun sample_size field")
            })?),
            sample_flags: Some(if s.is_sync {
                SAMPLE_FLAGS_SYNC
            } else {
                SAMPLE_FLAGS_NON_SYNC
            }),
            sample_composition_time_offset: any_cts.then_some(i64::from(s.composition_offset)),
        });
    }
    let mut tr_flags = TRUN_DATA_OFFSET_PRESENT
        | TRUN_SAMPLE_DURATION_PRESENT
        | TRUN_SAMPLE_SIZE_PRESENT
        | TRUN_SAMPLE_FLAGS_PRESENT;
    // Version 1 carries a signed composition offset (needed for B-frames).
    let version = if any_cts {
        tr_flags |= TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT;
        1u8
    } else {
        0u8
    };
    let trun = TrackFragmentRunBox {
        version,
        tr_flags,
        data_offset: Some(0),
        first_sample_flags: None,
        samples: run_samples,
    };
    let tfhd = TrackFragmentHeaderBox {
        flags: TFHD_DEFAULT_BASE_IS_MOOF,
        track_id,
        base_data_offset: None,
        sample_description_index: None,
        default_sample_duration: None,
        default_sample_size: None,
        default_sample_flags: None,
    };
    Ok((tfhd, trun))
}

/// Read version(8) and flags(24) from the body bytes (first 4 bytes of a FullBox payload).
fn read_ver_flags(body: &[u8]) -> Result<(u8, u32)> {
    if body.len() < 4 {
        return Err(Error::BufferTooShort {
            need: 4,
            have: body.len(),
            what: "FullBox version/flags",
        });
    }
    let ver = body[0];
    let flags = u32::from_be_bytes([0, body[1], body[2], body[3]]);
    Ok((ver, flags))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MovieFragmentHeaderBox {
    pub sequence_number: u32,
}

impl MovieFragmentHeaderBox {
    pub fn new(sequence_number: u32) -> Self {
        Self { sequence_number }
    }
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        let (_ver, _flags) = read_ver_flags(body)?;
        let payload = &body[FULLBOX_EXTRA_SIZE..];
        if payload.len() < 4 {
            return Err(Error::BufferTooShort {
                need: 4,
                have: payload.len(),
                what: "mfhd.seq",
            });
        }
        Ok(Self {
            sequence_number: u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]),
        })
    }
}

impl Serialize for MovieFragmentHeaderBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        BOX_HEADER_SIZE + FULLBOX_EXTRA_SIZE + 4
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
        buf[c..c + 4].copy_from_slice(b"mfhd");
        c += 4;
        buf[c..c + 4].copy_from_slice(&[0, 0, 0, 0]);
        c += 4;
        buf[c..c + 4].copy_from_slice(&self.sequence_number.to_be_bytes());
        Ok(c + 4)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TrackFragmentHeaderBox {
    pub flags: u32,
    pub track_id: u32,
    pub base_data_offset: Option<u64>,
    pub sample_description_index: Option<u32>,
    pub default_sample_duration: Option<u32>,
    pub default_sample_size: Option<u32>,
    pub default_sample_flags: Option<u32>,
}

impl TrackFragmentHeaderBox {
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        let (_ver, flags) = read_ver_flags(body)?;
        let mut c = FULLBOX_EXTRA_SIZE;
        if body.len() < c + 4 {
            return Err(Error::BufferTooShort {
                need: c + 4,
                have: body.len(),
                what: "tfhd.track_id",
            });
        }
        let tid = u32::from_be_bytes([body[c], body[c + 1], body[c + 2], body[c + 3]]);
        c += 4;

        let v_bdo = if flags & TFHD_BASE_DATA_OFFSET_PRESENT != 0 {
            if body.len() < c + 8 {
                return Err(Error::BufferTooShort {
                    need: c + 8,
                    have: body.len(),
                    what: "tfhd.bdo",
                });
            }
            let v = u64::from_be_bytes([
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
            Some(v)
        } else {
            None
        };
        let v_sdi = if flags & TFHD_SAMPLE_DESCRIPTION_INDEX_PRESENT != 0 {
            if body.len() < c + 4 {
                return Err(Error::BufferTooShort {
                    need: c + 4,
                    have: body.len(),
                    what: "tfhd.sdi",
                });
            }
            let v = u32::from_be_bytes([body[c], body[c + 1], body[c + 2], body[c + 3]]);
            c += 4;
            Some(v)
        } else {
            None
        };
        let v_dsd = if flags & TFHD_DEFAULT_SAMPLE_DURATION_PRESENT != 0 {
            if body.len() < c + 4 {
                return Err(Error::BufferTooShort {
                    need: c + 4,
                    have: body.len(),
                    what: "tfhd.dsd",
                });
            }
            let v = u32::from_be_bytes([body[c], body[c + 1], body[c + 2], body[c + 3]]);
            c += 4;
            Some(v)
        } else {
            None
        };
        let v_dss = if flags & TFHD_DEFAULT_SAMPLE_SIZE_PRESENT != 0 {
            if body.len() < c + 4 {
                return Err(Error::BufferTooShort {
                    need: c + 4,
                    have: body.len(),
                    what: "tfhd.dss",
                });
            }
            let v = u32::from_be_bytes([body[c], body[c + 1], body[c + 2], body[c + 3]]);
            c += 4;
            Some(v)
        } else {
            None
        };
        let v_dsf = if flags & TFHD_DEFAULT_SAMPLE_FLAGS_PRESENT != 0 {
            if body.len() < c + 4 {
                return Err(Error::BufferTooShort {
                    need: c + 4,
                    have: body.len(),
                    what: "tfhd.dsf",
                });
            }
            let v = u32::from_be_bytes([body[c], body[c + 1], body[c + 2], body[c + 3]]);
            // last tfhd field — no further cursor use
            Some(v)
        } else {
            None
        };
        Ok(TrackFragmentHeaderBox {
            flags,
            track_id: tid,
            base_data_offset: v_bdo,
            sample_description_index: v_sdi,
            default_sample_duration: v_dsd,
            default_sample_size: v_dss,
            default_sample_flags: v_dsf,
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
        let mut n = BOX_HEADER_SIZE + FULLBOX_EXTRA_SIZE + 4;
        if self.base_data_offset.is_some() {
            n += 8;
        }
        if self.sample_description_index.is_some() {
            n += 4;
        }
        if self.default_sample_duration.is_some() {
            n += 4;
        }
        if self.default_sample_size.is_some() {
            n += 4;
        }
        if self.default_sample_flags.is_some() {
            n += 4;
        }
        n
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let flags = self.effective_flags();
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let len = broadcast_common::len::fit_u32(need, "tfhd size")?;
        let mut c = 0;
        buf[c..c + 4].copy_from_slice(&len.to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"tfhd");
        c += 4;
        buf[c] = 0;
        let fb = flags.to_be_bytes();
        buf[c + 1] = fb[1];
        buf[c + 2] = fb[2];
        buf[c + 3] = fb[3];
        c += 4;
        buf[c..c + 4].copy_from_slice(&self.track_id.to_be_bytes());
        c += 4;
        if let Some(v) = self.base_data_offset {
            buf[c..c + 8].copy_from_slice(&v.to_be_bytes());
            c += 8;
        }
        if let Some(v) = self.sample_description_index {
            buf[c..c + 4].copy_from_slice(&v.to_be_bytes());
            c += 4;
        }
        if let Some(v) = self.default_sample_duration {
            buf[c..c + 4].copy_from_slice(&v.to_be_bytes());
            c += 4;
        }
        if let Some(v) = self.default_sample_size {
            buf[c..c + 4].copy_from_slice(&v.to_be_bytes());
            c += 4;
        }
        if let Some(v) = self.default_sample_flags {
            buf[c..c + 4].copy_from_slice(&v.to_be_bytes());
            c += 4;
        }
        Ok(c)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TrackFragmentBaseMediaDecodeTimeBox {
    version: u8,
    v0: u32,
    v1: u64,
}

impl TrackFragmentBaseMediaDecodeTimeBox {
    pub fn new_v0(t: u32) -> Self {
        Self {
            version: 0,
            v0: t,
            v1: t as u64,
        }
    }
    pub fn new_v1(t: u64) -> Self {
        Self {
            version: 1,
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
        let (ver, _flags) = read_ver_flags(body)?;
        let payload = &body[FULLBOX_EXTRA_SIZE..];
        if ver == 0 {
            if payload.len() < 4 {
                return Err(Error::BufferTooShort {
                    need: 4,
                    have: payload.len(),
                    what: "tfdt.v0",
                });
            }
            let v = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
            Ok(Self {
                version: ver,
                v0: v,
                v1: v as u64,
            })
        } else {
            if payload.len() < 8 {
                return Err(Error::BufferTooShort {
                    need: 8,
                    have: payload.len(),
                    what: "tfdt.v1",
                });
            }
            let v = u64::from_be_bytes([
                payload[0], payload[1], payload[2], payload[3], payload[4], payload[5], payload[6],
                payload[7],
            ]);
            Ok(Self {
                version: ver,
                v0: v as u32,
                v1: v,
            })
        }
    }
}

impl Serialize for TrackFragmentBaseMediaDecodeTimeBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        BOX_HEADER_SIZE + FULLBOX_EXTRA_SIZE + if self.version == 0 { 4 } else { 8 }
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
        buf[c..c + 4].copy_from_slice(b"tfdt");
        c += 4;
        buf[c] = self.version;
        buf[c + 1] = 0;
        buf[c + 2] = 0;
        buf[c + 3] = 0;
        c += 4;
        if self.version == 0 {
            buf[c..c + 4].copy_from_slice(&self.v0.to_be_bytes());
            c += 4;
        } else {
            buf[c..c + 8].copy_from_slice(&self.v1.to_be_bytes());
            c += 8;
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
    /// `trun.sample_composition_time_offset` (ISO/IEC 14496-12:2015 §8.8.8.2):
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

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TrackFragmentRunBox {
    pub version: u8,
    pub tr_flags: u32,
    pub data_offset: Option<i32>,
    pub first_sample_flags: Option<u32>,
    pub samples: Vec<TrunSample>,
}

impl TrackFragmentRunBox {
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        let (ver, tr_flags) = read_ver_flags(body)?;
        let payload = &body[FULLBOX_EXTRA_SIZE..];
        if payload.len() < 4 {
            return Err(Error::BufferTooShort {
                need: 4,
                have: payload.len(),
                what: "trun.sc",
            });
        }
        let mut c = 0usize;
        let sc = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize;
        c += 4;

        let data_offset = if tr_flags & TRUN_DATA_OFFSET_PRESENT != 0 {
            if payload.len() < c + 4 {
                return Err(Error::BufferTooShort {
                    need: c + 4,
                    have: payload.len(),
                    what: "trun.do",
                });
            }
            let v =
                i32::from_be_bytes([payload[c], payload[c + 1], payload[c + 2], payload[c + 3]]);
            c += 4;
            Some(v)
        } else {
            None
        };
        let fsf = if tr_flags & TRUN_FIRST_SAMPLE_FLAGS_PRESENT != 0 {
            if payload.len() < c + 4 {
                return Err(Error::BufferTooShort {
                    need: c + 4,
                    have: payload.len(),
                    what: "trun.fsf",
                });
            }
            let v =
                u32::from_be_bytes([payload[c], payload[c + 1], payload[c + 2], payload[c + 3]]);
            c += 4;
            Some(v)
        } else {
            None
        };

        let has_dur = tr_flags & TRUN_SAMPLE_DURATION_PRESENT != 0;
        let has_sz = tr_flags & TRUN_SAMPLE_SIZE_PRESENT != 0;
        let has_flg = tr_flags & TRUN_SAMPLE_FLAGS_PRESENT != 0;
        let has_cto = tr_flags & TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT != 0;

        // Each present per-sample field is a 32-bit word (ISO/IEC 14496-12
        // §8.8.8.2); bound the wire-declared `sample_count` against the
        // remaining buffer before trusting it for `Vec::with_capacity` (#983).
        // The loop below still walks the raw (unbounded) `sc` — each iteration
        // re-checks its own bounds and errors out on truncated input, exactly
        // as the sibling `stts`/`stsc`/etc. parsers do.
        let entry_size = [has_dur, has_sz, has_flg, has_cto]
            .iter()
            .filter(|&&present| present)
            .count()
            * 4;
        let mut samples = Vec::with_capacity(crate::init_segment::bounded_entry_count(
            payload.len().saturating_sub(c),
            entry_size,
            sc,
        ));
        for _ in 0..sc {
            let mut s = TrunSample::new();
            if has_dur {
                if payload.len() < c + 4 {
                    return Err(Error::BufferTooShort {
                        need: c + 4,
                        have: payload.len(),
                        what: "trun.dur",
                    });
                }
                s.sample_duration = Some(u32::from_be_bytes([
                    payload[c],
                    payload[c + 1],
                    payload[c + 2],
                    payload[c + 3],
                ]));
                c += 4;
            }
            if has_sz {
                if payload.len() < c + 4 {
                    return Err(Error::BufferTooShort {
                        need: c + 4,
                        have: payload.len(),
                        what: "trun.sz",
                    });
                }
                s.sample_size = Some(u32::from_be_bytes([
                    payload[c],
                    payload[c + 1],
                    payload[c + 2],
                    payload[c + 3],
                ]));
                c += 4;
            }
            if has_flg {
                if payload.len() < c + 4 {
                    return Err(Error::BufferTooShort {
                        need: c + 4,
                        have: payload.len(),
                        what: "trun.flg",
                    });
                }
                s.sample_flags = Some(u32::from_be_bytes([
                    payload[c],
                    payload[c + 1],
                    payload[c + 2],
                    payload[c + 3],
                ]));
                c += 4;
            }
            if has_cto {
                if payload.len() < c + 4 {
                    return Err(Error::BufferTooShort {
                        need: c + 4,
                        have: payload.len(),
                        what: "trun.cto",
                    });
                }
                if ver == 1 {
                    s.sample_composition_time_offset = Some(i64::from(i32::from_be_bytes([
                        payload[c],
                        payload[c + 1],
                        payload[c + 2],
                        payload[c + 3],
                    ])));
                } else {
                    // Version 0's field is unsigned (§8.8.8.2), so the full
                    // 32-bit range is a legal positive offset.
                    s.sample_composition_time_offset = Some(i64::from(u32::from_be_bytes([
                        payload[c],
                        payload[c + 1],
                        payload[c + 2],
                        payload[c + 3],
                    ])));
                }
                c += 4;
            }
            samples.push(s);
        }
        Ok(TrackFragmentRunBox {
            version: ver,
            tr_flags,
            data_offset,
            first_sample_flags: fsf,
            samples,
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
        (n * 4) as usize
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
        let checks: [FieldCheck; 4] = [
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
        let mut n = BOX_HEADER_SIZE + FULLBOX_EXTRA_SIZE + 4;
        if flags & TRUN_DATA_OFFSET_PRESENT != 0 {
            n += 4;
        }
        if flags & TRUN_FIRST_SAMPLE_FLAGS_PRESENT != 0 {
            n += 4;
        }
        n + self.samples.len() * Self::record_stride(flags)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let flags = self.effective_flags();
        self.check_sample_fields(flags)?;
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let len = broadcast_common::len::fit_u32(need, "trun size")?;
        let mut c = 0;
        buf[c..c + 4].copy_from_slice(&len.to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"trun");
        c += 4;
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
        buf[c] = version;
        let fb = flags.to_be_bytes();
        buf[c + 1] = fb[1];
        buf[c + 2] = fb[2];
        buf[c + 3] = fb[3];
        c += 4;
        let sample_count = broadcast_common::len::fit_u32(self.samples.len(), "sample_count")?;
        buf[c..c + 4].copy_from_slice(&sample_count.to_be_bytes());
        c += 4;
        if let Some(v) = self.data_offset {
            buf[c..c + 4].copy_from_slice(&v.to_be_bytes());
            c += 4;
        }
        if let Some(v) = self.first_sample_flags {
            buf[c..c + 4].copy_from_slice(&v.to_be_bytes());
            c += 4;
        }
        let has_dur = flags & TRUN_SAMPLE_DURATION_PRESENT != 0;
        let has_sz = flags & TRUN_SAMPLE_SIZE_PRESENT != 0;
        let has_flg = flags & TRUN_SAMPLE_FLAGS_PRESENT != 0;
        let has_cto = flags & TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT != 0;
        for s in &self.samples {
            if has_dur {
                buf[c..c + 4].copy_from_slice(&s.sample_duration.unwrap_or(0).to_be_bytes());
                c += 4;
            }
            if has_sz {
                buf[c..c + 4].copy_from_slice(&s.sample_size.unwrap_or(0).to_be_bytes());
                c += 4;
            }
            if has_flg {
                buf[c..c + 4].copy_from_slice(&s.sample_flags.unwrap_or(0).to_be_bytes());
                c += 4;
            }
            if has_cto {
                let offset = s.sample_composition_time_offset.unwrap_or(0);
                if version == 1 {
                    let signed = i32::try_from(offset).map_err(|_| {
                        Error::InvalidInput(
                            "trun v1 sample_composition_time_offset leaves the signed 32-bit range",
                        )
                    })?;
                    buf[c..c + 4].copy_from_slice(&signed.to_be_bytes());
                } else {
                    let unsigned = u32::try_from(offset).map_err(|_| {
                        Error::InvalidInput(
                            "trun v0 sample_composition_time_offset leaves the unsigned 32-bit range",
                        )
                    })?;
                    buf[c..c + 4].copy_from_slice(&unsigned.to_be_bytes());
                }
                c += 4;
            }
        }
        Ok(c)
    }
}
/// A `traf` or `moof` child this crate does not model as a typed field,
/// round-tripped verbatim so a parse → serialize cannot drop it (audit
/// r05-W11).
///
/// `box_type` is the wire four-CC and `data` the payload after the 8-byte
/// `size`+`type` header — which includes the 16-byte `usertype` of a `uuid`
/// child (ISO/IEC 14496-12:2015 §4.2): an ISMV `moov`/`trak` uuid, a Smooth
/// `tfxd`/`tfrf`, or a `uuid`-form `pssh`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct OpaqueChild {
    /// The child's four-CC.
    pub box_type: [u8; 4],
    /// The child's payload after `size`/`type`: the `usertype` of a `uuid`
    /// included, an 8-byte `largesize` field excluded (its form is
    /// [`OpaqueChild::largesize`]).
    pub data: Vec<u8>,
    /// Whether the wire used the `size == 1` + 64-bit `largesize` form
    /// (ISO/IEC 14496-12:2015 §4.2), so the round trip re-emits it.
    pub largesize: bool,
    /// Whether the wire used the `size == 0` form ("extends to the end of the
    /// enclosing container", §4.2). Only written back when this child is the
    /// container's last, so a sibling appended later is not swallowed.
    pub to_end: bool,
}

/// The opaque payload of a child box: everything after `size`/`type` — which
/// keeps a `uuid`'s 16-byte usertype but **not** an 8-byte `largesize` field,
/// whose form is remembered separately ([`OpaqueChild::largesize`]) and
/// re-emitted with the header.
fn opaque_payload(largesize: bool, whole: &[u8]) -> Result<Vec<u8>> {
    let header_size = BOX_HEADER_SIZE + if largesize { LARGESIZE_SIZE } else { 0 };
    if whole.len() < header_size {
        return Err(Error::BufferTooShort {
            need: header_size,
            have: whole.len(),
            what: "opaque child payload",
        });
    }
    // Skip `size`+`type` **and** the 8 `largesize` bytes when that form was
    // used: the serializer writes its own `largesize`, so leaving these in the
    // payload would land them in the body as junk (audit item 1, round 3).
    Ok(whole[header_size..].to_vec())
}

impl OpaqueChild {
    /// Build one from a child's four-CC and payload (everything after
    /// `size`/`type` — the usertype of a `uuid` included; the compact header
    /// form is used).
    pub fn new(box_type: [u8; 4], data: Vec<u8>) -> Self {
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
    pub(crate) fn serialize_child_into(&self, buf: &mut [u8], is_last: bool) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        // See `init_segment::OpaqueBox`: the header is written here, not
        // through `BoxHeader`, because the payload already carries a `uuid`'s
        // usertype.
        let mut c = 0usize;
        if self.to_end && is_last {
            buf[c..c + 4].copy_from_slice(&0u32.to_be_bytes());
            c += 4;
            buf[c..c + 4].copy_from_slice(&self.box_type);
            c += 4;
        } else if self.largesize {
            buf[c..c + 4].copy_from_slice(&1u32.to_be_bytes());
            c += 4;
            buf[c..c + 4].copy_from_slice(&self.box_type);
            c += 4;
            buf[c..c + 8].copy_from_slice(&(need as u64).to_be_bytes());
            c += 8;
        } else {
            let len = broadcast_common::len::fit_u32(need, "opaque child size")?;
            buf[c..c + 4].copy_from_slice(&len.to_be_bytes());
            c += 4;
            buf[c..c + 4].copy_from_slice(&self.box_type);
            c += 4;
        }
        buf[c..c + self.data.len()].copy_from_slice(&self.data);
        Ok(c + self.data.len())
    }
}

impl OpaqueChild {
    /// The header this child writes: 8 bytes, or 16 under
    /// [`OpaqueChild::largesize`] (§4.2).
    fn header_len(&self) -> usize {
        BOX_HEADER_SIZE + if self.largesize { LARGESIZE_SIZE } else { 0 }
    }
}

/// One child of a [`TrackFragmentBox`], in wire order.
///
/// A `traf` carries `tfhd`/`tfdt`/`trun` *and* (per ISO/IEC 14496-12:2015
/// §6.2.3) optional `sbgp`/`sgpd`/`subs`/`saiz`/`saio`/`meta`. This crate
/// models only the first three; [`TrackFragmentBox::order`] keeps every child
/// in the sequence it appeared on the wire, so a parse → serialize — and any
/// rewrite such as [`protect_media_segment`] — reproduces the file's own
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

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TrackFragmentBox {
    pub tfhd: TrackFragmentHeaderBox,
    pub tfdt: Option<TrackFragmentBaseMediaDecodeTimeBox>,
    pub trun: Vec<TrackFragmentRunBox>,
    /// Every child of this `traf` in the order it appeared on the wire.
    ///
    /// A `traf` parsed from bytes therefore round-trips byte-for-byte, and any
    /// child this crate does not model (`sgpd`/`sbgp` roll recovery and
    /// `seig`, `subs`, `sdtp`, an existing `senc`/`saiz`/`saio`, `meta`)
    /// survives a rewrite instead of being silently stripped. An empty `order`
    /// means "one of each typed child, in the conventional sequence" — the
    /// state every builder in this workspace produces; see
    /// [`TrackFragmentBox::into_bytes`].
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
            order: Vec::new(),
        }
    }

    pub fn parse_body(body: &[u8]) -> Result<Self> {
        use crate::box_types::parse_box;
        let mut tfhd: Option<TrackFragmentHeaderBox> = None;
        let mut tfdt: Option<TrackFragmentBaseMediaDecodeTimeBox> = None;
        let mut trun: Vec<TrackFragmentRunBox> = Vec::new();
        let mut order: Vec<TrafChild> = Vec::new();
        let mut remaining = body;
        while !remaining.is_empty() {
            let (bx, consumed) = parse_box(remaining)?;
            if bx.header.box_type.is(b"tfhd") {
                tfhd = Some(TrackFragmentHeaderBox::parse_body(bx.body)?);
                order.push(TrafChild::Tfhd);
            } else if bx.header.box_type.is(b"tfdt") {
                tfdt = Some(TrackFragmentBaseMediaDecodeTimeBox::parse_body(bx.body)?);
                order.push(TrafChild::Tfdt);
            } else if bx.header.box_type.is(b"trun") {
                let parsed = TrackFragmentRunBox::parse_body(bx.body)?;
                order.push(TrafChild::Trun(trun.len()));
                trun.push(parsed);
            } else {
                // `bx.body` starts after the *whole* header, usertype and
                // largesize included, so a `uuid` child (Smooth `tfxd`, a
                // PlayReady `tfrf`) would lose its 16-byte usertype on the way
                // back out. The payload is therefore everything after the
                // 8-byte `size`+`type` header (audit item 1).
                order.push(TrafChild::Opaque(OpaqueChild {
                    box_type: bx.header.box_type.0,
                    data: opaque_payload(bx.header.has_largesize(), &remaining[..consumed])?,
                    largesize: bx.header.has_largesize(),
                    to_end: bx.header.size == 0,
                }));
            }
            if consumed == 0 {
                break;
            }
            remaining = &remaining[consumed.min(remaining.len())..];
        }
        let tfhd = tfhd.ok_or(Error::BufferTooShort {
            need: 1,
            have: 0,
            what: "traf missing tfhd",
        })?;
        // A `traf` with zero `trun`s is legal: §8.8.3/§8.8.8 require none, and
        // the `tfhd` `duration-is-empty` flag (`TFHD_DURATION_IS_EMPTY`) exists
        // precisely to declare a fragment that carries no samples for this
        // track (a sparse track inside an otherwise-populated `moof`). The old
        // "traf missing trun" rejection failed the whole `moof` — and with it
        // `Fmp4Demux`, `cenc_decrypt` and `protect_media_segment` — on
        // conformant input (audit r05-W11).
        Ok(TrackFragmentBox {
            tfhd,
            tfdt,
            trun,
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
        let mut out = Vec::with_capacity(2 + self.trun.len());
        out.push(TrafChild::Tfhd);
        if self.tfdt.is_some() {
            out.push(TrafChild::Tfdt);
        }
        for i in 0..self.trun.len() {
            out.push(TrafChild::Trun(i));
        }
        out
    }

    /// Serialize this `traf` to its own box bytes (header + body).
    ///
    /// Standalone because [`protect_media_segment`] appends `senc`/`saiz`/`saio`
    /// to the `traf` it rewrites and then has to patch the leading size field.
    pub fn into_bytes(&self) -> Result<Vec<u8>> {
        let mut buf = alloc::vec![0u8; self.serialized_len()];
        let n = self.serialize_into(&mut buf)?;
        buf.truncate(n);
        Ok(buf)
    }
}

impl Serialize for TrackFragmentBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HEADER_SIZE;
        for child in self.child_order() {
            n += match child {
                TrafChild::Tfhd => self.tfhd.serialized_len(),
                TrafChild::Tfdt => self.tfdt.as_ref().map_or(0, Serialize::serialized_len),
                TrafChild::Trun(i) => self.trun.get(i).map_or(0, Serialize::serialized_len),
                TrafChild::Opaque(ref o) => o.serialized_len(),
            };
        }
        n
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let len = broadcast_common::len::fit_u32(need, "traf size")?;
        buf[..4].copy_from_slice(&len.to_be_bytes());
        buf[4..8].copy_from_slice(b"traf");
        let mut c = BOX_HEADER_SIZE;
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
        use crate::box_types::parse_box;
        let mut mfhd: Option<MovieFragmentHeaderBox> = None;
        let mut traf: Vec<TrackFragmentBox> = Vec::new();
        let mut order: Vec<MoofChild> = Vec::new();
        let mut remaining = body;
        while !remaining.is_empty() {
            let (bx, consumed) = parse_box(remaining)?;
            if bx.header.box_type.is(b"mfhd") {
                mfhd = Some(MovieFragmentHeaderBox::parse_body(bx.body)?);
                order.push(MoofChild::Mfhd);
            } else if bx.header.box_type.is(b"traf") {
                let parsed = TrackFragmentBox::parse_body(bx.body)?;
                order.push(MoofChild::Traf(traf.len()));
                traf.push(parsed);
            } else {
                order.push(MoofChild::Opaque(OpaqueChild {
                    box_type: bx.header.box_type.0,
                    data: opaque_payload(bx.header.has_largesize(), &remaining[..consumed])?,
                    largesize: bx.header.has_largesize(),
                    to_end: bx.header.size == 0,
                }));
            }
            if consumed == 0 {
                break;
            }
            remaining = &remaining[consumed.min(remaining.len())..];
        }
        let mfhd = mfhd.ok_or(Error::BufferTooShort {
            need: 1,
            have: 0,
            what: "moof missing mfhd",
        })?;
        if traf.is_empty() {
            return Err(Error::BufferTooShort {
                need: 1,
                have: 0,
                what: "moof missing traf",
            });
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

    /// [`MovieFragmentBox::child_order`] for a rewriting caller (it has to be
    /// visible to [`protect_media_segment`], which lives outside this impl).
    pub(crate) fn child_order_for_rewrite(&self) -> Vec<MoofChild> {
        self.child_order()
    }
}

impl Serialize for MovieFragmentBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HEADER_SIZE;
        for child in self.child_order() {
            n += match child {
                MoofChild::Mfhd => self.mfhd.serialized_len(),
                MoofChild::Traf(i) => self.traf.get(i).map_or(0, Serialize::serialized_len),
                MoofChild::Opaque(ref o) => o.serialized_len(),
            };
        }
        n
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let len = broadcast_common::len::fit_u32(need, "moof size")?;
        buf[..4].copy_from_slice(&len.to_be_bytes());
        buf[4..8].copy_from_slice(b"moof");
        let mut c = BOX_HEADER_SIZE;
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
// §12.3, ISO/IEC 14496-12:2015 §8.7.8-9 — issue #564 Task 3 (muxer emission).
//
// `senc`/`saiz`/`saio` are deliberately **not** modelled as fields on
// [`TrackFragmentBox`]: every call site that builds one (e.g.
// `pipeline::build_media_segment_with_events`) goes through
// [`TrackFragmentBox::new`], and the boxes are only known to the caller that
// protects the track. Instead this is a *post-processing* pass over an
// already-built movie fragment (`moof`), driven by the caller's
// [`crate::cenc::SampleEncryptionEntry`] list (from
// [`crate::media::TrackEncryption::samples`]) — it composes with any CMAF
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
const SENC_ENTRIES_OFFSET: u64 = 16;

/// Per-`subsample` fixed fields written by `senc` when
/// [`crate::cenc::SENC_FLAG_USE_SUBSAMPLE_ENCRYPTION`] is set — ISO/IEC
/// 23001-7 §12.3.2: `subsample_count` (2 bytes).
const SAIZ_SUBSAMPLE_COUNT_SIZE: usize = 2;
/// One `(bytes_of_clear_data, bytes_of_protected_data)` subsample entry:
/// 2-byte clear count + 4-byte protected count — ISO/IEC 23001-7 §12.3.2.
const SAIZ_SUBSAMPLE_ENTRY_SIZE: usize = 6;

/// The `aux_info_type` values that name a CENC scheme. ISO/IEC 23001-7 §12.3's
/// `aux_info_type` defaults to `cenc`, and the four-CCs below are the schemes
/// that specification defines.
const CENC_AUX_INFO_TYPES: [[u8; 4]; 4] = [*b"cenc", *b"cens", *b"cbc1", *b"cbcs"];

/// The `aux_info_type` of a `saiz`/`saio` box's opaque payload (the bytes
/// after its 8-byte header), or `None` when absent.
///
/// Both are FullBoxes: a version-0 box carries the field only when flag bit 0
/// is set (§8.7.8.2, §8.7.9.2); version 1 always carries it.
fn aux_info_type_of(payload: &[u8], box_type: &[u8; 4]) -> Option<[u8; 4]> {
    let _ = box_type;
    if payload.len() < 4 {
        return None;
    }
    let version = payload[0];
    let flags = u32::from_be_bytes([0, payload[1], payload[2], payload[3]]);
    if version == 0 && flags & 0x01 == 0 {
        return None;
    }
    let ty = payload.get(4..8)?;
    Some([ty[0], ty[1], ty[2], ty[3]])
}

/// One protected track's per-sample CENC aux info for a single movie
/// fragment, for [`protect_media_segment`] — the movie-fragment half of
/// issue #564's muxer emission ([`crate::init_segment::protect_init_segment`]
/// is the init-segment half).
pub struct FragmentProtection<'a> {
    /// The `tfhd.track_id` of the `traf` to protect.
    pub track_id: u32,
    /// Per-sample IV + subsample map for **this fragment's samples only**, in
    /// decode order. Must have exactly as many entries as the matching
    /// `traf`'s total `trun` sample count (checked). A `Media` muxed across
    /// several CMAF media segments (e.g. via `Segmenter`) protects each
    /// segment with the slice of `Track::encryption`'s samples covering that
    /// segment.
    pub entries: &'a [crate::cenc::SampleEncryptionEntry],
    /// [`crate::cenc::TrackEncryptionBox::default_per_sample_iv_size`] for
    /// this track — the fixed IV length written into `senc`.
    pub per_sample_iv_size: u8,
}

/// The `senc`/`saiz`/`saio` triple built for one protected `traf`.
struct CencFragmentBoxes {
    senc: crate::cenc::SampleEncryptionBox,
    saiz: crate::cenc::SampleAuxInfoSizesBox,
    saio: crate::cenc::SampleAuxInfoOffsetsBox,
}

impl CencFragmentBoxes {
    fn added_len(&self) -> usize {
        self.senc.serialized_len() + self.saiz.serialized_len() + self.saio.serialized_len()
    }
}

/// Build the `senc`/`saiz`/`saio` boxes for one protected track's fragment,
/// or `None` when this track's fragment has nothing meaningful for them to
/// carry (issue R3, ISO/IEC 23001-7 §12.2/§12.3).
///
/// `senc`'s syntax (§12.3) writes, per sample, `InitializationVector[Per_Sample_IV_Size]`
/// then — only when `UseSubSampleEncryption` is set — a subsample map. When a
/// track uses a **constant IV** (`per_sample_iv_size == 0`, the per-sample IV
/// lives once in `tenc.default_constant_IV` instead — §12.2's
/// `default_isProtected==1 && default_Per_Sample_IV_Size==0` case) *and* whole-
/// sample protection (no subsample split needed), every one of `senc`'s
/// `sample_count` entries is **zero bytes wide**: nothing after `sample_count`
/// itself. That box asserts a sample count the bitstream carries no data to
/// corroborate, while recording no per-sample information a decryptor could
/// use — precisely the shape [`crate::cenc::SampleEncryptionBox::parse_body`]
/// rejects as [`Error::InvalidInput`] (a `senc` "declares samples but carries
/// no per-sample data"), and rightly so: a `sample_count`-only box with no
/// backing bytes is unverifiable, not merely unusual. There is nothing this
/// track's fragment needs `senc`/`saiz`/`saio` for at all in that case — a
/// decryptor already has everything it needs from `tenc`'s constant IV and the
/// (subsample-free) whole-sample protection convention — so the correct fix is
/// to omit the triple entirely, not to shrink it into an unparseable shape.
///
/// When the schema is `cbcs` with [`crate::ConstantIvSenc::Emit`] (the
/// default), a constant-IV track instead sets `per_sample_iv_size = 16` in
/// `tenc` and carries the constant IV replicated in every `senc` entry — so
/// this function's normal emission path (below) produces a regular `senc`
/// with 16-byte IVs per sample. The `None` return above is only reached for
/// the [`crate::ConstantIvSenc::Omit`] opt-out.
///
/// `saio.offsets[0]` (when `Some` is returned) is a placeholder (`0`) —
/// [`protect_media_segment`] back-patches it once every box's final position
/// in the rebuilt `moof` is known.
fn build_cenc_fragment_boxes(p: &FragmentProtection<'_>) -> Result<Option<CencFragmentBoxes>> {
    let use_subsamples = p.entries.iter().any(|e| !e.subsamples.is_empty());
    if p.per_sample_iv_size == 0 && !use_subsamples {
        return Ok(None);
    }
    let flags = if use_subsamples {
        crate::cenc::SENC_FLAG_USE_SUBSAMPLE_ENCRYPTION
    } else {
        0
    };
    let senc = crate::cenc::SampleEncryptionBox {
        version: 0,
        flags,
        per_sample_iv_size: p.per_sample_iv_size,
        entries: p.entries.to_vec(),
    };

    let mut sizes = Vec::with_capacity(p.entries.len());
    for e in p.entries {
        let mut sz = p.per_sample_iv_size as usize;
        if use_subsamples {
            sz += SAIZ_SUBSAMPLE_COUNT_SIZE + e.subsamples.len() * SAIZ_SUBSAMPLE_ENTRY_SIZE;
        }
        if sz > u8::MAX as usize {
            return Err(Error::InvalidInput(
                "protect_media_segment: per-sample aux info size exceeds 255 bytes (saiz sample_info_size is u8)",
            ));
        }
        sizes.push(sz as u8);
    }
    let uniform = sizes
        .first()
        .copied()
        .filter(|first| sizes.iter().all(|s| s == first));
    let sample_count = broadcast_common::len::fit_u32(sizes.len(), "saiz sample_count")?;
    let saiz = crate::cenc::SampleAuxInfoSizesBox {
        version: 0,
        flags: 0,
        aux_info_type: None,
        aux_info_type_parameter: None,
        default_sample_info_size: uniform.unwrap_or(0),
        sample_count,
        sample_info_sizes: if uniform.is_some() { Vec::new() } else { sizes },
    };
    let saio = crate::cenc::SampleAuxInfoOffsetsBox {
        version: 0,
        flags: 0,
        aux_info_type: None,
        aux_info_type_parameter: None,
        offsets: alloc::vec![0u64],
    };
    Ok(Some(CencFragmentBoxes { senc, saiz, saio }))
}

/// Rewrite an already-built **single-fragment** CMAF media segment (`styp`
/// [+ `emsg`*] + one `moof` + `mdat`) so each track named in `protections`
/// gets CENC `senc`/`saiz`/`saio` boxes appended to its `traf` (issue #564
/// Task 3) — except a track whose fragment has nothing for those boxes to
/// carry (constant IV + whole-sample protection: see this module's private
/// `build_cenc_fragment_boxes`, issue R3), whose `traf` is left unmodified
/// entirely; its samples are still encrypted, decryptable from `tenc`'s
/// `default_constant_IV` alone.
///
/// This is the normal CMAF media-segment case (one `moof`/`mdat` pair per
/// segment). Only the buffer's *first* `moof` is located and rewritten — a
/// buffer containing more than one `moof` (a multi-fragment segment) is not
/// supported; any `moof`s beyond the first are copied through untouched as
/// part of the verbatim suffix, not protected.
///
/// Every `trun.data_offset` in the (possibly-grown) `moof` — protected track
/// or not — is shifted by the exact number of bytes the new boxes add, so
/// every sample in the unchanged `mdat` still resolves correctly against the
/// `default-base-is-moof` base (see the module docs above for why this, and
/// `saio.offset[0]`, are moof-relative). `media_segment` may be a bare
/// `styp`+`moof`+`mdat` triple or a larger buffer with more boxes before/after
/// (e.g. a whole `CmafMux` output including `ftyp`/`moov`) — only the (first)
/// `moof` span is touched; everything before and after it is copied through
/// verbatim. Returns the input unchanged when `protections` is empty.
pub fn protect_media_segment(
    media_segment: &[u8],
    protections: &[FragmentProtection<'_>],
) -> Result<Vec<u8>> {
    if protections.is_empty() {
        return Ok(media_segment.to_vec());
    }

    let mut prefix_len = 0usize;
    let mut moof_len = None;
    for step in crate::box_types::box_iter(media_segment) {
        let (box_ref, consumed) = step?;
        if box_ref.header.box_type.is(b"moof") {
            moof_len = Some(consumed);
            break;
        }
        prefix_len += consumed;
    }
    let moof_len = moof_len.ok_or(Error::UnexpectedBox { expected: "moof" })?;
    let moof_bytes = &media_segment[prefix_len..prefix_len + moof_len];
    let suffix = &media_segment[prefix_len + moof_len..];

    let mut moof = MovieFragmentBox::parse_body(&moof_bytes[BOX_HEADER_SIZE..])?;

    // Build senc/saiz/saio for each protected traf, index-aligned with
    // `moof.traf`.
    let mut built: Vec<Option<CencFragmentBoxes>> = (0..moof.traf.len()).map(|_| None).collect();
    for p in protections {
        let idx = moof
            .traf
            .iter()
            .position(|t| t.tfhd.track_id == p.track_id)
            .ok_or(Error::InvalidInput(
                "protect_media_segment: track_id not present in moof",
            ))?;
        let sample_count: usize = moof.traf[idx].trun.iter().map(|r| r.samples.len()).sum();
        if sample_count != p.entries.len() {
            return Err(Error::InvalidInput(
                "protect_media_segment: entries.len() must equal the traf's total trun sample count",
            ));
        }
        // A `traf` that already carries a CENC `senc`/`saiz`/`saio` —
        // preserved as opaque children since the W11 fix — would end up with a
        // *second* set appended, and a decryptor has no way to tell which pair
        // applies (ISO/IEC 23001-7 §12.3 defines one `senc` per `traf`).
        // Protection is idempotent-by-rejection: the caller must strip the
        // existing triple (or use the already-protected segment as-is).
        //
        // `senc` is CENC by definition; `saiz`/`saio` are generic
        // auxiliary-information boxes (§8.7.8/§8.7.9) that other schemes also
        // use, identified by `aux_info_type`. Only a CENC-typed pair (or one
        // with the field absent, which §8.7.8.2 defines as `cenc`) would
        // collide, so a differently-typed pair is left alone (audit item 7,
        // round 3).
        for child in &moof.traf[idx].order {
            let TrafChild::Opaque(o) = child else {
                continue;
            };
            let collides = match &o.box_type {
                b"senc" => true,
                b"saiz" | b"saio" => aux_info_type_of(&o.data, &o.box_type)
                    .is_none_or(|ty| CENC_AUX_INFO_TYPES.contains(&ty)),
                _ => false,
            };
            if collides {
                return Err(Error::InvalidInput(
                    "protect_media_segment: traf already carries a CENC senc/saiz/saio",
                ));
            }
        }
        built[idx] = build_cenc_fragment_boxes(p)?;
    }

    // Pass A: each traf's "base" (tfhd/tfdt/trun plus its own opaque
    // children) length, and its final length once its senc/saiz/saio (if
    // protected) are appended.
    let base_lens: Vec<usize> = moof.traf.iter().map(|t| t.serialized_len()).collect();
    let final_lens: Vec<usize> = base_lens
        .iter()
        .zip(&built)
        .map(|(&base, b)| base + b.as_ref().map(CencFragmentBoxes::added_len).unwrap_or(0))
        .collect();

    // The rebuilt `moof` in **wire order**. A `moof` may carry opaque children
    // (`meta`, `pssh`, a `uuid`) at any position, and pass C emits them; the
    // old length was `header + mfhd + Σ traf`, which under-counted by exactly
    // those bytes, so any `moof` with a moof-level opaque child failed the
    // consistency check at the end (audit item 2).
    let order = moof.child_order_for_rewrite();
    let new_moof_len = BOX_HEADER_SIZE
        + order
            .iter()
            .map(|child| match child {
                MoofChild::Mfhd => moof.mfhd.serialized_len(),
                MoofChild::Traf(i) => final_lens.get(*i).copied().unwrap_or(0),
                MoofChild::Opaque(o) => o.serialized_len(),
            })
            .sum::<usize>();
    let delta = new_moof_len as i64 - moof_len as i64;

    // Pass B: compute each protected traf's saio.offset[0] (moof-relative) in
    // the same wire order pass C writes, and shift every trun.data_offset by
    // `delta`.
    let mut running = BOX_HEADER_SIZE as u64;
    for child in &order {
        let traf_index = match child {
            MoofChild::Mfhd => {
                running += moof.mfhd.serialized_len() as u64;
                continue;
            }
            MoofChild::Opaque(o) => {
                running += o.serialized_len() as u64;
                continue;
            }
            MoofChild::Traf(i) => *i,
        };
        let Some(traf) = moof.traf.get_mut(traf_index) else {
            return Err(Error::InvalidInput(
                "protect_media_segment: moof child index out of range",
            ));
        };
        if let Some(b) = built[traf_index].as_mut() {
            let senc_start = running + base_lens[traf_index] as u64;
            b.saio.offsets[0] = senc_start + SENC_ENTRIES_OFFSET;
        }
        running += final_lens[traf_index] as u64;

        for run in &mut traf.trun {
            if run.tr_flags & TRUN_DATA_OFFSET_PRESENT != 0 {
                let current = run.data_offset.ok_or(Error::InvalidInput(
                    "protect_media_segment: trun declares data_offset but carries none",
                ))?;
                let shifted = i64::from(current)
                    .checked_add(delta)
                    .and_then(|v| i32::try_from(v).ok())
                    .ok_or(Error::InvalidInput(
                        "protect_media_segment: trun.data_offset leaves the 32-bit field",
                    ))?;
                run.data_offset = Some(shifted);
            }
        }
    }
    // Pass C: serialize the rebuilt moof. `TrackFragmentBox::serialize_into`
    // only knows about tfhd/tfdt/trun plus its preserved opaque children, so
    // its own leading size field (the "base" length) is back-patched to the
    // final length once senc/saiz/saio have been appended for a protected
    // traf. The `moof`'s own children are emitted in the order they were
    // parsed (opaque children included), so a rewrite cannot reorder or drop
    // them (audit r05-W11).
    let mut moof_out = alloc::vec![0u8; new_moof_len];
    let moof_size = broadcast_common::len::fit_u32(new_moof_len, "moof size")?;
    moof_out[0..4].copy_from_slice(&moof_size.to_be_bytes());
    moof_out[4..8].copy_from_slice(b"moof");
    let mut c = BOX_HEADER_SIZE;
    for child in moof.child_order_for_rewrite() {
        match child {
            MoofChild::Mfhd => c += moof.mfhd.serialize_into(&mut moof_out[c..])?,
            MoofChild::Traf(i) => {
                let start = c;
                let traf = moof.traf.get(i).ok_or(Error::InvalidInput(
                    "protect_media_segment: moof child index out of range",
                ))?;
                c += traf.serialize_into(&mut moof_out[c..])?;
                if let Some(b) = built.get(i).and_then(|b| b.as_ref()) {
                    c += b.senc.serialize_into(&mut moof_out[c..])?;
                    c += b.saiz.serialize_into(&mut moof_out[c..])?;
                    c += b.saio.serialize_into(&mut moof_out[c..])?;
                    let final_len = broadcast_common::len::fit_u32(c - start, "traf size")?;
                    moof_out[start..start + 4].copy_from_slice(&final_len.to_be_bytes());
                }
            }
            MoofChild::Opaque(ref o) => c += o.serialize_into(&mut moof_out[c..])?,
        }
    }
    if c != new_moof_len {
        return Err(Error::InvalidInput(
            "moof length/senc offset consistency check failed",
        ));
    }

    let mut out = Vec::with_capacity(prefix_len + new_moof_len + suffix.len());
    out.extend_from_slice(&media_segment[..prefix_len]);
    out.extend_from_slice(&moof_out);
    out.extend_from_slice(suffix);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn mfhd_round_trip() {
        let m = MovieFragmentHeaderBox::new(42);
        let b = m.to_bytes();
        let p = MovieFragmentHeaderBox::parse_body(&b[8..]).unwrap();
        assert_eq!(p.sequence_number, 42);
        assert_eq!(b, p.to_bytes());
    }
    #[test]
    fn mfhd_len() {
        assert_eq!(MovieFragmentHeaderBox::new(1).serialized_len(), 16);
    }

    #[test]
    fn tfhd_minimal() {
        let t = TrackFragmentHeaderBox {
            flags: 0,
            track_id: 1,
            base_data_offset: None,
            sample_description_index: None,
            default_sample_duration: None,
            default_sample_size: None,
            default_sample_flags: None,
        };
        let b = t.to_bytes();
        let p = TrackFragmentHeaderBox::parse_body(&b[8..]).unwrap();
        assert_eq!(p.track_id, 1);
        assert_eq!(b, p.to_bytes());
    }
    #[test]
    fn tfhd_subset() {
        let t = TrackFragmentHeaderBox {
            flags: TFHD_DEFAULT_SAMPLE_DURATION_PRESENT | TFHD_DEFAULT_SAMPLE_SIZE_PRESENT,
            track_id: 7,
            base_data_offset: None,
            sample_description_index: None,
            default_sample_duration: Some(512),
            default_sample_size: Some(3933),
            default_sample_flags: None,
        };
        let b = t.to_bytes();
        let p = TrackFragmentHeaderBox::parse_body(&b[8..]).unwrap();
        assert_eq!(p.default_sample_duration, Some(512));
        assert_eq!(p.default_sample_size, Some(3933));
        assert_eq!(b, p.to_bytes());
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
            track_id: 2,
            base_data_offset: Some(0x1234567890ABCDEF),
            sample_description_index: Some(1),
            default_sample_duration: Some(1024),
            default_sample_size: Some(256),
            default_sample_flags: Some(0x01010000),
        };
        let b = t.to_bytes();
        let p = TrackFragmentHeaderBox::parse_body(&b[8..]).unwrap();
        assert_eq!(p.base_data_offset, t.base_data_offset);
        assert_eq!(p.flags, flags);
        assert_eq!(b, p.to_bytes());
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
            track_id: 4,
            base_data_offset: Some(0x1000),
            sample_description_index: Some(1),
            default_sample_duration: Some(512),
            default_sample_size: Some(99),
            default_sample_flags: Some(0x01010000),
        };
        let b = t.to_bytes();
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
            track_id: 4,
            base_data_offset: None,
            sample_description_index: None,
            default_sample_duration: None,
            default_sample_size: None,
            default_sample_flags: None,
        };
        let b = t.to_bytes();
        assert_eq!(b.len(), 16, "no value means no field on the wire");
        let p = TrackFragmentHeaderBox::parse_body(&b[8..]).unwrap();
        assert_eq!(p.flags, 0, "the unmatched presence bit is not written");
        assert_eq!(p.sample_description_index, None);
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
        };
        let b = trun.to_bytes();
        let p = TrackFragmentRunBox::parse_body(&b[8..]).unwrap();
        assert_eq!(p.data_offset, Some(716), "data_offset must survive");
        assert_eq!(
            TrackFragmentRunBox::record_stride(p.tr_flags),
            4,
            "only sample_duration"
        );
        assert_eq!(p.samples[0].sample_duration, Some(512));
        assert_eq!(b, p.to_bytes());

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
        };
        let err = gapped.try_to_bytes().unwrap_err();
        assert!(
            matches!(err, Error::InvalidInput(_)),
            "expected InvalidInput, got {err:?}"
        );
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
            track_id: 1,
            base_data_offset: Some(0x40),
            sample_description_index: None,
            default_sample_duration: None,
            default_sample_size: None,
            default_sample_flags: None,
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
        let b = t.to_bytes();
        assert_eq!(b.len(), 16);
        let p = TrackFragmentBaseMediaDecodeTimeBox::parse_body(&b[8..]).unwrap();
        assert_eq!(p.base_media_decode_time(), 12345);
        assert!(p.version() == 0);
        assert_eq!(b, p.to_bytes());
    }
    #[test]
    fn tfdt_v1() {
        let t = TrackFragmentBaseMediaDecodeTimeBox::new_v1(0x123456789AB);
        let b = t.to_bytes();
        assert_eq!(b.len(), 20);
        let p = TrackFragmentBaseMediaDecodeTimeBox::parse_body(&b[8..]).unwrap();
        assert_eq!(p.base_media_decode_time(), 0x123456789AB);
        assert!(p.version() == 1);
        assert_eq!(b, p.to_bytes());
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
        };
        let b = tr.to_bytes();
        let p = TrackFragmentRunBox::parse_body(&b[8..]).unwrap();
        assert_eq!(p.samples.len(), 2);
        assert_eq!(p.samples[0].sample_size, Some(3933));
        assert_eq!(p.samples[0].sample_composition_time_offset, Some(1024));
        assert_eq!(b, p.to_bytes());
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
        };
        let orig = t.to_bytes();
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
        };
        let mb = m.to_bytes();
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
        assert_eq!(m2.to_bytes(), orig);
    }

    #[test]
    fn traf_round_trip() {
        let tfhd = TrackFragmentHeaderBox {
            flags: TFHD_DEFAULT_SAMPLE_DURATION_PRESENT
                | TFHD_DEFAULT_SAMPLE_SIZE_PRESENT
                | TFHD_DEFAULT_SAMPLE_FLAGS_PRESENT
                | TFHD_DEFAULT_BASE_IS_MOOF,
            track_id: 1,
            base_data_offset: None,
            sample_description_index: None,
            default_sample_duration: Some(512),
            default_sample_size: Some(3933),
            default_sample_flags: Some(0x01010000),
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
        };
        let traf = TrackFragmentBox::new(tfhd, Some(tfdt), vec![trun]);
        let b = traf.to_bytes();
        let p = TrackFragmentBox::parse_body(&b[8..]).unwrap();
        assert_eq!(p.tfhd.track_id, 1);
        assert!(p.tfdt.is_some());
        assert_eq!(p.trun.len(), 1);
        assert_eq!(b, p.to_bytes());
    }

    #[test]
    fn moof_round_trip() {
        let mfhd = MovieFragmentHeaderBox::new(1);
        let tfhd = TrackFragmentHeaderBox {
            flags: TFHD_DEFAULT_SAMPLE_DURATION_PRESENT
                | TFHD_DEFAULT_SAMPLE_SIZE_PRESENT
                | TFHD_DEFAULT_SAMPLE_FLAGS_PRESENT
                | TFHD_DEFAULT_BASE_IS_MOOF,
            track_id: 1,
            base_data_offset: None,
            sample_description_index: None,
            default_sample_duration: Some(512),
            default_sample_size: Some(3933),
            default_sample_flags: Some(0x01010000),
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
        };
        let traf = TrackFragmentBox::new(tfhd, Some(tfdt), vec![trun]);
        let moof = MovieFragmentBox::new(mfhd, vec![traf]);
        let b = moof.to_bytes();
        let p = MovieFragmentBox::parse_body(&b[8..]).unwrap();
        assert_eq!(p.mfhd.sequence_number, 1);
        assert_eq!(p.traf.len(), 1);
        assert_eq!(b, p.to_bytes());
    }

    // ── r05-W11: a traf with no trun, and unknown children preserved ──────

    /// ISO/IEC 14496-12:2015 `traf` declarations — `trun` is optional (the
    /// syntax is `tfhd` then a `trun` *loop*), and §8.8.7.1's
    /// `duration-is-empty` flag (`0x010000`) exists to declare a fragment
    /// with no samples for one track of a multi-track `moof`. The parser used
    /// to reject such a `traf` with "traf missing trun", which failed the
    /// whole `moof` — and with it `Fmp4Demux`, `cenc_decrypt` and
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
        assert_eq!(traf.tfhd.track_id, 7);
        assert_eq!(traf.tfhd.flags, TFHD_DURATION_IS_EMPTY);
        assert!(traf.trun.is_empty());
        let out = traf.to_bytes();
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
        assert_eq!(traf.tfhd.track_id, 1);
        assert_eq!(traf.trun.len(), 1);
        assert_eq!(traf.order.len(), 3);
        let out = traf.to_bytes();
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
        let out = moof.to_bytes();
        assert_eq!(&out[8..], body, "moof must round-trip byte-identically");
    }

    #[test]
    fn real_fixture_first_moof_byte_identical() {
        let data = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../fixtures/transmux/h264_aac_frag.mp4"
        ))
        .unwrap();
        use crate::box_types::parse_box;
        let mut remaining: &[u8] = &data;
        while !remaining.is_empty() {
            let (bx, consumed) = parse_box(remaining).unwrap();
            if bx.header.box_type.is(b"moof") {
                let moof_bytes = &remaining[..consumed];
                let parsed = MovieFragmentBox::parse_body(bx.body).unwrap();
                assert_eq!(parsed.mfhd.sequence_number, 1);
                assert_eq!(parsed.traf.len(), 2);
                assert_eq!(parsed.traf[0].tfhd.track_id, 1);
                assert_eq!(parsed.traf[1].tfhd.track_id, 2);
                assert!(parsed.traf[0].tfdt.is_some());
                assert_eq!(parsed.traf[0].tfdt.unwrap().base_media_decode_time(), 0);
                assert_eq!(parsed.traf[0].trun.len(), 1);
                assert_eq!(parsed.traf[0].trun[0].samples.len(), 25);
                let serialized = parsed.to_bytes();
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

    // ── Issue R3: constant-IV + whole-sample `senc` producer/parser mismatch ─

    /// A constant-IV (`per_sample_iv_size == 0`), whole-sample-protected
    /// (`subsamples` empty) track has nothing for `senc` to carry: every one
    /// of `sample_count`'s entries would be zero bytes wide. That degenerate
    /// shape is exactly what [`crate::cenc::SampleEncryptionBox::parse_body`]
    /// rejects — proven directly here against a hand-built body matching what
    /// [`build_cenc_fragment_boxes`] *used* to emit for this case, so the
    /// producer/parser mismatch this issue is about is reproduced, not just
    /// asserted. [`build_cenc_fragment_boxes`] must therefore return `None`.
    #[test]
    fn build_cenc_fragment_boxes_omits_senc_for_constant_iv_whole_sample() {
        // Reproduce the old shape directly: a `senc` FullBox body declaring
        // `sample_count == 2` with `per_sample_iv_size == 0` and no
        // `UseSubSampleEncryption` flag — i.e. every entry is zero bytes.
        let degenerate_senc_body: [u8; 4] = 2u32.to_be_bytes(); // sample_count = 2
        let rejected = crate::cenc::SampleEncryptionBox::parse_body(
            &degenerate_senc_body,
            0,
            0, // flags: UseSubSampleEncryption clear
            0, // per_sample_iv_size
        );
        assert!(
            matches!(rejected, Err(Error::InvalidInput(_))),
            "this crate's own senc parser must reject a sample_count with no backing per-sample \
             bytes, got {rejected:?}"
        );

        // The exact shape that would produce that rejected body: a
        // `FragmentProtection` with a constant IV (`per_sample_iv_size: 0`)
        // and whole-sample protection (empty `subsamples` on every entry).
        let entries = alloc::vec![
            crate::cenc::SampleEncryptionEntry {
                initialization_vector: Vec::new(),
                subsamples: Vec::new(),
            },
            crate::cenc::SampleEncryptionEntry {
                initialization_vector: Vec::new(),
                subsamples: Vec::new(),
            },
        ];
        let protection = FragmentProtection {
            track_id: 1,
            entries: &entries,
            per_sample_iv_size: 0,
        };
        let built = build_cenc_fragment_boxes(&protection).expect("must not error");
        assert!(
            built.is_none(),
            "a constant-IV whole-sample track has nothing for senc/saiz/saio to carry \
             (ISO/IEC 23001-7 §12.2/§12.3) — build_cenc_fragment_boxes must return None, not a \
             senc shape this crate's own parser rejects"
        );
    }

    /// End-to-end: [`protect_media_segment`] over a constant-IV,
    /// whole-sample-protected track must leave the `moof` byte-identical to
    /// the unprotected input — no `senc`/`saiz`/`saio` appended — so there is
    /// nothing for this crate's own hardened `senc` parser to ever reject on
    /// its own output (the hard round-trip invariant this issue is about).
    #[test]
    fn protect_media_segment_constant_iv_whole_sample_round_trips_with_no_senc() {
        const TRACK_ID: u32 = 1;

        let tfhd = TrackFragmentHeaderBox {
            flags: 0,
            track_id: TRACK_ID,
            base_data_offset: None,
            sample_description_index: None,
            default_sample_duration: None,
            default_sample_size: None,
            default_sample_flags: None,
        };
        let trun = TrackFragmentRunBox {
            version: 0,
            tr_flags: TRUN_SAMPLE_SIZE_PRESENT,
            data_offset: None,
            first_sample_flags: None,
            samples: vec![
                TrunSample {
                    sample_duration: None,
                    sample_size: Some(4),
                    sample_flags: None,
                    sample_composition_time_offset: None,
                },
                TrunSample {
                    sample_duration: None,
                    sample_size: Some(4),
                    sample_flags: None,
                    sample_composition_time_offset: None,
                },
            ],
        };
        let traf = TrackFragmentBox::new(tfhd, None, vec![trun]);
        let moof = MovieFragmentBox::new(MovieFragmentHeaderBox::new(1), vec![traf]);
        let unprotected = moof.to_bytes();

        let entries = alloc::vec![
            crate::cenc::SampleEncryptionEntry {
                initialization_vector: Vec::new(),
                subsamples: Vec::new(),
            },
            crate::cenc::SampleEncryptionEntry {
                initialization_vector: Vec::new(),
                subsamples: Vec::new(),
            },
        ];
        let protection = FragmentProtection {
            track_id: TRACK_ID,
            entries: &entries,
            per_sample_iv_size: 0,
        };

        let protected = protect_media_segment(&unprotected, &[protection])
            .expect("protect_media_segment must not error on a constant-IV whole-sample track");
        assert_eq!(
            protected, unprotected,
            "no senc/saiz/saio bytes must be appended when there is nothing for them to carry"
        );

        // Re-parse with this crate's own parser: still a well-formed moof.
        let reparsed = MovieFragmentBox::parse_body(&protected[BOX_HEADER_SIZE..])
            .expect("protected output must still parse as a valid moof");
        assert_eq!(reparsed.traf.len(), 1);
        assert_eq!(reparsed.traf[0].trun[0].samples.len(), 2);
    }
}
