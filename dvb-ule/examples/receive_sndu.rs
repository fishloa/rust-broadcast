/// Read the committed RFC 4326 Appendix B SNDU fixture, fragment it across two
/// MPEG-2 TS packet payloads, and reassemble it with `UleReceiver` — proving
/// PUSI + Payload-Pointer de-fragmentation.
///
/// ```sh
/// cargo run -p dvb-ule --example receive_sndu
/// ```
use std::fs;

use dvb_ule::{Sndu, UleReceiver};

fn main() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/appendix_b.bin");
    let sndu_bytes = match fs::read(path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("fixture not available ({e}); nothing to do");
            return;
        }
    };

    // Parse the whole SNDU once for reference.
    let sndu = Sndu::parse(&sndu_bytes).unwrap();
    println!("Appendix B SNDU: {} bytes", sndu_bytes.len());
    println!(
        "Type: {} (0x{:04X})",
        sndu.type_field,
        sndu.type_field.to_u16()
    );
    println!("PDU: {} bytes", sndu.pdu().len());

    // Fragment across two TS packet payloads at byte 20.
    let split = 20;
    let mut rx = UleReceiver::new();

    // Packet 1: PUSI=1, Payload Pointer = 0 (SNDU starts immediately).
    let mut p1 = vec![0x00u8]; // payload pointer
    p1.extend_from_slice(&sndu_bytes[..split]);
    let done = rx.push(&p1, true);
    println!(
        "after packet 1 (PUSI=1, PP=0): {} SNDU(s) complete",
        done.len()
    );
    assert!(done.is_empty(), "SNDU should still be in reassembly");

    // Packet 2: PUSI=0 continuation with the rest + 0xFF padding.
    let mut p2 = sndu_bytes[split..].to_vec();
    p2.extend_from_slice(&[0xFF, 0xFF, 0xFF]); // End Indicator + padding
    let done = rx.push(&p2, false);
    println!(
        "after packet 2 (PUSI=0 continuation): {} SNDU(s) complete",
        done.len()
    );

    assert_eq!(done.len(), 1, "exactly one reassembled SNDU");
    assert_eq!(done[0], sndu_bytes, "reassembled bytes match the fixture");
    let re = Sndu::parse(&done[0]).unwrap();
    assert_eq!(re, sndu, "reassembled SNDU parses identically");
    println!("reassembly byte-exact + CRC valid: OK");
}
