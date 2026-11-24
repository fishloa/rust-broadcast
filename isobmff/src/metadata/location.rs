//! Recording location: the QuickTime `©xyz` ISO 6709 string and the 3GPP
//! `loci` Location Information box.
//!
//! `©xyz` is kept verbatim (it is a QuickTime international-text atom) and its
//! coordinates are parsed on demand; a form this crate cannot parse yields
//! `None` rather than a guessed position.
//!
//! `loci` is 3GPP TS 26.244 Table 8.10: a full box, a 16-bit packed language
//! (one pad bit + three five-bit letters), a null-terminated place name, an
//! 8-bit role, three 16.16 fixed-point coordinates and two more
//! null-terminated strings.

use alloc::string::String;
use alloc::vec::Vec;

use broadcast_common::len::{need_in, need_out};
use broadcast_common::{Parse, Serialize};

use crate::box_types::{
    BOX_HEADER_MIN_SIZE, FULLBOX_EXTRA_SIZE, FULLBOX_FLAGS_OFFSET, box_len, write_box_header,
};
use crate::error::{Error, Result};

use super::{FIXED_16_16_LEN, U16_LEN, U32_LEN};

/// The `loci` box type (3GPP TS 26.244 Table 8.10).
pub const LOCI: [u8; 4] = *b"loci";

/// The full-box header plus the packed language field.
const PREFIX_LEN: usize = FULLBOX_EXTRA_SIZE + U16_LEN;
/// The role field.
const ROLE_LEN: usize = 1;
/// The three fixed-point coordinates.
const COORDINATES_LEN: usize = FIXED_16_16_LEN * 3;
/// The smallest body: prefix, an empty name, role, coordinates and two empty
/// strings.
const MIN_BODY_LEN: usize = PREFIX_LEN + 1 + ROLE_LEN + COORDINATES_LEN + 2;
/// The fixed-point scale of a `loci` coordinate (16.16).
const FIXED_16_16_SCALE: f64 = 65536.0;
/// Bits in the fractional half of a 16.16 value.
const FIXED_16_16_FRACTION_BITS: u32 = 16;

/// The length of a three-letter language code.
pub const LANGUAGE_CODE_LEN: usize = 3;
/// The index of the altitude component in an ISO 6709 string.
const ALTITUDE_COMPONENT: usize = 2;
/// The 5-bit width of one packed language subfield.
const LANGUAGE_SUBFIELD_BITS: u16 = 5;
/// The mask of one packed language subfield.
const LANGUAGE_SUBFIELD_MASK: u16 = 0x1f;
/// The shift of the first packed language subfield.
const LANGUAGE_FIRST_SHIFT: u16 = 10;
/// The shift of the second packed language subfield.
const LANGUAGE_SECOND_SHIFT: u16 = 5;
/// The shift of the third packed language subfield.
const LANGUAGE_THIRD_SHIFT: u16 = 0;
/// The ASCII value each packed language letter is stored relative to.
const LANGUAGE_ASCII_BASE: u8 = 0x60;
/// The number of letters in the alphabet.
const LANGUAGE_LETTERS: u8 = 26;

/// An ISO 6709 geographic point string.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Iso6709 {
    raw: String,
}

impl Iso6709 {
    /// Wrap the string exactly as it was carried.
    #[must_use]
    pub fn new(raw: impl Into<String>) -> Self {
        Self { raw: raw.into() }
    }

    /// The original string, written back unchanged.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// Latitude in degrees, when the string has a parseable first component.
    #[must_use]
    pub fn latitude(&self) -> Option<f64> {
        self.components().first().copied()
    }

    /// Longitude in degrees, when the string has a parseable second component.
    #[must_use]
    pub fn longitude(&self) -> Option<f64> {
        self.components().get(1).copied()
    }

    /// Altitude in metres, when the string carries a third component.
    #[must_use]
    pub fn altitude(&self) -> Option<f64> {
        self.components().get(ALTITUDE_COMPONENT).copied()
    }

    fn components(&self) -> Vec<f64> {
        let raw = self.raw.trim_end_matches('/');
        let mut tokens = Vec::new();
        let mut start = 0usize;
        for (index, byte) in raw.bytes().enumerate().skip(1) {
            if byte == b'+' || byte == b'-' {
                tokens.push(&raw[start..index]);
                start = index;
            }
        }
        if start < raw.len() {
            tokens.push(&raw[start..]);
        }
        tokens
            .into_iter()
            .map(str::parse)
            .collect::<core::result::Result<Vec<f64>, _>>()
            .unwrap_or_default()
    }
}

impl core::fmt::Display for Iso6709 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.raw)
    }
}

/// The 3GPP Location Information box (`loci`, TS 26.244 Table 8.10).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Loci {
    /// FullBox version.
    pub version: u8,
    /// FullBox flags.
    pub flags: u32,
    /// The packed language field (one pad bit + three five-bit letters).
    pub language: u16,
    /// The null-terminated place name, without its terminator.
    pub name: Vec<u8>,
    /// The role code (0 shooting, 1 real, 2 fictional).
    pub role: u8,
    /// Longitude as a 16.16 fixed-point value.
    pub longitude: i32,
    /// Latitude as a 16.16 fixed-point value.
    pub latitude: i32,
    /// Altitude as a 16.16 fixed-point value, in metres.
    pub altitude: i32,
    /// The null-terminated astronomical body, without its terminator.
    pub astronomical_body: Vec<u8>,
    /// The null-terminated additional notes, without the terminator.
    pub additional_notes: Vec<u8>,
}

impl Loci {
    /// The place name, when it is UTF-8.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        core::str::from_utf8(&self.name).ok()
    }

    /// The astronomical body, when it is UTF-8.
    #[must_use]
    pub fn astronomical_body(&self) -> Option<&str> {
        core::str::from_utf8(&self.astronomical_body).ok()
    }

    /// The additional notes, when they are UTF-8.
    #[must_use]
    pub fn additional_notes(&self) -> Option<&str> {
        core::str::from_utf8(&self.additional_notes).ok()
    }

    /// The longitude in degrees.
    #[must_use]
    pub fn longitude_degrees(&self) -> f64 {
        f64::from(self.longitude) / FIXED_16_16_SCALE
    }

    /// The latitude in degrees.
    #[must_use]
    pub fn latitude_degrees(&self) -> f64 {
        f64::from(self.latitude) / FIXED_16_16_SCALE
    }

    /// The altitude in metres.
    #[must_use]
    pub fn altitude_metres(&self) -> f64 {
        f64::from(self.altitude) / FIXED_16_16_SCALE
    }

    fn body_len(&self) -> usize {
        PREFIX_LEN
            + self.name.len()
            + 1
            + ROLE_LEN
            + COORDINATES_LEN
            + self.astronomical_body.len()
            + 1
            + self.additional_notes.len()
            + 1
    }
}

impl<'a> Parse<'a> for Loci {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let item = crate::box_types::parse_box(bytes)?.0;
        if !item.header.box_type.is(&LOCI) {
            return Err(Error::UnexpectedBox { expected: "loci" });
        }
        Self::parse_body(item.body)
    }
}

impl Loci {
    /// Parse a `loci` from its body.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, MIN_BODY_LEN, "loci body")?;
        let version = body[0];
        let flags = broadcast_common::wire::u24_at(body, FULLBOX_FLAGS_OFFSET);
        let language = super::u16_at(body, FULLBOX_EXTRA_SIZE);
        let mut cursor = PREFIX_LEN;
        let name = take_c_string(body, &mut cursor, "loci name")?;
        need_in(
            body,
            cursor + ROLE_LEN + COORDINATES_LEN,
            "loci coordinates",
        )?;
        let role = body[cursor];
        cursor += ROLE_LEN;
        let longitude = i32::from_be_bytes(
            body[cursor..cursor + U32_LEN]
                .try_into()
                .map_err(|_| Error::InvalidInput("loci longitude"))?,
        );
        cursor += U32_LEN;
        let latitude = i32::from_be_bytes(
            body[cursor..cursor + U32_LEN]
                .try_into()
                .map_err(|_| Error::InvalidInput("loci latitude"))?,
        );
        cursor += U32_LEN;
        let altitude = i32::from_be_bytes(
            body[cursor..cursor + U32_LEN]
                .try_into()
                .map_err(|_| Error::InvalidInput("loci altitude"))?,
        );
        cursor += U32_LEN;
        let astronomical_body = take_c_string(body, &mut cursor, "loci astronomical body")?;
        let additional_notes = take_c_string(body, &mut cursor, "loci additional notes")?;
        if cursor != body.len() {
            return Err(Error::InvalidInput("loci has trailing bytes"));
        }
        Ok(Self {
            version,
            flags,
            language,
            name,
            role,
            longitude,
            latitude,
            altitude,
            astronomical_body,
            additional_notes,
        })
    }
}

fn take_c_string(body: &[u8], cursor: &mut usize, what: &'static str) -> Result<Vec<u8>> {
    let rest = body
        .get(*cursor..)
        .ok_or(Error::InvalidInput("loci string starts past the body"))?;
    let len = rest
        .iter()
        .position(|byte| *byte == 0)
        .ok_or(Error::InvalidInput(what))?;
    let value = rest[..len].to_vec();
    *cursor += len + 1;
    Ok(value)
}

impl Serialize for Loci {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + self.body_len())
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = write_box_header(buf, &LOCI, need)?;
        buf[cursor] = self.version;
        cursor += 1;
        let flags = self.flags.to_be_bytes();
        buf[cursor..cursor + (FULLBOX_EXTRA_SIZE - 1)].copy_from_slice(&flags[1..]);
        cursor += FULLBOX_EXTRA_SIZE - 1;
        buf[cursor..cursor + U16_LEN].copy_from_slice(&self.language.to_be_bytes());
        cursor += U16_LEN;
        buf[cursor..cursor + self.name.len()].copy_from_slice(&self.name);
        cursor += self.name.len();
        buf[cursor] = 0;
        cursor += 1;
        buf[cursor] = self.role;
        cursor += ROLE_LEN;
        for value in [self.longitude, self.latitude, self.altitude] {
            buf[cursor..cursor + U32_LEN].copy_from_slice(&value.to_be_bytes());
            cursor += U32_LEN;
        }
        buf[cursor..cursor + self.astronomical_body.len()].copy_from_slice(&self.astronomical_body);
        cursor += self.astronomical_body.len();
        buf[cursor] = 0;
        cursor += 1;
        buf[cursor..cursor + self.additional_notes.len()].copy_from_slice(&self.additional_notes);
        cursor += self.additional_notes.len();
        buf[cursor] = 0;
        cursor += 1;
        Ok(cursor)
    }
}

/// The 5-bit subfield packing of an ISO 639-2/T code (Apple `ISO language
/// codes`): each letter maps to `ASCII - 0x60`, three five-bit subfields in
/// the low 15 bits.
#[must_use]
pub fn packed_language_code(code: &[u8; LANGUAGE_CODE_LEN]) -> u16 {
    let mut packed = 0u16;
    for letter in code {
        packed = (packed << LANGUAGE_SUBFIELD_BITS)
            | u16::from(letter.wrapping_sub(LANGUAGE_ASCII_BASE));
    }
    packed
}

/// The unpacked three-letter code of a packed language field, when every
/// subfield is a letter.
#[must_use]
pub fn unpack_language_code(packed: u16) -> Option<String> {
    let mut out = String::with_capacity(LANGUAGE_CODE_LEN);
    for shift in [
        LANGUAGE_FIRST_SHIFT,
        LANGUAGE_SECOND_SHIFT,
        LANGUAGE_THIRD_SHIFT,
    ] {
        let value = ((packed >> shift) & LANGUAGE_SUBFIELD_MASK) as u8;
        if value == 0 || value > LANGUAGE_LETTERS {
            return None;
        }
        out.push(char::from(b'a' + value - 1));
    }
    Some(out)
}

/// The fractional half of a 16.16 value.
#[must_use]
pub fn fixed_16_16_fraction(value: i32) -> u16 {
    (value as u32 & ((1u32 << FIXED_16_16_FRACTION_BITS) - 1)) as u16
}

/// Build the 3GPP packed language field from a code.
#[must_use]
pub const fn packed_language(code: [u8; LANGUAGE_CODE_LEN]) -> u16 {
    (code[0].wrapping_sub(LANGUAGE_ASCII_BASE) as u16) << LANGUAGE_FIRST_SHIFT
        | (code[1].wrapping_sub(LANGUAGE_ASCII_BASE) as u16) << LANGUAGE_SECOND_SHIFT
        | code[2].wrapping_sub(LANGUAGE_ASCII_BASE) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_language_round_trips() {
        for code in [*b"eng", *b"und", *b"jpn"] {
            let packed = packed_language(code);
            assert_eq!(
                unpack_language_code(packed).as_deref(),
                Some(core::str::from_utf8(&code).unwrap())
            );
        }
        assert_eq!(packed_language(*b"jpn"), 0x2A0E);
    }

    #[test]
    fn loci_round_trips() {
        let loci = Loci {
            version: 0,
            flags: 0,
            language: packed_language(*b"eng"),
            name: b"Place".to_vec(),
            role: 1,
            longitude: 0x0010_0000,
            latitude: -0x0010_0000,
            altitude: 0x0001_0000,
            astronomical_body: b"earth".to_vec(),
            additional_notes: b"note".to_vec(),
        };
        let bytes = loci.try_to_bytes().expect("loci writes");
        let parsed = Loci::parse(&bytes).expect("loci parses");
        assert_eq!(parsed, loci);
        assert!((parsed.longitude_degrees() - 16.0).abs() < f64::EPSILON);
        assert!((parsed.latitude_degrees() + 16.0).abs() < f64::EPSILON);
        assert!((parsed.altitude_metres() - 1.0).abs() < f64::EPSILON);
        assert_eq!(parsed.name(), Some("Place"));
        assert_eq!(parsed.astronomical_body(), Some("earth"));
        assert_eq!(parsed.additional_notes(), Some("note"));
    }
}
