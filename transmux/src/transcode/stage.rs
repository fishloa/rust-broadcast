//! [`TranscodeStage`]: one track through a decoder and an encoder.

use alloc::boxed::Box;
use alloc::collections::VecDeque;

use alloc::vec::Vec;

use broadcast_codec::{CodecError, CodecParameters, ConfigurationRecord, Decoder, Encoder, Packet};
use broadcast_common::Demand;
use broadcast_common::stage::{Stage, Timestamp};

use crate::error::{Error, Result};
use crate::ir::{Sample, SampleTrim, TrackId, TrackSpec};
use crate::mux_stage::MuxInput;
use crate::transcode::bridge::{
    codec_parameters_from_track, in_band_record, track_spec_from_codec_parameters,
};

/// Builds a decoder for new parameters, for a source change the current
/// decoder cannot adopt itself.
pub type DecoderFactory =
    Box<dyn FnMut(&CodecParameters) -> core::result::Result<Box<dyn Decoder>, CodecError>>;

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
    /// Rebuilds the decoder when a source change cannot be adopted in place.
    factory: Option<DecoderFactory>,
    /// The source parameters the decoder was built for.
    source: CodecParameters,
    /// The first coded access unit since the last announcement: where an
    /// encoder that writes its configuration record in-band put it.
    first_unit: Option<Vec<u8>>,
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
    /// checked against. A source re-announcement that changes them is handed
    /// to [`Decoder::reconfigure`]; without [a factory](Self::factory), a
    /// decoder that refuses it leaves the change
    /// [`Error::TranscodeSourceChanged`].
    #[must_use]
    pub fn new(track: TrackId, decoder: Box<dyn Decoder>, encoder: Box<dyn Encoder>) -> Self {
        Self {
            track,
            source: decoder.parameters().clone(),
            decoder,
            encoder,
            factory: None,
            first_unit: None,
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

    /// Build a fresh decoder through `factory` when a source change is one
    /// the current decoder cannot adopt in place.
    ///
    /// The factory receives the new source parameters and returns a decoder
    /// built for them; the stage keeps the encoder, the announced output
    /// track and the samples it holds. Without a factory, a decoder that
    /// refuses [`Decoder::reconfigure`] leaves the source change
    /// [`Error::TranscodeSourceChanged`].
    #[must_use]
    pub fn factory(mut self, factory: DecoderFactory) -> Self {
        self.factory = Some(factory);
        self
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
        let mut packet = Packet::borrowed(&sample.data)
            .with_key(sample.flags.is_sync)
            .with_trim_start(sample.trim.start())
            .with_trim_end(sample.trim.end());
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

    /// The parameters the track is announced with: the encoder's, with the
    /// in-band configuration record derived from the first access unit when
    /// the encoder says it wrote one there.
    fn announcement(&self, current: &CodecParameters) -> Result<Option<CodecParameters>> {
        if current.configuration_record() != ConfigurationRecord::InBand {
            return Ok(Some(current.clone()));
        }
        let Some(first) = self.first_unit.as_deref() else {
            return Ok(None);
        };
        let Some(record) = in_band_record(current.codec(), first)? else {
            if current.codec().requires_configuration_record() {
                return Err(Error::TranscodeUnrepresentable {
                    what: "the in-band configuration record of this codec",
                });
            }
            return Ok(Some(current.clone().with_no_configuration_record()));
        };
        Ok(Some(current.clone().with_extradata(record)))
    }

    /// Announce the track when the encoder has reported parameters (and
    /// re-announce only when they change), then release the samples it was
    /// withholding.
    fn publish(&mut self) -> Result<()> {
        let Some(current) = self.encoder.parameters().cloned() else {
            return Ok(());
        };
        if self.announced.as_ref() != Some(&current) {
            let Some(parameters) = self.announcement(&current)? else {
                return Ok(());
            };
            let spec = track_spec_from_codec_parameters(self.track, &parameters)?;
            self.ready.push_back(MuxInput::Track(Box::new(spec)));
            self.announced = Some(current);
            self.first_unit = None;
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
            if self.first_unit.is_none() {
                self.first_unit = Some(packet.data().to_vec());
            }
            let sample = Sample::new(
                packet.data().to_vec(),
                packet.dts(),
                packet.pts(),
                None,
                packet.key(),
            )?
            .with_trim(SampleTrim::new(packet.trim_start(), packet.trim_end()));
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

    /// A source re-announcement of this stage's track: drain, then let the
    /// decoder adopt the new parameters, or rebuild it through the factory.
    fn source_changed(&mut self, spec: &TrackSpec) -> Result<()> {
        let parameters = codec_parameters_from_track(spec)?;
        if parameters == self.source {
            return Ok(());
        }
        self.drain_decoded()?;
        match self.decoder.reconfigure(parameters.clone()) {
            Ok(()) => {
                self.source = parameters;
                Ok(())
            }
            Err(CodecError::CannotReconfigure { .. }) => {
                let Some(factory) = self.factory.as_mut() else {
                    return Err(Error::TranscodeSourceChanged { track: self.track });
                };
                match factory(&parameters) {
                    Ok(decoder) => {
                        self.decoder = decoder;
                        self.decoder.discontinuity();
                        self.source = parameters;
                        Ok(())
                    }
                    Err(source) => Err(Error::Transcode {
                        track: self.track,
                        source,
                    }),
                }
            }
            Err(source) => Err(Error::Transcode {
                track: self.track,
                source,
            }),
        }
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
            MuxInput::Discontinuity { track } => {
                if track.is_none_or(|track| track == self.track) {
                    self.decoder.discontinuity();
                }
                self.ready.push_back(MuxInput::Discontinuity { track });
                Ok(())
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
