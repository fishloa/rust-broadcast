//! Build an LTC codeword from typed fields and serialize it to wire bytes —
//! SMPTE ST 12-1:2014 §9.2.
//!
//! Run with `cargo run -p st12-1 --example build_frame`.

use broadcast_common::Serialize;
use st12_1::LtcFrame;

fn main() {
    let frame = LtcFrame {
        hours: 1,
        minutes: 23,
        seconds: 45,
        frames: 13,
        drop_frame_flag: false,
        color_frame_flag: true,
        flag_bit_27: true,
        flag_bit_43: true,
        flag_bit_58: false,
        flag_bit_59: true,
        user_bits: [1, 2, 3, 4, 5, 6, 7, 8],
    };

    let mut bytes = [0u8; st12_1::FRAME_LEN];
    frame.serialize_into(&mut bytes).expect("serialize");

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
        "time address: {:02}:{:02}:{:02}:{:02}",
        frame.hours, frame.minutes, frame.seconds, frame.frames
    );
    println!("drop_frame_flag:  {}", frame.drop_frame_flag);
    println!("color_frame_flag: {}", frame.color_frame_flag);
    println!("user bits: {:X?}", frame.user_bits);
}
