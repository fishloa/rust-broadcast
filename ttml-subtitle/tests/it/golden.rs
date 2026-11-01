//! Byte-for-byte golden gate: the parse result (as a `Debug` dump) and the
//! serialized XML of every committed fixture and a set of edge documents must
//! equal the files in `tests/golden/`, which were generated from the
//! pre-quick-xml code on `origin/main` (see `tests/golden/README.md`). Set
//! `GOLDEN_BLESS=<dir>` to write the files instead of comparing.
#![cfg(feature = "std")]

use std::fs;
use std::path::{Path, PathBuf};

use ttml_subtitle::Document;

fn check(name: &str, actual: &str) {
    if let Ok(dir) = std::env::var("GOLDEN_BLESS") {
        let path = Path::new(&dir).join(name);
        fs::create_dir_all(&dir).expect("create golden dir");
        fs::write(&path, actual).expect("write golden");
        return;
    }
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name);
    let expected = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    assert_eq!(actual, expected, "{name} differs from the golden output");
}

fn golden_for(stem: &str, xml: &str) {
    let mut doc = Document::parse_str(xml).unwrap_or_else(|e| panic!("{stem}: {e}"));
    check(&format!("{stem}.parse.txt"), &format!("{doc:#?}\n"));
    check(&format!("{stem}.xml"), &doc.to_xml());
}

#[test]
fn committed_fixtures_match_golden() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut names: Vec<_> = fs::read_dir(&dir)
        .expect("fixtures dir")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "ttml"))
        .collect();
    names.sort();
    assert_eq!(names.len(), 11);
    for path in names {
        let stem = path
            .file_stem()
            .expect("stem")
            .to_string_lossy()
            .into_owned();
        golden_for(&stem, &fs::read_to_string(&path).expect("read fixture"));
    }
}

const EDGE_DOCS: &[(&str, &str)] = &[
    (
        "edge-specials",
        r#"<tt xmlns="http://www.w3.org/ns/ttml" xml:lang="en"><body><div><p begin="0s" end="1s" region="a&amp;b&lt;c&gt;d&quot;e&apos;f">x &amp; y &lt; z &gt; &quot;q&quot; 'a'</p></div></body></tt>"#,
    ),
    (
        "edge-mixed-content",
        r#"<tt xmlns="http://www.w3.org/ns/ttml" xml:lang="en"><body><div><p begin="0s" end="2s">Hello <span>big <span>nested</span></span> world<br/>next &amp;<!-- c --> line</p></div></body></tt>"#,
    ),
    (
        "edge-foreign",
        r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:acme="urn:acme:ext" xml:lang="en"><metadata acme:rating="PG"><acme:thing acme:id="t1">hello<acme:inner>deep</acme:inner></acme:thing></metadata><body><div acme:zone="a"><p begin="0s" end="5s" acme:cue="7"><span acme:inline="i1">Hello</span><br acme:br="b1"/><acme:para>px</acme:para></p></div></body></tt>"#,
    ),
    (
        "edge-metadata-pure-and-impure",
        r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:ttm="http://www.w3.org/ns/ttml#metadata" xmlns:ebuttm="urn:ebu:tt:metadata" xml:lang="en"><head><metadata><ttm:title>T &amp; U</ttm:title><ttm:desc><b/>x</ttm:desc><ebuttm:documentMetadata><ebuttm:conformsToStandard>urn:a</ebuttm:conformsToStandard></ebuttm:documentMetadata><ebuttm:documentMetadata><ebuttm:other/></ebuttm:documentMetadata></metadata></head><body/></tt>"#,
    ),
    (
        "edge-prefix-rebinding",
        r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:v="urn:one" xml:lang="en"><body><div><p begin="0s" end="1s"><span xmlns:v="urn:two" v:x="2">t</span></p></div></body></tt>"#,
    ),
];

#[test]
fn edge_documents_match_golden() {
    for (stem, xml) in EDGE_DOCS {
        golden_for(stem, xml);
    }
}
