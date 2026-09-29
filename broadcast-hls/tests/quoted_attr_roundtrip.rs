//! Lossless quoting round-trip for unmodeled attribute-list attributes
//! (issue #1045 / audit BH-C1).
//!
//! `broadcast_hls::MasterPlaylist::parse` used to strip the surrounding `"`
//! from every quoted-string attribute value into `Variant::extra_attrs` /
//! `IFrameVariant::extra_attrs` (and every other `extra_attrs` field) with
//! no record of whether it had been quoted, so a value like `AUDIO="a1"`
//! and an (invalid, but not rejected) `AUDIO=a1` became indistinguishable
//! once parsed, and re-rendering guessed the kind from a small table of
//! known RFC 8216bis attribute names — silently losing quotes on any
//! attribute this crate hadn't been taught about (e.g. a private `X-`
//! extension). `extra_attrs` now stores an [`AttrValue`] per entry,
//! recorded from the real wire token at parse time, so quoting round-trips
//! losslessly for ANY attribute name, known or not.
//!
//! [`AttrValue`] is opaque and only constructible through
//! [`AttrValue::quoted`]/[`AttrValue::bare`]/[`AttrValue::for_attr`], each
//! of which validates its content and returns `Result` (issue #1045 T12,
//! coordinator review): a quoted-string containing `"`/CR/LF, or a bare
//! value containing a character that would require quoting, is REJECTED
//! at construction rather than silently mangled (e.g. percent-encoded) or
//! emitted raw — cf. #1129. This keeps `render_attribute_list`/`to_m3u8`
//! infallible without ever letting an invalid value exist to render.
//!
//! This is a **byte-level** comparison of the rendered attribute tokens
//! against the source tokens — the `parse -> render -> parse` document-
//! equality check in `hls_fixture_corpus.rs` is blind to quoting (parse
//! strips quotes on both sides of that comparison), which is exactly why
//! this bug survived.

use std::fs;
use std::path::PathBuf;

use broadcast_hls::{AttrValue, MasterPlaylist, parse_attribute_list, render_attribute_list};

fn repo_fixture(rel: &str) -> String {
    let path = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/.."))
        .join("fixtures/hls")
        .join(rel);
    fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read committed fixture {}: {e}", path.display()))
}

/// The real fixture's first `#EXT-X-STREAM-INF` line carries
/// `CLOSED-CAPTIONS="cc",AUDIO="a1",SUBTITLES="sub1"` (`fixtures/hls/real/
/// bipbop-fmp4-hevc/master.m3u8:24`) — every one of these is an unmodeled,
/// RFC 8216bis-quoted-string attribute on `Variant::extra_attrs`.
#[test]
fn real_stream_inf_quoted_attrs_round_trip_byte_exact() {
    let text = repo_fixture("real/bipbop-fmp4-hevc/master.m3u8");
    let parsed = MasterPlaylist::parse(&text).expect("bipbop-fmp4-hevc/master.m3u8 must parse");

    let variant = &parsed.variants[0];
    // `AVERAGE-BANDWIDTH`/`FRAME-RATE` are also unmodeled (decimal, so their
    // quoting is not at issue here); the three quoted-string attributes this
    // regression targets are asserted individually below.
    assert_eq!(
        variant
            .extra_attrs
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("AUDIO", "a1"),
            ("AVERAGE-BANDWIDTH", "2190673"),
            ("CLOSED-CAPTIONS", "cc"),
            ("FRAME-RATE", "60.000"),
            ("SUBTITLES", "sub1"),
        ],
        "sorted-by-name extra_attrs recovered from the fixture line"
    );
    // Recorded as quoted, not just content-sniffed correctly by accident.
    for (k, v) in &variant.extra_attrs {
        if k == "AUDIO" || k == "CLOSED-CAPTIONS" || k == "SUBTITLES" {
            assert!(v.is_quoted(), "{k} must be recorded as quoted, got {v:?}");
        }
    }

    let rendered = parsed.to_m3u8().unwrap();
    let rendered_stream_inf_line = rendered
        .lines()
        .find(|l| l.starts_with("#EXT-X-STREAM-INF:"))
        .expect("rendered output must contain a #EXT-X-STREAM-INF line");

    for token in [
        "AUDIO=\"a1\"",
        "CLOSED-CAPTIONS=\"cc\"",
        "SUBTITLES=\"sub1\"",
    ] {
        assert!(
            rendered_stream_inf_line.contains(token),
            "rendered #EXT-X-STREAM-INF line must contain the byte-exact quoted \
             token {token:?} (source: CLOSED-CAPTIONS=\"cc\",AUDIO=\"a1\",SUBTITLES=\"sub1\"), got:\n{rendered_stream_inf_line}"
        );
    }

    // Re-parsing the rendered output must recover the identical extra_attrs
    // (a stronger check than the corpus test's parsed-document equality,
    // which can't see the quoting because parse strips quotes on both sides).
    let reparsed = MasterPlaylist::parse(&rendered).expect("rendered output must re-parse");
    assert_eq!(reparsed.variants[0].extra_attrs, variant.extra_attrs);
}

/// `CLOSED-CAPTIONS` is the one attribute in this set that is EITHER a
/// quoted-string OR the bare enumerated token `NONE` (RFC 8216bis §4.4.6.2:
/// "the value can be either a quoted-string or an enumerated-string with the
/// value NONE") — `NONE` must NOT gain quotes on render, since
/// `CLOSED-CAPTIONS="NONE"` and `CLOSED-CAPTIONS="NONE"` are spec-distinct
/// values.
#[test]
fn closed_captions_none_is_never_quoted() {
    let text = "#EXTM3U\n\
                #EXT-X-STREAM-INF:BANDWIDTH=1000,CLOSED-CAPTIONS=NONE\n\
                v.m3u8\n";
    let parsed = MasterPlaylist::parse(text).expect("must parse");
    assert_eq!(
        parsed.variants[0].extra_attrs,
        vec![(
            "CLOSED-CAPTIONS".to_string(),
            AttrValue::bare("NONE").unwrap()
        )]
    );
    let rendered = parsed.to_m3u8().unwrap();
    assert!(
        rendered.contains("CLOSED-CAPTIONS=NONE"),
        "CLOSED-CAPTIONS=NONE must round-trip unquoted, got:\n{rendered}"
    );
    assert!(!rendered.contains("CLOSED-CAPTIONS=\"NONE\""));
}

/// **Lossless for an attribute this crate has never heard of** (issue
/// #1045): a private `X-`-prefixed quoted-string attribute (not in the
/// known-name table `AttrValue::for_attr` uses for programmatic
/// construction) must still round-trip its quotes byte-exactly, because
/// parsing now records the true wire form directly rather than guessing
/// from a name table.
#[test]
fn unknown_x_attribute_quoting_round_trips_byte_exact() {
    let text = "#EXTM3U\n\
                #EXT-X-STREAM-INF:BANDWIDTH=1000,X-FOO=\"bar\",X-BAR=baz\n\
                v.m3u8\n";
    let parsed = MasterPlaylist::parse(text).expect("must parse");
    assert_eq!(
        parsed.variants[0].extra_attrs,
        vec![
            ("X-BAR".to_string(), AttrValue::bare("baz").unwrap()),
            ("X-FOO".to_string(), AttrValue::quoted("bar").unwrap()),
        ]
    );

    let rendered = parsed.to_m3u8().unwrap();
    let stream_inf_line = rendered
        .lines()
        .find(|l| l.starts_with("#EXT-X-STREAM-INF:"))
        .expect("rendered output must contain a #EXT-X-STREAM-INF line");
    assert!(
        stream_inf_line.contains("X-FOO=\"bar\""),
        "unknown quoted attribute X-FOO must keep its quotes, got:\n{stream_inf_line}"
    );
    assert!(
        stream_inf_line.contains("X-BAR=baz"),
        "unknown bare attribute X-BAR must stay unquoted, got:\n{stream_inf_line}"
    );

    let reparsed = MasterPlaylist::parse(&rendered).expect("rendered output must re-parse");
    assert_eq!(
        reparsed.variants[0].extra_attrs,
        parsed.variants[0].extra_attrs
    );
}

/// **T12** (issue #1045, coordinator review): RFC 8216 §4.2 forbids a
/// quoted-string from containing `"`, CR or LF — [`AttrValue::quoted`]
/// must reject each of them rather than silently mangling (e.g.
/// percent-encoding) or accepting the value.
#[test]
fn quoted_rejects_each_forbidden_character() {
    for bad in ["has\"quote", "has\rcr", "has\nlf", "a\"\r\nb"] {
        assert!(
            AttrValue::quoted(bad).is_err(),
            "AttrValue::quoted must reject {bad:?}"
        );
    }
    // A value with none of them is accepted.
    assert!(AttrValue::quoted("some value, with a comma").is_ok());
}

/// [`AttrValue::bare`] must reject `,`, `"`, CR, LF and whitespace — any of
/// which would require the value to be quoted instead.
#[test]
fn bare_rejects_each_forbidden_character() {
    for bad in ["has,comma", "has\"quote", "has\rcr", "has\nlf", "has space"] {
        assert!(
            AttrValue::bare(bad).is_err(),
            "AttrValue::bare must reject {bad:?}"
        );
    }
    assert!(AttrValue::bare("PLAIN-TOKEN123").is_ok());
    assert!(
        AttrValue::bare("").is_ok(),
        "empty bare value is not itself forbidden"
    );
}

/// A caller-constructed value containing the forbidden characters (e.g.
/// from a third-party ad-decision service feeding `ssai-runtime`) must be
/// rejected before it can ever reach `render_attribute_list`/`to_m3u8` — there
/// is nothing left to sanitize at render time because no invalid
/// `AttrValue` can exist.
#[test]
fn injection_attempt_is_rejected_at_construction_not_rendered() {
    let malicious = "cc\"\r\n#EXT-X-ENDLIST\r\n";
    assert!(
        AttrValue::quoted(malicious).is_err(),
        "a value containing '\"'/CR/LF must be rejected, not rendered"
    );
    assert!(
        AttrValue::for_attr("X-INJECT", malicious).is_err(),
        "for_attr must propagate the same rejection"
    );
}

/// A valid caller-constructed value round-trips through
/// `AttrValue::quoted`/`bare` and a full parse -> render -> parse cycle
/// byte-exactly.
#[test]
fn valid_constructed_value_round_trips() {
    use broadcast_hls::Variant;

    let master = MasterPlaylist {
        variants: vec![Variant {
            bandwidth: 1000,
            codecs: Some("avc1.640020".to_string()),
            uri: "v.m3u8".to_string(),
            extra_attrs: vec![
                ("X-BARE".to_string(), AttrValue::bare("plain").unwrap()),
                (
                    "X-QUOTED".to_string(),
                    AttrValue::quoted("some, value").unwrap(),
                ),
            ],
            ..Default::default()
        }],
        ..Default::default()
    };

    let rendered = master.to_m3u8().unwrap();
    assert!(rendered.contains("X-QUOTED=\"some, value\""));
    assert!(rendered.contains("X-BARE=plain"));

    let reparsed = MasterPlaylist::parse(&rendered).expect("must re-parse");
    assert_eq!(
        reparsed.variants[0].extra_attrs,
        master.variants[0].extra_attrs
    );
}

/// The shared workspace tokenizer/renderer (issue #1140 T12): exercised
/// directly as public API, the same functions `ssai-runtime` and
/// `timed-metadata` now call instead of each carrying their own copy.
#[test]
fn public_tokenizer_and_renderer_round_trip_quoted_and_bare_values() {
    let (map, quoted) =
        parse_attribute_list(r#"ID="a1",CLASS="com.example",DURATION=15.5,X-BARE=plain"#);
    assert_eq!(map.get("ID").map(String::as_str), Some("a1"));
    assert_eq!(map.get("DURATION").map(String::as_str), Some("15.5"));
    assert!(quoted.contains("ID"));
    assert!(quoted.contains("CLASS"));
    assert!(!quoted.contains("DURATION"));
    assert!(!quoted.contains("X-BARE"));

    let attrs = vec![
        ("ID".to_string(), AttrValue::quoted("a1").unwrap()),
        ("DURATION".to_string(), AttrValue::bare("15.5").unwrap()),
    ];
    let mut out = String::new();
    render_attribute_list(&mut out, &attrs);
    assert_eq!(out, ",ID=\"a1\",DURATION=15.5");
}

/// Injection attempt through the public tokenizer's companion renderer
/// (issue #1140 T12): a `"`/CR/LF supplied to `AttrValue::quoted` is
/// rejected before `render_attribute_list` ever sees it.
#[test]
fn public_renderer_cannot_be_fed_an_injected_value() {
    assert!(AttrValue::quoted("a1\"\r\n#EXT-X-ENDLIST").is_err());
}
