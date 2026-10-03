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
//! - **The timestamp is the presentation time, not the decode time**
//!   (RFC 6184 §5.1: "The RTP timestamp is set to the sampling timestamp of
//!   the content"; receivers "SHOULD use the RTP timestamp for synchronizing
//!   the display process"). So [`RtpPacketiser`] stamps each access unit with
//!   the sample's **`pts`**, and the depacketisers read it back into `pts` —
//!   a stream with B-frame reordering must not be presented in decode order at
//!   the wrong instants. RTP carries no second timestamp, so a decode time is
//!   not recoverable from the wire: the single-shot depacketiser reconstructs
//!   the decode timeline from the whole stream it holds (the presentation
//!   instants re-laid in wire order, delayed by the reorder depth) and reports
//!   a reorder through [`RtpTimingWarning`]; the streaming depacketiser keeps
//!   the documented low-delay model (`dts == pts`) and reports it through
//!   [`crate::rtp_stream::RtpLossEvent::NonMonotonicTimestamp`].
//! - **H.264** (RFC 6184): single-NAL packets (NAL type 1–23), STAP-A
//!   (type 24) aggregation for the SPS+PPS parameter sets, and FU-A (type 28)
//!   fragmentation of any NAL larger than the MTU. Video IR samples are 4-byte
//!   length-prefixed NALs ([`crate::annexb`]); the length prefixes are stripped
//!   on packetise and re-added on depacketise.
//! - **AAC** (RFC 3640, `AAC-hbr`): an AU-headers-length (16-bit, in bits)
//!   prefix + one 2-byte AU-header (`sizeLength=13; indexLength=3`) + the raw
//!   access unit.
//! - **SDP** (RFC 4566 + `fmtp`; the SDP text is written with `sdp-types`, so
//!   `RtpOutput::sdp` is `std`-only): `sprop-parameter-sets` carries base64 SPS,PPS
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

use alloc::collections::VecDeque;
#[cfg(any(feature = "std", test))]
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

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
/// The `c=` connection address [`build_sdp_with_connection`] emits by
/// default: the same loopback the `o=` line names (RFC 8866 §5.7).
pub const LOCAL_CONNECTION_ADDRESS: core::net::IpAddr =
    core::net::IpAddr::V4(core::net::Ipv4Addr::LOCALHOST);
/// First payload type of the dynamic range (RFC 3551 §6).
const DYNAMIC_PT_MIN: u8 = 96;
/// Last payload type of the dynamic range (RFC 3551 §6).
const DYNAMIC_PT_MAX: u8 = 127;
/// Default video RTP clock rate (RFC 6184 — H.264 is carried at 90 kHz).
pub const VIDEO_CLOCK_RATE: u32 = 90_000;
/// Default audio RTP clock rate used by [`RtpInputStream::new`] when the
/// caller does not supply the stream's own (RFC 3551 §4.5 lists 48000 as a
/// registered audio clock rate; the negotiated rate belongs in the SDP's
/// `a=rtpmap`, RFC 3640 §4.1 for `mpeg4-generic`).
pub const DEFAULT_AAC_CLOCK_RATE: u32 = 48_000;

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
#[cfg(feature = "std")]
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
    /// The session-level SDP describing every stream (RFC 4566). `std` only:
    /// it is written with `sdp-types`.
    #[cfg(feature = "std")]
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

/// Hands out dynamic payload types (RFC 3551 §6: "This profile reserves
/// payload type numbers in the range 96-127 exclusively for dynamic
/// assignment") for one session's streams, never reusing a type and never
/// handing out one this crate has already bound to a fixed encoding.
///
/// Payload types are per-session bindings, so two streams sharing one produces
/// an SDP that describes both as the same encoding — a receiver decodes the
/// second as the first. Before this, a *third* AVC (or AAC) track got
/// `video_pt + 2` because only "has a video track been seen" was tracked, so
/// tracks 2 and 3 collided, and `96 + 2 = 98` additionally collided with
/// [`DEFAULT_KLV_PT`].
struct PtAllocator {
    /// The next candidate type. Advanced past every allocation, so no value is
    /// ever handed out twice.
    next: u8,
}

impl PtAllocator {
    /// Start allocating at `base`. A base outside the dynamic range (as an
    /// explicit dynamic value, not a static one) yields types in the dynamic
    /// range rather than a static assignment silently overwriting one.
    fn new(base: u8) -> Self {
        let next = if (DYNAMIC_PT_MIN..=DYNAMIC_PT_MAX).contains(&base) {
            base
        } else {
            DYNAMIC_PT_MIN
        };
        Self { next }
    }

    /// Claim `base` for a caller that asked for it, so no automatic allocation
    /// ever returns it again.
    fn reserve_through(&mut self, base: u8) {
        self.next = self.next.max(base.saturating_add(1));
    }

    /// The next unused payload type, skipping the ones this crate binds to a
    /// fixed encoding ([`DEFAULT_KLV_PT`]) and the static range entirely.
    fn allocate(&mut self) -> Option<u8> {
        loop {
            if self.next > DYNAMIC_PT_MAX {
                return None;
            }
            let pt = self.next;
            self.next = self.next.checked_add(1)?;
            if pt != DEFAULT_KLV_PT {
                return Some(pt);
            }
        }
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
        #[cfg(feature = "std")]
        let mut sdp_media: Vec<sdp_types::Media> = Vec::new();
        // One allocator per session, shared by both kinds: a payload type is a
        // session-wide binding (RFC 3551 §6), so a video and an audio stream
        // must never be handed the same one either.
        let mut pts = PtAllocator::new(self.video_pt.min(self.audio_pt));
        // Both kinds' preferred bases are claimed up front, not lazily: the
        // defaults (96 video, 97 audio) must survive a session whose tracks
        // are ordered audio-first, and a base must never be handed to the
        // *other* kind's second track before its own first track claims it.
        let mut next_video_pt = Some(self.video_pt);
        let mut next_audio_pt = Some(self.audio_pt);
        pts.reserve_through(self.video_pt);
        pts.reserve_through(self.audio_pt);

        for track in &media.tracks {
            match &track.spec.config {
                CodecConfig::Avc { config, .. } => {
                    // A caller-supplied base is used only if nothing has
                    // claimed it; allocation then continues from there, so a
                    // third video track gets its own type instead of colliding
                    // with the second.
                    let pt = match next_video_pt.take() {
                        Some(base) => {
                            pts.reserve_through(base);
                            base
                        }
                        None => pts.allocate().ok_or(Error::InvalidInput(
                            "no dynamic RTP payload type left (RFC 3551 §6's range is 96-127): too many streams share this session",
                        ))?,
                    };
                    let packets = self.packetise_video(track, pt)?;
                    streams.push(RtpStream {
                        pt,
                        kind: RtpMediaKind::H264,
                        packets,
                    });
                    #[cfg(feature = "std")]
                    sdp_media.push(sdp_video(pt, &config.config));
                    #[cfg(not(feature = "std"))]
                    let _ = config;
                }
                CodecConfig::Aac {
                    esds,
                    channel_count,
                    sample_rate,
                    ..
                } => {
                    let pt = match next_audio_pt.take() {
                        Some(base) => {
                            pts.reserve_through(base);
                            base
                        }
                        None => pts.allocate().ok_or(Error::InvalidInput(
                            "no dynamic RTP payload type left (RFC 3551 §6's range is 96-127): too many streams share this session",
                        ))?,
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
                    #[cfg(feature = "std")]
                    sdp_media.push(sdp_audio(pt, clock, *channel_count, asc));
                    #[cfg(not(feature = "std"))]
                    let _ = (asc, channel_count);
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
        Ok(RtpOutput {
            streams,
            #[cfg(feature = "std")]
            sdp: build_sdp(sdp_media),
        })
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
        if self.stap_a_parameter_sets
            && let CodecConfig::Avc { config, .. } = &track.spec.config
        {
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

        for (i, sample) in track.samples.iter().enumerate() {
            // Rescale to the 90 kHz RTP clock if the IR timescale differs.
            timestamp = rescale_ts(sample_pts(track, i), timescale, VIDEO_CLOCK_RATE);
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

    /// Packetise one AAC track (`AAC-hbr`).
    ///
    /// The AAC-hbr payload header (AU-headers-length + AU-header) is
    /// interleaved with the audio access unit, so the full packet is built
    /// in a `BytesMut` (which copies). The header is small (4 bytes) and
    /// audio AUs are typically <1 kB.
    ///
    /// An access unit that does not fit the payload budget is fragmented over
    /// consecutive packets per RFC 3640 §3.2.3.1: each fragment carries its
    /// own AU-header with the AU's **full** size (§3.2.3.2 — "the AU size
    /// indicates the size of the entire AU and not the size of the
    /// fragment"), they all share one RTP timestamp, and the marker bit is
    /// set on the last one only (§3.1). That is the ordinary case for
    /// high-rate audio (a 640 kb/s 5.1 AAC frame is ~3.7 kB, well over a
    /// typical MTU) — previously such a track could not be packetised at all.
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
        // Payload budget per packet after the fixed header and the two
        // AAC-hbr header fields.
        let per_packet = self
            .mtu
            .checked_sub(RTP_HEADER_LEN + AAC_AU_HEADERS_LENGTH_LEN + AAC_AU_HEADER_LEN)
            .filter(|&b| b > 0)
            .ok_or(Error::InvalidInput(
                "MTU too small for an AAC-hbr packet header",
            ))?;
        for (i, sample) in track.samples.iter().enumerate() {
            let au = &sample.data;
            if au.len() >= (1usize << AAC_SIZE_LENGTH) {
                return Err(Error::InvalidValue {
                    field: "aac_au_size",
                    value: au.len() as u64,
                    reason: "exceeds 13-bit AAC-hbr AU-size field",
                });
            }
            let timestamp = rescale_ts(sample_pts(track, i), timescale, clock);
            // AU-headers-length is in BITS: one 2-byte header = 16 bits. One
            // header per packet, and it always states the AU's full size
            // (§3.2.3.2), whether or not this packet carries all of it.
            let au_headers_len_bits = (AAC_AU_HEADER_LEN * 8) as u16;
            // AU-header: AU-size(13) | AU-Index(3). AU-Index = 0 (single AU).
            let hdr = (au.len() as u16) << AAC_INDEX_LENGTH;
            // Fragmentation (§3.2.3.1): the marker is set on the last fragment
            // only, and every fragment shares this AU's timestamp.
            let num_frags = au.len().div_ceil(per_packet).max(1);
            for f in 0..num_frags {
                let start = f * per_packet;
                let end = (start + per_packet).min(au.len());
                let is_last = f == num_frags - 1;
                // Build the full AAC-hbr header (RTP fixed header + AU-headers
                // prefix + AU-header) in a BytesMut, then extend with the
                // payload. This copies — the interleaving makes a zero-copy
                // approach impractical without a vectored I/O consumer.
                let rtp_hdr = rtp_header(pt, is_last, seq.next(), timestamp, self.ssrc);
                let frag = &au[start..end];
                let mut buf = bytes::BytesMut::with_capacity(
                    rtp_hdr.len() + AAC_AU_HEADERS_LENGTH_LEN + AAC_AU_HEADER_LEN + frag.len(),
                );
                buf.extend_from_slice(&rtp_hdr);
                buf.extend_from_slice(&au_headers_len_bits.to_be_bytes());
                buf.extend_from_slice(&hdr.to_be_bytes());
                buf.extend_from_slice(frag);
                let full = buf.freeze();
                let header_len = rtp_hdr.len() + AAC_AU_HEADERS_LENGTH_LEN + AAC_AU_HEADER_LEN;
                packets.push(RtpPacket {
                    header: full.slice(0..header_len),
                    payload: full.slice(header_len..),
                });
            }
        }
        Ok(packets)
    }
}

/// The **presentation** timestamp of sample `i`, in the track's media
/// timescale, **relative to the track's first sample** (so the emitted RTP
/// timestamp series starts at 0 for the first AU regardless of where the
/// source timeline sits — RFC 3550 §5.1 only constrains the increments).
///
/// RFC 6184 §5.1 is explicit: "The RTP timestamp is set to the sampling
/// timestamp of the content", and receivers "SHOULD use the RTP timestamp for
/// synchronizing the display process". That is a *presentation* time, so a
/// reordered stream (B-frames) must stamp each access unit with its **`pts`**,
/// not its `dts`: stamping the decode time makes a receiver present in decode
/// order at the wrong instants.
///
/// Read from the sample's own **absolute** `pts` when both it and the first
/// sample's are known. `pts` is `None` only for a section-carried track (which
/// RTP never packetises), so the fallback is the running sum of preceding
/// durations — which for a stream with no composition offset is the same
/// series (in that case `dts == pts` at every sample, and both paths agree).
fn sample_pts(track: &crate::media::Track, i: usize) -> u64 {
    if let (Some(first), Some(cur)) = (
        track.samples.first().and_then(|s| s.pts),
        track.samples.get(i).and_then(|s| s.pts),
    ) {
        // A hostile pair of timestamps can invert the subtraction; the
        // conversion is checked rather than an `as` cast, which would wrap a
        // large negative into a huge tick count.
        return u64::try_from((cur - first).max(0)).unwrap_or(0);
    }
    track.samples[..i]
        .iter()
        .map(|s| u64::from(s.duration.unwrap_or(0)))
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

/// Assemble the full session-level SDP from the per-media sections.
///
/// Includes a session-level `c=` line, because RFC 4566 §5.7 requires one:
/// "A session description MUST contain either at least one `c=` field in each
/// media description or a single `c=` field at the session level." The media
/// descriptions here use port 0 (the packets are handed over as
/// [`RtpPacket`]s, not sent from this process), so a strict parser enforcing
/// that requirement had nothing to read and rejected the description. The
/// address is the loopback the `o=` line already names; a caller that
/// transmits elsewhere uses [`build_sdp_with_connection`].
#[cfg(feature = "std")]
fn build_sdp(medias: Vec<sdp_types::Media>) -> String {
    build_sdp_with_connection(LOCAL_CONNECTION_ADDRESS, medias)
}

/// Assemble a session-level SDP with an explicit `c=` connection address
/// (RFC 8866 §5.7). The session SDP `RtpOutput::sdp` carries uses
/// [`LOCAL_CONNECTION_ADDRESS`].
///
/// The address is an [`IpAddr`](core::net::IpAddr), not a string, so it cannot
/// carry anything SDP would misread: the `<addrtype>` subfield (`IP4`/`IP6`)
/// follows the address's own family (RFC 8866 §5.7: "This memo only defines
/// `IP4` and `IP6`"), and no CR, LF or space can be smuggled into the line.
/// `Session::write` performs no CR/LF escaping of the session name or
/// attribute values; only the connection address is typed, and every other
/// value is produced internally by this module (fmtp strings are built here,
/// never from caller text).
///
/// The session is written with `sdp-types` 0.2 (`Session::write`), so this is
/// `std`-only.
#[cfg(feature = "std")]
pub fn build_sdp_with_connection(
    connection_address: core::net::IpAddr,
    medias: Vec<sdp_types::Media>,
) -> String {
    let mut session = sdp_types::Session::new(
        sdp_types::Origin::with_ip_addr(0, 0, core::net::Ipv4Addr::LOCALHOST),
        "transmux RTP",
    );
    // `from_ip_addr` derives `IP4`/`IP6` from the address family, so no CR/LF or
    // space can reach the c= line (the typed address is the injection guard).
    session.connection = Some(sdp_types::Connection::from_ip_addr(connection_address));
    session.medias = medias;
    let mut out = Vec::new();
    // Writing into a `Vec<u8>` cannot fail; every field is a `String`.
    let _ = session.write(&mut out);
    String::from_utf8_lossy(&out).into_owned()
}

/// SDP media section for an H.264 video stream (RFC 6184 §8.1).
#[cfg(feature = "std")]
fn sdp_video(
    pt: u8,
    config: &crate::avc_config::AVCDecoderConfigurationRecord,
) -> sdp_types::Media {
    let profile_level_id = format!(
        "{:02X}{:02X}{:02X}",
        config.profile_indication, config.profile_compatibility, config.level_indication
    );
    let sprop = config
        .sps
        .iter()
        .map(|n| base64_encode(&n.0))
        .chain(config.pps.iter().map(|n| base64_encode(&n.0)))
        .collect::<Vec<_>>()
        .join(",");
    let mut media = sdp_types::Media::new(
        sdp_types::MediaType::Video,
        0,
        sdp_types::TransportProto::RtpAvp,
        pt,
    );
    media.add_attribute(sdp_types::RtpMap::new(pt, "H264", VIDEO_CLOCK_RATE));
    // fmtp parameter lists are codec payload-format logic (owner decision,
    // spec §9.2): built as a string, never through `sdp_types::Fmtp`, whose
    // `Display` joins with `;` and no space and would change the bytes.
    media.add_attribute_with_value(
        "fmtp",
        format!(
            "{pt} packetization-mode=1; profile-level-id={profile_level_id}; sprop-parameter-sets={sprop}"
        ),
    );
    media
}

/// SDP media section for an AAC audio stream (`mpeg4-generic`, RFC 3640 §4.1).
#[cfg(feature = "std")]
fn sdp_audio(pt: u8, clock: u32, channels: u16, asc: &[u8]) -> sdp_types::Media {
    let config = hex_encode(asc);
    let mut media = sdp_types::Media::new(
        sdp_types::MediaType::Audio,
        0,
        sdp_types::TransportProto::RtpAvp,
        pt,
    );
    media.add_attribute(sdp_types::RtpMap::with_encoding_params(
        pt,
        "mpeg4-generic",
        clock,
        channels,
    ));
    media.add_attribute_with_value(
        "fmtp",
        format!(
            "{pt} streamtype=5; profile-level-id=1; mode=AAC-hbr; config={config}; \
             sizeLength={AAC_SIZE_LENGTH}; indexLength={AAC_INDEX_LENGTH}; \
             indexDeltaLength={AAC_INDEX_DELTA_LENGTH}"
        ),
    );
    media
}

// ---------------------------------------------------------------------------
// Depacketiser input
// ---------------------------------------------------------------------------

/// One RTP stream fed to [`RtpDepacketiser`]: its kind, clock, codec config
/// and packets.
///
/// The clock rate is **not** optional: an RTP timestamp is a count in the
/// stream's own clock (RFC 3550 §5.1, RFC 3551 §4.2 for the video/audio
/// defaults), so a track built without it states a duration in the wrong
/// unit — the pre-fix depacketiser stamped every track, audio included, at
/// the 90 kHz video clock.
///
/// `config` is the codec configuration for this stream, which RTP itself
/// never carries (it is negotiated in the SDP; see [`crate::rtp_sdp`]). It is
/// required for audio: an AAC track's initialisation data *is* its
/// `AudioSpecificConfig`, and no honest placeholder for it exists. Video
/// accepts the placeholder the wire implies when it is absent (an `avcC`
/// with no parameter sets — the SDP's `sprop-parameter-sets` supplies
/// them), because transmux's depacketiser only needs a track identity and
/// the samples to verify a round trip.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RtpInputStream {
    /// The payload format carried on this stream.
    pub kind: RtpMediaKind,
    /// The stream's RTP clock rate in Hz — 90000 for H.264
    /// ([`VIDEO_CLOCK_RATE`], RFC 6184 §8.1's `H264/90000`) and the audio
    /// sample rate for `mpeg4-generic` (RFC 3640 §4.1). Becomes the IR
    /// track's timescale.
    pub clock_rate: u32,
    /// Codec configuration for this stream (from the session's SDP).
    /// Required for [`RtpMediaKind::Aac`].
    pub config: Option<CodecConfig>,
    /// Whether [`Self::with_clock_rate`] was called, i.e. the caller asserted
    /// the rate rather than leaving the constructor's default in place. A
    /// stream that asserts one and then supplies a config is checked for
    /// agreement instead of being silently overridden.
    clock_rate_explicit: bool,
    /// The RTP packets in arrival (sequence) order.
    pub packets: Vec<Vec<u8>>,
}

impl RtpInputStream {
    /// A stream carrying `kind` with no codec config, at the codec's default
    /// clock rate ([`VIDEO_CLOCK_RATE`] for H.264, [`DEFAULT_AAC_CLOCK_RATE`]
    /// for AAC).
    ///
    /// An AAC stream's clock **is** its sampling rate (RFC 3640 §3.1: "If an
    /// MPEG-4 audio stream is transported, the rate SHOULD be set to the same
    /// value as the sampling rate of the audio stream"), so the default is
    /// only a placeholder until a config arrives: [`Self::with_config`] then
    /// takes the rate from the config, and a [`Self::with_clock_rate`] that
    /// disagrees is an error.
    pub fn new(kind: RtpMediaKind, packets: Vec<Vec<u8>>) -> Self {
        Self {
            kind,
            clock_rate: match kind {
                RtpMediaKind::H264 => VIDEO_CLOCK_RATE,
                RtpMediaKind::Aac => DEFAULT_AAC_CLOCK_RATE,
            },
            config: None,
            clock_rate_explicit: false,
            packets,
        }
    }

    /// Set this stream's codec configuration (required for AAC).
    ///
    /// For AAC the clock rate is derived from the config's own sample rate,
    /// because that is what RFC 3640 §3.1 defines it to be — a stream whose
    /// declared clock disagreed with its `AudioSpecificConfig` would be timed
    /// at the wrong rate by every sample duration this crate computes. Set
    /// [`Self::with_clock_rate`] first to assert a specific value: the two are
    /// checked for agreement.
    pub fn with_config(mut self, config: CodecConfig) -> Self {
        if let CodecConfig::Aac { sample_rate, .. } = &config {
            // The declared rate is only "asserted" when the caller set one
            // explicitly; the constructor's placeholder is replaced.
            if !self.clock_rate_explicit {
                self.clock_rate = *sample_rate;
            }
        }
        self.config = Some(config);
        self
    }

    /// Override this stream's RTP clock rate in Hz.
    pub fn with_clock_rate(mut self, clock_rate: u32) -> Self {
        self.clock_rate = clock_rate;
        self.clock_rate_explicit = true;
        self
    }
}

impl RtpInputStream {
    /// Validate this stream's kind/clock/config against each other, returning
    /// the clock rate to use.
    ///
    /// RFC 3640 §3.1 fixes an audio stream's RTP clock rate at its sampling
    /// rate, and a `CodecConfig::Aac` on an `H264` stream (or vice versa) would
    /// make the depacketiser parse one payload format as another. Both are
    /// caller mistakes with no sensible interpretation, so they are errors
    /// rather than silent adjustments.
    fn validated_clock_rate(&self) -> Result<u32> {
        match &self.config {
            Some(CodecConfig::Aac { sample_rate, .. }) => {
                if self.kind != RtpMediaKind::Aac {
                    return Err(Error::InvalidInput(
                        "an RTP stream carrying an AAC config must have kind Aac: the \
                         payload format is what the packets are parsed as",
                    ));
                }
                if self.clock_rate != *sample_rate {
                    return Err(Error::InvalidValue {
                        field: "rtp_clock_rate",
                        value: u64::from(self.clock_rate),
                        reason: "an MPEG-4 audio stream's RTP clock rate is its \
                                 sampling rate (RFC 3640 §3.1), so it must equal the \
                                 AudioSpecificConfig's sample rate",
                    });
                }
                Ok(*sample_rate)
            }
            Some(CodecConfig::Avc { .. }) => {
                if self.kind != RtpMediaKind::H264 {
                    return Err(Error::InvalidInput(
                        "an RTP stream carrying an AVC config must have kind H264: the \
                         payload format is what the packets are parsed as",
                    ));
                }
                if self.clock_rate != VIDEO_CLOCK_RATE {
                    return Err(Error::InvalidValue {
                        field: "rtp_clock_rate",
                        value: u64::from(self.clock_rate),
                        reason: "RFC 6184 §8.1 fixes the H.264 RTP clock rate at 90 kHz",
                    });
                }
                Ok(self.clock_rate)
            }
            // A config for some other codec, or none at all: the caller has
            // declared the clock itself, so honour it (the batch path fills in
            // a placeholder video config when none is given).
            Some(_) | None => {
                if self.clock_rate == 0 {
                    return Err(Error::InvalidInput(
                        "RTP stream has no clock rate: an RTP timestamp's unit is the \
stream's own clock (RFC 3550 §5.1), so a track cannot be timed without it",
                    ));
                }
                Ok(self.clock_rate)
            }
        }
    }
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
    /// Timing warnings raised by the last [`Unpackage::unpackage`] call, in the
    /// order they were raised — see [`RtpTimingWarning`] and
    /// [`RtpDepacketiser::poll_timing_warning`]. Bounded by
    /// [`MAX_TIMING_WARNINGS`].
    timing_warnings: VecDeque<RtpTimingWarning>,
    /// Warnings dropped because the queue was full, summed over the session —
    /// see [`RtpDepacketiser::dropped_timing_warnings`].
    dropped_timing_warnings: u64,
}

impl RtpDepacketiser {
    /// Create a new depacketiser.
    pub fn new() -> Self {
        Self::default()
    }

    /// Take the next timing assumption this depacketiser had to make while
    /// reassembling, or `None` when there are no more.
    ///
    /// [`Unpackage::unpackage`] cannot return anything but `Media`, so a
    /// condition the RTP wire genuinely cannot express is reported here rather
    /// than being silently absorbed — the same reason
    /// [`crate::rtp_stream::RtpStreamDepacketiser`] has
    /// `poll_loss_event`. Drain it after every
    /// [`unpackage`](Unpackage::unpackage) call.
    pub fn poll_timing_warning(&mut self) -> Option<RtpTimingWarning> {
        self.timing_warnings.pop_front()
    }

    /// How many warnings have been dropped because more than
    /// [`MAX_TIMING_WARNINGS`] were raised in one
    /// [`unpackage`](Unpackage::unpackage) call.
    ///
    /// The queue is bounded so a hostile stream (a timestamp that reorders on
    /// every packet) cannot make the depacketiser allocate without limit; a
    /// caller that needs an exact count of reordering events should use that
    /// count rather than assume the queue is exhaustive.
    pub fn dropped_timing_warnings(&self) -> u64 {
        self.dropped_timing_warnings
    }
}

/// How many [`RtpTimingWarning`]s one
/// [`unpackage`](Unpackage::unpackage) call retains.
///
/// RTP is untrusted remote input: a stream whose timestamps reorder on every
/// packet would otherwise queue one warning per access unit, growing without
/// bound over a long session. The cap is far above the reorder events a real
/// stream produces (a B-frame group raises one per backward step, and a
/// pathological sequence a handful more), so a caller draining the queue sees
/// every event it would act on, while the memory cost stays constant.
pub const MAX_TIMING_WARNINGS: usize = 1024;

/// A field the batch [`RtpDepacketiser`] saw that RTP carries no way to express
/// exactly, because it carries exactly one timestamp per access unit — the
/// *sampling (presentation)* time (RFC 6184 §5.1 for H.264, RFC 3640 §3.3.1 for
/// AAC) — and no decode time at all.
///
/// Nothing has been mis-assembled when one of these fires: the access units are
/// byte-exact and their `pts` values are the wire's own. What it means is that
/// the **decode timeline** (the IR's [`Sample::dts`](crate::ir::Sample::dts))
/// was reconstructed rather than read — for a reordered stream it is the
/// presentation instants re-laid in wire order and delayed by the reorder depth,
/// which is exact, but a consumer doing its own composition-offset arithmetic
/// should know the wire did not state those offsets itself.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RtpTimingWarning {
    /// An access unit's presentation timestamp was earlier than a previous
    /// one's: the stream's presentation order is not its wire (decode) order,
    /// so its decode timeline was reconstructed (see the type docs) rather than
    /// read from the wire.
    ReorderedPresentationTimestamps {
        /// The zero-based index of the stream this was observed on, in the
        /// order the streams were passed to
        /// [`RtpInput::streams`](RtpInput).
        stream_index: usize,
        /// The previous access unit's presentation timestamp, in the stream's
        /// clock (`u64`, already unwrapped).
        previous_pts: u64,
        /// This access unit's, which is earlier.
        pts: u64,
    },
}

// `RtpTimingWarning` is a data-carrying ADT (each variant is a distinct
// structured signal, not a flat spec code) — see this crate's
// `tests/label_coverage.rs` SKIP list, so it is intentionally exempt from the
// #204 `name()`/`impl_spec_display!` convention.

impl Unpackage for RtpDepacketiser {
    type Input = RtpInput;
    type Media = Media;
    type Error = Error;

    fn unpackage(&mut self, input: RtpInput) -> Result<Media> {
        self.timing_warnings.clear();
        self.dropped_timing_warnings = 0;
        let mut tracks = Vec::new();
        for (idx, stream) in input.streams.iter().enumerate() {
            let clock_rate = stream.validated_clock_rate()?;
            let samples = match stream.kind {
                RtpMediaKind::H264 => depacketise_video(&stream.packets)?,
                RtpMediaKind::Aac => depacketise_audio(&stream.packets)?,
            };
            // RFC 6184 §5.1 / RFC 3640 §3.3.1: the wire timestamp is the
            // sampling (presentation) time, one per access unit. A step
            // backward means presentation order is not wire (decode) order, so
            // `dts == pts` below is an assumption, not a fact — report it
            // rather than absorbing it (see [`RtpTimingWarning`]).
            let mut prev: Option<i64> = None;
            let mut wrap_check = RtpWrapState::default();
            for au in &samples {
                let pts = wrap_check.push(au.timestamp);
                if let Some(previous_pts) = prev
                    && pts < previous_pts
                {
                    if self.timing_warnings.len() == MAX_TIMING_WARNINGS {
                        // Bounded: drop the oldest so the newest (which is the
                        // one a caller is most likely acting on) is kept, and
                        // count the loss so it is never silent.
                        self.timing_warnings.pop_front();
                        self.dropped_timing_warnings += 1;
                    }
                    self.timing_warnings.push_back(
                        RtpTimingWarning::ReorderedPresentationTimestamps {
                            stream_index: idx,
                            previous_pts: u64::try_from(previous_pts).unwrap_or(0),
                            pts: u64::try_from(pts).unwrap_or(0),
                        },
                    );
                }
                prev = Some(pts);
            }
            tracks.push(RtpTrack {
                // Track IDs are 1-based. `u32::try_from` rather than `as u32`:
                // a stream index past `u32::MAX` is not reachable (the caller
                // had to build the `Vec`), and the fallback states that rather
                // than truncating an index into a colliding ID.
                id: u32::try_from(idx).unwrap_or(u32::MAX).saturating_add(1),
                kind: stream.kind,
                clock_rate,
                config: stream.config.clone(),
                samples,
            });
        }
        rtp_tracks_to_media(tracks)
    }
}

/// A reassembled RTP track: its payload-format kind, RTP clock, the codec
/// config carried alongside the wire packets (RTP itself has none — see
/// [`RtpInputStream::config`]) and the reassembled coded samples.
struct RtpTrack {
    id: u32,
    kind: RtpMediaKind,
    clock_rate: u32,
    config: Option<CodecConfig>,
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

/// Reconstruct the IR timing pair (`dts`, `pts`) of a stream from its wire
/// (presentation) timestamps, in wire order.
///
/// RTP carries exactly one timestamp per access unit and it is the *sampling /
/// presentation* time (RFC 6184 §5.1 for H.264, RFC 3640 §3.3.1 for AAC); no
/// decode time exists on the wire, and the sequence number carries no timing.
/// The IR, however, wants an absolute `dts`, an exactly-representable
/// composition offset (`pts >= dts`, always), and a non-decreasing `dts`.
///
/// Two cases:
///
/// - **Presentation order is wire order** (`pts` non-decreasing): there is no
///   reordering, so decode time *is* presentation time and `dts = pts`
///   unchanged. This is also the only correct answer for a variable-frame-rate
///   or lossy stream, where the steps are not a fixed grid — laying one down
///   would misplace every sample after the first irregular step.
/// - **Reordered** (some `pts` earlier than one already sent — B-frames): the
///   decode timeline is its own uniform grid, one frame period apart in wire
///   order, starting `latch` periods *before* the first presentation instant
///   so that `dts <= pts` holds at every sample (a frame cannot be presented
///   before it is decoded). The frame period is the **greatest common divisor**
///   of the presented steps: every step of a uniform grid is a whole number of
///   periods, so the smallest *positive* step alone is wrong (a pyramid group's
///   adjacent same-reference frames are two periods apart) while the gcd
///   recovers the true period. `latch` is the largest number of earlier-sent
///   frames any one frame is presented before, i.e. exactly how far the decode
///   schedule has to run ahead.
///
/// The whole timeline is then translated so its first decode instant is 0 —
/// the IR's `Track::start_decode_time` is a `u64` anchor that must equal
/// `samples[0].dts`, and both series shift together, so every delta, the
/// ordering, and `dts <= pts` are preserved exactly.
///
/// Returns `(dts, pts)` per sample, in wire order.
fn decode_timeline(pts: &[i64]) -> (Vec<i64>, Vec<i64>) {
    if pts.windows(2).all(|w| w[1] >= w[0]) {
        return (pts.to_vec(), pts.to_vec());
    }
    // The decode instants are the *presentation* instants, re-laid in wire
    // (decode) order: every frame is presented exactly once, so the set of
    // decode times equals the set of presentation times, and only the
    // assignment changes. That is exact for a real reordered stream — the
    // durations come out as the encoder's own (a 23.976 fps stream at 90 kHz
    // alternates 3753/3754 ticks, which a fixed arithmetic step cannot
    // reproduce: the gcd of those steps is 1, so a "period" estimate would
    // declare a 1-tick frame and every duration would be 1 or 2).
    let mut sorted: Vec<i64> = pts.to_vec();
    sorted.sort_unstable();

    // The smallest constant delay that puts every decode instant at or before
    // its own presentation instant: the decoder must be able to have produced
    // the frame by the time it is presented. It is bounded by the real reorder
    // depth, because `sorted[i] - pts[i]` is at most as large as the interval
    // the reorder spans.
    let latch = sorted
        .iter()
        .zip(pts.iter())
        .map(|(s, p)| s.saturating_sub(*p))
        .max()
        .unwrap_or(0)
        .max(0);

    // The series stay on the wire's own absolute media-clock timeline — the
    // same timeline a non-reordered track keeps (its `dts == pts == the
    // unwrapped RTP timestamp`) — so every track in one `Media` shares one
    // epoch. Zero-basing only the reordered tracks would put them on a
    // different epoch from their own siblings, which is worse than either
    // choice made consistently; a consumer wanting a zero origin applies
    // `crate::rebase::rebase_to_zero` to the whole `Media`, which shifts every
    // track in lockstep.
    let mut dts: Vec<i64> = Vec::with_capacity(pts.len());
    let mut shifted_pts: Vec<i64> = Vec::with_capacity(pts.len());
    for (s, p) in sorted.iter().zip(pts.iter()) {
        let d = *s - latch;
        dts.push(d);
        shifted_pts.push((*p).max(d));
    }
    (dts, shifted_pts)
}

/// A decode-time delta as a sample duration: clamped at 0 (a non-decreasing
/// timeline cannot produce a negative one, but a hostile stream's clamped grid
/// can tie), and checked rather than truncated — a delta past `u32::MAX` ticks
/// is not a duration any container can carry, and saturating is the only
/// lossless choice left for a value that is already the length of the stream.
fn duration_from(delta: i64) -> u32 {
    u32::try_from(delta.max(0)).unwrap_or(u32::MAX)
}

/// Reassembled RTP samples, exposed on [`Media`] via a light wrapper. Since the
/// hub IR carries codec config, the depacketiser returns the raw reassembled
/// access units on each track's samples for round-trip verification; callers
/// pair them with the SDP-derived config as needed.
fn rtp_tracks_to_media(tracks: Vec<RtpTrack>) -> Result<Media> {
    use crate::pipeline::{Sample, TrackSpec};
    // Phase 1: unwrap each track's wire timestamps (32-bit wrap, once, here at
    // the demux edge — RFC 3550 §5.1) and reconstruct its decode timeline.
    //
    // Honest scope: RTP's timestamp origin is a *random* offset per stream
    // (§5.1), so without an RTCP SR (`rtcp::SenderReport`) NTP↔RTP mapping
    // these are absolute **media-clock** timelines with arbitrary epochs, not
    // wall-clock ones. That is exactly what the IR's `dts`/`pts` mean (ticks in
    // the track timescale), and it preserves real inter-sample timing.
    let mut prepared: Vec<(RtpTrack, TrackSpec, Vec<i64>, Vec<i64>)> =
        Vec::with_capacity(tracks.len());
    for t in tracks {
        let spec = track_spec(&t)?;
        let mut wrap = RtpWrapState::default();
        let pts_series: Vec<i64> = t.samples.iter().map(|au| wrap.push(au.timestamp)).collect();
        let (dts_series, pts_series) = decode_timeline(&pts_series);
        prepared.push((t, spec, dts_series, pts_series));
    }

    // Phase 2: one origin for the whole `Media`, applied to every track, so a
    // reordered video track and a non-reordered audio track share one epoch
    // and no `dts` is negative. The IR's `Track::start_decode_time` is a `u64`
    // anchor that must equal `samples[0].dts`, which a negative value cannot
    // be; zero-basing only the reordered tracks (the previous behaviour) put
    // them on a different epoch from their own siblings. `crate::rebase`
    // shifts every track of a `Media` together, so keeping the shift here
    // constant across tracks is what makes that consistent.
    let origin = prepared
        .iter()
        .flat_map(|(_, _, dts, _)| dts.iter().copied())
        .min()
        .unwrap_or(0)
        .min(0);

    let ir_tracks = prepared
        .into_iter()
        .map(
            |(t, spec, dts_series, pts_series)| -> Result<crate::media::Track> {
                // A pure translation by one constant: `origin` is the lowest decode
                // instant in the whole `Media`, so no `dts` becomes negative and no
                // duration changes. (The `max(0)` is defensive — the translation
                // cannot produce a negative value by construction.)
                let dts_series: Vec<i64> =
                    dts_series.iter().map(|d| (*d - origin).max(0)).collect();
                let pts_series: Vec<i64> =
                    pts_series.iter().map(|p| (*p - origin).max(0)).collect();
                let n = dts_series.len();
                // The duration to reuse when a decode step is zero — see the
                // per-sample fallback below. Seeded with the first positive step in
                // the series, because a stream can open with repeated decode
                // instants (the latch can clamp several frames onto the same tick)
                // and the frames before the first positive step still need a
                // duration. `None` only when the whole series is one instant.
                let mut previous_nonzero_duration: Option<i64> =
                    dts_series.windows(2).map(|w| w[1] - w[0]).find(|d| *d > 0);
                let samples: Vec<Sample> = t
                    .samples
                    .iter()
                    .enumerate()
                    .map(|(i, au)| {
                        let pts = pts_series[i];
                        let dts = dts_series[i];
                        // Duration = the decode delta to the next AU; the final AU
                        // reuses the previous delta (the same one-behind rule
                        // `ts_demux`/`flv` use), and a single-AU track has no
                        // measurable duration at all.
                        //
                        // A **non-final** frame never gets a zero duration: two AUs
                        // stamped with the same instant is something the wire can
                        // produce and the decode timeline then gives them a zero
                        // step, but every container writer in this crate rejects a
                        // zero-duration sample (it describes a frame the decoder is
                        // told to replace instantly). The fallback is the most
                        // recent non-zero duration, and 1 tick if there has not
                        // been one yet — a duration the stream does not have is
                        // still less wrong than one no writer accepts.
                        let delta = if i + 1 < n {
                            let d = dts_series[i + 1] - dts;
                            if d > 0 {
                                Some(d)
                            } else {
                                previous_nonzero_duration
                            }
                        } else if n >= 2 {
                            let d = dts - dts_series[i - 1];
                            if d > 0 {
                                Some(d)
                            } else {
                                previous_nonzero_duration
                            }
                        } else {
                            // A single-access-unit track has no measurable delta at
                            // all. The IR requires a duration on every timed
                            // sample, and no writer accepts zero, so the floor of
                            // one tick is what is left — the same value the
                            // per-sample fallback uses when nothing better is
                            // known.
                            Some(1)
                        };
                        if let Some(d) = delta
                            && d > 0
                        {
                            previous_nonzero_duration = Some(d);
                        }
                        let duration = delta.map(|d| duration_from(d.max(1)));
                        Sample {
                            data: au.data.clone().into(),
                            dts: Some(dts),
                            pts: Some(pts),
                            duration,
                            flags: crate::ir::SampleFlags::new(au.is_sync),
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
                    .map(|d| u64::try_from(d.max(0)).unwrap_or(0))
                    .unwrap_or(0);
                Ok(crate::media::Track::new_at(spec, samples, anchor))
            },
        )
        .collect::<Result<Vec<_>>>()?;
    // The movie timescale is the video clock (the same choice `ts_demux`
    // makes): a non-zero movie timescale is what every consumer's rescale
    // arithmetic divides by, and RTP's own are the only rates known here.
    Ok(Media::new(ir_tracks, VIDEO_CLOCK_RATE))
}

/// The IR [`TrackSpec`] for a depacketised stream: its own kind and RTP
/// clock, plus the codec config supplied alongside the wire packets.
///
/// The clock rate is load-bearing — it becomes the track's timescale, and
/// every timestamp/duration this module emits is a count in it (RFC 3550
/// §5.1). Before this took the stream's own clock, every track — audio
/// included — was declared at [`VIDEO_CLOCK_RATE`] with an AVC config, so a
/// consumer writing a container from the result described audio timing as
/// 90 kHz video.
///
/// Audio has no placeholder: an AAC track's initialisation data *is* its
/// `AudioSpecificConfig` (carried in the SDP `config=` parameter, RFC 3640
/// §4.1), and inventing one would state a sample rate and channel
/// configuration the stream may not have. Video accepts the empty-parameter-
/// set placeholder the wire implies, since the samples are self-describing
/// enough for a round-trip check.
fn track_spec(t: &RtpTrack) -> Result<crate::pipeline::TrackSpec> {
    use crate::pipeline::TrackSpec;
    let config = match (&t.config, t.kind) {
        (Some(c), _) => c.clone(),
        (None, RtpMediaKind::H264) => placeholder_avc_config(),
        (None, RtpMediaKind::Aac) => {
            return Err(Error::InvalidInput(
                "an AAC RTP stream needs its AudioSpecificConfig: RTP carries no codec config, so it must come from the session's SDP `config=` parameter (RFC 3640 §4.1) - a fabricated one would name a sample rate and channel count the stream may not have",
            ));
        }
    };
    Ok(TrackSpec::new(t.id, t.clock_rate, config))
}

/// Minimal AVC config for a stream whose parameter sets live in the SDP's
/// `sprop-parameter-sets` (RFC 6184 §8.1) rather than on the RTP wire, which
/// carries none. The samples are the payload of interest; this is only a
/// track identity, and is never serialized to a container here.
fn placeholder_avc_config() -> CodecConfig {
    use crate::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
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
    CodecConfig::Avc {
        config: AVCConfigurationBox::new(record),
        width: 0,
        height: 0,
    }
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
    // Sequence number of the previous packet. Every packet of a stream
    // increments this by one (RFC 3550 §5.1: "sequence number: increments by 1
    // per packet"), so a jump means a packet of the access unit under
    // construction never arrived — whether it was a single-NAL packet, a
    // STAP-A, or a fragment of an FU-A run.
    let mut last_seq: Option<u16> = None;

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

        // A sequence-number gap means a packet of this access unit never
        // arrived. The check runs *before* the timestamp flush below, so the AU
        // being closed is the one marked damaged (it is missing a NAL) rather
        // than the *new* AU's first packet being discarded with it: RTP is
        // untrusted UDP input, and one lost packet must cost exactly the AU it
        // damaged.
        //
        // Three shapes, all of which leave the AU under construction
        // incomplete:
        //  - the gap falls between two packets of the current AU (any kind);
        //  - an FU-A run was open and the next packet is not its continuation
        //    (the `fu_active` case, handled in the FU-A arm);
        //  - the gap falls immediately *before* an FU-A start, which means the
        //    run that start belongs to lost its first fragment (or the AU
        //    before it lost its last one) — either way this AU is incomplete.
        let gap = last_seq.is_some_and(|previous| previous.wrapping_add(1) != hdr.sequence);
        last_seq = Some(hdr.sequence);
        if gap {
            //  Drop the NALs collected for the AU under construction, and mark
            //  the open FU-A run damaged so its continuation cannot join a
            //  later run.
            cur_nals.clear();
            fu_buf.clear();
            fu_active = false;
        }
        if let Some(ts) = cur_ts
            && ts != hdr.timestamp
            && !cur_nals.is_empty()
        {
            flush_au(&mut aus, &mut cur_nals, ts);
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
                    // A continuation fragment with no preceding start: a
                    // mid-stream capture (the run began before the first
                    // packet we saw) or one lost start packet — or a hole in
                    // the run detected just above. RFC 6184 §5.8 gives no way
                    // to recover the missing NAL header or body, so this
                    // fragment cannot be turned into a NAL — but it is *this
                    // NAL* that is unusable, not the stream: skip it and
                    // carry on, exactly as
                    // [`crate::rtp_stream::RtpStreamDepacketiser`] does (which
                    // records `DamagedAccessUnit`). Failing the whole input
                    // meant one lost packet made every packet after it
                    // unreadable too.
                    continue;
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
///
/// Handles RFC 3640 §3.2.3.1 fragmentation: an access unit larger than the
/// payload budget is split across packets that **share one RTP timestamp**,
/// with the marker bit set on the last fragment (§3.1). Each fragment's
/// AU-header carries its `AU-size` as the size of the *entire* AU, not of the
/// fragment (§3.2.3.2) — "the AU size indicates the size of the entire AU and
/// not the size of the fragment ... particularly useful after losing a packet
/// carrying the last fragment of an AU". A 640 kb/s 5.1 AAC stream has ~3.7 kB
/// frames, far over a typical MTU, so this is the ordinary case for
/// high-rate audio, not an exotic one: before this, such a packet was rejected
/// as `BufferTooShort` and the whole input failed.
///
/// A fragment run is only accepted when it is **contiguous**: every fragment
/// must be the next sequence number (RFC 3550 §5.1: "sequence number:
/// increments by 1 per packet"), and the accumulated bytes must equal the
/// declared `AU-size` exactly. A run with a hole is discarded rather than
/// emitted with its middle missing, and a run whose bytes exceed the declared
/// size (duplicated fragments) is discarded too — the header states the size,
/// so a mismatch means the run is not the AU it claims to be. Either way
/// reassembly resumes at the next access unit.
pub(crate) fn reassemble_audio(packets: &[Vec<u8>]) -> Result<Vec<ReassembledAu>> {
    let mut aus = Vec::new();
    /// One AU under reassembly from fragments (RFC 3640 §3.2.3.1).
    struct PendingAu {
        timestamp: u32,
        /// The AU's full size, from any fragment's AU-header.
        size: usize,
        /// Sequence number of the fragment most recently appended, so the next
        /// one can be required to follow it (RFC 3550 §5.1).
        seq: u16,
        /// Whether a hole was seen, making this run unusable even if enough
        /// bytes eventually accumulate.
        gap: bool,
        data: Vec<u8>,
    }
    let mut pending: Option<PendingAu> = None;

    /// Push the AU under reassembly, but only if it is exactly the AU it
    /// declares itself to be: no hole in the fragment run, and the bytes
    /// accumulated equal the AU-header's `AU-size` (§3.2.3.2). Anything else
    /// is dropped rather than handed to a decoder as a partial frame.
    fn complete_if_exact(aus: &mut Vec<ReassembledAu>, pending: &mut Option<PendingAu>) {
        if pending
            .as_ref()
            .is_some_and(|p| !p.gap && !p.data.is_empty() && p.data.len() == p.size)
            && let Some(p) = pending.take()
        {
            aus.push(ReassembledAu {
                timestamp: p.timestamp,
                is_sync: true,
                data: p.data,
            });
        }
    }

    /// Drop the AU under reassembly whatever its state — used when a new AU
    /// begins, since its remaining fragments are never coming.
    fn discard(pending: &mut Option<PendingAu>) {
        pending.take();
    }

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
        // A packet carries complete AUs or a single fragment of one (§3.2.3),
        // so more than one header means each header is a whole AU.
        let mut sizes = Vec::with_capacity(num_headers);
        for h in 0..num_headers {
            let hoff = off + h * AAC_AU_HEADER_LEN;
            let ah = u16::from_be_bytes([payload[hoff], payload[hoff + 1]]);
            sizes.push((ah >> AAC_INDEX_LENGTH) as usize);
        }
        off += header_bytes;

        // A different timestamp ends any AU under reassembly (§3.1: one
        // timestamp only ever refers to fragments of one AU).
        if pending
            .as_ref()
            .is_some_and(|p| p.timestamp != hdr.timestamp)
        {
            complete_if_exact(&mut aus, &mut pending);
            discard(&mut pending);
        }

        for size in sizes {
            let available = payload.len().saturating_sub(off);
            let take = size.min(available);
            match pending.as_mut() {
                Some(p) if p.size == size => {
                    // An ordinary continuation fragment of the AU in flight —
                    // provided it really is the next packet of the run.
                    if p.seq.wrapping_add(1) == hdr.sequence {
                        p.data.extend_from_slice(&payload[off..off + take]);
                        p.seq = hdr.sequence;
                    } else {
                        // A hole: this fragment does not follow the last one,
                        // so the AU's middle is missing. Mark the run unusable
                        // and keep it as the new (partial) run of this packet
                        // so a later contiguous run can still be recognised.
                        complete_if_exact(&mut aus, &mut pending);
                        discard(&mut pending);
                        pending = Some(PendingAu {
                            timestamp: hdr.timestamp,
                            size,
                            seq: hdr.sequence,
                            gap: true,
                            data: payload[off..off + take].to_vec(),
                        });
                    }
                }
                Some(_) => {
                    // A fragment whose declared AU-size disagrees with the one
                    // in flight: it belongs to a different AU.
                    complete_if_exact(&mut aus, &mut pending);
                    discard(&mut pending);
                    pending = Some(PendingAu {
                        timestamp: hdr.timestamp,
                        size,
                        seq: hdr.sequence,
                        gap: false,
                        data: payload[off..off + take].to_vec(),
                    });
                }
                None => {
                    pending = Some(PendingAu {
                        timestamp: hdr.timestamp,
                        size,
                        seq: hdr.sequence,
                        gap: false,
                        data: payload[off..off + take].to_vec(),
                    });
                }
            }
            off += take;

            complete_if_exact(&mut aus, &mut pending);
        }
        // The marker marks the last fragment of an AU (§3.1). A complete AU
        // was already pushed above; what is left here is a run that ended
        // short, which is dropped.
        if hdr.marker {
            complete_if_exact(&mut aus, &mut pending);
            discard(&mut pending);
        }
    }
    complete_if_exact(&mut aus, &mut pending);
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
        if let Some(ts) = cur_ts
            && ts != hdr.timestamp
            && !cur.is_empty()
        {
            units.push(core::mem::take(&mut cur));
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
// base64 + hex (RFC 4648) via the `base64` and `hex` crates
// ---------------------------------------------------------------------------

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};

/// Decoder used for SDP `sprop-parameter-sets`, DRM headers and Smooth/DASH
/// payloads: padding optional, trailing bits tolerated. Real producers emit
/// unpadded and non-canonical base64; the crate default is strict.
const LENIENT: GeneralPurpose = GeneralPurpose::new(
    &base64::alphabet::STANDARD,
    GeneralPurposeConfig::new()
        .with_decode_padding_mode(DecodePaddingMode::Indifferent)
        .with_decode_allow_trailing_bits(true),
);

/// Base64-encode bytes (RFC 4648 §4, with `=` padding).
pub fn base64_encode(data: &[u8]) -> String {
    STANDARD.encode(data)
}

/// Base64-decode a string (RFC 4648 §4); padding is optional and non-canonical
/// trailing bits are tolerated; any other invalid input is an error.
pub fn base64_decode(s: &str) -> Result<Vec<u8>> {
    LENIENT.decode(s).map_err(|e| match e {
        base64::DecodeError::InvalidByte(_, byte) => Error::InvalidValue {
            field: "base64",
            value: u64::from(byte),
            reason: "not a base64 character",
        },
        _ => Error::InvalidValue {
            field: "base64",
            value: s.len() as u64,
            reason: "invalid base64 length",
        },
    })
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
#[cfg(any(feature = "std", test))]
use broadcast_common::hex::hex_encode;

/// Hex-decode a string; rejects odd lengths and invalid nibbles.
pub fn hex_decode(s: &str) -> Result<Vec<u8>> {
    ::hex::decode(s).map_err(|e| match e {
        ::hex::FromHexError::OddLength => Error::InvalidValue {
            field: "hex",
            value: s.len() as u64,
            reason: "odd-length hex string",
        },
        ::hex::FromHexError::InvalidHexCharacter { c, .. } => Error::InvalidValue {
            field: "hex",
            value: u64::from(c),
            reason: "not a hex digit",
        },
        ::hex::FromHexError::InvalidStringLength => Error::InvalidValue {
            field: "hex",
            value: s.len() as u64,
            reason: "invalid hex string length",
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real 23.976 fps H.264 stream at the 90 kHz RTP clock presents its
    /// frames at `round(i * 90000 * 1001 / 24000)` ticks — i.e. steps that
    /// alternate **3753 and 3754**. Estimating a "period" from those steps
    /// (their greatest common divisor is 1) would declare a one-tick frame and
    /// give every sample a duration of 1 or 2 ticks, which is 2000x too short.
    /// This is the shape `fixtures/ts/h264/high.ts` has in miniature.
    /// A deterministic pseudo-random reordered sequence, for the scale tests
    /// below (no external RNG dependency in a `no_std` crate's tests).
    fn pseudo_random_pts(n: usize, seed: u64) -> alloc::vec::Vec<i64> {
        let mut state = seed | 1;
        let mut next = move || {
            // xorshift64; the constants are the standard ones for the shift
            // triple (13, 7, 17).
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut out = alloc::vec::Vec::with_capacity(n);
        let mut t = 0i64;
        for _ in 0..n {
            // Presentation instants advance by 1..4 periods...
            t += 1000 * (1 + (next() % 4) as i64);
            out.push(t);
        }
        // ...but are *sent* in a shuffled order (the reorder).
        for i in (1..out.len()).rev() {
            let j = (next() % (i as u64 + 1)) as usize;
            out.swap(i, j);
        }
        out
    }

    /// The decode timeline must equal the brute-force definition (each decode
    /// instant is the i-th smallest presentation instant, shifted by the
    /// smallest delay that keeps `dts <= pts`) — checked on a random reordered
    /// sequence, so the O(n log n) sort-based implementation cannot drift from
    /// what it claims to compute.
    /// Two access units stamped with the *same* RTP timestamp give the decode
    /// timeline a zero step. A non-final frame must still carry a usable
    /// duration — every container writer in this crate rejects a zero-duration
    /// sample — so the fallback is the last non-zero duration, or 1 tick if
    /// there has not been one.
    #[test]
    fn equal_timestamp_aus_never_get_a_zero_duration() {
        // pts 0, 3000, 3000, 6000: the repeated instant is the wire saying two
        // AUs share one sampling time.
        let packets: alloc::vec::Vec<alloc::vec::Vec<u8>> = [0u32, 3000, 3000, 6000]
            .iter()
            .enumerate()
            .map(|(i, ts)| {
                let mut p = alloc::vec![0x80u8, 0x80 | 96];
                p.extend_from_slice(&(i as u16).to_be_bytes());
                p.extend_from_slice(&ts.to_be_bytes());
                p.extend_from_slice(&[0, 0, 0, 0]);
                p.extend_from_slice(&[0x41, 0xAA]);
                p
            })
            .collect();
        let ir = RtpDepacketiser::new()
            .unpackage(crate::rtp::RtpInput {
                streams: alloc::vec![crate::rtp::RtpInputStream::new(RtpMediaKind::H264, packets,)],
            })
            .expect("depacketise");
        let samples = &ir.tracks[0].samples;
        assert_eq!(samples.len(), 4);
        for (i, s) in samples.iter().enumerate() {
            let d = s.duration.expect("a multi-sample track times every sample");
            if i + 1 < samples.len() {
                assert!(
                    d > 0,
                    "non-final sample {i} must not have a zero duration: {:?}",
                    samples
                        .iter()
                        .map(|s| s.duration)
                        .collect::<alloc::vec::Vec<_>>()
                );
            }
        }
        // The repeated instant's AU reuses the previous frame's period.
        assert_eq!(samples[1].duration, Some(3000));
        assert_eq!(
            samples[2].duration,
            Some(3000),
            "the duplicate instant must reuse the previous non-zero duration"
        );
    }

    #[test]
    fn decode_timeline_matches_a_brute_force_reference() {
        let pts = pseudo_random_pts(2000, 0x9E37_79B9_7F4A_7C15);
        let (dts, shifted) = decode_timeline(&pts);
        let reference = brute_force_decode_timeline(&pts);
        assert_eq!(dts, reference.0, "dts must match the reference");
        assert_eq!(shifted, reference.1, "pts must match the reference");
    }

    /// The brute-force definition of the decode timeline, used only as a test
    /// oracle: `sorted[i] - latch - first`, with the latch the smallest delay
    /// such that every `dts <= pts`. Deliberately written as directly as
    /// possible (no helper reuse) so it cannot share a bug with the
    /// implementation.
    fn brute_force_decode_timeline(pts: &[i64]) -> (alloc::vec::Vec<i64>, alloc::vec::Vec<i64>) {
        let mut sorted = pts.to_vec();
        sorted.sort_unstable();
        let mut latch = 0i64;
        for (s, p) in sorted.iter().zip(pts.iter()) {
            if s - p > latch {
                latch = s - p;
            }
        }
        let mut dts = alloc::vec::Vec::new();
        let mut out_pts = alloc::vec::Vec::new();
        for (s, p) in sorted.iter().zip(pts.iter()) {
            let d = *s - latch;
            dts.push(d);
            out_pts.push((*p).max(d));
        }
        (dts, out_pts)
    }

    /// The latch used to be computed with a per-index scan of the preceding
    /// samples — O(n²) on a hostile batch input (90 000 access units is ~4e9
    /// comparisons). This must simply finish: 200 000 reordered access units
    /// is far past any real capture, and the old shape would take minutes in a
    /// debug build.
    #[test]
    fn decode_timeline_is_not_quadratic() {
        let pts = pseudo_random_pts(200_000, 0x1234_5678_9ABC_DEF0);
        let (dts, shifted) = decode_timeline(&pts);
        assert_eq!(dts.len(), 200_000);
        assert_eq!(shifted.len(), 200_000);
        assert!(
            dts.windows(2).all(|w| w[1] >= w[0]),
            "dts must be non-decreasing at scale"
        );
        assert!(
            dts.iter().zip(&shifted).all(|(d, p)| d <= p),
            "dts must stay at or behind presentation at scale"
        );
        // The latch is exactly the deepest reorder in the sequence: every
        // decode instant sits at or before its own presentation instant, and
        // the timeline is the sorted instants shifted by one constant — so the
        // *durations* still sum to the span, whatever the shuffle.
        let first = dts[0];
        let durations: i64 = dts.windows(2).map(|w| w[1] - w[0]).sum();
        let span = pts.iter().max().copied().unwrap_or(0) - pts.iter().min().copied().unwrap_or(0);
        assert_eq!(
            durations, span,
            "the decode timeline must span the wire's own instants exactly"
        );
        // The shift is the deepest reorder of the sequence, so a case that
        // shuffles every instant is legitimately as deep as the largest
        // instant itself; the bound that matters is that it cannot exceed the
        // data (the check is here so a future change that multiplies it is
        // caught).
        let max_instant = pts.iter().copied().max().unwrap_or(0);
        assert!(
            first >= pts[0] - max_instant - span,
            "the latch must be bounded by the sequence's own instants"
        );
    }

    #[test]
    fn decode_timeline_handles_23_976_fps_tick_jitter() {
        // The presentation instants of a 24-frame 23.976 fps stream.
        let times: alloc::vec::Vec<i64> = (0..24i64)
            .map(|i| (i * 90_000 * 1001 + 12_000) / 24_000)
            .collect();
        // A real reorder pattern (display order 0,3,1,2,6,4,5,... sent in
        // decode order), so `pts` is genuinely out of order on the wire.
        let order: [usize; 24] = [
            0, 3, 1, 2, 6, 4, 5, 9, 7, 8, 12, 10, 11, 15, 13, 14, 18, 16, 17, 21, 19, 20, 23, 22,
        ];
        let pts: alloc::vec::Vec<i64> = order.iter().map(|i| times[*i]).collect();
        let (dts, shifted) = decode_timeline(&pts);

        assert!(
            !pts.windows(2).all(|w| w[1] >= w[0]),
            "the fixture must actually be reordered"
        );
        assert!(
            dts.windows(2).all(|w| w[1] >= w[0]),
            "dts must be non-decreasing: {dts:?}"
        );
        assert!(
            dts.iter().zip(&shifted).all(|(d, p)| d <= p),
            "dts must never run ahead of presentation"
        );
        // Durations are the encoder's own jittered frame times, not 1-2 ticks.
        let durations: alloc::vec::Vec<i64> = dts.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(
            durations.iter().all(|d| (3753..=3754).contains(d)),
            "every duration must be one jittered frame period, got {durations:?}"
        );
        assert!(
            durations.contains(&3753) && durations.contains(&3754),
            "the jitter must be preserved, not flattened: {durations:?}"
        );
    }

    #[test]
    fn decode_timeline_is_pts_when_presentation_order_is_wire_order() {
        // A variable-frame-rate series (a dropped frame, or a genuinely VFR
        // encoder): the steps are 3000 and 6000. There is no reordering, so
        // the decode timeline must be the presentation series itself — laying
        // down a uniform grid would misplace the third sample by 3000.
        let pts = [0i64, 3000, 9000];
        let (dts, shifted) = decode_timeline(&pts);
        assert_eq!(
            (dts.as_slice(), shifted.as_slice()),
            (&[0i64, 3000, 9000][..], &[0i64, 3000, 9000][..]),
            "a non-decreasing (VFR) series must leave dts == pts"
        );
        // The durations the caller derives from it are the real steps.
        assert_eq!(dts[1] - dts[0], 3000);
        assert_eq!(dts[2] - dts[1], 6000);
    }

    #[test]
    fn decode_timeline_recovers_the_standard_decode_order() {
        // A B-frame reorder: two frames sent out of presentation order.
        // Presentation instants are 0, 3, 1, 2 (x1000); the standard decode
        // order for that pattern is one frame period apart (the middle frames
        // are latched), which is exactly a zero-based uniform grid.
        let pts = [0i64, 3000, 1000, 2000];
        let (dts, shifted) = decode_timeline(&pts);
        assert_eq!(dts, alloc::vec![-1000, 0, 1000, 2000]);
        // The presentation instants are unchanged (the absolute timeline), so
        // the composition offsets (`pts - dts`) are 1000, 3000, 0, 0 — the
        // latch the reorder required (the two frames held back have no
        // offset).
        assert_eq!(shifted, alloc::vec![0, 3000, 1000, 2000]);

        // A pyramid GOP: 0, 4, 2, 1, 3, 8, 6, 5, 7 (x1000). The period is
        // 1000 — the smallest *positive* step here is 2000, so a "smallest
        // step" period would be wrong; the gcd of the steps is 1000.
        let pts = [0i64, 4000, 2000, 1000, 3000, 8000, 6000, 5000, 7000];
        let (dts, _) = decode_timeline(&pts);
        // The instants are zero-based here (the series starts at 0) and the
        // latch is one period, so the decode timeline runs one period ahead of
        // them.
        assert_eq!(
            dts,
            alloc::vec![-2000, -1000, 0, 1000, 2000, 3000, 4000, 5000, 6000],
            "a pyramid group must decode one period apart, in wire order"
        );
    }

    #[test]
    fn decode_timeline_properties_hold_for_reordered_series() {
        for pts in [
            alloc::vec![0i64, 3000, 1000, 2000],
            alloc::vec![0i64, 4000, 2000, 1000, 3000],
            alloc::vec![0i64, 4000, 2000, 1000, 3000, 8000, 6000, 5000, 7000],
            alloc::vec![100i64, 700, 400, 500],
        ] {
            let (dts, shifted) = decode_timeline(&pts);
            assert_eq!(dts.len(), pts.len());
            assert!(
                dts[0] >= pts[0] - 10_000,
                "the timeline stays on the wire's own epoch: {dts:?}"
            );
            assert!(
                dts.windows(2).all(|w| w[1] >= w[0]),
                "dts must be non-decreasing: {dts:?}"
            );
            assert!(
                dts.iter().zip(&shifted).all(|(d, p)| d <= p),
                "dts must never run ahead of presentation: {dts:?} vs {shifted:?}"
            );
            assert!(
                shifted.iter().all(|p| *p >= 0),
                "presentation instants must stay non-negative: {shifted:?}"
            );
        }
    }

    #[test]
    fn decode_timeline_never_advances_a_hostile_reorder() {
        // A hostile sequence whose first presentation instant is already past
        // the grid position of a later one: `pts[1] = 0` while the grid wants
        // 1000. The clamp gives that frame `dts = pts` (a zero offset) rather
        // than `pts < dts`, which no container can express, and the sequence
        // stays non-decreasing.
        let pts = [1000i64, 0, 500];
        let (dts, shifted) = decode_timeline(&pts);
        assert!(
            dts.windows(2).all(|w| w[1] >= w[0]),
            "must stay non-decreasing: {dts:?}"
        );
        assert!(
            dts.iter().zip(&shifted).all(|(d, p)| d <= p),
            "must stay at or behind presentation: {dts:?} vs {shifted:?}"
        );
    }

    #[test]
    fn decode_timeline_edge_cases_are_total() {
        let (dts, pts) = decode_timeline(&[]);
        assert!(dts.is_empty() && pts.is_empty());
        assert_eq!(
            decode_timeline(&[1234]),
            (alloc::vec![1234], alloc::vec![1234])
        );
        // All-equal instants: no positive step, so no grid — `pts` is already
        // non-decreasing and is returned as-is.
        let (dts, pts) = decode_timeline(&[7, 7, 7]);
        assert_eq!(dts, alloc::vec![7, 7, 7]);
        assert_eq!(pts, alloc::vec![7, 7, 7]);
    }

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
