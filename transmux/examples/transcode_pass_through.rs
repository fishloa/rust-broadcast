//! Transcode one track through the codec seam and write the result back out.
//!
//! ```text
//! cargo run -p transmux --all-features --example transcode_pass_through -- out.cmaf
//! ```
//!
//! The fixture is `fixtures/ts/h264-aac.ts` (public, permissively licensed).
//! The codec pair below is deliberately trivial — it is *not* a codec, and it
//! does not produce a playable audio stream:
//!
//! - the "decoder" turns each AAC packet into one silent PCM frame,
//! - the "encoder" turns each frame back into an opaque packet and reports
//!   parameters of its own (48 kHz mono, a different time base),
//!
//! which is exactly enough to show what the stage does: the output track is
//! announced from the encoder's parameters, the init segment describes *that*
//! track, and the video track passes through untouched. A real adapter is a
//! separate crate outside this workspace (see the design's libav wall); this
//! crate only provides the place it plugs into.

use std::num::NonZeroU32;
use std::path::PathBuf;
use std::time::Duration;

use broadcast_codec::{
    AudioFrame, AudioParameters, AudioSamples, ChannelLayout, CodecError, CodecId, CodecParameters,
    Decoder, Encoder, Frame, Packet, SampleLayout,
};
use broadcast_common::stage::{Stage, Timestamp};
use transmux::mux_stage::{MuxInput, MuxOutput};
use transmux::{
    CmafMuxStage, CodecConfig, FragmentCut, Sample, TrackId, TrackSpec, TranscodeStage,
    codec_parameters_from_track,
};

/// The audio format the "encoder" claims.
const OUTPUT_RATE: u32 = 48_000;
/// Its media timescale, in ticks per second.
const OUTPUT_TIMESCALE: u32 = 1_000;
/// Samples in each synthetic PCM frame.
const FRAME_SAMPLES: usize = 1024;
/// `AudioSpecificConfig` for AAC LC, 48 kHz, mono (ISO/IEC 14496-3 §1.6.2.1).
const OUTPUT_ASC: [u8; 2] = [0x11, 0x88];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: transcode_pass_through <out.cmaf>")?;

    let ts = test_fixtures::read("ts/h264-aac.ts");
    let (specs, samples) = demux(&ts)?;
    let audio = specs
        .iter()
        .find(|spec| matches!(spec.config, CodecConfig::Aac { .. }))
        .cloned()
        .ok_or("the fixture carries no AAC track")?;
    let video = specs
        .iter()
        .find(|spec| matches!(spec.config, CodecConfig::Avc { .. }))
        .cloned()
        .ok_or("the fixture carries no AVC track")?;

    let mut stage = TranscodeStage::new(
        audio.track_id,
        Box::new(SilentDecoder::new(codec_parameters_from_track(&audio)?)),
        Box::new(OpaqueEncoder::new()),
    );

    let mut announced = Vec::new();
    let mut rest = Vec::new();
    for spec in &specs {
        stage.feed(MuxInput::Track(Box::new(spec.clone())), Timestamp::ZERO)?;
        collect(&mut stage, &mut announced, &mut rest);
    }
    for (track, sample) in samples {
        stage.feed(MuxInput::Sample { track, sample }, Timestamp::ZERO)?;
        collect(&mut stage, &mut announced, &mut rest);
    }
    stage.finish()?;
    collect(&mut stage, &mut announced, &mut rest);

    let mut mux = CmafMuxStage::new(
        broadcast_codec::Timescale::try_new(OUTPUT_TIMESCALE)?,
        FragmentCut::AnchorKeyframe {
            track: video.track_id,
            target: Duration::from_secs(1),
        },
        1,
    );
    let mut bytes = Vec::new();
    for track in announced {
        mux.feed(track, Timestamp::ZERO)?;
    }
    for input in rest {
        mux.feed(input, Timestamp::ZERO)?;
        drain(&mut mux, &mut bytes);
    }
    mux.finish()?;
    drain(&mut mux, &mut bytes);

    std::fs::write(&out, &bytes)?;
    println!(
        "wrote {} bytes to {} ({} tracks)",
        bytes.len(),
        out.display(),
        specs.len()
    );
    Ok(())
}

/// A track's announced description and its coded samples, in demux order.
type Demuxed = (Vec<TrackSpec>, Vec<(TrackId, Sample)>);

/// Demux the fixture into its announced tracks and their samples.
fn demux(ts: &[u8]) -> Result<Demuxed, transmux::Error> {
    let mut demux = transmux::StreamingTsDemux::new();
    let mut specs = Vec::new();
    let mut samples = Vec::new();
    demux.feed(ts)?;
    demux.finish()?;
    while let Some(event) = demux.poll_event() {
        match event {
            transmux::DemuxEvent::TrackAdded(spec) | transmux::DemuxEvent::TrackUpdated(spec) => {
                specs.push(spec)
            }
            transmux::DemuxEvent::Sample {
                track_id, sample, ..
            } => samples.push((track_id, sample)),
            _ => {}
        }
    }
    Ok((specs, samples))
}

/// Take everything the stage has ready: announcements apart from the rest,
/// because a muxer builds its init segment from the whole announced set.
fn collect(stage: &mut TranscodeStage, announced: &mut Vec<MuxInput>, rest: &mut Vec<MuxInput>) {
    while let Some(input) = stage.poll() {
        match input {
            MuxInput::Track(_) => announced.push(input),
            other => rest.push(other),
        }
    }
}

fn drain(mux: &mut CmafMuxStage, bytes: &mut Vec<u8>) {
    while let Some(output) = mux.poll() {
        match output {
            MuxOutput::Init(data) | MuxOutput::Bytes(data) => bytes.extend_from_slice(&data),
            MuxOutput::Segment(segment) => bytes.extend_from_slice(&segment.bytes),
            _ => {}
        }
    }
}

/// Turns one packet into one silent PCM frame. Not a decoder.
struct SilentDecoder {
    parameters: CodecParameters,
    pending: bool,
}

impl SilentDecoder {
    fn new(parameters: CodecParameters) -> Self {
        Self {
            parameters,
            pending: false,
        }
    }
}

impl Decoder for SilentDecoder {
    fn parameters(&self) -> &CodecParameters {
        &self.parameters
    }

    fn send(&mut self, _packet: Packet<'_>) -> Result<(), CodecError> {
        self.pending = true;
        Ok(())
    }

    fn receive(&mut self) -> Result<Option<Frame>, CodecError> {
        if !self.pending {
            return Ok(None);
        }
        self.pending = false;
        let rate = NonZeroU32::new(OUTPUT_RATE).ok_or_else(|| CodecError::failed("rate"))?;
        let frame = AudioFrame::new(
            rate,
            ChannelLayout::Mono,
            SampleLayout::Interleaved,
            AudioSamples::S16(vec![0; FRAME_SAMPLES]),
        )
        .ok_or_else(|| CodecError::failed("frame"))?;
        Ok(Some(Frame::Audio(frame)))
    }

    fn finish(&mut self) -> Result<(), CodecError> {
        Ok(())
    }

    fn reset(&mut self) {}
}

/// Turns each frame into an opaque packet and reports its own parameters. Not
/// an encoder.
struct OpaqueEncoder {
    parameters: Option<CodecParameters>,
    clock: i64,
    ready: bool,
}

impl OpaqueEncoder {
    fn new() -> Self {
        Self {
            parameters: None,
            clock: 0,
            ready: false,
        }
    }
}

impl Encoder for OpaqueEncoder {
    fn send(&mut self, frame: Frame) -> Result<(), CodecError> {
        let Frame::Audio(frame) = frame else {
            return Err(CodecError::unsupported("audio only"));
        };
        self.ready = true;
        self.clock = self.clock.saturating_add(
            i64::try_from(frame.samples().len()).unwrap_or(0) * i64::from(OUTPUT_TIMESCALE)
                / i64::from(OUTPUT_RATE),
        );
        self.parameters = Some(
            CodecParameters::new(
                CodecId::Aac,
                broadcast_codec::Timescale::try_new(OUTPUT_TIMESCALE)
                    .map_err(|_| CodecError::failed("timescale"))?,
            )
            .with_extradata(OUTPUT_ASC.to_vec())
            .with_audio(AudioParameters::new(
                NonZeroU32::new(OUTPUT_RATE).ok_or_else(|| CodecError::failed("rate"))?,
                ChannelLayout::Mono,
            )),
        );
        Ok(())
    }

    fn receive(&mut self) -> Result<Option<Packet<'static>>, CodecError> {
        if !self.ready {
            return Ok(None);
        }
        self.ready = false;
        Ok(Some(
            Packet::owned(vec![0u8; 4])
                .with_dts(self.clock)
                .with_pts(self.clock)
                .with_key(true),
        ))
    }

    fn finish(&mut self) -> Result<(), CodecError> {
        Ok(())
    }

    fn parameters(&self) -> Option<&CodecParameters> {
        self.parameters.as_ref()
    }
}
