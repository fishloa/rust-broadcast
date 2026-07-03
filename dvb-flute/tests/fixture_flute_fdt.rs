//! FLUTE FDT-packet fixture: parse the committed `flute_fdt.bin` (an ALC/LCT
//! packet built to the RFC 6726 §3.4 FDT-packet shape — TOI = 0, an EXT_FDT
//! extension at HET 192, an 8-byte Small-Block-Systematic FEC Payload ID, and a
//! truncated FDT-Instance XML payload), assert the decoded LCT flag-driven
//! field widths and EXT_FDT fields, and verify a byte-exact round-trip.

use std::fs;

use dvb_flute::{
    AlcPacket, ExtFdt, FEC_PAYLOAD_ID_128_LEN, FLUTE_VERSION, FecPayloadId128, HET_EXT_FDT, TOI_FDT,
};

fn fixture() -> Vec<u8> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/dvb-flute/flute_fdt.bin"
    );
    fs::read(path).expect("fixture flute_fdt.bin must be committed")
}

#[test]
fn parses_flute_fdt_lct_widths() {
    let data = fixture();
    let pkt = AlcPacket::parse(&data, FEC_PAYLOAD_ID_128_LEN).unwrap();

    // LCT v1, and the flag-driven widths reconstructed from the wire.
    assert_eq!(pkt.lct.version, 1);
    assert_eq!(pkt.lct.c_flag(), 0); // CCI 4 bytes
    assert_eq!(pkt.lct.s_flag(), 1); // TSI 4 bytes
    assert_eq!(pkt.lct.o_flag(), 1); // TOI 4 bytes
    assert_eq!(pkt.lct.h_flag(), 0);
    assert_eq!(pkt.lct.cci.len(), 4);
    assert_eq!(pkt.lct.tsi.len(), 4);
    assert_eq!(pkt.lct.toi.len(), 4);

    // HDR_LEN = (4 fixed + 4 CCI + 4 TSI + 4 TOI + 4 EXT_FDT) / 4 = 5 words.
    assert_eq!(pkt.lct.hdr_len(), 5);

    // TOI = 0 ⇒ FDT Instance.
    assert_eq!(pkt.lct.toi, &TOI_FDT.to_be_bytes());
}

#[test]
fn parses_ext_fdt() {
    let data = fixture();
    let pkt = AlcPacket::parse(&data, FEC_PAYLOAD_ID_128_LEN).unwrap();

    assert_eq!(pkt.lct.extensions.len(), 1);
    let ext = &pkt.lct.extensions[0];
    assert_eq!(ext.het, HET_EXT_FDT);
    assert!(ext.is_fixed(), "EXT_FDT (HET 192) is fixed-length");
    assert_eq!(ext.serialized_len(), 4);

    let fdt = ExtFdt::parse(ext.content).unwrap();
    assert_eq!(fdt.version, FLUTE_VERSION);
    assert_eq!(fdt.instance_id, 5);
}

#[test]
fn parses_fec_payload_id_and_opaque_xml() {
    let data = fixture();
    let pkt = AlcPacket::parse(&data, FEC_PAYLOAD_ID_128_LEN).unwrap();

    let fpid = FecPayloadId128::parse(pkt.fec_payload_id).unwrap();
    assert_eq!(fpid.source_block_number, 0);
    assert_eq!(fpid.source_block_length, 1);
    assert_eq!(fpid.encoding_symbol_id, 0);

    // The FDT Instance body is XML — opaque to this crate.
    let xml = core::str::from_utf8(pkt.payload).unwrap();
    assert!(xml.starts_with("<?xml"));
    assert!(xml.contains("FDT-Instance"));
}

#[test]
fn byte_exact_round_trip() {
    let data = fixture();
    let pkt = AlcPacket::parse(&data, FEC_PAYLOAD_ID_128_LEN).unwrap();

    let mut out = vec![0u8; pkt.serialized_len()];
    let n = pkt.serialize_into(&mut out).unwrap();
    assert_eq!(n, data.len());
    assert_eq!(out, data, "serialize must be byte-identical to the fixture");

    // serialize → parse → equal.
    assert_eq!(AlcPacket::parse(&out, FEC_PAYLOAD_ID_128_LEN).unwrap(), pkt);
}

#[test]
fn truncated_header_is_rejected() {
    let data = fixture();
    // Cut the buffer mid-LCT-header (the HDR_LEN claims 20 header bytes).
    let short = &data[..10];
    assert!(AlcPacket::parse(short, FEC_PAYLOAD_ID_128_LEN).is_err());
}
