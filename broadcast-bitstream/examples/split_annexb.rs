//! Split one Annex B elementary stream into NAL units and print their types.
//!
//! Usage: `cargo run -p broadcast-bitstream --example split_annexb -- <file>`
//! e.g. `fixtures/candidates/h264.annexb`.

use std::env;
use std::fs::File;
use std::io::Read;

use broadcast_bitstream::annexb::iter_annexb_nals;
use broadcast_bitstream::nal::{AvcNalUnitType, NalCodec, nal_unit_type};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = env::args()
        .nth(1)
        .ok_or("usage: split_annexb <annexb-file>")?;
    let mut file = File::open(&path)?;
    let len = usize::try_from(file.metadata()?.len())?;
    let mut data = vec![0u8; len];
    file.read_exact(&mut data)?;

    let mut count = 0usize;
    for (index, nal) in iter_annexb_nals(&data).enumerate() {
        let ty = nal_unit_type(NalCodec::Avc, nal)
            .map(AvcNalUnitType::from_u8)
            .map_or_else(|| "short".to_owned(), |t| t.to_string());
        println!("nal {index}: type {ty}, {} bytes", nal.len());
        count += 1;
    }
    println!("{count} NAL units in {} bytes", data.len());
    Ok(())
}
