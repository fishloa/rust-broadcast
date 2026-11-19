//! Attribute parsing edges: finite `xs:double`, byte ranges, the
//! resolve-to-zero xlink, and which descriptor child a typed field reads.

use broadcast_dash::Mpd;
use broadcast_dash::error::MpdError;
use broadcast_dash::model::ByteRange;

const HEAD: &str = r#"<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" xmlns:xlink="http://www.w3.org/1999/xlink" type="static" minBufferTime="PT1S" mediaPresentationDuration="PT1S">"#;
const TAIL: &str = "</MPD>";

fn parse(body: &str) -> Mpd {
    let xml = format!("{HEAD}{body}{TAIL}");
    Mpd::parse(xml.as_bytes()).unwrap_or_else(|e| panic!("{xml}: {e}"))
}

fn error(body: &str) -> MpdError {
    let xml = format!("{HEAD}{body}{TAIL}");
    match Mpd::parse(xml.as_bytes()) {
        Ok(mpd) => panic!("{xml} parsed: {mpd:?}"),
        Err(e) => e,
    }
}

#[test]
fn a_non_finite_xs_double_is_refused() {
    for bad in ["NaN", "inf", "-inf", "Infinity", "nan"] {
        let e = error(&format!(
            r#"<PatchLocation ttl="{bad}">http://p/</PatchLocation>"#
        ));
        assert!(
            matches!(&e, MpdError::Invalid { what: "ttl", .. }),
            "{bad}: {e:?}"
        );
        let e = error(&format!(
            r#"<Period id="0"><AdaptationSet mimeType="video/mp4"><Representation id="r" bandwidth="1"><SegmentBase availabilityTimeOffset="{bad}"/></Representation></AdaptationSet></Period>"#
        ));
        assert!(
            matches!(
                &e,
                MpdError::Invalid {
                    what: "availabilityTimeOffset",
                    ..
                }
            ),
            "{bad}: {e:?}"
        );
    }
}

#[test]
fn the_infinite_tokens_are_accepted() {
    let mpd = parse(
        r#"<Period id="0"><AdaptationSet mimeType="video/mp4"><Representation id="r" bandwidth="1"><SegmentBase availabilityTimeOffset="INF"/></Representation></AdaptationSet></Period>"#,
    );
    let base = mpd.periods[0].adaptation_sets[0].representations[0]
        .segment
        .base
        .as_ref()
        .unwrap();
    assert_eq!(base.info.availability_time_offset, Some(f64::INFINITY));

    let mpd = parse(
        r#"<Period id="0"><AdaptationSet mimeType="video/mp4"><Representation id="r" bandwidth="1"><SegmentBase availabilityTimeOffset="-INF"/></Representation></AdaptationSet></Period>"#,
    );
    let base = mpd.periods[0].adaptation_sets[0].representations[0]
        .segment
        .base
        .as_ref()
        .unwrap();
    assert_eq!(base.info.availability_time_offset, Some(f64::NEG_INFINITY));
}

fn segment_base(index_range: &str) -> Mpd {
    parse(&format!(
        r#"<Period id="0"><AdaptationSet mimeType="video/mp4"><Representation id="r" bandwidth="1"><SegmentBase indexRange="{index_range}"/></Representation></AdaptationSet></Period>"#
    ))
}

fn index_range(mpd: &Mpd) -> Option<ByteRange> {
    mpd.periods[0].adaptation_sets[0].representations[0]
        .segment
        .base
        .as_ref()
        .unwrap()
        .info
        .index_range
}

#[test]
fn a_byte_range_is_inclusive_and_may_be_open_ended() {
    let mpd = segment_base("5-5");
    assert_eq!(index_range(&mpd), Some(ByteRange::new(5, 5)));
    let mpd = segment_base("5-9");
    assert_eq!(index_range(&mpd), Some(ByteRange::new(5, 9)));
    let mpd = segment_base("5-");
    let mut open = ByteRange::default();
    open.first = 5;
    assert_eq!(index_range(&mpd), Some(open));
}

#[test]
fn an_inverted_or_malformed_byte_range_is_refused() {
    for bad in ["9-3", "5", "-9", "a-9", "5-b", "5-9-1"] {
        let e = error(&format!(
            r#"<Period id="0"><AdaptationSet mimeType="video/mp4"><Representation id="r" bandwidth="1"><SegmentBase indexRange="{bad}"/></Representation></AdaptationSet></Period>"#
        ));
        assert!(
            matches!(
                &e,
                MpdError::Invalid {
                    what: "indexRange",
                    ..
                }
            ),
            "{bad}: {e:?}"
        );
    }
}

#[test]
fn a_resolve_to_zero_xlink_removes_its_element() {
    let mpd = parse(r#"<Period id="0" xlink:href="urn:mpeg:dash:resolve-to-zero:2013"/>"#);
    assert!(mpd.periods.is_empty());
    assert!(mpd.features.xlink_resolve_to_zero);

    let mpd = parse(
        r#"<Period id="0"><AdaptationSet mimeType="video/mp4" xlink:href="urn:mpeg:dash:resolve-to-zero:2013"/></Period>"#,
    );
    assert_eq!(mpd.periods.len(), 1);
    assert!(mpd.periods[0].adaptation_sets.is_empty());
    assert!(mpd.features.xlink_resolve_to_zero);

    let mpd = parse(r#"<Period id="0" xlink:href="http://example/period"/>"#);
    assert_eq!(mpd.periods.len(), 1);
    assert!(!mpd.features.xlink_resolve_to_zero);
    assert_eq!(
        mpd.periods[0].xlink.as_ref().map(|x| x.href.as_str()),
        Some("http://example/period")
    );
}

#[test]
fn pssh_and_url_query_info_read_their_own_namespace_and_name() {
    let mpd = parse(
        r#"<Period id="0"><AdaptationSet mimeType="video/mp4"><ContentProtection schemeIdUri="urn:mpeg:dash:mp4protection:2011"><pssh xmlns="urn:not-cenc">AAAA</pssh><cenc:padding xmlns:cenc="urn:mpeg:cenc:2013">BBBB</cenc:padding></ContentProtection></AdaptationSet></Period>"#,
    );
    let cp = &mpd.periods[0].adaptation_sets[0].content_protections[0];
    assert_eq!(cp.pssh, None);

    let mpd = parse(
        r#"<Period id="0"><AdaptationSet mimeType="video/mp4"><ContentProtection schemeIdUri="urn:mpeg:dash:mp4protection:2011"><cenc:pssh xmlns:cenc="urn:mpeg:cenc:2013"> AAAA </cenc:pssh></ContentProtection></AdaptationSet></Period>"#,
    );
    let cp = &mpd.periods[0].adaptation_sets[0].content_protections[0];
    assert_eq!(cp.pssh.as_deref(), Some("AAAA"));
}

#[test]
fn url_query_info_reads_its_own_namespace_and_name() {
    let mpd = parse(
        r#"<Period id="0"><AdaptationSet mimeType="video/mp4"><EssentialProperty schemeIdUri="urn:x"><UrlQueryInfo xmlns="urn:mpeg:dash:schema:urlparam:2014" useMPDUrlQuery="true"/></EssentialProperty></AdaptationSet></Period>"#,
    );
    let desc = &mpd.periods[0].adaptation_sets[0].essential_properties[0];
    assert_eq!(
        desc.url_query.as_ref().map(|q| q.use_mpd_url_query),
        Some(true)
    );

    let mpd = parse(
        r#"<Period id="0"><AdaptationSet mimeType="video/mp4"><EssentialProperty schemeIdUri="urn:x"><UrlQueryInfo useMPDUrlQuery="true"/></EssentialProperty></AdaptationSet></Period>"#,
    );
    let desc = &mpd.periods[0].adaptation_sets[0].essential_properties[0];
    assert_eq!(desc.url_query, None);

    let mpd = parse(
        r#"<Period id="0"><AdaptationSet mimeType="video/mp4"><EssentialProperty schemeIdUri="urn:x"><Foo xmlns="urn:mpeg:dash:schema:urlparam:2014" useMPDUrlQuery="true"/></EssentialProperty></AdaptationSet></Period>"#,
    );
    let desc = &mpd.periods[0].adaptation_sets[0].essential_properties[0];
    assert_eq!(desc.url_query, None);
}
