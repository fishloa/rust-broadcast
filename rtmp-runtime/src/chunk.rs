//! RTMP chunk stream — basic header, message header, extended timestamp
//! (Adobe RTMP 1.0 §5.3).
//!
//! See [`docs/rtmp.md`](../docs/rtmp.md) §3 (RTMP Chunk Stream) for the wire
//! layout: chunk format (§5.3.1), basic header (§5.3.1.1), the four message
//! header `fmt` variants (§5.3.1.2), and extended timestamp (§5.3.1.3).
//!
//! This module implements the chunk **header** wire types (`BasicHeader`,
//! `MessageHeader`) and the stateful reassembly engine built on top of them:
//! [`ChunkAssembler`] (inbound, tracks prior-chunk state per csid so `fmt`
//! 1/2/3 headers can inherit the fields they omit, and reassembles chunked
//! payload back into whole [`Message`]s) and [`ChunkWriter`] (outbound,
//! splits a [`Message`] into `fmt` 0 + 3 chunks at the configured chunk
//! size).

use std::collections::HashMap;

use broadcast_common::{Parse, Serialize};

use crate::RtmpError;

type Result<T> = core::result::Result<T, RtmpError>;

/// Default maximum chunk size (§5.3, §5.4.1): 128 bytes, in effect until a
/// Set Chunk Size protocol control message changes it.
pub const DEFAULT_CHUNK_SIZE: u32 = 128;

/// Largest chunk size [`ChunkAssembler::set_chunk_size`]/[`ChunkWriter::set_chunk_size`]
/// will adopt from a Set Chunk Size protocol control message (§5.4.1): 16
/// MiB. The wire field is a 31-bit value (up to ~2 GiB), but no real
/// publisher/player needs a chunk size anywhere near that — a single chunk
/// this large would already hold many seconds of encoded audio/video — so
/// this is a defensive ceiling, not a spec limit: values above it are
/// clamped down rather than rejected, matching the existing floor-of-1
/// behaviour for values below it.
pub const MAX_CHUNK_SIZE: u32 = 16 * 1024 * 1024;

/// Largest total `message_length` (§5.3.1.2) [`ChunkAssembler`] will begin
/// buffering for a single reassembled message: 8 MiB. `message_length` is a
/// fully attacker-controlled 24-bit wire field (max ~16 MiB); real RTMP
/// audio/video/command messages are always far smaller than this (a single
/// compressed video frame, even a keyframe, is normally well under 1 MiB),
/// so this is a generous ceiling that still bounds worst-case allocation
/// per in-progress message. A Type 0/1 header declaring a larger
/// `message_length` is rejected by [`ChunkAssembler`] before any buffer for
/// it is allocated.
pub const MAX_MESSAGE_LEN: u32 = 8 * 1024 * 1024;

/// Largest number of distinct chunk stream ids [`ChunkAssembler`] will track
/// reassembly state for concurrently. A well-behaved publisher uses only a
/// handful of chunk streams (2/3 for control/command traffic, plus a few
/// more for audio/video) — this bound is generous headroom above that, not
/// a spec limit — so a flood of chunks opening many distinct (and mostly
/// bogus) csids is rejected rather than growing the per-csid state map
/// without bound.
pub const MAX_CSIDS: usize = 64;

/// The 24-bit sentinel value that, in a Type 0/1/2 message header's
/// `timestamp`/`timestamp delta` field, signals that the field does not carry
/// the real value: the full 32-bit value instead follows in a 4-byte
/// Extended Timestamp (§5.3.1.3). Per §5.3.1.2.1, any real timestamp/delta
/// `>= EXTENDED_TIMESTAMP_MARKER` is encoded this way.
pub const EXTENDED_TIMESTAMP_MARKER: u32 = 0x00FF_FFFF;

/// Byte width of a 24-bit (`u24`) wire field (`timestamp`, `timestamp delta`,
/// `message length`).
const U24_LEN: usize = 3;
/// Byte width of the Extended Timestamp field (§5.3.1.3).
const EXTENDED_TIMESTAMP_LEN: usize = 4;

/// Byte width of a Type 0 message header (§5.3.1.2.1), excluding any
/// Extended Timestamp.
const TYPE0_LEN: usize = 11;
/// Byte width of a Type 1 message header (§5.3.1.2.2), excluding any
/// Extended Timestamp.
const TYPE1_LEN: usize = 7;
/// Byte width of a Type 2 message header (§5.3.1.2.3), excluding any
/// Extended Timestamp.
const TYPE2_LEN: usize = 3;
/// Byte width of a Type 3 message header (§5.3.1.2.4): always empty.
const TYPE3_LEN: usize = 0;

/// Chunk stream id (csid) offset added back on the 2-/3-byte basic header
/// forms (§5.3.1.1): both forms carry `csid - 64`.
const BASIC_HEADER_CSID_OFFSET: u32 = 64;
/// The 2-byte basic header form's marker value in byte 0's low 6 bits.
const BASIC_HEADER_2BYTE_MARKER: u8 = 0;
/// The 3-byte basic header form's marker value in byte 0's low 6 bits.
const BASIC_HEADER_3BYTE_MARKER: u8 = 1;
/// Bit shift of the 2-bit `fmt` field within basic header byte 0.
const BASIC_HEADER_FMT_SHIFT: u8 = 6;
/// Mask for the low 6 bits of basic header byte 0 (the 1-byte csid, or the
/// 2-/3-byte form marker).
const BASIC_HEADER_MARKER_MASK: u8 = 0x3F;

/// Smallest csid encodable in the basic header's 1-byte form (§5.3.1.1).
/// Csid values 0 and 1 are reserved as the 2-/3-byte form markers and so can
/// never appear as a literal 1-byte-form csid; csid 2 is additionally
/// reserved by the spec for low-level protocol control messages/commands but
/// remains structurally encodable.
const BASIC_HEADER_1BYTE_MIN_CSID: u32 = 2;
/// Largest csid encodable in the basic header's 1-byte form.
const BASIC_HEADER_1BYTE_MAX_CSID: u32 = 63;
/// Smallest csid encodable in the basic header's 2-byte form.
const BASIC_HEADER_2BYTE_MIN_CSID: u32 = 64;
/// Largest csid encodable in the basic header's 2-byte form (csids 64-319
/// are also representable in the 3-byte form; 2-byte is the minimal one).
const BASIC_HEADER_2BYTE_MAX_CSID: u32 = 319;
/// Smallest csid that requires the basic header's 3-byte form.
const BASIC_HEADER_3BYTE_MIN_CSID: u32 = 320;
/// Largest csid the protocol supports at all (§5.3.1.1: "up to 65597 chunk
/// streams, IDs 3-65599" — the 3-byte form's 16-bit `csid - 64` field tops
/// out here).
const BASIC_HEADER_3BYTE_MAX_CSID: u32 = 65599;

// ── u24 helpers ─────────────────────────────────────────────────────────

/// Read a 3-byte big-endian unsigned integer. `b` must have at least
/// [`U24_LEN`] bytes (caller-checked).
fn read_u24_be(b: &[u8]) -> u32 {
    (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2])
}

/// Write `v`'s low 24 bits as big-endian into `buf`. `buf` must have at
/// least [`U24_LEN`] bytes (caller-checked). Bits above the low 24 are
/// silently dropped — every wire user of this helper (`message length`, and
/// `timestamp`/`timestamp delta` after the extended-timestamp check) is
/// only ever asked to write a value already known to fit.
fn write_u24_be(v: u32, buf: &mut [u8]) {
    buf[0] = (v >> 16) as u8;
    buf[1] = (v >> 8) as u8;
    buf[2] = v as u8;
}

// ── fmt (chunk type) ────────────────────────────────────────────────────

/// The 2-bit `fmt` field selecting one of the 4 Chunk Message Header formats
/// (§5.3.1.1, §5.3.1.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Fmt {
    /// Type 0 (§5.3.1.2.1): the full 11-byte header.
    Type0,
    /// Type 1 (§5.3.1.2.2): 7-byte header, inherits `message stream id`.
    Type1,
    /// Type 2 (§5.3.1.2.3): 3-byte header, inherits length/type/stream id.
    Type2,
    /// Type 3 (§5.3.1.2.4): no header, inherits everything.
    Type3,
}

impl Fmt {
    /// The spec token for this `fmt` value.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Fmt::Type0 => "type 0",
            Fmt::Type1 => "type 1",
            Fmt::Type2 => "type 2",
            Fmt::Type3 => "type 3",
        }
    }

    /// Decode the 2-bit wire value (0..=3) into a [`Fmt`].
    ///
    /// # Errors
    /// [`RtmpError::Malformed`] if `bits` is not in `0..=3`.
    pub const fn from_bits(bits: u8) -> core::result::Result<Self, RtmpError> {
        match bits {
            0 => Ok(Fmt::Type0),
            1 => Ok(Fmt::Type1),
            2 => Ok(Fmt::Type2),
            3 => Ok(Fmt::Type3),
            _ => Err(RtmpError::Malformed {
                what: "chunk fmt (must be 0..=3)",
            }),
        }
    }

    /// Encode this `fmt` back to its 2-bit wire value (0..=3).
    #[must_use]
    pub const fn to_bits(self) -> u8 {
        match self {
            Fmt::Type0 => 0,
            Fmt::Type1 => 1,
            Fmt::Type2 => 2,
            Fmt::Type3 => 3,
        }
    }
}

broadcast_common::impl_spec_display!(Fmt);

// ── Basic Header (§5.3.1.1) ─────────────────────────────────────────────

/// One of the three basic header forms, chosen purely by csid range
/// (§5.3.1.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BasicHeaderForm {
    One,
    Two,
    Three,
}

/// The minimal basic-header form that can encode `csid`.
fn basic_header_form(csid: u32) -> Result<BasicHeaderForm> {
    match csid {
        BASIC_HEADER_1BYTE_MIN_CSID..=BASIC_HEADER_1BYTE_MAX_CSID => Ok(BasicHeaderForm::One),
        BASIC_HEADER_2BYTE_MIN_CSID..=BASIC_HEADER_2BYTE_MAX_CSID => Ok(BasicHeaderForm::Two),
        BASIC_HEADER_3BYTE_MIN_CSID..=BASIC_HEADER_3BYTE_MAX_CSID => Ok(BasicHeaderForm::Three),
        _ => Err(RtmpError::Malformed {
            what: "chunk stream id (must be 2..=65599)",
        }),
    }
}

/// Chunk Basic Header (§5.3.1.1): 1 to 3 bytes encoding the 2-bit `fmt` and
/// the chunk stream id (csid). Length depends only on the csid value; the
/// implementation SHOULD (and this [`Serialize`] impl does) use the smallest
/// form that holds the id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BasicHeader {
    /// Selects which of the 4 Chunk Message Header formats follows.
    pub fmt: Fmt,
    /// Chunk stream id. Valid range 2..=65599 on the wire (0/1 are the
    /// 2-/3-byte form markers, not real ids; 2 is further reserved by the
    /// spec for low-level protocol control messages/commands but is still a
    /// structurally valid basic-header value).
    pub chunk_stream_id: u32,
}

impl<'a> Parse<'a> for BasicHeader {
    type Error = RtmpError;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.is_empty() {
            return Err(RtmpError::BufferTooShort {
                need: 1,
                have: 0,
                what: "chunk basic header",
            });
        }
        let byte0 = bytes[0];
        let fmt = Fmt::from_bits((byte0 >> BASIC_HEADER_FMT_SHIFT) & 0x03)?;
        let marker = byte0 & BASIC_HEADER_MARKER_MASK;

        let chunk_stream_id = match marker {
            BASIC_HEADER_2BYTE_MARKER => {
                if bytes.len() < 2 {
                    return Err(RtmpError::BufferTooShort {
                        need: 2,
                        have: bytes.len(),
                        what: "chunk basic header (2-byte form)",
                    });
                }
                u32::from(bytes[1]) + BASIC_HEADER_CSID_OFFSET
            }
            BASIC_HEADER_3BYTE_MARKER => {
                if bytes.len() < 3 {
                    return Err(RtmpError::BufferTooShort {
                        need: 3,
                        have: bytes.len(),
                        what: "chunk basic header (3-byte form)",
                    });
                }
                // §5.3.1.1: csid = (byte2 * 256) + byte1 + 64 — byte 1 is
                // the low byte, byte 2 the high byte (little-endian).
                u32::from(bytes[1]) + u32::from(bytes[2]) * 256 + BASIC_HEADER_CSID_OFFSET
            }
            literal => u32::from(literal),
        };

        Ok(BasicHeader {
            fmt,
            chunk_stream_id,
        })
    }
}

impl Serialize for BasicHeader {
    type Error = RtmpError;

    fn serialized_len(&self) -> usize {
        match basic_header_form(self.chunk_stream_id) {
            Ok(BasicHeaderForm::One) => 1,
            Ok(BasicHeaderForm::Two) => 2,
            Ok(BasicHeaderForm::Three) => 3,
            // Out-of-range csid: nominal upper bound. `serialize_into`
            // performs the real validation and returns the error.
            Err(_) => 3,
        }
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let form = basic_header_form(self.chunk_stream_id)?;
        let fmt_bits = self.fmt.to_bits() << BASIC_HEADER_FMT_SHIFT;

        match form {
            BasicHeaderForm::One => {
                if buf.is_empty() {
                    return Err(RtmpError::BufferTooShort {
                        need: 1,
                        have: 0,
                        what: "chunk basic header output (1-byte form)",
                    });
                }
                buf[0] = fmt_bits | (self.chunk_stream_id as u8);
                Ok(1)
            }
            BasicHeaderForm::Two => {
                if buf.len() < 2 {
                    return Err(RtmpError::BufferTooShort {
                        need: 2,
                        have: buf.len(),
                        what: "chunk basic header output (2-byte form)",
                    });
                }
                buf[0] = fmt_bits | BASIC_HEADER_2BYTE_MARKER;
                buf[1] = (self.chunk_stream_id - BASIC_HEADER_CSID_OFFSET) as u8;
                Ok(2)
            }
            BasicHeaderForm::Three => {
                if buf.len() < 3 {
                    return Err(RtmpError::BufferTooShort {
                        need: 3,
                        have: buf.len(),
                        what: "chunk basic header output (3-byte form)",
                    });
                }
                buf[0] = fmt_bits | BASIC_HEADER_3BYTE_MARKER;
                let rel = self.chunk_stream_id - BASIC_HEADER_CSID_OFFSET;
                buf[1] = rel as u8;
                buf[2] = (rel >> 8) as u8;
                Ok(3)
            }
        }
    }
}

// ── Message Header (§5.3.1.2) ───────────────────────────────────────────

/// Whether `field` (a 24-bit `timestamp`/`timestamp delta`) needs the 4-byte
/// Extended Timestamp (§5.3.1.3): any value `>= EXTENDED_TIMESTAMP_MARKER`.
fn needs_extended_timestamp(field: u32) -> bool {
    field >= EXTENDED_TIMESTAMP_MARKER
}

/// Chunk Message Header: one of 4 formats selected by [`Fmt`] (§5.3.1.2),
/// carrying decreasing field sets — each format after Type 0 inherits the
/// fields it omits from the preceding chunk on the same chunk stream.
///
/// Reassembling that "preceding chunk" state (so Type 1/2/3 headers can be
/// resolved to absolute values) is a stateful job for the chunk-stream
/// reassembler (#738 Task 4), not this header type. In particular, a Type 3
/// header carrying zero bytes here does *not* by itself tell you whether an
/// Extended Timestamp follows it on the wire: per §5.3.1.3, a Type 3 chunk
/// carries the 4-byte Extended Timestamp when — and only when — the most
/// recent Type 0/1/2 chunk on the same csid itself used one. Deciding that
/// requires exactly that per-csid state, so [`MessageHeader::parse`] never
/// consumes an Extended Timestamp for `Fmt::Type3`; the reassembler must
/// apply this rule itself once it is tracking that state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MessageHeader {
    /// Type 0 (§5.3.1.2.1, 11 bytes on the wire before any Extended
    /// Timestamp). MUST be used at the start of a chunk stream and whenever
    /// the stream timestamp goes backward.
    Type0 {
        /// Absolute timestamp of the message (already resolved from any
        /// Extended Timestamp).
        timestamp: u32,
        /// Length in bytes of the whole message (not the chunk payload).
        message_length: u32,
        /// Message type id (§6/§7.1).
        message_type_id: u8,
        /// Message stream id. The wire encoding of this field alone is
        /// little-endian (§5.3.1.2.1).
        message_stream_id: u32,
    },
    /// Type 1 (§5.3.1.2.2, 7 bytes before any Extended Timestamp). No
    /// message stream id — inherits the preceding chunk's.
    Type1 {
        /// Delta from the previous chunk's timestamp on this csid (already
        /// resolved from any Extended Timestamp).
        timestamp_delta: u32,
        /// Length in bytes of the whole message.
        message_length: u32,
        /// Message type id (§6/§7.1).
        message_type_id: u8,
    },
    /// Type 2 (§5.3.1.2.3, 3 bytes before any Extended Timestamp). Neither
    /// stream id nor message length included — both inherited.
    Type2 {
        /// Delta from the previous chunk's timestamp on this csid (already
        /// resolved from any Extended Timestamp).
        timestamp_delta: u32,
    },
    /// Type 3 (§5.3.1.2.4, 0 bytes). Stream id, message length, message type
    /// id, and timestamp delta are all inherited from the preceding chunk on
    /// the same csid.
    Type3,
}

impl MessageHeader {
    /// Parse the message header that follows a [`BasicHeader`] carrying
    /// `fmt`. Returns the parsed variant and the number of bytes consumed
    /// from `bytes` — the fixed per-`fmt` header length, plus 4 more if a
    /// Type 0/1/2 field's 24-bit value read exactly [`EXTENDED_TIMESTAMP_MARKER`]
    /// (in which case the real, unresolved value is read from the following
    /// 4-byte big-endian Extended Timestamp — see §5.3.1.3).
    ///
    /// # Errors
    /// [`RtmpError::BufferTooShort`] if `bytes` does not hold the full fixed
    /// header (and, when signalled, the Extended Timestamp).
    pub fn parse(fmt: Fmt, bytes: &[u8]) -> Result<(Self, usize)> {
        match fmt {
            Fmt::Type0 => {
                if bytes.len() < TYPE0_LEN {
                    return Err(RtmpError::BufferTooShort {
                        need: TYPE0_LEN,
                        have: bytes.len(),
                        what: "type 0 message header",
                    });
                }
                let raw_timestamp = read_u24_be(&bytes[0..U24_LEN]);
                let message_length = read_u24_be(&bytes[U24_LEN..2 * U24_LEN]);
                let message_type_id = bytes[2 * U24_LEN];
                let message_stream_id =
                    u32::from_le_bytes([bytes[7], bytes[8], bytes[9], bytes[10]]);

                let (timestamp, consumed) = resolve_extended(raw_timestamp, bytes, TYPE0_LEN)?;

                Ok((
                    MessageHeader::Type0 {
                        timestamp,
                        message_length,
                        message_type_id,
                        message_stream_id,
                    },
                    consumed,
                ))
            }
            Fmt::Type1 => {
                if bytes.len() < TYPE1_LEN {
                    return Err(RtmpError::BufferTooShort {
                        need: TYPE1_LEN,
                        have: bytes.len(),
                        what: "type 1 message header",
                    });
                }
                let raw_delta = read_u24_be(&bytes[0..U24_LEN]);
                let message_length = read_u24_be(&bytes[U24_LEN..2 * U24_LEN]);
                let message_type_id = bytes[2 * U24_LEN];

                let (timestamp_delta, consumed) = resolve_extended(raw_delta, bytes, TYPE1_LEN)?;

                Ok((
                    MessageHeader::Type1 {
                        timestamp_delta,
                        message_length,
                        message_type_id,
                    },
                    consumed,
                ))
            }
            Fmt::Type2 => {
                if bytes.len() < TYPE2_LEN {
                    return Err(RtmpError::BufferTooShort {
                        need: TYPE2_LEN,
                        have: bytes.len(),
                        what: "type 2 message header",
                    });
                }
                let raw_delta = read_u24_be(&bytes[0..U24_LEN]);
                let (timestamp_delta, consumed) = resolve_extended(raw_delta, bytes, TYPE2_LEN)?;

                Ok((MessageHeader::Type2 { timestamp_delta }, consumed))
            }
            Fmt::Type3 => Ok((MessageHeader::Type3, TYPE3_LEN)),
        }
    }
}

/// Shared tail of Type 0/1/2 parsing: given the 24-bit field already read at
/// `bytes[..3]`, resolve it to its real value (reading the trailing 4-byte
/// Extended Timestamp if the field read the sentinel), and return
/// `(value, total_consumed)` where `total_consumed = fixed_len (+4)`.
fn resolve_extended(raw: u32, bytes: &[u8], fixed_len: usize) -> Result<(u32, usize)> {
    if raw == EXTENDED_TIMESTAMP_MARKER {
        let need = fixed_len + EXTENDED_TIMESTAMP_LEN;
        if bytes.len() < need {
            return Err(RtmpError::BufferTooShort {
                need,
                have: bytes.len(),
                what: "extended timestamp",
            });
        }
        let ext = u32::from_be_bytes([
            bytes[fixed_len],
            bytes[fixed_len + 1],
            bytes[fixed_len + 2],
            bytes[fixed_len + 3],
        ]);
        Ok((ext, need))
    } else {
        Ok((raw, fixed_len))
    }
}

impl Serialize for MessageHeader {
    type Error = RtmpError;

    fn serialized_len(&self) -> usize {
        match self {
            MessageHeader::Type0 { timestamp, .. } => TYPE0_LEN + extended_len(*timestamp),
            MessageHeader::Type1 {
                timestamp_delta, ..
            } => TYPE1_LEN + extended_len(*timestamp_delta),
            MessageHeader::Type2 { timestamp_delta } => TYPE2_LEN + extended_len(*timestamp_delta),
            MessageHeader::Type3 => TYPE3_LEN,
        }
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match *self {
            MessageHeader::Type0 {
                timestamp,
                message_length,
                message_type_id,
                message_stream_id,
            } => {
                let extended = needs_extended_timestamp(timestamp);
                let written = TYPE0_LEN + if extended { EXTENDED_TIMESTAMP_LEN } else { 0 };
                if buf.len() < written {
                    return Err(RtmpError::BufferTooShort {
                        need: written,
                        have: buf.len(),
                        what: "type 0 message header output",
                    });
                }
                let field = if extended {
                    EXTENDED_TIMESTAMP_MARKER
                } else {
                    timestamp
                };
                write_u24_be(field, &mut buf[0..U24_LEN]);
                write_u24_be(message_length, &mut buf[U24_LEN..2 * U24_LEN]);
                buf[2 * U24_LEN] = message_type_id;
                buf[7..11].copy_from_slice(&message_stream_id.to_le_bytes());
                if extended {
                    buf[11..15].copy_from_slice(&timestamp.to_be_bytes());
                }
                Ok(written)
            }
            MessageHeader::Type1 {
                timestamp_delta,
                message_length,
                message_type_id,
            } => {
                let extended = needs_extended_timestamp(timestamp_delta);
                let written = TYPE1_LEN + if extended { EXTENDED_TIMESTAMP_LEN } else { 0 };
                if buf.len() < written {
                    return Err(RtmpError::BufferTooShort {
                        need: written,
                        have: buf.len(),
                        what: "type 1 message header output",
                    });
                }
                let field = if extended {
                    EXTENDED_TIMESTAMP_MARKER
                } else {
                    timestamp_delta
                };
                write_u24_be(field, &mut buf[0..U24_LEN]);
                write_u24_be(message_length, &mut buf[U24_LEN..2 * U24_LEN]);
                buf[2 * U24_LEN] = message_type_id;
                if extended {
                    buf[7..11].copy_from_slice(&timestamp_delta.to_be_bytes());
                }
                Ok(written)
            }
            MessageHeader::Type2 { timestamp_delta } => {
                let extended = needs_extended_timestamp(timestamp_delta);
                let written = TYPE2_LEN + if extended { EXTENDED_TIMESTAMP_LEN } else { 0 };
                if buf.len() < written {
                    return Err(RtmpError::BufferTooShort {
                        need: written,
                        have: buf.len(),
                        what: "type 2 message header output",
                    });
                }
                let field = if extended {
                    EXTENDED_TIMESTAMP_MARKER
                } else {
                    timestamp_delta
                };
                write_u24_be(field, &mut buf[0..U24_LEN]);
                if extended {
                    buf[3..7].copy_from_slice(&timestamp_delta.to_be_bytes());
                }
                Ok(written)
            }
            MessageHeader::Type3 => Ok(0),
        }
    }
}

/// Extra bytes (0 or 4) [`Serialize`] will write for a 24-bit
/// `timestamp`/`timestamp delta` value.
fn extended_len(field: u32) -> usize {
    if needs_extended_timestamp(field) {
        EXTENDED_TIMESTAMP_LEN
    } else {
        0
    }
}

// ── Message (the assembled unit) ────────────────────────────────────────

/// One fully reassembled RTMP message: the payload of a single message
/// stream at a single (resolved, absolute) timestamp (§6.1). Produced by
/// [`ChunkAssembler::push`] and consumed by [`ChunkWriter::write`].
///
/// Task 5 (`message.rs`) adds typed interpretation of `payload`/
/// `message_type_id`; this carrier stays stable underneath that.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// Chunk stream id this message was carried on.
    pub chunk_stream_id: u32,
    /// Absolute timestamp (already resolved from any timestamp delta /
    /// Extended Timestamp — never a delta).
    pub timestamp: u32,
    /// Message type id (§6/§7.1).
    pub message_type_id: u8,
    /// Message stream id.
    pub message_stream_id: u32,
    /// The whole message payload (reassembled across every chunk it was
    /// split into).
    pub payload: Vec<u8>,
}

// ── ChunkAssembler (stateful inbound reassembly, §5.3) ──────────────────

/// Per-csid reassembly state: the most recently resolved header fields (used
/// by `fmt` 1/2/3 to inherit the fields they omit) plus the payload
/// accumulated so far for the message currently in progress on this csid.
#[derive(Debug, Clone, Default)]
struct CsidState {
    /// Absolute timestamp of the current/most recent message on this csid.
    timestamp: u32,
    /// Delta most recently applied to reach `timestamp` — re-applied
    /// unchanged when a Type 3 chunk begins a new message (inherits the
    /// prior delta). Per §3.1.2 Type 3: when a Type 3 chunk immediately
    /// follows a Type 0 chunk with no intervening Type 1/2, its implied
    /// delta equals the Type 0 chunk's own absolute timestamp — so a Type 0
    /// chunk seeds this field with its `timestamp`, not `0`.
    timestamp_delta: u32,
    /// Total length in bytes of the current/most recent message.
    message_length: u32,
    /// Message type id of the current/most recent message.
    message_type_id: u8,
    /// Message stream id of the current/most recent message.
    message_stream_id: u32,
    /// Whether the most recent Type 0/1/2 header on this csid used the
    /// Extended Timestamp (§5.3.1.3) — a Type 3 chunk then also carries (and
    /// must consume) that 4-byte field, per csid, until the next Type 0/1/2
    /// changes the flag.
    extended: bool,
    /// Whether a Type 0/1/2 header has ever been seen on this csid (Type
    /// 1/2/3 headers inherit from it; nothing to inherit before the first
    /// Type 0).
    initialized: bool,
    /// Whether a message is currently mid-accumulation on this csid (a
    /// prior chunk started it but its `message_length` bytes are not all
    /// in yet). Distinguishes, for a Type 3 chunk, a **continuation** of
    /// that in-progress message (`true`) from the **start of a new**
    /// message reusing the prior header (`false`, at a message boundary) —
    /// `payload.len()` alone can't tell them apart once a message has
    /// completed and `payload` was reset to empty for the next one.
    in_progress: bool,
    /// Payload bytes accumulated so far for the message in progress
    /// (`message_length` total once complete). Reset to empty once a
    /// message completes.
    payload: Vec<u8>,
}

/// Stateful inbound chunk reassembler (§5.3): feed inbound bytes, get back
/// each complete [`Message`] as soon as its last chunk arrives.
///
/// Maintains per-`chunk_stream_id` state, so multiple chunk streams may be
/// interleaved on the same connection (as the wire format requires) and are
/// each reassembled independently.
#[derive(Debug)]
pub struct ChunkAssembler {
    chunk_size: u32,
    csids: HashMap<u32, CsidState>,
    /// Bytes carried over from a previous `push` call that did not yet form
    /// a complete chunk (partial basic header, message header, extended
    /// timestamp, or payload slice).
    pending: Vec<u8>,
}

impl Default for ChunkAssembler {
    fn default() -> Self {
        Self::new()
    }
}

impl ChunkAssembler {
    /// New assembler, chunk size at the §5.3 default (128 bytes).
    #[must_use]
    pub fn new() -> Self {
        Self {
            chunk_size: DEFAULT_CHUNK_SIZE,
            csids: HashMap::new(),
            pending: Vec::new(),
        }
    }

    /// Update the chunk size in effect for subsequent chunks (called on
    /// receipt of a Set Chunk Size protocol control message, §5.4.1). Floored
    /// at 1 (a chunk size of 0 would never make progress splitting payload)
    /// and capped at [`MAX_CHUNK_SIZE`], matching
    /// [`ChunkWriter::set_chunk_size`]'s floor/cap.
    pub fn set_chunk_size(&mut self, n: u32) {
        self.chunk_size = n.clamp(1, MAX_CHUNK_SIZE);
    }

    /// Feed inbound bytes; returns each complete [`Message`] decoded from
    /// the buffer (in arrival order), leaving any trailing partial chunk or
    /// partial message buffered internally for the next call.
    ///
    /// Callers that need to react to a message's side effects (e.g. a Set
    /// Chunk Size protocol control message, §5.4.1) **before** parsing the
    /// bytes that follow it in the same input — because the sender may
    /// switch to the new chunk size for its very next chunk — should use the
    /// crate-internal incremental `feed`/`next_message` pair instead,
    /// dispatching each message immediately. `push` collects every message
    /// from `input` under a single, unchanging `chunk_size`, which
    /// misparses a buffer that itself contains a Set Chunk Size followed by
    /// chunks already framed at the new size (see
    /// [`ServerSession`](crate::server::ServerSession), which uses the
    /// incremental form for exactly this reason).
    ///
    /// # Errors
    /// [`RtmpError::Malformed`] on structurally invalid input (e.g. a Type
    /// 1/2/3 chunk on a csid that has never seen a Type 0). Never errors
    /// merely because the input ends mid-chunk — that is buffered, not an
    /// error.
    pub fn push(&mut self, input: &[u8]) -> Result<Vec<Message>> {
        self.feed(input);
        let mut out = Vec::new();
        while let Some(msg) = self.next_message()? {
            out.push(msg);
        }
        Ok(out)
    }

    /// Buffer inbound bytes without parsing them yet. Pair with repeated
    /// calls to [`next_message`](Self::next_message) to parse and dispatch
    /// one message at a time (see [`push`](Self::push)'s docs for why this
    /// matters for Set Chunk Size).
    pub(crate) fn feed(&mut self, input: &[u8]) {
        self.pending.extend_from_slice(input);
    }

    /// Parse and return the next complete [`Message`] out of the
    /// previously-[`feed`](Self::feed) bytes, or `Ok(None)` if what remains
    /// buffered isn't (yet) a complete message. Internally keeps parsing
    /// individual chunks — which may belong to other interleaved csids, or
    /// be a non-final chunk of the same in-progress message — until either a
    /// full message is assembled or the buffered bytes run out.
    ///
    /// # Errors
    /// Same as [`push`](Self::push).
    pub(crate) fn next_message(&mut self) -> Result<Option<Message>> {
        loop {
            match Self::try_parse_one(&self.pending, &self.csids, self.chunk_size) {
                Ok(Some(parsed)) => {
                    self.pending.drain(..parsed.consumed);
                    let state = self.csids.entry(parsed.csid).or_default();
                    state.timestamp = parsed.timestamp;
                    state.timestamp_delta = parsed.timestamp_delta;
                    state.message_length = parsed.message_length;
                    state.message_type_id = parsed.message_type_id;
                    state.message_stream_id = parsed.message_stream_id;
                    state.extended = parsed.extended;
                    state.initialized = true;
                    if parsed.payload.len() as u32 == parsed.message_length {
                        state.payload.clear();
                        state.in_progress = false;
                        return Ok(Some(Message {
                            chunk_stream_id: parsed.csid,
                            timestamp: parsed.timestamp,
                            message_type_id: parsed.message_type_id,
                            message_stream_id: parsed.message_stream_id,
                            payload: parsed.payload,
                        }));
                    }
                    state.payload = parsed.payload;
                    state.in_progress = true;
                    // This chunk only partially filled its message (or
                    // belongs to a different, interleaved csid) — keep
                    // looping to try the next chunk in `pending`.
                }
                Ok(None) => return Ok(None),
                Err(e) => return Err(e),
            }
        }
    }

    /// Attempt to parse exactly one chunk (basic header + message header +
    /// any extended timestamp + its payload slice) from the front of `buf`,
    /// resolving it against the existing per-csid `states` without mutating
    /// them. Returns:
    /// - `Ok(Some(_))` — a full chunk was parsed; `consumed` bytes should be
    ///   dropped from the front of the caller's buffer and the returned
    ///   resolved fields committed to that csid's state.
    /// - `Ok(None)` — not enough bytes yet for a full chunk (structurally
    ///   plausible so far); caller should wait for more input.
    /// - `Err(_)` — structurally invalid input.
    fn try_parse_one(
        buf: &[u8],
        states: &HashMap<u32, CsidState>,
        chunk_size: u32,
    ) -> Result<Option<ParsedChunk>> {
        let bh = match BasicHeader::parse(buf) {
            Ok(bh) => bh,
            Err(RtmpError::BufferTooShort { .. }) => return Ok(None),
            Err(e) => return Err(e),
        };
        // `BasicHeader::parse` having succeeded means `buf` holds at least
        // as many bytes as this form needs; re-derive that count from the
        // marker bits actually on the wire (not from `basic_header_form`,
        // which reflects the *minimal* form for the csid and may disagree
        // with a wire that legally used a longer form for a csid in the
        // 64..=319 overlap range).
        let marker = buf[0] & BASIC_HEADER_MARKER_MASK;
        let header_len = match marker {
            BASIC_HEADER_2BYTE_MARKER => 2,
            BASIC_HEADER_3BYTE_MARKER => 3,
            _ => 1,
        };

        let existing = states.get(&bh.chunk_stream_id);

        // Remote-DoS guard: a flood of chunks opening distinct, previously
        // unseen csids would otherwise grow `states` without bound (one
        // `CsidState`, each with its own payload buffer, per bogus csid).
        // Reject before the caller ever inserts a new entry for this csid.
        if existing.is_none() && states.len() >= MAX_CSIDS {
            return Err(RtmpError::Malformed {
                what: "too many concurrent chunk stream ids (csid flood)",
            });
        }

        let rest = &buf[header_len..];
        let (mh, mh_consumed) = match MessageHeader::parse(bh.fmt, rest) {
            Ok(v) => v,
            Err(RtmpError::BufferTooShort { .. }) => return Ok(None),
            Err(e) => return Err(e),
        };
        let mut consumed = header_len + mh_consumed;

        // Resolve this chunk's header fields (timestamp/length/type/stream
        // id/extended-flag) and whether it begins a new message or
        // continues the one already in progress on this csid.
        let (resolved, starts_new) = match (bh.fmt, mh) {
            (
                Fmt::Type0,
                MessageHeader::Type0 {
                    timestamp,
                    message_length,
                    message_type_id,
                    message_stream_id,
                },
            ) => {
                let used_extended = mh_consumed > TYPE0_LEN;
                (
                    ResolvedHeader {
                        // §3.1.2: a Type 3 immediately following this Type 0
                        // (no intervening Type 1/2) implies a delta equal to
                        // this Type 0's own absolute timestamp.
                        timestamp_delta: timestamp,
                        timestamp,
                        message_length,
                        message_type_id,
                        message_stream_id,
                        extended: used_extended,
                    },
                    true,
                )
            }
            // TODO(#738 follow-up): a Type 1/2 header arriving while a
            // message is already `in_progress` on this csid (a header
            // interleaved mid-message, rather than at a message boundary)
            // is not detected here — it silently resets `payload`/state and
            // drops the in-flight bytes rather than erroring. Real streams
            // shouldn't do this, but a malformed/desynced one could; needs
            // its own test + design before implementing.
            (
                Fmt::Type1,
                MessageHeader::Type1 {
                    timestamp_delta,
                    message_length,
                    message_type_id,
                },
            ) => {
                let existing = existing.ok_or(RtmpError::Malformed {
                    what: "type 1 chunk header on a csid with no prior chunk to inherit from",
                })?;
                let used_extended = mh_consumed > TYPE1_LEN;
                (
                    ResolvedHeader {
                        timestamp: existing.timestamp.wrapping_add(timestamp_delta),
                        timestamp_delta,
                        message_length,
                        message_type_id,
                        message_stream_id: existing.message_stream_id,
                        extended: used_extended,
                    },
                    true,
                )
            }
            (Fmt::Type2, MessageHeader::Type2 { timestamp_delta }) => {
                let existing = existing.ok_or(RtmpError::Malformed {
                    what: "type 2 chunk header on a csid with no prior chunk to inherit from",
                })?;
                let used_extended = mh_consumed > TYPE2_LEN;
                (
                    ResolvedHeader {
                        timestamp: existing.timestamp.wrapping_add(timestamp_delta),
                        timestamp_delta,
                        message_length: existing.message_length,
                        message_type_id: existing.message_type_id,
                        message_stream_id: existing.message_stream_id,
                        extended: used_extended,
                    },
                    true,
                )
            }
            (Fmt::Type3, MessageHeader::Type3) => {
                let existing = existing.ok_or(RtmpError::Malformed {
                    what: "type 3 chunk header on a csid with no prior chunk to inherit from",
                })?;
                let continuation = existing.in_progress;
                if existing.extended {
                    if buf.len() < consumed + EXTENDED_TIMESTAMP_LEN {
                        return Ok(None);
                    }
                    // Present per §3.1.3 whenever the most recent Type 0/1/2
                    // on this csid used one. A continuation chunk's message
                    // timestamp is already fixed (ignore the value); a
                    // new-message Type 3 re-applies the inherited delta
                    // (also ignoring the value: Type 3 has nothing of its
                    // own to contribute, by definition it inherits).
                    consumed += EXTENDED_TIMESTAMP_LEN;
                }
                if continuation {
                    (
                        ResolvedHeader {
                            timestamp: existing.timestamp,
                            timestamp_delta: existing.timestamp_delta,
                            message_length: existing.message_length,
                            message_type_id: existing.message_type_id,
                            message_stream_id: existing.message_stream_id,
                            extended: existing.extended,
                        },
                        false,
                    )
                } else {
                    (
                        ResolvedHeader {
                            timestamp: existing.timestamp.wrapping_add(existing.timestamp_delta),
                            timestamp_delta: existing.timestamp_delta,
                            message_length: existing.message_length,
                            message_type_id: existing.message_type_id,
                            message_stream_id: existing.message_stream_id,
                            extended: existing.extended,
                        },
                        true,
                    )
                }
            }
            // `MessageHeader::parse` is always called with the `Fmt` that
            // selects its own variant, so every other pairing is
            // unreachable.
            _ => unreachable!("MessageHeader::parse always returns the variant for its Fmt"),
        };

        // Remote-DoS guard: `message_length` is a fully attacker-controlled
        // 24-bit wire field (Type 0/1 headers set it directly; Type 2/3
        // inherit an already-checked value). Reject before any payload
        // buffer for this message is allocated — see `MAX_MESSAGE_LEN`'s
        // doc for why this bound is safe for real RTMP traffic.
        if resolved.message_length > MAX_MESSAGE_LEN {
            return Err(RtmpError::Malformed {
                what: "message length exceeds the maximum accepted message size",
            });
        }

        let already_accumulated = if starts_new {
            0
        } else {
            existing.map(|s| s.payload.len()).unwrap_or(0)
        };
        let remaining_needed =
            (resolved.message_length as usize).saturating_sub(already_accumulated);
        let take = (chunk_size as usize).min(remaining_needed);

        if buf.len() < consumed + take {
            return Ok(None);
        }

        // No `Vec::with_capacity(resolved.message_length)` here: that would
        // pre-reserve up to `MAX_MESSAGE_LEN` bytes off a single attacker-
        // supplied header field, before a single payload byte has actually
        // arrived. The payload instead grows incrementally via
        // `extend_from_slice` below, chunk by chunk, as real bytes show up —
        // pre-reserving the claimed length buys almost nothing since the
        // data arrives in `chunk_size` pieces anyway.
        let mut payload = if starts_new {
            Vec::new()
        } else {
            existing.map(|s| s.payload.clone()).unwrap_or_default()
        };
        payload.extend_from_slice(&buf[consumed..consumed + take]);
        consumed += take;

        Ok(Some(ParsedChunk {
            csid: bh.chunk_stream_id,
            consumed,
            timestamp: resolved.timestamp,
            timestamp_delta: resolved.timestamp_delta,
            message_length: resolved.message_length,
            message_type_id: resolved.message_type_id,
            message_stream_id: resolved.message_stream_id,
            extended: resolved.extended,
            payload,
        }))
    }
}

/// Header fields resolved for one chunk, after applying `fmt`-specific
/// inheritance from the csid's prior state.
struct ResolvedHeader {
    timestamp: u32,
    timestamp_delta: u32,
    message_length: u32,
    message_type_id: u8,
    message_stream_id: u32,
    extended: bool,
}

/// One fully-parsed chunk (header resolved + its payload slice taken),
/// ready to be committed to the owning [`ChunkAssembler`]'s per-csid state.
struct ParsedChunk {
    csid: u32,
    consumed: usize,
    timestamp: u32,
    timestamp_delta: u32,
    message_length: u32,
    message_type_id: u8,
    message_stream_id: u32,
    extended: bool,
    payload: Vec<u8>,
}

// ── ChunkWriter (outbound, §5.3) ─────────────────────────────────────────

/// Stateless-per-message outbound chunk writer (§5.3): serializes a
/// [`Message`] into chunk bytes at the current chunk size.
///
/// Simple, always-correct strategy: the first chunk is always Type 0 (full
/// absolute-timestamp header) and every continuation chunk is Type 3
/// (0-byte header, inheriting everything). This is spec-valid — Type 1/2's
/// more compact delta-based headers are a size optimisation this writer
/// does not perform.
#[derive(Debug, Clone)]
pub struct ChunkWriter {
    chunk_size: u32,
}

impl Default for ChunkWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl ChunkWriter {
    /// New writer, chunk size at the §5.3 default (128 bytes).
    #[must_use]
    pub fn new() -> Self {
        Self {
            chunk_size: DEFAULT_CHUNK_SIZE,
        }
    }

    /// Update the chunk size used for subsequent [`ChunkWriter::write`]
    /// calls (called on sending a Set Chunk Size protocol control message,
    /// §5.4.1). Capped at [`MAX_CHUNK_SIZE`] (the floor-of-1 is applied in
    /// [`ChunkWriter::write`] itself).
    pub fn set_chunk_size(&mut self, n: u32) {
        self.chunk_size = n.min(MAX_CHUNK_SIZE);
    }

    /// Serialize `msg` into chunk bytes at the current chunk size: a Type 0
    /// first chunk carrying up to `chunk_size` payload bytes, then Type 3
    /// continuation chunks for the remainder.
    ///
    /// # Panics
    /// If `msg.chunk_stream_id` is outside the basic header's encodable
    /// range (2..=65599) — the same precondition [`BasicHeader::serialize_into`]
    /// enforces. Every `chunk_stream_id` produced by [`ChunkAssembler::push`]
    /// satisfies this (`BasicHeader::parse` never yields one outside the
    /// range), so a `Message` round-tripped from the assembler never panics
    /// here; callers building a `Message` from scratch must respect it.
    #[must_use]
    pub fn write(&mut self, msg: &Message) -> Vec<u8> {
        let chunk_size = (self.chunk_size as usize).max(1);
        let message_length = msg.payload.len() as u32;
        let extended = needs_extended_timestamp(msg.timestamp);

        let mut out = Vec::with_capacity(TYPE0_LEN + msg.payload.len() + 16);

        let bh0 = BasicHeader {
            fmt: Fmt::Type0,
            chunk_stream_id: msg.chunk_stream_id,
        };
        let mh0 = MessageHeader::Type0 {
            timestamp: msg.timestamp,
            message_length,
            message_type_id: msg.message_type_id,
            message_stream_id: msg.message_stream_id,
        };
        write_serialized(&mut out, &bh0);
        write_serialized(&mut out, &mh0);

        let mut offset = 0usize;
        let take0 = chunk_size.min(msg.payload.len());
        out.extend_from_slice(&msg.payload[offset..offset + take0]);
        offset += take0;

        while offset < msg.payload.len() {
            let bh = BasicHeader {
                fmt: Fmt::Type3,
                chunk_stream_id: msg.chunk_stream_id,
            };
            write_serialized(&mut out, &bh);
            if extended {
                out.extend_from_slice(&msg.timestamp.to_be_bytes());
            }
            let take = chunk_size.min(msg.payload.len() - offset);
            out.extend_from_slice(&msg.payload[offset..offset + take]);
            offset += take;
        }

        out
    }
}

/// Serialize `item` and append the bytes to `out`.
///
/// # Panics
/// If `item.serialize_into` errors (only possible, for the [`BasicHeader`]s
/// this is used with, when `chunk_stream_id` is outside the encodable
/// range) — see [`ChunkWriter::write`]'s panics section.
fn write_serialized<T: Serialize<Error = RtmpError>>(out: &mut Vec<u8>, item: &T) {
    let len = item.serialized_len();
    let start = out.len();
    out.resize(start + len, 0);
    let n = item
        .serialize_into(&mut out[start..])
        .expect("valid chunk_stream_id (2..=65599) is a ChunkWriter::write precondition");
    out.truncate(start + n);
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── u24 helper ───────────────────────────────────────────────────────

    #[test]
    fn u24_round_trip_zero() {
        let mut buf = [0xFFu8; U24_LEN];
        write_u24_be(0, &mut buf);
        assert_eq!(buf, [0, 0, 0]);
        assert_eq!(read_u24_be(&buf), 0);
    }

    #[test]
    fn u24_round_trip_max() {
        let mut buf = [0u8; U24_LEN];
        write_u24_be(0x00FF_FFFF, &mut buf);
        assert_eq!(buf, [0xFF, 0xFF, 0xFF]);
        assert_eq!(read_u24_be(&buf), 0x00FF_FFFF);
    }

    #[test]
    fn u24_round_trip_mid_value() {
        let mut buf = [0u8; U24_LEN];
        write_u24_be(0x0012_3456, &mut buf);
        assert_eq!(buf, [0x12, 0x34, 0x56]);
        assert_eq!(read_u24_be(&buf), 0x0012_3456);
    }

    // ── Fmt ──────────────────────────────────────────────────────────────

    #[test]
    fn fmt_from_bits_round_trip() {
        for (bits, fmt) in [
            (0u8, Fmt::Type0),
            (1, Fmt::Type1),
            (2, Fmt::Type2),
            (3, Fmt::Type3),
        ] {
            let parsed = Fmt::from_bits(bits).unwrap();
            assert_eq!(parsed, fmt);
            assert_eq!(parsed.to_bits(), bits);
        }
    }

    #[test]
    fn fmt_from_bits_out_of_range_is_malformed() {
        assert!(matches!(
            Fmt::from_bits(4),
            Err(RtmpError::Malformed { .. })
        ));
    }

    #[test]
    fn fmt_display_matches_name() {
        assert_eq!(Fmt::Type0.to_string(), "type 0");
        assert_eq!(Fmt::Type3.to_string(), "type 3");
    }

    // ── BasicHeader: 1-byte form ─────────────────────────────────────────

    #[test]
    fn basic_header_one_byte_form_round_trip_build_serialize_parse() {
        for csid in [BASIC_HEADER_1BYTE_MIN_CSID, 5, BASIC_HEADER_1BYTE_MAX_CSID] {
            let bh = BasicHeader {
                fmt: Fmt::Type1,
                chunk_stream_id: csid,
            };
            let mut buf = [0u8; 1];
            let n = bh.serialize_into(&mut buf).unwrap();
            assert_eq!(n, 1, "csid {csid} must use the 1-byte form");
            let parsed = BasicHeader::parse(&buf).unwrap();
            assert_eq!(parsed, bh);
        }
    }

    #[test]
    fn basic_header_one_byte_form_parse_serialize_byte_identical() {
        // fmt=1 (bits 01), csid=5: byte0 = 0b01_000101 = 0x45.
        let bytes = [0x45u8];
        let bh = BasicHeader::parse(&bytes).unwrap();
        assert_eq!(bh.fmt, Fmt::Type1);
        assert_eq!(bh.chunk_stream_id, 5);
        let mut buf = [0u8; 1];
        bh.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, bytes);
    }

    // ── BasicHeader: 2-byte form ─────────────────────────────────────────

    #[test]
    fn basic_header_two_byte_form_round_trip_boundaries() {
        for csid in [
            BASIC_HEADER_2BYTE_MIN_CSID,
            200,
            BASIC_HEADER_2BYTE_MAX_CSID,
        ] {
            let bh = BasicHeader {
                fmt: Fmt::Type2,
                chunk_stream_id: csid,
            };
            let mut buf = [0u8; 2];
            let n = bh.serialize_into(&mut buf).unwrap();
            assert_eq!(n, 2, "csid {csid} must use the minimal 2-byte form");
            let parsed = BasicHeader::parse(&buf).unwrap();
            assert_eq!(parsed, bh);
        }
    }

    #[test]
    fn basic_header_two_byte_form_parse_serialize_byte_identical() {
        // fmt=2 (bits 10), marker 0, csid-64 = 0 => csid 64.
        let bytes = [0b10_000000u8, 0x00];
        let bh = BasicHeader::parse(&bytes).unwrap();
        assert_eq!(bh.fmt, Fmt::Type2);
        assert_eq!(bh.chunk_stream_id, 64);
        let mut buf = [0u8; 2];
        bh.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, bytes);
    }

    // ── BasicHeader: 3-byte form ─────────────────────────────────────────

    #[test]
    fn basic_header_three_byte_form_round_trip_boundaries() {
        for csid in [
            BASIC_HEADER_3BYTE_MIN_CSID,
            40000,
            BASIC_HEADER_3BYTE_MAX_CSID,
        ] {
            let bh = BasicHeader {
                fmt: Fmt::Type3,
                chunk_stream_id: csid,
            };
            let mut buf = [0u8; 3];
            let n = bh.serialize_into(&mut buf).unwrap();
            assert_eq!(n, 3, "csid {csid} must use the 3-byte form");
            let parsed = BasicHeader::parse(&buf).unwrap();
            assert_eq!(parsed, bh);
        }
    }

    #[test]
    fn basic_header_three_byte_form_parse_serialize_byte_identical() {
        // fmt=0, marker 1, csid-64 = 0xFFFF (LE: byte1=0xFF, byte2=0xFF) => csid 65599.
        let bytes = [0b00_000001u8, 0xFF, 0xFF];
        let bh = BasicHeader::parse(&bytes).unwrap();
        assert_eq!(bh.fmt, Fmt::Type0);
        assert_eq!(bh.chunk_stream_id, BASIC_HEADER_3BYTE_MAX_CSID);
        let mut buf = [0u8; 3];
        bh.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, bytes);
    }

    #[test]
    fn basic_header_2byte_and_3byte_csid_are_little_endian() {
        // csid = 64 + 0x0102 = 0x0142 = 322. 3-byte form: byte1=low=0x02, byte2=high=0x01.
        let bh = BasicHeader {
            fmt: Fmt::Type0,
            chunk_stream_id: 64 + 0x0102,
        };
        let mut buf = [0u8; 3];
        bh.serialize_into(&mut buf).unwrap();
        assert_eq!(buf[1], 0x02, "low byte of csid-64 must come first");
        assert_eq!(buf[2], 0x01, "high byte of csid-64 must come second");
        assert_eq!(BasicHeader::parse(&buf).unwrap(), bh);
    }

    // ── BasicHeader: errors ──────────────────────────────────────────────

    #[test]
    fn basic_header_csid_zero_is_malformed_on_serialize() {
        let bh = BasicHeader {
            fmt: Fmt::Type0,
            chunk_stream_id: 0,
        };
        let mut buf = [0u8; 3];
        assert!(matches!(
            bh.serialize_into(&mut buf),
            Err(RtmpError::Malformed { .. })
        ));
    }

    #[test]
    fn basic_header_csid_one_is_malformed_on_serialize() {
        let bh = BasicHeader {
            fmt: Fmt::Type0,
            chunk_stream_id: 1,
        };
        let mut buf = [0u8; 3];
        assert!(matches!(
            bh.serialize_into(&mut buf),
            Err(RtmpError::Malformed { .. })
        ));
    }

    #[test]
    fn basic_header_csid_above_max_is_malformed_on_serialize() {
        let bh = BasicHeader {
            fmt: Fmt::Type0,
            chunk_stream_id: BASIC_HEADER_3BYTE_MAX_CSID + 1,
        };
        let mut buf = [0u8; 3];
        assert!(matches!(
            bh.serialize_into(&mut buf),
            Err(RtmpError::Malformed { .. })
        ));
    }

    #[test]
    fn basic_header_empty_input_is_buffer_too_short() {
        assert!(matches!(
            BasicHeader::parse(&[]),
            Err(RtmpError::BufferTooShort {
                need: 1,
                have: 0,
                ..
            })
        ));
    }

    #[test]
    fn basic_header_truncated_two_byte_form_is_buffer_too_short() {
        let bytes = [0b00_000000u8]; // marker=0 (2-byte form) but only 1 byte given.
        assert!(matches!(
            BasicHeader::parse(&bytes),
            Err(RtmpError::BufferTooShort {
                need: 2,
                have: 1,
                ..
            })
        ));
    }

    #[test]
    fn basic_header_truncated_three_byte_form_is_buffer_too_short() {
        let bytes = [0b00_000001u8, 0xAB]; // marker=1 (3-byte form) but only 2 bytes given.
        assert!(matches!(
            BasicHeader::parse(&bytes),
            Err(RtmpError::BufferTooShort {
                need: 3,
                have: 2,
                ..
            })
        ));
    }

    // ── MessageHeader: Type 0 ────────────────────────────────────────────

    #[test]
    fn type0_round_trip_build_serialize_parse_no_extended() {
        let mh = MessageHeader::Type0 {
            timestamp: 0x0011_2233,
            message_length: 0x0004_5566,
            message_type_id: 0x09,
            message_stream_id: 0xAABB_CCDD,
        };
        let mut buf = [0u8; TYPE0_LEN];
        let n = mh.serialize_into(&mut buf).unwrap();
        assert_eq!(n, TYPE0_LEN);
        let (parsed, consumed) = MessageHeader::parse(Fmt::Type0, &buf).unwrap();
        assert_eq!(consumed, TYPE0_LEN);
        assert_eq!(parsed, mh);
    }

    #[test]
    fn type0_parse_serialize_byte_identical_no_extended_and_le_stream_id() {
        // timestamp = 0x001122 (< marker, no extension).
        // message_length = 0x334455.
        // message_type_id = 0x09.
        // message_stream_id = 0xAABBCCDD, wire LE => DD CC BB AA.
        let bytes: [u8; TYPE0_LEN] = [
            0x00, 0x11, 0x22, // timestamp
            0x33, 0x44, 0x55, // message_length
            0x09, // message_type_id
            0xDD, 0xCC, 0xBB, 0xAA, // message_stream_id, little-endian
        ];
        let (mh, consumed) = MessageHeader::parse(Fmt::Type0, &bytes).unwrap();
        assert_eq!(consumed, TYPE0_LEN);
        assert_eq!(
            mh,
            MessageHeader::Type0 {
                timestamp: 0x0000_1122,
                message_length: 0x0033_4455,
                message_type_id: 0x09,
                message_stream_id: 0xAABB_CCDD,
            }
        );
        let mut buf = [0u8; TYPE0_LEN];
        mh.serialize_into(&mut buf).unwrap();
        assert_eq!(
            buf, bytes,
            "byte-identical round trip, LE stream id included"
        );
    }

    #[test]
    fn type0_extended_timestamp_parse_serialize_byte_identical() {
        // 24-bit timestamp field = sentinel 0xFFFFFF => extended 4-byte BE
        // timestamp follows, value 0x01020304 (chosen so BE != LE, catching
        // an endianness bug in the extended field).
        let bytes: [u8; TYPE0_LEN + 4] = [
            0xFF, 0xFF, 0xFF, // timestamp sentinel
            0x00, 0x00, 0x10, // message_length
            0x08, // message_type_id
            0x01, 0x00, 0x00, 0x00, // message_stream_id = 1, LE
            0x01, 0x02, 0x03, 0x04, // extended timestamp, big-endian
        ];
        let (mh, consumed) = MessageHeader::parse(Fmt::Type0, &bytes).unwrap();
        assert_eq!(consumed, TYPE0_LEN + 4);
        assert_eq!(
            mh,
            MessageHeader::Type0 {
                timestamp: 0x0102_0304,
                message_length: 0x0000_0010,
                message_type_id: 0x08,
                message_stream_id: 1,
            }
        );
        let mut buf = [0u8; TYPE0_LEN + 4];
        let n = mh.serialize_into(&mut buf).unwrap();
        assert_eq!(n, TYPE0_LEN + 4);
        assert_eq!(
            buf, bytes,
            "extended timestamp path must round-trip byte-identically"
        );
    }

    #[test]
    fn type0_timestamp_exactly_at_marker_boundary_uses_extended_path() {
        // timestamp == EXTENDED_TIMESTAMP_MARKER exactly: per spec this MUST
        // still go through the extended-timestamp path (">=", not ">").
        let mh = MessageHeader::Type0 {
            timestamp: EXTENDED_TIMESTAMP_MARKER,
            message_length: 10,
            message_type_id: 1,
            message_stream_id: 0,
        };
        assert_eq!(mh.serialized_len(), TYPE0_LEN + 4);
        let mut buf = [0u8; TYPE0_LEN + 4];
        let n = mh.serialize_into(&mut buf).unwrap();
        assert_eq!(n, TYPE0_LEN + 4);
        assert_eq!(
            &buf[0..3],
            [0xFF, 0xFF, 0xFF],
            "24-bit field must be the sentinel"
        );
        assert_eq!(
            &buf[11..15],
            &EXTENDED_TIMESTAMP_MARKER.to_be_bytes()[..],
            "extended field carries the real value"
        );
        let (parsed, consumed) = MessageHeader::parse(Fmt::Type0, &buf).unwrap();
        assert_eq!(consumed, TYPE0_LEN + 4);
        assert_eq!(parsed, mh);
    }

    // ── MessageHeader: Type 1 ────────────────────────────────────────────

    #[test]
    fn type1_round_trip_build_serialize_parse_no_extended() {
        let mh = MessageHeader::Type1 {
            timestamp_delta: 20,
            message_length: 32,
            message_type_id: 8,
        };
        let mut buf = [0u8; TYPE1_LEN];
        let n = mh.serialize_into(&mut buf).unwrap();
        assert_eq!(n, TYPE1_LEN);
        let (parsed, consumed) = MessageHeader::parse(Fmt::Type1, &buf).unwrap();
        assert_eq!(consumed, TYPE1_LEN);
        assert_eq!(parsed, mh);
    }

    #[test]
    fn type1_extended_timestamp_parse_serialize_byte_identical() {
        let bytes: [u8; TYPE1_LEN + 4] = [
            0xFF, 0xFF, 0xFF, // timestamp_delta sentinel
            0x00, 0x00, 0x20, // message_length
            0x09, // message_type_id
            0x0A, 0x0B, 0x0C, 0x0D, // extended timestamp delta, big-endian
        ];
        let (mh, consumed) = MessageHeader::parse(Fmt::Type1, &bytes).unwrap();
        assert_eq!(consumed, TYPE1_LEN + 4);
        assert_eq!(
            mh,
            MessageHeader::Type1 {
                timestamp_delta: 0x0A0B_0C0D,
                message_length: 0x0000_0020,
                message_type_id: 0x09,
            }
        );
        let mut buf = [0u8; TYPE1_LEN + 4];
        mh.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, bytes);
    }

    // ── MessageHeader: Type 2 ────────────────────────────────────────────

    #[test]
    fn type2_round_trip_build_serialize_parse_no_extended() {
        let mh = MessageHeader::Type2 {
            timestamp_delta: 20,
        };
        let mut buf = [0u8; TYPE2_LEN];
        let n = mh.serialize_into(&mut buf).unwrap();
        assert_eq!(n, TYPE2_LEN);
        let (parsed, consumed) = MessageHeader::parse(Fmt::Type2, &buf).unwrap();
        assert_eq!(consumed, TYPE2_LEN);
        assert_eq!(parsed, mh);
    }

    #[test]
    fn type2_extended_timestamp_parse_serialize_byte_identical() {
        let bytes: [u8; TYPE2_LEN + 4] = [
            0xFF, 0xFF, 0xFF, // timestamp_delta sentinel
            0x11, 0x22, 0x33, 0x44, // extended timestamp delta, big-endian
        ];
        let (mh, consumed) = MessageHeader::parse(Fmt::Type2, &bytes).unwrap();
        assert_eq!(consumed, TYPE2_LEN + 4);
        assert_eq!(
            mh,
            MessageHeader::Type2 {
                timestamp_delta: 0x1122_3344,
            }
        );
        let mut buf = [0u8; TYPE2_LEN + 4];
        mh.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, bytes);
    }

    // ── MessageHeader: Type 3 ────────────────────────────────────────────

    #[test]
    fn type3_round_trip_is_zero_bytes() {
        let mh = MessageHeader::Type3;
        assert_eq!(mh.serialized_len(), 0);
        let mut buf: [u8; 0] = [];
        let n = mh.serialize_into(&mut buf).unwrap();
        assert_eq!(n, 0);
        let (parsed, consumed) = MessageHeader::parse(Fmt::Type3, &[]).unwrap();
        assert_eq!(consumed, 0);
        assert_eq!(parsed, MessageHeader::Type3);
    }

    // ── MessageHeader: errors ────────────────────────────────────────────

    #[test]
    fn type0_truncated_input_is_buffer_too_short() {
        let bytes = [0u8; TYPE0_LEN - 1];
        assert!(matches!(
            MessageHeader::parse(Fmt::Type0, &bytes),
            Err(RtmpError::BufferTooShort {
                need: TYPE0_LEN,
                ..
            })
        ));
    }

    #[test]
    fn type0_extended_marker_but_truncated_extended_field_is_buffer_too_short() {
        let mut bytes = [0u8; TYPE0_LEN + 2]; // only 2 of the 4 extended bytes.
        bytes[0] = 0xFF;
        bytes[1] = 0xFF;
        bytes[2] = 0xFF;
        assert!(matches!(
            MessageHeader::parse(Fmt::Type0, &bytes),
            Err(RtmpError::BufferTooShort {
                need,
                ..
            }) if need == TYPE0_LEN + 4
        ));
    }

    // ── Mutation-check sentinels ─────────────────────────────────────────
    // These pin exact wire-byte expectations (not just self-round-trip),
    // so a serializer that silently drops the extended-timestamp tail, or
    // mis-orders the little-endian message_stream_id, fails a test above:
    // `type0_parse_serialize_byte_identical_no_extended_and_le_stream_id`
    // hand-builds its expected bytes with the stream id reversed from host
    // order, and `type0_extended_timestamp_parse_serialize_byte_identical`
    // hand-builds a 15-byte fixture whose length alone (`TYPE0_LEN + 4`)
    // fails if the extended tail is ever omitted.

    #[test]
    fn message_stream_id_le_differs_from_be_for_asymmetric_value() {
        // Sanity check that our fixture value's LE and BE encodings differ,
        // so the byte-identical test above truly exercises endianness (a
        // palindromic value like 0x01010101 would pass either order).
        let v: u32 = 0xAABB_CCDD;
        assert_ne!(v.to_le_bytes(), v.to_be_bytes());
    }

    // ── ChunkAssembler / ChunkWriter ─────────────────────────────────────

    fn msg(csid: u32, timestamp: u32, type_id: u8, stream_id: u32, payload: Vec<u8>) -> Message {
        Message {
            chunk_stream_id: csid,
            timestamp,
            message_type_id: type_id,
            message_stream_id: stream_id,
            payload,
        }
    }

    #[test]
    fn writer_assembler_round_trip_small_message_single_chunk() {
        let original = msg(4, 1000, 9, 1, vec![0xAB; 50]);
        let mut writer = ChunkWriter::new();
        let bytes = writer.write(&original);

        let mut assembler = ChunkAssembler::new();
        let out = assembler.push(&bytes).unwrap();
        assert_eq!(out.len(), 1, "one message must come back out");
        assert_eq!(out[0], original);
    }

    #[test]
    fn writer_assembler_round_trip_message_larger_than_chunk_size() {
        // 300-byte payload at the default 128-byte chunk size => 3 chunks
        // (128 + 128 + 44): Type 0 first chunk, two Type 3 continuations.
        let original = msg(6, 5000, 9, 42, (0u8..=255).cycle().take(300).collect());
        let mut writer = ChunkWriter::new();
        let bytes = writer.write(&original);

        // Sanity: verify the byte stream really contains 3 chunks (1 basic
        // header for csid 6 is 1 byte; Type 0 header is TYPE0_LEN; then 128
        // payload bytes; then two Type 3 (1-byte basic header, 0-byte
        // message header) + payload chunks of 128 and 44).
        let expected_len = 1 + TYPE0_LEN + 128 + (1 + 128) + (1 + 44);
        assert_eq!(bytes.len(), expected_len);

        let mut assembler = ChunkAssembler::new();
        let out = assembler.push(&bytes).unwrap();
        assert_eq!(
            out.len(),
            1,
            "the 3 chunks must reassemble into ONE message"
        );
        assert_eq!(out[0], original);
        assert_eq!(out[0].payload.len(), 300);
    }

    #[test]
    fn assembler_multi_chunk_payload_reassembled_in_order() {
        // Hand-built stream: Type 0 header (csid 3, len 10, type 8, stream
        // 0, timestamp 0) with 4 payload bytes, chunk size forced to 4, then
        // two Type 3 continuations of 4 and 2 bytes — assert the payload
        // comes back concatenated in the right order, not reordered.
        let mut assembler = ChunkAssembler::new();
        assembler.set_chunk_size(4);

        let bh0 = BasicHeader {
            fmt: Fmt::Type0,
            chunk_stream_id: 3,
        };
        let mh0 = MessageHeader::Type0 {
            timestamp: 0,
            message_length: 10,
            message_type_id: 8,
            message_stream_id: 0,
        };
        let mut input = Vec::new();
        write_serialized(&mut input, &bh0);
        write_serialized(&mut input, &mh0);
        input.extend_from_slice(&[1, 2, 3, 4]);

        let bh3 = BasicHeader {
            fmt: Fmt::Type3,
            chunk_stream_id: 3,
        };
        write_serialized(&mut input, &bh3);
        input.extend_from_slice(&[5, 6, 7, 8]);
        write_serialized(&mut input, &bh3);
        input.extend_from_slice(&[9, 10]);

        let out = assembler.push(&input).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].payload, vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
    }

    #[test]
    fn assembler_header_inheritance_type0_type1_type2_type3() {
        // fmt0 (ts=1000, len=5, type=8, stream=7) -> fmt1 (delta=20) ->
        // fmt2 (delta=30) -> fmt3 (inherits fmt2's delta=30). Each chunk's
        // message completes in one go (message_length == chunk_size == 5)
        // so every header starts a fresh message on this csid.
        let mut assembler = ChunkAssembler::new();
        assembler.set_chunk_size(5);
        let csid = 5;

        let mut input = Vec::new();
        write_serialized(
            &mut input,
            &BasicHeader {
                fmt: Fmt::Type0,
                chunk_stream_id: csid,
            },
        );
        write_serialized(
            &mut input,
            &MessageHeader::Type0 {
                timestamp: 1000,
                message_length: 5,
                message_type_id: 8,
                message_stream_id: 7,
            },
        );
        input.extend_from_slice(&[0; 5]);

        write_serialized(
            &mut input,
            &BasicHeader {
                fmt: Fmt::Type1,
                chunk_stream_id: csid,
            },
        );
        write_serialized(
            &mut input,
            &MessageHeader::Type1 {
                timestamp_delta: 20,
                message_length: 5,
                message_type_id: 8,
            },
        );
        input.extend_from_slice(&[1; 5]);

        write_serialized(
            &mut input,
            &BasicHeader {
                fmt: Fmt::Type2,
                chunk_stream_id: csid,
            },
        );
        write_serialized(
            &mut input,
            &MessageHeader::Type2 {
                timestamp_delta: 30,
            },
        );
        input.extend_from_slice(&[2; 5]);

        write_serialized(
            &mut input,
            &BasicHeader {
                fmt: Fmt::Type3,
                chunk_stream_id: csid,
            },
        );
        input.extend_from_slice(&[3; 5]);

        let out = assembler.push(&input).unwrap();
        assert_eq!(out.len(), 4);

        assert_eq!(out[0].timestamp, 1000);
        assert_eq!(out[0].message_stream_id, 7);
        assert_eq!(out[0].message_type_id, 8);
        assert_eq!(out[0].payload, vec![0; 5]);

        assert_eq!(out[1].timestamp, 1020, "fmt1: 1000 + delta 20");
        assert_eq!(out[1].message_stream_id, 7, "fmt1 inherits stream id");
        assert_eq!(out[1].message_type_id, 8);
        assert_eq!(out[1].payload, vec![1; 5]);

        assert_eq!(out[2].timestamp, 1050, "fmt2: 1020 + delta 30");
        assert_eq!(out[2].message_stream_id, 7, "fmt2 inherits stream id");
        assert_eq!(out[2].message_type_id, 8, "fmt2 inherits type id");
        assert_eq!(out[2].payload, vec![2; 5], "fmt2 inherits message length");

        assert_eq!(
            out[3].timestamp, 1080,
            "fmt3 (new message) inherits fmt2's delta 30: 1050 + 30"
        );
        assert_eq!(out[3].message_stream_id, 7, "fmt3 inherits stream id");
        assert_eq!(out[3].message_type_id, 8, "fmt3 inherits type id");
        assert_eq!(out[3].payload, vec![3; 5], "fmt3 inherits message length");
    }

    #[test]
    fn assembler_mid_stream_set_chunk_size_changes_split_boundary() {
        // First message at chunk_size 128 (default): a 10-byte message on
        // csid 7 fits in one chunk. Then shrink chunk_size to 4 and send a
        // second 10-byte message on the same csid (fresh Type 0): it must
        // now arrive in 3 physical chunks (4 + 4 + 2), and pushing only the
        // first two must NOT complete the message yet.
        let mut assembler = ChunkAssembler::new();
        let csid = 7;

        let first = msg(csid, 100, 8, 1, vec![0xAA; 10]);
        let mut writer = ChunkWriter::new();
        let first_bytes = writer.write(&first);
        let out = assembler.push(&first_bytes).unwrap();
        assert_eq!(out, vec![first]);

        assembler.set_chunk_size(4);
        let bh0 = BasicHeader {
            fmt: Fmt::Type0,
            chunk_stream_id: csid,
        };
        let mh0 = MessageHeader::Type0 {
            timestamp: 200,
            message_length: 10,
            message_type_id: 8,
            message_stream_id: 1,
        };
        let mut chunk1 = Vec::new();
        write_serialized(&mut chunk1, &bh0);
        write_serialized(&mut chunk1, &mh0);
        chunk1.extend_from_slice(&[1, 2, 3, 4]);
        let out = assembler.push(&chunk1).unwrap();
        assert!(
            out.is_empty(),
            "only 4 of 10 payload bytes arrived, message must not complete yet"
        );

        let bh3 = BasicHeader {
            fmt: Fmt::Type3,
            chunk_stream_id: csid,
        };
        let mut chunk2 = Vec::new();
        write_serialized(&mut chunk2, &bh3);
        chunk2.extend_from_slice(&[5, 6, 7, 8]);
        let out = assembler.push(&chunk2).unwrap();
        assert!(out.is_empty(), "8 of 10 payload bytes, still incomplete");

        let mut chunk3 = Vec::new();
        write_serialized(&mut chunk3, &bh3);
        chunk3.extend_from_slice(&[9, 10]);
        let out = assembler.push(&chunk3).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].payload, vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
    }

    #[test]
    fn writer_assembler_round_trip_extended_timestamp_split_across_chunks() {
        // timestamp >= EXTENDED_TIMESTAMP_MARKER forces the Type 0 header
        // (and every Type 3 continuation) to carry the 4-byte Extended
        // Timestamp (§3.1.3) — payload longer than chunk_size so at least
        // one Type 3 continuation chunk exercises the "fmt3 also carries
        // extended timestamp" edge.
        let original = msg(8, EXTENDED_TIMESTAMP_MARKER + 12345, 9, 2, vec![0x7E; 300]);
        let mut writer = ChunkWriter::new();
        let bytes = writer.write(&original);

        // Sanity: the first chunk's basic header + Type0 header must be
        // TYPE0_LEN + 4 (extended) bytes, and each Type 3 continuation
        // basic header must be immediately followed by 4 extended bytes
        // before its payload slice.
        let bh_len = 1; // csid 8 fits the 1-byte basic header form.
        let first_header_len = bh_len + TYPE0_LEN + EXTENDED_TIMESTAMP_LEN;
        assert_eq!(&bytes[bh_len..bh_len + 3], [0xFF, 0xFF, 0xFF]);
        let ext_offset = bh_len + TYPE0_LEN;
        assert_eq!(
            &bytes[ext_offset..ext_offset + 4],
            &original.timestamp.to_be_bytes()
        );
        let first_payload_take = 128usize;
        let second_chunk_start = first_header_len + first_payload_take;
        // second chunk: 1-byte Type 3 basic header + 4-byte extended ts.
        assert_eq!(bytes[second_chunk_start] >> 6, Fmt::Type3.to_bits());
        let second_ext_offset = second_chunk_start + 1;
        assert_eq!(
            &bytes[second_ext_offset..second_ext_offset + 4],
            &original.timestamp.to_be_bytes(),
            "fmt3 continuation must carry the same extended timestamp"
        );

        let mut assembler = ChunkAssembler::new();
        let out = assembler.push(&bytes).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0], original);
        assert_eq!(out[0].timestamp, EXTENDED_TIMESTAMP_MARKER + 12345);
    }

    #[test]
    fn assembler_partial_feed_split_mid_header_no_drop_or_duplicate() {
        let original = msg(9, 42, 8, 3, vec![0x11; 200]);
        let mut writer = ChunkWriter::new();
        let bytes = writer.write(&original);

        // Split at an arbitrary offset that lands inside the Type 0 header
        // (byte 3 of 11), well before any payload.
        let split_at = 4;
        assert!(split_at < 1 + TYPE0_LEN);

        let mut assembler = ChunkAssembler::new();
        let out1 = assembler.push(&bytes[..split_at]).unwrap();
        assert!(out1.is_empty(), "partial header must not error or complete");
        let out2 = assembler.push(&bytes[split_at..]).unwrap();
        assert_eq!(out2.len(), 1, "message must complete exactly once");
        assert_eq!(out2[0], original);
    }

    #[test]
    fn assembler_partial_feed_split_mid_payload_no_drop_or_duplicate() {
        let original = msg(10, 42, 8, 3, vec![0x22; 300]);
        let mut writer = ChunkWriter::new();
        let bytes = writer.write(&original);

        // Split partway through the first (128-byte) payload chunk.
        let split_at = 1 + TYPE0_LEN + 60;
        let mut assembler = ChunkAssembler::new();
        let out1 = assembler.push(&bytes[..split_at]).unwrap();
        assert!(out1.is_empty());
        let out2 = assembler.push(&bytes[split_at..]).unwrap();
        assert_eq!(out2.len(), 1);
        assert_eq!(out2[0], original);
    }

    #[test]
    fn assembler_partial_feed_byte_at_a_time_never_drops_or_duplicates() {
        let original = msg(11, 7, 9, 4, vec![0x33; 260]);
        let mut writer = ChunkWriter::new();
        let bytes = writer.write(&original);

        let mut assembler = ChunkAssembler::new();
        let mut collected = Vec::new();
        for b in &bytes {
            collected.extend(assembler.push(std::slice::from_ref(b)).unwrap());
        }
        assert_eq!(collected.len(), 1);
        assert_eq!(collected[0], original);
    }

    #[test]
    fn assembler_type1_on_unseen_csid_is_malformed() {
        let mut assembler = ChunkAssembler::new();
        let mut input = Vec::new();
        write_serialized(
            &mut input,
            &BasicHeader {
                fmt: Fmt::Type1,
                chunk_stream_id: 20,
            },
        );
        write_serialized(
            &mut input,
            &MessageHeader::Type1 {
                timestamp_delta: 5,
                message_length: 3,
                message_type_id: 1,
            },
        );
        input.extend_from_slice(&[0, 0, 0]);
        assert!(matches!(
            assembler.push(&input),
            Err(RtmpError::Malformed { .. })
        ));
    }

    #[test]
    fn assembler_type3_on_unseen_csid_is_malformed() {
        let mut assembler = ChunkAssembler::new();
        let mut input = Vec::new();
        write_serialized(
            &mut input,
            &BasicHeader {
                fmt: Fmt::Type3,
                chunk_stream_id: 21,
            },
        );
        assert!(matches!(
            assembler.push(&input),
            Err(RtmpError::Malformed { .. })
        ));
    }

    #[test]
    fn assembler_truncated_input_never_panics_across_many_split_points() {
        let original = msg(12, 99, 8, 5, vec![0x44; 400]);
        let mut writer = ChunkWriter::new();
        let bytes = writer.write(&original);

        // Feed every possible byte-prefix of the stream to a fresh
        // assembler each time: none may panic, and any that do parse fully
        // must reproduce the original message exactly.
        for split in 0..=bytes.len() {
            let mut assembler = ChunkAssembler::new();
            let first = assembler.push(&bytes[..split]);
            let Ok(first_msgs) = first else {
                continue;
            };
            let second = assembler.push(&bytes[split..]).unwrap();
            let mut all = first_msgs;
            all.extend(second);
            assert_eq!(all, vec![original.clone()]);
        }
    }

    #[test]
    fn writer_default_chunk_size_matches_assembler_default() {
        assert_eq!(ChunkWriter::new().chunk_size, DEFAULT_CHUNK_SIZE);
        assert_eq!(ChunkAssembler::new().chunk_size, DEFAULT_CHUNK_SIZE);
    }

    #[test]
    fn assembler_type3_immediately_after_type0_uses_type0_timestamp_as_implied_delta() {
        // §3.1.2: a Type 3 chunk that starts a NEW message immediately after
        // a Type 0 (nothing intervening) has an implied `timestamp_delta`
        // equal to that Type 0's own absolute timestamp — NOT 0. Every other
        // existing test intervenes a Type 1/2 before the fmt3, so this is
        // the only test that reaches this resolution path directly (bites a
        // mutation of `timestamp_delta: timestamp` -> `timestamp_delta: 0`
        // in the Type 0 arm).
        let mut assembler = ChunkAssembler::new();
        let csid = 40;

        let mut input = Vec::new();
        write_serialized(
            &mut input,
            &BasicHeader {
                fmt: Fmt::Type0,
                chunk_stream_id: csid,
            },
        );
        write_serialized(
            &mut input,
            &MessageHeader::Type0 {
                timestamp: 1000,
                message_length: 5,
                message_type_id: 8,
                message_stream_id: 2,
            },
        );
        input.extend_from_slice(&[1, 2, 3, 4, 5]);

        // Immediately (same csid, nothing between) a fmt3 chunk starting a
        // new message, inheriting message_length 5 from the Type 0 above.
        write_serialized(
            &mut input,
            &BasicHeader {
                fmt: Fmt::Type3,
                chunk_stream_id: csid,
            },
        );
        input.extend_from_slice(&[9, 9, 9, 9, 9]);

        let out = assembler.push(&input).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].timestamp, 1000);
        assert_eq!(
            out[1].timestamp, 2000,
            "fmt3 immediately after fmt0 implies delta == the fmt0's own timestamp (1000), not 0: 1000 + 1000"
        );
    }

    #[test]
    fn assembler_type2_on_unseen_csid_is_malformed() {
        let mut assembler = ChunkAssembler::new();
        let mut input = Vec::new();
        write_serialized(
            &mut input,
            &BasicHeader {
                fmt: Fmt::Type2,
                chunk_stream_id: 22,
            },
        );
        write_serialized(&mut input, &MessageHeader::Type2 { timestamp_delta: 5 });
        assert!(matches!(
            assembler.push(&input),
            Err(RtmpError::Malformed { .. })
        ));
    }

    #[test]
    fn assembler_set_chunk_size_zero_is_floored_to_one() {
        let mut assembler = ChunkAssembler::new();
        assembler.set_chunk_size(0);
        assert_eq!(assembler.chunk_size, 1, "floored at 1, not stuck at 0");

        // A message chunked consistently with the floored size (1 payload
        // byte per physical chunk) still assembles correctly — the floor
        // makes progress possible rather than wedging on every push.
        let csid = 41;
        let mut input = Vec::new();
        write_serialized(
            &mut input,
            &BasicHeader {
                fmt: Fmt::Type0,
                chunk_stream_id: csid,
            },
        );
        write_serialized(
            &mut input,
            &MessageHeader::Type0 {
                timestamp: 1,
                message_length: 3,
                message_type_id: 8,
                message_stream_id: 0,
            },
        );
        input.push(0xAA);
        write_serialized(
            &mut input,
            &BasicHeader {
                fmt: Fmt::Type3,
                chunk_stream_id: csid,
            },
        );
        input.push(0xBB);
        write_serialized(
            &mut input,
            &BasicHeader {
                fmt: Fmt::Type3,
                chunk_stream_id: csid,
            },
        );
        input.push(0xCC);

        let out = assembler.push(&input).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].payload, vec![0xAA, 0xBB, 0xCC]);
    }

    // ── Remote-DoS caps (excessive-allocation guard) ────────────────────

    /// A complete, well-formed one-chunk message on `csid`: a Type 0 header
    /// declaring `message_length == payload.len()`, immediately followed by
    /// `payload` (so it fits in the default 128-byte chunk size and
    /// completes in a single chunk).
    fn single_chunk(csid: u32, payload: &[u8]) -> Vec<u8> {
        let mut input = Vec::new();
        write_serialized(
            &mut input,
            &BasicHeader {
                fmt: Fmt::Type0,
                chunk_stream_id: csid,
            },
        );
        write_serialized(
            &mut input,
            &MessageHeader::Type0 {
                timestamp: 0,
                message_length: payload.len() as u32,
                message_type_id: 9,
                message_stream_id: 1,
            },
        );
        input.extend_from_slice(payload);
        input
    }

    #[test]
    fn oversized_message_length_header_is_rejected_without_allocating() {
        // Mutation check: a Type 0 header claims a ~16 MiB message_length
        // (the max a 24-bit field can encode) but only ever supplies a
        // single default-chunk-size (128-byte) slice of payload after it —
        // exactly the shape of the excessive-allocation DoS (attacker never
        // has to send anywhere near the claimed length). Without the
        // MAX_MESSAGE_LEN cap this used to `Vec::with_capacity(message_length)`
        // (~16 MiB) right here and return `Ok(vec![])` (message merely
        // in-progress, no error) — this test would then fail, since it
        // asserts an `Err` instead.
        let mut assembler = ChunkAssembler::new();
        let mut input = Vec::new();
        write_serialized(
            &mut input,
            &BasicHeader {
                fmt: Fmt::Type0,
                chunk_stream_id: 4,
            },
        );
        write_serialized(
            &mut input,
            &MessageHeader::Type0 {
                timestamp: 0,
                message_length: 0x00FF_FFFF, // ~16 MiB: the largest 24-bit value.
                message_type_id: 9,
                message_stream_id: 1,
            },
        );
        input.extend(std::iter::repeat_n(0u8, DEFAULT_CHUNK_SIZE as usize));

        let err = assembler.push(&input).expect_err(
            "a message_length beyond MAX_MESSAGE_LEN must be rejected before any \
             message_length-sized buffer is allocated",
        );
        assert!(matches!(err, RtmpError::Malformed { .. }));
    }

    #[test]
    fn message_length_at_the_cap_is_accepted() {
        // Boundary check: exactly MAX_MESSAGE_LEN must still be accepted
        // (only values strictly above the cap are rejected).
        let mut assembler = ChunkAssembler::new();
        let mut input = Vec::new();
        write_serialized(
            &mut input,
            &BasicHeader {
                fmt: Fmt::Type0,
                chunk_stream_id: 4,
            },
        );
        write_serialized(
            &mut input,
            &MessageHeader::Type0 {
                timestamp: 0,
                message_length: MAX_MESSAGE_LEN,
                message_type_id: 9,
                message_stream_id: 1,
            },
        );
        input.extend(std::iter::repeat_n(0u8, DEFAULT_CHUNK_SIZE as usize));

        // Not yet complete (only one chunk of a much larger message has
        // arrived) but must not be rejected outright.
        assert!(assembler.push(&input).is_ok());
    }

    #[test]
    fn csid_flood_beyond_max_csids_is_rejected() {
        // Mutation check: fill the bound with MAX_CSIDS distinct,
        // well-formed chunk streams first (none of these may error — the
        // cap must not reject legitimate, moderate csid usage), then assert
        // that one more previously-unseen csid is rejected rather than
        // silently growing the per-csid state map without bound. Without
        // the MAX_CSIDS cap this last `push` would also return `Ok(_)`,
        // failing this test's `Err` assertion.
        let mut assembler = ChunkAssembler::new();
        for i in 0..MAX_CSIDS {
            let csid = BASIC_HEADER_1BYTE_MIN_CSID + i as u32;
            let out = assembler
                .push(&single_chunk(csid, &[0xAB]))
                .unwrap_or_else(|e| panic!("csid {csid} (#{i}, within the bound) rejected: {e}"));
            assert_eq!(out.len(), 1);
        }

        let flood_csid = BASIC_HEADER_1BYTE_MIN_CSID + MAX_CSIDS as u32;
        let err = assembler
            .push(&single_chunk(flood_csid, &[0xCD]))
            .expect_err("a new csid beyond MAX_CSIDS must be rejected, not silently accepted");
        assert!(matches!(err, RtmpError::Malformed { .. }));
    }

    #[test]
    fn csid_flood_cap_does_not_count_repeats_of_the_same_csid() {
        // A single csid reused for many messages must never itself trip the
        // MAX_CSIDS cap (the bound is on distinct concurrent csids, not on
        // total message count).
        let mut assembler = ChunkAssembler::new();
        for i in 0..(MAX_CSIDS * 4) {
            let out = assembler
                .push(&single_chunk(BASIC_HEADER_1BYTE_MIN_CSID, &[i as u8]))
                .expect("repeated use of a single already-known csid must never be rejected");
            assert_eq!(out.len(), 1);
        }
    }

    #[test]
    fn set_chunk_size_is_capped_at_max_chunk_size() {
        let mut assembler = ChunkAssembler::new();
        assembler.set_chunk_size(u32::MAX);
        assert_eq!(assembler.chunk_size, MAX_CHUNK_SIZE);

        let mut writer = ChunkWriter::new();
        writer.set_chunk_size(u32::MAX);
        assert_eq!(writer.chunk_size, MAX_CHUNK_SIZE);
    }
}
