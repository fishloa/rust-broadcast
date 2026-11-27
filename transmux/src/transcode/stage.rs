//! [`TranscodeStage`]: one track through a decoder and an encoder.

use alloc::boxed::Box;
use alloc::collections::VecDeque;

use broadcast_codec::{CodecError, CodecParameters, Decoder, Encoder, Packet};
use broadcast_common::Demand;
use broadcast_common::stage::{Stage, Timestamp};

use crate::error::{Error, Result};
use crate::ir::{Sample, TrackId, TrackSpec};
use crate::mux_stage::MuxInput;
use crate::transcode::bridge::{codec_parameters_from_track, track_spec_from_codec_parameters};

/// The track's samples through a caller-supplied decoder and encoder.
///
/// Built for one track: every other track's inputs pass through untouched and
/// in order. See the [module docs](crate::transcode) for the contract.
///
/// `Debug` deliberately not implemented: a boxed codec need not be.
#[non_exhaustive]
pub struct TranscodeStage {
    track: TrackId,
    decoder: Box<dyn Decoder>,
    encoder: Box<dyn Encoder>,
    /// The source parameters the decoder was built for.
    source: CodecParameters,
    /// What downstream was last told this track is, once the encoder reported
    /// parameters.
    announced: Option<CodecParameters>,
    /// Output samples the encoder produced before the encoder could name its
    /// parameters.
    pending: VecDeque<Sample>,
    pending_bytes: u64,
    pending_limit: Option<u64>,
    /// Whether the source ever delivered a sample for this track.
    saw_samples: bool,
    removed: bool,
    ready: VecDeque<MuxInput>,
    finished: bool,
}

impl TranscodeStage {
    /// A stage that runs `track`'s samples through `decoder` and `encoder`.
    ///
    /// The caller builds both, typically through a
    /// `broadcast_codec::Registry`; the decoder's own
    /// [`parameters`](Decoder::parameters) are what the source track is
    /// checked against, so a source re-announcement that changes them is
    /// refused rather than decoded with the wrong configuration.
    #[must_use]
    pub fn new(track: TrackId, decoder: Box<dyn Decoder>, encoder: Box<dyn Encoder>) -> Self {
        Self {
            track,
            source: decoder.parameters().clone(),
            decoder,
            encoder,
            announced: None,
            pending: VecDeque::new(),
            pending_bytes: 0,
            pending_limit: None,
            saw_samples: false,
            removed: false,
            ready: VecDeque::new(),
            finished: false,
        }
    }

    /// Bound the output samples held before the encoder reports its
    /// parameters, in bytes. Past `max_bytes` [`Stage::feed`] fails with
    /// [`Error::TranscodePendingLimit`]. Unset means unbounded: no
    /// library-chosen number.
    #[must_use]
    pub fn pending_limit(mut self, max_bytes: u64) -> Self {
        self.pending_limit = Some(max_bytes);
        self
    }

    /// The track this stage transcodes.
    #[must_use]
    pub fn track(&self) -> TrackId {
        self.track
    }

    fn codec_error(&self, source: CodecError) -> Error {
        Error::Transcode {
            track: self.track,
            source,
        }
    }

    /// One sample as the seam's coded access unit, in the source's time base.
    fn packet<'a>(&self, sample: &'a Sample) -> Packet<'a> {
        let mut packet = Packet::borrowed(&sample.data).with_key(sample.flags.is_sync);
        if let Some(timing) = sample.timing {
            if let Some(dts) = timing.dts() {
                packet = packet.with_dts(dts);
            }
            if let Some(pts) = timing.pts() {
                packet = packet.with_pts(pts);
            }
        }
        packet
    }

    /// Announce the track when the encoder has reported parameters (and
    /// re-announce only when they change), then release the samples it was
    /// withholding.
    fn publish(&mut self) -> Result<()> {
        let Some(parameters) = self.encoder.parameters().cloned() else {
            return Ok(());
        };
        if self.announced.as_ref() != Some(&parameters) {
            let spec = track_spec_from_codec_parameters(self.track, &parameters)?;
            self.ready.push_back(MuxInput::Track(Box::new(spec)));
            self.announced = Some(parameters);
        }
        self.pending_bytes = 0;
        while let Some(sample) = self.pending.pop_front() {
            self.ready.push_back(MuxInput::Sample {
                track: self.track,
                sample,
            });
        }
        Ok(())
    }

    /// Take one coded access unit from the encoder.
    fn receive_encoded(&mut self) -> Result<()> {
        loop {
            let Some(packet) = self.encoder.receive().map_err(|e| self.codec_error(e))? else {
                return Ok(());
            };
            let sample = Sample::new(
                packet.data().to_vec(),
                packet.dts(),
                packet.pts(),
                None,
                packet.key(),
            )?;
            // `publish` announces before it releases anything, so a sample
            // never reaches downstream ahead of the track description it
            // belongs to.
            if self.encoder.parameters().is_some() {
                self.publish()?;
            }
            if self.announced.is_some() {
                self.ready.push_back(MuxInput::Sample {
                    track: self.track,
                    sample,
                });
            } else {
                self.pending_bytes = self.pending_bytes.saturating_add(sample.data.len() as u64);
                if let Some(limit) = self.pending_limit
                    && self.pending_bytes > limit
                {
                    return Err(Error::TranscodePendingLimit { limit });
                }
                self.pending.push_back(sample);
            }
        }
    }

    /// Drain every decoded frame the decoder will give: each goes to the
    /// encoder, whose output is taken as it appears.
    fn drain_decoded(&mut self) -> Result<()> {
        loop {
            let Some(frame) = self.decoder.receive().map_err(|e| self.codec_error(e))? else {
                return Ok(());
            };
            self.encoder.send(frame).map_err(|e| self.codec_error(e))?;
            self.receive_encoded()?;
        }
    }

    /// Decode one sample, then take everything it released.
    fn transcode(&mut self, sample: &Sample) -> Result<()> {
        self.saw_samples = true;
        let packet = self.packet(sample);
        self.decoder.send(packet).map_err(|e| self.codec_error(e))?;
        self.drain_decoded()
    }

    /// Flush the decoder then the encoder: nothing decoded is lost.
    fn flush(&mut self) -> Result<()> {
        if self.finished {
            return Ok(());
        }
        self.finished = true;
        self.decoder.finish().map_err(|e| self.codec_error(e))?;
        self.drain_decoded()?;
        self.encoder.finish().map_err(|e| self.codec_error(e))?;
        self.receive_encoded()
    }

    /// A source re-announcement of this stage's track.
    fn source_changed(&mut self, spec: &TrackSpec) -> Result<()> {
        let parameters = codec_parameters_from_track(spec)?;
        if parameters == self.source {
            return Ok(());
        }
        Err(Error::TranscodeSourceChanged { track: self.track })
    }
}

impl Stage for TranscodeStage {
    type In<'a> = MuxInput;
    type Out = MuxInput;
    type Error = Error;

    fn feed(&mut self, input: MuxInput, _now: Timestamp) -> Result<()> {
        match input {
            MuxInput::Track(spec) => {
                if spec.track_id == self.track {
                    return self.source_changed(&spec);
                }
                self.ready.push_back(MuxInput::Track(spec));
                Ok(())
            }
            MuxInput::Sample { track, sample } => {
                if track != self.track {
                    self.ready.push_back(MuxInput::Sample { track, sample });
                    return Ok(());
                }
                if self.removed {
                    return Err(Error::InvalidInput(
                        "transcode: a sample for a track the source removed",
                    ));
                }
                self.transcode(&sample)
            }
            MuxInput::TrackRemoved { track } => {
                if track == self.track {
                    self.flush()?;
                    self.removed = true;
                }
                self.ready.push_back(MuxInput::TrackRemoved { track });
                Ok(())
            }
            other => {
                self.ready.push_back(other);
                Ok(())
            }
        }
    }

    fn poll(&mut self) -> Option<MuxInput> {
        self.ready.pop_front()
    }

    fn finish(&mut self) -> Result<()> {
        self.flush()?;
        if self.announced.is_none() && self.saw_samples {
            return Err(Error::TranscodeNoParameters { track: self.track });
        }
        Ok(())
    }

    fn demand(&self) -> Demand {
        Demand::default()
    }
}
