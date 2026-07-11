//! The RTP fixed header, CSRC list, and generic header extension — RFC 3550
//! §5.1 / §5.3.1. See `rtp-packet/docs/rtp-header.md` for the curated spec
//! transcription this module implements field-for-field.

use alloc::vec::Vec;

use broadcast_common::{Parse, Serialize};

use crate::error::{Error, Result};

// ---------------------------------------------------------------------------
// Named constants (no magic numbers) — RFC 3550 §5.1 / §5.3.1
// ---------------------------------------------------------------------------

/// RTP version — "the version defined by this specification is two (2)"
/// (docs/rtp-header.md §5.1, `version (V), 2 bits`).
pub const RTP_VERSION: u8 = 2;

/// Length of the fixed header (12 bytes: byte0 + byte1 + seq(2) + ts(4) +
/// ssrc(4)) before any CSRC identifiers, per the §5.1 bit diagram.
pub const FIXED_HEADER_LEN: usize = 12;

/// Byte width of one CSRC identifier (§5.1: "32 bits" each).
const CSRC_ITEM_LEN: usize = 4;

/// Maximum CSRC count — the `CC` field is 4 bits (§5.1).
pub const MAX_CSRC_COUNT: usize = 15;

/// `version` field shift within byte 0 (top 2 bits).
const VERSION_SHIFT: u8 = 6;
/// `padding (P)` bit within byte 0 (bit 5).
const PADDING_BIT_MASK: u8 = 0x20;
/// `extension (X)` bit within byte 0 (bit 4).
const EXTENSION_BIT_MASK: u8 = 0x10;
/// `CC` field mask within byte 0 (low 4 bits).
const CC_MASK: u8 = 0x0F;
/// `marker (M)` bit within byte 1 (bit 7).
const MARKER_BIT_MASK: u8 = 0x80;
/// `payload type (PT)` field mask within byte 1 (low 7 bits, §5.1).
const PAYLOAD_TYPE_MASK: u8 = 0x7F;
/// Maximum `payload type` value — `PT` is a 7-bit field (§5.1).
pub const MAX_PAYLOAD_TYPE: u8 = 0x7F;

/// Length of the header-extension prefix — `defined by profile`(16 bits) +
/// `length`(16 bits) = 4 bytes (§5.3.1 bit diagram).
const EXTENSION_HEADER_LEN: usize = 4;
/// Byte width of one extension "word" — `length` counts 32-bit words,
/// "excluding the four-octet extension header" (§5.3.1).
const EXTENSION_WORD_LEN: usize = 4;
/// Maximum extension length in words — `length` is a 16-bit field (§5.3.1).
const MAX_EXTENSION_WORDS: usize = u16::MAX as usize;

/// Maximum padding-octet count — the trailing count byte is 8 bits (§5.1:
/// "the last octet of the padding contains a count of how many padding
/// octets should be ignored, including itself").
pub const MAX_PADDING_COUNT: usize = u8::MAX as usize;

// ---------------------------------------------------------------------------
// HeaderExtension — RFC 3550 §5.3.1
// ---------------------------------------------------------------------------

/// The RTP generic header extension (§5.3.1): a 16-bit profile-specific
/// identifier + opaque profile-specific data.
///
/// The `data` bytes are genuinely opaque at this layer — RFC 3550 itself
/// defines no further structure ("the actual format of the extension is
/// specified by the profile"). This mirrors the project's precedent for
/// spec-opaque payloads (e.g. `smpte2038`'s undecoded ST 291-1
/// checksum/parity): a raw `&[u8]` here is not a "raw-byte API" violation
/// because there is no further spec structure to type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct HeaderExtension<'a> {
    /// `defined by profile` — a 16-bit identifier whose meaning is entirely
    /// profile-specific (§5.3.1).
    pub profile_id: u16,
    /// The extension data. Its on-the-wire `length` (in 32-bit words) is
    /// always derived from `data.len() / 4` on serialize — never stored as an
    /// independent field that could disagree with the slice. `data.len()`
    /// MUST be a multiple of 4 (§5.3.1: `length` counts whole 32-bit words).
    pub data: &'a [u8],
}

impl HeaderExtension<'_> {
    /// The `length` field value that will be written on serialize: the number
    /// of 32-bit words in `data` (§5.3.1, "excluding the four-octet extension
    /// header").
    #[must_use]
    pub fn length_words(&self) -> usize {
        self.data.len() / EXTENSION_WORD_LEN
    }
}

// ---------------------------------------------------------------------------
// RtpPacket — RFC 3550 §5.1 fixed header + CSRC list + extension + payload
// ---------------------------------------------------------------------------

/// A parsed (or to-be-serialized) RTP packet: the §5.1 fixed header, the CSRC
/// list, the optional §5.3.1 header extension, an optional padding region, and
/// the payload.
///
/// `version` is not a stored field: RFC 3550 fixes it at 2, so [`parse`]
/// rejects any other value and [`serialize_into`] always writes 2 — storing a
/// field that can only ever legally hold one value would just be another way
/// for caller state to disagree with the wire (see [`RTP_VERSION`]).
///
/// `P` (padding) and `X` (extension) are likewise never stored directly: they
/// are derived from `padding.is_some()` / `extension.is_some()` on serialize,
/// and `CC` is derived from `csrc.len()` — the same "derive from the typed
/// data, never trust an independent flag" discipline used throughout this
/// project (see the module doc's citation of docs/rtp-header.md).
///
/// [`parse`]: Parse::parse
/// [`serialize_into`]: Serialize::serialize_into
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct RtpPacket<'a> {
    /// `marker (M)` — profile-specific, opaque at this layer (§5.1).
    pub marker: bool,
    /// `payload type (PT)`, 7 bits (§5.1).
    pub payload_type: u8,
    /// `sequence number`, 16 bits (§5.1).
    pub sequence_number: u16,
    /// `timestamp`, 32 bits (§5.1).
    pub timestamp: u32,
    /// `SSRC` — synchronization source identifier, 32 bits (§5.1).
    pub ssrc: u32,
    /// The CSRC identifier list, 0–15 entries (§5.1 `CC`/CSRC list). `CC` is
    /// always `csrc.len()` — there is no separate stored count.
    pub csrc: Vec<u32>,
    /// The §5.3.1 header extension, if `X=1`.
    pub extension: Option<HeaderExtension<'a>>,
    /// The trailing padding region, if `P=1`: the raw octets as they appear on
    /// the wire, whose **last byte** is the padding count "including itself"
    /// (§5.1). Storing the whole slice (rather than just a count) preserves
    /// whatever content precedes the count byte byte-exactly on round-trip,
    /// since RFC 3550 does not constrain it.
    pub padding: Option<&'a [u8]>,
    /// The payload: whatever bytes remain after the fixed header, CSRC list,
    /// and extension, with the trailing `padding` (if any) already excluded.
    pub payload: &'a [u8],
}

impl RtpPacket<'_> {
    /// `CC` — the CSRC count that will be written on serialize: always
    /// `csrc.len()` (§5.1).
    #[must_use]
    pub fn csrc_count(&self) -> usize {
        self.csrc.len()
    }
}

impl<'a> Parse<'a> for RtpPacket<'a> {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < FIXED_HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: FIXED_HEADER_LEN,
                have: bytes.len(),
                what: "RTP fixed header",
            });
        }

        let byte0 = bytes[0];
        let version = byte0 >> VERSION_SHIFT;
        if version != RTP_VERSION {
            return Err(Error::InvalidVersion(version));
        }
        let padding_flag = byte0 & PADDING_BIT_MASK != 0;
        let extension_flag = byte0 & EXTENSION_BIT_MASK != 0;
        let cc = usize::from(byte0 & CC_MASK);

        let byte1 = bytes[1];
        let marker = byte1 & MARKER_BIT_MASK != 0;
        let payload_type = byte1 & PAYLOAD_TYPE_MASK;

        let sequence_number = u16::from_be_bytes([bytes[2], bytes[3]]);
        let timestamp = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let ssrc = u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);

        let mut pos = FIXED_HEADER_LEN;

        // --- CSRC list (§5.1) ---
        let csrc_bytes_len = cc * CSRC_ITEM_LEN;
        if bytes.len() < pos + csrc_bytes_len {
            return Err(Error::BufferTooShort {
                need: pos + csrc_bytes_len,
                have: bytes.len(),
                what: "CSRC list",
            });
        }
        let mut csrc = Vec::with_capacity(cc);
        for i in 0..cc {
            let off = pos + i * CSRC_ITEM_LEN;
            csrc.push(u32::from_be_bytes([
                bytes[off],
                bytes[off + 1],
                bytes[off + 2],
                bytes[off + 3],
            ]));
        }
        pos += csrc_bytes_len;

        // --- Header extension (§5.3.1) ---
        let extension = if extension_flag {
            if bytes.len() < pos + EXTENSION_HEADER_LEN {
                return Err(Error::BufferTooShort {
                    need: pos + EXTENSION_HEADER_LEN,
                    have: bytes.len(),
                    what: "header extension profile-id/length",
                });
            }
            let profile_id = u16::from_be_bytes([bytes[pos], bytes[pos + 1]]);
            let length_words = usize::from(u16::from_be_bytes([bytes[pos + 2], bytes[pos + 3]]));
            let data_len = length_words * EXTENSION_WORD_LEN;
            let data_start = pos + EXTENSION_HEADER_LEN;
            let data_end = data_start + data_len;
            if bytes.len() < data_end {
                return Err(Error::BufferTooShort {
                    need: data_end,
                    have: bytes.len(),
                    what: "header extension data",
                });
            }
            pos = data_end;
            Some(HeaderExtension {
                profile_id,
                data: &bytes[data_start..data_end],
            })
        } else {
            None
        };

        // --- Padding (§5.1) + payload ---
        let (payload, padding) = if padding_flag {
            if bytes.len() <= pos {
                return Err(Error::InvalidPadding {
                    count: 0,
                    reason: "padding bit set but no bytes remain for the count octet",
                });
            }
            // bytes.len() > pos (checked above) implies bytes.len() >= 1.
            let count = bytes[bytes.len() - 1];
            if count == 0 {
                return Err(Error::InvalidPadding {
                    count,
                    reason: "padding count must be >= 1 (it counts itself)",
                });
            }
            let remaining = bytes.len() - pos;
            if usize::from(count) > remaining {
                return Err(Error::InvalidPadding {
                    count,
                    reason: "padding count exceeds the bytes remaining after the header",
                });
            }
            let split = bytes.len() - usize::from(count);
            (&bytes[pos..split], Some(&bytes[split..]))
        } else {
            (&bytes[pos..], None)
        };

        Ok(Self {
            marker,
            payload_type,
            sequence_number,
            timestamp,
            ssrc,
            csrc,
            extension,
            padding,
            payload,
        })
    }
}

impl Serialize for RtpPacket<'_> {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        FIXED_HEADER_LEN
            + self.csrc.len() * CSRC_ITEM_LEN
            + self
                .extension
                .map(|e| EXTENSION_HEADER_LEN + e.data.len())
                .unwrap_or(0)
            + self.payload.len()
            + self.padding.map(<[u8]>::len).unwrap_or(0)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::BufferTooShort {
                need: len,
                have: buf.len(),
                what: "RTP packet serialize output",
            });
        }
        if self.csrc.len() > MAX_CSRC_COUNT {
            return Err(Error::InvalidValue {
                field: "csrc.len()",
                value: self.csrc.len() as u64,
                reason: "exceeds the 4-bit CC field maximum (15)",
            });
        }
        if self.payload_type > MAX_PAYLOAD_TYPE {
            return Err(Error::InvalidValue {
                field: "payload_type",
                value: u64::from(self.payload_type),
                reason: "exceeds the 7-bit PT field maximum (127)",
            });
        }
        if let Some(ext) = &self.extension {
            if ext.data.len() % EXTENSION_WORD_LEN != 0 {
                return Err(Error::ExtensionNotWordAligned {
                    data_len: ext.data.len(),
                });
            }
            let words = ext.length_words();
            if words > MAX_EXTENSION_WORDS {
                return Err(Error::InvalidValue {
                    field: "extension.data.len()/4",
                    value: words as u64,
                    reason: "exceeds the 16-bit length field maximum",
                });
            }
        }
        if let Some(pad) = self.padding {
            if pad.is_empty() {
                return Err(Error::InvalidPadding {
                    count: 0,
                    reason: "a Some(padding) slice must be non-empty",
                });
            }
            if pad.len() > MAX_PADDING_COUNT {
                return Err(Error::InvalidValue {
                    field: "padding.len()",
                    value: pad.len() as u64,
                    reason: "exceeds the 8-bit padding-count field maximum (255)",
                });
            }
            // pad.is_empty() was rejected above, so pad.len() >= 1.
            let last = pad[pad.len() - 1];
            if usize::from(last) != pad.len() {
                return Err(Error::InvalidPadding {
                    count: last,
                    reason: "the last padding byte must equal the padding slice's own length",
                });
            }
        }

        let mut byte0 = RTP_VERSION << VERSION_SHIFT;
        if self.padding.is_some() {
            byte0 |= PADDING_BIT_MASK;
        }
        if self.extension.is_some() {
            byte0 |= EXTENSION_BIT_MASK;
        }
        byte0 |= (self.csrc.len() as u8) & CC_MASK;
        buf[0] = byte0;

        let mut byte1 = self.payload_type & PAYLOAD_TYPE_MASK;
        if self.marker {
            byte1 |= MARKER_BIT_MASK;
        }
        buf[1] = byte1;

        buf[2..4].copy_from_slice(&self.sequence_number.to_be_bytes());
        buf[4..8].copy_from_slice(&self.timestamp.to_be_bytes());
        buf[8..12].copy_from_slice(&self.ssrc.to_be_bytes());

        let mut pos = FIXED_HEADER_LEN;
        for &c in &self.csrc {
            buf[pos..pos + CSRC_ITEM_LEN].copy_from_slice(&c.to_be_bytes());
            pos += CSRC_ITEM_LEN;
        }

        if let Some(ext) = &self.extension {
            buf[pos..pos + 2].copy_from_slice(&ext.profile_id.to_be_bytes());
            let words = ext.length_words() as u16;
            buf[pos + 2..pos + EXTENSION_HEADER_LEN].copy_from_slice(&words.to_be_bytes());
            pos += EXTENSION_HEADER_LEN;
            buf[pos..pos + ext.data.len()].copy_from_slice(ext.data);
            pos += ext.data.len();
        }

        buf[pos..pos + self.payload.len()].copy_from_slice(self.payload);
        pos += self.payload.len();

        if let Some(pad) = self.padding {
            buf[pos..pos + pad.len()].copy_from_slice(pad);
            pos += pad.len();
        }

        Ok(pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn simple_packet() -> RtpPacket<'static> {
        RtpPacket {
            marker: true,
            payload_type: 97,
            sequence_number: 5,
            timestamp: 0x0000_1400,
            ssrc: 0x1234_5678,
            csrc: Vec::new(),
            extension: None,
            padding: None,
            payload: &[0xAA, 0xBB, 0xCC, 0xDD],
        }
    }

    #[test]
    fn simple_round_trip() {
        let p = simple_packet();
        let mut out = vec![0u8; p.serialized_len()];
        p.serialize_into(&mut out).unwrap();
        assert_eq!(RtpPacket::parse(&out).unwrap(), p);
        assert_eq!(out[0], 0x80); // V=2 P=0 X=0 CC=0
        assert_eq!(out[1], 0x80 | 97); // M=1 PT=97
    }

    #[test]
    fn rejects_bad_version() {
        let p = simple_packet();
        let mut out = vec![0u8; p.serialized_len()];
        p.serialize_into(&mut out).unwrap();
        out[0] = (1 << VERSION_SHIFT) | (out[0] & 0x3F); // version = 1
        assert!(matches!(
            RtpPacket::parse(&out),
            Err(Error::InvalidVersion(1))
        ));
    }

    #[test]
    fn csrc_round_trip() {
        let mut p = simple_packet();
        p.csrc = vec![0x1111_1111, 0x2222_2222, 0x3333_3333];
        let mut out = vec![0u8; p.serialized_len()];
        p.serialize_into(&mut out).unwrap();
        assert_eq!(out[0] & 0x0F, 3, "CC derived from csrc.len()");
        let reparsed = RtpPacket::parse(&out).unwrap();
        assert_eq!(reparsed, p);
    }

    #[test]
    fn rejects_csrc_over_15() {
        let mut p = simple_packet();
        p.csrc = vec![0; 16];
        let mut out = vec![0u8; p.serialized_len()];
        assert!(matches!(
            p.serialize_into(&mut out),
            Err(Error::InvalidValue {
                field: "csrc.len()",
                ..
            })
        ));
    }

    #[test]
    fn extension_round_trip() {
        let mut p = simple_packet();
        p.extension = Some(HeaderExtension {
            profile_id: 0xBEDE,
            data: &[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08],
        });
        let mut out = vec![0u8; p.serialized_len()];
        p.serialize_into(&mut out).unwrap();
        assert_eq!(out[0] & 0x10, 0x10, "X bit set");
        let reparsed = RtpPacket::parse(&out).unwrap();
        assert_eq!(reparsed, p);
        assert_eq!(reparsed.extension.unwrap().length_words(), 2);
    }

    #[test]
    fn extension_zero_length_is_valid() {
        // §5.3.1: "therefore zero is a valid length".
        let mut p = simple_packet();
        p.extension = Some(HeaderExtension {
            profile_id: 0x0001,
            data: &[],
        });
        let mut out = vec![0u8; p.serialized_len()];
        p.serialize_into(&mut out).unwrap();
        let reparsed = RtpPacket::parse(&out).unwrap();
        assert_eq!(reparsed, p);
    }

    #[test]
    fn rejects_extension_not_word_aligned() {
        let mut p = simple_packet();
        p.extension = Some(HeaderExtension {
            profile_id: 1,
            data: &[0x01, 0x02, 0x03], // 3 bytes, not a multiple of 4
        });
        let mut out = vec![0u8; FIXED_HEADER_LEN + EXTENSION_HEADER_LEN + 3 + p.payload.len()];
        assert!(matches!(
            p.serialize_into(&mut out),
            Err(Error::ExtensionNotWordAligned { data_len: 3 })
        ));
    }

    #[test]
    fn padding_round_trip() {
        let mut p = simple_packet();
        // 4 pad octets; the last byte (the count, "including itself") is 4.
        p.padding = Some(&[0x00, 0x00, 0x00, 0x04]);
        let mut out = vec![0u8; p.serialized_len()];
        p.serialize_into(&mut out).unwrap();
        assert_eq!(out[0] & 0x20, 0x20, "P bit set");
        assert_eq!(*out.last().unwrap(), 4, "trailing pad-count byte");
        let reparsed = RtpPacket::parse(&out).unwrap();
        assert_eq!(reparsed, p);
        assert_eq!(reparsed.payload, p.payload, "payload stripped of padding");
    }

    #[test]
    fn rejects_padding_count_mismatch() {
        let mut p = simple_packet();
        p.padding = Some(&[0x00, 0x00, 0x00, 0x03]); // len=4 but last byte says 3
        let mut out = vec![0u8; p.serialized_len()];
        assert!(matches!(
            p.serialize_into(&mut out),
            Err(Error::InvalidPadding { count: 3, .. })
        ));
    }

    #[test]
    fn rejects_padding_count_exceeding_available() {
        // Hand-build a packet with P=1 and a count byte bigger than the bytes
        // actually present after the fixed header.
        let mut bytes = vec![0u8; FIXED_HEADER_LEN + 2];
        bytes[0] = 0x80 | 0x20; // V=2 P=1
        bytes[1] = 97;
        *bytes.last_mut().unwrap() = 0xFF; // count says 255, only 2 bytes present
        assert!(matches!(
            RtpPacket::parse(&bytes),
            Err(Error::InvalidPadding { count: 0xFF, .. })
        ));
    }

    #[test]
    fn field_mutation_changes_bytes() {
        let a = simple_packet();
        let mut b = a.clone();
        b.sequence_number = a.sequence_number.wrapping_add(1);
        let mut oa = vec![0u8; a.serialized_len()];
        let mut ob = vec![0u8; b.serialized_len()];
        a.serialize_into(&mut oa).unwrap();
        b.serialize_into(&mut ob).unwrap();
        assert_ne!(oa, ob);
        // The change is localized to exactly the 2-byte sequence-number field.
        assert_eq!(oa[0], ob[0]);
        assert_eq!(oa[1], ob[1]);
        assert_ne!(&oa[2..4], &ob[2..4]);
        assert_eq!(&oa[4..], &ob[4..]);
    }
}
