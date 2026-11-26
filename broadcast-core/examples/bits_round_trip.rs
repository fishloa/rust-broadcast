//! Read and write a run of dense sub-byte fields, then read them back.
//!
//! DVB physical-layer signalling packs 1-bit flags, 3-bit codes and wide
//! sizes back to back with no padding. `BitWriter` and `BitReader` walk such
//! a stream most-significant bit first.
//!
//! Run with: `cargo run -p broadcast-core --example bits_round_trip`

use broadcast_core::bits::{BitReader, BitWriter};

fn main() {
    let mut buf = [0u8; 4];

    let mut writer = BitWriter::new(&mut buf);
    writer.write_bits(0b101, 3).expect("3-bit field");
    writer
        .write_bits(0x1FF, 9)
        .expect("9-bit field crossing a byte");
    writer.write_bool(true).expect("1-bit flag");
    writer.write_bits(0xBEEF, 16).expect("16-bit field");
    let written = writer.bits_written();
    writer.align_to_byte().expect("pad to a byte");

    let mut reader = BitReader::new(&buf);
    assert_eq!(reader.read_bits(3).expect("3-bit field"), 0b101);
    assert_eq!(reader.read_bits(9).expect("9-bit field"), 0x1FF);
    assert!(reader.read_bool().expect("1-bit flag"));
    assert_eq!(reader.read_bits(16).expect("16-bit field"), 0xBEEF);
    assert_eq!(reader.bits_read(), written);

    println!(
        "wrote {written} bits into {} bytes and read them back",
        buf.len()
    );
}
