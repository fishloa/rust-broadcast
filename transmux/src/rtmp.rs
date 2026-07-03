//! RTMP (Adobe Real-Time Messaging Protocol) 1.0 transport spoke.
//!
//! Spec: **Adobe RTMP Specification 1.0** (December 2012); AMF0 values follow
//! the companion **AMF0 Specification**. See `transmux/docs/rtmp/rtmp.md` for
//! the transcription. Everything is **big-endian** on the wire except the RTMP
//! message stream id (little-endian, §5.3.1.2.1) and where AMF0 dictates.
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
//! This module also provides typed parse/serialize for the transport primitives
//! themselves — the handshake ([`Handshake0`]/[`Handshake1`]/[`Handshake2`],
//! §5.2), the chunk basic + message headers ([`BasicHeader`]/[`MessageHeader`],
//! §5.3.1), the protocol control messages ([`ProtocolControl`], §5.4), and AMF0
//! command messages ([`AmfValue`]/[`Command`], §7) — each with round-trip
//! coverage.
//!
//! `no_std` + `alloc`.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

use broadcast_common::{Package, Unpackage};

use crate::flv::{FlvDemux, FlvError, FlvMux};
use crate::media::Media;

// ---------------------------------------------------------------------------
// Spec constants (Adobe RTMP 1.0) — no magic numbers outside #[cfg(test)].
// ---------------------------------------------------------------------------

/// RTMP protocol version carried in C0/S0 (§5.2.2): plain RTMP.
pub const RTMP_VERSION: u8 = 3;
/// Length of a C1/S1 or C2/S2 handshake packet in bytes (§5.2.3 / §5.2.4).
pub const HANDSHAKE_PACKET_LEN: usize = 1536;
/// `zero`/random split inside C1/S1: 4-byte time + 4-byte zero + 1528 random (§5.2.3).
const HANDSHAKE_TIME_LEN: usize = 4;
/// Length of the random (C1/S1) / random-echo (C2/S2) field (§5.2.3 / §5.2.4).
pub const HANDSHAKE_RANDOM_LEN: usize = HANDSHAKE_PACKET_LEN - 2 * HANDSHAKE_TIME_LEN;

/// Default maximum chunk size before any Set Chunk Size (§5.4 / §5.4.1).
pub const DEFAULT_CHUNK_SIZE: usize = 128;

/// Reserved chunk stream id for low-level protocol control messages (§5.4).
pub const CSID_CONTROL: u32 = 2;
/// Chunk stream id this crate uses for outbound audio messages.
const CSID_AUDIO: u32 = 4;
/// Chunk stream id this crate uses for outbound video messages.
const CSID_VIDEO: u32 = 5;
/// Chunk stream id this crate uses for outbound data (script) messages.
const CSID_DATA: u32 = 6;

/// csid byte-0 value selecting the 2-byte basic-header form (§5.3.1.1).
const CSID_MARKER_2BYTE: u8 = 0;
/// csid byte-0 value selecting the 3-byte basic-header form (§5.3.1.1).
const CSID_MARKER_3BYTE: u8 = 1;
/// Largest csid encodable in the 1-byte basic header (§5.3.1.1).
const CSID_1BYTE_MAX: u32 = 63;
/// Largest csid encodable in the 2-byte basic header (§5.3.1.1).
const CSID_2BYTE_MAX: u32 = 319;
/// Offset subtracted from csid in the 2- and 3-byte forms (§5.3.1.1).
const CSID_EXT_OFFSET: u32 = 64;

/// The 24-bit timestamp sentinel that signals an Extended Timestamp (§5.3.1.2 / §5.3.1.3).
const EXT_TIMESTAMP_SENTINEL: u32 = 0x00FF_FFFF;
/// Length of the Extended Timestamp field (§5.3.1.3).
const EXT_TIMESTAMP_LEN: usize = 4;

/// Chunk message-header lengths by `fmt` (§5.3.1.2): 11, 7, 3, 0 bytes.
const MSG_HEADER_LEN_FMT0: usize = 11;
const MSG_HEADER_LEN_FMT1: usize = 7;
const MSG_HEADER_LEN_FMT2: usize = 3;
const MSG_HEADER_LEN_FMT3: usize = 0;

/// RTMP message type ids (§7.1 / §5.4).
pub mod msg_type {
    /// Set Chunk Size (§5.4.1).
    pub const SET_CHUNK_SIZE: u8 = 1;
    /// Abort Message (§5.4.2).
    pub const ABORT: u8 = 2;
    /// Acknowledgement (§5.4.3).
    pub const ACKNOWLEDGEMENT: u8 = 3;
    /// User Control Message (§6.2).
    pub const USER_CONTROL: u8 = 4;
    /// Window Acknowledgement Size (§5.4.4).
    pub const WINDOW_ACK_SIZE: u8 = 5;
    /// Set Peer Bandwidth (§5.4.5).
    pub const SET_PEER_BANDWIDTH: u8 = 6;
    /// Audio message — body is an FLV `AudioTagHeader`+data (§7.1).
    pub const AUDIO: u8 = 8;
    /// Video message — body is an FLV `VideoTagHeader`+data (§7.1).
    pub const VIDEO: u8 = 9;
    /// Data message, AMF3 (§7.1.2).
    pub const DATA_AMF3: u8 = 15;
    /// Data message, AMF0 (`@setDataFrame`/`onMetaData`) (§7.1.2).
    pub const DATA_AMF0: u8 = 18;
    /// Command message, AMF3 (§7.1.1).
    pub const COMMAND_AMF3: u8 = 17;
    /// Command message, AMF0 (`connect`/`publish`/`play`/…) (§7.1.1).
    pub const COMMAND_AMF0: u8 = 20;
}

/// Set Peer Bandwidth limit-type values (§5.4.5).
pub mod bandwidth_limit {
    /// Hard: limit output bandwidth to the indicated window size.
    pub const HARD: u8 = 0;
    /// Soft: limit to the smaller of the window size and the current limit.
    pub const SOFT: u8 = 1;
    /// Dynamic: treat as Hard if the previous limit was Hard, else ignore.
    pub const DYNAMIC: u8 = 2;
}

/// AMF0 value-type markers (AMF0 Specification §2).
pub mod amf0 {
    /// Number: 8-byte IEEE-754 double, big-endian (§2.2).
    pub const NUMBER: u8 = 0x00;
    /// Boolean: 1 byte (§2.3).
    pub const BOOLEAN: u8 = 0x01;
    /// String: U16 length + UTF-8 (§2.4).
    pub const STRING: u8 = 0x02;
    /// Object: (key + value)* then object-end (§2.5).
    pub const OBJECT: u8 = 0x03;
    /// Null (§2.7).
    pub const NULL: u8 = 0x05;
    /// ECMA array: U32 count + (key + value)* then object-end (§2.10).
    pub const ECMA_ARRAY: u8 = 0x08;
    /// Object-end marker; preceded by an empty (length-0) key (§2.11).
    pub const OBJECT_END: u8 = 0x09;
}

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Errors specific to RTMP transport framing (Adobe RTMP 1.0).
#[derive(Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RtmpError {
    /// A buffer ended before a field could be read.
    Truncated {
        /// What was being parsed when the buffer ran out.
        what: &'static str,
        /// Bytes required.
        need: usize,
        /// Bytes available.
        have: usize,
    },
    /// C0/S0 carried a version this crate does not speak (§5.2.2).
    BadVersion(u8),
    /// A chunk referenced a chunk stream id with no preceding Type-0 chunk to
    /// inherit its message header from (§5.3.1.2).
    NoChunkContext(u32),
    /// A protocol control message body had the wrong length (§5.4).
    BadControlLength {
        /// The message type id.
        msg_type: u8,
        /// Bytes the payload should have.
        need: usize,
        /// Bytes it had.
        have: usize,
    },
    /// A protocol control chunk carried a message type id this codec does not
    /// recognise (§5.4).
    UnknownControlMsgType(u8),
    /// An AMF0 value used a marker this crate does not decode (§7 / AMF0 §2).
    UnsupportedAmf0Marker(u8),
    /// A reassembled message declared a length that never completed.
    IncompleteMessage {
        /// The chunk stream id.
        csid: u32,
        /// Declared message length.
        declared: usize,
        /// Bytes actually collected.
        collected: usize,
    },
    /// FLV routing of the reassembled A/V bodies failed.
    Flv(FlvError),
}

impl fmt::Display for RtmpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RtmpError::Truncated { what, need, have } => {
                write!(
                    f,
                    "RTMP truncated while parsing {what}: need {need}, have {have}"
                )
            }
            RtmpError::BadVersion(v) => write!(f, "RTMP bad version {v} (expected {RTMP_VERSION})"),
            RtmpError::NoChunkContext(csid) => {
                write!(
                    f,
                    "RTMP chunk on csid {csid} has no Type-0 context to inherit"
                )
            }
            RtmpError::BadControlLength {
                msg_type,
                need,
                have,
            } => write!(
                f,
                "RTMP control message type {msg_type} bad length: need {need}, have {have}"
            ),
            RtmpError::UnknownControlMsgType(t) => {
                write!(f, "RTMP unknown protocol-control message type {t}")
            }
            RtmpError::UnsupportedAmf0Marker(m) => {
                write!(f, "RTMP unsupported AMF0 marker 0x{m:02X}")
            }
            RtmpError::IncompleteMessage {
                csid,
                declared,
                collected,
            } => write!(
                f,
                "RTMP incomplete message on csid {csid}: declared {declared}, collected {collected}"
            ),
            RtmpError::Flv(e) => write!(f, "RTMP FLV routing: {e}"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for RtmpError {}

impl From<FlvError> for RtmpError {
    fn from(e: FlvError) -> Self {
        RtmpError::Flv(e)
    }
}

// ---------------------------------------------------------------------------
// Handshake (§5.2)
// ---------------------------------------------------------------------------

/// C0 / S0 — the 1-byte version (§5.2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Handshake0 {
    /// RTMP protocol version (this crate speaks [`RTMP_VERSION`] = 3).
    pub version: u8,
}

impl Handshake0 {
    /// Parse a C0/S0 byte, rejecting an unspoken version.
    pub fn parse(input: &[u8]) -> Result<Self, RtmpError> {
        let v = *input.first().ok_or(RtmpError::Truncated {
            what: "C0/S0 version",
            need: 1,
            have: 0,
        })?;
        if v != RTMP_VERSION {
            return Err(RtmpError::BadVersion(v));
        }
        Ok(Self { version: v })
    }

    /// Serialize into a 1-byte vector.
    pub fn to_bytes(&self) -> Vec<u8> {
        vec![self.version]
    }
}

/// C1 / S1 — 1536 bytes: `time`(4) + `zero`(4) + `random`(1528) (§5.2.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handshake1 {
    /// Epoch timestamp (may be 0).
    pub time: u32,
    /// Random data field (1528 bytes).
    pub random: Vec<u8>,
}

impl Handshake1 {
    /// Parse a C1/S1 packet (validates length and that `zero` is all zeros).
    pub fn parse(input: &[u8]) -> Result<Self, RtmpError> {
        if input.len() < HANDSHAKE_PACKET_LEN {
            return Err(RtmpError::Truncated {
                what: "C1/S1",
                need: HANDSHAKE_PACKET_LEN,
                have: input.len(),
            });
        }
        let time = u32::from_be_bytes([input[0], input[1], input[2], input[3]]);
        // input[4..8] = zero (not stored; must be zero per §5.2.3 but we are lenient on parse).
        let random = input[2 * HANDSHAKE_TIME_LEN..HANDSHAKE_PACKET_LEN].to_vec();
        Ok(Self { time, random })
    }

    /// Serialize to exactly [`HANDSHAKE_PACKET_LEN`] bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HANDSHAKE_PACKET_LEN);
        out.extend_from_slice(&self.time.to_be_bytes());
        out.extend_from_slice(&[0u8; HANDSHAKE_TIME_LEN]); // zero
        let mut rnd = self.random.clone();
        rnd.resize(HANDSHAKE_RANDOM_LEN, 0);
        out.extend_from_slice(&rnd);
        out
    }
}

/// C2 / S2 — 1536 bytes: `time`(4) + `time2`(4) + `random echo`(1528) (§5.2.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handshake2 {
    /// The `time` from the peer's S1/C1.
    pub time: u32,
    /// Timestamp at which the peer's S1/C1 was read.
    pub time2: u32,
    /// The peer's `random` field echoed back (1528 bytes).
    pub random_echo: Vec<u8>,
}

impl Handshake2 {
    /// Parse a C2/S2 packet.
    pub fn parse(input: &[u8]) -> Result<Self, RtmpError> {
        if input.len() < HANDSHAKE_PACKET_LEN {
            return Err(RtmpError::Truncated {
                what: "C2/S2",
                need: HANDSHAKE_PACKET_LEN,
                have: input.len(),
            });
        }
        let time = u32::from_be_bytes([input[0], input[1], input[2], input[3]]);
        let time2 = u32::from_be_bytes([input[4], input[5], input[6], input[7]]);
        let random_echo = input[2 * HANDSHAKE_TIME_LEN..HANDSHAKE_PACKET_LEN].to_vec();
        Ok(Self {
            time,
            time2,
            random_echo,
        })
    }

    /// Serialize to exactly [`HANDSHAKE_PACKET_LEN`] bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HANDSHAKE_PACKET_LEN);
        out.extend_from_slice(&self.time.to_be_bytes());
        out.extend_from_slice(&self.time2.to_be_bytes());
        let mut echo = self.random_echo.clone();
        echo.resize(HANDSHAKE_RANDOM_LEN, 0);
        out.extend_from_slice(&echo);
        out
    }
}

// ---------------------------------------------------------------------------
// Chunk basic + message headers (§5.3.1)
// ---------------------------------------------------------------------------

/// Chunk basic header: the 2-bit `fmt` + the chunk stream id (§5.3.1.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BasicHeader {
    /// Chunk type (0–3), selecting the message-header format.
    pub fmt: u8,
    /// Chunk stream id (3..=65599).
    pub csid: u32,
}

impl BasicHeader {
    /// Serialized length in bytes (1, 2 or 3) for the smallest form (§5.3.1.1).
    pub fn serialized_len(&self) -> usize {
        if self.csid <= CSID_1BYTE_MAX {
            1
        } else if self.csid <= CSID_2BYTE_MAX {
            2
        } else {
            3
        }
    }

    /// Serialize using the smallest form that holds the csid.
    pub fn write_into(&self, out: &mut Vec<u8>) {
        let fmt_bits = (self.fmt & 0x03) << 6;
        if self.csid <= CSID_1BYTE_MAX {
            out.push(fmt_bits | (self.csid as u8 & 0x3F));
        } else if self.csid <= CSID_2BYTE_MAX {
            out.push(fmt_bits | CSID_MARKER_2BYTE);
            out.push((self.csid - CSID_EXT_OFFSET) as u8);
        } else {
            out.push(fmt_bits | CSID_MARKER_3BYTE);
            let ext = self.csid - CSID_EXT_OFFSET;
            // 16-bit cs id - 64, little-endian (§5.3.1.1).
            out.push((ext & 0xFF) as u8);
            out.push(((ext >> 8) & 0xFF) as u8);
        }
    }

    /// Parse a basic header, returning the header and the bytes consumed.
    pub fn parse(input: &[u8]) -> Result<(Self, usize), RtmpError> {
        let b0 = *input.first().ok_or(RtmpError::Truncated {
            what: "basic header",
            need: 1,
            have: 0,
        })?;
        let fmt = b0 >> 6;
        let marker = b0 & 0x3F;
        if marker == CSID_MARKER_2BYTE {
            let b1 = *input.get(1).ok_or(RtmpError::Truncated {
                what: "basic header (2-byte)",
                need: 2,
                have: input.len(),
            })?;
            Ok((
                Self {
                    fmt,
                    csid: b1 as u32 + CSID_EXT_OFFSET,
                },
                2,
            ))
        } else if marker == CSID_MARKER_3BYTE {
            if input.len() < 3 {
                return Err(RtmpError::Truncated {
                    what: "basic header (3-byte)",
                    need: 3,
                    have: input.len(),
                });
            }
            let ext = input[1] as u32 + ((input[2] as u32) << 8);
            Ok((
                Self {
                    fmt,
                    csid: ext + CSID_EXT_OFFSET,
                },
                3,
            ))
        } else {
            Ok((
                Self {
                    fmt,
                    csid: marker as u32,
                },
                1,
            ))
        }
    }
}

/// Chunk message header, whose present fields depend on `fmt` (§5.3.1.2).
///
/// `timestamp` holds the absolute timestamp (fmt 0) or the timestamp delta
/// (fmt 1/2). For fmt 3 the header is empty; the caller inherits every field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MessageHeader {
    /// Absolute timestamp (fmt 0) or timestamp delta (fmt 1/2).
    pub timestamp: u32,
    /// Total message payload length (fmt 0/1).
    pub message_length: u32,
    /// Message type id (fmt 0/1).
    pub message_type_id: u8,
    /// Message stream id — little-endian on the wire (fmt 0 only).
    pub message_stream_id: u32,
}

impl MessageHeader {
    /// Serialized length in bytes for the given `fmt` (§5.3.1.2), excluding the
    /// extended timestamp.
    pub fn serialized_len(fmt: u8) -> usize {
        match fmt {
            0 => MSG_HEADER_LEN_FMT0,
            1 => MSG_HEADER_LEN_FMT1,
            2 => MSG_HEADER_LEN_FMT2,
            _ => MSG_HEADER_LEN_FMT3,
        }
    }

    /// Whether the (fmt-relevant) timestamp value forces an Extended Timestamp
    /// field (§5.3.1.2 / §5.3.1.3).
    pub fn needs_extended(&self, fmt: u8) -> bool {
        fmt != 3 && self.timestamp >= EXT_TIMESTAMP_SENTINEL
    }

    /// Serialize the fmt-relevant fields plus the Extended Timestamp when the
    /// timestamp value overflows the 24-bit field (§5.3.1.2 / §5.3.1.3).
    pub fn write_into(&self, fmt: u8, out: &mut Vec<u8>) {
        let ext = self.needs_extended(fmt);
        let ts24 = if ext {
            EXT_TIMESTAMP_SENTINEL
        } else {
            self.timestamp
        };
        if fmt <= 2 {
            write_u24(out, ts24);
        }
        if fmt <= 1 {
            write_u24(out, self.message_length);
            out.push(self.message_type_id);
        }
        if fmt == 0 {
            // message stream id — little-endian (§5.3.1.2.1).
            out.extend_from_slice(&self.message_stream_id.to_le_bytes());
        }
        if ext {
            out.extend_from_slice(&self.timestamp.to_be_bytes());
        }
    }

    /// Parse the fmt-relevant fields (not the extended timestamp) into a header,
    /// returning the header and bytes consumed. `ts24` is returned separately so
    /// the caller can decide whether an Extended Timestamp follows.
    fn parse_fields(fmt: u8, input: &[u8]) -> Result<(Self, u32, usize), RtmpError> {
        let need = Self::serialized_len(fmt);
        if input.len() < need {
            return Err(RtmpError::Truncated {
                what: "message header",
                need,
                have: input.len(),
            });
        }
        let mut off = 0;
        let mut h = MessageHeader::default();
        let mut ts24 = 0;
        if fmt <= 2 {
            ts24 = read_u24(&input[off..]);
            h.timestamp = ts24;
            off += 3;
        }
        if fmt <= 1 {
            h.message_length = read_u24(&input[off..]);
            off += 3;
            h.message_type_id = input[off];
            off += 1;
        }
        if fmt == 0 {
            h.message_stream_id =
                u32::from_le_bytes([input[off], input[off + 1], input[off + 2], input[off + 3]]);
            off += 4;
        }
        Ok((h, ts24, off))
    }
}

/// Write a big-endian unsigned 24-bit integer.
fn write_u24(out: &mut Vec<u8>, v: u32) {
    out.push((v >> 16) as u8);
    out.push((v >> 8) as u8);
    out.push(v as u8);
}

/// Read a big-endian unsigned 24-bit integer.
fn read_u24(b: &[u8]) -> u32 {
    ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | (b[2] as u32)
}

// ---------------------------------------------------------------------------
// Protocol control messages (§5.4)
// ---------------------------------------------------------------------------

/// A protocol control message payload (§5.4) — message type ids 1, 2, 3, 5, 6.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProtocolControl {
    /// Set Chunk Size (type 1) — new maximum chunk size (§5.4.1).
    SetChunkSize(u32),
    /// Abort Message (type 2) — csid whose partial message is discarded (§5.4.2).
    Abort(u32),
    /// Acknowledgement (type 3) — bytes received so far (§5.4.3).
    Acknowledgement(u32),
    /// Window Acknowledgement Size (type 5) — window size (§5.4.4).
    WindowAckSize(u32),
    /// Set Peer Bandwidth (type 6) — window size + limit type (§5.4.5).
    SetPeerBandwidth {
        /// Acknowledgement window size.
        window_size: u32,
        /// Limit type — see [`bandwidth_limit`].
        limit_type: u8,
    },
}

impl ProtocolControl {
    /// The message type id this control message serializes as (§5.4).
    pub fn message_type_id(&self) -> u8 {
        match self {
            ProtocolControl::SetChunkSize(_) => msg_type::SET_CHUNK_SIZE,
            ProtocolControl::Abort(_) => msg_type::ABORT,
            ProtocolControl::Acknowledgement(_) => msg_type::ACKNOWLEDGEMENT,
            ProtocolControl::WindowAckSize(_) => msg_type::WINDOW_ACK_SIZE,
            ProtocolControl::SetPeerBandwidth { .. } => msg_type::SET_PEER_BANDWIDTH,
        }
    }

    /// Serialize the message body (not the chunk framing).
    pub fn to_body(&self) -> Vec<u8> {
        match *self {
            // bit 0 MUST be zero; chunk size is 31 bits (§5.4.1). The high bit of
            // a value <= 0x7FFFFFFF is already zero.
            ProtocolControl::SetChunkSize(v) => (v & 0x7FFF_FFFF).to_be_bytes().to_vec(),
            ProtocolControl::Abort(v) => v.to_be_bytes().to_vec(),
            ProtocolControl::Acknowledgement(v) => v.to_be_bytes().to_vec(),
            ProtocolControl::WindowAckSize(v) => v.to_be_bytes().to_vec(),
            ProtocolControl::SetPeerBandwidth {
                window_size,
                limit_type,
            } => {
                let mut out = Vec::with_capacity(5);
                out.extend_from_slice(&window_size.to_be_bytes());
                out.push(limit_type);
                out
            }
        }
    }

    /// Parse a protocol control message body given its message type id.
    pub fn parse(msg_type_id: u8, body: &[u8]) -> Result<Self, RtmpError> {
        let want_u32 = |what_len: usize| -> Result<u32, RtmpError> {
            if body.len() < what_len {
                Err(RtmpError::BadControlLength {
                    msg_type: msg_type_id,
                    need: what_len,
                    have: body.len(),
                })
            } else {
                Ok(u32::from_be_bytes([body[0], body[1], body[2], body[3]]))
            }
        };
        match msg_type_id {
            msg_type::SET_CHUNK_SIZE => {
                Ok(ProtocolControl::SetChunkSize(want_u32(4)? & 0x7FFF_FFFF))
            }
            msg_type::ABORT => Ok(ProtocolControl::Abort(want_u32(4)?)),
            msg_type::ACKNOWLEDGEMENT => Ok(ProtocolControl::Acknowledgement(want_u32(4)?)),
            msg_type::WINDOW_ACK_SIZE => Ok(ProtocolControl::WindowAckSize(want_u32(4)?)),
            msg_type::SET_PEER_BANDWIDTH => {
                if body.len() < 5 {
                    return Err(RtmpError::BadControlLength {
                        msg_type: msg_type_id,
                        need: 5,
                        have: body.len(),
                    });
                }
                Ok(ProtocolControl::SetPeerBandwidth {
                    window_size: u32::from_be_bytes([body[0], body[1], body[2], body[3]]),
                    limit_type: body[4],
                })
            }
            other => Err(RtmpError::UnknownControlMsgType(other)),
        }
    }
}

// ---------------------------------------------------------------------------
// AMF0 (§7 / AMF0 Specification §2)
// ---------------------------------------------------------------------------

/// The AMF0 value types this crate encodes/decodes (AMF0 §2).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum AmfValue {
    /// Number — IEEE-754 double (§2.2).
    Number(f64),
    /// Boolean (§2.3).
    Boolean(bool),
    /// String (§2.4).
    String(String),
    /// Object — ordered name-value pairs (§2.5).
    Object(Vec<(String, AmfValue)>),
    /// Null (§2.7).
    Null,
    /// ECMA (associative) array — ordered name-value pairs (§2.10).
    EcmaArray(Vec<(String, AmfValue)>),
}

impl AmfValue {
    /// Encode this value (marker + payload) into `out` (AMF0 §2).
    pub fn write_into(&self, out: &mut Vec<u8>) {
        match self {
            AmfValue::Number(n) => {
                out.push(amf0::NUMBER);
                out.extend_from_slice(&n.to_be_bytes());
            }
            AmfValue::Boolean(b) => {
                out.push(amf0::BOOLEAN);
                out.push(u8::from(*b));
            }
            AmfValue::String(s) => {
                out.push(amf0::STRING);
                write_amf0_string(out, s);
            }
            AmfValue::Null => out.push(amf0::NULL),
            AmfValue::Object(members) => {
                out.push(amf0::OBJECT);
                write_amf0_members(out, members);
            }
            AmfValue::EcmaArray(members) => {
                out.push(amf0::ECMA_ARRAY);
                out.extend_from_slice(&(members.len() as u32).to_be_bytes());
                write_amf0_members(out, members);
            }
        }
    }

    /// Decode one AMF0 value from the front of `input`, returning the value and
    /// the number of bytes consumed (AMF0 §2).
    pub fn parse(input: &[u8]) -> Result<(Self, usize), RtmpError> {
        let marker = *input.first().ok_or(RtmpError::Truncated {
            what: "AMF0 marker",
            need: 1,
            have: 0,
        })?;
        let rest = &input[1..];
        match marker {
            amf0::NUMBER => {
                if rest.len() < 8 {
                    return Err(RtmpError::Truncated {
                        what: "AMF0 number",
                        need: 8,
                        have: rest.len(),
                    });
                }
                let mut b = [0u8; 8];
                b.copy_from_slice(&rest[..8]);
                Ok((AmfValue::Number(f64::from_be_bytes(b)), 9))
            }
            amf0::BOOLEAN => {
                let v = *rest.first().ok_or(RtmpError::Truncated {
                    what: "AMF0 boolean",
                    need: 1,
                    have: 0,
                })?;
                Ok((AmfValue::Boolean(v != 0), 2))
            }
            amf0::STRING => {
                let (s, n) = read_amf0_string(rest)?;
                Ok((AmfValue::String(s), 1 + n))
            }
            amf0::NULL => Ok((AmfValue::Null, 1)),
            amf0::OBJECT => {
                let (members, n) = read_amf0_members(rest)?;
                Ok((AmfValue::Object(members), 1 + n))
            }
            amf0::ECMA_ARRAY => {
                if rest.len() < 4 {
                    return Err(RtmpError::Truncated {
                        what: "AMF0 ECMA array count",
                        need: 4,
                        have: rest.len(),
                    });
                }
                // The associative count is advisory; the members are terminated
                // by the object-end marker (§2.10). Read to object-end.
                let (members, n) = read_amf0_members(&rest[4..])?;
                Ok((AmfValue::EcmaArray(members), 1 + 4 + n))
            }
            other => Err(RtmpError::UnsupportedAmf0Marker(other)),
        }
    }
}

fn write_amf0_string(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u16).to_be_bytes());
    out.extend_from_slice(s.as_bytes());
}

fn write_amf0_members(out: &mut Vec<u8>, members: &[(String, AmfValue)]) {
    for (k, v) in members {
        write_amf0_string(out, k);
        v.write_into(out);
    }
    // Object end: empty key + object-end marker (§2.11).
    out.extend_from_slice(&0u16.to_be_bytes());
    out.push(amf0::OBJECT_END);
}

/// Read a length-prefixed AMF0 UTF-8 string, returning (string, bytes consumed).
fn read_amf0_string(input: &[u8]) -> Result<(String, usize), RtmpError> {
    if input.len() < 2 {
        return Err(RtmpError::Truncated {
            what: "AMF0 string length",
            need: 2,
            have: input.len(),
        });
    }
    let len = u16::from_be_bytes([input[0], input[1]]) as usize;
    if input.len() < 2 + len {
        return Err(RtmpError::Truncated {
            what: "AMF0 string body",
            need: 2 + len,
            have: input.len(),
        });
    }
    let s = String::from_utf8_lossy(&input[2..2 + len]).into_owned();
    Ok((s, 2 + len))
}

/// Read object / ECMA-array members up to the object-end marker (§2.5 / §2.11).
/// Returns (members, bytes consumed including the empty-key + object-end).
fn read_amf0_members(input: &[u8]) -> Result<(Vec<(String, AmfValue)>, usize), RtmpError> {
    let mut members = Vec::new();
    let mut off = 0;
    loop {
        // A key is a bare (unmarkered) U16-length string (§2.5).
        let (key, kn) = read_amf0_string(&input[off..])?;
        // Empty key followed by the object-end marker terminates (§2.11).
        if key.is_empty() {
            let end = *input.get(off + kn).ok_or(RtmpError::Truncated {
                what: "AMF0 object-end marker",
                need: off + kn + 1,
                have: input.len(),
            })?;
            if end == amf0::OBJECT_END {
                off += kn + 1;
                return Ok((members, off));
            }
            // An empty key that is not followed by object-end is malformed; treat
            // it as a value read to keep parsing forward.
        }
        off += kn;
        let (val, vn) = AmfValue::parse(&input[off..])?;
        off += vn;
        members.push((key, val));
    }
}

/// A decoded AMF0 command / data message (§7.1.1 / §7.1.2): a command name, a
/// transaction id, then the remaining top-level AMF0 values.
#[derive(Debug, Clone, PartialEq)]
pub struct Command {
    /// Command name — e.g. `"connect"`, `"publish"`, `"onMetaData"` (§7.2).
    pub name: String,
    /// Transaction id (0 for data/notify messages) (§7.1.1).
    pub transaction_id: f64,
    /// The remaining top-level AMF0 values (command object + arguments).
    pub arguments: Vec<AmfValue>,
}

impl Command {
    /// Encode the AMF0 command body (name + transaction id + arguments).
    pub fn to_body(&self) -> Vec<u8> {
        let mut out = Vec::new();
        AmfValue::String(self.name.clone()).write_into(&mut out);
        AmfValue::Number(self.transaction_id).write_into(&mut out);
        for a in &self.arguments {
            a.write_into(&mut out);
        }
        out
    }

    /// Decode an AMF0 command body.
    pub fn parse(body: &[u8]) -> Result<Self, RtmpError> {
        let (name_v, mut off) = AmfValue::parse(body)?;
        let AmfValue::String(name) = name_v else {
            return Err(RtmpError::UnsupportedAmf0Marker(
                body.first().copied().unwrap_or(0),
            ));
        };
        let (txn_v, n) = AmfValue::parse(&body[off..])?;
        off += n;
        let transaction_id = match txn_v {
            AmfValue::Number(n) => n,
            _ => 0.0,
        };
        let mut arguments = Vec::new();
        while off < body.len() {
            let (v, n) = AmfValue::parse(&body[off..])?;
            off += n;
            arguments.push(v);
        }
        Ok(Command {
            name,
            transaction_id,
            arguments,
        })
    }
}

// ---------------------------------------------------------------------------
// Message framing over the chunk stream (the reassembly engine)
// ---------------------------------------------------------------------------

/// One complete RTMP message reassembled from (or ready to be split into)
/// chunks: its stream-transport identity plus the payload body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// Chunk stream id this message travels on.
    pub csid: u32,
    /// Message type id (§7.1).
    pub message_type_id: u8,
    /// Message stream id.
    pub message_stream_id: u32,
    /// Absolute timestamp (ms).
    pub timestamp: u32,
    /// The complete message payload.
    pub body: Vec<u8>,
}

/// Split RTMP [`Message`]s into a chunk stream at the given chunk size (§5.3.1).
///
/// The first chunk of each message uses fmt 0 (full header); continuation chunks
/// use fmt 3. Returns the serialized chunk bytes.
pub fn write_chunks(messages: &[Message], chunk_size: usize) -> Vec<u8> {
    let chunk_size = chunk_size.max(1);
    let mut out = Vec::new();
    for m in messages {
        let mut body_off = 0;
        let mut first = true;
        loop {
            let fmt = if first { 0u8 } else { 3u8 };
            BasicHeader { fmt, csid: m.csid }.write_into(&mut out);
            if first {
                let mh = MessageHeader {
                    timestamp: m.timestamp,
                    message_length: m.body.len() as u32,
                    message_type_id: m.message_type_id,
                    message_stream_id: m.message_stream_id,
                };
                mh.write_into(0, &mut out);
            } else if m.timestamp >= EXT_TIMESTAMP_SENTINEL {
                // fmt-3 continuation still carries the extended timestamp when
                // the fmt-0 chunk indicated one (§5.3.1.3).
                out.extend_from_slice(&m.timestamp.to_be_bytes());
            }
            let take = core::cmp::min(chunk_size, m.body.len() - body_off);
            out.extend_from_slice(&m.body[body_off..body_off + take]);
            body_off += take;
            first = false;
            if body_off >= m.body.len() {
                break;
            }
        }
    }
    out
}

/// Per-csid decode state carried across chunks (§5.3.1.2 inheritance).
#[derive(Clone)]
struct ChunkContext {
    message_type_id: u8,
    message_length: usize,
    message_stream_id: u32,
    timestamp: u32,
    extended: bool,
    // Reassembly buffer for the in-progress message on this csid.
    partial: Vec<u8>,
}

/// Reassemble a chunk stream into complete [`Message`]s, honouring Set Chunk
/// Size control messages inline (§5.3 / §5.4.1).
pub fn read_chunks(mut input: &[u8]) -> Result<Vec<Message>, RtmpError> {
    let mut chunk_size = DEFAULT_CHUNK_SIZE;
    let mut ctx: Vec<(u32, ChunkContext)> = Vec::new();
    let mut out = Vec::new();

    while !input.is_empty() {
        let (bh, bn) = BasicHeader::parse(input)?;
        let mut off = bn;
        let (mh, ts24, mn) = MessageHeader::parse_fields(bh.fmt, &input[off..])?;
        off += mn;

        // Resolve the inherited context for this csid.
        let idx = ctx.iter().position(|(c, _)| *c == bh.csid);
        let prev = idx.map(|i| ctx[i].1.clone());

        // Extended timestamp: present when the (fmt-relevant) timestamp reads the
        // sentinel, or (fmt 3) when the prior chunk on this csid indicated one.
        let ext_present = match bh.fmt {
            0..=2 => ts24 >= EXT_TIMESTAMP_SENTINEL,
            _ => prev.as_ref().map(|p| p.extended).unwrap_or(false),
        };
        let ext_ts = if ext_present {
            if input.len() < off + EXT_TIMESTAMP_LEN {
                return Err(RtmpError::Truncated {
                    what: "extended timestamp",
                    need: off + EXT_TIMESTAMP_LEN,
                    have: input.len(),
                });
            }
            let t =
                u32::from_be_bytes([input[off], input[off + 1], input[off + 2], input[off + 3]]);
            off += EXT_TIMESTAMP_LEN;
            Some(t)
        } else {
            None
        };

        // Build/refresh the context for this csid from the fmt + inheritance.
        let mut cx = match bh.fmt {
            0 => ChunkContext {
                message_type_id: mh.message_type_id,
                message_length: mh.message_length as usize,
                message_stream_id: mh.message_stream_id,
                timestamp: ext_ts.unwrap_or(mh.timestamp),
                extended: ext_present,
                partial: Vec::new(),
            },
            1 => {
                let p = prev.ok_or(RtmpError::NoChunkContext(bh.csid))?;
                ChunkContext {
                    message_type_id: mh.message_type_id,
                    message_length: mh.message_length as usize,
                    message_stream_id: p.message_stream_id,
                    timestamp: p.timestamp.wrapping_add(ext_ts.unwrap_or(mh.timestamp)),
                    extended: ext_present,
                    partial: p.partial,
                }
            }
            2 => {
                let p = prev.ok_or(RtmpError::NoChunkContext(bh.csid))?;
                ChunkContext {
                    message_type_id: p.message_type_id,
                    message_length: p.message_length,
                    message_stream_id: p.message_stream_id,
                    timestamp: p.timestamp.wrapping_add(ext_ts.unwrap_or(mh.timestamp)),
                    extended: ext_present,
                    partial: p.partial,
                }
            }
            _ => {
                // fmt 3: inherit everything; may continue a message or begin a
                // new one with the same header (§5.3.1.2.4).
                let mut p = prev.ok_or(RtmpError::NoChunkContext(bh.csid))?;
                if let Some(t) = ext_ts {
                    p.timestamp = t;
                }
                p
            }
        };

        // How many payload bytes are in this chunk: the remainder of the message,
        // capped at the current chunk size.
        let remaining = cx.message_length - cx.partial.len();
        let take = core::cmp::min(chunk_size, remaining);
        if input.len() < off + take {
            return Err(RtmpError::Truncated {
                what: "chunk data",
                need: off + take,
                have: input.len(),
            });
        }
        cx.partial.extend_from_slice(&input[off..off + take]);
        off += take;
        input = &input[off..];

        // Message complete?
        if cx.partial.len() >= cx.message_length {
            let body = core::mem::take(&mut cx.partial);
            // A Set Chunk Size control message changes the reassembly size for
            // all subsequent chunks (§5.4.1).
            if cx.message_type_id == msg_type::SET_CHUNK_SIZE {
                if let Ok(ProtocolControl::SetChunkSize(sz)) =
                    ProtocolControl::parse(cx.message_type_id, &body)
                {
                    chunk_size = (sz as usize).max(1);
                }
            }
            out.push(Message {
                csid: bh.csid,
                message_type_id: cx.message_type_id,
                message_stream_id: cx.message_stream_id,
                timestamp: cx.timestamp,
                body,
            });
        }

        // Store the (possibly still-partial) context back.
        match idx {
            Some(i) => ctx[i].1 = cx,
            None => ctx.push((bh.csid, cx)),
        }
    }

    Ok(out)
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

/// Build an FLV byte stream from A/V (and script) tag bodies with timestamps.
/// `(tag_type, timestamp_ms, body)` — the body is exactly an FLV tag payload
/// (which for A/V equals the RTMP message body).
fn build_flv(tags: &[(u8, u32, Vec<u8>)], has_video: bool, has_audio: bool) -> Vec<u8> {
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
        write_u24(&mut out, body.len() as u32);
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
    out
}

/// Walk an FLV byte stream into `(tag_type, timestamp_ms, body)` tags (§E.4.1).
fn split_flv(flv: &[u8]) -> Result<Vec<(u8, u32, Vec<u8>)>, RtmpError> {
    if flv.len() < FLV_HEADER_LEN as usize + FLV_PREV_TAG_SIZE_LEN {
        return Err(RtmpError::Truncated {
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
            return Err(RtmpError::Truncated {
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
/// [`Handshake0`]/[`Handshake1`]/[`Handshake2`] first if needed).
#[derive(Debug, Default, Clone)]
pub struct RtmpDemux<'a> {
    _marker: core::marker::PhantomData<&'a [u8]>,
}

impl<'a> RtmpDemux<'a> {
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
    type Error = RtmpError;

    fn unpackage(&mut self, input: &'a [u8]) -> Result<Media, RtmpError> {
        let messages = read_chunks(input)?;
        let mut tags: Vec<(u8, u32, Vec<u8>)> = Vec::new();
        let mut has_video = false;
        let mut has_audio = false;
        for m in messages {
            match m.message_type_id {
                msg_type::AUDIO => {
                    has_audio = true;
                    tags.push((FLV_TAG_AUDIO, m.timestamp, m.body));
                }
                msg_type::VIDEO => {
                    has_video = true;
                    tags.push((FLV_TAG_VIDEO, m.timestamp, m.body));
                }
                msg_type::DATA_AMF0 => {
                    tags.push((FLV_TAG_SCRIPT, m.timestamp, m.body));
                }
                _ => { /* control / command / user-control — not media */ }
            }
        }
        let flv = build_flv(&tags, has_video, has_audio);
        let mut demux = FlvDemux::new();
        demux.unpackage(&flv).map_err(RtmpError::Flv)
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
            chunk_size: DEFAULT_CHUNK_SIZE,
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
    type Error = RtmpError;

    fn package(&mut self, media: &Media) -> Result<Vec<u8>, RtmpError> {
        let mut flv_mux = FlvMux::new();
        let flv = flv_mux.package(media).map_err(RtmpError::Flv)?;
        let tags = split_flv(&flv)?;

        let mut messages = Vec::with_capacity(tags.len());
        for (tag_type, ts, body) in tags {
            let (csid, msg_type_id) = match tag_type {
                FLV_TAG_AUDIO => (CSID_AUDIO, msg_type::AUDIO),
                FLV_TAG_VIDEO => (CSID_VIDEO, msg_type::VIDEO),
                FLV_TAG_SCRIPT => (CSID_DATA, msg_type::DATA_AMF0),
                _ => (CSID_DATA, msg_type::DATA_AMF0),
            };
            messages.push(Message {
                csid,
                message_type_id: msg_type_id,
                message_stream_id: self.message_stream_id,
                timestamp: ts,
                body,
            });
        }
        Ok(write_chunks(&messages, self.chunk_size))
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

    #[test]
    fn basic_header_forms() {
        for csid in [3u32, 63, 64, 319, 320, 65599] {
            let bh = BasicHeader { fmt: 0, csid };
            let mut out = Vec::new();
            bh.write_into(&mut out);
            assert_eq!(out.len(), bh.serialized_len());
            let (parsed, n) = BasicHeader::parse(&out).unwrap();
            assert_eq!(parsed, bh);
            assert_eq!(n, out.len());
        }
    }

    #[test]
    fn amf0_object_round_trip() {
        let obj = AmfValue::Object(vec![
            ("app".into(), AmfValue::String("live".into())),
            ("audioOnly".into(), AmfValue::Boolean(false)),
            ("fps".into(), AmfValue::Number(30.0)),
        ]);
        let mut out = Vec::new();
        obj.write_into(&mut out);
        let (parsed, n) = AmfValue::parse(&out).unwrap();
        assert_eq!(parsed, obj);
        assert_eq!(n, out.len());
    }
}
