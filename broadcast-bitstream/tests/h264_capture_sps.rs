//! Real-fixture check: the SPS of a broadcast capture, found with this crate's
//! own Annex B iterator and decoded by this crate alone.
//!
//! `fixtures/ts/h264-aac.ts` is an MPEG-2 transport stream carrying H.264
//! (320x240 Main profile, level 1.3) and AAC. The transport layer is walked with
//! `mpeg-ts` + `mpeg-pes` (dev-dependencies here) so the elementary-stream
//! bytes are real broadcast bytes, not a hand-built bitstream; finding the
//! parameter set inside them is this crate's job.

#![cfg(feature = "h264")]

use broadcast_bitstream::rfc6381::rfc6381_avc1;
use broadcast_bitstream::sps::{decode_avc_sps, is_high_profile};
use broadcast_bitstream::{AvcNalUnitType, iter_annexb_nals, nal_unit_type};
use mpeg_pes::PesAssembler;
use mpeg_ts::ts::TsPacket;
use std::collections::BTreeMap;

/// One TS packet is 188 bytes (ISO/IEC 13818-1 §2.4.3.2).
const TS_PACKET_SIZE: usize = 188;

/// First SPS NAL unit carried by the file's video PES payloads, in file order.
fn first_sps(file: &[u8]) -> Option<Vec<u8>> {
    let mut assemblers: BTreeMap<u16, PesAssembler> = BTreeMap::new();
    for packet in file.chunks_exact(TS_PACKET_SIZE) {
        let Ok(ts) = TsPacket::parse(packet) else {
            continue;
        };
        let Some(payload) = ts.payload else {
            continue;
        };
        let assembler = assemblers.entry(ts.header.pid.value()).or_default();
        let Some(pes) = assembler.feed(ts.header.pusi, payload) else {
            continue;
        };
        for nal in iter_annexb_nals(&pes) {
            if is_avc_sps(nal) {
                return Some(nal.to_vec());
            }
        }
    }
    None
}

/// Whether one Annex B NAL unit is an H.264 sequence parameter set.
fn is_avc_sps(nal: &[u8]) -> bool {
    nal_unit_type(broadcast_bitstream::NalCodec::Avc, nal)
        .is_some_and(|code| AvcNalUnitType::from_u8(code) == AvcNalUnitType::Sps)
}

#[test]
fn the_capture_sps_decodes_to_its_profile_and_codec_string() {
    let file = test_fixtures::read("ts/h264-aac.ts");
    let sps = first_sps(&file).expect("the capture carries an H.264 SPS");
    let info = decode_avc_sps(&sps).expect("the capture's SPS decodes");

    assert_eq!((info.width(), info.height()), (320, 240));
    assert_eq!(info.profile_idc(), 77, "Main profile");
    assert_eq!(info.constraint_flags(), 0x40);
    assert_eq!(info.level_idc(), 13);
    // Main profile is not one of the high-profile SPS variants.
    assert!(!is_high_profile(info.profile_idc()));
    assert_eq!(
        rfc6381_avc1(
            info.profile_idc(),
            info.constraint_flags(),
            info.level_idc()
        ),
        "avc1.4D400D"
    );
}
