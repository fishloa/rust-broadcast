//! Advanced: implement the project's symmetric `Parse` / `Serialize` contract
//! for your own wire type, then round-trip it.
//!
//! Every wire structure in every `dvb-*` crate implements this same pair, and
//! is round-trip tested (parse → serialize → byte-identical). This shows the
//! shape you follow when adding a new type.
//!
//! Run with: `cargo run -p dvb-common --example implement_parse_serialize`

use dvb_common::{Parse, Serialize};

/// A toy 3-byte header: a 1-byte tag and a big-endian 16-bit value.
#[derive(Debug, PartialEq, Eq)]
struct Toy {
    tag: u8,
    value: u16,
}

const LEN: usize = 3;

#[derive(Debug)]
#[allow(dead_code)] // fields are surfaced via Debug in the error path
enum ToyError {
    TooShort { need: usize, have: usize },
}

impl<'a> Parse<'a> for Toy {
    type Error = ToyError;
    fn parse(bytes: &'a [u8]) -> Result<Self, Self::Error> {
        if bytes.len() < LEN {
            return Err(ToyError::TooShort {
                need: LEN,
                have: bytes.len(),
            });
        }
        Ok(Toy {
            tag: bytes[0],
            value: u16::from_be_bytes([bytes[1], bytes[2]]),
        })
    }
}

impl Serialize for Toy {
    type Error = ToyError;
    fn serialized_len(&self) -> usize {
        LEN
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        if buf.len() < LEN {
            return Err(ToyError::TooShort {
                need: LEN,
                have: buf.len(),
            });
        }
        buf[0] = self.tag;
        buf[1..3].copy_from_slice(&self.value.to_be_bytes());
        Ok(LEN)
    }
}

fn main() {
    let wire = [0x42, 0x12, 0x34];
    let toy = Toy::parse(&wire).expect("parse");
    println!("parsed: {toy:?}");

    let back = toy.to_bytes();
    println!("re-serialized: {back:02X?}");

    // The hard project invariant: parse → serialize is byte-identical.
    assert_eq!(back, wire);
    assert_eq!(Toy::parse(&back).unwrap(), toy);
    println!("round-trip holds ✔");
}
