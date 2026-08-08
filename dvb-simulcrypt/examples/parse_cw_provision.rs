/// Read the committed `cw_provision.bin` fixture (an ECMG⇔SCS `CW_provision`
/// message), parse it against the ECMG⇔SCS interface, walk its parameters
/// (treating the CW inside `CP_CW_combination` as opaque), and byte-exact
/// round-trip it.
///
/// ```sh
/// cargo run -p dvb-simulcrypt --example parse_cw_provision
/// ```
use std::fs;

use broadcast_common::traits::Serialize;
use dvb_simulcrypt::{
    EcmgScsParameterType, Interface, MessageType, ParameterType, SimulcryptMessage,
};

fn main() {
    // Fixtures live in the workspace-shared `fixtures/` tree, not under the
    // crate. A committed fixture that cannot be read is a bug, not a reason
    // to skip — so a missing/unreadable fixture is a hard failure.
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/dvb-simulcrypt/cw_provision.bin"
    );
    let bytes = fs::read(path)
        .unwrap_or_else(|e| panic!("committed fixture {path} could not be read: {e}"));

    let msg = SimulcryptMessage::parse_on(Interface::EcmgScs, &bytes)
        .expect("CW_provision fixture must parse");

    println!("interface: {}", msg.interface());
    println!(
        "message_type: {} (0x{:04X})",
        msg.message_type,
        msg.message_type.to_u16()
    );
    assert!(matches!(
        msg.message_type,
        MessageType::EcmgScs(dvb_simulcrypt::EcmgScsMessageType::CwProvision)
    ));

    for p in &msg.parameters {
        print!(
            "  {} (0x{:04X}), {} bytes:",
            p.ptype,
            p.ptype.to_u16(),
            p.value.len()
        );
        for b in p.value {
            print!(" {b:02X}");
        }
        println!();
    }

    // The CP_CW_combination's CW is opaque — we only locate the parameter.
    // Guard: the parameter must carry at least 2 bytes for the CP number.
    if let Some(cpcw) = msg.find(ParameterType::EcmgScs(
        EcmgScsParameterType::CpCwCombination,
    )) {
        if let Some(cp_bytes) = cpcw.value.get(0..2) {
            let cp = u16::from_be_bytes([cp_bytes[0], cp_bytes[1]]);
            println!(
                "CP_CW_combination: CP={cp}, CW={} opaque bytes",
                cpcw.value.len() - 2
            );
        } else {
            println!(
                "CP_CW_combination: value too short ({} bytes, need ≥2)",
                cpcw.value.len()
            );
        }
    }

    // Byte-exact round-trip.
    let mut out = vec![0u8; msg.serialized_len()];
    let n = msg.serialize_into(&mut out).unwrap();
    assert_eq!(n, bytes.len());
    assert_eq!(
        out, bytes,
        "serialize must reproduce the fixture byte-for-byte"
    );
    println!("byte-exact round-trip: OK");
}
