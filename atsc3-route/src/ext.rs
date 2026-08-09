//! ROUTE-specific LCT header extensions — A/331 Annex A §A.3.7
//! (`EXT_ROUTE_PRESENTATION_TIME`) and §A.3.8 (`EXT_TOL`), transcribed at
//! `atsc3/docs/a331-route.md` §2. Neither extension exists in `rmt-flute`
//! today — that crate stops at the generic RFC 5651 §5.2 extension-chain
//! shape ([`rmt_flute::HeaderExtension`]) plus the base LCT/ALC/FLUTE
//! extensions (`EXT_TIME`, `EXT_FTI`, `EXT_FDT`, `EXT_CENC`).
//!
//! Both types here decode/encode only the extension **content** (the bytes
//! after `HET`, and after `HEL` for the variable-length form) — the same
//! split [`rmt_flute::ExtFdt`]/[`rmt_flute::ExtCenc`]/[`rmt_flute::ExtTime`]
//! use, so a caller wraps the result in a
//! [`rmt_flute::HeaderExtension`] to splice it into an
//! [`rmt_flute::LctHeader::extensions`] chain via `to_extension`.

use broadcast_common::{Parse, Serialize};
use rmt_flute::HeaderExtension;

use crate::error::{Error, Result};

// ---------------------------------------------------------------------------
// EXT_ROUTE_PRESENTATION_TIME — §A.3.7.1, Figure A.3.5 (HET = 66)
// ---------------------------------------------------------------------------

/// HET for `EXT_ROUTE_PRESENTATION_TIME` (§A.3.7.1). Variable-length
/// extension (HET < 128, carries `HEL`).
pub const HET_EXT_ROUTE_PRESENTATION_TIME: u8 = 66;

/// Content length in bytes of `EXT_ROUTE_PRESENTATION_TIME`: `reserved`(16
/// bits) + NTP timestamp high word(32) + low word(32) = 10 bytes. Together
/// with `HET`+`HEL` (2 bytes) this is the extension's documented 12-byte
/// total (Figure A.3.5).
pub const EXT_ROUTE_PRESENTATION_TIME_CONTENT_LEN: usize = 10;

/// `EXT_ROUTE_PRESENTATION_TIME` — the full 64-bit NTP presentation time of
/// an MDE (Media Delivery Event) Random Access Point (§A.3.7.1).
///
/// Present **only** in the first LCT packet of an MDE data block containing a
/// Random Access Point; its presence at all is the indicator that MDE mode is
/// in use for the stream. A/331 requires any packet carrying this extension
/// to *also* carry RFC 5651's `EXT_TIME` ([`rmt_flute::ExtTime`], HET = 2)
/// with both `SCT-High`/`SCT-Low` set (§A.3.7.2) — that companion requirement
/// is documentation, not enforced by this type (it has no visibility into
/// sibling extensions in the chain; a caller composing the full chain is
/// responsible for it).
///
/// The `reserved` 16 bits (Figure A.3.5) are accepted as any value on parse
/// (ignored, per this crate's judgement call — A/331 does not state a "shall
/// be zero" constraint for this field, unlike some other reserved fields) and
/// always written as zero on serialize.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ExtRoutePresentationTime {
    /// The full 64-bit NTP timestamp (high word << 32 | low word) of the
    /// presentation time. Must be greater than the companion `EXT_TIME`'s SCT
    /// value (§A.3.7.2) — not validated here (cross-extension constraint).
    pub ntp_timestamp: u64,
}

impl<'a> Parse<'a> for ExtRoutePresentationTime {
    type Error = Error;

    /// Decode from the *content* of a [`HeaderExtension`] whose `HET` is
    /// [`HET_EXT_ROUTE_PRESENTATION_TIME`] (`reserved`(16) | NTP high(32) |
    /// NTP low(32)).
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() != EXT_ROUTE_PRESENTATION_TIME_CONTENT_LEN {
            return Err(Error::BufferTooShort {
                need: EXT_ROUTE_PRESENTATION_TIME_CONTENT_LEN,
                have: bytes.len(),
                what: "EXT_ROUTE_PRESENTATION_TIME content",
            });
        }
        let high = u32::from_be_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]);
        let low = u32::from_be_bytes([bytes[6], bytes[7], bytes[8], bytes[9]]);
        Ok(ExtRoutePresentationTime {
            ntp_timestamp: (u64::from(high) << 32) | u64::from(low),
        })
    }
}

impl Serialize for ExtRoutePresentationTime {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        EXT_ROUTE_PRESENTATION_TIME_CONTENT_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.len() < EXT_ROUTE_PRESENTATION_TIME_CONTENT_LEN {
            return Err(Error::OutputBufferTooSmall {
                need: EXT_ROUTE_PRESENTATION_TIME_CONTENT_LEN,
                have: buf.len(),
            });
        }
        buf[0] = 0;
        buf[1] = 0;
        let high = (self.ntp_timestamp >> 32) as u32;
        let low = self.ntp_timestamp as u32;
        buf[2..6].copy_from_slice(&high.to_be_bytes());
        buf[6..10].copy_from_slice(&low.to_be_bytes());
        Ok(EXT_ROUTE_PRESENTATION_TIME_CONTENT_LEN)
    }
}

impl ExtRoutePresentationTime {
    /// Build a variable-length [`HeaderExtension`] (`HET` =
    /// [`HET_EXT_ROUTE_PRESENTATION_TIME`]) carrying this extension, writing
    /// its content into `scratch`.
    pub fn to_extension<'a>(
        &self,
        scratch: &'a mut [u8; EXT_ROUTE_PRESENTATION_TIME_CONTENT_LEN],
    ) -> Result<HeaderExtension<'a>> {
        self.serialize_into(scratch)?;
        Ok(HeaderExtension::new(
            HET_EXT_ROUTE_PRESENTATION_TIME,
            &scratch[..],
        ))
    }
}

// ---------------------------------------------------------------------------
// EXT_TOL — Transport Object Length (§A.3.8.1, Figures A.3.6/A.3.7)
// ---------------------------------------------------------------------------

/// HET for the 24-bit form of `EXT_TOL` (§A.3.8.1, Figure A.3.6). Fixed-length
/// extension (one 32-bit word, no `HEL`).
pub const HET_EXT_TOL_24: u8 = 194;
/// HET for the 48-bit form of `EXT_TOL` (§A.3.8.1, Figure A.3.7).
/// Variable-length extension (`HEL` = 2, two 32-bit words).
pub const HET_EXT_TOL_48: u8 = 67;

/// Maximum value representable by the 24-bit `EXT_TOL` form.
pub const MAX_TOL_24: u32 = 0x00FF_FFFF;
/// Maximum value representable by the 48-bit `EXT_TOL` form.
pub const MAX_TOL_48: u64 = 0x0000_FFFF_FFFF_FFFF;

/// Content length in bytes of the 24-bit `EXT_TOL` form.
pub const EXT_TOL_24_CONTENT_LEN: usize = 3;
/// Content length in bytes of the 48-bit `EXT_TOL` form.
pub const EXT_TOL_48_CONTENT_LEN: usize = 6;

/// `EXT_TOL` — Transport Object Length (§A.3.8.1): the delivery object's
/// transfer length *after* any content encoding (e.g. gzip), learned via an
/// LCT extension instead of (or alongside) RFC 5775's `EXT_FTI`
/// ([`rmt_flute::AlcPacket`]'s `EXT_FTI`, HET 64).
///
/// Two width variants exist, distinguished by `HET` rather than by a flag
/// bit — [`ExtTol::Bits24`] ([`HET_EXT_TOL_24`], fixed-length) and
/// [`ExtTol::Bits48`] ([`HET_EXT_TOL_48`], variable-length). §A.3.8.1: "when
/// EXT_FTI is not present, then either the 24-bit or 48-bit version of
/// EXT_TOL should be present" — i.e. at most one length-signalling mechanism
/// is expected per delivery object (a "should", not a "shall"; not enforced
/// here since it is a cross-extension/cross-packet constraint).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ExtTol {
    /// 24-bit Transfer Length (`HET` = [`HET_EXT_TOL_24`], `<= `[`MAX_TOL_24`]).
    Bits24(u32),
    /// 48-bit Transfer Length (`HET` = [`HET_EXT_TOL_48`], `<= `[`MAX_TOL_48`]).
    Bits48(u64),
}

impl ExtTol {
    /// Spec label for this width variant.
    pub fn name(&self) -> &'static str {
        match self {
            ExtTol::Bits24(_) => "EXT_TOL (24-bit)",
            ExtTol::Bits48(_) => "EXT_TOL (48-bit)",
        }
    }

    /// The `HET` value this variant is carried under.
    pub fn het(&self) -> u8 {
        match self {
            ExtTol::Bits24(_) => HET_EXT_TOL_24,
            ExtTol::Bits48(_) => HET_EXT_TOL_48,
        }
    }

    /// The Transfer Length value, widened to `u64` regardless of variant.
    pub fn transfer_length(&self) -> u64 {
        match self {
            ExtTol::Bits24(v) => u64::from(*v),
            ExtTol::Bits48(v) => *v,
        }
    }

    /// Content length in bytes for this variant (excludes `HET`/`HEL`).
    pub fn content_len(&self) -> usize {
        match self {
            ExtTol::Bits24(_) => EXT_TOL_24_CONTENT_LEN,
            ExtTol::Bits48(_) => EXT_TOL_48_CONTENT_LEN,
        }
    }

    /// Decode `EXT_TOL` content for a given `het` (must be
    /// [`HET_EXT_TOL_24`] or [`HET_EXT_TOL_48`] — the two forms are
    /// distinguished by which `HET` the caller already matched in the
    /// extension chain, not inferred from `content` alone, so this takes
    /// `het` explicitly rather than implementing the single-argument
    /// [`broadcast_common::Parse`] trait).
    pub fn parse(het: u8, content: &[u8]) -> Result<Self> {
        match het {
            HET_EXT_TOL_24 => {
                if content.len() != EXT_TOL_24_CONTENT_LEN {
                    return Err(Error::BufferTooShort {
                        need: EXT_TOL_24_CONTENT_LEN,
                        have: content.len(),
                        what: "EXT_TOL (24-bit) content",
                    });
                }
                let v = u32::from_be_bytes([0, content[0], content[1], content[2]]);
                Ok(ExtTol::Bits24(v))
            }
            HET_EXT_TOL_48 => {
                if content.len() != EXT_TOL_48_CONTENT_LEN {
                    return Err(Error::BufferTooShort {
                        need: EXT_TOL_48_CONTENT_LEN,
                        have: content.len(),
                        what: "EXT_TOL (48-bit) content",
                    });
                }
                // 48 bits split across two 32-bit words: 16 bits in word 1,
                // 32 bits in word 2 (Figure A.3.7).
                let high16 = u16::from_be_bytes([content[0], content[1]]);
                let low32 = u32::from_be_bytes([content[2], content[3], content[4], content[5]]);
                let v = (u64::from(high16) << 32) | u64::from(low32);
                Ok(ExtTol::Bits48(v))
            }
            _ => Err(Error::InvalidField {
                what: "EXT_TOL HET",
                reason: "must be 194 (24-bit form) or 67 (48-bit form)",
            }),
        }
    }

    /// Encode the extension content into `out` (must be at least
    /// [`Self::content_len`] bytes). Returns bytes written.
    pub fn serialize_content_into(&self, out: &mut [u8]) -> Result<usize> {
        let need = self.content_len();
        if out.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: out.len(),
            });
        }
        match self {
            ExtTol::Bits24(v) => {
                if *v > MAX_TOL_24 {
                    return Err(Error::FieldTooWide {
                        what: "EXT_TOL (24-bit) Transfer Length",
                        value: u64::from(*v),
                        bits: 24,
                    });
                }
                let b = v.to_be_bytes();
                out[0..3].copy_from_slice(&b[1..4]);
            }
            ExtTol::Bits48(v) => {
                if *v > MAX_TOL_48 {
                    return Err(Error::FieldTooWide {
                        what: "EXT_TOL (48-bit) Transfer Length",
                        value: *v,
                        bits: 48,
                    });
                }
                let high16 = (*v >> 32) as u16;
                let low32 = *v as u32;
                out[0..2].copy_from_slice(&high16.to_be_bytes());
                out[2..6].copy_from_slice(&low32.to_be_bytes());
            }
        }
        Ok(need)
    }

    /// Build a [`HeaderExtension`] (fixed-length for [`ExtTol::Bits24`],
    /// variable-length for [`ExtTol::Bits48`]) carrying this `EXT_TOL`,
    /// writing its content into `scratch`.
    pub fn to_extension<'a>(
        &self,
        scratch: &'a mut [u8; EXT_TOL_48_CONTENT_LEN],
    ) -> Result<HeaderExtension<'a>> {
        let n = self.serialize_content_into(scratch)?;
        Ok(HeaderExtension::new(self.het(), &scratch[..n]))
    }
}

broadcast_common::impl_spec_display!(ExtTol, Bits24, Bits48);

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn presentation_time_round_trip() {
        let t = ExtRoutePresentationTime {
            ntp_timestamp: 0x1122_3344_5566_7788,
        };
        let mut content = [0u8; EXT_ROUTE_PRESENTATION_TIME_CONTENT_LEN];
        let n = t.serialize_into(&mut content).unwrap();
        assert_eq!(n, EXT_ROUTE_PRESENTATION_TIME_CONTENT_LEN);
        assert_eq!(&content[0..2], &[0, 0]); // reserved
        assert_eq!(&content[2..6], &[0x11, 0x22, 0x33, 0x44]);
        assert_eq!(&content[6..10], &[0x55, 0x66, 0x77, 0x88]);
        assert_eq!(ExtRoutePresentationTime::parse(&content).unwrap(), t);

        let mut scratch = [0u8; EXT_ROUTE_PRESENTATION_TIME_CONTENT_LEN];
        let ext = t.to_extension(&mut scratch).unwrap();
        assert_eq!(ext.het, HET_EXT_ROUTE_PRESENTATION_TIME);
        assert!(!ext.is_fixed());
        // HET+HEL(2) + content(10) = 12 bytes total (Figure A.3.5).
        assert_eq!(ext.serialized_len(), 12);
    }

    #[test]
    fn presentation_time_reserved_bits_ignored_on_parse() {
        let mut content = [0u8; EXT_ROUTE_PRESENTATION_TIME_CONTENT_LEN];
        content[0] = 0xFF;
        content[1] = 0xFF;
        content[2..6].copy_from_slice(&1u32.to_be_bytes());
        content[6..10].copy_from_slice(&2u32.to_be_bytes());
        let t = ExtRoutePresentationTime::parse(&content).unwrap();
        assert_eq!(t.ntp_timestamp, (1u64 << 32) | 2);
    }

    #[test]
    fn ext_tol_24_round_trip() {
        let t = ExtTol::Bits24(0x00_ABCDEF);
        let mut scratch = [0u8; EXT_TOL_48_CONTENT_LEN];
        let ext = t.to_extension(&mut scratch).unwrap();
        assert_eq!(ext.het, HET_EXT_TOL_24);
        assert!(ext.is_fixed());
        assert_eq!(ext.content, &[0xAB, 0xCD, 0xEF]);
        assert_eq!(ext.serialized_len(), 4); // one word

        let re = ExtTol::parse(HET_EXT_TOL_24, ext.content).unwrap();
        assert_eq!(re, t);
        assert_eq!(re.transfer_length(), 0x00_ABCDEF);
    }

    #[test]
    fn ext_tol_48_round_trip() {
        // Two 32-bit words: 16 bits in word 1, 32 in word 2.
        let t = ExtTol::Bits48(0x0000_ABCD_1234_5678);
        let mut scratch = [0u8; EXT_TOL_48_CONTENT_LEN];
        let ext = t.to_extension(&mut scratch).unwrap();
        assert_eq!(ext.het, HET_EXT_TOL_48);
        assert!(!ext.is_fixed());
        assert_eq!(ext.content, &[0xAB, 0xCD, 0x12, 0x34, 0x56, 0x78]);
        assert_eq!(ext.serialized_len(), 8); // HEL=2, two words

        let re = ExtTol::parse(HET_EXT_TOL_48, ext.content).unwrap();
        assert_eq!(re, t);
        assert_eq!(re.transfer_length(), 0x0000_ABCD_1234_5678);
    }

    #[test]
    fn ext_tol_24_rejects_overwide_value() {
        let t = ExtTol::Bits24(MAX_TOL_24 + 1);
        let mut scratch = [0u8; EXT_TOL_48_CONTENT_LEN];
        assert!(matches!(
            t.to_extension(&mut scratch),
            Err(Error::FieldTooWide { .. })
        ));
    }

    #[test]
    fn ext_tol_rejects_unknown_het() {
        let content = [0u8; 3];
        assert!(matches!(
            ExtTol::parse(0x99, &content),
            Err(Error::InvalidField { .. })
        ));
    }

    #[test]
    fn display_uses_name() {
        assert_eq!(ExtTol::Bits24(1).to_string(), "EXT_TOL (24-bit)(0x01)");
        let s = ExtTol::Bits48(1).to_string();
        assert!(s.starts_with("EXT_TOL (48-bit)(0x"));
    }

    #[test]
    fn mutating_ntp_timestamp_changes_wire() {
        let mk = |ts: u64| {
            let t = ExtRoutePresentationTime { ntp_timestamp: ts };
            let mut out = [0u8; EXT_ROUTE_PRESENTATION_TIME_CONTENT_LEN];
            t.serialize_into(&mut out).unwrap();
            out
        };
        assert_ne!(mk(1), mk(2));
    }
}
