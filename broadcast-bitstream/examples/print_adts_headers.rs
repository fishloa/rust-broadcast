//! Walk one ADTS elementary stream and print each frame's header facts.
//!
//! Usage: `cargo run -p broadcast-bitstream --example print_adts_headers -- <file>`
//! e.g. `fixtures/audio/aac.adts`.

use std::env;
use std::fs::File;
use std::io::Read;

use broadcast_bitstream::aac_asc::{SamplingFrequencyIndex, adts_header_len, parse_adts_header};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = env::args()
        .nth(1)
        .ok_or("usage: print_adts_headers <adts-file>")?;
    let mut file = File::open(&path)?;
    let len = usize::try_from(file.metadata()?.len())?;
    let mut data = vec![0u8; len];
    file.read_exact(&mut data)?;

    let mut off = 0usize;
    let mut frames = 0usize;
    while off < data.len() {
        let hdr = parse_adts_header(&data[off..])?;
        let len = hdr.frame_length() as usize;
        if len < adts_header_len(&hdr) || off + len > data.len() {
            break;
        }
        let core_aot = hdr.profile() + 1;
        let rate = SamplingFrequencyIndex::from(hdr.sampling_frequency_index())
            .table_hz()
            .map_or_else(|| "explicit".to_owned(), |hz| format!("{hz} Hz"));
        println!(
            "frame {frames}: AOT {core_aot}, {rate}, channel_configuration {}, {len} bytes",
            hdr.channel_configuration()
        );
        off += len;
        frames += 1;
    }
    println!("{frames} ADTS frames in {} bytes", data.len());
    Ok(())
}
