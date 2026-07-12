//! Parse a spec-derived LTC codeword (see `docs/st12-1.md`'s "Worked vector"
//! section), print its decoded fields — including the `FrameRate`-dependent
//! polarity-correction and binary-group-flag bits — then round-trip it back
//! to bytes and confirm the output is byte-identical.
//!
//! Run with `cargo run -p st12-1 --example parse_frame`.

use broadcast_common::{Parse, Serialize};
use st12_1::{FrameRate, LtcFrame};

fn main() {
    // Time Address = 01:23:45:13 (docs/st12-1.md's worked example), computed
    // independently from ST 12-1 Tables 2-5, not by round-tripping through
    // this crate's own serializer.
    #[rustfmt::skip]
    let bytes: [u8; st12_1::FRAME_LEN] =
        [0x13, 0x29, 0x35, 0x4C, 0x53, 0x6A, 0x71, 0x88, 0xFC, 0xBF];

    let frame = LtcFrame::parse(&bytes).expect("parse LTC codeword");
    println!(
        "time address: {:02}:{:02}:{:02}:{:02}",
        frame.hours, frame.minutes, frame.seconds, frame.frames
    );
    println!("drop_frame_flag:  {}", frame.drop_frame_flag);
    println!("color_frame_flag: {}", frame.color_frame_flag);
    println!("user bits: {:X?}", frame.user_bits);

    for rate in [FrameRate::Fps30, FrameRate::Fps25, FrameRate::Fps24] {
        let bgf = frame.binary_group_flags(rate);
        println!(
            "{rate}: polarity_correction={} binary_group_usage={}",
            frame.polarity_correction(rate),
            bgf.usage()
        );
    }

    let mut out = [0u8; st12_1::FRAME_LEN];
    frame.serialize_into(&mut out).expect("serialize");
    assert_eq!(out, bytes, "byte-identical round trip");
    println!("round trip byte-identical: OK ({} bytes)", out.len());
}
