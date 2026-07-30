#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Cap input to 256 KiB to prevent OOM from deeply nested XML
    if data.len() > 256 * 1024 {
        return;
    }
    // XML must be valid UTF-8; skip non-UTF-8 input
    if let Ok(xml) = core::str::from_utf8(data) {
        // Parse and validate — must not panic
        let _ = ttml_subtitle::Document::parse_str(xml);
    }
});
