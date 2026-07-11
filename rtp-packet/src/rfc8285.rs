//! RFC 8285 one-byte/two-byte RTP header-extension element multiplexing.
//!
//! This is a profile-specific interpretation of the RFC 3550 §5.3.1 opaque
//! [`HeaderExtension`] `data` — see `rtp-packet/docs/rfc8285_header_ext.md`
//! for the curated spec transcription this module implements field-for-field
//! (cite that file, not this doc comment, as the field-semantics oracle).
//!
//! Entry point: [`parse_extensions`], given a [`HeaderExtension`] borrowed
//! out of a parsed [`RtpPacket`](crate::RtpPacket), inspects `profile_id` and
//! dispatches to the one-byte ([`OneByteElements`]) or two-byte
//! ([`TwoByteElements`]) form, returning [`Error::NotRfc8285Extension`] for
//! any other `profile_id` (not a malformed-packet error — RFC 8285
//! interpretation is opt-in and profile-scoped).

use alloc::vec::Vec;

use broadcast_common::{Parse, Serialize};

use crate::error::{Error, Result};
use crate::header::HeaderExtension;

// ---------------------------------------------------------------------------
// Named constants (no magic numbers) — RFC 8285 §4.1.2 / §4.2 / §4.3
// ---------------------------------------------------------------------------

/// The fixed `profile_id` bit pattern identifying the one-byte header form
/// (§4.2: "MUST have the fixed bit pattern `0xBEDE`").
pub const ONE_BYTE_PROFILE_ID: u16 = 0xBEDE;

/// Mask isolating the top 12 bits of `profile_id` that identify the two-byte
/// header form (§4.3 bit diagram: `0x100` in the top 12 bits, `appbits` in
/// the bottom 4).
const TWO_BYTE_PROFILE_ID_MASK: u16 = 0xFFF0;
/// The fixed top-12-bit pattern (`0x100`, shifted into position) identifying
/// the two-byte header form (§4.3).
const TWO_BYTE_PROFILE_ID_PREFIX: u16 = 0x1000;

/// The reserved one-byte-form local identifier that halts extension parsing
/// (§4.1.2/§4.2: "the reserved value of 15").
const ONE_BYTE_STOP_ID: u8 = 15;
/// A literal zero byte is always a single padding byte in the byte-by-byte
/// scan (§4.1.2: "padding bytes have the value of 0 (zero)").
const PADDING_BYTE: u8 = 0x00;

/// Minimum valid one-byte-form local identifier (§4.2: "range 1-14
/// inclusive"; 0 is reserved for padding).
const ONE_BYTE_ID_MIN: u8 = 1;
/// Maximum valid one-byte-form local identifier (§4.2; 15 is reserved).
const ONE_BYTE_ID_MAX: u8 = 14;
/// Minimum one-byte-form element data length in bytes (§4.2: the `len`
/// nibble encodes `data.len() - 1`, so the shortest representable element is
/// 1 byte).
const ONE_BYTE_DATA_MIN: usize = 1;
/// Maximum one-byte-form element data length in bytes (§4.2: `len` nibble
/// `15` -> 16 bytes).
const ONE_BYTE_DATA_MAX: usize = 16;

/// Minimum valid two-byte-form local identifier. Per RFC 8285 §4.1.2/§5,
/// "0 is reserved for padding in **both** forms" (not just the one-byte
/// form, despite §4.3 in isolation reading as "range 1-255 inclusive" —
/// see `docs/rfc8285_header_ext.md`'s "Judgment calls" section).
const TWO_BYTE_ID_MIN: u8 = 1;
/// Maximum two-byte-form element data length in bytes (§4.3: an 8-bit
/// length field, stored directly with no `-1` bias, unlike the one-byte
/// form).
const TWO_BYTE_DATA_MAX: usize = u8::MAX as usize;

/// The alignment (in bytes) the overall RFC 3550 §5.3.1 extension `data`
/// must be padded to (a whole number of 32-bit words).
const EXT_ALIGN: usize = 4;

// ---------------------------------------------------------------------------
// OneByteId / TwoByteId — validating local-identifier newtypes
// ---------------------------------------------------------------------------

/// A validated RFC 8285 one-byte-form local identifier (§4.2: range `1..=14`
/// inclusive). `0` (padding) and `15` (reserved/"stop") are not representable
/// — [`OneByteId::new`] rejects them, so an [`OneByteElement`] can never hold
/// a reserved ID value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct OneByteId(u8);

impl OneByteId {
    /// Construct a validated one-byte-form identifier. Returns
    /// [`Error::InvalidOneByteExtensionId`] for `0`, `15`, or any value
    /// outside the 4-bit field (`> 15`).
    pub fn new(id: u8) -> Result<Self> {
        if (ONE_BYTE_ID_MIN..=ONE_BYTE_ID_MAX).contains(&id) {
            Ok(Self(id))
        } else {
            Err(Error::InvalidOneByteExtensionId(id))
        }
    }

    /// The raw `1..=14` identifier value.
    #[must_use]
    pub fn get(self) -> u8 {
        self.0
    }
}

/// A validated RFC 8285 two-byte-form local identifier (§4.1.2/§5: `0` is
/// reserved for padding "in both forms"; §4.3's own field is otherwise the
/// full 8-bit range, `1..=255`). [`TwoByteId::new`] rejects `0`, so a
/// [`TwoByteElement`] can never hold the reserved padding ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct TwoByteId(u8);

impl TwoByteId {
    /// Construct a validated two-byte-form identifier. Returns
    /// [`Error::InvalidTwoByteExtensionId`] for `0`.
    pub fn new(id: u8) -> Result<Self> {
        if id >= TWO_BYTE_ID_MIN {
            Ok(Self(id))
        } else {
            Err(Error::InvalidTwoByteExtensionId)
        }
    }

    /// The raw `1..=255` identifier value.
    #[must_use]
    pub fn get(self) -> u8 {
        self.0
    }
}

// ---------------------------------------------------------------------------
// OneByteElement / OneByteElements — RFC 8285 §4.2
// ---------------------------------------------------------------------------

/// A single RFC 8285 one-byte-form extension element (§4.2): a validated
/// `1..=14` local identifier plus `1..=16` bytes of element data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct OneByteElement<'a> {
    /// The element's local identifier.
    pub id: OneByteId,
    /// The element's data, `1..=16` bytes (§4.2: `len` nibble = `data.len()
    /// - 1`).
    pub data: &'a [u8],
}

/// A parsed (or to-be-serialized) sequence of RFC 8285 one-byte-form
/// extension elements — the full contents of a [`HeaderExtension`] whose
/// `profile_id == `[`ONE_BYTE_PROFILE_ID`], excluding padding bytes and
/// anything after a stop point (§4.1.2/§4.2).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct OneByteElements<'a>(pub Vec<OneByteElement<'a>>);

impl<'a> OneByteElements<'a> {
    /// The parsed elements, in wire order.
    #[must_use]
    pub fn elements(&self) -> &[OneByteElement<'a>] {
        &self.0
    }
}

impl<'a> IntoIterator for OneByteElements<'a> {
    type Item = OneByteElement<'a>;
    type IntoIter = alloc::vec::IntoIter<OneByteElement<'a>>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a> Parse<'a> for OneByteElements<'a> {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let mut elements = Vec::new();
        let mut pos = 0;
        while pos < bytes.len() {
            let b = bytes[pos];
            if b == PADDING_BYTE {
                // A literal 0x00 byte is always one padding byte (§4.1.2).
                pos += 1;
                continue;
            }
            let id_nibble = b >> 4;
            if id_nibble == ONE_BYTE_STOP_ID || id_nibble == 0 {
                // id_nibble == 15: the reserved "stop" marker (§4.2).
                // id_nibble == 0 (but b != 0, so the length nibble is
                // nonzero): the malformed "ID 0 with length > 0" case
                // (§4.1.2), which must also terminate parsing. Both cases:
                // ignore the length field, stop, keep prior elements.
                break;
            }
            let len = usize::from(b & 0x0F) + 1; // len nibble = data.len() - 1
            let data_start = pos + 1;
            let data_end = data_start + len;
            if bytes.len() < data_end {
                return Err(Error::BufferTooShort {
                    need: data_end,
                    have: bytes.len(),
                    what: "RFC 8285 one-byte extension element data",
                });
            }
            let id =
                OneByteId::new(id_nibble).expect("id_nibble is 1..=14: 0 and 15 handled above");
            elements.push(OneByteElement {
                id,
                data: &bytes[data_start..data_end],
            });
            pos = data_end;
        }
        Ok(Self(elements))
    }
}

impl Serialize for OneByteElements<'_> {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let raw: usize = self.0.iter().map(|e| 1 + e.data.len()).sum();
        raw.div_ceil(EXT_ALIGN) * EXT_ALIGN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::BufferTooShort {
                need: len,
                have: buf.len(),
                what: "RFC 8285 one-byte extension elements serialize output",
            });
        }
        let mut pos = 0;
        for e in &self.0 {
            if !(ONE_BYTE_DATA_MIN..=ONE_BYTE_DATA_MAX).contains(&e.data.len()) {
                return Err(Error::InvalidValue {
                    field: "OneByteElement::data.len()",
                    value: e.data.len() as u64,
                    reason: "must be 1..=16 bytes (RFC 8285 §4.2: len nibble = data.len() - 1)",
                });
            }
            let len_nibble = (e.data.len() - 1) as u8;
            buf[pos] = (e.id.get() << 4) | len_nibble;
            pos += 1;
            buf[pos..pos + e.data.len()].copy_from_slice(e.data);
            pos += e.data.len();
        }
        for b in &mut buf[pos..len] {
            *b = PADDING_BYTE;
        }
        Ok(len)
    }
}

// ---------------------------------------------------------------------------
// TwoByteElement / TwoByteElements — RFC 8285 §4.3
// ---------------------------------------------------------------------------

/// A single RFC 8285 two-byte-form extension element (§4.3): a validated
/// `1..=255` local identifier plus `0..=255` bytes of element data (stored
/// directly, with no `-1` length bias — unlike the one-byte form).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TwoByteElement<'a> {
    /// The element's local identifier.
    pub id: TwoByteId,
    /// The element's data, `0..=255` bytes (§4.3: "The value zero (0)
    /// indicates that there is no subsequent data").
    pub data: &'a [u8],
}

/// A parsed (or to-be-serialized) sequence of RFC 8285 two-byte-form
/// extension elements — the full contents of a [`HeaderExtension`] whose
/// `profile_id & 0xFFF0 == 0x1000` (§4.3), excluding padding bytes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TwoByteElements<'a>(pub Vec<TwoByteElement<'a>>);

impl<'a> TwoByteElements<'a> {
    /// The parsed elements, in wire order.
    #[must_use]
    pub fn elements(&self) -> &[TwoByteElement<'a>] {
        &self.0
    }
}

impl<'a> IntoIterator for TwoByteElements<'a> {
    type Item = TwoByteElement<'a>;
    type IntoIter = alloc::vec::IntoIter<TwoByteElement<'a>>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a> Parse<'a> for TwoByteElements<'a> {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let mut elements = Vec::new();
        let mut pos = 0;
        while pos < bytes.len() {
            let id_byte = bytes[pos];
            if id_byte == PADDING_BYTE {
                // A literal 0x00 byte is always one padding byte (§4.1.2/§5:
                // "0 is reserved for padding in both forms"), consumed
                // before ever looking for a following length byte.
                pos += 1;
                continue;
            }
            let len_pos = pos + 1;
            if bytes.len() <= len_pos {
                return Err(Error::BufferTooShort {
                    need: len_pos + 1,
                    have: bytes.len(),
                    what: "RFC 8285 two-byte extension element length byte",
                });
            }
            let len = usize::from(bytes[len_pos]);
            let data_start = len_pos + 1;
            let data_end = data_start + len;
            if bytes.len() < data_end {
                return Err(Error::BufferTooShort {
                    need: data_end,
                    have: bytes.len(),
                    what: "RFC 8285 two-byte extension element data",
                });
            }
            let id = TwoByteId::new(id_byte).expect("id_byte != 0, checked above");
            elements.push(TwoByteElement {
                id,
                data: &bytes[data_start..data_end],
            });
            pos = data_end;
        }
        Ok(Self(elements))
    }
}

impl Serialize for TwoByteElements<'_> {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let raw: usize = self.0.iter().map(|e| 2 + e.data.len()).sum();
        raw.div_ceil(EXT_ALIGN) * EXT_ALIGN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::BufferTooShort {
                need: len,
                have: buf.len(),
                what: "RFC 8285 two-byte extension elements serialize output",
            });
        }
        let mut pos = 0;
        for e in &self.0 {
            if e.data.len() > TWO_BYTE_DATA_MAX {
                return Err(Error::InvalidValue {
                    field: "TwoByteElement::data.len()",
                    value: e.data.len() as u64,
                    reason: "exceeds the 8-bit length field maximum (255)",
                });
            }
            buf[pos] = e.id.get();
            buf[pos + 1] = e.data.len() as u8;
            pos += 2;
            buf[pos..pos + e.data.len()].copy_from_slice(e.data);
            pos += e.data.len();
        }
        for b in &mut buf[pos..len] {
            *b = PADDING_BYTE;
        }
        Ok(len)
    }
}

// ---------------------------------------------------------------------------
// ExtensionElements — top-level profile_id dispatch
// ---------------------------------------------------------------------------

/// The result of dispatching a [`HeaderExtension`] to its RFC 8285 form by
/// `profile_id` (see [`parse_extensions`]). A data-carrying dispatch wrapper
/// (in the same spirit as this workspace's `AnyTableSection`/`AnyDescriptor`
/// dispatch enums) — not a spec/field label, so it is exempt from the #204
/// `name()`/`Display` convention (see `tests/label_coverage.rs`'s SKIP list).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum ExtensionElements<'a> {
    /// §4.2 one-byte-form elements (`profile_id == 0xBEDE`).
    OneByte(OneByteElements<'a>),
    /// §4.3 two-byte-form elements (`profile_id & 0xFFF0 == 0x1000`).
    TwoByte(TwoByteElements<'a>),
}

impl Serialize for ExtensionElements<'_> {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        match self {
            Self::OneByte(e) => e.serialized_len(),
            Self::TwoByte(e) => e.serialized_len(),
        }
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        match self {
            Self::OneByte(e) => e.serialize_into(buf),
            Self::TwoByte(e) => e.serialize_into(buf),
        }
    }
}

/// Decode a [`HeaderExtension`]'s opaque `data` as RFC 8285 multiplexed
/// extension elements, dispatching on `profile_id` (§4.1.2):
///
/// - `0xBEDE` -> one-byte form ([`OneByteElements`], via [`ONE_BYTE_PROFILE_ID`])
/// - `& 0xFFF0 == 0x1000` -> two-byte form ([`TwoByteElements`], §4.3)
/// - anything else -> [`Error::NotRfc8285Extension`] — **not** a
///   malformed-packet error, since RFC 8285 interpretation of the RFC 3550
///   §5.3.1 opaque extension is profile-scoped and opt-in.
pub fn parse_extensions<'a>(ext: &HeaderExtension<'a>) -> Result<ExtensionElements<'a>> {
    if ext.profile_id == ONE_BYTE_PROFILE_ID {
        Ok(ExtensionElements::OneByte(OneByteElements::parse(
            ext.data,
        )?))
    } else if ext.profile_id & TWO_BYTE_PROFILE_ID_MASK == TWO_BYTE_PROFILE_ID_PREFIX {
        Ok(ExtensionElements::TwoByte(TwoByteElements::parse(
            ext.data,
        )?))
    } else {
        Err(Error::NotRfc8285Extension {
            profile_id: ext.profile_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    // -- OneByteId / TwoByteId validation -----------------------------------

    #[test]
    fn one_byte_id_rejects_padding_and_stop() {
        assert!(matches!(
            OneByteId::new(0),
            Err(Error::InvalidOneByteExtensionId(0))
        ));
        assert!(matches!(
            OneByteId::new(15),
            Err(Error::InvalidOneByteExtensionId(15))
        ));
        assert!(OneByteId::new(1).is_ok());
        assert!(OneByteId::new(14).is_ok());
    }

    #[test]
    fn two_byte_id_rejects_zero() {
        assert!(matches!(
            TwoByteId::new(0),
            Err(Error::InvalidTwoByteExtensionId)
        ));
        assert!(TwoByteId::new(1).is_ok());
        assert!(TwoByteId::new(255).is_ok());
    }

    // -- One-byte form round trips ------------------------------------------

    #[test]
    fn one_byte_round_trip_single_element() {
        let elements = OneByteElements(vec![OneByteElement {
            id: OneByteId::new(3).unwrap(),
            data: &[0xAA, 0xBB],
        }]);
        let mut out = vec![0u8; elements.serialized_len()];
        elements.serialize_into(&mut out).unwrap();
        // header byte: id=3, len nibble = 2-1=1
        assert_eq!(out[0], (3 << 4) | 1);
        assert_eq!(&out[1..3], &[0xAA, 0xBB]);
        // padded to a 4-byte multiple (3 bytes of content -> 4)
        assert_eq!(out.len(), 4);
        assert_eq!(out[3], 0x00);
        let reparsed = OneByteElements::parse(&out).unwrap();
        assert_eq!(reparsed, elements);
    }

    #[test]
    fn one_byte_spec_worked_example_structure() {
        // RFC 8285 §4.2 worked example structure: elem(L=0 -> 1 byte),
        // elem(L=1 -> 2 bytes), elem(L=3 -> 4 bytes). The RFC gives concrete
        // hex only for the profile id + length; IDs and data bytes here are
        // concrete values we chose to instantiate that structure (see
        // docs/rfc8285_header_ext.md). Padding is placed at the tail here
        // (this crate's `Serialize` always canonicalizes padding to a single
        // trailing run, since RFC 8285 assigns it no semantic content — see
        // `one_byte_reparses_interspersed_padding_to_the_same_elements` below
        // for the RFC diagram's own inter-element padding placement).
        let elements = OneByteElements(vec![
            OneByteElement {
                id: OneByteId::new(1).unwrap(),
                data: &[0x11],
            },
            OneByteElement {
                id: OneByteId::new(2).unwrap(),
                data: &[0x22, 0x33],
            },
            OneByteElement {
                id: OneByteId::new(3).unwrap(),
                data: &[0x44, 0x55, 0x66, 0x77],
            },
        ]);
        let mut out = vec![0u8; elements.serialized_len()];
        elements.serialize_into(&mut out).unwrap();
        let expected = [
            1 << 4, // ID=1 L=0 (1 byte)
            0x11,
            (2 << 4) | 1, // ID=2 L=1 (2 bytes)
            0x22,
            0x33,
            (3 << 4) | 3, // ID=3 L=3 (4 bytes)
            0x44,
            0x55,
            0x66,
            0x77,
            0x00,
            0x00, // 2 trailing pad bytes
        ];
        assert_eq!(out, expected);
        assert_eq!(
            out.len(),
            12,
            "3 words, matching length=3 in the RFC diagram"
        );
        let reparsed = OneByteElements::parse(&out).unwrap();
        assert_eq!(reparsed, elements);
    }

    #[test]
    fn one_byte_reparses_interspersed_padding_to_the_same_elements() {
        // The RFC 8285 §4.2 diagram itself places its 2 padding bytes
        // *between* the second and third elements, not at the tail (padding
        // "MAY be placed between extension elements, if desired for
        // alignment, or after the last extension element" -- §4.1.2).
        // Parsing MUST still recover the same elements regardless of where
        // the (semantically meaningless) padding bytes fall.
        #[rustfmt::skip]
        let interspersed: [u8; 12] = [
            1 << 4, 0x11,
            (2 << 4) | 1, 0x22, 0x33,
            0x00, 0x00, // pad between elements, per the RFC's own diagram
            (3 << 4) | 3, 0x44, 0x55, 0x66, 0x77,
        ];
        let parsed = OneByteElements::parse(&interspersed).unwrap();
        let expected_elements = OneByteElements(vec![
            OneByteElement {
                id: OneByteId::new(1).unwrap(),
                data: &[0x11],
            },
            OneByteElement {
                id: OneByteId::new(2).unwrap(),
                data: &[0x22, 0x33],
            },
            OneByteElement {
                id: OneByteId::new(3).unwrap(),
                data: &[0x44, 0x55, 0x66, 0x77],
            },
        ]);
        assert_eq!(parsed, expected_elements, "decoded elements are identical");

        // Re-serializing canonicalizes the padding to the tail: NOT
        // byte-identical to `interspersed`, but re-parsing the canonical
        // form still yields the same elements (semantic round trip).
        let mut out = vec![0u8; parsed.serialized_len()];
        parsed.serialize_into(&mut out).unwrap();
        assert_ne!(
            out, interspersed,
            "padding position is canonicalized, not preserved verbatim"
        );
        assert_eq!(OneByteElements::parse(&out).unwrap(), expected_elements);
    }

    #[test]
    fn one_byte_stop_marker_halts_parsing() {
        // elem(id=1, 1 byte data), then ID=15 (stop) with a nonzero length
        // nibble that MUST be ignored, then a trailing byte that must NOT be
        // parsed as another element.
        let bytes = [1 << 4, 0xAA, (15 << 4) | 5, 0xFF];
        let parsed = OneByteElements::parse(&bytes).unwrap();
        assert_eq!(parsed.elements().len(), 1);
        assert_eq!(parsed.elements()[0].id.get(), 1);
        assert_eq!(parsed.elements()[0].data, &[0xAA]);
    }

    #[test]
    fn one_byte_malformed_id_zero_with_length_halts_parsing() {
        // §4.1.2: an element with ID 0 and a length field > 0 is malformed;
        // the length field MUST be ignored and processing MUST terminate,
        // keeping only prior elements. Byte 0x05 = id nibble 0, len nibble 5
        // (nonzero byte, so NOT the plain 0x00 padding case).
        let bytes = [1 << 4, 0xAA, 0x05, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
        let parsed = OneByteElements::parse(&bytes).unwrap();
        assert_eq!(parsed.elements().len(), 1);
        assert_eq!(parsed.elements()[0].data, &[0xAA]);
    }

    #[test]
    fn one_byte_pure_padding_byte_is_skipped_not_terminal() {
        // A literal 0x00 byte is plain padding and parsing continues past
        // it (unlike the id=0-with-nonzero-length case above).
        let bytes = [
            1 << 4,
            0xAA,
            0x00, // pad
            2 << 4,
            0xBB,
        ];
        let parsed = OneByteElements::parse(&bytes).unwrap();
        assert_eq!(parsed.elements().len(), 2);
        assert_eq!(parsed.elements()[1].id.get(), 2);
        assert_eq!(parsed.elements()[1].data, &[0xBB]);
    }

    #[test]
    fn one_byte_rejects_data_len_out_of_range() {
        let elements = OneByteElements(vec![OneByteElement {
            id: OneByteId::new(1).unwrap(),
            data: &[],
        }]);
        let mut out = vec![0u8; 4];
        assert!(matches!(
            elements.serialize_into(&mut out),
            Err(Error::InvalidValue {
                field: "OneByteElement::data.len()",
                ..
            })
        ));
    }

    #[test]
    fn one_byte_truncated_element_data_is_buffer_too_short() {
        // Header claims len nibble 3 (=> 4 bytes) but only 2 remain.
        let bytes = [(1 << 4) | 3, 0xAA, 0xBB];
        assert!(matches!(
            OneByteElements::parse(&bytes),
            Err(Error::BufferTooShort { .. })
        ));
    }

    #[test]
    fn one_byte_empty_is_valid() {
        let elements = OneByteElements::default();
        assert_eq!(elements.serialized_len(), 0);
        let mut out: [u8; 0] = [];
        elements.serialize_into(&mut out).unwrap();
        let reparsed = OneByteElements::parse(&[]).unwrap();
        assert_eq!(reparsed, elements);
    }

    // -- Two-byte form round trips -------------------------------------------

    #[test]
    fn two_byte_round_trip_single_element() {
        let elements = TwoByteElements(vec![TwoByteElement {
            id: TwoByteId::new(200).unwrap(),
            data: &[0x01, 0x02, 0x03],
        }]);
        let mut out = vec![0u8; elements.serialized_len()];
        elements.serialize_into(&mut out).unwrap();
        assert_eq!(out[0], 200);
        assert_eq!(out[1], 3);
        assert_eq!(&out[2..5], &[0x01, 0x02, 0x03]);
        assert_eq!(out.len(), 8, "5 bytes of content padded to 8");
        let reparsed = TwoByteElements::parse(&out).unwrap();
        assert_eq!(reparsed, elements);
    }

    #[test]
    fn two_byte_spec_worked_example_structure() {
        // RFC 8285 §4.3 worked example structure: elem(L=0 -> 0 bytes),
        // elem(L=1 -> 1 byte), elem(L=4 -> 4 bytes). Padding is placed at
        // the tail here (see the one-byte-form comment above for why this
        // crate's `Serialize` canonicalizes padding placement).
        let elements = TwoByteElements(vec![
            TwoByteElement {
                id: TwoByteId::new(10).unwrap(),
                data: &[],
            },
            TwoByteElement {
                id: TwoByteId::new(20).unwrap(),
                data: &[0x99],
            },
            TwoByteElement {
                id: TwoByteId::new(30).unwrap(),
                data: &[0x01, 0x02, 0x03, 0x04],
            },
        ]);
        let mut out = vec![0u8; elements.serialized_len()];
        elements.serialize_into(&mut out).unwrap();
        let expected = [
            10, 0, // ID=10 L=0 (0 bytes)
            20, 1, 0x99, // ID=20 L=1 (1 byte)
            30, 4, // ID=30 L=4 (4 bytes)
            0x01, 0x02, 0x03, 0x04, 0x00, // 1 trailing pad byte
        ];
        assert_eq!(out, expected);
        assert_eq!(
            out.len(),
            12,
            "3 words, matching length=3 in the RFC diagram"
        );
        let reparsed = TwoByteElements::parse(&out).unwrap();
        assert_eq!(reparsed, elements);
    }

    #[test]
    fn two_byte_reparses_interspersed_padding_to_the_same_elements() {
        // As with the one-byte form: the RFC 8285 §4.3 diagram places its
        // padding byte between the second and third elements. Decoding must
        // still recover the same elements; re-serializing canonicalizes the
        // padding to the tail (not byte-identical to the original, but
        // semantically equal once re-parsed).
        #[rustfmt::skip]
        let interspersed: [u8; 12] = [
            10, 0,
            20, 1, 0x99,
            0x00, // pad between elements, per the RFC's own diagram
            30, 4, 0x01, 0x02, 0x03, 0x04,
        ];
        let parsed = TwoByteElements::parse(&interspersed).unwrap();
        let expected_elements = TwoByteElements(vec![
            TwoByteElement {
                id: TwoByteId::new(10).unwrap(),
                data: &[],
            },
            TwoByteElement {
                id: TwoByteId::new(20).unwrap(),
                data: &[0x99],
            },
            TwoByteElement {
                id: TwoByteId::new(30).unwrap(),
                data: &[0x01, 0x02, 0x03, 0x04],
            },
        ]);
        assert_eq!(parsed, expected_elements, "decoded elements are identical");

        let mut out = vec![0u8; parsed.serialized_len()];
        parsed.serialize_into(&mut out).unwrap();
        assert_ne!(
            out, interspersed,
            "padding position is canonicalized, not preserved verbatim"
        );
        assert_eq!(TwoByteElements::parse(&out).unwrap(), expected_elements);
    }

    #[test]
    fn two_byte_zero_id_byte_is_padding_not_an_element() {
        // A literal 0x00 byte is always a padding byte in the byte-by-byte
        // scan, even in two-byte form -- it must never be interpreted as
        // the start of an id=0 element header.
        let bytes = [10u8, 0, 0x00, 20, 1, 0x77];
        let parsed = TwoByteElements::parse(&bytes).unwrap();
        assert_eq!(parsed.elements().len(), 2);
        assert_eq!(parsed.elements()[0].id.get(), 10);
        assert_eq!(parsed.elements()[0].data, &[] as &[u8]);
        assert_eq!(parsed.elements()[1].id.get(), 20);
        assert_eq!(parsed.elements()[1].data, &[0x77]);
    }

    #[test]
    fn two_byte_truncated_length_byte_is_buffer_too_short() {
        let bytes = [10u8]; // id byte with no following length byte
        assert!(matches!(
            TwoByteElements::parse(&bytes),
            Err(Error::BufferTooShort { .. })
        ));
    }

    #[test]
    fn two_byte_truncated_element_data_is_buffer_too_short() {
        let bytes = [10u8, 4, 0x01, 0x02]; // claims 4 bytes, only 2 present
        assert!(matches!(
            TwoByteElements::parse(&bytes),
            Err(Error::BufferTooShort { .. })
        ));
    }

    #[test]
    fn two_byte_empty_is_valid() {
        let elements = TwoByteElements::default();
        assert_eq!(elements.serialized_len(), 0);
        let reparsed = TwoByteElements::parse(&[]).unwrap();
        assert_eq!(reparsed, elements);
    }

    // -- Top-level dispatch ---------------------------------------------------

    #[test]
    fn parse_extensions_dispatches_one_byte() {
        let data = [1 << 4, 0xAA, 0x00, 0x00];
        let ext = HeaderExtension {
            profile_id: ONE_BYTE_PROFILE_ID,
            data: &data,
        };
        let parsed = parse_extensions(&ext).unwrap();
        assert!(matches!(parsed, ExtensionElements::OneByte(_)));
    }

    #[test]
    fn parse_extensions_dispatches_two_byte() {
        let data = [10u8, 1, 0xAA, 0x00];
        let ext = HeaderExtension {
            profile_id: 0x1005, // 0x1000 | appbits(5)
            data: &data,
        };
        let parsed = parse_extensions(&ext).unwrap();
        assert!(matches!(parsed, ExtensionElements::TwoByte(_)));
    }

    #[test]
    fn parse_extensions_rejects_unknown_profile() {
        let data: [u8; 0] = [];
        let ext = HeaderExtension {
            profile_id: 0x1234,
            data: &data,
        };
        assert!(matches!(
            parse_extensions(&ext),
            Err(Error::NotRfc8285Extension { profile_id: 0x1234 })
        ));
    }

    #[test]
    fn extension_elements_serialize_round_trip_via_dispatch() {
        let data = [(5 << 4) | 1, 0x01, 0x02, 0x00];
        let ext = HeaderExtension {
            profile_id: ONE_BYTE_PROFILE_ID,
            data: &data,
        };
        let elements = parse_extensions(&ext).unwrap();
        let mut out = vec![0u8; elements.serialized_len()];
        elements.serialize_into(&mut out).unwrap();
        assert_eq!(out, data);
    }
}
