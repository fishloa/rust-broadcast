//! Microsoft Smooth Streaming ([MS-SSTR]) output gate (issue #473) + the
//! client-manifest parser / codec glue that inverts it (issue #759, T1).
//!
//! Input IR: `TsDemux`-of-`fixtures/ts/h264_aac.ts` (75 video + 131 audio
//! samples). Each test bites by parsing the produced outputs (manifest XML +
//! fragment boxes) and asserting against what the crate itself demuxes — never
//! a bare substring or a hardcoded offset. The manifest XML is walked with the
//! crate's own [`transmux::smooth_parse::SmoothManifest::parse`] — not a
//! second hand-rolled test parser — so these tests double as the parser's
//! strongest bite: round-tripping the real writer's output.

use std::path::PathBuf;

use broadcast_common::{Package, Parse, Unpackage};
use transmux::aac_asc::AudioSpecificConfig;
use transmux::box_types::parse_box;
use transmux::media::{Fmp4Demux, Media};
use transmux::pipeline::{CodecConfig, build_init_segment};
use transmux::smooth::{FOURCC_AACL, FOURCC_H264, SmoothOutput, SmoothPackager, TFXD_UUID};
use transmux::smooth_parse::{SmoothManifest, StreamType, track_spec_from_quality_level};
use transmux::ts_demux::TsDemux;

// ---------------------------------------------------------------------------
// Fixtures + packaging
// ---------------------------------------------------------------------------

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures")
}

fn demux_media() -> Media {
    let ts = std::fs::read(fixtures_dir().join("ts/h264_aac.ts"))
        .expect("h264_aac.ts fixture must exist");
    let mut demux = TsDemux::new();
    demux.unpackage(&ts[..]).expect("demux h264_aac.ts")
}

fn build_smooth() -> (Media, SmoothOutput) {
    let media = demux_media();
    let mut pkg = SmoothPackager::default();
    let out = pkg.package(&media).expect("package Smooth");
    (media, out)
}

// ---------------------------------------------------------------------------
// Oracle helpers — pull expected values from the demuxed IR.
// ---------------------------------------------------------------------------

fn video_track(media: &Media) -> &transmux::media::Track {
    media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("video track")
}

fn audio_track(media: &Media) -> &transmux::media::Track {
    media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Aac { .. }))
        .expect("audio track")
}

/// Demuxed SPS + ASC bytes for the oracle checks.
fn demuxed_sps(media: &Media) -> Vec<u8> {
    match &video_track(media).spec.config {
        CodecConfig::Avc { config, .. } => config.config.sps.first().expect("SPS").0.clone(),
        _ => unreachable!(),
    }
}

fn demuxed_asc(media: &Media) -> Vec<u8> {
    match &audio_track(media).spec.config {
        CodecConfig::Aac { esds, .. } => esds
            .es_descriptor
            .decoder_config
            .as_ref()
            .and_then(|dc| dc.decoder_specific_info.as_ref())
            .expect("ASC")
            .data
            .clone(),
        _ => unreachable!(),
    }
}

// ---------------------------------------------------------------------------
// Test 1 — Manifest shape (parsed via the real client-manifest parser).
// ---------------------------------------------------------------------------

#[test]
fn manifest_shape() {
    let (_media, out) = build_smooth();
    let manifest = SmoothManifest::parse(&out.manifest).expect("parse manifest");

    assert_eq!(manifest.major_version, 2, "MajorVersion must be 2");
    assert!(manifest.duration.is_some(), "must carry a Duration");
    assert_eq!(
        manifest.streams.len(),
        2,
        "exactly two StreamIndex (video + audio)"
    );

    let has_video = manifest
        .streams
        .iter()
        .any(|s| s.stream_type == StreamType::Video);
    let has_audio = manifest
        .streams
        .iter()
        .any(|s| s.stream_type == StreamType::Audio);
    assert!(has_video, "a video StreamIndex");
    assert!(has_audio, "an audio StreamIndex");

    for si in &manifest.streams {
        assert_eq!(
            si.qualities.len(),
            1,
            "each StreamIndex has exactly one QualityLevel"
        );
    }
}

// ---------------------------------------------------------------------------
// Test 2 — Codec signalling correct (parsed model vs the packager's INPUTS).
// ---------------------------------------------------------------------------

#[test]
fn codec_signalling_correct() {
    let (media, out) = build_smooth();
    let manifest = SmoothManifest::parse(&out.manifest).expect("parse manifest");

    let video_si = manifest
        .streams
        .iter()
        .find(|s| s.stream_type == StreamType::Video)
        .expect("video StreamIndex");
    let audio_si = manifest
        .streams
        .iter()
        .find(|s| s.stream_type == StreamType::Audio)
        .expect("audio StreamIndex");

    let video_ql = &video_si.qualities[0];
    let audio_ql = &audio_si.qualities[0];

    // Video: FourCC H264 + CodecPrivateData (already hex-decoded by the
    // parser) = start-code SPS+PPS whose SPS matches the demuxed track, and
    // whose bitrate matches what the packager computed.
    assert_eq!(video_ql.four_cc, FOURCC_H264);
    assert!(video_ql.bitrate > 0, "video bitrate must be positive");
    let cpd = &video_ql.codec_private_data;
    assert_eq!(
        &cpd[0..4],
        &[0x00, 0x00, 0x00, 0x01],
        "SPS start code prefix"
    );
    let sps = demuxed_sps(&media);
    assert_eq!(
        &cpd[4..4 + sps.len()],
        &sps[..],
        "CodecPrivateData SPS must equal the demuxed SPS bytes"
    );
    let rest = &cpd[4 + sps.len()..];
    assert_eq!(
        &rest[0..4],
        &[0x00, 0x00, 0x00, 0x01],
        "a PPS start code must follow the SPS"
    );

    // Audio: FourCC AACL + CodecPrivateData == demuxed ASC bytes, bitrate positive.
    assert_eq!(audio_ql.four_cc, FOURCC_AACL);
    assert!(audio_ql.bitrate > 0, "audio bitrate must be positive");
    let want_asc = demuxed_asc(&media);
    assert_eq!(
        audio_ql.codec_private_data, want_asc,
        "audio CodecPrivateData must equal the demuxed ASC bytes"
    );

    // SamplingRate/Channels must match the decoded ASC.
    let asc = AudioSpecificConfig::parse(&want_asc).expect("parse ASC");
    let want_rate = asc.sampling_frequency.unwrap_or_else(|| {
        const RATES: [u32; 13] = [
            96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350,
        ];
        RATES[asc.sampling_frequency_index.raw() as usize]
    });
    assert_eq!(
        audio_ql.sampling_rate,
        Some(want_rate),
        "SamplingRate must match ASC"
    );
    assert_eq!(
        audio_ql.channels,
        Some(asc.channel_configuration.raw() as u16),
        "Channels must match ASC channel configuration"
    );
}

// ---------------------------------------------------------------------------
// Test 3 — Fragment `c` timeline (parsed + expanded via `enumerate_chunks`).
// ---------------------------------------------------------------------------

#[test]
fn fragment_c_timeline() {
    let (media, out) = build_smooth();
    let manifest = SmoothManifest::parse(&out.manifest).expect("parse manifest");

    for si in &manifest.streams {
        let track = match si.stream_type {
            StreamType::Video => video_track(&media),
            StreamType::Audio => audio_track(&media),
            StreamType::Text => panic!("unexpected text stream"),
            _ => panic!("unexpected stream type"),
        };

        // Number of `c` == number of emitted fragments for this track.
        let emitted = out
            .fragments
            .iter()
            .filter(|f| f.track_id == track.spec.track_id)
            .count();
        assert_eq!(
            si.chunks_list.len(),
            emitted,
            "`c` count must equal emitted fragments for {}",
            si.stream_type
        );
        assert!(emitted > 0, "at least one fragment for {}", si.stream_type);

        // Expand the timeline (also exercises the bounded `r`-run expansion
        // path against real, non-adversarial input) and check the sum of
        // durations against the track total duration in TimeScale ticks.
        let expanded = si.enumerate_chunks().expect("enumerate_chunks");
        assert_eq!(expanded.len(), emitted);
        let sum_d: u64 = expanded.iter().map(|(_, d)| *d).sum();
        let media_ticks: u64 = track
            .samples
            .iter()
            .map(|s| s.duration.unwrap_or(0) as u64)
            .sum();
        let ts = track.spec.timescale.max(1) as u64;
        let expected = (media_ticks * 10_000_000 + ts / 2) / ts;
        assert_eq!(
            sum_d, expected,
            "sum of c@d must equal the track total duration ({})",
            si.stream_type
        );

        // Only the first `c` carries an explicit @t; every entry has a `d`.
        assert!(
            si.chunks_list[0].t.is_some(),
            "first c must carry an explicit @t"
        );
        for c in &si.chunks_list {
            assert!(c.d.is_some(), "every c must carry @d");
        }
    }
}

// ---------------------------------------------------------------------------
// Test 4 — Fragment box structure + tfxd.
// ---------------------------------------------------------------------------

/// Find the tfxd uuid box inside a moof, returning (AbsoluteTime, Duration).
fn find_tfxd(fragment: &[u8]) -> (u64, u64) {
    // Walk top-level boxes to the moof.
    let mut off = 0usize;
    while off + 8 <= fragment.len() {
        let size = u32::from_be_bytes([
            fragment[off],
            fragment[off + 1],
            fragment[off + 2],
            fragment[off + 3],
        ]) as usize;
        let ty = &fragment[off + 4..off + 8];
        if ty == b"moof" {
            // Search for a uuid box with the tfxd usertype within the moof.
            let moof = &fragment[off..off + size];
            let mut i = 8usize;
            // Descend: mfhd, traf(...). We scan for "uuid" fourcc + TFXD usertype.
            while i + 8 <= moof.len() {
                let bsz =
                    u32::from_be_bytes([moof[i], moof[i + 1], moof[i + 2], moof[i + 3]]) as usize;
                let bty = &moof[i + 4..i + 8];
                if bty == b"uuid" && i + 8 + 16 <= moof.len() && moof[i + 8..i + 24] == TFXD_UUID {
                    // body after box header(8) + usertype(16): version/flags(4) then two u64s.
                    let p = &moof[i + 24 + 4..];
                    let at = u64::from_be_bytes([p[0], p[1], p[2], p[3], p[4], p[5], p[6], p[7]]);
                    let du =
                        u64::from_be_bytes([p[8], p[9], p[10], p[11], p[12], p[13], p[14], p[15]]);
                    return (at, du);
                }
                // Descend into traf container to keep scanning children.
                if bty == b"traf" {
                    i += 8;
                    continue;
                }
                if bsz == 0 {
                    break;
                }
                i += bsz;
            }
            panic!("tfxd uuid not found in moof");
        }
        if size == 0 {
            break;
        }
        off += size;
    }
    panic!("moof not found");
}

#[test]
fn fragment_box_structure_and_tfxd() {
    let (media, out) = build_smooth();
    let vid_id = video_track(&media).spec.track_id;
    let video_frags: Vec<_> = out
        .fragments
        .iter()
        .filter(|f| f.track_id == vid_id)
        .collect();
    assert!(!video_frags.is_empty(), "video fragments present");

    let mut last_seq = 0u32;
    for (idx, frag) in video_frags.iter().enumerate() {
        // Parses as moof + mdat.
        let (bx0, c0) = parse_box(&frag.data).expect("parse first box");
        // First box is styp; then moof, then mdat.
        assert_eq!(&bx0.header.box_type.0, b"styp");
        let (bx1, c1) = parse_box(&frag.data[c0..]).expect("parse second box");
        assert_eq!(&bx1.header.box_type.0, b"moof", "second box is moof");
        let (bx2, _c2) = parse_box(&frag.data[c0 + c1..]).expect("parse third box");
        assert_eq!(&bx2.header.box_type.0, b"mdat", "third box is mdat");

        // tfxd present with the right UUID + AbsoluteTime == fragment start.
        let (at, du) = find_tfxd(&frag.data);
        assert_eq!(
            at, frag.start_time,
            "tfxd AbsoluteTime must equal the fragment start (frag {idx})"
        );
        assert_eq!(
            du, frag.duration,
            "tfxd Duration must equal the fragment duration"
        );

        // Sequence numbers increase.
        assert!(
            frag.sequence_number > last_seq,
            "sequence numbers must strictly increase"
        );
        last_seq = frag.sequence_number;
    }
}

// ---------------------------------------------------------------------------
// Test 5 — Lossless round-trip (Smooth fragmentation is lossless), driven off
// the demuxed track spec directly (no init-segment synthesis needed here —
// see the parsed-manifest variant below for that).
// ---------------------------------------------------------------------------

#[test]
fn lossless_round_trip_video() {
    let (media, out) = build_smooth();
    let vid = video_track(&media);
    let vid_id = vid.spec.track_id;

    // Build a fragmented-MP4 file: the CMAF init segment + every video
    // fragment's moof+mdat concatenated (drop the per-fragment styp so
    // Fmp4Demux sees a clean moov + moof/mdat stream).
    let specs = vec![vid.spec.clone()];
    let mut file = build_init_segment(&specs, media.movie_timescale).expect("init segment");

    for frag in out.fragments.iter().filter(|f| f.track_id == vid_id) {
        // Strip the leading styp; append moof + mdat.
        let (styp, sc) = parse_box(&frag.data).unwrap();
        assert_eq!(&styp.header.box_type.0, b"styp");
        file.extend_from_slice(&frag.data[sc..]);
    }

    let media2 = Fmp4Demux::new().unpackage(&file[..]).expect("re-demux");
    let vid2 = media2
        .tracks
        .iter()
        .find(|t| t.spec.track_id == vid_id)
        .expect("video track in re-demux");

    assert_eq!(
        vid2.samples.len(),
        vid.samples.len(),
        "sample count preserved (75 video samples)"
    );
    assert_eq!(vid.samples.len(), 75, "fixture has 75 video samples");

    for (i, (a, b)) in vid.samples.iter().zip(vid2.samples.iter()).enumerate() {
        assert_eq!(
            a.data, b.data,
            "coded NAL payload byte-identical at sample {i}"
        );
        assert_eq!(a.duration, b.duration, "duration preserved at sample {i}");
        assert_eq!(
            a.flags.is_sync, b.flags.is_sync,
            "sync flag preserved at sample {i}"
        );
    }
}

// ---------------------------------------------------------------------------
// Test 6 — Codec glue: parsed CodecPrivateData → synthesized init segment
// that Fmp4Demux accepts (the Smooth-pull puller's exact need, issue #759 T1).
// No init segment ever comes over the wire in Smooth — this proves the
// manifest-only codec config is enough to reconstruct one.
// ---------------------------------------------------------------------------

#[test]
fn parsed_manifest_codec_glue_builds_working_init_segment() {
    let (media, out) = build_smooth();
    let manifest = SmoothManifest::parse(&out.manifest).expect("parse manifest");

    let vid = video_track(&media);
    let aud = audio_track(&media);

    let video_si = manifest
        .streams
        .iter()
        .find(|s| s.stream_type == StreamType::Video)
        .expect("video StreamIndex");
    let audio_si = manifest
        .streams
        .iter()
        .find(|s| s.stream_type == StreamType::Audio)
        .expect("audio StreamIndex");

    let video_spec = track_spec_from_quality_level(
        vid.spec.track_id,
        vid.spec.timescale,
        StreamType::Video,
        &video_si.qualities[0],
    )
    .expect("synthesize video TrackSpec from CodecPrivateData");
    let audio_spec = track_spec_from_quality_level(
        aud.spec.track_id,
        aud.spec.timescale,
        StreamType::Audio,
        &audio_si.qualities[0],
    )
    .expect("synthesize audio TrackSpec from CodecPrivateData");

    // The synthesized config must match the TS source's actual codec/geometry.
    match &video_spec.config {
        CodecConfig::Avc {
            config,
            width,
            height,
        } => {
            let sps = demuxed_sps(&media);
            assert_eq!(config.config.sps.first().unwrap().0, sps);
            let info = transmux::sps::decode_avc_sps(&sps).expect("decode real SPS");
            assert_eq!(*width, info.width as u16, "width must match the TS source");
            assert_eq!(
                *height, info.height as u16,
                "height must match the TS source"
            );
        }
        _ => panic!("expected CodecConfig::Avc"),
    }
    match &audio_spec.config {
        CodecConfig::Aac {
            sample_rate,
            channel_count,
            ..
        } => {
            let want_asc = demuxed_asc(&media);
            let asc = AudioSpecificConfig::parse(&want_asc).unwrap();
            let want_rate = asc.sampling_frequency.unwrap_or(44100);
            assert_eq!(
                *sample_rate, want_rate,
                "sample_rate must match the TS source"
            );
            assert_eq!(
                *channel_count,
                asc.channel_configuration.raw() as u16,
                "channel_count must match the TS source"
            );
        }
        _ => panic!("expected CodecConfig::Aac"),
    }

    // Build the init segment purely from the manifest-derived specs — no
    // Smooth init segment exists on the wire, so this IS the bootstrap.
    let specs = vec![video_spec, audio_spec];
    let mut file = build_init_segment(&specs, media.movie_timescale)
        .expect("build init segment from synthesized TrackSpecs");

    // Feed it the video fragments (styp stripped) exactly like a real puller
    // would append fetched fragment responses.
    for frag in out
        .fragments
        .iter()
        .filter(|f| f.track_id == vid.spec.track_id)
    {
        let (styp, sc) = parse_box(&frag.data).unwrap();
        assert_eq!(&styp.header.box_type.0, b"styp");
        file.extend_from_slice(&frag.data[sc..]);
    }

    let media2 = Fmp4Demux::new()
        .unpackage(&file[..])
        .expect("Fmp4Demux must accept the synthesized init segment + fragments");
    let vid2 = media2
        .tracks
        .iter()
        .find(|t| t.spec.track_id == vid.spec.track_id)
        .expect("video track in re-demux");
    assert_eq!(
        vid2.samples.len(),
        vid.samples.len(),
        "every video sample survives through the synthesized init segment"
    );
}

// ---------------------------------------------------------------------------
// Empty media is rejected.
// ---------------------------------------------------------------------------

#[test]
fn empty_media_rejected() {
    let media = Media::new(vec![], 90_000);
    let mut pkg = SmoothPackager::default();
    assert!(pkg.package(&media).is_err(), "empty Media must not package");
}
