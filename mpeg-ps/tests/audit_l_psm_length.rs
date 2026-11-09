use broadcast_common::{Parse, Serialize, crc32_mpeg2};
use mpeg_ps::ProgramStreamMap;

fn spec_psm() -> Vec<u8> {
    let mut b = vec![
        0x00, 0x00, 0x01, 0xBC, 0x00, 0x0E, 0xE1, 0xFF, 0x00, 0x00, 0x00, 0x04, 0x1B, 0xE0, 0x00,
        0x00,
    ];
    let crc = crc32_mpeg2::compute(&b);
    b.extend_from_slice(&crc.to_be_bytes());
    b
}

#[test]
fn psm_length_counts_the_crc_on_parse() {
    let b = spec_psm();
    assert_eq!(usize::from(u16::from_be_bytes([b[4], b[5]])), b.len() - 6);
    let psm = ProgramStreamMap::parse(&b).expect("spec-conformant PSM must parse");
    assert_eq!(psm.elementary_stream_map.len(), 1);
    assert_eq!(psm.elementary_stream_map[0].stream_type, 0x1B);
}

#[test]
fn psm_length_counts_the_crc_on_serialize() {
    let b = spec_psm();
    let psm = ProgramStreamMap {
        current_next_indicator: true,
        single_extension_stream_flag: true,
        version: 1,
        program_stream_info: &[],
        elementary_stream_map: vec![mpeg_ps::EsMapEntry {
            stream_type: 0x1B,
            elementary_stream_id: 0xE0,
            stream_id_extension: None,
            descriptors: &[],
        }],
        crc: 0,
    };
    let mut o = vec![0u8; psm.serialized_len()];
    psm.serialize_into(&mut o).unwrap();
    assert_eq!(
        u16::from_be_bytes([o[4], o[5]]) as usize,
        o.len() - 6,
        "program_stream_map_length must cover everything after the field incl. CRC_32; spec bytes {b:02x?} ours {o:02x?}"
    );
}

#[test]
fn psm_in_a_pack_does_not_swallow_following_pes() {
    let mut pack = vec![0x00, 0x00, 0x01, 0xBA, 0x44, 0x00, 0x04, 0x00, 0x04, 0x01, 0x43, 0x36, 0x3B, 0xF8];
    pack.extend_from_slice(&spec_psm());
    let pes = [0x00, 0x00, 0x01, 0xE0, 0x00, 0x08, 0x80, 0x00, 0x00, 1, 2, 3, 4, 5];
    pack.extend_from_slice(&pes);
    let (p, consumed) = mpeg_ps::program_stream::parse_pack(&pack).expect("pack with real PSM");
    let p = p.unwrap();
    assert!(p.psm.is_some());
    assert_eq!(p.pes_packets.len(), 1);
    assert_eq!(consumed, pack.len());
}
