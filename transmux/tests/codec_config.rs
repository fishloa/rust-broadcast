//! Real-fixture SPS decode + RFC 6381 gate test (#425).
//!
//! EXIT CRITERIA:
//! 1. Every H.264 fixture in fixtures/ts/h264/ decodes to the exact oracle values.
//! 2. h264_aac.ts at the repo root decodes correctly.
//! 3. Every HEVC fixture decodes with correct profile / bit-depth / dimensions.
//! 4. Scaling-matrix skip path verified (unit test in transmux::sps).
//! 5. avcC round-trip: SPS decode → build AVCDecoderConfigurationRecord → verify
//!    profile/compat/level bytes match SPS bytes exactly.

use broadcast_common::Serialize;
use mpeg_pes::PesAssembler;
use mpeg_ts::OwnedTsPacket;
use transmux::annexb::iter_annexb_nals;
use transmux::{AVCDecoderConfigurationRecord, AvcSps, AvcSpsInfo, HevcNalUnit, HevcSpsInfo};

// ---- Helpers -----------------------------------------------------------------

/// Feed a TS file (by path) for a given PID, return all NAL units found via
/// Annex B demux.
fn demux_annexb_nals<P: AsRef<std::path::Path>>(path: P, pid: u16) -> Vec<Vec<u8>> {
    let ts = std::fs::read(path.as_ref()).expect("fixture must exist");
    assert_eq!(ts.len() % 188, 0, "TS must be 188-byte aligned");

    let mut assembler = PesAssembler::new();
    let mut nals: Vec<Vec<u8>> = Vec::new();

    for chunk in ts.chunks_exact(188) {
        let raw: [u8; 188] = chunk.try_into().unwrap();
        let pkt = OwnedTsPacket::parse(raw).expect("valid TS packet");
        let payload = match pkt.payload() {
            Some(p) => p,
            None => continue,
        };
        if pkt.pid != pid {
            continue;
        }
        if let Some(completed) = assembler.feed(pkt.pusi, payload) {
            // Skip PES header: 6 bytes minimum (start code + stream_id + length)
            let body = if completed.len() > 6 {
                &completed[6..]
            } else {
                &completed
            };
            for nal in iter_annexb_nals(body) {
                if !nal.is_empty() {
                    nals.push(nal.to_vec());
                }
            }
        }
    }
    // Flush
    if let Some(completed) = assembler.flush() {
        let body = if completed.len() > 6 {
            &completed[6..]
        } else {
            &completed
        };
        for nal in iter_annexb_nals(body) {
            if !nal.is_empty() {
                nals.push(nal.to_vec());
            }
        }
    }
    nals
}

/// Find the first SPS NAL (NAL unit type 7 for AVC) in a list of NAL units.
fn find_first_sps(nals: &[Vec<u8>]) -> Option<Vec<u8>> {
    nals.iter()
        .find(|nal| !nal.is_empty() && (nal[0] & 0x1F) == 7)
        .cloned()
}

/// Find the first HEVC SPS NAL (NAL unit type 33).
fn find_first_hevc_sps(nals: &[Vec<u8>]) -> Option<Vec<u8>> {
    nals.iter()
        .find(|nal| nal.len() >= 2 && ((nal[0] >> 1) & 0x3F) == 33)
        .cloned()
}

/// Decode an H.264 SPS and return its info.
fn decode_sps(nal: &[u8]) -> AvcSpsInfo {
    let sps = AvcSps(nal.to_vec());
    sps.decode().expect("SPS decode must succeed")
}

/// Decode an HEVC SPS and return its info.
fn decode_hevc(nal: &[u8]) -> HevcSpsInfo {
    let nalu = HevcNalUnit::new(nal.to_vec());
    nalu.decode_sps()
        .expect("HEVC SPS decode must succeed")
        .expect("must be SPS")
}

fn fixture_path(rel: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(rel)
}

// ---- H.264 tests ------------------------------------------------------------

#[test]
fn h264_baseline_sps() {
    let path = fixture_path("fixtures/ts/h264/baseline.ts");
    let nals = demux_annexb_nals(path, 0x0100);
    let sps_nal = find_first_sps(&nals).expect("must find SPS");
    let info = decode_sps(&sps_nal);

    // Oracle: profile 66, constraint 0xC0, level 13, chroma 1, 8-bit, 320×240
    assert_eq!(info.profile_idc, 66);
    assert_eq!(info.constraint_flags, 0xC0);
    assert_eq!(info.level_idc, 13);
    assert_eq!(info.chroma_format_idc, 1);
    assert_eq!(info.bit_depth_luma, 8);
    assert_eq!(info.bit_depth_chroma, 8);
    assert_eq!(info.width, 320);
    assert_eq!(info.height, 240);
    assert!(info.frame_mbs_only);

    let rfc = AvcSps(sps_nal).rfc6381().expect("rfc6381");
    assert_eq!(rfc, "avc1.42C00D");
}

#[test]
fn h264_main_sps() {
    let path = fixture_path("fixtures/ts/h264/main.ts");
    let nals = demux_annexb_nals(path, 0x0100);
    let sps_nal = find_first_sps(&nals).expect("must find SPS");
    let info = decode_sps(&sps_nal);

    assert_eq!(info.profile_idc, 77);
    assert_eq!(info.constraint_flags, 0x40);
    assert_eq!(info.level_idc, 13);
    assert_eq!(info.chroma_format_idc, 1);
    assert_eq!(info.bit_depth_luma, 8);
    assert_eq!(info.bit_depth_chroma, 8);
    assert_eq!(info.width, 320);
    assert_eq!(info.height, 240);
    assert!(info.frame_mbs_only);

    let rfc = AvcSps(sps_nal).rfc6381().expect("rfc6381");
    assert_eq!(rfc, "avc1.4D400D");
}

#[test]
fn h264_high_sps() {
    let path = fixture_path("fixtures/ts/h264/high.ts");
    let nals = demux_annexb_nals(path, 0x0100);
    let sps_nal = find_first_sps(&nals).expect("must find SPS");
    let info = decode_sps(&sps_nal);

    assert_eq!(info.profile_idc, 100);
    assert_eq!(info.constraint_flags, 0x00);
    assert_eq!(info.level_idc, 13);
    assert_eq!(info.chroma_format_idc, 1);
    assert_eq!(info.bit_depth_luma, 8);
    assert_eq!(info.bit_depth_chroma, 8);
    assert_eq!(info.width, 320);
    assert_eq!(info.height, 240);
    assert!(info.frame_mbs_only);

    let rfc = AvcSps(sps_nal).rfc6381().expect("rfc6381");
    assert_eq!(rfc, "avc1.64000D");
}

#[test]
fn h264_high10_sps() {
    let path = fixture_path("fixtures/ts/h264/high10.ts");
    let nals = demux_annexb_nals(path, 0x0100);
    let sps_nal = find_first_sps(&nals).expect("must find SPS");
    let info = decode_sps(&sps_nal);

    assert_eq!(info.profile_idc, 110);
    assert_eq!(info.constraint_flags, 0x00);
    assert_eq!(info.level_idc, 13);
    assert_eq!(info.chroma_format_idc, 1);
    assert_eq!(info.bit_depth_luma, 10);
    assert_eq!(info.bit_depth_chroma, 10);
    assert_eq!(info.width, 320);
    assert_eq!(info.height, 240);
    assert!(info.frame_mbs_only);

    let rfc = AvcSps(sps_nal).rfc6381().expect("rfc6381");
    assert_eq!(rfc, "avc1.6E000D");
}

#[test]
fn h264_high422_sps() {
    let path = fixture_path("fixtures/ts/h264/high422.ts");
    let nals = demux_annexb_nals(path, 0x0100);
    let sps_nal = find_first_sps(&nals).expect("must find SPS");
    let info = decode_sps(&sps_nal);

    assert_eq!(info.profile_idc, 122);
    assert_eq!(info.constraint_flags, 0x00);
    assert_eq!(info.level_idc, 13);
    assert_eq!(info.chroma_format_idc, 2);
    assert_eq!(info.bit_depth_luma, 8);
    assert_eq!(info.bit_depth_chroma, 8);
    assert_eq!(info.width, 320);
    assert_eq!(info.height, 240);
    assert!(info.frame_mbs_only);

    let rfc = AvcSps(sps_nal).rfc6381().expect("rfc6381");
    assert_eq!(rfc, "avc1.7A000D");
}

#[test]
fn h264_high444_sps() {
    let path = fixture_path("fixtures/ts/h264/high444.ts");
    let nals = demux_annexb_nals(path, 0x0100);
    let sps_nal = find_first_sps(&nals).expect("must find SPS");
    let info = decode_sps(&sps_nal);

    assert_eq!(info.profile_idc, 244);
    assert_eq!(info.constraint_flags, 0x00);
    assert_eq!(info.level_idc, 13);
    assert_eq!(info.chroma_format_idc, 3);
    assert_eq!(info.bit_depth_luma, 8);
    assert_eq!(info.bit_depth_chroma, 8);
    assert_eq!(info.width, 320);
    assert_eq!(info.height, 240);
    assert!(info.frame_mbs_only);

    let rfc = AvcSps(sps_nal).rfc6381().expect("rfc6381");
    assert_eq!(rfc, "avc1.F4000D");
}

#[test]
fn h264_interlaced_sps() {
    let path = fixture_path("fixtures/ts/h264/interlaced.ts");
    let nals = demux_annexb_nals(path, 0x0100);
    let sps_nal = find_first_sps(&nals).expect("must find SPS");
    let info = decode_sps(&sps_nal);

    assert_eq!(info.profile_idc, 100);
    assert_eq!(info.constraint_flags, 0x00);
    assert_eq!(info.level_idc, 30);
    assert_eq!(info.chroma_format_idc, 1);
    assert_eq!(info.bit_depth_luma, 8);
    assert_eq!(info.bit_depth_chroma, 8);
    assert_eq!(info.width, 720);
    assert_eq!(info.height, 576);
    // interlaced = field coding → frame_mbs_only_flag = 0
    assert!(!info.frame_mbs_only);

    let rfc = AvcSps(sps_nal).rfc6381().expect("rfc6381");
    assert_eq!(rfc, "avc1.64001E");
}

#[test]
fn h264_high_1080_cropped_sps() {
    let path = fixture_path("fixtures/ts/h264/high_1080_cropped.ts");
    let nals = demux_annexb_nals(path, 0x0100);
    let sps_nal = find_first_sps(&nals).expect("must find SPS");
    let info = decode_sps(&sps_nal);

    assert_eq!(info.profile_idc, 100);
    assert_eq!(info.constraint_flags, 0x00);
    assert_eq!(info.chroma_format_idc, 1);
    assert_eq!(info.bit_depth_luma, 8);
    assert_eq!(info.bit_depth_chroma, 8);
    // 1088 coded → 1080 displayed (crop_bottom = 4)
    assert_eq!(info.width, 1920);
    assert_eq!(info.height, 1080);
    assert!(info.frame_mbs_only);
}

#[test]
fn h264_aac_ts_sps() {
    let path = fixture_path("fixtures/ts/h264_aac.ts");
    let nals = demux_annexb_nals(path, 0x0100);
    let sps_nal = find_first_sps(&nals).expect("must find SPS");
    let info = decode_sps(&sps_nal);

    assert_eq!(info.profile_idc, 77);
    assert_eq!(info.constraint_flags, 0x40);
    assert_eq!(info.level_idc, 13);
    assert_eq!(info.chroma_format_idc, 1);
    assert_eq!(info.width, 320);
    assert_eq!(info.height, 240);
    assert!(info.frame_mbs_only);

    let rfc = AvcSps(sps_nal).rfc6381().expect("rfc6381");
    assert_eq!(rfc, "avc1.4D400D");
}

// ---- HEVC tests -------------------------------------------------------------

#[test]
fn hevc_main_sps() {
    let path = fixture_path("fixtures/ts/hevc/main.ts");
    let nals = demux_annexb_nals(path, 0x0100);
    let sps_nal = find_first_hevc_sps(&nals).expect("must find HEVC SPS");
    let info = decode_hevc(&sps_nal);

    assert_eq!(info.general_profile_idc, 1);
    assert_eq!(info.chroma_format_idc, 1);
    assert_eq!(info.bit_depth_luma, 8);
    assert_eq!(info.bit_depth_chroma, 8);
    assert_eq!(info.width, 320);
    assert_eq!(info.height, 240);

    let nalu = HevcNalUnit::new(sps_nal);
    let rfc = nalu.rfc6381().expect("rfc6381").expect("must be SPS");
    assert!(!rfc.is_empty());
    assert!(rfc.starts_with("hvc1."));
}

#[test]
fn hevc_main10_sps() {
    let path = fixture_path("fixtures/ts/hevc/main10.ts");
    let nals = demux_annexb_nals(path, 0x0100);
    let sps_nal = find_first_hevc_sps(&nals).expect("must find HEVC SPS");
    let info = decode_hevc(&sps_nal);

    assert_eq!(info.general_profile_idc, 2);
    assert_eq!(info.chroma_format_idc, 1);
    assert_eq!(info.bit_depth_luma, 10);
    assert_eq!(info.bit_depth_chroma, 10);
    assert_eq!(info.width, 320);
    assert_eq!(info.height, 240);

    let nalu = HevcNalUnit::new(sps_nal);
    let rfc = nalu.rfc6381().expect("rfc6381").expect("must be SPS");
    assert!(!rfc.is_empty());
    assert!(rfc.starts_with("hvc1."));
}

// ---- avcC round-trip --------------------------------------------------------

#[test]
fn avcc_round_trip_from_sps_decode() {
    // Decode a fixture SPS → build avcC whose profile/compat/level come from
    // decode() → assert the avcC bytes match SPS bytes 1/2/3.
    let path = fixture_path("fixtures/ts/h264/high.ts");
    let nals = demux_annexb_nals(path, 0x0100);
    let sps_nal = find_first_sps(&nals).expect("must find SPS");
    let info = decode_sps(&sps_nal);

    // The SPS NAL has: [0]=NAL header, [1]=profile_idc, [2]=constraint, [3]=level_idc
    assert_eq!(info.profile_idc, sps_nal[1]);
    assert_eq!(info.constraint_flags, sps_nal[2]);
    assert_eq!(info.level_idc, sps_nal[3]);

    // Find first PPS (NAL type 8)
    let pps_nal = nals
        .iter()
        .find(|nal| !nal.is_empty() && (nal[0] & 0x1F) == 8)
        .expect("must find PPS");

    let record = AVCDecoderConfigurationRecord {
        configuration_version: 1,
        profile_indication: info.profile_idc,
        profile_compatibility: info.constraint_flags,
        level_indication: info.level_idc,
        length_size_minus_one: 3,
        sps: vec![AvcSps(sps_nal.to_vec())],
        pps: vec![transmux::AvcPps(pps_nal.to_vec())],
        chroma_format: if info.profile_idc >= 100 {
            Some(info.chroma_format_idc)
        } else {
            None
        },
        bit_depth_luma_minus8: if info.profile_idc >= 100 {
            Some(info.bit_depth_luma - 8)
        } else {
            None
        },
        bit_depth_chroma_minus8: if info.profile_idc >= 100 {
            Some(info.bit_depth_chroma - 8)
        } else {
            None
        },
        sps_ext: vec![],
    };

    let serialized = record.to_bytes();

    // Bytes 1/2/3 of avcC are profile_indication, profile_compatibility, level_indication
    assert_eq!(
        serialized[1], sps_nal[1],
        "avcC profile must match SPS profile"
    );
    assert_eq!(
        serialized[2], sps_nal[2],
        "avcC compatibility must match SPS constraint_flags"
    );
    assert_eq!(
        serialized[3], sps_nal[3],
        "avcC level must match SPS level_idc"
    );
}
