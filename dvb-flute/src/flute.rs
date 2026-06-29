//! FLUTE — File Delivery over Unidirectional Transport (RFC 6726 §3.4).
//!
//! FLUTE is built on ALC ([`crate::alc`]) + LCT ([`crate::lct`]). It adds two
//! fixed-length LCT header extensions — **EXT_FDT** (HET 192) and **EXT_CENC**
//! (HET 193) — and the **TOI = 0** convention for carrying FDT Instances.
//!
//! ⚠ The FDT Instance body itself is an **XML document** and is out of scope of
//! this binary crate (RFC 6726 §3.4.2): it rides as the encoding-symbol payload
//! after the FEC Payload ID. This module covers only the binary LCT/ALC framing
//! extensions; expose the payload bytes and parse the XML in a separate layer.

use crate::error::{Error, Result};
use crate::ext::HeaderExtension;

/// HET for EXT_FDT (FDT Instance Header) — RFC 6726 §3.4.1. Fixed-length.
pub const HET_EXT_FDT: u8 = 192;
/// HET for EXT_CENC (FDT Instance Content Encoding) — RFC 6726 §3.4.3. Fixed.
pub const HET_EXT_CENC: u8 = 193;

/// The reserved TOI value for FDT Instances (RFC 6726 §3.3). FDT Instances are
/// carried in ALC packets with TOI = 0.
pub const TOI_FDT: u32 = 0;

/// FLUTE version carried in EXT_FDT's `V` field (RFC 6726 = 2).
pub const FLUTE_VERSION: u8 = 2;

/// Maximum FDT Instance ID (20-bit field).
pub const FDT_INSTANCE_ID_MAX: u32 = (1 << 20) - 1;

/// EXT_FDT — FDT Instance Header (RFC 6726 §3.4.1, HET = 192, fixed-length).
///
/// Layout (one 32-bit word): `HET(8) | V(4) | FDT Instance ID(20)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ExtFdt {
    /// FLUTE version (`V`, 4 bits). RFC 6726 = [`FLUTE_VERSION`] (2).
    pub version: u8,
    /// FDT Instance ID (20 bits) — identifies the FDT Instance in the session.
    pub instance_id: u32,
}

impl ExtFdt {
    /// Decode from the 3 content bytes of a fixed-length [`HeaderExtension`]
    /// whose HET is [`HET_EXT_FDT`] (the 24 bits after the HET byte).
    pub fn parse(content: &[u8]) -> Result<Self> {
        if content.len() != 3 {
            return Err(Error::BufferTooShort {
                need: 3,
                have: content.len(),
                what: "EXT_FDT content",
            });
        }
        // 24 bits: V(4) | instance_id(20).
        let v = content[0] >> 4;
        let instance_id =
            ((content[0] as u32 & 0x0F) << 16) | ((content[1] as u32) << 8) | content[2] as u32;
        Ok(ExtFdt {
            version: v,
            instance_id,
        })
    }

    /// Encode the 3 content bytes (the 24 bits after HET).
    pub fn to_content(&self) -> Result<[u8; 3]> {
        if self.version > 0x0F {
            return Err(Error::FieldTooWide {
                what: "EXT_FDT V",
                value: self.version as u64,
                bits: 4,
            });
        }
        if self.instance_id > FDT_INSTANCE_ID_MAX {
            return Err(Error::FieldTooWide {
                what: "FDT Instance ID",
                value: self.instance_id as u64,
                bits: 20,
            });
        }
        Ok([
            (self.version << 4) | ((self.instance_id >> 16) as u8 & 0x0F),
            (self.instance_id >> 8) as u8,
            self.instance_id as u8,
        ])
    }

    /// Build a fixed-length [`HeaderExtension`] (HET = 192) for this EXT_FDT,
    /// writing the 3 content bytes into `scratch`.
    pub fn to_extension<'a>(&self, scratch: &'a mut [u8; 3]) -> Result<HeaderExtension<'a>> {
        *scratch = self.to_content()?;
        Ok(HeaderExtension::new(HET_EXT_FDT, &scratch[..]))
    }
}

/// Content-encoding algorithm of an FDT Instance payload (RFC 6726 §3.4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum CencAlgorithm {
    /// 0 — null (no content encoding).
    Null,
    /// 1 — ZLIB (RFC 1950).
    Zlib,
    /// 2 — DEFLATE (RFC 1951).
    Deflate,
    /// 3 — GZIP (RFC 1952).
    Gzip,
    /// Any other (unassigned) value.
    Other(u8),
}

impl CencAlgorithm {
    /// Decode a CENC algorithm byte.
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => CencAlgorithm::Null,
            1 => CencAlgorithm::Zlib,
            2 => CencAlgorithm::Deflate,
            3 => CencAlgorithm::Gzip,
            other => CencAlgorithm::Other(other),
        }
    }

    /// The wire byte for this algorithm.
    pub fn to_u8(self) -> u8 {
        match self {
            CencAlgorithm::Null => 0,
            CencAlgorithm::Zlib => 1,
            CencAlgorithm::Deflate => 2,
            CencAlgorithm::Gzip => 3,
            CencAlgorithm::Other(v) => v,
        }
    }

    /// Spec label.
    pub fn name(&self) -> &'static str {
        match self {
            CencAlgorithm::Null => "null",
            CencAlgorithm::Zlib => "ZLIB",
            CencAlgorithm::Deflate => "DEFLATE",
            CencAlgorithm::Gzip => "GZIP",
            CencAlgorithm::Other(_) => "reserved",
        }
    }
}

broadcast_common::impl_spec_display!(CencAlgorithm, Other);

/// EXT_CENC — FDT Instance Content Encoding Header (RFC 6726 §3.4.3, HET = 193,
/// fixed-length).
///
/// Layout (one 32-bit word): `HET(8) | CENC(8) | Reserved(16)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ExtCenc {
    /// Content-encoding algorithm of the FDT Instance payload.
    pub algorithm: CencAlgorithm,
}

impl ExtCenc {
    /// Decode from the 3 content bytes of a fixed-length [`HeaderExtension`]
    /// whose HET is [`HET_EXT_CENC`]: `CENC(8) | Reserved(16)`.
    pub fn parse(content: &[u8]) -> Result<Self> {
        if content.len() != 3 {
            return Err(Error::BufferTooShort {
                need: 3,
                have: content.len(),
                what: "EXT_CENC content",
            });
        }
        // Reserved 16 bits MUST be 0 (ignored on reception, but we surface it).
        Ok(ExtCenc {
            algorithm: CencAlgorithm::from_u8(content[0]),
        })
    }

    /// Encode the 3 content bytes (`CENC | Reserved=0`).
    pub fn to_content(&self) -> [u8; 3] {
        [self.algorithm.to_u8(), 0, 0]
    }

    /// Build a fixed-length [`HeaderExtension`] (HET = 193) for this EXT_CENC,
    /// writing the 3 content bytes into `scratch`.
    pub fn to_extension<'a>(&self, scratch: &'a mut [u8; 3]) -> HeaderExtension<'a> {
        *scratch = self.to_content();
        HeaderExtension::new(HET_EXT_CENC, &scratch[..])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn ext_fdt_round_trip() {
        let f = ExtFdt {
            version: FLUTE_VERSION,
            instance_id: 0x0_ABCD,
        };
        let c = f.to_content().unwrap();
        // V=2 (0x2_), instance_id high nibble = 0x0 -> 0x20; then 0xAB, 0xCD.
        assert_eq!(c, [0x20, 0xAB, 0xCD]);
        assert_eq!(ExtFdt::parse(&c).unwrap(), f);

        // As an extension: HET=192 (fixed), 4 bytes total.
        let mut scratch = [0u8; 3];
        let ext = f.to_extension(&mut scratch).unwrap();
        assert_eq!(ext.het, HET_EXT_FDT);
        assert!(ext.is_fixed());
        assert_eq!(ext.serialized_len(), 4);
    }

    #[test]
    fn ext_fdt_max_instance_id() {
        let f = ExtFdt {
            version: 2,
            instance_id: FDT_INSTANCE_ID_MAX,
        };
        let c = f.to_content().unwrap();
        assert_eq!(c, [0x2F, 0xFF, 0xFF]);
        assert_eq!(ExtFdt::parse(&c).unwrap(), f);
    }

    #[test]
    fn ext_fdt_rejects_overwide_instance_id() {
        let f = ExtFdt {
            version: 2,
            instance_id: FDT_INSTANCE_ID_MAX + 1,
        };
        assert!(matches!(f.to_content(), Err(Error::FieldTooWide { .. })));
    }

    #[test]
    fn ext_cenc_round_trip() {
        for algo in [
            CencAlgorithm::Null,
            CencAlgorithm::Zlib,
            CencAlgorithm::Deflate,
            CencAlgorithm::Gzip,
            CencAlgorithm::Other(7),
        ] {
            let e = ExtCenc { algorithm: algo };
            let c = e.to_content();
            assert_eq!(c[1..], [0, 0]);
            assert_eq!(ExtCenc::parse(&c).unwrap(), e);
        }
        assert_eq!(CencAlgorithm::Gzip.to_string(), "GZIP");
        assert_eq!(CencAlgorithm::Other(7).to_string(), "reserved(0x07)");
    }
}
