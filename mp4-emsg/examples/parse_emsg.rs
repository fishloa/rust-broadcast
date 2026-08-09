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
    // Fixtures live in the workspace-shared `fixtures/` tree, not under the
    // crate. A committed fixture that cannot be read is a bug, not a reason
    // to skip — so a missing/unreadable fixture is a hard failure.
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/shared/scte35_emsg_v0.bin"
    );
    let bytes = fs::read(path)
        .unwrap_or_else(|e| panic!("committed fixture {path} could not be read: {e}"));

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
    if b.is_scte35()
        && let Some(&first) = b.message_data.first()
    {
        // The SCTE 35 splice_info_section starts with table_id 0xFC.
        println!("  message_data[0] = 0x{first:02X} (SCTE 35 table_id)");
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
