#![no_main]

use libfuzzer_sys::fuzz_target;

// Fuzz the Smooth Streaming client Manifest on arbitrary UTF-8 text (#738,
// moved to broadcast-smooth in #2356). The parser handles untrusted remote
// XML and must not panic on any input, however malformed; when it does accept
// a document, serializing the model and parsing it again must give the same
// model.
fuzz_target!(|data: &[u8]| {
    if let Ok(s) = core::str::from_utf8(data) {
        if let Ok(manifest) = broadcast_smooth::SmoothManifest::parse(s) {
            let xml = manifest.to_xml();
            let reparsed = broadcast_smooth::SmoothManifest::parse(&xml)
                .expect("a serialized manifest re-parses");
            assert_eq!(reparsed, manifest, "the model round-trips through its serializer");
        }
    }
});
