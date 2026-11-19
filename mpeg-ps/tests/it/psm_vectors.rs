//! Hand-built Program Stream vectors (Table 2-38/2-41 shapes the committed
//! ffmpeg capture does not carry: it has no Program Stream Map at all).
//!
//! Every pack header is the committed fixture's own
//! (`fixtures/ps/ffmpeg-mpeg2-ps.mpg` bytes 0..14: SCR 0, `program_mux_rate`
//! 0x10CD8E, no stuffing), every PSM is built by `ProgramStreamMap`'s own
//! serializer (so its CRC_32 is the real MPEG-2 CRC-32), and the PES packet is
//! a minimal PTS-only packet.

use broadcast_common::{Parse, Serialize};
use mpeg_ps::DescriptorLoop;
use mpeg_ps::{Error, EsMapEntry, ProgramStreamMap};

const PACK_HEADER: &[u8] = &[
    0x00, 0x00, 0x01, 0xBA, 0x44, 0x00, 0x04, 0x00, 0x04, 0x01, 0x43, 0x36, 0x3B, 0xF8,
];
const PES: &[u8] = &[
    0x00, 0x00, 0x01, 0xE0, 0x00, 0x0A, 0x80, 0x80, 0x05, 0x21, 0x00, 0x01, 0x00, 0x01, 0xAA, 0xBB,
];
const PROGRAM_END_CODE: &[u8] = &[0x00, 0x00, 0x01, 0xB9];

/// A PSM for one MPEG-2 video elementary stream 0xE0, no descriptors.
pub(crate) fn psm(version: u8) -> Vec<u8> {
    ProgramStreamMap {
        current_next_indicator: true,
        single_extension_stream_flag: false,
        version,
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
    .try_to_bytes()
    .expect("the vector PSM serializes")
}

fn pack(psm_bytes: &[u8]) -> Vec<u8> {
    let mut b = PACK_HEADER.to_vec();
    b.extend_from_slice(psm_bytes);
    b.extend_from_slice(PES);
    b
}

/// A PSM for one `stream_id` 0xFD elementary stream with a stream_id_extension
/// and no descriptors: the pseudo descriptor is `pseudo_descriptor_tag` 0x01,
/// `pseudo_descriptor_length` 1, marker + extension 0x8F (ISO/IEC 13818-1
/// §2.5.4.2).
pub(crate) fn psm_with_extension() -> Vec<u8> {
    ProgramStreamMap {
        current_next_indicator: true,
        single_extension_stream_flag: false,
        version: 1,
        program_stream_info: DescriptorLoop::new(&[]),
        elementary_stream_map: vec![EsMapEntry {
            stream_type: 0x06,
            elementary_stream_id: 0xFD,
            stream_id_extension: Some(0x0F),
            pseudo_descriptor_tag: Some(0x01),
            pseudo_descriptor_length: Some(0x01),
            descriptors: DescriptorLoop::new(&[]),
        }],
        crc: 0,
    }
    .try_to_bytes()
    .expect("the vector PSM serializes")
}

/// Two packs, each a PSM (versions 1 and 2) followed by one PES; the wire PSM
/// length and the re-serialized PSM length are equal here, so this is the
/// control vector: it walks identically before and after the wire-length fix.
pub(crate) fn two_psms() -> Vec<u8> {
    let mut b = pack(&psm(1));
    b.extend_from_slice(&pack(&psm(2)));
    b.extend_from_slice(PROGRAM_END_CODE);
    b
}

/// Pack 1's PSM declares `elementary_stream_map_length` 4 for one entry with
/// no descriptors, and its `program_stream_map_length` (0x0011, CRC_32
/// included) leaves 3 further bytes in the ES loop that no entry claims. The
/// parsed struct cannot represent them, so its re-serialized length is 3 bytes
/// shorter than the wire PSM. Pack 2 is a well-formed PSM (version 2).
pub(crate) fn trailing_es_bytes() -> Vec<u8> {
    let mut first = PACK_HEADER.to_vec();
    first.extend_from_slice(&[
        0x00, 0x00, 0x01, 0xBC, 0x00, 0x11, 0xE1, 0xFF, 0x00, 0x00, 0x00, 0x04, 0x02, 0xE0, 0x00,
        0x00, 0x58, 0x59, 0x5A, 0xEE, 0x41, 0x5B, 0xB1,
    ]);
    first.extend_from_slice(PES);
    first.extend_from_slice(&pack(&psm(2)));
    first.extend_from_slice(PROGRAM_END_CODE);
    first
}

/// Both PSM descriptor loops are typed and walked: the program loop carries
/// a registration descriptor (0x05) and the ES loop a subtitling descriptor
/// (0x59), each reaching the caller's own decoder through `parse_with`, while
/// the wire bytes stay untouched (mpeg-ps keeps no `dvb-si` dependency —
/// `dvb-si` sits above `mpeg-ts`, which sits above this crate, so the caller
/// names the decoder).
#[test]
fn psm_descriptor_loops_walk_typed_and_keep_bytes() {
    use broadcast_common::{Parse, Serialize};

    let prog = [0x05u8, 0x04, b'H', b'D', b'M', b'V'];
    let es = [
        0x59u8, 0x08, b'e', b'n', b'g', 0x00, 0x10, 0x00, 0x01, 0x00, 0x02,
    ];
    let psm = ProgramStreamMap {
        current_next_indicator: true,
        single_extension_stream_flag: false,
        version: 1,
        program_stream_info: DescriptorLoop::new(&prog),
        elementary_stream_map: vec![EsMapEntry {
            stream_type: 0x02,
            elementary_stream_id: 0xE0,
            stream_id_extension: None,
            pseudo_descriptor_tag: None,
            pseudo_descriptor_length: None,
            descriptors: DescriptorLoop::new(&es),
        }],
        crc: 0,
    };

    let mut wire = vec![0u8; psm.serialized_len()];
    psm.serialize_into(&mut wire).unwrap();
    let parsed = ProgramStreamMap::parse(&wire).unwrap();
    assert_eq!(parsed.try_to_bytes().unwrap(), wire);

    let prog_tags: Vec<u8> = parsed
        .program_stream_info
        .parse_with(|tag, _| tag)
        .filter_map(Result::ok)
        .collect();
    assert_eq!(prog_tags, [0x05]);

    let es_loop = &parsed.elementary_stream_map[0].descriptors;
    let es_tags: Vec<u8> = es_loop
        .parse_with(|tag, _| tag)
        .filter_map(Result::ok)
        .collect();
    assert_eq!(es_tags, [0x59]);

    assert_eq!(parsed.program_stream_info.raw(), &prog[..]);
    assert_eq!(es_loop.raw(), &es[..]);

    let entries: Vec<_> = parsed
        .program_stream_info
        .entries()
        .map(|e| e.unwrap())
        .collect();
    assert_eq!(entries.len(), 1);
    assert_eq!((entries[0].tag, entries[0].body), (0x05, &b"HDMV"[..]));
}

/// A hand-assembled Table 2-41 PSM: `current_next_indicator` 0,
/// `single_extension_stream_flag` 0, version 0x15, a non-empty program
/// descriptor loop, and three elementary-stream entries, the middle one
/// carrying a 3-byte pseudo descriptor (`elementary_stream_id` 0xFD) plus one
/// descriptor. Every field is distinct so a wrong bit position cannot hide.
fn table_2_41_psm() -> Vec<u8> {
    let mut wire = vec![
        0x00, 0x00, 0x01, 0xBC, // packet_start_code_prefix + map_stream_id
        0x00, 0x25, // program_stream_map_length = 37
        0x35, // current_next=0 | single_extension=0 | reserved=1 | version=0x15
        0xFF, // reserved + marker_bit
        0x00, 0x06, // program_stream_info_length = 6
        0x05, 0x04, b'H', b'D', b'M', b'V', // registration descriptor
        0x00, 0x15, // elementary_stream_map_length = 21
        0x02, 0xE0, 0x00, 0x00, // video, no descriptors
        0x06, 0xFD, 0x00, 0x06, // stream_id_extension entry, ES_info_length 6
        0x01, 0x04, 0xC5, // pseudo_descriptor_tag 1, length 4, marker+0x45
        0x59, 0x01, 0xAB, // its own descriptor
        0x03, 0xC1, 0x00, 0x03, // audio-ish entry with one descriptor
        0x59, 0x01, 0xAB,
    ];
    let crc = broadcast_common::crc32_mpeg2::compute(&wire);
    wire.extend_from_slice(&crc.to_be_bytes());
    wire
}

#[test]
fn psm_flags_and_every_entry_decode_field_by_field() {
    let wire = table_2_41_psm();
    let psm = ProgramStreamMap::parse(&wire).expect("the hand-built PSM parses");

    assert!(!psm.current_next_indicator);
    assert!(!psm.single_extension_stream_flag);
    assert_eq!(psm.version, 0x15);
    assert_eq!(
        psm.program_stream_info.raw(),
        &[0x05, 0x04, b'H', b'D', b'M', b'V']
    );

    assert_eq!(psm.elementary_stream_map.len(), 3);
    let video = &psm.elementary_stream_map[0];
    assert_eq!(
        (video.stream_type, video.elementary_stream_id),
        (0x02, 0xE0)
    );
    assert_eq!(video.stream_id_extension, None);
    assert_eq!(video.pseudo_descriptor_tag, None);
    assert!(video.descriptors.raw().is_empty());

    let extended = &psm.elementary_stream_map[1];
    assert_eq!(
        (extended.stream_type, extended.elementary_stream_id),
        (0x06, 0xFD)
    );
    assert_eq!(extended.stream_id_extension, Some(0x45));
    assert_eq!(extended.pseudo_descriptor_tag, Some(0x01));
    assert_eq!(extended.pseudo_descriptor_length, Some(0x04));
    assert_eq!(extended.descriptors.raw(), &[0x59, 0x01, 0xAB]);

    let third = &psm.elementary_stream_map[2];
    assert_eq!(
        (third.stream_type, third.elementary_stream_id),
        (0x03, 0xC1)
    );
    assert_eq!(third.stream_id_extension, None);
    assert_eq!(third.descriptors.raw(), &[0x59, 0x01, 0xAB]);

    let mut out = vec![0u8; psm.serialized_len()];
    assert_eq!(psm.serialize_into(&mut out).unwrap(), wire.len());
    assert_eq!(out, wire, "serialize must reproduce the hand-built bytes");
}

#[test]
fn a_cleared_extension_marker_is_rejected() {
    let mut wire = table_2_41_psm();
    wire[28] &= 0x7F; // clear bit 7 of the marker+elementary_stream_id_extension byte
    let crc_at = wire.len() - 4;
    let crc = broadcast_common::crc32_mpeg2::compute(&wire[..crc_at]);
    wire[crc_at..].copy_from_slice(&crc.to_be_bytes());
    assert_eq!(
        ProgramStreamMap::parse(&wire).unwrap_err(),
        Error::BadMarker("elementary_stream_id_extension marker")
    );
}

#[test]
fn a_set_single_extension_stream_flag_decodes_the_entry_as_plain() {
    let psm = ProgramStreamMap {
        current_next_indicator: true,
        single_extension_stream_flag: true,
        version: 0x15,
        program_stream_info: DescriptorLoop::new(&[]),
        elementary_stream_map: vec![EsMapEntry {
            stream_type: 0x06,
            elementary_stream_id: 0xFD,
            stream_id_extension: None,
            pseudo_descriptor_tag: None,
            pseudo_descriptor_length: None,
            descriptors: DescriptorLoop::new(&[]),
        }],
        crc: 0,
    };
    let wire = psm.try_to_bytes().unwrap();
    let parsed = ProgramStreamMap::parse(&wire).unwrap();
    assert!(parsed.current_next_indicator);
    assert!(parsed.single_extension_stream_flag);
    assert_eq!(parsed.elementary_stream_map[0].stream_id_extension, None);
    assert_eq!(parsed.try_to_bytes().unwrap(), wire);
}
