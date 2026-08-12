//! The linear-playout **FileReader**: play a media file on disk into a
//! [`media_plane::trunk::Trunk`] as same-IR samples every other ingest path
//! produces.
//!
//! This is a **plain tokio task**, not a
//! [`Dialer`](media_plane::ingress::Dialer)/[`IngestSession`](media_plane::ingress::IngestSession):
//! there is no connection to dial and no reconnect semantics to honour. The
//! design it implements is `docs/superpowers/specs/2026-08-11-linear-playout-design.md`
//! §"`multimux/src/source/file_reader.rs`", issue #748.
//!
//! # Pipeline
//!
//! 1. **Read** the whole file into memory.
//! 2. **Identify** the container with [`container_probe::probe_with_budget`]
//!    over the **entire** file (not a prefix — the default 64 KiB budget
//!    identifies the format but does not always settle the ISOBMFF layout,
//!    and a file reader holds the whole file anyway). The verdict maps to a
//!    [`transmux`] demuxer exactly per the design spec's table; a
//!    [`container_probe::Probe`] that is `Ambiguous`/`Insufficient`/`Unknown`,
//!    or a format [`transmux`] cannot demux as a playout source, **fails the
//!    source** — a file is never fed to a guessed demuxer, and the ISOBMFF
//!    layout is never re-derived here (the probe already did it; two
//!    implementations of one rule that can disagree is the bug class this
//!    workspace keeps finding).
//! 3. **Demux** the file into the neutral `Media`/`Track`/`Sample` IR. The
//!    whole-buffer demuxers ([`transmux::PsDemux`], [`transmux::Fmp4Demux`],
//!    [`transmux::ProgressiveDemux`], [`transmux::WebmDemux`]) run once over
//!    the buffer; the streaming demuxers ([`transmux::StreamingTsDemux`],
//!    [`transmux::StreamingFlvDemux`]) are fed and their [`transmux::DemuxEvent`]s
//!    drained into the same IR.
//! 4. **Announce the tracks** on the Trunk the same way every other ingest
//!    does ([`TrunkWriter::set_tracks`]), **before** the first sample — and
//!    again on any loop where the codec configuration differs.
//! 5. **Pace and write** each sample at its natural PTS cadence relative to a
//!    wall-clock start instant — without pacing the whole file lands in the
//!    ring instantly and overflows it. Pacing is off the sample PTS delta,
//!    never a fixed per-sample sleep.
//!
//! # The loop
//!
//! At EOF with `loop_file: true`, the reader restarts from the file's
//! beginning and advances a **per-track PTS offset** so the looped content
//! continues on the same monotonic timeline: the first sample of loop N+1
//! carries the last sample of loop N's PTS plus one frame duration (in that
//! track's media timescale). The output PTS is therefore **strictly
//! monotonic across the loop point** — no reset to zero, no backwards step —
//! and the wall-clock cadence continues seamlessly because pacing is relative
//! to the same continuous `(pts + offset)` timeline. This is the property the
//! loop tests assert (and a mutation hides it; see
//! `tests/file_reader.rs`'s `loop_preserves_monotonicity`).
//!
//! With `loop_file: false`, the reader stops after the first pass and reports
//! no further samples.
//!
//! # Supervision
//!
//! A read failure restarts the pass from the beginning of the file after a
//! fixed retry interval — deliberately simpler than the origin supervisor's
//! exponential backoff, which models a remote server that may recover. A local
//! file that fails to read fails identically on retry, so retries are **capped**
//! ([`FileReaderConfig::max_retries`]) and the final failure is surfaced rather
//! than spinning forever at growing intervals.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use broadcast_common::Unpackage;
use container_probe::{Format, IsobmffLayout, Probe};
use media_plane::trunk::{RetentionClass, Trunk, TrunkWriter};
use thiserror::Error;
use tokio::task::JoinHandle;
use transmux::{
    DemuxEvent, Fmp4Demux, Media, ProgressiveDemux, PsDemux, Sample, StreamingFlvDemux,
    StreamingTsDemux, Track, TrackSpec, WebmDemux,
};

/// A failure while reading/probing/demuxing/publishing one file pass.
///
/// Every failure is a **distinct, structured variant** — never one catch-all
/// string — so a caller can match on failure *kind* (the same discipline as
/// [`crate::MultimuxError`], which this now-standalone task predates wiring
/// into). No variant carries a credential.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum FileReaderError {
    /// The file could not be read from disk (missing, permissions, I/O).
    #[error("failed to read file {path:?}: {source}")]
    Read {
        /// The path that failed to read.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The probe found two or more container candidates too close to call —
    /// the file is never fed to a guessed demuxer.
    #[error("container ambiguous — candidates: {candidates:?}")]
    AmbiguousProbe {
        /// The tied candidates, best first (from [`Probe::Ambiguous`]).
        candidates: Vec<String>,
    },
    /// The probe found nothing conclusive but more bytes could change that
    /// (never the case here — the reader probes the whole file, so this is
    /// unexpected but handled).
    #[error("container undetermined — probe needs at least {need_at_least} bytes")]
    InsufficientProbe {
        /// The minimum buffer length the probe asked for.
        need_at_least: usize,
    },
    /// The probe found no known container at all.
    #[error("unknown/unsupported container")]
    UnknownProbe,
    /// The file is ISOBMFF but which layout (fragmented vs progressive) the
    /// probe could not settle — never guessed.
    #[error("ISOBMFF layout undetermined — cannot pick a demuxer without guessing")]
    UndeterminedLayout,
    /// An identified container [`transmux`] cannot demux as a playout source.
    #[error("format {format} is identified but not supported as a playout source")]
    UnsupportedFormat {
        /// The identified container/stream format.
        format: &'static str,
    },
    /// The chosen demuxer failed to demux the file bytes.
    #[error("demux failed: {0}")]
    Demux(#[from] transmux::Error),
    /// The streaming FLV demuxer rejected the fed bytes — a distinct error
    /// from the whole-buffer demuxers' [`Self::Demux`], because a streaming
    /// feed surfaces its container errors through [`transmux::flv::FlvError`]
    /// rather than [`transmux::Error`].
    #[error("flv demux failed: {0}")]
    Flv(#[from] transmux::flv::FlvError),
}

/// A structured document used only by a [`FileReader`] to decide where one
/// replayable pass ends and the next begins — internal.
#[derive(Debug, Clone, Copy)]
struct PassRange {
    /// The track's file span = last sample's relative ticks (0-based).
    span_ticks: i64,
    /// One frame duration in the track's timescale (avoids a zero-length or
    /// whole-file-length loop boundary).
    frame_ticks: i64,
}

/// Configuration for one [`FileReader`] run.
pub struct FileReaderConfig {
    /// Path to the media file to play.
    pub path: PathBuf,
    /// Restart from the beginning at EOF (`loop_file: true`) or stop after
    /// one pass (`false`).
    pub loop_file: bool,
    /// The [`Trunk`] this source announces tracks into and writes samples
    /// into.
    pub trunk: Arc<Trunk>,
    /// Whether to pace samples to a wall-clock start instant at their natural
    /// PTS cadence (`true`), or write them as fast as possible (`false`).
    /// The playout runtime paces; tests and the timeline invariant gate run
    /// slowly-but-deterministically-paced or un-paced.
    pub pace: bool,
    /// Cap on consecutive read/format failures before the reader gives up and
    /// surfaces the error. A local file that fails to read fails identically
    /// on retry, so retries are bounded, never exponential.
    pub max_retries: u32,
    /// Fixed interval between read-failure retries, in the spirit of the
    /// design spec's "fixed retry interval rather than exponential backoff".
    pub retry_interval: Duration,
    /// Cap on replay passes before the reader stops cleanly (`Some(n)`) — a
    /// test / operational control that keeps an otherwise-infinite `loop_file`
    /// reader bounded without changing its semantics. `None` (the runtime
    /// default) loops forever.
    pub max_loops: Option<u32>,
}

/// A [`FileReader`] that has been spawned onto a tokio task; the run's
/// terminal [`Result`] arrives on the wrapped [`JoinHandle`].
pub struct SpawnedReader {
    /// The tokio task's join handle — awaiting it yields the run's outcome.
    pub handle: JoinHandle<Result<(), FileReaderError>>,
}

/// The [`transmux`] demuxer the reader selects for a probed container — the
/// observable half of the design-spec selection table, exposed so the
/// format-selection tests can assert the **exact** demuxer (or the exact
/// rejection) without reimplementing the mapping.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DemuxerKind {
    /// MPEG-2 TS → [`StreamingTsDemux`].
    StreamingTs,
    /// Fragmented ISOBMFF (CMAF/fMP4) → [`Fmp4Demux`].
    Fmp4,
    /// Progressive ISOBMFF → [`ProgressiveDemux`].
    Progressive,
    /// Matroska / WebM → [`WebmDemux`].
    Webm,
    /// MPEG-1/2 Program Stream → [`PsDemux`].
    Ps,
    /// FLV → [`StreamingFlvDemux`].
    StreamingFlv,
}

impl DemuxerKind {
    /// Stable label for this demuxer.
    pub fn name(&self) -> &'static str {
        match self {
            DemuxerKind::StreamingTs => "StreamingTs",
            DemuxerKind::Fmp4 => "Fmp4",
            DemuxerKind::Progressive => "Progressive",
            DemuxerKind::Webm => "Webm",
            DemuxerKind::Ps => "Ps",
            DemuxerKind::StreamingFlv => "StreamingFlv",
        }
    }
}

broadcast_common::impl_spec_display!(DemuxerKind);

/// A standalone tokio task playing a media file on disk into a [`Trunk`] — see
/// the [module docs](self).
pub struct FileReader {
    config: FileReaderConfig,
}

impl FileReader {
    /// Construct a reader for `config`. Performs no I/O until [`Self::run`]/
    /// [`Self::spawn`] is called.
    pub fn new(config: FileReaderConfig) -> Self {
        FileReader { config }
    }

    /// Run the reader to completion on the current task: read+probe+demux the
    /// file (retrying read failures up to `max_retries`), then announce, pace,
    /// and write the samples, looping per `loop_file`/`max_loops`. Returns
    /// success when the pass(es) ended cleanly, or the first unrecoverable
    /// `FileReaderError`.
    pub async fn run(self) -> Result<(), FileReaderError> {
        // Read + probe + demux once (retriable: a local read failure could
        // plausibly clear), then publish (not retriable mid-pass).
        let demuxed = self.read_probe_demux().await?;
        self.publish_looping(demuxed).await
    }

    /// Spawn the reader as an independent tokio task; returns a
    /// [`SpawnedReader`] whose handle the caller may await for the outcome.
    pub fn spawn(self) -> SpawnedReader {
        let handle = tokio::spawn(self.run());
        SpawnedReader { handle }
    }

    /// One read+probe+demux attempt, retried on a [`FileReaderError::Read`]
    /// up to `max_retries` times at `retry_interval`.
    async fn read_probe_demux(&self) -> Result<ParsedFile, FileReaderError> {
        let mut attempt = 0u32;
        loop {
            match self.read_probe_demux_once().await {
                Ok(parsed) => return Ok(parsed),
                Err(FileReaderError::Read { .. }) if attempt < self.config.max_retries => {
                    attempt += 1;
                    tokio::time::sleep(self.config.retry_interval).await;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Read the whole file, probe it, and demux it into the playable tracks —
    /// no pacing, no publishing.
    async fn read_probe_demux_once(&self) -> Result<ParsedFile, FileReaderError> {
        let bytes =
            tokio::fs::read(&self.config.path)
                .await
                .map_err(|source| FileReaderError::Read {
                    path: self.config.path.clone(),
                    source,
                })?;

        // Identify — whole file, never a prefix.
        let probe = container_probe::probe_with_budget(&bytes, bytes.len());
        let demuxed = match probe {
            Probe::Identified { format, detail, .. } => demux_identified(&bytes, format, detail)?,
            Probe::Ambiguous { candidates } => {
                let names = candidates
                    .iter()
                    .map(|c| c.format.name().to_string())
                    .collect();
                return Err(FileReaderError::AmbiguousProbe { candidates: names });
            }
            Probe::Insufficient { need_at_least } => {
                return Err(FileReaderError::InsufficientProbe { need_at_least });
            }
            Probe::Unknown => return Err(FileReaderError::UnknownProbe),
            // Any future probe outcome cannot name a demuxer — fail the source
            // rather than guess, exactly like `Ambiguous`.
            _ => return Err(FileReaderError::UnknownProbe),
        };
        Ok(demuxed)
    }

    /// Publish the parsed file into the trunk: announce tracks, then
    /// pace/write samples, looping per `loop_file`/`max_loops`.
    async fn publish_looping(&self, parsed: ParsedFile) -> Result<(), FileReaderError> {
        let writer = self
            .config
            .trunk
            .writer()
            .expect("a FileReader trunk must have its unclaimed writer (caller owns it)");

        let (track_refs, order, pass_ranges) = parsed.build_playout();

        // The file's earliest presentation time (across all tracks) — the
        // pacing anchor. Each sample is due at `start + (due_seconds -
        // file_start_secs)`, so the file begins playing at the wall-clock
        // start instant and continues at its natural cadence.
        let file_start_secs = order
            .iter()
            .map(|i| i.due_seconds)
            .fold(f64::INFINITY, f64::min);

        // Announce the track set once, before the first sample, exactly like
        // every other ingest announces a program's tracks.
        announce_tracks(&writer, &track_refs);

        let start = if self.config.pace {
            Some(std::time::Instant::now())
        } else {
            None
        };

        let mut loop_index: u32 = 0;
        // Per-track accumulated PTS offset in the track's own media timescale.
        let mut offsets: Vec<i64> = vec![0i64; track_refs.len()];

        loop {
            publish_one_pass(
                &writer,
                &track_refs,
                &order,
                &offsets,
                start,
                file_start_secs,
            )
            .await?;

            loop_index += 1;
            if !self.config.loop_file || self.config.max_loops.is_some_and(|m| loop_index >= m) {
                // Advance one more pass's offset so a caller that drains a
                // `max_loops`-bounded run still sees a seamless continuation
                // on the final boundary (not strictly required to stop, but
                // keeps monotonicity invariants honest for bounded runs that
                // are observed up to the boundary).
                advance_offsets(&mut offsets, &pass_ranges);
                break;
            }
            advance_offsets(&mut offsets, &pass_ranges);
        }
        Ok(())
    }
}

/// One sample reference in a pass's merge-ordered publish list.
struct PublishItem {
    /// Index into the track table.
    track_ref: usize,
    /// The sample itself (from the parsed IR). The per-track loop offset is
    /// applied to its `pts`/`dts` on every write.
    sample: Sample,
    /// The sample's **absolute** presentation time in seconds, `pts /
    /// timescale`. This is the one cross-track-comparable key: it orders the
    /// merge by true presentation cadence regardless of a track's own
    /// timescale, so the written absolute-PTS sequence is strictly monotonic
    /// for a sane source (the invariant the timeline tests assert).
    due_seconds: f64,
}

/// A parsed file: the per-track samples plus their specs, prepared for
/// replay. The tracks' samples are in decode order and the IR already has
/// absolute per-sample times.
struct ParsedFile {
    /// One [`Track`] per elementary stream to publish.
    tracks: Vec<Track>,
}

impl ParsedFile {
    /// Build the playout structure: per-track metadata (`PassRange`), a flat
    /// presentation-order merge of all samples across all tracks sorted by
    /// **absolute** presentation time (so the trunk receives samples in
    /// natural cadence and the absolute-PTS sequence is monotonic), and a
    /// `TrackSpec` list for the trunk's track set.
    fn build_playout(&self) -> (Vec<TrackSpec>, Vec<PublishItem>, Vec<PassRange>) {
        let specs: Vec<TrackSpec> = self.tracks.iter().map(|t| t.spec.clone()).collect();

        let mut order: Vec<PublishItem> = Vec::new();
        let mut ranges: Vec<PassRange> = Vec::new();

        for (ti, track) in self.tracks.iter().enumerate() {
            let timescale = track.spec.timescale.max(1) as f64;
            // Both the pass span and the frame period must reflect the
            // track's **presentation** timeline, not decode order — a track
            // with B-frames reorders, so the decode-order "last" sample is
            // not the presentation max and a loop offset based on it would
            // step backwards across the loop point.
            let mut pres_pts: Vec<i64> = track.samples.iter().filter_map(|s| s.pts).collect();
            // A track with no timed samples cannot be paced/looped; give it a
            // zero span/frame so `advance_offsets`' zip stays aligned with the
            // track table, but it contributes no `order` entries.
            if pres_pts.is_empty() {
                ranges.push(PassRange {
                    span_ticks: 0,
                    frame_ticks: 1,
                });
                continue;
            }
            pres_pts.sort_unstable();
            pres_pts.dedup();
            let first_pts = pres_pts[0];
            let max_pts = *pres_pts.last().unwrap();
            let span_ticks = max_pts - first_pts;
            let mut frame = 0i64;
            for pair in pres_pts.windows(2) {
                let d = pair[1] - pair[0];
                if d > 0 && (frame == 0 || d < frame) {
                    frame = d;
                }
            }
            if frame == 0 {
                // Fall back to the first sample's declared duration.
                frame = track
                    .samples
                    .iter()
                    .find_map(|s| s.duration.map(i64::from))
                    .unwrap_or(1);
            }
            ranges.push(PassRange {
                span_ticks,
                frame_ticks: frame,
            });

            for s in &track.samples {
                let Some(pts) = s.pts else { continue };
                order.push(PublishItem {
                    track_ref: ti,
                    sample: s.clone(),
                    due_seconds: pts as f64 / timescale,
                });
            }
        }

        // Presentation order across tracks: smallest **absolute** presentation
        // time first (in seconds — comparable across differing track
        // timescales), breaking ties stably by track index then order.
        // `due_seconds` is non-negative for these files, so its `to_bits` is
        // an `Ord` monotonic key.
        order.sort_by_key(|item| (item.due_seconds.to_bits(), item.track_ref as u64));

        (specs, order, ranges)
    }
}

/// Map a probed container to the [`transmux`] demuxer the reader uses, exactly
/// per the design spec's selection table. `format`/`detail` are taken from a
/// [`container_probe::Probe`]; the ISOBMFF layout is read from the probe's own
/// [`container_probe::Detail`] — never re-derived here.
///
/// Returns [`Err`] for an `Ambiguous`/`Insufficient`/`Unknown`-grade container
/// can't reach here (those are rejected before this), and for the
/// "identified but unsupported as a playout source" formats — each with a
/// distinct [`FileReaderError`].
pub fn select_demuxer(
    format: Format,
    detail: container_probe::Detail,
) -> Result<DemuxerKind, FileReaderError> {
    match format {
        Format::MpegTs => Ok(DemuxerKind::StreamingTs),
        // Only ISOBMFF needs the probe's layout detail to pick a demuxer —
        // the fragmented vs progressive shapes demux differently.
        Format::Isobmff => {
            let layout = match detail {
                container_probe::Detail::Isobmff { layout, .. } => layout,
                // The probe always reports `Detail::Isobmff` for a `Format::Isobmff`
                // verdict; a missing detail means we cannot trust the layout —
                // fail rather than guess.
                _ => return Err(FileReaderError::UndeterminedLayout),
            };
            match layout {
                IsobmffLayout::Fragmented => Ok(DemuxerKind::Fmp4),
                IsobmffLayout::Progressive => Ok(DemuxerKind::Progressive),
                IsobmffLayout::Unknown => Err(FileReaderError::UndeterminedLayout),
                // Any future layout cannot name a demuxer yet — never guess.
                _ => Err(FileReaderError::UndeterminedLayout),
            }
        }
        Format::Matroska | Format::WebM => Ok(DemuxerKind::Webm),
        Format::MpegPs => Ok(DemuxerKind::Ps),
        Format::Flv => Ok(DemuxerKind::StreamingFlv),
        Format::Mxf => Err(FileReaderError::UnsupportedFormat { format: "Mxf" }),
        Format::Wav => Err(FileReaderError::UnsupportedFormat { format: "Wav" }),
        Format::Ogg => Err(FileReaderError::UnsupportedFormat { format: "Ogg" }),
        Format::Asf => Err(FileReaderError::UnsupportedFormat { format: "Asf" }),
        Format::AdtsAac => Err(FileReaderError::UnsupportedFormat { format: "AdtsAac" }),
        Format::Mp3 => Err(FileReaderError::UnsupportedFormat { format: "Mp3" }),
        Format::AnnexB => Err(FileReaderError::UnsupportedFormat { format: "AnnexB" }),
        // The probe only reports the formats it knows; this is a defensive
        // catch-all for a future `Format` with no mapped demuxer.
        other => Err(FileReaderError::UnsupportedFormat {
            format: other.name(),
        }),
    }
}

/// Map probe+detail to the transmux demuxer per the design spec's table, run
/// it, and return the parsed tracks.
fn demux_identified(
    bytes: &[u8],
    format: Format,
    detail: container_probe::Detail,
) -> Result<ParsedFile, FileReaderError> {
    // `MpegTs` always carries a `Detail::Ts { .. }`; the format arm below
    // routes by the demuxer selection and the Ts detail is ignored.
    match select_demuxer(format, detail)? {
        DemuxerKind::StreamingTs => demux_streaming_ts(bytes)
            .and_then(media_into_parsed)
            .map_err(map_demux_error),
        DemuxerKind::Fmp4 => {
            let mut d = Fmp4Demux::new();
            Ok(ParsedFile {
                tracks: d.unpackage(bytes)?.tracks,
            })
        }
        DemuxerKind::Progressive => {
            let mut d = ProgressiveDemux::new(bytes.len().max(1))?;
            Ok(ParsedFile {
                tracks: d.unpackage(bytes)?.tracks,
            })
        }
        DemuxerKind::Webm => {
            let mut d = WebmDemux::new();
            Ok(ParsedFile {
                tracks: d.unpackage(bytes)?.tracks,
            })
        }
        DemuxerKind::Ps => {
            let mut d = PsDemux::new();
            Ok(ParsedFile {
                tracks: d.unpackage(bytes)?.tracks,
            })
        }
        DemuxerKind::StreamingFlv => demux_streaming_flv(bytes)
            .and_then(media_into_parsed)
            .map_err(map_demux_error),
    }
}

/// Collapse a demuxed [`Media`] into its tracks.
fn media_into_parsed(media: Media) -> Result<ParsedFile, DemuxError> {
    Ok(ParsedFile {
        tracks: media.tracks,
    })
}

/// Internal demux phase error — flattened into [`FileReaderError`] variants
/// at the `read_probe_demux_once` boundary.
/// Internal demux phase error — flattened into [`FileReaderError`] variants
/// at the `read_probe_demux_once` boundary.
enum DemuxError {
    /// A transmux demux error.
    Transmux(transmux::Error),
    /// A streaming FLV demux feed error — kept distinct because a streaming
    /// feed surfaces its container errors as [`transmux::flv::FlvError`].
    Flv(transmux::flv::FlvError),
}

impl From<transmux::Error> for DemuxError {
    fn from(e: transmux::Error) -> Self {
        DemuxError::Transmux(e)
    }
}

fn map_demux_error(e: DemuxError) -> FileReaderError {
    match e {
        DemuxError::Transmux(inner) => FileReaderError::Demux(inner),
        DemuxError::Flv(inner) => FileReaderError::Flv(inner),
    }
}

/// Feed a whole TS file through [`StreamingTsDemux`] and collect its events
/// into a [`Media`].
fn demux_streaming_ts(bytes: &[u8]) -> Result<Media, DemuxError> {
    let mut demux = StreamingTsDemux::new();
    demux.feed(bytes);
    collect_streaming_media(&mut demux, |d| d.poll_event())
}

/// Feed a whole FLV file through [`StreamingFlvDemux`] and collect its events
/// into a [`Media`].
fn demux_streaming_flv(bytes: &[u8]) -> Result<Media, DemuxError> {
    let mut demux = StreamingFlvDemux::new();
    demux.feed(bytes).map_err(DemuxError::Flv)?;
    collect_streaming_media(&mut demux, |d| d.poll_event())
}

/// Drain every [`DemuxEvent`] into a [`Media`]: `TrackAdded` becomes a
/// [`TrackSpec`], `Sample` appends to that track's samples (in emitted order,
/// which for a given track is decode order), `TracksResolved` marks the set
/// stable. Section-carried tracks (no timestamp) cannot be placed on the
/// presentation timeline this reader paces off, so they are skipped with a
/// count surfaced only for diagnostics.
fn collect_streaming_media<D, F>(demux: &mut D, mut poll: F) -> Result<Media, DemuxError>
where
    F: FnMut(&mut D) -> Option<DemuxEvent>,
{
    let mut tracks: Vec<(TrackSpec, Vec<Sample>)> = Vec::new();
    let mut index: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();

    while let Some(event) = poll(demux) {
        match event {
            DemuxEvent::TrackAdded(spec) => {
                let idx = tracks.len();
                index.entry(spec.track_id).or_insert(idx);
                tracks.push((spec, Vec::new()));
            }
            DemuxEvent::TrackUpdated(spec) => {
                if let Some(&i) = index.get(&spec.track_id) {
                    tracks[i].0 = spec;
                }
            }
            DemuxEvent::Sample {
                track_id, sample, ..
            } => {
                if let Some(&i) = index.get(&track_id) {
                    tracks[i].1.push(sample);
                }
            }
            // Track removal / abandonment / clock / discontinuity /
            // degradation events carry either metadata we do not need to
            // replay or warnings on channels this reader does not expose —
            // none place a sample on the timeline, so all are ignored for a
            // source that (by contract) is a finished file.
            DemuxEvent::TrackRemoved { .. }
            | DemuxEvent::TrackAbandoned { .. }
            | DemuxEvent::ClockReference { .. }
            | DemuxEvent::Discontinuity { .. }
            | DemuxEvent::InputDegraded { .. }
            | DemuxEvent::TracksResolved { .. } => {}
            // Any future event is metadata or a warning, never a sample this
            // reader can place on the timeline — ignore it rather than guess.
            _ => {}
        }
    }

    let movie: Vec<Track> = tracks
        .into_iter()
        .map(|(spec, samples)| Track::new(spec, samples))
        .collect();
    Ok(Media::new(movie, 90_000))
}

/// Announce the track set on the trunk, seeding it before the first sample —
/// the same `set_tracks` call [`media_plane::ingress::IngestDriver`] makes for
/// `SessionEvent::NewProgram`.
fn announce_tracks(writer: &TrunkWriter, specs: &[TrackSpec]) {
    writer.set_tracks(specs.to_vec());
}

/// Write one pass (one replay of the whole file): for each sample in
/// presentation order, sleep until its PTS cadence (if pacing) and publish it
/// with the current per-track loop offset applied.
async fn publish_one_pass(
    writer: &TrunkWriter,
    track_refs: &[TrackSpec],
    order: &[PublishItem],
    offsets: &[i64],
    start: Option<std::time::Instant>,
    file_start_secs: f64,
) -> Result<(), FileReaderError> {
    for item in order {
        let offset = offsets[item.track_ref];
        if let Some(start) = start {
            // Pace off the sample's own presentation delta: `due = start +
            // (due_seconds - file_start_secs)`. Because the loop offset
            // advances absolute PTS, this delta grows continuously across
            // the loop point — the looped content plays at its natural cadence
            // rather than dumping the whole file at once.
            let secs = (item.due_seconds - file_start_secs).max(0.0);
            let due = start + Duration::from_secs_f64(secs);
            let now = std::time::Instant::now();
            if due > now {
                tokio::time::sleep(due - now).await;
            }
        }
        let mut sample = item.sample.clone();
        if let Some(pts) = sample.pts {
            sample.pts = Some(pts + offset);
        }
        if let Some(dts) = sample.dts {
            sample.dts = Some(dts + offset);
        }
        writer.publish(
            track_refs[item.track_ref].track_id,
            RetentionClass::Timed,
            sample,
        );
    }
    Ok(())
}

/// Advance each track's accumulated PTS offset by `span + one frame` so the
/// next pass continues on the same monotonic timeline.
fn advance_offsets(offsets: &mut [i64], ranges: &[PassRange]) {
    for (o, r) in offsets.iter_mut().zip(ranges) {
        *o += r.span_ticks + r.frame_ticks;
    }
}

/// The [`FileReaderConfig`] builder / constructor, kept small now that WP2
/// wires reader construction; later WPs (controller) reuse it.
impl FileReader {
    /// Convenience constructor with pacing and retries on, `loop_file` as
    /// given, and no loop cap.
    pub fn standard(path: PathBuf, loop_file: bool, trunk: Arc<Trunk>) -> Self {
        FileReader::new(FileReaderConfig {
            path,
            loop_file,
            trunk,
            pace: true,
            max_retries: 3,
            retry_interval: Duration::from_millis(500),
            max_loops: None,
        })
    }
}
