//! ALC — Asynchronous Layered Coding packet (RFC 5775 §2, §4).
//!
//! An ALC packet (the UDP payload) = **LCT header + FEC Payload ID + Encoding
//! Symbol(s)**. ALC v1 uses LCT v1 ([`crate::lct`]).
//!
//! ⚠ The concrete FEC Payload ID bit layout is **not defined by RFC 5775** — it
//! depends on the FEC Scheme / FEC Encoding ID in use (RFC 5052 and the FEC
//! Scheme document). This crate therefore treats the FEC Payload ID as opaque
//! bytes in [`AlcPacket::fec_payload_id`]; the caller, knowing the FEC scheme,
//! slices it. One concrete layout (Small Block Systematic, `fec_id` = 129) is
//! provided as [`FecPayloadId128`] for convenience.
//!
//! A *data-less* ALC packet (RFC 5775 §4.1) carries the LCT header only — no FEC
//! Payload ID and no payload; that maps to an [`AlcPacket`] with an empty
//! `fec_payload_id` and empty `payload`.

use crate::error::{Error, Result};
use crate::lct::LctHeader;

/// HET for ALC's EXT_FTI (FEC Object Transmission Information) — RFC 5775 §4.2.
/// Variable-length form (HET 0..=127). The HEC body is FEC-scheme dependent.
pub const HET_EXT_FTI: u8 = 64;

/// ALC PSI bit: SPI (Source Packet Indicator) — RFC 5775 §2.1, the high PSI bit.
/// SPI = 1 ⇒ source-data FEC Payload ID format; 0 ⇒ repair-data format.
pub const PSI_SPI: u8 = 0b10;

/// A parsed ALC packet (RFC 5775 §4.1): an [`LctHeader`] followed by an opaque
/// FEC Payload ID and the encoding-symbol payload.
///
/// Nothing is stored raw beyond the application-opaque FEC Payload ID and
/// payload regions; the LCT header is fully typed and re-serialized from its
/// fields. `fec_payload_id_len` must be supplied to [`AlcPacket::parse`]
/// because RFC 5775 does not define the FEC Payload ID size (it is FEC-scheme
/// dependent).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AlcPacket<'a> {
    /// The LCT header (RFC 5651).
    pub lct: LctHeader<'a>,
    /// The opaque FEC Payload ID bytes (FEC-scheme dependent; may be empty for
    /// a data-less control packet).
    pub fec_payload_id: &'a [u8],
    /// The encoding-symbol payload bytes (may be empty for a data-less packet).
    pub payload: &'a [u8],
}

impl<'a> AlcPacket<'a> {
    /// Construct an ALC packet from its parts.
    pub fn new(lct: LctHeader<'a>, fec_payload_id: &'a [u8], payload: &'a [u8]) -> Self {
        AlcPacket {
            lct,
            fec_payload_id,
            payload,
        }
    }

    /// `true` if the high PSI bit (SPI, source-packet indicator) is set.
    pub fn spi(&self) -> bool {
        self.lct.psi & PSI_SPI != 0
    }

    /// Total serialized length in bytes.
    pub fn serialized_len(&self) -> usize {
        self.lct.serialized_len() + self.fec_payload_id.len() + self.payload.len()
    }

    /// Parse an ALC packet. `fec_payload_id_len` is the FEC-scheme-defined size
    /// of the FEC Payload ID in bytes (use `0` for a data-less packet that
    /// carries no FEC Payload ID and no payload).
    pub fn parse(data: &'a [u8], fec_payload_id_len: usize) -> Result<Self> {
        let (lct, used) = LctHeader::parse(data)?;
        let rest = &data[used..];
        if rest.len() < fec_payload_id_len {
            return Err(Error::BufferTooShort {
                need: fec_payload_id_len,
                have: rest.len(),
                what: "ALC FEC Payload ID",
            });
        }
        let fec_payload_id = &rest[..fec_payload_id_len];
        let payload = &rest[fec_payload_id_len..];
        Ok(AlcPacket {
            lct,
            fec_payload_id,
            payload,
        })
    }

    /// Serialize the ALC packet into `out`. Returns bytes written.
    pub fn serialize_into(&self, out: &mut [u8]) -> Result<usize> {
        let total = self.serialized_len();
        if out.len() < total {
            return Err(Error::OutputBufferTooSmall {
                need: total,
                have: out.len(),
            });
        }
        let mut off = self.lct.serialize_into(out)?;
        out[off..off + self.fec_payload_id.len()].copy_from_slice(self.fec_payload_id);
        off += self.fec_payload_id.len();
        out[off..off + self.payload.len()].copy_from_slice(self.payload);
        off += self.payload.len();
        Ok(off)
    }
}

/// FEC Payload ID for Small Block Systematic codes (`fec_id` = 128/129),
/// reproduced from RFC 5445 as an *illustrative* layout (RFC 5775 itself
/// defines no FEC Payload ID format). 8 bytes: a 32-bit `source_block_number`,
/// a 16-bit `source_block_length`, and a 16-bit `encoding_symbol_id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct FecPayloadId128 {
    /// Coding-block position within the object.
    pub source_block_number: u32,
    /// Number of source symbols (user-data segments) in the block.
    pub source_block_length: u16,
    /// Symbol index; `< source_block_length` ⇒ source symbol, else parity.
    pub encoding_symbol_id: u16,
}

/// Wire size in bytes of a [`FecPayloadId128`].
pub const FEC_PAYLOAD_ID_128_LEN: usize = 8;

impl FecPayloadId128 {
    /// Serialized length (always [`FEC_PAYLOAD_ID_128_LEN`]).
    pub fn serialized_len(&self) -> usize {
        FEC_PAYLOAD_ID_128_LEN
    }

    /// Parse from exactly the first 8 bytes of `data`.
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < FEC_PAYLOAD_ID_128_LEN {
            return Err(Error::BufferTooShort {
                need: FEC_PAYLOAD_ID_128_LEN,
                have: data.len(),
                what: "FEC Payload ID (fec_id 128/129)",
            });
        }
        Ok(FecPayloadId128 {
            source_block_number: u32::from_be_bytes([data[0], data[1], data[2], data[3]]),
            source_block_length: u16::from_be_bytes([data[4], data[5]]),
            encoding_symbol_id: u16::from_be_bytes([data[6], data[7]]),
        })
    }

    /// Serialize into `out` (8 bytes). Returns bytes written.
    pub fn serialize_into(&self, out: &mut [u8]) -> Result<usize> {
        if out.len() < FEC_PAYLOAD_ID_128_LEN {
            return Err(Error::OutputBufferTooSmall {
                need: FEC_PAYLOAD_ID_128_LEN,
                have: out.len(),
            });
        }
        out[0..4].copy_from_slice(&self.source_block_number.to_be_bytes());
        out[4..6].copy_from_slice(&self.source_block_length.to_be_bytes());
        out[6..8].copy_from_slice(&self.encoding_symbol_id.to_be_bytes());
        Ok(FEC_PAYLOAD_ID_128_LEN)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lct::{LctHeader, LCT_VERSION};
    use alloc::vec;

    fn lct_with_tsi() -> ([u8; 4], [u8; 4]) {
        // CCI (4) + TSI (4, S=1) so the ALC TSI-non-zero rule holds.
        ([0u8; 4], [0x00, 0x00, 0x00, 0x07])
    }

    #[test]
    fn alc_packet_round_trip() {
        let (cci, tsi) = lct_with_tsi();
        let lct = LctHeader {
            version: LCT_VERSION,
            psi: PSI_SPI,
            close_session: false,
            close_object: false,
            codepoint: 0x80,
            cci: &cci,
            tsi: &tsi,
            toi: &[],
            extensions: vec![],
        };
        let fpid = [0x00u8, 0x00, 0x00, 0x01, 0x00, 0x05, 0x00, 0x02];
        let payload = [0xDEu8, 0xAD, 0xBE, 0xEF];
        let pkt = AlcPacket::new(lct, &fpid, &payload);
        assert!(pkt.spi());

        let mut out = vec![0u8; pkt.serialized_len()];
        let n = pkt.serialize_into(&mut out).unwrap();
        assert_eq!(n, pkt.serialized_len());

        let re = AlcPacket::parse(&out, FEC_PAYLOAD_ID_128_LEN).unwrap();
        assert_eq!(re, pkt);
        assert_eq!(re.fec_payload_id, &fpid);
        assert_eq!(re.payload, &payload);
    }

    #[test]
    fn data_less_packet_has_no_fpid_or_payload() {
        let (cci, tsi) = lct_with_tsi();
        let lct = LctHeader {
            version: LCT_VERSION,
            psi: 0,
            close_session: true,
            close_object: false,
            codepoint: 0,
            cci: &cci,
            tsi: &tsi,
            toi: &[],
            extensions: vec![],
        };
        let pkt = AlcPacket::new(lct, &[], &[]);
        let mut out = vec![0u8; pkt.serialized_len()];
        pkt.serialize_into(&mut out).unwrap();
        let re = AlcPacket::parse(&out, 0).unwrap();
        assert_eq!(re, pkt);
        assert!(re.fec_payload_id.is_empty());
        assert!(re.payload.is_empty());
    }

    #[test]
    fn fec_payload_id_128_exact_bytes() {
        let f = FecPayloadId128 {
            source_block_number: 0x0102_0304,
            source_block_length: 0x0506,
            encoding_symbol_id: 0x0708,
        };
        let mut out = [0u8; FEC_PAYLOAD_ID_128_LEN];
        f.serialize_into(&mut out).unwrap();
        assert_eq!(out, [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);
        assert_eq!(FecPayloadId128::parse(&out).unwrap(), f);
    }

    #[test]
    fn mutating_payload_changes_wire() {
        let (cci, tsi) = lct_with_tsi();
        let mk = |p: &[u8]| {
            let lct = LctHeader {
                version: LCT_VERSION,
                psi: 0,
                close_session: false,
                close_object: false,
                codepoint: 0,
                cci: &cci,
                tsi: &tsi,
                toi: &[],
                extensions: vec![],
            };
            let pkt = AlcPacket::new(lct, &[], p);
            let mut out = vec![0u8; pkt.serialized_len()];
            pkt.serialize_into(&mut out).unwrap();
            out
        };
        assert_ne!(mk(&[1, 2, 3, 4]), mk(&[1, 2, 3, 5]));
    }
}
