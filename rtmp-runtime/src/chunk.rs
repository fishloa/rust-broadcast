//! RTMP chunk stream — basic header, message header, extended timestamp
//! (Adobe RTMP 1.0 §5.3).
//!
//! See [`docs/rtmp.md`](../docs/rtmp.md) §3 (RTMP Chunk Stream) for the wire
//! layout: chunk format (§5.3.1), basic header (§5.3.1.1), the four message
//! header `fmt` variants (§5.3.1.2), and extended timestamp (§5.3.1.3).
//!
//! This module implements the chunk **header** wire types only — reassembling
//! a stream of chunks into whole messages (tracking prior-chunk state per
//! csid so `fmt` 1/2/3 headers can inherit the fields they omit) is a
//! stateful job that lands in a later task (#738 Task 4).

use broadcast_common::{Parse, Serialize};

use crate::RtmpError;

type Result<T> = core::result::Result<T, RtmpError>;

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
}
