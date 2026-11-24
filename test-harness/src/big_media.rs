//! Incremental writers that grow a TS or FLV file to a target size in bounded
//! memory, for tests of the file-playout path (issue #4057).
//!
//! A 256 MiB capture cannot be committed as a fixture, so these writers
//! generate one at test time: demux a small committed fixture once, then append
//! its samples pass after pass, each pass's timestamps advanced by that track's
//! own span, flushing every pass to disk. Live bytes stay at one pass plus the
//! muxer's own state, which the module's tests assert with `test_alloc`'s
//! `ProcessPeak` allocator (a dev-dependency, so not linkable from here).

use std::fmt;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::time::Duration;

use broadcast_common::{Package, Serialize, Unpackage};
use rtmp_runtime::flv::{FlvHeader, FlvTag, TagType};
use transmux::{
    CodecConfig, FlvDemux, FlvPayloadKind, Media, StreamingTsMux, TsDemux, flv_frame_payloads,
    flv_sequence_header_payloads, on_metadata_body,
};

/// Nanoseconds in one second: turns a timescale-tick span into a [`Duration`]
/// without a floating-point round trip that could drift off the written
/// timestamps.
const NANOS_PER_SECOND: u64 = 1_000_000_000;

/// What an incremental write produced.
#[derive(Debug, Clone)]
pub struct BigMedia {
    /// Exact number of bytes written to the file.
    pub bytes: u64,
    /// `(track id, samples written)` in source track order.
    pub samples_per_track: Vec<(u32, u64)>,
    /// Media timeline the written samples span.
    pub duration: Duration,
}

/// Write an MPEG-2 TS file of at least `target_bytes` by repeating
/// `source_fixture`'s samples through a single [`StreamingTsMux`].
pub fn write_big_ts(path: &Path, source_fixture: &Path, target_bytes: u64) -> BigMedia {
    let source = read_source(source_fixture);
    let media = expect(
        "demux TS source",
        TsDemux::new().unpackage(source.as_slice()),
    );
    let step = pass_step(&media);
    let mut mux = StreamingTsMux::new();
    let mut out = BufWriter::new(expect("create output", File::create(path)));
    let mut written = 0u64;
    let mut passes = 0u64;
    while written < target_bytes {
        let shifted = media_at_pass(&media, &step, passes);
        let chunk = expect("mux TS pass", mux.package(&shifted));
        expect("write TS pass", out.write_all(&chunk));
        written += expect("sum chunk length", u64::try_from(chunk.len()));
        passes += 1;
    }
    expect("flush TS output", out.flush());
    BigMedia {
        bytes: written,
        samples_per_track: counts(&media, passes),
        duration: step.duration_for(passes),
    }
}

/// Write an FLV file of at least `target_bytes` by repeating `source_fixture`'s
/// samples through the FLV tag builders, framing each tag as it is produced.
pub fn write_big_flv(path: &Path, source_fixture: &Path, target_bytes: u64) -> BigMedia {
    let source = read_source(source_fixture);
    let media = expect(
        "demux FLV source",
        FlvDemux::new().unpackage(source.as_slice()),
    );
    let step = pass_step(&media);
    let mut out = BufWriter::new(expect("create output", File::create(path)));
    let mut written = write_flv_preamble(&mut out, &media);
    let mut passes = 0u64;
    while written < target_bytes {
        let shifted = media_at_pass(&media, &step, passes);
        for payload in expect("build FLV frame payloads", flv_frame_payloads(&shifted)) {
            written += write_flv_tag(
                &mut out,
                payload_tag_type(payload.kind),
                payload.timestamp_ms,
                &payload.body,
            );
        }
        passes += 1;
    }
    expect("flush FLV output", out.flush());
    BigMedia {
        bytes: written,
        samples_per_track: counts(&media, passes),
        duration: step.duration_for(passes),
    }
}

/// FLV header, `onMetaData` and the AVC/AAC sequence headers, written once.
///
/// `onMetaData`'s `duration` is omitted: the file's final length is not known
/// until the target is reached, and FLV makes that field optional (see
/// [`on_metadata_body`]).
fn write_flv_preamble(out: &mut impl Write, media: &Media) -> u64 {
    let has_video = media
        .tracks
        .iter()
        .any(|t| matches!(t.spec.config, CodecConfig::Avc { .. }));
    let has_audio = media
        .tracks
        .iter()
        .any(|t| matches!(t.spec.config, CodecConfig::Aac { .. }));
    let header = expect(
        "serialize FLV header",
        FlvHeader::new(has_audio, has_video).try_to_bytes(),
    );
    expect("write FLV header", out.write_all(&header));
    let mut written = expect("sum FLV header length", u64::try_from(header.len()));
    let meta = expect("build onMetaData", on_metadata_body(media, None));
    written += write_flv_tag(out, TagType::Script, 0, &meta);
    for payload in expect(
        "build FLV sequence headers",
        flv_sequence_header_payloads(media),
    ) {
        written += write_flv_tag(
            out,
            payload_tag_type(payload.kind),
            payload.timestamp_ms,
            &payload.body,
        );
    }
    written
}

/// Frame one FLV tag body and write it.
fn write_flv_tag(out: &mut impl Write, tag_type: TagType, timestamp_ms: u32, body: &[u8]) -> u64 {
    let bytes = expect(
        "serialize FLV tag",
        FlvTag::new(tag_type, timestamp_ms, body).try_to_bytes(),
    );
    expect("write FLV tag", out.write_all(&bytes));
    expect("sum FLV tag length", u64::try_from(bytes.len()))
}

/// [`FlvPayloadKind`] is `#[non_exhaustive]` but its only two variants are the
/// tag types a media payload can carry, so anything else falls to audio rather
/// than inventing a third tag type.
fn payload_tag_type(kind: FlvPayloadKind) -> TagType {
    if kind == FlvPayloadKind::Video {
        TagType::Video
    } else {
        TagType::Audio
    }
}

fn read_source(path: &Path) -> Vec<u8> {
    expect("read source fixture", std::fs::read(path))
}

/// The one media-time step every pass advances by: the longest track's span,
/// kept as an exact ticks/timescale rational.
///
/// Per-track spans differ (audio runs a frame past video, a 90 kHz and a 44.1
/// kHz track round differently), so advancing each track by its *own* span
/// makes the tracks drift apart by that difference on every pass — seconds of
/// A/V skew across a few hundred passes. One file-wide step in seconds, applied
/// to every track, keeps the source's A/V offset.
#[derive(Debug, Clone, Copy)]
struct PassStep {
    ticks: u64,
    timescale: u32,
}

impl PassStep {
    /// `pass` steps expressed in `timescale` ticks: the step's exact rational
    /// multiplied by the pass count first, then divided once, so the only
    /// rounding is a single floor rather than an accumulated per-pass error.
    fn offset_for(&self, pass: u64, timescale: u32) -> i64 {
        let ticks = u128::from(self.ticks)
            .checked_mul(u128::from(pass))
            .and_then(|product| product.checked_mul(u128::from(timescale)))
            .map(|product| product / u128::from(self.timescale))
            .and_then(|ticks| i64::try_from(ticks).ok());
        expect(
            "pass offset fits i64",
            ticks.ok_or("pass offset overflows i64"),
        )
    }

    /// The media timeline `passes` passes span: `passes * step`, in the same
    /// exact-rational form.
    fn duration_for(&self, passes: u64) -> Duration {
        let nanos = u128::from(self.ticks)
            .checked_mul(u128::from(passes))
            .and_then(|product| product.checked_mul(u128::from(NANOS_PER_SECOND)))
            .map(|product| product / u128::from(self.timescale))
            .and_then(|nanos| u64::try_from(nanos).ok());
        Duration::from_nanos(expect(
            "pass duration fits u64 nanoseconds",
            nanos.ok_or("pass duration overflows u64 nanoseconds"),
        ))
    }
}

/// Each track's own span in ticks (`last end - first dts`).
fn track_spans(media: &Media) -> Vec<u64> {
    media
        .tracks
        .iter()
        .map(|track| {
            let mut first: Option<i64> = None;
            let mut last_end: Option<i64> = None;
            for sample in &track.samples {
                let Some(timing) = sample.timing.as_ref() else {
                    continue;
                };
                let Some(dts) = timing.dts() else {
                    continue;
                };
                first = Some(first.map_or(dts, |f| f.min(dts)));
                let end = dts.saturating_add(i64::from(timing.duration().unwrap_or(0)));
                last_end = Some(last_end.map_or(end, |e| e.max(end)));
            }
            match (first, last_end) {
                (Some(first), Some(end)) => u64::try_from(end.saturating_sub(first)).unwrap_or(0),
                _ => 0,
            }
        })
        .collect()
}

/// The longest track span as a [`PassStep`], comparing spans as seconds
/// cross-multiplied so tracks with different timescales compare exactly.
fn pass_step(media: &Media) -> PassStep {
    let spans = track_spans(media);
    let timescales: Vec<u32> = media
        .tracks
        .iter()
        .map(|track| track.spec.timescale.get())
        .collect();
    let mut longest: Option<(u64, u32)> = None;
    for (&span, &timescale) in spans.iter().zip(&timescales) {
        longest = match longest {
            Some((best_span, best_timescale))
                if u128::from(span) * u128::from(best_timescale)
                    <= u128::from(best_span) * u128::from(timescale) =>
            {
                Some((best_span, best_timescale))
            }
            _ => Some((span, timescale)),
        };
    }
    let (ticks, timescale) = expect(
        "source has a timed track",
        longest.ok_or("source has no tracks"),
    );
    PassStep { ticks, timescale }
}

/// `media` shifted so pass `pass` continues the timeline `step` starts.
fn media_at_pass(media: &Media, step: &PassStep, pass: u64) -> Media {
    let mut shifted = media.clone();
    for track in &mut shifted.tracks {
        let offset = step.offset_for(pass, track.spec.timescale.get());
        if offset == 0 {
            continue;
        }
        for sample in &mut track.samples {
            if let Some(timing) = sample.timing.as_mut() {
                timing.shift(offset);
            }
        }
    }
    shifted
}

fn counts(media: &Media, passes: u64) -> Vec<(u32, u64)> {
    media
        .tracks
        .iter()
        .map(|track| {
            let per_pass = expect("sum sample count", u64::try_from(track.samples.len()));
            (track.spec.track_id.value(), per_pass * passes)
        })
        .collect()
}

/// Unwrap one harness step, naming it in the failure message. The writers
/// return [`BigMedia`] rather than `Result` because a missing fixture, an
/// unwritable temp dir or a demux failure is a broken test, not a case a test
/// should branch on.
fn expect<T, E: fmt::Display>(what: &str, result: Result<T, E>) -> T {
    match result {
        Ok(value) => value,
        Err(error) => panic!("big_media: {what}: {error}"),
    }
}
