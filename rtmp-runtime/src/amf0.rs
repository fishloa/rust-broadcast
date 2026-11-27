//! AMF0 (Action Message Format 0) value encoding/decoding, used by RTMP
//! command and data messages.
//!
//! See `private/specs-verified/rtmp/rtmp-runtime/rtmp.md` §8 (AMF0) for the full transcription
//! of `[AMF0]` (the companion spec this doc cites): §8.1 for the type marker
//! table, §8.2 for the wire encoding of each data type needed by RTMP command
//! messages.
//!
//! RTMP does not wrap command/data message bodies in AMF0's own
//! `amf-packet` framing — a message body is simply a sequence of AMF0
//! `value-type`s (marker + body) concatenated one after another (§8.2). This
//! module implements that sequence-of-values contract via [`Amf0Value`]
//! (single value parse/serialize) and [`Command`] (the `name` +
//! `transaction_id` + `arguments` sequence carried by Command Messages,
//! §7.1.1).
//!
//! # Scope
//!
//! Implements the markers actually needed to decode/encode the ingest
//! command set (`connect`/`createStream`/`publish`/`_result`/`onStatus`/…):
//! Number, Boolean, String, Object, Null, Undefined, ECMA Array, Strict
//! Array, plus Date and Long String (trivial once String/Number exist).
//!
//! Out of scope, and rejected as [`RtmpError::UnsupportedAmf0Marker`] rather than
//! panicking or silently misparsing: `movieclip-marker` (`0x04`, reserved),
//! `reference-marker` (`0x07`), `unsupported-marker` (`0x0D`),
//! `recordset-marker` (`0x0E`, reserved), `xml-document-marker` (`0x0F`),
//! `typed-object-marker` (`0x10`), and `avmplus-object-marker` (`0x11`, the
//! AMF3 switch — AMF3 itself, `[AMF3]`, is a separate spec and explicitly
//! out of scope for this ingest engine per private/specs-verified/rtmp/rtmp-runtime/rtmp.md §8.3).
//!
//! [`Amf0Value`] is a data-carrying ADT (like `Fmt`/`MessageHeader`'s payload
//! variants), not a closed label enum, so it is a `#204` `label_coverage`
//! SKIP-list candidate rather than a `name()`/`impl_spec_display!` target
//! (tracked for the crate's Task 10 label-coverage pass).

use broadcast_core::error::BufferTooShort;
use broadcast_core::{Parse, Serialize};

use crate::RtmpError;
use alloc::string::String;
use alloc::vec::Vec;

type Result<T> = core::result::Result<T, RtmpError>;

/// AMF0 type markers (`[AMF0]` §2.1, private/specs-verified/rtmp/rtmp-runtime/rtmp.md §8.1) — 1 byte.
pub mod marker {
    /// Number (§2.2): `DOUBLE`, 8-byte big-endian IEEE-754.
    pub const NUMBER: u8 = 0x00;
    /// Boolean (§2.3): 1 byte, `0` = false, nonzero = true.
    pub const BOOLEAN: u8 = 0x01;
    /// String (§2.4): `U16` length + UTF-8 bytes.
    pub const STRING: u8 = 0x02;
    /// Object (§2.5): key/value pairs terminated by [`OBJECT_END`].
    pub const OBJECT: u8 = 0x03;
    /// null (§2.7): no payload.
    pub const NULL: u8 = 0x05;
    /// undefined (§2.8): no payload.
    pub const UNDEFINED: u8 = 0x06;
    /// ECMA Array (§2.10): `U32` associative-count + key/value pairs
    /// terminated by [`OBJECT_END`].
    pub const ECMA_ARRAY: u8 = 0x08;
    /// Object End (§2.11): always preceded by an empty (`U16` = 0) key — the
    /// 3-byte sequence `00 00 09`.
    pub const OBJECT_END: u8 = 0x09;
    /// Strict Array (§2.12): `U32` count + that many values, ordinal only.
    pub const STRICT_ARRAY: u8 = 0x0A;
    /// Date (§2.13): `DOUBLE` (ms since Unix epoch, UTC) + reserved `S16`
    /// time zone (MUST be `0x0000`).
    pub const DATE: u8 = 0x0B;
    /// Long String (§2.14): `U32` length + UTF-8 bytes, for strings over
    /// 65535 bytes.
    pub const LONG_STRING: u8 = 0x0C;
}

/// Maximum AMF0 container nesting depth (Object/ECMA Array/Strict Array)
/// [`Amf0Value::parse`] will descend into. Bounds recursion so a
/// pathologically nested input returns [`RtmpError::Amf0DepthExceeded`] instead of
/// overflowing the native call stack — the guard is checked on *entering*
/// each nested container, so recursion never actually reaches an
/// attacker-chosen depth, only this constant.
pub const MAX_AMF0_DEPTH: usize = 32;

/// Maximum number of AMF0 values one byte sequence may decode into, counting
/// every value read at any nesting level (containers included). Bounds
/// *width* the way [`MAX_AMF0_DEPTH`] bounds depth: millions of one-byte
/// nulls inside a Strict Array, or three-byte empty-key pairs inside an
/// Object, otherwise expand into millions of `Amf0Value`s (plus a `String`
/// per key) from a single message. A decode past the limit returns
/// [`RtmpError::Amf0TooManyValues`].
pub const MAX_AMF0_VALUES: usize = 4096;

/// Count one more decoded value against `values`, rejecting the sequence once
/// it passes [`MAX_AMF0_VALUES`].
fn count_value(values: &mut usize) -> Result<()> {
    *values = values.checked_add(1).ok_or(RtmpError::Amf0TooManyValues {
        max: MAX_AMF0_VALUES,
    })?;
    if *values > MAX_AMF0_VALUES {
        return Err(RtmpError::Amf0TooManyValues {
            max: MAX_AMF0_VALUES,
        });
    }
    Ok(())
}

const MARKER_LEN: usize = 1;
const NUMBER_LEN: usize = 8;
const BOOLEAN_LEN: usize = 1;
const U16_LEN: usize = 2;
const U32_LEN: usize = core::mem::size_of::<u32>();
const DATE_RESERVED_LEN: usize = 2;
/// `00 00 09`: empty key + [`marker::OBJECT_END`].
const OBJECT_END_LEN: usize = 3;
/// Offset of the [`marker::OBJECT_END`] byte within that three-byte sequence.
const OBJECT_END_MARKER_OFFSET: usize = 2;

/// A single AMF0 value (`[AMF0]` §2, private/specs-verified/rtmp/rtmp-runtime/rtmp.md §8.2).
///
/// AMF3 (`avmplus-object-marker`, `0x11`) and the reserved/legacy markers
/// (`movieclip`, `reference`, `unsupported`, `recordset`, `xml-document`,
/// `typed-object`) are out of scope — see the module doc.
///
/// `#[non_exhaustive]`: this models a documented subset of `[AMF0]` §2's
/// value types (see the module doc's Scope section); a future release may
/// add a variant for one of the currently-`Unsupported` markers (e.g. a
/// typed AMF3 bridge) without that being a breaking change for existing
/// `match` callers.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Amf0Value {
    /// Number (§2.2).
    Number(f64),
    /// Boolean (§2.3).
    Boolean(bool),
    /// String (§2.4): `U16`-length UTF-8, at most 65535 bytes.
    String(String),
    /// Object (§2.5): ordered key/value pairs.
    Object(Vec<(String, Amf0Value)>),
    /// null (§2.7).
    Null,
    /// undefined (§2.8).
    Undefined,
    /// ECMA Array (§2.10): an associative array, encoded like Object plus a
    /// leading (informational) `U32` count.
    EcmaArray(Vec<(String, Amf0Value)>),
    /// Strict Array (§2.12): an ordinal array of values.
    StrictArray(Vec<Amf0Value>),
    /// Date (§2.13): milliseconds since the Unix epoch, UTC.
    Date(f64),
    /// Long String (§2.14): `U32`-length UTF-8, for strings over 65535
    /// bytes.
    LongString(String),
}

fn buffer_too_short(need: usize, have: usize, what: &'static str) -> RtmpError {
    RtmpError::BufferTooShort(BufferTooShort { need, have, what })
}

/// Read a `U16`-length-prefixed UTF-8 string from the front of `bytes`.
/// Returns the decoded string and the total bytes consumed (`2 + len`).
/// Used for both the String value body and Object/ECMA-Array keys, which
/// share this exact encoding (§2.4 / §2.5).
fn read_utf8_short(bytes: &[u8]) -> Result<(String, usize)> {
    if bytes.len() < U16_LEN {
        return Err(buffer_too_short(U16_LEN, bytes.len(), "AMF0 string length"));
    }
    let len = u16::from_be_bytes([bytes[0], bytes[1]]) as usize;
    // `len` is at most `u16::MAX`, so `U16_LEN + len` cannot actually
    // overflow `usize` on any real target, but guard it anyway (rather than
    // a bare `+`) so this stays correct even on a hypothetical narrow
    // `usize` platform, and matches the same guard on `read_utf8_long`
    // below where the addend genuinely can overflow.
    let total = U16_LEN.checked_add(len).ok_or(RtmpError::FieldOverflow(
        broadcast_core::len::FieldOverflow {
            field: "AMF0 string length",
            value: u64::from(u16::MAX),
            max: u64::try_from(usize::MAX).unwrap_or(u64::MAX),
        },
    ))?;
    if bytes.len() < total {
        return Err(buffer_too_short(total, bytes.len(), "AMF0 string"));
    }
    let s = String::from_utf8(bytes[U16_LEN..total].to_vec())
        .map_err(|_| RtmpError::Amf0Utf8 { what: "string" })?;
    Ok((s, total))
}

/// Read a `U32`-length-prefixed UTF-8 string (Long String, §2.14).
fn read_utf8_long(bytes: &[u8]) -> Result<(String, usize)> {
    if bytes.len() < U32_LEN {
        return Err(buffer_too_short(
            U32_LEN,
            bytes.len(),
            "AMF0 long string length",
        ));
    }
    let len = crate::be_u32(bytes, 0) as usize;
    // `len` can be up to `u32::MAX`: on a 32-bit target, `U32_LEN + len`
    // wraps `usize` (e.g. `len == usize::MAX - 1`), which would make
    // `bytes.len() < total` compare against a garbage-small wrapped value
    // and misparse a length claim that should instead be rejected as too
    // short. `checked_add` catches that instead of wrapping.
    let total = U32_LEN.checked_add(len).ok_or(RtmpError::FieldOverflow(
        broadcast_core::len::FieldOverflow {
            field: "AMF0 long string length",
            value: u64::from(u32::MAX),
            max: u64::try_from(usize::MAX).unwrap_or(u64::MAX),
        },
    ))?;
    if bytes.len() < total {
        return Err(buffer_too_short(total, bytes.len(), "AMF0 long string"));
    }
    let s = String::from_utf8(bytes[U32_LEN..total].to_vec()).map_err(|_| RtmpError::Amf0Utf8 {
        what: "long string",
    })?;
    Ok((s, total))
}

/// Parse the key/value pairs of an Object or ECMA Array body (§2.5/§2.10),
/// starting right after any leading count field. Terminates on an empty key
/// followed by [`marker::OBJECT_END`] (§2.11). `depth` is the nesting depth
/// of the *values* in this container (already incremented past the
/// container itself by the caller). `values` counts every value decoded so
/// far, shared across the whole top-level sequence.
fn parse_pairs(
    bytes: &[u8],
    depth: usize,
    values: &mut usize,
) -> Result<(Vec<(String, Amf0Value)>, usize)> {
    let mut consumed = 0;
    let mut pairs = Vec::new();
    loop {
        let (key, key_len) = read_utf8_short(&bytes[consumed..])?;
        let after_key = consumed + key_len;
        if key.is_empty() {
            if bytes.len() < after_key + MARKER_LEN {
                return Err(buffer_too_short(
                    after_key + MARKER_LEN,
                    bytes.len(),
                    "amf0 object-end marker",
                ));
            }
            if bytes[after_key] == marker::OBJECT_END {
                return Ok((pairs, after_key + MARKER_LEN));
            }
        }
        let (value, value_len) = parse_value(&bytes[after_key..], depth, values)?;
        pairs.push((key, value));
        consumed = after_key
            .checked_add(value_len)
            .ok_or(RtmpError::FieldOverflow(
                broadcast_core::len::FieldOverflow {
                    field: "AMF0 object pair length",
                    value: u64::from(u32::try_from(after_key).unwrap_or(u32::MAX))
                        + u64::from(u32::try_from(value_len).unwrap_or(u32::MAX)),
                    max: u64::try_from(usize::MAX).unwrap_or(u64::MAX),
                },
            ))?;
    }
}

/// Parse one AMF0 value (marker + body) from the front of `bytes`, ignoring
/// any surplus trailing bytes, and return it with the number of bytes its
/// encoding occupied. That count comes from the parser itself, so a caller
/// walking a sequence of values advances by what was actually read rather
/// than by a re-serialized length, which only agrees while every encoding is
/// canonical. `depth` counts container nesting already
/// entered (0 at the top level); checked *before* descending into a nested
/// Object/ECMA-Array/Strict-Array so recursion is bounded by
/// [`MAX_AMF0_DEPTH`] regardless of how deeply the input is (adversarially)
/// nested. `values` counts every value decoded across the whole top-level
/// sequence, bounded by [`MAX_AMF0_VALUES`].
fn parse_value(bytes: &[u8], depth: usize, values: &mut usize) -> Result<(Amf0Value, usize)> {
    if bytes.is_empty() {
        return Err(buffer_too_short(MARKER_LEN, 0, "amf0 value marker"));
    }
    count_value(values)?;
    let body = &bytes[MARKER_LEN..];
    match bytes[0] {
        marker::NUMBER => {
            if body.len() < NUMBER_LEN {
                return Err(buffer_too_short(NUMBER_LEN, body.len(), "amf0 number"));
            }
            let mut b = [0u8; NUMBER_LEN];
            b.copy_from_slice(&body[..NUMBER_LEN]);
            Ok((
                Amf0Value::Number(f64::from_be_bytes(b)),
                MARKER_LEN + NUMBER_LEN,
            ))
        }
        marker::BOOLEAN => {
            if body.is_empty() {
                return Err(buffer_too_short(BOOLEAN_LEN, 0, "amf0 boolean"));
            }
            Ok((Amf0Value::Boolean(body[0] != 0), MARKER_LEN + BOOLEAN_LEN))
        }
        marker::STRING => {
            let (s, len) = read_utf8_short(body)?;
            Ok((Amf0Value::String(s), MARKER_LEN + len))
        }
        marker::OBJECT => {
            if depth >= MAX_AMF0_DEPTH {
                return Err(RtmpError::Amf0DepthExceeded {
                    max: MAX_AMF0_DEPTH,
                });
            }
            let (pairs, len) = parse_pairs(body, depth + 1, values)?;
            Ok((Amf0Value::Object(pairs), MARKER_LEN + len))
        }
        marker::NULL => Ok((Amf0Value::Null, MARKER_LEN)),
        marker::UNDEFINED => Ok((Amf0Value::Undefined, MARKER_LEN)),
        marker::ECMA_ARRAY => {
            if depth >= MAX_AMF0_DEPTH {
                return Err(RtmpError::Amf0DepthExceeded {
                    max: MAX_AMF0_DEPTH,
                });
            }
            if body.len() < U32_LEN {
                return Err(buffer_too_short(
                    U32_LEN,
                    body.len(),
                    "amf0 ecma array count",
                ));
            }
            // The associative-count is informational only (§2.10); the
            // object-end terminator is authoritative, so it is read and
            // discarded rather than cross-checked against the parsed pair
            // count.
            let (pairs, len) = parse_pairs(&body[U32_LEN..], depth + 1, values)?;
            Ok((Amf0Value::EcmaArray(pairs), MARKER_LEN + U32_LEN + len))
        }
        marker::STRICT_ARRAY => {
            if depth >= MAX_AMF0_DEPTH {
                return Err(RtmpError::Amf0DepthExceeded {
                    max: MAX_AMF0_DEPTH,
                });
            }
            if body.len() < U32_LEN {
                return Err(buffer_too_short(
                    U32_LEN,
                    body.len(),
                    "amf0 strict array count",
                ));
            }
            let count = crate::be_u32(body, 0);
            let mut rest = &body[U32_LEN..];
            let mut elements = Vec::new();
            for _ in 0..count {
                let (value, consumed) = parse_value(rest, depth + 1, values)?;
                elements.push(value);
                rest = rest.get(consumed..).ok_or(RtmpError::FieldOverflow(
                    broadcast_core::len::FieldOverflow {
                        field: "AMF0 strict array element length",
                        value: u64::try_from(consumed).unwrap_or(u64::MAX),
                        max: u64::try_from(rest.len()).unwrap_or(u64::MAX),
                    },
                ))?;
            }
            let elements_len =
                body[U32_LEN..]
                    .len()
                    .checked_sub(rest.len())
                    .ok_or(RtmpError::Malformed {
                        what: "AMF0 strict array body",
                    })?;
            Ok((
                Amf0Value::StrictArray(elements),
                MARKER_LEN + U32_LEN + elements_len,
            ))
        }
        marker::DATE => {
            if body.len() < NUMBER_LEN + DATE_RESERVED_LEN {
                return Err(buffer_too_short(
                    NUMBER_LEN + DATE_RESERVED_LEN,
                    body.len(),
                    "amf0 date",
                ));
            }
            let mut b = [0u8; NUMBER_LEN];
            b.copy_from_slice(&body[..NUMBER_LEN]);
            let tz = u16::from_be_bytes([body[NUMBER_LEN], body[NUMBER_LEN + 1]]);
            if tz != 0 {
                return Err(RtmpError::Amf0DateTimeZone { value: tz });
            }
            Ok((
                Amf0Value::Date(f64::from_be_bytes(b)),
                MARKER_LEN + NUMBER_LEN + DATE_RESERVED_LEN,
            ))
        }
        marker::LONG_STRING => {
            let (s, len) = read_utf8_long(body)?;
            Ok((Amf0Value::LongString(s), MARKER_LEN + len))
        }
        other => Err(RtmpError::UnsupportedAmf0Marker { marker: other }),
    }
}

/// The `FieldOverflow` for a value sequence whose total length does not fit a
/// `usize`: the sum of real element lengths against the addressable maximum.
fn sequence_length_overflow(so_far: usize, next: usize) -> RtmpError {
    RtmpError::FieldOverflow(broadcast_core::len::FieldOverflow {
        field: "amf0 value sequence length",
        value: u64::from(u32::try_from(so_far).unwrap_or(u32::MAX))
            + u64::from(u32::try_from(next).unwrap_or(u32::MAX)),
        max: u64::try_from(usize::MAX).unwrap_or(u64::MAX),
    })
}

fn pairs_body_len(pairs: &[(String, Amf0Value)]) -> usize {
    pairs
        .iter()
        .map(|(k, v)| U16_LEN + k.len() + v.serialized_len())
        .sum::<usize>()
        + OBJECT_END_LEN
}

fn write_pairs(pairs: &[(String, Amf0Value)], buf: &mut [u8]) -> Result<usize> {
    let mut offset = 0;
    for (k, v) in pairs {
        let key_total = U16_LEN + k.len();
        if buf.len() < offset + key_total {
            return Err(buffer_too_short(
                offset + key_total,
                buf.len(),
                "amf0 object key output",
            ));
        }
        let key_len = broadcast_core::len::fit_u16(k.len(), "amf0 object key length")?;
        buf[offset..offset + U16_LEN].copy_from_slice(&key_len.to_be_bytes());
        buf[offset + U16_LEN..offset + key_total].copy_from_slice(k.as_bytes());
        offset += key_total;
        offset += v.serialize_into(&mut buf[offset..])?;
    }
    if buf.len() < offset + OBJECT_END_LEN {
        return Err(buffer_too_short(
            offset + OBJECT_END_LEN,
            buf.len(),
            "amf0 object-end output",
        ));
    }
    buf[offset] = 0;
    buf[offset + 1] = 0;
    buf[offset + OBJECT_END_MARKER_OFFSET] = marker::OBJECT_END;
    Ok(offset + OBJECT_END_LEN)
}

impl<'a> Parse<'a> for Amf0Value {
    type Error = RtmpError;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let mut values = 0usize;
        let (value, _) = parse_value(bytes, 0, &mut values)?;
        Ok(value)
    }
}

impl Serialize for Amf0Value {
    type Error = RtmpError;

    fn serialized_len(&self) -> usize {
        MARKER_LEN
            + match self {
                Amf0Value::Number(_) | Amf0Value::Date(_) => NUMBER_LEN,
                Amf0Value::Boolean(_) => BOOLEAN_LEN,
                Amf0Value::String(s) => U16_LEN + s.len(),
                Amf0Value::LongString(s) => U32_LEN + s.len(),
                Amf0Value::Object(pairs) => pairs_body_len(pairs),
                Amf0Value::Null | Amf0Value::Undefined => 0,
                Amf0Value::EcmaArray(pairs) => U32_LEN + pairs_body_len(pairs),
                Amf0Value::StrictArray(values) => {
                    U32_LEN + values.iter().map(Serialize::serialized_len).sum::<usize>()
                }
            }
            + match self {
                Amf0Value::Date(_) => DATE_RESERVED_LEN,
                _ => 0,
            }
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let written = self.serialized_len();
        if buf.len() < written {
            return Err(buffer_too_short(written, buf.len(), "amf0 value output"));
        }
        let (marker_byte, body) = buf[..written].split_at_mut(MARKER_LEN);
        match self {
            Amf0Value::Number(v) => {
                marker_byte[0] = marker::NUMBER;
                body[..NUMBER_LEN].copy_from_slice(&v.to_be_bytes());
            }
            Amf0Value::Boolean(v) => {
                marker_byte[0] = marker::BOOLEAN;
                body[0] = u8::from(*v);
            }
            Amf0Value::String(s) => {
                if s.len() > usize::from(u16::MAX) {
                    return Err(RtmpError::Amf0StringTooLong { len: s.len() });
                }
                marker_byte[0] = marker::STRING;
                body[..U16_LEN].copy_from_slice(&(s.len() as u16).to_be_bytes());
                body[U16_LEN..].copy_from_slice(s.as_bytes());
            }
            Amf0Value::LongString(s) => {
                marker_byte[0] = marker::LONG_STRING;
                let len = broadcast_core::len::fit_u32(s.len(), "amf0 long string length")?;
                body[..U32_LEN].copy_from_slice(&len.to_be_bytes());
                body[U32_LEN..].copy_from_slice(s.as_bytes());
            }
            Amf0Value::Object(pairs) => {
                marker_byte[0] = marker::OBJECT;
                write_pairs(pairs, body)?;
            }
            Amf0Value::Null => marker_byte[0] = marker::NULL,
            Amf0Value::Undefined => marker_byte[0] = marker::UNDEFINED,
            Amf0Value::EcmaArray(pairs) => {
                marker_byte[0] = marker::ECMA_ARRAY;
                let count = broadcast_core::len::fit_u32(pairs.len(), "amf0 ECMA array count")?;
                body[..U32_LEN].copy_from_slice(&count.to_be_bytes());
                write_pairs(pairs, &mut body[U32_LEN..])?;
            }
            Amf0Value::StrictArray(values) => {
                marker_byte[0] = marker::STRICT_ARRAY;
                let count = broadcast_core::len::fit_u32(values.len(), "amf0 strict array count")?;
                body[..U32_LEN].copy_from_slice(&count.to_be_bytes());
                let mut offset = U32_LEN;
                for v in values {
                    offset += v.serialize_into(&mut body[offset..])?;
                }
            }
            Amf0Value::Date(v) => {
                marker_byte[0] = marker::DATE;
                body[..NUMBER_LEN].copy_from_slice(&v.to_be_bytes());
                body[NUMBER_LEN..NUMBER_LEN + DATE_RESERVED_LEN].copy_from_slice(&[0, 0]);
            }
        }
        Ok(written)
    }
}

/// An RTMP Command Message body (§7.1.1): `name` (AMF0 String) + AMF0
/// `transaction_id` (AMF0 Number) + zero or more argument values (typically
/// a Command Object, then optional trailing arguments) — see private/specs-verified/rtmp/rtmp-runtime/rtmp.md
/// §8.2's closing note and §6.1 (Command Message).
///
/// This is the message-body-level container: unlike [`Amf0Value`] it is not
/// itself a single AMF0 `value-type`, so it uses inherent `parse`/`to_body`
/// methods rather than the [`Parse`]/[`Serialize`] traits.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Command {
    /// The command name, e.g. `"connect"`, `"createStream"`, `"publish"`,
    /// `"_result"`.
    pub name: String,
    /// The transaction id correlating a response to its request (`0` for
    /// commands that expect no response, e.g. `onStatus`).
    pub transaction_id: f64,
    /// The remaining AMF0 values in the command body, in wire order
    /// (conventionally: Command Object, then any further arguments).
    pub arguments: Vec<Amf0Value>,
}

impl Command {
    /// Parse a Command Message body: AMF0 String (`name`) + AMF0 Number
    /// (`transaction_id`) + the remaining AMF0 values (`arguments`), read
    /// until `payload` is exhausted.
    ///
    /// # Errors
    /// [`RtmpError::CommandNameNotString`] if the first value is not a
    /// String, [`RtmpError::CommandTransactionIdNotNumber`] if the second is
    /// not a Number; the remaining variants are propagated from
    /// [`Amf0Value::parse`].
    pub fn parse(payload: &[u8]) -> Result<Self> {
        let mut values = 0usize;
        let (name_value, name_len) = parse_value(payload, 0, &mut values)?;
        let mut offset = name_len;
        let name = match name_value {
            Amf0Value::String(s) => s,
            value => return Err(RtmpError::CommandNameNotString { value }),
        };

        let (txn_value, txn_len) =
            parse_value(payload.get(offset..).unwrap_or_default(), 0, &mut values)?;
        offset = offset.checked_add(txn_len).ok_or(RtmpError::FieldOverflow(
            broadcast_core::len::FieldOverflow {
                field: "RTMP command transaction id offset",
                value: u64::try_from(offset).unwrap_or(u64::MAX),
                max: u64::try_from(usize::MAX).unwrap_or(u64::MAX),
            },
        ))?;
        let transaction_id = match txn_value {
            Amf0Value::Number(n) => n,
            value => return Err(RtmpError::CommandTransactionIdNotNumber { value }),
        };

        let mut arguments = Vec::new();
        while offset < payload.len() {
            let (value, len) = parse_value(&payload[offset..], 0, &mut values)?;
            offset = offset.checked_add(len).ok_or(RtmpError::FieldOverflow(
                broadcast_core::len::FieldOverflow {
                    field: "RTMP command argument offset",
                    value: u64::try_from(offset).unwrap_or(u64::MAX),
                    max: u64::try_from(usize::MAX).unwrap_or(u64::MAX),
                },
            ))?;
            arguments.push(value);
        }

        Ok(Command {
            name,
            transaction_id,
            arguments,
        })
    }

    /// Serialize this command back to an AMF0 command payload: `name` +
    /// `transaction_id` + `arguments`, in that order.
    ///
    /// # Errors
    ///
    /// [`RtmpError::FieldOverflow`] when any string, key or array exceeds the
    /// AMF0 16/32-bit length prefix (a peer-chosen publish name echoed into an
    /// `onStatus` description can reach this).
    pub fn to_body(&self) -> Result<Vec<u8>> {
        let mut out = Amf0Value::String(self.name.clone()).try_to_bytes()?;
        out.extend(Amf0Value::Number(self.transaction_id).try_to_bytes()?);
        for arg in &self.arguments {
            out.extend(arg.try_to_bytes()?);
        }
        Ok(out)
    }
}
/// Parse a sequence of AMF0 values that fill `body`, the inverse of
/// [`amf0_values`].
///
/// A script-data body is such a sequence: the handler name, then one or more
/// values (Adobe FLV v10.1 §E.4.1).
///
/// # Errors
///
/// [`RtmpError::BufferTooShort`] when a value runs past `body`, and the
/// object/array limits [`Amf0Value::parse`] applies.
pub fn amf0_parse_values(body: &[u8]) -> Result<Vec<Amf0Value>> {
    let mut values_out = Vec::new();
    let mut offset = 0usize;
    let mut count = 0usize;
    while offset < body.len() {
        let (value, used) = parse_value(&body[offset..], 0, &mut count)?;
        values_out.push(value);
        offset = offset.checked_add(used).ok_or(RtmpError::FieldOverflow(
            broadcast_core::len::FieldOverflow {
                field: "AMF0 value offset",
                value: u64::try_from(offset).unwrap_or(u64::MAX),
                max: u64::try_from(usize::MAX).unwrap_or(u64::MAX),
            },
        ))?;
    }
    Ok(values_out)
}

/// Encode `values` in order as one AMF0 byte sequence: what a message body
/// carrying several bare values is (`onMetaData` is a String followed by an
/// ECMA Array, §2 of the AMF0 spec).
///
/// # Errors
///
/// [`RtmpError::FieldOverflow`] when a string, key or array exceeds its AMF0
/// length prefix (see [`Amf0Value`]'s own serializer).
pub fn amf0_values(values: &[Amf0Value]) -> Result<Vec<u8>> {
    let len = values.iter().try_fold(0usize, |acc, value| {
        let value_len = value.serialized_len();
        acc.checked_add(value_len)
            .ok_or_else(|| sequence_length_overflow(acc, value_len))
    })?;
    let mut out = alloc::vec![0u8; len];
    let mut written = 0usize;
    for value in values {
        written += value.serialize_into(&mut out[written..])?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(v: &Amf0Value) {
        let bytes = v.try_to_bytes().unwrap();
        assert_eq!(bytes.len(), v.serialized_len());
        let parsed = Amf0Value::parse(&bytes).expect("parse");
        assert_eq!(&parsed, v);
        // parse -> serialize -> byte-identical
        assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
    }

    #[test]
    fn a_value_sequence_encodes_each_value_in_order() {
        let values = [
            Amf0Value::String("onMetaData".to_string()),
            Amf0Value::EcmaArray(vec![("duration".to_string(), Amf0Value::Number(1.5))]),
        ];
        let body = amf0_values(&values).expect("encode");
        let mut expected = values[0].try_to_bytes().unwrap();
        expected.extend(values[1].try_to_bytes().unwrap());
        assert_eq!(body, expected);
    }

    #[test]
    fn a_value_sequence_parses_back_what_it_encoded() {
        let values = [
            Amf0Value::String("onMetaData".to_string()),
            Amf0Value::EcmaArray(vec![
                ("duration".to_string(), Amf0Value::Number(1.5)),
                ("stereo".to_string(), Amf0Value::Boolean(false)),
            ]),
        ];
        let body = amf0_values(&values).expect("encode");
        assert_eq!(amf0_parse_values(&body).expect("parse"), values);
    }

    #[test]
    fn a_truncated_value_sequence_is_an_error_not_a_partial_list() {
        let values = [Amf0Value::String("onMetaData".to_string())];
        let mut body = amf0_values(&values).expect("encode");
        body.pop();
        assert!(amf0_parse_values(&body).is_err());
    }

    /// #2761: the overflow error carried `value = u64::MAX, max = u64::MAX`,
    /// telling a caller nothing. It now carries the real current length and
    /// the addressable maximum.
    #[test]
    fn a_sequence_length_overflow_reports_the_real_lengths() {
        let err = sequence_length_overflow(6, 8);
        let RtmpError::FieldOverflow(overflow) = err else {
            panic!("expected FieldOverflow, got {err:?}");
        };
        assert_eq!(overflow.value, 14);
        assert_eq!(
            overflow.max,
            u64::try_from(usize::MAX).expect("usize fits u64 on every supported target")
        );
    }

    /// #2761: `Command::parse` advanced by each parsed value's re-serialized
    /// length. The parser now reports what it consumed, so a value whose
    /// encoding is not its canonical form (here AMF0 permits any non-zero
    /// Boolean byte, and `0x02` re-serializes as `0x01`) no longer shifts the
    /// boundary of the following argument.
    #[test]
    fn command_argument_boundaries_come_from_the_parse_not_a_re_derived_length() {
        let mut payload = Amf0Value::String("connect".to_string())
            .try_to_bytes()
            .unwrap();
        payload.extend(Amf0Value::Number(1.0).try_to_bytes().unwrap());
        for i in 0..8u8 {
            payload.extend([marker::BOOLEAN, i]);
        }

        let command = Command::parse(&payload).expect("any boolean byte is legal AMF0");
        assert_eq!(
            command.arguments,
            (0..8u8)
                .map(|i| Amf0Value::Boolean(i != 0))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn number_round_trips() {
        round_trip(&Amf0Value::Number(0.0));
        round_trip(&Amf0Value::Number(-1.5));
        round_trip(&Amf0Value::Number(1_000_000.25));
    }

    #[test]
    fn boolean_round_trips() {
        round_trip(&Amf0Value::Boolean(true));
        round_trip(&Amf0Value::Boolean(false));
    }

    /// An Object key of 65 536 bytes cannot fit its 16-bit length prefix
    /// (#1129): unfixed, `(k.len() as u16)` silently wrapped 65536 to 0
    /// while the full key bytes were still written, misframing the object.
    #[test]
    fn object_oversized_key_length_errors() {
        let key = "k".repeat(65536);
        let value = Amf0Value::Object(vec![(key, Amf0Value::Null)]);
        let err = value.try_to_bytes().unwrap_err();
        assert!(
            matches!(
                err,
                RtmpError::FieldOverflow(broadcast_core::len::FieldOverflow {
                    field: "amf0 object key length",
                    ..
                })
            ),
            "expected FieldOverflow for amf0 object key length, got {err:?}"
        );
    }

    /// The AMF0 String marker carries a 16-bit length prefix, so a longer
    /// value is rejected rather than written with a wrapped length.
    #[test]
    fn string_beyond_the_16_bit_prefix_is_rejected() {
        let value = Amf0Value::String("x".repeat(0x1_0000));
        let err = value.try_to_bytes().unwrap_err();
        assert!(
            matches!(err, RtmpError::Amf0StringTooLong { len } if len == 0x1_0000),
            "expected Amf0StringTooLong for a 65 536-byte string, got {err:?}"
        );
        let boundary = Amf0Value::String("x".repeat(usize::from(u16::MAX)));
        round_trip(&boundary);
    }

    /// The boundary: exactly 65 535 bytes (u16::MAX) still round-trips.
    #[test]
    fn object_max_key_length_round_trips() {
        let key = "k".repeat(65535);
        round_trip(&Amf0Value::Object(vec![(key, Amf0Value::Null)]));
    }

    #[test]
    fn string_round_trips_including_empty_and_multibyte() {
        round_trip(&Amf0Value::String(String::new()));
        round_trip(&Amf0Value::String("live".to_string()));
        round_trip(&Amf0Value::String("héllo wörld 日本語".to_string()));
    }

    #[test]
    fn null_and_undefined_round_trip() {
        round_trip(&Amf0Value::Null);
        round_trip(&Amf0Value::Undefined);
    }

    #[test]
    fn date_round_trips() {
        round_trip(&Amf0Value::Date(1_700_000_000_000.0));
    }

    #[test]
    fn long_string_round_trips() {
        round_trip(&Amf0Value::LongString("x".repeat(70_000)));
    }

    #[test]
    fn object_round_trips_including_nested_object() {
        round_trip(&Amf0Value::Object(vec![]));
        round_trip(&Amf0Value::Object(vec![
            ("app".to_string(), Amf0Value::String("live".to_string())),
            ("audioSampleRate".to_string(), Amf0Value::Number(44100.0)),
            ("live".to_string(), Amf0Value::Boolean(true)),
        ]));
        // Nested Object (the `connect` command object has string/number/
        // boolean fields, and encoders may nest e.g. a capabilities object).
        round_trip(&Amf0Value::Object(vec![(
            "capabilities".to_string(),
            Amf0Value::Object(vec![("videoCodecs".to_string(), Amf0Value::Number(252.0))]),
        )]));
    }

    #[test]
    fn ecma_array_round_trips() {
        round_trip(&Amf0Value::EcmaArray(vec![]));
        round_trip(&Amf0Value::EcmaArray(vec![
            ("duration".to_string(), Amf0Value::Number(0.0)),
            ("width".to_string(), Amf0Value::Number(1920.0)),
        ]));
    }

    #[test]
    fn strict_array_round_trips() {
        round_trip(&Amf0Value::StrictArray(vec![]));
        round_trip(&Amf0Value::StrictArray(vec![
            Amf0Value::Number(1.0),
            Amf0Value::String("two".to_string()),
            Amf0Value::Boolean(false),
            Amf0Value::Object(vec![("k".to_string(), Amf0Value::Null)]),
        ]));
    }

    #[test]
    fn ecma_array_count_is_informational_not_cross_checked() {
        // A real encoder writes the true pair count, but §2.10 makes the
        // object-end terminator authoritative — a lying count must still
        // parse correctly off the terminator, not the count field.
        let mut bytes = vec![marker::ECMA_ARRAY];
        bytes.extend_from_slice(&999u32.to_be_bytes()); // lying count
        bytes.extend_from_slice(&1u16.to_be_bytes());
        bytes.extend_from_slice(b"k");
        bytes.push(marker::NULL);
        bytes.extend_from_slice(&[0, 0, marker::OBJECT_END]);

        let parsed = Amf0Value::parse(&bytes).expect("parse");
        assert_eq!(
            parsed,
            Amf0Value::EcmaArray(vec![("k".to_string(), Amf0Value::Null)])
        );
    }

    #[test]
    fn depth_guard_rejects_pathological_nesting_without_stack_overflow() {
        // Build a deeply-nested Object payload by pure byte manipulation
        // (no recursive construction/serialization of our own types), so
        // the test itself can never stack-overflow regardless of the depth
        // guard's correctness.
        let mut inner = vec![marker::NULL];
        for _ in 0..(MAX_AMF0_DEPTH * 4) {
            let mut wrapped = vec![marker::OBJECT];
            wrapped.extend_from_slice(&1u16.to_be_bytes());
            wrapped.push(b'a');
            wrapped.extend_from_slice(&inner);
            wrapped.extend_from_slice(&[0, 0, marker::OBJECT_END]);
            inner = wrapped;
        }

        let result = Amf0Value::parse(&inner);
        assert!(matches!(
            result,
            Err(RtmpError::Amf0DepthExceeded { max }) if max == MAX_AMF0_DEPTH
        ));
    }

    #[test]
    fn depth_guard_allows_nesting_at_the_limit() {
        let mut inner = vec![marker::NULL];
        for _ in 0..(MAX_AMF0_DEPTH - 1) {
            let mut wrapped = vec![marker::OBJECT];
            wrapped.extend_from_slice(&1u16.to_be_bytes());
            wrapped.push(b'a');
            wrapped.extend_from_slice(&inner);
            wrapped.extend_from_slice(&[0, 0, marker::OBJECT_END]);
            inner = wrapped;
        }
        assert!(Amf0Value::parse(&inner).is_ok());
    }

    /// #2813: width was unbounded — an 8 MiB Strict Array of one-byte nulls
    /// decoded into millions of `Amf0Value`s, and an Object of three-byte
    /// empty-key pairs into millions of `(String, Amf0Value)` pairs. The
    /// value counter now rejects the element that crosses
    /// [`MAX_AMF0_VALUES`].
    #[test]
    fn value_count_guard_rejects_wide_containers() {
        let count = u32::try_from(MAX_AMF0_VALUES + 1).expect("limit fits u32");
        let mut bytes = vec![marker::STRICT_ARRAY];
        bytes.extend_from_slice(&count.to_be_bytes());
        bytes.extend(core::iter::repeat_n(marker::NULL, MAX_AMF0_VALUES + 1));

        let result = Amf0Value::parse(&bytes);
        assert!(matches!(
            result,
            Err(RtmpError::Amf0TooManyValues { max }) if max == MAX_AMF0_VALUES
        ));
    }

    #[test]
    fn value_count_guard_allows_a_container_at_the_limit() {
        let count = u32::try_from(MAX_AMF0_VALUES - 1).expect("limit fits u32");
        let mut bytes = vec![marker::STRICT_ARRAY];
        bytes.extend_from_slice(&count.to_be_bytes());
        bytes.extend(core::iter::repeat_n(marker::NULL, MAX_AMF0_VALUES - 1));

        let parsed =
            Amf0Value::parse(&bytes).expect("the array itself plus its elements is the limit");
        assert_eq!(
            parsed,
            Amf0Value::StrictArray(vec![Amf0Value::Null; MAX_AMF0_VALUES - 1])
        );
    }

    /// #2813: the counter spans the whole top-level value sequence, so a
    /// Command Message cannot dodge it by spreading its values over several
    /// arguments.
    #[test]
    fn command_parse_counts_values_across_every_argument() {
        let mut payload = Amf0Value::String("connect".to_string())
            .try_to_bytes()
            .expect("encode name");
        payload.extend(
            Amf0Value::Number(1.0)
                .try_to_bytes()
                .expect("encode transaction id"),
        );
        payload.extend(core::iter::repeat_n(marker::NULL, MAX_AMF0_VALUES));

        let result = Command::parse(&payload);
        assert!(matches!(
            result,
            Err(RtmpError::Amf0TooManyValues { max }) if max == MAX_AMF0_VALUES
        ));
    }

    #[test]
    fn dropping_object_end_marker_is_rejected() {
        // Mutation check: an Object with its `00 00 09` terminator dropped
        // must fail to parse (BufferTooShort), not silently succeed.
        let full = Amf0Value::Object(vec![("k".to_string(), Amf0Value::Null)])
            .try_to_bytes()
            .unwrap();
        let truncated = &full[..full.len() - 3];
        assert!(matches!(
            Amf0Value::parse(truncated),
            Err(RtmpError::BufferTooShort(..))
        ));
    }

    #[test]
    fn mis_sized_string_length_is_rejected() {
        // Mutation check: claiming a longer string length than the buffer
        // actually holds must fail (BufferTooShort), not read garbage.
        let mut bytes = vec![marker::STRING];
        bytes.extend_from_slice(&100u16.to_be_bytes()); // claims 100 bytes
        bytes.extend_from_slice(b"short"); // only 5 present
        assert!(matches!(
            Amf0Value::parse(&bytes),
            Err(RtmpError::BufferTooShort(..))
        ));
    }

    #[test]
    fn invalid_utf8_string_is_malformed() {
        let mut bytes = vec![marker::STRING];
        bytes.extend_from_slice(&2u16.to_be_bytes());
        bytes.extend_from_slice(&[0xFF, 0xFE]); // invalid UTF-8
        assert!(matches!(
            Amf0Value::parse(&bytes),
            Err(RtmpError::Amf0Utf8 { what: "string" })
        ));
    }

    #[test]
    fn unsupported_marker_is_rejected_not_panicking() {
        assert!(matches!(
            Amf0Value::parse(&[0x11]), // avmplus-object-marker (AMF3 switch)
            Err(RtmpError::UnsupportedAmf0Marker { marker: 0x11 })
        ));
        assert!(matches!(
            Amf0Value::parse(&[0x07]), // reference-marker
            Err(RtmpError::UnsupportedAmf0Marker { marker: 0x07 })
        ));
    }

    #[test]
    fn date_rejects_nonzero_reserved_timezone() {
        let mut bytes = vec![marker::DATE];
        bytes.extend_from_slice(&0.0f64.to_be_bytes());
        bytes.extend_from_slice(&1u16.to_be_bytes()); // reserved must be 0
        assert!(matches!(
            Amf0Value::parse(&bytes),
            Err(RtmpError::Amf0DateTimeZone { value: 1 })
        ));
    }

    #[test]
    fn empty_buffer_and_truncated_marker_are_buffer_too_short_not_panics() {
        assert!(matches!(
            Amf0Value::parse(&[]),
            Err(RtmpError::BufferTooShort(..))
        ));
        assert!(matches!(
            Amf0Value::parse(&[marker::NUMBER, 0, 0, 0]),
            Err(RtmpError::BufferTooShort(..))
        ));
    }

    // ── Command ──────────────────────────────────────────────────────────

    fn connect_command() -> Command {
        Command {
            name: "connect".to_string(),
            transaction_id: 1.0,
            arguments: vec![Amf0Value::Object(vec![
                ("app".to_string(), Amf0Value::String("live".to_string())),
                (
                    "flashVer".to_string(),
                    Amf0Value::String("FMLE/3.0".to_string()),
                ),
                (
                    "tcUrl".to_string(),
                    Amf0Value::String("rtmp://example.test/live".to_string()),
                ),
                ("fpad".to_string(), Amf0Value::Boolean(false)),
            ])],
        }
    }

    fn publish_command() -> Command {
        Command {
            name: "publish".to_string(),
            transaction_id: 5.0,
            arguments: vec![
                Amf0Value::Null,
                Amf0Value::String("stream_key_123".to_string()),
                Amf0Value::String("live".to_string()),
            ],
        }
    }

    #[test]
    fn connect_command_round_trips_byte_identically() {
        let cmd = connect_command();
        let bytes = cmd.to_body().unwrap();
        let parsed = Command::parse(&bytes).expect("parse connect");
        assert_eq!(parsed, cmd);
        assert_eq!(parsed.to_body().unwrap(), bytes);
    }

    #[test]
    fn publish_command_round_trips_byte_identically() {
        let cmd = publish_command();
        let bytes = cmd.to_body().unwrap();
        let parsed = Command::parse(&bytes).expect("parse publish");
        assert_eq!(parsed, cmd);
        assert_eq!(parsed.to_body().unwrap(), bytes);
    }

    #[test]
    fn command_name_must_be_string() {
        let bytes = Amf0Value::Number(1.0).try_to_bytes().unwrap();
        assert!(matches!(
            Command::parse(&bytes),
            Err(RtmpError::CommandNameNotString { .. })
        ));
    }

    #[test]
    fn command_transaction_id_must_be_number() {
        let mut bytes = Amf0Value::String("connect".to_string())
            .try_to_bytes()
            .unwrap();
        bytes.extend(
            Amf0Value::String("not a number".to_string())
                .try_to_bytes()
                .unwrap(),
        );
        assert!(matches!(
            Command::parse(&bytes),
            Err(RtmpError::CommandTransactionIdNotNumber { .. })
        ));
    }

    // ── 32-bit overflow guards (mutation checks) ─────────────────────────

    #[test]
    fn long_string_length_overflowing_usize_is_rejected_not_wrapped() {
        // Mutation check: a Long String claiming `usize::MAX - 1` bytes must
        // be rejected as too-short/malformed, not silently wrap `U32_LEN +
        // len` into a small `total` that then compares as if the string
        // were actually present.
        let mut bytes = vec![marker::LONG_STRING];
        bytes.extend_from_slice(&(u32::MAX - 1).to_be_bytes());
        bytes.extend_from_slice(b"short");
        let err = Amf0Value::parse(&bytes).unwrap_err();
        assert!(matches!(
            err,
            RtmpError::Malformed { .. } | RtmpError::BufferTooShort(..)
        ));
    }

    // ── serde (feature "serde") ───────────────────────────────────────────

    #[cfg(feature = "serde")]
    #[test]
    fn amf0_value_and_command_serde_round_trip() {
        let value = Amf0Value::Object(vec![
            ("app".to_string(), Amf0Value::String("live".to_string())),
            ("live".to_string(), Amf0Value::Boolean(true)),
            ("duration".to_string(), Amf0Value::Number(0.0)),
            (
                "items".to_string(),
                Amf0Value::StrictArray(vec![Amf0Value::Null, Amf0Value::Undefined]),
            ),
        ]);
        let json = serde_json::to_string(&value).expect("serialize Amf0Value");
        let back: Amf0Value = serde_json::from_str(&json).expect("deserialize Amf0Value");
        assert_eq!(back, value);

        let cmd = publish_command();
        let json = serde_json::to_string(&cmd).expect("serialize Command");
        let back: Command = serde_json::from_str(&json).expect("deserialize Command");
        assert_eq!(back, cmd);
    }

    /// Offsets through nested containers (an object holding an ECMA array
    /// holding a >65535-byte Long String, then a Date) are exact in both
    /// directions.
    #[test]
    fn nested_container_offsets_pin_parse_and_serialize() {
        let long = "L".repeat(70_000);
        let nested = Amf0Value::StrictArray(vec![
            Amf0Value::Object(vec![
                ("a".to_string(), Amf0Value::Number(1.5)),
                (
                    "b".to_string(),
                    Amf0Value::EcmaArray(vec![
                        ("x".to_string(), Amf0Value::LongString(long)),
                        ("y".to_string(), Amf0Value::Null),
                    ]),
                ),
            ]),
            Amf0Value::Date(1.0e12),
            Amf0Value::Boolean(true),
            Amf0Value::Undefined,
        ]);
        let bytes = nested.try_to_bytes().expect("serialize");
        assert_eq!(bytes.len(), nested.serialized_len());

        assert_eq!(
            &bytes[..10],
            &[
                marker::STRICT_ARRAY,
                0,
                0,
                0,
                4,
                marker::OBJECT,
                0,
                1,
                b'a',
                marker::NUMBER
            ]
        );
        assert_eq!(&bytes[10..18], &1.5f64.to_be_bytes());
        assert_eq!(
            &bytes[18..26],
            &[0, 1, b'b', marker::ECMA_ARRAY, 0, 0, 0, 2]
        );
        assert_eq!(&bytes[26..30], &[0, 1, b'x', marker::LONG_STRING]);
        assert_eq!(&bytes[30..34], &70_000u32.to_be_bytes());
        assert_eq!(&bytes[34..36], b"LL");

        let long_end = 34 + 70_000;
        assert_eq!(&bytes[long_end..long_end + 4], &[0, 1, b'y', marker::NULL]);
        assert_eq!(
            &bytes[long_end + 4..long_end + 10],
            &[0, 0, marker::OBJECT_END, 0, 0, marker::OBJECT_END]
        );
        assert_eq!(bytes[long_end + 10], marker::DATE);
        assert_eq!(
            &bytes[long_end + 11..long_end + 19],
            &1.0e12f64.to_be_bytes()
        );
        assert_eq!(&bytes[long_end + 19..long_end + 21], &[0, 0]);
        assert_eq!(
            &bytes[long_end + 21..long_end + 24],
            &[marker::BOOLEAN, 1, marker::UNDEFINED]
        );
        assert_eq!(bytes.len(), long_end + 24);

        let mut values = 0usize;
        let (parsed, consumed) = parse_value(&bytes, 0, &mut values).expect("parse");
        assert_eq!(parsed, nested);
        assert_eq!(consumed, bytes.len());
    }

    /// Every AMF0 variant writes exactly the byte count it declares.
    #[test]
    fn every_variant_writes_its_declared_length() {
        let variants = vec![
            Amf0Value::Number(-1.5),
            Amf0Value::Boolean(false),
            Amf0Value::String("hello".to_string()),
            Amf0Value::Object(vec![("k".to_string(), Amf0Value::Null)]),
            Amf0Value::Null,
            Amf0Value::Undefined,
            Amf0Value::EcmaArray(vec![("k".to_string(), Amf0Value::Date(0.0))]),
            Amf0Value::StrictArray(vec![Amf0Value::Boolean(true), Amf0Value::Null]),
            Amf0Value::Date(1.0e12),
            Amf0Value::LongString("x".repeat(70_000)),
        ];
        for value in variants {
            let mut buf = vec![0u8; value.serialized_len()];
            assert_eq!(
                value.serialize_into(&mut buf).unwrap(),
                value.serialized_len()
            );
            let mut values = 0usize;
            let (parsed, consumed) = parse_value(&buf, 0, &mut values).unwrap();
            assert_eq!(consumed, buf.len());
            assert_eq!(parsed, value);
        }
    }

    /// The Long String length prefix accepts exactly that many bytes, rejects
    /// one fewer, and an empty long string is valid.
    #[test]
    fn long_string_length_prefix_boundaries() {
        assert_eq!(
            Amf0Value::parse(&[marker::LONG_STRING, 0, 0, 0, 0]).unwrap(),
            Amf0Value::LongString(String::new())
        );
        let mut ok = vec![marker::LONG_STRING, 0, 0, 0, 3];
        ok.extend_from_slice(b"abc");
        assert_eq!(
            Amf0Value::parse(&ok).unwrap(),
            Amf0Value::LongString("abc".to_string())
        );
        assert!(matches!(
            Amf0Value::parse(&ok[..ok.len() - 1]),
            Err(RtmpError::BufferTooShort(_))
        ));
    }

    /// Truncating the encoding at any point inside a nested value is a
    /// BufferTooShort, never a mis-framed shorter value.
    #[test]
    fn truncating_a_nested_encoding_is_always_buffer_too_short() {
        let value = Amf0Value::StrictArray(vec![
            Amf0Value::Object(vec![("k".to_string(), Amf0Value::Date(1.0e12))]),
            Amf0Value::EcmaArray(vec![("s".to_string(), Amf0Value::String("v".to_string()))]),
        ]);
        let bytes = value.try_to_bytes().unwrap();
        for cut in 1..bytes.len() {
            assert!(
                matches!(
                    Amf0Value::parse(&bytes[..cut]),
                    Err(RtmpError::BufferTooShort(_))
                ),
                "cut at {cut} must be BufferTooShort"
            );
        }
    }

    /// A truncated container reports the exact byte count its next field
    /// needs.
    #[test]
    fn truncated_containers_report_the_exact_need() {
        let bytes = [marker::ECMA_ARRAY, 0, 0, 0, 0];
        match Amf0Value::parse(&bytes) {
            Err(RtmpError::BufferTooShort(b)) => {
                assert_eq!(b.need, 2);
                assert_eq!(b.have, 0);
            }
            other => panic!("expected BufferTooShort, got {other:?}"),
        }

        let bytes = [marker::DATE, 0, 0, 0, 0, 0, 0];
        match Amf0Value::parse(&bytes) {
            Err(RtmpError::BufferTooShort(b)) => {
                assert_eq!(b.need, 10);
                assert_eq!(b.have, 6);
            }
            other => panic!("expected BufferTooShort, got {other:?}"),
        }

        let bytes = [marker::OBJECT, 0, 0];
        match Amf0Value::parse(&bytes) {
            Err(RtmpError::BufferTooShort(b)) => {
                assert_eq!(b.need, 3);
                assert_eq!(b.have, 2);
            }
            other => panic!("expected BufferTooShort, got {other:?}"),
        }
    }

    /// Serializing an object into a pre-filled buffer writes every byte of
    /// its range, including both object-end zeroes.
    #[test]
    fn object_serialize_overwrites_every_byte_of_its_range() {
        let value = Amf0Value::Object(vec![("k".to_string(), Amf0Value::Null)]);
        let mut buf = vec![0xFFu8; value.serialized_len()];
        assert_eq!(
            value.serialize_into(&mut buf).unwrap(),
            value.serialized_len()
        );
        assert_eq!(
            buf,
            vec![
                marker::OBJECT,
                0,
                1,
                b'k',
                marker::NULL,
                0,
                0,
                marker::OBJECT_END
            ]
        );
    }

    /// The depth guard counts ECMA-array and strict-array nesting too.
    #[test]
    fn depth_guard_counts_every_container_kind() {
        let mut inner = vec![marker::NULL];
        for _ in 0..(MAX_AMF0_DEPTH + 1) {
            let mut wrapped = vec![marker::ECMA_ARRAY, 0, 0, 0, 1];
            wrapped.extend_from_slice(&1u16.to_be_bytes());
            wrapped.push(b'a');
            wrapped.extend_from_slice(&inner);
            wrapped.extend_from_slice(&[0, 0, marker::OBJECT_END]);
            inner = wrapped;
        }
        assert!(matches!(
            Amf0Value::parse(&inner),
            Err(RtmpError::Amf0DepthExceeded { .. })
        ));

        let mut inner = vec![marker::NULL];
        for _ in 0..(MAX_AMF0_DEPTH + 1) {
            let mut wrapped = vec![marker::STRICT_ARRAY, 0, 0, 0, 1];
            wrapped.extend_from_slice(&inner);
            inner = wrapped;
        }
        assert!(matches!(
            Amf0Value::parse(&inner),
            Err(RtmpError::Amf0DepthExceeded { .. })
        ));
    }
}
