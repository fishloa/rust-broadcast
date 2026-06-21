/// Build a ULE SNDU from typed fields, serialize it (recomputing Length +
/// CRC-32), and dump the wire bytes.
///
/// ```sh
/// cargo run -p dvb-ule --example build_sndu
/// ```
use dvb_ule::{Sndu, TypeField};

fn main() {
    // An IPv6 SNDU with an NPA destination address (D=0), carrying a short
    // opaque PDU. Mirrors the RFC 4326 Appendix B shape (Type 0x86DD, D=0).
    let pdu = [
        0x60, 0x00, 0x00, 0x00, 0x00, 0x04, 0x3a, 0x40, 0xDE, 0xAD, 0xBE, 0xEF,
    ];
    let sndu = Sndu::new(
        TypeField::EtherType(0x86DD),
        Some([0x00, 0x01, 0x02, 0x03, 0x04, 0x05]),
        &pdu,
    );

    let mut bytes = vec![0u8; sndu.serialized_len()];
    let n = sndu.serialize_into(&mut bytes).unwrap();

    println!("SNDU: {n} bytes");
    println!("D-bit: {}", u8::from(sndu.d_bit()));
    println!(
        "Type: {} (0x{:04X})",
        sndu.type_field(),
        sndu.type_field().to_u16()
    );
    println!("Length field: {}", sndu.length_field());
    if let Some(npa) = sndu.dest_address {
        print!("NPA dest:");
        for b in &npa {
            print!(" {b:02X}");
        }
        println!();
    }
    print!("wire bytes:");
    for b in &bytes {
        print!(" {b:02X}");
    }
    println!();

    // Round-trip sanity (parse re-validates the CRC).
    assert_eq!(Sndu::parse(&bytes).unwrap(), sndu);
    println!("round-trip (incl. CRC check): OK");
}
