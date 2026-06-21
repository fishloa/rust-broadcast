//! ULE Type field interpretation — RFC 4326 §4.4 / §5, RFC 5163 §3.
//!
//! The 16-bit Type field of an SNDU base header (or of a chained extension
//! header) is split at decimal `1536` (`0x0600`):
//!
//! - `Type < 0x0600` — a Next-Header (Extension Header): a 5-bit zero prefix,
//!   a 3-bit `H-LEN`, and an 8-bit `H-Type` (§5, Figure 7).
//! - `Type >= 0x0600` — an EtherType for the carried PDU (§4.4.2).

/// The boundary between Next-Header codes and EtherTypes (RFC 4326 §4.4):
/// decimal 1536. Values below are Next-Headers; values at or above are
/// EtherTypes.
pub const ETHERTYPE_BOUNDARY: u16 = 0x0600;

/// IPv4 EtherType (RFC 4326 §4.7.2).
pub const ETHERTYPE_IPV4: u16 = 0x0800;
/// IPv6 EtherType (RFC 4326 §4.7.3).
pub const ETHERTYPE_IPV6: u16 = 0x86DD;

/// A decoded ULE Type field (RFC 4326 §4.4).
///
/// `Type < 0x0600` is a [`TypeField::NextHeader`] (the start of an extension
/// header); `Type >= 0x0600` is a [`TypeField::EtherType`] naming the PDU that
/// directly follows the Type field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum TypeField {
    /// A Next-Header: `H-LEN` (3 bits) + `H-Type` (8 bits), with the 5-bit
    /// zero prefix that keeps the raw value `< 0x0600`.
    NextHeader {
        /// 3-bit length/class selector (RFC 4326 §5). `0` = Mandatory; `1..=5`
        /// = Optional (total extension bytes = `2 * h_len`).
        h_len: u8,
        /// 8-bit extension-header type code.
        h_type: u8,
    },
    /// An EtherType (`>= 0x0600`) of the carried PDU (RFC 4326 §4.4.2).
    EtherType(u16),
}

impl TypeField {
    /// Decode a raw 16-bit Type-field value.
    pub fn from_u16(raw: u16) -> Self {
        if raw < ETHERTYPE_BOUNDARY {
            // 5-bit zero | 3-bit H-LEN | 8-bit H-Type.
            let h_len = ((raw >> 8) & 0x07) as u8;
            let h_type = (raw & 0x00FF) as u8;
            TypeField::NextHeader { h_len, h_type }
        } else {
            TypeField::EtherType(raw)
        }
    }

    /// Encode back to the raw 16-bit wire value.
    pub fn to_u16(self) -> u16 {
        match self {
            TypeField::NextHeader { h_len, h_type } => ((h_len as u16 & 0x07) << 8) | h_type as u16,
            TypeField::EtherType(et) => et,
        }
    }

    /// `true` if this Type field starts an extension header (Next-Header).
    pub fn is_next_header(self) -> bool {
        matches!(self, TypeField::NextHeader { .. })
    }

    /// Spec label for this kind of Type field.
    pub fn name(&self) -> &'static str {
        match self {
            TypeField::NextHeader { .. } => "next-header",
            TypeField::EtherType(_) => "ethertype",
        }
    }
}

dvb_common::impl_spec_display!(TypeField);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundary_split() {
        assert!(TypeField::from_u16(0x05FF).is_next_header());
        assert!(!TypeField::from_u16(0x0600).is_next_header());
        assert_eq!(TypeField::from_u16(0x0800), TypeField::EtherType(0x0800));
        assert_eq!(TypeField::from_u16(0x86DD), TypeField::EtherType(0x86DD));
    }

    #[test]
    fn next_header_split_round_trips() {
        // 0x0301 = H-LEN 3, H-Type 0x01 (TimeStamp).
        let t = TypeField::from_u16(0x0301);
        assert_eq!(
            t,
            TypeField::NextHeader {
                h_len: 3,
                h_type: 0x01
            }
        );
        assert_eq!(t.to_u16(), 0x0301);

        // Mandatory: 0x0001 = H-LEN 0, H-Type 0x01 (Bridged-Frame).
        let m = TypeField::from_u16(0x0001);
        assert_eq!(
            m,
            TypeField::NextHeader {
                h_len: 0,
                h_type: 0x01
            }
        );
        assert_eq!(m.to_u16(), 0x0001);
    }

    #[test]
    fn all_u16_round_trip() {
        for raw in 0u32..=0xFFFF {
            let raw = raw as u16;
            assert_eq!(TypeField::from_u16(raw).to_u16(), raw, "raw={raw:#06X}");
        }
    }
}
