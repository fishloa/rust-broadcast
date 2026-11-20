//! CI Plus (TS 103 205) serialized-length contracts, parse offsets, ceilings,
//! descriptor headers and enum labels (audit 23).

use broadcast_common::{Parse, Serialize};
use dvb_ci::Error;
use dvb_ci::ci_plus::cicam_player::{
    ComponentType, ControlCommand, InputStatus, PlayStatus, PlayerAssetEnd,
    PlayerCapabilitiesReply, PlayerControlReq, PlayerInfoReply, PlayerPlayReq, PlayerStartReply,
    PlayerStartReq, PlayerStatusError, PlayerStop, PlayerUpdateReply, PlayerUpdateReq,
    PlayerVerifyReply, PlayerVerifyReq, PlayerVerifyStatus, SeekMode, UpdateStatus,
};
use dvb_ci::reserved::{ReservedOnes, ReservedZero};

const MIN: dvb_ci::length::LengthForm = dvb_ci::length::LengthForm::Minimal;

fn serialize_len<T: Serialize<Error = Error>>(value: &T) -> (usize, Vec<u8>) {
    let mut buf = vec![0u8; value.serialized_len()];
    let n = value.serialize_into(&mut buf).expect("serialize");
    assert_eq!(n, value.serialized_len(), "return != serialized_len");
    assert_eq!(n, buf.len(), "return != produced length");
    (n, buf)
}

#[test]
fn cicam_player_verify_req_contract() {
    let value = PlayerVerifyReq {
        length_form: MIN,
        service_location: b"<xml/>",
    };
    let (n, buf) = serialize_len(&value);
    assert_eq!(n, 12);
    assert_eq!(&buf[..4], &[0x9F, 0xA0, 0x00, 0x08]);
    assert_eq!(&buf[4..6], &[0x00, 0x06]);
}

#[test]
fn cicam_player_verify_reply_contract() {
    let value = PlayerVerifyReply {
        length_form: MIN,
        player_verify_status: PlayerVerifyStatus::Error,
    };
    let (n, buf) = serialize_len(&value);
    assert_eq!(n, 5);
    assert_eq!(buf, [0x9F, 0xA0, 0x01, 0x01, 0x01]);
}

#[test]
fn cicam_player_capabilities_reply_contract() {
    let value = PlayerCapabilitiesReply {
        length_form: MIN,
        component_types: vec![
            ComponentType {
                stream_content: 0x01,
                component_type: 0x11,
                reserved: ReservedOnes::from_bits(0),
            },
            ComponentType {
                stream_content: 0x02,
                component_type: 0x22,
                reserved: ReservedOnes::from_bits(0),
            },
            ComponentType {
                stream_content: 0x03,
                component_type: 0x33,
                reserved: ReservedOnes::from_bits(0),
            },
        ],
    };
    let (n, buf) = serialize_len(&value);
    assert_eq!(n, 12);
    assert_eq!(
        buf,
        [
            0x9F, 0xA0, 0x03, 0x08, 0x00, 0x03, 0x10, 0x11, 0x20, 0x22, 0x30, 0x33
        ]
    );
    assert_eq!(PlayerCapabilitiesReply::parse(&buf).unwrap(), value);
}

#[test]
fn cicam_player_start_req_contract() {
    let value = PlayerStartReq {
        length_form: MIN,
        input_max_bitrate: 0x0102,
        output_max_bitrate: 0x0304,
        linear_channel: true,
        pmt: &[0x02, 0xAA, 0xBB],
        reserved: ReservedZero::from_bits(0),
    };
    let (n, buf) = serialize_len(&value);
    assert_eq!(n, 14);
    assert_eq!(
        buf,
        [
            0x9F, 0xA0, 0x04, 0x0A, 0x01, 0x02, 0x03, 0x04, 0x80, 0x00, 0x03, 0x02, 0xAA, 0xBB
        ]
    );
}

#[test]
fn cicam_player_start_reply_contract() {
    let value = PlayerStartReply {
        length_form: MIN,
        lts_id: 7,
        input_status: InputStatus::RequestRefused,
    };
    let (n, buf) = serialize_len(&value);
    assert_eq!(n, 6);
    assert_eq!(buf, [0x9F, 0xA0, 0x05, 0x02, 0x07, 0x01]);
}

#[test]
fn cicam_player_play_req_contract() {
    let value = PlayerPlayReq {
        length_form: MIN,
        service_location: b"<xml/>",
    };
    let (n, buf) = serialize_len(&value);
    assert_eq!(n, 12);
    assert_eq!(&buf[..4], &[0x9F, 0xA0, 0x06, 0x08]);
}

#[test]
fn cicam_player_status_error_contract() {
    let value = PlayerStatusError {
        length_form: MIN,
        valid_lts_id: true,
        lts_id: 7,
        player_status: PlayStatus::ContentBlocked,
        reserved: ReservedZero::from_bits(0),
    };
    let (n, buf) = serialize_len(&value);
    assert_eq!(n, 7);
    assert_eq!(buf, [0x9F, 0xA0, 0x07, 0x03, 0x01, 0x07, 0x03]);
}

#[test]
fn cicam_player_control_req_contract() {
    let position = PlayerControlReq {
        length_form: MIN,
        lts_id: 7,
        command: ControlCommand::SetPosition {
            seek_mode: SeekMode::Relative,
            seek_position: 0x0102_0304,
        },
    };
    let (n, buf) = serialize_len(&position);
    assert_eq!(n, 11);
    assert_eq!(
        buf,
        [
            0x9F, 0xA0, 0x08, 0x07, 0x07, 0x01, 0x01, 0x01, 0x02, 0x03, 0x04
        ]
    );
    assert_eq!(PlayerControlReq::parse(&buf).unwrap(), position);

    let speed = PlayerControlReq {
        length_form: MIN,
        lts_id: 7,
        command: ControlCommand::SetSpeed { speed: -50 },
    };
    let (n, buf) = serialize_len(&speed);
    assert_eq!(n, 8);
    assert_eq!(buf, [0x9F, 0xA0, 0x08, 0x04, 0x07, 0x02, 0xFF, 0xCE]);
}

#[test]
fn cicam_player_info_reply_contract() {
    let value = PlayerInfoReply {
        length_form: MIN,
        lts_id: 7,
        duration: 0x0102_0304,
        position: 0x0506_0708,
        speed: -100,
    };
    let (n, buf) = serialize_len(&value);
    assert_eq!(n, 15);
    assert_eq!(
        buf,
        [
            0x9F, 0xA0, 0x0A, 0x0B, 0x07, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0xFF,
            0x9C
        ]
    );
}

#[test]
fn cicam_player_asset_end_contract() {
    let value = PlayerAssetEnd {
        length_form: MIN,
        lts_id: 7,
        beginning: true,
        reserved: ReservedOnes::from_bits(0x7F),
    };
    let (n, buf) = serialize_len(&value);
    assert_eq!(n, 6);
    assert_eq!(buf, [0x9F, 0xA0, 0x0D, 0x02, 0x07, 0xFF]);
}

#[test]
fn cicam_player_update_reply_contract() {
    let value = PlayerUpdateReply {
        length_form: MIN,
        lts_id: 7,
        update_status: UpdateStatus::RequestRefused,
    };
    let (n, buf) = serialize_len(&value);
    assert_eq!(n, 6);
    assert_eq!(buf, [0x9F, 0xA0, 0x0F, 0x02, 0x07, 0x01]);
}

#[test]
fn cicam_player_update_req_contract() {
    let value = PlayerUpdateReq {
        length_form: MIN,
        lts_id: 7,
        pmt: &[0x02, 0xAA, 0xBB],
    };
    let (n, buf) = serialize_len(&value);
    assert_eq!(n, 10);
    assert_eq!(
        buf,
        [0x9F, 0xA0, 0x0E, 0x06, 0x07, 0x00, 0x03, 0x02, 0xAA, 0xBB]
    );
}

#[test]
fn cicam_player_stop_contract() {
    let value = PlayerStop {
        length_form: MIN,
        lts_id: 7,
    };
    let (n, buf) = serialize_len(&value);
    assert_eq!(n, 5);
    assert_eq!(buf, [0x9F, 0xA0, 0x0B, 0x01, 0x07]);
}

#[test]
fn cicam_player_enum_labels() {
    assert_eq!(PlayerVerifyStatus::Ok.name(), "ok");
    assert_eq!(PlayerVerifyStatus::Error.name(), "error");
    assert_eq!(PlayerVerifyStatus::Reserved(2).name(), "reserved");
    assert_eq!(InputStatus::Ok.name(), "ok");
    assert_eq!(InputStatus::RequestRefused.name(), "request_refused");
    assert_eq!(InputStatus::Reserved(2).name(), "reserved");
    assert_eq!(PlayStatus::PlayNotPossible.name(), "play_not_possible");
    assert_eq!(PlayStatus::Unrecoverable.name(), "unrecoverable_error");
    assert_eq!(PlayStatus::ContentBlocked.name(), "content_blocked");
    assert_eq!(PlayStatus::Reserved(0).name(), "reserved");
    assert_eq!(SeekMode::Absolute.name(), "absolute");
    assert_eq!(SeekMode::Relative.name(), "relative");
    assert_eq!(SeekMode::Reserved(2).name(), "reserved");
    assert_eq!(UpdateStatus::Ok.name(), "ok");
    assert_eq!(UpdateStatus::RequestRefused.name(), "request_refused");
    assert_eq!(UpdateStatus::Reserved(2).name(), "reserved");
}

#[test]
fn ci_plus_resource_labels() {
    use dvb_ci::ci_plus::{CiPlusResource, classify};
    let table = [
        (
            dvb_ci::ci_plus::MULTISTREAM,
            "multistream",
            CiPlusResource::Multistream,
        ),
        (
            dvb_ci::ci_plus::CONTENT_CONTROL,
            "content_control",
            CiPlusResource::ContentControl,
        ),
        (
            dvb_ci::ci_plus::SAMPLE_DECRYPTION,
            "sample_decryption",
            CiPlusResource::SampleDecryption,
        ),
        (
            dvb_ci::ci_plus::CICAM_PLAYER,
            "cicam_player",
            CiPlusResource::CicamPlayer,
        ),
        (
            dvb_ci::ci_plus::FILE_RETRIEVAL,
            "file_retrieval",
            CiPlusResource::FileRetrieval,
        ),
    ];
    for (id, label, expected) in table {
        assert_eq!(classify(id), Some(expected));
        assert_eq!(expected.name(), label);
        assert_eq!(format!("{expected}"), label);
    }
    assert_eq!(
        CiPlusResource::MultistreamHostControl(
            dvb_ci::ci_plus::multistream_host_control::HostControlMode::BaseV3
        )
        .name(),
        "multistream_host_control"
    );
}

#[test]
fn sample_decryption_info_reply_and_returns() {
    use dvb_ci::ci_plus::sample_decryption::SdInfoReply;
    let value = SdInfoReply {
        length_form: MIN,
        drm_system_ids: vec![0x0102, 0x0304, 0x0506],
        drm_uuids: vec![[0x11; 16], [0x22; 16]],
    };
    let (n, buf) = serialize_len(&value);
    assert_eq!(n, 44);
    assert_eq!(&buf[..4], &[0x9F, 0x98, 0x01, 0x28]);
    assert_eq!(buf[4], 0x03);
    assert_eq!(&buf[5..11], &[0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);
    assert_eq!(buf[11], 0x02);
    assert_eq!(buf[12], 0x11);
}

#[test]
fn sample_decryption_start_and_update_contracts() {
    use dvb_ci::ci_plus::sample_decryption::{
        DrmMetadataRecord, DrmStatus, SdStart, SdStartReply, SdUpdate, SdUpdateReply,
        TransmissionStatus,
    };
    let metadata = [0xDE, 0xAD, 0xBE];
    let record = DrmMetadataRecord {
        drm_metadata_source: 0x05,
        drm_system_id: 0x1234,
        drm_uuid: [0x33; 16],
        drm_metadata: &metadata,
    };
    let start = SdStart {
        length_form: MIN,
        lts_id: 0x07,
        program_number: 0x0102,
        payload: dvb_ci::ci_plus::sample_decryption::SamplePayload::Ts(vec![record.clone()]),
        reserved: ReservedZero::from_bits(0),
    };
    let (n, buf) = serialize_len(&start);
    assert_eq!(n, 33);
    assert_eq!(&buf[..4], &[0x9F, 0x98, 0x02, 0x1D]);
    assert_eq!(buf[4], 0x07);
    assert_eq!(&buf[5..7], &[0x01, 0x02]);
    assert_eq!(buf[7], 0x01);
    assert_eq!(SdStart::parse(&buf).unwrap(), start);

    let update = SdUpdate {
        length_form: MIN,
        lts_id: 0x07,
        payload: dvb_ci::ci_plus::sample_decryption::SamplePayload::Ts(vec![record.clone()]),
        reserved: ReservedZero::from_bits(0),
    };
    let (n, buf) = serialize_len(&update);
    assert_eq!(n, 31);
    assert_eq!(&buf[..4], &[0x9F, 0x98, 0x04, 0x1B]);
    assert_eq!(buf[4], 0x07);
    assert_eq!(SdUpdate::parse(&buf).unwrap(), update);

    let start_reply = SdStartReply {
        length_form: MIN,
        lts_id: 0x07,
        transmission_status: TransmissionStatus::CicamBusy,
        drm_status: DrmStatus::NoEntitlement,
        drm_system_id: 0x1234,
        drm_uuid: [0x33; 16],
        buffer_size: 0x2000,
        data_block_size: 0x0400,
    };
    let (n, buf) = serialize_len(&start_reply);
    assert_eq!(n, 29);
    assert_eq!(&buf[..4], &[0x9F, 0x98, 0x03, 0x19]);

    let update_reply = SdUpdateReply {
        length_form: MIN,
        lts_id: 0x07,
        drm_status: DrmStatus::Undetermined,
    };
    let (n, buf) = serialize_len(&update_reply);
    assert_eq!(n, 6);
    assert_eq!(buf, [0x9F, 0x98, 0x05, 0x02, 0x07, 0x01]);

    let debug = format!("{record:?}");
    assert!(debug.contains("DrmMetadataRecord"), "{debug}");
    assert!(debug.contains("<3 bytes redacted>"), "{debug}");
    assert!(!debug.contains("222"), "{debug}");
}

#[test]
fn sample_decryption_track_reserved_survives_round_trip() {
    use dvb_ci::ci_plus::sample_decryption::{
        DrmMetadataRecord, SamplePayload, SampleTrack, SdUpdate,
    };
    let metadata = [0x01];
    let value = SdUpdate {
        length_form: MIN,
        lts_id: 1,
        payload: SamplePayload::Tracks(vec![SampleTrack {
            track_pid: mpeg_ts::pid::Pid::from_const(0x0200),
            records: vec![DrmMetadataRecord {
                drm_metadata_source: 1,
                drm_system_id: 2,
                drm_uuid: [0x44; 16],
                drm_metadata: &metadata,
            }],
            reserved: ReservedZero::from_bits(5),
        }]),
        reserved: ReservedZero::from_bits(0),
    };
    let (n, buf) = serialize_len(&value);
    assert_eq!(n, 3 + 1 + 2 + 1 + 3 + 21 + 1);
    assert_eq!(buf[7], 0xA2);
    assert_eq!(buf[8], 0x00);
    assert_eq!(SdUpdate::parse(&buf).unwrap(), value);
}

#[test]
fn sample_decryption_status_labels() {
    use dvb_ci::ci_plus::sample_decryption::{DrmStatus, TransmissionStatus};
    assert_eq!(
        TransmissionStatus::ReadyToReceive.name(),
        "ready_to_receive"
    );
    assert_eq!(TransmissionStatus::Reserved(3).name(), "reserved");
    assert_eq!(DrmStatus::DecryptionPossible.name(), "decryption_possible");
    assert_eq!(DrmStatus::Undetermined.name(), "status_undetermined");
    assert_eq!(DrmStatus::NoEntitlement.name(), "error_no_entitlement");
    assert_eq!(DrmStatus::Reserved(3).name(), "reserved");
}

#[test]
fn lsc_v4_info_and_ip_config_contracts() {
    use dvb_ci::ci_plus::low_speed_comms_v4::{
        CommsInfoReply, CommsIpConfigReply, ConnectionState, IpConfig,
    };
    let info = CommsInfoReply {
        length_form: MIN,
        lts_id: 7,
        status: true,
        source_ip_address: [0x11; 16],
        source_port: 0x1234,
        input_delivery_pid: mpeg_ts::pid::Pid::from_const(0x0567),
        reserved_status: ReservedZero::from_bits(0),
        reserved_pid: ReservedZero::from_bits(0),
    };
    let (n, buf) = serialize_len(&info);
    assert_eq!(n, 26);
    assert_eq!(&buf[..4], &[0x9F, 0x8C, 0x08, 0x16]);
    assert_eq!(buf[4], 0x07);
    assert_eq!(buf[5], 0x01);
    assert_eq!(&buf[22..24], &[0x12, 0x34]);
    assert_eq!(&buf[24..26], &[0x05, 0x67]);
    assert_eq!(CommsInfoReply::parse(&buf).unwrap(), info);

    let config = IpConfig::new([0x21; 16], [0x22; 16], [0x23; 16])
        .with_dhcp_server([0x24; 16])
        .with_dns_servers(vec![[0x25; 16], [0x26; 16]]);
    let reply = CommsIpConfigReply {
        length_form: MIN,
        connection_state: ConnectionState::Connected,
        physical_address: [0x01, 0x02, 0x03, 0x04, 0x05, 0x06],
        ip_config: Some(config),
        reserved: ReservedZero::from_bits(0),
    };
    let (n, buf) = serialize_len(&reply);
    assert_eq!(n, 108);
    assert_eq!(&buf[..4], &[0x9F, 0x8C, 0x0A, 0x68]);
    assert_eq!(buf[4], 0x40);
    assert_eq!(&buf[5..11], &[0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);
    assert_eq!(&buf[11..27], &[0x21; 16]);
    assert_eq!(buf[75], 0x02);
    assert_eq!(&buf[76..92], &[0x25; 16]);
    assert_eq!(CommsIpConfigReply::parse(&buf).unwrap(), reply);
    let mut truncated = buf[..100].to_vec();
    truncated[3] = 96;
    assert!(CommsIpConfigReply::parse(&truncated).is_err());
}

#[test]
fn lsc_v4_descriptor_headers_and_ceilings() {
    use dvb_ci::ci_plus::low_speed_comms_v4::{
        HybridDescriptor, IpConnectionType, IpProtocolVersion, MulticastDescriptor,
    };
    let hybrid = HybridDescriptor {
        lts_id: 7,
        ip_connection_type: IpConnectionType::HostnameDescriptor,
        inner: &[0xAA, 0xBB, 0xCC],
    };
    assert_eq!(hybrid.serialized_len(), 7);
    let (n, buf) = serialize_len(&hybrid);
    assert_eq!(n, 7);
    assert_eq!(buf, [0x05, 0x05, 0x07, 0x04, 0xAA, 0xBB, 0xCC]);
    assert_eq!(HybridDescriptor::parse(&buf).unwrap(), hybrid);

    let max = vec![0xAAu8; 253];
    let at_limit = HybridDescriptor {
        lts_id: 1,
        ip_connection_type: IpConnectionType::IpDescriptor,
        inner: &max,
    };
    let mut out = vec![0u8; at_limit.serialized_len()];
    assert_eq!(at_limit.serialize_into(&mut out).unwrap(), 257);
    assert_eq!(out[1], 0xFF);
    let over = vec![0xAAu8; 254];
    let too_long = HybridDescriptor {
        lts_id: 1,
        ip_connection_type: IpConnectionType::IpDescriptor,
        inner: &over,
    };
    let mut out = vec![0u8; too_long.serialized_len()];
    assert!(matches!(
        too_long.serialize_into(&mut out),
        Err(Error::LengthTooLarge(256))
    ));

    assert!(matches!(
        HybridDescriptor::parse(&[0x05]),
        Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: 2,
                have: 1,
                ..
            }
        ))
    ));
    assert!(matches!(
        HybridDescriptor::parse(&[0x05, 0x00]),
        Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: 2,
                have: 0,
                ..
            }
        ))
    ));
    assert!(matches!(
        HybridDescriptor::parse(&[0x05, 0x03, 0xAA]),
        Err(Error::LengthMismatch(
            broadcast_common::error::LengthMismatch {
                declared: 3,
                actual: 1,
                ..
            }
        ))
    ));

    let multicast = MulticastDescriptor {
        ip_protocol_version: IpProtocolVersion::Ipv4,
        ip_address: [0x31; 16],
        multicast_port: 0x1234,
        include_sources: true,
        source_addresses: vec![[0x41; 16], [0x42; 16]],
    };
    assert_eq!(multicast.serialized_len(), 55);
    let (n, buf) = serialize_len(&multicast);
    assert_eq!(n, 55);
    assert_eq!(&buf[..2], &[0x06, 0x35]);
    assert_eq!(buf[2], 0x01);
    assert_eq!(&buf[19..21], &[0x12, 0x34]);
    assert_eq!(buf[21], 0x01);
    assert_eq!(buf[22], 0x02);
    assert_eq!(MulticastDescriptor::parse(&buf).unwrap(), multicast);
    let mut truncated = buf[..50].to_vec();
    truncated[1] = 48;
    assert!(MulticastDescriptor::parse(&truncated).is_err());
}

#[test]
fn lsc_v4_enum_labels() {
    use dvb_ci::ci_plus::low_speed_comms_v4::{
        ConnectionState, IpConnectionType, IpProtocolVersion,
    };
    assert_eq!(ConnectionState::Disconnected.name(), "disconnected");
    assert_eq!(ConnectionState::Connected.name(), "connected");
    assert_eq!(ConnectionState::Reserved(2).name(), "reserved");
    assert_eq!(IpProtocolVersion::Ipv4.name(), "ipv4");
    assert_eq!(IpProtocolVersion::Ipv6.name(), "ipv6");
    assert_eq!(IpProtocolVersion::Reserved(0).name(), "reserved");
    assert_eq!(IpConnectionType::IpDescriptor.name(), "ip_descriptor");
    assert_eq!(
        IpConnectionType::HostnameDescriptor.name(),
        "hostname_descriptor"
    );
    assert_eq!(
        IpConnectionType::MulticastDescriptor.name(),
        "multicast_descriptor"
    );
    assert_eq!(IpConnectionType::Reserved(0).name(), "reserved");
}

#[test]
fn multistream_host_control_contracts() {
    use dvb_ci::ci_plus::multistream_host_control::{
        HostControlMode, MultistreamHostControlApdu, TuneIpReq, TuneLcnReq, TuneTripletReq,
        TunerStatusDsd, TunerStatusReply, TunerStatusReq,
    };
    let triplet = TuneTripletReq {
        length_form: MIN,
        background_tune: true,
        tune_quietly: true,
        keep_app_running: false,
        original_network_id: 0x0102,
        transport_stream_id: 0x0304,
        service_id: 0x0506,
        delivery_system_descriptor_tag: 0x7F,
        descriptor_tag_extension: Some(0x09),
        reserved: ReservedZero::from_bits(0),
        reserved_ext: ReservedZero::from_bits(0),
    };
    let (n, buf) = serialize_len(&triplet);
    assert_eq!(n, 13);
    assert_eq!(
        buf,
        [
            0x9F, 0x84, 0x09, 0x09, 0x06, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x7F, 0x09
        ]
    );
    assert_eq!(TuneTripletReq::parse(&buf).unwrap(), triplet);

    let lcn = TuneLcnReq {
        length_form: MIN,
        background_tune: true,
        tune_quietly: true,
        keep_app_running: false,
        logical_channel_number: 0x0123,
        reserved: ReservedZero::from_bits(0),
    };
    let (n, buf) = serialize_len(&lcn);
    assert_eq!(n, 7);
    assert_eq!(buf, [0x9F, 0x84, 0x07, 0x03, 0x01, 0x81, 0x23]);
    assert_eq!(TuneLcnReq::parse(&buf).unwrap(), lcn);

    let ip = TuneIpReq {
        length_form: MIN,
        mode: HostControlMode::MultiStream,
        background_tune: true,
        tune_quietly: false,
        keep_app_running: false,
        service_location_data: &[0xAA, 0xBB, 0xCC],
        reserved: ReservedZero::from_bits(0),
    };
    let (n, buf) = serialize_len(&ip);
    assert_eq!(n, 9);
    assert_eq!(buf, [0x9F, 0x84, 0x08, 0x05, 0x40, 0x03, 0xAA, 0xBB, 0xCC]);
    assert_eq!(
        TuneIpReq::parse_mode(&buf, HostControlMode::MultiStream).unwrap(),
        ip
    );

    let reply = TunerStatusReply {
        length_form: MIN,
        ip_tune_capable: true,
        dsds: vec![
            TunerStatusDsd {
                connected: true,
                delivery_system_descriptor_tag: 0x7F,
                descriptor_tag_extension: Some(0x05),
                reserved: ReservedZero::from_bits(0),
                reserved_ext: ReservedZero::from_bits(0),
            },
            TunerStatusDsd {
                connected: false,
                delivery_system_descriptor_tag: 0x11,
                descriptor_tag_extension: None,
                reserved: ReservedZero::from_bits(0),
                reserved_ext: ReservedZero::from_bits(0),
            },
        ],
    };
    let (n, buf) = serialize_len(&reply);
    assert_eq!(n, 11);
    assert_eq!(
        buf,
        [
            0x9F, 0x84, 0x0B, 0x07, 0x82, 0x01, 0x7F, 0x05, 0x00, 0x11, 0x00
        ]
    );
    assert_eq!(TunerStatusReply::parse(&buf).unwrap(), reply);

    let (_, status_req) = serialize_len(&TunerStatusReq { length_form: MIN });
    let (_, triplet_req) = serialize_len(&triplet);
    let (_, lcn_req) = serialize_len(&lcn);
    let (_, ip_req) = serialize_len(&ip);
    let (_, status_reply) = serialize_len(&reply);
    for (wire, expected) in [
        (&triplet_req, "tune_triplet_req"),
        (&lcn_req, "tune_lcn_req"),
        (&ip_req, "tune_ip_req"),
        (&status_req, "tuner_status_req"),
        (&status_reply, "tuner_status_reply"),
    ] {
        let parsed = MultistreamHostControlApdu::parse_mode(wire, HostControlMode::MultiStream)
            .expect("dispatches");
        let label = match parsed {
            MultistreamHostControlApdu::TuneTripletReq(_) => "tune_triplet_req",
            MultistreamHostControlApdu::TuneLcnReq(_) => "tune_lcn_req",
            MultistreamHostControlApdu::TuneIpReq(_) => "tune_ip_req",
            MultistreamHostControlApdu::TunerStatusReq(_) => "tuner_status_req",
            MultistreamHostControlApdu::TunerStatusReply(_) => "tuner_status_reply",
            _ => "unknown",
        };
        assert_eq!(label, expected);
        assert_eq!(parsed.serialized_len(), wire.len());
    }
    assert_eq!(HostControlMode::MultiStream.name(), "multi_stream");
    assert_eq!(HostControlMode::BaseV3.name(), "base_v3");
}

#[test]
fn multistream_contracts() {
    use dvb_ci::ci_plus::multistream::{
        CicamMultistreamCapability, PidSelectReply, PidSelectReq, PidSelectRequest,
        PidSelectedEntry,
    };
    let capability = CicamMultistreamCapability {
        length_form: MIN,
        max_local_ts: 3,
        max_descramblers: 0x0102,
    };
    let (n, buf) = serialize_len(&capability);
    assert_eq!(n, 7);
    assert_eq!(buf, [0x9F, 0x92, 0x00, 0x03, 0x03, 0x01, 0x02]);
    assert_eq!(CicamMultistreamCapability::parse(&buf).unwrap(), capability);

    let req = PidSelectReq {
        length_form: MIN,
        lts_id: 7,
        pids: vec![
            PidSelectRequest {
                critical_for_descrambling: true,
                pid: mpeg_ts::pid::Pid::from_const(0x0102),
                reserved: ReservedZero::from_bits(0),
            },
            PidSelectRequest {
                critical_for_descrambling: false,
                pid: mpeg_ts::pid::Pid::from_const(0x0203),
                reserved: ReservedZero::from_bits(0),
            },
            PidSelectRequest {
                critical_for_descrambling: false,
                pid: mpeg_ts::pid::Pid::from_const(0x0304),
                reserved: ReservedZero::from_bits(0),
            },
        ],
    };
    assert_eq!(req.serialized_len(), 12);
    let (n, buf) = serialize_len(&req);
    assert_eq!(n, 12);
    assert_eq!(&buf[..4], &[0x9F, 0x92, 0x01, 0x08]);
    assert_eq!(buf[4], 0x07);
    assert_eq!(buf[5], 0x03);
    assert_eq!(&buf[6..8], &[0x21, 0x02]);
    assert_eq!(PidSelectReq::parse(&buf).unwrap(), req);

    let reply = PidSelectReply {
        length_form: MIN,
        lts_id: 7,
        pid_selection: true,
        pids: vec![
            PidSelectedEntry {
                pid_selected: true,
                pid: mpeg_ts::pid::Pid::from_const(0x0102),
                reserved: ReservedZero::from_bits(0),
            },
            PidSelectedEntry {
                pid_selected: false,
                pid: mpeg_ts::pid::Pid::from_const(0x0203),
                reserved: ReservedZero::from_bits(0),
            },
            PidSelectedEntry {
                pid_selected: true,
                pid: mpeg_ts::pid::Pid::from_const(0x0304),
                reserved: ReservedZero::from_bits(0),
            },
        ],
        reserved: ReservedZero::from_bits(0),
    };
    assert_eq!(reply.serialized_len(), 13);
    let (n, buf) = serialize_len(&reply);
    assert_eq!(n, 13);
    assert_eq!(&buf[..4], &[0x9F, 0x92, 0x02, 0x09]);
    assert_eq!(buf[4], 0x07);
    assert_eq!(buf[5], 0x01);
    assert_eq!(buf[6], 0x03);
    assert_eq!(PidSelectReply::parse(&buf).unwrap(), reply);
}

#[test]
fn content_control_contracts() {
    use dvb_ci::ci_plus::content_control::{
        CcPinEvent, CcPinReply, DatatypeId, OperatingMode, SacDatatype,
    };
    let reply = CcPinReply {
        length_form: MIN,
        lts_id: Some(7),
        pincode_status: 0x09,
        reserved: ReservedZero::from_bits(0),
        reserved_lts: ReservedZero::from_bits(0),
    };
    let (n, buf) = serialize_len(&reply);
    assert_eq!(n, 7);
    assert_eq!(buf, [0x9F, 0x90, 0x14, 0x03, 0x01, 0x07, 0x09]);
    assert_eq!(CcPinReply::parse(&buf).unwrap(), reply);

    let event = CcPinEvent {
        length_form: MIN,
        lts_id: 7,
        program_number: 0x0102,
        pincode_status: 0x03,
        rating: 0x04,
        pin_event_time_utc: 0x0001_0203_0405,
        pin_event_time_centiseconds: 0x06,
        private_data: [0x11; 15],
    };
    let (n, buf) = serialize_len(&event);
    assert_eq!(n, 30);
    assert_eq!(&buf[..4], &[0x9F, 0x90, 0x15, 0x1A]);
    assert_eq!(&buf[9..14], &[0x01, 0x02, 0x03, 0x04, 0x05]);
    assert_eq!(buf[14], 0x06);
    assert_eq!(CcPinEvent::parse(&buf).unwrap(), event);

    let datatype = SacDatatype {
        datatype_id: DatatypeId::OperatingMode,
        value: &[0xAA, 0xBB, 0xCC, 0xDD],
    };
    assert_eq!(datatype.serialized_len(), 7);

    assert_eq!(OperatingMode::Timeshift.to_u8(), 0x01);
    assert_eq!(OperatingMode::UnattendedRecording.to_u8(), 0x02);
    assert_eq!(OperatingMode::Other(0x33).to_u8(), 0x33);
    assert_eq!(OperatingMode::Timeshift.name(), "Timeshift");
    assert_eq!(
        OperatingMode::UnattendedRecording.name(),
        "Unattended_Recording"
    );
    assert_eq!(OperatingMode::Other(0x33).name(), "reserved");
}

#[test]
fn ca_support_ms_ca_pmt_program_info_high_nibble() {
    use dvb_ci::ci_plus::ca_support::MsCaPmt;
    use dvb_ci::objects::ca_pmt::{CaPmtCmdId, CaPmtListManagement, CaPmtStream};
    let program = vec![0x09u8; 300];
    let es = vec![0x09u8; 2];
    let value = MsCaPmt {
        length_form: MIN,
        lts_id: 7,
        list_management: CaPmtListManagement::Only,
        program_number: 0x0001,
        pmt_pid: mpeg_ts::pid::Pid::from_const(0x0100),
        version_number: 3,
        current_next_indicator: true,
        cmd_id: Some(CaPmtCmdId::OkDescrambling),
        program_ca_descriptors: &program,
        streams: vec![CaPmtStream {
            stream_type: 0x81,
            elementary_pid: mpeg_ts::pid::Pid::from_const(0x0200),
            cmd_id: Some(CaPmtCmdId::Query),
            ca_descriptors: &es,
            reserved_pid: ReservedOnes::from_bits(0b111),
            reserved_es_info: ReservedOnes::from_bits(0b1111),
        }],
        reserved_pmt_pid: ReservedOnes::from_bits(0b111),
        reserved_version: ReservedOnes::from_bits(0b11),
        reserved_program_info: ReservedOnes::from_bits(0b1111),
    };
    let (n, buf) = serialize_len(&value);
    assert_eq!(n, 324);
    assert_eq!(&buf[..3], &[0x9F, 0x80, 0x32]);
    assert_eq!(buf[13], 0xF1);
    assert_eq!(buf[14], 0x2D);
    assert_eq!(MsCaPmt::parse(&buf).unwrap(), value);
}

#[test]
fn ci_plus_descriptor_tlv_ceiling() {
    use dvb_ci::ci_plus::descriptors::{CiplusInitializationVectorDescriptor, IV_DESCRIPTOR_TAG};
    let value = CiplusInitializationVectorDescriptor {
        iv_data: &[0xA1, 0xA2],
    };
    let (n, buf) = serialize_len(&value);
    assert_eq!(n, 4);
    assert_eq!(buf, [IV_DESCRIPTOR_TAG, 0x02, 0xA1, 0xA2]);
    assert_eq!(
        CiplusInitializationVectorDescriptor::parse(&buf).unwrap(),
        value
    );

    let max = vec![0xA1u8; 255];
    let at_limit = CiplusInitializationVectorDescriptor { iv_data: &max };
    let mut out = vec![0u8; at_limit.serialized_len()];
    assert_eq!(at_limit.serialize_into(&mut out).unwrap(), 257);
    assert_eq!(out[1], 0xFF);
    let over = vec![0xA1u8; 256];
    let too_long = CiplusInitializationVectorDescriptor { iv_data: &over };
    let mut out = vec![0u8; too_long.serialized_len()];
    assert!(matches!(
        too_long.serialize_into(&mut out),
        Err(Error::LengthTooLarge(256))
    ));

    assert!(matches!(
        CiplusInitializationVectorDescriptor::parse(&[IV_DESCRIPTOR_TAG]),
        Err(Error::BufferTooShort(
            broadcast_common::error::BufferTooShort {
                need: 2,
                have: 1,
                ..
            }
        ))
    ));
    assert!(matches!(
        CiplusInitializationVectorDescriptor::parse(&[IV_DESCRIPTOR_TAG, 0x02, 0xAA]),
        Err(Error::LengthMismatch(
            broadcast_common::error::LengthMismatch {
                declared: 2,
                actual: 1,
                ..
            }
        ))
    ));
}

#[test]
fn file_retrieval_ack_labels() {
    use dvb_ci::ci_plus::file_retrieval::AckCode;
    assert_eq!(AckCode::Ok.name(), "ok");
    assert_eq!(
        AckCode::UnknownDomainIdentifier.name(),
        "unknown_domain_identifier"
    );
    assert_eq!(AckCode::Reserved(0).name(), "reserved");
}
