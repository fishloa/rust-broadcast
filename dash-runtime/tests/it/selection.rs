//! AdaptationSet / Representation selection (DESIGN 5.1).

use crate::common;

use common::load;
use dash_runtime::mpd::Mpd;
use dash_runtime::selection::{
    DropReason, Policy, RepRef, SetClass, classify, lang_matches, select,
};

const NS: &str = "urn:mpeg:dash:schema:mpd:2011";

fn mpd(period_body: &str) -> Mpd {
    let xml = format!(
        r#"<MPD xmlns="{NS}" xmlns:cenc="urn:mpeg:cenc:2013" type="static" mediaPresentationDuration="PT10S"><Period>{period_body}</Period></MPD>"#
    );
    Mpd::parse(xml.as_bytes()).unwrap_or_else(|e| panic!("{e}"))
}

const TPL: &str =
    r#"<SegmentTemplate timescale="1" duration="2" media="$RepresentationID$-$Number$.m4s"/>"#;

fn set(attrs: &str, inner: &str, reps: &[(&str, u64, &str)]) -> String {
    let reps: String = reps
        .iter()
        .map(|(id, bw, extra)| format!(r#"<Representation id="{id}" bandwidth="{bw}" {extra}/>"#))
        .collect();
    format!(r#"<AdaptationSet {attrs}>{inner}{TPL}{reps}</AdaptationSet>"#)
}

fn drops(sel: &dash_runtime::selection::Selection) -> Vec<(usize, Option<usize>, DropReason)> {
    sel.dropped
        .iter()
        .map(|d| (d.set, d.rep, d.reason))
        .collect()
}

#[test]
fn real_fixtures_select_one_video_and_one_audio() {
    for rel in [
        "dash/manifest.mpd",
        "dash/timeline-gpac/timeline.mpd",
        "dash/segmentlist/list.mpd",
        "dash/livesim2/segtimeline_time_a.mpd",
        "dash/live-number/live.mpd",
    ] {
        let l = load(rel);
        let sel = select(&l.mpd, 0, &Policy::default());
        assert_eq!(sel.variants.len(), 1, "{rel}");
        assert_eq!(sel.chosen_variant, Some(0), "{rel}");
        assert!(sel.chosen_audio.is_some(), "{rel}");
        assert!(sel.dropped.is_empty(), "{rel}: {:?}", sel.dropped);
        let v = sel.variants[0].resolve(&l.mpd).unwrap();
        assert_eq!(classify(v.1), SetClass::Video);
    }
}

#[test]
fn every_fixture_selects_without_panicking_and_never_selects_text() {
    let root = test_fixtures::path("dash");
    let mut stack = vec![root];
    let mut seen = 0;
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "mpd") {
                let m = Mpd::parse(&std::fs::read(&p).unwrap()).unwrap();
                for pi in 0..m.periods.len() {
                    let sel = select(&m, pi, &Policy::default());
                    for v in &sel.variants {
                        let (_, s, _) = v.resolve(&m).unwrap();
                        assert_eq!(classify(s), SetClass::Video, "{}", p.display());
                    }
                    for a in &sel.audio_sets {
                        let (_, s, _) = a.reps[0].resolve(&m).unwrap();
                        assert_eq!(classify(s), SetClass::Audio, "{}", p.display());
                    }
                }
                seen += 1;
            }
        }
    }
    assert!(seen >= 24);
}

#[test]
fn video_picks_highest_bandwidth_within_policy_else_lowest() {
    let m = mpd(&set(
        r#"contentType="video" mimeType="video/mp4""#,
        "",
        &[
            ("lo", 500_000, r#"height="360" codecs="avc1.4d401e""#),
            ("mid", 1_500_000, r#"height="540" codecs="avc1.4d401f""#),
            ("hi", 4_000_000, r#"height="1080" codecs="hvc1.1.6.L120""#),
        ],
    ));
    let pick = |p: &Policy| {
        let s = select(&m, 0, p);
        assert_eq!(
            s.variants.len(),
            3,
            "variants lists every Representation in MPD order"
        );
        s.chosen_variant
    };
    assert_eq!(pick(&Policy::default()), Some(2));
    let mut p = Policy::default();
    p.max_bandwidth = Some(2_000_000);
    assert_eq!(pick(&p), Some(1));
    p.max_bandwidth = None;
    p.max_height = Some(540);
    assert_eq!(pick(&p), Some(1));
    p.max_height = None;
    p.codec_prefixes = vec!["avc1".to_owned()];
    assert_eq!(pick(&p), Some(1));
    p.codec_prefixes = vec!["vp09".to_owned()];
    assert_eq!(pick(&p), Some(0), "nothing allowed: the lowest bandwidth");
    p.codec_prefixes.clear();
    p.max_bandwidth = Some(1);
    assert_eq!(pick(&p), Some(0));
}

#[test]
fn video_prefers_the_main_role_set() {
    let role = |v: &str| format!(r#"<Role schemeIdUri="urn:mpeg:dash:role:2011" value="{v}"/>"#);
    let m = mpd(&format!(
        "{}{}",
        set(
            r#"contentType="video" mimeType="video/mp4""#,
            &role("alternate"),
            &[("alt", 9, "")]
        ),
        set(
            r#"contentType="video" mimeType="video/mp4""#,
            &role("main"),
            &[("main", 1, "")]
        ),
    ));
    let s = select(&m, 0, &Policy::default());
    assert_eq!(s.variants.len(), 2);
    assert_eq!(s.chosen_variant, Some(1));
    assert_eq!(s.variants[1], RepRef::at(0, 1, 0));
}

#[test]
fn audio_language_role_and_fallbacks() {
    let role = |v: &str| format!(r#"<Role schemeIdUri="urn:mpeg:dash:role:2011" value="{v}"/>"#);
    let m = mpd(&format!(
        "{}{}{}{}",
        set(
            r#"contentType="audio" mimeType="audio/mp4" lang="fr""#,
            &role("main"),
            &[("fr", 64_000, "")]
        ),
        set(
            r#"contentType="audio" mimeType="audio/mp4" lang="en-GB""#,
            &role("commentary"),
            &[("en-com", 64_000, "")]
        ),
        set(
            r#"contentType="audio" mimeType="audio/mp4" lang="en-US""#,
            &role("alternate"),
            &[("en-lo", 32_000, ""), ("en-hi", 96_000, "")]
        ),
        set(
            r#"contentType="audio" mimeType="audio/mp4" lang="de""#,
            "",
            &[("de", 64_000, "")]
        ),
    ));
    let chosen = |lang: Option<&str>| {
        let mut p = Policy::default();
        p.preferred_audio_language = lang.map(str::to_owned);
        let s = select(&m, 0, &p);
        let r = s.chosen_audio.unwrap();
        r.resolve(&m).unwrap().2.id.clone()
    };
    assert_eq!(
        chosen(Some("en")),
        "en-hi",
        "prefix match, skipping the commentary set, best bandwidth"
    );
    assert_eq!(chosen(Some("EN")), "en-hi", "case-insensitive");
    assert_eq!(chosen(Some("en-US")), "en-hi");
    assert_eq!(chosen(Some("de")), "de");
    assert_eq!(chosen(Some("ja")), "fr", "no match: Role main");
    assert_eq!(chosen(None), "fr");
    let no_main = mpd(&set(
        r#"contentType="audio" mimeType="audio/mp4""#,
        "",
        &[("only", 1, "")],
    ));
    assert_eq!(
        select(&no_main, 0, &Policy::default())
            .chosen_audio
            .unwrap()
            .resolve(&no_main)
            .unwrap()
            .2
            .id,
        "only"
    );
    assert!(lang_matches("en", "en-GB"));
    assert!(lang_matches("en-GB", "en-gb"));
    assert!(!lang_matches("en", "eng"));
    assert!(!lang_matches("en-GB", "en"));
}

#[test]
fn drops_and_their_reasons() {
    let prop = |scheme: &str| format!(r#"<EssentialProperty schemeIdUri="{scheme}" value="1"/>"#);
    let xml = [
        set(r#"contentType="video" mimeType="video/mp4" group="1""#, "", &[("ok", 1, "")]),
        set(r#"contentType="text" mimeType="application/mp4" codecs="stpp""#, "", &[("sub", 1, "")]),
        set(r#"contentType="video" mimeType="video/mp2t""#, "", &[("ts", 1, "")]),
        set(r#"contentType="video" mimeType="video/webm""#, "", &[("wm", 1, "")]),
        set(
            r#"contentType="video" mimeType="video/mp4""#,
            &prop("http://dashif.org/guidelines/trickmode"),
            &[("trick", 1, "")],
        ),
        set(
            r#"contentType="video" mimeType="video/mp4" group="1""#,
            r#"<ContentProtection schemeIdUri="urn:mpeg:dash:mp4protection:2011" value="cenc" cenc:default_KID="00000000-0000-0000-0000-000000000000"/>"#,
            &[("drm-alt", 1, "")],
        ),
        set(
            r#"contentType="audio" mimeType="audio/mp4" group="2""#,
            r#"<ContentProtection schemeIdUri="urn:mpeg:dash:mp4protection:2011" value="cenc"/>"#,
            &[("drm-only", 1, "")],
        ),
        set(r#"contentType="video" mimeType="video/mp4""#, &prop("urn:vendor:unknown"), &[("ess", 1, "")]),
        set(
            r#"contentType="video" mimeType="video/mp4""#,
            "",
            &[("base", 1, ""), ("dep", 1, r#"dependencyId="base""#), ("repess", 1, "")],
        ),
        r#"<AdaptationSet xmlns:xlink="http://www.w3.org/1999/xlink" xlink:href="http://x/set.xml" contentType="video"/>"#.to_owned(),
        set(r#"contentType="metadata""#, "", &[("meta", 1, "")]),
    ]
    .concat();
    let m = mpd(&xml);
    let s = select(&m, 0, &Policy::default());
    let d = drops(&s);
    assert!(d.contains(&(1, None, DropReason::Text)), "{d:?}");
    assert!(d.contains(&(2, None, DropReason::Mp2tSegments)));
    assert!(d.contains(&(3, None, DropReason::WebM)));
    assert!(d.contains(&(4, None, DropReason::TrickMode)));
    assert!(d.contains(&(5, None, DropReason::ProtectedAlternativeExists)));
    assert!(d.contains(&(6, None, DropReason::UnsupportedEncryption)));
    assert!(d.contains(&(7, None, DropReason::UnknownEssentialProperty)));
    assert!(d.contains(&(8, Some(1), DropReason::DependencyRepresentation)));
    assert!(d.contains(&(9, None, DropReason::UnresolvedXlink)));
    assert!(d.contains(&(10, None, DropReason::NotMedia)));
    let kept: Vec<_> = s
        .variants
        .iter()
        .map(|v| v.resolve(&m).unwrap().2.id.clone())
        .collect();
    assert_eq!(kept, vec!["ok", "base", "repess"]);
    assert!(s.audio_sets.is_empty() && s.chosen_audio.is_none());
}

#[test]
fn invalid_segment_info_drops_only_that_representation() {
    let m = mpd(&format!(
        r#"<AdaptationSet contentType="video" mimeType="video/mp4">{TPL}
           <Representation id="good" bandwidth="1"/>
           <Representation id="bad" bandwidth="2"><SegmentTemplate media="$Nope$"/></Representation>
           <Representation id="seq" bandwidth="3"><SegmentTemplate media="$Number$-$SubNumber$"/></Representation>
           <Representation id="k" bandwidth="4"><SegmentTemplate><SegmentTimeline><S t="0" d="2" k="2"/></SegmentTimeline></SegmentTemplate></Representation>
           </AdaptationSet>"#
    ));
    let s = select(&m, 0, &Policy::default());
    assert_eq!(s.variants.len(), 1);
    let bad = s
        .dropped
        .iter()
        .filter(|d| d.reason == DropReason::InvalidSegmentInfo)
        .count();
    assert_eq!(bad, 3);
}

#[test]
fn unknown_period_selects_nothing() {
    let m = mpd("");
    let s = select(&m, 7, &Policy::default());
    assert!(s.variants.is_empty() && s.chosen_variant.is_none());
}

#[test]
fn codec_policy_is_applied_across_adaptation_sets() {
    let m = mpd(&format!(
        "{}{}",
        set(
            r#"contentType="video" mimeType="video/mp4""#,
            "",
            &[("hvc", 1_000_000, r#"codecs="hvc1.1.6.L120""#)]
        ),
        set(
            r#"contentType="video" mimeType="video/mp4""#,
            "",
            &[("avc", 2_000_000, r#"codecs="avc1.4d401f""#)]
        ),
    ));
    let mut p = Policy::default();
    p.codec_prefixes = vec!["avc1".to_owned()];
    let s = select(&m, 0, &p);
    let idx = s.chosen_variant.unwrap();
    let rep = s.variants[idx].resolve(&m).unwrap().2.id.clone();
    assert_eq!(rep, "avc", "chosen video rep should be the avc one");

    let ma = mpd(&format!(
        "{}{}",
        set(
            r#"contentType="audio" mimeType="audio/mp4" lang="en""#,
            "",
            &[("ec", 64_000, r#"codecs="ec-3""#)]
        ),
        set(
            r#"contentType="audio" mimeType="audio/mp4" lang="en""#,
            "",
            &[("aac", 64_000, r#"codecs="mp4a.40.2""#)]
        ),
    ));
    let mut pa = Policy::default();
    pa.codec_prefixes = vec!["mp4a".to_owned()];
    let sa = select(&ma, 0, &pa);
    let r = sa.chosen_audio.unwrap().resolve(&ma).unwrap().2.id.clone();
    assert_eq!(r, "aac");
}
