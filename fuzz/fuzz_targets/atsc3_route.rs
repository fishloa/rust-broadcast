#![no_main]

use broadcast_common::{Parse, Serialize};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // ATSC A/331 Annex A ROUTE binary framing: SourceFecPayloadId,
    // RepairFecPayloadId, ExtRoutePresentationTime, and RoutePacket all
    // implement Parse/Serialize. Fuzz each with byte-exact round-trip
    // validation — must not panic on malformed input, and a successful
    // parse must round-trip byte-identically.

    // Test 1: SourceFecPayloadId (Compact No-Code FEC scheme, §A.3.5.1)
    if let Ok(id) = atsc3_route::fec::SourceFecPayloadId::parse(data) {
        let reserialized = id.to_bytes();
        if let Ok(reparsed) = atsc3_route::fec::SourceFecPayloadId::parse(&reserialized) {
            assert_eq!(
                reserialized,
                reparsed.to_bytes(),
                "SourceFecPayloadId roundtrip mismatch"
            );
        }
    }

    // Test 2: RepairFecPayloadId (RaptorQ FEC scheme, §A.3.5.2)
    if let Ok(id) = atsc3_route::fec::RepairFecPayloadId::parse(data) {
        let reserialized = id.to_bytes();
        if let Ok(reparsed) = atsc3_route::fec::RepairFecPayloadId::parse(&reserialized) {
            assert_eq!(
                reserialized,
                reparsed.to_bytes(),
                "RepairFecPayloadId roundtrip mismatch"
            );
        }
    }

    // Test 3: ExtRoutePresentationTime (EXT_ROUTE_PRESENTATION_TIME content, §A.3.7.1)
    if let Ok(ext) = atsc3_route::ExtRoutePresentationTime::parse(data) {
        let reserialized = ext.to_bytes();
        if let Ok(reparsed) = atsc3_route::ExtRoutePresentationTime::parse(&reserialized) {
            assert_eq!(
                reserialized,
                reparsed.to_bytes(),
                "ExtRoutePresentationTime roundtrip mismatch"
            );
        }
    }

    // Test 4: RoutePacket (composed ALC/LCT packet, §A.3.4/§A.3.6)
    if let Ok(pkt) = atsc3_route::RoutePacket::parse(data) {
        let reserialized = pkt.to_bytes();
        if let Ok(reparsed) = atsc3_route::RoutePacket::parse(&reserialized) {
            assert_eq!(
                reserialized,
                reparsed.to_bytes(),
                "RoutePacket roundtrip mismatch"
            );
        }
    }
});
