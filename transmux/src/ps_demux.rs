//! MPEG-2 Program Stream demuxer → hub [`Media`] IR.
//!
//! `PsDemux` is an **input** side of the any-to-any container hub: it consumes a
//! raw MPEG-1/2 Program Stream (`.mpg` / `.vob`) and produces the neutral
//! [`Media`] IR (one [`Track`] per elementary stream, coded samples in decode
//! order), implementing the abstract [`broadcast_common::Unpackage`] trait so
//! `{PS} → IR → {any}` composes with the existing
//! [`CmafMux`](crate::media::CmafMux) / [`HlsPackager`](crate::media::HlsPackager)
//! packagers — mirroring [`TsDemux`](crate::TsDemux).
//!
//! Pipeline: PS pack layer ([`mpeg_ps`]) → per-`stream_id` PES reassembly
//! ([`mpeg_pes`]) → codec-config recovery (H.264 in-band SPS/PPS → `avcC`, AC-3
//! syncframe BSI → `dac3`) → length-prefixed video / raw audio samples.
//!
//! Unlike a Transport Stream, a Program Stream carries no PMT here: the fixture
//! has no [`ProgramStreamMap`](mpeg_ps::ProgramStreamMap), so elementary streams
//! are mapped by `stream_id` (ISO/IEC 13818-1 Table 2-22): H.264 video from the
//! video range 0xE0–0xEF, AC-3 audio from `private_stream_1` (0xBD, with the
//! 4-byte substream header stripped). Unknown `stream_id`s are skipped, never
//! fatal.
//!
//! A PS also does not stamp every frame: one PES packet concatenates several
//! access units and carries a PTS/DTS only for the first one. Video access units
//! are therefore recovered by splitting the reassembled Annex B byte stream on
//! access-unit-delimiter (NAL type 9) boundaries; timing is anchored off the
//! PES-level PTS/DTS that are present and the constant frame duration derived
//! from the stamped decode timestamps.
//!
//! HEVC / DTS / other codecs are not carried here (skipped, never fatal).
//!
//! # Spec
//!
//! - **Program Stream framing (pack header / system header / PSM)**: ISO/IEC
//!   13818-1 (ITU-T H.222.0) §2.5 — via [`mpeg_ps`].
//! - **PES reassembly + PTS/DTS**: ISO/IEC 13818-1 §2.4.3.6 / §2.4.3.7 (via
//!   [`mpeg_pes`], 33-bit @ 90 kHz).
//! - **`stream_id` assignment**: ISO/IEC 13818-1 Table 2-22.
//! - **AC-3 in `private_stream_1`**: ETSI TS 101 154 — the 4-byte substream
//!   header (`substream_id` + `number_of_frames` + `first_access_unit_pointer`)
//!   precedes the AC-3 syncframes.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::marker::PhantomData;

use broadcast_common::Unpackage;
use mpeg_ps::program_stream::parse_all_packs;

use crate::ac3::Ac3SyncframeInfo;
use crate::annexb::iter_annexb_nals;
use crate::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
use crate::error::{Error, Result};
use crate::media::{Media, Track};
use crate::nalu_types::{AvcPps, AvcSps};
use crate::pipeline::{CodecConfig, Sample, TrackSpec};

// ── stream_id → codec (ISO/IEC 13818-1 Table 2-22) ──────────────────────────

/// Low bound of the H.264/video `stream_id` range (`1110 xxxx`, 0xE0–0xEF).
const STREAM_ID_VIDEO_LO: u8 = 0xE0;
/// High bound of the video `stream_id` range.
const STREAM_ID_VIDEO_HI: u8 = 0xEF;
/// `private_stream_1` `stream_id` — carries AC-3 audio (Table 2-22).
const STREAM_ID_PRIVATE_1: u8 = 0xBD;

// ── private_stream_1 (AC-3) substream header ─────────────────────────────────

/// Length of the `private_stream_1` AC-3 substream header before the syncframes:
/// `substream_id`(1) + `number_of_frames`(1) + `first_access_unit_pointer`(2).
const PRIVATE1_AC3_HEADER_LEN: usize = 4;

// ── H.264 NAL / config constants (ISO/IEC 14496-10 / 14496-15) ───────────────

/// NAL length-field width for `mdat` samples: 4-byte prefixes → `lengthSizeMinusOne = 3`.
const NAL_LENGTH_SIZE_MINUS_ONE: u8 = 3;
/// H.264 `nal_unit_type` for an access-unit delimiter (AUD, Table 7-1).
const H264_NAL_AUD: u8 = 9;
/// H.264 `nal_unit_type` for SPS (Table 7-1).
const H264_NAL_SPS: u8 = 7;
/// H.264 `nal_unit_type` for PPS (Table 7-1).
const H264_NAL_PPS: u8 = 8;
/// H.264 `nal_unit_type` for a coded slice of an IDR picture (Table 7-1).
const H264_NAL_IDR: u8 = 5;
/// Mask for the H.264 5-bit `nal_unit_type` in the NAL header byte.
const H264_NAL_TYPE_MASK: u8 = 0x1F;

// ── Timestamps / timescale ───────────────────────────────────────────────────

/// Video media timescale (90 kHz — the PS/PES timestamp clock).
const VIDEO_TIMESCALE: u32 = 90_000;
/// Audio sample size in bits carried in the sample entry (PCM-equivalent; 16).
const AUDIO_SAMPLE_SIZE_BITS: u16 = 16;
/// 33-bit PTS/DTS modulus, for wrap-around unrolling (§2.4.3.7, 90 kHz clock).
const TS_WRAP: i128 = 1 << 33;
/// Half the 33-bit range — the threshold used to detect a backward wrap.
const TS_WRAP_HALF: i128 = TS_WRAP / 2;
/// Fallback per-frame duration (90 kHz ticks) when only one stamped frame exists.
const DEFAULT_FRAME_DURATION: i128 = 3600;

/// Codec class recovered from a `stream_id`. Data-carrying dispatch discriminant,
/// not a spec label enum — hence no `name()`/`Display` (see the
/// `tests/label_coverage.rs` policy).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Codec {
    H264,
    Ac3,
}

impl Codec {
    /// Map a PES `stream_id` to a supported [`Codec`], or `None` if the demuxer
    /// does not carry it (skipped, never fatal).
    fn from_stream_id(stream_id: u8) -> Option<Self> {
        match stream_id {
            STREAM_ID_VIDEO_LO..=STREAM_ID_VIDEO_HI => Some(Codec::H264),
            STREAM_ID_PRIVATE_1 => Some(Codec::Ac3),
            _ => None,
        }
    }
}

/// One elementary stream discovered by `stream_id`, its reassembled elementary
/// byte stream, and the PES-level timestamps captured at each fragment boundary.
struct ElementaryStream {
    codec: Codec,
    /// Concatenated elementary bytes across every PES fragment (Annex B for
    /// video; raw AC-3 syncframes for audio, substream header already stripped).
    es_bytes: Vec<u8>,
    /// PES-level `(byte_offset, pts, dts)` captured at the start of each PES
    /// fragment that carried a PTS. `byte_offset` is the offset into `es_bytes`
    /// at which that fragment's payload begins.
    stamps: Vec<Stamp>,
}

/// A PES-level timestamp anchored at a byte offset within the reassembled ES.
#[derive(Debug, Clone, Copy)]
struct Stamp {
    offset: usize,
    pts: Option<u64>,
    dts: Option<u64>,
}

/// A single recovered access unit with its (optional) presentation/decode
/// timestamps. Video AUs are Annex B; audio "AUs" are raw AC-3 frames.
struct AccessUnit {
    data: Vec<u8>,
    pts: Option<u64>,
    dts: Option<u64>,
}

/// Demux an MPEG-1/2 Program Stream byte slice into a [`Media`].
///
/// Walks the packs, reassembles per-`stream_id` PES into elementary byte
/// streams, splits them into access units (video: Annex B AUD boundaries; audio:
/// AC-3 syncframes), recovers codec config from the in-band headers, and emits
/// length-prefixed video / raw audio samples in decode order.
///
/// The `'a` parameter ties the demuxer to the byte-slice lifetime it consumes via
/// [`Unpackage::Input`]; construct one per call with [`PsDemux::new`].
#[derive(Debug, Default, Clone)]
pub struct PsDemux<'a> {
    _marker: PhantomData<&'a [u8]>,
}

impl<'a> PsDemux<'a> {
    /// Create a new demuxer.
    pub fn new() -> Self {
        Self {
            _marker: PhantomData,
        }
    }

    /// Demux `input` (a whole MPEG-1/2 Program Stream) into a [`Media`].
    ///
    /// This is the inherent form of [`Unpackage::unpackage`]; both produce the
    /// same result. See the type-level docs for the pipeline.
    pub fn demux(&mut self, input: &'a [u8]) -> Result<Media> {
        let (packs, _trailing) = parse_all_packs(input).map_err(Error::Ps)?;

        // ── Pass 1: per-stream_id PES reassembly into elementary byte streams ──
        // Insertion order of `stream_id`s is preserved so tracks come out in the
        // order the streams first appear (video before audio, as stored).
        let mut order: Vec<u8> = Vec::new();
        let mut streams: BTreeMap<u8, ElementaryStream> = BTreeMap::new();

        for pack in &packs {
            for pes in &pack.pes_packets {
                let sid = pes.stream_id.0;
                let Some(codec) = Codec::from_stream_id(sid) else {
                    continue;
                };
                // Strip the private_stream_1 AC-3 substream header from audio
                // payloads; video payloads are the Annex B bytes verbatim.
                let payload: &[u8] = match codec {
                    Codec::Ac3 => {
                        if pes.payload.len() <= PRIVATE1_AC3_HEADER_LEN {
                            continue;
                        }
                        &pes.payload[PRIVATE1_AC3_HEADER_LEN..]
                    }
                    Codec::H264 => pes.payload,
                };
                if payload.is_empty() {
                    continue;
                }
                let (pts, dts) = pes
                    .header
                    .as_ref()
                    .map(|h| (h.pts.map(|p| p.0), h.dts.map(|d| d.0)))
                    .unwrap_or((None, None));

                let es = streams.entry(sid).or_insert_with(|| {
                    order.push(sid);
                    ElementaryStream {
                        codec,
                        es_bytes: Vec::new(),
                        stamps: Vec::new(),
                    }
                });
                let offset = es.es_bytes.len();
                if pts.is_some() || dts.is_some() {
                    es.stamps.push(Stamp { offset, pts, dts });
                }
                es.es_bytes.extend_from_slice(payload);
            }
        }

        // ── Pass 2: build one track per elementary stream, in first-seen order ──
        let mut tracks: Vec<Track> = Vec::new();
        let mut track_id: u32 = 1;
        for sid in &order {
            let es = &streams[sid];
            let built = match es.codec {
                Codec::H264 => build_h264_track(es, track_id),
                Codec::Ac3 => build_ac3_track(es, track_id),
            };
            if let Some(track) = built {
                tracks.push(track);
                track_id += 1;
            }
        }

        Ok(Media::new(tracks, VIDEO_TIMESCALE))
    }
}

impl<'a> Unpackage for PsDemux<'a> {
    type Input = &'a [u8];
    type Media = Media;
    type Error = Error;

    fn unpackage(&mut self, input: &'a [u8]) -> Result<Media> {
        self.demux(input)
    }
}

/// Positions of every start code's first `00` (of the trailing `00 00 01`) in an
/// Annex B byte stream. Used to split the reassembled video ES into access units.
fn start_code_positions(data: &[u8]) -> Vec<usize> {
    let mut positions = Vec::new();
    let n = data.len();
    let mut p = 0usize;
    while p + 3 <= n {
        if data[p] == 0 && data[p + 1] == 0 && data[p + 2] == 1 {
            positions.push(p);
            p += 3;
        } else {
            p += 1;
        }
    }
    positions
}

/// Split a reassembled Annex B byte stream into access units at every
/// access-unit-delimiter (NAL type 9). Each returned slice is a byte range of
/// `data` beginning at the AUD's leading `00 00 01` (or the earlier `00` padding
/// byte, preserved so the length→Annex B→length round-trip stays byte-exact).
///
/// Bytes before the first AUD are attached to the first AU (there are none in a
/// well-formed stream that opens with an AUD).
fn split_access_units(data: &[u8]) -> Vec<(usize, usize)> {
    let codes = start_code_positions(data);
    // Offsets at which a new access unit begins (each AUD start code).
    let mut au_starts: Vec<usize> = Vec::new();
    for &pos in &codes {
        // NAL header byte follows the 3-byte `00 00 01`.
        if pos + 3 < data.len() && (data[pos + 3] & H264_NAL_TYPE_MASK) == H264_NAL_AUD {
            // Include any single leading `00` (4-byte start code) in the AU.
            let start = if pos > 0 && data[pos - 1] == 0 {
                pos - 1
            } else {
                pos
            };
            au_starts.push(start);
        }
    }
    if au_starts.is_empty() {
        return if data.is_empty() {
            Vec::new()
        } else {
            alloc::vec![(0, data.len())]
        };
    }
    // First AU absorbs any preamble bytes before the first AUD.
    let mut ranges: Vec<(usize, usize)> = Vec::with_capacity(au_starts.len());
    let n = au_starts.len();
    for i in 0..n {
        let start = if i == 0 { 0 } else { au_starts[i] };
        let end = if i + 1 < n {
            au_starts[i + 1]
        } else {
            data.len()
        };
        ranges.push((start, end));
    }
    ranges
}

/// Extend a running unwrapped timestamp by the delta to the next raw 33-bit
/// value, correcting for a single 90 kHz wrap in either direction (§2.4.3.7).
fn unwrap_ts(prev_unwrapped: i128, prev_raw: u64, raw: u64) -> i128 {
    let mut delta = raw as i128 - prev_raw as i128;
    if delta > TS_WRAP_HALF {
        delta -= TS_WRAP;
    } else if delta < -TS_WRAP_HALF {
        delta += TS_WRAP;
    }
    prev_unwrapped + delta
}

/// Assign each access unit its (optional) PTS/DTS from the PES-level stamps.
///
/// A stamp applies to the first access unit whose byte range begins at or after
/// the stamp's offset — i.e. the first AU that starts in that PES fragment.
/// Timestamps are unwrapped across the 33-bit wrap using stamp (stream) order.
fn assign_stamps(ranges: &[(usize, usize)], stamps: &[Stamp]) -> Vec<(Option<u64>, Option<u64>)> {
    let mut out = alloc::vec![(None, None); ranges.len()];
    // Unwrap the stamp timestamps across the 33-bit wrap, in stamp order.
    let mut si = 0usize;
    let (mut prev_pts_raw, mut prev_pts_uw): (Option<u64>, i128) = (None, 0);
    let (mut prev_dts_raw, mut prev_dts_uw): (Option<u64>, i128) = (None, 0);
    for (ai, &(start, _end)) in ranges.iter().enumerate() {
        // Consume the last stamp whose offset falls at/before this AU's start,
        // preferring the earliest AU that begins in the fragment.
        while si < stamps.len() && stamps[si].offset <= start {
            let s = stamps[si];
            let pts_uw = s.pts.map(|p| match prev_pts_raw {
                Some(pr) => {
                    let uw = unwrap_ts(prev_pts_uw, pr, p);
                    prev_pts_uw = uw;
                    prev_pts_raw = Some(p);
                    uw
                }
                None => {
                    prev_pts_uw = p as i128;
                    prev_pts_raw = Some(p);
                    p as i128
                }
            });
            let dts_uw = s.dts.map(|d| match prev_dts_raw {
                Some(pr) => {
                    let uw = unwrap_ts(prev_dts_uw, pr, d);
                    prev_dts_uw = uw;
                    prev_dts_raw = Some(d);
                    uw
                }
                None => {
                    prev_dts_uw = d as i128;
                    prev_dts_raw = Some(d);
                    d as i128
                }
            });
            // Stamp applies to the AU that begins this fragment (this AU), only
            // if not already stamped (first AU wins for the fragment).
            if out[ai].0.is_none() && out[ai].1.is_none() {
                out[ai] = (pts_uw.map(|v| v as u64), dts_uw.map(|v| v as u64));
            }
            si += 1;
        }
    }
    out
}

/// Recover H.264 config + build video samples (Annex B → length-prefixed).
///
/// Splits the reassembled Annex B stream into access units on AUD boundaries,
/// stamps each with the PES-level PTS/DTS at its fragment start, then emits
/// decode-ordered length-prefixed samples. Returns `None` if in-band SPS/PPS
/// cannot be found (skip, never fatal).
fn build_h264_track(es: &ElementaryStream, track_id: u32) -> Option<Track> {
    let ranges = split_access_units(&es.es_bytes);
    if ranges.is_empty() {
        return None;
    }
    let stamped = assign_stamps(&ranges, &es.stamps);

    // Recover SPS/PPS (first of each) and per-AU IDR flags.
    let mut sps: Option<Vec<u8>> = None;
    let mut pps: Option<Vec<u8>> = None;
    let mut units: Vec<AccessUnit> = Vec::with_capacity(ranges.len());
    for (i, &(start, end)) in ranges.iter().enumerate() {
        let au = &es.es_bytes[start..end];
        for nal in iter_annexb_nals(au) {
            match nal[0] & H264_NAL_TYPE_MASK {
                H264_NAL_SPS if sps.is_none() => sps = Some(nal.to_vec()),
                H264_NAL_PPS if pps.is_none() => pps = Some(nal.to_vec()),
                _ => {}
            }
        }
        units.push(AccessUnit {
            data: au.to_vec(),
            pts: stamped[i].0,
            dts: stamped[i].1,
        });
    }
    let sps = sps?;
    let pps = pps?;
    if sps.len() < 4 {
        return None;
    }

    let record = AVCDecoderConfigurationRecord {
        configuration_version: 1,
        // profile_idc / constraint_flags / level_idc live at SPS bytes 1..=3
        // (after the 1-byte NAL header) — ISO/IEC 14496-15 §5.3.3.1.
        profile_indication: sps[1],
        profile_compatibility: sps[2],
        level_indication: sps[3],
        length_size_minus_one: NAL_LENGTH_SIZE_MINUS_ONE,
        sps: alloc::vec![AvcSps(sps)],
        pps: alloc::vec![AvcPps(pps)],
        chroma_format: None,
        bit_depth_luma_minus8: None,
        bit_depth_chroma_minus8: None,
        sps_ext: alloc::vec![],
    };
    let config = AVCConfigurationBox::new(record);

    // Fill in DTS for unstamped AUs by anchoring off the stamped ones and the
    // constant frame duration, so every sample carries a decode time.
    let dts = interpolate_dts(&units);
    let pts = interpolate_pts(&units, &dts);

    // Decode order = ascending DTS (stable — preserves stream order for ties).
    let mut order: Vec<usize> = (0..units.len()).collect();
    order.sort_by_key(|&i| dts[i]);

    let samples: Vec<Sample> = order
        .iter()
        .enumerate()
        .map(|(pos, &i)| {
            let dur = frame_duration(&order, &dts, pos);
            let is_idr = au_is_idr(&units[i].data);
            let composition_offset = (pts[i] - dts[i]) as i32;
            Sample::from_annexb(&units[i].data, dur, is_idr, composition_offset)
        })
        .collect();

    Some(Track::new(
        TrackSpec::new(
            track_id,
            VIDEO_TIMESCALE,
            CodecConfig::Avc {
                config,
                width: 0,
                height: 0,
            },
        ),
        samples,
    ))
}

/// True if an Annex B access unit contains an IDR slice NAL (type 5).
fn au_is_idr(au: &[u8]) -> bool {
    iter_annexb_nals(au).any(|nal| (nal[0] & H264_NAL_TYPE_MASK) == H264_NAL_IDR)
}

/// Derive the constant per-frame duration (90 kHz ticks) from the stamped DTS
/// deltas, falling back to [`DEFAULT_FRAME_DURATION`] when fewer than two are
/// known.
fn stamped_frame_duration(units: &[AccessUnit]) -> i128 {
    let stamped: Vec<(usize, i128)> = units
        .iter()
        .enumerate()
        .filter_map(|(i, u)| u.dts.map(|d| (i, d as i128)))
        .collect();
    if stamped.len() < 2 {
        return DEFAULT_FRAME_DURATION;
    }
    let (i0, d0) = stamped[0];
    let (i1, d1) = *stamped.last().unwrap();
    let span_idx = (i1 - i0) as i128;
    let span_dts = d1 - d0;
    if span_idx > 0 && span_dts > 0 {
        (span_dts / span_idx).max(1)
    } else {
        DEFAULT_FRAME_DURATION
    }
}

/// Per-AU decode timestamp: the explicit PES DTS where present, else anchored off
/// the nearest known DTS by the constant frame duration (index distance).
fn interpolate_dts(units: &[AccessUnit]) -> Vec<i128> {
    let dur = stamped_frame_duration(units);
    let n = units.len();
    let mut dts = alloc::vec![0i128; n];
    // Find the first stamped anchor to seed the whole run.
    let anchor = units
        .iter()
        .enumerate()
        .find_map(|(i, u)| u.dts.map(|d| (i, d as i128)));
    let (anchor_idx, anchor_dts) = anchor.unwrap_or((0, 0));
    for (i, slot) in dts.iter_mut().enumerate() {
        *slot = match units[i].dts {
            Some(d) => d as i128,
            None => anchor_dts + (i as i128 - anchor_idx as i128) * dur,
        };
    }
    dts
}

/// Per-AU presentation timestamp: the explicit PES PTS where present, else the
/// AU's decode time (no reordering information available for unstamped frames).
fn interpolate_pts(units: &[AccessUnit], dts: &[i128]) -> Vec<i128> {
    units
        .iter()
        .enumerate()
        .map(|(i, u)| u.pts.map(|p| p as i128).unwrap_or(dts[i]))
        .collect()
}

/// Duration of the sample at decode position `pos`: the gap to the next
/// decode-ordered DTS; the final sample reuses the previous gap.
fn frame_duration(order: &[usize], dts: &[i128], pos: usize) -> u32 {
    let n = order.len();
    let dur = if pos + 1 < n {
        (dts[order[pos + 1]] - dts[order[pos]]).max(0)
    } else if pos > 0 {
        (dts[order[pos]] - dts[order[pos - 1]]).max(0)
    } else {
        DEFAULT_FRAME_DURATION
    };
    dur as u32
}

/// Split a reassembled AC-3 byte stream into individual syncframes at each
/// `0x0B77` syncword. Each returned slice runs from one syncword to the next.
fn split_ac3_frames(data: &[u8]) -> Vec<(usize, usize)> {
    // AC-3 syncword (0x0B77) offsets.
    let mut syncs: Vec<usize> = Vec::new();
    let mut i = 0usize;
    while i + 1 < data.len() {
        if data[i] == 0x0B && data[i + 1] == 0x77 {
            syncs.push(i);
            i += 2;
        } else {
            i += 1;
        }
    }
    let mut ranges = Vec::with_capacity(syncs.len());
    for k in 0..syncs.len() {
        let start = syncs[k];
        let end = if k + 1 < syncs.len() {
            syncs[k + 1]
        } else {
            data.len()
        };
        ranges.push((start, end));
    }
    ranges
}

/// Recover AC-3 config (syncframe BSI → `dac3`) + one raw sample per syncframe.
/// Returns `None` if no valid AC-3 syncframe is found (skip, never fatal).
fn build_ac3_track(es: &ElementaryStream, track_id: u32) -> Option<Track> {
    let info = Ac3SyncframeInfo::from_es(&es.es_bytes).ok()?;
    let sample_rate = info.sample_rate;
    let channel_count = info.channel_count() as u16;
    let config = info.into_dac3();

    let frames = split_ac3_frames(&es.es_bytes);
    if frames.is_empty() {
        return None;
    }
    let samples: Vec<Sample> = frames
        .iter()
        .map(|&(s, e)| Sample::from_raw(es.es_bytes[s..e].to_vec(), 0))
        .collect();

    Some(Track::new(
        TrackSpec::new(
            track_id,
            sample_rate,
            CodecConfig::Ac3 {
                config,
                channel_count,
                sample_rate,
                sample_size: AUDIO_SAMPLE_SIZE_BITS,
            },
        ),
        samples,
    ))
}
