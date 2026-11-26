//! Shared fixture parsing and comparison for the spike's tests.

use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use broadcast_codec::{
    AudioParameters, AudioSamples, ChannelLayout, CodecId, CodecParameters, CodecProvider, Decoder,
    Direction, Encoder, EncoderConfig, Frame, Packet, Registry, Timescale,
};
use codec_symphonia::adts::AdtsFrames;
use oxideav_mp3::frame::FrameWalker;
use test_fixtures::process::{output_bounded, tool_available};

/// The AAC fixture's sample rate.
pub const SAMPLE_RATE: u32 = 48_000;
/// Samples one AAC access unit decodes to.
pub const AAC_FRAME_SAMPLES: usize = 1024;
/// The largest priming delay the alignment search tries, in samples.
const MAX_PRIMING: i64 = 4096;
/// Bytes per f32 sample.
const F32_BYTES: usize = 4;
/// The deadline for an ffmpeg run.
const FFMPEG_DEADLINE: Duration = Duration::from_secs(60);
/// Bytes of PCM fed to the decoder per packet.
const PCM_PACKET_BYTES: usize = 4096;
/// Bytes in one signed 16-bit PCM sample.
const PCM_SAMPLE_BYTES: usize = 2;
/// The Ogg page header length up to the segment count.
const OGG_HEADER_BYTES: usize = 27;
/// Ogg's final lacing value: the packet continues on the next page.
const OGG_CONTINUED_LACING: u8 = 255;
/// The fixed WAV header before the first chunk.
const WAV_HEADER_BYTES: usize = 12;
/// A RIFF chunk's id and length.
const WAV_CHUNK_HEADER_BYTES: usize = 8;

/// The mono audio parameters for `codec`.
pub fn mono_parameters(codec: CodecId) -> CodecParameters {
    let rate = std::num::NonZeroU32::new(SAMPLE_RATE).expect("a non-zero sample rate");
    CodecParameters::new(codec, Timescale::from_nonzero(rate))
        .with_audio(AudioParameters::new(rate, ChannelLayout::Mono))
}

/// One AAC packet per ADTS frame in the committed fixture.
pub fn aac_packets() -> Vec<Vec<u8>> {
    let data = test_fixtures::read("audio/aac.adts");
    AdtsFrames::new(&data)
        .map(|frame| frame.bytes().to_vec())
        .collect()
}

/// One MP3 packet per MPEG audio frame in the committed fixture.
pub fn mp3_packets() -> Vec<Vec<u8>> {
    let data = test_fixtures::read("audio/audio.mp3");
    FrameWalker::new(&data)
        .map(|frame| frame.data.to_vec())
        .collect()
}

/// The OpusHead and the audio packets from the committed Ogg fixture.
pub fn opus_stream() -> (Vec<u8>, Vec<Vec<u8>>) {
    let data = test_fixtures::read("audio/opus.ogg");
    let mut packets = Vec::new();
    let mut partial = Vec::new();
    let mut pos = 0;
    while pos + OGG_HEADER_BYTES <= data.len() {
        assert_eq!(&data[pos..pos + 4], b"OggS");
        let segments = usize::from(data[pos + 26]);
        let table = pos + OGG_HEADER_BYTES;
        let mut payload = table + segments;
        let mut end = payload;
        for index in 0..segments {
            end += usize::from(data[table + index]);
        }
        assert!(end <= data.len());
        let mut cursor = payload;
        for index in 0..segments {
            let lace = data[table + index];
            let length = usize::from(lace);
            partial.extend_from_slice(&data[cursor..cursor + length]);
            cursor += length;
            if lace < OGG_CONTINUED_LACING {
                packets.push(std::mem::take(&mut partial));
            }
        }
        payload = end;
        pos = payload;
        assert_eq!(cursor, payload);
    }
    assert!(packets.len() > 2);
    let head = packets.remove(0);
    let _tags = packets.remove(0);
    (head, packets)
}

/// The raw sample bytes from the committed WAV fixture.
pub fn pcm_bytes() -> Vec<u8> {
    let data = test_fixtures::read("audio/pcm-s16le.wav");
    assert_eq!(&data[..4], b"RIFF");
    assert_eq!(&data[8..12], b"WAVE");
    let mut pos = WAV_HEADER_BYTES;
    while pos + WAV_CHUNK_HEADER_BYTES <= data.len() {
        let id = &data[pos..pos + 4];
        let size = u32::from_le_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]])
            as usize;
        let body = pos + WAV_CHUNK_HEADER_BYTES;
        if id == b"data" {
            return data[body..body + size].to_vec();
        }
        pos = body + size + (size & 1);
    }
    panic!("the WAV fixture has no data chunk");
}

/// Split PCM bytes into aligned packets.
pub fn pcm_packets(bytes: &[u8]) -> Vec<Vec<u8>> {
    assert!(bytes.len().is_multiple_of(PCM_SAMPLE_BYTES));
    bytes.chunks(PCM_PACKET_BYTES).map(<[u8]>::to_vec).collect()
}

/// Decode every packet through a `Box<dyn Decoder>` and return mono f32.
pub fn decode_mono(
    registry: &Registry,
    parameters: &CodecParameters,
    packets: &[Vec<u8>],
) -> Vec<f32> {
    let mut decoder = registry.decoder(parameters).expect("a decoder");
    let mut decoded = Vec::new();
    for packet in packets {
        decoder
            .send(Packet::borrowed(packet))
            .expect("the packet is accepted");
        drain(&mut *decoder, &mut decoded);
    }
    decoder.finish().expect("end of input is accepted");
    drain(&mut *decoder, &mut decoded);
    decoded
}

/// Drain one decoder into `decoded`.
fn drain(decoder: &mut dyn Decoder, decoded: &mut Vec<f32>) {
    while let Some(frame) = decoder.receive().expect("receive succeeds") {
        let Frame::Audio(audio) = frame else {
            panic!("an audio decoder emitted a non-audio frame");
        };
        assert_eq!(audio.channels(), ChannelLayout::Mono);
        assert_eq!(
            audio.sample_rate(),
            std::num::NonZeroU32::new(SAMPLE_RATE).expect("non-zero")
        );
        match audio.samples() {
            AudioSamples::S16(values) => {
                decoded.extend(values.iter().map(|s| f32::from(*s) / 32768.0))
            }
            AudioSamples::F32(values) => decoded.extend_from_slice(values),
            other => panic!("unexpected decoded sample format {other:?}"),
        }
    }
}

/// Decode `fixture` with ffmpeg into mono f32, or `None` when it is absent.
pub fn ffmpeg_f32(fixture: &str) -> Option<Vec<f32>> {
    if !tool_available("ffmpeg", "-version") {
        eprintln!("skip: ffmpeg is not on PATH, so the external decode did NOT run");
        return None;
    }
    let mut command = Command::new("ffmpeg");
    command
        .arg("-v")
        .arg("error")
        .arg("-i")
        .arg(test_fixtures::path(fixture))
        .arg("-f")
        .arg("f32le")
        .arg("-ar")
        .arg(SAMPLE_RATE.to_string())
        .arg("-ac")
        .arg("1")
        .arg("-");
    let output = output_bounded(&mut command, FFMPEG_DEADLINE).expect("ffmpeg runs");
    assert!(
        output.status.success(),
        "ffmpeg failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Some(
        output
            .stdout
            .chunks_exact(F32_BYTES)
            .map(|sample| f32::from_le_bytes([sample[0], sample[1], sample[2], sample[3]]))
            .collect(),
    )
}

/// Align `b` against `a` by the offset with the smallest overlapping RMS.
///
/// Equal-length streams are compared at offset 0. Streams of different lengths
/// are searched over a bounded priming delay. Returns the offset (leading
/// samples of `a` before `b[0]`), the overlap length, the largest absolute
/// difference and the RMS difference.
pub fn align(a: &[f32], b: &[f32]) -> (i64, usize, f32, f32) {
    assert!(!a.is_empty() && !b.is_empty());
    if a.len() == b.len() {
        return difference(a, 0, b, 0, a.len(), 0);
    }
    let mut best: Option<(i64, usize, f32, f32)> = None;
    for offset in -MAX_PRIMING..=MAX_PRIMING {
        let Some((a_start, b_start, overlap)) = overlap(a.len(), b.len(), offset) else {
            continue;
        };
        let candidate = difference(a, a_start, b, b_start, overlap, offset);
        if best.is_none_or(|(_, _, _, current)| candidate.3 < current) {
            best = Some(candidate);
        }
    }
    best.expect("the two streams overlap")
}

/// The difference between two aligned spans.
fn difference(
    a: &[f32],
    a_start: usize,
    b: &[f32],
    b_start: usize,
    overlap: usize,
    offset: i64,
) -> (i64, usize, f32, f32) {
    let mut largest = 0.0_f32;
    let mut sum = 0.0_f64;
    for index in 0..overlap {
        let value = (a[a_start + index] - b[b_start + index]).abs();
        largest = largest.max(value);
        sum += f64::from(value) * f64::from(value);
    }
    let rms = (sum / overlap as f64).sqrt() as f32;
    (offset, overlap, largest, rms)
}

/// The overlapping ranges for `offset`.
fn overlap(a_len: usize, b_len: usize, offset: i64) -> Option<(usize, usize, usize)> {
    if offset >= 0 {
        let start = usize::try_from(offset).ok()?;
        let overlap = a_len.saturating_sub(start).min(b_len);
        (overlap > 0).then_some((start, 0, overlap))
    } else {
        let start = usize::try_from(-offset).ok()?;
        let overlap = b_len.saturating_sub(start).min(a_len);
        (overlap > 0).then_some((0, start, overlap))
    }
}

/// A provider that counts how often its decoder factory was used.
pub struct CountingProvider {
    inner: Box<dyn CodecProvider>,
    calls: Arc<AtomicUsize>,
}

impl CountingProvider {
    /// Wrap `inner` and hand back the shared counter.
    pub fn new(inner: Box<dyn CodecProvider>) -> (Self, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        (
            Self {
                inner,
                calls: Arc::clone(&calls),
            },
            calls,
        )
    }
}

impl CodecProvider for CountingProvider {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn info(&self) -> broadcast_codec::ProviderInfo {
        self.inner.info()
    }

    fn descriptors(&self) -> Vec<broadcast_codec::CodecDescriptor> {
        self.inner.descriptors()
    }

    fn probe(
        &self,
        implementation: &str,
        direction: Direction,
        codec: CodecId,
    ) -> broadcast_codec::Probe {
        self.inner.probe(implementation, direction, codec)
    }

    fn make_decoder(
        &self,
        implementation: &str,
        parameters: &CodecParameters,
    ) -> Result<Box<dyn Decoder>, broadcast_codec::CodecError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.make_decoder(implementation, parameters)
    }

    fn make_encoder(
        &self,
        implementation: &str,
        config: &EncoderConfig,
    ) -> Result<Box<dyn Encoder>, broadcast_codec::CodecError> {
        self.inner.make_encoder(implementation, config)
    }
}
