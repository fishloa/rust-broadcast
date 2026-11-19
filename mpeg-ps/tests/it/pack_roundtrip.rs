//! Whole-pack serialization (ISO/IEC 13818-1 §2.5.3.1/2.5.3.5/2.5.4): a pack
//! carrying a system header, a PSM and several PES packets, and the
//! `pack_stuffing_length` boundary (Table 2-39, a 3-bit field).

use broadcast_common::Serialize;
use mpeg_pes::{PesPacket, StreamId};
use mpeg_ps::{
    DescriptorLoop, Error, EsMapEntry, PackHeader, ProgramStreamMap, Scr, SystemHeader,
    program_stream,
};

fn pack_header() -> PackHeader<'static> {
    PackHeader {
        scr: Scr {
            base: 0,
            extension: 0,
        },
        program_mux_rate: 0x10CD8E,
        stuffing_length: 0,
        stuffing: &[],
        reserved: 0x1F,
    }
}

fn system_header() -> SystemHeader {
    SystemHeader {
        rate_bound: 0x2A5A5,
        audio_bound: 0x15,
        fixed_flag: true,
        csps_flag: false,
        system_audio_lock_flag: true,
        system_video_lock_flag: true,
        video_bound: 0x1B,
        packet_rate_restriction_flag: true,
        reserved_bits: 0x7F,
        std_buffer_bounds: Vec::new(),
        trailing_reserved: Vec::new(),
    }
}

fn psm() -> ProgramStreamMap<'static> {
    ProgramStreamMap {
        current_next_indicator: false,
        single_extension_stream_flag: false,
        version: 1,
        program_stream_info: DescriptorLoop::new(&[]),
        elementary_stream_map: vec![EsMapEntry {
            stream_type: 0x02,
            elementary_stream_id: 0xE0,
            stream_id_extension: None,
            pseudo_descriptor_tag: None,
            pseudo_descriptor_length: None,
            descriptors: DescriptorLoop::new(&[]),
        }],
        crc: 0,
    }
}

fn pes(payload: &'static [u8]) -> PesPacket<'static> {
    PesPacket::new(StreamId::new(0xBE), payload)
}

fn parts() -> (
    PackHeader<'static>,
    SystemHeader,
    ProgramStreamMap<'static>,
    [PesPacket<'static>; 3],
) {
    (
        pack_header(),
        system_header(),
        psm(),
        [pes(&[0xAA; 3]), pes(&[0xBB; 5]), pes(&[0xCC; 7])],
    )
}

fn expected_wire() -> Vec<u8> {
    let (pack_header, system_header, psm, pes) = parts();
    let mut wire = pack_header.try_to_bytes().unwrap();
    wire.extend_from_slice(&system_header.try_to_bytes().unwrap());
    wire.extend_from_slice(&psm.try_to_bytes().unwrap());
    for packet in &pes {
        wire.extend_from_slice(&packet.try_to_bytes().unwrap());
    }
    wire
}

#[test]
fn a_parsed_pack_with_every_element_misizes_nothing() {
    let wire = expected_wire();
    let (parsed, consumed) = program_stream::parse_pack(&wire).unwrap();
    let pack = parsed.expect("not an end code");
    assert_eq!(consumed, wire.len());

    let (pack_header, system_header, psm, pes) = parts();
    assert_eq!(pack.pack_header, pack_header);
    assert_eq!(pack.system_header.as_ref(), Some(&system_header));
    assert_eq!(
        pack.psm.as_ref().unwrap().try_to_bytes().unwrap(),
        psm.try_to_bytes().unwrap()
    );
    assert_eq!(pack.pes_packets.len(), 3);
    for (got, want) in pack.pes_packets.iter().zip(&pes) {
        assert_eq!(got.try_to_bytes().unwrap(), want.try_to_bytes().unwrap());
    }

    let sum = pack_header.serialized_len()
        + system_header.serialized_len()
        + psm.serialized_len()
        + pes.iter().map(Serialize::serialized_len).sum::<usize>();
    assert_eq!(pack.serialized_len(), sum);
    assert_eq!(pack.serialized_len(), wire.len());

    let mut out = vec![0u8; pack.serialized_len()];
    let written = pack.serialize_into(&mut out).unwrap();
    assert_eq!(written, out.len());
    assert_eq!(out, wire);
}

#[test]
fn maximum_stuffing_length_is_accepted_and_one_more_is_rejected() {
    let mut header = pack_header();
    header.stuffing_length = 7;
    header.stuffing = &[0xFF; 7];
    let mut out = vec![0u8; header.serialized_len()];
    assert_eq!(header.serialize_into(&mut out).unwrap(), 21);
    assert_eq!(out[13], 0xFF);
    assert_eq!(&out[14..21], &[0xFF; 7]);

    let mut too_long = pack_header();
    too_long.stuffing_length = 8;
    too_long.stuffing = &[0xFF; 8];
    let mut out = vec![0u8; too_long.serialized_len()];
    assert_eq!(
        too_long.serialize_into(&mut out).unwrap_err(),
        Error::StuffingLengthTooLarge(8)
    );
}
