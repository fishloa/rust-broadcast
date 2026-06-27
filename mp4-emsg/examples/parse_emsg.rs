/// Read the committed `scte35_emsg_v0.bin` fixture (a version 0 `emsg` carrying
/// a real SCTE 35 splice_info_section in `message_data`), parse it, print the
/// decoded fields, and prove a byte-exact round-trip + recomputed `size`.
///
/// ```sh
/// cargo run -p mp4-emsg --example parse_emsg
/// ```
use std::fs;

use mp4_emsg::EmsgBox;

fn main() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/scte35_emsg_v0.bin"
    );
    let bytes = match fs::read(path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("fixture not available ({e}); nothing to do");
            return;
        }
    };

    let b = EmsgBox::parse(&bytes).unwrap();
    println!("emsg box: {} bytes (version {})", bytes.len(), b.version());
    println!("  scheme_id_uri: {:?}", b.scheme_id_uri);
    println!("  value:         {:?}", b.value);
    println!("  timescale:     {}", b.timescale);
    println!("  presentation:  {:?}", b.presentation_time);
    println!("  event_duration:{}", b.event_duration);
    println!("  id:            {}", b.id);
    println!("  message_data:  {} bytes", b.message_data.len());
    println!("  is_scte35:     {}", b.is_scte35());
    if b.is_scte35() {
        if let Some(&first) = b.message_data.first() {
            // The SCTE 35 splice_info_section starts with table_id 0xFC.
            println!("  message_data[0] = 0x{first:02X} (SCTE 35 table_id)");
        }
    }

    // Byte-exact round-trip: serialize recomputes the size field.
    let out = b.to_vec().unwrap();
    assert_eq!(
        out, bytes,
        "serialize must be byte-identical to the fixture"
    );
    assert_eq!(out.len(), b.serialized_len());
    println!("round-trip byte-exact + size recomputed: OK");
}
