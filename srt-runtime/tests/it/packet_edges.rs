//! Packet-codec edge cases: Key Material widths, handshake extension offsets,
//! reserved control fields, Stream ID words and data-packet field decoding
//! (audit 21 #2930).

use broadcast_core::Serialize;
use core::time::Duration;
use srt_runtime::Error;
use srt_runtime::packet::handshake::{
    HS_EXT_FLAG_CONFIG, HS_EXT_FLAG_HSREQ, HS_EXT_FLAG_KMREQ, encode_stream_id,
};
use srt_runtime::packet::{
    Cipher, ControlPacket, DataPacket, EncryptionField, EncryptionKeyField, ExtensionType,
    GroupFlags, GroupMembershipExtension, GroupType, HandshakeExtensionBlock,
    HandshakeExtensionFlags, HandshakeExtensionMessageFlags, HandshakeExtensions, HandshakeType,
    HsExtMessage, KeyMaterial, KmAuth, KmKeyFlag, PacketPosition, SocketId, SrtTimestamp,
    StreamEncapsulation, UserDefinedPacket,
};

/// A wire word from its big-endian bytes.
fn word(bytes: &[u8]) -> u32 {
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

/// One control packet header with the given type/subtype/info words.
fn control_header(control_type: u16, subtype: u16, info: u32) -> Vec<u8> {
    let mut bytes = vec![0u8; 16];
    let word0 = 0x8000_0000 | (u32::from(control_type) << 16) | u32::from(subtype);
    bytes[0..4].copy_from_slice(&word0.to_be_bytes());
    bytes[4..8].copy_from_slice(&info.to_be_bytes());
    bytes
}

/// A Key Material message with the widest shape (`KK=Both`, AES-256, a set
/// AUTH) encodes to hand-computed bytes and parses back equal.
#[test]
fn key_material_both_aes256_hand_written_bytes() {
    let salt: [u8; 16] = [
        0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E,
        0x1F,
    ];
    let icv: [u8; 8] = [0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27];
    let x: [u8; 32] = [
        0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3A, 0x3B, 0x3C, 0x3D, 0x3E,
        0x3F, 0x40, 0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4A, 0x4B, 0x4C, 0x4D,
        0x4E, 0x4F,
    ];
    let o: [u8; 32] = [
        0x50, 0x51, 0x52, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5A, 0x5B, 0x5C, 0x5D, 0x5E,
        0x5F, 0x60, 0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6A, 0x6B, 0x6C, 0x6D,
        0x6E, 0x6F,
    ];
    let km = KeyMaterial {
        kk: KmKeyFlag::Both,
        keki: 0x1122_3344,
        cipher: Cipher::AesCtr,
        auth: KmAuth::Reserved(7),
        se: StreamEncapsulation::MpegTsSrt,
        salt: &salt,
        icv,
        x_sek: &x,
        o_sek: Some(&o),
    };
    assert_eq!(km.serialized_len(), 16 + 16 + 8 + 32 + 32);
    let mut buf = vec![0u8; km.serialized_len()];
    assert_eq!(km.serialize_into(&mut buf).unwrap(), buf.len());

    let mut expected = Vec::new();
    expected.extend_from_slice(&0x1220_2903u32.to_be_bytes());
    expected.extend_from_slice(&0x1122_3344u32.to_be_bytes());
    expected.extend_from_slice(&0x0207_0200u32.to_be_bytes());
    expected.extend_from_slice(&0x0000_0408u32.to_be_bytes());
    expected.extend_from_slice(&salt);
    expected.extend_from_slice(&icv);
    expected.extend_from_slice(&x);
    expected.extend_from_slice(&o);
    assert_eq!(buf, expected);
    assert_eq!(word(&buf[12..16]), 0x0000_0408, "SLen/4 = 4, KLen/4 = 8");

    let parsed = KeyMaterial::parse(&buf).unwrap();
    assert_eq!(parsed, km);
    assert_eq!(parsed.o_sek, Some(&o[..]));
    assert_eq!(parsed.salt, &salt[..]);
}

/// A `KK=Both` message whose `oSEK` is a different length is refused.
#[test]
fn key_material_rejects_mismatched_o_sek_length() {
    let salt = [0xAAu8; 16];
    let x = [0xEEu8; 32];
    let o = [0xDDu8; 24];
    let km = KeyMaterial {
        kk: KmKeyFlag::Both,
        keki: 0,
        cipher: Cipher::AesCtr,
        auth: KmAuth::None,
        se: StreamEncapsulation::MpegTsSrt,
        salt: &salt,
        icv: [0; 8],
        x_sek: &x,
        o_sek: Some(&o),
    };
    let mut buf = vec![0u8; km.serialized_len()];
    assert!(matches!(
        km.serialize_into(&mut buf),
        Err(Error::InvalidKeyMaterial {
            field: "KK/oSEK",
            ..
        })
    ));
}

/// `KmAuth::to_bits` returns the wire byte for both variants.
#[test]
fn km_auth_to_bits_is_the_wire_value() {
    assert_eq!(KmAuth::None.to_bits(), 0);
    assert_eq!(KmAuth::Reserved(7).to_bits(), 7);
    assert_eq!(KmAuth::from_bits(7), KmAuth::Reserved(7));
}

/// The three extension-flag accessors read only their own bit.
#[test]
fn handshake_extension_flags_read_only_their_own_bit() {
    for (bits, hsreq, kmreq, config) in [
        (0u16, false, false, false),
        (HS_EXT_FLAG_HSREQ, true, false, false),
        (HS_EXT_FLAG_KMREQ, false, true, false),
        (HS_EXT_FLAG_CONFIG, false, false, true),
        (
            HS_EXT_FLAG_HSREQ | HS_EXT_FLAG_KMREQ | HS_EXT_FLAG_CONFIG,
            true,
            true,
            true,
        ),
    ] {
        let flags = HandshakeExtensionFlags(bits);
        assert_eq!(flags.hsreq(), hsreq, "HSREQ for {bits:#06x}");
        assert_eq!(flags.kmreq(), kmreq, "KMREQ for {bits:#06x}");
        assert_eq!(flags.config(), config, "CONFIG for {bits:#06x}");
    }
}

/// The Stream ID extension is 32-bit little-endian words; `"abcde"` pads to
/// eight bytes and trims back, and a non-multiple of four is an error with
/// the padded length as `need`.
#[test]
fn stream_id_extension_undoes_word_storage_and_padding() {
    let contents = encode_stream_id("abcde");
    assert_eq!(contents.len(), 8);
    let block = HandshakeExtensionBlock {
        ext_type: ExtensionType::Sid,
        contents: &contents,
    };
    assert_eq!(block.as_stream_id().unwrap(), "abcde");

    let short = HandshakeExtensionBlock {
        ext_type: ExtensionType::Sid,
        contents: &[0x01, 0x02, 0x03],
    };
    match short.as_stream_id() {
        Err(Error::BufferTooShort(b)) => {
            assert_eq!(b.need, 4);
            assert_eq!(b.have, 3);
        }
        other => panic!("expected BufferTooShort, got {other:?}"),
    }
}

/// An extension block whose declared length runs past the CIF reports both
/// the declared (in 4-byte words) and the remaining byte counts exactly.
#[test]
fn extension_block_overrun_reports_exact_numbers() {
    let bytes = [0x00, 0x05, 0x00, 0x02, 0xAA, 0xBB];
    match HandshakeExtensions::parse(&bytes) {
        Err(Error::ExtensionOverrun {
            declared,
            remaining,
        }) => {
            assert_eq!(declared, 2);
            assert_eq!(remaining, 2);
        }
        other => panic!("expected ExtensionOverrun, got {other:?}"),
    }
}

/// The Group Membership Extension serializes into exactly eight bytes.
#[test]
fn group_membership_serialize_requires_eight_bytes() {
    let group = GroupMembershipExtension {
        group_id: 42,
        group_type: GroupType::Broadcast,
        flags: GroupFlags(0x01),
        weight: 3,
    };
    let mut ok = [0u8; 8];
    assert_eq!(group.serialized_len(), 8);
    assert_eq!(group.serialize_into(&mut ok).unwrap(), 8);
    assert_eq!(&ok, &[0, 0, 0, 42, 1, 1, 0, 3]);

    let mut small = [0u8; 7];
    assert!(matches!(
        group.serialize_into(&mut small),
        Err(Error::OutputBufferTooSmall(_))
    ));
}

/// Only the `M` bit means message-number sync.
#[test]
fn group_flags_message_number_sync_reads_the_m_bit() {
    assert!(GroupFlags(0x01).message_number_sync());
    assert!(!GroupFlags(0x00).message_number_sync());
    assert!(!GroupFlags(0x02).message_number_sync());
}

/// A data packet decodes every field: the 31-bit sequence number, all four
/// `KK` values, and a message number that is not the mask.
#[test]
fn data_packet_parse_decodes_every_field() {
    for (bits, want) in [
        (0b00u32, EncryptionKeyField::NotEncrypted),
        (0b01, EncryptionKeyField::Even),
        (0b10, EncryptionKeyField::Odd),
        (0b11, EncryptionKeyField::Reserved(3)),
    ] {
        let mut bytes = [0u8; 20];
        bytes[0..4].copy_from_slice(&0x1234u32.to_be_bytes());
        let word1 = (0b10u32 << 30) | (1 << 29) | (bits << 27) | 0x0000_0007;
        bytes[4..8].copy_from_slice(&word1.to_be_bytes());
        bytes[8..12].copy_from_slice(&0x0A0B_0C0Du32.to_be_bytes());
        bytes[12..16].copy_from_slice(&0x1122_3344u32.to_be_bytes());
        bytes[16..20].copy_from_slice(b"data");

        let dp = DataPacket::parse(&bytes).unwrap();
        assert_eq!(dp.seq_number, 0x1234);
        assert_eq!(dp.message_number, 7);
        assert_eq!(dp.key_flag, want);
        assert_eq!(dp.position, PacketPosition::First);
        assert!(dp.in_order);
        assert!(!dp.retransmitted);
        assert_eq!(dp.timestamp, SrtTimestamp::from_wire(0x0A0B_0C0D));
        assert_eq!(dp.dest_socket_id, SocketId::from_wire(0x1122_3344));
        assert_eq!(dp.data, b"data");
    }
    assert_eq!(EncryptionKeyField::Reserved(1).to_bits(), 1);

    let mut out_of_order = [0u8; 16];
    let word1 = (0b10u32 << 30) | (0b11u32 << 27);
    out_of_order[4..8].copy_from_slice(&word1.to_be_bytes());
    let dp = DataPacket::parse(&out_of_order).unwrap();
    assert!(!dp.in_order, "the O bit is clear");
    assert!(!dp.retransmitted);
}

/// A non-zero reserved control field is rejected, with its spec name.
#[test]
fn non_zero_reserved_control_fields_are_rejected() {
    const KEEPALIVE: u16 = 0x0001;
    match ControlPacket::parse(&control_header(KEEPALIVE, 1, 0)) {
        Err(Error::ReservedBits(r)) => {
            assert_eq!(r.field, "Subtype");
            assert_eq!(r.expected, 0);
            assert_eq!(r.found, 1);
        }
        other => panic!("expected ReservedBits, got {other:?}"),
    }
    match ControlPacket::parse(&control_header(KEEPALIVE, 0, 1)) {
        Err(Error::ReservedBits(r)) => {
            assert_eq!(r.field, "Type-specific Information");
            assert_eq!(r.expected, 0);
            assert_eq!(r.found, 1);
        }
        other => panic!("expected ReservedBits, got {other:?}"),
    }
    assert!(ControlPacket::parse(&control_header(KEEPALIVE, 0, 0)).is_ok());
}

/// A Handshake Extension Message serializes into exactly twelve bytes.
#[test]
fn hs_ext_message_serialize_length_is_twelve() {
    let msg = HsExtMessage {
        srt_version: 0x0105_0000,
        srt_flags: HandshakeExtensionMessageFlags(0x11),
        receiver_tsbpd_delay: Duration::from_millis(120),
        sender_tsbpd_delay: Duration::from_millis(250),
    };
    assert_eq!(msg.serialized_len(), 12);
    let mut buf = [0u8; 12];
    assert_eq!(msg.serialize_into(&mut buf).unwrap(), 12);
    assert_eq!(buf, msg.to_bytes());

    let mut too_small = [0u8; 11];
    assert!(matches!(
        msg.serialize_into(&mut too_small),
        Err(Error::OutputBufferTooSmall(_))
    ));
}

/// A data packet (F=0) is refused by the control-packet parser.
#[test]
fn control_packet_parse_rejects_a_data_packet() {
    let mut bytes = control_header(0x0001, 0, 0);
    bytes[0] &= 0x7F; // clear the F bit
    assert!(matches!(
        ControlPacket::parse(&bytes),
        Err(Error::WrongPacketKind {
            expected: "control packet (F=1)"
        })
    ));
}

/// A User-Defined control packet round-trips, keeping its 16-bit subtype in
/// the first header word.
#[test]
fn user_defined_control_packet_round_trips_its_subtype() {
    let packet = ControlPacket::UserDefined(UserDefinedPacket {
        control_type: 0x7FFF,
        subtype: 3,
        type_specific_info: 5,
        timestamp: SrtTimestamp::from_wire(0x0A0B_0C0D),
        dest_socket_id: SocketId::from_wire(0x1122_3344),
        cif: &[0xDE, 0xAD, 0xBE, 0xEF],
    });
    let bytes = packet.try_to_bytes().unwrap();
    assert_eq!(&bytes[..4], &0xFFFF_0003u32.to_be_bytes());
    assert_eq!(ControlPacket::parse(&bytes).unwrap(), packet);
}

/// Every spec/field enum label is its literal token, and `Display` agrees
/// (the labelled catch-all renders as `reserved(0xNN)`).
#[test]
fn spec_labels_and_display_match_their_literal_tokens() {
    for (field, token) in [
        (EncryptionField::NoEncryption, "no encryption advertised"),
        (EncryptionField::Aes128, "AES-128"),
        (EncryptionField::Aes192, "AES-192"),
        (EncryptionField::Aes256, "AES-256"),
    ] {
        assert_eq!(field.name(), token);
        assert_eq!(format!("{field}"), token);
    }
    assert_eq!(EncryptionField::Reserved(1).name(), "reserved");
    assert_eq!(
        format!("{}", EncryptionField::Reserved(1)),
        "reserved(0x01)"
    );

    for (ty, token) in [
        (HandshakeType::Done, "DONE"),
        (HandshakeType::Agreement, "AGREEMENT"),
        (HandshakeType::Conclusion, "CONCLUSION"),
        (HandshakeType::Wavehand, "WAVEHAND"),
        (HandshakeType::Induction, "INDUCTION"),
    ] {
        assert_eq!(ty.name(), token);
        assert_eq!(format!("{ty}"), token);
    }
    assert_eq!(HandshakeType::Reserved(2).name(), "reserved");
    assert_eq!(format!("{}", HandshakeType::Reserved(2)), "reserved(0x02)");

    for (ext, token) in [
        (ExtensionType::HsReq, "SRT_CMD_HSREQ"),
        (ExtensionType::HsRsp, "SRT_CMD_HSRSP"),
        (ExtensionType::KmReq, "SRT_CMD_KMREQ"),
        (ExtensionType::KmRsp, "SRT_CMD_KMRSP"),
        (ExtensionType::Sid, "SRT_CMD_SID"),
        (ExtensionType::Congestion, "SRT_CMD_CONGESTION"),
        (ExtensionType::Filter, "SRT_CMD_FILTER"),
        (ExtensionType::Group, "SRT_CMD_GROUP"),
    ] {
        assert_eq!(ext.name(), token);
        assert_eq!(format!("{ext}"), token);
    }
    assert_eq!(ExtensionType::Reserved(9).name(), "reserved");
    assert_eq!(format!("{}", ExtensionType::Reserved(9)), "reserved(0x09)");

    for (group, token) in [
        (GroupType::Undefined, "undefined"),
        (GroupType::Broadcast, "broadcast"),
        (GroupType::MainBackup, "main/backup"),
        (GroupType::Balancing, "balancing"),
        (GroupType::Multicast, "multicast"),
    ] {
        assert_eq!(group.name(), token);
        assert_eq!(format!("{group}"), token);
    }
    assert_eq!(GroupType::Reserved(9).name(), "reserved");
    assert_eq!(format!("{}", GroupType::Reserved(9)), "reserved(0x09)");

    for (flag, token) in [
        (KmKeyFlag::NoSek, "no SEK"),
        (KmKeyFlag::Even, "even key"),
        (KmKeyFlag::Odd, "odd key"),
        (KmKeyFlag::Both, "both keys"),
    ] {
        assert_eq!(flag.name(), token);
        assert_eq!(format!("{flag}"), token);
    }

    for (cipher, token) in [
        (Cipher::None, "none / KEKI-indexed"),
        (Cipher::AesCtr, "AES-CTR"),
    ] {
        assert_eq!(cipher.name(), token);
        assert_eq!(format!("{cipher}"), token);
    }
    assert_eq!(Cipher::Reserved(1).name(), "reserved");
    assert_eq!(format!("{}", Cipher::Reserved(1)), "reserved(0x01)");

    assert_eq!(KmAuth::None.name(), "none / KEKI-indexed");
    assert_eq!(format!("{}", KmAuth::None), "none / KEKI-indexed");
    assert_eq!(KmAuth::Reserved(1).name(), "reserved");
    assert_eq!(format!("{}", KmAuth::Reserved(1)), "reserved(0x01)");

    for (se, token) in [
        (
            StreamEncapsulation::Unspecified,
            "unspecified / KEKI-indexed",
        ),
        (StreamEncapsulation::MpegTsUdp, "MPEG-TS/UDP"),
        (StreamEncapsulation::MpegTsSrt, "MPEG-TS/SRT"),
    ] {
        assert_eq!(se.name(), token);
        assert_eq!(format!("{se}"), token);
    }
    assert_eq!(StreamEncapsulation::Reserved(3).name(), "reserved");
    assert_eq!(
        format!("{}", StreamEncapsulation::Reserved(3)),
        "reserved(0x03)"
    );

    for (position, token) in [
        (PacketPosition::First, "first"),
        (PacketPosition::Middle, "middle"),
        (PacketPosition::Last, "last"),
        (PacketPosition::Solo, "solo"),
    ] {
        assert_eq!(position.name(), token);
        assert_eq!(format!("{position}"), token);
    }

    for (kk, token) in [
        (EncryptionKeyField::NotEncrypted, "not encrypted"),
        (EncryptionKeyField::Even, "even key"),
        (EncryptionKeyField::Odd, "odd key"),
    ] {
        assert_eq!(kk.name(), token);
        assert_eq!(format!("{kk}"), token);
    }
    assert_eq!(EncryptionKeyField::Reserved(3).name(), "reserved");
    assert_eq!(
        format!("{}", EncryptionKeyField::Reserved(3)),
        "reserved(0x03)"
    );
}
