#![no_main]

use broadcast_core::{Parse, Serialize};
use libfuzzer_sys::fuzz_target;

/// The parse must accept only a complete, compact box (`size` outside the
/// `0`/`1` sentinels and equal to the input length): the typed boxes keep the
/// body only, so a `largesize` or to-end form would not round-trip the header.
fn compact(input: &[u8]) -> bool {
    let mut size = [0u8; 4];
    size.copy_from_slice(&input[..4]);
    let size = u32::from_be_bytes(size);
    size > 1 && usize::try_from(size).ok() == Some(input.len())
}

fuzz_target!(|data: &[u8]| {
    // The `iTunSMPB` value is hex text, not a box; fuzz that parser on any
    // input and require a successful parse to reproduce its own text.
    if let Ok(text) = core::str::from_utf8(data)
        && let Ok(itunsmpb) = isobmff::metadata::Itunsmpb::parse_text(text)
    {
        assert_eq!(itunsmpb.text(), text);
        let _ = (
            itunsmpb.priming(),
            itunsmpb.padding(),
            itunsmpb.original_samples(),
        );
    }
    if data.len() < 8 || !compact(data) {
        return;
    }
    match &data[4..8] {
        b"udta" => {
            if let Ok(udta) = isobmff::metadata::Udta::parse(data) {
                let bytes = udta.try_to_bytes().expect("udta serialises");
                assert_eq!(bytes, data, "udta round trip");
            }
        }
        b"meta" => {
            if let Ok(meta) = isobmff::metadata::Meta::parse(data) {
                let bytes = meta.try_to_bytes().expect("meta serialises");
                assert_eq!(bytes, data, "meta round trip");
            }
        }
        _ => {}
    }
});
