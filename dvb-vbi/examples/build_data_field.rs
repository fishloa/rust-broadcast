/// Build a VBI PES data field (ETSI EN 301 775 §4.4) from typed fields —
/// a VPS unit, a WSS unit, and an EBU Teletext unit — serialize it (recomputing
/// each `data_unit_length`), and dump the wire bytes.
///
/// ```sh
/// cargo run -p dvb-vbi --example build_data_field
/// ```
use dvb_vbi::{
    DataField, DataUnit, FRAMING_CODE_EBU, LineHeader, TeletextDataField, VpsDataField,
    WssDataField,
};

fn main() {
    // VPS on line 16, first field.
    let vps = DataUnit::vps(VpsDataField {
        header: LineHeader::new(true, 16),
        vps_data_block: [
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D,
        ],
    });

    // WSS on line 23, first field, 14-bit payload 0x1234.
    let wss = DataUnit::wss(WssDataField {
        header: LineHeader::new(true, 23),
        wss_data_block: 0x1234,
    });

    // EBU Teletext (non-subtitle) on line 7, second field.
    let txt = DataUnit::teletext(
        dvb_vbi::DataUnitId::EbuTeletextNonSubtitle,
        TeletextDataField {
            header: LineHeader::new(false, 7),
            framing_code: FRAMING_CODE_EBU,
            txt_data_block: [0xAB; 42],
        },
    );

    // data_identifier 0x10 (EBU Teletext combined with VPS / WSS / ...).
    let field = DataField::new(0x10, vec![vps, wss, txt]);

    let mut bytes = vec![0u8; field.serialized_len()];
    let n = field.serialize_into(&mut bytes).unwrap();

    println!("PES data field: {n} bytes");
    println!("data_identifier: 0x{:02X}", field.data_identifier);
    println!("data units: {}", field.data_units.len());
    for u in &field.data_units {
        println!(
            "  {} (id 0x{:02X}), data_unit_length {}",
            u.id,
            u.id.to_u8(),
            u.data_unit_length()
        );
    }
    print!("wire bytes:");
    for b in &bytes {
        print!(" {b:02X}");
    }
    println!();

    // Round-trip sanity.
    assert_eq!(DataField::parse(&bytes).unwrap(), field);
    println!("round-trip: OK");
}
