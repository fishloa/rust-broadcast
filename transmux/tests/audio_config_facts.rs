//! Audio config facts on `CodecConfig` (issue #1162 R2): channels including
//! LFE, sample rate, nominal bitrate.
//!
//! Expected values are what an independent tool reports for the very same
//! committed files (`ffprobe` 8.x: `-show_entries stream=sample_rate,channels,
//! bit_rate`), cross-checked against the spec tables the accessors cite (ETSI
//! TS 102 366 Table F.4.1 / §F.6.2.2; ISO/IEC 11172-3 bitrate table):
//!
//! | fixture | ffprobe |
//! |---|---|
//! | Sintel AC-3 5.1 (`sintel-ac3-eac3-aligned.ts`) | ac3, 48000 Hz, 6 ch, 448000 b/s |
//! | Sintel E-AC-3 5.1 (same file) | eac3, 48000 Hz, 6 ch, 256000 b/s |
//! | `aac-5_1-640k.ts` | aac, 48000 Hz, 6 ch, bit_rate N/A in the container |
//! | `h264_aac.ts` | aac, 44100 Hz, 1 ch |
//! | `legacy/mpeg2_mp2.ts` | mp2, 44100 Hz, 1 ch, 384000 b/s |
//! | `dts/dts_core.ts` | dts, 48000 Hz, 2 ch, 768000 b/s |
//! | `dolby/ac3.ts` | ac3, 44100 Hz, 1 ch, 192000 b/s |

use broadcast_common::Unpackage;
use transmux::{CodecConfig, Media, TsDemux};

fn demux(rel: &str) -> Media {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    TsDemux::new().unpackage(&bytes).expect("demux fixture")
}

/// The first audio track whose config matches `pick`.
fn track<'a>(m: &'a Media, pick: impl Fn(&CodecConfig) -> bool) -> &'a CodecConfig {
    &m.tracks
        .iter()
        .find(|t| pick(&t.spec.config))
        .expect("track present")
        .spec
        .config
}

const SINTEL: &str = "tests/fixtures/audio_pes_straddle/sintel-ac3-eac3-aligned.ts";

#[test]
fn ac3_5_1_counts_lfe_and_keeps_full_bandwidth_count() {
    let m = demux(SINTEL);
    let c = track(&m, |c| matches!(c, CodecConfig::Ac3 { .. }));
    let CodecConfig::Ac3 { channel_count, .. } = c else {
        unreachable!()
    };
    // `channel_count` stays full-bandwidth (3/2 = 5); the LFE is added on top.
    assert_eq!(*channel_count, 5);
    assert_eq!(c.channels_with_lfe(), Some(6));
    assert_eq!(c.audio_sample_rate(), Some(48_000));
    assert_eq!(c.nominal_bitrate(), Some(448_000));
}

#[test]
fn eac3_5_1() {
    let m = demux(SINTEL);
    let c = track(&m, |c| matches!(c, CodecConfig::Eac3 { .. }));
    assert_eq!(c.channels_with_lfe(), Some(6));
    assert_eq!(c.audio_sample_rate(), Some(48_000));
    assert_eq!(c.nominal_bitrate(), Some(256_000));
}

#[test]
fn aac_5_1_has_rate_and_channels_but_no_nominal_bitrate() {
    let m = demux("../fixtures/ts/aac-5_1-640k.ts");
    let c = track(&m, |c| matches!(c, CodecConfig::Aac { .. }));
    // ISO/IEC 14496-3 Table 1.19: channelConfiguration 6 = 5.1 = 6 channels.
    assert_eq!(c.channels_with_lfe(), Some(6));
    assert_eq!(c.audio_sample_rate(), Some(48_000));
    // ADTS/ASC state no bit rate (ffprobe: N/A) -> honestly None.
    assert_eq!(c.nominal_bitrate(), None);
}

#[test]
fn aac_mono() {
    let m = demux("../fixtures/ts/h264_aac.ts");
    let c = track(&m, |c| matches!(c, CodecConfig::Aac { .. }));
    assert_eq!(c.channels_with_lfe(), Some(1));
    assert_eq!(c.audio_sample_rate(), Some(44_100));
}

#[test]
fn mpeg_audio_layer2_bitrate_from_header() {
    let m = demux("../fixtures/ts/legacy/mpeg2_mp2.ts");
    let c = track(&m, |c| matches!(c, CodecConfig::MpegAudio { .. }));
    assert_eq!(c.channels_with_lfe(), Some(1));
    assert_eq!(c.audio_sample_rate(), Some(44_100));
    assert_eq!(c.nominal_bitrate(), Some(384_000));
}

#[test]
fn dts_core() {
    let m = demux("../fixtures/ts/dts/dts_core.ts");
    let c = track(&m, |c| matches!(c, CodecConfig::Dts { .. }));
    assert_eq!(c.channels_with_lfe(), Some(2));
    assert_eq!(c.audio_sample_rate(), Some(48_000));
    assert_eq!(c.nominal_bitrate(), Some(768_000));
}

#[test]
fn ac3_mono_fixture() {
    let m = demux("../fixtures/ts/dolby/ac3.ts");
    let c = track(&m, |c| matches!(c, CodecConfig::Ac3 { .. }));
    assert_eq!(c.channels_with_lfe(), Some(1));
    assert_eq!(c.audio_sample_rate(), Some(44_100));
    assert_eq!(c.nominal_bitrate(), Some(192_000));
}

#[test]
fn video_track_has_no_audio_facts() {
    let m = demux("../fixtures/ts/h264_aac.ts");
    let v = track(&m, |c| matches!(c, CodecConfig::Avc { .. }));
    assert_eq!(v.audio_sample_rate(), None);
    assert_eq!(v.channels_with_lfe(), None);
    assert_eq!(v.nominal_bitrate(), None);
}
