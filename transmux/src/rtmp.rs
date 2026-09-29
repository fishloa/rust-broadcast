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
// No longer `Eq` (only `PartialEq`): `RtmpError::Flv` wraps `FlvError`,
// which lost `Eq` for the same reason (issue #1140 T12 cascade).
#[derive(Debug, PartialEq)]
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
    /// An AMF0 String's byte length exceeds the 16-bit length prefix (§2.4).
    /// This crate does not implement the AMF0 Long String type (§2.13) — the
    /// caller must shorten the value.
    AmfStringTooLong {
        /// The string's byte length.
        len: usize,
    },
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
    /// An AMF0 object/ECMA-array nesting exceeded [`MAX_AMF0_DEPTH`].
    ///
    /// The AMF0 wire format has no depth limit, so `AmfValue::parse` (and
    /// therefore `Command::parse`) is fed arbitrary peer-supplied bytes. Each
    /// nesting level costs about four bytes (`03 00 01 6B`), so without a cap a
    /// 1 MiB `connect` command recurses ~250 000 frames deep and overflows the
    /// stack — an abort no caller can catch.
    Amf0TooDeep {
        /// The depth that was reached.
        depth: usize,
    },
    /// A length or count did not fit the wire field it is written to (#1129):
    /// a chunk `message_length` or FLV `DataSize` (both UI24) that would have
    /// silently wrapped past 16 MiB.
    FieldOverflow(broadcast_common::len::FieldOverflow),
}

/// Maximum AMF0 object/ECMA-array nesting [`AmfValue::parse`] accepts.
///
/// No conformant RTMP command comes close: the deepest real message this crate
/// sees is `connect`'s command object with a nested `objectEncoding`, two
/// levels down. 32 leaves generous headroom for a vendor's custom metadata
/// while keeping the parser's stack use bounded whatever a peer sends.
pub const MAX_AMF0_DEPTH: usize = 32;

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
            RtmpError::Amf0TooDeep { depth } => write!(
                f,
                "RTMP AMF0 nesting depth {depth} exceeds the {MAX_AMF0_DEPTH}-level cap"
            ),
            RtmpError::AmfStringTooLong { len } => {
                write!(
                    f,
                    "RTMP AMF0 string of {len} bytes exceeds the u16 length prefix (long string type not implemented)"
                )
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
            RtmpError::FieldOverflow(e) => write!(f, "RTMP {e}"),
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

impl From<broadcast_common::len::FieldOverflow> for RtmpError {
    fn from(e: broadcast_common::len::FieldOverflow) -> Self {
        RtmpError::FieldOverflow(e)
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

    /// Whether `msg_type_id` is one of the protocol control message types this
    /// codec handles (§5.4: 1, 2, 3, 5, 6 — note 4 is a User Control message,
    /// which is an *application* message, not protocol control).
    ///
    /// A cheap guard for callers that must ask "is this a control message?"
    /// about every completed message: [`parse`](Self::parse) already rejects an
    /// unknown type id, but a caller in a hot loop should not build its closure
    /// and body-slice checks only to be told the message is ordinary media.
    pub fn is_control_type(msg_type_id: u8) -> bool {
        matches!(
            msg_type_id,
            msg_type::SET_CHUNK_SIZE
                | msg_type::ABORT
                | msg_type::ACKNOWLEDGEMENT
                | msg_type::WINDOW_ACK_SIZE
                | msg_type::SET_PEER_BANDWIDTH
        )
    }

    /// Parse a protocol control message body given its message type id.
    ///
    /// Returns [`RtmpError::UnknownControlMsgType`] for a type id that is not
    /// protocol control at all — check [`is_control_type`](Self::is_control_type)
    /// first when that is an expected case rather than an error.
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
    ///
    /// Errors with [`RtmpError::AmfStringTooLong`] if a `String` (or an
    /// `Object`/`EcmaArray` member key) exceeds the 16-bit AMF0 String length
    /// prefix — this crate does not implement the AMF0 Long String type.
    pub fn write_into(&self, out: &mut Vec<u8>) -> Result<(), RtmpError> {
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
                write_amf0_string(out, s)?;
            }
            AmfValue::Null => out.push(amf0::NULL),
            AmfValue::Object(members) => {
                out.push(amf0::OBJECT);
                write_amf0_members(out, members)?;
            }
            AmfValue::EcmaArray(members) => {
                out.push(amf0::ECMA_ARRAY);
                out.extend_from_slice(&(members.len() as u32).to_be_bytes());
                write_amf0_members(out, members)?;
            }
        }
        Ok(())
    }

    /// Decode one AMF0 value from the front of `input`, returning the value and
    /// the number of bytes consumed (AMF0 §2).
    ///
    /// Object/ECMA-array nesting is capped at [`MAX_AMF0_DEPTH`]: the wire
    /// format has no depth limit and this is the entry point for peer-supplied
    /// command bytes, so an uncapped recursive descent would overflow the stack
    /// (an abort, not a catchable error) on a hostile or corrupt message.
    pub fn parse(input: &[u8]) -> Result<(Self, usize), RtmpError> {
        Self::parse_at_depth(input, 0)
    }

    /// [`parse`](Self::parse) with the current nesting depth threaded through.
    fn parse_at_depth(input: &[u8], depth: usize) -> Result<(Self, usize), RtmpError> {
        if depth > MAX_AMF0_DEPTH {
            return Err(RtmpError::Amf0TooDeep { depth });
        }
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
                let (members, n) = read_amf0_members(rest, depth + 1)?;
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
                let (members, n) = read_amf0_members(&rest[4..], depth + 1)?;
                Ok((AmfValue::EcmaArray(members), 1 + 4 + n))
            }
            other => Err(RtmpError::UnsupportedAmf0Marker(other)),
        }
    }
}

fn write_amf0_string(out: &mut Vec<u8>, s: &str) -> Result<(), RtmpError> {
    if s.len() > usize::from(u16::MAX) {
        return Err(RtmpError::AmfStringTooLong { len: s.len() });
    }
    out.extend_from_slice(&(s.len() as u16).to_be_bytes());
    out.extend_from_slice(s.as_bytes());
    Ok(())
}

fn write_amf0_members(out: &mut Vec<u8>, members: &[(String, AmfValue)]) -> Result<(), RtmpError> {
    for (k, v) in members {
        write_amf0_string(out, k)?;
        v.write_into(out)?;
    }
    // Object end: empty key + object-end marker (§2.11).
    out.extend_from_slice(&0u16.to_be_bytes());
    out.push(amf0::OBJECT_END);
    Ok(())
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
fn read_amf0_members(
    input: &[u8],
    depth: usize,
) -> Result<(Vec<(String, AmfValue)>, usize), RtmpError> {
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
        let (val, vn) = AmfValue::parse_at_depth(&input[off..], depth)?;
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
    ///
    /// Errors with [`RtmpError::AmfStringTooLong`] if the command name or any
    /// argument String (or object/array member key) exceeds the AMF0 String
    /// length prefix.
    pub fn to_body(&self) -> Result<Vec<u8>, RtmpError> {
        let mut out = Vec::new();
        AmfValue::String(self.name.clone()).write_into(&mut out)?;
        AmfValue::Number(self.transaction_id).write_into(&mut out)?;
        for a in &self.arguments {
            a.write_into(&mut out)?;
        }
        Ok(out)
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
///
/// # Errors
///
/// Returns [`RtmpError::FieldOverflow`] if a message body is 16 MiB (2^24) or
/// larger — `message_length` is a UI24 field (§5.3.1.2) and cannot represent it.
pub fn write_chunks(messages: &[Message], chunk_size: usize) -> Result<Vec<u8>, RtmpError> {
    let chunk_size = chunk_size.max(1);
    let mut out = Vec::new();
    for m in messages {
        let message_length = broadcast_common::len::fit_u24(m.body.len(), "message_length")?;
        let mut body_off = 0;
        let mut first = true;
        loop {
            let fmt = if first { 0u8 } else { 3u8 };
            BasicHeader { fmt, csid: m.csid }.write_into(&mut out);
            if first {
                let mh = MessageHeader {
                    timestamp: m.timestamp,
                    message_length,
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
    Ok(out)
}

/// Per-csid decode state carried across chunks (§5.3.1.2 inheritance).
#[derive(Clone)]
struct ChunkContext {
    message_type_id: u8,
    message_length: usize,
    message_stream_id: u32,
    timestamp: u32,
    /// The **delta** the most recent fmt-1/fmt-2 chunk declared
    /// (§5.3.1.2.2/.3). A fmt-3 chunk that begins a new message inherits this
    /// delta — or, when the chunk carries its own extended timestamp, that
    /// value, which stands in for the delta the 24-bit field could not hold —
    /// and adds it to the running timestamp rather than reusing the timestamp
    /// itself (§5.3.1.2.4) — see `read_chunks`.
    timestamp_delta: u32,
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
        //
        // The fmt-1/fmt-2 timestamp field is a *delta* from the previous
        // message on this csid (§5.3.1.2.2/.3), and the extended timestamp
        // overriding it when the sentinel was read is a delta as well.
        let delta = ext_ts.unwrap_or(mh.timestamp);
        let mut cx = match bh.fmt {
            0 => ChunkContext {
                message_type_id: mh.message_type_id,
                message_length: mh.message_length as usize,
                message_stream_id: mh.message_stream_id,
                timestamp: ext_ts.unwrap_or(mh.timestamp),
                // fmt 0 carries an absolute timestamp, so there is no delta to
                // inherit (§5.3.1.2.1).
                timestamp_delta: 0,
                extended: ext_present,
                partial: Vec::new(),
            },
            1 => {
                let p = prev.ok_or(RtmpError::NoChunkContext(bh.csid))?;
                ChunkContext {
                    message_type_id: mh.message_type_id,
                    message_length: mh.message_length as usize,
                    message_stream_id: p.message_stream_id,
                    timestamp: p.timestamp.wrapping_add(delta),
                    timestamp_delta: delta,
                    extended: ext_present,
                    // fmt 1 starts a NEW message (§5.3.1.2.2): any bytes still
                    // buffered from an incomplete earlier message on this csid
                    // belong to that aborted message, never to this one.
                    partial: Vec::new(),
                }
            }
            2 => {
                let p = prev.ok_or(RtmpError::NoChunkContext(bh.csid))?;
                ChunkContext {
                    message_type_id: p.message_type_id,
                    message_length: p.message_length,
                    message_stream_id: p.message_stream_id,
                    timestamp: p.timestamp.wrapping_add(delta),
                    timestamp_delta: delta,
                    extended: ext_present,
                    // fmt 2 starts a NEW message too (§5.3.1.2.3).
                    partial: Vec::new(),
                }
            }
            _ => {
                // fmt 3: inherit the previous chunk's header outright, and
                // either continue its message or (when nothing is buffered)
                // begin a new one (§5.3.1.2.4).
                let mut p = prev.ok_or(RtmpError::NoChunkContext(bh.csid))?;
                let starting_new_message = p.partial.is_empty();
                if starting_new_message {
                    // §5.3.1.2.4: this is the framing used for "a stream of
                    // messages of exactly the same size, stream ID and spacing
                    // in time" — the message length, type and stream ID are
                    // identical, and the **timestamp advances by the delta the
                    // preceding fmt-1/fmt-2 chunk declared**. Reusing the
                    // timestamp made every fmt-3-started message carry the
                    // identical DTS, collapsing constant-rate audio (librtmp
                    // and ffmpeg send fmt 2 then fmt 3) into duplicate
                    // timestamps with zero durations downstream.
                    //
                    // When this chunk carries its own extended timestamp, that
                    // value *is* this message's delta — it stands in for the
                    // 24-bit delta field the fmt-1/fmt-2 predecessor could not
                    // hold (§5.3.1.3: the extended field carries the same
                    // quantity the truncated one would have). It is **not**
                    // absolute: assigning it as one would jump the clock to
                    // whatever the delta happened to be.
                    let advance = ext_ts.unwrap_or(p.timestamp_delta);
                    p.timestamp = p.timestamp.wrapping_add(advance);
                }
                // A fmt-3 **continuation** chunk changes nothing about the
                // timestamp: it is mid-message, and the timestamp belongs to
                // the message. In particular its extended field (present
                // because the message's own timestamp is >= the sentinel,
                // §5.3.1.3) repeats that timestamp and must not be re-applied
                // as a delta.
                p
            }
        };

        // How many payload bytes are in this chunk: the remainder of the message,
        // capped at the current chunk size. `partial` can only be non-empty here
        // for a fmt-3 continuation (fmt 0/1/2 reset it above), where each chunk
        // takes at most `remaining`, so `len <= message_length` holds — but the
        // length is wire-declared, so don't let a hostile stream underflow it.
        let remaining = cx.message_length.checked_sub(cx.partial.len()).ok_or(
            RtmpError::IncompleteMessage {
                csid: bh.csid,
                declared: cx.message_length,
                collected: cx.partial.len(),
            },
        )?;
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
            // Almost every message is media, not protocol control: check the
            // type id before paying for a body parse (and before treating a
            // media body as a malformed control message).
            match ProtocolControl::is_control_type(cx.message_type_id) {
                false => {}
                true => match ProtocolControl::parse(cx.message_type_id, &body) {
                    // A Set Chunk Size control message changes the reassembly size
                    // for all subsequent chunks (§5.4.1).
                    Ok(ProtocolControl::SetChunkSize(sz)) => {
                        chunk_size = (sz as usize).max(1);
                    }
                    // An Abort Message (§5.4.2) tells the receiver to discard the
                    // in-progress message on the csid its body names, so the
                    // sender can re-send the rest of it. Ignoring this left the
                    // aborted bytes buffered, and the next chunk for that csid was
                    // appended to a message the peer had already abandoned, so the
                    // two were emitted as one misframed message.
                    Ok(ProtocolControl::Abort(target)) => {
                        if let Some(i) = ctx.iter().position(|(c, _)| *c == target) {
                            ctx[i].1.partial.clear();
                            // The delta belonged to the message being
                            // abandoned: a fmt-3 re-send after the abort must
                            // start from the aborted message's timestamp, not
                            // advance by a delta that described a message
                            // nobody ever received.
                            ctx[i].1.timestamp_delta = 0;
                        }
                    }
                    _ => {}
                },
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
///
/// # Errors
///
/// Returns [`RtmpError::FieldOverflow`] if a tag body is 16 MiB (2^24) or
/// larger — `DataSize` is a UI24 field (§E.4.1) and cannot represent it.
fn build_flv(
    tags: &[(u8, u32, Vec<u8>)],
    has_video: bool,
    has_audio: bool,
) -> Result<Vec<u8>, RtmpError> {
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
        let flv = build_flv(&tags, has_video, has_audio)?;
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
        write_chunks(&messages, self.chunk_size)
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
        obj.write_into(&mut out).unwrap();
        let (parsed, n) = AmfValue::parse(&out).unwrap();
        assert_eq!(parsed, obj);
        assert_eq!(n, out.len());
    }

    /// §5.3.1.2: fmt 0/1/2 chunk headers each START a new message; only fmt 3
    /// continues one. A stale `partial` from an incomplete earlier message on
    /// the same csid must never be merged into (or underflow) the new message.
    #[test]
    fn fmt1_header_starts_a_fresh_message_on_the_same_csid() {
        // fmt-0 chunk: csid 3, declared length 200, but only its first
        // 128-byte chunk is fed — the message stays incomplete on this csid.
        let mut input = Vec::new();
        BasicHeader { fmt: 0, csid: 3 }.write_into(&mut input);
        MessageHeader {
            timestamp: 0,
            message_length: 200,
            message_type_id: msg_type::VIDEO,
            message_stream_id: 1,
        }
        .write_into(0, &mut input);
        let first_chunk: Vec<u8> = (0..DEFAULT_CHUNK_SIZE).map(|i| i as u8).collect();
        input.extend_from_slice(&first_chunk);

        // fmt-1 chunk on the same csid: a NEW message of 10 bytes.
        let second_body: Vec<u8> = (0xF0u8..0xF0 + 10).collect();
        BasicHeader { fmt: 1, csid: 3 }.write_into(&mut input);
        MessageHeader {
            timestamp: 7,
            message_length: second_body.len() as u32,
            message_type_id: msg_type::VIDEO,
            message_stream_id: 0,
        }
        .write_into(1, &mut input);
        input.extend_from_slice(&second_body);

        let msgs = read_chunks(&input).expect("fmt-1 must start a new message");
        assert_eq!(
            msgs.len(),
            1,
            "only the complete 10-byte message may be emitted"
        );
        assert_eq!(msgs[0].body, second_body);
        assert!(
            msgs.iter().all(|m| m.body.len() != 256),
            "the stale fmt-0 partial must not be merged into the new message"
        );
    }

    /// Regression guard for the reset above: a normal message split across a
    /// fmt-0 chunk + a fmt-3 continuation (the only form that continues a
    /// message, §5.3.1.2.4) must still reassemble byte-exactly.
    #[test]
    fn fmt3_continuation_still_reassembles_a_split_message() {
        let body: Vec<u8> = (0..200u32).map(|i| (i % 251) as u8).collect();
        let mut input = Vec::new();
        BasicHeader { fmt: 0, csid: 3 }.write_into(&mut input);
        MessageHeader {
            timestamp: 0,
            message_length: body.len() as u32,
            message_type_id: msg_type::VIDEO,
            message_stream_id: 1,
        }
        .write_into(0, &mut input);
        input.extend_from_slice(&body[..DEFAULT_CHUNK_SIZE]);
        // fmt-3 carries no header fields at all (§5.3.1.2.4).
        BasicHeader { fmt: 3, csid: 3 }.write_into(&mut input);
        input.extend_from_slice(&body[DEFAULT_CHUNK_SIZE..]);

        let msgs = read_chunks(&input).expect("fmt-3 must continue the message");
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].body, body);
    }

    /// The control-type guard `read_chunks` uses so ordinary media never pays
    /// for (or is misread as) a protocol control message body.
    #[test]
    fn control_type_guard_matches_the_control_message_set() {
        for t in [
            msg_type::SET_CHUNK_SIZE,
            msg_type::ABORT,
            msg_type::ACKNOWLEDGEMENT,
            msg_type::WINDOW_ACK_SIZE,
            msg_type::SET_PEER_BANDWIDTH,
        ] {
            assert!(
                ProtocolControl::is_control_type(t),
                "{t} is protocol control"
            );
        }
        for t in [
            msg_type::AUDIO,
            msg_type::VIDEO,
            msg_type::DATA_AMF0,
            msg_type::DATA_AMF3,
            0,
            0xFF,
        ] {
            assert!(
                !ProtocolControl::is_control_type(t),
                "{t} is not protocol control"
            );
            // ...and `parse` agrees, so the guard is not a second source of truth.
            assert!(ProtocolControl::parse(t, &[]).is_err());
        }
    }

    /// A 16 MiB (2^24) message body cannot fit the 24-bit `message_length`
    /// field (#1129): unfixed, `m.body.len() as u32` then `write_u24` kept
    /// only the low 24 bits, silently misframing the chunk header.
    #[test]
    fn write_chunks_oversized_body_errors() {
        let messages = alloc::vec![Message {
            csid: 4,
            message_type_id: msg_type::VIDEO,
            message_stream_id: 1,
            timestamp: 0,
            body: alloc::vec![0u8; 1 << 24],
        }];
        let err = write_chunks(&messages, DEFAULT_CHUNK_SIZE).unwrap_err();
        assert!(
            matches!(
                err,
                RtmpError::FieldOverflow(broadcast_common::len::FieldOverflow {
                    field: "message_length",
                    ..
                })
            ),
            "expected FieldOverflow for message_length, got {err:?}"
        );
    }

    /// The boundary: exactly (2^24 - 1) bytes still writes and reassembles.
    /// A `Set Chunk Size` control message ahead of it raises the reader's
    /// reassembly size to match (§5.4.1), so this is a single chunk on the
    /// wire (the field-width boundary under test, not the unrelated
    /// chunk-splitting loop).
    #[test]
    fn write_chunks_max_body_round_trips() {
        let body = alloc::vec![0xAAu8; (1 << 24) - 1];
        let big_chunk_size = body.len();
        let messages = alloc::vec![
            Message {
                csid: 2,
                message_type_id: msg_type::SET_CHUNK_SIZE,
                message_stream_id: 0,
                timestamp: 0,
                body: ProtocolControl::SetChunkSize(big_chunk_size as u32).to_body(),
            },
            Message {
                csid: 4,
                message_type_id: msg_type::VIDEO,
                message_stream_id: 1,
                timestamp: 0,
                body: body.clone(),
            },
        ];
        let wire = write_chunks(&messages, big_chunk_size).unwrap();
        let msgs = read_chunks(&wire).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[1].body, body);
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
                RtmpError::FieldOverflow(broadcast_common::len::FieldOverflow {
                    field: "DataSize",
                    ..
                })
            ),
            "expected FieldOverflow for DataSize, got {err:?}"
        );
    }

    /// §5.3.1.2.4: a fmt-3 chunk that *begins* a new message carries no header
    /// of its own — it inherits the previous header plus the **timestamp
    /// delta** the preceding fmt-2 chunk declared, which advances the running
    /// timestamp. `librtmp` and `ffmpeg` send constant-rate audio exactly this
    /// way (one fmt-2, then fmt-3 forever). Reusing the previous timestamp
    /// instead gave every one of those messages an identical DTS, so
    /// `FlvDemux` saw zero-duration samples and downstream playback collapsed.
    #[test]
    fn fmt3_started_message_advances_by_the_inherited_delta() {
        const DELTA: u32 = 23; // ms, one 1024-sample AAC frame at 44.1 kHz
        const FRAME: usize = 8; // tiny payload, so each fits in one chunk

        let mut input = Vec::new();
        // fmt 0: the first message, absolute timestamp 0.
        BasicHeader { fmt: 0, csid: 6 }.write_into(&mut input);
        MessageHeader {
            timestamp: 0,
            message_length: FRAME as u32,
            message_type_id: msg_type::AUDIO,
            message_stream_id: 1,
        }
        .write_into(0, &mut input);
        input.extend_from_slice(&[0xAF, 0x01, 0, 0x11, 0x22, 0x33, 0x44, 0x55]);
        // fmt 2: second message, timestamp as a DELTA of 23.
        BasicHeader { fmt: 2, csid: 6 }.write_into(&mut input);
        MessageHeader {
            timestamp: DELTA,
            message_length: FRAME as u32,
            message_type_id: 0,
            message_stream_id: 0,
        }
        .write_into(2, &mut input);
        input.extend_from_slice(&[0xAF, 0x01, 0, 0x11, 0x22, 0x33, 0x44, 0x55]);
        // fmt 3: third and fourth messages — no header bytes at all, each
        // advancing by the same inherited delta.
        for _ in 0..2 {
            BasicHeader { fmt: 3, csid: 6 }.write_into(&mut input);
            input.extend_from_slice(&[0xAF, 0x01, 0, 0x11, 0x22, 0x33, 0x44, 0x55]);
        }

        let msgs = read_chunks(&input).expect("chunk stream");
        let stamps: Vec<u32> = msgs.iter().map(|m| m.timestamp).collect();
        // Bites: without the delta the last two were [46, 46] instead.
        assert_eq!(stamps, vec![0, DELTA, 2 * DELTA, 3 * DELTA]);
    }

    /// A fmt-3 chunk that *continues* a split message must NOT advance the
    /// timestamp — only one that starts a new message does.
    #[test]
    fn fmt3_continuation_does_not_advance_the_timestamp() {
        let body: Vec<u8> = (0..200u32).map(|i| (i % 251) as u8).collect();
        let mut input = Vec::new();
        BasicHeader { fmt: 0, csid: 3 }.write_into(&mut input);
        MessageHeader {
            timestamp: 100,
            message_length: body.len() as u32,
            message_type_id: msg_type::VIDEO,
            message_stream_id: 1,
        }
        .write_into(0, &mut input);
        input.extend_from_slice(&body[..DEFAULT_CHUNK_SIZE]);
        BasicHeader { fmt: 3, csid: 3 }.write_into(&mut input);
        input.extend_from_slice(&body[DEFAULT_CHUNK_SIZE..]);

        let msgs = read_chunks(&input).expect("chunk stream");
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].timestamp, 100, "continuation keeps the timestamp");
        assert_eq!(msgs[0].body, body);
    }

    /// §5.4.2: an Abort Message discards the in-progress message on the csid
    /// its body names. Ignoring it left the abandoned bytes buffered, so the
    /// sender's re-send was appended to them and emitted as one misframed
    /// message.
    #[test]
    fn abort_message_discards_the_partial_on_the_target_csid() {
        // csid 3: an incomplete 200-byte message (only its first chunk).
        let mut input = Vec::new();
        BasicHeader { fmt: 0, csid: 3 }.write_into(&mut input);
        MessageHeader {
            timestamp: 0,
            message_length: 200,
            message_type_id: msg_type::VIDEO,
            message_stream_id: 1,
        }
        .write_into(0, &mut input);
        input.extend_from_slice(&[0xAA; DEFAULT_CHUNK_SIZE]);

        // "Abort csid 3" (§5.4.2), on its own csid 2 with the minimal body.
        let abort_body = ProtocolControl::Abort(3).to_body();
        BasicHeader { fmt: 0, csid: 2 }.write_into(&mut input);
        MessageHeader {
            timestamp: 0,
            message_length: abort_body.len() as u32,
            message_type_id: msg_type::ABORT,
            message_stream_id: 0,
        }
        .write_into(0, &mut input);
        input.extend_from_slice(&abort_body);

        // The sender then re-sends the message using **only fmt-3 chunks** —
        // §5.3.1.2.4's "messages of exactly the same size, stream ID and
        // spacing in time", inheriting the first chunk's header. Each fmt-3
        // chunk carries one chunk_size of payload.
        let resent: Vec<u8> = (0..200u32).map(|i| (i % 251) as u8).collect();
        for piece in resent.chunks(DEFAULT_CHUNK_SIZE) {
            BasicHeader { fmt: 3, csid: 3 }.write_into(&mut input);
            input.extend_from_slice(piece);
        }

        let msgs = read_chunks(&input).expect("chunk stream");
        let video: Vec<&Message> = msgs
            .iter()
            .filter(|m| m.message_type_id == msg_type::VIDEO)
            .collect();
        assert_eq!(video.len(), 1, "the aborted message is never emitted");
        // Bites: without the discard the first fmt-3 chunk completed the
        // *abandoned* 200-byte message (128 stale bytes + 72 of the re-send),
        // and the stream was misframed from there on (`NoChunkContext`).
        assert_eq!(
            video[0].body, resent,
            "only the re-sent message may be emitted on csid 3"
        );
        assert!(
            msgs.iter().all(|m| !m.body.starts_with(&[0xAA, 0xAA])),
            "no emitted message may contain the abandoned bytes"
        );
    }

    /// r04-W27: AMF0 object nesting is capped, so a hostile `connect` command
    /// cannot recurse the parser into a stack overflow (which aborts the
    /// process and cannot be caught).
    #[test]
    fn deeply_nested_amf0_is_rejected_not_a_stack_overflow() {
        // `depth` nested objects, each 4 bytes (`03 00 01 'k'`), then a number.
        fn nested(depth: usize) -> Vec<u8> {
            let mut out = Vec::new();
            for _ in 0..depth {
                out.push(amf0::OBJECT);
                out.extend_from_slice(&1u16.to_be_bytes());
                out.push(b'k');
            }
            out.push(amf0::NUMBER);
            out.extend_from_slice(&0f64.to_be_bytes());
            for _ in 0..depth {
                out.extend_from_slice(&0u16.to_be_bytes());
                out.push(amf0::OBJECT_END);
            }
            out
        }
        // Exactly the cap parses; one past it is an error, not a crash.
        assert_eq!(MAX_AMF0_DEPTH, 32, "the documented cap");
        assert!(
            AmfValue::parse(&nested(32)).is_ok(),
            "exactly 32 levels must parse"
        );
        let err33 = AmfValue::parse(&nested(33)).expect_err("33 levels must be rejected");
        assert!(
            matches!(err33, RtmpError::Amf0TooDeep { depth: 33 }),
            "expected Amf0TooDeep {{ depth: 33 }}, got {err33:?}"
        );
        let err = AmfValue::parse(&nested(MAX_AMF0_DEPTH + 1))
            .expect_err("nesting past the cap must be rejected");
        assert!(
            matches!(err, RtmpError::Amf0TooDeep { .. }),
            "expected Amf0TooDeep, got {err:?}"
        );
        // And a message built to exhaust the stack (~250k levels from ~1 MiB)
        // is rejected immediately rather than aborting.
        assert!(AmfValue::parse(&nested(250_000)).is_err());
    }

    /// §5.3.1.3: when a chunk's 24-bit timestamp field reads the sentinel, the
    /// real value follows as a 4-byte Extended Timestamp. For fmt 1/2 that
    /// field is a **delta** (it stands in for the truncated one), and a fmt-3
    /// chunk that begins a new message advances by *that* delta — it is not an
    /// absolute time. A fmt-3 **continuation** chunk carries the message's own
    /// extended timestamp, which must not move the clock at all.
    #[test]
    fn fmt3_uses_the_extended_timestamp_as_a_delta() {
        // >= the 24-bit sentinel, so every timestamp field here is extended.
        const BIG_DELTA: u32 = 0x0100_0000;
        // Two chunks per message, so each has a fmt-3 continuation.
        const FRAME: usize = 2 * DEFAULT_CHUNK_SIZE;
        const PAYLOAD: [u8; DEFAULT_CHUNK_SIZE] = [0xAF; DEFAULT_CHUNK_SIZE];

        // A message of FRAME bytes whose head chunk is `head` and whose tail is
        // a fmt-3 continuation. `ts` is the message's resolved timestamp; a
        // fmt-3 chunk repeats the 4-byte extended timestamp only when that
        // timestamp is past the sentinel (§5.3.1.3).
        fn push_split_message(out: &mut Vec<u8>, ts: u32, head: impl FnOnce(&mut Vec<u8>)) {
            head(out);
            out.extend_from_slice(&PAYLOAD);
            BasicHeader { fmt: 3, csid: 4 }.write_into(out);
            if ts >= EXT_TIMESTAMP_SENTINEL {
                out.extend_from_slice(&ts.to_be_bytes());
            }
            out.extend_from_slice(&PAYLOAD);
        }

        let mut input = Vec::new();
        // fmt 0: absolute timestamp 0.
        push_split_message(&mut input, 0, |out| {
            BasicHeader { fmt: 0, csid: 4 }.write_into(out);
            MessageHeader {
                timestamp: 0,
                message_length: FRAME as u32,
                message_type_id: msg_type::AUDIO,
                message_stream_id: 1,
            }
            .write_into(0, out);
        });
        // fmt 2: delta BIG_DELTA, carried in the Extended Timestamp because it
        // does not fit the 24-bit field (§5.3.1.3). `write_into` emits both the
        // sentinel and the extended value.
        push_split_message(&mut input, BIG_DELTA, |out| {
            BasicHeader { fmt: 2, csid: 4 }.write_into(out);
            MessageHeader {
                timestamp: BIG_DELTA,
                message_length: FRAME as u32, // not written by fmt 2; inherited
                message_type_id: 0,
                message_stream_id: 0,
            }
            .write_into(2, out);
        });

        let msgs = read_chunks(&input).expect("chunk stream");
        let stamps: Vec<u32> = msgs.iter().map(|m| m.timestamp).collect();
        assert_eq!(
            stamps,
            vec![0, BIG_DELTA],
            "the fmt-2 message takes the extended value as a delta from 0, and              its fmt-3 continuation leaves the timestamp alone"
        );

        // Now a fmt-3 chunk that *begins* a new message, carrying its own
        // extended timestamp. That value is the new message's delta: the clock
        // must advance by it, not jump to it. Bites: the old code assigned it
        // absolutely, so the stamp stayed at BIG_DELTA instead of 2*BIG_DELTA.
        let mut whole = input.clone();
        BasicHeader { fmt: 3, csid: 4 }.write_into(&mut whole);
        whole.extend_from_slice(&BIG_DELTA.to_be_bytes());
        whole.extend_from_slice(&PAYLOAD);
        BasicHeader { fmt: 3, csid: 4 }.write_into(&mut whole);
        whole.extend_from_slice(&BIG_DELTA.to_be_bytes());
        whole.extend_from_slice(&PAYLOAD);

        let msgs = read_chunks(&whole).expect("chunk stream");
        let stamps: Vec<u32> = msgs.iter().map(|m| m.timestamp).collect();
        assert_eq!(
            stamps,
            vec![0, BIG_DELTA, 2 * BIG_DELTA],
            "a fmt-3-started message advances by the extended delta"
        );
    }

    /// §5.3.1.2.4 + §5.3.1.3 reached through the *same* code path with a small
    /// delta: a fmt-1 chunk followed by a fmt-3 new message advances by the
    /// stored delta, and a fmt-3 continuation in between does not.
    #[test]
    fn fmt3_delta_is_applied_once_per_new_message_only() {
        const DELTA: u32 = 5;
        const FRAME: usize = 64; // one chunk each

        let mut input = Vec::new();
        for (fmt, ts, mtype) in [(0u8, 0u32, msg_type::AUDIO), (1, DELTA, 0), (3, 0, 0)] {
            BasicHeader { fmt, csid: 5 }.write_into(&mut input);
            if fmt <= 1 {
                MessageHeader {
                    timestamp: ts,
                    message_length: FRAME as u32,
                    message_type_id: mtype,
                    message_stream_id: 1,
                }
                .write_into(fmt, &mut input);
            }
            input.extend_from_slice(&[0x11u8; FRAME]);
        }

        {
            let mut k = 0usize;
            while k + 1 < input.len() {
                let b = input[k];
                if matches!(b, 0x04 | 0x84 | 0xC4) {
                    std::eprintln!(
                        "bh @{k} = {b:02X} next {:02X?}",
                        &input[k + 1..(k + 6).min(input.len())]
                    );
                }
                k += 1;
            }
            std::eprintln!("total {}", input.len());
        }
        let msgs = read_chunks(&input).expect("chunk stream");
        let stamps: Vec<u32> = msgs.iter().map(|m| m.timestamp).collect();
        assert_eq!(stamps, vec![0, DELTA, 2 * DELTA]);
    }

    /// After an Abort, the abandoned message's delta must be forgotten: a fmt-3
    /// re-send starts from the aborted message's timestamp, not one further
    /// delta on.
    #[test]
    fn abort_resets_the_delta_for_a_fmt3_resend() {
        const DELTA: u32 = 40;
        const FRAME: usize = 64;

        let mut input = Vec::new();
        // fmt 0 (absolute 1000), then a fmt-1 declaring a 40 ms delta.
        BasicHeader { fmt: 0, csid: 3 }.write_into(&mut input);
        MessageHeader {
            timestamp: 1000,
            message_length: FRAME as u32,
            message_type_id: msg_type::AUDIO,
            message_stream_id: 1,
        }
        .write_into(0, &mut input);
        input.extend_from_slice(&[0x22u8; FRAME]);
        BasicHeader { fmt: 1, csid: 3 }.write_into(&mut input);
        MessageHeader {
            timestamp: DELTA,
            message_length: FRAME as u32,
            message_type_id: msg_type::AUDIO, // fmt 1 still carries the type
            message_stream_id: 0,             // not written for fmt 1; inherited
        }
        .write_into(1, &mut input);
        input.extend_from_slice(&[0x22u8; FRAME]);
        // Abort csid 3 (§5.4.2).
        let abort_body = ProtocolControl::Abort(3).to_body();
        BasicHeader { fmt: 0, csid: 2 }.write_into(&mut input);
        MessageHeader {
            timestamp: 0,
            message_length: abort_body.len() as u32,
            message_type_id: msg_type::ABORT,
            message_stream_id: 0,
        }
        .write_into(0, &mut input);
        input.extend_from_slice(&abort_body);
        // fmt 3: a new message, re-sent after the abort.
        BasicHeader { fmt: 3, csid: 3 }.write_into(&mut input);
        input.extend_from_slice(&[0x22u8; FRAME]);

        let msgs = read_chunks(&input).expect("chunk stream");
        let audio: Vec<u32> = msgs
            .iter()
            .filter(|m| m.message_type_id == msg_type::AUDIO)
            .map(|m| m.timestamp)
            .collect();
        // Bites: without the delta reset the re-send is stamped 1040+40=1080.
        assert_eq!(audio, vec![1000, 1040, 1040]);
    }
}
