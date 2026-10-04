#![no_main]

use libfuzzer_sys::fuzz_target;
use rtsp_runtime::{SessionHeader, Transport};

// RFC 2326 §12.39 `Transport` and §12.37 `Session` header parsers (owner decision (c): rtsp-runtime
// owns these grammars). Arbitrary text must never panic, and every accepted value must satisfy the
// round-trip invariants: parse -> serialize -> parse is equal, and the canonical form is idempotent.
fuzz_target!(|data: &[u8]| {
    let Ok(s) = core::str::from_utf8(data) else {
        return;
    };
    if let Ok(a) = Transport::parse(s) {
        let canon = a.to_header_value().unwrap();
        let b = Transport::parse(&canon).expect("canonical Transport must re-parse");
        assert_eq!(a, b);
        assert_eq!(b.to_header_value().unwrap(), canon);
    }
    if let Ok(a) = SessionHeader::parse(s) {
        // Receive is lenient (any non-empty id without control characters is kept verbatim so
        // it can be echoed back) while emit is strict: an id that would swallow the `;`
        // separator (e.g. a lone `"`) parses but cannot be serialised unambiguously. That
        // asymmetry is documented (docs/session-header.md) and unit-tested, so the harness
        // accepts exactly `HeaderSerialize` there and holds every other value to the
        // round-trip invariants.
        match a.to_header_value() {
            Ok(canon) => {
                let b = SessionHeader::parse(&canon).expect("canonical Session must re-parse");
                assert_eq!(a, b);
                assert_eq!(b.to_header_value().unwrap(), canon);
            }
            Err(rtsp_runtime::Error::HeaderSerialize(_)) => {}
            Err(e) => panic!("unexpected Session serialise error: {e:?}"),
        }
    }
});
