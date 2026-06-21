/// Build a FLUTE/ALC packet from typed fields — an LCT header (TOI = 0) with an
/// EXT_FDT extension, a Small-Block-Systematic FEC Payload ID, and an XML FDT
/// Instance payload — then serialize it (recomputing all flags + HDR_LEN) and
/// dump the wire bytes.
///
/// ```sh
/// cargo run -p dvb-flute --example build_lct
/// ```
use dvb_flute::{
    AlcPacket, ExtFdt, FecPayloadId128, LctHeader, FLUTE_VERSION, HET_EXT_FDT, LCT_VERSION,
};

fn main() {
    // TOI = 0 (FDT Instance), so the TOI field is present and zero (O=0, H=1
    // gives a 2-byte TOI; we use O=1 here for a clean 4-byte zero TOI).
    let cci = [0u8, 0, 0, 1];
    let tsi = [0u8, 0, 0, 0x2A]; // S=1, H=0 — non-zero TSI (required by ALC)
    let toi = [0u8, 0, 0, 0]; // O=1, H=0 — TOI = 0 (FDT Instance)

    // EXT_FDT (HET 192, fixed): FLUTE v2, FDT Instance ID 5.
    let ext_fdt = ExtFdt {
        version: FLUTE_VERSION,
        instance_id: 5,
    };
    let mut fdt_scratch = [0u8; 3];
    let ext = ext_fdt.to_extension(&mut fdt_scratch).unwrap();

    let lct = LctHeader {
        version: LCT_VERSION,
        psi: 0,
        close_session: false,
        close_object: false,
        codepoint: 0, // Compact No-Code FEC (Encoding ID 0)
        cci: &cci,
        tsi: &tsi,
        toi: &toi,
        extensions: vec![ext],
    };

    // FEC Payload ID (Small Block Systematic, fec_id 128/129).
    let fpid = FecPayloadId128 {
        source_block_number: 0,
        source_block_length: 1,
        encoding_symbol_id: 0,
    };
    let mut fpid_bytes = [0u8; 8];
    fpid.serialize_into(&mut fpid_bytes).unwrap();

    // A tiny (truncated) FDT Instance XML body as the packet payload. The XML
    // itself is out of scope of this crate — it rides as opaque payload.
    let xml = br#"<?xml version="1.0"?><FDT-Instance Expires="0"/>"#;

    let pkt = AlcPacket::new(lct, &fpid_bytes, xml);

    let mut bytes = vec![0u8; pkt.serialized_len()];
    let n = pkt.serialize_into(&mut bytes).unwrap();

    println!("ALC/FLUTE packet: {n} bytes");
    println!("LCT version: {}", pkt.lct.version);
    println!(
        "flags  C={} S={} O={} H={}",
        pkt.lct.c_flag(),
        pkt.lct.s_flag(),
        pkt.lct.o_flag(),
        pkt.lct.h_flag()
    );
    println!("HDR_LEN: {} words", pkt.lct.hdr_len());
    println!("TOI bytes: {:02X?} (0 => FDT Instance)", pkt.lct.toi);
    println!("EXT_FDT instance_id: {}", ext_fdt.instance_id);
    println!("FEC Payload ID: {:02X?}", pkt.fec_payload_id);
    println!("payload (XML, opaque): {} bytes", pkt.payload.len());
    print!("wire bytes:");
    for b in &bytes {
        print!(" {b:02X}");
    }
    println!();

    // Round-trip sanity (FEC Payload ID len = 8 for fec_id 128/129).
    let re = AlcPacket::parse(&bytes, dvb_flute::FEC_PAYLOAD_ID_128_LEN).unwrap();
    assert_eq!(re, pkt);
    // The EXT_FDT extension decodes back.
    let re_fdt = ExtFdt::parse(re.lct.extensions[0].content).unwrap();
    assert_eq!(re.lct.extensions[0].het, HET_EXT_FDT);
    assert_eq!(re_fdt, ext_fdt);
    println!("round-trip: OK");
}
