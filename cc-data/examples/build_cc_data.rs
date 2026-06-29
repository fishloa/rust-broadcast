//! Advanced: build a `cc_data()` from typed triplets, serialize, and round-trip.
//!
//! Run with: `cargo run -p cc-data --example build_cc_data`

use broadcast_common::{Parse, Serialize};
use cc_data::{CcData, CcTriplet, CcType};

fn main() {
    let cc = CcData {
        process_cc_data_flag: true,
        triplets: vec![
            CcTriplet {
                cc_valid: true,
                cc_type: CcType::Dtvcc708Start,
                cc_data_1: 0xC1,
                cc_data_2: 0x02,
            },
            CcTriplet {
                cc_valid: true,
                cc_type: CcType::Ntsc608Field1,
                cc_data_1: 0x94,
                cc_data_2: 0x2C,
            },
        ],
    };

    let bytes = cc.to_bytes();
    println!("serialized {} bytes: {:02X?}", bytes.len(), bytes);

    let back = CcData::parse(&bytes).expect("round-trip parse");
    assert_eq!(back, cc, "round-trip must be lossless");
    println!(
        "round-trip OK — {} triplets ({} 608, {} 708)",
        back.triplets.len(),
        back.cea608().count(),
        back.cea708().count()
    );
}
