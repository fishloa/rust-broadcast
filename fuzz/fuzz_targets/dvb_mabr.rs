#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Cap input to 256 KiB to prevent OOM from deeply nested XML
    if data.len() > 256 * 1024 {
        return;
    }
    // XML must be valid UTF-8; skip non-UTF-8 input
    if let Ok(xml) = core::str::from_utf8(data) {
        // Parse both MulticastServerConfiguration and MulticastGatewayConfiguration
        // — must not panic on malformed input
        let _ = dvb_mabr::config::MulticastServerConfiguration::parse_str(xml);
        let _ = dvb_mabr::config::MulticastGatewayConfiguration::parse_str(xml);
    }
});
