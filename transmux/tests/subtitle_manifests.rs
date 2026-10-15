//! Subtitle tracks (`stpp` TTML / `wvtt` WebVTT, ISO/IEC 14496-30) in the DASH
//! MPD and the Smooth client manifest.
//!
//! Before this, `DashPackager` answered `UnsupportedCodec { "Subtitle" }` and
//! `SmoothPackager` `InvalidInput` for any subtitle track, although the CMAF mux
//! has re-muxed them since #1186.
//!
//! Spec: DASH signalling per ETSI TS 103 285 §7.1.1 (`@contentType="text"`,
//! `application/mp4`, `@codecs` beginning `stpp`; bare `stpp` is allowed and
//! means `stpp.ttml.etd1`); Smooth per [MS-SSTR] §2.2.2.3 (`Type="text"`,
//! `Subtype` required) and §2.2.2.5 (FourCC: a registered MPEG4-RA code).
//!
//! Real artifact: `tests/fixtures/mp4/cmaf/av_subtitle_frag.mp4` (ffmpeg CMAF,
//! AVC + `stpp`/TTML). `wvtt` has no real producer available (ffmpeg
//! cannot write it), so it is a spec-vector.

use broadcast_common::{Package, Unpackage};
use transmux::pipeline::{CodecConfig, SubtitleFormat, SubtitleSampleEntry};
use transmux::{
    DashPackager, Fmp4Demux, Media, Sample, SmoothPackager, Track, TrackId, TrackSpec,
    WvttSampleEntry,
};

fn load_av_subtitle() -> Media {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/mp4/cmaf/av_subtitle_frag.mp4"
    );
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    Fmp4Demux::new().unpackage(&bytes).expect("demux fixture")
}

fn is_subtitle(t: &Track) -> bool {
    matches!(t.spec.config, CodecConfig::Subtitle { .. })
}

/// The text of the element starting at the first `<tag` that contains
/// `needle` up to its closing `>`: enough to read one start tag's attributes.
fn start_tag_containing<'a>(xml: &'a str, tag: &str, needle: &str) -> &'a str {
    let open = format!("<{tag}");
    let mut from = 0;
    while let Some(i) = xml[from..].find(&open) {
        let start = from + i;
        let end = start + xml[start..].find('>').expect("tag end");
        if xml[start..=end].contains(needle) {
            return &xml[start..=end];
        }
        from = end;
    }
    panic!("no <{tag} ...{needle}...> in:\n{xml}");
}

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

#[test]
fn dash_mpd_carries_a_stpp_adaptation_set_for_the_real_ttml_track() {
    let media = load_av_subtitle();
    assert!(media.tracks.iter().any(is_subtitle), "fixture has stpp");
    let mpd = DashPackager::default().package(&media).expect("MPD");

    let set = start_tag_containing(&mpd, "AdaptationSet", "contentType=\"text\"");
    assert!(set.contains("mimeType=\"application/mp4\""), "{set}");

    let rep = start_tag_containing(&mpd, "Representation", "codecs=\"stpp\"");
    assert!(rep.contains("mimeType=\"application/mp4\""), "{rep}");
    assert!(rep.contains("bandwidth=\""), "{rep}");
    for geometry in [" width=", " height=", " frameRate=", " audioSamplingRate="] {
        assert!(
            !rep.contains(geometry),
            "text Representation has {geometry}: {rep}"
        );
    }
    // The text set follows the video set (the fixture carries AVC + stpp).
    let video = mpd.find("contentType=\"video\"").expect("video set");
    let text = mpd.find("contentType=\"text\"").expect("text set");
    assert!(video < text);
}

#[test]
fn dash_mpd_carries_a_wvtt_adaptation_set() {
    let mpd = DashPackager::default().package(&wvtt_media()).expect("MPD");
    start_tag_containing(&mpd, "AdaptationSet", "contentType=\"text\"");
    let rep = start_tag_containing(&mpd, "Representation", "codecs=\"wvtt\"");
    assert!(rep.contains("mimeType=\"application/mp4\""), "{rep}");
}

#[test]
fn dash_still_rejects_a_subtitle_track_with_no_isobmff_sample_entry() {
    let spec = TrackSpec::new(
        TrackId::new(9),
        1000,
        CodecConfig::Subtitle {
            format: SubtitleFormat::DvbBitmap,
            sample_entry: None,
        },
    );
    let media = Media::new(vec![Track::new(spec, vec![])], 1000);
    let err = DashPackager::default().package(&media).unwrap_err();
    assert!(
        matches!(err, transmux::Error::UnsupportedCodec { codec: "Subtitle" }),
        "{err:?}"
    );
}

#[test]
fn smooth_manifest_carries_a_text_stream_and_its_fragments() {
    let media = load_av_subtitle();
    let sub = media.tracks.iter().find(|t| is_subtitle(t)).unwrap();
    let out = SmoothPackager::default().package(&media).expect("smooth");

    let stream = start_tag_containing(&out.manifest, "StreamIndex", "Type=\"text\"");
    assert!(stream.contains("Subtype=\"SUBT\""), "{stream}");
    let ql = start_tag_containing(&out.manifest, "QualityLevel", "FourCC=\"stpp\"");
    for video_or_audio in ["MaxWidth", "SamplingRate", "Channels", "CodecPrivateData"] {
        assert!(
            !ql.contains(video_or_audio),
            "text QualityLevel has {video_or_audio}: {ql}"
        );
    }

    // The text track's fragments are present and carry the original TTML
    // sample bytes unchanged.
    let frags: Vec<_> = out
        .fragments
        .iter()
        .filter(|f| f.track_id == sub.spec.track_id)
        .collect();
    assert!(!frags.is_empty(), "no fragments for the subtitle track");
    let first_sample = &sub.samples[0].data[..];
    assert!(
        frags.iter().any(|f| f
            .data
            .windows(first_sample.len())
            .any(|w| w == first_sample)),
        "the first TTML document is not inside any text fragment"
    );
}

#[test]
fn smooth_wvtt_text_stream_uses_the_wvtt_fourcc() {
    let out = SmoothPackager::default()
        .package(&wvtt_media())
        .expect("smooth");
    start_tag_containing(&out.manifest, "StreamIndex", "Type=\"text\"");
    start_tag_containing(&out.manifest, "QualityLevel", "FourCC=\"wvtt\"");
}

#[test]
fn smooth_rejects_a_subtitle_track_with_no_isobmff_sample_entry() {
    let spec = TrackSpec::new(
        TrackId::new(9),
        1000,
        CodecConfig::Subtitle {
            format: SubtitleFormat::Teletext,
            sample_entry: None,
        },
    );
    let media = Media::new(vec![Track::new(spec, vec![])], 1000);
    assert!(SmoothPackager::default().package(&media).is_err());
}
