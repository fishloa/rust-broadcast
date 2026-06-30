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

// tfhd flags
#[allow(dead_code)]
const TFHD_BASE_DATA_OFFSET_PRESENT: u32 = 0x000001;
const TFHD_SAMPLE_DESCRIPTION_INDEX_PRESENT: u32 = 0x000002;
const TFHD_DEFAULT_SAMPLE_DURATION_PRESENT: u32 = 0x000008;
const TFHD_DEFAULT_SAMPLE_SIZE_PRESENT: u32 = 0x000010;
const TFHD_DEFAULT_SAMPLE_FLAGS_PRESENT: u32 = 0x000020;
#[allow(dead_code)]
const TFHD_DURATION_IS_EMPTY: u32 = 0x010000;
#[allow(dead_code)]
const TFHD_DEFAULT_BASE_IS_MOOF: u32 = 0x020000;

// trun flags
const TRUN_DATA_OFFSET_PRESENT: u32 = 0x000001;
const TRUN_FIRST_SAMPLE_FLAGS_PRESENT: u32 = 0x000004;
const TRUN_SAMPLE_DURATION_PRESENT: u32 = 0x000100;
const TRUN_SAMPLE_SIZE_PRESENT: u32 = 0x000200;
const TRUN_SAMPLE_FLAGS_PRESENT: u32 = 0x000400;
const TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT: u32 = 0x000800;

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
