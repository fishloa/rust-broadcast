//! Payload plug-ins: decode a data track's samples through `transmux`'s
//! [`PayloadRegistry`] while inspecting a file.
//!
//! A [`transmux::ir::CodecConfig::Data`] track carries bytes this workspace
//! never interprets on its own (camera telemetry, KLV metadata). A plug-in is
//! the decoder that does, and each one is its own cargo feature on this crate:
//! selecting `--plugin <name>` on `media-doctor check` builds a registry from
//! the compiled-in decoders and prints each claimed sample's values.
//!
//! TS and MP4 inputs are driven through their streaming demux stages, so a
//! data track's samples are decoded one at a time rather than by holding the
//! whole file. Other containers are not demuxed by `media-doctor` today, so
//! they yield no payload output.

use std::collections::BTreeMap;
use std::fs::File;
use std::io;

use broadcast_common::{Stage, Timestamp};
use transmux::StreamingTsDemux;
use transmux::ir::{DemuxEvent, TrackId, TrackSpec};
use transmux::mp4_demux::{Mp4Demux, Mp4DemuxConfig, Mp4Input};
pub use transmux::payload::PayloadRegistry;
use transmux::payload::PayloadValue;

use crate::mp4_stream::read_range;

/// The payload plug-ins compiled into this build.
pub const PLUGINS: &[&str] = &[
    #[cfg(feature = "motion-imagery-metadata")]
    transmux::payload::St0601Decoder::NAME,
];

/// Build a registry holding the plug-ins named in `names`.
///
/// # Errors
///
/// A message naming the unknown plug-in and listing the compiled-in ones, for
/// a name no feature provides.
pub fn registry(names: &[String]) -> Result<PayloadRegistry, String> {
    let mut registry = PayloadRegistry::new();
    for name in names {
        let decoder = decoder_for(name).ok_or_else(|| unknown_plugin(name))?;
        registry.register(decoder);
    }
    Ok(registry)
}

/// The decoder a compiled-in plug-in name selects.
fn decoder_for(name: &str) -> Option<Box<dyn transmux::payload::PayloadDecoder>> {
    match name {
        #[cfg(feature = "motion-imagery-metadata")]
        transmux::payload::St0601Decoder::NAME => Some(Box::new(transmux::payload::St0601Decoder)),
        _ => None,
    }
}

/// The message an unknown `--plugin` value reports.
fn unknown_plugin(name: &str) -> String {
    if PLUGINS.is_empty() {
        format!(
            "unknown payload plug-in `{name}`: no plug-ins are compiled into this build \
             (enable a plug-in feature, e.g. `motion-imagery-metadata`)"
        )
    } else {
        format!(
            "unknown payload plug-in `{name}`; compiled-in plug-ins: {}",
            PLUGINS.join(", ")
        )
    }
}

/// What one decoded sample reports.
pub struct DecodedSample<'a> {
    /// The owning track's id.
    pub track: u32,
    /// The sample's 0-based index within that track.
    pub sample: u64,
    /// The claiming plug-in's name.
    pub plugin: &'static str,
    /// The decoded values.
    pub values: &'a [PayloadValue],
}

/// Collects track specs and routes each claimed sample's values to `emit`.
struct Reporter<'a> {
    registry: &'a PayloadRegistry,
    specs: BTreeMap<TrackId, TrackSpec>,
    sample_index: BTreeMap<TrackId, u64>,
    emit: &'a mut dyn FnMut(DecodedSample<'_>),
}

impl<'a> Reporter<'a> {
    fn new(registry: &'a PayloadRegistry, emit: &'a mut dyn FnMut(DecodedSample<'_>)) -> Self {
        Self {
            registry,
            specs: BTreeMap::new(),
            sample_index: BTreeMap::new(),
            emit,
        }
    }

    fn event(&mut self, event: &DemuxEvent) {
        match event {
            DemuxEvent::TrackAdded(spec) | DemuxEvent::TrackUpdated(spec) => {
                self.specs.insert(spec.track_id, spec.clone());
            }
            DemuxEvent::TrackRemoved { track_id, .. } => {
                self.specs.remove(track_id);
            }
            DemuxEvent::Sample {
                track_id, sample, ..
            } => {
                let Some(spec) = self.specs.get(track_id) else {
                    return;
                };
                let index = {
                    let index = self.sample_index.entry(*track_id).or_insert(0);
                    let this = *index;
                    *index = index.saturating_add(1);
                    this
                };
                if let Some(Ok((plugin, values))) = self.registry.decode(spec, sample) {
                    (self.emit)(DecodedSample {
                        track: track_id.value(),
                        sample: index,
                        plugin,
                        values: &values,
                    });
                }
            }
            _ => {}
        }
    }
}

/// Decode every claimed sample of an in-memory TS image.
pub fn decode_ts(ts: &[u8], registry: &PayloadRegistry, emit: &mut dyn FnMut(DecodedSample<'_>)) {
    let mut demux = StreamingTsDemux::new();
    let mut reporter = Reporter::new(registry, emit);
    if demux.feed(ts).is_err() {
        return;
    }
    let _ = demux.finish();
    while let Some(event) = demux.poll_event() {
        reporter.event(&event);
    }
}

/// Decode every claimed sample of a seekable MP4 file, reading only the byte
/// ranges the demux stage asks for.
///
/// # Errors
///
/// The file read error, when a demanded range cannot be read.
pub fn decode_mp4(
    file: &mut File,
    len: u64,
    registry: &PayloadRegistry,
    emit: &mut dyn FnMut(DecodedSample<'_>),
) -> io::Result<()> {
    let config = Mp4DemuxConfig::default().with_total_len(Some(len));
    let mut demux = Mp4Demux::seekable(config);
    let mut reporter = Reporter::new(registry, emit);
    loop {
        let demand = demux.demand();
        let Some(offset) = demand.offset else {
            break;
        };
        let bytes = read_range(file, offset, demand.want_bytes)?;
        if demux
            .feed(
                Mp4Input::At {
                    offset,
                    bytes: bytes.into(),
                },
                Timestamp::ZERO,
            )
            .is_err()
        {
            break;
        }
        while let Some(event) = demux.poll() {
            reporter.event(&event);
        }
    }
    let _ = demux.finish();
    while let Some(event) = demux.poll() {
        reporter.event(&event);
    }
    Ok(())
}
