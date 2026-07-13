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

impl Serialize for TrackFragmentHeaderBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HEADER_SIZE + FULLBOX_EXTRA_SIZE + 4;
        if self.flags & TFHD_BASE_DATA_OFFSET_PRESENT != 0 {
            n += 8;
        }
        if self.flags & TFHD_SAMPLE_DESCRIPTION_INDEX_PRESENT != 0 {
            n += 4;
        }
        if self.flags & TFHD_DEFAULT_SAMPLE_DURATION_PRESENT != 0 {
            n += 4;
        }
        if self.flags & TFHD_DEFAULT_SAMPLE_SIZE_PRESENT != 0 {
            n += 4;
        }
        if self.flags & TFHD_DEFAULT_SAMPLE_FLAGS_PRESENT != 0 {
            n += 4;
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
        let mut c = 0;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"tfhd");
        c += 4;
        buf[c] = 0;
        let fb = self.flags.to_be_bytes();
        buf[c + 1] = fb[1];
        buf[c + 2] = fb[2];
        buf[c + 3] = fb[3];
        c += 4;
        buf[c..c + 4].copy_from_slice(&self.track_id.to_be_bytes());
        c += 4;
        if self.flags & TFHD_BASE_DATA_OFFSET_PRESENT != 0 {
            buf[c..c + 8].copy_from_slice(&self.base_data_offset.unwrap_or(0).to_be_bytes());
            c += 8;
        }
        if self.flags & TFHD_SAMPLE_DESCRIPTION_INDEX_PRESENT != 0 {
            buf[c..c + 4]
                .copy_from_slice(&self.sample_description_index.unwrap_or(1).to_be_bytes());
            c += 4;
        }
        if self.flags & TFHD_DEFAULT_SAMPLE_DURATION_PRESENT != 0 {
            buf[c..c + 4].copy_from_slice(&self.default_sample_duration.unwrap_or(0).to_be_bytes());
            c += 4;
        }
        if self.flags & TFHD_DEFAULT_SAMPLE_SIZE_PRESENT != 0 {
            buf[c..c + 4].copy_from_slice(&self.default_sample_size.unwrap_or(0).to_be_bytes());
            c += 4;
        }
        if self.flags & TFHD_DEFAULT_SAMPLE_FLAGS_PRESENT != 0 {
            buf[c..c + 4].copy_from_slice(&self.default_sample_flags.unwrap_or(0).to_be_bytes());
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
    pub sample_composition_time_offset: Option<i32>,
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

        let mut samples = Vec::with_capacity(sc);
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
                    s.sample_composition_time_offset = Some(i32::from_be_bytes([
                        payload[c],
                        payload[c + 1],
                        payload[c + 2],
                        payload[c + 3],
                    ]));
                } else {
                    s.sample_composition_time_offset = Some(u32::from_be_bytes([
                        payload[c],
                        payload[c + 1],
                        payload[c + 2],
                        payload[c + 3],
                    ]) as i32);
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

impl Serialize for TrackFragmentRunBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HEADER_SIZE + FULLBOX_EXTRA_SIZE + 4;
        if self.tr_flags & TRUN_DATA_OFFSET_PRESENT != 0 {
            n += 4;
        }
        if self.tr_flags & TRUN_FIRST_SAMPLE_FLAGS_PRESENT != 0 {
            n += 4;
        }
        n + self.samples.len() * Self::record_stride(self.tr_flags)
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
        buf[c..c + 4].copy_from_slice(b"trun");
        c += 4;
        buf[c] = self.version;
        let fb = self.tr_flags.to_be_bytes();
        buf[c + 1] = fb[1];
        buf[c + 2] = fb[2];
        buf[c + 3] = fb[3];
        c += 4;
        buf[c..c + 4].copy_from_slice(&(self.samples.len() as u32).to_be_bytes());
        c += 4;
        if self.tr_flags & TRUN_DATA_OFFSET_PRESENT != 0 {
            buf[c..c + 4].copy_from_slice(&self.data_offset.unwrap_or(0).to_be_bytes());
            c += 4;
        }
        if self.tr_flags & TRUN_FIRST_SAMPLE_FLAGS_PRESENT != 0 {
            buf[c..c + 4].copy_from_slice(&self.first_sample_flags.unwrap_or(0).to_be_bytes());
            c += 4;
        }
        let has_dur = self.tr_flags & TRUN_SAMPLE_DURATION_PRESENT != 0;
        let has_sz = self.tr_flags & TRUN_SAMPLE_SIZE_PRESENT != 0;
        let has_flg = self.tr_flags & TRUN_SAMPLE_FLAGS_PRESENT != 0;
        let has_cto = self.tr_flags & TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT != 0;
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
                buf[c..c + 4]
                    .copy_from_slice(&s.sample_composition_time_offset.unwrap_or(0).to_be_bytes());
                c += 4;
            }
        }
        Ok(c)
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TrackFragmentBox {
    pub tfhd: TrackFragmentHeaderBox,
    pub tfdt: Option<TrackFragmentBaseMediaDecodeTimeBox>,
    pub trun: Vec<TrackFragmentRunBox>,
}

impl TrackFragmentBox {
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        use crate::box_types::parse_box;
        let mut tfhd: Option<TrackFragmentHeaderBox> = None;
        let mut tfdt: Option<TrackFragmentBaseMediaDecodeTimeBox> = None;
        let mut trun: Vec<TrackFragmentRunBox> = Vec::new();
        let mut remaining = body;
        while !remaining.is_empty() {
            let (bx, consumed) = parse_box(remaining)?;
            if bx.header.box_type.is(b"tfhd") {
                tfhd = Some(TrackFragmentHeaderBox::parse_body(bx.body)?);
            } else if bx.header.box_type.is(b"tfdt") {
                tfdt = Some(TrackFragmentBaseMediaDecodeTimeBox::parse_body(bx.body)?);
            } else if bx.header.box_type.is(b"trun") {
                trun.push(TrackFragmentRunBox::parse_body(bx.body)?);
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
        if trun.is_empty() {
            return Err(Error::BufferTooShort {
                need: 1,
                have: 0,
                what: "traf missing trun",
            });
        }
        Ok(TrackFragmentBox { tfhd, tfdt, trun })
    }
}

impl Serialize for TrackFragmentBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HEADER_SIZE + self.tfhd.serialized_len();
        if let Some(ref t) = self.tfdt {
            n += t.serialized_len();
        }
        for r in &self.trun {
            n += r.serialized_len();
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
        let mut c = 0;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"traf");
        c += 4;
        c += self.tfhd.serialize_into(&mut buf[c..])?;
        if let Some(ref t) = self.tfdt {
            c += t.serialize_into(&mut buf[c..])?;
        }
        for r in &self.trun {
            c += r.serialize_into(&mut buf[c..])?;
        }
        Ok(c)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MovieFragmentBox {
    pub mfhd: MovieFragmentHeaderBox,
    pub traf: Vec<TrackFragmentBox>,
}

impl MovieFragmentBox {
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        use crate::box_types::parse_box;
        let mut mfhd: Option<MovieFragmentHeaderBox> = None;
        let mut traf: Vec<TrackFragmentBox> = Vec::new();
        let mut remaining = body;
        while !remaining.is_empty() {
            let (bx, consumed) = parse_box(remaining)?;
            if bx.header.box_type.is(b"mfhd") {
                mfhd = Some(MovieFragmentHeaderBox::parse_body(bx.body)?);
            } else if bx.header.box_type.is(b"traf") {
                traf.push(TrackFragmentBox::parse_body(bx.body)?);
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
        Ok(MovieFragmentBox { mfhd, traf })
    }
}

impl Serialize for MovieFragmentBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = BOX_HEADER_SIZE + self.mfhd.serialized_len();
        for t in &self.traf {
            n += t.serialized_len();
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
        let mut c = 0;
        buf[c..c + 4].copy_from_slice(&(need as u32).to_be_bytes());
        c += 4;
        buf[c..c + 4].copy_from_slice(b"moof");
        c += 4;
        c += self.mfhd.serialize_into(&mut buf[c..])?;
        for t in &self.traf {
            c += t.serialize_into(&mut buf[c..])?;
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// CENC movie-fragment protection (`senc`/`saiz`/`saio`) — ISO/IEC 23001-7
// §12.3, ISO/IEC 14496-12:2015 §8.7.8-9 — issue #564 Task 3 (muxer emission).
//
// `senc`/`saiz`/`saio` are deliberately **not** modelled as fields on
// [`TrackFragmentBox`]: every existing call site that constructs one (e.g.
// `pipeline::build_media_segment_with_events`) uses a plain struct literal
// with exactly `{ tfhd, tfdt, trun }`, so adding required fields there would
// force editing that call site too. Instead this is a *post-processing* pass
// over an already-built movie fragment (`moof`), driven by the caller's
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

/// Build the `senc`/`saiz`/`saio` boxes for one protected track's fragment.
/// `saio.offsets[0]` is a placeholder (`0`) — [`protect_media_segment`]
/// back-patches it once every box's final position in the rebuilt `moof` is
/// known.
fn build_cenc_fragment_boxes(p: &FragmentProtection<'_>) -> Result<CencFragmentBoxes> {
    let use_subsamples = p.entries.iter().any(|e| !e.subsamples.is_empty());
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
    let saiz = crate::cenc::SampleAuxInfoSizesBox {
        version: 0,
        flags: 0,
        aux_info_type: None,
        aux_info_type_parameter: None,
        default_sample_info_size: uniform.unwrap_or(0),
        sample_info_sizes: if uniform.is_some() { Vec::new() } else { sizes },
    };
    let saio = crate::cenc::SampleAuxInfoOffsetsBox {
        version: 0,
        flags: 0,
        aux_info_type: None,
        aux_info_type_parameter: None,
        offsets: alloc::vec![0u64],
    };
    Ok(CencFragmentBoxes { senc, saiz, saio })
}

/// Rewrite an already-built **single-fragment** CMAF media segment (`styp`
/// [+ `emsg`*] + one `moof` + `mdat`) so each track named in `protections`
/// gets CENC `senc`/`saiz`/`saio` boxes appended to its `traf` (issue #564
/// Task 3).
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
        built[idx] = Some(build_cenc_fragment_boxes(p)?);
    }

    // Pass A: each traf's "base" (tfhd+tfdt+trun) length, and its final
    // length once its senc/saiz/saio (if protected) are appended.
    let base_lens: Vec<usize> = moof.traf.iter().map(|t| t.serialized_len()).collect();
    let final_lens: Vec<usize> = base_lens
        .iter()
        .zip(&built)
        .map(|(&base, b)| base + b.as_ref().map(CencFragmentBoxes::added_len).unwrap_or(0))
        .collect();

    let mfhd_len = moof.mfhd.serialized_len();
    let new_moof_len = BOX_HEADER_SIZE + mfhd_len + final_lens.iter().sum::<usize>();
    let delta = new_moof_len as i64 - moof_len as i64;

    // Pass B: compute each protected traf's saio.offset[0] (moof-relative)
    // and shift every trun.data_offset by `delta`.
    let mut running = (BOX_HEADER_SIZE + mfhd_len) as u64;
    for (i, traf) in moof.traf.iter_mut().enumerate() {
        if let Some(b) = built[i].as_mut() {
            let senc_start = running + base_lens[i] as u64;
            b.saio.offsets[0] = senc_start + SENC_ENTRIES_OFFSET;
        }
        running += final_lens[i] as u64;

        for run in &mut traf.trun {
            if run.tr_flags & TRUN_DATA_OFFSET_PRESENT != 0 {
                run.data_offset = Some(run.data_offset.unwrap_or(0) + delta as i32);
            }
        }
    }

    // Pass C: serialize the rebuilt moof. `TrackFragmentBox::serialize_into`
    // only knows about tfhd/tfdt/trun, so its own leading size field (the
    // "base" length) is back-patched to the final length once senc/saiz/saio
    // have been appended for a protected traf.
    let mut moof_out = alloc::vec![0u8; new_moof_len];
    moof_out[0..4].copy_from_slice(&(new_moof_len as u32).to_be_bytes());
    moof_out[4..8].copy_from_slice(b"moof");
    let mut c = BOX_HEADER_SIZE;
    c += moof.mfhd.serialize_into(&mut moof_out[c..])?;
    for (i, traf) in moof.traf.iter().enumerate() {
        let start = c;
        c += traf.serialize_into(&mut moof_out[c..])?;
        if let Some(b) = &built[i] {
            c += b.senc.serialize_into(&mut moof_out[c..])?;
            c += b.saiz.serialize_into(&mut moof_out[c..])?;
            c += b.saio.serialize_into(&mut moof_out[c..])?;
            let final_len = (c - start) as u32;
            moof_out[start..start + 4].copy_from_slice(&final_len.to_be_bytes());
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
        let t = TrackFragmentHeaderBox {
            flags: TFHD_BASE_DATA_OFFSET_PRESENT
                | TFHD_SAMPLE_DESCRIPTION_INDEX_PRESENT
                | TFHD_DEFAULT_SAMPLE_DURATION_PRESENT
                | TFHD_DEFAULT_SAMPLE_SIZE_PRESENT
                | TFHD_DEFAULT_SAMPLE_FLAGS_PRESENT
                | TFHD_DEFAULT_BASE_IS_MOOF,
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
        assert_eq!(p.flags, t.flags);
        assert_eq!(b, p.to_bytes());
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
        let traf = TrackFragmentBox {
            tfhd,
            tfdt: Some(tfdt),
            trun: vec![trun],
        };
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
        let traf = TrackFragmentBox {
            tfhd,
            tfdt: Some(tfdt),
            trun: vec![trun],
        };
        let moof = MovieFragmentBox {
            mfhd,
            traf: vec![traf],
        };
        let b = moof.to_bytes();
        let p = MovieFragmentBox::parse_body(&b[8..]).unwrap();
        assert_eq!(p.mfhd.sequence_number, 1);
        assert_eq!(p.traf.len(), 1);
        assert_eq!(b, p.to_bytes());
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
}
