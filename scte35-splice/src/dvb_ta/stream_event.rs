//! DSM-CC_stream_event_payload_binary() — ETSI TS 103 752-1 V1.2.1 §6.3.1,
//! Table 3 + Table 4 (PDF pp.18–20).
//!
//! **NEW binary syntax** (DVB Targeted Advertising Part 1). For *distribution*
//! signalling a full SCTE 35 message section may be carried inside a DSM-CC
//! stream event (rather than directly on a TS PID). The payload is built in this
//! binary form, then base-64 encoded (IETF RFC 4648) and inserted as the private
//! data of a "do it now" stream-event descriptor per ETSI TS 102 809.
//!
//! Two carriage modes (selected by `event_type`):
//! - `event_type == 0` — the SCTE 35 section is conveyed **directly** inline.
//! - `event_type == 1` — the payload **references** a DSM-CC object-carousel file
//!   (by DVB URI) that contains the SCTE 35 section (for larger sections).
//!
//! ## Bit-width sourcing
//!
//! Following the transcription's verified reading of the mis-registered render:
//! `private_data_specifier` is **32 bits** (the `private_data_length-4` byte loop
//! consumes 4 bytes for it first). See `docs/dvb_ta/dsmcc-stream-event.md`.

use alloc::vec::Vec;

use crate::error::{Error, Result};
use crate::section::SpliceInfoSection;
use broadcast_common::{Parse, Serialize};

/// `event_type` value for inline carriage (`SCTE_35_section()` follows).
pub const EVENT_TYPE_INLINE: u8 = 0;

/// `event_type` value for carousel-object reference.
pub const EVENT_TYPE_REFERENCE: u8 = 1;

/// `timeline_type` value selecting a TEMI timeline; gates the two `temi_*` bytes.
pub const TIMELINE_TYPE_TEMI: u8 = 0x2;

/// `timeline_type` — §6.3.1, Table 4 (4 bits).
///
/// Identifies the timeline that PTS values in the SCTE 35 section reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum TimelineType {
    /// `0x0` — no timeline used.
    None,
    /// `0x1` — PTS in the SCTE 35 message references video PTS.
    VideoPts,
    /// `0x2` — PTS references the time in a TEMI timeline associated with the
    /// service.
    Temi,
    /// `0x3`–`0xF` — reserved for future use, carried verbatim.
    Reserved(u8),
}

impl TimelineType {
    /// Decode the 4-bit `timeline_type` field (only the low nibble is used).
    #[must_use]
    pub fn from_bits(v: u8) -> Self {
        match v & 0x0F {
            0x0 => Self::None,
            0x1 => Self::VideoPts,
            0x2 => Self::Temi,
            other => Self::Reserved(other),
        }
    }

    /// The 4-bit wire value (low nibble).
    #[must_use]
    pub const fn bits(self) -> u8 {
        match self {
            Self::None => 0x0,
            Self::VideoPts => 0x1,
            Self::Temi => 0x2,
            Self::Reserved(v) => v & 0x0F,
        }
    }

    /// Human-readable spec label (ETSI TS 103 752-1 Table 4).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::None => "no timeline used",
            Self::VideoPts => "PTS references video PTS",
            Self::Temi => "PTS references TEMI timeline",
            Self::Reserved(_) => "reserved",
        }
    }
}
broadcast_common::impl_spec_display!(TimelineType, Reserved);

/// The SCTE 35 carriage body of a stream-event payload (`event_type`-selected).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
#[non_exhaustive]
pub enum Scte35Carriage<'a> {
    /// `event_type == 0` — the SCTE 35 section conveyed directly inline.
    #[cfg_attr(feature = "serde", serde(borrow))]
    Inline(SpliceInfoSection<'a>),
    /// `event_type == 1` — a DVB URI naming the carousel object that contains the
    /// SCTE 35 section.
    CarouselObject(&'a [u8]),
}

impl Scte35Carriage<'_> {
    /// The `event_type` byte for this carriage mode.
    #[must_use]
    pub const fn event_type(&self) -> u8 {
        match self {
            Self::Inline(_) => EVENT_TYPE_INLINE,
            Self::CarouselObject(_) => EVENT_TYPE_REFERENCE,
        }
    }
}

/// `DSM-CC_stream_event_payload_binary()` — §6.3.1, Table 3.
///
/// The binary payload (before base-64 encoding) carried in a "do it now" DSM-CC
/// stream-event descriptor. `DVB_data_length` is recomputed on serialize, so it
/// is not stored.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct StreamEventPayload<'a> {
    /// `timeline_type` (4 bits, Table 4).
    pub timeline_type: TimelineType,
    /// `(temi_component_tag, temi_timeline_id)` — present iff `timeline_type ==
    /// 0x2` (§6.3.1).
    pub temi: Option<(u8, u8)>,
    /// Optional private data: `(private_data_specifier, private_data_byte[])`.
    /// `None` ⇒ `private_data_length == 0`.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub private_data: Option<PrivateData<'a>>,
    /// The SCTE 35 carriage (inline section or carousel-object reference).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub carriage: Scte35Carriage<'a>,
}

/// Private-data block of a [`StreamEventPayload`] (§6.3.1): a 32-bit
/// `private_data_specifier` (ETSI TS 101 162) followed by `private_data_byte`s.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PrivateData<'a> {
    /// `private_data_specifier` (32 bits, ETSI TS 101 162).
    pub specifier: u32,
    /// `private_data_byte` sequence (`private_data_length - 4` bytes).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub bytes: &'a [u8],
}

impl StreamEventPayload<'_> {
    /// Length in bytes of the region counted by `DVB_data_length`: the fields
    /// after `DVB_data_length` up to (but excluding) `private_data_length`.
    fn dvb_data_len(&self) -> usize {
        // reserved+event_type+timeline_type byte (1) + optional temi (2) +
        // reserved_zero_future_use run (0 — we emit none).
        1 + if self.temi.is_some() { 2 } else { 0 }
    }

    /// Length of the `private_data_length` field's value (0 or `4 + bytes`).
    fn private_data_field_len(&self) -> usize {
        match &self.private_data {
            Some(p) => 4 + p.bytes.len(),
            None => 0,
        }
    }
}

impl<'a> Parse<'a> for StreamEventPayload<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        // DVB_data_length(8) + [reserved(3)+event_type(1)+timeline_type(4)](8) ...
        if bytes.len() < 2 {
            return Err(Error::BufferTooShort {
                need: 2,
                have: bytes.len(),
                what: "DSM-CC_stream_event_payload_binary header",
            });
        }
        let dvb_data_length = bytes[0] as usize;
        // The DVB_data region begins at byte 1 and is dvb_data_length bytes long.
        if bytes.len() < 1 + dvb_data_length {
            return Err(Error::LengthOverflow {
                declared: dvb_data_length,
                available: bytes.len().saturating_sub(1),
                what: "DSM-CC_stream_event_payload_binary DVB_data_length",
            });
        }
        if dvb_data_length < 1 {
            return Err(Error::BufferTooShort {
                need: 1,
                have: dvb_data_length,
                what: "DSM-CC_stream_event_payload_binary flags byte",
            });
        }
        let flags = bytes[1];
        let event_type = (flags >> 4) & 0x01;
        let timeline_type = TimelineType::from_bits(flags & 0x0F);
        let temi = if timeline_type == TimelineType::Temi {
            if dvb_data_length < 3 || bytes.len() < 4 {
                return Err(Error::BufferTooShort {
                    need: 4,
                    have: bytes.len(),
                    what: "DSM-CC_stream_event_payload_binary temi fields",
                });
            }
            Some((bytes[2], bytes[3]))
        } else {
            None
        };
        // Any remaining DVB_data bytes after the flags byte (and optional temi)
        // are reserved_zero_future_use; skip to the end of the DVB_data region.
        let mut pos = 1 + dvb_data_length;

        // private_data_length(8).
        if bytes.len() <= pos {
            return Err(Error::BufferTooShort {
                need: pos + 1,
                have: bytes.len(),
                what: "DSM-CC_stream_event_payload_binary private_data_length",
            });
        }
        let private_data_length = bytes[pos] as usize;
        pos += 1;
        let private_data = if private_data_length > 0 {
            if private_data_length < 4 {
                return Err(Error::InvalidValue {
                    field: "DSM-CC_stream_event_payload_binary.private_data_length",
                    reason: "must be >= 4 (32-bit private_data_specifier) when present",
                });
            }
            if bytes.len() < pos + private_data_length {
                return Err(Error::LengthOverflow {
                    declared: private_data_length,
                    available: bytes.len().saturating_sub(pos),
                    what: "DSM-CC_stream_event_payload_binary private_data",
                });
            }
            let specifier =
                u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]]);
            let pd_bytes = &bytes[pos + 4..pos + private_data_length];
            pos += private_data_length;
            Some(PrivateData {
                specifier,
                bytes: pd_bytes,
            })
        } else {
            None
        };

        let carriage = if event_type == EVENT_TYPE_REFERENCE {
            // carousel_object_name_length(8) + chars.
            if bytes.len() <= pos {
                return Err(Error::BufferTooShort {
                    need: pos + 1,
                    have: bytes.len(),
                    what: "DSM-CC_stream_event_payload_binary carousel_object_name_length",
                });
            }
            let name_len = bytes[pos] as usize;
            pos += 1;
            if bytes.len() < pos + name_len {
                return Err(Error::LengthOverflow {
                    declared: name_len,
                    available: bytes.len().saturating_sub(pos),
                    what: "DSM-CC_stream_event_payload_binary carousel_object_name",
                });
            }
            Scte35Carriage::CarouselObject(&bytes[pos..pos + name_len])
        } else {
            // The remainder is the SCTE_35_section().
            Scte35Carriage::Inline(SpliceInfoSection::parse(&bytes[pos..])?)
        };

        Ok(Self {
            timeline_type,
            temi,
            private_data,
            carriage,
        })
    }
}

impl Serialize for StreamEventPayload<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = 1 /* DVB_data_length */ + self.dvb_data_len();
        n += 1; // private_data_length field
        n += self.private_data_field_len();
        match &self.carriage {
            Scte35Carriage::Inline(s) => n += s.serialized_len(),
            Scte35Carriage::CarouselObject(name) => n += 1 + name.len(),
        }
        n
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let dvb_data_len = self.dvb_data_len();
        if dvb_data_len > u8::MAX as usize {
            return Err(Error::InvalidValue {
                field: "DSM-CC_stream_event_payload_binary.DVB_data_length",
                reason: "exceeds 8-bit field",
            });
        }
        buf[0] = dvb_data_len as u8;
        // reserved(3)=1, event_type(1), timeline_type(4).
        buf[1] = 0xE0 | (self.carriage.event_type() << 4) | self.timeline_type.bits();
        let mut pos = 2;
        if let Some((tag, id)) = self.temi {
            buf[pos] = tag;
            buf[pos + 1] = id;
            pos += 2;
        }
        debug_assert_eq!(pos, 1 + dvb_data_len);

        let pdl = self.private_data_field_len();
        if pdl > u8::MAX as usize {
            return Err(Error::InvalidValue {
                field: "DSM-CC_stream_event_payload_binary.private_data_length",
                reason: "exceeds 8-bit field",
            });
        }
        buf[pos] = pdl as u8;
        pos += 1;
        if let Some(p) = &self.private_data {
            buf[pos..pos + 4].copy_from_slice(&p.specifier.to_be_bytes());
            pos += 4;
            buf[pos..pos + p.bytes.len()].copy_from_slice(p.bytes);
            pos += p.bytes.len();
        }

        match &self.carriage {
            Scte35Carriage::Inline(s) => {
                let w = s.serialize_into(&mut buf[pos..])?;
                pos += w;
            }
            Scte35Carriage::CarouselObject(name) => {
                if name.len() > u8::MAX as usize {
                    return Err(Error::InvalidValue {
                        field: "DSM-CC_stream_event_payload_binary.carousel_object_name_length",
                        reason: "exceeds 8-bit field",
                    });
                }
                buf[pos] = name.len() as u8;
                pos += 1;
                buf[pos..pos + name.len()].copy_from_slice(name);
                pos += name.len();
            }
        }
        debug_assert_eq!(pos, need);
        Ok(need)
    }
}

/// Base-64 encode a serialized [`StreamEventPayload`] for insertion as the
/// `privateDataByte` of a "do it now" stream-event descriptor (§6.3.1, RFC 4648).
///
/// This is the standard RFC 4648 base-64 alphabet with `=` padding. Provided as a
/// `no_std`/`alloc` convenience so callers do not pull a base-64 dependency just
/// to follow the carriage recipe.
#[must_use]
pub fn base64_encode(data: &[u8]) -> Vec<u8> {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        out.push(ALPHABET[(b0 >> 2) as usize]);
        out.push(ALPHABET[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize]);
        if chunk.len() > 1 {
            out.push(ALPHABET[(((b1 & 0x0F) << 2) | (b2 >> 6)) as usize]);
        } else {
            out.push(b'=');
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(b2 & 0x3F) as usize]);
        } else {
            out.push(b'=');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{AnyCommand, TimeSignal};
    use crate::time::SpliceTime;

    fn sample_section() -> Vec<u8> {
        let ts = TimeSignal {
            splice_time: SpliceTime::with_pts(0x0012_3456),
        };
        SpliceInfoSection::new_clear(AnyCommand::TimeSignal(ts), &[]).to_bytes()
    }

    #[test]
    fn inline_round_trip() {
        let section_bytes = sample_section();
        let section = SpliceInfoSection::parse(&section_bytes).unwrap();
        let payload = StreamEventPayload {
            timeline_type: TimelineType::VideoPts,
            temi: None,
            private_data: None,
            carriage: Scte35Carriage::Inline(section),
        };
        let bytes = payload.to_bytes();
        // DVB_data_length = 1 (just the flags byte, no temi).
        assert_eq!(bytes[0], 1);
        // flags: reserved 0xE0 | event_type 0 | timeline 0x1 = 0xE1.
        assert_eq!(bytes[1], 0xE1);
        // private_data_length = 0.
        assert_eq!(bytes[2], 0);
        // then the section (table_id 0xFC).
        assert_eq!(bytes[3], 0xFC);
        let back = StreamEventPayload::parse(&bytes).unwrap();
        assert_eq!(payload, back);
        assert_eq!(back.to_bytes(), bytes);
    }

    #[test]
    fn temi_and_private_data_round_trip() {
        let section_bytes = sample_section();
        let section = SpliceInfoSection::parse(&section_bytes).unwrap();
        let pd = [0xAA, 0xBB];
        let payload = StreamEventPayload {
            timeline_type: TimelineType::Temi,
            temi: Some((0x07, 0x10)),
            private_data: Some(PrivateData {
                specifier: 0x0000_0028, // a TS 101 162 PDS value
                bytes: &pd,
            }),
            carriage: Scte35Carriage::Inline(section),
        };
        let bytes = payload.to_bytes();
        // DVB_data_length = 1 + 2 (temi) = 3.
        assert_eq!(bytes[0], 3);
        // flags: 0xE0 | timeline 0x2 = 0xE2.
        assert_eq!(bytes[1], 0xE2);
        assert_eq!(bytes[2], 0x07); // temi_component_tag
        assert_eq!(bytes[3], 0x10); // temi_timeline_id
        // private_data_length = 4 + 2 = 6.
        assert_eq!(bytes[4], 6);
        assert_eq!(&bytes[5..9], &[0x00, 0x00, 0x00, 0x28]);
        assert_eq!(&bytes[9..11], &pd);
        let back = StreamEventPayload::parse(&bytes).unwrap();
        assert_eq!(payload, back);
        assert_eq!(back.to_bytes(), bytes);
    }

    #[test]
    fn carousel_reference_round_trip() {
        let name = b"dvb://1.2.3/MyCarouselObject";
        let payload = StreamEventPayload {
            timeline_type: TimelineType::None,
            temi: None,
            private_data: None,
            carriage: Scte35Carriage::CarouselObject(name),
        };
        let bytes = payload.to_bytes();
        // flags: event_type 1 -> 0xE0 | 0x10 = 0xF0.
        assert_eq!(bytes[1], 0xF0);
        // private_data_length = 0; then name_len, then name.
        assert_eq!(bytes[2], 0);
        assert_eq!(bytes[3] as usize, name.len());
        assert_eq!(&bytes[4..], &name[..]);
        let back = StreamEventPayload::parse(&bytes).unwrap();
        assert_eq!(payload, back);
        assert_eq!(back.to_bytes(), bytes);
    }

    #[test]
    fn field_mutation_bites() {
        let name = b"dvb://1/x";
        let a = StreamEventPayload {
            timeline_type: TimelineType::None,
            temi: None,
            private_data: None,
            carriage: Scte35Carriage::CarouselObject(name),
        };
        let b = StreamEventPayload {
            timeline_type: TimelineType::VideoPts,
            ..a.clone()
        };
        assert_ne!(a.to_bytes(), b.to_bytes());
    }

    #[test]
    fn base64_matches_rfc4648_vectors() {
        // RFC 4648 §10 test vectors.
        assert_eq!(base64_encode(b""), b"");
        assert_eq!(base64_encode(b"f"), b"Zg==");
        assert_eq!(base64_encode(b"fo"), b"Zm8=");
        assert_eq!(base64_encode(b"foo"), b"Zm9v");
        assert_eq!(base64_encode(b"foob"), b"Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), b"Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), b"Zm9vYmFy");
    }
}
