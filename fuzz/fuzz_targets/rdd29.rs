#![no_main]

use broadcast_common::{Parse, Serialize};
use libfuzzer_sys::fuzz_target;
use rdd29::AtmosFrame;

fuzz_target!(|data: &[u8]| {
    let Ok(frame) = AtmosFrame::parse(data) else {
        return;
    };
    let serialized = frame.to_bytes();

    // The serialized form must parse back to the same value.
    let reparsed = AtmosFrame::parse(&serialized).expect("serialized frame must reparse");
    assert_eq!(reparsed, frame, "parse(serialize(frame)) != frame");

    // Byte-exact against the INPUT (not a re-serialization), over the parsed extent.
    // A non-minimal Plex(n) escape in the input (docs/rdd29.md §3.4) decodes
    // unambiguously but re-serializes shorter, so exactness is only required when the
    // lengths agree; any shortening is covered by the reparse check above.
    if serialized.len() <= data.len() {
        let extent_matches_length =
            AtmosFrame::parse(&data[..serialized.len()]).is_ok_and(|f| f == frame);
        if extent_matches_length {
            assert_eq!(
                serialized,
                &data[..serialized.len()],
                "ATMOSFrame round trip is not byte-exact over the parsed extent"
            );
        }
    }
});
