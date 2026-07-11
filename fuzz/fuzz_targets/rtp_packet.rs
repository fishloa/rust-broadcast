#![no_main]

use broadcast_common::{Parse, Serialize};
use libfuzzer_sys::fuzz_target;
use rtp_packet::RtpPacket;
use rtp_packet::rfc8285::{ExtensionElements, OneByteElements, TwoByteElements, parse_extensions};

fuzz_target!(|data: &[u8]| {
    // RFC 3550 §5.1 fixed header + round trip.
    if let Ok(pkt) = RtpPacket::parse(data) {
        let serialized = pkt.to_bytes();
        if let Ok(reparsed) = RtpPacket::parse(&serialized) {
            assert_eq!(
                serialized,
                reparsed.to_bytes(),
                "rtp packet roundtrip mismatch"
            );
        }

        // If the fuzzed packet happens to carry a header extension, also
        // exercise the RFC 8285 dispatch (issue #655) on it.
        if let Some(ext) = &pkt.extension {
            if let Ok(elements) = parse_extensions(ext) {
                match &elements {
                    ExtensionElements::OneByte(e) => {
                        let _ = e.to_bytes();
                    }
                    ExtensionElements::TwoByte(e) => {
                        let _ = e.to_bytes();
                    }
                }
            }
        }
    }

    // Directly fuzz both RFC 8285 element-container parsers on arbitrary
    // bytes, independent of RtpPacket framing, for much better coverage of
    // the padding/stop-marker/malformed-ID-0 scan logic (§4.1.2/§4.2/§4.3).
    if let Ok(one_byte) = OneByteElements::parse(data) {
        let serialized = one_byte.to_bytes();
        if let Ok(reparsed) = OneByteElements::parse(&serialized) {
            assert_eq!(
                serialized,
                reparsed.to_bytes(),
                "one-byte extension elements roundtrip mismatch"
            );
        }
    }
    if let Ok(two_byte) = TwoByteElements::parse(data) {
        let serialized = two_byte.to_bytes();
        if let Ok(reparsed) = TwoByteElements::parse(&serialized) {
            assert_eq!(
                serialized,
                reparsed.to_bytes(),
                "two-byte extension elements roundtrip mismatch"
            );
        }
    }
});
