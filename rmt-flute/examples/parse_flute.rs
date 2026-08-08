/// Read the committed FLUTE FDT-packet fixture, parse the ALC/LCT framing +
/// EXT_FDT extension, and report the decoded fields (the XML FDT Instance body
/// is left opaque — out of scope of this binary crate).
///
/// ```sh
/// cargo run -p rmt-flute --example parse_flute
/// ```
use std::fs;

use rmt_flute::{AlcPacket, ExtFdt, FEC_PAYLOAD_ID_128_LEN, FecPayloadId128, HET_EXT_FDT};

fn main() {
    // Fixtures live in the workspace-shared `fixtures/rmt-flute/` directory,
    // not under the crate. This example previously pointed at a
    // `tests/fixtures/` path that has never existed, and its "nothing to do"
    // fallback silently swallowed the failure — so it never actually ran.
    // Missing fixture is now a hard error: an example that quietly does
    // nothing is worse than one that fails.
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/rmt-flute/flute_fdt.bin"
    );
    let data = fs::read(path)
        .unwrap_or_else(|e| panic!("committed fixture {path} could not be read: {e}"));

    // FLUTE default FEC is Compact No-Code (Encoding ID 0): a 16-bit Source
    // Block Number + 16-bit Encoding Symbol ID = 4 bytes. But this fixture uses
    // the Small-Block-Systematic 8-byte FEC Payload ID for a richer example.
    let pkt = AlcPacket::parse(&data, FEC_PAYLOAD_ID_128_LEN).unwrap();

    println!("ALC/FLUTE packet: {} bytes", data.len());
    println!("LCT version: {}", pkt.lct.version);
    println!(
        "flags  C={} S={} O={} H={}  close_session={} close_object={}",
        pkt.lct.c_flag(),
        pkt.lct.s_flag(),
        pkt.lct.o_flag(),
        pkt.lct.h_flag(),
        pkt.lct.close_session,
        pkt.lct.close_object,
    );
    println!("HDR_LEN: {} words", pkt.lct.hdr_len());
    println!("CCI: {:02X?}", pkt.lct.cci);
    println!("TSI: {:02X?}", pkt.lct.tsi);
    println!("TOI: {:02X?}  (0 => FDT Instance)", pkt.lct.toi);

    for ext in &pkt.lct.extensions {
        if ext.het == HET_EXT_FDT {
            let fdt = ExtFdt::parse(ext.content).unwrap();
            println!(
                "EXT_FDT: FLUTE v{} FDT Instance ID {}",
                fdt.version, fdt.instance_id
            );
        } else {
            println!("extension HET {} ({} bytes)", ext.het, ext.serialized_len());
        }
    }

    let fpid = FecPayloadId128::parse(pkt.fec_payload_id).unwrap();
    println!(
        "FEC Payload ID: sbn={} sbl={} esi={}",
        fpid.source_block_number, fpid.source_block_length, fpid.encoding_symbol_id
    );
    println!(
        "FDT Instance XML payload ({} bytes, opaque): {}",
        pkt.payload.len(),
        core::str::from_utf8(pkt.payload).unwrap_or("<non-utf8>")
    );

    // Byte-exact round-trip.
    let mut out = vec![0u8; pkt.serialized_len()];
    let n = pkt.serialize_into(&mut out).unwrap();
    assert_eq!(n, data.len());
    assert_eq!(out, data, "serialize must be byte-identical to the fixture");
    println!("byte-exact round-trip: OK");
}
