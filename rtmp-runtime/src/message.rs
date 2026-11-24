//! RTMP message assembly from chunks, protocol control messages, and the
//! message type catalogue (Adobe RTMP 1.0 §5.4, §6, §7.1).
//!
//! See `private/specs-verified/rtmp/rtmp-runtime/rtmp.md` §4 (Protocol Control Messages), §5
//! (RTMP Message Format + User Control, §6.1/§6.2), and §6 (RTMP Message
//! Types, §7.1) for the wire layout.
//!
//! This module adds typed interpretation on top of the [`crate::chunk::Message`]
//! carrier: the message type id ([`crate::chunk::Message::message_type_id`], a
//! [`MessageTypeId`]), the Chunk-Stream-layer protocol control messages
//! ([`ProtocolControl`], §4/§5.4), and the streaming-layer user control events
//! ([`UserControl`], §5.3/§6.2/§6.7).

use broadcast_common::error::BufferTooShort;
use broadcast_common::len::need_in;
use broadcast_common::{Parse, Serialize};
use core::time::Duration;

use crate::RtmpError;
use crate::chunk::{MAX_CHUNK_SIZE, Message};
use crate::ids::{ChunkStreamId, MessageStreamId, SequenceNumber, Timestamp};
use alloc::vec::Vec;

type Result<T> = core::result::Result<T, RtmpError>;

// ── Message type ids (§6 / §7.1) ────────────────────────────────────────

/// RTMP message type id (§6, RTMP Message Types §7.1; the `Message Type ID`
/// field of the chunk message header, §5.3.1). Ids `1..=6` are
/// Chunk-Stream-layer protocol control (§5.4); the rest are streaming-layer
/// message types (§6/§7.1). A value §7.1 does not define is kept in
/// [`MessageTypeId::Other`], since a peer may send one and the payload is
/// still carried.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum MessageTypeId {
    /// `1` — Set Chunk Size (§5.4.1).
    SetChunkSize,
    /// `2` — Abort Message (§5.4.2).
    Abort,
    /// `3` — Acknowledgement (§5.4.3).
    Acknowledgement,
    /// `4` — User Control Message (§6.2/§7.1.7).
    UserControl,
    /// `5` — Window Acknowledgement Size (§5.4.4).
    WindowAckSize,
    /// `6` — Set Peer Bandwidth (§5.4.5).
    SetPeerBandwidth,
    /// `8` — Audio Message (§7.1.4).
    Audio,
    /// `9` — Video Message (§7.1.5).
    Video,
    /// `15` — Data Message, AMF3-encoded (§7.1.2).
    DataAmf3,
    /// `17` — Command Message, AMF3-encoded (§7.1.1).
    CommandAmf3,
    /// `18` — Data Message, AMF0-encoded (§7.1.2).
    DataAmf0,
    /// `20` — Command Message, AMF0-encoded (§7.1.1).
    CommandAmf0,
    /// `22` — Aggregate Message (§7.1.6).
    Aggregate,
    /// A message type id §7.1 does not define.
    Other(u8),
}

impl MessageTypeId {
    /// Decode a wire message type id (lossless: an undefined id becomes
    /// [`MessageTypeId::Other`]).
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => MessageTypeId::SetChunkSize,
            2 => MessageTypeId::Abort,
            3 => MessageTypeId::Acknowledgement,
            4 => MessageTypeId::UserControl,
            5 => MessageTypeId::WindowAckSize,
            6 => MessageTypeId::SetPeerBandwidth,
            8 => MessageTypeId::Audio,
            9 => MessageTypeId::Video,
            15 => MessageTypeId::DataAmf3,
            17 => MessageTypeId::CommandAmf3,
            18 => MessageTypeId::DataAmf0,
            20 => MessageTypeId::CommandAmf0,
            22 => MessageTypeId::Aggregate,
            other => MessageTypeId::Other(other),
        }
    }

    /// The wire value.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            MessageTypeId::SetChunkSize => 1,
            MessageTypeId::Abort => 2,
            MessageTypeId::Acknowledgement => 3,
            MessageTypeId::UserControl => 4,
            MessageTypeId::WindowAckSize => 5,
            MessageTypeId::SetPeerBandwidth => 6,
            MessageTypeId::Audio => 8,
            MessageTypeId::Video => 9,
            MessageTypeId::DataAmf3 => 15,
            MessageTypeId::CommandAmf3 => 17,
            MessageTypeId::DataAmf0 => 18,
            MessageTypeId::CommandAmf0 => 20,
            MessageTypeId::Aggregate => 22,
            MessageTypeId::Other(v) => v,
        }
    }

    /// Spec label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            MessageTypeId::SetChunkSize => "set chunk size",
            MessageTypeId::Abort => "abort",
            MessageTypeId::Acknowledgement => "acknowledgement",
            MessageTypeId::UserControl => "user control",
            MessageTypeId::WindowAckSize => "window acknowledgement size",
            MessageTypeId::SetPeerBandwidth => "set peer bandwidth",
            MessageTypeId::Audio => "audio",
            MessageTypeId::Video => "video",
            MessageTypeId::DataAmf3 => "data (AMF3)",
            MessageTypeId::CommandAmf3 => "command (AMF3)",
            MessageTypeId::DataAmf0 => "data (AMF0)",
            MessageTypeId::CommandAmf0 => "command (AMF0)",
            MessageTypeId::Aggregate => "aggregate",
            MessageTypeId::Other(_) => "other",
        }
    }
}

impl From<u8> for MessageTypeId {
    fn from(v: u8) -> Self {
        MessageTypeId::from_u8(v)
    }
}

impl From<MessageTypeId> for u8 {
    fn from(id: MessageTypeId) -> Self {
        id.to_u8()
    }
}

broadcast_common::impl_spec_display!(MessageTypeId, Other);

/// Chunk stream id protocol control messages and user control messages
/// MUST/SHOULD be sent on (§4, §5.3).
pub const CONTROL_CHUNK_STREAM_ID: ChunkStreamId = ChunkStreamId::new(2);
/// Message stream id protocol control messages MUST use, and user control
/// messages SHOULD use (§4, §5.3): the control stream.
pub const CONTROL_MESSAGE_STREAM_ID: MessageStreamId = MessageStreamId::new(0);

/// Byte width of a `u32` protocol control field (chunk size, chunk stream
/// id, sequence number, window ack size).
const CONTROL_FIELD_LEN: usize = core::mem::size_of::<u32>();
/// Byte width of the Set Peer Bandwidth payload: window size (4) + limit
/// type (1).
const SET_PEER_BANDWIDTH_LEN: usize = CONTROL_FIELD_LEN + 1;

/// Bit mask isolating the reserved top bit of the Set Chunk Size payload
/// (§5.4.1: 1 reserved bit, MUST be 0, + 31-bit chunk size).
const SET_CHUNK_SIZE_RESERVED_MASK: u32 = 0x8000_0000;
/// Bit mask isolating the 31-bit chunk size field of the Set Chunk Size
/// payload.
const SET_CHUNK_SIZE_VALUE_MASK: u32 = 0x7FFF_FFFF;

/// Payload length of a Set Buffer Length user-control event: stream id and
/// buffer length, both u32 (RTMP §5.4.3).
const SET_BUFFER_LENGTH_LEN: usize = 2 * CONTROL_FIELD_LEN;

fn read_u32_be(b: &[u8]) -> u32 {
    let mut field = [0_u8; CONTROL_FIELD_LEN];
    field.copy_from_slice(&b[..CONTROL_FIELD_LEN]);
    u32::from_be_bytes(field)
}

fn need_u32(bytes: &[u8], what: &'static str) -> Result<u32> {
    if bytes.len() < CONTROL_FIELD_LEN {
        return Err(RtmpError::BufferTooShort(BufferTooShort {
            need: CONTROL_FIELD_LEN,
            have: bytes.len(),
            what,
        }));
    }
    Ok(read_u32_be(bytes))
}

// ── Set Peer Bandwidth Limit Type (§5.4.5) ──────────────────────────────

/// Set Peer Bandwidth's `Limit Type` byte (§5.4.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum LimitType {
    /// Peer SHOULD limit output to exactly the indicated window.
    Hard,
    /// Peer SHOULD limit output to the indicated window or its current
    /// limit, whichever is smaller.
    Soft,
    /// If the previous Limit Type was Hard, treat as Hard; otherwise
    /// ignore this message.
    Dynamic,
}

impl LimitType {
    /// The spec token for this limit type.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            LimitType::Hard => "hard",
            LimitType::Soft => "soft",
            LimitType::Dynamic => "dynamic",
        }
    }

    /// Decode the wire byte (0..=2) into a [`LimitType`].
    ///
    /// # Errors
    /// [`RtmpError::InvalidLimitType`] if `v` is not in `0..=2`.
    pub const fn from_u8(v: u8) -> core::result::Result<Self, RtmpError> {
        match v {
            0 => Ok(LimitType::Hard),
            1 => Ok(LimitType::Soft),
            2 => Ok(LimitType::Dynamic),
            value => Err(RtmpError::InvalidLimitType { value }),
        }
    }

    /// Encode this limit type back to its wire byte (0..=2).
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            LimitType::Hard => 0,
            LimitType::Soft => 1,
            LimitType::Dynamic => 2,
        }
    }
}

broadcast_common::impl_spec_display!(LimitType);

/// A validated RTMP chunk size (§5.4.1): `1..=0x7FFF_FFFF`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChunkSize(u32);

impl ChunkSize {
    /// Minimum valid chunk size.
    pub const MIN: ChunkSize = ChunkSize(1);

    /// A size this crate's own constants already validated. Never called with
    /// an unchecked value.
    pub(crate) const fn new_unchecked(value: u32) -> Self {
        Self(value)
    }
    /// Maximum valid chunk size.
    pub const MAX: u32 = 0x7FFF_FFFF;

    /// Construct a validated chunk size.
    pub fn new(value: u32) -> Result<Self> {
        Self::try_new(value)
    }

    /// Construct a validated chunk size: `1..=0x7FFF_FFFF` (the 31-bit wire
    /// field, §5.4.1) and within this engine's [`MAX_CHUNK_SIZE`] ceiling.
    ///
    /// The ceiling is enforced here rather than by clamping later, so the
    /// engine never adopts a chunk size other than the one announced.
    pub const fn try_new(value: u32) -> Result<Self> {
        if value == 0 || value & SET_CHUNK_SIZE_RESERVED_MASK != 0 {
            return Err(RtmpError::chunk_size_out_of_range(value));
        }
        if value > MAX_CHUNK_SIZE {
            return Err(RtmpError::chunk_size_out_of_range(value));
        }
        Ok(Self(value))
    }

    #[must_use]
    /// The raw chunk size.
    pub const fn value(self) -> u32 {
        self.0
    }
}

impl TryFrom<u32> for ChunkSize {
    type Error = RtmpError;
    fn try_from(value: u32) -> Result<Self> {
        Self::new(value)
    }
}

impl From<ChunkSize> for u32 {
    fn from(c: ChunkSize) -> Self {
        c.0
    }
}

// ── Protocol control messages (§4 / §5.4) ───────────────────────────────

/// A Chunk-Stream-layer protocol control message (§4, spec §5.4):
/// message type ids `1`, `2`, `3`, `5`, `6`. MUST use message stream id 0
/// and chunk stream id 2 ([`CONTROL_MESSAGE_STREAM_ID`] /
/// [`CONTROL_CHUNK_STREAM_ID`]); effective immediately on receipt.
///
/// `#[non_exhaustive]`: `§4`'s protocol control catalogue is closed today,
/// but this mirrors [`UserControl`]/[`crate::amf0::Amf0Value`] so a future
/// addition never breaks an existing `match`.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolControl {
    /// Set Chunk Size (§5.4.1): the new maximum chunk size (`1..=0x7FFF_FFFF`).
    SetChunkSize(ChunkSize),
    /// Abort Message (§5.4.2): discard any partially-received message on
    /// this chunk stream id.
    Abort {
        /// The chunk stream id whose in-progress message should be
        /// discarded.
        chunk_stream_id: u32,
    },
    /// Acknowledgement (§5.4.3): total bytes received so far.
    Acknowledgement(SequenceNumber),
    /// Window Acknowledgement Size (§5.4.4): the sender's advertised
    /// window size.
    WindowAckSize(u32),
    /// Set Peer Bandwidth (§5.4.5): limit the peer's output bandwidth.
    SetPeerBandwidth {
        /// The acknowledgement window size to limit the peer to.
        ack_window_size: u32,
        /// How strictly the peer should observe the limit.
        limit_type: LimitType,
    },
}

impl ProtocolControl {
    /// The spec token for this protocol control message.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            ProtocolControl::SetChunkSize(_) => "set chunk size",
            ProtocolControl::Abort { .. } => "abort message",
            ProtocolControl::Acknowledgement(_) => "acknowledgement",
            ProtocolControl::WindowAckSize(_) => "window acknowledgement size",
            ProtocolControl::SetPeerBandwidth { .. } => "set peer bandwidth",
        }
    }

    /// This variant's message type id (§6/§7.1).
    #[must_use]
    pub fn message_type_id(&self) -> MessageTypeId {
        match self {
            ProtocolControl::SetChunkSize(_) => MessageTypeId::SetChunkSize,
            ProtocolControl::Abort { .. } => MessageTypeId::Abort,
            ProtocolControl::Acknowledgement(_) => MessageTypeId::Acknowledgement,
            ProtocolControl::WindowAckSize(_) => MessageTypeId::WindowAckSize,
            ProtocolControl::SetPeerBandwidth { .. } => MessageTypeId::SetPeerBandwidth,
        }
    }

    /// Interpret an already-reassembled [`Message`] as a protocol control
    /// message, dispatching on `message.message_type_id`.
    ///
    /// Returns `Ok(None)` if `message.message_type_id` is not one of the
    /// protocol control ids (`1`, `2`, `3`, `5`, `6`) — the caller should
    /// then dispatch it elsewhere (user control, audio/video, command, …).
    ///
    /// # Errors
    /// [`RtmpError::BufferTooShort`] if the payload is shorter than the
    /// message type requires; [`RtmpError::Malformed`] if a field violates
    /// its wire constraint (reserved bit set, out-of-range limit type).
    pub fn from_message(message: &Message) -> Result<Option<Self>> {
        Self::from_payload(message.message_type_id, &message.payload)
    }

    /// Parse a protocol control payload given its message type id. Not a
    /// [`Parse`] impl: unlike every other wire type in this crate, a
    /// protocol control payload alone is ambiguous (e.g. a bare 4-byte
    /// payload is `SetChunkSize`, `Abort`, `Acknowledgement`, or
    /// `WindowAckSize` depending on the message type id carried alongside
    /// it in the [`Message`] header) — the type id is required context.
    ///
    /// # Errors
    /// See [`ProtocolControl::from_message`].
    pub fn from_payload(message_type_id: MessageTypeId, payload: &[u8]) -> Result<Option<Self>> {
        match message_type_id {
            MessageTypeId::SetChunkSize => {
                let raw = need_u32(payload, "set chunk size payload")?;
                if raw & SET_CHUNK_SIZE_RESERVED_MASK != 0 {
                    return Err(RtmpError::chunk_size_out_of_range(raw));
                }
                Ok(Some(ProtocolControl::SetChunkSize(ChunkSize::try_new(
                    raw & SET_CHUNK_SIZE_VALUE_MASK,
                )?)))
            }
            MessageTypeId::Abort => {
                let chunk_stream_id = need_u32(payload, "abort message payload")?;
                Ok(Some(ProtocolControl::Abort { chunk_stream_id }))
            }
            MessageTypeId::Acknowledgement => {
                let sequence_number = need_u32(payload, "acknowledgement payload")?;
                Ok(Some(ProtocolControl::Acknowledgement(SequenceNumber::new(
                    sequence_number,
                ))))
            }
            MessageTypeId::WindowAckSize => {
                let window = need_u32(payload, "window acknowledgement size payload")?;
                Ok(Some(ProtocolControl::WindowAckSize(window)))
            }
            MessageTypeId::SetPeerBandwidth => {
                need_in(
                    payload,
                    SET_PEER_BANDWIDTH_LEN,
                    "set peer bandwidth payload",
                )?;
                let ack_window_size = read_u32_be(&payload[0..CONTROL_FIELD_LEN]);
                let limit_type = LimitType::from_u8(payload[CONTROL_FIELD_LEN])?;
                Ok(Some(ProtocolControl::SetPeerBandwidth {
                    ack_window_size,
                    limit_type,
                }))
            }
            _ => Ok(None),
        }
    }

    /// The message payload, built without a fallible length check: the
    /// result length is the type's own [`Serialize::serialized_len`].
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = alloc::vec![0u8; self.serialized_len()];
        match *self {
            ProtocolControl::SetChunkSize(size) => {
                buf[0..CONTROL_FIELD_LEN].copy_from_slice(&size.value().to_be_bytes());
            }
            ProtocolControl::Abort { chunk_stream_id } => {
                buf[0..CONTROL_FIELD_LEN].copy_from_slice(&chunk_stream_id.to_be_bytes());
            }
            ProtocolControl::Acknowledgement(sequence_number) => {
                buf[0..CONTROL_FIELD_LEN].copy_from_slice(&sequence_number.get().to_be_bytes());
            }
            ProtocolControl::WindowAckSize(window) => {
                buf[0..CONTROL_FIELD_LEN].copy_from_slice(&window.to_be_bytes());
            }
            ProtocolControl::SetPeerBandwidth {
                ack_window_size,
                limit_type,
            } => {
                buf[0..CONTROL_FIELD_LEN].copy_from_slice(&ack_window_size.to_be_bytes());
                buf[CONTROL_FIELD_LEN] = limit_type.to_u8();
            }
        }
        buf
    }

    /// Wrap this protocol control message in a [`Message`] ready to hand to
    /// [`crate::chunk::ChunkWriter`] — chunk stream id
    /// [`CONTROL_CHUNK_STREAM_ID`], message stream id
    /// [`CONTROL_MESSAGE_STREAM_ID`], timestamp `0` (protocol control
    /// messages take effect immediately; timestamps are not meaningful).
    #[must_use]
    pub fn to_message(&self) -> Message {
        let payload = self.to_bytes();
        Message {
            chunk_stream_id: CONTROL_CHUNK_STREAM_ID,
            timestamp: Timestamp::ZERO,
            message_type_id: self.message_type_id(),
            message_stream_id: CONTROL_MESSAGE_STREAM_ID,
            payload,
        }
    }
}

broadcast_common::impl_spec_display!(ProtocolControl);

impl Serialize for ProtocolControl {
    type Error = RtmpError;

    fn serialized_len(&self) -> usize {
        match self {
            ProtocolControl::SetChunkSize(_)
            | ProtocolControl::Abort { .. }
            | ProtocolControl::Acknowledgement(_)
            | ProtocolControl::WindowAckSize(_) => CONTROL_FIELD_LEN,
            ProtocolControl::SetPeerBandwidth { .. } => SET_PEER_BANDWIDTH_LEN,
        }
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let written = self.serialized_len();
        need_in(buf, written, "protocol control payload output")?;
        match *self {
            ProtocolControl::SetChunkSize(size) => {
                buf[0..CONTROL_FIELD_LEN].copy_from_slice(&size.value().to_be_bytes());
            }
            ProtocolControl::Abort { chunk_stream_id } => {
                buf[0..CONTROL_FIELD_LEN].copy_from_slice(&chunk_stream_id.to_be_bytes());
            }
            ProtocolControl::Acknowledgement(sequence_number) => {
                buf[0..CONTROL_FIELD_LEN].copy_from_slice(&sequence_number.get().to_be_bytes());
            }
            ProtocolControl::WindowAckSize(window) => {
                buf[0..CONTROL_FIELD_LEN].copy_from_slice(&window.to_be_bytes());
            }
            ProtocolControl::SetPeerBandwidth {
                ack_window_size,
                limit_type,
            } => {
                buf[0..CONTROL_FIELD_LEN].copy_from_slice(&ack_window_size.to_be_bytes());
                buf[CONTROL_FIELD_LEN] = limit_type.to_u8();
            }
        }
        Ok(written)
    }
}

// ── User control messages (§5.3 / §6.2 / §6.7) ──────────────────────────

/// Byte width of the User Control Message's 16-bit `Event Type` field.
const EVENT_TYPE_LEN: usize = 2;

/// User Control Message event types (§6.7, spec §7.1.7).
mod event_type {
    pub const STREAM_BEGIN: u16 = 0;
    pub const STREAM_EOF: u16 = 1;
    pub const STREAM_DRY: u16 = 2;
    pub const SET_BUFFER_LENGTH: u16 = 3;
    pub const STREAM_IS_RECORDED: u16 = 4;
    // Event value 5 is not defined by the spec.
    pub const PING_REQUEST: u16 = 6;
    pub const PING_RESPONSE: u16 = 7;
}

/// A User Control Message event (§5.3, message type id 4; event-data
/// formats per §6.7, spec §7.1.7). SHOULD use message stream id 0 and,
/// over the chunk stream, csid 2. Effective on receipt; timestamps
/// ignored.
///
/// `#[non_exhaustive]`: §6.7's event-type catalogue (event value 5 is
/// already an unassigned gap) can grow; a new event type must not be a
/// breaking change for existing `match` callers.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserControl {
    /// Stream Begin (event 0, server→client): the stream is now
    /// functional/usable. By default sent on stream id 0 right after a
    /// successful `connect`.
    StreamBegin(u32),
    /// Stream EOF (event 1, server→client): playback requested on this
    /// stream has ended.
    StreamEof(u32),
    /// StreamDry (event 2, server→client): no more data on the stream
    /// (server-detected idle).
    StreamDry(u32),
    /// SetBufferLength (event 3, client→server): the client's playback
    /// buffer size, sent before the server starts sending the stream.
    SetBufferLength {
        /// The stream this buffer length applies to.
        stream_id: u32,
        /// The client's buffer length, a span carried on the wire as whole
        /// milliseconds.
        buffer: Duration,
    },
    /// StreamIsRecorded (event 4, server→client): the stream is a
    /// recorded (not live) stream.
    StreamIsRecorded(u32),
    /// PingRequest (event 6, server→client): test reachability; the
    /// client MUST reply with PingResponse.
    PingRequest(u32),
    /// PingResponse (event 7, client→server): reply to PingRequest,
    /// echoing its timestamp.
    PingResponse(u32),
}

impl UserControl {
    /// The spec token for this user control event.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            UserControl::StreamBegin(_) => "stream begin",
            UserControl::StreamEof(_) => "stream eof",
            UserControl::StreamDry(_) => "stream dry",
            UserControl::SetBufferLength { .. } => "set buffer length",
            UserControl::StreamIsRecorded(_) => "stream is recorded",
            UserControl::PingRequest(_) => "ping request",
            UserControl::PingResponse(_) => "ping response",
        }
    }

    /// This event's 16-bit event type value (§6.7).
    #[must_use]
    pub fn event_type(&self) -> u16 {
        match self {
            UserControl::StreamBegin(_) => event_type::STREAM_BEGIN,
            UserControl::StreamEof(_) => event_type::STREAM_EOF,
            UserControl::StreamDry(_) => event_type::STREAM_DRY,
            UserControl::SetBufferLength { .. } => event_type::SET_BUFFER_LENGTH,
            UserControl::StreamIsRecorded(_) => event_type::STREAM_IS_RECORDED,
            UserControl::PingRequest(_) => event_type::PING_REQUEST,
            UserControl::PingResponse(_) => event_type::PING_RESPONSE,
        }
    }

    /// The message payload, built without a fallible length check: the
    /// result length is the type's own [`Serialize::serialized_len`].
    ///
    /// # Errors
    /// [`RtmpError::FieldOverflow`] if a `SetBufferLength` span is longer
    /// than the message's 32-bit millisecond field.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut buf = alloc::vec![0u8; self.serialized_len()];
        buf[0..EVENT_TYPE_LEN].copy_from_slice(&self.event_type().to_be_bytes());
        let data = &mut buf[EVENT_TYPE_LEN..];
        match *self {
            UserControl::StreamBegin(stream_id)
            | UserControl::StreamEof(stream_id)
            | UserControl::StreamDry(stream_id)
            | UserControl::StreamIsRecorded(stream_id)
            | UserControl::PingRequest(stream_id)
            | UserControl::PingResponse(stream_id) => {
                data[0..CONTROL_FIELD_LEN].copy_from_slice(&stream_id.to_be_bytes());
            }
            UserControl::SetBufferLength { stream_id, buffer } => {
                data[0..CONTROL_FIELD_LEN].copy_from_slice(&stream_id.to_be_bytes());
                let millis = u32::try_from(buffer.as_millis()).map_err(|_| {
                    RtmpError::FieldOverflow(broadcast_common::len::FieldOverflow {
                        field: "set_buffer_length.buffer",
                        value: buffer.as_millis() as u64,
                        max: u64::from(u32::MAX),
                    })
                })?;
                data[CONTROL_FIELD_LEN..2 * CONTROL_FIELD_LEN]
                    .copy_from_slice(&millis.to_be_bytes());
            }
        }
        Ok(buf)
    }

    /// Wrap this user control event in a [`Message`] ready to hand to
    /// [`crate::chunk::ChunkWriter`] — chunk stream id
    /// [`CONTROL_CHUNK_STREAM_ID`], message stream id
    /// [`CONTROL_MESSAGE_STREAM_ID`], timestamp `0` (effective on receipt;
    /// timestamps are not meaningful).
    ///
    /// # Errors
    /// See [`UserControl::to_bytes`].
    pub fn to_message(&self) -> Result<Message> {
        Ok(Message {
            chunk_stream_id: CONTROL_CHUNK_STREAM_ID,
            timestamp: Timestamp::ZERO,
            message_type_id: MessageTypeId::UserControl,
            message_stream_id: CONTROL_MESSAGE_STREAM_ID,
            payload: self.to_bytes()?,
        })
    }
}

broadcast_common::impl_spec_display!(UserControl);

impl<'a> Parse<'a> for UserControl {
    type Error = RtmpError;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(bytes, EVENT_TYPE_LEN, "user control event type")?;
        let event = u16::from_be_bytes([bytes[0], bytes[1]]);
        let data = &bytes[EVENT_TYPE_LEN..];
        match event {
            event_type::STREAM_BEGIN => Ok(UserControl::StreamBegin(need_u32(
                data,
                "stream begin event data",
            )?)),
            event_type::STREAM_EOF => Ok(UserControl::StreamEof(need_u32(
                data,
                "stream eof event data",
            )?)),
            event_type::STREAM_DRY => Ok(UserControl::StreamDry(need_u32(
                data,
                "stream dry event data",
            )?)),
            event_type::SET_BUFFER_LENGTH => {
                need_in(data, SET_BUFFER_LENGTH_LEN, "set buffer length event data")?;
                Ok(UserControl::SetBufferLength {
                    stream_id: read_u32_be(&data[0..CONTROL_FIELD_LEN]),
                    buffer: Duration::from_millis(u64::from(read_u32_be(
                        &data[CONTROL_FIELD_LEN..SET_BUFFER_LENGTH_LEN],
                    ))),
                })
            }
            event_type::STREAM_IS_RECORDED => Ok(UserControl::StreamIsRecorded(need_u32(
                data,
                "stream is recorded event data",
            )?)),
            event_type::PING_REQUEST => Ok(UserControl::PingRequest(need_u32(
                data,
                "ping request event data",
            )?)),
            event_type::PING_RESPONSE => Ok(UserControl::PingResponse(need_u32(
                data,
                "ping response event data",
            )?)),
            other => Err(RtmpError::UnsupportedUserControlEvent { value: other }),
        }
    }
}

impl Serialize for UserControl {
    type Error = RtmpError;

    fn serialized_len(&self) -> usize {
        let data_len = match self {
            UserControl::SetBufferLength { .. } => SET_BUFFER_LENGTH_LEN,
            _ => CONTROL_FIELD_LEN,
        };
        EVENT_TYPE_LEN + data_len
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let written = self.serialized_len();
        need_in(buf, written, "user control event output")?;
        buf[0..EVENT_TYPE_LEN].copy_from_slice(&self.event_type().to_be_bytes());
        let data = &mut buf[EVENT_TYPE_LEN..written];
        match *self {
            UserControl::StreamBegin(stream_id)
            | UserControl::StreamEof(stream_id)
            | UserControl::StreamDry(stream_id)
            | UserControl::StreamIsRecorded(stream_id)
            | UserControl::PingRequest(stream_id)
            | UserControl::PingResponse(stream_id) => {
                data[0..CONTROL_FIELD_LEN].copy_from_slice(&stream_id.to_be_bytes());
            }
            UserControl::SetBufferLength { stream_id, buffer } => {
                let millis = u32::try_from(buffer.as_millis()).map_err(|_| {
                    RtmpError::FieldOverflow(broadcast_common::len::FieldOverflow {
                        field: "set_buffer_length.buffer",
                        value: buffer.as_millis() as u64,
                        max: u64::from(u32::MAX),
                    })
                })?;
                data[0..CONTROL_FIELD_LEN].copy_from_slice(&stream_id.to_be_bytes());
                data[CONTROL_FIELD_LEN..2 * CONTROL_FIELD_LEN]
                    .copy_from_slice(&millis.to_be_bytes());
            }
        }
        Ok(written)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(message_type_id: MessageTypeId, payload: Vec<u8>) -> Message {
        Message {
            chunk_stream_id: CONTROL_CHUNK_STREAM_ID,
            timestamp: Timestamp::ZERO,
            message_type_id,
            message_stream_id: CONTROL_MESSAGE_STREAM_ID,
            payload,
        }
    }

    // ── LimitType ────────────────────────────────────────────────────────

    /// An invalid chunk size can no longer reach a [`Message`]: the
    /// `ChunkSize` constructor is the only way in (§5.4.1), and once built
    /// `to_message` is total — so this asserts both halves end to end, the
    /// rejection and then the framed bytes of a valid size.
    #[test]
    fn invalid_chunk_size_never_reaches_a_message() {
        assert!(matches!(
            ChunkSize::try_new(0),
            Err(RtmpError::ChunkSizeOutOfRange { value: 0 })
        ));
        assert!(matches!(
            ChunkSize::try_new(0x8000_0000),
            Err(RtmpError::ChunkSizeOutOfRange { value: 0x8000_0000 })
        ));
        let msg = ProtocolControl::SetChunkSize(ChunkSize::try_new(4096).unwrap()).to_message();
        assert_eq!(msg.message_type_id, MessageTypeId::SetChunkSize);
        assert_eq!(msg.payload, vec![0x00, 0x00, 0x10, 0x00]);
    }

    #[test]
    fn limit_type_round_trip_and_name() {
        for (byte, lt, name) in [
            (0u8, LimitType::Hard, "hard"),
            (1, LimitType::Soft, "soft"),
            (2, LimitType::Dynamic, "dynamic"),
        ] {
            let parsed = LimitType::from_u8(byte).unwrap();
            assert_eq!(parsed, lt);
            assert_eq!(parsed.to_u8(), byte);
            assert_eq!(parsed.name(), name);
            assert_eq!(parsed.to_string(), name);
        }
    }

    #[test]
    fn limit_type_out_of_range_is_malformed() {
        assert!(matches!(
            LimitType::from_u8(3),
            Err(RtmpError::InvalidLimitType { value: 3 })
        ));
    }

    // ── ProtocolControl round-trips ──────────────────────────────────────

    fn protocol_control_round_trip(pc: ProtocolControl) {
        let bytes = pc.try_to_bytes().unwrap();
        let parsed = ProtocolControl::from_payload(pc.message_type_id(), &bytes)
            .unwrap()
            .expect("known protocol control type id");
        assert_eq!(parsed, pc);

        // parse -> serialize -> byte-identical
        let msg = message(pc.message_type_id(), bytes.clone());
        let via_message = ProtocolControl::from_message(&msg).unwrap().unwrap();
        assert_eq!(via_message, pc);
        assert_eq!(via_message.try_to_bytes().unwrap(), bytes);
    }

    #[test]
    fn set_chunk_size_round_trips() {
        protocol_control_round_trip(ProtocolControl::SetChunkSize(
            ChunkSize::try_new(4096).unwrap(),
        ));
    }

    #[test]
    fn abort_round_trips() {
        protocol_control_round_trip(ProtocolControl::Abort { chunk_stream_id: 7 });
    }

    #[test]
    fn acknowledgement_round_trips() {
        protocol_control_round_trip(ProtocolControl::Acknowledgement(SequenceNumber::new(
            1_048_576,
        )));
    }

    #[test]
    fn window_ack_size_round_trips() {
        protocol_control_round_trip(ProtocolControl::WindowAckSize(2_500_000));
    }

    #[test]
    fn set_peer_bandwidth_round_trips_every_limit_type() {
        for limit_type in [LimitType::Hard, LimitType::Soft, LimitType::Dynamic] {
            protocol_control_round_trip(ProtocolControl::SetPeerBandwidth {
                ack_window_size: 2_500_000,
                limit_type,
            });
        }
    }

    #[test]
    fn protocol_control_to_bytes_matches_the_serializer() {
        for pc in [
            ProtocolControl::SetChunkSize(ChunkSize::try_new(4096).unwrap()),
            ProtocolControl::Abort { chunk_stream_id: 7 },
            ProtocolControl::Acknowledgement(SequenceNumber::new(1_048_576)),
            ProtocolControl::WindowAckSize(2_500_000),
            ProtocolControl::SetPeerBandwidth {
                ack_window_size: 2_500_000,
                limit_type: LimitType::Hard,
            },
            ProtocolControl::SetPeerBandwidth {
                ack_window_size: 2_500_000,
                limit_type: LimitType::Soft,
            },
            ProtocolControl::SetPeerBandwidth {
                ack_window_size: 2_500_000,
                limit_type: LimitType::Dynamic,
            },
        ] {
            assert_eq!(pc.to_bytes(), pc.try_to_bytes().unwrap());
            assert_eq!(pc.to_message().payload, pc.to_bytes());
        }
    }

    #[test]
    fn set_chunk_size_reserved_top_bit_rejected_on_parse() {
        let bytes = 0x8000_1000u32.to_be_bytes().to_vec();
        assert!(matches!(
            ProtocolControl::from_payload(MessageTypeId::SetChunkSize, &bytes),
            Err(RtmpError::ChunkSizeOutOfRange { value: 0x8000_1000 })
        ));
    }

    #[test]
    fn set_chunk_size_zero_rejected() {
        let bytes = 0u32.to_be_bytes().to_vec();
        assert!(matches!(
            ProtocolControl::from_payload(MessageTypeId::SetChunkSize, &bytes),
            Err(RtmpError::ChunkSizeOutOfRange { value: 0 })
        ));
        assert!(ChunkSize::try_new(0).is_err());
    }

    #[test]
    fn set_chunk_size_serialize_layout_matches_spec() {
        // §5.4.1: reserved bit 0, 31-bit chunk size, big-endian.
        let bytes = ProtocolControl::SetChunkSize(ChunkSize::try_new(1).unwrap())
            .try_to_bytes()
            .unwrap();
        assert_eq!(bytes, vec![0x00, 0x00, 0x00, 0x01]);
    }

    #[test]
    fn set_peer_bandwidth_serialize_layout_matches_spec() {
        let bytes = ProtocolControl::SetPeerBandwidth {
            ack_window_size: 0x0002_5000,
            limit_type: LimitType::Dynamic,
        }
        .try_to_bytes()
        .unwrap();
        assert_eq!(bytes, vec![0x00, 0x02, 0x50, 0x00, 0x02]);
    }

    #[test]
    fn set_peer_bandwidth_wrong_limit_type_mapping_would_fail() {
        // Mutation check: swapping Hard/Dynamic's wire values would break this.
        assert_eq!(LimitType::Hard.to_u8(), 0);
        assert_eq!(LimitType::Dynamic.to_u8(), 2);
        assert_ne!(LimitType::Hard.to_u8(), LimitType::Dynamic.to_u8());
    }

    #[test]
    fn from_message_none_for_non_control_type_id() {
        let msg = message(MessageTypeId::Audio, vec![0u8; 4]);
        assert!(ProtocolControl::from_message(&msg).unwrap().is_none());
    }

    #[test]
    fn from_message_some_for_control_type_id() {
        let msg = message(
            MessageTypeId::WindowAckSize,
            1_000_000u32.to_be_bytes().to_vec(),
        );
        assert!(ProtocolControl::from_message(&msg).unwrap().is_some());
    }

    #[test]
    fn to_message_uses_control_csid_and_stream_id() {
        let msg = ProtocolControl::SetChunkSize(ChunkSize::try_new(4096).unwrap()).to_message();
        assert_eq!(msg.chunk_stream_id, CONTROL_CHUNK_STREAM_ID);
        assert_eq!(msg.message_stream_id, CONTROL_MESSAGE_STREAM_ID);
        assert_eq!(msg.message_type_id, MessageTypeId::SetChunkSize);
    }

    #[test]
    fn protocol_control_display_matches_name() {
        assert_eq!(
            ProtocolControl::Acknowledgement(SequenceNumber::new(1)).to_string(),
            ProtocolControl::Acknowledgement(SequenceNumber::new(1)).name()
        );
    }

    // ── UserControl round-trips ──────────────────────────────────────────

    fn user_control_round_trip(uc: UserControl) {
        let bytes = uc.try_to_bytes().unwrap();
        let parsed = UserControl::parse(&bytes).unwrap();
        assert_eq!(parsed, uc);
        assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
    }

    #[test]
    fn stream_begin_round_trips() {
        user_control_round_trip(UserControl::StreamBegin(1));
    }

    #[test]
    fn stream_begin_serialize_layout_matches_spec() {
        // §6.7: event type 0x0000 + 4-byte stream id, big-endian.
        let bytes = UserControl::StreamBegin(1).try_to_bytes().unwrap();
        assert_eq!(bytes, vec![0x00, 0x00, 0x00, 0x00, 0x00, 0x01]);
    }

    #[test]
    fn user_control_to_bytes_matches_the_serializer() {
        for uc in [
            UserControl::StreamBegin(1),
            UserControl::StreamEof(1),
            UserControl::StreamDry(1),
            UserControl::StreamIsRecorded(1),
            UserControl::PingRequest(1),
            UserControl::PingResponse(1),
            UserControl::SetBufferLength {
                stream_id: 1,
                buffer: Duration::from_millis(3_000),
            },
        ] {
            assert_eq!(uc.to_bytes().unwrap(), uc.try_to_bytes().unwrap());
            assert_eq!(uc.to_message().unwrap().payload, uc.to_bytes().unwrap());
        }
    }

    #[test]
    fn stream_eof_round_trips() {
        user_control_round_trip(UserControl::StreamEof(1));
    }

    #[test]
    fn stream_dry_round_trips() {
        user_control_round_trip(UserControl::StreamDry(1));
    }

    #[test]
    fn set_buffer_length_round_trips() {
        user_control_round_trip(UserControl::SetBufferLength {
            stream_id: 1,
            buffer: Duration::from_millis(3000),
        });
    }

    #[test]
    fn stream_is_recorded_round_trips() {
        user_control_round_trip(UserControl::StreamIsRecorded(1));
    }

    #[test]
    fn ping_request_round_trips() {
        user_control_round_trip(UserControl::PingRequest(0x1234_5678));
    }

    #[test]
    fn ping_response_round_trips() {
        user_control_round_trip(UserControl::PingResponse(0x1234_5678));
    }

    #[test]
    fn unrecognised_event_type_is_unsupported() {
        // Event value 5 is not defined by the spec.
        let bytes = [0x00, 0x05, 0x00, 0x00, 0x00, 0x01];
        assert!(matches!(
            UserControl::parse(&bytes),
            Err(RtmpError::UnsupportedUserControlEvent { value: 5 })
        ));
    }

    #[test]
    fn user_control_event_type_wrong_mapping_would_fail() {
        // Mutation check: swapping StreamBegin/StreamEof's event-type
        // values would break this.
        assert_eq!(UserControl::StreamBegin(0).event_type(), 0);
        assert_eq!(UserControl::StreamEof(0).event_type(), 1);
    }

    #[test]
    fn user_control_display_matches_name() {
        assert_eq!(
            UserControl::StreamBegin(1).to_string(),
            UserControl::StreamBegin(1).name()
        );
    }

    #[test]
    fn to_message_uses_control_csid_and_user_control_type_id() {
        let msg = UserControl::StreamBegin(1).to_message().unwrap();
        assert_eq!(msg.chunk_stream_id, CONTROL_CHUNK_STREAM_ID);
        assert_eq!(msg.message_stream_id, CONTROL_MESSAGE_STREAM_ID);
        assert_eq!(msg.message_type_id, MessageTypeId::UserControl);
    }

    #[test]
    fn message_type_ids_round_trip_every_defined_value() {
        for (id, value) in [
            (MessageTypeId::SetChunkSize, 1u8),
            (MessageTypeId::Abort, 2),
            (MessageTypeId::Acknowledgement, 3),
            (MessageTypeId::UserControl, 4),
            (MessageTypeId::WindowAckSize, 5),
            (MessageTypeId::SetPeerBandwidth, 6),
            (MessageTypeId::Audio, 8),
            (MessageTypeId::Video, 9),
            (MessageTypeId::DataAmf3, 15),
            (MessageTypeId::CommandAmf3, 17),
            (MessageTypeId::DataAmf0, 18),
            (MessageTypeId::CommandAmf0, 20),
            (MessageTypeId::Aggregate, 22),
        ] {
            assert_eq!(id.to_u8(), value, "{}", id.name());
            assert_eq!(MessageTypeId::from_u8(value), id);
            assert_eq!(id.to_string(), id.name());
        }
    }

    #[test]
    fn an_undefined_message_type_id_is_kept_as_other_and_displays_its_byte() {
        // 7, 10..=14, 16, 19 and 21 are not in §7.1's table.
        for value in [0u8, 7, 10, 16, 19, 21, 200] {
            assert_eq!(
                MessageTypeId::from_u8(value),
                MessageTypeId::Other(value),
                "type id {value}"
            );
            assert_eq!(MessageTypeId::from_u8(value).to_u8(), value);
        }
        assert_eq!(MessageTypeId::Other(7).name(), "other");
        assert_eq!(MessageTypeId::Other(7).to_string(), "other(0x07)");
    }

    // ── Truncated User Control payloads (audit 22) ────────────────────────

    #[test]
    fn truncated_four_byte_user_control_data_is_rejected() {
        for event in [
            event_type::STREAM_BEGIN,
            event_type::STREAM_EOF,
            event_type::STREAM_DRY,
            event_type::STREAM_IS_RECORDED,
            event_type::PING_REQUEST,
            event_type::PING_RESPONSE,
        ] {
            for data_len in 0..4usize {
                let mut bytes = event.to_be_bytes().to_vec();
                bytes.extend(core::iter::repeat_n(0u8, data_len));
                assert!(
                    matches!(
                        UserControl::parse(&bytes),
                        Err(RtmpError::BufferTooShort(BufferTooShort {
                            need: 4,
                            have,
                            ..
                        })) if have == data_len
                    ),
                    "event 0x{event:04X} with {data_len} data bytes must be rejected"
                );
            }
        }
    }

    #[test]
    fn set_buffer_length_requires_both_four_byte_fields() {
        for data_len in 0..8usize {
            let mut bytes = event_type::SET_BUFFER_LENGTH.to_be_bytes().to_vec();
            bytes.extend(core::iter::repeat_n(0u8, data_len));
            assert!(
                matches!(
                    UserControl::parse(&bytes),
                    Err(RtmpError::BufferTooShort(BufferTooShort {
                        need: 8,
                        have,
                        ..
                    })) if have == data_len
                ),
                "set buffer length with {data_len} data bytes must be rejected"
            );
        }
        // §6.7: event type 0x0003, stream id 9, buffer 0x000003E8 ms.
        let bytes = [0x00, 0x03, 0x00, 0x00, 0x00, 0x09, 0x00, 0x00, 0x03, 0xE8];
        assert_eq!(
            UserControl::parse(&bytes).unwrap(),
            UserControl::SetBufferLength {
                stream_id: 9,
                buffer: Duration::from_millis(1_000),
            }
        );
    }

    #[test]
    fn protocol_control_names_are_pinned() {
        for (pc, name) in [
            (
                ProtocolControl::SetChunkSize(ChunkSize::try_new(4096).unwrap()),
                "set chunk size",
            ),
            (
                ProtocolControl::Abort { chunk_stream_id: 7 },
                "abort message",
            ),
            (
                ProtocolControl::Acknowledgement(SequenceNumber::new(1)),
                "acknowledgement",
            ),
            (
                ProtocolControl::WindowAckSize(2_500_000),
                "window acknowledgement size",
            ),
            (
                ProtocolControl::SetPeerBandwidth {
                    ack_window_size: 2_500_000,
                    limit_type: LimitType::Hard,
                },
                "set peer bandwidth",
            ),
        ] {
            assert_eq!(pc.name(), name);
            assert_eq!(pc.to_string(), name);
        }
    }

    #[test]
    fn user_control_names_are_pinned() {
        for (uc, name) in [
            (UserControl::StreamBegin(1), "stream begin"),
            (UserControl::StreamEof(1), "stream eof"),
            (UserControl::StreamDry(1), "stream dry"),
            (
                UserControl::SetBufferLength {
                    stream_id: 1,
                    buffer: Duration::from_millis(3_000),
                },
                "set buffer length",
            ),
            (UserControl::StreamIsRecorded(1), "stream is recorded"),
            (UserControl::PingRequest(1), "ping request"),
            (UserControl::PingResponse(1), "ping response"),
        ] {
            assert_eq!(uc.name(), name);
            assert_eq!(uc.to_string(), name);
        }
    }

    #[test]
    fn conversions_expose_the_spec_values() {
        // §7.1: Audio(8) and Video(9).
        assert_eq!(u8::from(MessageTypeId::Audio), 8);
        assert_eq!(u8::from(MessageTypeId::Video), 9);
        assert_eq!(u32::from(ChunkSize::new(4096).unwrap()), 4096);
    }

    #[test]
    fn a_header_and_a_message_carry_the_undecoded_message_type_id() {
        use crate::chunk::{ChunkWriter, MessageHeader};

        let header = MessageHeader::Type0 {
            timestamp: Timestamp::ZERO,
            message_length: 4,
            message_type_id: MessageTypeId::Other(99),
            message_stream_id: MessageStreamId::new(1),
        };
        let bytes = header.try_to_bytes().expect("header");
        let (back, consumed) =
            MessageHeader::parse(crate::chunk::Fmt::Type0, &bytes).expect("parse");
        assert_eq!(consumed, bytes.len());
        assert_eq!(back, header);

        let message = message(MessageTypeId::Other(99), vec![0; 4]);
        let wire = ChunkWriter::new().write(&message).expect("write");
        // Basic header (1) + three 24-bit header fields before the type id.
        assert_eq!(wire[7], 99, "the wire carries the raw message type id");
        let mut assembler = crate::chunk::ChunkAssembler::new();
        assert_eq!(assembler.push(&wire).expect("push"), vec![message]);
    }
}
