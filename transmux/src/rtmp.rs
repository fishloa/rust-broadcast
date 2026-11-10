//! RTMP (Adobe Real-Time Messaging Protocol) 1.0 transport spoke.
//!
//! Spec: **Adobe RTMP Specification 1.0** (December 2012); AMF0 values follow
//! the companion **AMF0 Specification**. See `private/specs-verified/mpeg/transmux/rtmp/rtmp.md` for
//! the transcription. Everything is **big-endian** on the wire except the RTMP
//! message stream id (little-endian, §5.3.1.2.1) and where AMF0 dictates.
//!
//! The RTMP wire layer itself — the handshake (§5.2), the chunk stream (§5.3),
//! the protocol control messages (§5.4) and AMF0 commands (§7) — lives in the
//! `rtmp-runtime` crate ([`rtmp_runtime::handshake`],
//! [`rtmp_runtime::chunk`], [`rtmp_runtime::message`], [`rtmp_runtime::amf0`]).
//! This module is only the container spoke that bridges it to the [`Media`] IR.
//!
//! RTMP carries FLV-format audio/video: an RTMP **Audio (type 8)** message body
//! is an FLV `AudioTagHeader`+data and a **Video (type 9)** message body is an
//! FLV `VideoTagHeader`+data (Adobe FLV v10.1 Annex E §E.4.2 / §E.4.3). So this
//! spoke reuses the crate's FLV spoke ([`crate::flv::FlvDemux`] /
//! [`crate::flv::FlvMux`]) to reach the [`Media`] IR rather than re-implementing
//! FLV parsing:
//!
//! - [`RtmpDemux`] ([`Unpackage`]): de-frames the chunk stream (§5.3),
//!   reassembles complete messages, collects the A/V (and script) message
//!   bodies, rebuilds an FLV byte stream from them, and hands it to
//!   [`FlvDemux`] → [`Media`].
//! - [`RtmpMux`] ([`Package`]): serialises the [`Media`] to FLV via
//!   [`FlvMux`], splits the FLV tags, wraps each A/V tag
//!   body as an RTMP message and chunks them (§5.3).
//!
//! `no_std` + `alloc`.

use alloc::vec::Vec;
use core::fmt;

use broadcast_common::{Package, Unpackage};
use rtmp_runtime::chunk::{ChunkAssembler, ChunkWriter, DEFAULT_CHUNK_SIZE, Message};
use rtmp_runtime::message::msg_type;

use crate::flv::{FlvDemux, FlvError, FlvMux};
use crate::media::Media;

// ---------------------------------------------------------------------------
// This muxer's chunk stream choice (not a wire constant)
// ---------------------------------------------------------------------------

/// Chunk stream id this crate uses for outbound audio messages.
const CSID_AUDIO: u32 = 4;
/// Chunk stream id this crate uses for outbound video messages.
const CSID_VIDEO: u32 = 5;
/// Chunk stream id this crate uses for outbound data (script) messages.
const CSID_DATA: u32 = 6;

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Errors specific to RTMP transport framing (Adobe RTMP 1.0).
// No longer `Eq` (only `PartialEq`): `RtmpFlvError::Flv` wraps `FlvError`,
// which lost `Eq` for the same reason (issue #1140 T12 cascade).
#[derive(Debug, PartialEq, Clone)]
#[non_exhaustive]
pub enum RtmpFlvError {
    /// A buffer ended before a field could be read.
    Truncated {
        /// What was being parsed when the buffer ran out.
        what: &'static str,
        /// Bytes required.
        need: usize,
        /// Bytes available.
        have: usize,
    },
    /// FLV routing of the reassembled A/V bodies failed.
    Flv(FlvError),
    /// A [`RtmpMux::chunk_size`] exceeded the `u32` the chunk writer takes.
    ChunkSizeTooLarge {
        /// The requested chunk size.
        size: usize,
    },
    /// The RTMP wire layer (chunk stream assembly / chunking) failed.
    Rtmp(rtmp_runtime::RtmpError),
    /// A length did not fit the wire field it is written to (#1129).
    FieldOverflow(broadcast_common::len::FieldOverflow),
}

impl fmt::Display for RtmpFlvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RtmpFlvError::Truncated { what, need, have } => {
                write!(
                    f,
                    "RTMP truncated while parsing {what}: need {need}, have {have}"
                )
            }
            RtmpFlvError::Flv(e) => write!(f, "RTMP FLV routing: {e}"),
            RtmpFlvError::ChunkSizeTooLarge { size } => write!(
                f,
                "RTMP chunk size {size} does not fit the u32 chunk size field"
            ),
            RtmpFlvError::Rtmp(e) => write!(f, "RTMP wire: {e}"),
            RtmpFlvError::FieldOverflow(e) => write!(f, "RTMP {e}"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for RtmpFlvError {}

impl From<FlvError> for RtmpFlvError {
    fn from(e: FlvError) -> Self {
        RtmpFlvError::Flv(e)
    }
}

impl From<broadcast_common::len::FieldOverflow> for RtmpFlvError {
    fn from(e: broadcast_common::len::FieldOverflow) -> Self {
        RtmpFlvError::FieldOverflow(e)
    }
}

impl From<rtmp_runtime::RtmpError> for RtmpFlvError {
    fn from(e: rtmp_runtime::RtmpError) -> Self {
        RtmpFlvError::Rtmp(e)
    }
}

// ---------------------------------------------------------------------------
// FLV bridge: rebuild / split FLV from A/V message bodies
// ---------------------------------------------------------------------------

/// FLV signature `"FLV"` (Adobe FLV v10.1 §E.2).
const FLV_SIGNATURE: [u8; 3] = *b"FLV";
/// FLV file-format version (§E.2).
const FLV_VERSION: u8 = 1;
/// FLV header length (§E.2).
const FLV_HEADER_LEN: u32 = 9;
/// `TypeFlags` bit: audio present (§E.2).
const FLV_FLAG_AUDIO: u8 = 0x04;
/// `TypeFlags` bit: video present (§E.2).
const FLV_FLAG_VIDEO: u8 = 0x01;
/// FLV tag header length (§E.4.1).
const FLV_TAG_HEADER_LEN: usize = 11;
/// FLV `PreviousTagSize` trailer length (§E.4.1).
const FLV_PREV_TAG_SIZE_LEN: usize = 4;
/// FLV tag type: audio (§E.4.1) — matches RTMP [`msg_type::AUDIO`].
const FLV_TAG_AUDIO: u8 = msg_type::AUDIO;
/// FLV tag type: video (§E.4.1) — matches RTMP [`msg_type::VIDEO`].
const FLV_TAG_VIDEO: u8 = msg_type::VIDEO;
/// FLV tag type: script data (§E.4.1).
const FLV_TAG_SCRIPT: u8 = msg_type::DATA_AMF0;

/// Write a big-endian unsigned 24-bit integer.
fn write_u24(out: &mut Vec<u8>, v: u32) {
    out.push((v >> 16) as u8);
    out.push((v >> 8) as u8);
    out.push(v as u8);
}

/// Read a big-endian unsigned 24-bit integer.
fn read_u24(b: &[u8]) -> u32 {
    (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2])
}

/// Build an FLV byte stream from A/V (and script) tag bodies with timestamps.
/// `(tag_type, timestamp_ms, body)` — the body is exactly an FLV tag payload
/// (which for A/V equals the RTMP message body).
///
/// # Errors
///
/// Returns [`RtmpFlvError::FieldOverflow`] if a tag body is 16 MiB (2^24) or larger —
/// `DataSize` is a UI24 field (§E.4.1) and cannot represent it.
fn build_flv(
    tags: &[(u8, u32, Vec<u8>)],
    has_video: bool,
    has_audio: bool,
) -> Result<Vec<u8>, RtmpFlvError> {
    let mut out = Vec::new();
    out.extend_from_slice(&FLV_SIGNATURE);
    out.push(FLV_VERSION);
    let mut flags = 0u8;
    if has_video {
        flags |= FLV_FLAG_VIDEO;
    }
    if has_audio {
        flags |= FLV_FLAG_AUDIO;
    }
    out.push(flags);
    out.extend_from_slice(&FLV_HEADER_LEN.to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes()); // PreviousTagSize0 = 0
    for (tag_type, ts, body) in tags {
        let start = out.len();
        out.push(*tag_type);
        let data_size = broadcast_common::len::fit_u24(body.len(), "DataSize")?;
        write_u24(&mut out, data_size);
        // Timestamp UI24 + extended high byte.
        out.push((*ts >> 16) as u8);
        out.push((*ts >> 8) as u8);
        out.push(*ts as u8);
        out.push((*ts >> 24) as u8);
        out.extend_from_slice(&[0, 0, 0]); // StreamID = 0
        out.extend_from_slice(body);
        let tag_size = (out.len() - start) as u32;
        out.extend_from_slice(&tag_size.to_be_bytes());
    }
    Ok(out)
}

/// Walk an FLV byte stream into `(tag_type, timestamp_ms, body)` tags (§E.4.1).
fn split_flv(flv: &[u8]) -> Result<Vec<(u8, u32, Vec<u8>)>, RtmpFlvError> {
    if flv.len() < FLV_HEADER_LEN as usize + FLV_PREV_TAG_SIZE_LEN {
        return Err(RtmpFlvError::Truncated {
            what: "FLV header",
            need: FLV_HEADER_LEN as usize + FLV_PREV_TAG_SIZE_LEN,
            have: flv.len(),
        });
    }
    let data_offset = u32::from_be_bytes([flv[5], flv[6], flv[7], flv[8]]) as usize;
    let mut off = data_offset.max(FLV_HEADER_LEN as usize) + FLV_PREV_TAG_SIZE_LEN;
    let mut tags = Vec::new();
    while off + FLV_TAG_HEADER_LEN <= flv.len() {
        let tag_type = flv[off];
        let data_size = read_u24(&flv[off + 1..]) as usize;
        let ts_lo = read_u24(&flv[off + 4..]);
        let ts_ext = flv[off + 7] as u32;
        let timestamp = (ts_ext << 24) | ts_lo;
        let body_start = off + FLV_TAG_HEADER_LEN;
        let body_end = body_start + data_size;
        if body_end + FLV_PREV_TAG_SIZE_LEN > flv.len() {
            return Err(RtmpFlvError::Truncated {
                what: "FLV tag body",
                need: body_end + FLV_PREV_TAG_SIZE_LEN,
                have: flv.len(),
            });
        }
        tags.push((tag_type, timestamp, flv[body_start..body_end].to_vec()));
        off = body_end + FLV_PREV_TAG_SIZE_LEN;
    }
    Ok(tags)
}

// ---------------------------------------------------------------------------
// RtmpDemux — Unpackage<Input = &[u8]>
// ---------------------------------------------------------------------------

/// Demux an RTMP chunk stream into a [`Media`] (Adobe RTMP 1.0).
///
/// Reassembles the chunk stream (§5.3), collects the Audio (type 8) / Video
/// (type 9) / data (type 18 `onMetaData`) message bodies — which are FLV tag
/// bodies — rebuilds an FLV byte stream from them, and routes it through
/// [`FlvDemux`] to the IR. Protocol control and command
/// messages are consumed by the reassembly (Set Chunk Size adjusts the chunk
/// size) and otherwise ignored for the media path.
///
/// The input is the post-handshake chunk stream (drive the handshake with
/// [`rtmp_runtime::handshake`] first if needed).
#[derive(Debug, Default, Clone)]
pub struct RtmpDemux<'a> {
    _marker: core::marker::PhantomData<&'a [u8]>,
}

impl RtmpDemux<'_> {
    /// Create a new demuxer.
    pub fn new() -> Self {
        Self {
            _marker: core::marker::PhantomData,
        }
    }
}

impl<'a> Unpackage for RtmpDemux<'a> {
    type Input = &'a [u8];
    type Media = Media;
    type Error = RtmpFlvError;

    fn unpackage(&mut self, input: &'a [u8]) -> Result<Media, RtmpFlvError> {
        let messages = ChunkAssembler::new().push(input)?;
        let mut tags: Vec<(u8, u32, Vec<u8>)> = Vec::new();
        let mut has_video = false;
        let mut has_audio = false;
        for m in messages {
            match m.message_type_id {
                msg_type::AUDIO => {
                    has_audio = true;
                    tags.push((FLV_TAG_AUDIO, m.timestamp, m.payload));
                }
                msg_type::VIDEO => {
                    has_video = true;
                    tags.push((FLV_TAG_VIDEO, m.timestamp, m.payload));
                }
                msg_type::DATA_AMF0 => {
                    tags.push((FLV_TAG_SCRIPT, m.timestamp, m.payload));
                }
                _ => { /* control / command / user-control — not media */ }
            }
        }
        let flv = build_flv(&tags, has_video, has_audio)?;
        let mut demux = FlvDemux::new();
        demux.unpackage(&flv).map_err(RtmpFlvError::Flv)
    }
}

// ---------------------------------------------------------------------------
// RtmpMux — Package<Output = Vec<u8>>
// ---------------------------------------------------------------------------

/// Mux a [`Media`] into an RTMP chunk stream (Adobe RTMP 1.0).
///
/// Serialises the IR to FLV via [`FlvMux`], splits it into
/// FLV tags, wraps each tag body as the corresponding RTMP message — Audio
/// (type 8), Video (type 9) or Data (type 18) — on a per-kind chunk stream id,
/// and chunks them at [`chunk_size`](RtmpMux::chunk_size) (§5.3). The output is
/// the post-handshake chunk stream (emit a Set Chunk Size control message and
/// the handshake separately if the peer needs them).
#[derive(Debug, Clone)]
pub struct RtmpMux {
    /// Maximum chunk size (§5.4.1); a smaller value fragments large video
    /// messages across more chunks.
    pub chunk_size: usize,
    /// Message stream id assigned to the A/V messages.
    pub message_stream_id: u32,
}

impl Default for RtmpMux {
    fn default() -> Self {
        Self {
            chunk_size: usize::try_from(DEFAULT_CHUNK_SIZE).unwrap_or(usize::MAX),
            message_stream_id: 1,
        }
    }
}

impl RtmpMux {
    /// Create a muxer with the default chunk size (128).
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a muxer with an explicit chunk size.
    pub fn with_chunk_size(chunk_size: usize) -> Self {
        Self {
            chunk_size: chunk_size.max(1),
            message_stream_id: 1,
        }
    }
}

impl Package for RtmpMux {
    type Media = Media;
    type Output = Vec<u8>;
    type Error = RtmpFlvError;

    fn package(&mut self, media: &Media) -> Result<Vec<u8>, RtmpFlvError> {
        let mut flv_mux = FlvMux::new();
        let flv = flv_mux.package(media).map_err(RtmpFlvError::Flv)?;
        let tags = split_flv(&flv)?;

        let mut writer = ChunkWriter::new();
        writer.set_chunk_size(
            u32::try_from(self.chunk_size).map_err(|_| RtmpFlvError::ChunkSizeTooLarge {
                size: self.chunk_size,
            })?,
        );

        let mut out = Vec::new();
        for (tag_type, ts, body) in tags {
            let (chunk_stream_id, message_type_id) = match tag_type {
                FLV_TAG_AUDIO => (CSID_AUDIO, msg_type::AUDIO),
                FLV_TAG_VIDEO => (CSID_VIDEO, msg_type::VIDEO),
                FLV_TAG_SCRIPT => (CSID_DATA, msg_type::DATA_AMF0),
                _ => (CSID_DATA, msg_type::DATA_AMF0),
            };
            let message = Message {
                chunk_stream_id,
                timestamp: ts,
                message_type_id,
                message_stream_id: self.message_stream_id,
                payload: body,
            };
            out.extend_from_slice(&writer.write(&message)?);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn u24_round_trip() {
        let mut out = Vec::new();
        write_u24(&mut out, 0x123456);
        assert_eq!(out, [0x12, 0x34, 0x56]);
        assert_eq!(read_u24(&out), 0x123456);
    }

    /// The same UI24 `DataSize` overflow, via the FLV bridge `build_flv`
    /// (unpackage path).
    #[test]
    fn build_flv_oversized_tag_errors() {
        let tags = alloc::vec![(FLV_TAG_VIDEO, 0u32, alloc::vec![0u8; 1 << 24])];
        let err = build_flv(&tags, true, false).unwrap_err();
        assert!(
            matches!(
                err,
                RtmpFlvError::FieldOverflow(broadcast_common::len::FieldOverflow {
                    field: "DataSize",
                    ..
                })
            ),
            "expected FieldOverflow for DataSize, got {err:?}"
        );
    }

    #[test]
    fn media_chunk_stream_ids_are_distinct_and_outside_the_control_stream() {
        for media in [CSID_AUDIO, CSID_VIDEO, CSID_DATA] {
            assert_ne!(media, rtmp_runtime::message::CONTROL_CHUNK_STREAM_ID);
        }
        assert_ne!(CSID_AUDIO, CSID_VIDEO);
        assert_ne!(CSID_AUDIO, CSID_DATA);
        assert_ne!(CSID_VIDEO, CSID_DATA);
    }

    #[test]
    fn a_chunk_size_beyond_the_u32_field_is_reported_not_wrapped() {
        let mut mux = RtmpMux::with_chunk_size(usize::MAX);
        let media = Media::new(Vec::new(), 1000);
        let err = mux.package(&media).expect_err("chunk size too large");
        assert_eq!(err, RtmpFlvError::ChunkSizeTooLarge { size: usize::MAX });
    }
}
