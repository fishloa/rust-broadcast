//! ROUTE FEC Payload ID layouts — A/331 Annex A §A.3.5.1 (source flows) and
//! §A.3.5.2 (repair flows), transcribed at `atsc3/docs/a331-route.md` §3.
//!
//! The 32-bit "FEC Payload ID" that follows the LCT header in every ROUTE
//! ALC packet ([`rmt_flute::AlcPacket::fec_payload_id`] carries it as an
//! opaque slice, since RFC 5775 leaves its format FEC-scheme-dependent) has
//! exactly **two** ROUTE-defined layouts, selected by the LCT `PSI` "SPI" bit
//! ([`rmt_flute::PSI_SPI`], [`rmt_flute::AlcPacket::spi`]):
//!
//! - **source flows** (SPI = 1) — Compact No-Code FEC scheme (§A.3.5.1,
//!   Figure A.3.3): a single 32-bit `start_offset`.
//! - **repair flows** (SPI = 0) — RaptorQ FEC scheme, RFC 6330 §3.2
//!   (§A.3.5.2, Figure A.3.4): an 8-bit Source Block Number (`SBN`) followed
//!   by a 24-bit Encoding Symbol ID (`ESI`).
//!
//! Both layouts are exactly 4 bytes ([`ROUTE_FEC_PAYLOAD_ID_LEN`]).
//!
//! ⚠ **Repair-flow coverage gap**: the real fixtures this crate is verified
//! against (`fixtures/atsc3/route-*.bin`, see `fixtures/atsc3/PROVENANCE.md`)
//! were captured from a session that ran with the LCT SPI bit set on every
//! single packet (8,885 frames scanned across both capture files) — i.e. no
//! FEC-repair flow was ever active. [`RepairFecPayloadId`] is implemented
//! directly from A/331's own Figure A.3.4 bit-diagram (SBN 8 bits / ESI 24
//! bits, matching RFC 6330 §3.2 exactly — this table was corrected 2026-08-09
//! after independently re-counting the figure's bit ruler; it previously,
//! wrongly, read SBN 16 / ESI 16) and unit-tested against hand-built vectors,
//! but has **no real-capture corroboration**. [`SourceFecPayloadId`] and the
//! SPI-bit dispatch in [`crate::RoutePacket`] *are* real-fixture verified.

use broadcast_common::{Parse, Serialize};

use crate::error::{Error, Result};

/// Wire size in bytes of a ROUTE FEC Payload ID (both layouts): 32 bits.
pub const ROUTE_FEC_PAYLOAD_ID_LEN: usize = 4;

/// Maximum value of the 24-bit Encoding Symbol ID (`ESI`) field.
pub const MAX_ESI: u32 = 0x00FF_FFFF;

// ---------------------------------------------------------------------------
// SourceFecPayloadId — Compact No-Code FEC scheme (§A.3.5.1, Figure A.3.3)
// ---------------------------------------------------------------------------

/// FEC Payload ID for ROUTE source flows (SPI = 1): Compact No-Code FEC
/// scheme, A/331 §A.3.5.1 / Figure A.3.3.
///
/// A single 32-bit unsigned integer: the octet offset, from the first octet
/// of the delivery object, of the first octet of the fragment carried in
/// *this* packet's payload. `0` when the packet carries the entire object
/// (§A.3.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SourceFecPayloadId {
    /// Octet offset of this fragment within the delivery object.
    pub start_offset: u32,
}

impl<'a> Parse<'a> for SourceFecPayloadId {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < ROUTE_FEC_PAYLOAD_ID_LEN {
            return Err(Error::BufferTooShort {
                need: ROUTE_FEC_PAYLOAD_ID_LEN,
                have: bytes.len(),
                what: "source-flow FEC Payload ID (start_offset)",
            });
        }
        Ok(SourceFecPayloadId {
            start_offset: u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        })
    }
}

impl Serialize for SourceFecPayloadId {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        ROUTE_FEC_PAYLOAD_ID_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.len() < ROUTE_FEC_PAYLOAD_ID_LEN {
            return Err(Error::OutputBufferTooSmall {
                need: ROUTE_FEC_PAYLOAD_ID_LEN,
                have: buf.len(),
            });
        }
        buf[0..4].copy_from_slice(&self.start_offset.to_be_bytes());
        Ok(ROUTE_FEC_PAYLOAD_ID_LEN)
    }
}

// ---------------------------------------------------------------------------
// RepairFecPayloadId — RaptorQ FEC scheme (§A.3.5.2, Figure A.3.4, RFC 6330 §3.2)
// ---------------------------------------------------------------------------

/// FEC Payload ID for ROUTE repair flows (SPI = 0): RaptorQ FEC scheme,
/// A/331 §A.3.5.2 / Figure A.3.4, matching RFC 6330 §3.2's `SBN`/`ESI` pair.
///
/// **Not real-fixture verified** — see the module doc's coverage-gap note.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct RepairFecPayloadId {
    /// Source Block Number (8 bits).
    pub sbn: u8,
    /// Encoding Symbol ID (24 bits, `<= `[`MAX_ESI`]).
    pub esi: u32,
}

impl<'a> Parse<'a> for RepairFecPayloadId {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < ROUTE_FEC_PAYLOAD_ID_LEN {
            return Err(Error::BufferTooShort {
                need: ROUTE_FEC_PAYLOAD_ID_LEN,
                have: bytes.len(),
                what: "repair-flow FEC Payload ID (SBN/ESI)",
            });
        }
        let sbn = bytes[0];
        let esi = u32::from_be_bytes([0, bytes[1], bytes[2], bytes[3]]);
        Ok(RepairFecPayloadId { sbn, esi })
    }
}

impl Serialize for RepairFecPayloadId {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        ROUTE_FEC_PAYLOAD_ID_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.len() < ROUTE_FEC_PAYLOAD_ID_LEN {
            return Err(Error::OutputBufferTooSmall {
                need: ROUTE_FEC_PAYLOAD_ID_LEN,
                have: buf.len(),
            });
        }
        if self.esi > MAX_ESI {
            return Err(Error::FieldTooWide {
                what: "ESI",
                value: self.esi as u64,
                bits: 24,
            });
        }
        let esi_bytes = self.esi.to_be_bytes();
        buf[0] = self.sbn;
        buf[1..4].copy_from_slice(&esi_bytes[1..4]);
        Ok(ROUTE_FEC_PAYLOAD_ID_LEN)
    }
}

// ---------------------------------------------------------------------------
// RouteFecPayloadId — SPI-bit dispatch
// ---------------------------------------------------------------------------

/// The FEC Payload ID, decoded per the LCT `PSI` "SPI" bit of the enclosing
/// packet ([`rmt_flute::PSI_SPI`]): [`SourceFecPayloadId`] when SPI = 1,
/// [`RepairFecPayloadId`] when SPI = 0 (A/331 §A.4.2.4).
///
/// This is a dispatch enum selecting between two disjoint wire layouts by an
/// out-of-band bit the caller already knows (the packet's SPI), not itself a
/// spec/field token — like the workspace's `Any*` dispatch enums, it is
/// exempt from the #204 label convention (see `tests/label_coverage.rs`'s
/// SKIP list) and does not implement [`broadcast_common::Parse`] directly:
/// that trait's single-argument `parse(bytes)` signature has no way to carry
/// the SPI bit the format needs. Use [`RouteFecPayloadId::parse`] instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum RouteFecPayloadId {
    /// Source-flow (Compact No-Code) FEC Payload ID.
    Source(SourceFecPayloadId),
    /// Repair-flow (RaptorQ) FEC Payload ID.
    Repair(RepairFecPayloadId),
}

impl RouteFecPayloadId {
    /// Wire size in bytes (always [`ROUTE_FEC_PAYLOAD_ID_LEN`]).
    pub fn serialized_len(&self) -> usize {
        ROUTE_FEC_PAYLOAD_ID_LEN
    }

    /// Decode the FEC Payload ID for a packet whose LCT `PSI` SPI bit is
    /// `spi` (`true` = source flow, `false` = repair flow).
    pub fn parse(bytes: &[u8], spi: bool) -> Result<Self> {
        if spi {
            SourceFecPayloadId::parse(bytes).map(RouteFecPayloadId::Source)
        } else {
            RepairFecPayloadId::parse(bytes).map(RouteFecPayloadId::Repair)
        }
    }

    /// Serialize into `out`. Returns bytes written.
    pub fn serialize_into(&self, out: &mut [u8]) -> Result<usize> {
        match self {
            RouteFecPayloadId::Source(s) => s.serialize_into(out),
            RouteFecPayloadId::Repair(r) => r.serialize_into(out),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_round_trip() {
        let id = SourceFecPayloadId {
            start_offset: 107_008,
        };
        let mut out = [0u8; ROUTE_FEC_PAYLOAD_ID_LEN];
        let n = id.serialize_into(&mut out).unwrap();
        assert_eq!(n, ROUTE_FEC_PAYLOAD_ID_LEN);
        assert_eq!(out, [0x00, 0x01, 0xA2, 0x00]); // 107008 = 0x0001A200
        assert_eq!(SourceFecPayloadId::parse(&out).unwrap(), id);
    }

    #[test]
    fn source_zero_offset_means_whole_object() {
        let id = SourceFecPayloadId { start_offset: 0 };
        let mut out = [0u8; ROUTE_FEC_PAYLOAD_ID_LEN];
        id.serialize_into(&mut out).unwrap();
        assert_eq!(out, [0, 0, 0, 0]);
    }

    #[test]
    fn repair_round_trip() {
        let id = RepairFecPayloadId {
            sbn: 0x07,
            esi: 0x00_ABCDEF,
        };
        let mut out = [0u8; ROUTE_FEC_PAYLOAD_ID_LEN];
        let n = id.serialize_into(&mut out).unwrap();
        assert_eq!(n, ROUTE_FEC_PAYLOAD_ID_LEN);
        assert_eq!(out, [0x07, 0xAB, 0xCD, 0xEF]);
        assert_eq!(RepairFecPayloadId::parse(&out).unwrap(), id);
    }

    #[test]
    fn repair_max_esi_round_trips() {
        let id = RepairFecPayloadId {
            sbn: 0xFF,
            esi: MAX_ESI,
        };
        let mut out = [0u8; ROUTE_FEC_PAYLOAD_ID_LEN];
        id.serialize_into(&mut out).unwrap();
        assert_eq!(out, [0xFF, 0xFF, 0xFF, 0xFF]);
        assert_eq!(RepairFecPayloadId::parse(&out).unwrap(), id);
    }

    #[test]
    fn repair_rejects_overwide_esi() {
        let id = RepairFecPayloadId {
            sbn: 0,
            esi: MAX_ESI + 1,
        };
        let mut out = [0u8; ROUTE_FEC_PAYLOAD_ID_LEN];
        assert!(matches!(
            id.serialize_into(&mut out),
            Err(Error::FieldTooWide { what: "ESI", .. })
        ));
    }

    #[test]
    fn dispatch_picks_layout_from_spi() {
        let source_bytes = [0x00, 0x01, 0xA2, 0x00];
        let repair_bytes = [0x07, 0xAB, 0xCD, 0xEF];

        let src = RouteFecPayloadId::parse(&source_bytes, true).unwrap();
        assert_eq!(
            src,
            RouteFecPayloadId::Source(SourceFecPayloadId {
                start_offset: 107_008
            })
        );

        let rep = RouteFecPayloadId::parse(&repair_bytes, false).unwrap();
        assert_eq!(
            rep,
            RouteFecPayloadId::Repair(RepairFecPayloadId {
                sbn: 0x07,
                esi: 0x00_ABCDEF
            })
        );

        // The SAME bytes decode differently depending only on SPI — proving
        // the dispatch is load-bearing, not a no-op.
        let same_bytes_as_source = RouteFecPayloadId::parse(&source_bytes, false).unwrap();
        assert_ne!(src, same_bytes_as_source);
    }

    #[test]
    fn mutating_start_offset_changes_wire_bytes() {
        let mk = |off: u32| {
            let id = SourceFecPayloadId { start_offset: off };
            let mut out = [0u8; ROUTE_FEC_PAYLOAD_ID_LEN];
            id.serialize_into(&mut out).unwrap();
            out
        };
        assert_ne!(mk(0), mk(1408));
    }
}
