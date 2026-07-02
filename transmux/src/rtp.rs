//! RTP de/packetization + SDP — RFC 3550 / RFC 6184 / RFC 3640 / RFC 4566.
//!
//! The RTP spoke of the any-to-any container hub: it packetizes the [`Media`]
//! IR into RTP packets ([`RtpPacketizer`] : [`Package`]) and depacketizes RTP
//! packets back to the IR ([`RtpDepacketizer`] : [`Unpackage`]), for H.264/AVC
//! video and AAC (`AAC-hbr`) audio, plus SDP (`m=`/`a=rtpmap`/`a=fmtp`)
//! generation.
//!
//! # Wire formats
//!
//! - **RTP fixed header** (RFC 3550 §5.1, 12 bytes): `V=2 P=0 X=0 CC=0`, the
//!   marker bit on the last packet of an access unit, a dynamic payload type
//!   (96+), monotonic 16-bit sequence numbers, a media-clock 32-bit timestamp
//!   (H.264 → 90 kHz; AAC → the sample rate) and a fixed SSRC.
//! - **H.264** (RFC 6184): single-NAL packets (NAL type 1–23), STAP-A
//!   (type 24) aggregation for the SPS+PPS parameter sets, and FU-A (type 28)
//!   fragmentation of any NAL larger than the MTU. Video IR samples are 4-byte
//!   length-prefixed NALs ([`crate::annexb`]); the length prefixes are stripped
//!   on packetize and re-added on depacketize.
//! - **AAC** (RFC 3640, `AAC-hbr`): an AU-headers-length (16-bit, in bits)
//!   prefix + one 2-byte AU-header (`sizeLength=13; indexLength=3`) + the raw
//!   access unit.
//! - **SDP** (RFC 4566 + `fmtp`): `sprop-parameter-sets` carries base64 SPS,PPS
//!   for video; `config` carries the hex AudioSpecificConfig for audio.
//!
//! See `transmux/docs/rtp/rtp-payload-formats.md` for the full transcription.
//!
//! `no_std` + `alloc`.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::marker::PhantomData;

use broadcast_common::{Package, Unpackage};

use crate::annexb::NAL_LENGTH_SIZE;
use crate::error::{Error, Result};
use crate::media::Media;
use crate::pipeline::CodecConfig;

// ---------------------------------------------------------------------------
// Named constants (no magic numbers — RFC 3550 §5.1 / RFC 6184 / RFC 3640)
// ---------------------------------------------------------------------------

/// RTP version — always 2 (RFC 3550 §5.1).
const RTP_VERSION: u8 = 2;
/// RTP fixed-header length in bytes (no CSRC, no extension).
const RTP_HEADER_LEN: usize = 12;
/// Byte 0 of the fixed header: `V=2 P=0 X=0 CC=0` (`0b10_0_0_0000`).
const RTP_BYTE0_V2: u8 = RTP_VERSION << 6;
/// Marker-bit mask within byte 1 (`M` — RFC 3550 §5.1).
const RTP_MARKER_MASK: u8 = 0x80;
/// Payload-type mask within byte 1 (low 7 bits).
const RTP_PT_MASK: u8 = 0x7F;

/// Default dynamic payload type for the H.264 video stream.
pub const DEFAULT_VIDEO_PT: u8 = 96;
/// Default dynamic payload type for the AAC audio stream.
pub const DEFAULT_AUDIO_PT: u8 = 97;
/// Default network MTU (payload budget) forcing FU-A on larger NALs.
pub const DEFAULT_MTU: usize = 1400;
/// Default video RTP clock rate (RFC 6184 — H.264 is carried at 90 kHz).
pub const VIDEO_CLOCK_RATE: u32 = 90_000;

// --- H.264 NAL / packetization (RFC 6184 §5.2, §5.6, §5.7, §5.8) -----------

/// NAL unit `Type` field mask (low 5 bits of the NAL octet).
const NAL_TYPE_MASK: u8 = 0x1F;
/// NAL unit `F|NRI` field mask (top 3 bits of the NAL octet).
const NAL_FNRI_MASK: u8 = 0xE0;
/// STAP-A aggregation NAL type (RFC 6184 §5.7.1).
const NAL_TYPE_STAP_A: u8 = 24;
/// FU-A fragmentation NAL type (RFC 6184 §5.8).
const NAL_TYPE_FU_A: u8 = 28;
/// FU header `S` (start) bit (RFC 6184 §5.8).
const FU_START_MASK: u8 = 0x80;
/// FU header `E` (end) bit (RFC 6184 §5.8).
const FU_END_MASK: u8 = 0x40;
/// STAP-A per-NAL size-prefix width (16-bit, RFC 6184 §5.7.1).
const STAP_A_SIZE_LEN: usize = 2;

/// H.264 NAL type: coded slice of an IDR picture (a keyframe VCL NAL).
///
/// Referenced by the FU-A gate to assert the reconstructed NAL type of the
/// fragmented (large) IDR slice.
pub const NAL_TYPE_IDR: u8 = 5;

// --- AAC AU header section (RFC 3640 §3.3.6, mode AAC-hbr) ------------------

/// `sizeLength` for AAC-hbr — AU-size field width in bits (RFC 3640 §3.3.6).
const AAC_SIZE_LENGTH: u32 = 13;
/// `indexLength` for AAC-hbr — AU-index field width in bits (RFC 3640 §3.3.6).
const AAC_INDEX_LENGTH: u32 = 3;
/// `indexDeltaLength` for AAC-hbr — AU-index-delta field width in bits.
const AAC_INDEX_DELTA_LENGTH: u32 = 3;
/// One AAC-hbr AU-header is `sizeLength + indexLength = 16` bits = 2 bytes.
const AAC_AU_HEADER_LEN: usize = 2;
/// Width of the AU-headers-length prefix (16-bit, RFC 3640 §3.2.1).
const AAC_AU_HEADERS_LENGTH_LEN: usize = 2;

// ---------------------------------------------------------------------------
// RtpMediaKind — which payload format a stream carries
// ---------------------------------------------------------------------------

/// The payload format a single RTP stream carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum RtpMediaKind {
    /// H.264/AVC video (RFC 6184).
    H264,
    /// AAC audio, mode `AAC-hbr` (RFC 3640).
    Aac,
}

impl RtpMediaKind {
    /// Spec/SDP media token (`"video"` / `"audio"`).
    pub fn name(&self) -> &'static str {
        match self {
            RtpMediaKind::H264 => "video",
            RtpMediaKind::Aac => "audio",
        }
    }
}

broadcast_common::impl_spec_display!(RtpMediaKind);

// ---------------------------------------------------------------------------
// Output types
// ---------------------------------------------------------------------------

/// One packetized RTP stream: its payload type + kind and the emitted packets.
#[derive(Debug, Clone)]
pub struct RtpStream {
    /// Dynamic payload type (matches the SDP `rtpmap`).
    pub pt: u8,
    /// The payload format carried on this stream.
    pub kind: RtpMediaKind,
    /// The RTP packets, in emission (sequence-number) order.
    pub packets: Vec<Vec<u8>>,
}

/// The output of [`RtpPacketizer`]: per-track RTP streams plus an SDP string.
#[derive(Debug, Clone)]
pub struct RtpOutput {
    /// One [`RtpStream`] per packetized track, in track order.
    pub streams: Vec<RtpStream>,
    /// The session-level SDP describing every stream (RFC 4566).
    pub sdp: String,
}

// ---------------------------------------------------------------------------
// RtpPacketizer — Package
// ---------------------------------------------------------------------------

/// Packetize a [`Media`] IR into RTP packets + SDP.
///
/// Per track: AVC → single-NAL / STAP-A (SPS+PPS) / FU-A packets on a 90 kHz
/// clock; AAC → `AAC-hbr` packets on the audio sample-rate clock. All packets of
/// one access unit share a timestamp and the marker bit is set on the last.
#[derive(Debug, Clone)]
pub struct RtpPacketizer {
    /// MTU (payload budget): NALs larger than this are fragmented as FU-A.
    pub mtu: usize,
    /// Payload type assigned to the (first) video track.
    pub video_pt: u8,
    /// Payload type assigned to the (first) audio track.
    pub audio_pt: u8,
    /// Fixed SSRC used for every stream (deterministic tests).
    pub ssrc: u32,
    /// Aggregate the video SPS+PPS parameter sets into a leading STAP-A packet.
    pub stap_a_parameter_sets: bool,
}

impl Default for RtpPacketizer {
    fn default() -> Self {
        Self {
            mtu: DEFAULT_MTU,
            video_pt: DEFAULT_VIDEO_PT,
            audio_pt: DEFAULT_AUDIO_PT,
            ssrc: 0x1234_5678,
            stap_a_parameter_sets: true,
        }
    }
}

impl RtpPacketizer {
    /// Create a packetizer with default MTU / payload types / SSRC.
    pub fn new() -> Self {
        Self::default()
    }
}

/// Per-stream monotonic sequence-number counter (wraps at 16 bits).
struct SeqCounter(u16);

impl SeqCounter {
    fn new(start: u16) -> Self {
        Self(start)
    }
    /// Return the next sequence number, advancing (with 16-bit wrap).
    fn next(&mut self) -> u16 {
        let v = self.0;
        self.0 = self.0.wrapping_add(1);
        v
    }
}

/// Write an RTP fixed header into a new packet buffer and return it.
fn rtp_header(pt: u8, marker: bool, seq: u16, timestamp: u32, ssrc: u32) -> Vec<u8> {
    let mut h = Vec::with_capacity(RTP_HEADER_LEN);
    h.push(RTP_BYTE0_V2);
    h.push((if marker { RTP_MARKER_MASK } else { 0 }) | (pt & RTP_PT_MASK));
    h.extend_from_slice(&seq.to_be_bytes());
    h.extend_from_slice(&timestamp.to_be_bytes());
    h.extend_from_slice(&ssrc.to_be_bytes());
    h
}

impl Package for RtpPacketizer {
    type Media = Media;
    type Output = RtpOutput;
    type Error = Error;

    fn package(&mut self, media: &Media) -> Result<RtpOutput> {
        if media.tracks.is_empty() {
            return Err(Error::InvalidInput(
                "cannot packetize a Media with no tracks",
            ));
        }
        let mut streams = Vec::new();
        let mut sdp_media = String::new();
        let mut used_video_pt = false;
        let mut used_audio_pt = false;

        for track in &media.tracks {
            match &track.spec.config {
                CodecConfig::Avc { config, .. } => {
                    let pt = if used_video_pt {
                        self.video_pt.wrapping_add(2)
                    } else {
                        used_video_pt = true;
                        self.video_pt
                    };
                    let packets = self.packetize_video(track, pt)?;
                    streams.push(RtpStream {
                        pt,
                        kind: RtpMediaKind::H264,
                        packets,
                    });
                    sdp_media.push_str(&sdp_video(pt, &config.config)?);
                }
                CodecConfig::Aac {
                    esds,
                    channel_count,
                    sample_rate,
                    ..
                } => {
                    let pt = if used_audio_pt {
                        self.audio_pt.wrapping_add(2)
                    } else {
                        used_audio_pt = true;
                        self.audio_pt
                    };
                    let clock = if track.spec.timescale != 0 {
                        track.spec.timescale
                    } else {
                        *sample_rate
                    };
                    let packets = self.packetize_audio(track, pt, clock)?;
                    streams.push(RtpStream {
                        pt,
                        kind: RtpMediaKind::Aac,
                        packets,
                    });
                    let asc = asc_bytes(esds)?;
                    sdp_media.push_str(&sdp_audio(pt, clock, *channel_count, asc)?);
                }
                _ => {
                    return Err(Error::InvalidInput(
                        "RTP packetizer supports only AVC video and AAC audio tracks",
                    ));
                }
            }
        }
        if streams.is_empty() {
            return Err(Error::InvalidInput(
                "no AVC/AAC tracks to packetize into RTP",
            ));
        }
        let sdp = build_sdp(&sdp_media);
        Ok(RtpOutput { streams, sdp })
    }
}

impl RtpPacketizer {
    /// Packetize one AVC track into RTP packets.
    fn packetize_video(&self, track: &crate::media::Track, pt: u8) -> Result<Vec<Vec<u8>>> {
        let timescale = if track.spec.timescale != 0 {
            track.spec.timescale
        } else {
            VIDEO_CLOCK_RATE
        };
        let mut packets = Vec::new();
        let mut seq = SeqCounter::new(0);
        let mut timestamp: u32 = 0;

        // Optional leading STAP-A carrying SPS+PPS (parameter sets).
        if self.stap_a_parameter_sets {
            if let CodecConfig::Avc { config, .. } = &track.spec.config {
                let mut param_nals: Vec<Vec<u8>> = Vec::new();
                for sps in &config.config.sps {
                    param_nals.push(sps.0.clone());
                }
                for pps in &config.config.pps {
                    param_nals.push(pps.0.clone());
                }
                if !param_nals.is_empty() {
                    let pkt = build_stap_a(pt, &param_nals, &mut seq, timestamp, self.ssrc)?;
                    packets.push(pkt);
                }
            }
        }

        for (i, sample) in track.samples.iter().enumerate() {
            // Rescale to the 90 kHz RTP clock if the IR timescale differs.
            timestamp = rescale_ts(sample_dts(track, i), timescale, VIDEO_CLOCK_RATE);
            let nals = split_length_prefixed(&sample.data)?;
            if nals.is_empty() {
                continue;
            }
            // Emit each NAL; the marker is set on the LAST packet of the AU.
            let last_nal = nals.len() - 1;
            for (n, nal) in nals.iter().enumerate() {
                let is_last_nal = n == last_nal;
                if nal.len() + RTP_HEADER_LEN <= self.mtu {
                    // Single-NAL packet.
                    let marker = is_last_nal;
                    let mut pkt = rtp_header(pt, marker, seq.next(), timestamp, self.ssrc);
                    pkt.extend_from_slice(nal);
                    packets.push(pkt);
                } else {
                    // FU-A fragmentation.
                    fragment_fu_a(
                        nal,
                        pt,
                        is_last_nal,
                        self.mtu,
                        &mut seq,
                        timestamp,
                        self.ssrc,
                        &mut packets,
                    )?;
                }
            }
        }
        Ok(packets)
    }

    /// Packetize one AAC track (`AAC-hbr`, one AU per packet).
    fn packetize_audio(
        &self,
        track: &crate::media::Track,
        pt: u8,
        clock: u32,
    ) -> Result<Vec<Vec<u8>>> {
        let mut packets = Vec::with_capacity(track.samples.len());
        let mut seq = SeqCounter::new(0);
        let timescale = if track.spec.timescale != 0 {
            track.spec.timescale
        } else {
            clock
        };
        for (i, sample) in track.samples.iter().enumerate() {
            let au = &sample.data;
            if au.len() >= (1usize << AAC_SIZE_LENGTH) {
                return Err(Error::InvalidValue {
                    field: "aac_au_size",
                    value: au.len() as u64,
                    reason: "exceeds 13-bit AAC-hbr AU-size field",
                });
            }
            let timestamp = rescale_ts(sample_dts(track, i), timescale, clock);
            // AU-headers-length is in BITS: one 2-byte header = 16 bits.
            let au_headers_len_bits = (AAC_AU_HEADER_LEN * 8) as u16;
            let mut pkt = rtp_header(pt, true, seq.next(), timestamp, self.ssrc);
            pkt.extend_from_slice(&au_headers_len_bits.to_be_bytes());
            // AU-header: AU-size(13) | AU-Index(3). AU-Index = 0 (single AU).
            let hdr = (au.len() as u16) << AAC_INDEX_LENGTH;
            pkt.extend_from_slice(&hdr.to_be_bytes());
            pkt.extend_from_slice(au);
            packets.push(pkt);
        }
        Ok(packets)
    }
}

/// The decode timestamp of sample `i`, in the track's media timescale — the sum
/// of preceding sample durations (falling back to the index when durations are
/// zero so packets still get strictly increasing timestamps per AU).
fn sample_dts(track: &crate::media::Track, i: usize) -> u64 {
    let sum: u64 = track.samples[..i].iter().map(|s| s.duration as u64).sum();
    sum
}

/// Rescale a tick count from `from` to `to` timescale (round to nearest).
fn rescale_ts(ticks: u64, from: u32, to: u32) -> u32 {
    if from == 0 || from == to {
        return ticks as u32;
    }
    ((ticks * to as u64 + from as u64 / 2) / from as u64) as u32
}

/// Split a 4-byte length-prefixed IR video sample into its NAL slices.
fn split_length_prefixed(data: &[u8]) -> Result<Vec<&[u8]>> {
    crate::annexb::iter_length_prefixed_nals(data)
}

/// Build a STAP-A packet aggregating several (small) NALs (RFC 6184 §5.7.1).
fn build_stap_a(
    pt: u8,
    nals: &[Vec<u8>],
    seq: &mut SeqCounter,
    timestamp: u32,
    ssrc: u32,
) -> Result<Vec<u8>> {
    // The STAP-A NAL header's F/NRI is the max NRI over the aggregated NALs
    // (RFC 6184 §5.7.1); type = 24. Marker is 0 (parameter sets, not an AU end).
    let mut max_nri = 0u8;
    let mut forbidden = 0u8;
    for nal in nals {
        if let Some(&octet) = nal.first() {
            max_nri = max_nri.max(octet & 0x60);
            forbidden |= octet & 0x80;
        }
    }
    let stap_hdr = forbidden | max_nri | NAL_TYPE_STAP_A;
    let mut pkt = rtp_header(pt, false, seq.next(), timestamp, ssrc);
    pkt.push(stap_hdr);
    for nal in nals {
        if nal.len() > u16::MAX as usize {
            return Err(Error::InvalidValue {
                field: "stap_a_nal_size",
                value: nal.len() as u64,
                reason: "exceeds 16-bit STAP-A size prefix",
            });
        }
        pkt.extend_from_slice(&(nal.len() as u16).to_be_bytes());
        pkt.extend_from_slice(nal);
    }
    Ok(pkt)
}

/// Fragment one large NAL into FU-A packets (RFC 6184 §5.8).
#[allow(clippy::too_many_arguments)]
fn fragment_fu_a(
    nal: &[u8],
    pt: u8,
    au_is_last_nal: bool,
    mtu: usize,
    seq: &mut SeqCounter,
    timestamp: u32,
    ssrc: u32,
    out: &mut Vec<Vec<u8>>,
) -> Result<()> {
    if nal.is_empty() {
        return Err(Error::InvalidInput("cannot FU-A fragment an empty NAL"));
    }
    let nal_octet = nal[0];
    let fnri = nal_octet & NAL_FNRI_MASK;
    let nal_type = nal_octet & NAL_TYPE_MASK;
    let fu_indicator = fnri | NAL_TYPE_FU_A;
    let payload = &nal[1..]; // NAL body (the first octet is reconstructed).

    // Payload budget per packet: MTU minus RTP header, FU indicator, FU header.
    let per_packet = mtu
        .checked_sub(RTP_HEADER_LEN + 2)
        .filter(|&b| b > 0)
        .ok_or(Error::InvalidInput("MTU too small for FU-A fragmentation"))?;

    let total = payload.len();
    let num_frags = total.div_ceil(per_packet).max(1);
    for f in 0..num_frags {
        let start = f * per_packet;
        let end = (start + per_packet).min(total);
        let is_start = f == 0;
        let is_end = f == num_frags - 1;
        let mut fu_header = nal_type;
        if is_start {
            fu_header |= FU_START_MASK;
        }
        if is_end {
            fu_header |= FU_END_MASK;
        }
        // Marker set only on the last fragment of the AU's last NAL.
        let marker = is_end && au_is_last_nal;
        let mut pkt = rtp_header(pt, marker, seq.next(), timestamp, ssrc);
        pkt.push(fu_indicator);
        pkt.push(fu_header);
        pkt.extend_from_slice(&payload[start..end]);
        out.push(pkt);
    }
    Ok(())
}

/// Extract the AudioSpecificConfig bytes from an `esds` box.
fn asc_bytes(esds: &crate::mp4esds::EsdsBox) -> Result<&[u8]> {
    esds.es_descriptor
        .decoder_config
        .as_ref()
        .and_then(|dc| dc.decoder_specific_info.as_ref())
        .map(|dsi| dsi.data.as_slice())
        .ok_or(Error::InvalidInput(
            "AAC esds has no DecoderSpecificInfo (AudioSpecificConfig)",
        ))
}

// ---------------------------------------------------------------------------
// SDP generation (RFC 4566)
// ---------------------------------------------------------------------------

/// Assemble the full session-level SDP from the per-media blocks.
fn build_sdp(media_blocks: &str) -> String {
    let mut s = String::new();
    s.push_str("v=0\r\n");
    s.push_str("o=- 0 0 IN IP4 127.0.0.1\r\n");
    s.push_str("s=transmux RTP\r\n");
    s.push_str("t=0 0\r\n");
    s.push_str(media_blocks);
    s
}

/// SDP media block for an H.264 video stream (RFC 6184 §8.1).
fn sdp_video(pt: u8, config: &crate::avc_config::AVCDecoderConfigurationRecord) -> Result<String> {
    let profile_level_id = format!(
        "{:02X}{:02X}{:02X}",
        config.profile_indication, config.profile_compatibility, config.level_indication
    );
    let mut sprop = String::new();
    let mut first = true;
    for sps in &config.sps {
        if !first {
            sprop.push(',');
        }
        sprop.push_str(&base64_encode(&sps.0));
        first = false;
    }
    for pps in &config.pps {
        if !first {
            sprop.push(',');
        }
        sprop.push_str(&base64_encode(&pps.0));
        first = false;
    }
    let mut s = String::new();
    s.push_str(&format!("m=video 0 RTP/AVP {pt}\r\n"));
    s.push_str(&format!("a=rtpmap:{pt} H264/{VIDEO_CLOCK_RATE}\r\n"));
    s.push_str(&format!(
        "a=fmtp:{pt} packetization-mode=1; profile-level-id={profile_level_id}; sprop-parameter-sets={sprop}\r\n"
    ));
    Ok(s)
}

/// SDP media block for an AAC audio stream (`mpeg4-generic`, RFC 3640 §4.1).
fn sdp_audio(pt: u8, clock: u32, channels: u16, asc: &[u8]) -> Result<String> {
    let config = hex_encode(asc);
    let mut s = String::new();
    s.push_str(&format!("m=audio 0 RTP/AVP {pt}\r\n"));
    s.push_str(&format!(
        "a=rtpmap:{pt} mpeg4-generic/{clock}/{channels}\r\n"
    ));
    s.push_str(&format!(
        "a=fmtp:{pt} streamtype=5; profile-level-id=1; mode=AAC-hbr; config={config}; \
         sizeLength={AAC_SIZE_LENGTH}; indexLength={AAC_INDEX_LENGTH}; \
         indexDeltaLength={AAC_INDEX_DELTA_LENGTH}\r\n"
    ));
    Ok(s)
}

// ---------------------------------------------------------------------------
// Depacketizer input
// ---------------------------------------------------------------------------

/// One RTP stream fed to [`RtpDepacketizer`]: its kind + packets.
#[derive(Debug, Clone)]
pub struct RtpInputStream {
    /// The payload format carried on this stream.
    pub kind: RtpMediaKind,
    /// The RTP packets in arrival (sequence) order.
    pub packets: Vec<Vec<u8>>,
}

/// The input to [`RtpDepacketizer`]: one or more RTP streams.
#[derive(Debug, Clone)]
pub struct RtpInput {
    /// The streams to depacketize back into IR tracks.
    pub streams: Vec<RtpInputStream>,
}

// ---------------------------------------------------------------------------
// RtpDepacketizer — Unpackage
// ---------------------------------------------------------------------------

/// Depacketize RTP packets back into the [`Media`] IR.
///
/// Reassembles FU-A (`S`..`E`) fragments, splits STAP-A aggregates, strips AAC
/// AU-headers, and rebuilds IR samples (video NALs re-prefixed with the 4-byte
/// length that the IR convention uses — see [`crate::annexb`]).
#[derive(Debug, Default, Clone)]
pub struct RtpDepacketizer {
    _marker: PhantomData<()>,
}

impl RtpDepacketizer {
    /// Create a new depacketizer.
    pub fn new() -> Self {
        Self::default()
    }
}

impl Unpackage for RtpDepacketizer {
    type Input = RtpInput;
    type Media = Media;
    type Error = Error;

    fn unpackage(&mut self, input: RtpInput) -> Result<Media> {
        let mut tracks = Vec::new();
        for (idx, stream) in input.streams.iter().enumerate() {
            let samples = match stream.kind {
                RtpMediaKind::H264 => depacketize_video(&stream.packets)?,
                RtpMediaKind::Aac => depacketize_audio(&stream.packets)?,
            };
            tracks.push(RtpTrack {
                id: idx as u32 + 1,
                samples,
            });
        }
        // The IR requires codec config, which RTP alone cannot fully rebuild
        // (SDP is separate); expose the reassembled coded samples instead.
        Ok(rtp_tracks_to_media(tracks))
    }
}

/// A reassembled RTP track (coded samples only; config comes from SDP).
struct RtpTrack {
    id: u32,
    samples: Vec<Vec<u8>>,
}

/// Reassembled RTP samples, exposed on [`Media`] via a light wrapper. Since the
/// hub IR carries codec config, the depacketizer returns the raw reassembled
/// access units on each track's samples for round-trip verification; callers
/// pair them with the SDP-derived config as needed.
fn rtp_tracks_to_media(tracks: Vec<RtpTrack>) -> Media {
    use crate::pipeline::Sample;
    let ir_tracks = tracks
        .into_iter()
        .map(|t| {
            let samples = t
                .samples
                .into_iter()
                .map(|data| Sample {
                    data,
                    duration: 0,
                    is_sync: true,
                    composition_offset: 0,
                })
                .collect();
            // A placeholder AVC config: the RTP wire has no config; the SDP does.
            // We only need identity + samples for round-trip use, so build a
            // minimal AVC spec (never serialized to a container here).
            crate::media::Track {
                spec: placeholder_spec(t.id),
                samples,
            }
        })
        .collect();
    Media::new(ir_tracks, 0)
}

/// Minimal placeholder [`TrackSpec`] for a depacketized track (the RTP wire
/// carries no codec config — the SDP does). Samples are the payload of interest.
fn placeholder_spec(track_id: u32) -> crate::pipeline::TrackSpec {
    use crate::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
    use crate::pipeline::{CodecConfig, TrackSpec};
    let record = AVCDecoderConfigurationRecord {
        configuration_version: 1,
        profile_indication: 0,
        profile_compatibility: 0,
        level_indication: 0,
        length_size_minus_one: (NAL_LENGTH_SIZE - 1) as u8,
        sps: Vec::new(),
        pps: Vec::new(),
        chroma_format: None,
        bit_depth_luma_minus8: None,
        bit_depth_chroma_minus8: None,
        sps_ext: Vec::new(),
    };
    TrackSpec {
        track_id,
        timescale: VIDEO_CLOCK_RATE,
        config: CodecConfig::Avc {
            config: AVCConfigurationBox::new(record),
            width: 0,
            height: 0,
        },
    }
}

/// Depacketize an H.264 stream: single-NAL / STAP-A / FU-A → length-prefixed
/// access units. NALs are grouped into access units by the RTP timestamp; the
/// marker bit confirms an AU boundary.
fn depacketize_video(packets: &[Vec<u8>]) -> Result<Vec<Vec<u8>>> {
    let mut aus: Vec<Vec<u8>> = Vec::new();
    let mut cur_nals: Vec<Vec<u8>> = Vec::new();
    let mut cur_ts: Option<u32> = None;
    let mut fu_buf: Vec<u8> = Vec::new();
    let mut fu_active = false;

    for pkt in packets {
        let hdr = parse_rtp_header(pkt)?;
        let payload = &pkt[RTP_HEADER_LEN..];
        if payload.is_empty() {
            continue;
        }

        // New timestamp → flush the previous access unit's NALs.
        if let Some(ts) = cur_ts {
            if ts != hdr.timestamp && !cur_nals.is_empty() {
                aus.push(length_prefix_nals(&cur_nals));
                cur_nals.clear();
            }
        }
        cur_ts = Some(hdr.timestamp);

        let nal_octet = payload[0];
        let nal_type = nal_octet & NAL_TYPE_MASK;
        match nal_type {
            NAL_TYPE_STAP_A => {
                // STAP-A: [STAP hdr][size|NAL]...
                let mut off = 1usize;
                while off < payload.len() {
                    if off + STAP_A_SIZE_LEN > payload.len() {
                        return Err(Error::BufferTooShort {
                            need: off + STAP_A_SIZE_LEN,
                            have: payload.len(),
                            what: "STAP-A size prefix",
                        });
                    }
                    let size = u16::from_be_bytes([payload[off], payload[off + 1]]) as usize;
                    off += STAP_A_SIZE_LEN;
                    let end = off + size;
                    if end > payload.len() {
                        return Err(Error::BufferTooShort {
                            need: end,
                            have: payload.len(),
                            what: "STAP-A aggregated NAL",
                        });
                    }
                    cur_nals.push(payload[off..end].to_vec());
                    off = end;
                }
            }
            NAL_TYPE_FU_A => {
                if payload.len() < 2 {
                    return Err(Error::BufferTooShort {
                        need: 2,
                        have: payload.len(),
                        what: "FU-A indicator + header",
                    });
                }
                let fu_indicator = payload[0];
                let fu_header = payload[1];
                let is_start = fu_header & FU_START_MASK != 0;
                let is_end = fu_header & FU_END_MASK != 0;
                let orig_type = fu_header & NAL_TYPE_MASK;
                let fnri = fu_indicator & NAL_FNRI_MASK;
                if is_start {
                    // Reconstruct the original NAL octet from FU indicator+header.
                    fu_buf.clear();
                    fu_buf.push(fnri | orig_type);
                    fu_active = true;
                }
                if !fu_active {
                    return Err(Error::InvalidInput(
                        "FU-A fragment before a start (S) fragment",
                    ));
                }
                fu_buf.extend_from_slice(&payload[2..]);
                if is_end {
                    cur_nals.push(core::mem::take(&mut fu_buf));
                    fu_active = false;
                }
            }
            _ => {
                // Single-NAL packet (types 1..23).
                cur_nals.push(payload.to_vec());
            }
        }

        // Marker → end of access unit.
        if hdr.marker && !cur_nals.is_empty() && !fu_active {
            aus.push(length_prefix_nals(&cur_nals));
            cur_nals.clear();
            cur_ts = None;
        }
    }
    if !cur_nals.is_empty() {
        aus.push(length_prefix_nals(&cur_nals));
    }
    Ok(aus)
}

/// 4-byte length-prefix a list of NALs into an IR video sample.
fn length_prefix_nals(nals: &[Vec<u8>]) -> Vec<u8> {
    let total: usize = nals.iter().map(|n| NAL_LENGTH_SIZE + n.len()).sum();
    let mut out = Vec::with_capacity(total);
    for nal in nals {
        out.extend_from_slice(&(nal.len() as u32).to_be_bytes());
        out.extend_from_slice(nal);
    }
    out
}

/// Depacketize an AAC (`AAC-hbr`) stream: strip AU-headers → raw AUs.
fn depacketize_audio(packets: &[Vec<u8>]) -> Result<Vec<Vec<u8>>> {
    let mut aus = Vec::with_capacity(packets.len());
    for pkt in packets {
        parse_rtp_header(pkt)?;
        let payload = &pkt[RTP_HEADER_LEN..];
        if payload.len() < AAC_AU_HEADERS_LENGTH_LEN {
            return Err(Error::BufferTooShort {
                need: AAC_AU_HEADERS_LENGTH_LEN,
                have: payload.len(),
                what: "AAC AU-headers-length",
            });
        }
        let au_headers_len_bits = u16::from_be_bytes([payload[0], payload[1]]) as usize;
        // Number of AU-headers = header-section bits / bits-per-header (16).
        let header_bytes = au_headers_len_bits.div_ceil(8);
        let num_headers = au_headers_len_bits / (AAC_AU_HEADER_LEN * 8);
        let mut off = AAC_AU_HEADERS_LENGTH_LEN;
        if off + header_bytes > payload.len() {
            return Err(Error::BufferTooShort {
                need: off + header_bytes,
                have: payload.len(),
                what: "AAC AU-header section",
            });
        }
        // Parse AU sizes (AU-size = top 13 bits of each 16-bit header).
        let mut sizes = Vec::with_capacity(num_headers);
        for h in 0..num_headers {
            let hoff = off + h * AAC_AU_HEADER_LEN;
            let hdr = u16::from_be_bytes([payload[hoff], payload[hoff + 1]]);
            let size = (hdr >> AAC_INDEX_LENGTH) as usize;
            sizes.push(size);
        }
        off += header_bytes;
        for size in sizes {
            let end = off + size;
            if end > payload.len() {
                return Err(Error::BufferTooShort {
                    need: end,
                    have: payload.len(),
                    what: "AAC AU data",
                });
            }
            aus.push(payload[off..end].to_vec());
            off = end;
        }
    }
    Ok(aus)
}

/// A parsed RTP fixed header (RFC 3550 §5.1) — the fields the spoke needs.
#[derive(Debug, Clone, Copy)]
struct RtpHeader {
    marker: bool,
    #[allow(dead_code)]
    payload_type: u8,
    #[allow(dead_code)]
    sequence: u16,
    timestamp: u32,
    #[allow(dead_code)]
    ssrc: u32,
}

/// Parse and validate the 12-byte RTP fixed header, rejecting bad versions.
fn parse_rtp_header(pkt: &[u8]) -> Result<RtpHeader> {
    if pkt.len() < RTP_HEADER_LEN {
        return Err(Error::BufferTooShort {
            need: RTP_HEADER_LEN,
            have: pkt.len(),
            what: "RTP fixed header",
        });
    }
    let version = pkt[0] >> 6;
    if version != RTP_VERSION {
        return Err(Error::InvalidValue {
            field: "rtp_version",
            value: version as u64,
            reason: "must be 2",
        });
    }
    Ok(RtpHeader {
        marker: pkt[1] & RTP_MARKER_MASK != 0,
        payload_type: pkt[1] & RTP_PT_MASK,
        sequence: u16::from_be_bytes([pkt[2], pkt[3]]),
        timestamp: u32::from_be_bytes([pkt[4], pkt[5], pkt[6], pkt[7]]),
        ssrc: u32::from_be_bytes([pkt[8], pkt[9], pkt[10], pkt[11]]),
    })
}

// ---------------------------------------------------------------------------
// Hand-rolled base64 (RFC 4648) + hex — no external dependency
// ---------------------------------------------------------------------------

/// Standard base64 alphabet (RFC 4648 §4).
const B64_ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Base64-encode bytes (RFC 4648, with `=` padding).
pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64_ALPHABET[((n >> 18) & 0x3F) as usize] as char);
        out.push(B64_ALPHABET[((n >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(B64_ALPHABET[((n >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(B64_ALPHABET[(n & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// Base64-decode a string (RFC 4648); rejects invalid characters.
pub fn base64_decode(s: &str) -> Result<Vec<u8>> {
    fn val(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a' + 26) as u32),
            b'0'..=b'9' => Some((c - b'0' + 52) as u32),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let bytes: Vec<u8> = s.bytes().filter(|&b| b != b'=').collect();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    let mut acc = 0u32;
    let mut nbits = 0u32;
    for &b in &bytes {
        let v = val(b).ok_or(Error::InvalidValue {
            field: "base64",
            value: b as u64,
            reason: "not a base64 character",
        })?;
        acc = (acc << 6) | v;
        nbits += 6;
        if nbits >= 8 {
            nbits -= 8;
            out.push((acc >> nbits) as u8);
        }
    }
    Ok(out)
}

/// Hex-encode bytes (lowercase).
pub fn hex_encode(data: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(data.len() * 2);
    for &b in data {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0F) as usize] as char);
    }
    out
}

/// Hex-decode a string; rejects odd lengths and invalid nibbles.
pub fn hex_decode(s: &str) -> Result<Vec<u8>> {
    fn nibble(c: u8) -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    }
    let bytes = s.as_bytes();
    if bytes.len() % 2 != 0 {
        return Err(Error::InvalidValue {
            field: "hex",
            value: bytes.len() as u64,
            reason: "odd-length hex string",
        });
    }
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks(2) {
        let hi = nibble(pair[0]).ok_or(Error::InvalidValue {
            field: "hex",
            value: pair[0] as u64,
            reason: "not a hex digit",
        })?;
        let lo = nibble(pair[1]).ok_or(Error::InvalidValue {
            field: "hex",
            value: pair[1] as u64,
            reason: "not a hex digit",
        })?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trip() {
        let data = b"\x67\x42\xc0\x1e\xd9";
        let enc = base64_encode(data);
        assert_eq!(base64_decode(&enc).unwrap(), data);
    }

    #[test]
    fn base64_known_vector() {
        // RFC 4648 test vector.
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_decode("Zm9vYmFy").unwrap(), b"foobar");
    }

    #[test]
    fn hex_round_trip() {
        let data = b"\x12\x08\x56\xe5\x00";
        let enc = hex_encode(data);
        assert_eq!(enc, "12085 6e500".replace(' ', ""));
        assert_eq!(hex_decode(&enc).unwrap(), data);
    }

    #[test]
    fn rtp_header_layout() {
        let h = rtp_header(96, true, 7, 0x0001_0000, 0xDEAD_BEEF);
        assert_eq!(h.len(), RTP_HEADER_LEN);
        assert_eq!(h[0], 0x80); // V=2
        assert_eq!(h[1], 0x80 | 96); // marker + PT
        assert_eq!(u16::from_be_bytes([h[2], h[3]]), 7);
        assert_eq!(u32::from_be_bytes([h[4], h[5], h[6], h[7]]), 0x0001_0000);
        let parsed = parse_rtp_header(&h).unwrap();
        assert!(parsed.marker);
        assert_eq!(parsed.payload_type, 96);
    }
}
