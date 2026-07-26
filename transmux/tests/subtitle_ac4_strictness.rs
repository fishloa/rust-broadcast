//! Media plane step 2d: codec coverage (`CodecConfig::Subtitle` + the `ac-4`
//! demux arm) lands BEFORE the two silent drops become typed errors.
//!
//! **The ordering guard** (headline test): a real CMAF fixture carrying a
//! subtitle (`stpp`/TTML) track alongside an AVC track must demux with the
//! subtitle track PRESENT. This is the test that fails if the strictness
//! phase (turning `Fmp4Demux`'s "sample entry not reconstructable" skip into
//! an error) shipped BEFORE the coverage phase — reversing the order would
//! turn "CMAF-with-subtitles demuxes to its A/V tracks" into
//! "CMAF-with-subtitles fails".
//!
//! Also covers: the `ac-4` demux arm now reconstructing `CodecConfig::Ac4`
//! (full mux+demux round trip, since `build_trak` already supported `ac-4`
//! output), and the Phase-2 strictness test itself — a genuinely unsupported
//! sample entry now yields a typed, named error instead of vanishing.
//!
//! Fixtures: `fixtures/mp4/cmaf/av_subtitle_frag.mp4` (real ffmpeg-generated
//! CMAF with an AVC + `stpp` track — see `PROVENANCE.md` in that directory)
//! and `fixtures/mp4/cmaf/av_frag.mp4` (existing real AVC+AAC CMAF fixture,
//! reused here to get a real `avcC` config for the strictness test without
//! hand-fabricating one).

use std::fs;
use std::path::PathBuf;

use broadcast_common::{Package, Parse, Serialize, Unpackage};
use transmux::pipeline::{CodecConfig, SubtitleFormat, build_init_segment};
use transmux::{Ac4SpecificBox, CmafMux, Error, Fmp4Demux, Media, Sample, Track, TrackSpec};

fn fixture(name: &str) -> Vec<u8> {
    let path: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "tests",
        "fixtures",
        "mp4",
        "cmaf",
        name,
    ]
    .iter()
    .collect();
    fs::read(&path).unwrap_or_else(|e| panic!("read fixture {}: {e}", path.display()))
}

// ── The ordering guard ──────────────────────────────────────────────────────

#[test]
fn ordering_guard_subtitle_track_present_after_demux() {
    let file = fixture("av_subtitle_frag.mp4");

    let mut demux = Fmp4Demux::new();
    let media = demux
        .unpackage(&file)
        .expect("a CMAF file with AVC + stpp tracks must still demux (codec coverage first)");

    assert_eq!(media.tracks.len(), 2, "AVC video + stpp subtitle track");

    let video = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("the AVC video track must be present");
    assert!(
        !video.samples.is_empty(),
        "the video track must carry samples"
    );

    // The headline assertion: the subtitle track is PRESENT, not dropped.
    let subtitle = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Subtitle { .. }))
        .expect(
            "the stpp subtitle track must be present as CodecConfig::Subtitle \
             — this is the test that fails if strictness shipped before coverage",
        );
    assert!(
        matches!(
            subtitle.spec.config,
            CodecConfig::Subtitle {
                format: SubtitleFormat::Ttml
            }
        ),
        "stpp must map to SubtitleFormat::Ttml, got {:?}",
        subtitle.spec.config
    );
    assert!(
        !subtitle.samples.is_empty(),
        "the subtitle track must carry its (opaque TTML) samples too"
    );
}

// ── AC-4: full mux -> demux round trip ─────────────────────────────────────

#[test]
fn ac4_round_trips_through_cmaf_mux_then_demux() {
    // The same real, recorded `dac4` (ac4_dsi_v1) oracle body used by
    // `tests/codecs_new.rs::DAC4_ORACLE` (29 bytes, real Dolby AC-4 init
    // segment; source mp4 non-redistributable, issue #431).
    const DAC4_ORACLE: &[u8] = &[
        0x20, 0xa4, 0x01, 0x40, 0x00, 0x00, 0x00, 0x1f, 0xff, 0xff, 0xff, 0xe0, 0x01, 0x0f, 0xf8,
        0x80, 0x00, 0x00, 0x42, 0x00, 0x00, 0x25, 0x01, 0x00, 0x00, 0x00, 0x30, 0x08, 0x00,
    ];
    let config = Ac4SpecificBox::parse(DAC4_ORACLE).expect("parse dac4 oracle");

    let spec = TrackSpec::new(
        1,
        48_000,
        CodecConfig::Ac4 {
            config: config.clone(),
            channel_count: 2,
            sample_rate: 48_000,
            sample_size: 16,
        },
    );
    let samples = vec![
        Sample::from_raw(vec![0xAA, 0xBB, 0xCC, 0xDD], Some(0), Some(0), Some(1536)),
        Sample::from_raw(vec![0x11, 0x22, 0x33], Some(1536), Some(1536), Some(1536)),
    ];
    let media = Media::new(vec![Track::new(spec, samples.clone())], 48_000);

    // Mux: build_trak already had an Ac4 arm (only the demux arm was
    // missing), so this direction worked before this change too.
    let bytes = CmafMux::new(1)
        .package(&media)
        .expect("CmafMux must mux an Ac4 track (already supported before step 2d)");

    // Demux: this is the new arm added in step 2d.
    let mut demux = Fmp4Demux::new();
    let reparsed = demux
        .unpackage(&bytes)
        .expect("ac-4 sample entry must now demux to CodecConfig::Ac4, not error");

    assert_eq!(reparsed.tracks.len(), 1);
    let track = &reparsed.tracks[0];
    match &track.spec.config {
        CodecConfig::Ac4 {
            config: got,
            channel_count,
            sample_rate,
            sample_size,
        } => {
            assert_eq!(*channel_count, 2);
            assert_eq!(*sample_rate, 48_000);
            assert_eq!(*sample_size, 16);
            let mut body = vec![0u8; got.serialized_len()];
            got.serialize_into(&mut body).expect("serialize dac4");
            assert_eq!(
                body, DAC4_ORACLE,
                "dac4 body must survive mux -> demux byte-identical"
            );
        }
        other => panic!("expected CodecConfig::Ac4, got {other:?}"),
    }
    assert_eq!(track.samples.len(), 2, "both AC-4 samples must round-trip");
    assert_eq!(
        track.samples[0].data.as_ref(),
        &[0xAA, 0xBB, 0xCC, 0xDD][..]
    );
    assert_eq!(track.samples[1].data.as_ref(), &[0x11, 0x22, 0x33][..]);
}

// ── Phase 2 strictness: a genuinely unsupported sample entry now errors,
//    naming it, instead of being silently skipped ──────────────────────────

#[test]
fn fmp4_demux_errors_naming_a_genuinely_unsupported_sample_entry() {
    // Start from a real, previously-demuxed AVC config (av_frag.mp4) so the
    // surrounding bytes (avcC/SPS/PPS) are genuine, not hand-fabricated —
    // only the sample entry's four-CC is deliberately mutated to a value no
    // `SampleEntryVariant` matches, simulating a codec this crate has no
    // `CodecConfig` reconstruction for.
    let file = fixture("av_frag.mp4");
    let mut demux = Fmp4Demux::new();
    let media = demux.unpackage(&file).expect("demux av_frag.mp4");
    let avc_spec = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("av_frag.mp4 must have an AVC track")
        .spec
        .clone();

    let init = build_init_segment(&[avc_spec], media.movie_timescale)
        .expect("build a fresh init segment for the real AVC config");

    // Mutate the `avc1` sample-entry four-CC to an unrecognised one. The box
    // size is unchanged (a 4-byte in-place swap), so the buffer stays
    // structurally well-formed; only the codec dispatch fails.
    let mut mutated = init.clone();
    let pos = mutated
        .windows(4)
        .position(|w| w == b"avc1")
        .expect("avc1 fourcc must be present in the init segment");
    mutated[pos..pos + 4].copy_from_slice(b"zzz9");
    assert_ne!(mutated, init, "mutation must actually change the bytes");

    let mut demux2 = Fmp4Demux::new();
    let err = demux2
        .unpackage(&mutated)
        .expect_err("an unrecognised sample entry must now error, not be silently skipped");
    match err {
        Error::UnsupportedSampleEntry { fourcc } => {
            assert_eq!(fourcc, "zzz9", "error must name the offending sample entry");
        }
        other => panic!("expected UnsupportedSampleEntry, got {other:?}"),
    }
}
