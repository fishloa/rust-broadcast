//! The PES-extension / ESCR / DSM trick-mode field matrix: every flag subset
//! round-trips byte-exactly, every truncation and marker-bit fault is
//! rejected, and the trick-mode byte encoding is pinned per Table 2-24.

use mpeg_pes::{
    Error, Escr, PStdBuffer, PesExtension, PesExtensionField, PesHeader, PesPacket,
    ProgramPacketSequenceCounter, ScramblingControl, StreamId, TrickMode,
};

const PACK_HEADER: [u8; 3] = [0x91, 0x23, 0x45];
const EXT_REST: [u8; 2] = [0x66, 0x77];
const PAYLOAD: [u8; 2] = [0xDE, 0xAD];

fn extension_for(combo: u32) -> PesExtension<'static> {
    let mut ext = PesExtension::new();
    ext.reserved_bits = 0b010;
    if combo & 0x01 != 0 {
        ext.pes_private_data = Some([
            0x10, 0x21, 0x32, 0x43, 0x54, 0x65, 0x76, 0x87, 0x98, 0xA9, 0xBA, 0xCB, 0xDC, 0xED,
            0xFE, 0x0F,
        ]);
    }
    if combo & 0x02 != 0 {
        ext.pack_header = Some(&PACK_HEADER);
    }
    if combo & 0x04 != 0 {
        ext.program_packet_sequence_counter = Some(ProgramPacketSequenceCounter {
            counter: 0x55,
            mpeg1_mpeg2_identifier: combo & 0x08 != 0,
            original_stuff_length: 0x2A,
        });
    }
    if combo & 0x10 != 0 {
        ext.p_std_buffer = Some(PStdBuffer {
            scale: combo & 0x20 != 0,
            size: 0x1ABC,
        });
    }
    if combo & 0x40 != 0 {
        ext.pes_extension_field = Some(PesExtensionField {
            stream_id_extension_flag: combo & 0x80 != 0,
            low_bits: 0x2B,
            rest: &EXT_REST,
        });
    }
    ext
}

fn packet_with_extension(ext: PesExtension<'static>) -> PesPacket<'static> {
    let mut header = PesHeader::new();
    header.pes_extension = Some(ext);
    let mut packet = PesPacket::new(StreamId::new(0xBD), &PAYLOAD);
    packet.header = Some(header);
    packet
}

#[test]
fn every_extension_flag_subset_round_trips_byte_exactly() {
    for combo in 0u32..256 {
        let ext = extension_for(combo);
        let packet = packet_with_extension(ext);
        let mut buf = vec![0u8; packet.serialized_len()];
        let written = packet.serialize_into(&mut buf).expect("serialize");
        assert_eq!(written, buf.len(), "combo {combo:#010b}");

        let parsed = PesPacket::parse(&buf).expect("parse");
        assert_eq!(parsed, packet, "combo {combo:#010b}");

        let mut again = vec![0u8; parsed.serialized_len()];
        parsed.serialize_into(&mut again).expect("re-serialize");
        assert_eq!(again, buf, "combo {combo:#010b}");
    }
}

#[test]
fn a_missing_pack_field_length_reports_the_byte_it_needs() {
    let buf = [0x00, 0x00, 0x01, 0xBD, 0x00, 0x04, 0x80, 0x01, 0x01, 0x40];
    let err = PesPacket::parse(&buf).expect_err("no pack_field_length byte");
    match err {
        Error::BufferTooShort(e) => {
            assert_eq!(e.need, 2);
            assert_eq!(e.have, 1);
        }
        other => panic!("expected BufferTooShort, got {other:?}"),
    }
}

#[test]
fn a_missing_extension_field_length_reports_the_byte_it_needs() {
    let buf = [0x00, 0x00, 0x01, 0xBD, 0x00, 0x04, 0x80, 0x01, 0x01, 0x01];
    let err = PesPacket::parse(&buf).expect_err("no PES_extension_field_length byte");
    match err {
        Error::BufferTooShort(e) => {
            assert_eq!(e.need, 2);
            assert_eq!(e.have, 1);
        }
        other => panic!("expected BufferTooShort, got {other:?}"),
    }
}

#[test]
fn serialize_optional_fields_counts_the_extension_bytes() {
    let mut header = PesHeader::new();
    header.pes_extension = Some(extension_for(0xFF));
    let needed = header.optional_len();
    let mut buf = vec![0u8; needed];
    let written = header.serialize_optional_fields(&mut buf).expect("fits");
    assert_eq!(written, needed);
}

#[test]
fn a_full_extension_declares_its_exact_serialized_length() {
    let ext = extension_for(0xFF);
    assert_eq!(
        ext.serialized_len(),
        1 + 16 + 1 + PACK_HEADER.len() + 2 + 2 + 1 + 1 + 2
    );
}

#[test]
fn a_cleared_program_packet_sequence_counter_marker_is_rejected_in_each_byte() {
    let ext = extension_for(0x04);
    let packet = packet_with_extension(ext);
    let mut buf = vec![0u8; packet.serialized_len()];
    packet.serialize_into(&mut buf).expect("serialize");
    assert_eq!(buf[9], 0x24);
    assert_eq!(buf[10], 0xD5);
    assert_eq!(buf[11], 0xAA);

    let mut first = buf.clone();
    first[10] &= 0x7F;
    assert!(matches!(
        PesPacket::parse(&first),
        Err(Error::BadMarker("program_packet_sequence_counter"))
    ));

    let mut second = buf.clone();
    second[11] &= 0x7F;
    assert!(matches!(
        PesPacket::parse(&second),
        Err(Error::BadMarker("program_packet_sequence_counter"))
    ));
}

#[test]
fn every_trick_mode_variant_has_an_exact_wire_byte() {
    let cases = [
        (
            TrickMode::FastForward {
                field_id: 2,
                intra_slice_refresh: true,
                frequency_truncation: 3,
            },
            0x17,
        ),
        (TrickMode::SlowMotion { rep_cntrl: 0x15 }, 0x35),
        (
            TrickMode::FreezeFrame {
                field_id: 1,
                reserved: 0b101,
            },
            0x4D,
        ),
        (
            TrickMode::FastReverse {
                field_id: 3,
                intra_slice_refresh: true,
                frequency_truncation: 1,
            },
            0x7D,
        ),
        (TrickMode::SlowReverse { rep_cntrl: 0x1E }, 0x9E),
        (
            TrickMode::Reserved {
                trick_mode_control: 0b101,
                data: 0x1F,
            },
            0xBF,
        ),
    ];
    for (mode, byte) in cases {
        assert_eq!(mode.to_byte().expect("in range"), byte, "{mode:?}");
        assert_eq!(TrickMode::from_byte(byte), mode);
    }
}

#[test]
fn escr_reports_the_27mhz_clock_and_rejects_each_cleared_marker() {
    let escr = Escr {
        base: 0x1_2345_6789,
        extension: 0x123,
        reserved: 0b11,
    };
    let field = escr.to_field_bytes().expect("in range");
    let parsed = Escr::from_field_bytes(&field).expect("markers set");
    assert_eq!(parsed, escr);
    assert_eq!(
        parsed.to_clock27().expect("on the clock").ticks(),
        0x1_2345_6789 * 300 + 0x123
    );

    for index in [0usize, 2, 4] {
        let mut bad = field;
        bad[index] &= !0x04;
        assert_eq!(
            Escr::from_field_bytes(&bad),
            Err(Error::BadMarker("ESCR")),
            "marker in byte {index}"
        );
    }
    let mut bad = field;
    bad[5] &= !0x01;
    assert_eq!(Escr::from_field_bytes(&bad), Err(Error::BadMarker("ESCR")));
}

#[test]
fn scrambling_control_values_encode_into_the_header_flag_byte() {
    for value in 0u8..=3 {
        let mut header = PesHeader::new();
        header.scrambling_control = ScramblingControl::from_bits(value);
        let mut packet = PesPacket::new(StreamId::new(0xBD), &PAYLOAD);
        packet.header = Some(header);

        let mut buf = vec![0u8; packet.serialized_len()];
        packet.serialize_into(&mut buf).expect("serialize");
        assert_eq!((buf[6] >> 4) & 0x03, value);

        let parsed = PesPacket::parse(&buf).expect("parse");
        assert_eq!(parsed, packet);
    }
    assert_eq!(ScramblingControl::Reserved(0b100).to_bits(), 0);
    assert_eq!(ScramblingControl::Reserved(0b111).to_bits(), 3);
    assert_eq!(ScramblingControl::NotScrambled.to_bits(), 0);
}
