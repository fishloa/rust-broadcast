//! Hand-built Program Stream vectors (Table 2-38/2-41 shapes the committed
//! ffmpeg capture does not carry: it has no Program Stream Map at all).
//!
//! Every pack header is the committed fixture's own
//! (`fixtures/ps/ffmpeg-mpeg2-ps.mpg` bytes 0..14: SCR 0, `program_mux_rate`
//! 0x10CD8E, no stuffing), every PSM is built by `ProgramStreamMap`'s own
//! serializer (so its CRC_32 is the real MPEG-2 CRC-32), and the PES packet is
//! a minimal PTS-only packet.

use broadcast_common::Serialize;
use mpeg_ps::{EsMapEntry, ProgramStreamMap};

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
        program_stream_info: &[],
        elementary_stream_map: vec![EsMapEntry {
            stream_type: 0x02,
            elementary_stream_id: 0xE0,
            stream_id_extension: None,
            descriptors: &[],
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
