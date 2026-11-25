#![no_main]

use broadcast_common::Serialize;
use broadcast_matroska::{Element, Limits};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let limits = Limits::default();
    let mut position = 0usize;
    while position < data.len() {
        let rest = &data[position..];
        let Ok((element, used)) = Element::parse_at(rest, &limits) else {
            break;
        };
        if used == 0 || used > rest.len() {
            break;
        }
        if let Ok(bytes) = element.try_to_bytes() {
            let _ = bytes == rest[..used];
        }
        position += used;
    }
});
