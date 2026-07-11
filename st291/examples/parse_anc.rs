/// Parse the committed ANC fixture at runtime, report the decoded ANC packets,
/// and verify a byte-exact round-trip.
///
/// ```sh
/// cargo run -p st291 --example parse_anc
/// ```
use std::fs;

use st291::AncDataPacket;

fn main() {
    // Resolve relative to the crate so it runs from any cwd.
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/st291/anc.bin");
    let data = match fs::read(path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("fixture not available ({e}); nothing to do");
            return;
        }
    };

    let pkt = AncDataPacket::parse(&data).unwrap();

    println!("File: {path}");
    println!("Size: {} bytes", data.len());
    println!("PTS: {} (90 kHz units)", pkt.pts);
    println!("PES_priority={}", pkt.pes_priority);
    println!("ANC packets: {}", pkt.anc_packets.len());
    for (i, anc) in pkt.anc_packets.iter().enumerate() {
        print!(
            "  [{i}] c_not_y={} line={} h_off={} DID={:#05X} SDID={:#05X} \
             data_count={:#05X} udw=[",
            anc.c_not_y_channel_flag,
            anc.line_number,
            anc.horizontal_offset,
            anc.did,
            anc.sdid,
            anc.data_count,
        );
        for (j, w) in anc.user_data_words.iter().enumerate() {
            if j > 0 {
                print!(", ");
            }
            print!("{w:#05X}");
        }
        println!("] checksum={:#05X}", anc.checksum);
    }
    println!("stuffing bytes: {}", pkt.stuffing_bytes);

    // Byte-exact round-trip.
    let mut out = vec![0u8; pkt.serialized_len()];
    pkt.serialize_into(&mut out).unwrap();
    assert_eq!(
        out, data,
        "round-trip must be byte-identical to the fixture"
    );
    println!("byte-exact round-trip: OK");
}
