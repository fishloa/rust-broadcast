//! Parse the real ATSC 3.0 ROUTE FDT-Instance capture fixture
//! (`fixtures/atsc3/route-fdt-instance-2020-11-05.bin` — see
//! `fixtures/atsc3/PROVENANCE.md`) with [`atsc3_route::RoutePacket`], walk
//! its LCT header-extension chain (via `rmt-flute`) to find `EXT_FTI` and
//! `EXT_FDT`, decode the Codepoint, and print the recovered FDT-Instance XML.
//!
//! Run with `cargo run -p atsc3-route --example parse_route_fdt`.

use std::path::PathBuf;

use atsc3_route::RoutePacket;
use broadcast_common::Parse;

fn main() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("fixtures")
        .join("atsc3")
        .join("route-fdt-instance-2020-11-05.bin");
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));

    let pkt = RoutePacket::parse(&bytes).expect("parse ROUTE FDT-Instance packet");

    println!(
        "TSI={:?} TOI={:?} CP={} ({}) SPI={}",
        pkt.lct.tsi,
        pkt.lct.toi,
        pkt.lct.codepoint,
        pkt.codepoint(),
        pkt.spi()
    );
    println!("FEC Payload ID: {:?}", pkt.fec_payload_id);

    for ext in &pkt.lct.extensions {
        match ext.het {
            rmt_flute::ALC_HET_EXT_FTI => {
                println!("EXT_FTI HEC: {:02x?}", ext.content);
            }
            rmt_flute::HET_EXT_FDT => {
                let fdt = rmt_flute::ExtFdt::parse(ext.content).expect("EXT_FDT content");
                println!(
                    "EXT_FDT: FLUTE version={} instance_id={}",
                    fdt.version, fdt.instance_id
                );
            }
            other => println!("unrecognised extension HET={other}"),
        }
    }

    let xml = std::str::from_utf8(pkt.payload).expect("FDT-Instance payload is valid UTF-8");
    println!(
        "payload ({} bytes): {}",
        pkt.payload.len(),
        &xml[..80.min(xml.len())]
    );
    assert!(xml.contains("<FDT-Instance"));
}
