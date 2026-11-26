//! Serialized-length contracts, ceilings and label tables for the EN 50221
//! application objects (audit 23): every `serialize_into` return, the
//! 12-bit length fields, the 255-byte ceilings and every `name()` label.

use broadcast_core::error::{BufferTooShort, LengthMismatch};
use broadcast_core::{Parse, Serialize};
use dvb_ci::Error;
use dvb_ci::length::{LengthForm, decode_form, encode_into, encode_with};
use dvb_ci::objects::application_info::{ApplicationInfo, ApplicationType};
use dvb_ci::objects::ca_pmt::{CaPmt, CaPmtCmdId, CaPmtListManagement, CaPmtStream};
use dvb_ci::objects::ca_pmt_reply::{CaEnable, CaPmtReply, CaPmtReplyStream};
use dvb_ci::objects::date_time::DateTime;
use dvb_ci::objects::host_control::Replace;
use dvb_ci::objects::low_speed_comms::{
    CommsCmd, CommsCmdParams, CommsCommandId, CommsReply, CommsReplyId, ConnectionDescriptor,
    ConnectionDescriptorType,
};
use dvb_ci::objects::mmi_display::{
    DisplayControlCmd, DisplayReply, DisplayReplyBody, DisplayReplyId, DownloadReply,
    DownloadReplyId, GraphicsCharacteristics, KeypadControlCmd, MmiMode, PixelDepth,
};
use dvb_ci::objects::mmi_high::Enq;
use dvb_ci::reserved::ReservedOnes;
use dvb_ci::resource::{
    APPLICATION_INFORMATION, CONDITIONAL_ACCESS_SUPPORT, DATE_TIME, HOST_CONTROL, MMI,
    RESOURCE_MANAGER, ResourceId,
};
use mpeg_ts::pid::Pid;

fn serialize_len<T: Serialize<Error = Error>>(value: &T) -> (usize, Vec<u8>) {
    let mut buf = vec![0u8; value.serialized_len()];
    let n = value.serialize_into(&mut buf).expect("serialize");
    (n, buf)
}

#[test]
fn length_form_name_labels() {
    assert_eq!(LengthForm::Minimal.name(), "minimal");
    assert_eq!(LengthForm::Long { octets: 2 }.name(), "long");
}

#[test]
fn encode_with_layout_boundaries() {
    let cases: &[(usize, LengthForm, &[u8])] = &[
        (0x7F, LengthForm::Long { octets: 1 }, &[0x81, 0x7F]),
        (0x80, LengthForm::Long { octets: 2 }, &[0x82, 0x00, 0x80]),
        (0xFF, LengthForm::Long { octets: 2 }, &[0x82, 0x00, 0xFF]),
        (
            0x100,
            LengthForm::Long { octets: 3 },
            &[0x83, 0x00, 0x01, 0x00],
        ),
    ];
    for &(value, form, wire) in cases {
        let mut buf = [0u8; 8];
        let n = encode_with(value, form, &mut buf).expect("encode_with");
        assert_eq!(&buf[..n], wire, "value {value:#X}");
        assert_eq!(n, wire.len());
        let (decoded, hdr, decoded_form) = decode_form(&buf[..n]).expect("decode_form");
        assert_eq!(decoded, value);
        assert_eq!(hdr, n);
        assert_eq!(decoded_form, form);
    }
    let mut buf = [0u8; 8];
    assert_eq!(encode_into(0x80, &mut buf).unwrap(), 2);
    assert_eq!(&buf[..2], &[0x81, 0x80]);
    assert_eq!(encode_into(0xFF, &mut buf).unwrap(), 2);
    assert_eq!(&buf[..2], &[0x81, 0xFF]);
    assert_eq!(encode_into(0x100, &mut buf).unwrap(), 3);
    assert_eq!(&buf[..3], &[0x82, 0x01, 0x00]);
}

#[test]
fn resource_id_accessors_and_labels() {
    let id = ResourceId(0x0041_08C5);
    assert_eq!(id.resource_class(), 0x41);
    assert_eq!(id.resource_type(), 0x23);
    assert_eq!(id.resource_version(), 5);
    assert_eq!(id.name(), "unknown");
    assert_eq!(format!("{id}"), "resource_id(0x004108C5)");
    let table = [
        (RESOURCE_MANAGER, "resource_manager"),
        (APPLICATION_INFORMATION, "application_information"),
        (CONDITIONAL_ACCESS_SUPPORT, "conditional_access_support"),
        (HOST_CONTROL, "host_control"),
        (DATE_TIME, "date_time"),
        (MMI, "mmi"),
    ];
    for (id, label) in table {
        assert_eq!(id.name(), label);
        assert_eq!(format!("{id}"), format!("{label}(0x{:08X})", id.0));
    }
}

#[test]
fn application_info_menu_string_ceiling() {
    let menu = vec![b'x'; 255];
    let info = ApplicationInfo {
        length_form: LengthForm::Minimal,
        application_type: ApplicationType::from_u8(0x01),
        application_manufacturer: 0x0001,
        manufacturer_code: 0x0002,
        menu_string: &menu,
    };
    let (n, buf) = serialize_len(&info);
    assert_eq!(n, info.serialized_len());
    assert_eq!(n, buf.len());
    assert_eq!(n, 267);
    assert_eq!(&buf[..6], &[0x9F, 0x80, 0x21, 0x82, 0x01, 0x05]);
    assert_eq!(buf[11], 255);
    assert_eq!(ApplicationInfo::parse(&buf).unwrap(), info);

    let too_long = vec![b'x'; 256];
    let mut over = info.clone();
    over.menu_string = &too_long;
    let mut out = vec![0u8; over.serialized_len()];
    assert!(matches!(
        over.serialize_into(&mut out),
        Err(Error::InvalidObject {
            what: "application_info",
            ..
        })
    ));
}

#[test]
fn application_info_truncated_menu_reports_declared_and_actual() {
    let wire = [
        0x9F, 0x80, 0x21, 0x0D, 0x01, 0x00, 0x01, 0x00, 0x02, 0x0A, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE,
        0xFF, 0x00,
    ];
    match ApplicationInfo::parse(&wire).unwrap_err() {
        Error::LengthMismatch(LengthMismatch {
            declared, actual, ..
        }) => assert_eq!((declared, actual), (10, 7)),
        other => panic!("expected LengthMismatch, got {other:?}"),
    }
}

fn reserved_ones<const BITS: u32>(bits: u64) -> ReservedOnes<BITS> {
    ReservedOnes::from_bits(bits)
}

fn ca_stream<'a>(pid: u16, cmd_id: Option<CaPmtCmdId>, descriptors: &'a [u8]) -> CaPmtStream<'a> {
    CaPmtStream {
        stream_type: 0x81,
        elementary_pid: Pid::from_const(pid),
        cmd_id,
        ca_descriptors: descriptors,
        reserved_pid: reserved_ones(0b111),
        reserved_es_info: reserved_ones(0b1111),
    }
}

#[test]
fn ca_pmt_long_program_and_es_info_lengths() {
    let program = vec![0x09u8; 300];
    let es = vec![0x09u8; 270];
    let pmt = CaPmt {
        length_form: LengthForm::Minimal,
        list_management: CaPmtListManagement::Only,
        program_number: 0x0001,
        version_number: 3,
        current_next_indicator: true,
        cmd_id: Some(CaPmtCmdId::OkDescrambling),
        program_ca_descriptors: &program,
        streams: vec![ca_stream(0x0200, Some(CaPmtCmdId::Query), &es)],
        reserved_version: reserved_ones(0b11),
        reserved_program_info: reserved_ones(0b1111),
    };
    let (n, buf) = serialize_len(&pmt);
    assert_eq!(n, pmt.serialized_len());
    assert_eq!(n, buf.len());
    assert_eq!(n, 3 + 3 + 6 + 301 + 5 + 271);
    assert_eq!(buf[10], 0xF0 | 0x01);
    assert_eq!(buf[11], 0x2D);
    assert_eq!(buf[316], 0xF0 | 0x01);
    assert_eq!(buf[317], 0x0F);
    assert_eq!(CaPmt::parse(&buf).unwrap(), pmt);
}

#[test]
fn ca_pmt_program_info_length_ceiling() {
    for (descriptor_len, ok) in [(4094usize, true), (4095, false)] {
        let program = vec![0x09u8; descriptor_len];
        let pmt = CaPmt {
            length_form: LengthForm::Minimal,
            list_management: CaPmtListManagement::First,
            program_number: 7,
            version_number: 0,
            current_next_indicator: false,
            cmd_id: Some(CaPmtCmdId::OkMmi),
            program_ca_descriptors: &program,
            streams: Vec::new(),
            reserved_version: reserved_ones(0b11),
            reserved_program_info: reserved_ones(0b1111),
        };
        let mut out = vec![0u8; pmt.serialized_len()];
        let result = pmt.serialize_into(&mut out);
        assert_eq!(result.is_ok(), ok, "descriptor len {descriptor_len}");
        if ok {
            let n = result.unwrap();
            assert_eq!(n, 3 + 3 + 6 + 4095);
            assert_eq!(out[10], 0xF0 | 0x0F);
            assert_eq!(out[11], 0xFF);
        }
    }
}

#[test]
fn ca_pmt_cmd_id_labels() {
    let table = [
        (CaPmtCmdId::OkDescrambling, "ok_descrambling"),
        (CaPmtCmdId::OkMmi, "ok_mmi"),
        (CaPmtCmdId::Query, "query"),
        (CaPmtCmdId::NotSelected, "not_selected"),
        (CaPmtCmdId::Rfu(0x7F), "reserved"),
    ];
    for (id, label) in table {
        assert_eq!(id.name(), label);
    }
}

#[test]
fn ca_pmt_reply_truncated_es_reports_the_exact_need() {
    let reply = CaPmtReply {
        length_form: LengthForm::Minimal,
        program_number: 1,
        version_number: 1,
        current_next_indicator: true,
        ca_enable: Some(CaEnable::Possible),
        streams: vec![
            CaPmtReplyStream {
                elementary_pid: Pid::from_const(0x0200),
                ca_enable: Some(CaEnable::Possible),
                reserved_pid: reserved_ones(0b111),
                reserved_ca_enable: ReservedOnes::default(),
            },
            CaPmtReplyStream {
                elementary_pid: Pid::from_const(0x0201),
                ca_enable: Some(CaEnable::NotPossibleNoEntitlement),
                reserved_pid: reserved_ones(0b111),
                reserved_ca_enable: ReservedOnes::default(),
            },
        ],
        reserved_version: reserved_ones(0b11),
        reserved_ca_enable: ReservedOnes::default(),
    };
    let (n, buf) = serialize_len(&reply);
    assert_eq!(n, 14);
    let mut truncated = buf[..13].to_vec();
    truncated[3] = 0x09;
    let err = CaPmtReply::parse(&truncated).unwrap_err();
    match err {
        Error::BufferTooShort(BufferTooShort { need, have, .. }) => {
            assert_eq!((need, have), (10, 9));
        }
        other => panic!("expected BufferTooShort, got {other:?}"),
    }
}

#[test]
fn date_time_serialize_into_returns_seven_bytes() {
    let dt = DateTime {
        length_form: LengthForm::Minimal,
        utc_time: [0xE0, 0x00, 0x12, 0x34, 0x56],
        local_offset: Some(-60),
    };
    let (n, buf) = serialize_len(&dt);
    assert_eq!(n, 11);
    assert_eq!(n, dt.serialized_len());
    assert_eq!(n, buf.len());
    assert_eq!(
        buf,
        [
            0x9F, 0x84, 0x41, 0x07, 0xE0, 0x00, 0x12, 0x34, 0x56, 0xFF, 0xC4
        ]
    );
    assert_eq!(DateTime::parse(&buf).unwrap(), dt);
}

#[test]
fn replace_serialize_into_returns_the_body_end() {
    let r = Replace {
        length_form: LengthForm::Minimal,
        replacement_ref: 0x07,
        replaced_pid: Pid::from_const(0x0102),
        replacement_pid: Pid::from_const(0x0203),
        replaced_reserved: reserved_ones(0b111),
        replacement_reserved: reserved_ones(0b111),
    };
    let (n, buf) = serialize_len(&r);
    assert_eq!(n, 9);
    assert_eq!(n, r.serialized_len());
    assert_eq!(buf, [0x9F, 0x84, 0x01, 0x05, 0x07, 0xE1, 0x02, 0xE2, 0x03]);
    assert_eq!(Replace::parse(&buf).unwrap(), r);
}

#[test]
fn enq_serialized_len_covers_prefix_plus_text() {
    let enq = Enq {
        length_form: LengthForm::Minimal,
        blind_answer: true,
        answer_text_length: 4,
        text_chars: b"PIN",
        reserved: ReservedOnes::from_bits(0x7F),
    };
    assert_eq!(enq.serialized_len(), 9);
    let (n, buf) = serialize_len(&enq);
    assert_eq!(n, 9);
    assert_eq!(buf, [0x9F, 0x88, 0x07, 0x05, 0xFF, 0x04, b'P', b'I', b'N']);
    assert_eq!(Enq::parse(&buf).unwrap(), enq);
}

#[test]
fn comms_cmd_serialize_into_returns_the_parameter_end() {
    let descriptor = ConnectionDescriptor {
        length_form: LengthForm::Minimal,
        descriptor_type: ConnectionDescriptorType::CableReturnChannelDescriptor,
        payload: &[0x2A],
    };
    let connect = CommsCmd {
        length_form: LengthForm::Minimal,
        command_id: CommsCommandId::ConnectOnChannel,
        params: CommsCmdParams::Connect {
            connection_descriptor: descriptor,
            retry_count: 3,
            timeout: 9,
        },
    };
    let (n, buf) = serialize_len(&connect);
    assert_eq!(n, 13);
    assert_eq!(n, connect.serialized_len());
    assert_eq!(
        buf,
        [
            0x9F, 0x8C, 0x00, 0x09, 0x01, 0x9F, 0x8C, 0x01, 0x02, 0x02, 0x2A, 0x03, 0x09
        ]
    );
    assert_eq!(CommsCmd::parse(&buf).unwrap(), connect);

    let set_params = CommsCmd {
        length_form: LengthForm::Minimal,
        command_id: CommsCommandId::SetParams,
        params: CommsCmdParams::SetParams {
            buffer_size: 0x40,
            timeout: 0x0A,
        },
    };
    let (n, buf) = serialize_len(&set_params);
    assert_eq!(n, 7);
    assert_eq!(n, set_params.serialized_len());
    assert_eq!(buf, [0x9F, 0x8C, 0x00, 0x03, 0x03, 0x40, 0x0A]);
    assert_eq!(CommsCmd::parse(&buf).unwrap(), set_params);
}

#[test]
fn comms_reply_serialize_into_returns_the_body_end() {
    let reply = CommsReply {
        length_form: LengthForm::Minimal,
        reply_id: CommsReplyId::StatusReply,
        return_value: 1,
    };
    let (n, buf) = serialize_len(&reply);
    assert_eq!(n, 6);
    assert_eq!(n, reply.serialized_len());
    assert_eq!(buf, [0x9F, 0x8C, 0x02, 0x02, 0x04, 0x01]);
    assert_eq!(CommsReply::parse(&buf).unwrap(), reply);
}

#[test]
fn display_reply_serialize_into_returns_the_graphics_end() {
    let reply = DisplayReply {
        length_form: LengthForm::Minimal,
        reply_id: DisplayReplyId::ListGraphicOverlayCharacteristics,
        body: DisplayReplyBody::Graphics(GraphicsCharacteristics {
            display_horizontal_size: 0x0280,
            display_vertical_size: 0x01E0,
            aspect_ratio_information: 1,
            graphics_relation_to_video: 2,
            multiple_depths: true,
            display_bytes: 0x123,
            composition_buffer_bytes: 0x45,
            object_cache_bytes: 0x67,
            depths: vec![PixelDepth {
                display_depth: 5,
                pixels_per_byte: 6,
                region_overhead: 0x99,
                reserved: reserved_ones(0b11),
            }],
        }),
    };
    let (n, buf) = serialize_len(&reply);
    assert_eq!(n, 16);
    assert_eq!(n, reply.serialized_len());
    assert_eq!(n, buf.len());
    assert_eq!(
        buf,
        [
            0x9F, 0x88, 0x02, 0x0C, 0x04, 0x02, 0x80, 0x01, 0xE0, 0x15, 0x12, 0x34, 0x56, 0x71,
            0xBB, 0x99
        ]
    );
    assert_eq!(DisplayReply::parse(&buf).unwrap(), reply);
}

#[test]
fn download_reply_serialize_into_returns_the_body_end() {
    let reply = DownloadReply {
        length_form: LengthForm::Minimal,
        object_id: 0x1234,
        reply_id: DownloadReplyId::DownloadOk,
    };
    let (n, buf) = serialize_len(&reply);
    assert_eq!(n, 7);
    assert_eq!(n, reply.serialized_len());
    assert_eq!(buf, [0x9F, 0x88, 0x17, 0x03, 0x12, 0x34, 0x00]);
    assert_eq!(DownloadReply::parse(&buf).unwrap(), reply);
}

#[test]
fn mmi_and_comms_enum_labels() {
    let connection = [
        (
            ConnectionDescriptorType::SiTelephoneDescriptor,
            "SI_Telephone_Descriptor",
        ),
        (
            ConnectionDescriptorType::CableReturnChannelDescriptor,
            "Cable_Return_Channel_Descriptor",
        ),
        (ConnectionDescriptorType::Reserved(0x7F), "reserved"),
    ];
    for (value, label) in connection {
        assert_eq!(value.name(), label);
    }
    let display_control = [
        (DisplayControlCmd::SetMmiMode, "set_mmi_mode"),
        (
            DisplayControlCmd::GetDisplayCharacterTableList,
            "get_display_character_table_list",
        ),
        (
            DisplayControlCmd::GetInputCharacterTableList,
            "get_input_character_table_list",
        ),
        (
            DisplayControlCmd::GetOverlayGraphicsCharacteristics,
            "get_overlay_graphics_characteristics",
        ),
        (
            DisplayControlCmd::GetFullScreenGraphicsCharacteristics,
            "get_full-screen_graphics_characteristics",
        ),
        (DisplayControlCmd::Reserved(0x7F), "reserved"),
    ];
    for (value, label) in display_control {
        assert_eq!(value.name(), label);
    }
    let modes = [
        (MmiMode::HighLevel, "high_level"),
        (
            MmiMode::LowLevelOverlayGraphics,
            "low_level_overlay_graphics",
        ),
        (
            MmiMode::LowLevelFullScreenGraphics,
            "low_level_full_screen_graphics",
        ),
        (MmiMode::Reserved(0x7F), "reserved"),
    ];
    for (value, label) in modes {
        assert_eq!(value.name(), label);
    }
    let reply_ids = [
        (DisplayReplyId::MmiModeAck, "mmi_mode_ack"),
        (
            DisplayReplyId::ListDisplayCharacterTables,
            "list_display_character_tables",
        ),
        (
            DisplayReplyId::ListInputCharacterTables,
            "list_input_character_tables",
        ),
        (
            DisplayReplyId::ListGraphicOverlayCharacteristics,
            "list_graphic_overlay_characteristics",
        ),
        (
            DisplayReplyId::ListFullScreenGraphicCharacteristics,
            "list_full_screen_graphic_characteristics",
        ),
        (
            DisplayReplyId::UnknownDisplayControlCmd,
            "unknown display_control_cmd",
        ),
        (DisplayReplyId::UnknownMmiMode, "unknown_mmi_mode"),
        (
            DisplayReplyId::UnknownCharacterTable,
            "unknown_character_table",
        ),
        (DisplayReplyId::Reserved(0x7F), "reserved"),
    ];
    for (value, label) in reply_ids {
        assert_eq!(value.name(), label);
    }
    let keypad = [
        (
            KeypadControlCmd::InterceptAllKeypresses,
            "intercept_all_keypresses",
        ),
        (
            KeypadControlCmd::IgnoreAllKeypresses,
            "ignore_all_keypresses",
        ),
        (
            KeypadControlCmd::InterceptSelectedKeypress,
            "intercept_selected_keypress",
        ),
        (
            KeypadControlCmd::IgnoreSelectedKeypress,
            "ignore_selected_keypress",
        ),
        (KeypadControlCmd::RejectKeypress, "reject_keypress"),
        (KeypadControlCmd::Reserved(0x7F), "reserved"),
    ];
    for (value, label) in keypad {
        assert_eq!(value.name(), label);
    }
    let downloads = [
        (DownloadReplyId::DownloadOk, "Download OK"),
        (
            DownloadReplyId::NotAnObjectDataSegment,
            "Not an object data segment",
        ),
        (DownloadReplyId::MemoryExhausted, "Memory exhausted"),
        (DownloadReplyId::Reserved(0x7F), "reserved"),
    ];
    for (value, label) in downloads {
        assert_eq!(value.name(), label);
    }
}
