//! `dash_runtime::mpd::Mpd` on the real DASH fixtures: the parsed structure,
//! the SegmentTemplate each level merges in, and how an empty BaseURL drops
//! out of the chain. Ported from transmux's second reader (issue #2340).

use broadcast_common::timescale::Timescale;

fn timescale(value: u32) -> Timescale {
    Timescale::try_new(value).expect("a non-zero timescale")
}
use dash_runtime::mpd::PresentationType;
use dash_runtime::mpd::inherit::SegmentInfo;
use dash_runtime::mpd::inherit::effective_common;
use dash_runtime::mpd::inherit::resolve_segment_info;

const NS: &str = "urn:mpeg:dash:schema:mpd:2011";

fn load(rel: &str) -> dash_runtime::mpd::Mpd {
    let bytes = std::fs::read(test_fixtures::path(rel)).expect("fixture");
    dash_runtime::mpd::Mpd::parse(&bytes).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn parse_str(xml: &str) -> Result<dash_runtime::mpd::Mpd, dash_runtime::error::MpdError> {
    dash_runtime::mpd::Mpd::parse(xml.as_bytes())
}

fn template(
    period: &dash_runtime::mpd::Period,
    set: &dash_runtime::mpd::AdaptationSet,
    rep: &dash_runtime::mpd::Representation,
) -> dash_runtime::mpd::inherit::TemplateInfo {
    match resolve_segment_info(period, set, rep).expect("SegmentTemplate") {
        SegmentInfo::Template(t) => t,
        other => panic!("expected a SegmentTemplate, got {other:?}"),
    }
}

#[test]
fn real_fixture_structure() {
    let mpd = load("dash/manifest.mpd");
    assert_eq!(
        mpd.profiles,
        vec!["urn:mpeg:dash:profile:isoff-live:2011".to_string()]
    );
    assert_eq!(mpd.kind, PresentationType::Static);
    assert_eq!(
        mpd.media_presentation_duration,
        Some(std::time::Duration::from_secs(3)),
        "mediaPresentationDuration=\"PT3.0S\""
    );
    assert_eq!(mpd.periods.len(), 1, "exactly one Period");

    let period = &mpd.periods[0];
    assert_eq!(period.id.as_deref(), Some("0"));
    assert_eq!(period.start, Some(std::time::Duration::ZERO));
    assert_eq!(
        period.adaptation_sets.len(),
        2,
        "exactly two AdaptationSets (video + audio)"
    );

    let video_set = period
        .adaptation_sets
        .iter()
        .find(|s| s.content_type.as_deref() == Some("video"))
        .expect("video AdaptationSet");
    let audio_set = period
        .adaptation_sets
        .iter()
        .find(|s| s.content_type.as_deref() == Some("audio"))
        .expect("audio AdaptationSet");
    assert_eq!(video_set.representations.len(), 1);
    assert_eq!(audio_set.representations.len(), 1);

    let v = &video_set.representations[0];
    assert_eq!(v.id, "0");
    assert_eq!(v.bandwidth, 58141);
    assert_eq!(v.common.codecs.as_deref(), Some("avc1.4d400d"));
    assert_eq!(v.common.mime_type.as_deref(), Some("video/mp4"));
    assert_eq!(v.common.width, Some(320));
    assert_eq!(v.common.height, Some(240));

    let v_st = template(period, video_set, v);
    assert_eq!(v_st.multi.timescale, timescale(90000));
    assert_eq!(
        v_st.initialization.as_deref(),
        Some("init-stream$RepresentationID$.m4s")
    );
    assert_eq!(
        v_st.media.as_deref(),
        Some("chunk-stream$RepresentationID$-$Number%05d$.m4s")
    );
    assert_eq!(v_st.multi.start_number, 1);
    assert_eq!(v_st.multi.duration, None, "SegmentTimeline addressing");
    let v_tl = v_st.multi.timeline.expect("video SegmentTimeline");
    assert_eq!(v_tl.entries.len(), 1, "one S run, t=2070 d=90000 r=2");
    assert_eq!(v_tl.entries[0].t, Some(2070));
    assert_eq!(v_tl.entries[0].d, 90000);
    assert_eq!(v_tl.entries[0].r, 2);

    let a = &audio_set.representations[0];
    assert_eq!(a.id, "1");
    assert_eq!(a.bandwidth, 96013);
    assert_eq!(a.common.codecs.as_deref(), Some("mp4a.40.2"));
    assert_eq!(a.common.mime_type.as_deref(), Some("audio/mp4"));
    assert_eq!(a.common.audio_sampling_rate.as_deref(), Some("44100"));

    let a_st = template(period, audio_set, a);
    assert_eq!(a_st.multi.timescale, timescale(44100));
    assert_eq!(
        a_st.media.as_deref(),
        Some("chunk-stream$RepresentationID$-$Number%05d$.m4s")
    );
    let a_tl = a_st.multi.timeline.expect("audio SegmentTimeline");
    let runs: Vec<(Option<u64>, u64, i64)> = a_tl.entries.iter().map(|e| (e.t, e.d, e.r)).collect();
    assert_eq!(
        runs,
        vec![
            (Some(0), 41984, 0),
            (None, 44032, 0),
            (None, 45056, 0),
            (None, 3072, 0),
        ],
        "four unequal-duration S runs, only the first carrying @t"
    );
}

#[test]
fn inheritance_fixture_merges_every_level_in_order() {
    let mpd = load("dash/manifest-inheritance.mpd");
    assert_eq!(mpd.periods.len(), 1);
    let period = &mpd.periods[0];
    assert_eq!(
        period.base_urls.first().map(|b| b.url.as_str()),
        Some("https://dash.example.com/vod/"),
        "MPD-level BaseURL is reported on the Period level here"
    );
    assert_eq!(period.adaptation_sets.len(), 2);

    let video = &period.adaptation_sets[0];
    let audio = &period.adaptation_sets[1];
    assert_eq!(video.content_type.as_deref(), Some("video"));
    assert_eq!(audio.content_type.as_deref(), Some("audio"));

    let v = &video.representations[0];
    assert_eq!(v.id, "0");
    let v_st = template(period, video, v);
    assert_eq!(v_st.multi.timescale, timescale(90000), "from the Period");
    assert_eq!(v_st.multi.start_number, 1, "from the Period");
    assert_eq!(
        v_st.media.as_deref(),
        Some("chunk-stream$RepresentationID$-$Number%05d$.m4s"),
        "from the AdaptationSet"
    );
    assert_eq!(
        v_st.initialization.as_deref(),
        Some("init-stream$RepresentationID$.m4s"),
        "from the Representation"
    );
    assert_eq!(v_st.multi.duration, None);
    let v_tl = v_st.multi.timeline.expect("from the Period");
    assert_eq!(v_tl.entries.len(), 1);
    assert_eq!(v_tl.entries[0].t, Some(2070));
    assert_eq!(v_tl.entries[0].d, 90000);
    assert_eq!(v_tl.entries[0].r, 2);

    let a = &audio.representations[0];
    assert_eq!(a.id, "1");
    let a_st = template(period, audio, a);
    assert_eq!(
        a_st.multi.timescale,
        timescale(44100),
        "the audio set restates @timescale"
    );
    assert_eq!(a_st.multi.start_number, 1);
    assert_eq!(
        a_st.media.as_deref(),
        Some("chunk-stream$RepresentationID$-$Number%05d$.m4s"),
        "inherited from the audio AdaptationSet"
    );
    let a_tl = a_st
        .multi
        .timeline
        .expect("the Representation's own timeline");
    assert_eq!(
        a_tl.entries.len(),
        4,
        "the Representation's own SegmentTimeline wins over the Period's"
    );
    assert_eq!(a_tl.entries[0].t, Some(0));
    assert_eq!(a_tl.entries[3].d, 3072);

    let v_effective = effective_common(video, v);
    assert_eq!(v_effective.codecs.as_deref(), Some("avc1.4d400d"));
    assert_eq!(v_effective.width, Some(320));
}

#[test]
fn empty_base_urls_are_skipped() {
    let xml = format!(
        r#"<MPD xmlns="{NS}" profiles="p">
        <BaseURL>https://cdn.example.com/</BaseURL>
        <Period>
            <BaseURL></BaseURL>
            <BaseURL>vod/</BaseURL>
            <AdaptationSet contentType="video">
                <Representation id="0" bandwidth="1">
                    <BaseURL>   </BaseURL>
                </Representation>
            </AdaptationSet>
        </Period>
    </MPD>"#
    );
    let mpd = parse_str(&xml).expect("parse");
    let period = &mpd.periods[0];
    let set = &period.adaptation_sets[0];
    let rep = &set.representations[0];
    let urls: Vec<&str> = mpd
        .base_urls
        .iter()
        .chain(period.base_urls.iter())
        .chain(rep.base_urls.iter())
        .map(|b| b.url.as_str())
        .filter(|u| !u.trim().is_empty())
        .collect();
    assert_eq!(
        urls,
        vec!["https://cdn.example.com/", "vod/"],
        "an empty (or whitespace-only) BaseURL never becomes the base"
    );
}

#[test]
fn foreign_attribute_with_a_dash_local_name_is_ignored() {
    let mpd = parse_str(&format!(
        r#"<MPD xmlns="{NS}" xmlns:ext="urn:x" profiles="p" ext:type="dynamic"/>"#
    ))
    .expect("parse");
    assert_eq!(mpd.kind, PresentationType::Static);
}
