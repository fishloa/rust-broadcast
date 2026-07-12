//! Build an ST 337 burst from typed preamble fields + a payload, and
//! serialize it to wire bytes — SMPTE ST 337:2015 §7.
//!
//! Run with `cargo run -p st337 --example build_burst`.

use broadcast_common::Serialize;
use st337::{Burst, DataMode};

fn main() {
    // A minimal four-word-preamble burst carrying an arbitrary payload.
    // `data_type` 1 is used here purely as an example value (this crate does
    // not define a data_type -> codec mapping -- see docs/st337.md).
    let payload = b"example non-PCM burst payload bytes";
    let burst = Burst::new(
        1,                // data_type
        DataMode::Mode16, // data_mode -- the only mode this crate supports
        false,            // error_flag
        0,                // data_type_dependent
        0,                // data_stream_number (0 = main audio service)
        None,             // extended (Pe/Pf) -- only used when data_type == 31
        payload,
    )
    .expect("build burst");

    let mut bytes = vec![0u8; burst.serialized_len()];
    burst.serialize_into(&mut bytes).expect("serialize");

    println!("serialized {} bytes:", bytes.len());
    println!(
        "{}",
        bytes
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    println!(
        "Pa={:#06x} Pb={:#06x} length_code={} bits",
        u16::from_le_bytes([bytes[0], bytes[1]]),
        u16::from_le_bytes([bytes[2], bytes[3]]),
        burst.preamble.length_code
    );
}
