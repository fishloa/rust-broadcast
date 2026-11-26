//! Build and serialize an `ftyp` box.
//!
//! ```bash
//! cargo run -p isobmff --example build_ftyp
//! ```

use broadcast_core::Serialize;
use isobmff::segments::FileTypeBox;

fn main() {
    let ftyp = FileTypeBox {
        major_brand: *b"iso5",
        minor_version: 0,
        compatible_brands: vec![*b"iso5", *b"iso6", *b"mp41"],
    };
    let bytes = ftyp.try_to_bytes().expect("serialize ftyp");
    println!("{} bytes", bytes.len());
    let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02x}")).collect();
    println!("{}", hex.join(" "));
    let reparsed = FileTypeBox::parse_box(&bytes).expect("parse ftyp");
    assert_eq!(reparsed, ftyp);
}
