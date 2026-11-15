//! The LCT identifier fields (RFC 5651 §5.1) and the FEC Payload ID (RFC 5775
//! §4.1, RFC 5740 §4.2.1).
//!
//! ## TSI and TOI
//!
//! TSI and TOI are numeric identifiers, but their wire byte-widths are driven
//! by the `S`/`O`/`H` flags: the TSI is `32*S + 16*H` bits and the TOI is
//! `32*O + 16*H` bits, so each is 0, 2, 4, 6, …, 14 bytes. A raw `&[u8]`
//! published for them handed callers bytes to assemble by hand, with nothing
//! checking the width against the flags or the value against the width.
//! [`Tsi`] and [`Toi`] carry the decoded integer plus the field width the wire
//! used, so the flags are a property of the value rather than something the
//! caller retypes as bytes.

use alloc::vec::Vec;
use broadcast_common::len::{need_in, need_out};

use crate::error::{Error, Result};

/// Largest wire width, in bytes, of a TSI or TOI. `O` is a two-bit field
/// (RFC 5651 §5.1), so the widest TOI is `4*3 + 2*1` = 14 bytes; the TSI is at
/// most `4*1 + 2*1` = 6.
pub const ID_MAX_LEN: usize = 14;

/// The shared body of [`Tsi`] and [`Toi`]: a value and the field width it
/// occupies on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
struct IdValue {
    value: u64,
    len: u8,
}

impl IdValue {
    /// Build from a value and a wire width, rejecting a width the flags cannot
    /// describe and a value the width cannot hold.
    fn new(field: &'static str, value: u64, len: u8) -> Result<Self> {
        let len = usize::from(len);
        if len % 2 != 0 || len > ID_MAX_LEN {
            return Err(Error::InvalidField(broadcast_common::error::InvalidField {
                field,
                reason: "wire width must be a whole number of 16-bit half-words, 0..=14 bytes",
            }));
        }
        if len < 8 && value > (1u64 << (8 * len)) - 1 {
            return Err(Error::FieldOverflow(
                broadcast_common::error::FieldOverflow {
                    field,
                    value,
                    max: (1u64 << (8 * len)) - 1,
                },
            ));
        }
        Ok(IdValue {
            value,
            len: len as u8,
        })
    }

    /// Read `len` big-endian bytes from `data`.
    fn parse(field: &'static str, data: &[u8], len: usize) -> Result<Self> {
        need_in(data, len, field)?;
        let mut v = 0u64;
        for &b in &data[..len] {
            v = (v << 8) | u64::from(b);
        }
        IdValue::new(field, v, len as u8)
    }

    /// Write the value as `len` big-endian bytes.
    fn write(&self, out: &mut [u8]) -> Result<usize> {
        let len = usize::from(self.len);
        need_out(out, len)?;
        let mut v = self.value;
        let mut i = len;
        let mut tmp = [0u8; ID_MAX_LEN];
        while i > 0 {
            i -= 1;
            tmp[i] = v as u8;
            v >>= 8;
        }
        out[..len].copy_from_slice(&tmp[..len]);
        Ok(len)
    }
}

/// An LCT Transport Session Identifier (RFC 5651 §5.1): `32*S + 16*H` bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Tsi(IdValue);

impl Tsi {
    const FIELD: &'static str = "TSI";

    /// A TSI of `value` occupying `len` bytes on the wire (0, 2, 4 or 6).
    ///
    /// # Errors
    ///
    /// [`Error::InvalidField`] for a width the flags cannot describe,
    /// [`Error::FieldOverflow`] if `value` does not fit `len` bytes.
    pub fn new(value: u64, len: u8) -> Result<Self> {
        IdValue::new(Self::FIELD, value, len).map(Tsi)
    }

    /// The identifier value.
    pub fn value(&self) -> u64 {
        self.0.value
    }

    /// The field width in bytes on the wire.
    pub fn len(&self) -> usize {
        usize::from(self.0.len)
    }

    /// `true` for a zero-width field (the `S=0`, `H=0` case).
    pub fn is_empty(&self) -> bool {
        self.0.len == 0
    }

    /// Parse `len` big-endian bytes.
    pub fn parse(data: &[u8], len: usize) -> Result<Self> {
        IdValue::parse(Self::FIELD, data, len).map(Tsi)
    }

    /// Write the identifier into `out`. Returns bytes written.
    pub fn serialize_into(&self, out: &mut [u8]) -> Result<usize> {
        self.0.write(out)
    }
}

/// An LCT Transport Object Identifier (RFC 5651 §5.1): `32*O + 16*H` bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Toi(IdValue);

impl Toi {
    const FIELD: &'static str = "TOI";

    /// A TOI of `value` occupying `len` bytes on the wire (0, 2, …, 14).
    ///
    /// # Errors
    ///
    /// [`Error::InvalidField`] for a width the flags cannot describe,
    /// [`Error::FieldOverflow`] if `value` does not fit `len` bytes.
    pub fn new(value: u64, len: u8) -> Result<Self> {
        IdValue::new(Self::FIELD, value, len).map(Toi)
    }

    /// The identifier value.
    pub fn value(&self) -> u64 {
        self.0.value
    }

    /// The field width in bytes on the wire.
    pub fn len(&self) -> usize {
        usize::from(self.0.len)
    }

    /// `true` for a zero-width field (the `O=0`, `H=0` case).
    pub fn is_empty(&self) -> bool {
        self.0.len == 0
    }

    /// Parse `len` big-endian bytes.
    pub fn parse(data: &[u8], len: usize) -> Result<Self> {
        IdValue::parse(Self::FIELD, data, len).map(Toi)
    }

    /// Write the identifier into `out`. Returns bytes written.
    pub fn serialize_into(&self, out: &mut [u8]) -> Result<usize> {
        self.0.write(out)
    }
}

/// A FEC Payload ID: the FEC-scheme-defined coding-block and symbol
/// identifier that follows the transport header (RFC 5775 §4.1 for ALC, RFC
/// 5740 §4.2.1 for NORM_DATA; the FLUSH/SQUELCH commands carry one too).
///
/// Its bit layout depends on the FEC Encoding ID (RFC 5052 §3/§8), so the
/// bytes stay opaque while their size does not: the value owns its bytes and
/// reports its own length, so no caller passes a separate
/// `fec_payload_id_len` and none can disagree with what is written.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct FecPayloadId {
    bytes: Vec<u8>,
}

impl FecPayloadId {
    /// A FEC Payload ID of `bytes`.
    pub fn new(bytes: &[u8]) -> Self {
        FecPayloadId {
            bytes: bytes.to_vec(),
        }
    }

    /// The FEC-scheme-defined bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The field width in bytes on the wire.
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// `true` for a packet that carries no FEC Payload ID (a data-less ALC
    /// packet, RFC 5775 §4.1).
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Parse a FEC Payload ID from exactly `len` bytes of `data`.
    pub fn parse(data: &[u8], len: usize) -> Result<Self> {
        need_in(data, len, "FEC Payload ID")?;
        Ok(FecPayloadId::new(&data[..len]))
    }

    /// Write the FEC Payload ID into `out`. Returns bytes written.
    pub fn serialize_into(&self, out: &mut [u8]) -> Result<usize> {
        need_out(out, self.len())?;
        out[..self.len()].copy_from_slice(&self.bytes);
        Ok(self.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The FEC Payload ID owns its bytes and reports their length, so parse and
    /// serialize agree on the width without a caller-supplied argument.
    #[test]
    fn a_fec_payload_id_carries_its_own_width() {
        let id = FecPayloadId::new(&[0x02, 0x03]);
        assert_eq!(id.len(), 2);
        let mut out = [0xFFu8; 4];
        assert_eq!(id.serialize_into(&mut out).expect("4-byte buffer"), 2);
        assert_eq!(out, [0x02, 0x03, 0xFF, 0xFF]);
        assert_eq!(
            FecPayloadId::parse(&[0x02, 0x03], 2).expect("two bytes"),
            id
        );
        assert!(FecPayloadId::parse(&[0x02], 2).is_err());
        assert!(FecPayloadId::new(&[]).is_empty());
    }

    #[test]
    fn a_value_wider_than_its_field_is_refused() {
        // The raw-byte shape let a caller write any length of bytes; the typed
        // value refuses one the wire field cannot hold.
        let err = Tsi::new(0x1_0000_0000, 4).unwrap_err();
        assert!(
            matches!(
                err,
                Error::FieldOverflow(broadcast_common::error::FieldOverflow {
                    field: "TSI",
                    value: 0x1_0000_0000,
                    max: 0xFFFF_FFFF
                })
            ),
            "got {err:?}"
        );
        assert!(Tsi::new(0xFFFF_FFFF, 4).is_ok());
        assert!(Tsi::new(0xFF, 2).is_ok());
        // A two-byte field cannot carry 0x1_0000.
        assert!(Tsi::new(0x1_0000, 2).is_err());
        assert!(Tsi::new(0xFFFF, 2).is_ok());
    }

    #[test]
    fn a_non_zero_value_cannot_ride_a_zero_width_field() {
        assert!(Tsi::new(0, 0).is_ok());
        assert!(Tsi::new(0, 0).unwrap().is_empty());
        assert!(Toi::new(1, 0).is_err());
    }

    #[test]
    fn widths_the_flags_cannot_describe_are_refused() {
        assert!(Tsi::new(0, 1).is_err());
        assert!(Toi::new(0, 16).is_err());
        assert!(Toi::new(0, 14).is_ok());
        let err = Toi::new(0, 3).unwrap_err();
        assert!(
            matches!(
                err,
                Error::InvalidField(broadcast_common::error::InvalidField { field: "TOI", .. })
            ),
            "got {err:?}"
        );
    }

    /// The widest field (`O=3, H=1` = fourteen bytes) writes big-endian with
    /// the leading bytes zero. Only fourteen bytes carry an identifier, so the
    /// value must be written out of the low bytes.
    #[test]
    fn a_wide_identifier_serializes_big_endian_with_leading_zeros() {
        let toi = Toi::new(0x0102_0304_0506, 14).expect("fits fourteen bytes");
        let mut out = [0xFFu8; 16];
        assert_eq!(toi.serialize_into(&mut out).expect("16-byte buffer"), 14);
        assert_eq!(
            out,
            [
                0, 0, 0, 0, 0, 0, 0, 0, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0xFF, 0xFF
            ]
        );
        assert_eq!(
            Toi::parse(&out[..14], 14).expect("fourteen bytes is a legal width"),
            toi
        );
    }

    #[test]
    fn parse_and_serialize_agree_on_the_wire() {
        let toi = Toi::parse(&[0x00, 0x00, 0x01, 0x02, 0x03, 0x04], 6).unwrap();
        assert_eq!(toi, Toi::new(0x0102_0304, 6).unwrap());
        assert_eq!(toi.value(), 0x0102_0304);
        assert_eq!(toi.len(), 6);
        let mut out = [0xFFu8; 8];
        assert_eq!(toi.serialize_into(&mut out).unwrap(), 6);
        assert_eq!(out, [0x00, 0x00, 0x01, 0x02, 0x03, 0x04, 0xFF, 0xFF]);
        assert_eq!(Tsi::parse(&[], 0).unwrap(), Tsi::new(0, 0).unwrap());
        assert!(Tsi::parse(&[0, 0, 0], 4).is_err());
    }
}
