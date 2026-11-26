//! Every `TryFrom<repr>` spec enum round-trips each wire value through
//! `try_from` and back through `From<Self> for repr`, and rejects an
//! unallocated value with the crate's own typed error carrying that value.

use broadcast_core::error::InvalidValue;
use dvb_t2mi::Error;
use dvb_t2mi::packet::PacketType;
use dvb_t2mi::payload::fef_null::S1Field;
use dvb_t2mi::payload::fef_subpart::{PrbsType, SubpartVariety};
use dvb_t2mi::payload::individual_addressing::AddressingFunctionTag;
use dvb_t2mi::payload::l1_current::FrequencySource;
use dvb_t2mi::payload::timestamp::Bandwidth;

fn invalid_value(field: &'static str, value: u64, err: Error) {
    match err {
        Error::InvalidValue(InvalidValue {
            field: f, value: v, ..
        }) => assert_eq!((f, v), (field, value)),
        other => panic!("expected InvalidValue for {field}, got {other:?}"),
    }
}

#[test]
fn packet_type() {
    for (byte, v) in [
        (0x00u8, PacketType::BasebandFrame),
        (0x01, PacketType::AuxiliaryIqData),
        (0x02, PacketType::ArbitraryCellInsertion),
        (0x10, PacketType::L1Current),
        (0x11, PacketType::L1Future),
        (0x12, PacketType::P2BiasBalancing),
        (0x20, PacketType::Timestamp),
        (0x21, PacketType::IndividualAddressing),
        (0x30, PacketType::FefPartNull),
        (0x31, PacketType::FefPartIqData),
        (0x32, PacketType::FefPartComposite),
        (0x33, PacketType::FefSubPart),
    ] {
        assert_eq!(PacketType::try_from(byte), Ok(v));
        assert_eq!(u8::from(v), byte);
    }
    assert_eq!(
        PacketType::try_from(0x03),
        Err(Error::InvalidPacketType { found: 0x03 })
    );
}

#[test]
fn frequency_source() {
    for (byte, v) in [
        (0b00u8, FrequencySource::UseL1CurrentData),
        (0b01, FrequencySource::UseIndividualAddressing),
        (0b10, FrequencySource::ManualPerModulator),
    ] {
        assert_eq!(FrequencySource::try_from(byte), Ok(v));
        assert_eq!(u8::from(v), byte);
    }
    invalid_value(
        "freq_source",
        3,
        FrequencySource::try_from(0b11).unwrap_err(),
    );
}

#[test]
fn s1_field() {
    for (byte, v) in [
        (0u8, S1Field::V0),
        (1, S1Field::V1),
        (2, S1Field::V2),
        (3, S1Field::V3),
        (4, S1Field::V4),
        (5, S1Field::V5),
        (6, S1Field::V6),
        (7, S1Field::V7),
    ] {
        assert_eq!(S1Field::try_from(byte), Ok(v));
        assert_eq!(u8::from(v), byte);
    }
    invalid_value("s1_field", 8, S1Field::try_from(8).unwrap_err());
}

#[test]
fn bandwidth() {
    for (byte, v) in [
        (0u8, Bandwidth::Mhz1_7),
        (1, Bandwidth::Mhz5),
        (2, Bandwidth::Mhz6),
        (3, Bandwidth::Mhz7),
        (4, Bandwidth::Mhz8),
        (5, Bandwidth::Mhz10),
    ] {
        assert_eq!(Bandwidth::try_from(byte), Ok(v));
        assert_eq!(u8::from(v), byte);
    }
    invalid_value("bw", 6, Bandwidth::try_from(6).unwrap_err());
}

#[test]
fn subpart_variety() {
    for (word, v) in [
        (0x0000u16, SubpartVariety::Null),
        (0x0001, SubpartVariety::Iq),
        (0x0002, SubpartVariety::Prbs),
        (0x0003, SubpartVariety::TxSigFef),
    ] {
        assert_eq!(SubpartVariety::try_from(word), Ok(v));
        assert_eq!(u16::from(v), word);
    }
    invalid_value(
        "subpart_variety",
        0x0100,
        SubpartVariety::try_from(0x0100).unwrap_err(),
    );
}

#[test]
fn prbs_type() {
    assert_eq!(PrbsType::try_from(0x00), Ok(PrbsType::UserDefined));
    assert_eq!(u8::from(PrbsType::UserDefined), 0x00);
    invalid_value("prbs_type", 0x01, PrbsType::try_from(0x01).unwrap_err());
}

#[test]
fn addressing_function_tag() {
    for (byte, v) in [
        (0x00u8, AddressingFunctionTag::TimeOffset),
        (0x01, AddressingFunctionTag::FrequencyOffset),
        (0x02, AddressingFunctionTag::Power),
        (0x03, AddressingFunctionTag::PrivateData),
        (0x04, AddressingFunctionTag::CellId),
        (0x05, AddressingFunctionTag::Enable),
        (0x06, AddressingFunctionTag::Bandwidth),
        (0x10, AddressingFunctionTag::AcePapr),
        (0x11, AddressingFunctionTag::MisoGroup),
        (0x12, AddressingFunctionTag::TrPapr),
        (0x13, AddressingFunctionTag::L1AcePapr),
        (0x15, AddressingFunctionTag::TxSigFefSeqNum),
        (0x16, AddressingFunctionTag::TxSigAuxStreamTxId),
        (0x17, AddressingFunctionTag::Frequency),
    ] {
        assert_eq!(AddressingFunctionTag::try_from(byte), Ok(v));
        assert_eq!(u8::from(v), byte);
    }
    invalid_value(
        "function_tag",
        0x14,
        AddressingFunctionTag::try_from(0x14).unwrap_err(),
    );
}
