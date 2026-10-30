//! Hostile MPD corpus: every bound is a typed error, nothing panics, nothing
//! allocates in proportion to a claimed size.

mod common;

use common::Rng;
use dash_runtime::address::{PlanOptions, plan_representation};
use dash_runtime::error::{Limit, MpdError, Unsupported};
use dash_runtime::limits::Limits;
use dash_runtime::mpd::Mpd;
use dash_runtime::selection::{Policy, RepRef, select};

const NS: &str = "urn:mpeg:dash:schema:mpd:2011";

fn wrap(inner: &str) -> String {
    format!(r#"<MPD xmlns="{NS}" type="static">{inner}</MPD>"#)
}

fn parse(s: &str) -> Result<Mpd, MpdError> {
    Mpd::parse(s.as_bytes())
}

fn too_large(r: Result<Mpd, MpdError>) -> Limit {
    match r {
        Err(MpdError::TooLarge { limit, .. }) => limit,
        other => panic!("expected TooLarge, got {other:?}"),
    }
}

#[test]
fn billion_laughs_is_refused_before_any_expansion() {
    let xml = r#"<?xml version="1.0"?>
<!DOCTYPE lolz [
 <!ENTITY lol "lol">
 <!ENTITY lol2 "&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;">
 <!ENTITY lol3 "&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;">
]>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011"><BaseURL>&lol3;</BaseURL></MPD>"#;
    match parse(xml) {
        Err(MpdError::Xml { message, .. }) => assert!(message.contains("DOCTYPE"), "{message}"),
        other => panic!("{other:?}"),
    }
    let external =
        r#"<!DOCTYPE MPD SYSTEM "file:///etc/passwd"><MPD xmlns="urn:mpeg:dash:schema:mpd:2011"/>"#;
    assert!(matches!(parse(external), Err(MpdError::Xml { .. })));
    assert!(matches!(
        parse(&wrap("<BaseURL>&undefined;</BaseURL>")),
        Err(MpdError::Xml { .. })
    ));
    assert_eq!(
        parse(&wrap("<BaseURL>a&amp;b&#65;</BaseURL>"))
            .unwrap()
            .base_urls[0]
            .url,
        "a&bA"
    );
}

#[test]
fn element_count_bound() {
    let ok = wrap(&"<Foo/>".repeat(99_998));
    assert!(parse(&ok).is_ok());
    let bad = wrap(&"<Foo/>".repeat(100_000));
    assert_eq!(too_large(parse(&bad)), Limit::Elements);
}

#[test]
fn depth_bound() {
    let nest = |n: usize| {
        let mut s = String::new();
        for _ in 0..n {
            s.push_str("<Foo>");
        }
        for _ in 0..n {
            s.push_str("</Foo>");
        }
        wrap(&s)
    };
    assert!(parse(&nest(30)).is_ok(), "MPD + 30 = depth 31");
    assert!(
        parse(&nest(31)).is_ok(),
        "MPD + 31 = depth 32, exactly the limit"
    );
    assert_eq!(too_large(parse(&nest(32))), Limit::Depth);
    assert_eq!(too_large(parse(&nest(5_000))), Limit::Depth);
}

#[test]
fn attribute_bounds() {
    let long = |n: usize| {
        wrap(&format!(
            r#"<BaseURL serviceLocation="{}">x</BaseURL>"#,
            "a".repeat(n)
        ))
    };
    assert!(parse(&long(8 * 1024)).is_ok());
    assert_eq!(too_large(parse(&long(8 * 1024 + 1))), Limit::AttributeBytes);
    let many = |n: usize| {
        let attrs: String = (0..n).map(|i| format!(r#" a{i}="1""#)).collect();
        wrap(&format!("<Foo{attrs}/>"))
    };
    assert!(parse(&many(256)).is_ok());
    assert_eq!(too_large(parse(&many(257))), Limit::Attributes);
}

#[test]
fn structure_bounds() {
    assert_eq!(
        too_large(parse(&wrap(&"<Period/>".repeat(1_001)))),
        Limit::Periods
    );
    assert!(parse(&wrap(&"<Period/>".repeat(1_000))).is_ok());
    let sets = format!("<Period>{}</Period>", "<AdaptationSet/>".repeat(1_001));
    assert_eq!(too_large(parse(&wrap(&sets))), Limit::AdaptationSets);
    let reps = format!(
        "<Period><AdaptationSet>{}</AdaptationSet></Period>",
        (0..1_001)
            .map(|i| format!(r#"<Representation id="{i}" bandwidth="1"/>"#))
            .collect::<String>()
    );
    assert_eq!(too_large(parse(&wrap(&reps))), Limit::Representations);
    assert_eq!(
        too_large(parse(&wrap(&"<BaseURL>x</BaseURL>".repeat(65)))),
        Limit::BaseUrls
    );
    let tl = format!(
        r#"<Period><AdaptationSet><Representation id="r" bandwidth="1"><SegmentTemplate media="$Number$" duration="1"><SegmentTimeline>{}</SegmentTimeline></SegmentTemplate></Representation></AdaptationSet></Period>"#,
        r#"<S d="1"/>"#.repeat(100_001)
    );
    let r = Mpd::parse_with(wrap(&tl).as_bytes(), &Limits::default());
    assert!(matches!(
        r,
        Err(MpdError::TooLarge {
            limit: Limit::TimelineEntries | Limit::Elements,
            ..
        })
    ));
    let mut loose_elements = Limits::default();
    loose_elements.max_elements = 200_000;
    loose_elements.max_timeline_entries = 100_000;
    assert_eq!(
        too_large(Mpd::parse_with(wrap(&tl).as_bytes(), &loose_elements)),
        Limit::TimelineEntries
    );
    let urls = format!(
        r#"<Period><AdaptationSet><Representation id="r" bandwidth="1"><SegmentList duration="1">{}</SegmentList></Representation></AdaptationSet></Period>"#,
        r#"<SegmentURL media="a"/>"#.repeat(100_001)
    );
    assert!(matches!(
        Mpd::parse(wrap(&urls).as_bytes()),
        Err(MpdError::TooLarge {
            limit: Limit::SegmentUrls | Limit::Elements,
            ..
        })
    ));
    let mut loose_elements = Limits::default();
    loose_elements.max_elements = 200_000;
    loose_elements.max_segment_urls = 100_000;
    assert_eq!(
        too_large(Mpd::parse_with(wrap(&urls).as_bytes(), &loose_elements)),
        Limit::SegmentUrls
    );
}

#[test]
fn input_size_bound_and_custom_limits() {
    let big = vec![b' '; 4 * 1024 * 1024 + 1];
    assert!(matches!(
        Mpd::parse(&big),
        Err(MpdError::TooLarge {
            limit: Limit::InputBytes,
            ..
        })
    ));
    let mut tight = Limits::default();
    tight.max_periods = 1;
    assert!(matches!(
        Mpd::parse_with(wrap("<Period/><Period/>").as_bytes(), &tight),
        Err(MpdError::TooLarge {
            limit: Limit::Periods,
            max: 1
        })
    ));
}

#[test]
fn byte_level_garbage() {
    assert!(matches!(
        Mpd::parse(&[b'<', b'M', 0xff, 0xfe, b'>']),
        Err(MpdError::InvalidUtf8 { valid_up_to: 2 })
    ));
    assert!(matches!(
        parse(&wrap("<BaseURL>a\u{0}b</BaseURL>")),
        Err(MpdError::Xml { .. })
    ));
    assert!(matches!(
        parse(&wrap("<BaseURL serviceLocation=\"a\u{0}b\">x</BaseURL>")),
        Err(MpdError::Xml { .. })
    ));
    assert!(matches!(
        parse(&wrap("<BaseURL>&#0;</BaseURL>")),
        Err(MpdError::Xml { .. })
    ));
    assert!(matches!(parse(""), Err(MpdError::Xml { .. })));
    assert!(matches!(parse("not xml at all"), Err(MpdError::Xml { .. })));
    assert!(matches!(
        parse(&format!(r#"<MPD xmlns="{NS}">"#)),
        Err(MpdError::Xml { .. })
    ));
    assert!(matches!(
        parse(&format!(r#"<MPD xmlns="{NS}"></Foo>"#)),
        Err(MpdError::Xml { .. })
    ));
    assert!(matches!(
        parse(&format!(r#"<MPD xmlns="{NS}"/><MPD xmlns="{NS}"/>"#)),
        Err(MpdError::Xml { .. })
    ));
    assert!(matches!(
        parse(&format!(r#"junk<MPD xmlns="{NS}"/>"#)),
        Err(MpdError::Xml { .. })
    ));
}

#[test]
fn not_an_mpd() {
    assert!(matches!(parse("<html/>"), Err(MpdError::NotMpd { .. })));
    assert!(matches!(
        parse(r#"<MPD xmlns="urn:other"/>"#),
        Err(MpdError::NotMpd { found }) if found == "MPD"
    ));
    assert!(matches!(parse("<MPD/>"), Err(MpdError::NotMpd { .. })));
    assert!(matches!(
        parse(&format!(r#"<Other xmlns="{NS}"/>"#)),
        Err(MpdError::NotMpd { .. })
    ));
}

#[test]
fn schema_violations_are_typed_not_panics() {
    for (what, xml) in [
        (
            "bad duration",
            r#"<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" mediaPresentationDuration="soon"/>"#,
        ),
        (
            "bad type",
            r#"<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="live"/>"#,
        ),
        (
            "bad time",
            r#"<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" availabilityStartTime="yesterday"/>"#,
        ),
    ] {
        assert!(
            matches!(parse(xml), Err(MpdError::Invalid { .. })),
            "{what}"
        );
    }
    assert!(matches!(
        parse(r#"<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="list"/>"#),
        Err(MpdError::Unsupported(Unsupported::PresentationTypeList))
    ));
    let rep = |attrs: &str| {
        wrap(&format!(
            r#"<Period><AdaptationSet><Representation {attrs}/></AdaptationSet></Period>"#
        ))
    };
    assert!(matches!(
        parse(&rep(r#"bandwidth="1""#)),
        Err(MpdError::Invalid { .. })
    ));
    assert!(matches!(
        parse(&rep(r#"id="x""#)),
        Err(MpdError::Invalid { .. })
    ));
    assert!(matches!(
        parse(&rep(r#"id="x" bandwidth="-1""#)),
        Err(MpdError::Invalid { .. })
    ));
    assert!(matches!(
        parse(&rep(r#"id="x" bandwidth="99999999999999999999999""#)),
        Err(MpdError::Invalid { .. })
    ));
    let seg = |inner: &str| {
        wrap(&format!(
            r#"<Period><AdaptationSet><Representation id="x" bandwidth="1">{inner}</Representation></AdaptationSet></Period>"#
        ))
    };
    assert!(matches!(
        parse(&seg(r#"<SegmentTemplate duration="0"/>"#)),
        Err(MpdError::Invalid { .. })
    ));
    assert!(matches!(
        parse(&seg(r#"<SegmentTemplate timescale="0"/>"#)),
        Err(MpdError::Invalid { .. })
    ));
    assert!(matches!(
        parse(&seg(
            r#"<SegmentTemplate><SegmentTimeline><S d="0"/></SegmentTimeline></SegmentTemplate>"#
        )),
        Err(MpdError::Invalid { .. })
    ));
    assert!(matches!(
        parse(&seg(
            r#"<SegmentTemplate><SegmentTimeline><S/></SegmentTimeline></SegmentTemplate>"#
        )),
        Err(MpdError::Invalid { .. })
    ));
    assert!(matches!(
        parse(&seg(r#"<SegmentBase indexRange="9-3"/>"#)),
        Err(MpdError::Invalid { .. })
    ));
    assert!(matches!(
        parse(&seg(r#"<SegmentBase indexRange="abc"/>"#)),
        Err(MpdError::Invalid { .. })
    ));
    assert!(matches!(
        parse(&seg(r#"<SegmentBase/><SegmentTemplate/>"#)),
        Err(MpdError::Invalid { .. })
    ));
}

#[test]
fn huge_timeline_repeat_is_bounded_lazily() {
    let xml = wrap(
        r#"<Period><AdaptationSet mimeType="video/mp4"><Representation id="r" bandwidth="1">
           <SegmentTemplate timescale="1" media="$Time$"><SegmentTimeline><S t="0" d="1" r="9223372036854775807"/></SegmentTimeline></SegmentTemplate>
           </Representation></AdaptationSet></Period>"#,
    );
    let mpd = parse(&xml).unwrap();
    assert!(plan_representation(&mpd, RepRef::at(0, 0, 0), &PlanOptions::default()).is_err());
}

#[test]
fn foreign_content_is_skipped_but_counted() {
    let xml = format!(
        r#"<MPD xmlns="{NS}" xmlns:x="urn:vendor" type="static"><x:Blob>{}</x:Blob><BaseURL>b/</BaseURL></MPD>"#,
        "<x:i/>".repeat(1_000)
    );
    let m = parse(&xml).unwrap();
    assert_eq!(m.base_urls.len(), 1);
}

#[test]
fn resolve_to_zero_xlink_removes_the_element() {
    let xml = wrap(
        r#"<Period xlink:href="urn:mpeg:dash:resolve-to-zero:2013" xmlns:xlink="http://www.w3.org/1999/xlink"/><Period id="keep"/>"#,
    );
    let m = parse(&xml).unwrap();
    assert_eq!(m.periods.len(), 1);
    assert_eq!(m.periods[0].id.as_deref(), Some("keep"));
    assert!(m.features.xlink_resolve_to_zero);
    let remote = wrap(
        r#"<Period xlink:href="http://x/p.xml" xlink:actuate="onLoad" xmlns:xlink="http://www.w3.org/1999/xlink"/>"#,
    );
    let m = parse(&remote).unwrap();
    assert_eq!(m.periods[0].xlink.as_ref().unwrap().href, "http://x/p.xml");
}

fn mutate(rng: &mut Rng, src: &[u8]) -> Vec<u8> {
    let mut b = src.to_vec();
    for _ in 0..=rng.below(4) {
        if b.is_empty() {
            break;
        }
        let at = usize::try_from(rng.below(b.len() as u64)).unwrap();
        match rng.below(5) {
            0 => b[at] = u8::try_from(rng.below(256)).unwrap(),
            1 => b[at] ^= 1 << rng.below(8),
            2 => {
                b.remove(at);
            }
            3 => b.insert(
                at,
                b"<>&\"'/=$%0123456789"[usize::try_from(rng.below(19)).unwrap()],
            ),
            _ => b.truncate(at.max(1)),
        }
    }
    b
}

#[test]
fn mutated_fixtures_never_panic() {
    let mut rng = Rng(0xDEAD_BEEF);
    let mut files = Vec::new();
    for entry in walkdir(&test_fixtures::path("dash")) {
        files.push(entry);
    }
    let (mut parsed, mut planned) = (0usize, 0usize);
    for f in &files {
        let src = std::fs::read(f).unwrap();
        for _ in 0..400 {
            let m = mutate(&mut rng, &src);
            let Ok(mpd) = Mpd::parse(&m) else { continue };
            parsed += 1;
            for (pi, p) in mpd.periods.iter().enumerate() {
                let _ = select(&mpd, pi, &Policy::default());
                for (si, s) in p.adaptation_sets.iter().enumerate() {
                    for ri in 0..s.representations.len() {
                        if let Ok(pl) = plan_representation(
                            &mpd,
                            RepRef::at(pi, si, ri),
                            &PlanOptions::default(),
                        ) {
                            planned += 1;
                            let _ = pl.plan.get(0);
                            let _ = pl.plan.iter().take(50).count();
                            let _ = pl.plan.init();
                        }
                    }
                }
            }
        }
    }
    eprintln!("mutants: {parsed} parsed, {planned} planned");
    assert!(parsed > 1_000, "only {parsed} mutants parsed");
    assert!(planned > 500, "only {planned} planned");
}

fn walkdir(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            out.extend(walkdir(&p));
        } else if p.extension().is_some_and(|x| x == "mpd") {
            out.push(p);
        }
    }
    out.sort();
    out
}
