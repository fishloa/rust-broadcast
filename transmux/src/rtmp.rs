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

use broadcast_common::{Package, Parse, Serialize, Unpackage};
use rtmp_runtime::RtmpError;
use rtmp_runtime::chunk::{ChunkAssembler, ChunkWriter, DEFAULT_CHUNK_SIZE, Message};
use rtmp_runtime::flv::{FlvHeader, FlvTag, TagType};
use rtmp_runtime::message::{ProtocolControl, msg_type};

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

/// Build an FLV byte stream from A/V (and script) tag bodies with timestamps.
/// `(tag_type, timestamp_ms, body)` — the body is exactly an FLV tag payload
/// (which for A/V equals the RTMP message body).
///
/// # Errors
///
/// Returns [`RtmpFlvError::FieldOverflow`] if a tag body is 16 MiB (2^24) or larger —
/// `DataSize` is a UI24 field (Adobe FLV v10.1 Annex E §E.4.1) and cannot represent it.
fn build_flv(
    tags: &[(TagType, u32, Vec<u8>)],
    has_video: bool,
    has_audio: bool,
) -> Result<Vec<u8>, RtmpFlvError> {
    let mut out = FlvHeader::new(has_audio, has_video).try_to_bytes()?;
    for (tag_type, timestamp, body) in tags {
        let tag = FlvTag::new(*tag_type, *timestamp, body);
        out.extend_from_slice(&tag.try_to_bytes()?);
    }
    Ok(out)
}

/// Walk an FLV byte stream into its tags (§E.4.1).
fn walk_flv_tags(flv: &[u8]) -> Result<Vec<FlvTag<'_>>, RtmpFlvError> {
    let header = FlvHeader::parse(flv)?;
    let mut tags = Vec::new();
    for tag in rtmp_runtime::flv::tags(flv, &header) {
        add_tag(&mut tags, tag)?;
    }
    Ok(tags)
}

/// Collect one walked tag, mapping the codec's `BufferTooShort` onto the
/// bridge's own truncation report.
fn add_tag<'a>(
    tags: &mut Vec<FlvTag<'a>>,
    tag: Result<FlvTag<'a>, RtmpError>,
) -> Result<(), RtmpFlvError> {
    match tag {
        Ok(tag) => {
            tags.push(tag);
            Ok(())
        }
        Err(RtmpError::BufferTooShort(e)) => Err(RtmpFlvError::Truncated {
            what: e.what,
            need: e.need,
            have: e.have,
        }),
        Err(e) => Err(RtmpFlvError::Rtmp(e)),
    }
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
        let mut assembler = ChunkAssembler::new();
        assembler.feed(input);
        let mut tags: Vec<(TagType, u32, Vec<u8>)> = Vec::new();
        let mut has_video = false;
        let mut has_audio = false;
        while let Some(m) = assembler.next_message()? {
            match m.message_type_id {
                msg_type::SET_CHUNK_SIZE => {
                    if let Some(ProtocolControl::SetChunkSize(size)) =
                        ProtocolControl::from_message(&m)?
                    {
                        assembler.set_chunk_size(size);
                    }
                }
                msg_type::AUDIO => {
                    has_audio = true;
                    tags.push((TagType::Audio, m.timestamp, m.payload));
                }
                msg_type::VIDEO => {
                    has_video = true;
                    tags.push((TagType::Video, m.timestamp, m.payload));
                }
                msg_type::DATA_AMF0 => {
                    tags.push((TagType::Script, m.timestamp, m.payload));
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
/// self-describing at any chunk size: at any size other than the §5.4.1
/// default of 128 bytes it opens with a Set Chunk Size control message, so a peer reassembles it
/// without being told the size out of band. The handshake stays the caller's
/// job.
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
        let chunk_size =
            u32::try_from(self.chunk_size).map_err(|_| RtmpFlvError::ChunkSizeTooLarge {
                size: self.chunk_size,
            })?;
        let mut flv_mux = FlvMux::new();
        let flv = flv_mux.package(media).map_err(RtmpFlvError::Flv)?;
        let tags = walk_flv_tags(&flv)?;

        let mut writer = ChunkWriter::new();
        let mut out = Vec::new();
        if chunk_size != DEFAULT_CHUNK_SIZE {
            let control = ProtocolControl::SetChunkSize(chunk_size).try_to_message()?;
            out.extend_from_slice(&writer.write(&control)?);
            writer.set_chunk_size(chunk_size);
        }
        for tag in tags {
            let (chunk_stream_id, message_type_id) = match tag.tag_type {
                TagType::Audio => (CSID_AUDIO, msg_type::AUDIO),
                TagType::Video => (CSID_VIDEO, msg_type::VIDEO),
                TagType::Script | TagType::Reserved(_) => (CSID_DATA, msg_type::DATA_AMF0),
                _ => (CSID_DATA, msg_type::DATA_AMF0),
            };
            let message = Message {
                chunk_stream_id,
                timestamp: tag.timestamp,
                message_type_id,
                message_stream_id: self.message_stream_id,
                payload: tag.body.to_vec(),
            };
            out.extend_from_slice(&writer.write(&message)?);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The same UI24 `DataSize` overflow, via the FLV bridge `build_flv`
    /// (unpackage path).
    #[test]
    fn build_flv_oversized_tag_errors() {
        let tags = alloc::vec![(TagType::Video, 0u32, alloc::vec![0u8; 1 << 24])];
        let err = build_flv(&tags, true, false).unwrap_err();
        assert!(
            matches!(
                err,
                RtmpFlvError::Rtmp(rtmp_runtime::RtmpError::FieldOverflow(
                    broadcast_common::len::FieldOverflow {
                        field: "DataSize",
                        ..
                    }
                ))
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
}
