//! Biting round-trip, multi-parameter boundary, field-mutation, and
//! enum-value-drift tests for the SimulCrypt generic message framing.

use dvb_common::traits::{Parse, Serialize};
use dvb_simulcrypt::{
    CpSigMessageType, CpSigParameterType, DataType, EcmgErrorStatus, EcmgScsMessageType,
    EcmgScsParameterType, EmmgErrorStatus, EmmgMuxMessageType, EmmgMuxParameterType, Interface,
    MessageType, Parameter, ParameterType, SectionTspktFlag, SimulcryptMessage,
};

/// Construct an ECMG channel_setup with Super_CAS_id + ECM_channel_id, assert
/// the exact hand-computed wire bytes, then reparse to equal.
#[test]
fn ecmg_channel_setup_exact_wire_bytes() {
    let ecm_channel_id = [0x00u8, 0x2A];
    let super_cas_id = [0x00u8, 0x01, 0x00, 0x02];
    let msg = SimulcryptMessage::new(
        0x03,
        MessageType::EcmgScs(EcmgScsMessageType::ChannelSetup),
        vec![
            Parameter::new(
                ParameterType::EcmgScs(EcmgScsParameterType::EcmChannelId),
                &ecm_channel_id,
            ),
            Parameter::new(
                ParameterType::EcmgScs(EcmgScsParameterType::SuperCasId),
                &super_cas_id,
            ),
        ],
    );

    let bytes = msg.to_bytes();
    // header: 03 | 0001 | message_length
    // body  : 000E 0002 002A | 0001 0004 00010002
    // body_len = 6 + 8 = 14 = 0x000E
    #[rustfmt::skip]
    let expected: &[u8] = &[
        0x03, 0x00, 0x01, 0x00, 0x0E,
        0x00, 0x0E, 0x00, 0x02, 0x00, 0x2A,
        0x00, 0x01, 0x00, 0x04, 0x00, 0x01, 0x00, 0x02,
    ];
    assert_eq!(bytes, expected, "hand-computed wire bytes");

    let parsed = SimulcryptMessage::parse_on(Interface::EcmgScs, &bytes).unwrap();
    assert_eq!(parsed, msg);
}

/// CW_provision with opaque CW, exact bytes + reparse.
#[test]
fn cw_provision_exact_wire_bytes() {
    let cp_cw = [0x12u8, 0x34, 0xDE, 0xAD, 0xBE, 0xEF]; // CP(2) + opaque CW(4)
    let cp_number = [0x12u8, 0x34];
    let msg = SimulcryptMessage::new(
        0x03,
        MessageType::EcmgScs(EcmgScsMessageType::CwProvision),
        vec![
            Parameter::new(
                ParameterType::EcmgScs(EcmgScsParameterType::CpNumber),
                &cp_number,
            ),
            Parameter::new(
                ParameterType::EcmgScs(EcmgScsParameterType::CpCwCombination),
                &cp_cw,
            ),
        ],
    );
    let bytes = msg.to_bytes();
    #[rustfmt::skip]
    let expected: &[u8] = &[
        0x03, 0x02, 0x01, 0x00, 0x10,            // hdr, body_len = 6+10 = 0x10
        0x00, 0x12, 0x00, 0x02, 0x12, 0x34,      // CP_number
        0x00, 0x14, 0x00, 0x06, 0x12, 0x34, 0xDE, 0xAD, 0xBE, 0xEF, // CP_CW_combination
    ];
    assert_eq!(bytes, expected);
    assert_eq!(
        SimulcryptMessage::parse_on(Interface::EcmgScs, &bytes).unwrap(),
        msg
    );
}

/// Field-mutation bite: change one typed field and the wire bytes must change.
#[test]
fn field_mutation_changes_wire() {
    let v = [0x00u8, 0x2A];
    let base = SimulcryptMessage::new(
        0x03,
        MessageType::EcmgScs(EcmgScsMessageType::ChannelSetup),
        vec![Parameter::new(
            ParameterType::EcmgScs(EcmgScsParameterType::EcmChannelId),
            &v,
        )],
    );
    let mut mutated = base.clone();
    mutated.message_type = MessageType::EcmgScs(EcmgScsMessageType::ChannelClose);
    assert_ne!(
        base.to_bytes(),
        mutated.to_bytes(),
        "message_type is rebuilt"
    );

    let v2 = [0x00u8, 0x2B];
    let mutated2 = SimulcryptMessage::new(
        0x03,
        MessageType::EcmgScs(EcmgScsMessageType::ChannelSetup),
        vec![Parameter::new(
            ParameterType::EcmgScs(EcmgScsParameterType::EcmChannelId),
            &v2,
        )],
    );
    assert_ne!(base.to_bytes(), mutated2.to_bytes(), "value is rebuilt");
}

/// Adding a parameter must change message_length and add a TLV — proves the
/// header length is recomputed, not echoed.
#[test]
fn message_length_recomputed() {
    let a = [0x00u8, 0x2A];
    let b = [0x00u8, 0x01];
    let one = SimulcryptMessage::new(
        0x03,
        MessageType::EcmgScs(EcmgScsMessageType::StreamTest),
        vec![Parameter::new(
            ParameterType::EcmgScs(EcmgScsParameterType::EcmChannelId),
            &a,
        )],
    );
    let two = SimulcryptMessage::new(
        0x03,
        MessageType::EcmgScs(EcmgScsMessageType::StreamTest),
        vec![
            Parameter::new(
                ParameterType::EcmgScs(EcmgScsParameterType::EcmChannelId),
                &a,
            ),
            Parameter::new(
                ParameterType::EcmgScs(EcmgScsParameterType::EcmStreamId),
                &b,
            ),
        ],
    );
    assert_eq!(one.body_len(), 6);
    assert_eq!(two.body_len(), 12);
    assert_eq!(one.to_bytes()[3..5], [0x00, 0x06]);
    assert_eq!(two.to_bytes()[3..5], [0x00, 0x0C]);
}

/// ≥2-parameter boundary: a stream_setup with four TLVs round-trips with order
/// preserved.
#[test]
fn multi_parameter_order_preserved() {
    let chan = [0x00u8, 0x2A];
    let stream = [0x00u8, 0x07];
    let ecm_id = [0xABu8, 0xCD];
    let cp_dur = [0x00u8, 0x14];
    let msg = SimulcryptMessage::new(
        0x03,
        MessageType::EcmgScs(EcmgScsMessageType::StreamSetup),
        vec![
            Parameter::new(
                ParameterType::EcmgScs(EcmgScsParameterType::EcmChannelId),
                &chan,
            ),
            Parameter::new(
                ParameterType::EcmgScs(EcmgScsParameterType::EcmStreamId),
                &stream,
            ),
            Parameter::new(ParameterType::EcmgScs(EcmgScsParameterType::EcmId), &ecm_id),
            Parameter::new(
                ParameterType::EcmgScs(EcmgScsParameterType::NominalCpDuration),
                &cp_dur,
            ),
        ],
    );
    let bytes = msg.to_bytes();
    let parsed = SimulcryptMessage::parse_on(Interface::EcmgScs, &bytes).unwrap();
    assert_eq!(parsed.parameters.len(), 4);
    // Order preserved.
    let types: Vec<u16> = parsed.parameters.iter().map(|p| p.ptype.to_u16()).collect();
    assert_eq!(types, vec![0x000E, 0x000F, 0x0019, 0x0010]);
    assert_eq!(parsed, msg);
}

/// EMMG/PDG⇔MUX data_provision round-trips with an opaque datagram, and the
/// SAME numeric message_type decodes differently per interface.
#[test]
fn emmg_data_provision_round_trip_and_scoping() {
    let client_id = [0x00u8, 0x00, 0x00, 0x05];
    let data_id = [0x00u8, 0x01];
    let datagram = [0xDEu8, 0xAD, 0xBE, 0xEF, 0x00, 0x11];
    let msg = SimulcryptMessage::new(
        0x03,
        MessageType::EmmgPdgMux(EmmgMuxMessageType::DataProvision),
        vec![
            Parameter::new(
                ParameterType::EmmgPdgMux(EmmgMuxParameterType::ClientId),
                &client_id,
            ),
            Parameter::new(
                ParameterType::EmmgPdgMux(EmmgMuxParameterType::DataId),
                &data_id,
            ),
            Parameter::new(
                ParameterType::EmmgPdgMux(EmmgMuxParameterType::Datagram),
                &datagram,
            ),
        ],
    );
    let bytes = msg.to_bytes();
    assert_eq!(
        SimulcryptMessage::parse_on(Interface::EmmgPdgMux, &bytes).unwrap(),
        msg
    );

    // 0x0211 means data_provision on EMMG/PDG⇔MUX but is reserved on ECMG⇔SCS.
    assert_eq!(
        MessageType::from_u16(Interface::EmmgPdgMux, 0x0211),
        MessageType::EmmgPdgMux(EmmgMuxMessageType::DataProvision)
    );
    assert_eq!(
        MessageType::from_u16(Interface::EcmgScs, 0x0211),
        MessageType::EcmgScs(EcmgScsMessageType::Reserved(0x0211))
    );
    // parameter_type 0x0003 = ECM_channel_id (ECMG) vs data_channel_id (EMMG).
    assert_eq!(
        ParameterType::from_u16(Interface::EcmgScs, 0x0003),
        ParameterType::EcmgScs(EcmgScsParameterType::DelayStart)
    );
    assert_eq!(
        ParameterType::from_u16(Interface::EmmgPdgMux, 0x0003),
        ParameterType::EmmgPdgMux(EmmgMuxParameterType::DataChannelId)
    );
}

/// The default `Parse` impl targets ECMG⇔SCS.
#[test]
fn default_parse_targets_ecmg() {
    let v = [0x00u8, 0x2A];
    let msg = SimulcryptMessage::new(
        0x03,
        MessageType::EcmgScs(EcmgScsMessageType::ChannelTest),
        vec![Parameter::new(
            ParameterType::EcmgScs(EcmgScsParameterType::EcmChannelId),
            &v,
        )],
    );
    let bytes = msg.to_bytes();
    assert_eq!(SimulcryptMessage::parse(&bytes).unwrap(), msg);
}

// ---------------------------------------------------------------------------
// Enum value drift: pin every variant to the doc's numeric value.
// ---------------------------------------------------------------------------

#[test]
fn ecmg_message_type_values() {
    use EcmgScsMessageType::*;
    for (e, v) in [
        (ChannelSetup, 0x0001u16),
        (ChannelTest, 0x0002),
        (ChannelStatus, 0x0003),
        (ChannelClose, 0x0004),
        (ChannelError, 0x0005),
        (StreamSetup, 0x0101),
        (StreamTest, 0x0102),
        (StreamStatus, 0x0103),
        (StreamCloseRequest, 0x0104),
        (StreamCloseResponse, 0x0105),
        (StreamError, 0x0106),
        (CwProvision, 0x0201),
        (EcmResponse, 0x0202),
    ] {
        assert_eq!(e.to_u16(), v, "{e:?}");
        assert_eq!(EcmgScsMessageType::from_u16(v), e, "0x{v:04X}");
    }
    assert_eq!(
        EcmgScsMessageType::from_u16(0x4242),
        EcmgScsMessageType::Reserved(0x4242)
    );
}

#[test]
fn emmg_message_type_values() {
    use EmmgMuxMessageType::*;
    for (e, v) in [
        (ChannelSetup, 0x0011u16),
        (ChannelTest, 0x0012),
        (ChannelStatus, 0x0013),
        (ChannelClose, 0x0014),
        (ChannelError, 0x0015),
        (StreamSetup, 0x0111),
        (StreamTest, 0x0112),
        (StreamStatus, 0x0113),
        (StreamCloseRequest, 0x0114),
        (StreamCloseResponse, 0x0115),
        (StreamError, 0x0116),
        (StreamBwRequest, 0x0117),
        (StreamBwAllocation, 0x0118),
        (DataProvision, 0x0211),
    ] {
        assert_eq!(e.to_u16(), v, "{e:?}");
        assert_eq!(EmmgMuxMessageType::from_u16(v), e, "0x{v:04X}");
    }
}

#[test]
fn ecmg_parameter_type_values() {
    use EcmgScsParameterType::*;
    for (e, v) in [
        (SuperCasId, 0x0001u16),
        (SectionTspktFlag, 0x0002),
        (DelayStart, 0x0003),
        (DelayStop, 0x0004),
        (TransitionDelayStart, 0x0005),
        (TransitionDelayStop, 0x0006),
        (EcmRepPeriod, 0x0007),
        (MaxStreams, 0x0008),
        (MinCpDuration, 0x0009),
        (LeadCw, 0x000A),
        (CwPerMsg, 0x000B),
        (MaxCompTime, 0x000C),
        (AccessCriteria, 0x000D),
        (EcmChannelId, 0x000E),
        (EcmStreamId, 0x000F),
        (NominalCpDuration, 0x0010),
        (AccessCriteriaTransferMode, 0x0011),
        (CpNumber, 0x0012),
        (CpDuration, 0x0013),
        (CpCwCombination, 0x0014),
        (EcmDatagram, 0x0015),
        (AcDelayStart, 0x0016),
        (AcDelayStop, 0x0017),
        (CwEncryption, 0x0018),
        (EcmId, 0x0019),
        (ErrorStatus, 0x7000),
        (ErrorInformation, 0x7001),
    ] {
        assert_eq!(e.to_u16(), v, "{e:?}");
        assert_eq!(EcmgScsParameterType::from_u16(v), e, "0x{v:04X}");
    }
}

#[test]
fn emmg_parameter_type_values() {
    use EmmgMuxParameterType::*;
    for (e, v) in [
        (ClientId, 0x0001u16),
        (SectionTspktFlag, 0x0002),
        (DataChannelId, 0x0003),
        (DataStreamId, 0x0004),
        (Datagram, 0x0005),
        (Bandwidth, 0x0006),
        (DataType, 0x0007),
        (DataId, 0x0008),
        (ErrorStatus, 0x7000),
        (ErrorInformation, 0x7001),
    ] {
        assert_eq!(e.to_u16(), v, "{e:?}");
        assert_eq!(EmmgMuxParameterType::from_u16(v), e, "0x{v:04X}");
    }
}

#[test]
fn data_type_and_flag_values() {
    assert_eq!(DataType::Emm.to_u8(), 0x00);
    assert_eq!(DataType::PrivateData.to_u8(), 0x01);
    assert_eq!(DataType::Ecm.to_u8(), 0x02);
    assert_eq!(DataType::from_u8(0x09), DataType::Reserved(0x09));

    assert_eq!(SectionTspktFlag::SectionFormat.to_u8(), 0x00);
    assert_eq!(SectionTspktFlag::TsPacketFormat.to_u8(), 0x01);
    assert_eq!(SectionTspktFlag::ArbitraryLength.to_u8(), 0x02);
}

#[test]
fn ecmg_error_status_values() {
    use EcmgErrorStatus::*;
    for (e, v) in [
        (InvalidMessage, 0x0001u16),
        (UnsupportedProtocolVersion, 0x0002),
        (UnknownMessageType, 0x0003),
        (MessageTooLong, 0x0004),
        (UnknownSuperCasId, 0x0005),
        (UnknownEcmChannelId, 0x0006),
        (UnknownEcmStreamId, 0x0007),
        (TooManyChannels, 0x0008),
        (TooManyStreamsOnChannel, 0x0009),
        (TooManyStreamsOnEcmg, 0x000A),
        (NotEnoughControlWords, 0x000B),
        (OutOfStorage, 0x000C),
        (OutOfResources, 0x000D),
        (UnknownParameterType, 0x000E),
        (InconsistentLength, 0x000F),
        (MissingMandatoryParameter, 0x0010),
        (InvalidParameterValue, 0x0011),
        (UnknownEcmId, 0x0012),
        (EcmChannelIdInUse, 0x0013),
        (EcmStreamIdInUse, 0x0014),
        (EcmIdInUse, 0x0015),
        (UnknownError, 0x7000),
        (UnrecoverableError, 0x7001),
    ] {
        assert_eq!(e.to_u16(), v, "{e:?}");
        assert_eq!(EcmgErrorStatus::from_u16(v), e, "0x{v:04X}");
    }
    assert_eq!(
        EcmgErrorStatus::from_u16(0x4242),
        EcmgErrorStatus::Reserved(0x4242)
    );
}

#[test]
fn emmg_error_status_values() {
    use EmmgErrorStatus::*;
    for (e, v) in [
        (InvalidMessage, 0x0001u16),
        (UnsupportedProtocolVersion, 0x0002),
        (UnknownMessageType, 0x0003),
        (MessageTooLong, 0x0004),
        (UnknownDataStreamId, 0x0005),
        (UnknownDataChannelId, 0x0006),
        (TooManyChannels, 0x0007),
        (TooManyStreamsOnChannel, 0x0008),
        (TooManyStreamsOnMux, 0x0009),
        (UnknownParameterType, 0x000A),
        (InconsistentLength, 0x000B),
        (MissingMandatoryParameter, 0x000C),
        (InvalidParameterValue, 0x000D),
        (UnknownClientId, 0x000E),
        (ExceededBandwidth, 0x000F),
        (UnknownDataId, 0x0010),
        (DataChannelIdInUse, 0x0011),
        (DataStreamIdInUse, 0x0012),
        (DataIdInUse, 0x0013),
        (ClientIdInUse, 0x0014),
        (UnknownError, 0x7000),
        (UnrecoverableError, 0x7001),
    ] {
        assert_eq!(e.to_u16(), v, "{e:?}");
        assert_eq!(EmmgErrorStatus::from_u16(v), e, "0x{v:04X}");
    }
    assert_eq!(
        EmmgErrorStatus::from_u16(0x4242),
        EmmgErrorStatus::Reserved(0x4242)
    );
}

#[test]
fn emmg_message_type_reserved_passthrough() {
    assert_eq!(
        EmmgMuxMessageType::from_u16(0x4242),
        EmmgMuxMessageType::Reserved(0x4242)
    );
    assert_eq!(EmmgMuxMessageType::Reserved(0x4242).to_u16(), 0x4242);
}

#[test]
fn protocol_version_is_03() {
    assert_eq!(Interface::EcmgScs.protocol_version(), 0x03);
    assert_eq!(Interface::EmmgPdgMux.protocol_version(), 0x03);
}

/// Smoke-test: a `CW_provision` message serialises to JSON and the expected
/// field names are present (requires the `serde` feature).
#[cfg(feature = "serde")]
#[test]
fn serde_cw_provision_smoke() {
    use dvb_simulcrypt::{EcmgScsMessageType, EcmgScsParameterType, Interface, MessageType};

    let cp_number = [0x00u8, 0x07];
    let cp_cw = [0x00u8, 0x07, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
    let msg = SimulcryptMessage::new(
        Interface::EcmgScs.protocol_version(),
        MessageType::EcmgScs(EcmgScsMessageType::CwProvision),
        vec![
            Parameter::new(
                ParameterType::EcmgScs(EcmgScsParameterType::CpNumber),
                &cp_number,
            ),
            Parameter::new(
                ParameterType::EcmgScs(EcmgScsParameterType::CpCwCombination),
                &cp_cw,
            ),
        ],
    );

    let json = serde_json::to_string(&msg).expect("serde_json::to_string");
    assert!(
        json.contains("\"message_type\""),
        "JSON missing 'message_type': {json}"
    );
    assert!(
        json.contains("\"parameters\""),
        "JSON missing 'parameters': {json}"
    );
    assert!(
        json.contains("CwProvision"),
        "JSON missing 'CwProvision' variant: {json}"
    );
}

// ---------------------------------------------------------------------------
// C(P)SIG ⇔ (P)SIG message_type drift: pin every variant.
// ---------------------------------------------------------------------------

#[test]
fn cpsig_message_type_values() {
    use CpSigMessageType::*;
    for (e, v) in [
        (ChannelSetup, 0x0301u16),
        (ChannelStatus, 0x0302),
        (ChannelTest, 0x0303),
        (ChannelClose, 0x0304),
        (ChannelError, 0x0305),
        (StreamSetup, 0x0311),
        (StreamStatus, 0x0312),
        (StreamTest, 0x0313),
        (StreamClose, 0x0314),
        (StreamCloseRequest, 0x0315),
        (StreamCloseResponse, 0x0316),
        (StreamError, 0x0317),
        (StreamServiceChange, 0x0318),
        (StreamTriggerEnableRequest, 0x0319),
        (StreamTriggerEnableResponse, 0x031A),
        (Trigger, 0x031B),
        (TableRequest, 0x031C),
        (TableResponse, 0x031D),
        (DescriptorInsertRequest, 0x031E),
        (DescriptorInsertResponse, 0x031F),
        (PidProvisionRequest, 0x0320),
        (PidProvisionResponse, 0x0321),
    ] {
        assert_eq!(e.to_u16(), v, "{e:?}");
        assert_eq!(CpSigMessageType::from_u16(v), e, "0x{v:04X}");
    }
    assert_eq!(
        CpSigMessageType::from_u16(0x4242),
        CpSigMessageType::Reserved(0x4242)
    );
}

// ---------------------------------------------------------------------------
// C(P)SIG ⇔ (P)SIG parameter_type drift: pin every variant (Table 36).
// ---------------------------------------------------------------------------

#[test]
fn cpsig_parameter_type_values() {
    use CpSigParameterType::*;
    for (e, v) in [
        (AccessCriteria, 0x000Du16),
        (BouquetId, 0x0100),
        (CaDescriptorInsertionMode, 0x0101),
        (CustomCaSystemId, 0x0102),
        (CustomChannelId, 0x0103),
        (CustomStreamId, 0x0104),
        (Descriptor, 0x0105),
        (DescriptorInsertStatus, 0x0106),
        (Duration, 0x0107),
        (EcmRelatedData, 0x0108),
        (EsId, 0x010B),
        (EventId, 0x010C),
        (EventRelatedData, 0x010D),
        (FlowId, 0x010E),
        (FlowPid, 0x010F),
        (FlowPidChangeRelatedData, 0x0110),
        (FlowSuperCasId, 0x0111),
        (FlowType, 0x0112),
        (InsertionDelay, 0x0113),
        (InsertionDelayType, 0x0114),
        (LastSectionIndicator, 0x0115),
        (LocationId, 0x0116),
        (MaxCompTime, 0x0117),
        (MaxStreams, 0x0118),
        (MpegSection, 0x0119),
        (NetworkId, 0x011A),
        (OriginalNetworkId, 0x011B),
        (PrivateData, 0x011C),
        (PrivateDataSpecifier, 0x011D),
        (PSigType, 0x011E),
        (SegmentNumber, 0x011F),
        (ServiceId, 0x0120),
        (ServiceParameters, 0x0121),
        (StartTime, 0x0122),
        (StreamChangeTimestamp, 0x0123),
        (StreamChangeType, 0x0124),
        (TableId, 0x0125),
        (TransactionId, 0x0126),
        (TransportStreamId, 0x0127),
        (TriggerId, 0x0128),
        (TriggerList, 0x0129),
        (TriggerType, 0x012A),
        (PdRelatedData, 0x012B),
        (FlowStreamType, 0x012C),
        (ErrorStatus, 0x7000),
        (ErrorInformation, 0x7001),
    ] {
        assert_eq!(e.to_u16(), v, "{e:?}");
        assert_eq!(CpSigParameterType::from_u16(v), e, "0x{v:04X}");
    }
    assert_eq!(
        CpSigParameterType::from_u16(0x4242),
        CpSigParameterType::Reserved(0x4242)
    );
}

// ---------------------------------------------------------------------------
// C(P)SIG ⇔ (P)SIG message round-trip: channel_setup with ≥2 parameters.
// ---------------------------------------------------------------------------

#[test]
fn cpsig_channel_setup_round_trip() {
    let custom_cas_id = [0x00u8, 0x01, 0x00, 0x02];
    let bouquet_id = [0x00u8, 0x2A];
    let msg = SimulcryptMessage::new(
        0x03,
        MessageType::CpSigPSig(CpSigMessageType::ChannelSetup),
        vec![
            Parameter::new(
                ParameterType::CpSigPSig(CpSigParameterType::CustomCaSystemId),
                &custom_cas_id,
            ),
            Parameter::new(
                ParameterType::CpSigPSig(CpSigParameterType::BouquetId),
                &bouquet_id,
            ),
        ],
    );
    let bytes = msg.to_bytes();
    let parsed = SimulcryptMessage::parse_on(Interface::CpSigPSig, &bytes).unwrap();
    assert_eq!(parsed, msg, "byte-exact round-trip");
    assert_eq!(parsed.parameters.len(), 2);

    // Mutate one parameter and assert bytes change.
    let mut mutated = msg.clone();
    let other_bouquet = [0x00u8, 0x2B];
    mutated.parameters[1] = Parameter::new(
        ParameterType::CpSigPSig(CpSigParameterType::BouquetId),
        &other_bouquet,
    );
    assert_ne!(
        msg.to_bytes(),
        mutated.to_bytes(),
        "mutating a parameter changes the wire bytes"
    );
}
