#![no_main]

use dvb_vbi::{DataField, DataUnit};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = DataUnit::parse(data);
    let Ok(parsed) = DataField::parse(data) else {
        return;
    };
    let mut buf = vec![0u8; parsed.serialized_len()];
    let written = parsed
        .serialize_into(&mut buf)
        .expect("a parsed data field serializes");
    assert_eq!(written, buf.len());
    let reparsed = DataField::parse(&buf).expect("serialized data field reparses");
    assert_eq!(parsed, reparsed, "vbi data field parse->serialize->parse");
});
