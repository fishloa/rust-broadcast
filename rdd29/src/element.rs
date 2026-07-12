//! Element framing — RDD 29:2019 §2/§4.1/§5.1 (`ReadElement()`, Table 1).

use broadcast_common::Parse;
use broadcast_common::bits::{BitReader, BitWriter};

use crate::audio_data_dlc::AudioDataDlc;
use crate::bed_definition::BedDefinition1;
use crate::error::{Error, Result};
use crate::frame_rate::FrameRate;
use crate::object_definition::ObjectDefinition1;
use crate::plex::{plex_bits, read_plex, write_plex};

/// `ATMOS_FRAME` (Table 1, §5.1.1): the frame-header element.
pub const ELEMENT_ID_ATMOS_FRAME: u32 = 0x08;
/// `BED_DEFINITION1` (Table 1).
pub const ELEMENT_ID_BED_DEFINITION1: u32 = 0x10;
/// `OBJECT_DEFINITION1` (Table 1).
pub const ELEMENT_ID_OBJECT_DEFINITION1: u32 = 0x40;
/// `AUDIO_DATA_DLC` (Table 1).
pub const ELEMENT_ID_AUDIO_DATA_DLC: u32 = 0x200;

/// The `ElementID` field (Table 1, §5.1.1): identifies the kind and
/// contents of a Dolby Atmos bitstream element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum ElementId {
    /// `0x08` — Frame Header.
    AtmosFrame,
    /// `0x10` — Bed Definition Type 1.
    BedDefinition1,
    /// `0x40` — Object Definition Type 1.
    ObjectDefinition1,
    /// `0x200` — Audio Data (DLC encoded).
    AudioDataDlc,
    /// Any other value, including the three explicitly-reserved Table 1
    /// codes (`0x20`, `0x80`, `0x100`) and any vendor/future extension —
    /// "the decoder shall skip the element" (§5.1.1), never an error.
    Reserved(u32),
}

impl ElementId {
    /// The spec token for this value ("reserved" for the reserved arm).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::AtmosFrame => "ATMOS_FRAME",
            Self::BedDefinition1 => "BED_DEFINITION1",
            Self::ObjectDefinition1 => "OBJECT_DEFINITION1",
            Self::AudioDataDlc => "AUDIO_DATA_DLC",
            Self::Reserved(_) => "reserved",
        }
    }

    /// Decode a wire `ElementID` value.
    #[must_use]
    pub fn from_wire(v: u32) -> Self {
        match v {
            ELEMENT_ID_ATMOS_FRAME => Self::AtmosFrame,
            ELEMENT_ID_BED_DEFINITION1 => Self::BedDefinition1,
            ELEMENT_ID_OBJECT_DEFINITION1 => Self::ObjectDefinition1,
            ELEMENT_ID_AUDIO_DATA_DLC => Self::AudioDataDlc,
            other => Self::Reserved(other),
        }
    }

    /// The wire value for this `ElementID`.
    #[must_use]
    pub fn to_wire(self) -> u32 {
        match self {
            Self::AtmosFrame => ELEMENT_ID_ATMOS_FRAME,
            Self::BedDefinition1 => ELEMENT_ID_BED_DEFINITION1,
            Self::ObjectDefinition1 => ELEMENT_ID_OBJECT_DEFINITION1,
            Self::AudioDataDlc => ELEMENT_ID_AUDIO_DATA_DLC,
            Self::Reserved(v) => v,
        }
    }
}

broadcast_common::impl_spec_display!(ElementId, Reserved);

/// Any parsed Dolby Atmos bitstream element — the result of one
/// `ReadElement()` dispatch (§4.1). A tag-dispatch enum (see this crate's
/// `tests/label_coverage.rs` SKIP list), not itself a spec/field label.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum AnyElement<'a> {
    /// A [`BedDefinition1`] element.
    BedDefinition1(BedDefinition1),
    /// An [`ObjectDefinition1`] element.
    ObjectDefinition1(ObjectDefinition1<'a>),
    /// An [`AudioDataDlc`] element.
    AudioDataDlc(AudioDataDlc<'a>),
    /// An element whose `ElementID` this crate does not interpret —
    /// including the three explicitly-reserved Table 1 codes and any
    /// vendor/future extension. Carries the raw `ElementID` and its
    /// `ElementSize`-bounded body bytes verbatim, matching §4.1's
    /// `default` case (`UnknownData … ElementSize * 8`) exactly.
    Unknown {
        /// The wire `ElementID` value.
        element_id: u32,
        /// The element's raw body bytes (`ElementSize` bytes).
        data: &'a [u8],
    },
}

impl<'a> AnyElement<'a> {
    fn element_id_wire(&self) -> u32 {
        match self {
            Self::BedDefinition1(_) => ELEMENT_ID_BED_DEFINITION1,
            Self::ObjectDefinition1(_) => ELEMENT_ID_OBJECT_DEFINITION1,
            Self::AudioDataDlc(_) => ELEMENT_ID_AUDIO_DATA_DLC,
            Self::Unknown { element_id, .. } => *element_id,
        }
    }

    fn body_len(&self) -> usize {
        use broadcast_common::Serialize;
        match self {
            Self::BedDefinition1(b) => b.serialized_len(),
            Self::ObjectDefinition1(o) => o.serialized_len(),
            Self::AudioDataDlc(d) => d.serialized_len(),
            Self::Unknown { data, .. } => data.len(),
        }
    }

    /// Total wire length (header + body) this element will occupy.
    #[must_use]
    pub fn serialized_len(&self) -> usize {
        element_header_len(self.element_id_wire(), self.body_len()) + self.body_len()
    }

    /// Serialize this element (header + body) into `buf`.
    ///
    /// # Errors
    /// [`Error::BufferTooShort`] if `buf` is smaller than
    /// [`Self::serialized_len`]; otherwise any error the concrete element
    /// type's own `serialize_into` can return.
    pub fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        use broadcast_common::Serialize;
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::BufferTooShort {
                need,
                have: buf.len(),
                what: "AnyElement",
            });
        }
        let body_len = self.body_len();
        write_element_header(&mut buf[..need], self.element_id_wire(), body_len)?;
        let header_len = need - body_len;
        match self {
            Self::BedDefinition1(b) => {
                b.serialize_into(&mut buf[header_len..need])?;
            }
            Self::ObjectDefinition1(o) => {
                o.serialize_into(&mut buf[header_len..need])?;
            }
            Self::AudioDataDlc(d) => {
                d.serialize_into(&mut buf[header_len..need])?;
            }
            Self::Unknown { data, .. } => {
                buf[header_len..need].copy_from_slice(data);
            }
        }
        Ok(need)
    }

    /// Parse one `ReadElement()` header + body from the start of `bytes`,
    /// returning the element and the total bytes consumed (header + body).
    ///
    /// `frame_rate` must be `Some` if `bytes` might hold an
    /// `OBJECT_DEFINITION1` element (its pan-info loop length depends on
    /// it, §5.4.1) — pass `None` only when the caller knows that cannot
    /// occur.
    ///
    /// # Errors
    /// Returns [`Error`] on any protocol violation or buffer underrun, or
    /// [`Error::InvalidValue`] if an `OBJECT_DEFINITION1` element is
    /// encountered with `frame_rate` `None`.
    pub fn parse_with_frame_rate(
        bytes: &'a [u8],
        frame_rate: Option<FrameRate>,
    ) -> Result<(Self, usize)> {
        let mut r = BitReader::new(bytes);
        let element_id = read_plex(&mut r, 8, "ElementID")? as u32;
        let element_size = read_plex(&mut r, 8, "ElementSize")?;
        debug_assert!(r.is_byte_aligned());
        let header_len = r.bits_read() / 8;
        let element_size = usize::try_from(element_size).map_err(|_| Error::InvalidValue {
            field: "ElementSize",
            value: element_size,
            reason: "does not fit in this platform's usize",
        })?;
        let body_end = header_len
            .checked_add(element_size)
            .ok_or(Error::InvalidValue {
                field: "ElementSize",
                value: element_size as u64,
                reason: "overflowed usize",
            })?;
        if body_end > bytes.len() {
            return Err(Error::BufferTooShort {
                need: body_end,
                have: bytes.len(),
                what: "element body",
            });
        }
        let body = &bytes[header_len..body_end];
        let element = match element_id {
            ELEMENT_ID_BED_DEFINITION1 => Self::BedDefinition1(BedDefinition1::parse(body)?),
            ELEMENT_ID_OBJECT_DEFINITION1 => {
                let frame_rate = frame_rate.ok_or(Error::InvalidValue {
                    field: "ObjectDefinition1",
                    value: 0,
                    reason: "parsing an OBJECT_DEFINITION1 element requires the enclosing \
                             ATMOSFrame's FrameRate as context",
                })?;
                Self::ObjectDefinition1(ObjectDefinition1::parse_with_frame_rate(body, frame_rate)?)
            }
            ELEMENT_ID_AUDIO_DATA_DLC => Self::AudioDataDlc(AudioDataDlc::parse(body)?),
            other => Self::Unknown {
                element_id: other,
                data: body,
            },
        };
        Ok((element, body_end))
    }
}

/// Byte length of a `ReadElement()` header (`ElementID` + `ElementSize`,
/// both `Plex(8)`) for the given `element_id`/`body_len`.
pub(crate) fn element_header_len(element_id: u32, body_len: usize) -> usize {
    let bits = plex_bits(u64::from(element_id), 8) + plex_bits(body_len as u64, 8);
    (bits as usize).div_ceil(8)
}

/// Write a `ReadElement()` header (`ElementID` + `ElementSize`) into the
/// start of `buf`. `buf` may be longer than the header itself (the body is
/// written separately by the caller).
pub(crate) fn write_element_header(buf: &mut [u8], element_id: u32, body_len: usize) -> Result<()> {
    let mut w = BitWriter::new(buf);
    write_plex(&mut w, u64::from(element_id), 8, "ElementID")?;
    write_plex(&mut w, body_len as u64, 8, "ElementSize")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn element_id_round_trips() {
        for wire in [
            ELEMENT_ID_ATMOS_FRAME,
            ELEMENT_ID_BED_DEFINITION1,
            0x20,
            ELEMENT_ID_OBJECT_DEFINITION1,
            0x80,
            0x100,
            ELEMENT_ID_AUDIO_DATA_DLC,
        ] {
            assert_eq!(ElementId::from_wire(wire).to_wire(), wire);
        }
    }

    #[test]
    fn unknown_element_round_trips_verbatim() {
        let data = [0xAAu8, 0xBB, 0xCC];
        let element = AnyElement::Unknown {
            element_id: 0x20,
            data: &data,
        };
        let mut buf = alloc::vec![0u8; element.serialized_len()];
        element.serialize_into(&mut buf).unwrap();

        let (parsed, consumed) = AnyElement::parse_with_frame_rate(&buf, None).unwrap();
        assert_eq!(consumed, buf.len());
        match parsed {
            AnyElement::Unknown {
                element_id,
                data: parsed_data,
            } => {
                assert_eq!(element_id, 0x20);
                assert_eq!(parsed_data, &data);
            }
            other => panic!("expected Unknown, got {other:?}"),
        }
    }

    #[test]
    fn object_definition_without_frame_rate_context_errs() {
        // Body content is irrelevant here: AnyElement must refuse to
        // dispatch OBJECT_DEFINITION1 without frame_rate context before it
        // ever looks at the body.
        let element = AnyElement::Unknown {
            element_id: ELEMENT_ID_OBJECT_DEFINITION1,
            data: &[0u8; 4],
        };
        let mut wire = alloc::vec![0u8; element.serialized_len()];
        element.serialize_into(&mut wire).unwrap();

        let err = AnyElement::parse_with_frame_rate(&wire, None).unwrap_err();
        assert!(matches!(err, Error::InvalidValue { .. }));
    }
}
