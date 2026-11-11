#![no_main]

use broadcast_common::{Parse, Serialize};
use libfuzzer_sys::fuzz_target;
use rtmp_runtime::flv::{FlvHeader, FlvTag, tags};

// The FLV header/tag codec (Adobe FLV v10.1 Annex E §E.2, §E.4.1) must never
// panic on arbitrary bytes: the header parse, the strict tag walk it admits,
// and a re-parse of every complete tag.
fuzz_target!(|data: &[u8]| {
    let Ok(header) = FlvHeader::parse(data) else {
        return;
    };
    let _ = header.try_to_bytes();
    for tag in tags(data, &header) {
        if let Ok(tag) = tag {
            let len = tag.serialized_len().min(data.len());
            if let Some(again) = data.get(..len) {
                let _ = FlvTag::parse(again);
            }
        }
    }
});
