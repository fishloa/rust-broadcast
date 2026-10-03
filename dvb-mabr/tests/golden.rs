//! Byte-for-byte golden gate: the parse result (as a `Debug` dump) and the
//! serialized XML of every committed fixture must equal the files in
//! `tests/golden/`, which were generated from the pre-quick-xml code on
//! `origin/main` (see `tests/golden/README.md`). Set `GOLDEN_BLESS=<dir>` to
//! write the files instead of comparing.
#![cfg(feature = "std")]

use std::fs;
use std::path::{Path, PathBuf};

use dvb_mabr::{MulticastGatewayConfiguration, MulticastServerConfiguration};

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

fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../fixtures/dvb-mabr")
        .join(name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"))
}

#[test]
fn server_fixture_matches_golden() {
    let parsed = MulticastServerConfiguration::parse_str(&fixture("annex-c1-server-config.xml"))
        .expect("parse");
    check("annex-c1.parse.txt", &format!("{parsed:#?}\n"));
    check("annex-c1.xml", &parsed.to_xml());
}

#[test]
fn gateway_fixtures_match_golden() {
    for (file, stem) in [
        ("annex-c2-gateway-bootstrap.xml", "annex-c2"),
        ("annex-c3-gateway-config.xml", "annex-c3"),
    ] {
        let parsed = MulticastGatewayConfiguration::parse_str(&fixture(file)).expect("parse");
        check(&format!("{stem}.parse.txt"), &format!("{parsed:#?}\n"));
        check(&format!("{stem}.xml"), &parsed.to_xml());
    }
}

/// Specials in attributes and text: `& < > " '` escaped exactly as before.
#[test]
fn specials_document_matches_golden() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<MulticastGatewayConfiguration schemaVersion="2" xmlns="urn:dvb:metadata:MulticastSessionConfiguration:2024">
  <MulticastSession serviceIdentifier="a&amp;b&lt;c&gt;d&quot;e&apos;f">
    <PresentationManifestLocator manifestId="m1" contentType="application/dash+xml">https://x/?a=1&amp;b=&lt;2&gt; &quot;q&quot; 'z'</PresentationManifestLocator>
  </MulticastSession>
</MulticastGatewayConfiguration>"#;
    let parsed = MulticastGatewayConfiguration::parse_str(xml).expect("parse");
    check("specials.parse.txt", &format!("{parsed:#?}\n"));
    check("specials.xml", &parsed.to_xml());
}
