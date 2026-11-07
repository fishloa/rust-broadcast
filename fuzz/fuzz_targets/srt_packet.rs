#![no_main]

use libfuzzer_sys::fuzz_target;
use srt_runtime::SrtPacket;
use srt_runtime::packet::{ControlPacket, DataPacket, KeyMaterial};

fuzz_target!(|data: &[u8]| {
    let _ = KeyMaterial::parse(data);
    let _ = DataPacket::parse(data);
    let _ = ControlPacket::parse(data);

    let Ok(parsed) = SrtPacket::parse(data) else {
        return;
    };
    let mut buf = vec![0u8; parsed.serialized_len()];
    let written = parsed
        .serialize_into(&mut buf)
        .expect("a parsed packet serializes into serialized_len bytes");
    assert_eq!(written, buf.len());
    let reparsed = SrtPacket::parse(&buf).expect("serialized packet reparses");
    assert_eq!(parsed, reparsed, "srt packet parse->serialize->parse");
});
