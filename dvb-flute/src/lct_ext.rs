//! LCT Header-Extension Type registry + the EXT_TIME typed extension
//! (RFC 5651 §5.2.1, §5.2.2, §9.2).

use alloc::vec::Vec;

use crate::error::{Error, Result};
use crate::ext::{HeaderExtension, WORD};

/// HET for EXT_NOP (No-Operation) — RFC 5651 §5.2.1.
pub const HET_EXT_NOP: u8 = 0;
/// HET for EXT_AUTH (Packet Authentication) — RFC 5651 §5.2.1.
pub const HET_EXT_AUTH: u8 = 1;
/// HET for EXT_TIME (Timing information) — RFC 5651 §5.2.2.
pub const HET_EXT_TIME: u8 = 2;

/// A known LCT Header Extension Type (RFC 5651 §5.2.1 / §9.2).
///
/// Covers the three base LCT-defined HET values; protocol-instantiation HETs
/// (`EXT_FTI` 64, `EXT_FDT` 192, `EXT_CENC` 193, …) live in their own modules
/// and fall under [`LctExtType::Other`] here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum LctExtType {
    /// EXT_NOP (HET 0) — content ignored by receivers.
    Nop,
    /// EXT_AUTH (HET 1) — packet authentication; format out-of-band.
    Auth,
    /// EXT_TIME (HET 2) — timing info (SCT/ERT/SLC).
    Time,
    /// Any other HET value (protocol-instantiation or unassigned).
    Other(u8),
}

impl LctExtType {
    /// Decode a HET value.
    pub fn from_het(het: u8) -> Self {
        match het {
            HET_EXT_NOP => LctExtType::Nop,
            HET_EXT_AUTH => LctExtType::Auth,
            HET_EXT_TIME => LctExtType::Time,
            other => LctExtType::Other(other),
        }
    }

    /// The HET value for this type.
    pub fn het(self) -> u8 {
        match self {
            LctExtType::Nop => HET_EXT_NOP,
            LctExtType::Auth => HET_EXT_AUTH,
            LctExtType::Time => HET_EXT_TIME,
            LctExtType::Other(v) => v,
        }
    }

    /// Spec label.
    pub fn name(&self) -> &'static str {
        match self {
            LctExtType::Nop => "EXT_NOP",
            LctExtType::Auth => "EXT_AUTH",
            LctExtType::Time => "EXT_TIME",
            LctExtType::Other(_) => "other",
        }
    }
}

broadcast_common::impl_spec_display!(LctExtType, Other);

// EXT_TIME Use-field bit masks (RFC 5651 §5.2.2, Figure 4). The Use field is
// the low 16 bits of the first 32-bit word; SCT-High is the MSB.
/// Use-field bit: Sender Current Time, high 32 bits present.
pub const USE_SCT_HIGH: u16 = 0x8000;
/// Use-field bit: Sender Current Time, low 32 bits present.
pub const USE_SCT_LOW: u16 = 0x4000;
/// Use-field bit: Expected Residual Time present.
pub const USE_ERT: u16 = 0x2000;
/// Use-field bit: Session Last Changed time present.
pub const USE_SLC: u16 = 0x1000;

// EXT_TIME Use-field sub-masks (RFC 5651 §5.2.2).
/// Mask for the PI-specific (protocol-instantiation) low 8 bits of the Use field.
const USE_PI_SPECIFIC_MASK: u16 = 0x00FF;
/// Mask for the reserved-by-LCT bits in the Use field (bits 8..=11).
const USE_RESERVED_MASK: u16 = 0x0F00;

/// A decoded EXT_TIME header extension (RFC 5651 §5.2.2, HET = 2).
///
/// Carries 0..4 32-bit time values selected by the 16-bit `Use` bit field. When
/// present they appear in the fixed order SCT-High, SCT-Low, ERT, SLC; each
/// `Some` value contributes one 32-bit word. The PI-specific low 8 bits of the
/// `Use` field are preserved verbatim in `pi_specific`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ExtTime {
    /// Sender Current Time, MS 32 bits (NTP seconds).
    pub sct_high: Option<u32>,
    /// Sender Current Time, LS 32 bits (NTP fraction). If set, `sct_high` MUST
    /// also be set.
    pub sct_low: Option<u32>,
    /// Expected Residual Time, seconds.
    pub ert: Option<u32>,
    /// Session Last Changed time, seconds.
    pub slc: Option<u32>,
    /// PI-specific low 8 bits of the Use field (out of scope of RFC 5651).
    pub pi_specific: u8,
}

impl ExtTime {
    /// Build the 16-bit Use field from the present values + PI-specific byte.
    pub fn use_field(&self) -> u16 {
        let mut u = self.pi_specific as u16;
        if self.sct_high.is_some() {
            u |= USE_SCT_HIGH;
        }
        if self.sct_low.is_some() {
            u |= USE_SCT_LOW;
        }
        if self.ert.is_some() {
            u |= USE_ERT;
        }
        if self.slc.is_some() {
            u |= USE_SLC;
        }
        u
    }

    /// Number of 32-bit time values that follow the first word.
    fn value_count(&self) -> usize {
        self.sct_high.is_some() as usize
            + self.sct_low.is_some() as usize
            + self.ert.is_some() as usize
            + self.slc.is_some() as usize
    }

    /// Total serialized length in bytes (first word + 4 bytes per value).
    pub fn serialized_len(&self) -> usize {
        WORD + WORD * self.value_count()
    }

    /// Decode an EXT_TIME from the *content* of a [`HeaderExtension`] whose HET
    /// is [`HET_EXT_TIME`] (the content is everything after HET+HEL: the 2-byte
    /// Use field followed by the time values).
    pub fn parse(content: &[u8]) -> Result<Self> {
        if content.len() < 2 {
            return Err(Error::BufferTooShort {
                need: 2,
                have: content.len(),
                what: "EXT_TIME Use field",
            });
        }
        let use_field = u16::from_be_bytes([content[0], content[1]]);
        let pi_specific = (use_field & USE_PI_SPECIFIC_MASK) as u8;
        // Reserved-by-LCT bits (Use & USE_RESERVED_MASK) MUST be 0.
        if use_field & USE_RESERVED_MASK != 0 {
            return Err(Error::InvalidField {
                what: "EXT_TIME Use reserved",
                reason: "reserved-by-LCT Use bits must be zero",
            });
        }
        if (use_field & USE_SCT_LOW != 0) && (use_field & USE_SCT_HIGH == 0) {
            return Err(Error::InvalidField {
                what: "EXT_TIME Use",
                reason: "SCT-Low set without SCT-High",
            });
        }

        let mut off = 2;
        let mut take = |present: bool| -> Result<Option<u32>> {
            if !present {
                return Ok(None);
            }
            if content.len() < off + WORD {
                return Err(Error::BufferTooShort {
                    need: off + WORD,
                    have: content.len(),
                    what: "EXT_TIME time value",
                });
            }
            let v = u32::from_be_bytes([
                content[off],
                content[off + 1],
                content[off + 2],
                content[off + 3],
            ]);
            off += WORD;
            Ok(Some(v))
        };
        let sct_high = take(use_field & USE_SCT_HIGH != 0)?;
        let sct_low = take(use_field & USE_SCT_LOW != 0)?;
        let ert = take(use_field & USE_ERT != 0)?;
        let slc = take(use_field & USE_SLC != 0)?;

        Ok(ExtTime {
            sct_high,
            sct_low,
            ert,
            slc,
            pi_specific,
        })
    }

    /// Encode the EXT_TIME content (Use field + present time values) into a
    /// freshly allocated buffer suitable for [`HeaderExtension::content`].
    pub fn to_content(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.serialized_len());
        out.extend_from_slice(&self.use_field().to_be_bytes());
        // Pad the Use field's word to 4 bytes? No — RFC 5651 packs the Use into
        // the same word as HET+HEL; the content here starts at the Use field
        // and the leading 2 HET/HEL bytes belong to the HeaderExtension. So the
        // first word is HET|HEL|Use, and our content begins at Use (2 bytes),
        // then 4 bytes per value — total content = 2 + 4*n, +2 (HET/HEL) = 4*(n+1).
        for v in [self.sct_high, self.sct_low, self.ert, self.slc]
            .into_iter()
            .flatten()
        {
            out.extend_from_slice(&v.to_be_bytes());
        }
        out
    }

    /// Build a generic [`HeaderExtension`] (HET = 2) carrying this EXT_TIME,
    /// borrowing from `scratch` (which must outlive the returned extension).
    pub fn to_extension<'a>(&self, scratch: &'a mut Vec<u8>) -> HeaderExtension<'a> {
        *scratch = self.to_content();
        HeaderExtension::new(HET_EXT_TIME, scratch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;
    use alloc::vec;

    #[test]
    fn ext_type_round_trip() {
        for het in [0u8, 1, 2, 64, 192, 255] {
            assert_eq!(LctExtType::from_het(het).het(), het);
        }
        assert_eq!(LctExtType::from_het(64), LctExtType::Other(64));
        assert_eq!(LctExtType::Time.to_string(), "EXT_TIME");
        assert_eq!(LctExtType::Other(64).to_string(), "other(0x40)");
    }

    #[test]
    fn ext_time_sct_high_low_round_trip() {
        let t = ExtTime {
            sct_high: Some(0x1122_3344),
            sct_low: Some(0x5566_7788),
            ert: None,
            slc: None,
            pi_specific: 0,
        };
        // Use = SCT_HIGH | SCT_LOW = 0xC000. Content = 2 + 8 = 10 bytes.
        assert_eq!(t.use_field(), 0xC000);
        let content = t.to_content();
        assert_eq!(content.len(), 10);
        assert_eq!(&content[0..2], &[0xC0, 0x00]);
        assert_eq!(&content[2..6], &[0x11, 0x22, 0x33, 0x44]);
        assert_eq!(&content[6..10], &[0x55, 0x66, 0x77, 0x88]);

        let re = ExtTime::parse(&content).unwrap();
        assert_eq!(re, t);

        // As a whole extension: HET=2, HEL = (2 + 2 + 8)/4 = 3.
        let mut scratch = vec![];
        let ext = t.to_extension(&mut scratch);
        assert_eq!(ext.het, 2);
        assert_eq!(ext.serialized_len(), 12);
        assert_eq!(ext.hel(), 3);
    }

    #[test]
    fn ext_time_all_four_values_in_order() {
        let t = ExtTime {
            sct_high: Some(1),
            sct_low: Some(2),
            ert: Some(3),
            slc: Some(4),
            pi_specific: 0xAB,
        };
        assert_eq!(t.use_field(), 0xF000 | 0x00AB);
        let content = t.to_content();
        let re = ExtTime::parse(&content).unwrap();
        assert_eq!(re, t);
        // Values follow in order.
        assert_eq!(&content[2..6], &1u32.to_be_bytes());
        assert_eq!(&content[6..10], &2u32.to_be_bytes());
        assert_eq!(&content[10..14], &3u32.to_be_bytes());
        assert_eq!(&content[14..18], &4u32.to_be_bytes());
    }

    #[test]
    fn ext_time_rejects_sct_low_without_high() {
        // Use = SCT_LOW only (0x4000) + one value.
        let content = [0x40u8, 0x00, 0x00, 0x00, 0x00, 0x01];
        assert!(matches!(
            ExtTime::parse(&content),
            Err(Error::InvalidField { .. })
        ));
    }
}
