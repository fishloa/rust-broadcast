//! `RoutePacket` — the composed ROUTE ALC/LCT packet, applying A/331 Annex
//! A's field-value constraints (§A.3.4/§A.3.6, transcribed at
//! `atsc3/docs/a331-route.md` §1) on top of [`rmt_flute::LctHeader`], plus
//! the SPI-bit-dispatched [`crate::RouteFecPayloadId`].
//!
//! This is the crate's one composing type — everything else
//! ([`crate::ext`], [`crate::fec`], [`crate::codepoint`]) is a standalone
//! piece of the ROUTE delta that a caller could also use directly against a
//! hand-driven [`rmt_flute::AlcPacket`].

use broadcast_common::{Parse, Serialize};
use rmt_flute::{LctHeader, PSI_SPI};

use crate::codepoint::Codepoint;
use crate::error::{Error, Result};
use crate::fec::{ROUTE_FEC_PAYLOAD_ID_LEN, RouteFecPayloadId};

/// ROUTE's mandated LCT version (§A.3.6 Table: `V` = `0001`) — "ROUTE version
/// number" per A/331's own reading of the field. Equal to
/// [`rmt_flute::LCT_VERSION`] (RFC 5651 LCT version 1); ROUTE does not define
/// its own version number space.
pub const ROUTE_VERSION: u8 = rmt_flute::LCT_VERSION;

/// ROUTE's mandated CCI length in bytes (§A.3.6 Table: `C` = `00`, so CCI is
/// `4*(0+1)` = 4 bytes).
pub const ROUTE_CCI_LEN: usize = 4;
/// ROUTE's mandated TSI length in bytes (§A.3.6 Table: `S` = `1`, `H` = `0`,
/// so TSI is `4*1 + 2*0` = 4 bytes).
pub const ROUTE_TSI_LEN: usize = 4;
/// ROUTE's mandated TOI length in bytes (§A.3.6 Table: `O` = `01`, `H` = `0`,
/// so TOI is `4*1 + 2*0` = 4 bytes).
pub const ROUTE_TOI_LEN: usize = 4;

/// PSI value ROUTE mandates for **source** packets (§A.3.6 Table: `PSI` =
/// `10`, both bits fixed).
pub const ROUTE_PSI_SOURCE: u8 = PSI_SPI;

/// A parsed (or to-be-serialized) ROUTE ALC/LCT packet (A/331 Annex A): an
/// [`LctHeader`] constrained to ROUTE's mandated field widths, followed by
/// the SPI-bit-selected [`RouteFecPayloadId`] and the opaque delivery-object
/// payload bytes.
///
/// Nothing is stored raw beyond the application-opaque `payload`; the LCT
/// header is fully typed (via `rmt-flute`) and the FEC Payload ID is decoded
/// into its [`RouteFecPayloadId`] variant rather than kept as bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct RoutePacket<'a> {
    /// The LCT header, constrained per A/331 §A.3.4/§A.3.6.
    pub lct: LctHeader<'a>,
    /// The FEC Payload ID, decoded per the packet's SPI bit.
    pub fec_payload_id: RouteFecPayloadId,
    /// The opaque delivery-object payload bytes.
    pub payload: &'a [u8],
}

/// Validate `lct` against A/331's ROUTE-mandated LCT field constraints
/// (§A.3.4/§A.3.6, `atsc3/docs/a331-route.md` §1's table).
fn validate_route_lct(lct: &LctHeader<'_>) -> Result<()> {
    if lct.version != ROUTE_VERSION {
        return Err(Error::InvalidField {
            what: "LCT V",
            reason: "ROUTE mandates LCT version 1 (V = 0001)",
        });
    }
    if lct.cci.len() != ROUTE_CCI_LEN {
        return Err(Error::InvalidField {
            what: "LCT C",
            reason: "ROUTE mandates C = 00 (4-byte CCI)",
        });
    }
    if lct.tsi.len() != ROUTE_TSI_LEN {
        return Err(Error::InvalidField {
            what: "LCT S/H",
            reason: "ROUTE mandates S = 1, H = 0 (4-byte TSI)",
        });
    }
    if lct.toi.len() != ROUTE_TOI_LEN {
        return Err(Error::InvalidField {
            what: "LCT O/H",
            reason: "ROUTE mandates O = 01, H = 0 (4-byte TOI)",
        });
    }
    // Source packets fix BOTH PSI bits (= 10); repair packets fix only the
    // high bit (SPI = 0) — the low bit is unconstrained by A/331 for repair
    // packets (§A.4.2.4, `atsc3/docs/a331-route.md` §1's PSI row).
    let spi = lct.psi & PSI_SPI != 0;
    if spi && lct.psi != ROUTE_PSI_SOURCE {
        return Err(Error::InvalidField {
            what: "LCT PSI",
            reason: "ROUTE source packets mandate PSI = 10 (both bits fixed)",
        });
    }
    Ok(())
}

impl RoutePacket<'_> {
    /// `true` if this is a source packet (SPI = 1); `false` if a repair
    /// packet (SPI = 0, §A.4.2.4).
    pub fn spi(&self) -> bool {
        self.lct.psi & PSI_SPI != 0
    }

    /// The decoded Codepoint semantics of [`LctHeader::codepoint`].
    pub fn codepoint(&self) -> Codepoint {
        Codepoint::from_u8(self.lct.codepoint)
    }
}

impl<'a> Parse<'a> for RoutePacket<'a> {
    type Error = Error;

    /// Parse a ROUTE ALC/LCT packet from the start of `data`: the LCT header
    /// (validated against ROUTE's mandated field widths/PSI), the 32-bit FEC
    /// Payload ID (decoded per the SPI bit), then the remaining bytes as the
    /// opaque payload.
    fn parse(data: &'a [u8]) -> Result<Self> {
        let (lct, used) = LctHeader::parse(data)?;
        validate_route_lct(&lct)?;

        let rest = &data[used..];
        if rest.len() < ROUTE_FEC_PAYLOAD_ID_LEN {
            return Err(Error::BufferTooShort {
                need: ROUTE_FEC_PAYLOAD_ID_LEN,
                have: rest.len(),
                what: "ROUTE FEC Payload ID",
            });
        }
        let spi = lct.psi & PSI_SPI != 0;
        let fec_payload_id = RouteFecPayloadId::parse(&rest[..ROUTE_FEC_PAYLOAD_ID_LEN], spi)?;
        let payload = &rest[ROUTE_FEC_PAYLOAD_ID_LEN..];

        Ok(RoutePacket {
            lct,
            fec_payload_id,
            payload,
        })
    }
}

impl Serialize for RoutePacket<'_> {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        self.lct.serialized_len() + ROUTE_FEC_PAYLOAD_ID_LEN + self.payload.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let total = self.serialized_len();
        if buf.len() < total {
            return Err(Error::OutputBufferTooSmall {
                need: total,
                have: buf.len(),
            });
        }
        validate_route_lct(&self.lct)?;

        let mut off = self.lct.serialize_into(buf)?;
        off += self.fec_payload_id.serialize_into(&mut buf[off..])?;
        buf[off..off + self.payload.len()].copy_from_slice(self.payload);
        off += self.payload.len();
        Ok(off)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use rmt_flute::{AlcPacket, HeaderExtension};

    use crate::fec::SourceFecPayloadId;

    #[test]
    fn source_packet_round_trip() {
        let cci = [0u8; 4];
        let tsi = 3000u32.to_be_bytes();
        let toi = 6034u32.to_be_bytes();
        let lct = LctHeader {
            version: ROUTE_VERSION,
            psi: ROUTE_PSI_SOURCE,
            close_session: false,
            close_object: false,
            codepoint: 128,
            cci: &cci,
            tsi: &tsi,
            toi: &toi,
            extensions: vec![],
        };
        let payload = [0xDEu8, 0xAD, 0xBE, 0xEF];
        let pkt = RoutePacket {
            lct,
            fec_payload_id: RouteFecPayloadId::Source(SourceFecPayloadId { start_offset: 1408 }),
            payload: &payload,
        };
        assert!(pkt.spi());
        assert!(pkt.codepoint().is_indirect());

        let mut out = vec![0u8; pkt.serialized_len()];
        let n = pkt.serialize_into(&mut out).unwrap();
        assert_eq!(n, out.len());

        let re = RoutePacket::parse(&out).unwrap();
        assert_eq!(re, pkt);
        assert_eq!(re.payload, &payload);
    }

    #[test]
    fn rejects_non_route_cci_width() {
        // C=1 (8-byte CCI) violates ROUTE's mandated C=00.
        let cci = [0u8; 8];
        let tsi = 0u32.to_be_bytes();
        let toi = 0u32.to_be_bytes();
        let lct = LctHeader {
            version: ROUTE_VERSION,
            psi: ROUTE_PSI_SOURCE,
            close_session: false,
            close_object: false,
            codepoint: 0,
            cci: &cci,
            tsi: &tsi,
            toi: &toi,
            extensions: vec![],
        };
        let mut out = vec![0u8; lct.serialized_len() + ROUTE_FEC_PAYLOAD_ID_LEN];
        let pkt = RoutePacket {
            lct,
            fec_payload_id: RouteFecPayloadId::Source(SourceFecPayloadId { start_offset: 0 }),
            payload: &[],
        };
        assert!(matches!(
            pkt.serialize_into(&mut out),
            Err(Error::InvalidField { what: "LCT C", .. })
        ));
    }

    #[test]
    fn rejects_source_packet_with_wrong_psi() {
        // SPI bit (high) set but low PSI bit also set (0b11): source packets
        // must be exactly 0b10.
        let cci = [0u8; 4];
        let tsi = 0u32.to_be_bytes();
        let toi = 0u32.to_be_bytes();
        let lct = LctHeader {
            version: ROUTE_VERSION,
            psi: 0b11,
            close_session: false,
            close_object: false,
            codepoint: 0,
            cci: &cci,
            tsi: &tsi,
            toi: &toi,
            extensions: vec![],
        };
        let mut out = vec![0u8; lct.serialized_len() + ROUTE_FEC_PAYLOAD_ID_LEN];
        let pkt = RoutePacket {
            lct,
            fec_payload_id: RouteFecPayloadId::Source(SourceFecPayloadId { start_offset: 0 }),
            payload: &[],
        };
        assert!(matches!(
            pkt.serialize_into(&mut out),
            Err(Error::InvalidField {
                what: "LCT PSI",
                ..
            })
        ));
    }

    #[test]
    fn repair_packet_low_psi_bit_is_unconstrained() {
        // SPI=0 (repair). Low PSI bit set (0b01) — allowed, A/331 does not
        // constrain it for repair packets.
        let cci = [0u8; 4];
        let tsi = 9000u32.to_be_bytes();
        let toi = 42u32.to_be_bytes();
        let lct = LctHeader {
            version: ROUTE_VERSION,
            psi: 0b01,
            close_session: false,
            close_object: false,
            codepoint: 0,
            cci: &cci,
            tsi: &tsi,
            toi: &toi,
            extensions: vec![],
        };
        use crate::fec::RepairFecPayloadId;
        let pkt = RoutePacket {
            lct,
            fec_payload_id: RouteFecPayloadId::Repair(RepairFecPayloadId { sbn: 1, esi: 2 }),
            payload: &[],
        };
        let mut out = vec![0u8; pkt.serialized_len()];
        assert!(pkt.serialize_into(&mut out).is_ok());
        let re = RoutePacket::parse(&out).unwrap();
        assert!(!re.spi());
        assert_eq!(re.fec_payload_id, pkt.fec_payload_id);
    }

    #[test]
    fn extension_chain_survives_round_trip() {
        let cci = [0u8; 4];
        let tsi = 3000u32.to_be_bytes();
        let toi = 6034u32.to_be_bytes();
        let fti_content = [0u8; 14];
        let exts = vec![HeaderExtension::new(
            rmt_flute::ALC_HET_EXT_FTI,
            &fti_content,
        )];
        let lct = LctHeader {
            version: ROUTE_VERSION,
            psi: ROUTE_PSI_SOURCE,
            close_session: false,
            close_object: true,
            codepoint: 8,
            cci: &cci,
            tsi: &tsi,
            toi: &toi,
            extensions: exts,
        };
        let payload = [1u8, 2, 3];
        let pkt = RoutePacket {
            lct,
            fec_payload_id: RouteFecPayloadId::Source(SourceFecPayloadId { start_offset: 0 }),
            payload: &payload,
        };
        let mut out = vec![0u8; pkt.serialized_len()];
        pkt.serialize_into(&mut out).unwrap();
        let re = RoutePacket::parse(&out).unwrap();
        assert_eq!(re, pkt);
        assert_eq!(re.lct.extensions.len(), 1);
    }

    // Sanity: RoutePacket composes cleanly with rmt_flute::AlcPacket for a
    // caller that wants both views (used only to prove the two crates agree
    // on the underlying LCT bytes, not part of the public API).
    #[test]
    fn agrees_with_rmt_flute_alc_packet_on_lct_bytes() {
        let cci = [0u8; 4];
        let tsi = 3000u32.to_be_bytes();
        let toi = 6034u32.to_be_bytes();
        let lct = LctHeader {
            version: ROUTE_VERSION,
            psi: ROUTE_PSI_SOURCE,
            close_session: false,
            close_object: false,
            codepoint: 128,
            cci: &cci,
            tsi: &tsi,
            toi: &toi,
            extensions: vec![],
        };
        let payload = [9u8, 9, 9, 9];
        let pkt = RoutePacket {
            lct: lct.clone(),
            fec_payload_id: RouteFecPayloadId::Source(SourceFecPayloadId { start_offset: 5 }),
            payload: &payload,
        };
        let mut out = vec![0u8; pkt.serialized_len()];
        pkt.serialize_into(&mut out).unwrap();

        let alc = AlcPacket::parse(&out, ROUTE_FEC_PAYLOAD_ID_LEN).unwrap();
        assert_eq!(alc.lct, lct);
        assert!(alc.spi());
    }
}
