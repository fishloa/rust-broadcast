//! Walk one MPEG-1/2/2.5 audio elementary stream and print each frame's header
//! facts (version, layer, bitrate, sample rate, frame length).
//!
//! Usage: `cargo run -p broadcast-bitstream --example print_mpeg_audio_headers -- <file>`
//! e.g. `fixtures/audio/audio.mp3` (skips a leading ID3v2 tag) or an MP2
//! elementary stream carried by a capture.

use std::env;
use std::fs::File;
use std::io::Read;

use broadcast_bitstream::MpegAudioFrameHeader;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = env::args()
        .nth(1)
        .ok_or("usage: print_mpeg_audio_headers <mpeg-audio-file>")?;
    let mut file = File::open(&path)?;
    let len = usize::try_from(file.metadata()?.len())?;
    let mut data = vec![0u8; len];
    file.read_exact(&mut data)?;

    let mut off = id3_skip(&data).unwrap_or(0);
    let mut frames = 0usize;
    while off < data.len() {
        let Ok(hdr) = MpegAudioFrameHeader::parse(&data[off..]) else {
            break;
        };
        let frame_len = hdr.frame_length();
        if off + frame_len > data.len() {
            break;
        }
        println!(
            "frame {frames}: {}, {}, {} bit/s, {} Hz, {} ch, {} bytes, {} samples",
            hdr.version(),
            hdr.layer(),
            hdr.bitrate_bps(),
            hdr.sample_rate(),
            hdr.channels(),
            frame_len,
            hdr.samples_per_frame()
        );
        off += frame_len;
        frames += 1;
    }
    println!("{frames} MPEG audio frames in {len} bytes");
    Ok(())
}

/// The number of bytes a leading ID3v2 tag occupies, or `None` when the file
/// does not start with one (ID3v2 §3.1).
fn id3_skip(data: &[u8]) -> Option<usize> {
    const SIGNATURE: &[u8] = b"ID3";
    const HEADER_LEN: usize = 10;
    const FOOTER_LEN: usize = 10;
    const FLAGS_OFFSET: usize = 5;
    const FLAG_FOOTER: u8 = 0x10;
    const SIZE_OFFSET: usize = 6;
    const SIZE_END: usize = SIZE_OFFSET + 4;
    const SYNC_SAFE_BITS: u32 = 7;
    const SYNC_SAFE_MASK: u32 = 0x7F;
    if data.len() < HEADER_LEN || !data.starts_with(SIGNATURE) {
        return None;
    }
    let size = data[SIZE_OFFSET..SIZE_END].iter().fold(0u32, |acc, &byte| {
        (acc << SYNC_SAFE_BITS) | u32::from(byte) & SYNC_SAFE_MASK
    });
    let footer = if data[FLAGS_OFFSET] & FLAG_FOOTER != 0 {
        FOOTER_LEN
    } else {
        0
    };
    Some(HEADER_LEN + size as usize + footer)
}
