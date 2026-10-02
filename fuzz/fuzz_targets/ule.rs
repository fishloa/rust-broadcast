#![no_main]

use libfuzzer_sys::fuzz_target;
use ule::{Sndu, UleReceiver};

fuzz_target!(|data: &[u8]| {
    // RFC 4326 SNDU: a successful parse (which already validated the CRC-32
    // trailer) must re-serialize to a canonical form: exactly
    // `serialized_len()` bytes, re-parsing equal and re-serializing identically.
    if let Ok(sndu) = Sndu::parse(data) {
        let len = sndu.serialized_len();
        let mut out = vec![0u8; len];
        let n = sndu
            .serialize_into(&mut out)
            .unwrap_or_else(|e| panic!("Sndu: serialize of parsed value failed: {e}"));
        assert_eq!(n, len, "Sndu: serialized_len disagrees with bytes written");
        let again = Sndu::parse(&out).expect("Sndu: re-parse of own output");
        assert_eq!(again, sndu, "Sndu: re-parse differs");
        let mut out2 = vec![0u8; len];
        again.serialize_into(&mut out2).unwrap();
        assert_eq!(out, out2, "Sndu: serialization not canonical");
    }

    // TS-level receiver: feed the input as TS payloads; it must never panic,
    // and every SNDU it emits is a complete wire SNDU.
    let mut rx = UleReceiver::new();
    for (i, chunk) in data.chunks(ule::TS_PAYLOAD_LEN).enumerate() {
        for bytes in rx.push(chunk, i % 2 == 0) {
            if let Ok(s) = Sndu::parse(&bytes) {
                let mut out = vec![0u8; s.serialized_len()];
                s.serialize_into(&mut out).expect("emitted SNDU serializes");
            }
        }
    }
});
