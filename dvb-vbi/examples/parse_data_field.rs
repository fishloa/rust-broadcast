/// Read the committed VBI PES data-field fixture, parse it into typed data
/// units, print a summary, and confirm a byte-exact round-trip.
///
/// ```sh
/// cargo run -p dvb-vbi --example parse_data_field
/// ```
use std::fs;

use dvb_vbi::{DataField, DataUnitPayload};

fn main() {
    // Fixtures live in the workspace-shared `fixtures/` tree, not under the
    // crate. A committed fixture that cannot be read is a bug, not a reason
    // to skip — so a missing/unreadable fixture is a hard failure.
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/dvb-vbi/vbi_data_field.bin"
    );
    let bytes = fs::read(path)
        .unwrap_or_else(|e| panic!("committed fixture {path} could not be read: {e}"));

    let field = DataField::parse(&bytes).expect("fixture must parse");
    println!("PES data field: {} bytes", bytes.len());
    println!("data_identifier: 0x{:02X}", field.data_identifier);
    println!("data units: {}", field.data_units.len());

    for u in &field.data_units {
        print!(
            "  {} (id 0x{:02X}), data_unit_length {}",
            u.id,
            u.id.to_u8(),
            u.data_unit_length()
        );
        match &u.payload {
            DataUnitPayload::Vps(f) => print!(" — line_offset {}", f.header.line_offset),
            DataUnitPayload::Wss(f) => print!(" — wss 0x{:04X}", f.wss_data_block),
            DataUnitPayload::ClosedCaptioning(f) => {
                print!(" — cc 0x{:04X}", f.closed_captioning_data_block)
            }
            DataUnitPayload::Teletext(f) => print!(" — framing 0x{:02X}", f.framing_code),
            DataUnitPayload::Monochrome(f) => print!(" — {} samples", f.samples.len()),
            DataUnitPayload::Stuffing { length } => print!(" — {length} stuffing bytes"),
            DataUnitPayload::Opaque(b) => print!(" — {} opaque bytes", b.len()),
            _ => {}
        }
        println!();
    }

    // Byte-exact round-trip.
    let mut out = vec![0u8; field.serialized_len()];
    let n = field.serialize_into(&mut out).unwrap();
    assert_eq!(n, bytes.len());
    assert_eq!(
        out, bytes,
        "serialize must be byte-identical to the fixture"
    );
    assert_eq!(DataField::parse(&out).unwrap(), field);
    println!("round-trip byte-exact: OK");
}
