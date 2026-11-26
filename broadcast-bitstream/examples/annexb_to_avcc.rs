//! Convert Annex B to the length-prefixed form and back; the bytes must match.
//!
//! Usage: `cargo run -p broadcast-bitstream --example annexb_to_avcc -- <file>`
//! e.g. `fixtures/candidates/h264.annexb`.

use std::env;
use std::fs::File;
use std::io::Read;

use broadcast_bitstream::annexb::{
    annexb_to_length_prefixed, iter_length_prefixed_nals, length_prefixed_to_annexb,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = env::args()
        .nth(1)
        .ok_or("usage: annexb_to_avcc <annexb-file>")?;
    let mut file = File::open(&path)?;
    let len = usize::try_from(file.metadata()?.len())?;
    let mut annexb = vec![0u8; len];
    file.read_exact(&mut annexb)?;

    // The length-prefixed form is canonical (4-byte start codes, trailing
    // zero padding stripped), so round-trip through it once first.
    let canonical = length_prefixed_to_annexb(&annexb_to_length_prefixed(&annexb))?;
    let length_prefixed = annexb_to_length_prefixed(&canonical);
    let back = length_prefixed_to_annexb(&length_prefixed)?;
    if back != canonical {
        return Err("Annex B -> length-prefixed -> Annex B changed the bytes".into());
    }

    let nals = iter_length_prefixed_nals(&length_prefixed)?;
    println!(
        "{} NAL units, {} bytes Annex B -> {} bytes length-prefixed -> identical",
        nals.len(),
        canonical.len(),
        length_prefixed.len()
    );
    Ok(())
}
