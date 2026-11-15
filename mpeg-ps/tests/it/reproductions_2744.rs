//! #2744: values the parser accepts that the serializer cannot write back
//! (zero `program_mux_rate`, a PSM over `MAP_LENGTH_MAX`), wire fields the
//! parser discards and the serializer invents (`pseudo_descriptor_tag` /
//! `pseudo_descriptor_length`), and no `Serialize` for `Pack`.

use broadcast_common::{Parse, Serialize};
use mpeg_ps::DescriptorLoop;
use mpeg_ps::{Error, PackHeader, ProgramStreamMap, Scr};

#[test]
fn a_zero_program_mux_rate_is_not_writable() {
    let bytes = vec![
        0x00, 0x00, 0x01, 0xBA, 0x44, 0x00, 0x04, 0x00, 0x04, 0x01, 0x00, 0x00, 0x03, 0xFC,
    ];
    assert_eq!(PackHeader::parse(&bytes).unwrap_err(), Error::ZeroMuxRate);
    let header = PackHeader {
        scr: Scr {
            base: 0,
            extension: 0,
        },
        program_mux_rate: 0,
        stuffing_length: 0,
        stuffing: &[],
        reserved: 0x1F,
    };
    let mut out = vec![0u8; header.serialized_len()];
    assert_eq!(
        header.serialize_into(&mut out).unwrap_err(),
        Error::ZeroMuxRate,
        "a header the serializer writes must parse, so a zero mux rate is refused"
    );
}

#[test]
fn the_psm_spec_length_max_is_enforced_on_both_sides() {
    let psm = ProgramStreamMap {
        current_next_indicator: true,
        single_extension_stream_flag: false,
        version: 0,
        program_stream_info: DescriptorLoop::new(&[0u8; 1009]),
        elementary_stream_map: vec![],
        crc: 0,
    };
    let mut buf = vec![0u8; psm.serialized_len()];
    assert!(matches!(
        psm.serialize_into(&mut buf),
        Err(Error::FieldOverflow(_))
    ));
    buf[4..6].copy_from_slice(&0x3FFu16.to_be_bytes());
    assert!(matches!(
        ProgramStreamMap::parse(&buf),
        Err(Error::MapLengthOverflow { .. })
    ));
}

#[test]
fn a_pack_reserializes_byte_exactly() {
    let data = crate::psm_vectors::two_psms();
    let scan = mpeg_ps::program_stream::scan_packs(&data);
    assert_eq!(scan.packs.len(), 2);
    let wire: Vec<u8> = scan
        .packs
        .iter()
        .flat_map(|pack| pack.try_to_bytes().expect("a parsed pack serializes"))
        .collect();
    assert_eq!(wire, data[..data.len() - 4]);
    assert_eq!(&data[data.len() - 4..], &[0x00, 0x00, 0x01, 0xB9]);
}

#[test]
fn the_pseudo_descriptor_tag_and_length_are_written_as_the_spec_fixes_them() {
    let wire = crate::psm_vectors::psm_with_extension();
    // prefix(6) flags(1) reserved(1) prog_info_len(2) es_map_len(2)
    // stream_type(1) elementary_stream_id(1) ES_info_length(2)
    assert_eq!(&wire[16..18], &[0x01, 0x01]);
    assert_eq!(wire[18], 0x8F);
    let parsed = ProgramStreamMap::parse(&wire).expect("the vector PSM parses");
    let mut out = vec![0u8; parsed.serialized_len()];
    parsed.serialize_into(&mut out).unwrap();
    assert_eq!(out, wire);
}
