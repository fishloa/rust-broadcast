//! Every SRT packet type round-trips through `broadcast_common::{Parse, Serialize}`.

use broadcast_common::{Parse, Serialize};
use core::fmt::Debug;
use srt_runtime::Error;
use srt_runtime::packet::misc::KeepAlivePacket;
use srt_runtime::packet::{
    Cipher, ControlPacket, DataPacket, EncryptionKeyField, GroupFlags, GroupMembershipExtension,
    GroupType, HandshakeExtensionMessageFlags, HsExtMessage, KeyMaterial, KmAuth, KmKeyFlag,
    PacketPosition, SrtPacket, StreamEncapsulation,
};

fn through_traits<'a, T>(value: &T, wire: &'a mut Vec<u8>)
where
    T: Parse<'a, Error = Error> + Serialize<Error = Error> + PartialEq + Debug,
{
    *wire = value.try_to_bytes().unwrap();
    assert_eq!(wire.len(), value.serialized_len());
    let parsed = T::parse(wire).unwrap();
    assert_eq!(&parsed, value);
    assert_eq!(parsed.try_to_bytes().unwrap(), *wire);
}

fn data_packet() -> DataPacket<'static> {
    DataPacket {
        seq_number: 0x0123_4567,
        position: PacketPosition::Solo,
        in_order: true,
        key_flag: EncryptionKeyField::Even,
        retransmitted: false,
        message_number: 0x0155_5555,
        timestamp: 0xAABB_CCDD,
        dest_socket_id: 0x1122_3344,
        data: &[0x47, 0x00, 0x01, 0x02],
    }
}

fn keep_alive() -> ControlPacket<'static> {
    ControlPacket::KeepAlive(KeepAlivePacket {
        timestamp: 7,
        dest_socket_id: 9,
        libsrt_pad: false,
    })
}

#[test]
fn data_packet_round_trips() {
    let mut wire = Vec::new();
    through_traits(&data_packet(), &mut wire);
    assert_eq!(wire.len(), 20);
}

#[test]
fn control_packet_round_trips() {
    let mut wire = Vec::new();
    through_traits(&keep_alive(), &mut wire);
}

#[test]
fn srt_packet_round_trips_both_kinds() {
    let mut wire = Vec::new();
    through_traits(&SrtPacket::Data(data_packet()), &mut wire);
    let mut wire = Vec::new();
    through_traits(&SrtPacket::Control(keep_alive()), &mut wire);
}

#[test]
fn key_material_round_trips() {
    let km = KeyMaterial {
        kk: KmKeyFlag::Even,
        keki: 0,
        cipher: Cipher::AesCtr,
        auth: KmAuth::None,
        se: StreamEncapsulation::MpegTsSrt,
        salt: &[0xAA; 16],
        icv: [1, 2, 3, 4, 5, 6, 7, 8],
        x_sek: &[0xEE; 16],
        o_sek: None,
    };
    let mut wire = Vec::new();
    through_traits(&km, &mut wire);
    assert_eq!(wire.len(), 56);
}

#[test]
fn hs_ext_message_round_trips() {
    let m = HsExtMessage {
        srt_version: 0x0001_0500,
        srt_flags: HandshakeExtensionMessageFlags(0x0000_00BF),
        receiver_tsbpd_delay_ms: 120,
        sender_tsbpd_delay_ms: 80,
    };
    let mut wire = Vec::new();
    through_traits(&m, &mut wire);
    assert_eq!(wire.len(), 12);
}

#[test]
fn group_membership_extension_round_trips() {
    let g = GroupMembershipExtension {
        group_id: 42,
        group_type: GroupType::MainBackup,
        flags: GroupFlags(0x01),
        weight: 7,
    };
    let mut wire = Vec::new();
    through_traits(&g, &mut wire);
}

#[test]
fn serialize_into_short_buffer_is_output_buffer_too_small() {
    let m = HsExtMessage {
        srt_version: 1,
        srt_flags: HandshakeExtensionMessageFlags(0),
        receiver_tsbpd_delay_ms: 0,
        sender_tsbpd_delay_ms: 0,
    };
    assert_eq!(
        Serialize::serialize_into(&m, &mut [0u8; 11]),
        Err(Error::OutputBufferTooSmall { need: 12, have: 11 })
    );
}
