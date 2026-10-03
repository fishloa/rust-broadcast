//! Truncation gate: cutting a committed fixture at ANY offset before the end of
//! its root end tag must be a structured `Error::XmlParse` (a network-truncated
//! configuration must never parse as a valid, shorter one). The only accepted
//! prefixes are the complete document plus trailing whitespace, asserted
//! explicitly as `Ok`.
#![cfg(feature = "std")]

use std::fs;
use std::path::PathBuf;

use dvb_mabr::{Error, MulticastGatewayConfiguration, MulticastServerConfiguration};

fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../fixtures/dvb-mabr")
        .join(name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"))
}

fn assert_every_truncation_errors<T: std::fmt::Debug>(
    name: &str,
    text: &str,
    parse: impl Fn(&str) -> Result<T, Error>,
) {
    // The prefix that is the whole document up to its root end tag.
    let complete = text.trim_end().len();
    assert!(complete < text.len() || parse(text).is_ok());
    for end in 1..text.len() {
        if !text.is_char_boundary(end) {
            continue;
        }
        let result = parse(&text[..end]);
        if end < complete {
            assert!(
                matches!(result, Err(Error::XmlParse(_))),
                "{name}: truncation at byte {end}/{} must be Err(XmlParse), got {result:?}",
                text.len()
            );
        } else {
            assert!(
                result.is_ok(),
                "{name}: prefix of {end} bytes is the complete document (plus whitespace) and must parse: {result:?}"
            );
        }
    }
    assert!(parse(text).is_ok(), "{name}: the full document parses");
}

#[test]
fn server_fixture_truncations_all_error() {
    let name = "annex-c1-server-config.xml";
    assert_every_truncation_errors(name, &fixture(name), |s| {
        MulticastServerConfiguration::parse_str(s)
    });
}

#[test]
fn gateway_fixture_truncations_all_error() {
    for name in [
        "annex-c2-gateway-bootstrap.xml",
        "annex-c3-gateway-config.xml",
    ] {
        assert_every_truncation_errors(name, &fixture(name), |s| {
            MulticastGatewayConfiguration::parse_str(s)
        });
    }
}
