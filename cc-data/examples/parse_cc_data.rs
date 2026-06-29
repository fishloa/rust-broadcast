//! Basic: parse a `cc_data()` byte sequence and split CEA-608 / CEA-708 triplets.
//!
//! Run with: `cargo run -p cc-data --example parse_cc_data`

use cc_data::CcData;
use broadcast_common::Parse;

fn main() {
    // cc_data: process_cc_data_flag=1, cc_count=2; one DTVCC-start triplet + one
    // 608-field-1 triplet; trailing 0xFF marker.
    #[rustfmt::skip]
    let bytes = [
        0b1100_0010, // reserved=1, process=1, zero=0, cc_count=2
        0xFF,        // reserved
        0xFF, 0xC1, 0x02, // one_bit+rsvd(F8) | valid=1 | type=3 (708 start); data C1 02
        0xFC, 0x94, 0x2C, // F8 | valid=1 | type=0 (608 field1); data 94 2C
        0xFF,        // marker
    ];

    let cc = CcData::parse(&bytes).expect("valid cc_data");
    println!("process_cc_data_flag : {}", cc.process_cc_data_flag);
    println!("triplets             : {}", cc.triplets.len());
    for t in &cc.triplets {
        println!(
            "  {:<16} valid={} data={:02X} {:02X}",
            t.cc_type.name(),
            t.cc_valid,
            t.cc_data_1,
            t.cc_data_2
        );
    }
    println!("CEA-608 triplets     : {}", cc.cea608().count());
    println!("CEA-708 triplets     : {}", cc.cea708().count());
}
