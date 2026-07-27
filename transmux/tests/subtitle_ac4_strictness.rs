//! Media plane step 2d: codec coverage (`CodecConfig::Subtitle` + the `ac-4`
//! demux arm), then media plane step-2 fix wave 1 (B2/B3): DEMUX is
//! lenient-but-loud, so a sample entry this crate genuinely cannot
//! reconstruct is skipped — recorded in `Media::skipped`, never silent —
//! rather than failing the whole file.
//!
//! **The ordering guard** (headline test): a real CMAF fixture carrying a
//! subtitle (`stpp`/TTML) track alongside an AVC track must demux with the
//! subtitle track PRESENT. This is the test that fails if codec coverage for
//! `stpp`/`wvtt`/`ac-4` had never landed at all — that gap would otherwise
//! turn "CMAF-with-subtitles demuxes to its A/V tracks" into
//! "CMAF-with-subtitles drops the subtitle track silently".
//!
//! Also covers: the `ac-4` demux arm reconstructing `CodecConfig::Ac4` (full
//! mux+demux round trip, since `build_trak` already supported `ac-4`
//! output), and the lenient-but-loud skip behaviour itself — both for a
//! single-track file (can only prove "not fatal, and named") and a
//! multi-track one (proves the OTHER tracks survive too).
//!
//! Fixtures: `fixtures/mp4/cmaf/av_subtitle_frag.mp4` (real ffmpeg-generated
//! CMAF with an AVC + `stpp` track), `fixtures/mp4/cmaf/av_aac_subtitle_frag.mp4`
//! (real ffmpeg-generated CMAF with AVC + AAC + `stpp` — the multi-track skip
//! test's fixture, mutated in-test) — see `PROVENANCE.md` in that directory —
//! and `fixtures/mp4/cmaf/av_frag.mp4` (existing real AVC+AAC CMAF fixture,
//! reused here to get a real `avcC` config for the single-track skip test
//! without hand-fabricating one).

use std::fs;
use std::path::PathBuf;

use broadcast_common::{Package, Parse, Serialize, Unpackage};
use transmux::pipeline::{CodecConfig, SubtitleFormat, build_init_segment};
use transmux::{Ac4SpecificBox, CmafMux, Fmp4Demux, Media, Sample, Track, TrackSpec};

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
    // T6 (test-integrity audit): non-empty alone lets a wrong `mdat` sample
    // range (any garbage bytes of the right length) pass. Check the sample
    // bytes are actually real TTML content, not merely present: this crate
    // never decodes subtitle payloads (they stay opaque, per the crate's
    // "parses codec config headers only" scope), but the fixture's
    // subtitle sample is a real ffmpeg TTML document, so its bytes must be
    // valid UTF-8 XML carrying the `<tt>` root element (RFC (W3C) TTML1 §5.3)
    // with the TTML namespace declaration — content a wrong byte range would
    // not coincidentally reproduce.
    for (i, s) in subtitle.samples.iter().enumerate() {
        let text = std::str::from_utf8(&s.data)
            .unwrap_or_else(|e| panic!("subtitle sample {i} must be valid UTF-8 TTML: {e}"));
        assert!(
            text.contains("<tt") && text.contains("http://www.w3.org/ns/ttml"),
            "subtitle sample {i} must contain real TTML markup (the <tt> root \
             element + its namespace), not just be non-empty bytes — this is \
             what catches a wrong mdat sample range; got {text:?}"
        );
    }
}

// ── B1: a subtitle-bearing CMAF asset repackages successfully end-to-end ───

#[test]
fn subtitle_bearing_cmaf_repackages_once_filtered_with_is_muxable_in_bmff() {
    // B1's regression: `CodecConfig::Subtitle` had no opaque-data-style
    // predicate covering it, so `build_init_segment` (called by `CmafMux`
    // and every other mux entry point) didn't skip it — it fell through to
    // `build_trak`'s `CodecConfig::Subtitle` arm and errored. Before this
    // fix, `is_opaque_data()` was the only filter predicate a caller had,
    // and it does not match `Subtitle` — so there was NO way to filter a
    // subtitle-bearing source down to something `CmafMux` would accept; a
    // "repackage this real subtitle-bearing CMAF asset" pipeline always
    // failed. `is_muxable_in_bmff()` (new in this fix) covers both `Data`
    // and `Subtitle`, so filtering with it is the fix.
    let file = fixture("av_subtitle_frag.mp4");
    let mut demux = Fmp4Demux::new();
    let media = demux
        .unpackage(&file)
        .expect("demux av_subtitle_frag.mp4 (AVC + stpp)");
    assert_eq!(media.tracks.len(), 2, "AVC video + stpp subtitle track");

    // Unfiltered: CmafMux must still reject the Subtitle track by name (MUX
    // = strict), not silently drop it or panic.
    let unfiltered_err = CmafMux::default()
        .package(&media)
        .expect_err("CmafMux must reject an unfiltered Subtitle track");
    assert!(
        matches!(
            unfiltered_err,
            transmux::Error::UnmuxableSubtitleTrack { .. }
        ),
        "must fail naming the Subtitle track specifically, got {unfiltered_err:?}"
    );

    // Filtered with the new predicate: CmafMux must now succeed — this is
    // the "repackages fine" outcome B1 regressed away from.
    let carriable = media
        .select_tracks_by(|t| t.spec.config.is_muxable_in_bmff())
        .expect("the AVC track remains carriable");
    assert_eq!(carriable.tracks.len(), 1, "only the AVC track remains");
    let fmp4 = CmafMux::default()
        .package(&carriable)
        .expect("CmafMux must succeed once the Subtitle track is filtered out");
    assert!(
        fmp4.windows(4).any(|w| w == b"moov"),
        "CmafMux output must contain a moov box for the carriable AVC track"
    );

    // Full end-to-end: the repackaged CMAF must itself re-demux cleanly.
    let mut redemux = Fmp4Demux::new();
    let round = redemux
        .unpackage(&fmp4)
        .expect("the repackaged CMAF must re-parse");
    assert_eq!(round.tracks.len(), 1);
    assert!(matches!(
        round.tracks[0].spec.config,
        CodecConfig::Avc { .. }
    ));
    assert!(!round.tracks[0].samples.is_empty());
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

// ── DEMUX = lenient but loud (media plane step-2 fix wave 1, B2/B3): a
//    genuinely unsupported sample entry is skipped, named, rather than
//    failing the whole file ──────────────────────────────────────────────

#[test]
fn fmp4_demux_skips_a_genuinely_unsupported_sample_entry_naming_it_not_failing_the_file() {
    // Start from a real, previously-demuxed AVC config (av_frag.mp4) so the
    // surrounding bytes (avcC/SPS/PPS) are genuine, not hand-fabricated —
    // only the sample entry's four-CC is deliberately mutated to a value no
    // `SampleEntryVariant` matches, simulating a codec this crate has no
    // `CodecConfig` reconstruction for (a QuickTime hint/chapter track,
    // `c608`/`c708`, GoPro `gpmd`, ...).
    //
    // This is a single-track file, so it can only prove the demux doesn't
    // hard-error and that it names what it skipped — it CANNOT distinguish
    // "that one track was skipped" from "the whole file was skipped" (a
    // single-track `Media` with zero tracks looks the same either way). The
    // multi-track test below (`fmp4_demux_skips_one_unmodelled_track_but_keeps_the_rest`)
    // is the one that actually bites that distinction.
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
    let out = demux2
        .unpackage(&mutated)
        .expect("an unrecognised sample entry must be skipped, not fail the whole file");
    assert!(
        out.tracks.is_empty(),
        "the only track was unrecognised, so no track survives"
    );
    assert_eq!(
        out.skipped.len(),
        1,
        "the skipped track must be recorded, not silently vanish"
    );
    assert_eq!(
        out.skipped[0].fourcc, "zzz9",
        "the skip record must name the offending sample entry"
    );
    assert!(
        out.skipped[0].reason.contains("zzz9"),
        "the skip reason must mention the offending fourcc too, got {:?}",
        out.skipped[0].reason
    );
}

#[test]
fn fmp4_demux_skips_one_unmodelled_track_but_keeps_the_rest() {
    // The bite the single-track test above cannot provide: a real 3-track
    // CMAF (AVC + AAC + `stpp` subtitle, `av_aac_subtitle_frag.mp4` — see
    // PROVENANCE.md) with ONLY the subtitle track's sample-entry four-CC
    // mutated to something no `SampleEntryVariant` matches. If a demuxer
    // were fatal on one bad track (the pre-fix `Fmp4Demux` behaviour), this
    // whole file would fail to demux at all; if it silently dropped the
    // skip without recording it (the pre-fix `ProgressiveDemux` behaviour),
    // `skipped` would stay empty. Only "2 tracks survive, the third is
    // named in `skipped`" proves DEMUX = lenient but loud actually holds
    // across more than one track.
    let file = fixture("av_aac_subtitle_frag.mp4");
    let mut probe = Fmp4Demux::new();
    let media = probe
        .unpackage(&file)
        .expect("demux av_aac_subtitle_frag.mp4 unmutated");
    assert_eq!(
        media.tracks.len(),
        3,
        "AVC + AAC + stpp subtitle, unmutated"
    );
    assert!(media.skipped.is_empty(), "nothing is unrecognised yet");

    // Mutate only the `stpp` four-CC (present exactly once in this fixture,
    // in the subtitle track's sample entry) to an unrecognised value. The
    // box size is unchanged (a 4-byte in-place swap), so the buffer stays
    // structurally well-formed; only the subtitle track's codec dispatch
    // fails.
    let mut mutated = file.clone();
    let pos = mutated
        .windows(4)
        .position(|w| w == b"stpp")
        .expect("stpp fourcc must be present exactly once in this fixture");
    assert!(
        !mutated[pos + 4..].windows(4).any(|w| w == b"stpp"),
        "stpp must appear exactly once so the mutation is unambiguous"
    );
    mutated[pos..pos + 4].copy_from_slice(b"zzz9");
    assert_ne!(mutated, file, "mutation must actually change the bytes");

    let mut demux = Fmp4Demux::new();
    let out = demux
        .unpackage(&mutated)
        .expect("one unmodelled track must not fail the whole file");

    assert_eq!(
        out.tracks.len(),
        2,
        "the AVC + AAC tracks must survive; only the mutated subtitle track is dropped"
    );
    assert!(
        out.tracks
            .iter()
            .any(|t| matches!(t.spec.config, CodecConfig::Avc { .. })),
        "the AVC track must still be present"
    );
    assert!(
        out.tracks
            .iter()
            .any(|t| matches!(t.spec.config, CodecConfig::Aac { .. })),
        "the AAC track must still be present"
    );
    assert!(
        out.tracks.iter().all(|t| !t.samples.is_empty()),
        "every surviving track must still carry its samples"
    );
    assert_eq!(
        out.skipped.len(),
        1,
        "exactly the mutated subtitle track must be recorded as skipped"
    );
    assert_eq!(
        out.skipped[0].fourcc, "zzz9",
        "the skip record must name the offending sample entry, not just count it"
    );
}
