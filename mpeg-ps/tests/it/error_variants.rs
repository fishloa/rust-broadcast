//! Error variants of the pack header, system header and program stream map,
//! each produced from the real ffmpeg fixture (or its serialized PSM) with the
//! smallest mutation that triggers it, asserting the exact variant and fields.
//! Serialize into a short buffer reports `OutputBufferTooSmall`.

use broadcast_common::{Parse, Serialize};
use mpeg_ps::program_stream;
use mpeg_ps::{Error, EsMapEntry, PackHeader, ProgramStreamMap, SystemHeader};

const PACK_HEADER_FIXED_LEN: usize = 14;
const MUX_RATE_BYTES: std::ops::Range<usize> = 10..13;
const PSM_STREAM_ID_AT: usize = 3;

fn fixture() -> Vec<u8> {
    test_fixtures::read("ps/ffmpeg-mpeg2-ps.mpg")
}

fn first_pack_and_system_header_bytes(data: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let (packs, _) = program_stream::parse_all_packs(data).unwrap();
    let ph_len = packs[0].pack_header.serialized_len();
    let sh_len = packs[0].system_header.as_ref().unwrap().serialized_len();
    (
        data[..ph_len].to_vec(),
        data[ph_len..ph_len + sh_len].to_vec(),
    )
}

#[test]
fn a_truncated_pack_header_reports_what_is_missing() {
    let (pack, _) = first_pack_and_system_header_bytes(&fixture());
    assert!(pack.len() >= PACK_HEADER_FIXED_LEN);
    assert_eq!(
        PackHeader::parse(&pack[..PACK_HEADER_FIXED_LEN - 1]).unwrap_err(),
        Error::BufferTooShort(broadcast_common::error::BufferTooShort {
            need: PACK_HEADER_FIXED_LEN,
            have: PACK_HEADER_FIXED_LEN - 1,
            what: "pack_header",
        })
    );
    assert_eq!(
        PackHeader::parse(&[]).unwrap_err(),
        Error::BufferTooShort(broadcast_common::error::BufferTooShort {
            need: PACK_HEADER_FIXED_LEN,
            have: 0,
            what: "pack_header",
        })
    );
}

#[test]
fn stuffing_declared_beyond_the_buffer_is_reported() {
    let (pack, _) = first_pack_and_system_header_bytes(&fixture());
    let mut stuffed = pack[..PACK_HEADER_FIXED_LEN].to_vec();
    stuffed[13] = (stuffed[13] & !0x07) | 0x05;
    assert_eq!(
        PackHeader::parse(&stuffed).unwrap_err(),
        Error::BufferTooShort(broadcast_common::error::BufferTooShort {
            need: PACK_HEADER_FIXED_LEN + 5,
            have: PACK_HEADER_FIXED_LEN,
            what: "pack_header stuffing bytes",
        })
    );
}

#[test]
fn a_zero_program_mux_rate_is_rejected_but_the_markers_are_kept() {
    let (pack, _) = first_pack_and_system_header_bytes(&fixture());
    PackHeader::parse(&pack).expect("the real pack header parses");
    let mut zero = pack.clone();
    zero[MUX_RATE_BYTES.start] = 0;
    zero[MUX_RATE_BYTES.start + 1] = 0;
    zero[MUX_RATE_BYTES.start + 2] = 0x03;
    assert_eq!(PackHeader::parse(&zero).unwrap_err(), Error::ZeroMuxRate);
}

#[test]
fn a_system_header_with_a_wrong_start_code_names_the_code() {
    let (_, system_header) = first_pack_and_system_header_bytes(&fixture());
    SystemHeader::parse(&system_header).expect("the real system header parses");
    let mut bad = system_header.clone();
    bad[3] = 0xBC;
    assert_eq!(
        SystemHeader::parse(&bad).unwrap_err(),
        Error::BadSystemHeaderStartCode(u32::from_be_bytes([bad[0], bad[1], bad[2], 0xBC]))
    );
    assert_eq!(
        SystemHeader::parse(&system_header[..3]).unwrap_err(),
        Error::BufferTooShort(broadcast_common::error::BufferTooShort {
            need: 6,
            have: 3,
            what: "system_header prefix",
        })
    );
}

fn psm_bytes() -> Vec<u8> {
    let psm = ProgramStreamMap {
        current_next_indicator: true,
        single_extension_stream_flag: false,
        version: 3,
        program_stream_info: &[],
        elementary_stream_map: vec![EsMapEntry {
            stream_type: 0x02,
            elementary_stream_id: 0xE0,
            stream_id_extension: None,
            descriptors: &[0x0A, 0x04, b'H', b'E', b'L', b'L'],
        }],
        crc: 0,
    };
    psm.try_to_bytes().unwrap()
}

#[test]
fn a_program_stream_map_with_the_wrong_map_stream_id_names_it() {
    let good = psm_bytes();
    ProgramStreamMap::parse(&good).expect("own output parses");
    let mut bad = good.clone();
    bad[PSM_STREAM_ID_AT] = 0xBD;
    assert_eq!(
        ProgramStreamMap::parse(&bad).unwrap_err(),
        Error::BadMapStreamId(0xBD)
    );
    let mut bad_prefix = good.clone();
    bad_prefix[0] = 0x01;
    assert_eq!(
        ProgramStreamMap::parse(&bad_prefix).unwrap_err(),
        Error::BadMapStreamId(good[PSM_STREAM_ID_AT])
    );
}

#[test]
fn a_truncated_program_stream_map_reports_the_minimum_length() {
    let good = psm_bytes();
    let err = ProgramStreamMap::parse(&good[..5]).unwrap_err();
    assert_eq!(
        err,
        Error::BufferTooShort(broadcast_common::error::BufferTooShort {
            need: 12,
            have: 5,
            what: "program_stream_map",
        })
    );
}

#[test]
fn serializing_into_a_short_buffer_reports_output_buffer_too_small_with_the_needed_length() {
    let (pack, system_header) = first_pack_and_system_header_bytes(&fixture());
    let ph = PackHeader::parse(&pack).unwrap();
    let sh = SystemHeader::parse(&system_header).unwrap();
    let to_short = |e: &Error| match e {
        Error::OutputBufferTooSmall(broadcast_common::error::OutputBufferTooSmall {
            need,
            have,
        }) => Some((*need, *have)),
        _ => None,
    };
    test_fixtures::assert_short_buffers_rejected(
        ph.serialized_len(),
        |buf| ph.serialize_into(buf),
        to_short,
    );
    test_fixtures::assert_short_buffers_rejected(
        sh.serialized_len(),
        |buf| sh.serialize_into(buf),
        to_short,
    );
    let psm_wire = psm_bytes();
    let psm = ProgramStreamMap::parse(&psm_wire).unwrap();
    test_fixtures::assert_short_buffers_rejected(
        psm.serialized_len(),
        |buf| psm.serialize_into(buf),
        to_short,
    );
}
