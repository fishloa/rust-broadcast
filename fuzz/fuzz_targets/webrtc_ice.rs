#![no_main]

use libfuzzer_sys::fuzz_target;
use webrtc_runtime::ice::{format_ice_server_links, parse_ice_server_links};

fuzz_target!(|data: &[u8]| {
    // WHIP/WHEP ICE server Link header parsing (RFC 8288): arbitrary header
    // values must not panic, and parse → format → reparse must be stable.
    if let Ok(header) = core::str::from_utf8(data) {
        let servers = parse_ice_server_links(header);
        if !servers.is_empty() {
            let formatted = format_ice_server_links(&servers);
            let reparsed = parse_ice_server_links(&formatted);
            assert_eq!(
                servers.len(),
                reparsed.len(),
                "ICE server link round-trip count mismatch"
            );
        }
    }
});
