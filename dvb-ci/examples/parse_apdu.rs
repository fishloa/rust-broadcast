//! Parse an APDU with the unified [`AnyApdu`] dispatcher and round-trip it.
//!
//! Run with: `cargo run -p dvb-ci --example parse_apdu`

use dvb_ci::objects::ca_info::CaInfo;
use dvb_ci::AnyApdu;
use dvb_common::Serialize;

fn main() {
    // A ca_info() APDU (9F 80 31) advertising two CA_system_ids: 0x0500, 0x0B00.
    let info = CaInfo {
        ca_system_ids: vec![0x0500, 0x0B00],
    };
    let wire = info.to_bytes();
    println!("ca_info APDU   : {:02X?}", wire);

    // Route it through the tag dispatcher without knowing the type in advance.
    let any = AnyApdu::parse(&wire).expect("valid APDU");
    println!("dispatched as  : {} (tag {})", any.name(), any.tag());

    if let AnyApdu::CaInfo(ci) = &any {
        println!("CA_system_ids  : {:04X?}", ci.ca_system_ids);
    }

    // Symmetric serialize: AnyApdu round-trips byte-for-byte.
    let back = any.to_bytes();
    assert_eq!(back, wire);
    println!("round-trip     : OK ({} bytes)", back.len());

    // An unrecognised tag (Tenq 9F 88 07, an MMI high-level object not yet
    // implemented) is preserved losslessly as AnyApdu::Unknown.
    let unknown_wire = [0x9F, 0x88, 0x07, 0x02, 0xAA, 0xBB];
    let unknown = AnyApdu::parse(&unknown_wire).expect("valid APDU framing");
    println!(
        "unknown tag    : {} -> preserved, round-trips: {}",
        unknown.name(),
        unknown.to_bytes() == unknown_wire
    );
}
