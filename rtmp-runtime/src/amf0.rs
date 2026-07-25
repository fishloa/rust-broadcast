//! AMF0 (Action Message Format 0) value encoding/decoding, used by RTMP
//! command and data messages.
//!
//! See [`docs/rtmp.md`](../docs/rtmp.md) §8 (AMF0) for the full transcription
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
//! Out of scope, and rejected as [`RtmpError::Unsupported`] rather than
//! panicking or silently misparsing: `movieclip-marker` (`0x04`, reserved),
//! `reference-marker` (`0x07`), `unsupported-marker` (`0x0D`),
//! `recordset-marker` (`0x0E`, reserved), `xml-document-marker` (`0x0F`),
//! `typed-object-marker` (`0x10`), and `avmplus-object-marker` (`0x11`, the
//! AMF3 switch — AMF3 itself, `[AMF3]`, is a separate spec and explicitly
//! out of scope for this ingest engine per docs/rtmp.md §8.3).
//!
//! [`Amf0Value`] is a data-carrying ADT (like `Fmt`/`MessageHeader`'s payload
//! variants), not a closed label enum, so it is a `#204` `label_coverage`
//! SKIP-list candidate rather than a `name()`/`impl_spec_display!` target
//! (tracked for the crate's Task 10 label-coverage pass).

use broadcast_common::{Parse, Serialize};

use crate::RtmpError;

type Result<T> = core::result::Result<T, RtmpError>;

/// AMF0 type markers (`[AMF0]` §2.1, docs/rtmp.md §8.1) — 1 byte.
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
/// pathologically nested input returns [`RtmpError::Unsupported`] instead of
/// overflowing the native call stack — the guard is checked on *entering*
/// each nested container, so recursion never actually reaches an
/// attacker-chosen depth, only this constant.
pub const MAX_AMF0_DEPTH: usize = 32;

const MARKER_LEN: usize = 1;
const NUMBER_LEN: usize = 8;
const BOOLEAN_LEN: usize = 1;
const U16_LEN: usize = 2;
const U32_LEN: usize = 4;
const DATE_RESERVED_LEN: usize = 2;
/// `00 00 09`: empty key + [`marker::OBJECT_END`].
const OBJECT_END_LEN: usize = 3;

/// A single AMF0 value (`[AMF0]` §2, docs/rtmp.md §8.2).
///
/// AMF3 (`avmplus-object-marker`, `0x11`) and the reserved/legacy markers
/// (`movieclip`, `reference`, `unsupported`, `recordset`, `xml-document`,
/// `typed-object`) are out of scope — see the module doc.
#[derive(Debug, Clone, PartialEq)]
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
    RtmpError::BufferTooShort { need, have, what }
}

/// Read a `U16`-length-prefixed UTF-8 string from the front of `bytes`.
/// Returns the decoded string and the total bytes consumed (`2 + len`).
/// Used for both the String value body and Object/ECMA-Array keys, which
/// share this exact encoding (§2.4 / §2.5).
fn read_utf8_short(bytes: &[u8], what: &'static str) -> Result<(String, usize)> {
    if bytes.len() < U16_LEN {
        return Err(buffer_too_short(U16_LEN, bytes.len(), what));
    }
    let len = u16::from_be_bytes([bytes[0], bytes[1]]) as usize;
    let total = U16_LEN + len;
    if bytes.len() < total {
        return Err(buffer_too_short(total, bytes.len(), what));
    }
    let s = String::from_utf8(bytes[U16_LEN..total].to_vec())
        .map_err(|_| RtmpError::Malformed { what })?;
    Ok((s, total))
}

/// Read a `U32`-length-prefixed UTF-8 string (Long String, §2.14).
fn read_utf8_long(bytes: &[u8], what: &'static str) -> Result<(String, usize)> {
    if bytes.len() < U32_LEN {
        return Err(buffer_too_short(U32_LEN, bytes.len(), what));
    }
    let len = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
    let total = U32_LEN + len;
    if bytes.len() < total {
        return Err(buffer_too_short(total, bytes.len(), what));
    }
    let s = String::from_utf8(bytes[U32_LEN..total].to_vec())
        .map_err(|_| RtmpError::Malformed { what })?;
    Ok((s, total))
}

/// Parse the key/value pairs of an Object or ECMA Array body (§2.5/§2.10),
/// starting right after any leading count field. Terminates on an empty key
/// followed by [`marker::OBJECT_END`] (§2.11). `depth` is the nesting depth
/// of the *values* in this container (already incremented past the
/// container itself by the caller).
fn parse_pairs(bytes: &[u8], depth: usize) -> Result<(Vec<(String, Amf0Value)>, usize)> {
    let mut consumed = 0;
    let mut pairs = Vec::new();
    loop {
        let (key, key_len) = read_utf8_short(&bytes[consumed..], "amf0 object key")?;
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
        let value = parse_value(&bytes[after_key..], depth)?;
        let value_len = value.serialized_len();
        pairs.push((key, value));
        consumed = after_key + value_len;
    }
}

/// Parse one AMF0 value (marker + body) from the front of `bytes`, ignoring
/// any surplus trailing bytes. `depth` counts container nesting already
/// entered (0 at the top level); checked *before* descending into a nested
/// Object/ECMA-Array/Strict-Array so recursion is bounded by
/// [`MAX_AMF0_DEPTH`] regardless of how deeply the input is (adversarially)
/// nested.
fn parse_value(bytes: &[u8], depth: usize) -> Result<Amf0Value> {
    if bytes.is_empty() {
        return Err(buffer_too_short(MARKER_LEN, 0, "amf0 value marker"));
    }
    let body = &bytes[MARKER_LEN..];
    match bytes[0] {
        marker::NUMBER => {
            if body.len() < NUMBER_LEN {
                return Err(buffer_too_short(NUMBER_LEN, body.len(), "amf0 number"));
            }
            let mut b = [0u8; NUMBER_LEN];
            b.copy_from_slice(&body[..NUMBER_LEN]);
            Ok(Amf0Value::Number(f64::from_be_bytes(b)))
        }
        marker::BOOLEAN => {
            if body.is_empty() {
                return Err(buffer_too_short(BOOLEAN_LEN, 0, "amf0 boolean"));
            }
            Ok(Amf0Value::Boolean(body[0] != 0))
        }
        marker::STRING => {
            let (s, _) = read_utf8_short(body, "amf0 string")?;
            Ok(Amf0Value::String(s))
        }
        marker::OBJECT => {
            if depth >= MAX_AMF0_DEPTH {
                return Err(RtmpError::Unsupported {
                    what: "amf0 nesting depth exceeded",
                });
            }
            let (pairs, _) = parse_pairs(body, depth + 1)?;
            Ok(Amf0Value::Object(pairs))
        }
        marker::NULL => Ok(Amf0Value::Null),
        marker::UNDEFINED => Ok(Amf0Value::Undefined),
        marker::ECMA_ARRAY => {
            if depth >= MAX_AMF0_DEPTH {
                return Err(RtmpError::Unsupported {
                    what: "amf0 nesting depth exceeded",
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
            let (pairs, _) = parse_pairs(&body[U32_LEN..], depth + 1)?;
            Ok(Amf0Value::EcmaArray(pairs))
        }
        marker::STRICT_ARRAY => {
            if depth >= MAX_AMF0_DEPTH {
                return Err(RtmpError::Unsupported {
                    what: "amf0 nesting depth exceeded",
                });
            }
            if body.len() < U32_LEN {
                return Err(buffer_too_short(
                    U32_LEN,
                    body.len(),
                    "amf0 strict array count",
                ));
            }
            let count = u32::from_be_bytes([body[0], body[1], body[2], body[3]]);
            let mut rest = &body[U32_LEN..];
            let mut values = Vec::new();
            for _ in 0..count {
                let value = parse_value(rest, depth + 1)?;
                let consumed = value.serialized_len();
                values.push(value);
                rest = &rest[consumed..];
            }
            Ok(Amf0Value::StrictArray(values))
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
                return Err(RtmpError::Malformed {
                    what: "amf0 date reserved time zone (must be 0x0000)",
                });
            }
            Ok(Amf0Value::Date(f64::from_be_bytes(b)))
        }
        marker::LONG_STRING => {
            let (s, _) = read_utf8_long(body, "amf0 long string")?;
            Ok(Amf0Value::LongString(s))
        }
        _ => Err(RtmpError::Unsupported {
            what: "amf0 value marker (reserved, legacy, or amf3-switch)",
        }),
    }
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
        buf[offset..offset + U16_LEN].copy_from_slice(&(k.len() as u16).to_be_bytes());
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
    buf[offset + 2] = marker::OBJECT_END;
    Ok(offset + OBJECT_END_LEN)
}

impl<'a> Parse<'a> for Amf0Value {
    type Error = RtmpError;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        parse_value(bytes, 0)
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
                    return Err(RtmpError::Unsupported {
                        what: "amf0 string exceeds u16 length (use long string)",
                    });
                }
                marker_byte[0] = marker::STRING;
                body[..U16_LEN].copy_from_slice(&(s.len() as u16).to_be_bytes());
                body[U16_LEN..].copy_from_slice(s.as_bytes());
            }
            Amf0Value::LongString(s) => {
                marker_byte[0] = marker::LONG_STRING;
                body[..U32_LEN].copy_from_slice(&(s.len() as u32).to_be_bytes());
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
                body[..U32_LEN].copy_from_slice(&(pairs.len() as u32).to_be_bytes());
                write_pairs(pairs, &mut body[U32_LEN..])?;
            }
            Amf0Value::StrictArray(values) => {
                marker_byte[0] = marker::STRICT_ARRAY;
                body[..U32_LEN].copy_from_slice(&(values.len() as u32).to_be_bytes());
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
/// a Command Object, then optional trailing arguments) — see docs/rtmp.md
/// §8.2's closing note and §6.1 (Command Message).
///
/// This is the message-body-level container: unlike [`Amf0Value`] it is not
/// itself a single AMF0 `value-type`, so it uses inherent `parse`/`to_body`
/// methods rather than the [`Parse`]/[`Serialize`] traits.
#[derive(Debug, Clone, PartialEq)]
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
    /// [`RtmpError::Malformed`] if the first value is not a String or the
    /// second is not a Number; [`RtmpError::BufferTooShort`] /
    /// [`RtmpError::Unsupported`] propagated from [`Amf0Value::parse`].
    pub fn parse(payload: &[u8]) -> Result<Self> {
        let name_value = Amf0Value::parse(payload)?;
        let mut offset = name_value.serialized_len();
        let name = match name_value {
            Amf0Value::String(s) => s,
            _ => {
                return Err(RtmpError::Malformed {
                    what: "rtmp command name (expected amf0 string)",
                });
            }
        };

        let txn_value = Amf0Value::parse(&payload[offset..])?;
        offset += txn_value.serialized_len();
        let transaction_id = match txn_value {
            Amf0Value::Number(n) => n,
            _ => {
                return Err(RtmpError::Malformed {
                    what: "rtmp command transaction id (expected amf0 number)",
                });
            }
        };

        let mut arguments = Vec::new();
        while offset < payload.len() {
            let value = Amf0Value::parse(&payload[offset..])?;
            offset += value.serialized_len();
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
    #[must_use]
    pub fn to_body(&self) -> Vec<u8> {
        let mut out = Amf0Value::String(self.name.clone()).to_bytes();
        out.extend(Amf0Value::Number(self.transaction_id).to_bytes());
        for arg in &self.arguments {
            out.extend(arg.to_bytes());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(v: &Amf0Value) {
        let bytes = v.to_bytes();
        assert_eq!(bytes.len(), v.serialized_len());
        let parsed = Amf0Value::parse(&bytes).expect("parse");
        assert_eq!(&parsed, v);
        // parse -> serialize -> byte-identical
        assert_eq!(parsed.to_bytes(), bytes);
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
        assert!(matches!(result, Err(RtmpError::Unsupported { .. })));
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

    #[test]
    fn dropping_object_end_marker_is_rejected() {
        // Mutation check: an Object with its `00 00 09` terminator dropped
        // must fail to parse (BufferTooShort), not silently succeed.
        let full = Amf0Value::Object(vec![("k".to_string(), Amf0Value::Null)]).to_bytes();
        let truncated = &full[..full.len() - 3];
        assert!(Amf0Value::parse(truncated).is_err());
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
            Err(RtmpError::BufferTooShort { .. })
        ));
    }

    #[test]
    fn invalid_utf8_string_is_malformed() {
        let mut bytes = vec![marker::STRING];
        bytes.extend_from_slice(&2u16.to_be_bytes());
        bytes.extend_from_slice(&[0xFF, 0xFE]); // invalid UTF-8
        assert!(matches!(
            Amf0Value::parse(&bytes),
            Err(RtmpError::Malformed { .. })
        ));
    }

    #[test]
    fn unsupported_marker_is_rejected_not_panicking() {
        assert!(matches!(
            Amf0Value::parse(&[0x11]), // avmplus-object-marker (AMF3 switch)
            Err(RtmpError::Unsupported { .. })
        ));
        assert!(matches!(
            Amf0Value::parse(&[0x07]), // reference-marker
            Err(RtmpError::Unsupported { .. })
        ));
    }

    #[test]
    fn date_rejects_nonzero_reserved_timezone() {
        let mut bytes = vec![marker::DATE];
        bytes.extend_from_slice(&0.0f64.to_be_bytes());
        bytes.extend_from_slice(&1u16.to_be_bytes()); // reserved must be 0
        assert!(matches!(
            Amf0Value::parse(&bytes),
            Err(RtmpError::Malformed { .. })
        ));
    }

    #[test]
    fn empty_buffer_and_truncated_marker_are_buffer_too_short_not_panics() {
        assert!(matches!(
            Amf0Value::parse(&[]),
            Err(RtmpError::BufferTooShort { .. })
        ));
        assert!(matches!(
            Amf0Value::parse(&[marker::NUMBER, 0, 0, 0]),
            Err(RtmpError::BufferTooShort { .. })
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
        let bytes = cmd.to_body();
        let parsed = Command::parse(&bytes).expect("parse connect");
        assert_eq!(parsed, cmd);
        assert_eq!(parsed.to_body(), bytes);
    }

    #[test]
    fn publish_command_round_trips_byte_identically() {
        let cmd = publish_command();
        let bytes = cmd.to_body();
        let parsed = Command::parse(&bytes).expect("parse publish");
        assert_eq!(parsed, cmd);
        assert_eq!(parsed.to_body(), bytes);
    }

    #[test]
    fn command_name_must_be_string() {
        let bytes = Amf0Value::Number(1.0).to_bytes();
        assert!(matches!(
            Command::parse(&bytes),
            Err(RtmpError::Malformed { .. })
        ));
    }

    #[test]
    fn command_transaction_id_must_be_number() {
        let mut bytes = Amf0Value::String("connect".to_string()).to_bytes();
        bytes.extend(Amf0Value::String("not a number".to_string()).to_bytes());
        assert!(matches!(
            Command::parse(&bytes),
            Err(RtmpError::Malformed { .. })
        ));
    }
}
