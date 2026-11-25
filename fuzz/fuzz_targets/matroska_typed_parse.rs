#![no_main]

use broadcast_common::{Parse, Serialize};
use broadcast_matroska::matroska::{Document, Element};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(document) = Document::parse(data) {
        if let Ok(bytes) = document.try_to_bytes() {
            let _ = Document::parse(&bytes) == Ok(document);
        }
    }
    if let Ok(element) = Element::parse(data) {
        if let Ok(bytes) = element.try_to_bytes() {
            let _ = Element::parse(&bytes) == Ok(element);
        }
    }
});
