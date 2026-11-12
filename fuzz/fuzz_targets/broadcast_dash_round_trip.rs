#![no_main]

use broadcast_dash::Mpd;
use broadcast_dash::error::{MpdError, Unsupported};
use libfuzzer_sys::fuzz_target;
use test_fixtures::fuzz::MAX_INPUT;

// Fuzz the broadcast-dash MPD parser + writer round trip: parse, render, re-parse
// and require the model to be byte-identical. Must not panic on hostile input.
fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_INPUT {
        return;
    }
    let Ok(mpd) = Mpd::parse(data) else { return };
    let xml = match mpd.to_xml() {
        Ok(xml) => xml,
        Err(MpdError::Unsupported(Unsupported::EventXmlPayload)) => return,
        Err(e) => panic!("a parsed MPD renders: {e}"),
    };
    let reparsed = Mpd::parse(xml.as_bytes()).expect("the writer output parses");
    assert_eq!(reparsed, mpd);
});
