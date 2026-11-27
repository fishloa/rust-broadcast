//! The codec seam in the pipeline: one stage that decodes and re-encodes one
//! track's samples, and the two conversions between this crate's [`TrackSpec`](crate::TrackSpec)
//! and `broadcast-codec`'s `CodecParameters`.
//!
//! Behind the `codec` feature, off by default and implied by nothing: it adds a
//! dependency on `broadcast-codec` (the trait vocabulary and raw frame types),
//! never on a codec. The caller names the decoder and encoder, typically
//! through a `broadcast_codec::Registry` built from adapter crates outside this
//! workspace, and this module holds no codec knowledge of its own: it decodes
//! and encodes nothing. Its one bitstream read is
//! `bridge::in_band_record`, which derives the record an encoder declared it
//! writes in-band.
//!
//! [`TranscodeStage`] is a stage over the mux vocabulary
//! ([`MuxInput`](crate::mux_stage::MuxInput) in and out), so it sits between a
//! demuxer and any muxer, segmenter or transform. It is built for one track:
//! every other track passes through untouched and in order.
//!
//! A codec change is a property of the whole track. The stage holds its output
//! until the encoder reports the parameters that name the new codec, then
//! announces the track once — built from those parameters — before its first
//! sample, so a downstream muxer's init segment, PMT or file header describes
//! the output rather than the input. A source `TrackUpdated` is absorbed when
//! it restates what the decoder was built for; one that changes it drains the
//! decoder and hands it to `Decoder::reconfigure`, and when the decoder
//! refuses, a caller-supplied
//! [`factory`](TranscodeStage::factory) builds a fresh one. With neither, the
//! change is [`Error::TranscodeSourceChanged`](crate::Error::TranscodeSourceChanged).
//!
//! An encoder that writes its configuration record in-band
//! (`ConfigurationRecord::InBand`) is announced from the record derived from
//! its first access unit: AAC from the ADTS header, H.264 from the SPS and
//! PPS. That derivation is the one place this module reads a bitstream.
//!
//! It is bounded by construction: what it holds is what the decoder and encoder
//! hold plus the samples that arrive before the encoder reports its parameters,
//! and the caller may cap that window
//! ([`TranscodeStage::pending_limit`](TranscodeStage::pending_limit), unlimited by default — no library-chosen
//! number).

mod bridge;
mod stage;

pub use bridge::{codec_parameters_from_track, track_spec_from_codec_parameters};
pub use stage::TranscodeStage;
