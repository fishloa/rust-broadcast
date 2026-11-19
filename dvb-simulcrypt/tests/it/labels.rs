//! Every registry `name()` label — the spec token a log or trace prints — is
//! pinned to its ETSI TS 103 197 V1.5.1 spelling, and each reserved catch-all
//! to `reserved`. `Display` must agree with `name()`.

use dvb_simulcrypt::{
    CpSigMessageType, CpSigParameterType, DataType, EcmgErrorStatus, EcmgScsMessageType,
    EcmgScsParameterType, EmmgErrorStatus, EmmgMuxMessageType, EmmgMuxParameterType, Interface,
    MessageType, ParameterType, SectionTspktFlag,
};

macro_rules! assert_label {
    ($value:expr, $label:literal) => {{
        let value = $value;
        assert_eq!(value.name(), $label);
        assert_eq!(value.to_string(), $label);
    }};
}

#[test]
fn interface_labels() {
    assert_label!(Interface::EcmgScs, "ECMG<->SCS");
    assert_label!(Interface::EmmgPdgMux, "EMMG/PDG<->MUX");
    assert_label!(Interface::CpSigPSig, "C(P)SIG<->(P)SIG");
}

#[test]
fn ecmg_scs_message_type_labels() {
    assert_label!(EcmgScsMessageType::ChannelSetup, "channel_setup");
    assert_label!(EcmgScsMessageType::CwProvision, "CW_provision");
    assert_label!(EcmgScsMessageType::EcmResponse, "ECM_response");
    let reserved = EcmgScsMessageType::Reserved(0x4242);
    assert_eq!(reserved.name(), "reserved");
    assert_eq!(reserved.to_string(), "reserved(0x4242)");
    assert_label!(
        MessageType::EcmgScs(EcmgScsMessageType::ChannelSetup),
        "channel_setup"
    );
}

#[test]
fn emmg_mux_message_type_labels() {
    assert_label!(EmmgMuxMessageType::DataProvision, "data_provision");
    assert_label!(EmmgMuxMessageType::StreamBwRequest, "stream_BW_request");
    let reserved = EmmgMuxMessageType::Reserved(0x4242);
    assert_eq!(reserved.name(), "reserved");
    assert_eq!(reserved.to_string(), "reserved(0x4242)");
    assert_label!(
        MessageType::EmmgPdgMux(EmmgMuxMessageType::DataProvision),
        "data_provision"
    );
}

#[test]
fn cpsig_message_type_labels() {
    assert_label!(CpSigMessageType::TableRequest, "table_request");
    assert_label!(
        CpSigMessageType::PidProvisionRequest,
        "PID_provision_request"
    );
    let reserved = CpSigMessageType::Reserved(0x4242);
    assert_eq!(reserved.name(), "reserved");
    assert_eq!(reserved.to_string(), "reserved(0x4242)");
    assert_label!(
        MessageType::CpSigPSig(CpSigMessageType::TableRequest),
        "table_request"
    );
}

#[test]
fn ecmg_scs_parameter_type_labels() {
    assert_label!(EcmgScsParameterType::EcmChannelId, "ECM_channel_id");
    assert_label!(EcmgScsParameterType::SuperCasId, "Super_CAS_id");
    let reserved = EcmgScsParameterType::Reserved(0x4242);
    assert_eq!(reserved.name(), "reserved");
    assert_eq!(reserved.to_string(), "reserved(0x4242)");
    assert_label!(
        ParameterType::EcmgScs(EcmgScsParameterType::EcmChannelId),
        "ECM_channel_id"
    );
}

#[test]
fn emmg_mux_parameter_type_labels() {
    assert_label!(EmmgMuxParameterType::Datagram, "datagram");
    assert_label!(EmmgMuxParameterType::ClientId, "client_id");
    let reserved = EmmgMuxParameterType::Reserved(0x4242);
    assert_eq!(reserved.name(), "reserved");
    assert_eq!(reserved.to_string(), "reserved(0x4242)");
    assert_label!(
        ParameterType::EmmgPdgMux(EmmgMuxParameterType::Datagram),
        "datagram"
    );
}

#[test]
fn cpsig_parameter_type_labels() {
    assert_label!(CpSigParameterType::BouquetId, "bouquet_id");
    assert_label!(CpSigParameterType::FlowPid, "flow_PID");
    let reserved = CpSigParameterType::Reserved(0x4242);
    assert_eq!(reserved.name(), "reserved");
    assert_eq!(reserved.to_string(), "reserved(0x4242)");
    assert_label!(
        ParameterType::CpSigPSig(CpSigParameterType::BouquetId),
        "bouquet_id"
    );
}

#[test]
fn value_sub_table_labels() {
    assert_label!(DataType::Emm, "EMM");
    assert_label!(DataType::PrivateData, "private data");
    assert_eq!(DataType::Reserved(0x09).name(), "reserved");
    assert_eq!(DataType::Reserved(0x09).to_string(), "reserved(0x09)");

    assert_label!(SectionTspktFlag::SectionFormat, "section_format");
    assert_label!(SectionTspktFlag::TsPacketFormat, "ts_packet_format");
    assert_label!(SectionTspktFlag::ArbitraryLength, "arbitrary_length");
    assert_eq!(SectionTspktFlag::Reserved(0x09).name(), "reserved");
    assert_eq!(
        SectionTspktFlag::Reserved(0x09).to_string(),
        "reserved(0x09)"
    );
}

#[test]
fn error_status_labels() {
    assert_label!(EcmgErrorStatus::MessageTooLong, "message too long");
    assert_label!(
        EcmgErrorStatus::EcmChannelIdInUse,
        "ECM_channel_id value already in use"
    );
    assert_eq!(EcmgErrorStatus::Reserved(0x4242).name(), "reserved");
    assert_eq!(
        EcmgErrorStatus::Reserved(0x4242).to_string(),
        "reserved(0x4242)"
    );

    assert_label!(
        EmmgErrorStatus::ClientIdInUse,
        "client_id value already in use"
    );
    assert_label!(EmmgErrorStatus::UnrecoverableError, "unrecoverable error");
    assert_eq!(EmmgErrorStatus::Reserved(0x4242).name(), "reserved");
    assert_eq!(
        EmmgErrorStatus::Reserved(0x4242).to_string(),
        "reserved(0x4242)"
    );
}
