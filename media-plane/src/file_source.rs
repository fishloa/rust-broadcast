//! The sans-IO core of linear file playout: probe a container, demux it into
//! the neutral IR, merge its tracks into one paced publish order, and replay
//! that order (looping, with a monotonic per-track offset) as a
//! [`SessionEvent`] stream through [`FileIngestSession`].
//!
//! No file is opened and no clock is read here beyond the wall-clock instants
//! pacing compares against; a host reads the bytes ([`demux_identified`]
//! takes them) and drives the session. The container is identified by
//! [`container_probe`]; the probe verdict selects the [`transmux`] demuxer
//! ([`select_demuxer`]) and a file is never fed to a guessed one.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Duration;

use broadcast_common::{Demand, Stage, Timestamp, Unpackage};
use container_probe::{Format, IsobmffLayout, Probe};
use thiserror::Error;
use transmux::{
    DemuxEvent, Fmp4Demux, Media, ProgressiveDemux, PsDemux, Sample, StreamingFlvDemux,
    StreamingTsDemux, Track, TrackId, TrackSpec, WebmDemux,
};

use crate::ingress::{IngestSession, ProgramId, SessionEvent};
use crate::trunk::RetentionClass;

/// A failure while reading/probing/demuxing/publishing one file pass.
///
/// Every failure is a **distinct, structured variant** — never one catch-all
/// string — so a caller can match on failure *kind* (the same discipline as
/// a host's own error type, which wraps this one). No variant carries a credential.
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
    /// The probe could not conclude from the bytes it was given, and asked for
    /// more than the file contains.
    ///
    /// `container-probe` takes a slice with no end-of-file signal, so it
    /// answers `Insufficient` whenever a longer buffer *could* decide — which
    /// for a 0-byte or near-empty file is honest but not actionable. This
    /// reader always probes the WHOLE file, so there are no more bytes to
    /// supply: the file is simply too short to identify.
    #[error(
        "file is too short to identify: {file_bytes} bytes, and the probe needs at least {need_at_least}"
    )]
    FileTooShortToIdentify {
        /// The minimum buffer length the probe asked for.
        need_at_least: usize,
        /// How many bytes the file actually holds.
        file_bytes: usize,
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
    /// The configured [`Trunk`](crate::trunk::Trunk) had already given up its [`TrunkWriter`](crate::trunk::TrunkWriter) — a
    /// `Trunk` has exactly one sample/event writer, so a second file reader
    /// over one `Arc<Trunk>` (or a caller that took the writer itself) cannot
    /// publish. Never a panic.
    #[error("the configured trunk's writer is already claimed")]
    WriterUnavailable,
    /// The configured [`Self::Read`] path is not a regular file — a character
    /// device, FIFO, socket, or directory cannot be played as a bounded media
    /// file (and an unbounded read here would grow until the process dies).
    #[error("path {path:?} is not a regular file (kind: {kind})")]
    NotARegularFile {
        /// The path that is not a regular file.
        path: PathBuf,
        /// The file-type label the metadata reported.
        kind: &'static str,
    },
    /// The file is a regular file but exceeds the configured size cap
    /// (the configured `max_file_bytes`) — an operator-supplied path is
    /// never read unbounded, so a multi-gigabyte (or `/dev/zero`-backed) file
    /// fails here rather than after the read has exhausted the process.
    #[error("file {path:?} is {size} bytes, over the {max} byte cap")]
    FileTooLarge {
        /// The path that exceeds the cap.
        path: PathBuf,
        /// The file's actual byte length.
        size: u64,
        /// The configured cap (the configured `max_file_bytes`).
        max: usize,
    },
    /// The file demuxed into zero timed (PTS-carrying) samples — nothing can
    /// be paced, looped, or written, so the source fails rather than spinning
    /// a no-op publish loop forever.
    #[error("file yielded no playable (timed) samples")]
    NoPlayableSamples,
    /// Advancing the per-track loop offset (or applying it to a sample's
    /// PTS/DTS) would overflow `i64` — a file with absurdly large `stts`
    /// deltas cannot be replayed, so the source fails instead of wrapping to
    /// a backwards timestamp.
    #[error("loop offset overflow applying per-track PTS offset")]
    OffsetOverflow,
    /// A sample's presentation time (or the pacing baseline derived from it)
    /// is not representable as a wall-clock [`Duration`] — non-finite,
    /// negative (after the file-start anchor is subtracted), or overflowing a
    /// 64-bit [`std::time::Instant`]. Reachable from a version-1 `tfdt`
    /// `baseMediaDecodeTime` (64-bit) with a small timescale; surfaced as a
    /// structured error rather than a [`Duration::from_secs_f64`] /
    /// `Instant::checked_add` panic.
    #[error("sample pacing time is not representable as a wall-clock duration: {value}")]
    PacingOverflow { value: f64 },
    /// The run was cancelled (its spawned reader was dropped) before it
    /// finished — SP1.4: dropping the owner cancels rather than detaches.
    #[error("file reader cancelled")]
    Cancelled,
}

/// Where one replayable pass ends and the next begins, so a loop can continue the timeline.
#[derive(Debug, Clone, Copy)]
pub struct PassRange {
    /// The track's file span = last sample's relative ticks (0-based).
    pub span_ticks: i64,
    /// One frame duration in the track's timescale (avoids a zero-length or
    /// whole-file-length loop boundary).
    pub frame_ticks: i64,
    /// The track's file span in seconds (`span_ticks / timescale`) — the
    /// cross-track pacing baseline advances by one pass's worth of *wall
    /// time* per loop, exactly as the PTS offsets advance by `span_ticks +
    /// frame_ticks` per track.
    pub span_secs: f64,
    /// One frame duration in seconds (`frame_ticks / timescale`).
    pub frame_secs: f64,
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

/// One sample reference in a pass's merge-ordered publish list.
pub struct PublishItem {
    /// Index into the track table.
    pub track_ref: usize,
    /// The sample itself (from the parsed IR). The per-track loop offset is
    /// applied to its `pts`/`dts` on every write.
    pub sample: Sample,
    /// The sample's **absolute** presentation time in seconds, `pts /
    /// timescale`. This is the one cross-track-comparable key: it orders the
    /// merge by true presentation cadence regardless of a track's own
    /// timescale, so the written absolute-PTS sequence is strictly monotonic
    /// for a sane source (the invariant the timeline tests assert).
    pub due_seconds: f64,
    /// The sample's **absolute decode** time in seconds, `dts / timescale`
    /// (falling back to `pts` when `dts` is absent). Intra-track samples are
    /// emitted in this order — decode order, so a B-frame reference is always
    /// published before the B-frame that will reference it, regardless of the
    /// presentation-time reorder.
    pub decode_seconds: f64,
}

/// A parsed file: the per-track samples plus their specs, prepared for
/// replay. The tracks' samples are in decode order and the IR already has
/// absolute per-sample times.
pub struct ParsedFile {
    /// One [`Track`] per elementary stream to publish.
    pub tracks: Vec<Track>,
}

impl ParsedFile {
    /// Build the playout structure: per-track metadata (`PassRange`), a flat
    /// **decode-ordered per track, presentation-interleaved across tracks**
    /// merge of all samples, and a `TrackSpec` list for the trunk's track set.
    ///
    /// Two distinct orderings coexist and are deliberately NOT collapsed into
    /// one sort key:
    /// * **within a track** samples are emitted in decode order (`dts`, falling
    ///   back to `pts`), so a B-frame reference is always published before the
    ///   B-frame that references it — a global presentation sort would emit the
    ///   referencing B-frame before its P-frame reference and break `trun`
    ///   decode.
    /// * **across tracks** the merge is interleaved by presentation time
    ///   (`pts`), the natural wall-clock cadence.
    pub fn build_playout(&self) -> (Vec<TrackSpec>, Vec<PublishItem>, Vec<PassRange>) {
        let specs: Vec<TrackSpec> = self.tracks.iter().map(|t| t.spec.clone()).collect();

        let mut per_track: Vec<std::collections::VecDeque<PublishItem>> = Vec::new();
        let mut ranges: Vec<PassRange> = Vec::new();

        for (ti, track) in self.tracks.iter().enumerate() {
            let timescale = track.spec.timescale.get().max(1) as f64;
            // Both the pass span and the frame period must reflect the
            // track's **presentation** timeline, not decode order — a track
            // with B-frames reorders, so the decode-order "last" sample is
            // not the presentation max and a loop offset based on it would
            // step backwards across the loop point.
            let mut pres_pts: Vec<i64> = track.samples.iter().filter_map(|s| s.timing.map(|t| t.pts)).collect();
            // A track with no timed samples cannot be paced/looped; give it a
            // zero span/frame so `advance_offsets`' zip stays aligned with the
            // track table, but it contributes no `order` entries.
            if pres_pts.is_empty() {
                ranges.push(PassRange {
                    span_ticks: 0,
                    frame_ticks: 1,
                    span_secs: 0.0,
                    frame_secs: 0.0,
                });
                per_track.push(std::collections::VecDeque::new());
                continue;
            }
            pres_pts.sort_unstable();
            pres_pts.dedup();
            let first_pts = pres_pts[0];
            let max_pts = pres_pts.last().copied().unwrap_or(first_pts);
            let span_ticks = max_pts.checked_sub(first_pts).unwrap_or(0);
            let mut frame = 0i64;
            for pair in pres_pts.windows(2) {
                let d = match pair[1].checked_sub(pair[0]) {
                    Some(d) => d,
                    None => continue,
                };
                if d > 0 && (frame == 0 || d < frame) {
                    frame = d;
                }
            }
            if frame == 0 {
                // Fall back to the first sample's *positive* declared
                // duration — a `Some(0)` duration is treated as absent so the
                // loop offset cannot become a permanent no-op.
                frame = track
                    .samples
                    .iter()
                    .filter_map(|s| s.timing.map(|t| t.duration).map(i64::from).filter(|&d| d > 0))
                    .next()
                    .unwrap_or(1);
            }
            let timescale_secs = 1.0f64 / timescale;
            ranges.push(PassRange {
                span_ticks,
                frame_ticks: frame,
                span_secs: span_ticks as f64 * timescale_secs,
                frame_secs: frame as f64 * timescale_secs,
            });

            // This track's timed samples, in decode order (`dts`, falling back
            // to `pts`). Keeping this intra-track order is what lets the
            // multi-way merge below interleave by presentation time *without*
            // ever reordering a track's own decode sequence.
            let mut mine: Vec<(f64, f64, Sample)> = track
                .samples
                .iter()
                .filter_map(|s| {
                    let pts = s.timing.map(|t| t.pts)?;
                    let dts = s.timing.map(|t| t.dts).or(s.timing.map(|t| t.pts))?;
                    Some((dts as f64 / timescale, pts as f64 / timescale, s.clone()))
                })
                .collect();
            mine.sort_by(|a, b| a.0.total_cmp(&b.0));
            per_track.push(
                mine.into_iter()
                    .map(|(decode_seconds, due_seconds, sample)| PublishItem {
                        track_ref: ti,
                        sample,
                        due_seconds,
                        decode_seconds,
                    })
                    .collect(),
            );
        }

        // Multi-way merge across tracks, keyed by presentation time (`pts`) —
        // the cross-track interleave. Each track's samples are already in
        // decode order, and taking from the front of each queue preserves that
        // within-track decode order: the only freedom is *which* track emits
        // next, decided by the smallest due presentation time (ties broken by
        // track index, then decode time). `due_seconds` can be negative from a
        // version-1 `ctts` composition offset, so compare with `total_cmp` — a
        // total order correct across zero and signs.
        let mut order: Vec<PublishItem> = Vec::new();
        loop {
            // Carry the winning key alongside the index rather than re-reading
            // `front()` on the current best. The re-read needed an `.expect()`
            // to restate an invariant the loop had already established, and a
            // production `.expect()` is a panic-class exception however sound
            // its argument (RELEASE-AUDIT §3 G). Keeping the key makes the
            // invariant structural: there is nothing left to assert.
            let mut best: Option<(usize, f64, usize, f64)> = None;
            for (ti, q) in per_track.iter().enumerate() {
                let Some(front) = q.front() else { continue };
                let key = (ti, front.due_seconds, front.track_ref, front.decode_seconds);
                let better = match &best {
                    None => true,
                    Some((_, due, tref, dec)) => key
                        .1
                        .total_cmp(due)
                        .then(key.2.cmp(tref))
                        .then(key.3.total_cmp(dec))
                        .is_lt(),
                };
                if better {
                    best = Some(key);
                }
            }
            let Some((bi, ..)) = best else { break };
            // `bi` indexed a queue whose `front()` succeeded this pass, so
            // `pop_front` cannot be `None`; handled rather than asserted.
            let Some(item) = per_track[bi].pop_front() else {
                break;
            };
            order.push(item);
        }

        (specs, order, ranges)
    }
}

/// Map a probed container to the [`transmux`] demuxer the reader uses, exactly
/// per the design spec's selection table. `format`/`detail` are taken from a
/// [`container_probe::Probe`]; the ISOBMFF layout is read from the probe's own
/// [`container_probe::Detail`] — never re-derived here.
///
/// Returns [`Err`] only for the "identified but unsupported as a playout
/// source" formats. An `Ambiguous`/`Insufficient`/`Unknown` verdict cannot
/// reach here — those are rejected earlier — so each error carries a
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

/// Identify `bytes` (the whole file, never a prefix) with `container-probe`
/// and demux it: [`demux_identified`] for a settled verdict, and a distinct
/// [`FileReaderError`] for every verdict that cannot name a demuxer.
/// CPU-heavy and synchronous: a host runs it off its async workers.
pub fn probe_and_demux(bytes: &[u8]) -> Result<ParsedFile, FileReaderError> {
    match container_probe::probe_with_budget(bytes, bytes.len()) {
        Probe::Identified { format, detail, .. } => demux_identified(bytes, format, detail),
        Probe::Ambiguous { candidates, .. } => Err(FileReaderError::AmbiguousProbe {
            candidates: candidates
                .iter()
                .map(|c| c.format.name().to_string())
                .collect(),
        }),
        // The whole file was probed, so "supply more bytes" cannot be
        // satisfied: report the file as too short to identify.
        Probe::Insufficient { need_at_least, .. } => Err(FileReaderError::FileTooShortToIdentify {
            need_at_least,
            file_bytes: bytes.len(),
        }),
        // Any other outcome cannot name a demuxer; fail rather than guess.
        _ => Err(FileReaderError::UnknownProbe),
    }
}

/// Map probe+detail to the transmux demuxer per the design spec's table, run
/// it, and return the parsed tracks.
pub fn demux_identified(
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
    // End of input: without `finish()` the demux never announces a track whose
    // sibling PID stayed silent and never emits each live track's held-back
    // final access unit (it needs the *next* unit for its duration).
    demux.finish();
    collect_streaming_media(&mut demux, |d| d.poll_event())
}

/// Feed a whole FLV file through [`StreamingFlvDemux`] and collect its events
/// into a [`Media`].
fn demux_streaming_flv(bytes: &[u8]) -> Result<Media, DemuxError> {
    let mut demux = StreamingFlvDemux::new();
    demux.feed(bytes).map_err(DemuxError::Flv)?;
    demux.finish();
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
    let mut index: std::collections::HashMap<TrackId, usize> = std::collections::HashMap::new();

    while let Some(event) = poll(demux) {
        match event {
            DemuxEvent::TrackAdded(spec) => {
                match index.get(&spec.track_id) {
                    // A repeat `TrackAdded` for an already-known id updates the
                    // existing track in place — never push an orphan duplicate.
                    Some(&i) => tracks[i].0 = spec,
                    None => {
                        let idx = tracks.len();
                        index.insert(spec.track_id, idx);
                        tracks.push((spec, Vec::new()));
                    }
                }
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
    Ok(Media::new(movie, MPEG_TS_MOVIE_TIMESCALE))
}

/// The movie timescale assigned to a TS-derived `Media`: the MPEG-2 TS
/// 90 kHz clock (a 27 MHz system clock divided by 300), per ISO/IEC 13818-1 —
/// every TS demuxer's track timescales are multiples of it. A named constant
/// (not a bare `90_000`) so the value's source is cited where it is used.
const MPEG_TS_MOVIE_TIMESCALE: u32 = 90_000;

/// Convert a non-negative seconds value into a wall-clock [`Duration`] without
/// panicking — `Duration::from_secs_f64` panics on a non-finite, negative, or
/// overflowing input, and `pts as f64 / timescale` is unbounded for a
/// version-1 `tfdt` 64-bit `baseMediaDecodeTime` with a small timescale.
///
/// Returns [`FileReaderError::PacingOverflow`] rather than allow any of those
/// panics to reach a paced run.
pub fn secs_to_duration(value: f64) -> Result<Duration, FileReaderError> {
    Duration::try_from_secs_f64(value).map_err(|_| FileReaderError::PacingOverflow { value })
}

/// Advance each track's accumulated PTS offset by `span + one frame` so the
/// next pass continues on the same monotonic timeline.
///
/// Returns `Err(OffsetOverflow)` if any track's accumulated offset would
/// exceed `i64::MAX` — a file with absurdly large `stts` deltas fails cleanly
/// rather than wrapping to a negative offset and replaying backwards.
pub fn advance_offsets(offsets: &mut [i64], ranges: &[PassRange]) -> Result<(), FileReaderError> {
    for (o, r) in offsets.iter_mut().zip(ranges) {
        let step = r.span_ticks.checked_add(r.frame_ticks);
        let Some(step) = step else {
            return Err(FileReaderError::OffsetOverflow);
        };
        let Some(next) = o.checked_add(step) else {
            return Err(FileReaderError::OffsetOverflow);
        };
        *o = next;
    }
    Ok(())
}

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
///
/// With `pace: true` (the route's setting), `poll` hands out only the samples
/// whose within-pass media time has elapsed on the wall clock — so the trunk
/// advances at roughly realtime instead of dumping the whole file in one drain
/// (which, with the 10 ms route tick, used to republish ~300× realtime). With
/// `pace: false` (the unit tests), every sample is due immediately — no wall
/// clock consulted, so the tests stay deterministic.
pub struct FileIngestSession {
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
    track_ids: Vec<TrackId>,
    /// The file's earliest presentation time in seconds, across all tracks —
    /// the pacing anchor each sample's within-pass offset is measured from.
    file_start_secs: f64,
    /// The current pass's sample queue, in presentation order, PTS-offset
    /// applied, with each sample's **within-pass** media time in seconds
    /// (`due_seconds - file_start_secs`, non-negative) for wall-clock pacing.
    queue: VecDeque<(u32, Sample, f64)>,
    /// The wall-clock instant the current pass began — each sample is due at
    /// `pass_wall_start + within_pass_secs`. `None` until the first sample
    /// phase.
    pass_wall_start: Option<std::time::Instant>,
    /// Gate emission on wall clock (`true`) or emit everything immediately
    /// (`false`).
    pace: bool,
    loop_file: bool,
    established: bool,
    announced: bool,
    started: bool,
    /// Samples handed out since the last [`Stage::feed`] — bounded by
    /// [`MAX_SAMPLES_PER_DRAIN`].
    drained_this_feed: usize,
}

impl FileIngestSession {
    /// Build the session from an already-demuxed file, honouring `loop_file`
    /// and `pace`. Fails ([`FileReaderError::NoPlayableSamples`]) if the file
    /// produced no timed samples.
    pub fn new(
        parsed: ParsedFile,
        program: ProgramId,
        loop_file: bool,
        pace: bool,
    ) -> Result<Self, FileReaderError> {
        let (specs, order, ranges) = parsed.build_playout();
        if order.is_empty() {
            return Err(FileReaderError::NoPlayableSamples);
        }
        let file_start_secs = order
            .iter()
            .map(|i| i.due_seconds)
            .fold(f64::INFINITY, f64::min);
        let track_ids: Vec<TrackId> = specs.iter().map(|t| t.track_id).collect();
        let mut session = FileIngestSession {
            program,
            tracks: specs.into(),
            order,
            ranges,
            offsets: vec![0i64; track_ids.len()],
            track_ids,
            file_start_secs,
            queue: VecDeque::new(),
            pass_wall_start: None,
            pace,
            loop_file,
            established: false,
            announced: false,
            started: false,
            drained_this_feed: 0,
        };
        // First pass: offsets are all zero (the file's own timeline), so this
        // cannot overflow; the checked path is still taken for uniformity.
        session.refill()?;
        Ok(session)
    }

    /// `true` once the finite (non-looping) file is fully drained and nothing
    /// further will be emitted — the controller uses this to finish cleanly.
    pub fn drained(&self) -> bool {
        self.started && !self.loop_file && self.queue.is_empty()
    }

    /// The wall-clock instant the front sample is due, for the controller to
    /// sleep until (rather than tick at 100 Hz). `None` when the queue is
    /// empty or `pace` is off.
    pub fn next_due(&self) -> Option<std::time::Instant> {
        let rel = self.queue.front()?.2;
        let base = self.pass_wall_start?;
        // `rel` is a non-negative within-pass media time, but its magnitude is
        // unbounded (version-1 `tfdt` is 64-bit); a non-representable value
        // clamps to "due now" so the controller does not sleep past forever,
        // and the actual failure surfaces through `poll`'s pacing path.
        let dur = secs_to_duration(rel).ok()?;
        base.checked_add(dur).or_else(|| {
            // Overflow → due immediately (never a panic, never sleeps forever).
            std::time::Instant::now().checked_sub(Duration::ZERO)
        })
    }

    /// Refill the queue from the already-parsed content, applying the current
    /// per-track PTS offset (the loop point), then advance the offset so the
    /// next pass continues monotonically. Reuses the parsed `order` — nothing
    /// is re-demuxed and the queue's capacity is reused, so memory is bounded
    /// at the file's own sample count no matter how many passes happen.
    fn refill(&mut self) -> Result<(), FileReaderError> {
        // Advance the pass's wall-clock baseline by one pass's worth of media
        // time *before* queueing the next pass, the same way `publish_looping`
        // advances its `start` — derived from the same `PassRange` data, not a
        // second mechanism. Without this, from pass 2 on every queued sample's
        // within-pass time is already behind `base.elapsed()`, so `poll` hands
        // out the whole pass in one drain and `next_due()` returns a past
        // instant — the original ~300× behaviour, restored from pass 2 onward.
        let pass_duration_secs = self
            .ranges
            .iter()
            .map(|r| r.span_secs + r.frame_secs)
            .fold(0.0f64, f64::max);
        if self.pace
            && let Some(base) = &mut self.pass_wall_start
        {
            *base = base
                .checked_add(secs_to_duration(pass_duration_secs)?)
                .ok_or(FileReaderError::PacingOverflow {
                    value: pass_duration_secs,
                })?;
        }
        self.queue.clear();
        for item in &self.order {
            let mut sample = item.sample.clone();
            let off = self.offsets[item.track_ref];
            if let Some(t) = sample.timing.as_mut() {
                if let Some(p) = t.pts.checked_add(off) { t.pts = p; } else { return Err(FileReaderError::OffsetOverflow); }
                if let Some(d) = t.dts.checked_add(off) { t.dts = d; } else { return Err(FileReaderError::OffsetOverflow); }
            }
            let within_pass = (item.due_seconds - self.file_start_secs).max(0.0);
            self.queue
                .push_back((self.track_ids[item.track_ref].value(), sample, within_pass));
        }
        advance_offsets(&mut self.offsets, &self.ranges)?;
        Ok(())
    }
}

impl Stage for FileIngestSession {
    type In<'a> = ();
    type Out = SessionEvent;
    type Error = FileReaderError;

    fn feed(&mut self, _input: (), _now: Timestamp) -> Result<(), Self::Error> {
        self.drained_this_feed = 0;
        // With `loop_file`, refill when the previous pass's queue has drained.
        if self.loop_file && self.queue.is_empty() {
            self.refill()?;
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
            if self.pace {
                self.pass_wall_start = Some(std::time::Instant::now());
            }
            return None;
        }
        if self.pace {
            // A burst (everything due at once after a stall) is handed out
            // in bounded drains; the controller loop comes back for the rest.
            if self.drained_this_feed >= MAX_SAMPLES_PER_DRAIN {
                return None;
            }
            // Handle out only the samples whose within-pass media time has
            // elapsed on the wall clock — pacing, so the trunk advances at
            // roughly realtime. A sample not yet due stops this drain (the
            // controller sleeps until `next_due` and feeds again).
            let rel = self.queue.front()?.2;
            let base = self.pass_wall_start?;
            // `rel` is unbounded (see `next_due`); a non-representable value
            // is treated as "elapsed" so the sample is emitted rather than
            // panicked over or held forever.
            match secs_to_duration(rel) {
                Ok(dur) if base.elapsed() < dur => return None,
                Ok(_) | Err(_) => {}
            }
        }
        self.drained_this_feed = self.drained_this_feed.saturating_add(1);
        self.queue
            .pop_front()
            .map(|(track_id, sample, _)| SessionEvent::Sample {
                program: self.program,
                track_id: TrackId::new(track_id),
                retention: RetentionClass::Timed,
                sample,
            })
    }

    fn finish(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn demand(&self) -> Demand {
        Demand::new(0)
    }
}

impl IngestSession for FileIngestSession {
    type Request = bytes::Bytes;
}

/// The most samples one drain of a paced [`FileIngestSession`] hands out
/// before the controller loop gets control back (then the next iteration
/// continues). A stall (a slow disk, a busy blocking pool) makes a whole
/// stretch of the file due at once; without a bound a single drain could
/// publish all of it, and any ring sized to absorb that would have to be sized
/// to the file.
pub const MAX_SAMPLES_PER_DRAIN: usize = 256;

/// The sample-ring depth a file route's `Trunk` needs: a few drains' worth,
/// **independent of the file's length** (audit r07-O4, issue #1083). The old
/// sizing, one entry per sample in the file plus 64, assumed the file was
/// published all at once, which the paced session stopped doing; it made a
/// route's memory grow with the file for no reader's benefit, since the
/// segmenter's cursor drains the ring every iteration.
pub const FILE_TIMED_RING: usize = MAX_SAMPLES_PER_DRAIN * 4;

#[cfg(test)]
mod tests {
    use super::*;
    use transmux::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};

    /// Demux a real TS fixture into a `ParsedFile`.
    fn demux_fixture() -> ParsedFile {
        let bytes = std::fs::read(test_fixtures::path("ts/h264-aac.ts")).expect("read fixture");
        probe_and_demux(&bytes).expect("demux fixture")
    }

    /// Drive a session's `poll` until `None`, collecting the DTS of every
    /// sample (skipping the program/established events). DTS (decode time) is
    /// the intra-track key the reader emits in — decode order — so it is the
    /// timestamp whose monotonicity the loop offset must preserve.
    fn drain_dts(sess: &mut FileIngestSession) -> Vec<(TrackId, i64)> {
        let mut dts = Vec::new();
        while let Some(ev) = sess.poll() {
            if let SessionEvent::Sample {
                track_id, sample, ..
            } = ev
                && let Some(t) = sample.timing
                && let d = t.dts
            {
                dts.push((track_id, d));
            }
        }
        dts
    }

    /// The loop offset keeps each track's published **DTS** strictly monotonic
    /// across the loop point — no reset to zero, no backwards step — which is
    /// the decode-order invariant a B-frame track (this fixture's video has
    /// `dts != pts`) depends on: the decoder consumes in DTS order, so a reset
    /// at the loop point would corrupt every frame that reorders.
    ///
    /// (The file's audio and video carry different timescales, so raw DTS are
    /// not comparable across tracks; the monotonic invariant is therefore held
    /// per track, which is exactly the property the offset exists to preserve.)
    ///
    /// MUTATION PROOF, recorded verbatim: removing the per-track PTS offset on
    /// refill (`sample.dts = Some(d)` instead of `Some(d + off)` in
    /// `FileIngestSession::refill`) makes each pass restart at the file's own
    /// per-track first DTS, so a track's pass-1 first sits at or below its own
    /// pass-0 last — and this test FAILS with a backwards DTS step across the
    /// loop point. Restoring the offset (and a `touch`) makes it pass again.
    #[test]
    fn loop_refill_keeps_dts_monotonic_across_loop_point() {
        let parsed = demux_fixture();
        let mut sess = FileIngestSession::new(parsed, ProgramId(0), true, false)
            .expect("fixture must have playable samples");

        // Pass the one-time setup gate, then collect pass 0's samples.
        let _ = sess.feed((), broadcast_common::Timestamp::from_nanos(0));
        let _setup = drain_dts(&mut sess);
        let pass0 = drain_dts(&mut sess);
        assert!(!pass0.is_empty(), "pass 0 must emit samples");

        // Trigger the loop refill (pass 1, offset advanced) and collect it.
        let _ = sess.feed((), broadcast_common::Timestamp::from_nanos(0));
        let pass1 = drain_dts(&mut sess);
        assert_eq!(
            pass0.len(),
            pass1.len(),
            "each pass emits the same sample count"
        );
        assert!(!pass1.is_empty(), "a looped pass must emit samples");

        // Per-track: concatenate pass0 + pass1 and assert strictly monotonic.
        let both: Vec<(TrackId, i64)> = pass0.into_iter().chain(pass1).collect();
        let mut by_track: std::collections::HashMap<TrackId, Vec<i64>> =
            std::collections::HashMap::new();
        for (track_id, d) in &both {
            by_track.entry(*track_id).or_default().push(*d);
        }
        for (track_id, seq) in &by_track {
            for pair in seq.windows(2) {
                assert!(
                    pair[1] > pair[0],
                    "DTS must be strictly monotonic across the loop point (track {track_id}): {} then {}",
                    pair[0],
                    pair[1]
                );
            }
        }
    }

    #[test]
    fn no_loop_drains_once_then_stops() {
        let parsed = demux_fixture();
        let mut sess = FileIngestSession::new(parsed, ProgramId(0), false, false)
            .expect("fixture must have playable samples");
        let _ = sess.feed((), broadcast_common::Timestamp::from_nanos(0));
        let _setup = drain_dts(&mut sess);
        let pass0 = drain_dts(&mut sess);
        assert!(!pass0.is_empty());
        // A further feed must not refill (loop is off); poll stays empty.
        let _ = sess.feed((), broadcast_common::Timestamp::from_nanos(0));
        assert_eq!(drain_dts(&mut sess).len(), 0, "loop:false must not refill");
    }

    /// The queue's length after a refill equals its length after the first
    /// fill — memory held by the session stays bounded at the file's own
    /// sample count no matter how many passes happen (it reuses the same
    /// parsed content and queue capacity, never growing per pass).
    #[test]
    fn queue_length_stays_bounded_across_passes() {
        let parsed = demux_fixture();
        let mut sess = FileIngestSession::new(parsed, ProgramId(0), true, false)
            .expect("fixture must have playable samples");
        let first = sess.queue.len();
        assert!(first > 0, "pass 0 fills the queue");

        // Pass the setup gate once.
        let _ = sess.feed((), broadcast_common::Timestamp::from_nanos(0));
        let _ = drain_dts(&mut sess);
        let _ = drain_dts(&mut sess);

        // Run several passes; after each refill the queue is the same size.
        for _ in 0..5 {
            let _ = drain_dts(&mut sess); // drain the current pass
            let _ = sess.feed((), broadcast_common::Timestamp::from_nanos(0)); // refill for next
            let after = sess.queue.len();
            assert_eq!(
                after, first,
                "queue length must not grow per pass: after a pass it holds {} not {}",
                after, first
            );
        }
    }

    /// A minimal AVC [`TrackSpec`] at the given timescale — `build_playout`
    /// reads only `timescale`/`track_id`, so the codec config body is inert.
    fn avc_spec(track_id: TrackId, timescale: u32) -> TrackSpec {
        TrackSpec::new(
            track_id,
            timescale,
            transmux::CodecConfig::Avc {
                config: AVCConfigurationBox {
                    config: AVCDecoderConfigurationRecord {
                        configuration_version: 1,
                        profile_indication: 66,
                        profile_compatibility: 0,
                        level_indication: 30,
                        length_size_minus_one: 3,
                        sps: vec![],
                        pps: vec![],
                        chroma_format: None,
                        bit_depth_luma_minus8: None,
                        bit_depth_chroma_minus8: None,
                        sps_ext: vec![],
                    },
                },
                width: 1920,
                height: 1080,
            },
        )
    }

    /// A single-track [`ParsedFile`] whose samples carry the given `pts`
    /// (with `duration = 10`, `dts = pts`).
    fn one_track_file(pts: &[i64]) -> ParsedFile {
        let samples = pts
            .iter()
            .map(|&p| Sample::new(vec![0], Some(p), Some(p), Some(10), true))
            .collect();
        ParsedFile {
            tracks: vec![Track::new(avc_spec(TrackId::new(1), 90_000), samples)],
        }
    }

    /// Finding 5: negative PTS (reachable from a version-1 `ctts` leading
    /// B-frame) must still merge in ascending presentation order. A
    /// `.to_bits()` key inverts negative magnitudes, so `-10` would sort after
    /// `0` and before `-5`.
    #[test]
    fn negative_pts_merge_ascending() {
        let parsed = one_track_file(&[-10, 0, -5, 5]);
        let (_, order, _) = parsed.build_playout();
        let due: Vec<i64> = order
            .iter()
            .map(|i| (i.due_seconds * 90_000.0).round() as i64)
            .collect();
        assert_eq!(
            due,
            vec![-10, -5, 0, 5],
            "presentation order must be ascending, negatives included"
        );
    }

    /// Finding 6: a zero-duration sample is treated as duration-absent, so the
    /// loop frame falls back to one tick — never zero (a zero frame makes
    /// `advance_offsets` a permanent no-op and every pass republish identical
    /// PTS).
    #[test]
    fn zero_duration_falls_back_to_unit_frame() {
        let samples = vec![Sample::new(vec![0], Some(0), Some(0), Some(0), true)];
        let parsed = ParsedFile {
            tracks: vec![Track::new(avc_spec(TrackId::new(1), 90_000), samples)],
        };
        let (_, _, ranges) = parsed.build_playout();
        assert_eq!(
            ranges[0].frame_ticks, 1,
            "a zero duration must fall back to a unit frame, never 0"
        );
    }

    /// Finding 7: advancing an offset past `i64::MAX` fails with
    /// [`FileReaderError::OffsetOverflow`] rather than wrapping to a negative
    /// offset and replaying backwards.
    #[test]
    fn advance_offsets_overflow_fails_cleanly() {
        let mut offsets = vec![i64::MAX - 5];
        let ranges = vec![PassRange {
            span_ticks: 100,
            frame_ticks: 10,
            span_secs: 1.0,
            frame_secs: 1.0,
        }];
        let err = advance_offsets(&mut offsets, &ranges);
        assert!(
            matches!(err, Err(FileReaderError::OffsetOverflow)),
            "offset past i64::MAX must be OffsetOverflow, got {err:?}"
        );
    }

    /// Finding 8: a file yielding zero timed (PTS-carrying) samples fails the
    /// route session with [`FileReaderError::NoPlayableSamples`] instead of
    /// spinning a no-op publish loop.
    #[test]
    fn no_playable_samples_fails_session() {
        let samples = vec![Sample::new(vec![0], None, None, None, false)];
        let parsed = ParsedFile {
            tracks: vec![Track::new(avc_spec(TrackId::new(1), 90_000), samples)],
        };
        let err = FileIngestSession::new(parsed, ProgramId(0), true, false);
        assert!(
            matches!(err, Err(FileReaderError::NoPlayableSamples)),
            "a file with no timed samples must fail"
        );
    }

    /// Audit r07-O4 (#1083): when a stall makes a long stretch of the file due
    /// at once, the paced session must hand it out in bounded drains, and the
    /// file route's length-independent sample ring must then carry every
    /// sample to the segmenter's cursor with no `Lagged` loss. The queue holds
    /// two ring-depths of due samples: handed out in one drain (the old
    /// behaviour) it would overflow the ring before the cursor was polled.
    #[test]
    fn a_due_burst_is_drained_in_bounded_steps_without_overflowing_the_ring() {
        let parsed = demux_fixture();
        let session = FileIngestSession::new(parsed, ProgramId(0), true, true).unwrap();
        let mut driver = crate::ingress::IngestDriver::new(
            session,
            crate::IngestConfig::new(crate::ingress::HandshakePolicy::establish_by(
                broadcast_common::Timestamp::from_nanos(u64::MAX),
            ))
            .with_trunk(
                crate::trunk::TrunkConfig::default()
                    .with_timed_capacity(std::num::NonZeroUsize::new(FILE_TIMED_RING).unwrap()),
            )
            .with_max_programs(std::num::NonZeroUsize::MIN),
        );
        driver.feed((), broadcast_common::Timestamp::ZERO);
        let trunk = driver
            .trunk(ProgramId(0))
            .cloned()
            .expect("the program was announced");
        let mut cursor = trunk.subscribe();

        // A stall: the whole of a two-ring-deep stretch is due right now.
        let burst = FILE_TIMED_RING * 2;
        let template = driver
            .session()
            .queue
            .front()
            .map(|(track, sample, _)| (*track, sample.clone()))
            .expect("the first pass is queued");
        {
            let session = driver.session_mut();
            session.queue.clear();
            for i in 0..burst {
                let mut sample = template.1.clone();
                let dts = i64::try_from(i).unwrap();
                if let Some(t) = sample.timing.as_mut() {
                    t.dts = dts;
                    t.pts = dts;
                } else {
                    sample.timing = Some(transmux::ir::SampleTiming::new(dts, dts, 0));
                }
                session.queue.push_back((template.0, sample, 0.0));
            }
            session.pass_wall_start =
                std::time::Instant::now().checked_sub(std::time::Duration::from_secs(3600));
        }

        let mut delivered = 0usize;
        let mut feeds = 0usize;
        while delivered < burst {
            feeds += 1;
            assert!(feeds <= burst, "no progress draining the burst");
            driver.feed((), broadcast_common::Timestamp::ZERO);
            let mut this_feed = 0usize;
            while let Some(item) = cursor.poll() {
                match item {
                    crate::trunk::SampleCursorItem::Timed { .. } => this_feed += 1,
                    other => panic!("the ring overflowed: {other:?}"),
                }
            }
            assert!(
                this_feed <= MAX_SAMPLES_PER_DRAIN,
                "one drain handed out {this_feed} samples (bound {MAX_SAMPLES_PER_DRAIN})"
            );
            delivered += this_feed;
        }
        assert_eq!(delivered, burst);
        assert!(
            feeds >= burst / MAX_SAMPLES_PER_DRAIN,
            "the burst must have taken several bounded drains, took {feeds}"
        );
    }
}
