//! RTP de/packetisation + SDP — RFC 3550 / RFC 6184 / RFC 3640 / RFC 4566.
//!
//! The RTP spoke of the any-to-any container hub: it packetises a live sample
//! stream into RTP packets ([`RtpPacketiserStage`] : [`Stage`]) and, through
//! [`crate::rtp_stream::RtpStreamDepacketiser`], depacketises RTP packets back
//! to the IR, for H.264/AVC video and AAC (`AAC-hbr`) audio, plus SDP
//! (`m=`/`a=rtpmap`/`a=fmtp`) generation ([`sdp_media`]).
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
//!   the display process"). So [`RtpPacketiserStage`] stamps each access unit with
//!   the sample's **`pts`**, and the depacketisers read it back into `pts` —
//!   a stream with B-frame reordering must not be presented in decode order at
//!   the wrong instants. RTP carries no second timestamp, so a decode time is
//!   not recoverable from the wire: the streaming depacketiser keeps the
//!   documented low-delay model (`dts == pts`) and reports it through
//!   [`crate::rtp_stream::RtpLossEvent::NonMonotonicTimestamp`].
//! - **H.264** (RFC 6184): single-NAL packets (NAL type 1–23), STAP-A
//!   (type 24) aggregation for the SPS+PPS parameter sets, and FU-A (type 28)
//!   fragmentation of any NAL larger than the MTU. Video IR samples are 4-byte
//!   length-prefixed NALs ([`crate::annexb`]); the length prefixes are stripped
//!   on packetise and re-added on depacketise.
//! - **AAC** (RFC 3640, `AAC-hbr`): an AU-headers-length (16-bit, in bits)
//!   prefix + one 2-byte AU-header (`sizeLength=13; indexLength=3`) + the raw
//!   access unit.
//! - **SDP** (RFC 4566 + `fmtp`; the media section is built with `sdp-types`,
//!   so [`sdp_media`] is `std`-only): `sprop-parameter-sets` carries base64
//!   SPS,PPS for video; `config` carries the hex AudioSpecificConfig for audio.
//! - **KLV** (RFC 6597, `smpte336m`): a SMPTE ST 336 KLV unit ([`crate::klv`])
//!   carried directly after the fixed header — no payload header — fragmented
//!   across sequential packets sharing one timestamp, marker on the last
//!   ([`packetise_klv`] / [`depacketise_klv`]).
//!
//! See `private/specs-verified/mpeg/transmux/rtp/rtp-payload-formats.md` for the full transcription.
//!
//! The packetiser stage holds only per-track sequence/timestamp state and one
//! access unit's packets; each packet leaves as one [`MuxOutput::Bytes`], the
//! whole RTP datagram. The depacketise side
//! ([`crate::rtp_stream::RtpStreamDepacketiser`]) reassembles fragments by
//! concatenation and may reasonably copy (issue #777).
//!
//! This module does **not** validate sequence-number continuity —
//! [`crate::rtp_stream`]'s stateful
//! [`crate::rtp_stream::RtpStreamDepacketiser`] does that (loss/
//! reorder detection, issue #779); see its module docs and
//! `private/specs-verified/mpeg/transmux/rtp/rtp-sequence-validation.md`.
//!
//! `no_std` + `alloc`.

use crate::nal::NAL_TYPE_MASK;
use alloc::collections::VecDeque;
#[cfg(feature = "std")]
use alloc::format;
use alloc::vec::Vec;
use broadcast_common::error::{BufferTooShort, InvalidValue};
use broadcast_common::wire::u16_at;

use broadcast_common::stage::{Stage, Timestamp};
use broadcast_common::timescale::{Rounding, Timescale, rescale_u64};
use broadcast_common::{Parse, Serialize};
use bytes::Bytes;
use rtp_packet::RtpPacket as RtpPacketWire;

use crate::annexb::NAL_LENGTH_SIZE;
use crate::error::{Error, Result};
use crate::ir::{Sample, TrackSpec};
use crate::mux_stage::{MuxInput, MuxOutput};
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
/// First payload type of the dynamic range (RFC 3551 §6).
const DYNAMIC_PT_MIN: u8 = 96;
/// Last payload type of the dynamic range (RFC 3551 §6).
const DYNAMIC_PT_MAX: u8 = 127;
/// Default video RTP clock rate (RFC 6184 — H.264 is carried at 90 kHz).
pub const VIDEO_CLOCK_RATE: u32 = 90_000;
/// [`VIDEO_CLOCK_RATE`] as a [`Timescale`], for the IR timescale of a 90 kHz track.
pub const VIDEO_CLOCK_TIMESCALE: Timescale = broadcast_common::timescale!(VIDEO_CLOCK_RATE);
/// Default audio RTP clock rate when the
/// caller does not supply the stream's own (RFC 3551 §4.5 lists 48000 as a
/// registered audio clock rate; the negotiated rate belongs in the SDP's
/// `a=rtpmap`, RFC 3640 §4.1 for `mpeg4-generic`).
pub const DEFAULT_AAC_CLOCK_RATE: u32 = 48_000;

/// Default dynamic payload type for a KLV metadata stream (RFC 6597).
pub const DEFAULT_KLV_PT: u8 = 98;
/// RFC 6597 SDP encoding name for SMPTE ST 336 KLV.
pub const KLV_ENCODING_NAME: &str = "smpte336m";

// --- H.264 NAL / packetisation (RFC 6184 §5.2, §5.6, §5.7, §5.8) -----------

/// NAL unit `F|NRI` field mask (top 3 bits of the NAL octet).
const NAL_FNRI_MASK: u8 = 0xE0;
const NAL_NRI_MASK: u8 = 0x60;
const NAL_FORBIDDEN_MASK: u8 = 0x80;
const FU_HEADER_PAIR_LEN: usize = 2;
const PAIR_WINDOW_LEN: usize = 2;
const DEFAULT_SSRC: u32 = 0x1234_5678;
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
pub const AAC_SIZE_LENGTH: u32 = 13;
/// `indexLength` for AAC-hbr — AU-index field width in bits (RFC 3640 §3.3.6).
pub const AAC_INDEX_LENGTH: u32 = 3;
/// `indexDeltaLength` for AAC-hbr — AU-index-delta field width in bits.
#[cfg(feature = "std")]
pub const AAC_INDEX_DELTA_LENGTH: u32 = 3;
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
    /// Split a whole RTP datagram into its fixed header (the first 12 bytes,
    /// RFC 3550 §5.1) and the payload that follows, borrowing the buffer.
    /// `None` for a datagram shorter than the fixed header.
    pub fn from_contiguous(datagram: Bytes) -> Option<Self> {
        if datagram.len() < RTP_HEADER_LEN {
            return None;
        }
        let payload = datagram.slice(RTP_HEADER_LEN..);
        Some(Self {
            header: datagram.slice(..RTP_HEADER_LEN),
            payload,
        })
    }

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

// ---------------------------------------------------------------------------
// RtpPacketiserStage — Stage<In = MuxInput, Out = MuxOutput>
// ---------------------------------------------------------------------------

/// Dynamic payload types (RFC 3551 §6) are per-session bindings, so one stage
/// allocates them for every stream it is given, never reusing a type and never
/// handing out one this crate binds to a fixed encoding.
///
/// Two streams sharing one type produce an SDP that describes both as the same
/// encoding — a receiver decodes the second as the first. Before this, a
/// *third* AVC (or AAC) track got `video_pt + 2` because only "has a video
/// track been seen" was tracked, so tracks 2 and 3 collided, and `96 + 2 = 98`
/// additionally collided with [`DEFAULT_KLV_PT`].
#[derive(Debug, Clone)]
struct PtAllocator {
    /// The next candidate type. Advanced past every allocation, so no value is
    /// ever handed out twice.
    next: u8,
}

/// The per-track packetising state.
#[derive(Debug, Clone)]
struct RtpTrackState {
    spec: TrackSpec,
    kind: RtpMediaKind,
    pt: u8,
    seq: SeqCounter,
    /// The first sample's presentation time; the emitted RTP timestamp series
    /// starts at 0 for the first access unit regardless of where the source
    /// timeline sits (RFC 3550 §5.1 only constrains the increments).
    first_pts: Option<i64>,
    /// The running sum of preceding durations — the timestamp fallback for a
    /// sample with no `pts`.
    ticks: u64,
}

impl RtpTrackState {
    /// This sample's RTP timestamp, in `clock`.
    ///
    /// RFC 6184 §5.1 is explicit: "The RTP timestamp is set to the sampling
    /// timestamp of the content", and receivers "SHOULD use the RTP timestamp
    /// for synchronizing the display process". That is a *presentation* time,
    /// so a reordered stream (B-frames) must stamp each access unit with its
    /// **`pts`**, not its `dts`: stamping the decode time makes a receiver
    /// present in decode order at the wrong instants.
    fn timestamp(&mut self, sample: &Sample, clock: Timescale) -> u32 {
        let pts = match (self.first_pts, sample.timing.and_then(|t| t.pts())) {
            (None, Some(current)) => {
                self.first_pts = Some(current);
                0
            }
            (Some(first), Some(current)) => {
                u64::try_from(current.saturating_sub(first).max(0)).unwrap_or(0)
            }
            _ => self.ticks,
        };
        let duration = sample.timing.and_then(|t| t.duration()).unwrap_or(0);
        self.ticks = self.ticks.saturating_add(u64::from(duration));
        rescale_ts(pts, self.spec.timescale, clock)
    }
}

/// Packetise a live sample stream into RTP packets (RFC 3550), one
/// [`MuxOutput::Bytes`] per packet.
///
/// Per track: AVC → single-NAL / STAP-A (SPS+PPS) / FU-A packets on a 90 kHz
/// clock ([`VIDEO_CLOCK_TIMESCALE`]); AAC → `AAC-hbr` packets on the audio
/// sample-rate clock. All packets of one access unit share a timestamp and the
/// marker bit is set on the last. The stage holds one access unit's packets at
/// a time.
///
/// [`MuxInput::Track`] announcements assign the session's dynamic payload
/// types; the SDP media section for a track's format is
/// [`sdp_media`], rendered per track because the stage's packet output carries
/// the payload type, not the track.
#[derive(Debug, Clone)]
pub struct RtpPacketiserStage {
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
    tracks: Vec<RtpTrackState>,
    pts: PtAllocator,
    next_video_pt: Option<u8>,
    next_audio_pt: Option<u8>,
    out: VecDeque<MuxOutput>,
}

impl Default for RtpPacketiserStage {
    fn default() -> Self {
        Self {
            mtu: DEFAULT_MTU,
            video_pt: DEFAULT_VIDEO_PT,
            audio_pt: DEFAULT_AUDIO_PT,
            ssrc: DEFAULT_SSRC,
            stap_a_parameter_sets: true,
            tracks: Vec::new(),
            pts: PtAllocator::new(DEFAULT_VIDEO_PT.min(DEFAULT_AUDIO_PT)),
            next_video_pt: Some(DEFAULT_VIDEO_PT),
            next_audio_pt: Some(DEFAULT_AUDIO_PT),
            out: VecDeque::new(),
        }
    }
}

impl RtpPacketiserStage {
    /// Create a packetiser with default MTU / payload types / SSRC.
    pub fn new() -> Self {
        let mut stage = Self::default();
        stage.pts = PtAllocator::new(stage.video_pt.min(stage.audio_pt));
        stage.pts.reserve_through(stage.video_pt);
        stage.pts.reserve_through(stage.audio_pt);
        stage.next_video_pt = Some(stage.video_pt);
        stage.next_audio_pt = Some(stage.audio_pt);
        stage
    }

    /// Set the MTU (the payload budget) on a freshly built stage.
    #[must_use]
    pub fn with_mtu(mut self, mtu: usize) -> Self {
        self.mtu = mtu;
        self
    }

    /// Set the fixed SSRC on a freshly built stage.
    #[must_use]
    pub fn with_ssrc(mut self, ssrc: u32) -> Self {
        self.ssrc = ssrc;
        self
    }

    /// Set whether SPS+PPS are aggregated into a leading STAP-A packet.
    #[must_use]
    pub fn with_stap_a_parameter_sets(mut self, aggregate: bool) -> Self {
        self.stap_a_parameter_sets = aggregate;
        self
    }

    /// Set the video track's preferred dynamic payload type.
    #[must_use]
    pub fn with_video_pt(mut self, pt: u8) -> Self {
        self.video_pt = pt;
        self.next_video_pt = Some(pt);
        self.pts.reserve_through(pt);
        self
    }

    /// Set the audio track's preferred dynamic payload type.
    #[must_use]
    pub fn with_audio_pt(mut self, pt: u8) -> Self {
        self.audio_pt = pt;
        self.next_audio_pt = Some(pt);
        self.pts.reserve_through(pt);
        self
    }

    /// Announce one track, assigning its payload type.
    fn announce(&mut self, spec: TrackSpec) -> Result<()> {
        if self.tracks.iter().any(|t| t.spec.track_id == spec.track_id) {
            return Err(Error::InvalidInput("rtp: duplicate track id"));
        }
        let (kind, pt) = match &spec.config {
            CodecConfig::Avc { .. } => {
                // A caller-supplied base is used only if nothing has claimed
                // it; allocation then continues from there, so a third video
                // track gets its own type instead of colliding with the second.
                let pt = match self.next_video_pt.take() {
                    Some(base) => {
                        self.pts.reserve_through(base);
                        base
                    }
                    None => self.pts.allocate().ok_or(Error::InvalidInput(
                        "no dynamic RTP payload type left (RFC 3551 §6's range is 96-127): too many streams share this session",
                    ))?,
                };
                (RtpMediaKind::H264, pt)
            }
            CodecConfig::Aac { .. } => {
                let pt = match self.next_audio_pt.take() {
                    Some(base) => {
                        self.pts.reserve_through(base);
                        base
                    }
                    None => self.pts.allocate().ok_or(Error::InvalidInput(
                        "no dynamic RTP payload type left (RFC 3551 §6's range is 96-127): too many streams share this session",
                    ))?,
                };
                (RtpMediaKind::Aac, pt)
            }
            _ => {
                return Err(Error::InvalidInput(
                    "RTP packetiser supports only AVC video and AAC audio tracks",
                ));
            }
        };
        let mut seq = SeqCounter::new(0);
        // Optional leading STAP-A carrying SPS+PPS (parameter sets).
        // STAP-A aggregation interleaves header bytes with payload — built in
        // a `BytesMut` (which copies); the parameter sets are small (typically
        // <1 kB total).
        let stap = if self.stap_a_parameter_sets
            && let CodecConfig::Avc { config, .. } = &spec.config
        {
            stap_a_packet(config, pt, self.ssrc, &mut seq, 0)?
        } else {
            None
        };
        self.tracks.push(RtpTrackState {
            spec,
            kind,
            pt,
            seq,
            first_pts: None,
            ticks: 0,
        });
        if let Some(packet) = stap {
            self.out.push_back(MuxOutput::Bytes(packet.as_contiguous()));
        }
        Ok(())
    }

    /// Packetise one AVC access unit.
    fn video_packets(
        &mut self,
        index: usize,
        sample: &Sample,
        timestamp: u32,
    ) -> Result<Vec<RtpPacket>> {
        let pt = self.tracks[index].pt;
        let mut packets = Vec::new();
        push_video_nals(
            &sample.data,
            pt,
            self.mtu,
            self.ssrc,
            &mut self.tracks[index].seq,
            timestamp,
            &mut packets,
        )?;
        Ok(packets)
    }

    /// Packetise one AAC access unit (`AAC-hbr`).
    ///
    /// The AAC-hbr payload header (AU-headers-length + AU-header) is
    /// interleaved with the audio access unit, so the full packet is built in a
    /// `BytesMut` (which copies). The header is small (4 bytes) and audio AUs
    /// are typically <1 kB.
    ///
    /// An access unit that does not fit the payload budget is fragmented over
    /// consecutive packets per RFC 3640 §3.2.3.1: each fragment carries its own
    /// AU-header with the AU's **full** size (§3.2.3.2 — "the AU size indicates
    /// the size of the entire AU and not the size of the fragment"), they all
    /// share one RTP timestamp, and the marker bit is set on the last one only
    /// (§3.1).
    fn audio_packets(&mut self, index: usize, sample: &Sample) -> Result<Vec<RtpPacket>> {
        let pt = self.tracks[index].pt;
        let sample_data = &sample.data;
        let per_packet = self
            .mtu
            .checked_sub(RTP_HEADER_LEN + AAC_AU_HEADERS_LENGTH_LEN + AAC_AU_HEADER_LEN)
            .filter(|&b| b > 0)
            .ok_or(Error::InvalidInput(
                "MTU too small for an AAC-hbr packet header",
            ))?;
        let au = sample_data;
        if au.len() >= (1usize << AAC_SIZE_LENGTH) {
            return Err(Error::InvalidValue(InvalidValue {
                field: "aac_au_size",
                value: au.len() as u64,
                reason: "exceeds 13-bit AAC-hbr AU-size field",
            }));
        }
        let clock = self.tracks[index].spec.timescale;
        let timestamp = self.tracks[index].timestamp(sample, clock);
        let mut packets = Vec::new();
        let au_headers_len_bits = (AAC_AU_HEADER_LEN * u8::BITS as usize) as u16;
        let hdr = (au.len() as u16) << AAC_INDEX_LENGTH;
        let num_frags = au.len().div_ceil(per_packet).max(1);
        for f in 0..num_frags {
            let start = f * per_packet;
            let end = (start + per_packet).min(au.len());
            let is_last = f == num_frags - 1;
            let seq = self.tracks[index].seq.next();
            let rtp_hdr = rtp_header(pt, is_last, seq, timestamp, self.ssrc);
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
        Ok(packets)
    }
}

impl Stage for RtpPacketiserStage {
    type In<'a> = MuxInput;
    type Out = MuxOutput;
    type Error = Error;

    fn feed(&mut self, input: MuxInput, _now: Timestamp) -> Result<()> {
        let (track, sample) = match input {
            MuxInput::Track(spec) => return self.announce(*spec),
            MuxInput::Sample { track, sample } => (track, sample),
            MuxInput::Discontinuity { .. } | MuxInput::Flush => return Ok(()),
        };
        let Some(index) = self.tracks.iter().position(|t| t.spec.track_id == track) else {
            return Err(Error::InvalidInput("rtp: sample for an unannounced track"));
        };
        let packets = match self.tracks[index].kind {
            RtpMediaKind::H264 => {
                let timestamp = self.tracks[index].timestamp(&sample, VIDEO_CLOCK_TIMESCALE);
                self.video_packets(index, &sample, timestamp)?
            }
            RtpMediaKind::Aac => self.audio_packets(index, &sample)?,
        };
        for packet in &packets {
            self.out.push_back(MuxOutput::Bytes(packet.as_contiguous()));
        }
        Ok(())
    }

    fn poll(&mut self) -> Option<MuxOutput> {
        self.out.pop_front()
    }

    fn finish(&mut self) -> Result<()> {
        if self.tracks.is_empty() && self.out.is_empty() {
            return Err(Error::InvalidInput(
                "no AVC/AAC tracks to packetise into RTP",
            ));
        }
        Ok(())
    }
}

/// Append one AVC access unit's packets to `packets`: a single-NAL packet
/// (zero-copy `Bytes::slice` of the sample) or an FU-A fragmentation (RFC
/// 6184 §5.8) of a NAL larger than `mtu`.
fn push_video_nals(
    sample_data: &Bytes,
    pt: u8,
    mtu: usize,
    ssrc: u32,
    seq: &mut SeqCounter,
    timestamp: u32,
    packets: &mut Vec<RtpPacket>,
) -> Result<()> {
    let nals = split_length_prefixed(sample_data)?;
    if nals.is_empty() {
        return Ok(());
    }
    let last_nal = nals.len() - 1;
    for (n, nal) in nals.iter().enumerate() {
        let is_last_nal = n == last_nal;
        if nal.len() + RTP_HEADER_LEN <= mtu {
            let header = rtp_header(pt, is_last_nal, seq.next(), timestamp, ssrc);
            let nal_offset = nal.as_ptr() as usize - sample_data.as_ptr() as usize;
            let payload = sample_data.slice(nal_offset..nal_offset + nal.len());
            packets.push(RtpPacket { header, payload });
        } else {
            fragment_fu_a(
                nal,
                sample_data,
                pt,
                is_last_nal,
                mtu,
                seq,
                timestamp,
                ssrc,
                packets,
            )?;
        }
    }
    Ok(())
}

/// The optional leading STAP-A packet aggregating an AVC track's SPS+PPS
/// parameter sets (RFC 6184 §5.7.1), or `None` when the record carries none.
fn stap_a_packet(
    config: &isobmff::avc_config::AVCConfigurationBox,
    pt: u8,
    ssrc: u32,
    seq: &mut SeqCounter,
    timestamp: u32,
) -> Result<Option<RtpPacket>> {
    let mut param_nals: Vec<Vec<u8>> = Vec::new();
    for sps in &config.config.sps {
        param_nals.push(sps.0.clone());
    }
    for pps in &config.config.pps {
        param_nals.push(pps.0.clone());
    }
    if param_nals.is_empty() {
        return Ok(None);
    }
    build_stap_a(pt, &param_nals, seq, timestamp, ssrc).map(Some)
}

/// Packetise **one** AVC sample into RTP packets, as a self-contained packet
/// group on a fresh sequence counter with RTP timestamp 0.
///
/// This is the per-sample form an egress that already tracks its own sequence
/// numbers and timestamps needs (a WebRTC viewer session patches both fields
/// onto the packet it actually sends); [`RtpPacketiserStage`] is the stateful
/// form that numbers a whole stream itself. `stap_a` leads the group with a
/// fresh STAP-A of the track's parameter sets, so a viewer joining at this
/// sample can resynchronise its decoder.
///
/// # Errors
///
/// [`Error::InvalidInput`] for a non-AVC track, and whatever the NAL split
/// reports for damaged sample data.
pub fn packetise_video_sample(
    spec: &TrackSpec,
    sample: &Sample,
    mtu: usize,
    pt: u8,
    ssrc: u32,
    stap_a: bool,
) -> Result<Vec<RtpPacket>> {
    let CodecConfig::Avc { config, .. } = &spec.config else {
        return Err(Error::InvalidInput(
            "packetise_video_sample takes an AVC track",
        ));
    };
    let mut seq = SeqCounter::new(0);
    let mut packets = Vec::new();
    if stap_a && let Some(packet) = stap_a_packet(config, pt, ssrc, &mut seq, 0)? {
        packets.push(packet);
    }
    push_video_nals(&sample.data, pt, mtu, ssrc, &mut seq, 0, &mut packets)?;
    Ok(packets)
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
        sequence_number: seq.into(),
        timestamp,
        ssrc: ssrc.into(),
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

/// Per-stream monotonic sequence-number counter (wraps at 16 bits).
#[derive(Debug, Clone)]
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

/// Rescale a tick count from `from` to `to` timescale (round to nearest).
///
/// RTP timestamps are modulo 2^32 (RFC 3550 §5.1), so the result wraps
/// into the 32-bit range rather than saturating or erroring.
pub fn rescale_ts(
    ticks: u64,
    from: broadcast_common::timescale::Timescale,
    to: broadcast_common::timescale::Timescale,
) -> u32 {
    let v = rescale_u64(ticks, from, to, Rounding::Nearest);
    (v & u64::from(u32::MAX)) as u32
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
            max_nri = max_nri.max(octet & NAL_NRI_MASK);
            forbidden |= octet & NAL_FORBIDDEN_MASK;
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
            return Err(Error::InvalidValue(InvalidValue {
                field: "stap_a_nal_size",
                value: nal.len() as u64,
                reason: "exceeds 16-bit STAP-A size prefix",
            }));
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
        .checked_sub(RTP_HEADER_LEN + FU_HEADER_PAIR_LEN)
        .filter(|&b| b > 0)
        .ok_or(Error::InvalidInput("MTU too small for FU-A fragmentation"))?;

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
        let rtp_hdr = rtp_header(pt, marker, seq.next(), timestamp, ssrc);
        let mut header_buf = bytes::BytesMut::with_capacity(rtp_hdr.len() + FU_HEADER_PAIR_LEN);
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
#[cfg(feature = "std")]
fn asc_bytes(esds: &isobmff::mp4esds::EsdsBox) -> Result<&[u8]> {
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

/// The SDP media section (`m=` with its `a=rtpmap`/`a=fmtp`) describing one
/// AVC video (RFC 6184 §8.1) or AAC (RFC 3640 §4.1) track's RTP payload
/// format, bound to payload type `pt`.
///
/// The packetiser stage emits whole RTP datagrams, which carry the payload
/// type but not the track, so a caller that needs a session description asks
/// for each track's section here with the same payload type it configured (or
/// read back off that track's first packet). Session-level lines are not
/// transmux's: build the session with
/// `rtsp_runtime::sdp::session_description` over these. `std` only.
///
/// # Errors
///
/// [`Error::InvalidInput`] for a codec RTP does not packetise, and for an
/// `esds` without a DecoderSpecificInfo (the AAC-hbr `config`).
#[cfg(feature = "std")]
pub fn sdp_media(spec: &TrackSpec, pt: u8) -> Result<sdp_types::Media> {
    match &spec.config {
        CodecConfig::Avc { config, .. } => Ok(sdp_video(pt, &config.config)),
        CodecConfig::Aac {
            esds,
            channel_count,
            ..
        } => {
            let asc = asc_bytes(esds)?;
            Ok(sdp_audio(pt, spec.timescale.get(), *channel_count, asc))
        }
        _ => Err(Error::InvalidInput(
            "only AVC video and AAC audio have an RTP payload format in this crate",
        )),
    }
}

/// SDP media section for an H.264 video stream (RFC 6184 §8.1).
#[cfg(feature = "std")]
fn sdp_video(
    pt: u8,
    config: &isobmff::avc_config::AVCDecoderConfigurationRecord,
) -> sdp_types::Media {
    let profile_level_id = format!(
        "{:02X}{:02X}{:02X}",
        config.profile_indication, config.profile_compatibility, config.level_indication
    );
    let sprop = config
        .sps
        .iter()
        .map(|n| broadcast_common::base64::base64_encode(&n.0))
        .chain(
            config
                .pps
                .iter()
                .map(|n| broadcast_common::base64::base64_encode(&n.0)),
        )
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
            "{pt} streamtype=5; profile-level-id=1; mode=AAC-hbr; config={config}; sizeLength={AAC_SIZE_LENGTH}; indexLength={AAC_INDEX_LENGTH}; indexDeltaLength={AAC_INDEX_DELTA_LENGTH}"
        ),
    );
    media
}

/// The RTP timestamp modulus: the header field is 32 bits (RFC 3550 §5.1), so
/// the media clock wraps every `2^32` ticks (at the 90 kHz video clock, ≈ 13.3
/// hours).
#[cfg(test)]
const RTP_TS_WRAP: i64 = 1 << 32;
/// Half the 32-bit range — the classic wrap-detection threshold: a step of
/// more than half the range is read as a wrap, not a real jump.
#[cfg(test)]
const RTP_TS_WRAP_MIDPOINT: i64 = RTP_TS_WRAP / 2;

/// Incremental 32-bit RTP-timestamp wrap-unroll (RFC 3550 §5.1), one access
/// unit at a time — the RTP analogue of `ts_demux`'s 33-bit `WrapState`, and
/// the **only** place this crate unwraps an RTP clock (media plane step 2c:
/// unwrapped once, at the demux edge, never re-derived downstream).
///
/// The delta is computed on the wrapped clock and applied to an unwrapped
/// accumulator, so ordinary small backward steps (B-frame reordering) survive
/// and only a near-full-range jump is treated as a wrap.
#[derive(Default)]
#[cfg(test)]
struct RtpWrapState {
    initialized: bool,
    prev_raw: u32,
    prev_uw: i64,
}

#[cfg(test)]
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
        if delta > RTP_TS_WRAP_MIDPOINT {
            delta -= RTP_TS_WRAP; // wrapped backward across 2^32
        } else if delta < -RTP_TS_WRAP_MIDPOINT {
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
/// `samples[0].timing.and_then(|t| t.dts())`, and both series shift together, so every delta, the
/// ordering, and `dts <= pts` are preserved exactly.
///
/// Returns `(dts, pts)` per sample, in wire order.
#[cfg(test)]
fn decode_timeline(pts: &[i64]) -> (Vec<i64>, Vec<i64>) {
    if pts.windows(PAIR_WINDOW_LEN).all(|w| w[1] >= w[0]) {
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
    // choice made consistently; a consumer wanting a zero origin feeds the
    // samples through `crate::rebase::RebaseStage`, which shifts every track
    // in lockstep.
    let mut dts: Vec<i64> = Vec::with_capacity(pts.len());
    let mut shifted_pts: Vec<i64> = Vec::with_capacity(pts.len());
    for (s, p) in sorted.iter().zip(pts.iter()) {
        let d = *s - latch;
        dts.push(d);
        shifted_pts.push((*p).max(d));
    }
    (dts, shifted_pts)
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
    reassemble_video_impl(packets)
}

fn reassemble_video_impl(packets: &[Vec<u8>]) -> Result<Vec<ReassembledAu>> {
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
        macro_rules! bad {
            ($e:expr) => {{
                return Err($e);
            }};
        }
        let hdr = match parse_rtp_header(pkt) {
            Ok(h) => h,
            Err(e) => bad!(e),
        };
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
                // Collected apart so a malformed aggregate contributes none
                // of its NALs (a half-parsed STAP-A would be a torn AU).
                let mut stap_nals: Vec<Vec<u8>> = Vec::new();
                while off < payload.len() {
                    if off + STAP_A_SIZE_LEN > payload.len() {
                        bad!(Error::BufferTooShort(BufferTooShort {
                            need: off + STAP_A_SIZE_LEN,
                            have: payload.len(),
                            what: "STAP-A size prefix",
                        }));
                    }
                    let size = u16_at(payload, off) as usize;
                    off += STAP_A_SIZE_LEN;
                    let end = off + size;
                    if end > payload.len() {
                        bad!(Error::BufferTooShort(BufferTooShort {
                            need: end,
                            have: payload.len(),
                            what: "STAP-A NAL",
                        }));
                    }
                    stap_nals.push(payload[off..end].to_vec());
                    off = end;
                }
                cur_nals.extend(stap_nals);
            }
            NAL_TYPE_FU_A => {
                if payload.len() < FU_HEADER_PAIR_LEN {
                    bad!(Error::BufferTooShort(BufferTooShort {
                        need: FU_HEADER_PAIR_LEN,
                        have: payload.len(),
                        what: "FU-A header",
                    }));
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
    reassemble_audio_impl(packets)
}

fn reassemble_audio_impl(packets: &[Vec<u8>]) -> Result<Vec<ReassembledAu>> {
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
        macro_rules! bad {
            ($e:expr) => {{
                return Err($e);
            }};
        }
        let hdr = match parse_rtp_header(pkt) {
            Ok(h) => h,
            Err(e) => bad!(e),
        };
        let payload = hdr.payload;
        if payload.len() < AAC_AU_HEADERS_LENGTH_LEN {
            bad!(Error::BufferTooShort(BufferTooShort {
                need: AAC_AU_HEADERS_LENGTH_LEN,
                have: payload.len(),
                what: "AAC AU-headers-length",
            }));
        }
        let au_headers_len_bits = u16_at(payload, 0) as usize;
        let header_bytes = au_headers_len_bits.div_ceil(u8::BITS as usize);
        let num_headers = au_headers_len_bits / (AAC_AU_HEADER_LEN * u8::BITS as usize);
        let mut off = AAC_AU_HEADERS_LENGTH_LEN;
        if off + header_bytes > payload.len() {
            bad!(Error::BufferTooShort(BufferTooShort {
                need: off + header_bytes,
                have: payload.len(),
                what: "AAC AU headers",
            }));
        }
        // A packet carries complete AUs or a single fragment of one (§3.2.3),
        // so more than one header means each header is a whole AU.
        let mut sizes = Vec::with_capacity(num_headers);
        for h in 0..num_headers {
            let hoff = off + h * AAC_AU_HEADER_LEN;
            let ah = u16_at(payload, hoff);
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

/// 4-byte length-prefix a list of NALs into an IR video sample.
fn length_prefix_nals(nals: &[Vec<u8>]) -> Vec<u8> {
    let total: usize = nals.iter().map(|n| NAL_LENGTH_SIZE + n.len()).sum();
    crate::annexb::length_prefix(total, nals.iter().map(Vec::as_slice))
}

/// A parsed RTP fixed header (RFC 3550 §5.1) — the fields the spoke needs.
/// Delegates the wire decode to [`rtp_packet::RtpPacket`]; transmux only ever
/// depacketises the simple `P=0 X=0 CC=0` case it itself emits, so only
/// `marker`/`sequence`/`timestamp`/`ssrc`/`payload` are read at call sites —
/// see #646. `sequence`/`ssrc` were dead until issue #779 gave
/// [`crate::rtp_stream::RtpStreamDepacketiser`] a reason to read them (loss
/// and reorder detection).
#[derive(Debug, Clone, Copy)]
pub(crate) struct RtpHeader<'a> {
    pub(crate) marker: bool,
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
        sequence: parsed.sequence_number.get(),
        timestamp: parsed.timestamp,
        ssrc: parsed.ssrc.get(),
        payload: parsed.payload,
    })
}

/// Map an [`rtp_packet::Error`] onto this crate's [`Error`].
fn map_rtp_error(e: rtp_packet::Error) -> Error {
    match e {
        rtp_packet::Error::BufferTooShort(e) => Error::BufferTooShort(e),
        rtp_packet::Error::UnsupportedVersion(e) => Error::InvalidValue(InvalidValue {
            field: "rtp_version",
            value: u64::from(e.found),
            reason: "must be 2",
        }),
        rtp_packet::Error::InvalidValue(e) => Error::InvalidValue(e),
        rtp_packet::Error::InvalidPadding { count, reason } => Error::InvalidValue(InvalidValue {
            field: "rtp_padding",
            value: u64::from(count),
            reason,
        }),
        rtp_packet::Error::ExtensionNotWordAligned { data_len } => {
            Error::InvalidValue(InvalidValue {
                field: "rtp_extension_length",
                value: data_len as u64,
                reason: "extension data length is not a multiple of 4 bytes",
            })
        }
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
// MPEG-2 TS over RTP (RFC 2250 §2) — MP2T
// ---------------------------------------------------------------------------

/// RFC 3551 §6 static payload type for a MPEG-2 transport stream (RFC 2250
/// §2 binds MP2T to it).
pub const MP2T_PT: u8 = 33;

/// RFC 2250 §2: a MP2T packet carries "no more than seven" whole TS packets.
pub const MP2T_TS_PACKETS_PER_PACKET: usize = 7;

/// RFC 2250 §2's fixed RTP clock rate for MP2T.
pub const MP2T_CLOCK_RATE: u32 = 90_000;

/// The largest payload one MP2T RTP packet may carry
/// ([`MP2T_TS_PACKETS_PER_PACKET`] whole TS packets).
pub const MP2T_MAX_PAYLOAD: usize = mpeg_ts::ts::TS_PACKET_SIZE * MP2T_TS_PACKETS_PER_PACKET;

/// Packetise a run of whole MPEG-2 TS packets into MP2T RTP packets
/// (RFC 2250 §2).
///
/// Each packet carries up to [`MP2T_TS_PACKETS_PER_PACKET`] whole TS packets
/// at the caller's 90 kHz timestamp, which is the same for every packet of
/// one run: RFC 2250 §2 has the timestamp advance with the media clock, which
/// is the caller's (an RTSP push reads its own `Instant`) and never this
/// crate's. Sequence numbers advance from [`initial_seq`](Self::new) and wrap
/// at 16 bits; the SSRC is fixed per packetiser.
///
/// A `ts_packets` slice that is not a whole number of TS packets is an error
/// rather than being split across packets: RFC 2250 §2 permits only whole TS
/// packets as the payload, so a split one would be undecodable.
#[derive(Debug, Clone)]
pub struct Mp2tPacketiser {
    ssrc: u32,
    seq: u16,
}

impl Mp2tPacketiser {
    /// Start sending from `initial_seq` under `ssrc` (RFC 3550 §5.1).
    #[must_use]
    pub fn new(ssrc: u32, initial_seq: u16) -> Self {
        Self {
            ssrc,
            seq: initial_seq,
        }
    }

    /// The sequence number the next packet will carry.
    #[must_use]
    pub fn next_seq(&self) -> u16 {
        self.seq
    }

    /// The serialized RTP packets carrying `ts_packets` at `timestamp`.
    pub fn packetise(&mut self, ts_packets: &[u8], timestamp: u32) -> Result<Vec<Vec<u8>>> {
        if !ts_packets.len().is_multiple_of(mpeg_ts::ts::TS_PACKET_SIZE) {
            return Err(Error::InvalidInput(
                "MP2T payload is not a whole number of 188-byte TS packets",
            ));
        }
        let count = ts_packets.len().div_ceil(MP2T_MAX_PAYLOAD);
        let mut packets = Vec::with_capacity(count);
        for payload in ts_packets.chunks(MP2T_MAX_PAYLOAD) {
            let wire = RtpPacketWire {
                marker: false,
                payload_type: MP2T_PT,
                sequence_number: self.seq.into(),
                timestamp,
                ssrc: self.ssrc.into(),
                csrc: Vec::new(),
                extension: None,
                padding: None,
                payload,
            };
            let mut out = alloc::vec![0u8; wire.serialized_len()];
            wire.serialize_into(&mut out)
                .map_err(|_| Error::InvalidInput("RTP packet did not fit its own length"))?;
            packets.push(out);
            self.seq = self.seq.wrapping_add(1);
        }
        Ok(packets)
    }
}

// ---------------------------------------------------------------------------
// base64 + hex (RFC 4648) via the `base64` and `hex` crates
// ---------------------------------------------------------------------------

/// Base64-decode a string (RFC 4648 §4); padding is optional and non-canonical
/// trailing bits are tolerated; any other invalid input is an error.
pub(crate) fn base64_decode(s: &str) -> Result<Vec<u8>> {
    broadcast_common::base64::base64_decode_lenient(s).map_err(|e| match e {
        broadcast_common::base64::Base64DecodeError::InvalidByte { byte, .. } => {
            Error::InvalidValue(InvalidValue {
                field: "base64",
                value: u64::from(byte),
                reason: "not a base64 character",
            })
        }
        _ => Error::InvalidValue(InvalidValue {
            field: "base64",
            value: s.len() as u64,
            reason: "invalid base64 length",
        }),
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
pub(crate) fn hex_decode(s: &str) -> Result<Vec<u8>> {
    broadcast_common::hex::hex_decode(s).ok_or_else(|| {
        if !s.len().is_multiple_of(PAIR_WINDOW_LEN) {
            return Error::InvalidValue(InvalidValue {
                field: "hex",
                value: s.len() as u64,
                reason: "odd-length hex string",
            });
        }
        match s.bytes().find(|b| !b.is_ascii_hexdigit()) {
            Some(b) => Error::InvalidValue(InvalidValue {
                field: "hex",
                value: u64::from(b),
                reason: "not a hex digit",
            }),
            None => Error::InvalidValue(InvalidValue {
                field: "hex",
                value: s.len() as u64,
                reason: "invalid hex string length",
            }),
        }
    })
}

#[cfg(test)]
fn test_placeholder_avc_config() -> CodecConfig {
    use isobmff::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
    CodecConfig::Avc {
        config: AVCConfigurationBox::new(AVCDecoderConfigurationRecord {
            configuration_version: 1,
            profile_indication: 0,
            profile_compatibility: 0,
            level_indication: 0,
            length_size_minus_one: 3,
            sps: alloc::vec::Vec::new(),
            pps: alloc::vec::Vec::new(),
            chroma_format: None,
            bit_depth_luma_minus8: None,
            bit_depth_chroma_minus8: None,
            sps_ext: alloc::vec::Vec::new(),
        }),
        width: 0,
        height: 0,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn hex_decode_reports_each_failure_class_as_before() {
        assert_eq!(hex_decode("00Ff").unwrap(), [0x00, 0xff]);
        let reason = |s: &str| match hex_decode(s) {
            Err(Error::InvalidValue(v)) => (v.value, v.reason),
            other => panic!("{other:?}"),
        };
        assert_eq!(reason("abc"), (3, "odd-length hex string"));
        assert_eq!(reason("0g"), (u64::from(b'g'), "not a hex digit"));
    }

    use super::*;

    /// `n` whole TS packets, each led by its own sync byte and numbered so a
    /// reordered or split packet shows up.
    fn ts_packets(n: usize) -> alloc::vec::Vec<u8> {
        let mut out = alloc::vec![0u8; n * mpeg_ts::ts::TS_PACKET_SIZE];
        for (i, packet) in out.chunks_mut(mpeg_ts::ts::TS_PACKET_SIZE).enumerate() {
            packet[0] = 0x47;
            packet[1] = u8::try_from(i).unwrap_or(0);
        }
        out
    }

    #[test]
    fn pt_allocator_hands_out_127_as_the_last_type_and_then_exhausts() {
        let mut allocator = PtAllocator::new(DYNAMIC_PT_MIN);
        let mut pts = alloc::vec::Vec::new();
        for _ in 0..40 {
            match allocator.allocate() {
                Some(pt) => pts.push(pt),
                None => break,
            }
        }
        assert_eq!(pts.len(), 31, "96..=127 minus the bound KLV type");
        assert_eq!(pts[0], 96);
        assert_eq!(pts.last(), Some(&127), "127 is still allocatable");
        assert!(!pts.contains(&DEFAULT_KLV_PT));
        assert!(pts.iter().all(|&pt| (96..=127).contains(&pt)));

        let mut allocator = PtAllocator::new(DYNAMIC_PT_MIN);
        for _ in 0..31 {
            assert!(allocator.allocate().is_some());
        }
        assert_eq!(allocator.allocate(), None);
    }

    #[test]
    fn mp2t_packs_seven_ts_packets_into_one_rtp_packet() {
        let mut packetiser = Mp2tPacketiser::new(0xDEAD_BEEF, 0x1234);
        let ts = ts_packets(MP2T_TS_PACKETS_PER_PACKET);
        let packets = packetiser.packetise(&ts, 0x0100_0000).expect("packetise");
        assert_eq!(packets.len(), 1);
        let packet = RtpPacketWire::parse(&packets[0]).expect("parse");
        assert_eq!(packet.payload_type, MP2T_PT);
        assert!(!packet.marker, "RFC 2250 §2 has no marker bit");
        assert_eq!(packet.sequence_number.get(), 0x1234);
        assert_eq!(packet.timestamp, 0x0100_0000);
        assert_eq!(packet.ssrc.get(), 0xDEAD_BEEF);
        assert_eq!(packet.payload, &ts[..]);
        assert_eq!(packetiser.next_seq(), 0x1235);
    }

    #[test]
    fn mp2t_payload_type_is_the_static_mp2t_one() {
        assert_eq!(MP2T_PT, 33, "RFC 3551 §6 static MP2T payload type");
    }

    #[test]
    fn mp2t_splits_eight_ts_packets_into_two_rtp_packets_in_order() {
        let mut packetiser = Mp2tPacketiser::new(0xDEAD_BEEF, 0);
        let ts = ts_packets(MP2T_TS_PACKETS_PER_PACKET + 1);
        let packets = packetiser.packetise(&ts, 9000).expect("packetise");
        assert_eq!(packets.len(), 2);
        let first = RtpPacketWire::parse(&packets[0]).expect("parse");
        let second = RtpPacketWire::parse(&packets[1]).expect("parse");
        assert_eq!(
            first.payload.len(),
            MP2T_MAX_PAYLOAD,
            "a full packet carries the maximum"
        );
        assert_eq!(
            second.payload.len(),
            mpeg_ts::ts::TS_PACKET_SIZE,
            "the remainder is one whole TS packet"
        );
        let mut reassembled = first.payload.to_vec();
        reassembled.extend_from_slice(second.payload);
        assert_eq!(reassembled, ts);
        assert_eq!(
            (first.sequence_number.get(), second.sequence_number.get()),
            (0, 1)
        );
        assert_eq!(first.timestamp, second.timestamp);
    }

    #[test]
    fn mp2t_rejects_a_partial_ts_packet() {
        let mut packetiser = Mp2tPacketiser::new(1, 0);
        let err = packetiser
            .packetise(&ts_packets(1)[..100], 0)
            .expect_err("a partial TS packet must be an error");
        assert!(matches!(err, Error::InvalidInput(_)), "{err:?}");
        assert_eq!(packetiser.next_seq(), 0, "nothing may be consumed");
    }

    #[test]
    fn mp2t_sequence_number_wraps_at_sixteen_bits() {
        let mut packetiser = Mp2tPacketiser::new(1, u16::MAX);
        let packets = packetiser
            .packetise(&ts_packets(MP2T_TS_PACKETS_PER_PACKET + 1), 0)
            .expect("packetise");
        let first = RtpPacketWire::parse(&packets[0]).expect("parse");
        let second = RtpPacketWire::parse(&packets[1]).expect("parse");
        assert_eq!(
            (first.sequence_number.get(), second.sequence_number.get()),
            (u16::MAX, 0)
        );
        assert_eq!(packetiser.next_seq(), 1, "wraps, never panics");
    }

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
        let track_id = crate::TrackId::new(1);
        let mut depack = crate::rtp_stream::RtpStreamDepacketiser::new(alloc::vec![
            crate::rtp_stream::RtpStreamTrack::new(
                track_id,
                RtpMediaKind::H264,
                test_placeholder_avc_config(),
                VIDEO_CLOCK_TIMESCALE,
            ),
        ]);
        let mut samples = alloc::vec::Vec::new();
        for packet in &packets {
            samples.extend(depack.push(track_id, packet).expect("depacketise"));
        }
        samples.extend(depack.flush(track_id).expect("flush"));
        assert_eq!(samples.len(), 4);
        for (i, s) in samples.iter().enumerate() {
            let d = s
                .timing
                .and_then(|t| t.duration())
                .expect("a multi-sample track times every sample");
            if i + 1 < samples.len() {
                assert!(
                    d > 0,
                    "non-final sample {i} must not have a zero duration: {:?}",
                    samples
                        .iter()
                        .map(|s| s.timing.and_then(|t| t.duration()))
                        .collect::<alloc::vec::Vec<_>>()
                );
            }
        }
        // The repeated instant's AU reuses the previous frame's period.
        assert_eq!(samples[1].timing.and_then(|t| t.duration()), Some(3000));
        assert_eq!(
            samples[2].timing.and_then(|t| t.duration()),
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
            !pts.windows(PAIR_WINDOW_LEN).all(|w| w[1] >= w[0]),
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
        let enc = broadcast_common::base64::base64_encode(data);
        assert_eq!(base64_decode(&enc).unwrap(), data);
    }

    #[test]
    fn base64_known_vector() {
        // RFC 4648 test vector.
        assert_eq!(
            broadcast_common::base64::base64_encode(b"foobar"),
            "Zm9vYmFy"
        );
        assert_eq!(broadcast_common::base64::base64_encode(b"fo"), "Zm8=");
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
        let wire = RtpPacketWire::parse(&h).unwrap();
        assert_eq!(wire.payload_type, 96);
    }

    fn rtp(seq: u16, ts: u32, marker: bool, payload: &[u8]) -> Vec<u8> {
        let mut p = alloc::vec![0x80u8, if marker { 0x80 | 96 } else { 96 }];
        p.extend_from_slice(&seq.to_be_bytes());
        p.extend_from_slice(&ts.to_be_bytes());
        p.extend_from_slice(&[0, 0, 0, 0]);
        p.extend_from_slice(payload);
        p
    }

    #[test]
    fn build_stap_a_carries_forbidden_and_max_nri_and_type_24() {
        let nals = alloc::vec![alloc::vec![0x27u8, 0xAA], alloc::vec![0x47u8, 0xBB]];
        let mut seq = SeqCounter::new(0);
        let pkt = build_stap_a(96, &nals, &mut seq, 0x1234, 0xDEAD_BEEF).expect("STAP-A");
        assert_eq!(pkt.payload[0], 0x58, "F=0, NRI=max(1,2)=2, type 24");

        let f = alloc::vec![alloc::vec![0x87u8, 0xCC]];
        let mut seq = SeqCounter::new(0);
        let pkt = build_stap_a(96, &f, &mut seq, 0, 1).expect("STAP-A");
        assert_eq!(pkt.payload[0], 0x98, "F=1, NRI=0, type 24");
        assert_ne!(pkt.payload[0] & 0x80, 0, "the F bit survives");
    }

    #[test]
    fn build_stap_a_bounds_the_16_bit_nal_size_prefix() {
        let at_max = alloc::vec![alloc::vec![0x67u8; 65_535]];
        let mut seq = SeqCounter::new(0);
        let pkt = build_stap_a(96, &at_max, &mut seq, 0, 0).expect("65535 fits the u16 prefix");
        assert_eq!(pkt.payload.len(), 1 + STAP_A_SIZE_LEN + 65_535);

        let over = alloc::vec![alloc::vec![0x67u8; 65_536]];
        let mut seq = SeqCounter::new(0);
        match build_stap_a(96, &over, &mut seq, 0, 0) {
            Err(Error::InvalidValue(e)) => {
                assert_eq!(e.field, "stap_a_nal_size");
                assert_eq!(e.value, 65_536);
            }
            other => panic!("expected a 16-bit overflow, got {other:?}"),
        }
    }

    #[test]
    fn fu_a_fills_the_payload_budget_and_rejects_an_empty_one() {
        let mut raw = alloc::vec![0x65u8];
        raw.extend((0..499u16).map(|i| i as u8));
        let data = Bytes::from(raw);
        let nal = &data[..];
        let mut out = Vec::new();
        let mut seq = SeqCounter::new(0);
        fragment_fu_a(nal, &data, 96, true, 100, &mut seq, 0, 0, &mut out).expect("FU-A");
        let sizes: Vec<usize> = out.iter().map(|p| p.payload.len()).collect();
        assert_eq!(
            sizes,
            alloc::vec![86, 86, 86, 86, 86, 69],
            "mtu - 14 per packet"
        );
        for pkt in &out {
            assert!(
                pkt.header.len() + pkt.payload.len() <= 100,
                "never over the MTU"
            );
        }
        assert_eq!(out[0].header.len() + out[0].payload.len(), 100);

        let mut out = Vec::new();
        match fragment_fu_a(nal, &data, 96, true, 14, &mut seq, 0, 0, &mut out) {
            Err(Error::InvalidInput(m)) => {
                assert_eq!(m, "MTU too small for FU-A fragmentation");
            }
            other => panic!("expected the MTU rejection, got {other:?}"),
        }
    }

    #[test]
    fn packetise_klv_requires_one_payload_byte() {
        let unit = Bytes::from_static(b"abcdef");
        match packetise_klv(&unit, 96, 0, 0, 0, RTP_HEADER_LEN) {
            Err(Error::InvalidInput(m)) => assert_eq!(m, "MTU too small for KLV-over-RTP"),
            other => panic!("expected the MTU rejection, got {other:?}"),
        }
        let packets =
            packetise_klv(&unit, 96, 0, 0, 0, RTP_HEADER_LEN + 1).expect("one byte per packet");
        assert_eq!(packets.len(), 6);
        assert!(packets.iter().all(|p| p.payload.len() == 1));
        assert!(
            packets
                .iter()
                .all(|p| p.header.len() + p.payload.len() <= 13)
        );
    }

    #[test]
    fn rtp_wrap_state_unrolls_across_the_32_bit_modulus() {
        let mut w = RtpWrapState::default();
        let base = 0xFFFF_F000u32;
        let a = w.push(base);
        assert_eq!(a, i64::from(base));
        let b = w.push(base.wrapping_add(3000));
        assert_eq!(b - a, 3000);
        let c = w.push(base.wrapping_add(6000));
        assert_eq!(c - b, 3000, "the 2^32 wrap keeps the 3000-tick step");
    }

    #[test]
    fn rtp_wrap_state_initialises_on_the_raw_first_timestamp() {
        let mut w = RtpWrapState::default();
        assert_eq!(w.push(0x9000_0000), 0x9000_0000i64);

        let mut w = RtpWrapState::default();
        let a = w.push(0x0001_0000);
        let b = w.push(0x0001_0000 - 3000);
        assert_eq!(b - a, -3000, "a small backward step is not a wrap");
    }

    #[test]
    fn rtp_wrap_state_splits_the_half_range_boundary() {
        let mut w = RtpWrapState::default();
        let a = w.push(0);
        let b = w.push(0x8000_0000);
        assert_eq!(b - a, 0x8000_0000, "exactly half the range is forward");

        let mut w = RtpWrapState::default();
        let a = w.push(0x8000_0000);
        let b = w.push(0);
        assert_eq!(b - a, -0x8000_0000, "exactly half the range is backward");

        let mut w = RtpWrapState::default();
        let a = w.push(1000);
        let b = w.push(0xFFFF_F000);
        assert_eq!(b - a, -5096, "a large positive step is a backward wrap");

        let mut w = RtpWrapState::default();
        let a = w.push(0xFFFF_F000);
        let b = w.push(1000);
        assert_eq!(b - a, 5096, "a large negative step is a forward wrap");
    }

    #[test]
    fn reassemble_video_flushes_on_a_timestamp_change_without_a_marker() {
        let packets = alloc::vec![
            rtp(0, 1, false, &[0x41, 0x11]),
            rtp(1, 2, true, &[0x41, 0x22]),
        ];
        let aus = reassemble_video(&packets).expect("reassemble");
        assert_eq!(aus.len(), 2, "the timestamp change closes the first AU");
        assert_eq!(aus[0].timestamp, 1);
        assert_eq!(aus[1].timestamp, 2);
    }

    #[test]
    fn reassemble_video_accepts_a_zero_size_stap_a_and_bounds_its_prefix() {
        let aus = reassemble_video(&[rtp(0, 1, true, &[0x18, 0x00, 0x00])])
            .expect("a zero-size aggregate entry is accepted");
        assert_eq!(aus.len(), 1);

        match reassemble_video(&[rtp(0, 1, true, &[0x18, 0x00])]) {
            Err(Error::BufferTooShort(e)) => assert_eq!(e.what, "STAP-A size prefix"),
            _ => panic!("expected the truncated STAP-A prefix"),
        }
    }

    #[test]
    fn reassemble_video_bounds_the_fu_a_header() {
        let aus = reassemble_video(&[rtp(0, 1, true, &[0x7C, 0xC5])])
            .expect("a start+end fragment with an empty body is valid");
        assert_eq!(aus.len(), 1);
        assert_eq!(aus[0].data, alloc::vec![0, 0, 0, 1, 0x65]);

        match reassemble_video(&[rtp(0, 1, true, &[0x7C])]) {
            Err(Error::BufferTooShort(e)) => assert_eq!(e.what, "FU-A header"),
            _ => panic!("expected the missing FU header"),
        }
    }

    fn audio_payload(headers_bits: u16, headers: &[u16], body: &[u8]) -> Vec<u8> {
        let mut p = headers_bits.to_be_bytes().to_vec();
        for h in headers {
            p.extend_from_slice(&h.to_be_bytes());
        }
        p.extend_from_slice(body);
        p
    }

    #[test]
    fn reassemble_audio_reads_every_au_header() {
        let payload = audio_payload(
            48,
            &[(2u16) << AAC_INDEX_LENGTH; 3],
            &[0x00, 0x10, 0x21, 0x22, 0x31, 0x32, 0x41, 0x42],
        );
        let aus = reassemble_audio(&[rtp(0, 1, true, &payload)]).expect("three AUs");
        assert_eq!(aus.len(), 3, "48 header bits declare exactly three AUs");
        assert_eq!(aus[0].data, alloc::vec![0x00, 0x10]);
        assert_eq!(aus[1].data, alloc::vec![0x21, 0x22]);
        assert_eq!(aus[2].data, alloc::vec![0x31, 0x32]);
    }

    #[test]
    fn reassemble_audio_bounds_the_headers_length_field() {
        let aus = reassemble_audio(&[rtp(0, 1, true, &[0, 0])])
            .expect("a headers-length-only packet carries no AU");
        assert!(aus.is_empty());

        match reassemble_audio(&[rtp(0, 1, true, &[0])]) {
            Err(Error::BufferTooShort(e)) => assert_eq!(e.what, "AAC AU-headers-length"),
            _ => panic!("expected the short headers length"),
        }
    }

    #[test]
    fn reassemble_audio_accepts_headers_exactly_filling_the_payload() {
        let payload = audio_payload(16, &[(2u16) << AAC_INDEX_LENGTH], &[]);
        assert_eq!(payload.len(), 4);
        let aus = reassemble_audio(&[rtp(0, 1, true, &payload)])
            .expect("headers ending exactly at the payload end are valid");
        assert!(aus.is_empty(), "no AU bytes followed the headers");
    }

    #[test]
    fn reassemble_audio_restarts_on_a_size_mismatch() {
        let first = audio_payload(16, &[(100u16) << AAC_INDEX_LENGTH], &[0xAA; 60]);
        let second = audio_payload(16, &[(80u16) << AAC_INDEX_LENGTH], &[0xBB; 80]);
        let aus = reassemble_audio(&[rtp(0, 1, false, &first), rtp(1, 1, true, &second)])
            .expect("reassemble");
        assert_eq!(
            aus.len(),
            1,
            "the size-100 AU is dropped, the size-80 one kept"
        );
        assert_eq!(aus[0].data, alloc::vec![0xBB; 80]);
    }

    #[test]
    fn reassemble_audio_drops_a_partial_au_when_the_timestamp_changes() {
        let partial = audio_payload(16, &[(4u16) << AAC_INDEX_LENGTH], &[0xAA; 2]);
        let complete = audio_payload(16, &[(4u16) << AAC_INDEX_LENGTH], &[0xBB; 4]);
        let aus = reassemble_audio(&[rtp(0, 1, false, &partial), rtp(1, 2, true, &complete)])
            .expect("reassemble");
        assert_eq!(aus.len(), 1);
        assert_eq!(aus[0].timestamp, 2);
        assert_eq!(aus[0].data, alloc::vec![0xBB; 4]);
    }

    #[test]
    fn map_rtp_error_keeps_the_offending_field_and_value() {
        match map_rtp_error(rtp_packet::Error::BufferTooShort(BufferTooShort {
            need: 12,
            have: 10,
            what: "RTP fixed header",
        })) {
            Error::BufferTooShort(e) => {
                assert_eq!((e.need, e.have), (12, 10));
                assert_eq!(e.what, "RTP fixed header");
            }
            other => panic!("expected BufferTooShort, got {other:?}"),
        }
        match map_rtp_error(rtp_packet::Error::UnsupportedVersion(
            broadcast_common::error::UnsupportedVersion {
                what: "RTP",
                found: 1,
            },
        )) {
            Error::InvalidValue(e) => {
                assert_eq!(e.field, "rtp_version");
                assert_eq!(e.value, 1);
            }
            other => panic!("expected the version rejection, got {other:?}"),
        }
        match map_rtp_error(rtp_packet::Error::InvalidValue(InvalidValue {
            field: "csrc",
            value: 9,
            reason: "too many",
        })) {
            Error::InvalidValue(e) => {
                assert_eq!(e.field, "csrc");
                assert_eq!(e.value, 9);
            }
            other => panic!("expected InvalidValue, got {other:?}"),
        }
        match map_rtp_error(rtp_packet::Error::InvalidPadding {
            count: 0,
            reason: "zero",
        }) {
            Error::InvalidValue(e) => {
                assert_eq!(e.field, "rtp_padding");
                assert_eq!(e.value, 0);
            }
            other => panic!("expected the padding rejection, got {other:?}"),
        }
        match map_rtp_error(rtp_packet::Error::ExtensionNotWordAligned { data_len: 3 }) {
            Error::InvalidValue(e) => {
                assert_eq!(e.field, "rtp_extension_length");
                assert_eq!(e.value, 3);
            }
            other => panic!("expected the extension rejection, got {other:?}"),
        }
    }

    #[test]
    fn base64_decode_reports_the_offending_byte() {
        match base64_decode("@@@") {
            Err(Error::InvalidValue(e)) => {
                assert_eq!(e.field, "base64");
                assert_eq!(e.value, u64::from(b'@'));
            }
            other => panic!("expected an invalid-byte rejection, got {other:?}"),
        }
    }
}
