//! Basic: the shared building blocks — MPEG-2 CRC-32 and BCD coding.
//!
//! Run with: `cargo run -p dvb-common --example crc_and_bcd`

use dvb_common::{bcd, crc32_mpeg2};

fn main() {
    // Every CRC-bearing PSI/SI section is protected by this CRC-32/MPEG-2.
    let payload = [0xDE, 0xAD, 0xBE, 0xEF];
    let crc = crc32_mpeg2::compute(&payload);
    println!("CRC-32/MPEG-2 of {payload:02X?} = {crc:#010X}");

    // Appending the CRC and re-running over the whole buffer yields 0 — the
    // standard "is this section intact?" check.
    let mut framed = payload.to_vec();
    framed.extend_from_slice(&crc.to_be_bytes());
    assert_eq!(
        crc32_mpeg2::compute(&framed),
        0,
        "CRC over data+CRC is zero"
    );
    println!(
        "verify (data+CRC) = {:#010X} (0 ⇒ intact)",
        crc32_mpeg2::compute(&framed)
    );

    // BCD is how SI encodes things like service IDs and dates.
    let byte = bcd::to_bcd_byte(42).expect("0..=99 fits one BCD byte");
    println!("42 as BCD       = {byte:#04X}");
    assert_eq!(bcd::from_bcd_byte(byte), Some(42));
}
