/// Build an MPEG-DASH `emsg` box from typed fields, serialize it (recomputing
/// the box `size` + `version`), and dump the wire bytes. Builds one version 0
/// (segment-relative) and one version 1 (representation-relative) box to show
/// the v0/v1 field-ordering difference.
///
/// ```sh
/// cargo run -p dvb-emsg --example build_emsg
/// ```
use dvb_emsg::{EmsgBox, PresentationTime};

fn dump(label: &str, b: &EmsgBox) {
    let bytes = b.to_vec().unwrap();
    println!("{label}: {} bytes, version {}", bytes.len(), b.version());
    print!("  wire:");
    for x in &bytes {
        print!(" {x:02X}");
    }
    println!();
    // Round-trip: parse re-validates the structure.
    assert_eq!(EmsgBox::parse(&bytes).unwrap(), *b);
    println!("  round-trip: OK");
}

fn main() {
    // A short SCTE 35 splice_info_section payload (truncated; just illustrative).
    let scte35 = [0xFCu8, 0x30, 0x11, 0x00, 0x00];

    // version 0 — segment-relative (presentation_time_delta). Strings first.
    let v0 = EmsgBox {
        scheme_id_uri: "urn:scte:scte35:2013:bin",
        value: "",
        timescale: 90_000,
        presentation_time: PresentationTime::Delta(0),
        event_duration: 0xFFFF_FFFF,
        id: 1,
        message_data: &scte35,
    };
    println!("is_scte35 (v0): {}", v0.is_scte35());
    dump("v0", &v0);

    // version 1 — representation-relative (presentation_time u64). Integers
    // first, then the strings.
    let v1 = EmsgBox {
        scheme_id_uri: "https://aomedia.org/emsg/ID3",
        value: "0",
        timescale: 1000,
        presentation_time: PresentationTime::Absolute(123_456_789),
        event_duration: 0,
        id: 42,
        message_data: b"ID3 metadata payload",
    };
    dump("v1", &v1);
}
