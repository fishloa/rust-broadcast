/// Build an ANC data PES packet from typed fields, serialize it, and dump the
/// wire bytes — including the 10-bit bit-packed ANC records.
///
/// ```sh
/// cargo run -p dvb-smpte2038 --example build_anc
/// ```
use dvb_smpte2038::{AncDataPacket, AncPacket};

fn main() {
    // Two ANC packets on the same line's PES, plus a few stuffing bytes.
    let pkt = AncDataPacket {
        pes_priority: false,
        copyright: false,
        original_or_copy: false,
        pts: 90_000, // 1.0 s at 90 kHz
        anc_packets: vec![
            AncPacket {
                c_not_y_channel_flag: false,
                line_number: 9,
                horizontal_offset: 0,
                did: 0x161,
                sdid: 0x101,
                data_count: 0x002, // low 8 bits => 2 user_data_words
                user_data_words: vec![0x2CF, 0x101],
                checksum: 0x233,
            },
            AncPacket {
                c_not_y_channel_flag: true,
                line_number: 0x2A,
                horizontal_offset: 0x10,
                did: 0x241,
                sdid: 0x102,
                data_count: 0x003, // 3 user_data_words
                user_data_words: vec![0x111, 0x222, 0x333],
                checksum: 0x1AB,
            },
        ],
        stuffing_bytes: 4,
    };

    let mut bytes = vec![0u8; pkt.serialized_len()];
    pkt.serialize_into(&mut bytes).unwrap();

    println!("ANC data PES packet: {} bytes", bytes.len());
    println!("PTS: {} (90 kHz units)", pkt.pts);
    println!("ANC packets: {}", pkt.anc_packets.len());
    for (i, anc) in pkt.anc_packets.iter().enumerate() {
        println!(
            "  [{i}] line={} h_off={} DID={:#05X} SDID={:#05X} data_count={:#05X} \
             udw_loop={} checksum={:#05X}",
            anc.line_number,
            anc.horizontal_offset,
            anc.did,
            anc.sdid,
            anc.data_count,
            anc.udw_loop_count(),
            anc.checksum,
        );
    }
    println!("stuffing bytes: {}", pkt.stuffing_bytes);
    print!("wire bytes:");
    for b in &bytes {
        print!(" {b:02X}");
    }
    println!();

    // Round-trip sanity.
    assert_eq!(AncDataPacket::parse(&bytes).unwrap(), pkt);
    println!("round-trip: OK");
}
