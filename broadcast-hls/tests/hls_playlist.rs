//! Gate tests for HLS playlist output — structural invariants on the
//! generated `#EXTM3U` text.
//!
//! RFC-8216 oracle validation (via `media_doctor::check_playlist`) lives in
//! `media-doctor/tests/broadcast_hls_oracle.rs` — a cross-crate test that
//! follows the governing rule that a test touching two crates lives in the
//! topologically highest one.

use broadcast_hls::{
    CencScheme, DecimalSeconds, MasterPlaylist, MediaPlaylist, MediaSegment, Variant,
    cenc_ext_x_key,
};

#[test]
fn media_playlist_rfc_valid() {
    let pl = MediaPlaylist {
        version: 3,
        target_duration: 10,
        media_sequence: 0,
        discontinuity_sequence: 0,
        segments: vec![
            MediaSegment {
                uri: "seg0.m4s".into(),
                duration: DecimalSeconds::new(9.009).unwrap(),
                discontinuous: false,
                parts: vec![],
                ..Default::default()
            },
            MediaSegment {
                uri: "seg1.m4s".into(),
                duration: DecimalSeconds::new(9.009).unwrap(),
                discontinuous: false,
                parts: vec![],
                ..Default::default()
            },
            MediaSegment {
                uri: "seg2.m4s".into(),
                duration: DecimalSeconds::new(3.003).unwrap(),
                discontinuous: false,
                parts: vec![],
                ..Default::default()
            },
        ],
        endlist: true,
        extra_tags: vec![
            "#EXT-X-DATERANGE:ID=\"ad-1\",START-DATE=\"2024-01-01T00:00:00.000Z\",DURATION=15.0"
                .into(),
        ],
        low_latency: None,
        iframes_only: false,
        open_segment: None,
        ..Default::default()
    };

    let m3u8 = pl.to_m3u8().unwrap();

    // Structural assertions.
    assert!(m3u8.starts_with("#EXTM3U\n"), "must start with #EXTM3U");
    assert_eq!(
        m3u8.matches("#EXTINF:").count(),
        3,
        "must have exactly 3 #EXTINF: lines"
    );
    assert!(
        m3u8.ends_with("#EXT-X-ENDLIST\n"),
        "must end with #EXT-X-ENDLIST"
    );
}

#[test]
fn master_playlist_structure() {
    let pl = MasterPlaylist {
        version: 6,
        variants: vec![
            Variant {
                bandwidth: 300_000,
                codecs: Some("avc1.64001e,mp4a.40.2".into()),
                resolution: Some((640, 360)),
                uri: "v300/index.m3u8".into(),
                extra_attrs: vec![],
            },
            Variant {
                bandwidth: 800_000,
                codecs: Some("avc1.640028,mp4a.40.2".into()),
                resolution: Some((1280, 720)),
                uri: "v800/index.m3u8".into(),
                extra_attrs: vec![],
            },
        ],
        iframe_variants: vec![],
        ..Default::default()
    };

    let m3u8 = pl.to_m3u8().unwrap();

    assert!(m3u8.starts_with("#EXTM3U"), "must start with #EXTM3U");
    assert_eq!(
        m3u8.matches("#EXT-X-STREAM-INF:").count(),
        2,
        "must have exactly 2 EXT-X-STREAM-INF lines"
    );
    assert!(
        m3u8.contains("v300/index.m3u8"),
        "must contain first variant URI"
    );
    assert!(
        m3u8.contains("v800/index.m3u8"),
        "must contain second variant URI"
    );
    assert!(
        m3u8.contains("RESOLUTION=640x360"),
        "must contain first resolution"
    );
    assert!(
        m3u8.contains("RESOLUTION=1280x720"),
        "must contain second resolution"
    );
}

// ---------------------------------------------------------------------------
// CENC/CBCS HLS signalling (issue #564): #EXT-X-KEY for cbcs, none for cenc.
// ---------------------------------------------------------------------------

const TEST_KID: [u8; 16] = [
    0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef,
];

#[test]
fn cbcs_emits_ext_x_key_sample_aes() {
    let tag = cenc_ext_x_key(
        CencScheme::Cbcs,
        &TEST_KID,
        "https://keyserver.example.com/key",
    )
    .expect("valid key_uri")
    .expect("cbcs must emit an EXT-X-KEY tag");
    assert_eq!(
        tag,
        "#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"https://keyserver.example.com/key\",\
         KEYFORMAT=\"urn:mpeg:dash:mp4protection:2011\",KEYFORMATVERSIONS=\"1\",\
         KEYID=0x0123456789abcdef0123456789abcdef"
    );

    // Wired into a real playlist via `extra_tags` (the established hook for
    // arbitrary tag lines, e.g. #EXT-X-DATERANGE) renders before the segments.
    let pl = MediaPlaylist {
        version: 6,
        target_duration: 6,
        media_sequence: 0,
        discontinuity_sequence: 0,
        segments: vec![MediaSegment {
            uri: "seg0.m4s".into(),
            duration: DecimalSeconds::new(6.0).unwrap(),
            discontinuous: false,
            parts: vec![],
            ..Default::default()
        }],
        endlist: true,
        extra_tags: vec![tag],
        low_latency: None,
        iframes_only: false,
        open_segment: None,
        ..Default::default()
    };
    let m3u8 = pl.to_m3u8().unwrap();
    let key_pos = m3u8.find("#EXT-X-KEY:").expect("EXT-X-KEY line present");
    let extinf_pos = m3u8.find("#EXTINF:").expect("EXTINF line present");
    assert!(
        key_pos < extinf_pos,
        "#EXT-X-KEY must precede the segments it protects"
    );
    assert_eq!(m3u8.matches("#EXT-X-KEY:").count(), 1);
}

#[test]
fn cenc_ctr_emits_sample_aes_ctr_ext_x_key() {
    // Audit BH-W1 (issue #1111): RFC 8216bis §4.4.4.4 defines
    // METHOD=SAMPLE-AES-CTR for `cenc`-protected fMP4, and the IV
    // attribute MUST NOT be present. The old assertion that `cenc` has no
    // HLS key tag contradicted the spec (and this crate's own
    // EncryptionMethod::SampleAesCtr).
    let tag = cenc_ext_x_key(
        CencScheme::Cenc,
        &TEST_KID,
        "https://keyserver.example.com/key",
    )
    .expect("valid key_uri")
    .expect("cenc must emit an EXT-X-KEY tag");
    assert_eq!(
        tag,
        "#EXT-X-KEY:METHOD=SAMPLE-AES-CTR,URI=\"https://keyserver.example.com/key\",\
         KEYFORMAT=\"urn:mpeg:dash:mp4protection:2011\",KEYFORMATVERSIONS=\"1\",\
         KEYID=0x0123456789abcdef0123456789abcdef"
    );
    assert!(
        !tag.contains("IV="),
        "SAMPLE-AES-CTR must not carry an IV (RFC 8216bis §4.4.4.4)"
    );
}

#[test]
fn rendition_report_without_last_msn_is_rejected() {
    // Audit BH-W2 (issue #1111): LAST-MSN is REQUIRED
    // (RFC 8216bis §4.4.5.3). Previously it silently defaulted to 0, so a
    // client following the report requested _HLS_msn=0 (long gone) and a
    // re-render fabricated LAST-MSN=0.
    let pl = "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXTINF:4,\nseg.m4s\n\
              #EXT-X-RENDITION-REPORT:URI=\"b.m3u8\"\n#EXT-X-ENDLIST\n";
    let err = MediaPlaylist::parse(pl).expect_err("missing LAST-MSN must be an error");
    assert!(
        err.to_string().contains("LAST-MSN"),
        "error should name the missing attribute: {err}"
    );

    // With LAST-MSN present it still parses.
    let ok = MediaPlaylist::parse(
        "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXTINF:4,\nseg.m4s\n\
         #EXT-X-RENDITION-REPORT:URI=\"b.m3u8\",LAST-MSN=42\n#EXT-X-ENDLIST\n",
    )
    .expect("LAST-MSN present must parse");
    assert_eq!(ok.rendition_reports[0].last_msn, 42);
}

#[test]
fn server_control_without_parts_renders_no_zero_part_inf() {
    // Audit BH-W3 (issue #1111): a Media Playlist may carry
    // #EXT-X-SERVER-CONTROL (blocking reload / delta updates) without any
    // Partial Segments (RFC 8216bis §4.4.3.8). Previously parse set
    // part_target/part_hold_back to 0, and re-render fabricated
    // PART-TARGET=0 / PART-HOLD-BACK=0.
    let src = "#EXTM3U\n#EXT-X-TARGETDURATION:4\n\
               #EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES,CAN-SKIP-UNTIL=36,HOLD-BACK=12\n\
               #EXTINF:4,\nseg.m4s\n#EXT-X-ENDLIST\n";
    let pl = MediaPlaylist::parse(src).expect("must parse");
    let ll = pl
        .low_latency
        .as_ref()
        .expect("SERVER-CONTROL sets low_latency");
    assert_eq!(ll.part_target, None, "no PART-INF seen -> no part target");
    assert_eq!(ll.part_hold_back, None, "no PART-HOLD-BACK seen");
    assert_eq!(ll.hold_back, Some(DecimalSeconds::new(12.0).unwrap()));

    let rendered = pl.to_m3u8().unwrap();
    assert!(
        !rendered.contains("#EXT-X-PART-INF"),
        "must not emit PART-INF the input never had:\n{rendered}"
    );
    assert!(
        !rendered.contains("PART-HOLD-BACK"),
        "must not emit PART-HOLD-BACK the input never had:\n{rendered}"
    );
    // Parse -> render -> parse is stable (no zero values creep back in).
    let pl2 = MediaPlaylist::parse(&rendered).expect("re-parse");
    assert_eq!(pl2.low_latency, pl.low_latency);
    assert_eq!(pl2.to_m3u8().unwrap(), rendered);
}

#[test]
fn ll_hls_and_master_durations_reject_non_conformant_decimals() {
    // Audit BH-W4 (issue #1111): the strict §4.2 grammar must gate EVERY
    // decimal attribute, not just EXTINF. These all used to parse via
    // f64::from_str's permissive grammar.
    let cases: &[(&str, &[&str])] = &[
        (
            "#EXT-X-PART-INF:PART-TARGET=%s",
            &["nan", "inf", "1e308", "+3", "-1"],
        ),
        (
            "#EXT-X-SERVER-CONTROL:PART-HOLD-BACK=%s",
            &["nan", "infinity", "1e5", "+2"],
        ),
        (
            "#EXT-X-SERVER-CONTROL:CAN-SKIP-UNTIL=%s",
            &["nan", "1e5", "-1"],
        ),
        ("#EXT-X-SERVER-CONTROL:HOLD-BACK=%s", &["nan", "1e5", "-1"]),
    ];
    for (tmpl, bads) in cases {
        for bad in *bads {
            let src = format!(
                "#EXTM3U\n#EXT-X-TARGETDURATION:4\n{}\n#EXTINF:4,\nseg.m4s\n",
                tmpl.replace("%s", bad)
            );
            assert!(
                MediaPlaylist::parse(&src).is_err(),
                "{} must be rejected",
                tmpl.replace("%s", bad)
            );
        }
    }
    // EXT-X-PART DURATION.
    for bad in ["nan", "1e5"] {
        let src = format!(
            "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXT-X-PART:DURATION={bad},URI=\"p.m4s\"\n#EXTINF:4,\nseg.m4s\n"
        );
        assert!(
            MediaPlaylist::parse(&src).is_err(),
            "EXT-X-PART DURATION={bad} must be rejected"
        );
    }
    // Master Playlist BANDWIDTH (decimal-integer): nan/exponent rejected.
    for bad in ["nan", "1e10", "+7", "-7", "1.5"] {
        let src = format!("#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH={bad}\nv.m3u8\n");
        assert!(
            MasterPlaylist::parse(&src).is_err(),
            "BANDWIDTH={bad} must be rejected"
        );
    }
}

/// Audit BH-W5 (issue #1111): a segment-defining tag must re-render *in
/// place*, not hoisted to one playlist-level block before every segment.
/// Hoisting a key rotation applies the last key to the whole playlist (the
/// earlier segments fail to decrypt) and collapses the
/// `#EXT-X-PROGRAM-DATE-TIME` timeline (the last one wins), so the tag
/// positions in the rendered bytes are checked directly, not just the
/// parsed structs.
#[test]
fn segment_defining_tags_render_in_place_not_hoisted() {
    let key_a = r#"#EXT-X-KEY:METHOD=AES-128,URI="https://example.com/a.key""#;
    let key_b = r#"#EXT-X-KEY:METHOD=AES-128,URI="https://example.com/b.key""#;
    let pdt0 = "#EXT-X-PROGRAM-DATE-TIME:2024-01-01T00:00:00.000Z";
    let pdt1 = "#EXT-X-PROGRAM-DATE-TIME:2024-01-01T00:06:00.000Z";
    let text = [
        "#EXTM3U",
        "#EXT-X-VERSION:3",
        "#EXT-X-TARGETDURATION:6",
        "#EXT-X-MEDIA-SEQUENCE:0",
        key_a,
        pdt0,
        "#EXTINF:6,",
        "s0.m4s",
        key_b,
        pdt1,
        "#EXTINF:6,",
        "s1.m4s",
        "#EXT-X-ENDLIST",
        "",
    ]
    .join(
        "
",
    );

    let pl = MediaPlaylist::parse(&text).expect("must parse");
    assert_eq!(
        pl.segments[0].pre_tags,
        vec![key_a.to_string(), pdt0.to_string()]
    );
    assert_eq!(
        pl.segments[1].pre_tags,
        vec![key_b.to_string(), pdt1.to_string()]
    );
    assert_eq!(pl.extra_tags, Vec::<String>::new(), "no hoisted block");

    let out = pl.to_m3u8().unwrap();
    assert_eq!(out, text, "segment-defining tags must render in place");
    let (ka, kb) = (out.find(key_a).unwrap(), out.find(key_b).unwrap());
    let (s0, s1) = (out.find("s0.m4s").unwrap(), out.find("s1.m4s").unwrap());
    assert!(
        ka < s0 && s0 < kb && kb < s1,
        "key A before s0, key B before s1"
    );
    let parsed = MediaPlaylist::parse(&out).expect("re-parse must succeed");
    assert_eq!(parsed, pl, "round trip must be lossless");
}

/// Audit BH-W5, open-segment half: segment-defining tags after the last
/// closed segment belong to the in-progress segment and must survive the
/// round trip (previously they were dropped or hoisted out of position).
#[test]
fn open_segment_keeps_its_segment_defining_tags() {
    let key_b = r#"#EXT-X-KEY:METHOD=AES-128,URI="https://example.com/b.key""#;
    let text = [
        "#EXTM3U",
        "#EXT-X-VERSION:9",
        "#EXT-X-TARGETDURATION:4",
        "#EXT-X-MEDIA-SEQUENCE:0",
        "#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=NO",
        "#EXTINF:4,",
        "s0.m4s",
        r#"#EXT-X-PART:DURATION=1,URI="s1.0.m4s""#,
        key_b,
        "",
    ]
    .join(
        "
",
    );

    let pl = MediaPlaylist::parse(&text).expect("must parse");
    let open = pl.open_segment.as_ref().expect("open segment expected");
    assert_eq!(open.pre_tags, vec![key_b.to_string()]);
    assert_eq!(pl.extra_tags, Vec::<String>::new());
    let out = pl.to_m3u8().unwrap();
    assert_eq!(out, text, "open-segment pre-tags must render in place");
    let parsed = MediaPlaylist::parse(&out).expect("re-parse must succeed");
    assert_eq!(parsed, pl, "round trip must be lossless");
}

/// Audit BH-W6 (issue #1111): `CODECS` is optional on `#EXT-X-STREAM-INF`,
/// so an absent attribute must stay absent — not become the invalid empty
/// `CODECS=""`, which a strict client rejects as a malformed codec list.
#[test]
fn stream_inf_without_codecs_omits_the_attribute() {
    let src = "#EXTM3U\n\
#EXT-X-STREAM-INF:BANDWIDTH=5000000\n\
v5.m3u8\n";
    let mpl = MasterPlaylist::parse(src).expect("must parse");
    assert_eq!(mpl.variants[0].codecs, None);
    let out = mpl.to_m3u8().unwrap();
    assert!(
        !out.contains("CODECS"),
        "an absent CODECS must not render CODECS=\"\":\n{out}"
    );
    assert!(out.contains("#EXT-X-STREAM-INF:BANDWIDTH=5000000\n"));
    // And the round trip stays lossless.
    assert_eq!(MasterPlaylist::parse(&out).expect("re-parse"), mpl);

    // A present CODECS still renders quoted (and still round-trips).
    let src = "#EXTM3U\n\
#EXT-X-STREAM-INF:BANDWIDTH=1000,CODECS=\"avc1.640020\"\n\
v.m3u8\n";
    let mpl = MasterPlaylist::parse(src).expect("must parse");
    assert_eq!(mpl.variants[0].codecs.as_deref(), Some("avc1.640020"));
    assert_eq!(mpl.to_m3u8().unwrap(), src);
}

/// Audit BH-W7 (issue #1111): a field the renderer places inside an RFC 8216
/// §4.2 quoted-string or emits as a URI must not be able to inject a tag
/// line or break out of the attribute list. `"`, CR and LF are the three
/// characters that can do either, so each is rejected (both by the
/// per-field constructors and by `to_m3u8`, for a playlist built by hand).
#[test]
fn render_rejects_injection_characters_in_uris_and_quoted_strings() {
    use broadcast_hls::{Error, Variant};

    for bad in [
        "#EXT-X-STREAM-INF:BANDWIDTH=1,CODECS=\"a\"\n#EXT-X-ENDLIST",
        "v.m3u8\n#EXT-X-ENDLIST",
        "v\"quoted\".m3u8",
        "v\rm.m3u8",
    ] {
        let mpl = MasterPlaylist {
            variants: vec![Variant {
                bandwidth: 1,
                uri: bad.to_string(),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(
            matches!(mpl.to_m3u8(), Err(Error::InvalidUri { .. })),
            "variant URI {bad:?} must be rejected"
        );
    }

    // A CODECS carrying a newline would inject a tag inside the attribute
    // list; a bare `"` would end the quoted-string early.
    for bad in ["avc1.640020\n#EXT-X-ENDLIST", "avc1.640020\",X=\"y"] {
        let mpl = MasterPlaylist {
            variants: vec![Variant {
                bandwidth: 1,
                codecs: Some(bad.to_string()),
                uri: "v.m3u8".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(
            matches!(mpl.to_m3u8(), Err(Error::InvalidQuotedString { .. })),
            "CODECS {bad:?} must be rejected"
        );
    }

    // A segment URI, and a `#EXT-X-DEFINE`/pre-tag body, are the same class
    // of vector on a Media Playlist.
    let pl = MediaPlaylist {
        target_duration: 6,
        segments: vec![MediaSegment {
            uri: "seg.m4s\n#EXT-X-ENDLIST".into(),
            duration: DecimalSeconds::new(6.0).unwrap(),
            ..Default::default()
        }],
        ..Default::default()
    };
    assert!(matches!(pl.to_m3u8(), Err(Error::InvalidUri { .. })));

    let pl = MediaPlaylist {
        target_duration: 6,
        extra_tags: vec!["#EXT-X-FOO:V=1\n#EXT-X-ENDLIST".into()],
        ..Default::default()
    };
    assert!(matches!(
        pl.to_m3u8(),
        Err(Error::InvalidQuotedString { .. })
    ));

    // The happy path still renders.
    let pl = MediaPlaylist {
        target_duration: 6,
        segments: vec![MediaSegment {
            uri: "seg.m4s".into(),
            duration: DecimalSeconds::new(6.0).unwrap(),
            ..Default::default()
        }],
        ..Default::default()
    };
    assert!(pl.to_m3u8().unwrap().contains("seg.m4s\n"));
    let mpl = MasterPlaylist {
        variants: vec![Variant {
            bandwidth: 1,
            codecs: Some("avc1.640020".to_string()),
            uri: "v.m3u8".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    };
    assert!(mpl.to_m3u8().unwrap().contains(",CODECS=\"avc1.640020\""));
}

/// Audit BH-W9 (issue #1111): `#EXT-X-STREAM-INF` MUST be followed by the
/// URI line of the Variant it describes (RFC 8216bis §4.4.6.2). A second
/// STREAM-INF, or EOF, before that line is a truncated playlist — silently
/// overwriting the pending variant (or dropping it) loses the information
/// before any downstream validator can see it.
#[test]
fn master_playlist_rejects_a_stream_inf_without_its_uri_line() {
    use broadcast_hls::Error;

    // Two STREAM-INF tags with only one URI line: the first variant's URI is
    // missing. Before the fix this parsed `Ok` with a single variant, so the
    // first one vanished without a trace.
    let truncated = "#EXTM3U\n\
#EXT-X-STREAM-INF:BANDWIDTH=1000,CODECS=\"avc1.640020\"\n\
#EXT-X-STREAM-INF:BANDWIDTH=2000,CODECS=\"avc1.640021\"\n\
v2.m3u8\n";
    let err = MasterPlaylist::parse(truncated)
        .expect_err("a second STREAM-INF before the first URI must error");
    assert!(matches!(err, Error::HlsParse { .. }), "got {err:?}");

    // ...and a trailing one with no URI at all (also the truncation case).
    let dangling = "#EXTM3U\n\
#EXT-X-STREAM-INF:BANDWIDTH=1000,CODECS=\"avc1.640020\"\n";
    let err = MasterPlaylist::parse(dangling).expect_err("a dangling STREAM-INF must error");
    assert!(matches!(err, Error::HlsParse { .. }), "got {err:?}");

    // The well-formed shape still parses, with both variants.
    let ok = "#EXTM3U\n\
#EXT-X-STREAM-INF:BANDWIDTH=1000,CODECS=\"avc1.640020\"\n\
v1.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=2000,CODECS=\"avc1.640021\"\n\
v2.m3u8\n";
    let mpl = MasterPlaylist::parse(ok).expect("well-formed master must parse");
    assert_eq!(mpl.variants.len(), 2);
}

/// Audit BH-W11 (issue #1111): the `#EXTINF` title (the text after the
/// duration's comma, RFC 8216bis §4.4.4.1) is part of the tag and must
/// survive the round trip — it used to be split off and discarded.
#[test]
fn extinf_title_round_trips() {
    let src = "#EXTM3U\n\
#EXT-X-VERSION:3\n\
#EXT-X-TARGETDURATION:6\n\
#EXT-X-MEDIA-SEQUENCE:0\n\
#EXTINF:6,Chapter 1\n\
s0.m4s\n\
#EXTINF:6,\n\
s1.m4s\n\
#EXT-X-ENDLIST\n";
    let pl = MediaPlaylist::parse(src).expect("must parse");
    assert_eq!(pl.segments[0].title.as_deref(), Some("Chapter 1"));
    assert_eq!(pl.segments[1].title, None, "no title after the bare comma");
    let out = pl.to_m3u8().unwrap();
    assert_eq!(out, src, "the title must round-trip byte-exactly");
    assert_eq!(MediaPlaylist::parse(&out).expect("re-parse"), pl);

    // A title is a free-form string on the line, so a CR/LF in it would
    // split the tag — rejected like any other quoted-string (BH-W7).
    let pl = MediaPlaylist {
        target_duration: 6,
        segments: vec![MediaSegment {
            uri: "s.m4s".into(),
            duration: DecimalSeconds::new(6.0).unwrap(),
            title: Some("a\n#EXT-X-ENDLIST".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };
    assert!(matches!(
        pl.to_m3u8(),
        Err(broadcast_hls::Error::InvalidQuotedString { .. })
    ));
}
