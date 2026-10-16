//! Every container that stores unmodelled children in `unknown_children`
//! must write them back in `to_xml` (review/signalling-captions finding 1:
//! `<metadata>` dropped `<smpte:image>`). One foreign-namespace marker per
//! container; each must survive parse -> to_xml -> parse.
#![cfg(feature = "std")]

use std::fs;
use std::path::PathBuf;

use ttml_subtitle::Document;

const HEAD: &str = r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:x="urn:example:foreign" xmlns:ttm="http://www.w3.org/ns/ttml#metadata" xmlns:ittm="http://www.w3.org/ns/ttml/profile/imsc1#metadata" xmlns:ebuttm="urn:ebu:tt:metadata" xmlns:tts="http://www.w3.org/ns/ttml#styling" xml:lang="en">"#;

fn m(n: usize) -> String {
    format!("<x:foo>MARK{n}</x:foo>")
}

/// (name, document) — one marker placed inside the named container.
fn cases() -> Vec<(&'static str, String)> {
    let w = |inner: String| format!("{HEAD}{inner}</tt>");
    let hd = |inner: String| w(format!("<head>{inner}</head><body/>"));
    let md = |inner: String| hd(format!("<metadata>{inner}</metadata>"));
    let res = |inner: String| hd(format!("<resources>{inner}</resources>"));
    vec![
        ("tt", w(format!("<body/>{}", m(0)))),
        ("head", hd(m(1))),
        ("metadata", md(m(2))),
        (
            "metadata-nested",
            md(format!("<metadata>{}</metadata>", m(3))),
        ),
        (
            "ttm:agent",
            md(format!(
                "<ttm:agent xml:id=\"a\" type=\"person\">{}</ttm:agent>",
                m(4)
            )),
        ),
        (
            "ttm:item",
            md(format!("<ttm:item name=\"n\">{}</ttm:item>", m(5))),
        ),
        (
            "ebuttm:documentMetadata",
            md(format!(
                "<ebuttm:documentMetadata>{}</ebuttm:documentMetadata>",
                m(6)
            )),
        ),
        ("ttm:title", md(format!("<ttm:title>t{}</ttm:title>", m(7)))),
        (
            "ttm:name",
            md(format!(
                "<ttm:agent xml:id=\"a\"><ttm:name type=\"full\">n{}</ttm:name></ttm:agent>",
                m(8)
            )),
        ),
        (
            "ittm:altText",
            md(format!("<ittm:altText>a{}</ittm:altText>", m(9))),
        ),
        ("styling", hd(format!("<styling>{}</styling>", m(10)))),
        (
            "style",
            hd(format!(
                "<styling><style xml:id=\"s\">{}</style></styling>",
                m(11)
            )),
        ),
        (
            "initial",
            hd(format!(
                "<styling><initial tts:color=\"red\">{}</initial></styling>",
                m(12)
            )),
        ),
        ("layout", hd(format!("<layout>{}</layout>", m(13)))),
        (
            "region",
            hd(format!(
                "<layout><region xml:id=\"r\">{}</region></layout>",
                m(14)
            )),
        ),
        ("body", w(format!("<body>{}</body>", m(15)))),
        ("div", w(format!("<body><div>{}</div></body>", m(16)))),
        ("p", w(format!("<body><div><p>{}</p></div></body>", m(17)))),
        (
            "span",
            w(format!(
                "<body><div><p><span>{}</span></p></div></body>",
                m(18)
            )),
        ),
        (
            "set",
            w(format!(
                "<body><div><p><set begin=\"1s\" tts:color=\"red\">{}</set>t</p></div></body>",
                m(19)
            )),
        ),
        (
            "p>metadata",
            w(format!(
                "<body><div><p><metadata>{}</metadata>t</p></div></body>",
                m(20)
            )),
        ),
        (
            "div>metadata",
            w(format!(
                "<body><div><metadata>{}</metadata></div></body>",
                m(21)
            )),
        ),
        (
            "audio",
            res(format!(
                "<audio xml:id=\"au\" src=\"a.wav\">{}</audio>",
                m(22)
            )),
        ),
        ("resources", res(m(23))),
        (
            "data",
            res(format!(
                "<data xml:id=\"d\" type=\"image/png\">{}</data>",
                m(24)
            )),
        ),
        (
            "image",
            res(format!(
                "<image xml:id=\"i\" type=\"image/png\" src=\"i.png\">{}</image>",
                m(25)
            )),
        ),
        (
            "font",
            res(format!("<font xml:id=\"f\" family=\"F\">{}</font>", m(26))),
        ),
        (
            "source",
            res(format!(
                "<audio xml:id=\"au\"><source src=\"a.wav\">{}</source></audio>",
                m(27)
            )),
        ),
    ]
}

#[test]
fn foreign_child_of_every_container_survives_to_xml() {
    let mut failures = Vec::new();
    for (name, xml) in cases() {
        let mut doc = match Document::parse_str(&xml) {
            Ok(d) => d,
            Err(e) => {
                failures.push(format!("{name}: input rejected: {e}"));
                continue;
            }
        };
        let out = doc.to_xml();
        let marker = xml
            .split("MARK")
            .nth(1)
            .and_then(|s| s.split('<').next())
            .unwrap();
        if !out.contains(&format!("MARK{marker}<")) {
            failures.push(format!("{name}: marker dropped by to_xml:\n{out}"));
            continue;
        }
        match Document::parse_str(&out) {
            Ok(re) => {
                let mut again = re.clone();
                if again.to_xml() != out {
                    failures.push(format!("{name}: to_xml not idempotent"));
                }
            }
            Err(e) => failures.push(format!("{name}: output does not re-parse: {e}\n{out}")),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n---\n"));
}

/// `<smpte:image>` (SMPTE-TT / IMSC image subtitles) keeps its prefix,
/// namespace, attributes and base64 text through `to_xml`.
#[test]
fn smpte_image_in_metadata_keeps_attributes_and_text() {
    let xml = r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:smpte="http://www.smpte-ra.org/schemas/2052-1/2010/smpte-tt"><head><metadata><smpte:image xml:id="img0" imagetype="PNG" encoding="Base64">iVBORw0KGgo=</smpte:image></metadata></head><body/></tt>"#;
    let mut doc = Document::parse_str(xml).unwrap();
    let out = doc.to_xml();
    let re = Document::parse_str(&out).unwrap();
    assert_eq!(re.tt.head, doc.tt.head, "{out}");
    assert!(out.contains("smpte:image"), "{out}");
    assert!(out.contains(r#"imagetype="PNG""#), "{out}");
    assert!(
        out.contains("http://www.smpte-ra.org/schemas/2052-1/2010/smpte-tt"),
        "{out}"
    );
}

/// Unmodelled children keep their document order.
#[test]
fn multiple_foreign_children_keep_order() {
    let xml = format!(
        "{HEAD}<head><metadata><x:foo>A</x:foo><x:foo>B</x:foo><x:foo>C</x:foo></metadata></head><body/></tt>"
    );
    let mut doc = Document::parse_str(&xml).unwrap();
    let out = doc.to_xml();
    let (a, b, c) = (
        out.find(">A<").unwrap(),
        out.find(">B<").unwrap(),
        out.find(">C<").unwrap(),
    );
    assert!(a < b && b < c, "{out}");
}

/// Every committed W3C IMSC fixture survives parse -> to_xml -> parse with
/// an identical tree.
#[test]
fn committed_fixtures_round_trip_through_to_xml() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut n = 0;
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "ttml") {
            continue;
        }
        let src = fs::read_to_string(&path).unwrap();
        let mut doc = Document::parse_str(&src).unwrap();
        let out = doc.to_xml();
        let re = Document::parse_str(&out)
            .unwrap_or_else(|e| panic!("{}: re-parse failed: {e}\n{out}", path.display()));
        assert_eq!(re.tt, doc.tt, "{} changed through to_xml", path.display());
        n += 1;
    }
    assert!(n >= 10, "expected the committed fixtures, saw {n}");
}
