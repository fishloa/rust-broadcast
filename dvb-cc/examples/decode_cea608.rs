//! Decode a CEA-608 (line-21) pop-on caption to on-screen text.
//!
//! Run with: `cargo run -p dvb-cc --example decode_cea608`

use dvb_cc::decode::{Cea608Channel, Cea608Decoder};

/// Add odd parity to a 7-bit value so the pairs look like real line-21 bytes.
fn par(v: u8) -> u8 {
    if (v & 0x7F).count_ones() % 2 == 0 {
        v | 0x80
    } else {
        v & 0x7F
    }
}

fn main() {
    let mut dec = Cea608Decoder::new();

    // A pop-on caption on CC1 (field 1):
    //   RCL  (14 20)  — Resume Caption Loading (pop-on)
    //   PAC  (14 70)  — row 15, white, indent 0
    //   "HELLO"
    //   EOC  (14 2F)  — flip back buffer to the screen
    let pairs: &[(u8, u8)] = &[
        (0x14, 0x20),
        (0x14, 0x70),
        (b'H', b'E'),
        (b'L', b'L'),
        (b'O', 0x00),
        (0x14, 0x2F),
    ];
    for &(b1, b2) in pairs {
        dec.push_pair(false, par(b1), par(b2));
    }

    println!("CC1 mode : {}", dec.mode(Cea608Channel::Cc1));
    println!("CC1 text : {:?}", dec.channel_text(Cea608Channel::Cc1));

    // A roll-up caption (2 rows) on CC1.
    let mut dec2 = Cea608Decoder::new();
    let roll: &[(u8, u8)] = &[
        (0x14, 0x25), // RU2
        (b'O', b'N'),
        (b'E', 0x00),
        (0x14, 0x2D), // CR
        (b'T', b'W'),
        (b'O', 0x00),
    ];
    for &(b1, b2) in roll {
        dec2.push_pair(false, par(b1), par(b2));
    }
    println!("\nRoll-up mode : {}", dec2.mode(Cea608Channel::Cc1));
    println!("Roll-up text :\n{}", dec2.channel_text(Cea608Channel::Cc1));
}
