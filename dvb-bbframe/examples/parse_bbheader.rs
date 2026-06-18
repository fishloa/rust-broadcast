//! Basic: parse a single DVB-S2 BBHEADER from raw bytes.
//!
//! Run with: `cargo run -p dvb-bbframe --example parse_bbheader`

use dvb_bbframe::crc::crc8;
use dvb_bbframe::header::Bbheader;

fn main() {
    // A 10-byte Normal-Mode BBHEADER: MATYPE-1/2, UPL, DFL, SYNC, SYNCD, CRC-8.
    // (UPL=1520 bits = a 188-byte TS packet, SYNC=0x47, TS/CCM stream.)
    #[rustfmt::skip]
    let mut header = [
        0xD8, 0x00, // MATYPE-1, MATYPE-2
        0x05, 0xF0, // UPL = 1520 bits
        0xE0, 0x30, // DFL = 57392 bits
        0x47,       // SYNC byte
        0x00, 0x00, // SYNCD
        0x00,       // CRC-8 — filled below
    ];
    // In Normal Mode the MODE field is folded into the CRC-8: a stored byte
    // equal to crc8(header[..9]) decodes back to Mode::Normal.
    header[9] = crc8(&header[..9]);

    let hdr = Bbheader::parse(&header).expect("valid BBHEADER");

    println!("mode    : {:?}", hdr.mode);
    println!("ts_gs   : {:?}", hdr.matype.ts_gs);
    println!("ccm     : {}", hdr.matype.ccm);
    println!("issyi   : {}", hdr.matype.issyi);
    println!("UPL     : {} bits", hdr.upl);
    println!("DFL     : {} bits", hdr.dfl);
    println!("SYNC    : {:#04X}", hdr.sync);
    println!("SYNCD   : {}", hdr.syncd);
}
