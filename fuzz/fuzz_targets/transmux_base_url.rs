#![no_main]

use libfuzzer_sys::fuzz_target;

// Fuzz `transmux::base_url`'s BaseURL / URL-reference resolution (SP3) on
// arbitrary UTF-8 text, both as a reference and as a `BaseURL` chain entry,
// against the synthetic base. Must never panic on any input.
fuzz_target!(|data: &[u8]| {
    if let Ok(s) = core::str::from_utf8(data) {
        let _ = transmux::base_url::resolve(None, s);
        let _ = transmux::base_url::resolve_chain(None, &[s.into()], s);
    }
});
