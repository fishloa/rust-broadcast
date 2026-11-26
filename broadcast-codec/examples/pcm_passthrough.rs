//! Drive a 16-bit PCM packet through a decoder and an encoder and prove the
//! bytes come back unchanged.

#[path = "common/pcm.rs"]
mod pcm;

use std::error::Error;
use std::num::NonZeroU32;

use broadcast_codec::{ChannelLayout, Decoder, Encoder, Packet};

/// Samples per second of the example stream.
const SAMPLE_RATE: u32 = 48_000;
/// Samples in one packet.
const PACKET_SAMPLES: usize = 1024;
/// Packets in the example stream.
const PACKET_COUNT: usize = 12;
/// Bytes in one signed 16-bit little-endian sample.
const BYTES_PER_SAMPLE: usize = 2;
/// Deterministic sample step, so the tone is reproducible.
const SAMPLE_STEP: i16 = 97;

fn drain(
    decoder: &mut impl Decoder,
    encoder: &mut impl Encoder,
    output: &mut Vec<u8>,
) -> Result<(), Box<dyn Error>> {
    while let Some(frame) = decoder.receive()? {
        encoder.send(frame)?;
        while let Some(packet) = encoder.receive()? {
            output.extend_from_slice(packet.data());
        }
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let sample_rate = NonZeroU32::new(SAMPLE_RATE).ok_or("sample rate must be non-zero")?;
    let channels = ChannelLayout::Mono;

    let mut input = Vec::new();
    for index in 0..PACKET_SAMPLES * PACKET_COUNT {
        let sample = (index as i16).wrapping_mul(SAMPLE_STEP);
        input.extend_from_slice(&sample.to_le_bytes());
    }

    let mut decoder = pcm::PcmDecoder::new(sample_rate, channels);
    let mut encoder = pcm::PcmEncoder::new(sample_rate, channels);
    let mut output = Vec::new();
    let packet_bytes = PACKET_SAMPLES * BYTES_PER_SAMPLE;

    for (index, chunk) in input.chunks(packet_bytes).enumerate() {
        decoder.send(Packet::borrowed(chunk).with_pts(index as i64))?;
        drain(&mut decoder, &mut encoder, &mut output)?;
    }
    decoder.finish()?;
    drain(&mut decoder, &mut encoder, &mut output)?;
    encoder.finish()?;
    drain(&mut decoder, &mut encoder, &mut output)?;

    if output != input {
        return Err(format!(
            "round trip mismatch: {} bytes in, {} bytes out",
            input.len(),
            output.len()
        )
        .into());
    }
    println!("PCM passthrough round-tripped {} bytes", input.len());
    Ok(())
}
