//! H.264/HEVC profile-matrix hardening — SPS decode + TS→IR→fMP4 gate (#563).
//!
//! `decode_avc_sps`/`decode_hevc_sps` and the TS→fMP4 path were built and
//! tested against common profiles (Main/High 4:2:0 8-bit). This file drives
//! both across the **full** fixture matrix — baseline/main/high/high10/
//! high422/high444/high_1080_cropped/interlaced (H.264) and main/main10
//! (HEVC) — asserting decode == an external ffprobe oracle recorded here as
//! literals, so the test is self-contained.
//!
//! # Ground truth (recorded by hand from ffprobe, not derived from the code
//! under test)
//!
//! ```text
//! ffprobe -v error -select_streams v -show_entries \
//!   stream=codec_name,width,height,profile,level,pix_fmt,field_order,r_frame_rate \
//!   -of default=noprint_wrappers=1 <fixture>
//! ```
//!
//! | fixture                        | profile | level | dims      | pix_fmt       | field_order |
//! |---------------------------------|---------|-------|-----------|---------------|-------------|
//! | h264/baseline.ts                 | 66      | 13    | 320x240   | yuv420p       | progressive |
//! | h264/main.ts                     | 77      | 13    | 320x240   | yuv420p       | progressive |
//! | h264/high.ts                     | 100     | 13    | 320x240   | yuv420p       | progressive |
//! | h264/high10.ts                   | 110     | 13    | 320x240   | yuv420p10le   | progressive |
//! | h264/high422.ts                  | 122     | 13    | 320x240   | yuv422p       | progressive |
//! | h264/high444.ts                  | 244     | 13    | 320x240   | yuv444p       | progressive |
//! | h264/high_1080_cropped.ts        | 100     | 40    | 1920x1080 | yuv420p       | progressive |
//! | h264/interlaced.ts               | 100     | 30    | 720x576   | yuv420p       | tt (interlaced) |
//! | hevc/main.ts                     | Main(1) | 60    | 320x240   | yuv420p       | unknown     |
//! | hevc/main10.ts                   | Main10(2)| 60   | 320x240   | yuv420p10le   | unknown     |
//!
//! `r_frame_rate` is `25/1` for every fixture in the matrix.
//!
//! All fixtures are ffprobe-identity-verified per #558.
//!
//! # What this file does NOT do
//!
//! Per the #563 boundary (a parallel branch owns `ts_demux.rs`/`media.rs`/
//! `pipeline.rs`), no `TrackSpec`/`Sample`/`CodecConfig` is ever hand-built —
//! every value asserted here comes from [`TsDemux`]'s own demux output.
//! `AVCDecoderConfigurationRecord.chroma_format`/`bit_depth_*_minus8` are not
//! currently populated by `ts_demux.rs` for AVC tracks (it hardcodes `None`;
//! HEVC's `HEVCDecoderConfigurationRecord` *is* fully populated) — that wiring
//! gap lives in `ts_demux.rs`, out of this story's file boundary. Instead this
//! file verifies AVC chroma/bit-depth correctness the way a real decoder
//! would: by decoding the **raw SPS NAL** the `avcC` actually carries
//! (`record.sps[0]`), which is unaffected by that gap.

use broadcast_common::{Package, Unpackage};
use transmux::TsDemux;
use transmux::media::{CmafMux, Media, Track};
use transmux::pipeline::CodecConfig;
use transmux::validate::{Severity, validate_init_segment};

// ---------------------------------------------------------------------------
// Ground-truth table
// ---------------------------------------------------------------------------

/// One row of ffprobe-recorded ground truth per fixture.
struct Oracle {
    /// Fixture file name (within `fixtures/ts/h264/` or `fixtures/ts/hevc/`).
    file: &'static str,
    /// `profile_idc` (H.264) / `general_profile_idc` (HEVC).
    profile: u8,
    /// `level_idc` (H.264) / `general_level_idc` (HEVC).
    level: u8,
    width: u32,
    height: u32,
    bit_depth_luma: u8,
    bit_depth_chroma: u8,
    chroma_format_idc: u8,
    /// True when the picture is field/interlaced-coded
    /// (H.264 `frame_mbs_only_flag == 0`; ffprobe `field_order` != progressive).
    interlaced: bool,
}

const H264_ORACLE: &[Oracle] = &[
    Oracle {
        file: "baseline.ts",
        profile: 66,
        level: 13,
        width: 320,
        height: 240,
        bit_depth_luma: 8,
        bit_depth_chroma: 8,
        chroma_format_idc: 1,
        interlaced: false,
    },
    Oracle {
        file: "main.ts",
        profile: 77,
        level: 13,
        width: 320,
        height: 240,
        bit_depth_luma: 8,
        bit_depth_chroma: 8,
        chroma_format_idc: 1,
        interlaced: false,
    },
    Oracle {
        file: "high.ts",
        profile: 100,
        level: 13,
        width: 320,
        height: 240,
        bit_depth_luma: 8,
        bit_depth_chroma: 8,
        chroma_format_idc: 1,
        interlaced: false,
    },
    Oracle {
        file: "high10.ts",
        profile: 110,
        level: 13,
        width: 320,
        height: 240,
        bit_depth_luma: 10,
        bit_depth_chroma: 10,
        chroma_format_idc: 1,
        interlaced: false,
    },
    Oracle {
        file: "high422.ts",
        profile: 122,
        level: 13,
        width: 320,
        height: 240,
        bit_depth_luma: 8,
        bit_depth_chroma: 8,
        chroma_format_idc: 2,
        interlaced: false,
    },
    Oracle {
        file: "high444.ts",
        profile: 244,
        level: 13,
        width: 320,
        height: 240,
        bit_depth_luma: 8,
        bit_depth_chroma: 8,
        chroma_format_idc: 3,
        interlaced: false,
    },
    Oracle {
        file: "high_1080_cropped.ts",
        profile: 100,
        level: 40,
        width: 1920,
        height: 1080,
        bit_depth_luma: 8,
        bit_depth_chroma: 8,
        chroma_format_idc: 1,
        interlaced: false,
    },
    Oracle {
        file: "interlaced.ts",
        profile: 100,
        level: 30,
        width: 720,
        height: 576,
        bit_depth_luma: 8,
        bit_depth_chroma: 8,
        chroma_format_idc: 1,
        interlaced: true,
    },
];

const HEVC_ORACLE: &[Oracle] = &[
    Oracle {
        file: "main.ts",
        profile: 1,
        // ffprobe prints the raw general_level_idc (ITU-T H.265 §A.3 Table
        // A.1 lists levels in multiples of 30, but ffprobe's `level` field is
        // the wire value verbatim, not `level_number × 10`): 60 = Level 2.0.
        level: 60,
        width: 320,
        height: 240,
        bit_depth_luma: 8,
        bit_depth_chroma: 8,
        chroma_format_idc: 1,
        interlaced: false,
    },
    Oracle {
        file: "main10.ts",
        profile: 2,
        level: 60,
        width: 320,
        height: 240,
        bit_depth_luma: 10,
        bit_depth_chroma: 10,
        chroma_format_idc: 1,
        interlaced: false,
    },
];

// ---------------------------------------------------------------------------
// Fixture / demux helpers
// ---------------------------------------------------------------------------

fn h264_fixture_path(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../fixtures/ts/h264")
        .join(name)
}

fn hevc_fixture_path(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../fixtures/ts/hevc")
        .join(name)
}

fn load_ts(path: &std::path::Path) -> Vec<u8> {
    let data = std::fs::read(path).unwrap_or_else(|_| panic!("{path:?} fixture must exist"));
    assert_eq!(
        data.len() % 188,
        0,
        "{path:?}: TS file must be whole 188-byte packets"
    );
    data
}

fn demux(path: &std::path::Path) -> Media {
    let ts = load_ts(path);
    TsDemux::new()
        .unpackage(&ts)
        .unwrap_or_else(|e| panic!("{path:?}: demux must succeed: {e:?}"))
}

/// The single AVC video track from a demuxed `Media`.
fn avc_track(media: &Media) -> &Track {
    let tracks: Vec<&Track> = media
        .tracks
        .iter()
        .filter(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .collect();
    assert_eq!(tracks.len(), 1, "must demux exactly one AVC video track");
    tracks[0]
}

/// The single HEVC video track from a demuxed `Media`.
fn hevc_track(media: &Media) -> &Track {
    let tracks: Vec<&Track> = media
        .tracks
        .iter()
        .filter(|t| matches!(t.spec.config, CodecConfig::Hevc { .. }))
        .collect();
    assert_eq!(tracks.len(), 1, "must demux exactly one HEVC video track");
    tracks[0]
}

/// HEVC NAL unit type from the 2-byte header: `(byte0 >> 1) & 0x3F`.
const H265_NAL_TYPE_SPS: u8 = 33;

fn hevc_nal_type(nal: &[u8]) -> Option<u8> {
    nal.first().map(|b| (b >> 1) & 0x3F)
}

// ---------------------------------------------------------------------------
// Test 1: H.264 table-driven SPS decode across the full profile matrix
// ---------------------------------------------------------------------------

#[test]
fn h264_matrix_decode_matches_ffprobe_oracle() {
    for oracle in H264_ORACLE {
        let media = demux(&h264_fixture_path(oracle.file));
        let track = avc_track(&media);

        let (config, width, height) = match &track.spec.config {
            CodecConfig::Avc {
                config,
                width,
                height,
            } => (config, *width as u32, *height as u32),
            other => panic!("{}: expected AVC config, got {other:?}", oracle.file),
        };

        // Dimensions come from the demuxed CodecConfig — must match ffprobe.
        assert_eq!(
            (width, height),
            (oracle.width, oracle.height),
            "{}: coded dimensions must match ffprobe",
            oracle.file
        );

        // Profile/level bytes are copied verbatim from the SPS into the avcC
        // header (ISO/IEC 14496-15 §5.3.3.1.2) — verify both agree with ffprobe.
        assert_eq!(
            config.config.profile_indication, oracle.profile,
            "{}: avcC AVCProfileIndication must match ffprobe profile_idc",
            oracle.file
        );
        assert_eq!(
            config.config.level_indication, oracle.level,
            "{}: avcC AVCLevelIndication must match ffprobe level",
            oracle.file
        );

        // Decode the raw SPS NAL the avcC actually carries — this is what a
        // real decoder parses for chroma/bit-depth/interlace (the avcC
        // extension bytes are redundant with, and in this codebase currently
        // not populated from, this same SPS for AVC — see the module doc).
        let sps = config
            .config
            .sps
            .first()
            .unwrap_or_else(|| panic!("{}: avcC must carry an SPS", oracle.file));
        let info = sps
            .decode()
            .unwrap_or_else(|e| panic!("{}: SPS decode must succeed: {e:?}", oracle.file));

        assert_eq!(
            info.profile_idc, oracle.profile,
            "{}: decoded profile_idc",
            oracle.file
        );
        assert_eq!(
            info.level_idc, oracle.level,
            "{}: decoded level_idc",
            oracle.file
        );
        assert_eq!(
            info.chroma_format_idc, oracle.chroma_format_idc,
            "{}: decoded chroma_format_idc",
            oracle.file
        );
        assert_eq!(
            info.bit_depth_luma, oracle.bit_depth_luma,
            "{}: decoded bit_depth_luma",
            oracle.file
        );
        assert_eq!(
            info.bit_depth_chroma, oracle.bit_depth_chroma,
            "{}: decoded bit_depth_chroma",
            oracle.file
        );
        assert_eq!(
            info.width, oracle.width,
            "{}: SPS-decoded width",
            oracle.file
        );
        assert_eq!(
            info.height, oracle.height,
            "{}: SPS-decoded height",
            oracle.file
        );
        assert_eq!(
            !info.frame_mbs_only, oracle.interlaced,
            "{}: frame_mbs_only_flag must invert to ffprobe's interlaced/progressive field_order",
            oracle.file
        );

        // VUI fps (#523/#546): every fixture in the matrix is 25 fps.
        let fps = info
            .fps
            .unwrap_or_else(|| panic!("{}: VUI fps must be present", oracle.file));
        assert!(
            (fps - 25.0_f32).abs() < 0.01,
            "{}: VUI fps must be 25.0, got {fps}",
            oracle.file
        );
    }
}

// ---------------------------------------------------------------------------
// Test 2: HEVC table-driven SPS decode (main / main10)
// ---------------------------------------------------------------------------

#[test]
fn hevc_matrix_decode_matches_ffprobe_oracle() {
    for oracle in HEVC_ORACLE {
        let media = demux(&hevc_fixture_path(oracle.file));
        let track = hevc_track(&media);

        let (config, width, height) = match &track.spec.config {
            CodecConfig::Hevc {
                config,
                width,
                height,
            } => (config, *width as u32, *height as u32),
            other => panic!("{}: expected HEVC config, got {other:?}", oracle.file),
        };

        assert_eq!(
            (width, height),
            (oracle.width, oracle.height),
            "{}: coded dimensions must match ffprobe",
            oracle.file
        );

        // Unlike AVC, HEVC's hvcC IS fully populated from decode_hevc_sps by
        // ts_demux.rs — verify the record's own fields directly.
        let record = &config.config;
        assert_eq!(
            record.general_profile_idc, oracle.profile,
            "{}: hvcC general_profile_idc",
            oracle.file
        );
        assert_eq!(
            record.general_level_idc, oracle.level,
            "{}: hvcC general_level_idc",
            oracle.file
        );
        assert_eq!(
            record.chroma_format_idc, oracle.chroma_format_idc,
            "{}: hvcC chroma_format_idc",
            oracle.file
        );
        assert_eq!(
            record.bit_depth_luma_minus8 + 8,
            oracle.bit_depth_luma,
            "{}: hvcC bit_depth_luma",
            oracle.file
        );
        assert_eq!(
            record.bit_depth_chroma_minus8 + 8,
            oracle.bit_depth_chroma,
            "{}: hvcC bit_depth_chroma",
            oracle.file
        );

        // Cross-check by decoding the raw SPS NAL the hvcC carries, same as
        // the AVC test does — must independently agree.
        let sps_nal = record
            .arrays
            .iter()
            .find(|a| a.nal_unit_type == H265_NAL_TYPE_SPS)
            .and_then(|a| a.nalus.first())
            .unwrap_or_else(|| panic!("{}: hvcC must carry an SPS array", oracle.file));
        assert_eq!(
            hevc_nal_type(&sps_nal.0),
            Some(H265_NAL_TYPE_SPS),
            "{}: SPS array NAL header must be type 33",
            oracle.file
        );
        let info = sps_nal
            .decode_sps()
            .unwrap_or_else(|e| panic!("{}: HEVC SPS decode must succeed: {e:?}", oracle.file))
            .unwrap_or_else(|| panic!("{}: NAL must decode as an SPS", oracle.file));

        assert_eq!(
            info.general_profile_idc, oracle.profile,
            "{}: SPS-decoded general_profile_idc",
            oracle.file
        );
        assert_eq!(
            info.general_level_idc, oracle.level,
            "{}: SPS-decoded general_level_idc",
            oracle.file
        );
        assert_eq!(
            info.chroma_format_idc, oracle.chroma_format_idc,
            "{}: SPS-decoded chroma_format_idc",
            oracle.file
        );
        assert_eq!(
            info.bit_depth_luma, oracle.bit_depth_luma,
            "{}: SPS-decoded bit_depth_luma",
            oracle.file
        );
        assert_eq!(
            info.bit_depth_chroma, oracle.bit_depth_chroma,
            "{}: SPS-decoded bit_depth_chroma",
            oracle.file
        );
        assert_eq!(
            info.width, oracle.width,
            "{}: SPS-decoded width",
            oracle.file
        );
        assert_eq!(
            info.height, oracle.height,
            "{}: SPS-decoded height",
            oracle.file
        );

        // VUI fps (#523/#546): both HEVC fixtures are 25 fps.
        let fps = info
            .fps
            .unwrap_or_else(|| panic!("{}: VUI fps must be present", oracle.file));
        assert!(
            (fps - 25.0_f32).abs() < 0.01,
            "{}: VUI fps must be 25.0, got {fps}",
            oracle.file
        );
    }
}

// ---------------------------------------------------------------------------
// Test 3: end-to-end TS → IR → fMP4 init segment, per fixture
// ---------------------------------------------------------------------------

/// Mux each H.264 fixture's demuxed `Media` to a CMAF init segment and verify
/// it validates clean, carries an `avc1`/`avcC` sample entry, and that the
/// `avcC` re-parses with the same profile/level/dimensions as the source.
#[test]
fn h264_matrix_ts_to_cmaf_init_segment() {
    for oracle in H264_ORACLE {
        let media = demux(&h264_fixture_path(oracle.file));

        let cmaf = CmafMux::default()
            .package(&media)
            .unwrap_or_else(|e| panic!("{}: package to CMAF: {e:?}", oracle.file));

        let init_errors: Vec<_> = validate_init_segment(&cmaf)
            .into_iter()
            .filter(|i| i.severity == Severity::Error)
            .collect();
        assert!(
            init_errors.is_empty(),
            "{}: init segment must have zero errors, got {init_errors:?}",
            oracle.file
        );

        assert!(
            contains_ascii(&cmaf, b"avcC"),
            "{}: CMAF init must carry an avcC config box",
            oracle.file
        );

        // Round-trip through our own Fmp4Demux: dims/profile/level survive
        // TS → IR → fMP4 → IR unchanged.
        let round: Media = transmux::Fmp4Demux::new()
            .unpackage(&cmaf)
            .unwrap_or_else(|e| panic!("{}: re-parse our CMAF: {e:?}", oracle.file));
        let round_track = avc_track(&round);
        match &round_track.spec.config {
            CodecConfig::Avc {
                config,
                width,
                height,
            } => {
                assert_eq!(
                    (*width as u32, *height as u32),
                    (oracle.width, oracle.height),
                    "{}: round-trip dims preserved",
                    oracle.file
                );
                assert_eq!(
                    config.config.profile_indication, oracle.profile,
                    "{}: round-trip avcC profile preserved",
                    oracle.file
                );
                assert_eq!(
                    config.config.level_indication, oracle.level,
                    "{}: round-trip avcC level preserved",
                    oracle.file
                );
                let info = config.config.sps[0]
                    .decode()
                    .expect("round-trip SPS must decode");
                assert_eq!(
                    info.chroma_format_idc, oracle.chroma_format_idc,
                    "{}: round-trip chroma_format_idc preserved",
                    oracle.file
                );
                assert_eq!(
                    info.bit_depth_luma, oracle.bit_depth_luma,
                    "{}: round-trip bit_depth_luma preserved",
                    oracle.file
                );
            }
            other => panic!("{}: round-trip must be AVC, got {other:?}", oracle.file),
        }
    }
}

/// Same as above for the HEVC matrix (main / main10): CMAF init validates
/// clean, carries `hvc1`/`hvcC`, and round-trips profile/level/chroma/bit-depth.
#[test]
fn hevc_matrix_ts_to_cmaf_init_segment() {
    for oracle in HEVC_ORACLE {
        let media = demux(&hevc_fixture_path(oracle.file));

        let cmaf = CmafMux::default()
            .package(&media)
            .unwrap_or_else(|e| panic!("{}: package to CMAF: {e:?}", oracle.file));

        let init_errors: Vec<_> = validate_init_segment(&cmaf)
            .into_iter()
            .filter(|i| i.severity == Severity::Error)
            .collect();
        assert!(
            init_errors.is_empty(),
            "{}: init segment must have zero errors, got {init_errors:?}",
            oracle.file
        );

        assert!(
            contains_ascii(&cmaf, b"hvcC"),
            "{}: CMAF init must carry an hvcC config box",
            oracle.file
        );

        let round: Media = transmux::Fmp4Demux::new()
            .unpackage(&cmaf)
            .unwrap_or_else(|e| panic!("{}: re-parse our CMAF: {e:?}", oracle.file));
        let round_track = hevc_track(&round);
        match &round_track.spec.config {
            CodecConfig::Hevc {
                config,
                width,
                height,
            } => {
                assert_eq!(
                    (*width as u32, *height as u32),
                    (oracle.width, oracle.height),
                    "{}: round-trip dims preserved",
                    oracle.file
                );
                assert_eq!(
                    config.config.general_profile_idc, oracle.profile,
                    "{}: round-trip hvcC profile preserved",
                    oracle.file
                );
                assert_eq!(
                    config.config.general_level_idc, oracle.level,
                    "{}: round-trip hvcC level preserved",
                    oracle.file
                );
                assert_eq!(
                    config.config.chroma_format_idc, oracle.chroma_format_idc,
                    "{}: round-trip hvcC chroma_format_idc preserved",
                    oracle.file
                );
                assert_eq!(
                    config.config.bit_depth_luma_minus8 + 8,
                    oracle.bit_depth_luma,
                    "{}: round-trip hvcC bit_depth_luma preserved",
                    oracle.file
                );
            }
            other => panic!("{}: round-trip must be HEVC, got {other:?}", oracle.file),
        }
    }
}

/// Whether `haystack` contains the 4-byte ASCII tag `needle`.
fn contains_ascii(haystack: &[u8], needle: &[u8; 4]) -> bool {
    haystack.windows(4).any(|w| w == needle)
}
