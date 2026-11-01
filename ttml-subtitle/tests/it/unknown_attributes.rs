//! #1110 TT-W1: an attribute the crate does not model on an element must survive
//! parse -> serialize, including one in a namespace the crate *does* model
//! (`tts:`, `ttp:`, `ttm:`, `xml:`) or none at all. Previously such an
//! attribute was silently dropped by `to_xml`.

use ttml_subtitle::Document;

const DOC: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<tt xmlns="http://www.w3.org/ns/ttml" xmlns:ttp="http://www.w3.org/ns/ttml#parameter" xmlns:tts="http://www.w3.org/ns/ttml#styling" xmlns:ttm="http://www.w3.org/ns/ttml#metadata" xml:lang="en" ttp:futureParam="p1" tts:futureStyle="s1" bareTt="b1" xml:futureXml="x1">
<head>
<metadata ttm:futureMeta="m1" bareMeta="m2"><ttm:title ttm:futureTitle="t1">T</ttm:title></metadata>
<styling><style xml:id="s" tts:color="red" tts:futureStyleOnStyle="s2"/></styling>
<layout><region xml:id="r1" tts:origin="0% 0%" tts:extent="100% 100%" tts:futureRegion="r2" bareRegion="r3"/></layout>
</head>
<body tts:futureBody="bd1"><div ttm:futureDiv="d1" bareDiv="d2"><p begin="00:00:01.000" end="00:00:02.000" tts:futureP="p2" bareP="p3" xml:futureP="p4">Hello <span tts:color="red" tts:futureSpan="sp1" bareSpan="sp2">w</span><br tts:futureBr="br1" bareBr="br2"/>x</p></div></body></tt>"#;

/// Every `(name, value)` pair planted above that no element models.
const PLANTED: &[(&str, &str)] = &[
    ("ttp:futureParam", "p1"),
    ("tts:futureStyle", "s1"),
    ("bareTt", "b1"),
    ("xml:futureXml", "x1"),
    ("ttm:futureMeta", "m1"),
    ("bareMeta", "m2"),
    ("ttm:futureTitle", "t1"),
    ("tts:futureStyleOnStyle", "s2"),
    ("tts:futureRegion", "r2"),
    ("bareRegion", "r3"),
    ("tts:futureBody", "bd1"),
    ("ttm:futureDiv", "d1"),
    ("bareDiv", "d2"),
    ("tts:futureP", "p2"),
    ("bareP", "p3"),
    ("xml:futureP", "p4"),
    ("tts:futureSpan", "sp1"),
    ("bareSpan", "sp2"),
    ("tts:futureBr", "br1"),
    ("bareBr", "br2"),
];

#[test]
fn unmodeled_attributes_survive_serialization() {
    let mut doc = Document::parse_str(DOC).expect("parse");
    let out = doc.to_xml();
    for (name, value) in PLANTED {
        let needle = format!(" {name}=\"{value}\"");
        assert!(
            out.contains(&needle),
            "{name}={value} was dropped on serialize:\n{out}"
        );
    }
    // No bogus prefix for the xml namespace.
    assert!(
        !out.contains("=\"http://www.w3.org/XML/1998/namespace\""),
        "the xml namespace must never be declared under another prefix:\n{out}"
    );
    // Re-parses to the same tree (and is then stable).
    let mut again = Document::parse_str(&out).expect("re-parse");
    assert_eq!(doc, again);
    assert_eq!(again.to_xml(), out);
}

#[test]
fn modeled_attributes_are_not_duplicated_as_foreign() {
    let mut doc = Document::parse_str(DOC).unwrap();
    let out = doc.to_xml();
    // `begin`/`end`/`tts:color`/`xml:lang` are modeled: each appears once per
    // element, never also replayed as a foreign attribute.
    assert_eq!(out.matches(" begin=\"").count(), 1, "{out}");
    assert_eq!(out.matches(" end=\"").count(), 1, "{out}");
    assert_eq!(out.matches(" xml:lang=\"").count(), 1, "{out}");
    assert_eq!(out.matches(" tts:color=\"red\"").count(), 2, "{out}");
}

/// The read-tracking above is only sound while `foreign_attributes(node)` is
/// the last attribute-touching field of every parse site's struct literal
/// (a later `attribute_value(..)` would be reported foreign as well as
/// parsed). Pin that lexically so a new field cannot silently break it.
#[test]
fn foreign_attributes_is_evaluated_after_every_field_read() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/document.rs"))
        .expect("read document.rs");
    let lines: Vec<&str> = src.lines().collect();
    let reads = [
        "attribute_value(",
        ".attribute(",
        ".attribute_in(",
        "parse_style_attributes(",
    ];
    let mut checked = 0;
    for (i, line) in lines.iter().enumerate() {
        if !line.contains("foreign_attributes: foreign_attributes(") {
            continue;
        }
        checked += 1;
        let indent = line.len() - line.trim_start().len();
        for later in &lines[i + 1..] {
            let ind = later.len() - later.trim_start().len();
            if !later.trim().is_empty() && ind < indent {
                break; // end of the struct literal
            }
            assert!(
                !reads.iter().any(|r| later.contains(r)),
                "document.rs line {}: `{}` reads an attribute after \
                 `foreign_attributes: foreign_attributes(..)` in the same literal",
                i + 1,
                later.trim()
            );
        }
    }
    assert!(checked > 20, "expected the parse sites, found {checked}");
}
