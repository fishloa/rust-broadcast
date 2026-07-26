#![no_main]

use libfuzzer_sys::fuzz_target;
use rtmp_runtime::server::ServerSession;

// Top-level RTMP ingest entry point (#738): arbitrary bytes fed straight into
// the handshake→chunk→message→AMF0→session state machine must never panic,
// however malformed (partial handshake, bogus chunk headers, truncated AMF0,
// out-of-order connect/publish, ...).
fuzz_target!(|data: &[u8]| {
    let mut session = ServerSession::with_defaults();
    let _ = session.handle_data(data);
});
