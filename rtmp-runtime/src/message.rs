//! RTMP message assembly from chunks, protocol control messages, and the
//! message type catalogue (Adobe RTMP 1.0 §5.4, §6, §7.1).
//!
//! See [`docs/rtmp.md`](../docs/rtmp.md) §4 (Protocol Control Messages), §5
//! (RTMP Message Format + User Control, §6.1/§6.2), and §6 (RTMP Message
//! Types, §7.1) for the wire layout.
//!
//! This module adds typed interpretation on top of the [`crate::chunk::Message`]
//! carrier: the message-type-id catalogue ([`msg_type`]), the Chunk-Stream-layer
//! protocol control messages ([`ProtocolControl`], §4/§5.4), and the
//! streaming-layer user control events ([`UserControl`], §5.3/§6.2/§6.7).

use broadcast_common::{Parse, Serialize};

use crate::RtmpError;
use crate::chunk::Message;

type Result<T> = core::result::Result<T, RtmpError>;

// ── Message type ids (§6 / §7.1) ────────────────────────────────────────

/// RTMP message type id catalogue (§6, RTMP Message Types §7.1). Type ids
/// `1..=6` are Chunk-Stream-layer protocol control (this doc's §4, spec
/// §5.4); the rest are streaming-layer message types (§6/§7.1).
pub mod msg_type {
    /// Set Chunk Size (§5.4.1).
    pub const SET_CHUNK_SIZE: u8 = 1;
    /// Abort Message (§5.4.2).
    pub const ABORT: u8 = 2;
    /// Acknowledgement (§5.4.3).
    pub const ACKNOWLEDGEMENT: u8 = 3;
    /// User Control Message (§6.2/§7.1.7).
    pub const USER_CONTROL: u8 = 4;
    /// Window Acknowledgement Size (§5.4.4).
    pub const WINDOW_ACK_SIZE: u8 = 5;
    /// Set Peer Bandwidth (§5.4.5).
    pub const SET_PEER_BANDWIDTH: u8 = 6;
    /// Audio Message (§7.1.4).
    pub const AUDIO: u8 = 8;
    /// Video Message (§7.1.5).
    pub const VIDEO: u8 = 9;
    /// Data Message, AMF3-encoded (§7.1.2).
    pub const DATA_AMF3: u8 = 15;
    /// Command Message, AMF3-encoded (§7.1.1).
    pub const COMMAND_AMF3: u8 = 17;
    /// Data Message, AMF0-encoded (§7.1.2).
    pub const DATA_AMF0: u8 = 18;
    /// Command Message, AMF0-encoded (§7.1.1).
    pub const COMMAND_AMF0: u8 = 20;
    /// Aggregate Message (§7.1.6).
    pub const AGGREGATE: u8 = 22;
}

/// Chunk stream id protocol control messages and user control messages
/// MUST/SHOULD be sent on (§4, §5.3).
pub const CONTROL_CHUNK_STREAM_ID: u32 = 2;
/// Message stream id protocol control messages MUST use, and user control
/// messages SHOULD use (§4, §5.3): the control stream.
pub const CONTROL_MESSAGE_STREAM_ID: u32 = 0;

/// Byte width of a `u32` protocol control field (chunk size, chunk stream
/// id, sequence number, window ack size).
const U32_LEN: usize = 4;
/// Byte width of the Set Peer Bandwidth payload: window size (4) + limit
/// type (1).
const SET_PEER_BANDWIDTH_LEN: usize = U32_LEN + 1;

/// Bit mask isolating the reserved top bit of the Set Chunk Size payload
/// (§5.4.1: 1 reserved bit, MUST be 0, + 31-bit chunk size).
const SET_CHUNK_SIZE_RESERVED_MASK: u32 = 0x8000_0000;
/// Bit mask isolating the 31-bit chunk size field of the Set Chunk Size
/// payload.
const SET_CHUNK_SIZE_VALUE_MASK: u32 = 0x7FFF_FFFF;

fn read_u32_be(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

fn need_u32(bytes: &[u8], what: &'static str) -> Result<u32> {
    if bytes.len() < U32_LEN {
        return Err(RtmpError::BufferTooShort {
            need: U32_LEN,
            have: bytes.len(),
            what,
        });
    }
    Ok(read_u32_be(bytes))
}

// ── Set Peer Bandwidth Limit Type (§5.4.5) ──────────────────────────────

/// Set Peer Bandwidth's `Limit Type` byte (§5.4.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    /// [`RtmpError::Malformed`] if `v` is not in `0..=2`.
    pub const fn from_u8(v: u8) -> core::result::Result<Self, RtmpError> {
        match v {
            0 => Ok(LimitType::Hard),
            1 => Ok(LimitType::Soft),
            2 => Ok(LimitType::Dynamic),
            _ => Err(RtmpError::Malformed {
                what: "set peer bandwidth limit type (must be 0..=2)",
            }),
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
    SetChunkSize(u32),
    /// Abort Message (§5.4.2): discard any partially-received message on
    /// this chunk stream id.
    Abort {
        /// The chunk stream id whose in-progress message should be
        /// discarded.
        chunk_stream_id: u32,
    },
    /// Acknowledgement (§5.4.3): total bytes received so far.
    Acknowledgement(u32),
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
    pub fn message_type_id(&self) -> u8 {
        match self {
            ProtocolControl::SetChunkSize(_) => msg_type::SET_CHUNK_SIZE,
            ProtocolControl::Abort { .. } => msg_type::ABORT,
            ProtocolControl::Acknowledgement(_) => msg_type::ACKNOWLEDGEMENT,
            ProtocolControl::WindowAckSize(_) => msg_type::WINDOW_ACK_SIZE,
            ProtocolControl::SetPeerBandwidth { .. } => msg_type::SET_PEER_BANDWIDTH,
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
    pub fn from_payload(message_type_id: u8, payload: &[u8]) -> Result<Option<Self>> {
        match message_type_id {
            msg_type::SET_CHUNK_SIZE => {
                let raw = need_u32(payload, "set chunk size payload")?;
                if raw & SET_CHUNK_SIZE_RESERVED_MASK != 0 {
                    return Err(RtmpError::Malformed {
                        what: "set chunk size reserved top bit (must be 0)",
                    });
                }
                let size = raw & SET_CHUNK_SIZE_VALUE_MASK;
                if size == 0 {
                    return Err(RtmpError::Malformed {
                        what: "set chunk size value (must be >= 1)",
                    });
                }
                Ok(Some(ProtocolControl::SetChunkSize(size)))
            }
            msg_type::ABORT => {
                let chunk_stream_id = need_u32(payload, "abort message payload")?;
                Ok(Some(ProtocolControl::Abort { chunk_stream_id }))
            }
            msg_type::ACKNOWLEDGEMENT => {
                let sequence_number = need_u32(payload, "acknowledgement payload")?;
                Ok(Some(ProtocolControl::Acknowledgement(sequence_number)))
            }
            msg_type::WINDOW_ACK_SIZE => {
                let window = need_u32(payload, "window acknowledgement size payload")?;
                Ok(Some(ProtocolControl::WindowAckSize(window)))
            }
            msg_type::SET_PEER_BANDWIDTH => {
                if payload.len() < SET_PEER_BANDWIDTH_LEN {
                    return Err(RtmpError::BufferTooShort {
                        need: SET_PEER_BANDWIDTH_LEN,
                        have: payload.len(),
                        what: "set peer bandwidth payload",
                    });
                }
                let ack_window_size = read_u32_be(&payload[0..U32_LEN]);
                let limit_type = LimitType::from_u8(payload[U32_LEN])?;
                Ok(Some(ProtocolControl::SetPeerBandwidth {
                    ack_window_size,
                    limit_type,
                }))
            }
            _ => Ok(None),
        }
    }

    /// Wrap this protocol control message in a [`Message`] ready to hand to
    /// [`crate::chunk::ChunkWriter`] — chunk stream id
    /// [`CONTROL_CHUNK_STREAM_ID`], message stream id
    /// [`CONTROL_MESSAGE_STREAM_ID`], timestamp `0` (protocol control
    /// messages take effect immediately; timestamps are not meaningful).
    #[must_use]
    pub fn to_message(&self) -> Message {
        Message {
            chunk_stream_id: CONTROL_CHUNK_STREAM_ID,
            timestamp: 0,
            message_type_id: self.message_type_id(),
            message_stream_id: CONTROL_MESSAGE_STREAM_ID,
            payload: self.to_bytes(),
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
            | ProtocolControl::WindowAckSize(_) => U32_LEN,
            ProtocolControl::SetPeerBandwidth { .. } => SET_PEER_BANDWIDTH_LEN,
        }
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let written = self.serialized_len();
        if buf.len() < written {
            return Err(RtmpError::BufferTooShort {
                need: written,
                have: buf.len(),
                what: "protocol control payload output",
            });
        }
        match *self {
            ProtocolControl::SetChunkSize(size) => {
                if size == 0 || size & SET_CHUNK_SIZE_RESERVED_MASK != 0 {
                    return Err(RtmpError::Malformed {
                        what: "set chunk size value (must be 1..=0x7FFF_FFFF)",
                    });
                }
                buf[0..U32_LEN].copy_from_slice(&size.to_be_bytes());
            }
            ProtocolControl::Abort { chunk_stream_id } => {
                buf[0..U32_LEN].copy_from_slice(&chunk_stream_id.to_be_bytes());
            }
            ProtocolControl::Acknowledgement(sequence_number) => {
                buf[0..U32_LEN].copy_from_slice(&sequence_number.to_be_bytes());
            }
            ProtocolControl::WindowAckSize(window) => {
                buf[0..U32_LEN].copy_from_slice(&window.to_be_bytes());
            }
            ProtocolControl::SetPeerBandwidth {
                ack_window_size,
                limit_type,
            } => {
                buf[0..U32_LEN].copy_from_slice(&ack_window_size.to_be_bytes());
                buf[U32_LEN] = limit_type.to_u8();
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
        /// The client's buffer length, in milliseconds.
        buffer_ms: u32,
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

    /// Wrap this user control event in a [`Message`] ready to hand to
    /// [`crate::chunk::ChunkWriter`] — chunk stream id
    /// [`CONTROL_CHUNK_STREAM_ID`], message stream id
    /// [`CONTROL_MESSAGE_STREAM_ID`], timestamp `0` (effective on receipt;
    /// timestamps are not meaningful).
    #[must_use]
    pub fn to_message(&self) -> Message {
        Message {
            chunk_stream_id: CONTROL_CHUNK_STREAM_ID,
            timestamp: 0,
            message_type_id: msg_type::USER_CONTROL,
            message_stream_id: CONTROL_MESSAGE_STREAM_ID,
            payload: self.to_bytes(),
        }
    }
}

broadcast_common::impl_spec_display!(UserControl);

impl<'a> Parse<'a> for UserControl {
    type Error = RtmpError;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < EVENT_TYPE_LEN {
            return Err(RtmpError::BufferTooShort {
                need: EVENT_TYPE_LEN,
                have: bytes.len(),
                what: "user control event type",
            });
        }
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
                if data.len() < 2 * U32_LEN {
                    return Err(RtmpError::BufferTooShort {
                        need: 2 * U32_LEN,
                        have: data.len(),
                        what: "set buffer length event data",
                    });
                }
                Ok(UserControl::SetBufferLength {
                    stream_id: read_u32_be(&data[0..U32_LEN]),
                    buffer_ms: read_u32_be(&data[U32_LEN..2 * U32_LEN]),
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
            _ => Err(RtmpError::Unsupported {
                what: "user control event type (unrecognised)",
            }),
        }
    }
}

impl Serialize for UserControl {
    type Error = RtmpError;

    fn serialized_len(&self) -> usize {
        let data_len = match self {
            UserControl::SetBufferLength { .. } => 2 * U32_LEN,
            _ => U32_LEN,
        };
        EVENT_TYPE_LEN + data_len
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let written = self.serialized_len();
        if buf.len() < written {
            return Err(RtmpError::BufferTooShort {
                need: written,
                have: buf.len(),
                what: "user control event output",
            });
        }
        buf[0..EVENT_TYPE_LEN].copy_from_slice(&self.event_type().to_be_bytes());
        let data = &mut buf[EVENT_TYPE_LEN..written];
        match *self {
            UserControl::StreamBegin(stream_id)
            | UserControl::StreamEof(stream_id)
            | UserControl::StreamDry(stream_id)
            | UserControl::StreamIsRecorded(stream_id)
            | UserControl::PingRequest(stream_id)
            | UserControl::PingResponse(stream_id) => {
                data[0..U32_LEN].copy_from_slice(&stream_id.to_be_bytes());
            }
            UserControl::SetBufferLength {
                stream_id,
                buffer_ms,
            } => {
                data[0..U32_LEN].copy_from_slice(&stream_id.to_be_bytes());
                data[U32_LEN..2 * U32_LEN].copy_from_slice(&buffer_ms.to_be_bytes());
            }
        }
        Ok(written)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(message_type_id: u8, payload: Vec<u8>) -> Message {
        Message {
            chunk_stream_id: CONTROL_CHUNK_STREAM_ID,
            timestamp: 0,
            message_type_id,
            message_stream_id: CONTROL_MESSAGE_STREAM_ID,
            payload,
        }
    }

    // ── LimitType ────────────────────────────────────────────────────────

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
            Err(RtmpError::Malformed { .. })
        ));
    }

    // ── ProtocolControl round-trips ──────────────────────────────────────

    fn protocol_control_round_trip(pc: ProtocolControl) {
        let bytes = pc.to_bytes();
        let parsed = ProtocolControl::from_payload(pc.message_type_id(), &bytes)
            .unwrap()
            .expect("known protocol control type id");
        assert_eq!(parsed, pc);

        // parse -> serialize -> byte-identical
        let msg = message(pc.message_type_id(), bytes.clone());
        let via_message = ProtocolControl::from_message(&msg).unwrap().unwrap();
        assert_eq!(via_message, pc);
        assert_eq!(via_message.to_bytes(), bytes);
    }

    #[test]
    fn set_chunk_size_round_trips() {
        protocol_control_round_trip(ProtocolControl::SetChunkSize(4096));
    }

    #[test]
    fn abort_round_trips() {
        protocol_control_round_trip(ProtocolControl::Abort { chunk_stream_id: 7 });
    }

    #[test]
    fn acknowledgement_round_trips() {
        protocol_control_round_trip(ProtocolControl::Acknowledgement(1_048_576));
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
    fn set_chunk_size_reserved_top_bit_rejected_on_parse() {
        let bytes = 0x8000_1000u32.to_be_bytes().to_vec();
        assert!(matches!(
            ProtocolControl::from_payload(msg_type::SET_CHUNK_SIZE, &bytes),
            Err(RtmpError::Malformed { .. })
        ));
    }

    #[test]
    fn set_chunk_size_zero_rejected() {
        let bytes = 0u32.to_be_bytes().to_vec();
        assert!(matches!(
            ProtocolControl::from_payload(msg_type::SET_CHUNK_SIZE, &bytes),
            Err(RtmpError::Malformed { .. })
        ));
        assert!(matches!(
            ProtocolControl::SetChunkSize(0).serialize_into(&mut [0u8; 4]),
            Err(RtmpError::Malformed { .. })
        ));
    }

    #[test]
    fn set_chunk_size_serialize_layout_matches_spec() {
        // §5.4.1: reserved bit 0, 31-bit chunk size, big-endian.
        let bytes = ProtocolControl::SetChunkSize(1).to_bytes();
        assert_eq!(bytes, vec![0x00, 0x00, 0x00, 0x01]);
    }

    #[test]
    fn set_peer_bandwidth_serialize_layout_matches_spec() {
        let bytes = ProtocolControl::SetPeerBandwidth {
            ack_window_size: 0x0002_5000,
            limit_type: LimitType::Dynamic,
        }
        .to_bytes();
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
        let msg = message(msg_type::AUDIO, vec![0u8; 4]);
        assert!(ProtocolControl::from_message(&msg).unwrap().is_none());
    }

    #[test]
    fn from_message_some_for_control_type_id() {
        let msg = message(
            msg_type::WINDOW_ACK_SIZE,
            1_000_000u32.to_be_bytes().to_vec(),
        );
        assert!(ProtocolControl::from_message(&msg).unwrap().is_some());
    }

    #[test]
    fn to_message_uses_control_csid_and_stream_id() {
        let msg = ProtocolControl::SetChunkSize(4096).to_message();
        assert_eq!(msg.chunk_stream_id, CONTROL_CHUNK_STREAM_ID);
        assert_eq!(msg.message_stream_id, CONTROL_MESSAGE_STREAM_ID);
        assert_eq!(msg.message_type_id, msg_type::SET_CHUNK_SIZE);
    }

    #[test]
    fn protocol_control_display_matches_name() {
        assert_eq!(
            ProtocolControl::Acknowledgement(1).to_string(),
            ProtocolControl::Acknowledgement(1).name()
        );
    }

    // ── UserControl round-trips ──────────────────────────────────────────

    fn user_control_round_trip(uc: UserControl) {
        let bytes = uc.to_bytes();
        let parsed = UserControl::parse(&bytes).unwrap();
        assert_eq!(parsed, uc);
        assert_eq!(parsed.to_bytes(), bytes);
    }

    #[test]
    fn stream_begin_round_trips() {
        user_control_round_trip(UserControl::StreamBegin(1));
    }

    #[test]
    fn stream_begin_serialize_layout_matches_spec() {
        // §6.7: event type 0x0000 + 4-byte stream id, big-endian.
        let bytes = UserControl::StreamBegin(1).to_bytes();
        assert_eq!(bytes, vec![0x00, 0x00, 0x00, 0x00, 0x00, 0x01]);
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
            buffer_ms: 3000,
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
            Err(RtmpError::Unsupported { .. })
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
        let msg = UserControl::StreamBegin(1).to_message();
        assert_eq!(msg.chunk_stream_id, CONTROL_CHUNK_STREAM_ID);
        assert_eq!(msg.message_stream_id, CONTROL_MESSAGE_STREAM_ID);
        assert_eq!(msg.message_type_id, msg_type::USER_CONTROL);
    }
}
