//! Build a ROUTE source-flow media-segment fragment from typed fields —
//! the same shape as the real capture fixture
//! `fixtures/atsc3/route-media-video-fragment-2020-11-05.bin` (TSI 3000,
//! TOI 6034, `CP` = 128, a `EXT_FTI` extension, and a Compact No-Code
//! `start_offset` FEC Payload ID) — and serialize it to wire bytes.
//!
//! Run with `cargo run -p atsc3-route --example build_route_media_fragment`.

use atsc3_route::{RouteFecPayloadId, RoutePacket, SourceFecPayloadId};
use broadcast_common::{Parse, Serialize};
use rmt_flute::{HeaderExtension, LctHeader};

fn main() {
    let cci = [0u8; 4]; // C = 00, ROUTE-mandated
    let tsi = 3000u32.to_be_bytes(); // S = 1, H = 0
    let toi = 6034u32.to_be_bytes(); // O = 01, H = 0

    // EXT_FTI (HET=64, HEL=4): 14-byte HEC, matching the real fixture's
    // shape (exact field semantics of the HEC beyond byte 4-5 are
    // unresolved — see `fixtures/atsc3/PROVENANCE.md`).
    let fti_hec = [0x00u8, 0x00, 0x00, 0x06, 0xff, 0xf3, 0, 0, 0, 0, 0, 0, 0, 0];
    let extensions = vec![HeaderExtension::new(rmt_flute::ALC_HET_EXT_FTI, &fti_hec)];

    let lct = LctHeader {
        version: rmt_flute::LCT_VERSION,
        psi: rmt_flute::PSI_SPI, // source flow
        close_session: false,
        close_object: false,
        codepoint: 128, // indirect -- resolved via SrcFlow.Payload@codePoint=128 in the S-TSID
        cci: &cci,
        tsi: &tsi,
        toi: &toi,
        extensions,
    };

    let payload = vec![0xABu8; 1408]; // one DASH-segment fragment, opaque to this crate

    let pkt = RoutePacket {
        lct,
        fec_payload_id: RouteFecPayloadId::Source(SourceFecPayloadId {
            start_offset: 107_008,
        }),
        payload: &payload,
    };

    let bytes = pkt.to_bytes();
    println!("serialized {} bytes", bytes.len());
    println!("first 32 bytes (LCT header): {:02x?}", &bytes[..32]);

    let re = RoutePacket::parse(&bytes).expect("round-trip parse");
    assert_eq!(re, pkt);
    println!(
        "round trip OK: TSI={:?} TOI={:?} CP={} SPI={}",
        re.lct.tsi,
        re.lct.toi,
        re.codepoint(),
        re.spi()
    );
}
