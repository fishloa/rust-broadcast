//! Byte-for-byte golden gate for the XML renderers and parsers: DASH MPD,
//! LL-DASH MPD, Smooth manifest and PlayReady WRMHEADER. The MPD reader is
//! dash-runtime's (issue #2340). The expected files in
//! `fixtures/golden/dash/` were generated from the pre-quick-xml code on `origin/main`
//! (see `fixtures/golden/dash/PROVENANCE.md`). Set `GOLDEN_BLESS=<dir>` to write the files
//! instead of comparing; a run with `CI` set refuses to write.
#![cfg(feature = "std")]

use std::fs;
use std::path::{Path, PathBuf};

use broadcast_common::{Package, Unpackage};
use transmux::dash::{
    Addressing, ContentProtectionSystem, DVB_DASH_PROFILE_PREFIX, DVB_HDR_DMI_SCHEME,
    DVB_HDR_DMI_ST2094_40, DashPackager, HDR_METADATA_SCHEME, HDR_METADATA_SMPTE2094_40,
    InbandEventStream, TrackSegments, TrickModeAdaptationSet, TrickModeRepr,
};
use transmux::ll_dash::UTCTIMING_HTTP_HEAD_2014;
use transmux::pipeline::{CodecConfig, SubtitleFormat, SubtitleSampleEntry};
use transmux::ts_demux::TsDemux;
use transmux::{
    AspectRatioIdc, Fmp4Demux, Media, MediaKind, Sample, SmoothManifest, SmoothPackager, Track,
    TrackId, TrackSpec, WvttSampleEntry, playready_wrmheader,
};

fn check(name: &str, actual: &str) {
    if let Ok(dir) = std::env::var("GOLDEN_BLESS") {
        assert!(
            std::env::var_os("CI").is_none(),
            "GOLDEN_BLESS rewrites committed goldens and is refused under CI"
        );
        let path = Path::new(&dir).join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create golden dir");
        }
        fs::write(&path, actual).unwrap_or_else(|e| panic!("write {path:?}: {e}"));
        return;
    }
    let path = test_fixtures::path(name);
    let expected = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    assert_eq!(actual, expected, "{name} differs from the golden output");
}

fn fixtures() -> PathBuf {
    test_fixtures::root()
}

fn media() -> transmux::media::Media {
    let ts = fs::read(fixtures().join("ts/h264-aac.ts")).expect("ts fixture");
    TsDemux::new().unpackage(&ts[..]).expect("demux")
}

/// The fixture media with HDR10+ (ST 2094-40) announced on every track.
fn all_hdr10plus(mut media: Media) -> Media {
    for track in &mut media.tracks {
        track.spec.hdr10plus = true;
    }
    media
}

/// The real HEVC HDR10+ stream (`private/fixtures/video/hdr10plus/`, see its
/// PROVENANCE.md): the only committed media whose video track carries an
/// `hvc1` config and the HDR10+ flag, so it is what exercises the DASH-IF and
/// DVB-DASH HDR `SupplementalProperty` writers.
fn hevc_hdr10plus_media() -> Option<Media> {
    let ts = test_fixtures::require_bytes("video/hdr10plus/tos-256x144-hdr10plus.ts", "hdr10plus")?;
    let mut media = TsDemux::new().unpackage(&ts[..]).expect("demux hdr10plus");
    media
        .tracks
        .retain(|t| matches!(t.spec.config, CodecConfig::Hevc { .. }));
    Some(media)
}

/// The real AVC + `stpp`/TTML capture (`fixtures/mp4/cmaf-av-subtitle`).
fn av_subtitle_media() -> Media {
    let bytes = test_fixtures::read("mp4/cmaf-av-subtitle/av-subtitle-frag.mp4");
    Fmp4Demux::new()
        .unpackage(&bytes[..])
        .expect("demux cmaf AVC + stpp fixture")
}

/// A spec-vector `wvtt` text track: no producer writes WebVTT in ISO BMFF.
fn wvtt_media() -> Media {
    let entry = WvttSampleEntry::new("WEBVTT\n\nNOTE spec vector\n");
    let spec = TrackSpec::new(
        TrackId::new(1),
        1000,
        CodecConfig::Subtitle {
            format: SubtitleFormat::WebVtt,
            sample_entry: Some(SubtitleSampleEntry::WebVtt(entry)),
        },
    );
    let mut payl = Vec::new();
    payl.extend_from_slice(&(8u32 + 2).to_be_bytes());
    payl.extend_from_slice(b"payl");
    payl.extend_from_slice(b"Hi");
    let mut vttc = Vec::new();
    vttc.extend_from_slice(&(8u32 + payl.len() as u32).to_be_bytes());
    vttc.extend_from_slice(b"vttc");
    vttc.extend_from_slice(&payl);
    let sample = Sample::new(vttc, Some(0), Some(0), Some(1000), true);
    Media::new(vec![Track::new(spec, vec![sample])], 1000)
}

/// A two-language audio pair plus a video track — the only way to reach
/// `AdaptationSet@lang` (no committed TS capture names a language code).
fn bilingual_media() -> Media {
    fn lang_es_info(lang: &[u8; 3]) -> Vec<u8> {
        vec![0x0a, 0x04, lang[0], lang[1], lang[2], 0x00]
    }
    let mut media = media();
    let source = media
        .tracks
        .iter()
        .find(|t| media_kind_of(t) == MediaKind::Audio)
        .expect("fixture has an audio track")
        .clone();
    let mut eng = source.clone();
    eng.spec.track_id = TrackId::new(90);
    eng.spec.es_info_descriptors = lang_es_info(b"eng");
    let mut fra = source;
    fra.spec.track_id = TrackId::new(91);
    fra.spec.es_info_descriptors = lang_es_info(b"fra");
    media.tracks.push(eng);
    media.tracks.push(fra);
    media
}

fn media_kind_of(track: &Track) -> MediaKind {
    match &track.spec.config {
        CodecConfig::Avc { .. } => MediaKind::Video,
        CodecConfig::Aac { .. } => MediaKind::Audio,
        _ => MediaKind::Text,
    }
}

/// The AVC track carrying the profile's own display geometry with a VUI
/// `aspect_ratio_idc` set to `3:2`, so `Representation@width`/`@height` (the
/// cropped samples) disagree with `@sar`.
fn squashed_media() -> Media {
    let mut media = media();
    let video = media
        .tracks
        .iter_mut()
        .find(|t| media_kind_of(t) == MediaKind::Video)
        .expect("fixture has a video track");
    let mut display = video.spec.display.clone().expect("AVC SPS geometry");
    display.aspect_ratio = Some(AspectRatioIdc::ExtendedSar {
        sar_width: 3,
        sar_height: 2,
    });
    video.spec.display = Some(display);
    media
}

#[cfg(feature = "cenc")]
fn encrypted_media(scheme: transmux::CencScheme, kid: [u8; 16]) -> Media {
    use transmux::{ConstantIvSenc, SampleEncryptionEntry, TrackEncryption, TrackEncryptionBox};
    let mut media = media();
    for track in &mut media.tracks {
        let samples = vec![
            SampleEncryptionEntry {
                initialization_vector: vec![0u8; 8],
                subsamples: vec![],
            };
            track.samples.len()
        ];
        track.encryption = Some(TrackEncryption::new(
            scheme,
            TrackEncryptionBox {
                version: 0,
                default_crypt_byte_block: 0,
                default_skip_byte_block: 0,
                default_is_protected: 1,
                default_per_sample_iv_size: 8,
                default_kid: kid,
                default_constant_iv: None,
            },
            samples,
            ConstantIvSenc::default(),
        ));
    }
    media
}

/// Run-length-encodable segment durations for every track.
fn timeline_segments(media: &transmux::media::Media) -> Vec<TrackSegments> {
    media
        .tracks
        .iter()
        .map(|t| TrackSegments {
            track_id: t.spec.track_id,
            durations: vec![180_000, 180_000, 180_000, 90_000, 45_000],
        })
        .collect()
}

#[test]
fn dash_mpd_variants_match_golden() {
    let media = media();
    let kid = [
        0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff,
        0x00,
    ];
    let variants: Vec<(&str, DashPackager)> = vec![
        ("default", DashPackager::default()),
        (
            "dynamic",
            DashPackager {
                dynamic: true,
                availability_start_time: Some("2026-01-01T00:00:00Z".into()),
                publish_time: Some("2026-01-01T00:00:01Z".into()),
                minimum_update_period: Some("PT2S".into()),
                time_shift_buffer_depth: Some("PT30S".into()),
                suggested_presentation_delay: Some("PT4S".into()),
                ..DashPackager::default()
            },
        ),
        (
            "timeline",
            DashPackager {
                addressing: Addressing::Timeline,
                segments: timeline_segments(&media),
                ..DashPackager::default()
            },
        ),
        (
            "protected",
            DashPackager {
                content_protection: vec![
                    ContentProtectionSystem {
                        scheme_id_uri: "urn:mpeg:dash:mp4protection:2011".to_string(),
                        value: Some("cenc".to_string()),
                        default_kid: Some(kid),
                        pssh: None,
                    },
                    ContentProtectionSystem {
                        scheme_id_uri: "urn:uuid:edef8ba9-79d6-4ace-a3c8-27dcd51d21ed".to_string(),
                        value: None,
                        default_kid: None,
                        pssh: Some(vec![0, 1, 2, 3, 250, 251, 252]),
                    },
                ],
                inband_event_streams: vec![InbandEventStream {
                    scheme_id_uri: "urn:scte:scte35:2013:bin".to_string(),
                    value: None,
                }],
                ..DashPackager::default()
            },
        ),
        (
            "trick",
            DashPackager {
                trick_mode: Some(TrickModeAdaptationSet {
                    id: 7,
                    main_adaptation_set_id: 0,
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
                }),
                ..DashPackager::default()
            },
        ),
    ];
    for (name, mut pkg) in variants {
        let mpd = pkg
            .package(&media)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        check(&format!("golden/dash/dash-{name}.mpd"), &mpd);
    }
}

#[test]
fn dash_mpd_element_coverage_matches_golden() {
    let kid = [0x11u8; 16];
    let trick = TrickModeAdaptationSet {
        id: 7,
        main_adaptation_set_id: 0,
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
    let variants: Vec<(&str, Media, DashPackager)> = vec![
        (
            "dvb-hdr",
            all_hdr10plus(media()),
            DashPackager {
                profiles: format!("{DVB_DASH_PROFILE_PREFIX}2020"),
                content_protection: vec![ContentProtectionSystem {
                    scheme_id_uri: "urn:mpeg:dash:mp4protection:2011".to_string(),
                    value: Some("cbcs".to_string()),
                    default_kid: Some(kid),
                    pssh: None,
                }],
                inband_event_streams: vec![InbandEventStream {
                    scheme_id_uri: "urn:scte:scte35:2013:bin".to_string(),
                    value: Some("cue".to_string()),
                }],
                trick_mode: Some(trick.clone()),
                ..DashPackager::default()
            },
        ),
        (
            "live",
            media(),
            DashPackager {
                dynamic: true,
                publish_time: Some("2026-01-01T00:00:01Z".into()),
                suggested_presentation_delay: Some("PT4S".into()),
                ..DashPackager::default()
            },
        ),
        ("languages", bilingual_media(), DashPackager::default()),
        ("sar-par", squashed_media(), DashPackager::default()),
        ("stpp", av_subtitle_media(), DashPackager::default()),
        ("wvtt", wvtt_media(), DashPackager::default()),
    ];
    for (name, source, mut pkg) in variants {
        let mpd = pkg
            .package(&source)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        check(&format!("golden/dash/dash-cover-{name}.mpd"), &mpd);
    }
}

/// The DVB-DASH HDR branch: an HEVC HDR10+ track under the DVB-DASH profile
/// emits both the DASH-IF HDR `SupplementalProperty` and its DVB dynamic-
/// mapping counterpart. `Option`, because the HEVC capture is a private
/// fixture: the golden gate skips only when `private/` is not checked out.
#[test]
fn dash_mpd_dvb_hdr10plus_coverage_matches_golden() {
    let Some(media) = hevc_hdr10plus_media() else {
        return;
    };
    let mpd = DashPackager {
        profiles: format!("{DVB_DASH_PROFILE_PREFIX}2020"),
        ..DashPackager::default()
    }
    .package(&media)
    .expect("DVB-DASH HDR10+ MPD");
    for scheme in [HDR_METADATA_SCHEME, DVB_HDR_DMI_SCHEME] {
        assert!(
            mpd.contains(&format!("schemeIdUri=\"{scheme}\"")),
            "the {scheme} property is emitted: {mpd}"
        );
    }
    assert!(mpd.contains(&format!("value=\"{HDR_METADATA_SMPTE2094_40}\"")));
    assert!(mpd.contains(&format!("value=\"{DVB_HDR_DMI_ST2094_40}\"")));
    check("golden/dash/dash-cover-dvb-hdr10plus.mpd", &mpd);
}

#[cfg(feature = "cenc")]
#[test]
fn dash_mpd_cbcs_coverage_matches_golden() {
    let kid = [0x11u8; 16];
    let mpd = DashPackager::default()
        .package(&encrypted_media(transmux::CencScheme::Cbcs, kid))
        .expect("cbcs MPD");
    check("golden/dash/dash-cover-cbcs.mpd", &mpd);
}

#[test]
fn ll_dash_mpd_variants_match_golden() {
    let media = media();
    let plain = crate::ll_packager::ll_packager(2.0, 0.5, 3000);
    let rate = crate::ll_packager::ll_packager(4.0, 1.0, 5000).with_playback_rate(0.96, 1.04);
    let utc = crate::ll_packager::ll_packager(2.0, 0.5, 3000)
        .with_utc_timing(UTCTIMING_HTTP_HEAD_2014, "https://time.example/x?a=1&b=2");
    let mut timeline = crate::ll_packager::ll_packager(2.0, 0.5, 3000);
    timeline.base.addressing = Addressing::Timeline;
    timeline.base.segments = timeline_segments(&media);
    for (name, mut pkg) in [
        ("plain", plain),
        ("rate", rate),
        ("utc", utc),
        ("timeline", timeline),
    ] {
        let mpd = pkg
            .package(&media)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        check(&format!("golden/dash/lldash-{name}.mpd"), &mpd);
    }
}

#[test]
fn smooth_manifest_matches_golden() {
    let out = SmoothPackager::default().package(&media()).expect("smooth");
    check("golden/smooth/smooth.manifest.xml", &out.manifest);
    let parsed = SmoothManifest::parse(&out.manifest).expect("parse own manifest");
    check("golden/smooth/smooth.parse.txt", &format!("{parsed:#?}\n"));
}

#[test]
fn smooth_manifest_element_coverage_matches_golden() {
    let mut two_audio = media();
    let source = two_audio
        .tracks
        .iter()
        .find(|t| media_kind_of(t) == MediaKind::Audio)
        .expect("fixture has an audio track")
        .clone();
    let mut second = source;
    second.spec.track_id = TrackId::new(92);
    two_audio.tracks.push(second);
    let variants: Vec<(&str, Media)> = vec![
        ("stpp", av_subtitle_media()),
        ("wvtt", wvtt_media()),
        ("two-audio", two_audio),
    ];
    for (name, media) in variants {
        let out = SmoothPackager::default().package(&media).expect(name);
        check(
            &format!("golden/smooth/smooth-cover-{name}.xml"),
            &out.manifest,
        );
    }
}

#[test]
fn playready_wrmheader_matches_golden() {
    let kid_a = [0x11u8; 16];
    let kid_b: [u8; 16] = core::array::from_fn(|i| i as u8);
    check(
        "golden/smooth/wrm-two-kids.xml",
        &playready_wrmheader(&[kid_a, kid_b], Some("https://la.example.com/x?a=1&b=2")),
    );
    check(
        "golden/smooth/wrm-no-url.xml",
        &playready_wrmheader(&[kid_a], None),
    );
    check(
        "golden/smooth/wrm-specials.xml",
        &playready_wrmheader(&[kid_a], Some("a&b<c>d\"e'f")),
    );
}

/// Every XML element name and attribute name the DASH/LL-DASH MPD writer
/// (`transmux/src/dash.rs`, `transmux/src/ll_dash/mpd.rs`) or the Smooth
/// manifest writer (`transmux/src/smooth.rs::render_manifest`) can emit. Pinned
/// so a name no golden exercises cannot slip in unnoticed.
const WRITER_ELEMENT_NAMES: &[&str] = &[
    "MPD",
    "Period",
    "AdaptationSet",
    "ContentProtection",
    "cenc:pssh",
    "InbandEventStream",
    "SupplementalProperty",
    "Role",
    "Representation",
    "AudioChannelConfiguration",
    "SegmentTemplate",
    "SegmentTimeline",
    "S",
    "ServiceDescription",
    "Latency",
    "PlaybackRate",
    "UTCTiming",
    "SmoothStreamingMedia",
    "StreamIndex",
    "QualityLevel",
    "c",
    "WRMHEADER",
    "DATA",
    "PROTECTINFO",
    "KIDS",
    "KID",
    "LA_URL",
];

const WRITER_ATTRIBUTE_NAMES: &[&str] = &[
    "xmlns",
    "xmlns:cenc",
    "profiles",
    "type",
    "minBufferTime",
    "mediaPresentationDuration",
    "availabilityStartTime",
    "publishTime",
    "minimumUpdatePeriod",
    "timeShiftBufferDepth",
    "suggestedPresentationDelay",
    "id",
    "start",
    "contentType",
    "mimeType",
    "segmentAlignment",
    "startWithSAP",
    "lang",
    "par",
    "maxPlayoutRate",
    "codingDependency",
    "schemeIdUri",
    "value",
    "cenc:default_KID",
    "codecs",
    "bandwidth",
    "width",
    "height",
    "frameRate",
    "sar",
    "audioSamplingRate",
    "timescale",
    "duration",
    "startNumber",
    "initialization",
    "media",
    "t",
    "d",
    "r",
    "availabilityTimeOffset",
    "availabilityTimeComplete",
    "target",
    "min",
    "max",
    "MajorVersion",
    "MinorVersion",
    "Duration",
    "TimeScale",
    "Type",
    "Name",
    "Subtype",
    "Chunks",
    "QualityLevels",
    "Url",
    "Index",
    "Bitrate",
    "FourCC",
    "MaxWidth",
    "MaxHeight",
    "CodecPrivateData",
    "SamplingRate",
    "Channels",
    "BitsPerSample",
    "AudioTag",
    "n",
    "version",
    "ALGID",
    "VALUE",
];

/// Names the writers can emit from some public-API configuration, paired with
/// a spec clause: the "Spec:" line of the coverage test asserts the clause is
/// cited in the writer's own source, and the "Golden:" line asserts a golden
/// file carries the name — one element/attribute name.
const WRITER_NAME_EVIDENCE: &[&str] = &[
    "Element MPD: Spec: §5.3.1.2 Golden: golden/dash/dash-default.mpd",
    "Element Period: Spec: §5.3.1 Golden: golden/dash/dash-default.mpd",
    "Element AdaptationSet: Spec: §5.3.3 Golden: golden/dash/dash-default.mpd",
    "Element ContentProtection: Spec: §5.8.4.1 Golden: golden/dash/dash-protected.mpd",
    "Element cenc:pssh: Spec: §12.1 Golden: golden/dash/dash-protected.mpd",
    "Element InbandEventStream: Spec: §5.3.3 Golden: golden/dash/dash-protected.mpd",
    "Element SupplementalProperty: Spec: §5.8.5.8 Golden: golden/dash/dash-trick.mpd",
    "Element Role: Spec: §5.8.5.5 Golden: golden/dash/dash-default.mpd",
    "Element Representation: Spec: §5.3.5 Golden: golden/dash/dash-default.mpd",
    "Element AudioChannelConfiguration: Spec: 23003 Golden: golden/dash/dash-default.mpd",
    "Element SegmentTemplate: Spec: §5.3.9.4.4 Golden: golden/dash/dash-default.mpd",
    "Element SegmentTimeline: Spec: §5.3.9.6 Golden: golden/dash/dash-timeline.mpd",
    "Element S: Spec: §5.3.9.6 Golden: golden/dash/dash-timeline.mpd",
    "Element ServiceDescription: Spec: §5.13.2 Golden: golden/dash/lldash-plain.mpd",
    "Element Latency: Spec: §5.13.2 Golden: golden/dash/lldash-plain.mpd",
    "Element PlaybackRate: Spec: PlaybackRate Golden: golden/dash/lldash-rate.mpd",
    "Element UTCTiming: Spec: §5.8.4.11 Golden: golden/dash/lldash-utc.mpd",
    "Element SmoothStreamingMedia: Spec: §2.2.2 Golden: golden/smooth/smooth.manifest.xml",
    "Element StreamIndex: Spec: §2.2.2.3 Golden: golden/smooth/smooth.manifest.xml",
    "Element QualityLevel: Spec: §2.2.2.5 Golden: golden/smooth/smooth.manifest.xml",
    "Element c: Spec: §2.2.2.6 Golden: golden/smooth/smooth.manifest.xml",
    "Element WRMHEADER: Spec: §3.2 Golden: golden/smooth/wrm-two-kids.xml",
    "Element DATA: Spec: §3.2 Golden: golden/smooth/wrm-two-kids.xml",
    "Element PROTECTINFO: Spec: §3.2 Golden: golden/smooth/wrm-two-kids.xml",
    "Element KIDS: Spec: §3.2 Golden: golden/smooth/wrm-two-kids.xml",
    "Element KID: Spec: §3.2 Golden: golden/smooth/wrm-two-kids.xml",
    "Element LA_URL: Spec: §3.2 Golden: golden/smooth/wrm-two-kids.xml",
    "Attribute schemeIdUri: Spec: DASH-IF-IOP-10.3.2.4 Golden: golden/dash/dash-cover-dvb-hdr10plus.mpd#\"http://dashif.org/metadata/hdr\"",
    "Attribute schemeIdUri: Spec: ETSI-TS-103-285 Golden: golden/dash/dash-cover-dvb-hdr10plus.mpd#\"urn:dvb:dash:hdr-dmi\"",
    "Attribute xmlns: Spec: §5.3.1.2 Golden: golden/dash/dash-default.mpd",
    "Attribute xmlns:cenc: Spec: 23001-7 Golden: golden/dash/dash-protected.mpd",
    "Attribute profiles: Spec: §5.3.1.2 Golden: golden/dash/dash-default.mpd",
    "Attribute type: Spec: §5.3.1.2 Golden: golden/dash/dash-dynamic.mpd",
    "Attribute minBufferTime: Spec: §5.3.1.2 Golden: golden/dash/dash-default.mpd",
    "Attribute mediaPresentationDuration: Spec: §5.3.1.2 Golden: golden/dash/dash-default.mpd",
    "Attribute availabilityStartTime: Spec: §5.3.1.2 Golden: golden/dash/dash-dynamic.mpd",
    "Attribute publishTime: Spec: §5.3.1.2 Golden: golden/dash/dash-dynamic.mpd",
    "Attribute minimumUpdatePeriod: Spec: §5.3.1.2 Golden: golden/dash/dash-dynamic.mpd",
    "Attribute timeShiftBufferDepth: Spec: §5.3.1.2 Golden: golden/dash/dash-dynamic.mpd",
    "Attribute suggestedPresentationDelay: Spec: §5.3.1.2 Golden: golden/dash/dash-dynamic.mpd",
    "Attribute id: Spec: §5.3.3 Golden: golden/dash/dash-default.mpd",
    "Attribute start: Spec: §5.3.1 Golden: golden/dash/dash-default.mpd",
    "Attribute contentType: Spec: §5.3.3 Golden: golden/dash/dash-default.mpd",
    "Attribute mimeType: Spec: §5.3.3 Golden: golden/dash/dash-default.mpd",
    "Attribute segmentAlignment: Spec: §5.3.3 Golden: golden/dash/dash-default.mpd",
    "Attribute startWithSAP: Spec: §5.3.3 Golden: golden/dash/dash-default.mpd",
    "Attribute lang: Spec: §5.3.3.2 Golden: golden/dash/dash-cover-languages.mpd",
    "Attribute par: Spec: §4.4 Golden: golden/dash/dash-default.mpd",
    "Attribute maxPlayoutRate: Spec: §5.8.5.8 Golden: golden/dash/dash-trick.mpd",
    "Attribute codingDependency: Spec: §5.8.5.8 Golden: golden/dash/dash-trick.mpd",
    "Attribute schemeIdUri: Spec: §5.8.4.1 Golden: golden/dash/dash-protected.mpd",
    "Attribute value: Spec: §5.8.4.1 Golden: golden/dash/dash-protected.mpd",
    "Attribute cenc:default_KID: Spec: 23001-7 Golden: golden/dash/dash-protected.mpd",
    "Attribute codecs: Spec: §5.3.5 Golden: golden/dash/dash-default.mpd",
    "Attribute bandwidth: Spec: §5.3.5 Golden: golden/dash/dash-default.mpd",
    "Attribute width: Spec: §5.3.5 Golden: golden/dash/dash-default.mpd",
    "Attribute height: Spec: §5.3.5 Golden: golden/dash/dash-default.mpd",
    "Attribute frameRate: Spec: §5.3.5 Golden: golden/dash/dash-default.mpd",
    "Attribute sar: Spec: §4.4 Golden: golden/dash/dash-default.mpd",
    "Attribute audioSamplingRate: Spec: §5.3.5 Golden: golden/dash/dash-default.mpd",
    "Attribute timescale: Spec: §5.3.9.4.4 Golden: golden/dash/dash-default.mpd",
    "Attribute duration: Spec: §5.3.9.4.4 Golden: golden/dash/dash-default.mpd",
    "Attribute startNumber: Spec: §5.3.9.4.4 Golden: golden/dash/dash-default.mpd",
    "Attribute initialization: Spec: §5.3.9.4.4 Golden: golden/dash/dash-default.mpd",
    "Attribute media: Spec: §5.3.9.4.4 Golden: golden/dash/dash-default.mpd",
    "Attribute t: Spec: §5.3.9.6 Golden: golden/dash/dash-timeline.mpd",
    "Attribute d: Spec: §5.3.9.6 Golden: golden/dash/dash-timeline.mpd",
    "Attribute r: Spec: §5.3.9.6 Golden: golden/dash/dash-timeline.mpd",
    "Attribute availabilityTimeOffset: Spec: §5.3.9.5.3 Golden: golden/dash/lldash-plain.mpd",
    "Attribute availabilityTimeComplete: Spec: §5.3.9.5.3 Golden: golden/dash/lldash-plain.mpd",
    "Attribute target: Spec: §5.13.2 Golden: golden/dash/lldash-plain.mpd",
    "Attribute min: Spec: PlaybackRate Golden: golden/dash/lldash-rate.mpd",
    "Attribute max: Spec: PlaybackRate Golden: golden/dash/lldash-rate.mpd",
    "Attribute MajorVersion: Spec: §2.2.2 Golden: golden/smooth/smooth.manifest.xml",
    "Attribute MinorVersion: Spec: §2.2.2 Golden: golden/smooth/smooth.manifest.xml",
    "Attribute Duration: Spec: §2.2.2 Golden: golden/smooth/smooth.manifest.xml",
    "Attribute TimeScale: Spec: §2.2.2 Golden: golden/smooth/smooth.manifest.xml",
    "Attribute Type: Spec: §2.2.2.3 Golden: golden/smooth/smooth.manifest.xml",
    "Attribute Name: Spec: §2.2.2.3 Golden: golden/smooth/smooth-cover-two-audio.xml",
    "Attribute Subtype: Spec: §2.2.2.3 Golden: golden/smooth/smooth.manifest.xml",
    "Attribute Chunks: Spec: §2.2.2.3 Golden: golden/smooth/smooth.manifest.xml",
    "Attribute QualityLevels: Spec: §2.2.2.3 Golden: golden/smooth/smooth.manifest.xml",
    "Attribute Url: Spec: §2.2.2.3 Golden: golden/smooth/smooth.manifest.xml",
    "Attribute Index: Spec: §2.2.2.5 Golden: golden/smooth/smooth.manifest.xml",
    "Attribute Bitrate: Spec: §2.2.2.5 Golden: golden/smooth/smooth.manifest.xml",
    "Attribute FourCC: Spec: §2.2.2.5 Golden: golden/smooth/smooth.manifest.xml",
    "Attribute MaxWidth: Spec: §2.2.2.5 Golden: golden/smooth/smooth.manifest.xml",
    "Attribute MaxHeight: Spec: §2.2.2.5 Golden: golden/smooth/smooth.manifest.xml",
    "Attribute CodecPrivateData: Spec: §2.2.2.5 Golden: golden/smooth/smooth.manifest.xml",
    "Attribute SamplingRate: Spec: §2.2.2.5 Golden: golden/smooth/smooth.manifest.xml",
    "Attribute Channels: Spec: §2.2.2.5 Golden: golden/smooth/smooth.manifest.xml",
    "Attribute BitsPerSample: Spec: §2.2.2.5 Golden: golden/smooth/smooth.manifest.xml",
    "Attribute AudioTag: Spec: §2.2.2.5 Golden: golden/smooth/smooth.manifest.xml",
    "Attribute n: Spec: §2.2.2.6 Golden: golden/smooth/smooth.manifest.xml",
    "Attribute version: Spec: §3.2 Golden: golden/smooth/wrm-two-kids.xml",
    "Attribute ALGID: Spec: §3.2 Golden: golden/smooth/wrm-two-kids.xml",
    "Attribute VALUE: Spec: §3.2 Golden: golden/smooth/wrm-two-kids.xml",
];

#[test]
fn writer_goldens_cover_every_emitted_name() {
    let mut evidence: Vec<(bool, &str, String, String, String)> = Vec::new();
    for entry in WRITER_NAME_EVIDENCE {
        fn part<'a>(entry: &str, (text, rest): &(&'a str, &'a str)) -> (&'a str, &'a str) {
            text.split_once(rest)
                .unwrap_or_else(|| panic!("malformed evidence entry: {entry}"))
        }
        let (tag, rest) = part(entry, &(entry, " "));
        let (name, spec_and_golden) = part(entry, &(rest, ": "));
        let (spec, golden) = part(entry, &(spec_and_golden, " Golden: "));
        let spec = part(entry, &(spec, "Spec:"))
            .1
            .trim_start_matches(' ')
            .to_string();
        let (golden, needle) = match golden.split_once('#') {
            None => (golden.to_string(), String::new()),
            Some((file, needle)) => (file.to_string(), needle.to_string()),
        };
        let spec = if needle.is_empty() {
            spec
        } else {
            format!(
                "{}?{}",
                spec_value_constant(&spec),
                needle.trim_matches('"')
            )
        };
        evidence.push((tag == "Element", name, spec, golden, needle));
    }
    for (is_element, name, _, _, _) in &evidence {
        let list = if *is_element {
            WRITER_ELEMENT_NAMES
        } else {
            WRITER_ATTRIBUTE_NAMES
        };
        assert!(
            list.contains(name),
            "{name} is pinned twice or missing from the name list"
        );
    }
    for name in WRITER_ELEMENT_NAMES.iter().chain(WRITER_ATTRIBUTE_NAMES) {
        assert!(
            evidence.iter().any(|(_, n, _, _, _)| n == name),
            "{name} has no evidence entry"
        );
    }

    let source = writer_source();
    for (is_element, name, spec, golden, needle) in &evidence {
        let golden = test_fixtures::read_to_string(golden.as_str());
        let spec = match spec.split_once('?') {
            None => spec.as_str(),
            Some((constant, value)) => {
                let declared = format!("pub const {constant}: &str = \"{value}\";");
                assert!(
                    source.contains(&declared),
                    "{name}: the writer does not declare {declared:?}"
                );
                constant
            }
        };
        assert!(
            source.contains(spec),
            "{name}: spec citation {spec:?} is not in the writer source"
        );
        let emitted = if needle.is_empty() {
            format!("{}{name}", if *is_element { "<" } else { " " })
        } else {
            needle.clone()
        };
        assert!(golden.contains(&emitted), "{name} is not in {golden:?}");
    }
}

/// The `dash.rs` constant that holds an attribute value no source literal
/// spells out: the DVB-DASH HDR `schemeIdUri`s are emitted through their
/// constants, so the coverage check pins the constant's declaration (its
/// name and value) where it would otherwise look for a literal, and checks
/// the golden against the value the constant declares.
fn spec_value_constant(spec: &str) -> &'static str {
    match spec {
        "DASH-IF-IOP-10.3.2.4" => "HDR_METADATA_SCHEME",
        "ETSI-TS-103-285" => "DVB_HDR_DMI_SCHEME",
        other => unreachable!("no constant is named for the spec citation {other:?}"),
    }
}

fn writer_source() -> String {
    let mut source = String::new();
    for path in &[
        "dash.rs",
        "ll_dash.rs",
        "ll_dash/mpd.rs",
        "smooth.rs",
        "drm.rs",
    ] {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(path);
        let text =
            fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        source.push_str(&workspace_guards::scan::non_test_source(&text));
    }
    source
}
