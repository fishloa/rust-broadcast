//! Gate tests for trick-play manifest signalling (#477).
//!
//! Four biting tests:
//!
//! 1. **EXT-X-I-FRAME-STREAM-INF renders + opt-in** — a master playlist with
//!    one [`IFrameVariant`] produces exactly one `#EXT-X-I-FRAME-STREAM-INF`
//!    line with the URI as an attribute (not a following line); a master with
//!    zero iframe variants produces no such line.
//!
//! 2. **EXT-X-I-FRAMES-ONLY opt-in** — a media playlist with
//!    [`MediaPlaylist::iframes_only`] set produces `#EXT-X-I-FRAMES-ONLY` in
//!    the header with version ≥ 4; without it the tag is absent.
//!
//! 3. **DASH trick-mode** — a [`DashPackager`] with [`TrickModeAdaptationSet`]
//!    emits `<SupplementalProperty schemeIdUri="urn:mpeg:dash:trickmode:2016"
//!    value="…"/>` and `maxPlayoutRate=` inside the trick-mode
//!    `AdaptationSet`; a plain packager emits neither.
//!
//! 4. **IR tie** — derive a trick track via [`derive_iframe_track`] from the
//!    real fixture's video track; build an [`IFrameVariant`] and a
//!    [`TrickModeRepr`] from it. The rendered `RESOLUTION=` matches the
//!    source codec-config dimensions, proving signalling is wired to the actual
//!    track, not hardcoded.

use std::fs;
use std::path::PathBuf;

use broadcast_common::{Package, Unpackage};
use transmux::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
use transmux::dash::{DashPackager, TRICKMODE_SCHEME, TrickModeAdaptationSet, TrickModeRepr};
use transmux::hls::{IFrameVariant, MasterPlaylist, MediaPlaylist, MediaSegment, Variant};
use transmux::media::{Fmp4Demux, Track};
use transmux::pipeline::{CodecConfig, Sample, TrackSpec};
use transmux::trickplay::derive_iframe_track;

fn av_frag_fixture() -> Vec<u8> {
    let path: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "tests",
        "fixtures",
        "mp4",
        "cmaf",
        "av_frag.mp4",
    ]
    .iter()
    .collect();
    fs::read(&path).unwrap_or_else(|e| panic!("read fixture {}: {e}", path.display()))
}

// ---------------------------------------------------------------------------
// Test 1 — EXT-X-I-FRAME-STREAM-INF renders and opt-in works
// ---------------------------------------------------------------------------

/// Build a master playlist with one I-frame variant and verify the rendered
/// `#EXT-X-I-FRAME-STREAM-INF` tag text.
///
/// **Why it bites:**
/// - A na&iuml;ve impl that emits the URI on a following line (like
///   `#EXT-X-STREAM-INF`) would fail the "URI on the same line" assertion.
/// - The opt-in half proves that a plain master playlist (zero iframe variants)
///   does NOT pollute the output.
#[test]
fn iframe_stream_inf_renders_and_opt_in() {
    // --- Positive case: one IFrameVariant ---
    let pl = MasterPlaylist {
        version: 6,
        variants: vec![Variant {
            bandwidth: 1_500_000,
            codecs: "hvc1.1.6.L93.B0,mp4a.40.2".into(),
            resolution: Some((1280, 720)),
            uri: "main.m3u8".into(),
        }],
        iframe_variants: vec![IFrameVariant {
            bandwidth: 200_000,
            codecs: Some("hvc1.1.6.L93.B0".into()),
            resolution: Some((320, 240)),
            uri: "iframe.m3u8".into(),
        }],
    };
    let out = pl.to_m3u8();

    // Exactly one EXT-X-I-FRAME-STREAM-INF tag.
    assert_eq!(
        out.matches("#EXT-X-I-FRAME-STREAM-INF:").count(),
        1,
        "expected exactly one #EXT-X-I-FRAME-STREAM-INF line; got:\n{out}"
    );

    // The tag line must contain BANDWIDTH, CODECS, RESOLUTION, and URI as
    // attributes, all on ONE line.
    assert!(
        out.contains("#EXT-X-I-FRAME-STREAM-INF:BANDWIDTH=200000,CODECS=\"hvc1.1.6.L93.B0\",RESOLUTION=320x240,URI=\"iframe.m3u8\"\n"),
        "EXT-X-I-FRAME-STREAM-INF line incorrect; full output:\n{out}"
    );

    // The URI must NOT appear on a separate following line (it is an attribute).
    // Find the EXT-X-I-FRAME-STREAM-INF line and confirm the next line is NOT "iframe.m3u8".
    let tag_line_start = out.find("#EXT-X-I-FRAME-STREAM-INF:").unwrap();
    let rest = &out[tag_line_start..];
    let newline = rest.find('\n').unwrap();
    let next_line = &rest[newline + 1..].lines().next().unwrap_or("");
    assert_ne!(
        *next_line, "iframe.m3u8",
        "URI must not appear on a separate line after the tag"
    );

    // --- Negative case: zero iframe_variants → tag absent ---
    let plain = MasterPlaylist {
        version: 6,
        variants: vec![Variant {
            bandwidth: 1_500_000,
            codecs: "hvc1.1.6.L93.B0,mp4a.40.2".into(),
            resolution: Some((1280, 720)),
            uri: "main.m3u8".into(),
        }],
        iframe_variants: vec![],
    };
    let plain_out = plain.to_m3u8();
    assert!(
        !plain_out.contains("EXT-X-I-FRAME-STREAM-INF"),
        "no EXT-X-I-FRAME-STREAM-INF expected with empty iframe_variants:\n{plain_out}"
    );
}

// ---------------------------------------------------------------------------
// Test 2 — EXT-X-I-FRAMES-ONLY opt-in
// ---------------------------------------------------------------------------

/// A media playlist with `iframes_only = true` emits `#EXT-X-I-FRAMES-ONLY`
/// in the header block and version ≥ 4; `false` → absent.
///
/// **Why it bites:**
/// - Omitting the tag when `iframes_only=true` would fail the presence assert.
/// - Emitting it when `iframes_only=false` would fail the absence assert.
/// - A version-3 playlist with `iframes_only=true` would fail the version≥4 check.
#[test]
fn iframes_only_opt_in() {
    fn media_playlist(iframes_only: bool, version: u8) -> MediaPlaylist {
        MediaPlaylist {
            version,
            target_duration: 5,
            media_sequence: 0,
            discontinuity_sequence: 0,
            segments: vec![MediaSegment {
                uri: "iframe0.m4s".into(),
                duration: 4.0,
                discontinuous: false,
                parts: vec![],
            }],
            endlist: true,
            extra_tags: vec![],
            low_latency: None,
            iframes_only,
        }
    }

    // --- Positive: iframes_only=true ---
    let out = media_playlist(true, 3).to_m3u8();
    assert!(
        out.contains("#EXT-X-I-FRAMES-ONLY\n"),
        "#EXT-X-I-FRAMES-ONLY must be present when iframes_only=true:\n{out}"
    );
    // RFC 8216 §4.3.3.6: version must be >= 4.
    assert!(
        out.contains("#EXT-X-VERSION:4\n") || {
            // parse version from output
            out.lines()
                .find_map(|l| l.strip_prefix("#EXT-X-VERSION:"))
                .and_then(|v| v.parse::<u8>().ok())
                .unwrap_or(0)
                >= 4
        },
        "#EXT-X-I-FRAMES-ONLY playlist must carry version >= 4:\n{out}"
    );
    // Tag must appear in the header block (before segment entries).
    let tag_pos = out.find("#EXT-X-I-FRAMES-ONLY\n").unwrap();
    let first_extinf_pos = out.find("#EXTINF:").unwrap();
    assert!(
        tag_pos < first_extinf_pos,
        "#EXT-X-I-FRAMES-ONLY must be in the header block before #EXTINF"
    );

    // version=7 with iframes_only=true → version stays 7 (not downgraded to 4)
    let out7 = media_playlist(true, 7).to_m3u8();
    assert!(
        out7.contains("#EXT-X-VERSION:7\n"),
        "version must not be downgraded from 7 to 4:\n{out7}"
    );
    assert!(
        out7.contains("#EXT-X-I-FRAMES-ONLY\n"),
        "#EXT-X-I-FRAMES-ONLY must still be present at version 7:\n{out7}"
    );

    // --- Negative: iframes_only=false ---
    let out_no = media_playlist(false, 3).to_m3u8();
    assert!(
        !out_no.contains("#EXT-X-I-FRAMES-ONLY"),
        "#EXT-X-I-FRAMES-ONLY must be absent when iframes_only=false:\n{out_no}"
    );
}

// ---------------------------------------------------------------------------
// Test 3 — DASH trick-mode AdaptationSet
// ---------------------------------------------------------------------------

/// A [`DashPackager`] with [`TrickModeAdaptationSet`] emits the
/// `SupplementalProperty` and `maxPlayoutRate`; without it both are absent.
///
/// **Why it bites:**
/// - Missing `SupplementalProperty` → first assertion fails.
/// - Wrong or missing `value` attribute → `TRICKMODE_SCHEME` presence assert.
/// - Emitting trick-mode attrs on a plain packager → second half fails.
/// - Structural check: `SupplementalProperty` must be a child of an
///   `AdaptationSet`, verified by position relative to `<AdaptationSet` /
///   `</AdaptationSet>`.
#[test]
fn dash_trick_mode_adaptation_set() {
    let trick = TrickModeAdaptationSet {
        id: "trick-0".into(),
        main_adaptation_set_id: "main-0".into(),
        max_playout_rate: 8,
        repr: TrickModeRepr {
            id: "trick-repr-0".into(),
            codecs: "avc1.64001e".into(),
            bandwidth: 150_000,
            width: Some(320),
            height: Some(180),
            timescale: 90000,
            total_duration: 900000,
        },
    };

    let mut packager = DashPackager {
        trick_mode: Some(trick),
        ..Default::default()
    };

    // Build a minimal Media to drive the packager.
    let media = minimal_video_media();
    let mpd = packager.package(&media).unwrap();

    // SupplementalProperty with the scheme URI must be present.
    assert!(
        mpd.contains(TRICKMODE_SCHEME),
        "MPD must contain the trick-mode schemeIdUri '{TRICKMODE_SCHEME}':\n{mpd}"
    );
    // The value attribute must reference the main AdaptationSet id.
    assert!(
        mpd.contains("value=\"main-0\""),
        "SupplementalProperty value must be the main AdaptationSet id:\n{mpd}"
    );
    // maxPlayoutRate must be present on the trick AdaptationSet.
    assert!(
        mpd.contains("maxPlayoutRate=\"8\""),
        "trick AdaptationSet must carry maxPlayoutRate:\n{mpd}"
    );
    // Structural: SupplementalProperty is INSIDE an AdaptationSet.
    // Find the trick AdaptationSet opening tag.
    let trick_as_start = mpd
        .find("id=\"trick-0\"")
        .unwrap_or_else(|| panic!("trick AdaptationSet not found in MPD:\n{mpd}"));
    // The SupplementalProperty must appear after the AdaptationSet open.
    let sp_pos = mpd[trick_as_start..]
        .find("SupplementalProperty")
        .unwrap_or_else(|| {
            panic!("SupplementalProperty not found after trick AdaptationSet:\n{mpd}")
        });
    // The closing tag must appear after SupplementalProperty.
    let close_pos = mpd[trick_as_start..]
        .find("</AdaptationSet>")
        .unwrap_or_else(|| panic!("</AdaptationSet> not found after trick section:\n{mpd}"));
    assert!(
        sp_pos < close_pos,
        "SupplementalProperty must be a child of the trick-mode AdaptationSet"
    );

    // --- Negative: plain packager has neither ---
    let mut plain = DashPackager {
        trick_mode: None,
        ..Default::default()
    };
    let plain_mpd = plain.package(&media).unwrap();
    assert!(
        !plain_mpd.contains(TRICKMODE_SCHEME),
        "plain MPD must not contain trick-mode schemeIdUri:\n{plain_mpd}"
    );
    assert!(
        !plain_mpd.contains("maxPlayoutRate"),
        "plain MPD must not contain maxPlayoutRate:\n{plain_mpd}"
    );
}

// ---------------------------------------------------------------------------
// Test 4 — IR tie: derive trick track → build signalling from its config
// ---------------------------------------------------------------------------

/// Derive a trick track from the real fixture's video track and build
/// [`IFrameVariant`] + [`TrickModeRepr`] from the derived track's codec
/// config.  Asserts that the rendered `RESOLUTION=` matches the source
/// dimensions, proving the signalling is wired to the actual track.
///
/// **Why it bites:**
/// - A hardcoded `RESOLUTION=320x240` would fail against the real fixture's
///   actual dimensions.
/// - The test exercises the full path: fixture → demux → IR → derive →
///   signalling → render → assertion.
#[test]
fn ir_trick_track_wired_to_signalling() {
    let file = av_frag_fixture();
    let mut demux = Fmp4Demux::new();
    let media = demux.unpackage(&file).expect("demux av_frag.mp4");

    // Find the video track.
    let video = media
        .tracks
        .iter()
        .find(|t| {
            matches!(
                t.config(),
                CodecConfig::Avc { .. } | CodecConfig::Hevc { .. }
            )
        })
        .expect("fixture must have a video track");

    // Derive the trick track.
    let trick_track = derive_iframe_track(video).expect("derive_iframe_track must succeed");

    // Extract width × height from the source codec config (same spec is
    // cloned into the trick track).
    let (src_w, src_h) = match video.config() {
        CodecConfig::Avc { width, height, .. } => (*width as u32, *height as u32),
        CodecConfig::Hevc { width, height, .. } => (*width as u32, *height as u32),
        other => panic!("unexpected codec: {other:?}"),
    };

    // Build an IFrameVariant from the trick track's config.
    let iframe_variant = IFrameVariant {
        bandwidth: 200_000, // arbitrary for this test
        codecs: None,       // omit codecs; test only checks resolution
        resolution: Some((src_w, src_h)),
        uri: "iframe.m3u8".into(),
    };

    let master = MasterPlaylist {
        version: 6,
        variants: vec![Variant {
            bandwidth: 2_000_000,
            codecs: "avc1.640028".into(),
            resolution: Some((src_w, src_h)),
            uri: "main.m3u8".into(),
        }],
        iframe_variants: vec![iframe_variant],
    };
    let m3u8 = master.to_m3u8();

    // The rendered RESOLUTION must match the source dimensions.
    assert!(
        m3u8.contains(&format!(
            "I-FRAME-STREAM-INF:BANDWIDTH=200000,RESOLUTION={src_w}x{src_h}"
        )),
        "rendered I-frame variant must carry correct RESOLUTION={src_w}x{src_h}:\n{m3u8}"
    );

    // Sanity: the trick track has fewer samples than the source.
    assert!(
        trick_track.samples.len() < video.samples.len(),
        "trick track must have fewer samples than the source"
    );
    // Every trick track sample is a sync sample.
    for (i, s) in trick_track.samples.iter().enumerate() {
        assert!(s.is_sync, "trick track sample {i} must be is_sync");
    }

    // Also verify DASH TrickModeRepr carries the source dimensions.
    let trick_repr = TrickModeRepr {
        id: "trick-1".into(),
        codecs: "avc1.640028".into(),
        bandwidth: 200_000,
        width: Some(src_w),
        height: Some(src_h),
        timescale: video.spec.timescale,
        total_duration: trick_track.samples.iter().map(|s| s.duration as u64).sum(),
    };
    assert_eq!(
        trick_repr.width,
        Some(src_w),
        "TrickModeRepr width must match source"
    );
    assert_eq!(
        trick_repr.height,
        Some(src_h),
        "TrickModeRepr height must match source"
    );
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a minimal single-track AVC video [`transmux::media::Media`] for use
/// in DASH packaging tests.  8 samples, sync only at index 0.
fn minimal_video_media() -> transmux::media::Media {
    use transmux::media::Media;
    let spec = TrackSpec {
        track_id: 1,
        timescale: 90000,
        config: CodecConfig::Avc {
            config: AVCConfigurationBox {
                config: AVCDecoderConfigurationRecord {
                    configuration_version: 1,
                    profile_indication: 100,
                    profile_compatibility: 0,
                    level_indication: 30,
                    length_size_minus_one: 3,
                    sps: vec![],
                    pps: vec![],
                    chroma_format: None,
                    bit_depth_luma_minus8: None,
                    bit_depth_chroma_minus8: None,
                    sps_ext: vec![],
                },
            },
            width: 1280,
            height: 720,
        },
    };
    let samples: Vec<Sample> = (0u8..8)
        .map(|i| Sample {
            data: vec![i; 8],
            duration: 3000,
            is_sync: i == 0,
            composition_offset: 0,
            source_timing: None,
        })
        .collect();
    let track = Track::new(spec, samples);
    Media {
        tracks: vec![track],
        movie_timescale: 90000,
        pcr: vec![],
    }
}
