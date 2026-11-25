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
//! This module is only the container spoke that bridges it to the mux stage;
//! the demux side is `rtmp-runtime`'s session engine feeding
//! [`StreamingFlvDemux`](crate::flv_stream::StreamingFlvDemux).
//!
//! RTMP carries FLV-format audio/video: an RTMP **Audio (type 8)** message body
//! is an FLV `AudioTagHeader`+data and a **Video (type 9)** message body is an
//! FLV `VideoTagHeader`+data (Adobe FLV v10.1 Annex E §E.4.2 / §E.4.3). So this
//! spoke reuses the crate's FLV tag pipeline rather than re-implementing FLV
//! parsing: [`RtmpMuxStage`] ([`Stage`]) runs the same FLV tag pipeline
//! [`crate::flv::FlvMuxStage`] does, wraps each tag body as an RTMP message
//! and chunks it (§5.3), one message at a time. The demux side de-frames the
//! chunk stream in `rtmp-runtime` and feeds the emitted FLV tags to
//! [`StreamingFlvDemux`](crate::flv_stream::StreamingFlvDemux).
//!
//! `no_std` + `alloc`.

use alloc::collections::VecDeque;

use broadcast_common::stage::{Stage, Timestamp};
use rtmp_runtime::chunk::{
    ChunkWriter, DEFAULT_CHUNK_SIZE_VALUE, MAX_CHUNK_SIZE, MAX_MESSAGE_LEN, Message,
};
use rtmp_runtime::flv::TagType;
use rtmp_runtime::message::{ChunkSize, MessageTypeId, ProtocolControl};

use crate::flv::{FlvError, FlvTagStream, OutTag};
use crate::mux_stage::{MuxInput, MuxOutput};

// ---------------------------------------------------------------------------
// This muxer's chunk stream choice (not a wire constant)
// ---------------------------------------------------------------------------

/// Chunk stream id this crate uses for outbound audio messages.
///
/// These three ids are the muxer's own chunk streams and must not overlap a
/// peer writer's: a connection that also writes through another chunk writer
/// (rtmp-runtime's session uses 3 for commands, 4 for audio and 5 for video)
/// has one per-chunk-stream state at the receiver, and two writers compacting
/// headers independently would each inherit the other's state.
const CSID_AUDIO: u32 = 6;
/// Chunk stream id this crate uses for outbound video messages.
const CSID_VIDEO: u32 = 7;
/// Chunk stream id this crate uses for outbound data (script) messages.
const CSID_DATA: u32 = 8;

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Errors specific to RTMP transport framing (Adobe RTMP 1.0).
// No longer `Eq` (only `PartialEq`): `RtmpFlvError::Flv` wraps `FlvError`,
// which lost `Eq` for the same reason (issue #1140 T12 cascade).
#[derive(Debug, PartialEq, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum RtmpFlvError {
    /// A buffer ended before a field could be read.
    #[error("RTMP truncated while parsing {what}: need {need}, have {have}")]
    Truncated {
        /// What was being parsed when the buffer ran out.
        what: &'static str,
        /// Bytes required.
        need: usize,
        /// Bytes available.
        have: usize,
    },
    /// FLV routing of the reassembled A/V bodies failed.
    #[error("RTMP FLV routing: {0}")]
    Flv(FlvError),
    /// A [`RtmpMuxStage::chunk_size`] was 0 or above `rtmp_runtime::chunk::MAX_CHUNK_SIZE`:
    /// the muxer would otherwise announce a size it does not frame at (the
    /// chunk writer clamps it).
    #[error("RTMP chunk size {size} is not in 1..={MAX_CHUNK_SIZE}")]
    ChunkSizeTooLarge {
        /// The requested chunk size.
        size: usize,
    },
    /// A tag body exceeded `rtmp_runtime::chunk::MAX_MESSAGE_LEN`; the message
    /// would be rejected by a peer on the other end.
    #[error("RTMP message body {len} bytes exceeds the {max}-byte message limit")]
    MessageTooLarge {
        /// The tag body length.
        len: usize,
        /// The largest message length a peer accepts.
        max: usize,
    },
    /// The RTMP wire layer (chunk stream assembly / chunking) failed.
    #[error("RTMP wire: {0}")]
    Rtmp(rtmp_runtime::RtmpError),
    /// A length did not fit the wire field it is written to (#1129).
    #[error("RTMP {0}")]
    FieldOverflow(broadcast_common::len::FieldOverflow),
}

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

// ---------------------------------------------------------------------------
// RtmpMuxStage — Stage<In = MuxInput, Out = MuxOutput>
// ---------------------------------------------------------------------------

/// Mux an RTMP chunk stream from a live sample stream (Adobe RTMP 1.0).
///
/// The stage form of the RTMP container writer: [`MuxInput::Track`]
/// announcements feed the same FLV tag pipeline [`crate::flv::FlvMuxStage`]
/// drives, and each FLV tag — `onMetaData`, the AVC/AAC sequence headers, then
/// one per frame — is wrapped as the corresponding RTMP message (Audio type 8,
/// Video type 9, Data type 18) on a per-kind chunk stream id and chunked at
/// [`chunk_size`](Self::chunk_size) (§5.3). The output is self-describing at
/// any chunk size: at any size other than the §5.4.1 default of 128 bytes it
/// opens with a Set Chunk Size control message, so a peer reassembles it
/// without being told the size out of band. The handshake stays the caller's
/// job.
///
/// The stage holds one finished RTMP message at a time; it never holds a whole
/// media. The chunk writer's per-chunk-stream state persists across calls, so
/// the stream is byte-identical to one written in a single pass.
#[derive(Debug, Clone)]
pub struct RtmpMuxStage {
    /// Maximum chunk size (§5.4.1); a smaller value fragments large video
    /// messages across more chunks.
    pub chunk_size: usize,
    /// Message stream id assigned to the A/V messages.
    pub message_stream_id: u32,
    stream: FlvTagStream,
    writer: ChunkWriter,
    opened: bool,
    /// Whether [`Stage::feed`] writes the stream's opening messages (the
    /// metadata body and the sequence headers). A caller that opens the
    /// connection itself — a publish client that sends `@setDataFrame` and
    /// the sequence headers through its own session — turns this off.
    opening: bool,
    out: VecDeque<MuxOutput>,
}

impl Default for RtmpMuxStage {
    fn default() -> Self {
        Self {
            chunk_size: usize::try_from(DEFAULT_CHUNK_SIZE_VALUE).unwrap_or(usize::MAX),
            message_stream_id: 1,
            stream: FlvTagStream::default(),
            writer: ChunkWriter::new(),
            opened: false,
            opening: true,
            out: VecDeque::new(),
        }
    }
}

impl RtmpMuxStage {
    /// Create a muxer with the default chunk size (128).
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a muxer with an explicit chunk size.
    ///
    /// The size is announced to the peer verbatim, so it must be exactly the
    /// size the chunk writer frames at: a size of 0 or above
    /// `rtmp_runtime::chunk::MAX_CHUNK_SIZE` is rejected with
    /// [`RtmpFlvError::ChunkSizeTooLarge`] rather than announced and then
    /// framed at a clamped one.
    pub fn with_chunk_size(chunk_size: usize) -> Self {
        Self {
            chunk_size,
            message_stream_id: 1,
            stream: FlvTagStream::default(),
            writer: ChunkWriter::new(),
            opened: false,
            opening: true,
            out: VecDeque::new(),
        }
    }

    /// A muxer that writes only the frame messages: the caller has already
    /// sent the connection's chunk-size announcement, its metadata and the
    /// sequence headers (a publish client does exactly that on its own
    /// chunk streams).
    #[must_use]
    pub fn frames_only(mut self) -> Self {
        self.opening = false;
        self
    }

    /// The chunk size as the wire field carries it, rejecting a value the chunk
    /// writer would clamp (so the announced size is the framed size).
    fn wire_chunk_size(&self) -> Result<u32, RtmpFlvError> {
        let size = u32::try_from(self.chunk_size)
            .ok()
            .filter(|size| (1..=MAX_CHUNK_SIZE).contains(size));
        size.ok_or(RtmpFlvError::ChunkSizeTooLarge {
            size: self.chunk_size,
        })
    }

    /// Open the stream: the Set Chunk Size control message when the chunk size
    /// is not the default, then the `onMetaData` and sequence-header messages.
    fn open(&mut self) -> Result<(), RtmpFlvError> {
        if self.opened {
            return Ok(());
        }
        let chunk_size = self.wire_chunk_size()?;
        if !self.opening {
            // The caller announced the size; this writer must still frame at
            // it, since a receiver applies the announcement to every chunk on
            // the connection.
            self.writer.set_chunk_size(ChunkSize::new(chunk_size)?);
            self.opened = true;
            return Ok(());
        }
        if chunk_size != DEFAULT_CHUNK_SIZE_VALUE {
            let chunk_size = ChunkSize::new(chunk_size)?;
            let control = ProtocolControl::SetChunkSize(chunk_size).to_message();
            let bytes = self.writer.write(&control)?;
            self.writer.set_chunk_size(chunk_size);
            self.out.push_back(MuxOutput::Bytes(bytes.into()));
        }
        let tags = self.stream.opening_tags(None).map_err(RtmpFlvError::Flv)?;
        self.push_tags(&tags)?;
        self.opened = true;
        Ok(())
    }

    fn push_tags(&mut self, tags: &[OutTag]) -> Result<(), RtmpFlvError> {
        for tag in tags {
            let message = self.message_for(tag)?;
            let bytes = self.writer.write(&message)?;
            self.out.push_back(MuxOutput::Bytes(bytes.into()));
        }
        Ok(())
    }

    /// Frame one FLV tag body as the RTMP message that carries it (§5.3,
    /// §7.1.4/§7.1.5).
    fn message_for(&self, tag: &OutTag) -> Result<Message, RtmpFlvError> {
        let max_message_len = usize::try_from(MAX_MESSAGE_LEN).unwrap_or(usize::MAX);
        if tag.body().len() > max_message_len {
            return Err(RtmpFlvError::MessageTooLarge {
                len: tag.body().len(),
                max: max_message_len,
            });
        }
        let (chunk_stream_id, message_type_id) = match tag.tag_type() {
            TagType::Video => (CSID_VIDEO, MessageTypeId::Video),
            TagType::Audio => (CSID_AUDIO, MessageTypeId::Audio),
            _ => (CSID_DATA, MessageTypeId::DataAmf0),
        };
        Ok(Message {
            chunk_stream_id: rtmp_runtime::ids::ChunkStreamId::new(chunk_stream_id),
            timestamp: rtmp_runtime::ids::Timestamp::from_millis(tag.timestamp()),
            message_type_id,
            message_stream_id: rtmp_runtime::ids::MessageStreamId::new(self.message_stream_id),
            payload: tag.body().to_vec(),
        })
    }
}

impl Stage for RtmpMuxStage {
    type In<'a> = MuxInput;
    type Out = MuxOutput;
    type Error = RtmpFlvError;

    fn feed(&mut self, input: MuxInput, _now: Timestamp) -> Result<(), RtmpFlvError> {
        match input {
            MuxInput::Track(spec) => {
                if self.opened {
                    return Err(RtmpFlvError::Flv(FlvError::TrackAfterStart));
                }
                self.stream.announce(*spec).map_err(RtmpFlvError::Flv)
            }
            MuxInput::Sample { track, sample } => {
                self.open()?;
                let tags = self.stream.push(track, sample).map_err(RtmpFlvError::Flv)?;
                self.push_tags(&tags)
            }
            MuxInput::Discontinuity { .. } => Ok(()),
            MuxInput::Flush => {
                let tags = self.stream.flush().map_err(RtmpFlvError::Flv)?;
                self.push_tags(&tags)
            }
        }
    }

    fn poll(&mut self) -> Option<MuxOutput> {
        self.out.pop_front()
    }

    fn finish(&mut self) -> Result<(), RtmpFlvError> {
        // A stage that was never fed still rejects a chunk size the writer
        // would clamp: the size is this muxer's own configuration, not
        // something a downstream failure could justify.
        self.wire_chunk_size()?;
        if !self.opened {
            return Ok(());
        }
        let tags = self.stream.flush().map_err(RtmpFlvError::Flv)?;
        self.push_tags(&tags)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tag body of 2^24 bytes cannot be described by the UI24 `DataSize`
    /// (§E.4.1): the shared FLV tag codec refuses it.
    #[test]
    fn a_tag_body_past_the_ui24_data_size_is_refused() {
        use broadcast_common::Serialize as _;
        let body = alloc::vec![0u8; 1 << 24];
        let err = rtmp_runtime::flv::FlvTag::new(rtmp_runtime::flv::TagType::Video, 0, &body)
            .try_to_bytes()
            .unwrap_err();
        assert!(
            matches!(
                err,
                rtmp_runtime::RtmpError::FieldOverflow(broadcast_common::len::FieldOverflow {
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
            assert_ne!(media, rtmp_runtime::message::CONTROL_CHUNK_STREAM_ID.get());
        }
        assert_ne!(CSID_AUDIO, CSID_VIDEO);
        assert_ne!(CSID_AUDIO, CSID_DATA);
        assert_ne!(CSID_VIDEO, CSID_DATA);
    }
}
