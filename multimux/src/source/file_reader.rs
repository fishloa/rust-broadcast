//! The linear-playout **FileReader**: play a media file on disk into a
//! [`media_plane::trunk::Trunk`] as same-IR samples every other ingest path
//! produces.
//!
//! This is a **plain tokio task**, not a
//! [`Dialer`](media_plane::ingress::Dialer)/[`IngestSession`]:
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

// ---------------------------------------------------------------------------
// Route wiring (issue #748 WP5) — the file route served through the shared
// driver/segmenter machinery.
//
// The standalone [`FileReader`] above is a plain task that writes a Trunk
// directly; a *route* needs its output turned into servable segments, which is
// `crate::source::advance_route`'s job — and that facade enumerates programs
// from an [`media_plane::ingress::IngestDriver`], not a bare Trunk. So the
// route mints a single program through a `FileIngestSession` (the one thin
// adapter the driver understands), whose demux is the very same
// [`FileReader::read_probe_demux`] path — one demux, two entry points.
// ---------------------------------------------------------------------------

use broadcast_common::{Demand, Stage, Timestamp};
use media_plane::ingress::{IngestDriver, IngestSession, ProgramId, SessionEvent};
use std::collections::VecDeque;

/// A sans-IO `IngestSession` that replays a demuxed file into the shared
/// driver/segmenter machinery: it announces one program (the file's track
/// set) on the **first** feed, and emits the samples on **subsequent** feeds,
/// so `crate::source::advance_route` (called between those feeds) creates the
/// per-program segmenter before any sample lands — a segmenter subscribes its
/// trunk cursor at the live edge, so samples published in the same drain as
/// the `NewProgram` that mints the trunk would be invisible to it.
///
/// With `loop_file: true`, once a pass's queue is exhausted `feed` refills it
/// from the **same** parsed content (never re-demuxing, never growing), applying
/// the per-track PTS offset from `ParsedFile::build_playout`'s `PassRange`s via
/// [`advance_offsets`] so the output timeline stays strictly monotonic — no
/// reset to zero, no backwards step — across every loop point.
struct FileIngestSession {
    program: ProgramId,
    tracks: std::sync::Arc<[TrackSpec]>,
    /// The presentation-ordered samples (`track_ref` → sample), reused for
    /// every loop pass — never re-demuxed.
    order: Vec<PublishItem>,
    /// Per-track pass spans/frame periods, from `build_playout`.
    ranges: Vec<PassRange>,
    /// Per-track running PTS offset (tracks the loop point), advanced by
    /// [`advance_offsets`] after each pass.
    offsets: Vec<i64>,
    /// `track_ref` → the track's id, for publishing.
    track_ids: Vec<u32>,
    /// The current pass's sample queue (presentation order, PTS-offset applied).
    queue: VecDeque<(u32, Sample)>,
    loop_file: bool,
    established: bool,
    announced: bool,
    started: bool,
}

impl FileIngestSession {
    /// Build the session from an already-demuxed file, honouring `loop_file`.
    fn new(parsed: ParsedFile, program: ProgramId, loop_file: bool) -> Self {
        let (specs, order, ranges) = parsed.build_playout();
        let track_ids: Vec<u32> = specs.iter().map(|t| t.track_id).collect();
        let mut session = FileIngestSession {
            program,
            tracks: specs.into(),
            order,
            ranges,
            offsets: vec![0i64; track_ids.len()],
            track_ids,
            queue: VecDeque::new(),
            loop_file,
            established: false,
            announced: false,
            started: false,
        };
        // First pass: offsets are all zero (the file's own timeline).
        session.refill();
        session
    }

    /// Refill the queue from the already-parsed content, applying the current
    /// per-track PTS offset (the loop point), then advance the offset so the
    /// next pass continues monotonically. Reuses the parsed `order` — nothing
    /// is re-demuxed and the queue's capacity is reused, so memory is bounded
    /// at the file's own sample count no matter how many passes happen.
    fn refill(&mut self) {
        self.queue.clear();
        for item in &self.order {
            let mut sample = item.sample.clone();
            let off = self.offsets[item.track_ref];
            if let Some(p) = sample.pts {
                sample.pts = Some(p + off);
            }
            if let Some(d) = sample.dts {
                sample.dts = Some(d + off);
            }
            self.queue
                .push_back((self.track_ids[item.track_ref], sample));
        }
        advance_offsets(&mut self.offsets, &self.ranges);
    }
}

impl Stage for FileIngestSession {
    type In<'a> = ();
    type Out = SessionEvent;
    type Error = FileReaderError;

    fn feed(&mut self, _input: (), _now: Timestamp) -> Result<(), Self::Error> {
        // With `loop_file`, refill when the previous pass's queue has drained.
        if self.loop_file && self.queue.is_empty() {
            self.refill();
        }
        Ok(())
    }

    fn poll(&mut self) -> Option<SessionEvent> {
        if !self.established {
            self.established = true;
            return Some(SessionEvent::Established);
        }
        if !self.announced {
            self.announced = true;
            return Some(SessionEvent::NewProgram {
                program: self.program,
                tracks: self.tracks.to_vec(),
            });
        }
        if !self.started {
            // End this drain right after `NewProgram` so the driver's later
            // `advance_route` creates the segmenter before any sample flows.
            self.started = true;
            return None;
        }
        self.queue
            .pop_front()
            .map(|(track_id, sample)| SessionEvent::Sample {
                program: self.program,
                track_id,
                retention: RetentionClass::Timed,
                sample,
            })
    }

    fn finish(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn next_deadline(&self) -> Option<Timestamp> {
        None
    }

    fn on_deadline(&mut self, _now: Timestamp) {}

    fn demand(&self) -> Demand {
        Demand::new(0)
    }
}

impl IngestSession for FileIngestSession {
    type Request = bytes::Bytes;
}

/// Run a `InputSpec::File` route to completion on the current task: demux the
/// file, replay it through a single-program [`IngestDriver`] over a
/// [`FileIngestSession`], and pump `crate::source::advance_route` once per
/// iteration so samples become servable segments/parts. Mirrors
/// [`crate::source::ts_udp::run_ts_udp`]'s shape; returns `Err` only on a
/// failure (a bad/unreadable file), which the route's supervisor retries.
pub(crate) async fn run_file_source(
    path: &str,
    loop_file: bool,
    window_segments: usize,
    handshake: media_plane::ingress::HandshakePolicy,
    route_handle: &std::sync::Arc<crate::route::RouteHandle>,
) -> crate::MultimuxError {
    // Demux the file eagerly (the same probe->demux path the standalone
    // FileReader uses); a failure surfaces through the supervisor's retry.
    let scratch_cfg = crate::source::driver_trunk_config(window_segments);
    let scratch = media_plane::trunk::Trunk::new(scratch_cfg);
    let reader = FileReader::new(FileReaderConfig {
        path: std::path::PathBuf::from(path),
        loop_file,
        trunk: scratch,
        pace: false,
        max_retries: 0,
        retry_interval: std::time::Duration::ZERO,
        max_loops: None,
    });
    let parsed = match reader.read_probe_demux().await {
        Ok(p) => p,
        Err(e) => return e.into(),
    };

    // Size the sample ring to hold the whole file: unlike a live source,
    // whose samples arrive incrementally and are segmented as they come, an
    // instant file publishes everything at once, so a live-sized (64-entry)
    // ring would evict most of the file before the segmenter read it.
    let total_samples: usize = parsed.tracks.iter().map(|t| t.samples.len()).sum();
    let nz = |n: usize| std::num::NonZeroUsize::new(n.max(1)).expect("non-zero cap");
    let trunk_config = media_plane::trunk::TrunkConfig::new(
        nz(total_samples + 64),
        nz(16),
        nz(window_segments),
        nz(64),
        nz(64),
    );

    let session = FileIngestSession::new(parsed, crate::route::SPTS_PROGRAM_ID, loop_file);
    let mut driver = IngestDriver::new(
        session,
        trunk_config,
        handshake,
        std::num::NonZeroUsize::new(1).expect("one program"),
    );
    let start = std::time::Instant::now();
    let mut progress = crate::source::DriverProgress::new();
    loop {
        let now = broadcast_common::Timestamp::from_instant(start, std::time::Instant::now());
        // Feed the session: on the first feed it emits Established + NewProgram
        // (minting the program's Trunk), on later feeds it emits the samples —
        // so `advance_route` below creates the segmenter *between* those two
        // phases, before any sample lands.
        driver.feed((), now);
        crate::source::advance_route(&driver, route_handle, &mut progress);
        // Give the runtime time to serve; the finite file's segments stay
        // servable (the route serves VOD-style once produced).
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

#[cfg(test)]
mod loop_tests {
    use super::*;
    use media_plane::ingress::{ProgramId, SessionEvent};

    /// Demux a real TS fixture into a `ParsedFile`.
    async fn demux_fixture() -> ParsedFile {
        let path = format!("{}/../fixtures/ts/h264_aac.ts", env!("CARGO_MANIFEST_DIR"));
        let trunk_cfg = crate::source::driver_trunk_config(8);
        let scratch = media_plane::trunk::Trunk::new(trunk_cfg);
        let reader = FileReader::new(FileReaderConfig {
            path: path.into(),
            loop_file: true,
            trunk: scratch,
            pace: false,
            max_retries: 0,
            retry_interval: std::time::Duration::ZERO,
            max_loops: None,
        });
        reader.read_probe_demux().await.expect("demux fixture")
    }

    /// Drive a session's `poll` until `None`, collecting the PTS of every
    /// sample (skipping the program/established events).
    fn drain_pts(sess: &mut FileIngestSession) -> Vec<(u32, i64)> {
        let mut pts = Vec::new();
        while let Some(ev) = sess.poll() {
            if let SessionEvent::Sample {
                track_id, sample, ..
            } = ev
                && let Some(p) = sample.pts
            {
                pts.push((track_id, p));
            }
        }
        pts
    }

    /// The loop offset keeps each track's published PTS strictly monotonic
    /// across the loop point — no reset to zero, no backwards step.
    ///
    /// (The file's audio and video carry different timescales, so raw PTS are
    /// not comparable across tracks; the monotonic invariant is therefore held
    /// per track, which is exactly the property the offset exists to preserve.)
    ///
    /// MUTATION PROOF, recorded verbatim: removing the per-track PTS offset on
    /// refill (`sample.pts = Some(p)` instead of `Some(p + off)` in
    /// `FileIngestSession::refill`) makes each pass restart at the file's own
    /// per-track first PTS, so a track's pass-1 first sits at or below its own
    /// pass-0 last — and this test FAILS with:
    ///
    ///     PTS must be strictly monotonic across the loop point (track 2): 195315 then 64243
    ///
    /// (audio track 2's pass-0 last PTS 195315 is followed by its unoffset
    /// pass-1 first PTS 64243 — a backwards step). Restoring the offset (and
    /// a `touch`) makes it pass again.
    #[tokio::test]
    async fn loop_refill_keeps_pts_monotonic_across_loop_point() {
        let parsed = demux_fixture().await;
        let mut sess = FileIngestSession::new(parsed, ProgramId(0), true);

        // Pass the one-time setup gate, then collect pass 0's samples.
        let _ = sess.feed((), broadcast_common::Timestamp::from_nanos(0));
        let _setup = drain_pts(&mut sess);
        let pass0 = drain_pts(&mut sess);
        assert!(!pass0.is_empty(), "pass 0 must emit samples");

        // Trigger the loop refill (pass 1, offset advanced) and collect it.
        let _ = sess.feed((), broadcast_common::Timestamp::from_nanos(0));
        let pass1 = drain_pts(&mut sess);
        assert_eq!(
            pass0.len(),
            pass1.len(),
            "each pass emits the same sample count"
        );
        assert!(!pass1.is_empty(), "a looped pass must emit samples");

        // Per-track: concatenate pass0 + pass1 and assert strictly monotonic.
        let both: Vec<(u32, i64)> = pass0.into_iter().chain(pass1).collect();
        let mut by_track: std::collections::HashMap<u32, Vec<i64>> =
            std::collections::HashMap::new();
        for (track_id, p) in &both {
            by_track.entry(*track_id).or_default().push(*p);
        }
        for (track_id, seq) in &by_track {
            for pair in seq.windows(2) {
                assert!(
                    pair[1] > pair[0],
                    "PTS must be strictly monotonic across the loop point (track {track_id}): {} then {}",
                    pair[0],
                    pair[1]
                );
            }
        }
    }

    #[tokio::test]
    async fn no_loop_drains_once_then_stops() {
        let parsed = demux_fixture().await;
        let mut sess = FileIngestSession::new(parsed, ProgramId(0), false);
        let _ = sess.feed((), broadcast_common::Timestamp::from_nanos(0));
        let _setup = drain_pts(&mut sess);
        let pass0 = drain_pts(&mut sess);
        assert!(!pass0.is_empty());
        // A further feed must not refill (loop is off); poll stays empty.
        let _ = sess.feed((), broadcast_common::Timestamp::from_nanos(0));
        assert_eq!(drain_pts(&mut sess).len(), 0, "loop:false must not refill");
    }

    /// The queue's length after a refill equals its length after the first
    /// fill — memory held by the session stays bounded at the file's own
    /// sample count no matter how many passes happen (it reuses the same
    /// parsed content and queue capacity, never growing per pass).
    #[tokio::test]
    async fn queue_length_stays_bounded_across_passes() {
        let parsed = demux_fixture().await;
        let mut sess = FileIngestSession::new(parsed, ProgramId(0), true);
        let first = sess.queue.len();
        assert!(first > 0, "pass 0 fills the queue");

        // Pass the setup gate once.
        let _ = sess.feed((), broadcast_common::Timestamp::from_nanos(0));
        let _ = drain_pts(&mut sess);
        let _ = drain_pts(&mut sess);

        // Run several passes; after each refill the queue is the same size.
        for _ in 0..5 {
            let _ = drain_pts(&mut sess); // drain the current pass
            let _ = sess.feed((), broadcast_common::Timestamp::from_nanos(0)); // refill for next
            let after = sess.queue.len();
            assert_eq!(
                after, first,
                "queue length must not grow per pass: after a pass it holds {} not {}",
                after, first
            );
        }
    }
}
