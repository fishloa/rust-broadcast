//! SNDU — SubNetwork Data Unit (RFC 4326 §4).
//!
//! Wire layout (Figure 1): `D` bit (1) + `Length` (15) + `Type` (16) +
//! optional 6-byte Destination NPA address (present iff `D = 0`) + PDU +
//! CRC-32 (4 bytes). The `Length` field counts from the byte *after* the Type
//! field up to and including the CRC (§4.2). The CRC-32 is the MPEG-2/DSM-CC
//! CRC over the whole SNDU excluding the 4-byte trailer (§4.6) — provided by
//! [`broadcast_common::crc32_mpeg2`].

use alloc::vec::Vec;

use crate::error::{Error, Result};
use crate::ext_header::PayloadChain;
use crate::type_field::TypeField;

/// Size in bytes of the SNDU base header (`D` + `Length` + `Type`).
pub const BASE_HEADER_LEN: usize = 4;
/// Size in bytes of the Destination NPA address (present when `D = 0`).
pub const NPA_LEN: usize = 6;
/// Size in bytes of the CRC-32 trailer.
pub const CRC_LEN: usize = 4;
/// The 15-bit `Length` value of an End Indicator — all length bits set.
pub const END_INDICATOR_LENGTH: u16 = 0x7FFF;
/// The two-byte value (`D = 1`, `Length = 0x7FFF`) that marks an End Indicator.
pub const END_INDICATOR: u16 = 0xFFFF;
/// The 0xFF byte used for TS-payload padding / stuffing (§4.3, §6).
pub const PADDING_BYTE: u8 = 0xFF;

/// Mask for the `D` bit (MSB of the first 16-bit word): `1` = no Destination
/// NPA address present (RFC 4326 §4.2).
pub(crate) const D_BIT_MASK: u16 = 0x8000;
/// Mask for the 15-bit `Length` field in the first 16-bit word (RFC 4326 §4.2).
pub(crate) const LENGTH_MASK: u16 = 0x7FFF;

/// A parsed/owned SubNetwork Data Unit (RFC 4326 §4).
///
/// Holds typed header fields plus a borrowed view of the PDU. `Length` and the
/// CRC are *not* stored: both are recomputed on serialize, so the round-trip is
/// driven entirely from the typed fields (no raw passthrough).
///
/// The base-header `Type` field is derived from the `payload` chain (via
/// [`PayloadChain::base_type`]) and is **not** stored as a separate field — the
/// chain is the single source of truth so the two can never diverge. Use
/// [`Sndu::type_field`] to read it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Sndu<'a> {
    /// The 6-byte Receiver Destination NPA address (§4.5), present iff `D = 0`.
    /// `D` is derived from `Some`/`None`: `D = 0` ⇔ `Some`.
    pub dest_address: Option<[u8; NPA_LEN]>,
    /// Decoded extension-header chain + PDU (§5). The base-header Type field is
    /// derived from the chain's `base_type()` on serialize.
    pub payload: PayloadChain<'a>,
}

impl<'a> Sndu<'a> {
    /// Construct an SNDU from a Type field, optional NPA address, and an opaque
    /// PDU (no extension headers).
    pub fn new(type_field: TypeField, dest_address: Option<[u8; NPA_LEN]>, pdu: &'a [u8]) -> Self {
        Sndu {
            dest_address,
            payload: PayloadChain {
                headers: Vec::new(),
                final_type: type_field,
                pdu,
            },
        }
    }

    /// The base-header Type field (§4.4): derived from the payload chain's
    /// [`PayloadChain::base_type`]. This is the value that appears on the wire
    /// at bytes `[2..4]` of the SNDU.
    pub fn type_field(&self) -> TypeField {
        self.payload.base_type()
    }

    /// The `D` bit value: `1` when no Destination Address is present.
    pub fn d_bit(&self) -> bool {
        self.dest_address.is_none()
    }

    /// The opaque PDU bytes (after any extension-header chain).
    pub fn pdu(&self) -> &'a [u8] {
        self.payload.pdu
    }

    /// The `Length` field as it appears on the wire: bytes counted from the
    /// byte *after* the Type field, up to and including the CRC (§4.2).
    /// = `NPA (if D=0)` + extension-chain content + PDU + CRC.
    pub fn length_field(&self) -> usize {
        let npa = if self.dest_address.is_some() {
            NPA_LEN
        } else {
            0
        };
        npa + self.payload.serialized_len() + CRC_LEN
    }

    /// Total serialized size in bytes (base header + everything the `Length`
    /// field counts).
    pub fn serialized_len(&self) -> usize {
        BASE_HEADER_LEN + self.length_field()
    }

    /// Parse an SNDU from the start of `data`. Requires the full SNDU
    /// (header..CRC) to be present; trailing bytes are ignored. Validates the
    /// CRC-32 trailer against a recomputed value (§4.6).
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        if data.len() < BASE_HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: BASE_HEADER_LEN,
                have: data.len(),
                what: "SNDU base header",
            });
        }
        let first = u16::from_be_bytes([data[0], data[1]]);
        let d_bit = (first & D_BIT_MASK) != 0;
        let length = first & LENGTH_MASK;
        let raw_type = u16::from_be_bytes([data[2], data[3]]);
        let type_field = TypeField::from_u16(raw_type);

        // The Length-counted region is `length` bytes starting at byte 4.
        let region_end = BASE_HEADER_LEN + length as usize;
        if length as usize > data.len() - BASE_HEADER_LEN {
            return Err(Error::InvalidLength {
                length,
                reason: "Length field exceeds available bytes",
            });
        }
        if (length as usize) < CRC_LEN {
            return Err(Error::InvalidLength {
                length,
                reason: "Length shorter than the CRC trailer",
            });
        }

        let dest_address = if d_bit {
            None
        } else {
            if BASE_HEADER_LEN + NPA_LEN > region_end {
                return Err(Error::InvalidLength {
                    length,
                    reason: "Length cannot hold the NPA address + CRC",
                });
            }
            let mut npa = [0u8; NPA_LEN];
            npa.copy_from_slice(&data[BASE_HEADER_LEN..BASE_HEADER_LEN + NPA_LEN]);
            Some(npa)
        };

        let payload_start = BASE_HEADER_LEN + if d_bit { 0 } else { NPA_LEN };
        let crc_start = region_end - CRC_LEN;
        if crc_start < payload_start {
            return Err(Error::InvalidLength {
                length,
                reason: "Length leaves no room for the payload",
            });
        }

        // CRC covers the whole SNDU from byte 0 up to (not including) the CRC.
        let computed = broadcast_common::crc32_mpeg2::compute(&data[..crc_start]);
        let found = u32::from_be_bytes([
            data[crc_start],
            data[crc_start + 1],
            data[crc_start + 2],
            data[crc_start + 3],
        ]);
        if computed != found {
            return Err(Error::CrcMismatch { computed, found });
        }

        let payload = PayloadChain::parse(type_field, &data[payload_start..crc_start])?;

        Ok(Sndu {
            dest_address,
            payload,
        })
    }

    /// Serialize the SNDU into `out`, recomputing `Length` and the CRC-32 from
    /// the typed fields. Returns the number of bytes written.
    pub fn serialize_into(&self, out: &mut [u8]) -> Result<usize> {
        let total = self.serialized_len();
        if out.len() < total {
            return Err(Error::OutputBufferTooSmall {
                need: total,
                have: out.len(),
            });
        }

        let length = self.length_field();
        if length > END_INDICATOR_LENGTH as usize {
            return Err(Error::FieldTooWide {
                what: "Length",
                value: length as u32,
                bits: 15,
            });
        }

        // Base Type field is always derived from the payload chain.
        let base_type = self.payload.base_type();

        // Byte 0..2: D | Length(15).
        let mut first = length as u16 & LENGTH_MASK;
        if self.d_bit() {
            first |= D_BIT_MASK;
        }
        out[0..2].copy_from_slice(&first.to_be_bytes());
        // Byte 2..4: Type.
        out[2..4].copy_from_slice(&base_type.to_u16().to_be_bytes());

        let mut off = BASE_HEADER_LEN;
        if let Some(npa) = self.dest_address {
            out[off..off + NPA_LEN].copy_from_slice(&npa);
            off += NPA_LEN;
        }
        let written = self
            .payload
            .serialize_into(&mut out[off..total - CRC_LEN])?;
        off += written;

        // CRC-32 over bytes [0, off).
        let crc = broadcast_common::crc32_mpeg2::compute(&out[..off]);
        out[off..off + CRC_LEN].copy_from_slice(&crc.to_be_bytes());
        off += CRC_LEN;
        Ok(off)
    }
}

/// `true` if the two bytes at the start of `data` are a ULE End Indicator
/// (`0xFFFF`: `D = 1`, `Length = 0x7FFF`) — no further SNDUs in this TS packet
/// (RFC 4326 §4.3).
pub fn is_end_indicator(data: &[u8]) -> bool {
    data.len() >= 2 && data[0] == PADDING_BYTE && data[1] == PADDING_BYTE
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    // A construct → serialize → assert exact wire bytes → reparse → equal test
    // for an IPv4 SNDU WITH an NPA address (D=0).
    #[test]
    fn d0_with_npa_exact_wire_bytes() {
        let pdu = [0x45u8, 0x00, 0x00, 0x14, 0xAB, 0xCD];
        let npa = [0x01u8, 0x02, 0x03, 0x04, 0x05, 0x06];
        let sndu = Sndu::new(TypeField::EtherType(0x0800), Some(npa), &pdu);

        // Length = NPA(6) + PDU(6) + CRC(4) = 16 = 0x10.
        assert_eq!(sndu.length_field(), 16);
        let mut out = vec![0u8; sndu.serialized_len()];
        let n = sndu.serialize_into(&mut out).unwrap();
        assert_eq!(n, 4 + 16);

        // Header: D=0 -> first byte high bit clear. Length 0x0010, Type 0x0800.
        assert_eq!(&out[0..2], &[0x00, 0x10]);
        assert_eq!(&out[2..4], &[0x08, 0x00]);
        // NPA then PDU.
        assert_eq!(&out[4..10], &npa);
        assert_eq!(&out[10..16], &pdu);
        // CRC = MPEG-2 over bytes [0,16).
        let crc = broadcast_common::crc32_mpeg2::compute(&out[..16]);
        assert_eq!(&out[16..20], &crc.to_be_bytes());

        // Reparse → equal.
        assert_eq!(Sndu::parse(&out).unwrap(), sndu);
    }

    // Same, for D=1 (no NPA).
    #[test]
    fn d1_without_npa_exact_wire_bytes() {
        let pdu = [0x60u8, 0x00, 0x00, 0x00];
        let sndu = Sndu::new(TypeField::EtherType(0x86DD), None, &pdu);

        // Length = PDU(4) + CRC(4) = 8.
        assert_eq!(sndu.length_field(), 8);
        let mut out = vec![0u8; sndu.serialized_len()];
        sndu.serialize_into(&mut out).unwrap();

        // D=1 -> high bit set: 0x8000 | 0x0008 = 0x8008.
        assert_eq!(&out[0..2], &[0x80, 0x08]);
        assert_eq!(&out[2..4], &[0x86, 0xDD]);
        assert_eq!(&out[4..8], &pdu);
        let crc = broadcast_common::crc32_mpeg2::compute(&out[..8]);
        assert_eq!(&out[8..12], &crc.to_be_bytes());

        assert_eq!(Sndu::parse(&out).unwrap(), sndu);
    }

    // Field-mutation bite: changing a PDU byte changes the serialized CRC, and
    // the original CRC then fails to parse.
    #[test]
    fn mutating_a_field_changes_crc() {
        let pdu = [0xDEu8, 0xAD, 0xBE, 0xEF];
        let a = Sndu::new(TypeField::EtherType(0x0800), None, &pdu);
        let mut buf_a = vec![0u8; a.serialized_len()];
        a.serialize_into(&mut buf_a).unwrap();

        let pdu2 = [0xDEu8, 0xAD, 0xBE, 0xEE]; // last byte changed
        let b = Sndu::new(TypeField::EtherType(0x0800), None, &pdu2);
        let mut buf_b = vec![0u8; b.serialized_len()];
        b.serialize_into(&mut buf_b).unwrap();

        assert_ne!(
            buf_a, buf_b,
            "different PDU must yield different wire bytes"
        );
        // The CRC trailers must differ.
        assert_ne!(buf_a[8..12], buf_b[8..12]);
    }

    #[test]
    fn npa_presence_drives_d_bit_and_length() {
        let pdu = [0u8; 10];
        let with = Sndu::new(TypeField::EtherType(0x0800), Some([0xFF; 6]), &pdu);
        let without = Sndu::new(TypeField::EtherType(0x0800), None, &pdu);
        assert!(!with.d_bit());
        assert!(without.d_bit());
        assert_eq!(with.length_field(), without.length_field() + NPA_LEN);
    }

    #[test]
    fn rejects_bad_crc() {
        let pdu = [1u8, 2, 3, 4];
        let sndu = Sndu::new(TypeField::EtherType(0x0800), None, &pdu);
        let mut buf = vec![0u8; sndu.serialized_len()];
        sndu.serialize_into(&mut buf).unwrap();
        let last = buf.len() - 1;
        buf[last] ^= 0x01;
        assert!(matches!(Sndu::parse(&buf), Err(Error::CrcMismatch { .. })));
    }

    #[test]
    fn rejects_length_overrun() {
        // Length claims 100 bytes but buffer is short.
        let data = [0x00u8, 0x64, 0x08, 0x00, 0x00];
        assert!(matches!(
            Sndu::parse(&data),
            Err(Error::InvalidLength { .. })
        ));
    }

    #[test]
    fn end_indicator_detected() {
        assert!(is_end_indicator(&[0xFF, 0xFF, 0xFF]));
        assert!(!is_end_indicator(&[0x00, 0x10]));
    }

    // type_field() accessor returns the chain's base type, which is the value
    // that will be serialized to the wire — no divergence possible.
    #[test]
    fn type_field_accessor_matches_serialized_wire() {
        let pdu = [0xAAu8; 4];
        let sndu = Sndu::new(TypeField::EtherType(0x0800), None, &pdu);
        assert_eq!(sndu.type_field(), TypeField::EtherType(0x0800));
        // Confirm it's what's on the wire.
        let mut buf = vec![0u8; sndu.serialized_len()];
        sndu.serialize_into(&mut buf).unwrap();
        let wire_type = u16::from_be_bytes([buf[2], buf[3]]);
        assert_eq!(sndu.type_field().to_u16(), wire_type);
    }
}
