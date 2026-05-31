//! T2-MI payload type 0x21: Individual addressing — §5.2.8.
//!
//! Carries per-transmitter addressing data: tx_identifier + function loop
//! with entries like ACE-PAPR (0x10), MISO group (0x11), Frequency (0x17), etc.

use std::fmt;

use dvb_common::{Parse, Serialize};

/// Function tags per §5.2.8.2 Table 5.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
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

impl TryFrom<u8> for AddressingFunctionTag {
    type Error = crate::Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x00 => Ok(AddressingFunctionTag::TimeOffset),
            0x01 => Ok(AddressingFunctionTag::FrequencyOffset),
            0x02 => Ok(AddressingFunctionTag::Power),
            0x03 => Ok(AddressingFunctionTag::PrivateData),
            0x04 => Ok(AddressingFunctionTag::CellId),
            0x05 => Ok(AddressingFunctionTag::Enable),
            0x06 => Ok(AddressingFunctionTag::Bandwidth),
            0x10 => Ok(AddressingFunctionTag::AcePapr),
            0x11 => Ok(AddressingFunctionTag::MisoGroup),
            0x12 => Ok(AddressingFunctionTag::TrPapr),
            0x13 => Ok(AddressingFunctionTag::L1AcePapr),
            0x15 => Ok(AddressingFunctionTag::TxSigFefSeqNum),
            0x16 => Ok(AddressingFunctionTag::TxSigAuxStreamTxId),
            0x17 => Ok(AddressingFunctionTag::Frequency),
            _ => Err(crate::Error::InvalidPacketType { found: value }),
        }
    }
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
pub struct FunctionEntry<'a> {
    /// Function tag identifying the entry type.
    pub tag: AddressingFunctionTag,
    /// Raw function body (including tag + length bytes).
    pub raw: &'a [u8],
}

/// Individual addressing payload (type 0x21) per ETSI TS 102 773 §5.2.8.
///
/// Layout:
/// - byte 0-1: tx_identifier (16 bits) — 0x0000 = broadcast
/// - byte 2: ind_addr_data_length (8 bits) — length of function loop in bytes
/// - bytes 3..: function loop entries
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndividualAddressingPayload<'a> {
    /// Transmitter identifier (0x0000 = broadcast).
    pub tx_identifier: u16,
    /// Length of the function loop in bytes.
    pub ind_addr_data_length: u8,
    /// Raw function loop data.
    pub ind_addr_data: &'a [u8],
}

impl<'a> Parse<'a> for IndividualAddressingPayload<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self, crate::error::Error> {
        if bytes.len() < 3 {
            return Err(crate::Error::BufferTooShort {
                need: 3,
                have: bytes.len(),
                what: "IndividualAddressingPayload header",
            });
        }

        let tx_identifier = u16::from_be_bytes([bytes[0], bytes[1]]);
        let ind_addr_data_length = bytes[2];

        Ok(IndividualAddressingPayload {
            tx_identifier,
            ind_addr_data_length,
            ind_addr_data: &bytes[3..],
        })
    }
}

impl Serialize for IndividualAddressingPayload<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        3 + self.ind_addr_data.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize, crate::error::Error> {
        if buf.len() < self.serialized_len() {
            return Err(crate::Error::OutputBufferTooSmall {
                need: self.serialized_len(),
                have: buf.len(),
            });
        }

        let tx_id = self.tx_identifier.to_be_bytes();
        buf[0] = tx_id[0];
        buf[1] = tx_id[1];
        buf[2] = self.ind_addr_data_length;

        if !self.ind_addr_data.is_empty() {
            buf[3..3 + self.ind_addr_data.len()].copy_from_slice(self.ind_addr_data);
        }

        Ok(self.serialized_len())
    }
}

impl fmt::Display for IndividualAddressingPayload<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "IndividualAddressing {{ tx_id: 0x{:04X}, addr_data_len: {} }}",
            self.tx_identifier, self.ind_addr_data_length
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
    fn address_function_tag_display() {
        assert_eq!(AddressingFunctionTag::AcePapr.to_string(), "AcePapr");
    }

    #[test]
    fn parse_extracts_tx_identifier_and_addressing_data() {
        let buf = [0x00u8, 0x05, 0x04, 0x10, 0x04, 0xCA, 0xFE];
        let result = IndividualAddressingPayload::parse(&buf).unwrap();
        assert_eq!(result.tx_identifier, 0x0005);
        assert_eq!(result.ind_addr_data_length, 4);
        assert_eq!(result.ind_addr_data, &[0x10, 0x04, 0xCA, 0xFE]);
    }

    #[test]
    fn parse_rejects_short_buffer() {
        assert!(IndividualAddressingPayload::parse(&[0x00, 0x00]).is_err());
    }

    #[test]
    fn serialize_round_trip() {
        let orig = IndividualAddressingPayload {
            tx_identifier: 0x0003,
            ind_addr_data_length: 4,
            ind_addr_data: &[0x00, 0x04, 0xDE, 0xAD],
        };
        let mut buf = vec![0u8; orig.serialized_len()];
        orig.serialize_into(&mut buf).unwrap();
        let parsed = IndividualAddressingPayload::parse(&buf).unwrap();
        assert_eq!(orig, parsed);
    }

    #[test]
    fn broadcast_tx_identifier() {
        let buf = [0x00u8, 0x00, 0x00];
        let result = IndividualAddressingPayload::parse(&buf).unwrap();
        assert_eq!(result.tx_identifier, 0x0000);
    }
}
