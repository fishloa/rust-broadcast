//! RTP de/packetisation + SDP — RFC 3550 / RFC 6184 / RFC 3640 / RFC 4566.
//!
//! The RTP spoke of the any-to-any container hub: it packetises the [`Media`]
//! IR into RTP packets ([`RtpPacketiser`] : [`Package`]) and depacketises RTP
//! packets back to the IR ([`RtpDepacketiser`] : [`Unpackage`]), for H.264/AVC
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
//!   on packetise and re-added on depacketise.
//! - **AAC** (RFC 3640, `AAC-hbr`): an AU-headers-length (16-bit, in bits)
//!   prefix + one 2-byte AU-header (`sizeLength=13; indexLength=3`) + the raw
//!   access unit.
//! - **SDP** (RFC 4566 + `fmtp`): `sprop-parameter-sets` carries base64 SPS,PPS
//!   for video; `config` carries the hex AudioSpecificConfig for audio.
//! - **KLV** (RFC 6597, `smpte336m`): a SMPTE ST 336 KLV unit ([`crate::klv`])
//!   carried directly after the fixed header — no payload header — fragmented
//!   across sequential packets sharing one timestamp, marker on the last
//!   ([`packetise_klv`] / [`depacketise_klv`]).
//!
//! See `transmux/docs/rtp/rtp-payload-formats.md` for the full transcription.
//!
//! This module is stateless: packetise takes the IR and returns
//! [`RtpPacket`]s whose payload is a zero-copy [`bytes::Bytes`] slice of
//! the sample data (single-NAL and FU-A fragmentation use
//! `Bytes::slice`; STAP-A aggregation and the audio AU-header path
//! interleave headers with payload so they build in a `BytesMut`, which
//! copies). The depacketise side (single-shot [`RtpDepacketiser`] and
//! streaming [`crate::rtp_stream::RtpStreamDepacketiser`]) reassembles
//! fragments by concatenation and may reasonably copy (issue #777).
//!
//! This module does **not** validate sequence-number continuity —
//! [`crate::rtp_stream`]'s stateful
//! [`crate::rtp_stream::RtpStreamDepacketiser`] does that (loss/
//! reorder detection, issue #779); see its module docs and
//! `transmux/docs/rtp/rtp-sequence-validation.md`.
//!
//! `no_std` + `alloc`.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::marker::PhantomData;

use broadcast_common::{Package, Parse, Serialize, Unpackage};
use bytes::Bytes;
use rtp_packet::RtpPacket as RtpPacketWire;

use crate::annexb::NAL_LENGTH_SIZE;
use crate::error::{Error, Result};
use crate::media::Media;
use crate::pipeline::CodecConfig;

// ---------------------------------------------------------------------------
// Named constants (no magic numbers — RFC 3550 §5.1 / RFC 6184 / RFC 3640)
// ---------------------------------------------------------------------------

/// RTP fixed-header length in bytes (no CSRC, no extension) — re-exported
/// from `rtp_packet` (RFC 3550 §5.1) so every existing `RTP_HEADER_LEN` use
/// site below keeps working unchanged. The fixed-header codec itself now
/// lives in the spec-complete `rtp-packet` crate (padding/CSRC/header
/// extension); transmux only ever emits/expects the simple `P=0 X=0 CC=0`
/// case, so this migration is internal-only (issue #646).
const RTP_HEADER_LEN: usize = rtp_packet::FIXED_HEADER_LEN;
/// Payload-type mask applied before handing a payload type to `rtp_packet`
/// (RFC 3550 §5.1, low 7 bits) — matches the masking this crate has always
/// applied here; transmux's dynamic payload types never legitimately exceed
/// 127, so this is defensive parity with the prior implementation.
const RTP_PT_MASK: u8 = 0x7F;

/// Default dynamic payload type for the H.264 video stream.
pub const DEFAULT_VIDEO_PT: u8 = 96;
/// Default dynamic payload type for the AAC audio stream.
pub const DEFAULT_AUDIO_PT: u8 = 97;
/// Default network MTU (payload budget) forcing FU-A on larger NALs.
pub const DEFAULT_MTU: usize = 1400;
/// Default video RTP clock rate (RFC 6184 — H.264 is carried at 90 kHz).
pub const VIDEO_CLOCK_RATE: u32 = 90_000;

/// Default dynamic payload type for a KLV metadata stream (RFC 6597).
pub const DEFAULT_KLV_PT: u8 = 98;
/// RFC 6597 SDP encoding name for SMPTE ST 336 KLV.
pub const KLV_ENCODING_NAME: &str = "smpte336m";

// --- H.264 NAL / packetisation (RFC 6184 §5.2, §5.6, §5.7, §5.8) -----------

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

/// One emitted RTP packet: a small, owned fixed header + a payload whose
/// [`Bytes`] is a zero-copy slice of the sample data on the single-NAL and
/// FU-A paths (the common cases). STAP-A aggregation and AAC AU-header
/// audio packets interleave header bytes with payload and are built in a
/// `BytesMut` (which copies); those paths are documented at each call site.
///
/// Callers that need a single contiguous `&[u8]` (e.g. the depacketise
/// path) can call [`RtpPacket::as_contiguous`].
#[derive(Debug, Clone)]
pub struct RtpPacket {
    /// The RTP fixed header (12 bytes) plus any payload-format headers
    /// (e.g. FU indicator + FU header, AAC AU-headers). Owned, small.
    pub header: Bytes,
    /// The payload. For single-NAL and FU-A packets this is a zero-copy
    /// [`Bytes::slice`] of the original sample data; for STAP-A and
    /// AAC-hbr it is an owned buffer.
    pub payload: Bytes,
}

impl RtpPacket {
    /// Return a single contiguous [`Bytes`] for this packet: the fixed
    /// header followed by the payload, concatenated. Allocates exactly
    /// `header.len() + payload.len()` bytes.
    pub fn as_contiguous(&self) -> Bytes {
        use bytes::BytesMut;
        let mut buf = BytesMut::with_capacity(self.header.len() + self.payload.len());
        buf.extend_from_slice(&self.header);
        buf.extend_from_slice(&self.payload);
        buf.freeze()
    }
}

/// One packetised RTP stream: its payload type + kind and the emitted packets.
#[derive(Debug, Clone)]
pub struct RtpStream {
    /// Dynamic payload type (matches the SDP `rtpmap`).
    pub pt: u8,
    /// The payload format carried on this stream.
    pub kind: RtpMediaKind,
    /// The RTP packets, in emission (sequence-number) order.
    pub packets: Vec<RtpPacket>,
}

/// The output of [`RtpPacketiser`]: per-track RTP streams plus an SDP string.
#[derive(Debug, Clone)]
pub struct RtpOutput {
    /// One [`RtpStream`] per packetised track, in track order.
    pub streams: Vec<RtpStream>,
    /// The session-level SDP describing every stream (RFC 4566).
    pub sdp: String,
}

// ---------------------------------------------------------------------------
// RtpPacketiser — Package
// ---------------------------------------------------------------------------

/// Packetise a [`Media`] IR into RTP packets + SDP.
///
/// Per track: AVC → single-NAL / STAP-A (SPS+PPS) / FU-A packets on a 90 kHz
/// clock; AAC → `AAC-hbr` packets on the audio sample-rate clock. All packets of
/// one access unit share a timestamp and the marker bit is set on the last.
#[derive(Debug, Clone)]
pub struct RtpPacketiser {
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

impl Default for RtpPacketiser {
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

impl RtpPacketiser {
    /// Create a packetiser with default MTU / payload types / SSRC.
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

/// Write an RTP fixed header into a new packet buffer and return it as
/// owned [`Bytes`].
///
/// Delegates the wire encoding to [`rtp_packet::RtpPacket`] (RFC 3550 §5.1);
/// transmux only ever emits the simple `P=0 X=0 CC=0` case (no CSRC list, no
/// header extension, no padding) — see issue #646.
fn rtp_header(pt: u8, marker: bool, seq: u16, timestamp: u32, ssrc: u32) -> Bytes {
    let pkt = RtpPacketWire {
        marker,
        payload_type: pt & RTP_PT_MASK,
        sequence_number: seq,
        timestamp,
        ssrc,
        csrc: Vec::new(),
        extension: None,
        padding: None,
        payload: &[],
    };
    let len = pkt.serialized_len();
    let mut buf = bytes::BytesMut::with_capacity(len);
    buf.resize(len, 0);
    pkt.serialize_into(&mut buf)
        .expect("simple V=2 P=0 X=0 CC=0 header always serializes");
    buf.freeze()
}

impl Package for RtpPacketiser {
    type Media = Media;
    type Output = RtpOutput;
    type Error = Error;

    fn package(&mut self, media: &Media) -> Result<RtpOutput> {
        if media.tracks.is_empty() {
            return Err(Error::InvalidInput(
                "cannot packetise a Media with no tracks",
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
                    let packets = self.packetise_video(track, pt)?;
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
                    let packets = self.packetise_audio(track, pt, clock)?;
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
                        "RTP packetiser supports only AVC video and AAC audio tracks",
                    ));
                }
            }
        }
        if streams.is_empty() {
            return Err(Error::InvalidInput(
                "no AVC/AAC tracks to packetise into RTP",
            ));
        }
        let sdp = build_sdp(&sdp_media);
        Ok(RtpOutput { streams, sdp })
    }
}

impl RtpPacketiser {
    /// Packetise one AVC track into RTP packets.
    ///
    /// Public for the zero-copy allocation test (`alloc_measurement.rs`);
    /// the main consumer calls [`Package::package`] instead.
    pub fn packetise_video(&self, track: &crate::media::Track, pt: u8) -> Result<Vec<RtpPacket>> {
        let timescale = if track.spec.timescale != 0 {
            track.spec.timescale
        } else {
            VIDEO_CLOCK_RATE
        };
        let mut packets = Vec::new();
        let mut seq = SeqCounter::new(0);
        let mut timestamp: u32 = 0;

        // Optional leading STAP-A carrying SPS+PPS (parameter sets).
        // STAP-A aggregation interleaves header bytes with payload — built
        // in a BytesMut (which copies); the parameter sets are small
        // (typically <1 kB total).
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
                    // Single-NAL packet — zero-copy payload via Bytes::slice.
                    let marker = is_last_nal;
                    let header = rtp_header(pt, marker, seq.next(), timestamp, self.ssrc);
                    // Locate the NAL slice within the sample's Bytes so we
                    // can share the backing buffer rather than copying.
                    let nal_offset = nal.as_ptr() as usize - sample.data.as_ptr() as usize;
                    let payload = sample.data.slice(nal_offset..nal_offset + nal.len());
                    packets.push(RtpPacket { header, payload });
                } else {
                    // FU-A fragmentation — zero-copy payload slices.
                    fragment_fu_a(
                        nal,
                        &sample.data,
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

    /// Packetise one AAC track (`AAC-hbr`, one AU per packet).
    /// The AAC-hbr payload header (AU-headers-length + AU-header) is
    /// interleaved with the audio access unit, so the full packet is built
    /// in a `BytesMut` (which copies). The header is small (4 bytes) and
    /// audio AUs are typically <1 kB.
    fn packetise_audio(
        &self,
        track: &crate::media::Track,
        pt: u8,
        clock: u32,
    ) -> Result<Vec<RtpPacket>> {
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
            // AU-header: AU-size(13) | AU-Index(3). AU-Index = 0 (single AU).
            let hdr = (au.len() as u16) << AAC_INDEX_LENGTH;
            // Build the full AAC-hbr header (RTP fixed header + AU-headers
            // prefix + AU-header) in a BytesMut, then extend with the
            // payload. This copies — the interleaving makes a zero-copy
            // approach impractical without a vectored I/O consumer.
            let rtp_hdr = rtp_header(pt, true, seq.next(), timestamp, self.ssrc);
            let mut buf = bytes::BytesMut::with_capacity(
                rtp_hdr.len() + AAC_AU_HEADERS_LENGTH_LEN + AAC_AU_HEADER_LEN + au.len(),
            );
            buf.extend_from_slice(&rtp_hdr);
            buf.extend_from_slice(&au_headers_len_bits.to_be_bytes());
            buf.extend_from_slice(&hdr.to_be_bytes());
            buf.extend_from_slice(au);
            let full = buf.freeze();
            // Split into header (RTP + AAC headers) and payload (AU) so the
            // consumer can access them separately, though they share one
            // backing buffer.
            let header_len = rtp_hdr.len() + AAC_AU_HEADERS_LENGTH_LEN + AAC_AU_HEADER_LEN;
            let header = full.slice(0..header_len);
            let payload = full.slice(header_len..);
            packets.push(RtpPacket { header, payload });
        }
        Ok(packets)
    }
}

/// The decode timestamp of sample `i`, in the track's media timescale,
/// **relative to the track's first sample** (so the emitted RTP timestamp
/// series starts at 0 for the first AU regardless of where the source
/// timeline sits — RFC 3550 §5.1 only constrains the increments).
///
/// Media plane step 2c: read from the sample's own **absolute** `dts` when
/// both it and the first sample's are known (the exact per-AU decode time the
/// demuxer recovered, including any composition reordering), falling back to
/// the running sum of preceding durations for a track whose samples carry no
/// timestamps (a section-carried track, which RTP never packetises anyway).
fn sample_dts(track: &crate::media::Track, i: usize) -> u64 {
    if let (Some(first), Some(cur)) = (
        track.samples.first().and_then(|s| s.dts),
        track.samples.get(i).and_then(|s| s.dts),
    ) {
        return (cur - first).max(0) as u64;
    }
    track.samples[..i]
        .iter()
        .map(|s| s.duration.unwrap_or(0) as u64)
        .sum()
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
/// STAP-A aggregation interleaves headers (NRI + type, per-NAL size
/// prefixes) with the parameter-set NAL payloads, so the whole packet is
/// built in a `BytesMut` (which copies). The parameter-set NALs are small
/// (SPS+PPS typically <1 kB), so this is negligible.
fn build_stap_a(
    pt: u8,
    nals: &[Vec<u8>],
    seq: &mut SeqCounter,
    timestamp: u32,
    ssrc: u32,
) -> Result<RtpPacket> {
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
    let total_nal_bytes: usize = nals.iter().map(|n| n.len() + STAP_A_SIZE_LEN).sum();
    let rtp_hdr = rtp_header(pt, false, seq.next(), timestamp, ssrc);
    let total = rtp_hdr.len() + 1 + total_nal_bytes;
    let mut buf = bytes::BytesMut::with_capacity(total);
    buf.extend_from_slice(&rtp_hdr);
    buf.extend_from_slice(&[stap_hdr]);
    for nal in nals {
        if nal.len() > u16::MAX as usize {
            return Err(Error::InvalidValue {
                field: "stap_a_nal_size",
                value: nal.len() as u64,
                reason: "exceeds 16-bit STAP-A size prefix",
            });
        }
        buf.extend_from_slice(&(nal.len() as u16).to_be_bytes());
        buf.extend_from_slice(nal);
    }
    // The STAP-A packet is fully built; no clean zero-copy split possible.
    let full = buf.freeze();
    let header = full.slice(0..rtp_hdr.len());
    let payload = full.slice(rtp_hdr.len()..);
    Ok(RtpPacket { header, payload })
}

/// Fragment one large NAL into FU-A packets (RFC 6184 §5.8).
/// Each FU-A fragment's payload is a zero-copy [`Bytes::slice`] of the
/// original sample data (the NAL body bytes after the first octet) —
/// the common case where this crate's move to `Sample.data: Bytes` pays
/// off on the RTP egress path.
#[allow(clippy::too_many_arguments)]
fn fragment_fu_a(
    nal: &[u8],
    sample_data: &Bytes,
    pt: u8,
    au_is_last_nal: bool,
    mtu: usize,
    seq: &mut SeqCounter,
    timestamp: u32,
    ssrc: u32,
    out: &mut Vec<RtpPacket>,
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

    // Compute the offset of `payload` within the sample's backing buffer
    // so we can slice the sample's Bytes zero-copy.
    let base_offset = nal.as_ptr() as usize - sample_data.as_ptr() as usize + 1;

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
        // Build the RTP + FU header (14 bytes, small and owned).
        let rtp_hdr = rtp_header(pt, marker, seq.next(), timestamp, ssrc);
        let mut header_buf = bytes::BytesMut::with_capacity(rtp_hdr.len() + 2);
        header_buf.extend_from_slice(&rtp_hdr);
        header_buf.extend_from_slice(&[fu_indicator, fu_header]);
        let header = header_buf.freeze();
        // Payload: zero-copy slice into the sample's backing buffer.
        let slice_start = base_offset + start;
        let slice_end = base_offset + end;
        let payload_slice = sample_data.slice(slice_start..slice_end);
        out.push(RtpPacket {
            header,
            payload: payload_slice,
        });
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
// Depacketiser input
// ---------------------------------------------------------------------------

/// One RTP stream fed to [`RtpDepacketiser`]: its kind + packets.
#[derive(Debug, Clone)]
pub struct RtpInputStream {
    /// The payload format carried on this stream.
    pub kind: RtpMediaKind,
    /// The RTP packets in arrival (sequence) order.
    pub packets: Vec<Vec<u8>>,
}

/// The input to [`RtpDepacketiser`]: one or more RTP streams.
#[derive(Debug, Clone)]
pub struct RtpInput {
    /// The streams to depacketise back into IR tracks.
    pub streams: Vec<RtpInputStream>,
}

// ---------------------------------------------------------------------------
// RtpDepacketiser — Unpackage
// ---------------------------------------------------------------------------

/// Depacketise RTP packets back into the [`Media`] IR.
///
/// Reassembles FU-A (`S`..`E`) fragments, splits STAP-A aggregates, strips AAC
/// AU-headers, and rebuilds IR samples (video NALs re-prefixed with the 4-byte
/// length that the IR convention uses — see [`crate::annexb`]).
#[derive(Debug, Default, Clone)]
pub struct RtpDepacketiser {
    _marker: PhantomData<()>,
}

impl RtpDepacketiser {
    /// Create a new depacketiser.
    pub fn new() -> Self {
        Self::default()
    }
}

impl Unpackage for RtpDepacketiser {
    type Input = RtpInput;
    type Media = Media;
    type Error = Error;

    fn unpackage(&mut self, input: RtpInput) -> Result<Media> {
        let mut tracks = Vec::new();
        for (idx, stream) in input.streams.iter().enumerate() {
            let samples = match stream.kind {
                RtpMediaKind::H264 => depacketise_video(&stream.packets)?,
                RtpMediaKind::Aac => depacketise_audio(&stream.packets)?,
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
    samples: Vec<ReassembledAu>,
}

/// The RTP timestamp modulus: the header field is 32 bits (RFC 3550 §5.1), so
/// the media clock wraps every `2^32` ticks (at the 90 kHz video clock, ≈ 13.3
/// hours).
const RTP_TS_WRAP: i64 = 1 << 32;
/// Half the 32-bit range — the classic wrap-detection threshold: a step of
/// more than half the range is read as a wrap, not a real jump.
const RTP_TS_WRAP_HALF: i64 = RTP_TS_WRAP / 2;

/// Incremental 32-bit RTP-timestamp wrap-unroll (RFC 3550 §5.1), one access
/// unit at a time — the RTP analogue of `ts_demux`'s 33-bit `WrapState`, and
/// the **only** place this crate unwraps an RTP clock (media plane step 2c:
/// unwrapped once, at the demux edge, never re-derived downstream).
///
/// The delta is computed on the wrapped clock and applied to an unwrapped
/// accumulator, so ordinary small backward steps (B-frame reordering) survive
/// and only a near-full-range jump is treated as a wrap.
#[derive(Default)]
struct RtpWrapState {
    initialized: bool,
    prev_raw: u32,
    prev_uw: i64,
}

impl RtpWrapState {
    /// Feed the next AU's raw 32-bit RTP timestamp, returning the unwrapped
    /// absolute value.
    fn push(&mut self, raw: u32) -> i64 {
        if !self.initialized {
            self.initialized = true;
            self.prev_raw = raw;
            self.prev_uw = raw as i64;
            return self.prev_uw;
        }
        let mut delta = raw as i64 - self.prev_raw as i64;
        if delta > RTP_TS_WRAP_HALF {
            delta -= RTP_TS_WRAP; // wrapped backward across 2^32
        } else if delta < -RTP_TS_WRAP_HALF {
            delta += RTP_TS_WRAP; // wrapped forward across 2^32
        }
        let uw = self.prev_uw + delta;
        self.prev_raw = raw;
        self.prev_uw = uw;
        uw
    }
}

/// Reassembled RTP samples, exposed on [`Media`] via a light wrapper. Since the
/// hub IR carries codec config, the depacketiser returns the raw reassembled
/// access units on each track's samples for round-trip verification; callers
/// pair them with the SDP-derived config as needed.
fn rtp_tracks_to_media(tracks: Vec<RtpTrack>) -> Media {
    use crate::pipeline::Sample;
    let ir_tracks = tracks
        .into_iter()
        .map(|t| {
            // Absolute dts/pts (media plane step 2c) from the RTP media clock,
            // with the 32-bit wrap unrolled ONCE here at the demux edge
            // ([`RtpWrapState`], RFC 3550 §5.1).
            //
            // Honest scope: RTP's timestamp origin is a *random* offset (§5.1),
            // so without an RTCP SR (`rtcp::SenderReport`) NTP↔RTP mapping this
            // is an absolute **media-clock** timeline with an arbitrary epoch,
            // not a wall-clock one. That is exactly what the IR's `dts`/`pts`
            // mean (ticks in the track timescale), and it preserves real
            // inter-sample timing — a consumer needing a zero origin applies
            // `crate::rebase::rebase_to_zero`. Discarding it (the pre-2c
            // behaviour) lost the timeline outright, so `None` would be the
            // strictly worse and less honest choice here.
            let mut wrap = RtpWrapState::default();
            let stamped: Vec<(i64, bool, Vec<u8>)> = t
                .samples
                .into_iter()
                .map(|au| (wrap.push(au.timestamp), au.is_sync, au.data))
                .collect();
            // Duration = delta to the next AU's decode time; the final AU
            // reuses the previous delta (the same one-behind rule
            // `ts_demux`/`flv` use), and a single-AU track has no measurable
            // duration at all.
            let n = stamped.len();
            let samples: Vec<Sample> = stamped
                .iter()
                .enumerate()
                .map(|(i, &(dts, is_sync, ref data))| {
                    let duration = if i + 1 < n {
                        Some((stamped[i + 1].0 - dts).max(0) as u32)
                    } else if n >= 2 {
                        Some((dts - stamped[i - 1].0).max(0) as u32)
                    } else {
                        None
                    };
                    Sample {
                        data: data.clone().into(),
                        dts: Some(dts),
                        pts: Some(dts),
                        duration,
                        flags: crate::ir::SampleFlags::new(is_sync),
                        provenance: None,
                    }
                })
                .collect();
            // A placeholder AVC config: the RTP wire has no config; the SDP does.
            // We only need identity + samples for round-trip use, so build a
            // minimal AVC spec (never serialized to a container here).
            let anchor = samples
                .first()
                .and_then(|s| s.dts)
                .map(|d| d.max(0) as u64)
                .unwrap_or(0);
            crate::media::Track::new_at(placeholder_spec(t.id), samples, anchor)
        })
        .collect();
    Media::new(ir_tracks, 0)
}

/// Minimal placeholder [`TrackSpec`] for a depacketised track (the RTP wire
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
    TrackSpec::new(
        track_id,
        VIDEO_CLOCK_RATE,
        CodecConfig::Avc {
            config: AVCConfigurationBox::new(record),
            width: 0,
            height: 0,
        },
    )
}

/// A reassembled access unit with its RTP presentation timestamp and a
/// random-access (sync) flag. RFC 6184 §5.7 (video) / RFC 3640 §3.2 (audio).
pub(crate) struct ReassembledAu {
    /// The AU's raw 32-bit RTP timestamp (RFC 3550 §5.1). Read by the
    /// streaming depayloader (rtp_stream, #700 Task 4) and, since media plane
    /// step 2c, by the batch path too — [`rtp_tracks_to_media`] unwraps it
    /// into the sample's absolute `dts`/`pts`.
    pub timestamp: u32,
    /// Whether this AU is a random-access point (an IDR for video).
    pub is_sync: bool,
    pub data: Vec<u8>,
}

/// H.264 FU-A/STAP-A/single-NAL reassembly (RFC 6184 §5.7/§5.8), preserving
/// the RTP timestamp and marking IDR access units as sync points.
pub(crate) fn reassemble_video(packets: &[Vec<u8>]) -> Result<Vec<ReassembledAu>> {
    let mut aus: Vec<ReassembledAu> = Vec::new();
    let mut cur_nals: Vec<Vec<u8>> = Vec::new();
    let mut cur_ts: Option<u32> = None;
    let mut fu_buf: Vec<u8> = Vec::new();
    let mut fu_active = false;

    fn flush_au(aus: &mut Vec<ReassembledAu>, nals: &mut Vec<Vec<u8>>, ts: u32) {
        if nals.is_empty() {
            return;
        }
        let is_sync = nals
            .iter()
            .any(|n| !n.is_empty() && (n[0] & NAL_TYPE_MASK) == NAL_TYPE_IDR);
        aus.push(ReassembledAu {
            timestamp: ts,
            is_sync,
            data: length_prefix_nals(nals),
        });
        nals.clear();
    }

    for pkt in packets {
        let hdr = parse_rtp_header(pkt)?;
        let payload = hdr.payload;
        if payload.is_empty() {
            continue;
        }
        if let Some(ts) = cur_ts {
            if ts != hdr.timestamp && !cur_nals.is_empty() {
                flush_au(&mut aus, &mut cur_nals, ts);
            }
        }
        cur_ts = Some(hdr.timestamp);

        let nal_type = payload[0] & NAL_TYPE_MASK;
        match nal_type {
            NAL_TYPE_STAP_A => {
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
                            what: "STAP-A NAL",
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
                        what: "FU-A header",
                    });
                }
                let fu_indicator = payload[0];
                let fu_header = payload[1];
                let is_start = fu_header & FU_START_MASK != 0;
                let is_end = fu_header & FU_END_MASK != 0;
                let orig_type = fu_header & NAL_TYPE_MASK;
                let fnri = fu_indicator & NAL_FNRI_MASK;
                if is_start {
                    fu_buf.clear();
                    fu_buf.push(fnri | orig_type);
                    fu_active = true;
                }
                if !fu_active {
                    return Err(Error::InvalidInput("FU-A fragment before start"));
                }
                fu_buf.extend_from_slice(&payload[2..]);
                if is_end {
                    cur_nals.push(core::mem::take(&mut fu_buf));
                    fu_active = false;
                }
            }
            _ => cur_nals.push(payload.to_vec()),
        }

        if hdr.marker && !cur_nals.is_empty() && !fu_active {
            let ts = hdr.timestamp;
            flush_au(&mut aus, &mut cur_nals, ts);
            cur_ts = None;
        }
    }
    if let Some(ts) = cur_ts {
        flush_au(&mut aus, &mut cur_nals, ts);
    }
    Ok(aus)
}

/// RFC 3640 AAC-hbr AU-header reassembly, preserving the RTP timestamp.
/// Audio AUs are always sync points.
pub(crate) fn reassemble_audio(packets: &[Vec<u8>]) -> Result<Vec<ReassembledAu>> {
    let mut aus = Vec::new();
    for pkt in packets {
        let hdr = parse_rtp_header(pkt)?;
        let payload = hdr.payload;
        if payload.len() < AAC_AU_HEADERS_LENGTH_LEN {
            return Err(Error::BufferTooShort {
                need: AAC_AU_HEADERS_LENGTH_LEN,
                have: payload.len(),
                what: "AAC AU-headers-length",
            });
        }
        let au_headers_len_bits = u16::from_be_bytes([payload[0], payload[1]]) as usize;
        let header_bytes = au_headers_len_bits.div_ceil(8);
        let num_headers = au_headers_len_bits / (AAC_AU_HEADER_LEN * 8);
        let mut off = AAC_AU_HEADERS_LENGTH_LEN;
        if off + header_bytes > payload.len() {
            return Err(Error::BufferTooShort {
                need: off + header_bytes,
                have: payload.len(),
                what: "AAC AU headers",
            });
        }
        let mut sizes = Vec::with_capacity(num_headers);
        for h in 0..num_headers {
            let hoff = off + h * AAC_AU_HEADER_LEN;
            let ah = u16::from_be_bytes([payload[hoff], payload[hoff + 1]]);
            sizes.push((ah >> AAC_INDEX_LENGTH) as usize);
        }
        off += header_bytes;
        for size in sizes {
            let end = off + size;
            if end > payload.len() {
                return Err(Error::BufferTooShort {
                    need: end,
                    have: payload.len(),
                    what: "AAC AU payload",
                });
            }
            aus.push(ReassembledAu {
                timestamp: hdr.timestamp,
                is_sync: true,
                data: payload[off..end].to_vec(),
            });
            off = end;
        }
    }
    Ok(aus)
}

/// Depacketise an H.264 stream: single-NAL / STAP-A / FU-A → length-prefixed
/// access units. NALs are grouped into access units by the RTP timestamp; the
/// marker bit confirms an AU boundary.
///
/// Keeps each AU's RTP timestamp + sync flag (media plane step 2c) so the
/// caller can build absolute `dts`/`pts`; the 32-bit wrap is unrolled once, in
/// [`rtp_tracks_to_media`].
fn depacketise_video(packets: &[Vec<u8>]) -> Result<Vec<ReassembledAu>> {
    reassemble_video(packets)
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

/// Depacketise an AAC (`AAC-hbr`) stream: strip AU-headers → raw AUs,
/// preserving each AU's RTP timestamp (see [`depacketise_video`]).
fn depacketise_audio(packets: &[Vec<u8>]) -> Result<Vec<ReassembledAu>> {
    reassemble_audio(packets)
}

/// A parsed RTP fixed header (RFC 3550 §5.1) — the fields the spoke needs.
/// Delegates the wire decode to [`rtp_packet::RtpPacket`]; transmux only ever
/// depacketises the simple `P=0 X=0 CC=0` case it itself emits, so only
/// `marker`/`sequence`/`timestamp`/`ssrc`/`payload` are read at call sites
/// (`payload_type` is carried through only for the unit test at the bottom of
/// this file) — see #646. `sequence`/`ssrc` were dead until issue #779 gave
/// [`crate::rtp_stream::RtpStreamDepacketiser`] a reason to read them (loss
/// and reorder detection).
#[derive(Debug, Clone, Copy)]
pub(crate) struct RtpHeader<'a> {
    pub(crate) marker: bool,
    #[allow(dead_code)]
    payload_type: u8,
    /// 16-bit sequence number (RFC 3550 §5.1), incrementing by one per
    /// packet and wrapping mod 65536 — read by
    /// [`crate::rtp_stream::RtpStreamDepacketiser`] for loss/reorder
    /// detection (issue #779).
    pub(crate) sequence: u16,
    pub(crate) timestamp: u32,
    /// Synchronization source identifier (RFC 3550 §5.1) — read by
    /// [`crate::rtp_stream::RtpStreamDepacketiser`] to detect a stream
    /// restart (a new SSRC is a new source, not a sequence gap; RFC 3550
    /// §8.2).
    pub(crate) ssrc: u32,
    /// The payload after the fixed header, CSRC list, and header extension
    /// (if a non-conforming sender added either — `rtp_packet` correctly
    /// skips them; the hand-rolled decode this replaces always assumed
    /// neither was present).
    payload: &'a [u8],
}

/// Parse and validate the RTP fixed header, rejecting bad versions.
pub(crate) fn parse_rtp_header(pkt: &[u8]) -> Result<RtpHeader<'_>> {
    let parsed = RtpPacketWire::parse(pkt).map_err(map_rtp_error)?;
    Ok(RtpHeader {
        marker: parsed.marker,
        payload_type: parsed.payload_type,
        sequence: parsed.sequence_number,
        timestamp: parsed.timestamp,
        ssrc: parsed.ssrc,
        payload: parsed.payload,
    })
}

/// Map an [`rtp_packet::Error`] onto this crate's [`Error`].
fn map_rtp_error(e: rtp_packet::Error) -> Error {
    match e {
        rtp_packet::Error::BufferTooShort { need, have, what } => {
            Error::BufferTooShort { need, have, what }
        }
        rtp_packet::Error::InvalidVersion(v) => Error::InvalidValue {
            field: "rtp_version",
            value: u64::from(v),
            reason: "must be 2",
        },
        rtp_packet::Error::InvalidValue {
            field,
            value,
            reason,
        } => Error::InvalidValue {
            field,
            value,
            reason,
        },
        rtp_packet::Error::InvalidPadding { count, reason } => Error::InvalidValue {
            field: "rtp_padding",
            value: u64::from(count),
            reason,
        },
        rtp_packet::Error::ExtensionNotWordAligned { data_len } => Error::InvalidValue {
            field: "rtp_extension_length",
            value: data_len as u64,
            reason: "extension data length is not a multiple of 4 bytes",
        },
        _ => Error::InvalidInput("invalid RTP header"),
    }
}

// ---------------------------------------------------------------------------
// KLV-over-RTP (RFC 6597) — SMPTE ST 336 KLV units
// ---------------------------------------------------------------------------

/// Packetise one KLV unit ([`crate::klv`]) into RTP packets (RFC 6597).
///
/// The KLV unit bytes are placed directly after the 12-byte fixed header (no
/// payload header). A unit larger than the MTU payload budget is fragmented in
/// sequential byte order across packets that **share `timestamp`**; the marker
/// bit is set only on the final (or only) packet, signalling a complete KLV
/// unit. `seq_start` is the sequence number of the first packet.
///
/// Returns at least one packet; `klv_unit` must be non-empty. Each fragment's
/// payload is a zero-copy [`Bytes::slice`] of the input.
pub fn packetise_klv(
    klv_unit: &Bytes,
    pt: u8,
    seq_start: u16,
    timestamp: u32,
    ssrc: u32,
    mtu: usize,
) -> Result<Vec<RtpPacket>> {
    if klv_unit.is_empty() {
        return Err(Error::InvalidInput("cannot packetise an empty KLV unit"));
    }
    // Payload budget per packet: MTU minus the fixed RTP header.
    let per_packet = mtu
        .checked_sub(RTP_HEADER_LEN)
        .filter(|&b| b > 0)
        .ok_or(Error::InvalidInput("MTU too small for KLV-over-RTP"))?;

    let total = klv_unit.len();
    let num_frags = total.div_ceil(per_packet).max(1);
    let mut seq = SeqCounter::new(seq_start);
    let mut packets = Vec::with_capacity(num_frags);
    for f in 0..num_frags {
        let start = f * per_packet;
        let end = (start + per_packet).min(total);
        let is_last = f == num_frags - 1;
        // All fragments of one KLV unit share the timestamp; marker on the last.
        let header = rtp_header(pt, is_last, seq.next(), timestamp, ssrc);
        let payload = klv_unit.slice(start..end);
        packets.push(RtpPacket { header, payload });
    }
    Ok(packets)
}

/// Reassemble KLV units from a stream of RTP packets (RFC 6597).
///
/// Fragments are concatenated in arrival order; a KLV unit is complete at the
/// packet whose marker bit is set (or, defensively, at a timestamp change).
/// Returns one `Vec<u8>` per reassembled KLV unit.
pub fn depacketise_klv(packets: &[Vec<u8>]) -> Result<Vec<Vec<u8>>> {
    let mut units: Vec<Vec<u8>> = Vec::new();
    let mut cur: Vec<u8> = Vec::new();
    let mut cur_ts: Option<u32> = None;

    for pkt in packets {
        let hdr = parse_rtp_header(pkt)?;
        // A timestamp change with buffered bytes ends the previous unit (a
        // dropped final/marker packet still flushes the accumulated fragments).
        if let Some(ts) = cur_ts {
            if ts != hdr.timestamp && !cur.is_empty() {
                units.push(core::mem::take(&mut cur));
            }
        }
        cur_ts = Some(hdr.timestamp);
        cur.extend_from_slice(hdr.payload);
        if hdr.marker {
            units.push(core::mem::take(&mut cur));
            cur_ts = None;
        }
    }
    if !cur.is_empty() {
        units.push(cur);
    }
    Ok(units)
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
///
/// Re-exported from [`broadcast_common::hex`], which holds the single
/// definition: `broadcast-hls` renders an `#EXT-X-KEY:KEYID=0x…` attribute
/// with the same encoder (issue #878) and cannot reach into this crate for it
/// (the dependency runs the other way). Only the *encoder* is shared —
/// [`hex_decode`] below stays here because it reports through this crate's
/// own [`Error`].
///
/// Imported privately, NOT re-exported: `transmux::rtp::hex_encode` is gone as
/// a public path. Callers use `broadcast_common::hex::hex_encode` directly —
/// one owner, one name, no compatibility alias to keep in step.
use broadcast_common::hex::hex_encode;

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
    fn reassemble_video_reports_timestamp_and_sync() {
        // Two single-NAL AUs at different RTP timestamps; first is an IDR (type 5),
        // second a non-IDR slice (type 1). Marker bit ends each AU.
        // RTP fixed header: V=2 (0x80), PT=96; seq; timestamp; ssrc=0.
        fn pkt(seq: u16, ts: u32, marker: bool, nal: &[u8]) -> Vec<u8> {
            let mut p = alloc::vec![0x80u8, if marker { 0x80 | 96 } else { 96 }];
            p.extend_from_slice(&seq.to_be_bytes());
            p.extend_from_slice(&ts.to_be_bytes());
            p.extend_from_slice(&[0, 0, 0, 0]); // ssrc
            p.extend_from_slice(nal);
            p
        }
        let idr = [0x65u8, 0xAA]; // nal_ref_idc=3, type=5 (IDR)
        let non = [0x41u8, 0xBB]; // nal_ref_idc=2, type=1 (non-IDR)
        let packets = alloc::vec![pkt(1, 1000, true, &idr), pkt(2, 4000, true, &non)];
        let aus = reassemble_video(&packets).unwrap();
        assert_eq!(aus.len(), 2);
        assert_eq!(aus[0].timestamp, 1000);
        assert!(aus[0].is_sync, "IDR AU must be sync");
        assert_eq!(aus[1].timestamp, 4000);
        assert!(!aus[1].is_sync, "non-IDR AU must not be sync");
        // data is length-prefixed NAL (4-byte length + NAL)
        assert_eq!(&aus[0].data[..4], &[0, 0, 0, 2]);
        assert_eq!(&aus[0].data[4..], &idr);
    }

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
