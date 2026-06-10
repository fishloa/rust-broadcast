//! T2-MI payload type 0x21: Individual addressing — §5.2.8.
//!
//! Carries per-transmitter addressing data: tx_identifier + function loop
//! with entries like ACE-PAPR (0x10), MISO group (0x11), Frequency (0x17), etc.

use std::fmt;

use num_enum::TryFromPrimitive;

use dvb_common::{Parse, Serialize};

/// Function tags per §5.2.8.2 Table 5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, TryFromPrimitive)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[repr(u8)]
#[non_exhaustive]
pub enum AddressingFunctionTag {
    /// Transmitter time offset.
    TimeOffset = 0x00,
    /// Transmitter frequency offset.
    FrequencyOffset = 0x01,
    /// Transmitter power.
    Power = 0x02,
    /// Private data.
    PrivateData = 0x03,
    /// Cell ID.
    CellId = 0x04,
    /// Enable.
    Enable = 0x05,
    /// Bandwidth (not applicable for T2).
    Bandwidth = 0x06,
    /// ACE-PAPR reduction (T2-specific).
    AcePapr = 0x10,
    /// MISO group (T2-specific).
    MisoGroup = 0x11,
    /// TR-PAPR reduction (T2-specific).
    TrPapr = 0x12,
    /// L1-ACE-PAPR (T2-specific).
    L1AcePapr = 0x13,
    /// TX-SIG FEF sequence number (T2-specific).
    TxSigFefSeqNum = 0x15,
    /// TX-SIG auxiliary stream TX ID (T2-specific).
    TxSigAuxStreamTxId = 0x16,
    /// Frequency (T2-specific).
    Frequency = 0x17,
}

impl From<AddressingFunctionTag> for u8 {
    fn from(tag: AddressingFunctionTag) -> Self {
        tag as u8
    }
}

impl fmt::Display for AddressingFunctionTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AddressingFunctionTag::TimeOffset => write!(f, "TimeOffset"),
            AddressingFunctionTag::FrequencyOffset => write!(f, "FrequencyOffset"),
            AddressingFunctionTag::Power => write!(f, "Power"),
            AddressingFunctionTag::PrivateData => write!(f, "PrivateData"),
            AddressingFunctionTag::CellId => write!(f, "CellId"),
            AddressingFunctionTag::Enable => write!(f, "Enable"),
            AddressingFunctionTag::Bandwidth => write!(f, "Bandwidth"),
            AddressingFunctionTag::AcePapr => write!(f, "AcePapr"),
            AddressingFunctionTag::MisoGroup => write!(f, "MisoGroup"),
            AddressingFunctionTag::TrPapr => write!(f, "TrPapr"),
            AddressingFunctionTag::L1AcePapr => write!(f, "L1AcePapr"),
            AddressingFunctionTag::TxSigFefSeqNum => write!(f, "TxSigFefSeqNum"),
            AddressingFunctionTag::TxSigAuxStreamTxId => write!(f, "TxSigAuxStreamTxId"),
            AddressingFunctionTag::Frequency => write!(f, "Frequency"),
        }
    }
}

/// A single function entry within individual addressing.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct FunctionEntry<'a> {
    /// Function tag identifying the entry type.
    pub tag: AddressingFunctionTag,
    /// Raw function body (including tag + length bytes).
    pub raw: &'a [u8],
}

/// Individual addressing payload (type 0x21) per ETSI TS 102 773 §5.2.8.1, Fig 11.
///
/// Top-level layout:
/// - byte 0: rfu (8 bits) — reserved, ignored on parse, preserved for round-trip
/// - byte 1: individual_addressing_length (8 bits) — length of the data loop in bytes
/// - bytes 2..: individual_addressing_data — a loop of per-transmitter entries, each
///   `tx_identifier(16) · function_loop_length(8) · function()…`. The tx_identifier
///   lives INSIDE each entry, not at the top level; the loop is kept raw here.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct IndividualAddressingPayload<'a> {
    /// Reserved-for-future-use byte (byte 0); preserved verbatim for round-trip.
    pub rfu: u8,
    /// Raw individual_addressing_data loop. Length is the 8-bit
    /// `individual_addressing_length` field, derived from this slice on serialize.
    pub individual_addressing_data: &'a [u8],
}

const HEADER_LEN: usize = 2;

impl<'a> Parse<'a> for IndividualAddressingPayload<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self, crate::error::Error> {
        if bytes.len() < HEADER_LEN {
            return Err(crate::Error::BufferTooShort {
                need: HEADER_LEN,
                have: bytes.len(),
                what: "IndividualAddressingPayload header",
            });
        }

        let rfu = bytes[0];
        let individual_addressing_length = bytes[1] as usize;
        // The data loop must hold exactly the declared number of bytes (§5.2.8.1).
        let need = HEADER_LEN + individual_addressing_length;
        if bytes.len() < need {
            return Err(crate::Error::BufferTooShort {
                need,
                have: bytes.len(),
                what: "IndividualAddressingPayload data",
            });
        }

        Ok(IndividualAddressingPayload {
            rfu,
            individual_addressing_data: &bytes[HEADER_LEN..need],
        })
    }
}

impl<'a> crate::traits::PayloadDef<'a> for IndividualAddressingPayload<'a> {
    const PACKET_TYPE: u8 = 0x21;
    const NAME: &'static str = "INDIVIDUAL_ADDRESSING";
}

impl Serialize for IndividualAddressingPayload<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN + self.individual_addressing_data.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize, crate::error::Error> {
        if buf.len() < self.serialized_len() {
            return Err(crate::Error::OutputBufferTooSmall {
                need: self.serialized_len(),
                have: buf.len(),
            });
        }

        // individual_addressing_length is an 8-bit field — the data loop cannot
        // exceed 255 bytes.
        if self.individual_addressing_data.len() > u8::MAX as usize {
            return Err(crate::Error::ReservedBitsViolation {
                field: "individual_addressing_length",
                reason: "individual_addressing_data exceeds 255 bytes (8-bit length field)",
            });
        }

        buf[0] = self.rfu;
        buf[1] = self.individual_addressing_data.len() as u8;

        if !self.individual_addressing_data.is_empty() {
            buf[HEADER_LEN..HEADER_LEN + self.individual_addressing_data.len()]
                .copy_from_slice(self.individual_addressing_data);
        }

        Ok(self.serialized_len())
    }
}

impl fmt::Display for IndividualAddressingPayload<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "IndividualAddressing {{ rfu: 0x{:02X}, addr_data_len: {} }}",
            self.rfu,
            self.individual_addressing_data.len()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addressing_function_tag_try_from_valid() {
        assert_eq!(
            AddressingFunctionTag::try_from(0x10),
            Ok(AddressingFunctionTag::AcePapr)
        );
        assert_eq!(
            AddressingFunctionTag::try_from(0x17),
            Ok(AddressingFunctionTag::Frequency)
        );
    }

    #[test]
    fn addressing_function_tag_try_from_rejects_unknown() {
        assert!(AddressingFunctionTag::try_from(0x14).is_err());
        assert!(AddressingFunctionTag::try_from(0xFF).is_err());
    }

    #[test]
    fn exhaustive_byte_sweep() {
        let mut matched = 0u16;
        for byte in 0u8..=0xFF {
            if let Ok(v) = AddressingFunctionTag::try_from(byte) {
                assert_eq!(v as u8, byte, "round-trip failed for {byte:#04x}");
                matched += 1;
            }
        }
        assert_eq!(matched, 14, "expected 14 matched variants");
    }

    #[test]
    fn address_function_tag_display() {
        assert_eq!(AddressingFunctionTag::AcePapr.to_string(), "AcePapr");
    }

    #[test]
    fn parse_extracts_rfu_and_addressing_data() {
        // rfu=0x00, length=4, then a 4-byte data loop. The data loop here is one
        // transmitter entry: tx_identifier=0x0005, function_loop_length=0x04, ...
        // — but parse keeps the whole loop raw.
        let buf = [0x00u8, 0x04, 0x00, 0x05, 0x04, 0x10];
        let result = IndividualAddressingPayload::parse(&buf).unwrap();
        assert_eq!(result.rfu, 0x00);
        assert_eq!(result.individual_addressing_data, &[0x00, 0x05, 0x04, 0x10]);
    }

    #[test]
    fn parse_preserves_rfu_byte() {
        let buf = [0xFFu8, 0x00];
        let result = IndividualAddressingPayload::parse(&buf).unwrap();
        assert_eq!(result.rfu, 0xFF);
        assert!(result.individual_addressing_data.is_empty());
    }

    #[test]
    fn parse_rejects_short_buffer() {
        assert!(IndividualAddressingPayload::parse(&[0x00]).is_err());
    }

    #[test]
    fn parse_rejects_truncated_data() {
        // declares 4 data bytes but only 2 follow
        assert!(IndividualAddressingPayload::parse(&[0x00, 0x04, 0xAA, 0xBB]).is_err());
    }

    #[test]
    fn serialize_round_trip() {
        let orig = IndividualAddressingPayload {
            rfu: 0x00,
            individual_addressing_data: &[0x00, 0x03, 0x04, 0xDE, 0xAD],
        };
        let mut buf = vec![0u8; orig.serialized_len()];
        orig.serialize_into(&mut buf).unwrap();
        // length field is derived from the data slice
        assert_eq!(buf[1], 5);
        let parsed = IndividualAddressingPayload::parse(&buf).unwrap();
        assert_eq!(orig, parsed);
    }

    #[test]
    fn serialize_empty_data() {
        let orig = IndividualAddressingPayload {
            rfu: 0x00,
            individual_addressing_data: &[],
        };
        let mut buf = vec![0u8; orig.serialized_len()];
        orig.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, [0x00, 0x00]);
    }
}
