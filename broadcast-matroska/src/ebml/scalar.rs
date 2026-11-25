//! EBML scalar payload codecs: unsigned and signed integers, floats, strings,
//! UTF-8 text, dates and binary blobs (RFC 8794 §7).
//!
//! Every codec keeps the width it was read at. §7.1/§7.2 allow an integer of
//! zero to eight octets, §7.3 a float of zero, four or eight, and §7.6 a date
//! of zero or eight, so a non-minimal integer or a four-octet float re-emits
//! exactly as read.

use broadcast_common::{Parse, Serialize};

use crate::error::{Error, Result};

/// Octets in a signed or unsigned integer payload (RFC 8794 §7.1, §7.2).
pub const MAX_INTEGER_OCTETS: usize = 8;
/// Bits in one octet.
const OCTET_BITS: u32 = 8;
/// Octets in a four-octet IEEE 754 float.
const FLOAT32_OCTETS: usize = 4;
/// Octets in an eight-octet IEEE 754 float.
const FLOAT64_OCTETS: usize = 8;
/// Octets in a date payload (RFC 8794 §7.6).
pub const DATE_OCTETS: usize = 8;
/// The value an empty numeric payload reads as (RFC 8794 §7.3).
const EMPTY_FLOAT_VALUE: f64 = 0.0;

/// Width of an integer or date payload, 0..=8 octets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct IntegerWidth(u8);

impl IntegerWidth {
    /// The empty payload, which reads as zero (§7.1, §7.2).
    pub const EMPTY: Self = Self(0);
    /// The widest payload.
    pub const MAX: Self = Self(MAX_INTEGER_OCTETS as u8);

    /// Build a width, rejecting anything outside 0..=8 octets.
    pub fn try_new(width: usize) -> Result<Self> {
        if width > MAX_INTEGER_OCTETS {
            return Err(Error::IntegerWidth { width });
        }
        let narrowed = u8::try_from(width).map_err(|_| Error::IntegerWidth { width })?;
        Ok(Self(narrowed))
    }

    /// The width in octets.
    #[must_use]
    pub const fn get(self) -> usize {
        self.0 as usize
    }
}

/// Width of a float payload (RFC 8794 §7.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum FloatWidth {
    /// The empty payload, which reads as zero.
    Empty,
    /// IEEE 754 binary32.
    Single,
    /// IEEE 754 binary64.
    Double,
}

impl FloatWidth {
    /// Octets this width occupies.
    #[must_use]
    pub const fn octets(self) -> usize {
        match self {
            Self::Empty => 0,
            Self::Single => FLOAT32_OCTETS,
            Self::Double => FLOAT64_OCTETS,
        }
    }

    /// Build a width from a payload length.
    pub fn try_new(width: usize) -> Result<Self> {
        match width {
            0 => Ok(Self::Empty),
            FLOAT32_OCTETS => Ok(Self::Single),
            FLOAT64_OCTETS => Ok(Self::Double),
            other => Err(Error::FloatWidth { width: other }),
        }
    }
}

/// An unsigned integer payload (RFC 8794 §7.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Unsigned {
    value: u64,
    width: IntegerWidth,
}

impl Unsigned {
    /// Build the narrowest non-empty encoding of `value`.
    pub fn new(value: u64) -> Self {
        let width = minimal_unsigned_width(value);
        Self { value, width }
    }

    /// Build a value at an explicit width.
    pub fn with_width(value: u64, width: IntegerWidth) -> Result<Self> {
        if width.get() == 0 {
            return if value == 0 {
                Ok(Self { value, width })
            } else {
                Err(Error::InvalidField(broadcast_common::error::InvalidField {
                    field: "Unsigned",
                    reason: "value does not fit the requested width",
                }))
            };
        }
        if width.get() < minimal_unsigned_width(value).get() {
            return Err(Error::InvalidField(broadcast_common::error::InvalidField {
                field: "Unsigned",
                reason: "value does not fit the requested width",
            }));
        }
        Ok(Self { value, width })
    }

    /// The value.
    #[must_use]
    pub const fn value(self) -> u64 {
        self.value
    }

    /// The encoded width.
    #[must_use]
    pub const fn width(self) -> IntegerWidth {
        self.width
    }
}

impl<'a> Parse<'a> for Unsigned {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let width = IntegerWidth::try_new(bytes.len())?;
        let mut value = 0u64;
        for &octet in bytes {
            value = (value << OCTET_BITS) | u64::from(octet);
        }
        Ok(Self { value, width })
    }
}

impl Serialize for Unsigned {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        self.width.get()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let width = self.width.get();
        let have = buf.len();
        let target = buf
            .get_mut(..width)
            .ok_or_else(|| missing_output(width, have))?;
        if width == 0 {
            return if self.value == 0 {
                Ok(0)
            } else {
                Err(Error::InvalidField(broadcast_common::error::InvalidField {
                    field: "Unsigned",
                    reason: "value does not fit the encoded width",
                }))
            };
        }
        if width < minimal_unsigned_width(self.value).get() {
            return Err(Error::InvalidField(broadcast_common::error::InvalidField {
                field: "Unsigned",
                reason: "value does not fit the encoded width",
            }));
        }
        for (index, slot) in target.iter_mut().enumerate() {
            let shift = OCTET_BITS * (width - index - 1) as u32;
            *slot = (self.value >> shift) as u8;
        }
        Ok(width)
    }
}

/// A signed integer payload (RFC 8794 §7.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Signed {
    value: i64,
    width: IntegerWidth,
}

impl Signed {
    /// Build the narrowest encoding of `value`.
    pub fn new(value: i64) -> Self {
        let width = minimal_signed_width(value);
        Self { value, width }
    }

    /// Build a value at an explicit width.
    pub fn with_width(value: i64, width: IntegerWidth) -> Result<Self> {
        let candidate = Self { value, width };
        candidate.check_width()?;
        Ok(candidate)
    }

    /// The value.
    #[must_use]
    pub const fn value(self) -> i64 {
        self.value
    }

    /// The encoded width.
    #[must_use]
    pub const fn width(self) -> IntegerWidth {
        self.width
    }

    /// Whether `value` fits a two's-complement payload of `width` octets.
    fn check_width(self) -> Result<()> {
        let bits = (self.width.get() * OCTET_BITS as usize) as u32;
        if bits == 0 {
            return if self.value == 0 {
                Ok(())
            } else {
                Err(Error::IntegerWidth { width: 0 })
            };
        }
        if bits >= i64::BITS {
            return Ok(());
        }
        let bound = 1i64 << (bits - 1);
        if self.value < -bound || self.value >= bound {
            return Err(Error::IntegerWidth {
                width: self.width.get(),
            });
        }
        Ok(())
    }
}

impl<'a> Parse<'a> for Signed {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let width = IntegerWidth::try_new(bytes.len())?;
        if bytes.is_empty() {
            return Ok(Self { value: 0, width });
        }
        let mut value = i64::from(bytes[0] as i8);
        for &octet in &bytes[1..] {
            value = (value << OCTET_BITS) | i64::from(octet);
        }
        Ok(Self { value, width })
    }
}

impl Serialize for Signed {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        self.width.get()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        self.check_width()?;
        let width = self.width.get();
        let have = buf.len();
        let target = buf
            .get_mut(..width)
            .ok_or_else(|| missing_output(width, have))?;
        let raw = self.value.to_be_bytes();
        let offset = raw.len() - width;
        target.copy_from_slice(&raw[offset..]);
        Ok(width)
    }
}

/// A float payload (RFC 8794 §7.3).
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Float {
    value: f64,
    width: FloatWidth,
}

impl Float {
    /// Build a binary64 value.
    #[must_use]
    pub const fn new(value: f64) -> Self {
        Self {
            value,
            width: FloatWidth::Double,
        }
    }

    /// Build a value at an explicit width.
    pub const fn with_width(value: f64, width: FloatWidth) -> Self {
        Self { value, width }
    }

    /// The value.
    #[must_use]
    pub const fn value(self) -> f64 {
        self.value
    }

    /// The encoded width.
    #[must_use]
    pub const fn width(self) -> FloatWidth {
        self.width
    }
}

impl<'a> Parse<'a> for Float {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let width = FloatWidth::try_new(bytes.len())?;
        let value = match width {
            FloatWidth::Empty => EMPTY_FLOAT_VALUE,
            FloatWidth::Single => {
                let raw: [u8; FLOAT32_OCTETS] = bytes
                    .try_into()
                    .map_err(|_| Error::FloatWidth { width: bytes.len() })?;
                f64::from(f32::from_be_bytes(raw))
            }
            FloatWidth::Double => {
                let raw: [u8; FLOAT64_OCTETS] = bytes
                    .try_into()
                    .map_err(|_| Error::FloatWidth { width: bytes.len() })?;
                f64::from_be_bytes(raw)
            }
        };
        Ok(Self { value, width })
    }
}

impl Serialize for Float {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        self.width.octets()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let width = self.width.octets();
        let have = buf.len();
        let target = buf
            .get_mut(..width)
            .ok_or_else(|| missing_output(width, have))?;
        match self.width {
            FloatWidth::Empty => {}
            FloatWidth::Single => target.copy_from_slice(&(self.value as f32).to_be_bytes()),
            FloatWidth::Double => target.copy_from_slice(&self.value.to_be_bytes()),
        }
        Ok(width)
    }
}

/// A string or UTF-8 payload, kept as the bytes read (RFC 8794 §7.4, §7.5).
///
/// §7.4 permits only printable ASCII in a String Element, but real senders
/// emit other bytes; the bytes are preserved verbatim so a file round-trips,
/// and [`as_str`](Self::as_str) decodes when the caller wants text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Text<'a> {
    bytes: &'a [u8],
}

impl<'a> Text<'a> {
    /// Build from bytes.
    #[must_use]
    pub const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    /// The raw bytes.
    #[must_use]
    pub const fn bytes(self) -> &'a [u8] {
        self.bytes
    }

    /// The payload as UTF-8, if it is valid UTF-8.
    #[must_use]
    pub fn as_str(self) -> Option<&'a str> {
        core::str::from_utf8(self.bytes).ok()
    }
}

impl<'a> Parse<'a> for Text<'a> {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Ok(Self::new(bytes))
    }
}

impl Serialize for Text<'_> {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        self.bytes.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let have = buf.len();
        let target = buf
            .get_mut(..self.bytes.len())
            .ok_or_else(|| missing_output(self.bytes.len(), have))?;
        target.copy_from_slice(self.bytes);
        Ok(self.bytes.len())
    }
}

/// A date payload: nanoseconds since 2001-01-01T00:00:00 UTC, or empty
/// (RFC 8794 §7.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Date {
    nanoseconds: i64,
    empty: bool,
}

impl Date {
    /// Build an eight-octet date.
    #[must_use]
    pub const fn new(nanoseconds: i64) -> Self {
        Self {
            nanoseconds,
            empty: false,
        }
    }

    /// The value in nanoseconds since 2001-01-01T00:00:00 UTC.
    #[must_use]
    pub const fn nanoseconds(self) -> i64 {
        self.nanoseconds
    }

    /// Whether the payload was empty.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.empty
    }
}

impl<'a> Parse<'a> for Date {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.is_empty() {
            return Ok(Self {
                nanoseconds: 0,
                empty: true,
            });
        }
        let raw: [u8; DATE_OCTETS] = bytes
            .try_into()
            .map_err(|_| Error::IntegerWidth { width: bytes.len() })?;
        Ok(Self {
            nanoseconds: i64::from_be_bytes(raw),
            empty: false,
        })
    }
}

impl Serialize for Date {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        if self.empty { 0 } else { DATE_OCTETS }
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if self.empty {
            return Ok(0);
        }
        let have = buf.len();
        let target = buf
            .get_mut(..DATE_OCTETS)
            .ok_or_else(|| missing_output(DATE_OCTETS, have))?;
        target.copy_from_slice(&self.nanoseconds.to_be_bytes());
        Ok(DATE_OCTETS)
    }
}

/// A binary payload the crate does not interpret (RFC 8794 §7.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Binary<'a> {
    bytes: &'a [u8],
}

impl<'a> Binary<'a> {
    /// Build from bytes.
    #[must_use]
    pub const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    /// The payload bytes.
    #[must_use]
    pub const fn bytes(self) -> &'a [u8] {
        self.bytes
    }
}

impl<'a> Parse<'a> for Binary<'a> {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        Ok(Self::new(bytes))
    }
}

impl Serialize for Binary<'_> {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        self.bytes.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let have = buf.len();
        let target = buf
            .get_mut(..self.bytes.len())
            .ok_or_else(|| missing_output(self.bytes.len(), have))?;
        target.copy_from_slice(self.bytes);
        Ok(self.bytes.len())
    }
}

/// The output-buffer-too-small error for a field of `need` bytes.
fn missing_output(need: usize, have: usize) -> Error {
    Error::OutputBufferTooSmall(broadcast_common::error::OutputBufferTooSmall { need, have })
}

/// Narrowest non-empty width that holds `value`.
fn minimal_unsigned_width(value: u64) -> IntegerWidth {
    let significant = (u64::BITS - value.leading_zeros()) as usize;
    let octets = significant.div_ceil(OCTET_BITS as usize).max(1);
    IntegerWidth(octets as u8)
}

/// Narrowest width whose two's-complement payload holds `value`.
fn minimal_signed_width(value: i64) -> IntegerWidth {
    let mut octets = 1usize;
    while octets < MAX_INTEGER_OCTETS {
        let bits = (octets * OCTET_BITS as usize) as u32;
        let bound = 1i64 << (bits - 1);
        if value >= -bound && value < bound {
            return IntegerWidth(octets as u8);
        }
        octets += 1;
    }
    IntegerWidth(MAX_INTEGER_OCTETS as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsigned_round_trips_every_width() {
        for (bytes, value) in [
            (&[][..], 0u64),
            (&[0x00, 0x00, 0x0F][..], 15),
            (&[0x0F, 0x42, 0x40][..], 1_000_000),
            (&[0xFF; MAX_INTEGER_OCTETS][..], u64::MAX),
        ] {
            let parsed = Unsigned::parse(bytes).unwrap();
            assert_eq!(parsed.value(), value);
            let out = parsed.try_to_bytes().unwrap();
            assert_eq!(out, bytes);
        }
    }

    #[test]
    fn signed_round_trips_negative_values() {
        for (bytes, value) in [
            (&[][..], 0i64),
            (&[0xFF][..], -1),
            (&[0xFF, 0xFF][..], -1),
            (&[0x80, 0x00][..], -32768),
        ] {
            let parsed = Signed::parse(bytes).unwrap();
            assert_eq!(parsed.value(), value);
            let out = parsed.try_to_bytes().unwrap();
            assert_eq!(out, bytes);
        }
    }

    #[test]
    fn float_widths_round_trip() {
        let single = Float::parse(&1.5f32.to_be_bytes()).unwrap();
        assert_eq!(single.width(), FloatWidth::Single);
        assert_eq!(single.try_to_bytes().unwrap(), 1.5f32.to_be_bytes());

        let double = Float::parse(&1.5f64.to_be_bytes()).unwrap();
        assert_eq!(double.value(), 1.5);
        assert_eq!(double.try_to_bytes().unwrap(), 1.5f64.to_be_bytes());

        let empty = Float::parse(&[]).unwrap();
        assert_eq!(empty.value(), 0.0);
        assert!(empty.try_to_bytes().unwrap().is_empty());
    }

    #[test]
    fn text_preserves_non_utf8_bytes() {
        let text = Text::parse(&[b'a', 0xE9]).unwrap();
        assert_eq!(text.as_str(), None);
        assert_eq!(text.try_to_bytes().unwrap(), [b'a', 0xE9]);
    }

    #[test]
    fn date_round_trips() {
        let date = Date::parse(&12345i64.to_be_bytes()).unwrap();
        assert_eq!(date.nanoseconds(), 12345);
        assert_eq!(date.try_to_bytes().unwrap(), 12345i64.to_be_bytes());
        let empty = Date::parse(&[]).unwrap();
        assert!(empty.is_empty());
        assert!(empty.try_to_bytes().unwrap().is_empty());
    }

    #[test]
    fn binary_round_trips() {
        let binary = Binary::parse(&[1, 2, 3]).unwrap();
        assert_eq!(binary.bytes(), &[1, 2, 3]);
        assert_eq!(binary.try_to_bytes().unwrap(), [1, 2, 3]);
    }
}
