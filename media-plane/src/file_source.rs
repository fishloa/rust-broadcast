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

use broadcast_common::timescale::Timescale;
use broadcast_common::{Demand, Stage, Timestamp};
use container_probe::{Format, IsobmffLayout, Probe};
use thiserror::Error;
use transmux::{
    DemuxEvent, Media, Sample, StreamingFlvDemux, StreamingTsDemux, Track, TrackId, TrackSpec,
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
    /// A whole-file read exceeded the configured size cap. No container is
    /// read whole any more, so nothing raises this today; the variant is kept
    /// until Phase 6 removes the cap field and this error with it.
    #[error("file {path:?} is {size} bytes, over the {max} byte cap")]
    FileTooLarge {
        /// The path that exceeds the cap.
        path: PathBuf,
        /// The file's actual byte length.
        size: u64,
        /// The configured cap (the configured `max_file_bytes`).
        max: usize,
    },
    /// The file was a different container when a looping reader re-probed it
    /// for the next pass — a streaming session drives one demuxer for its
    /// whole run and cannot switch containers mid-run.
    #[error("file {path:?} changed container between passes")]
    FileChangedContainer {
        /// The path whose container changed.
        path: PathBuf,
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
    /// The caller fed an input shape the container's demuxer cannot consume:
    /// a sequential chunk to the seekable MP4 stage, or a byte range to a
    /// forward-only TS/FLV demuxer.
    #[error("{container} playout cannot consume a {input} file input")]
    WrongInput {
        /// The container's own label.
        container: &'static str,
        /// The fed input's label.
        input: &'static str,
    },
}

/// Where one replayable pass ends and the next begins, so a loop can continue the timeline.
#[derive(Debug, Clone, Copy)]
pub struct PassRange {
    /// The track's file span = last sample's relative ticks (0-based).
    pub span_ticks: i64,
    /// One frame duration in the track's timescale (avoids a zero-length or
    /// whole-file-length loop boundary).
    pub frame_ticks: i64,
    /// The track's file span (`span_ticks / timescale`) — the cross-track
    /// pacing baseline advances by one pass's worth of *wall time* per loop,
    /// exactly as the PTS offsets advance by `span_ticks + frame_ticks` per
    /// track.
    pub span: Duration,
    /// One frame duration (`frame_ticks / timescale`).
    pub frame: Duration,
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
    /// Fragmented ISOBMFF (CMAF/fMP4) → [`Mp4Demux`](transmux::mp4_demux::Mp4Demux),
    /// driven by the streaming file session.
    Fmp4,
    /// Progressive ISOBMFF → [`Mp4Demux`](transmux::mp4_demux::Mp4Demux),
    /// driven by the streaming file session.
    Progressive,
    /// ISOBMFF whose fragmented/progressive layout the bounded probe could
    /// not settle (a large progressive file's `mdat` runs past the prefix) →
    /// [`Mp4Demux`](transmux::mp4_demux::Mp4Demux), which decides the layout
    /// itself while reading.
    Mp4,
    /// Matroska / WebM → [`MatroskaDemux`](transmux::matroska_demux::MatroskaDemux),
    /// driven by the streaming file session.
    Webm,
    /// MPEG-1/2 Program Stream → [`StreamingPsDemux`](transmux::StreamingPsDemux).
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
            DemuxerKind::Mp4 => "Mp4",
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
            let timescale = track.spec.timescale.get() as f64;
            // Both the pass span and the frame period must reflect the
            // track's **presentation** timeline, not decode order — a track
            // with B-frames reorders, so the decode-order "last" sample is
            // not the presentation max and a loop offset based on it would
            // step backwards across the loop point.
            let mut pres_pts: Vec<i64> = track
                .samples
                .iter()
                .filter_map(|s| s.timing.and_then(|t| t.pts()))
                .collect();
            // A track with no timed samples cannot be paced/looped; give it a
            // zero span/frame so `advance_offsets`' zip stays aligned with the
            // track table, but it contributes no `order` entries.
            if pres_pts.is_empty() {
                ranges.push(PassRange {
                    span_ticks: 0,
                    frame_ticks: 1,
                    span: Duration::ZERO,
                    frame: Duration::ZERO,
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
            for pair in pres_pts.windows(ADJACENT_PAIR_LEN) {
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
                    .filter_map(|s| {
                        s.timing
                            .and_then(|t| t.duration())
                            .map(i64::from)
                            .filter(|&d| d > 0)
                    })
                    .next()
                    .unwrap_or(1);
            }
            ranges.push(PassRange {
                span_ticks,
                frame_ticks: frame,
                span: ticks_to_duration(span_ticks, track.spec.timescale),
                frame: ticks_to_duration(frame, track.spec.timescale),
            });

            // This track's timed samples, in decode order (`dts`, falling back
            // to `pts`). Keeping this intra-track order is what lets the
            // multi-way merge below interleave by presentation time *without*
            // ever reordering a track's own decode sequence.
            let mut mine: Vec<(f64, f64, Sample)> = track
                .samples
                .iter()
                .filter_map(|s| {
                    let pts = s.timing.and_then(|t| t.pts())?;
                    let dts = s
                        .timing
                        .and_then(|t| t.dts())
                        .or(s.timing.and_then(|t| t.pts()))?;
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
                // One stage reads both layouts, so an unsettled layout (a
                // progressive `mdat` longer than the probe prefix) needs no
                // guess: `Mp4Demux` finds `moov` and the sample source itself.
                IsobmffLayout::Unknown => Ok(DemuxerKind::Mp4),
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

/// Largest prefix read to identify a playout file: 4 MiB.
///
/// A container probe needs a sync-stride lattice and a frame-length chain, not
/// the whole stream; 4 MiB covers every format this reader plays, while a
/// file that cannot be settled within it is reported rather than read whole.
pub const PROBE_PREFIX_MAX: usize = 4 << 20;

/// One step of the bounded-prefix probe loop, returned by [`probe_prefix`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeStep {
    /// Read the file's first `prefix_len` bytes and call again.
    NeedPrefix {
        /// How many leading bytes to hand to the next [`probe_prefix`] call.
        prefix_len: usize,
    },
    /// The container is identified and its playout demuxer selected.
    Identified(DemuxerKind),
}

impl ProbeStep {
    /// Stable label for this probe step.
    pub fn name(&self) -> &'static str {
        match self {
            ProbeStep::NeedPrefix { .. } => "need-prefix",
            ProbeStep::Identified(_) => "identified",
        }
    }
}

broadcast_common::impl_spec_display!(ProbeStep);

/// Identify a file from a bounded leading prefix instead of the whole file.
///
/// `prefix` is the file's first `prefix.len()` bytes and `file_len` its total
/// length. The caller starts from [`container_probe::DEFAULT_BUDGET`] bytes and
/// follows each [`ProbeStep::NeedPrefix`] until the demuxer is identified, the
/// [`PROBE_PREFIX_MAX`] cap is reached, or the file itself is exhausted.
///
/// # Errors
/// [`FileReaderError::AmbiguousProbe`] for a near-tie, [`FileReaderError::FileTooShortToIdentify`]
/// when `Insufficient` persists at the cap, [`FileReaderError::UnknownProbe`]
/// for any other verdict, and whatever [`select_demuxer`] rejects.
pub fn probe_prefix(prefix: &[u8], file_len: u64) -> Result<ProbeStep, FileReaderError> {
    match container_probe::probe_with_budget(prefix, prefix.len()) {
        Probe::Identified { format, detail, .. } => {
            Ok(ProbeStep::Identified(select_demuxer(format, detail)?))
        }
        Probe::Ambiguous { candidates, .. } => Err(FileReaderError::AmbiguousProbe {
            candidates: candidates
                .iter()
                .map(|candidate| candidate.format.name().to_string())
                .collect(),
        }),
        Probe::Insufficient { need_at_least, .. } => {
            let limit = usize::try_from(file_len)
                .unwrap_or(usize::MAX)
                .min(PROBE_PREFIX_MAX);
            if prefix.len() >= limit {
                return Err(FileReaderError::FileTooShortToIdentify {
                    need_at_least,
                    file_bytes: prefix.len(),
                });
            }
            let next = need_at_least.max(prefix.len() + 1).min(limit);
            Ok(ProbeStep::NeedPrefix { prefix_len: next })
        }
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
        // An MP4 is streamed by `StreamingFileSession` over
        // `Mp4Demux::seekable`; the whole-buffer path has no arm for it.
        DemuxerKind::Fmp4 | DemuxerKind::Progressive | DemuxerKind::Mp4 => {
            Err(FileReaderError::UnsupportedFormat {
                format: Format::Isobmff.name(),
            })
        }
        // A Matroska file streams through `MatroskaDemux` in
        // `StreamingFileSession`; this whole-buffer path has no arm for it.
        DemuxerKind::Webm => Err(FileReaderError::UnsupportedFormat {
            format: Format::Matroska.name(),
        }),
        // A Program Stream streams through `StreamingPsDemux` in
        // `StreamingFileSession`; this whole-buffer path has no arm for it.
        DemuxerKind::Ps => Err(FileReaderError::UnsupportedFormat {
            format: Format::MpegPs.name(),
        }),
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
    demux.feed(bytes).map_err(DemuxError::Transmux)?;
    // End of input: without `finish()` the demux never announces a track whose
    // sibling PID stayed silent and never emits each live track's held-back
    // final access unit (it needs the *next* unit for its duration).
    demux.finish().map_err(DemuxError::Transmux)?;
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
const MPEG_TS_MOVIE_TIMESCALE: Timescale = transmux::VIDEO_CLOCK_TIMESCALE;

/// Window size for walking consecutive presentation-timestamp pairs.
pub(crate) const ADJACENT_PAIR_LEN: usize = 2;

/// Divisor of the rounding term added before a nanosecond conversion.
const ROUNDING_DIVISOR: u128 = 2;

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

/// Convert a non-negative media-timescale tick count to a wall-clock
/// [`Duration`] (`ticks / timescale`).
pub(crate) fn ticks_to_duration(ticks: i64, timescale: Timescale) -> Duration {
    let ticks = u128::try_from(ticks).unwrap_or(0);
    let denom = u128::from(timescale.get());
    let nanos = (ticks * u128::from(broadcast_common::units::NANOS_PER_SEC)
        + denom / ROUNDING_DIVISOR)
        / denom;
    Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
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
        let pass_duration = self
            .ranges
            .iter()
            .map(|r| r.span + r.frame)
            .fold(Duration::ZERO, |acc, d| acc.max(d));
        if self.pace
            && let Some(base) = &mut self.pass_wall_start
        {
            *base = base
                .checked_add(pass_duration)
                .ok_or(FileReaderError::PacingOverflow {
                    value: pass_duration.as_secs_f64(),
                })?;
        }
        self.queue.clear();
        for item in &self.order {
            let mut sample = item.sample.clone();
            let off = self.offsets[item.track_ref];
            if let Some(t) = sample.timing.as_mut() {
                let current = t.pts().unwrap_or(0);
                let shifted = current
                    .checked_add(off)
                    .ok_or(FileReaderError::OffsetOverflow)?;
                t.shift(shifted - current);
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
    fn timescale(value: u32) -> Timescale {
        Timescale::try_new(value).expect("a non-zero timescale")
    }

    use super::*;
    use isobmff::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};

    /// Demux a real TS fixture into a `ParsedFile`.
    fn demux_fixture() -> ParsedFile {
        let bytes = std::fs::read(test_fixtures::path("ts/h264-aac.ts")).expect("read fixture");
        probe_and_demux(&bytes).expect("demux fixture")
    }

    /// A pass span is a `Duration` whose value is exactly `span_ticks /
    /// timescale` of wall time: 90 000 ticks at 90 kHz is 1 s, and a 1536-tick
    /// AAC frame at 48 kHz is 32 ms. (Retyped from an `f64` seconds field,
    /// #2731; this pins the conversion.)
    #[test]
    fn pass_span_is_ticks_over_timescale() {
        assert_eq!(
            ticks_to_duration(90_000, timescale(90_000)),
            Duration::from_secs(1)
        );
        assert_eq!(
            ticks_to_duration(1536, timescale(48_000)),
            Duration::from_millis(32)
        );
    }

    /// A real file's pacing span is the same wall time its tick count stands
    /// for, so the loop baseline advances by the typed value.
    #[test]
    fn fixture_pass_span_matches_its_tick_count() {
        let parsed = demux_fixture();
        let (_, _, ranges) = parsed.build_playout();
        let range = ranges.first().expect("at least one pass range");
        let ts = parsed.tracks[0].spec.timescale;
        assert_eq!(range.span, ticks_to_duration(range.span_ticks, ts));
        assert_eq!(range.frame, ticks_to_duration(range.frame_ticks, ts));
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
                && let Some(d) = sample.timing.and_then(|t| t.dts())
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
    fn avc_spec(track_id: TrackId, timescale: Timescale) -> TrackSpec {
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
            .collect::<Result<Vec<_>, _>>()
            .expect("valid timing");
        ParsedFile {
            tracks: vec![Track::new(
                avc_spec(TrackId::new(1), timescale(90_000)),
                samples,
            )],
        }
    }

    /// A single-track [`ParsedFile`] at an explicit timescale, each sample
    /// carrying the given `pts` (with `dts = pts`, `duration = 10`).
    fn one_track_file_at(pts: &[i64], timescale_hz: u32) -> ParsedFile {
        let samples = pts
            .iter()
            .map(|&p| Sample::new(vec![0], Some(p), Some(p), Some(10), true))
            .collect::<Result<Vec<_>, _>>()
            .expect("valid timing");
        ParsedFile {
            tracks: vec![Track::new(
                avc_spec(TrackId::new(1), timescale(timescale_hz)),
                samples,
            )],
        }
    }

    /// The loop frame period is the track's **minimum positive PTS gap**, not
    /// its first or last gap and not the declared sample duration: with gaps
    /// `[6000, 3000, 6000]` the frame is `3000`, so a loop pass abuts the
    /// previous one instead of overlapping or leaving a hole.
    #[test]
    fn frame_period_is_the_minimum_positive_pts_gap() {
        let parsed = one_track_file(&[0, 6000, 9000, 15000]);
        let (_, _, ranges) = parsed.build_playout();
        assert_eq!(
            ranges[0].frame_ticks, 3000,
            "the frame must be the smallest positive gap, not the last one"
        );
        assert_eq!(ranges[0].span_ticks, 15_000);
        assert_eq!(
            ranges[0].frame,
            Duration::from_nanos(33_333_333),
            "3000 ticks at 90 kHz is one 30 fps frame"
        );
    }

    /// A track with a single sample has no PTS gap, so its frame period comes
    /// from the sample's own declared duration — and a declared `Some(0)` is
    /// duration-absent, never a zero-length loop step.
    #[test]
    fn single_sample_track_uses_its_positive_declared_duration() {
        let samples =
            vec![Sample::new(vec![0], Some(0), Some(0), Some(3000), true).expect("valid timing")];
        let parsed = ParsedFile {
            tracks: vec![Track::new(
                avc_spec(TrackId::new(1), timescale(90_000)),
                samples,
            )],
        };
        let (_, _, ranges) = parsed.build_playout();
        assert_eq!(ranges[0].span_ticks, 0);
        assert_eq!(ranges[0].frame_ticks, 3000);
    }

    /// A pacing time that cannot be a wall-clock [`Duration`] (non-finite,
    /// negative, or overflowing) is a typed [`FileReaderError::PacingOverflow`],
    /// never a `Duration::ZERO` and never a panic.
    #[test]
    fn secs_to_duration_rejects_non_representable_values() {
        for bad in [f64::NAN, -1.0, 1e300] {
            assert!(
                matches!(
                    secs_to_duration(bad),
                    Err(FileReaderError::PacingOverflow { .. })
                ),
                "{bad} must be PacingOverflow"
            );
        }
        assert_eq!(
            secs_to_duration(1.5).expect("representable"),
            Duration::from_millis(1500)
        );
    }

    /// Tick-to-duration conversion rounds to the nearest nanosecond at the
    /// half-way point (the `+ denom / 2` half-up step), so 1 tick at 6 Hz is
    /// 166_666_667 ns, not the truncated 166_666_666 ns.
    #[test]
    fn ticks_to_duration_rounds_to_nearest_nanosecond() {
        assert_eq!(
            ticks_to_duration(90_000, timescale(90_000)),
            Duration::from_secs(1)
        );
        assert_eq!(
            ticks_to_duration(1, timescale(3)),
            Duration::from_nanos(333_333_333)
        );
        assert_eq!(
            ticks_to_duration(1, timescale(6)),
            Duration::from_nanos(166_666_667)
        );
    }

    /// A loop's second pass continues at `first pass max + one frame`, so the
    /// 48 kHz sample-count timeline is strictly monotonic across the loop
    /// point with no overlapping or skipped frame.
    #[test]
    fn second_loop_pass_continues_at_span_plus_one_frame() {
        let parsed = one_track_file_at(&[0, 1024], 48_000);
        let (_, _, ranges) = parsed.build_playout();
        assert_eq!(ranges[0].span_ticks, 1024);
        assert_eq!(ranges[0].frame_ticks, 1024);

        let mut sess = FileIngestSession::new(parsed, ProgramId(0), true, false)
            .expect("fixture must have playable samples");
        let _ = sess.feed((), broadcast_common::Timestamp::from_nanos(0));
        let _setup = drain_dts(&mut sess);
        let pass0: Vec<i64> = drain_dts(&mut sess).into_iter().map(|(_, d)| d).collect();
        assert_eq!(pass0, vec![0, 1024]);

        let _ = sess.feed((), broadcast_common::Timestamp::from_nanos(0));
        let pass1: Vec<i64> = drain_dts(&mut sess).into_iter().map(|(_, d)| d).collect();
        assert_eq!(
            pass1,
            vec![2048, 3072],
            "pass 2 starts at span + frame (2048) and ends at max + span + frame (3072)"
        );
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
        let samples =
            vec![Sample::new(vec![0], Some(0), Some(0), Some(0), true).expect("valid timing")];
        let parsed = ParsedFile {
            tracks: vec![Track::new(
                avc_spec(TrackId::new(1), timescale(90_000)),
                samples,
            )],
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
            span: Duration::from_secs(1),
            frame: Duration::from_secs(1),
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
        let samples = vec![Sample::new(vec![0], None, None, None, false).expect("valid timing")];
        let parsed = ParsedFile {
            tracks: vec![Track::new(
                avc_spec(TrackId::new(1), timescale(90_000)),
                samples,
            )],
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
                    t.shift(dts - t.dts().unwrap_or(dts));
                } else {
                    sample.timing = Some(transmux::ir::SampleTiming::timed(dts, dts));
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

    /// The file-route ring is a fixed multiple of one drain's bound, so its
    /// memory is independent of the file's length. Pinned by value: if either
    /// constant moves, the relationship (not just the product) is asserted.
    #[test]
    fn file_timed_ring_is_four_bounded_drains() {
        assert_eq!(MAX_SAMPLES_PER_DRAIN, 256);
        assert_eq!(FILE_TIMED_RING, 1024);
        assert_eq!(FILE_TIMED_RING, MAX_SAMPLES_PER_DRAIN * 4);
    }

    /// Every [`DemuxerKind`] label is its documented token, and `Display`
    /// renders that same token (workspace #204 convention).
    #[test]
    fn demuxer_kind_names_are_their_tokens() {
        for (kind, token) in [
            (DemuxerKind::StreamingTs, "StreamingTs"),
            (DemuxerKind::Fmp4, "Fmp4"),
            (DemuxerKind::Progressive, "Progressive"),
            (DemuxerKind::Mp4, "Mp4"),
            (DemuxerKind::Webm, "Webm"),
            (DemuxerKind::Ps, "Ps"),
            (DemuxerKind::StreamingFlv, "StreamingFlv"),
        ] {
            assert_eq!(kind.name(), token);
            assert_eq!(format!("{kind}"), token);
        }
    }

    /// Identify `bytes` and return the probe's format/detail — the real inputs
    /// `select_demuxer` is handed, never a hand-built `Detail`.
    fn identify(bytes: &[u8]) -> (Format, container_probe::Detail) {
        match container_probe::probe(bytes) {
            Probe::Identified { format, detail, .. } => (format, detail),
            other => panic!("expected an identified container, got {other:?}"),
        }
    }

    /// The ISOBMFF layout the probe reports selects exactly one streaming
    /// demuxer: a fragmented file gets `Fmp4`, a progressive one
    /// `Progressive`, and an unsettled layout gets the one `Mp4` stage, which
    /// decides the layout while reading.
    #[test]
    fn select_demuxer_maps_the_probed_isobmff_layout() {
        let frag = test_fixtures::read("mp4/h264-aac-frag.mp4");
        let (format, detail) = identify(&frag);
        assert_eq!(format, Format::Isobmff);
        assert_eq!(
            select_demuxer(format, detail).expect("fragmented ISOBMFF is supported"),
            DemuxerKind::Fmp4
        );

        let prog = test_fixtures::read("mp4/h264-aac-prog.mp4");
        let (format, detail) = identify(&prog);
        assert_eq!(format, Format::Isobmff);
        assert_eq!(
            select_demuxer(format, detail).expect("progressive ISOBMFF is supported"),
            DemuxerKind::Progressive
        );

        // A prefix holding only `ftyp` identifies as ISOBMFF but cannot settle
        // the layout; the unified stage still reads it.
        let head = test_fixtures::read("mp4/cmaf/av-frag.mp4");
        let (format, detail) = identify(&head[..64]);
        assert_eq!(format, Format::Isobmff);
        assert_eq!(
            select_demuxer(format, detail).expect("an unsettled ISOBMFF layout is supported"),
            DemuxerKind::Mp4
        );

        // An ISOBMFF verdict paired with a non-ISOBMFF detail cannot name a
        // layout either.
        let (_, ts_detail) = identify(&test_fixtures::read("ts/h264-aac.ts"));
        assert!(
            matches!(
                select_demuxer(Format::Isobmff, ts_detail),
                Err(FileReaderError::UndeterminedLayout)
            ),
            "a non-ISOBMFF detail under an ISOBMFF format must not pick a demuxer"
        );
    }

    /// A buffer far too short to identify is reported with the probe's own
    /// `need_at_least`, and that need strictly exceeds the bytes supplied so
    /// the documented caller loop can make progress.
    #[test]
    fn probe_and_demux_reports_a_too_short_file() {
        let bytes = [0u8; 3];
        match probe_and_demux(&bytes).err().expect("3 bytes cannot demux") {
            FileReaderError::FileTooShortToIdentify {
                need_at_least,
                file_bytes,
            } => {
                assert_eq!(file_bytes, 3);
                assert!(need_at_least > 3, "need {need_at_least} must exceed 3");
            }
            other => panic!("3 bytes must be FileTooShortToIdentify, got {other:?}"),
        }
    }

    /// A six-packet TS lattice (weak) followed by four chained ADTS frames
    /// ties the two probers at `LATTICE_WEAK`, which must surface as
    /// `AmbiguousProbe` naming both candidates — never a guessed demuxer.
    #[test]
    fn probe_and_demux_reports_an_ambiguous_probe() {
        const PACKET: usize = 188;
        const PACKETS: usize = 6;
        const ADTS_LEN: usize = 274;
        let mut bytes = vec![0u8; PACKETS * PACKET + 4 * ADTS_LEN];
        for i in 0..PACKETS {
            bytes[i * PACKET] = 0x47;
        }
        for k in 0..4 {
            let o = PACKETS * PACKET + k * ADTS_LEN;
            bytes[o] = 0xFF;
            bytes[o + 1] = 0xF1;
            bytes[o + 4] = ((ADTS_LEN >> 3) & 0xFF) as u8;
            bytes[o + 5] = ((ADTS_LEN & 0x07) as u8) << 5;
        }
        match probe_and_demux(&bytes).err().expect("the tie cannot demux") {
            FileReaderError::AmbiguousProbe { candidates } => {
                assert!(
                    candidates.iter().any(|c| c == "MpegTs"),
                    "the TS candidate must be named, got {candidates:?}"
                );
                assert!(
                    candidates.iter().any(|c| c == "AdtsAac" || c == "Mp3"),
                    "the elementary-stream candidate must be named, got {candidates:?}"
                );
            }
            other => panic!("a TS/ADTS tie must be AmbiguousProbe, got {other:?}"),
        }
    }

    /// An AVC track spec at `width` — so a mid-file config change is
    /// observable in the collected [`Media`].
    fn avc_spec_sized(track_id: TrackId, width: u16) -> TrackSpec {
        let base = avc_spec(track_id, timescale(90_000));
        let transmux::CodecConfig::Avc { config, height, .. } = base.config else {
            panic!("the helper builds an AVC track");
        };
        TrackSpec::new(
            track_id,
            timescale(90_000),
            transmux::CodecConfig::Avc {
                config,
                width,
                height,
            },
        )
    }

    /// Collect a scripted [`DemuxEvent`] sequence through the real
    /// [`collect_streaming_media`] translator.
    fn collect_events(events: Vec<DemuxEvent>) -> Media {
        let mut queue: VecDeque<DemuxEvent> = events.into();
        collect_streaming_media(&mut queue, |q| q.pop_front())
            .map_err(|_| ())
            .expect("scripted events collect")
    }

    /// A `TrackUpdated` (a new in-band SPS, or a changed PMT descriptor) must
    /// replace the collected track's config, so late codec config reaches the
    /// `Media` rather than being dropped.
    #[test]
    fn collect_streaming_media_applies_a_track_updated_spec() {
        let id = TrackId::new(1);
        let sample = Sample::new(vec![0], Some(0), Some(0), Some(10), true).expect("valid timing");
        let media = collect_events(vec![
            DemuxEvent::TrackAdded(avc_spec_sized(id, 640)),
            DemuxEvent::sample(id, sample.clone()),
            DemuxEvent::TrackUpdated(avc_spec_sized(id, 1280)),
            DemuxEvent::sample(id, sample),
        ]);
        assert_eq!(media.tracks.len(), 1);
        assert_eq!(media.tracks[0].samples.len(), 2);
        assert_eq!(
            media.tracks[0].spec,
            avc_spec_sized(id, 1280),
            "the updated spec must replace the originally-collected one"
        );
    }

    /// Every non-sample demux event is metadata: it adds no track and no
    /// sample, and never removes the one real track.
    #[test]
    fn collect_streaming_media_ignores_non_sample_events() {
        let id = TrackId::new(1);
        let provenance = transmux::EventProvenance::default();
        let sample = Sample::new(vec![0], Some(0), Some(0), Some(10), true).expect("valid timing");
        let media = collect_events(vec![
            DemuxEvent::TrackAdded(avc_spec_sized(id, 640)),
            DemuxEvent::clock_reference(0, None, timescale(27_000_000), false, provenance),
            DemuxEvent::discontinuity(Some(id), transmux::DiscontinuityKind::Signalled, provenance),
            DemuxEvent::input_degraded(
                Some(id),
                transmux::InputDegradation::TransportError,
                provenance,
            ),
            DemuxEvent::tracks_resolved(1),
            DemuxEvent::sample(id, sample),
            DemuxEvent::track_removed(id, provenance),
            DemuxEvent::track_abandoned(
                None,
                transmux::AbandonReason::ConfigUnrecoverable,
                provenance,
            ),
        ]);
        assert_eq!(media.tracks.len(), 1);
        assert_eq!(media.tracks[0].samples.len(), 1);
    }

    /// `drained()` is true only for a **started, finite** file whose queue has
    /// emptied — never on a fresh session, never before the first sample
    /// phase, and never for a looped file.
    #[test]
    fn drained_requires_a_started_finite_emptied_file() {
        let mut sess =
            FileIngestSession::new(one_track_file(&[0, 3000]), ProgramId(0), false, false)
                .expect("playable samples");
        assert!(!sess.drained(), "a fresh session has not started");
        let _ = sess.feed((), broadcast_common::Timestamp::ZERO);
        assert!(matches!(sess.poll(), Some(SessionEvent::Established)));
        assert!(matches!(sess.poll(), Some(SessionEvent::NewProgram { .. })));
        assert!(!sess.drained(), "announced but not yet started");
        assert!(sess.poll().is_none(), "the start gate ends this drain");
        assert!(!sess.drained(), "samples are still queued");
        while sess.poll().is_some() {}
        assert!(sess.drained(), "an emptied finite file is drained");

        let mut looped =
            FileIngestSession::new(one_track_file(&[0, 3000]), ProgramId(0), true, false)
                .expect("playable samples");
        let _ = looped.feed((), broadcast_common::Timestamp::ZERO);
        while looped.poll().is_some() {}
        assert!(
            !looped.drained(),
            "a looped file is never drained while looping"
        );
    }

    /// A paced session only hands out samples whose within-pass media time has
    /// elapsed on the wall clock, and `next_due` names the front sample's
    /// instant — never a sample 50 ms in the future on the first call.
    #[test]
    fn paced_poll_holds_samples_until_their_instant() {
        // 48 kHz: PTS 0 and 2400 are 50 ms apart.
        let mut sess = FileIngestSession::new(
            one_track_file_at(&[0, 2400], 48_000),
            ProgramId(0),
            false,
            true,
        )
        .expect("playable samples");
        assert_eq!(sess.next_due(), None, "no pass baseline before start");
        let _ = sess.feed((), broadcast_common::Timestamp::ZERO);
        assert!(matches!(sess.poll(), Some(SessionEvent::Established)));
        assert!(matches!(sess.poll(), Some(SessionEvent::NewProgram { .. })));
        assert!(sess.poll().is_none(), "the start gate ends this drain");
        let base = sess.pass_wall_start.expect("a paced pass has a baseline");
        assert_eq!(
            sess.next_due(),
            Some(base),
            "the first sample is due at the pass baseline"
        );
        assert!(
            matches!(sess.poll(), Some(SessionEvent::Sample { .. })),
            "the sample due at the baseline is handed out"
        );
        assert!(
            sess.poll().is_none(),
            "a sample 50 ms away must not be handed out yet"
        );
        sess.pass_wall_start = Some(
            base.checked_sub(Duration::from_secs(10))
                .expect("10 s before now"),
        );
        assert!(
            matches!(sess.poll(), Some(SessionEvent::Sample { .. })),
            "once the baseline is 10 s back, the second sample is due"
        );
        assert_eq!(sess.next_due(), None, "an empty queue has no due instant");
    }

    /// With pacing off, no wall clock is consulted: `next_due` is always
    /// `None`.
    #[test]
    fn unpaced_session_has_no_due_instant() {
        let mut sess =
            FileIngestSession::new(one_track_file(&[0, 3000]), ProgramId(0), false, false)
                .expect("playable samples");
        let _ = sess.feed((), broadcast_common::Timestamp::ZERO);
        assert!(matches!(sess.poll(), Some(SessionEvent::Established)));
        assert!(matches!(sess.poll(), Some(SessionEvent::NewProgram { .. })));
        assert!(sess.poll().is_none());
        assert_eq!(sess.next_due(), None);
    }

    /// Paced sample times are measured from the file's **earliest** PTS, not
    /// from zero: a file starting at 0.1 s is due at the pass baseline, and
    /// its second sample one frame later (33 ms), never 233 ms in.
    #[test]
    fn paced_sample_times_are_measured_from_the_file_start() {
        // 90 kHz: PTS 9000 and 12000 -> the file starts at 0.1 s and the
        // second sample is 3000 ticks (1/30 s) after the first.
        let mut sess =
            FileIngestSession::new(one_track_file(&[9000, 12000]), ProgramId(0), false, true)
                .expect("playable samples");
        let _ = sess.feed((), broadcast_common::Timestamp::ZERO);
        assert!(matches!(sess.poll(), Some(SessionEvent::Established)));
        assert!(matches!(sess.poll(), Some(SessionEvent::NewProgram { .. })));
        assert!(sess.poll().is_none());
        let base = sess.pass_wall_start.expect("a paced pass has a baseline");
        assert_eq!(
            sess.next_due(),
            Some(base),
            "the file-start sample is due at the baseline, not 0.1 s into it"
        );
        assert!(matches!(sess.poll(), Some(SessionEvent::Sample { .. })));
        assert!(
            sess.poll().is_none(),
            "the second sample is one frame (33 ms) away, not 233 ms"
        );
    }

    /// A loop pass advances the wall-clock baseline by exactly one pass
    /// (`span + frame`) — addition, not subtraction, or every pass after the
    /// first would run at full speed.
    #[test]
    fn loop_refill_advances_the_wall_clock_baseline_by_one_pass() {
        // 90 kHz, span 15000 ticks (0.1666..s), frame 3000 (0.0333..s): the
        // two together are exactly 18000 ticks = 200 ms.
        let mut sess = FileIngestSession::new(
            one_track_file(&[0, 3000, 9000, 15000]),
            ProgramId(0),
            true,
            true,
        )
        .expect("playable samples");
        let _ = sess.feed((), broadcast_common::Timestamp::ZERO);
        assert!(matches!(sess.poll(), Some(SessionEvent::Established)));
        assert!(matches!(sess.poll(), Some(SessionEvent::NewProgram { .. })));
        assert!(sess.poll().is_none());
        let first = sess.pass_wall_start.expect("a paced pass has a baseline");
        sess.queue.clear();
        let _ = sess.feed((), broadcast_common::Timestamp::ZERO);
        let second = sess.pass_wall_start.expect("a paced pass has a baseline");
        assert_eq!(
            second.duration_since(first),
            Duration::from_millis(200),
            "the second pass starts one span+frame (200 ms) after the first"
        );
    }

    /// The file session asks for no particular read size (it is pulled by its
    /// driver, not pushed a socket read).
    #[test]
    fn file_session_demand_is_zero() {
        let sess = FileIngestSession::new(one_track_file(&[0, 3000]), ProgramId(0), false, false)
            .expect("playable samples");
        assert_eq!(sess.demand(), Demand::new(0));
    }
}
