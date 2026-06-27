//! `emsg` — MPEG-DASH Event Message Box.
//!
//! Field semantics: DASH-IF IOP Part 10 §6.1 / Table 6-2 (free). Box syntax
//! (the `aligned(8) class EventMessageBox extends FullBox('emsg', version,
//! flags = 0)` declaration, field ordering, and the null-terminated-string
//! layout): ISO/IEC 23009-1 §5.10.3.3 (paid, **not vendored** — see the crate
//! root caveat). Both the field set/types and the v0/v1 ordering difference are
//! transcribed in `mp4-emsg/docs/emsg.md`.
//!
//! Wire layout (ISOBMFF `FullBox`):
//!
//! ```text
//! size      u32      total box size in bytes (recomputed on serialize)
//! type      'emsg'   the 4-byte box type
//! version   u8       0 or 1
//! flags     u24      0
//! ── body (version 0, segment-relative) ──
//! scheme_id_uri          null-terminated UTF-8
//! value                  null-terminated UTF-8
//! timescale              u32
//! presentation_time_delta u32
//! event_duration         u32
//! id                     u32
//! message_data[]         remaining bytes
//! ── body (version 1, representation-relative) ──
//! timescale              u32
//! presentation_time      u64
//! event_duration         u32
//! id                     u32
//! scheme_id_uri          null-terminated UTF-8
//! value                  null-terminated UTF-8
//! message_data[]         remaining bytes
//! ```
//!
//! Note the **field ordering differs between v0 and v1** (strings first in v0;
//! integers first / strings last in v1) per `docs/emsg.md`.

use alloc::vec::Vec;

use crate::error::{Error, Result};
use crate::version::EmsgVersion;

/// The 4-byte ISOBMFF box type for an Event Message Box.
pub const EMSG_BOX_TYPE: [u8; 4] = *b"emsg";
/// Size in bytes of the `FullBox` header: `size`(4) + `type`(4) + `version`(1)
/// + `flags`(3).
pub const FULLBOX_HEADER_LEN: usize = 12;
/// `flags` value mandated for `emsg` (DASH-IF Part 10 / ISO box syntax: 0).
pub const EMSG_FLAGS: u32 = 0;
/// The single string terminator byte for the null-terminated UTF-8 fields.
pub const STRING_TERMINATOR: u8 = 0x00;

const U32_LEN: usize = 4;
const U64_LEN: usize = 8;

/// The version-discriminated presentation-time field (DASH-IF Part 10 Table
/// 6-2): the *only* field whose type and reference point differ by version.
///
/// `version` is derived from this enum: [`PresentationTime::Delta`] ⇒ version 0
/// (segment-relative), [`PresentationTime::Absolute`] ⇒ version 1
/// (representation-relative).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum PresentationTime {
    /// **version 0**: `presentation_time_delta` (u32), relative to the
    /// segment's earliest presentation time, in units of `timescale`.
    Delta(u32),
    /// **version 1**: `presentation_time` (u64), relative to `Period@start`
    /// (adjusted by `@presentationTimeOffset`), in units of `timescale`.
    Absolute(u64),
}

impl PresentationTime {
    /// The `emsg` version implied by this presentation-time variant.
    pub fn version(self) -> EmsgVersion {
        match self {
            PresentationTime::Delta(_) => EmsgVersion::SegmentRelative,
            PresentationTime::Absolute(_) => EmsgVersion::RepresentationRelative,
        }
    }

    /// Spec label for this presentation-time variant.
    pub fn name(&self) -> &'static str {
        match self {
            PresentationTime::Delta(_) => "presentation_time_delta",
            PresentationTime::Absolute(_) => "presentation_time",
        }
    }
}

dvb_common::impl_spec_display!(PresentationTime, Delta, Absolute);

/// A parsed/owned MPEG-DASH Event Message Box (`emsg`).
///
/// Holds the typed `FullBox` body fields plus borrowed views of the two
/// null-terminated UTF-8 strings and the opaque `message_data`. The box `size`
/// and `version` are *not* stored: `size` is recomputed on serialize and
/// `version` is derived from [`PresentationTime`], so the round-trip is driven
/// entirely from the typed fields (no raw passthrough).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct EmsgBox<'a> {
    /// `scheme_id_uri` (Table 6-2): the URI defining the event scheme. Stored
    /// without the wire `0x00` terminator.
    pub scheme_id_uri: &'a str,
    /// `value` (Table 6-2): scheme-specific value, or the empty string when the
    /// scheme defines none. Stored without the wire `0x00` terminator.
    pub value: &'a str,
    /// `timescale` (Table 6-2, u32): ticks per second for the time fields;
    /// equal to the `mdhd` timescale of the Representation.
    pub timescale: u32,
    /// The presentation-time field — selects the box `version` (v0 delta vs v1
    /// absolute).
    pub presentation_time: PresentationTime,
    /// `event_duration` (Table 6-2, u32): event duration in `timescale` units.
    pub event_duration: u32,
    /// `id` (Table 6-2, u32): unique identifier distinguishing events with the
    /// same `scheme_id_uri`/`value` and detecting repetitions.
    pub id: u32,
    /// `message_data[]` (Table 6-2): the opaque scheme-specific payload (the
    /// remaining bytes of the box). Empty when the scheme needs none.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub message_data: &'a [u8],
}

impl<'a> EmsgBox<'a> {
    /// The `version` of this box, derived from [`Self::presentation_time`].
    pub fn version(&self) -> EmsgVersion {
        self.presentation_time.version()
    }

    /// `true` if `scheme_id_uri` names a SCTE 35 scheme (`urn:scte:scte35...`),
    /// in which case [`Self::message_data`] carries a SCTE 35
    /// `splice_info_section` (SCTE 214-1 / ANSI/SCTE 35 — see crate root).
    pub fn is_scte35(&self) -> bool {
        self.scheme_id_uri.starts_with(SCTE35_SCHEME_PREFIX)
    }

    /// Total serialized size in bytes (the `size` field value).
    pub fn serialized_len(&self) -> usize {
        FULLBOX_HEADER_LEN + self.body_len()
    }

    /// Size in bytes of the version-specific body (everything after the
    /// `FullBox` header).
    fn body_len(&self) -> usize {
        let strings = self.scheme_id_uri.len()
            + 1 // scheme terminator
            + self.value.len()
            + 1; // value terminator
        let ints = match self.presentation_time {
            // timescale + delta + event_duration + id
            PresentationTime::Delta(_) => U32_LEN * 4,
            // timescale + presentation_time(64) + event_duration + id
            PresentationTime::Absolute(_) => U32_LEN * 3 + U64_LEN,
        };
        strings + ints + self.message_data.len()
    }

    /// Parse an `emsg` box from the start of `data`. Requires the full box
    /// (`size` bytes) to be present; trailing bytes beyond `size` are ignored.
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        if data.len() < FULLBOX_HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: FULLBOX_HEADER_LEN,
                have: data.len(),
                what: "emsg FullBox header",
            });
        }

        let size = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
        let box_type = [data[4], data[5], data[6], data[7]];
        if box_type != EMSG_BOX_TYPE {
            return Err(Error::NotEmsg { found: box_type });
        }
        let version = data[8];
        // flags = data[9..12]; mandated 0, but parsed permissively (some muxers
        // are sloppy) — not stored since it is recomputed as 0 on serialize.

        if (size as usize) < FULLBOX_HEADER_LEN {
            return Err(Error::InvalidSize {
                size,
                reason: "size smaller than the FullBox header (0/1 large-size forms unsupported)",
            });
        }
        if size as usize > data.len() {
            return Err(Error::InvalidSize {
                size,
                reason: "size exceeds available bytes",
            });
        }

        let version = EmsgVersion::from_u8(version).ok_or(Error::UnsupportedVersion { version })?;

        // The box body region is [FULLBOX_HEADER_LEN, size).
        let body = &data[FULLBOX_HEADER_LEN..size as usize];

        match version {
            EmsgVersion::SegmentRelative => Self::parse_v0(body),
            EmsgVersion::RepresentationRelative => Self::parse_v1(body),
        }
    }

    /// version 0 body: scheme\0 value\0 timescale u32 ptd u32 dur u32 id u32
    /// message_data[].
    fn parse_v0(body: &'a [u8]) -> Result<Self> {
        let (scheme_id_uri, rest) = parse_cstr(body, "scheme_id_uri")?;
        let (value, rest) = parse_cstr(rest, "value")?;

        let need = U32_LEN * 4;
        if rest.len() < need {
            return Err(Error::BufferTooShort {
                need,
                have: rest.len(),
                what: "emsg v0 integer fields",
            });
        }
        let timescale = read_u32(&rest[0..U32_LEN]);
        let delta = read_u32(&rest[U32_LEN..U32_LEN * 2]);
        let event_duration = read_u32(&rest[U32_LEN * 2..U32_LEN * 3]);
        let id = read_u32(&rest[U32_LEN * 3..U32_LEN * 4]);
        let message_data = &rest[need..];

        Ok(EmsgBox {
            scheme_id_uri,
            value,
            timescale,
            presentation_time: PresentationTime::Delta(delta),
            event_duration,
            id,
            message_data,
        })
    }

    /// version 1 body: timescale u32 presentation_time u64 dur u32 id u32
    /// scheme\0 value\0 message_data[].
    fn parse_v1(body: &'a [u8]) -> Result<Self> {
        let need = U32_LEN + U64_LEN + U32_LEN + U32_LEN;
        if body.len() < need {
            return Err(Error::BufferTooShort {
                need,
                have: body.len(),
                what: "emsg v1 integer fields",
            });
        }
        let timescale = read_u32(&body[0..U32_LEN]);
        let presentation_time = read_u64(&body[U32_LEN..U32_LEN + U64_LEN]);
        let mut off = U32_LEN + U64_LEN;
        let event_duration = read_u32(&body[off..off + U32_LEN]);
        off += U32_LEN;
        let id = read_u32(&body[off..off + U32_LEN]);
        off += U32_LEN;

        let (scheme_id_uri, rest) = parse_cstr(&body[off..], "scheme_id_uri")?;
        let (value, message_data) = parse_cstr(rest, "value")?;

        Ok(EmsgBox {
            scheme_id_uri,
            value,
            timescale,
            presentation_time: PresentationTime::Absolute(presentation_time),
            event_duration,
            id,
            message_data,
        })
    }

    /// Serialize the box into `out`, recomputing the `size` field and writing
    /// `flags = 0`. Returns the number of bytes written.
    pub fn serialize_into(&self, out: &mut [u8]) -> Result<usize> {
        let total = self.serialized_len();
        if out.len() < total {
            return Err(Error::OutputBufferTooSmall {
                need: total,
                have: out.len(),
            });
        }
        if total > u32::MAX as usize {
            return Err(Error::FieldTooWide {
                what: "size",
                value: total as u64,
                bits: 32,
            });
        }

        // size u32, type 'emsg', version u8, flags u24.
        out[0..U32_LEN].copy_from_slice(&(total as u32).to_be_bytes());
        out[4..8].copy_from_slice(&EMSG_BOX_TYPE);
        out[8] = self.version().to_u8();
        // flags = 0 (3 bytes).
        out[9] = 0;
        out[10] = 0;
        out[11] = 0;

        let mut off = FULLBOX_HEADER_LEN;
        match self.presentation_time {
            PresentationTime::Delta(delta) => {
                // strings first.
                off = write_cstr(out, off, self.scheme_id_uri);
                off = write_cstr(out, off, self.value);
                off = write_u32(out, off, self.timescale);
                off = write_u32(out, off, delta);
                off = write_u32(out, off, self.event_duration);
                off = write_u32(out, off, self.id);
            }
            PresentationTime::Absolute(pt) => {
                // integers first, strings last.
                off = write_u32(out, off, self.timescale);
                off = write_u64(out, off, pt);
                off = write_u32(out, off, self.event_duration);
                off = write_u32(out, off, self.id);
                off = write_cstr(out, off, self.scheme_id_uri);
                off = write_cstr(out, off, self.value);
            }
        }
        out[off..off + self.message_data.len()].copy_from_slice(self.message_data);
        off += self.message_data.len();
        debug_assert_eq!(off, total);
        Ok(off)
    }

    /// Serialize into a freshly allocated `Vec`.
    pub fn to_vec(&self) -> Result<Vec<u8>> {
        let mut out = alloc::vec![0u8; self.serialized_len()];
        self.serialize_into(&mut out)?;
        Ok(out)
    }
}

/// The SCTE 35 scheme-URI prefix carried in `emsg.scheme_id_uri` (e.g.
/// `urn:scte:scte35:2013:bin`), per SCTE 214-1 / DASH-IF Part 10 §7.3, §9.2.5.
pub const SCTE35_SCHEME_PREFIX: &str = "urn:scte:scte35";

/// Parse a null-terminated UTF-8 string from the front of `data`, returning the
/// decoded `&str` (without the terminator) and the remaining bytes.
fn parse_cstr<'a>(data: &'a [u8], field: &'static str) -> Result<(&'a str, &'a [u8])> {
    let term = data
        .iter()
        .position(|&b| b == STRING_TERMINATOR)
        .ok_or(Error::InvalidString {
            field,
            reason: "missing null terminator",
        })?;
    let s = core::str::from_utf8(&data[..term]).map_err(|_| Error::InvalidString {
        field,
        reason: "invalid UTF-8",
    })?;
    Ok((s, &data[term + 1..]))
}

/// Write `s` followed by a null terminator at `off`; returns the new offset.
fn write_cstr(out: &mut [u8], off: usize, s: &str) -> usize {
    let bytes = s.as_bytes();
    out[off..off + bytes.len()].copy_from_slice(bytes);
    let term = off + bytes.len();
    out[term] = STRING_TERMINATOR;
    term + 1
}

fn read_u32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

fn read_u64(b: &[u8]) -> u64 {
    u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
}

fn write_u32(out: &mut [u8], off: usize, v: u32) -> usize {
    out[off..off + U32_LEN].copy_from_slice(&v.to_be_bytes());
    off + U32_LEN
}

fn write_u64(out: &mut [u8], off: usize, v: u64) -> usize {
    out[off..off + U64_LEN].copy_from_slice(&v.to_be_bytes());
    off + U64_LEN
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    // v0: construct → serialize → exact wire bytes → reparse → equal.
    #[test]
    fn v0_exact_wire_bytes_round_trip() {
        let msg = [0xDEu8, 0xAD, 0xBE, 0xEF];
        let b = EmsgBox {
            scheme_id_uri: "urn:example:scheme",
            value: "",
            timescale: 90_000,
            presentation_time: PresentationTime::Delta(0x0001_0203),
            event_duration: 0xFFFF_FFFF,
            id: 0x0000_002A,
            message_data: &msg,
        };
        assert_eq!(b.version(), EmsgVersion::SegmentRelative);

        let out = b.to_vec().unwrap();

        // Header.
        assert_eq!(&out[0..4], &(out.len() as u32).to_be_bytes());
        assert_eq!(&out[4..8], b"emsg");
        assert_eq!(out[8], 0); // version 0
        assert_eq!(&out[9..12], &[0, 0, 0]); // flags

        // Body: strings first.
        let scheme = b"urn:example:scheme\x00";
        assert_eq!(&out[12..12 + scheme.len()], scheme);
        let mut off = 12 + scheme.len();
        assert_eq!(out[off], 0x00); // empty value terminator
        off += 1;
        assert_eq!(&out[off..off + 4], &90_000u32.to_be_bytes());
        off += 4;
        assert_eq!(&out[off..off + 4], &0x0001_0203u32.to_be_bytes()); // delta
        off += 4;
        assert_eq!(&out[off..off + 4], &0xFFFF_FFFFu32.to_be_bytes());
        off += 4;
        assert_eq!(&out[off..off + 4], &0x0000_002Au32.to_be_bytes());
        off += 4;
        assert_eq!(&out[off..], &msg);

        assert_eq!(EmsgBox::parse(&out).unwrap(), b);
    }

    // v1: construct → serialize → exact wire bytes → reparse → equal.
    #[test]
    fn v1_exact_wire_bytes_round_trip() {
        let msg = [0x01u8, 0x02];
        let b = EmsgBox {
            scheme_id_uri: "urn:scte:scte35:2013:bin",
            value: "1001",
            timescale: 1000,
            presentation_time: PresentationTime::Absolute(0x0102_0304_0506_0708),
            event_duration: 250,
            id: 7,
            message_data: &msg,
        };
        assert_eq!(b.version(), EmsgVersion::RepresentationRelative);
        assert!(b.is_scte35());

        let out = b.to_vec().unwrap();

        assert_eq!(out[8], 1); // version 1
                               // Body: integers first.
        let mut off = 12;
        assert_eq!(&out[off..off + 4], &1000u32.to_be_bytes());
        off += 4;
        assert_eq!(&out[off..off + 8], &0x0102_0304_0506_0708u64.to_be_bytes());
        off += 8;
        assert_eq!(&out[off..off + 4], &250u32.to_be_bytes());
        off += 4;
        assert_eq!(&out[off..off + 4], &7u32.to_be_bytes());
        off += 4;
        // Then strings.
        let scheme = b"urn:scte:scte35:2013:bin\x00";
        assert_eq!(&out[off..off + scheme.len()], scheme);
        off += scheme.len();
        let value = b"1001\x00";
        assert_eq!(&out[off..off + value.len()], value);
        off += value.len();
        assert_eq!(&out[off..], &msg);

        assert_eq!(EmsgBox::parse(&out).unwrap(), b);
    }

    // The v0/v1 field-order difference: the SAME logical fields produce
    // DIFFERENT body byte orderings, yet both round-trip.
    #[test]
    fn v0_v1_field_order_differs_but_both_round_trip() {
        let msg = [0xAAu8, 0xBB];
        let v0 = EmsgBox {
            scheme_id_uri: "s",
            value: "v",
            timescale: 0x1111_1111,
            presentation_time: PresentationTime::Delta(0x2222_2222),
            event_duration: 0x3333_3333,
            id: 0x4444_4444,
            message_data: &msg,
        };
        let v1 = EmsgBox {
            scheme_id_uri: "s",
            value: "v",
            timescale: 0x1111_1111,
            presentation_time: PresentationTime::Absolute(0x2222_2222),
            event_duration: 0x3333_3333,
            id: 0x4444_4444,
            message_data: &msg,
        };

        let o0 = v0.to_vec().unwrap();
        let o1 = v1.to_vec().unwrap();

        // v0 has the strings right after the header; v1 has the timescale.
        assert_eq!(&o0[12..14], b"s\x00");
        assert_eq!(&o1[12..16], &0x1111_1111u32.to_be_bytes());
        // The two encodings of "the same logical fields" are NOT byte-equal.
        assert_ne!(o0, o1);

        // Both round-trip to their originals.
        assert_eq!(EmsgBox::parse(&o0).unwrap(), v0);
        assert_eq!(EmsgBox::parse(&o1).unwrap(), v1);
    }

    // Mutation bite: changing a field changes the wire bytes.
    #[test]
    fn mutating_a_field_changes_wire_bytes() {
        let mk = |id: u32| EmsgBox {
            scheme_id_uri: "urn:x",
            value: "",
            timescale: 48_000,
            presentation_time: PresentationTime::Delta(10),
            event_duration: 20,
            id,
            message_data: &[],
        };
        let a = mk(1).to_vec().unwrap();
        let b = mk(2).to_vec().unwrap();
        assert_ne!(a, b);
        // Same length (id is a fixed field), but differing bytes.
        assert_eq!(a.len(), b.len());
    }

    #[test]
    fn rejects_non_emsg_type() {
        let mut data = vec![0u8; FULLBOX_HEADER_LEN];
        data[0..4].copy_from_slice(&(FULLBOX_HEADER_LEN as u32).to_be_bytes());
        data[4..8].copy_from_slice(b"moof");
        assert!(matches!(EmsgBox::parse(&data), Err(Error::NotEmsg { .. })));
    }

    #[test]
    fn rejects_unsupported_version() {
        let b = EmsgBox {
            scheme_id_uri: "x",
            value: "",
            timescale: 1,
            presentation_time: PresentationTime::Delta(0),
            event_duration: 0,
            id: 0,
            message_data: &[],
        };
        let mut out = b.to_vec().unwrap();
        out[8] = 9; // bad version
        assert!(matches!(
            EmsgBox::parse(&out),
            Err(Error::UnsupportedVersion { version: 9 })
        ));
    }

    #[test]
    fn rejects_size_overrun() {
        let mut data = vec![0u8; FULLBOX_HEADER_LEN];
        data[0..4].copy_from_slice(&0xFFFF_FFFFu32.to_be_bytes()); // huge size
        data[4..8].copy_from_slice(b"emsg");
        assert!(matches!(
            EmsgBox::parse(&data),
            Err(Error::InvalidSize { .. })
        ));
    }

    #[test]
    fn rejects_missing_terminator() {
        // version 0, but the scheme string has no terminator before the box ends.
        let mut data = vec![0u8; 16];
        let size = data.len() as u32;
        data[0..4].copy_from_slice(&size.to_be_bytes());
        data[4..8].copy_from_slice(b"emsg");
        data[8] = 0; // version 0
        data[12..16].copy_from_slice(b"abcd"); // no 0x00
        assert!(matches!(
            EmsgBox::parse(&data),
            Err(Error::InvalidString { .. })
        ));
    }

    #[test]
    fn is_scte35_recognises_prefix() {
        let b = EmsgBox {
            scheme_id_uri: "urn:scte:scte35:2013:bin",
            value: "",
            timescale: 1,
            presentation_time: PresentationTime::Delta(0),
            event_duration: 0,
            id: 0,
            message_data: &[],
        };
        assert!(b.is_scte35());
        let nb = EmsgBox {
            scheme_id_uri: "urn:mpeg:dash:event:2012",
            ..b.clone()
        };
        assert!(!nb.is_scte35());
    }
}
