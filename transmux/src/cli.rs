//! `transmux` command-line packager — the any-to-any hub as a CLI (issue #482).
//!
//! Drives the existing streaming demux stages ([`StreamingTsDemux`],
//! [`Mp4Demux`], [`StreamingFlvDemux`], [`StreamingPsDemux`], [`WebmDemux`]) into the
//! existing mux stages ([`CmafMuxStage`], [`StreamingTsHlsSegmenter`],
//! [`DashPackager`], [`ProgressiveMuxStage`], [`TsMuxStage`]) as
//! `MuxOutput` arrives. It writes **no** new demux/mux logic — it is a
//! front-end that autodetects the input container, streams it through the hub,
//! and writes the chosen output. Samples are merged by decode time as
//! `mux_stage::drive_media` does, so a streamed output is the same bytes the
//! whole-`Media` path produced.
//!
//! Follows the workspace CLI standard (see `docs/CLI-STANDARD.md`): `clap`
//! derive, named flags (a single obvious positional `<IN>` input), auto
//! `--help`/`--version`, human output to stdout / diagnostics to stderr, exit `0`
//! on success and non-zero on error.
//!
//! ```text
//! transmux in.ts  -o out.cmaf   -f cmaf
//! transmux in.mp4 -o out.m3u8   -f hls   --segment-duration 4
//! transmux in.ts  -o out.m3u8   -f ts-hls
//! transmux in.webm -o out.mp4   -f progressive
//! ```
//!
//! Only this module (and `main.rs`) is `std`; the library stays `no_std`.

use crate::TrackId;
use std::fmt;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use broadcast_common::timescale::{Timescale, cmp_ticks};
use broadcast_common::{Stage, Timestamp, Unpackage};

use crate::dash::DashPackager;
use crate::data_identity::{DataIdentityMap, DataIdentityStage};
use crate::flv_stream::StreamingFlvDemux;
use crate::ir::{CodecConfig, DataCarriage, DataIdentity, DemuxEvent, Media, Sample, TrackSpec};
use crate::media::{CmafMuxStage, FragmentCut};
use crate::mp4_demux::{Mp4Demux, Mp4DemuxConfig, Mp4Input};
use crate::mux_stage::{MuxInput, MuxOutput, SegmentOut, TrackPart};
use crate::pipeline::{FragmentTrackData, build_init_segment, build_media_segment};
use crate::plan_pass::PlanPass;
use crate::progressive::ProgressiveMuxStage;
use crate::ps_demux::StreamingPsDemux;
use crate::segment_log::SegmentSummaryLog;
use crate::segmenter::{MediaClock, Segmenter, choose_anchor};
use crate::ts_demux::StreamingTsDemux;
use crate::ts_hls::StreamingTsHlsSegmenter;
use crate::ts_mux::{DEFAULT_PSI_INTERVAL, TsMuxStage};
use crate::webm_demux::WebmDemux;
#[cfg(feature = "cenc")]
use isobmff::box_types::KEY_ID_LEN;
use std::time::Duration;

// ---------------------------------------------------------------------------
// Container detection is delegated to `container-probe` (issue #960, audit
// r05-W29).
// ---------------------------------------------------------------------------
//
// An earlier `detect_container` here matched fixed byte offsets (a `0x47` at
// offset 0 *and* 188, a fourcc at offset 4, the EBML magic at 0). That silently
// missed a 192-byte M2TS capture and any capture that starts mid-packet, and it
// duplicated work `container-probe` was written to do properly — a stride×phase
// lattice search over 188/192/204/208, structural scoring, and an explicit
// `Insufficient` ("read more") answer instead of a guess. `container-probe` is a
// `cli`-only optional dependency, so this detection is available exactly where
// it is used: the `transmux` binary, never the `no_std` library.

/// Default segment duration in seconds when `--segment-duration` is omitted.
const DEFAULT_SEGMENT_DURATION_SECS: u32 = 6;
/// Chunks per segment in the LL-DASH profile: the chunk is half a segment.
const LL_DASH_CHUNKS_PER_SEGMENT: u32 = 2;
/// Hex digits in the KID half of a `--key` argument.
#[cfg(feature = "cenc")]
const KEY_ID_HEX_LEN: usize = KEY_ID_LEN * 2;

/// Default LL-DASH target latency in milliseconds (`Latency@target`), used when
/// `--ll` selects the DASH low-latency profile.
const LL_DASH_LATENCY_TARGET: Duration = Duration::from_secs(3);
/// The wall-clock length of one interleaved progressive chunk — half a second.
///
/// An interleaved file lets a player that can only read strictly forward start
/// decoding as soon as it has the first chunk of every track; one chunk per
/// track (what this CLI wrote before, audit r05-W25) means downloading
/// essentially the whole video track before the first audio sample arrives, so
/// fast start bought nothing on a simple HTTP-progressive client. Half a second
/// is the usual trade (chunk headers are 4-8 bytes per chunk).
const PROGRESSIVE_INTERLEAVE_MILLIS: u32 = 500;
/// `#EXT-X-VERSION` for the CMAF-HLS playlist: 7 is the floor for the
/// `#EXT-X-MAP` tag on an fMP4 playlist (RFC 8216 §7).
const HLS_CMAF_VERSION: u8 = 7;
/// The Media Initialization Section file the CMAF-HLS playlist's `#EXT-X-MAP`
/// points at, written beside the segments.
const HLS_INIT_FILE: &str = "init.mp4";

/// The current UTC time as an ISO-8601 `YYYY-MM-DDThh:mm:ssZ` string, for
/// `MPD@availabilityStartTime`.
///
/// The CLI cannot know the moment the stream *starts*, but a `dynamic` MPD whose
/// `availabilityStartTime` is the epoch (the previous placeholder) puts the live
/// edge 56 years in the past, so every player computes a segment number that
/// does not exist and the output is unplayable by construction (audit r05-W29).
/// "Now at packaging time" is the honest choice for a file-based CLI: the media
/// segments this run writes are available from roughly then.
fn availability_start_time_now() -> String {
    availability_start_time(jiff::Timestamp::now().as_second())
}

/// `YYYY-MM-DDThh:mm:ssZ` for `unix_secs` (whole seconds, UTC); an instant
/// outside `jiff`'s range falls back to the epoch rather than panicking.
fn availability_start_time(unix_secs: i64) -> String {
    let ts = jiff::Timestamp::from_second(unix_secs).unwrap_or(jiff::Timestamp::UNIX_EPOCH);
    // `Timestamp`'s Display is RFC 3339 UTC with a `Z` and, for a whole-second
    // instant, no fractional digits.
    format!("{ts}")
}

#[cfg(test)]
mod cli_error_display_tests {
    use super::CliError;
    use std::error::Error as _;

    #[test]
    fn every_variant_renders_its_text_and_chains_no_source() {
        let cases = [
            (
                CliError::Io(std::io::Error::other("disk gone")),
                "i/o error: disk gone",
            ),
            (
                CliError::Transmux(crate::Error::BoxSizeUnderflow {
                    size: 4,
                    header_size: 8,
                }),
                "transmux error: box size 4 is smaller than header (8 bytes)",
            ),
            (
                CliError::UnknownContainer,
                "unknown input container: leading bytes match no supported format (MPEG-TS, MP4/CMAF, MPEG-PS, WebM, FLV)",
            ),
            (
                CliError::UnsupportedContainer("MXF"),
                "unsupported input container MXF: this CLI demuxes MPEG-TS, MP4/CMAF, MPEG-PS, WebM/Matroska and FLV",
            ),
            (
                CliError::UtcTimingNeedsLl,
                "--utc-timing-url needs --ll (MPD/UTCTiming is emitted only for the low-latency DASH profile)",
            ),
            (
                CliError::UndeterminedFormat,
                "output format not given and not inferable from the output extension; pass -f/--format",
            ),
            (
                CliError::NoTracksSelected,
                "the --tracks selection matched no tracks in the input",
            ),
            (
                CliError::BadKey("k".to_string()),
                "invalid --key (k): expected <32-hex-KID>:<32-hex-key>",
            ),
            (
                CliError::ProgressiveNeedsSeekableOutput,
                "progressive MP4 output must be a seekable file, not standard output",
            ),
            (
                CliError::ManifestToStdout,
                "an HLS/DASH manifest needs a file output path; its segments are written beside it",
            ),
            (
                CliError::InterleaveBufferFull { limit: 100 },
                "streaming interleave buffered more than 100 bytes of samples",
            ),
            (
                CliError::WholeInputTooLarge { limit: 50 },
                "input needs a whole-file read larger than 50 bytes (--max-whole-input-bytes)",
            ),
            (
                CliError::BadDataIdentity("4=ts".to_string()),
                "invalid --data-identity (4=ts): expected <track>=ts:<stream_type>[:<descriptors-hex>] | mp4:<handler>:<sample-entry-hex> | matroska:<codec_id>[:<codec-private-hex>]",
            ),
        ];
        for (error, text) in cases {
            assert_eq!(error.to_string(), text);
            assert!(error.source().is_none(), "{text}");
        }
    }
}

#[cfg(test)]
mod availability_start_time_tests {
    use super::availability_start_time;

    #[test]
    fn formats_whole_seconds_utc_with_a_z_suffix() {
        assert_eq!(availability_start_time(0), "1970-01-01T00:00:00Z");
        assert_eq!(availability_start_time(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(
            availability_start_time(1_700_000_000),
            "2023-11-14T22:13:20Z"
        );
        assert_eq!(
            availability_start_time(4_107_542_400),
            "2100-03-01T00:00:00Z"
        );
        assert_eq!(availability_start_time(-1), "1969-12-31T23:59:59Z");
    }

    #[test]
    fn out_of_range_seconds_clamp_to_the_epoch_not_a_panic() {
        assert_eq!(availability_start_time(i64::MAX), "1970-01-01T00:00:00Z");
    }
}

// ---------------------------------------------------------------------------
// Detected container + output format
// ---------------------------------------------------------------------------

/// The input container as recognised by [`detect_container`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Container {
    /// MPEG-2 Transport Stream → [`StreamingTsDemux`].
    MpegTs,
    /// ISO-BMFF fragmented MP4 / CMAF → [`Mp4Demux`].
    Mp4,
    /// MPEG Program Stream → [`StreamingPsDemux`].
    MpegPs,
    /// WebM / Matroska (EBML) → [`WebmDemux`].
    WebM,
    /// FLV (Flash Video) → [`StreamingFlvDemux`].
    Flv,
}

impl Container {
    /// Spec/label token for the container, per the #204 label convention.
    pub fn name(&self) -> &'static str {
        match self {
            Container::MpegTs => "mpeg-ts",
            Container::Mp4 => "mp4",
            Container::MpegPs => "mpeg-ps",
            Container::WebM => "webm",
            Container::Flv => "flv",
        }
    }
}

broadcast_common::impl_spec_display!(Container);

/// The output packaging format selected by `-f/--format` (or inferred from the
/// output path extension).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum OutputFormat {
    /// CMAF/fMP4 single init+media segment → [`CmafMuxStage`].
    Cmaf,
    /// CMAF-HLS media playlist (`.m3u8`) → CMAF segments via [`Segmenter`].
    Hls,
    /// Classic HLS with MPEG-2 TS media segments →
    /// [`StreamingTsHlsSegmenter`].
    TsHls,
    /// DASH MPD manifest → [`DashPackager`].
    Dash,
    /// MPEG-2 Transport Stream → [`TsMuxStage`].
    Ts,
    /// Progressive single-file MP4 → [`ProgressiveMuxStage`].
    Progressive,
}

impl OutputFormat {
    /// Spec/label token for the format, per the #204 label convention.
    pub fn name(&self) -> &'static str {
        match self {
            OutputFormat::Cmaf => "cmaf",
            OutputFormat::Hls => "hls",
            OutputFormat::TsHls => "ts-hls",
            OutputFormat::Dash => "dash",
            OutputFormat::Ts => "ts",
            OutputFormat::Progressive => "progressive",
        }
    }

    /// Infer an output format from a file-name extension. Returns `None` for
    /// extensions that do not map to exactly one format.
    fn from_extension(ext: &str) -> Option<Self> {
        // Match on the lowercased extension. `.cmaf`/`.m4s`/`.mp4` are ambiguous
        // between several fMP4 flavours, so we pick the most specific mapping:
        // `.m3u8` is HLS, `.mpd` is DASH, `.ts` is a raw TS mux, `.cmaf` is CMAF,
        // and a plain `.mp4` is a progressive single file.
        match ext.to_ascii_lowercase().as_str() {
            "m3u8" => Some(OutputFormat::Hls),
            "mpd" => Some(OutputFormat::Dash),
            "ts" => Some(OutputFormat::Ts),
            "cmaf" | "m4s" => Some(OutputFormat::Cmaf),
            "mp4" | "m4v" => Some(OutputFormat::Progressive),
            _ => None,
        }
    }
}

broadcast_common::impl_spec_display!(OutputFormat);

// ---------------------------------------------------------------------------
// clap Args (derive)
// ---------------------------------------------------------------------------

/// An any-to-any media container packager: autodetect the input container, run
/// it through the neutral hub IR, and write the chosen output format.
#[derive(clap::Parser)]
#[command(name = "transmux", version, about, long_about = None)]
pub struct Args {
    /// Input media file (the container is autodetected from its leading bytes).
    /// May be given positionally or with `-i/--input`; `-` reads standard
    /// input, which cannot be the source of the two-pass progressive MP4.
    #[arg(value_name = "IN", required_unless_present = "input")]
    pub in_positional: Option<PathBuf>,

    /// Input media file (alternative to the positional `<IN>`).
    #[arg(
        short = 'i',
        long = "input",
        value_name = "PATH",
        conflicts_with = "in_positional"
    )]
    pub input: Option<PathBuf>,

    /// Output path (a file for CMAF/TS/progressive, or the playlist/manifest
    /// path for HLS/DASH).
    #[arg(short = 'o', long = "output", value_name = "PATH")]
    pub output: PathBuf,

    /// Output format. If omitted, it is inferred from the output path extension
    /// (`.m3u8`→hls, `.mpd`→dash, `.ts`→ts, `.cmaf`→cmaf, `.mp4`→progressive).
    #[arg(short = 'f', long = "format", value_enum)]
    pub format: Option<FormatArg>,

    /// Target media-segment duration in seconds (HLS/DASH/CMAF segmentation).
    #[arg(long = "segment-duration", value_name = "SECS", default_value_t = 6)]
    pub segment_duration: u32,

    /// Emit a low-latency variant where the selected format supports it
    /// (currently LL-DASH: chunked `SegmentTemplate` with an
    /// `availabilityTimeOffset`). Ignored by formats without a low-latency mode.
    #[arg(long = "ll")]
    pub ll: bool,

    /// DASH only: advertise `MPD/UTCTiming` (ISO/IEC 23009-1 §5.8.4.11) whose
    /// `@value` is this URL, read for wall-clock UTC via its HTTP `Date:`
    /// header. Omitted by default, so an offline artefact names no time source.
    #[arg(long = "utc-timing-url", value_name = "URL")]
    pub utc_timing_url: Option<String>,

    /// Restrict the output to these track IDs (comma-separated, e.g.
    /// `--tracks 1,2`). Default: all tracks.
    #[arg(long = "tracks", value_name = "IDS", value_delimiter = ',')]
    pub tracks: Vec<u32>,

    /// Decrypt CENC-protected input before packaging (requires the `cenc`
    /// feature). Supply content keys with repeated `--key <kid-hex>:<key-hex>`.
    #[cfg(feature = "cenc")]
    #[arg(long = "decrypt")]
    pub decrypt: bool,

    /// A CENC content key as `<16-byte-KID-hex>:<16-byte-key-hex>` (repeatable).
    /// Only meaningful with `--decrypt`.
    #[cfg(feature = "cenc")]
    #[arg(long = "key", value_name = "KID:KEY")]
    pub keys: Vec<String>,

    /// Cap, in bytes, on the whole-file read of an input this CLI cannot stream
    /// yet (MPEG-PS, WebM/Matroska). No default: unlimited unless given; over
    /// the cap the run fails with a typed error naming it.
    #[arg(long = "max-whole-input-bytes", value_name = "BYTES")]
    pub max_whole_input_bytes: Option<u64>,

    /// Map a data track's container identity for the output (repeatable):
    /// `ts:<stream_type>[:<descriptors-hex>]`,
    /// `mp4:<handler>:<sample-entry-hex>` or
    /// `matroska:<codec_id>[:<codec-private-hex>]`. Without a mapping a data
    /// track keeps its own identity and a muxer of another family refuses it.
    #[arg(long = "data-identity", value_name = "TRACK=SPEC")]
    pub data_identity: Vec<String>,
}

/// Hand-written: a derived `Debug` would print `keys` (raw `<KID>:<key>`
/// strings) verbatim, so a `dbg!`/panic message of `Args` would write
/// content keys to logs. Every other field is printed as-is; `keys` is
/// printed with each entry redacted down to its KID half via
/// `redact_key_spec` (a KID is not secret, the key half is).
impl fmt::Debug for Args {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut d = f.debug_struct("Args");
        d.field("in_positional", &self.in_positional)
            .field("input", &self.input)
            .field("output", &self.output)
            .field("format", &self.format)
            .field("segment_duration", &self.segment_duration)
            .field("ll", &self.ll)
            .field("tracks", &self.tracks)
            .field("max_whole_input_bytes", &self.max_whole_input_bytes)
            .field("data_identity", &self.data_identity);
        #[cfg(feature = "cenc")]
        {
            d.field("decrypt", &self.decrypt);
            let redacted_keys: Vec<String> = self.keys.iter().map(|s| redact_key_spec(s)).collect();
            d.field("keys", &redacted_keys);
        }
        d.finish()
    }
}

/// Redact a raw `--key` argument down to its (non-secret) KID half:
/// `<kid-hex>:<redacted>` when the argument's first `:`-separated field
/// parses as 32 hex characters, or just the argument's length otherwise.
/// Never returns the key half, and never the original string verbatim —
/// even a malformed argument (bad KID half) could still carry a decodable
/// key half after the `:`.
#[cfg(feature = "cenc")]
fn redact_key_spec(spec: &str) -> String {
    let kid_part = spec.split_once(':').map(|(k, _)| k).unwrap_or(spec);
    let is_kid_hex =
        kid_part.len() == KEY_ID_HEX_LEN && kid_part.bytes().all(|b| b.is_ascii_hexdigit());
    if is_kid_hex {
        format!("{kid_part}:<redacted>")
    } else {
        format!("<{}-byte argument>", spec.len())
    }
}

/// clap `ValueEnum` mirror of [`OutputFormat`] (kebab-case flag values).
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
#[non_exhaustive]
pub enum FormatArg {
    /// CMAF/fMP4 single init+media segment.
    Cmaf,
    /// CMAF-HLS media playlist.
    Hls,
    /// Classic HLS with MPEG-2 TS segments.
    #[value(name = "ts-hls")]
    TsHls,
    /// DASH MPD manifest.
    Dash,
    /// MPEG-2 Transport Stream.
    Ts,
    /// Progressive single-file MP4.
    Progressive,
}

impl From<FormatArg> for OutputFormat {
    fn from(a: FormatArg) -> Self {
        match a {
            FormatArg::Cmaf => OutputFormat::Cmaf,
            FormatArg::Hls => OutputFormat::Hls,
            FormatArg::TsHls => OutputFormat::TsHls,
            FormatArg::Dash => OutputFormat::Dash,
            FormatArg::Ts => OutputFormat::Ts,
            FormatArg::Progressive => OutputFormat::Progressive,
        }
    }
}

// ---------------------------------------------------------------------------
// CLI error type (std-only; carries dynamic context the library Error can't)
// ---------------------------------------------------------------------------

/// Errors surfaced by the CLI front-end. Wraps I/O, the library
/// [`Error`](crate::Error), and CLI-specific conditions (unknown container,
/// missing format, bad key). Never panics on bad input — always returns `Err`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CliError {
    /// Reading the input or writing the output failed.
    #[error("i/o error: {0}")]
    Io(std::io::Error),
    /// A demux or mux spoke rejected the media.
    #[error("transmux error: {0}")]
    Transmux(crate::Error),
    /// The input container could not be recognised from its leading bytes.
    #[error(
        "unknown input container: leading bytes match no supported format (MPEG-TS, MP4/CMAF, MPEG-PS, WebM, FLV)"
    )]
    UnknownContainer,
    /// The prober identified a container this CLI has no demuxer for.
    #[error(
        "unsupported input container {0}: this CLI demuxes MPEG-TS, MP4/CMAF, MPEG-PS, WebM/Matroska and FLV"
    )]
    UnsupportedContainer(&'static str),
    /// `--utc-timing-url` was given without `--ll`. `MPD/UTCTiming` is modelled
    /// only on the low-latency packager, so the flag would otherwise be dropped
    /// silently.
    #[error(
        "--utc-timing-url needs --ll (MPD/UTCTiming is emitted only for the low-latency DASH profile)"
    )]
    UtcTimingNeedsLl,
    /// No `-f/--format` was given and none could be inferred from the output
    /// path extension.
    #[error(
        "output format not given and not inferable from the output extension; pass -f/--format"
    )]
    UndeterminedFormat,
    /// The requested track-ID selection left no tracks.
    #[error("the --tracks selection matched no tracks in the input")]
    NoTracksSelected,
    /// A `--key` argument was malformed. Carries `redact_key_spec`'s
    /// output, never the raw argument — the key half must never reach an
    /// error message, `Display`, or (this enum derives `Debug`) a `dbg!`.
    #[error("invalid --key ({0}): expected <32-hex-KID>:<32-hex-key>")]
    BadKey(String),
    /// Progressive MP4 writes at offsets fixed by its sample plan, so its
    /// output must be a seekable file; standard output is not.
    #[error("progressive MP4 output must be a seekable file, not standard output")]
    ProgressiveNeedsSeekableOutput,
    /// A manifest's media segments are files written beside the manifest, so
    /// the manifest itself cannot go to standard output.
    #[error("an HLS/DASH manifest needs a file output path; its segments are written beside it")]
    ManifestToStdout,
    /// The decode-order interleave held more sample bytes than its caller set
    /// before it could release them.
    #[error("streaming interleave buffered more than {limit} bytes of samples")]
    InterleaveBufferFull {
        /// The cap, in bytes.
        limit: u64,
    },
    /// A whole-file read of an unstreamable container passed the caller's cap.
    #[error("input needs a whole-file read larger than {limit} bytes (--max-whole-input-bytes)")]
    WholeInputTooLarge {
        /// The cap, in bytes.
        limit: u64,
    },
    /// A `--data-identity` argument was malformed or named a bad track.
    #[error(
        "invalid --data-identity ({0}): expected <track>=ts:<stream_type>[:<descriptors-hex>] | mp4:<handler>:<sample-entry-hex> | matroska:<codec_id>[:<codec-private-hex>]"
    )]
    BadDataIdentity(String),
}

impl From<std::io::Error> for CliError {
    fn from(e: std::io::Error) -> Self {
        CliError::Io(e)
    }
}

impl From<crate::Error> for CliError {
    fn from(e: crate::Error) -> Self {
        CliError::Transmux(e)
    }
}

/// CLI result alias.
pub type CliResult<T> = Result<T, CliError>;

// ---------------------------------------------------------------------------
// Autodetect
// ---------------------------------------------------------------------------

/// Recognise the input container from its leading bytes, via
/// [`container_probe::probe`] (issue #960).
///
/// Every format this CLI can demux maps to exactly one
/// [`container_probe::Format`]; anything the prober identifies but this CLI has
/// no demuxer for (MXF, WAV, Ogg, ADTS, MP3, Annex B) is reported as
/// [`CliError::UnsupportedContainer`], and an inconclusive probe (the
/// `Insufficient`/`Ambiguous`/`Unknown` outcomes) as
/// [`CliError::UnknownContainer`] — never a guess.
///
/// # Errors
/// [`CliError::UnsupportedContainer`] / [`CliError::UnknownContainer`].
pub fn detect_container(data: &[u8]) -> CliResult<Container> {
    let format = match container_probe::probe(data) {
        container_probe::Probe::Identified { format, .. } => format,
        // A tie or an inconclusive read is not a decision: the caller (or a
        // user) must supply the format explicitly.
        _ => return Err(CliError::UnknownContainer),
    };
    container_from_format(format)
}

/// Map a probed format to the demuxer this CLI drives for it.
fn container_from_format(format: container_probe::Format) -> CliResult<Container> {
    use container_probe::Format;
    match format {
        Format::MpegTs => Ok(Container::MpegTs),
        Format::Isobmff => Ok(Container::Mp4),
        Format::MpegPs => Ok(Container::MpegPs),
        Format::WebM => Ok(Container::WebM),
        Format::Flv => Ok(Container::Flv),
        // Matroska is demuxable by the WebM demuxer (both are EBML; WebM is a
        // Matroska profile), so it maps to the same spoke.
        Format::Matroska => Ok(Container::WebM),
        other => Err(CliError::UnsupportedContainer(other.name())),
    }
}

// ---------------------------------------------------------------------------
// Core: streamed input → demux stage → decode-order interleave → mux stage
// ---------------------------------------------------------------------------

/// Default movie (`mvhd`) timescale for a container that does not declare one.
const DEFAULT_MOVIE_TIMESCALE: u32 = 1_000;
/// Bytes read from an input per feed.
const READ_CHUNK_BYTES: usize = 256 * 1024;
/// Largest amount of sample data the decode-order interleave holds before it
/// refuses. Caller policy: a real multiplex interleaves its tracks within a
/// few frames, so this only trips on a pathological source.
const MAX_INTERLEAVE_BYTES: u64 = 16 * 1024 * 1024;
/// Sample bytes buffered from a forward-only FLV before its track set is taken
/// as settled (FLV declares no track set up front).
const FLV_TRACK_SETTLE_BYTES: u64 = 1024 * 1024;

/// The packaged output of a run: raw bytes for binary formats, or (for HLS/DASH)
/// a text manifest plus the referenced media segments.
#[derive(Debug)]
#[non_exhaustive]
pub enum Output {
    /// A single binary artifact (CMAF, TS, progressive MP4).
    Bytes(Vec<u8>),
    /// A text manifest/playlist plus its referenced media segments
    /// (`(filename, bytes)`). The manifest is written to the `-o` path; each
    /// segment is written alongside it under its own name.
    Manifest {
        /// The playlist / MPD text.
        text: String,
        /// The referenced media segments as `(file_name, bytes)`.
        segments: Vec<(String, Vec<u8>)>,
    },
}

/// Options for [`run_bytes`] (the testable core, decoupled from clap and I/O).
#[derive(Debug, Clone)]
pub struct Opts {
    /// The output packaging format.
    pub format: OutputFormat,
    /// Target segment duration in seconds.
    pub segment_duration: u32,
    /// Low-latency mode where supported.
    pub low_latency: bool,
    /// `MPD/UTCTiming@value` for DASH output (ISO/IEC 23009-1 §5.8.4.11): the
    /// URL whose HTTP `Date` header is the wall-clock source. `None` **omits**
    /// the element — the default, because this CLI writes static files and must
    /// not bake a third-party time server into an offline artefact.
    pub utc_timing_url: Option<String>,
    /// Track-ID selection; empty = all tracks.
    pub tracks: Vec<TrackId>,
    /// Cap on a whole-file read of an unstreamable input (MPEG-PS/WebM).
    pub max_whole_input_bytes: Option<u64>,
    /// Data-track identity mappings applied before the muxer.
    pub data_identity: DataIdentityMap,
}

impl Default for Opts {
    fn default() -> Self {
        Self {
            format: OutputFormat::Cmaf,
            segment_duration: DEFAULT_SEGMENT_DURATION_SECS,
            low_latency: false,
            utc_timing_url: None,
            tracks: Vec::new(),
            max_whole_input_bytes: None,
            data_identity: DataIdentityMap::new(),
        }
    }
}

/// Where a run's `MuxOutput` goes as it arrives.
trait Emit {
    /// Append to the single binary output stream.
    fn append(&mut self, bytes: &[u8]) -> CliResult<()>;
    /// Write at an absolute byte offset (progressive MP4 only).
    fn patch(&mut self, offset: u64, bytes: &[u8]) -> CliResult<()>;
    /// Write a named file beside the main output (a segment or init file).
    fn side(&mut self, name: &str, bytes: &[u8]) -> CliResult<()>;
    /// Write the run's text manifest.
    fn text(&mut self, text: &str) -> CliResult<()>;
    /// Whether [`Emit::patch`] can work here.
    fn seekable(&self) -> bool;
}

/// An in-memory [`Emit`] for [`run_bytes`].
#[derive(Default)]
struct MemoryEmit {
    main: Vec<u8>,
    side: Vec<(String, Vec<u8>)>,
    text: Option<String>,
}

impl MemoryEmit {
    fn output(self) -> Output {
        match self.text {
            Some(text) => Output::Manifest {
                text,
                segments: self.side,
            },
            None => Output::Bytes(self.main),
        }
    }
}

impl Emit for MemoryEmit {
    fn append(&mut self, bytes: &[u8]) -> CliResult<()> {
        self.main.extend_from_slice(bytes);
        Ok(())
    }

    fn patch(&mut self, offset: u64, bytes: &[u8]) -> CliResult<()> {
        let start = usize::try_from(offset).map_err(|_| {
            crate::Error::InvalidInput("progressive: offset exceeds the address space")
        })?;
        let end = start
            .checked_add(bytes.len())
            .ok_or(crate::Error::InvalidInput(
                "progressive: write overflows the address space",
            ))?;
        if self.main.len() < end {
            self.main.resize(end, 0);
        }
        self.main
            .get_mut(start..end)
            .ok_or(crate::Error::InvalidInput(
                "progressive: write outside the file",
            ))?
            .copy_from_slice(bytes);
        Ok(())
    }

    fn side(&mut self, name: &str, bytes: &[u8]) -> CliResult<()> {
        self.side.push((name.to_string(), bytes.to_vec()));
        Ok(())
    }

    fn text(&mut self, text: &str) -> CliResult<()> {
        self.text = Some(text.to_string());
        Ok(())
    }

    fn seekable(&self) -> bool {
        true
    }
}

/// A file [`Emit`]: the main artifact at `main_path`, side files in its parent
/// directory.
struct FileEmit {
    main_path: PathBuf,
    dir: PathBuf,
    main: Option<File>,
}

impl FileEmit {
    fn new(path: &Path) -> Self {
        let dir = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        Self {
            main_path: path.to_path_buf(),
            dir,
            main: None,
        }
    }

    fn ensure_dir(&self) -> CliResult<()> {
        std::fs::create_dir_all(&self.dir)?;
        Ok(())
    }

    fn main(&mut self) -> CliResult<&mut File> {
        if self.main.is_none() {
            self.ensure_dir()?;
            self.main = Some(File::create(&self.main_path)?);
        }
        self.main
            .as_mut()
            .ok_or_else(|| CliError::Io(std::io::Error::other("output file not opened")))
    }
}

impl Emit for FileEmit {
    fn append(&mut self, bytes: &[u8]) -> CliResult<()> {
        self.main()?.write_all(bytes)?;
        Ok(())
    }

    fn patch(&mut self, offset: u64, bytes: &[u8]) -> CliResult<()> {
        let file = self.main()?;
        file.seek(SeekFrom::Start(offset))?;
        file.write_all(bytes)?;
        Ok(())
    }

    fn side(&mut self, name: &str, bytes: &[u8]) -> CliResult<()> {
        self.ensure_dir()?;
        std::fs::write(self.dir.join(name), bytes)?;
        Ok(())
    }

    fn text(&mut self, text: &str) -> CliResult<()> {
        self.ensure_dir()?;
        std::fs::write(&self.main_path, text)?;
        Ok(())
    }

    fn seekable(&self) -> bool {
        true
    }
}

/// Standard-output [`Emit`]: appends only.
struct StdoutEmit {
    out: std::io::Stdout,
}

impl StdoutEmit {
    fn new() -> Self {
        Self {
            out: std::io::stdout(),
        }
    }
}

impl Emit for StdoutEmit {
    fn append(&mut self, bytes: &[u8]) -> CliResult<()> {
        self.out.write_all(bytes)?;
        self.out.flush()?;
        Ok(())
    }

    fn patch(&mut self, _offset: u64, _bytes: &[u8]) -> CliResult<()> {
        Err(CliError::ProgressiveNeedsSeekableOutput)
    }

    fn side(&mut self, _name: &str, _bytes: &[u8]) -> CliResult<()> {
        Err(CliError::ManifestToStdout)
    }

    fn text(&mut self, _text: &str) -> CliResult<()> {
        Err(CliError::ManifestToStdout)
    }

    fn seekable(&self) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// Decode-order interleave: the one place samples of several tracks are merged
// by decode time, reproducing `mux_stage::drive_media`'s order for a stream
// whose tracks arrive in source order.
// ---------------------------------------------------------------------------

/// One track's queued samples and running decode time.
struct ReorderTrack {
    track: TrackId,
    timescale: Timescale,
    /// `(decode key in this track's timescale, sample)`, arrival order.
    queue: std::collections::VecDeque<(i64, Sample)>,
    /// The same samples ordered by decode time, for a format whose reference
    /// output was a whole-input sort (a demux may hand a track a sample out of
    /// decode order, and the batch path sorted it back).
    sorted: std::collections::BTreeMap<(i64, u64), Sample>,
    /// Insertion counter that keeps equal decode times in arrival order.
    seq: u64,
    /// Samples of a track's untimed head, held until the first timed sample
    /// fixes where the head plays (see [`crate::mux_stage`]).
    head: Vec<Sample>,
    /// Running decode time for a sample with no `dts`.
    running: i64,
    seen_timed: bool,
    buffered: u64,
}

/// A k-way merge by decode time with a watermark: a queued sample is released
/// once no not-yet-seen sample of another track could precede it.
#[derive(Default)]
struct Reorder {
    tracks: Vec<ReorderTrack>,
    buffered: u64,
    /// Sort each track by decode time before release (HLS/DASH, whose batch
    /// path merged a sorted sample list).
    sort_within_track: bool,
}

impl Reorder {
    fn new(specs: &[TrackSpec], sort_within_track: bool) -> Self {
        Self {
            tracks: specs
                .iter()
                .map(|spec| ReorderTrack {
                    track: spec.track_id,
                    timescale: spec.timescale,
                    queue: std::collections::VecDeque::new(),
                    sorted: std::collections::BTreeMap::new(),
                    seq: 0,
                    head: Vec::new(),
                    running: 0,
                    seen_timed: false,
                    buffered: 0,
                })
                .collect(),
            buffered: 0,
            sort_within_track,
        }
    }

    fn enqueue(sort: bool, state: &mut ReorderTrack, key: i64, sample: Sample) {
        if sort {
            state.sorted.insert((key, state.seq), sample);
            state.seq = state.seq.saturating_add(1);
        } else {
            state.queue.push_back((key, sample));
        }
    }

    fn front_key(state: &ReorderTrack) -> Option<i64> {
        if let Some((&(key, _), _)) = state.sorted.first_key_value() {
            return Some(key);
        }
        state.queue.front().map(|(key, _)| *key)
    }

    fn pop(state: &mut ReorderTrack, sort: bool) -> Option<Sample> {
        if sort {
            state.sorted.pop_first().map(|(_, sample)| sample)
        } else {
            state.queue.pop_front().map(|(_, sample)| sample)
        }
    }

    fn push(&mut self, track: TrackId, sample: Sample) -> CliResult<()> {
        let sort = self.sort_within_track;
        let Some(state) = self.tracks.iter_mut().find(|state| state.track == track) else {
            return Ok(());
        };
        let bytes = sample.data.len() as u64;
        let duration = sample
            .timing
            .and_then(|timing| timing.duration())
            .map_or(0, i64::from);
        if let Some(dts) = sample.timing.and_then(|timing| timing.dts()) {
            if !state.seen_timed {
                state.seen_timed = true;
                let mut key = dts.saturating_sub(state.head.iter().fold(0i64, |sum, head| {
                    sum.saturating_add(
                        head.timing
                            .and_then(|timing| timing.duration())
                            .map_or(0, i64::from),
                    )
                }));
                for head in core::mem::take(&mut state.head) {
                    let head_duration = head
                        .timing
                        .and_then(|timing| timing.duration())
                        .map_or(0, i64::from);
                    Self::enqueue(sort, state, key, head);
                    key = key.saturating_add(head_duration);
                }
            }
            state.running = dts.saturating_add(duration);
            Self::enqueue(sort, state, dts, sample);
        } else if sample.timing.is_none() {
            // A section-carried sample has no timestamp at all; the batch path
            // places it at the track's running decode time, so release it now.
            let key = state.running;
            Self::enqueue(sort, state, key, sample);
        } else if state.seen_timed {
            let key = state.running;
            state.running = state.running.saturating_add(duration);
            Self::enqueue(sort, state, key, sample);
        } else {
            state.head.push(sample);
        }
        state.buffered = state.buffered.saturating_add(bytes);
        self.buffered = self.buffered.saturating_add(bytes);
        if self.buffered > MAX_INTERLEAVE_BYTES {
            return Err(CliError::InterleaveBufferFull {
                limit: MAX_INTERLEAVE_BYTES,
            });
        }
        Ok(())
    }

    /// The queued sample with the smallest decode time, ties to the earlier
    /// announcement order (the one `drive_media` picks).
    fn earliest(&self) -> Option<(usize, i64)> {
        let mut best: Option<(usize, i64, Timescale)> = None;
        for (index, state) in self.tracks.iter().enumerate() {
            let Some(key) = Self::front_key(state) else {
                continue;
            };
            let earlier = best.is_none_or(|(_, best_key, best_scale)| {
                cmp_ticks(key, state.timescale, best_key, best_scale).is_lt()
            });
            if earlier {
                best = Some((index, key, state.timescale));
            }
        }
        best.map(|(index, key, _)| (index, key))
    }

    /// Release samples in decode order while every track that is still
    /// producing has a sample queued: the same k-way merge `drive_media`
    /// performs. A track whose queue is empty is waited for (one sample of
    /// lookahead is all the merge needs); at end of input every remaining
    /// sample is released.
    fn flush(
        &mut self,
        final_input: bool,
        feed: &mut dyn FnMut(TrackId, Sample) -> CliResult<()>,
    ) -> CliResult<()> {
        let sort = self.sort_within_track;
        if final_input {
            for state in &mut self.tracks {
                let mut key = state.running;
                for head in core::mem::take(&mut state.head) {
                    let duration = head
                        .timing
                        .and_then(|timing| timing.duration())
                        .map_or(0, i64::from);
                    Self::enqueue(sort, state, key, head);
                    key = key.saturating_add(duration);
                }
            }
        }
        loop {
            if !final_input
                && self
                    .tracks
                    .iter()
                    .any(|state| Self::front_key(state).is_none())
            {
                return Ok(());
            }
            let Some((index, _)) = self.earliest() else {
                return Ok(());
            };
            let state = &mut self.tracks[index];
            let Some(sample) = Self::pop(state, sort) else {
                return Ok(());
            };
            let bytes = sample.data.len() as u64;
            state.buffered = state.buffered.saturating_sub(bytes);
            self.buffered = self.buffered.saturating_sub(bytes);
            feed(state.track, sample)?;
        }
    }
}

// ---------------------------------------------------------------------------
// The mux stage and its output sinks
// ---------------------------------------------------------------------------

enum Kind {
    Cmaf(CmafMuxStage),
    Ts(TsMuxStage),
    Hls {
        segmenter: Segmenter,
        measured: Vec<f64>,
    },
    TsHls(StreamingTsHlsSegmenter),
    Dash(Box<DashStream>),
    Progressive(ProgressiveMuxStage),
}

/// Content keys for `--decrypt`. Without the `cenc` feature the CLI has no
/// `--decrypt` flag, so this is an uninhabited `Option` and the parameter
/// threads through unused (`CliStream::start` ignores it).
#[cfg(feature = "cenc")]
type DecryptKeys = Option<crate::cenc_decrypt::KeyMap>;
#[cfg(not(feature = "cenc"))]
type DecryptKeys = Option<core::convert::Infallible>;

struct CliStream {
    kind: Kind,
    reorder: Reorder,
    specs: Vec<TrackSpec>,
    opts: Opts,
    /// The `--decrypt` stage, standing between the demux events and the
    /// interleave/mux path. `None` when the caller did not ask to decrypt.
    #[cfg(feature = "cenc")]
    decrypt: Option<crate::cenc_decrypt::CencDecryptStage>,
}

impl CliStream {
    fn start(
        specs: Vec<TrackSpec>,
        movie_timescale: Timescale,
        opts: &Opts,
        emit: &mut dyn Emit,
        plan: Option<isobmff::plan::OwnedPlan>,
        keys: DecryptKeys,
    ) -> CliResult<Self> {
        let specs = apply_identity(&specs, &opts.data_identity)?;
        #[cfg(feature = "cenc")]
        let (decrypt, specs) = match keys {
            Some(keys) => {
                let mut stage = crate::cenc_decrypt::CencDecryptStage::new(keys);
                for spec in &specs {
                    stage.feed(MuxInput::Track(Box::new(spec.clone())), Timestamp::ZERO)?;
                }
                let clear = specs
                    .into_iter()
                    .map(|mut spec| {
                        spec.protection = None;
                        spec
                    })
                    .collect();
                (Some(stage), clear)
            }
            None => (None, specs),
        };
        #[cfg(not(feature = "cenc"))]
        let _ = keys;
        #[cfg(not(feature = "cenc"))]
        let specs = specs;
        let kind = match opts.format {
            OutputFormat::Cmaf => {
                let target = Duration::from_secs(u64::from(opts.segment_duration.max(1)));
                let cut = match choose_anchor(specs.iter().map(|spec| &spec.config)) {
                    Ok(index) => FragmentCut::AnchorKeyframe {
                        track: specs[index].track_id,
                        target,
                    },
                    Err(_) => FragmentCut::OnFlush,
                };
                let mut stage = CmafMuxStage::new(movie_timescale, cut, 1);
                for spec in &specs {
                    stage.feed(MuxInput::Track(Box::new(spec.clone())), Timestamp::ZERO)?;
                }
                Kind::Cmaf(stage)
            }
            OutputFormat::Ts => {
                let mut stage = TsMuxStage::new(Some(DEFAULT_PSI_INTERVAL));
                for spec in &specs {
                    stage.feed(MuxInput::Track(Box::new(spec.clone())), Timestamp::ZERO)?;
                }
                Kind::Ts(stage)
            }
            OutputFormat::Hls => {
                let target = Duration::from_secs(u64::from(opts.segment_duration.max(1)));
                let segmenter = Segmenter::new(specs.clone(), movie_timescale, target)?;
                emit.side(HLS_INIT_FILE, &segmenter.init_segment()?)?;
                Kind::Hls {
                    segmenter,
                    measured: Vec::new(),
                }
            }
            OutputFormat::TsHls => Kind::TsHls(StreamingTsHlsSegmenter::new(
                specs.clone(),
                opts.segment_duration,
                usize::MAX,
            )?),
            OutputFormat::Dash => Kind::Dash(Box::new(DashStream::new(
                specs.clone(),
                movie_timescale,
                opts,
            )?)),
            OutputFormat::Progressive => {
                let plan = plan.ok_or(crate::Error::ProgressiveNeedsReplayableInput)?;
                let layout = isobmff::writer::Layout::new(
                    true,
                    isobmff::writer::Interleave::ByDuration {
                        millis: PROGRESSIVE_INTERLEAVE_MILLIS,
                    },
                    isobmff::writer::Sink::Seekable,
                );
                let movie = isobmff::writer::MovieHeader::new(movie_timescale);
                let mut stage = ProgressiveMuxStage::new(plan, layout, movie);
                for spec in &specs {
                    stage.feed(MuxInput::Track(Box::new(spec.clone())), Timestamp::ZERO)?;
                }
                Kind::Progressive(stage)
            }
        };
        Ok(Self {
            kind,
            reorder: Reorder::new(&specs, matches!(opts.format, OutputFormat::Hls)),
            specs,
            opts: opts.clone(),
            #[cfg(feature = "cenc")]
            decrypt,
        })
    }

    fn event(&mut self, event: DemuxEvent, emit: &mut dyn Emit) -> CliResult<()> {
        match event {
            DemuxEvent::Sample { track_id, sample } => self.sample(track_id, sample, emit),
            DemuxEvent::TrackAdded(spec) | DemuxEvent::TrackUpdated(spec) => self.announce(spec),
            // A whole-`Media` drive never carried a discontinuity, so the CLI
            // ignores one here too: forwarding it would change a byte-exact
            // segmenter/TS muxer output against the established goldens.
            _ => Ok(()),
        }
    }

    /// Feed a track's changed `tenc` (or any other spec update) to the decrypt
    /// stage, whose held samples then become decipherable. A track the caller
    /// filtered out is dropped for the same reason [`Self::sample`] drops its
    /// samples: the muxers never see that track either.
    #[cfg(feature = "cenc")]
    fn announce(&mut self, spec: TrackSpec) -> CliResult<()> {
        let Some(stage) = self.decrypt.as_mut() else {
            return Ok(());
        };
        if !self.specs.iter().any(|kept| kept.track_id == spec.track_id) {
            return Ok(());
        }
        stage.feed(MuxInput::Track(Box::new(spec)), Timestamp::ZERO)?;
        self.drain_decrypt(None)
    }

    #[cfg(not(feature = "cenc"))]
    fn announce(&mut self, _spec: TrackSpec) -> CliResult<()> {
        Ok(())
    }

    fn sample(&mut self, track: TrackId, sample: Sample, emit: &mut dyn Emit) -> CliResult<()> {
        #[cfg(feature = "cenc")]
        if let Some(stage) = self.decrypt.as_mut() {
            if !self.specs.iter().any(|kept| kept.track_id == track) {
                return Ok(());
            }
            stage.feed(MuxInput::Sample { track, sample }, Timestamp::ZERO)?;
            return self.drain_decrypt(Some(emit));
        }
        self.reorder.push(track, sample)?;
        self.drain_order(false, emit)
    }

    /// Collect the decrypt stage's output and interleave the samples it
    /// released (a late `tenc` can release a whole held run at once).
    #[cfg(feature = "cenc")]
    fn drain_decrypt(&mut self, mut emit: Option<&mut dyn Emit>) -> CliResult<()> {
        loop {
            let Some(output) = self.decrypt.as_mut().and_then(|stage| stage.poll()) else {
                break;
            };
            if let MuxInput::Sample { track, sample } = output {
                self.reorder.push(track, sample)?;
            }
        }
        match emit.as_mut() {
            Some(emit) => self.drain_order(false, *emit),
            None => Ok(()),
        }
    }

    fn drain_order(&mut self, final_input: bool, emit: &mut dyn Emit) -> CliResult<()> {
        let mut reorder = core::mem::take(&mut self.reorder);
        let result = reorder.flush(final_input, &mut |track, sample| {
            self.input(MuxInput::Sample { track, sample })?;
            self.pump(emit)
        });
        self.reorder = reorder;
        result
    }

    fn input(&mut self, input: MuxInput) -> CliResult<()> {
        match &mut self.kind {
            Kind::Cmaf(stage) => stage.feed(input, Timestamp::ZERO)?,
            Kind::Ts(stage) => stage.feed(input, Timestamp::ZERO)?,
            Kind::Hls { segmenter, .. } => segmenter.feed(input, Timestamp::ZERO)?,
            Kind::TsHls(stage) => stage.feed(input, Timestamp::ZERO)?,
            Kind::Progressive(stage) => stage.feed(input, Timestamp::ZERO)?,
            Kind::Dash(stage) => stage.feed(input)?,
        }
        Ok(())
    }

    fn pump(&mut self, emit: &mut dyn Emit) -> CliResult<()> {
        let mut outputs = Vec::new();
        match &mut self.kind {
            Kind::Cmaf(stage) => drain_stage(stage, &mut outputs),
            Kind::Ts(stage) => drain_stage(stage, &mut outputs),
            Kind::Hls { segmenter, .. } => drain_stage(segmenter, &mut outputs),
            Kind::TsHls(stage) => drain_stage(stage, &mut outputs),
            Kind::Progressive(stage) => drain_stage(stage, &mut outputs),
            Kind::Dash(stage) => {
                for (name, bytes) in stage.take_files() {
                    emit.side(&name, &bytes)?;
                }
            }
        }
        self.emit_outputs(&outputs, emit)
    }

    fn emit_outputs(&mut self, outputs: &[MuxOutput], emit: &mut dyn Emit) -> CliResult<()> {
        for output in outputs {
            match (self.opts.format, output) {
                (OutputFormat::Cmaf | OutputFormat::Ts, MuxOutput::Init(bytes)) => {
                    emit.append(bytes)?
                }
                (OutputFormat::Cmaf | OutputFormat::Ts, MuxOutput::Bytes(bytes)) => {
                    emit.append(bytes)?
                }
                (OutputFormat::Cmaf | OutputFormat::Ts, MuxOutput::Segment(segment)) => {
                    emit.append(&segment.bytes)?
                }
                (OutputFormat::Hls, MuxOutput::Segment(segment)) => {
                    let index = match &self.kind {
                        Kind::Hls { measured, .. } => measured.len(),
                        _ => 0,
                    };
                    emit.side(&format!("seg{index}.m4s"), &segment.bytes)?;
                    let duration = segment_secs(&segment.bytes, &self.specs)?;
                    if let Kind::Hls { measured, .. } = &mut self.kind {
                        measured.push(duration);
                    }
                }
                (OutputFormat::TsHls, MuxOutput::Segment(segment)) => {
                    emit.side(
                        &format!("seg{}.ts", segment.summary.sequence),
                        &segment.bytes,
                    )?;
                }
                (OutputFormat::Progressive, MuxOutput::Bytes(bytes)) => emit.append(bytes)?,
                (OutputFormat::Progressive, MuxOutput::BytesAt { offset, bytes }) => {
                    emit.patch(*offset, bytes)?
                }
                (_, MuxOutput::Text(text)) => emit.text(text)?,
                _ => {}
            }
        }
        Ok(())
    }

    fn finish(&mut self, emit: &mut dyn Emit) -> CliResult<()> {
        self.drain_order(true, emit)?;
        match &mut self.kind {
            Kind::Cmaf(stage) => stage.finish()?,
            Kind::Ts(stage) => stage.finish()?,
            Kind::Hls { segmenter, .. } => segmenter.finish()?,
            Kind::TsHls(stage) => stage.finish()?,
            Kind::Progressive(stage) => stage.finish()?,
            Kind::Dash(stage) => stage.finish_segmenter()?,
        }
        self.pump(emit)?;
        match &mut self.kind {
            Kind::Hls { measured, .. } => {
                let text = render_cmaf_hls_playlist(measured)?;
                emit.text(&text)?;
            }
            Kind::TsHls(stage) => {
                let text = stage.playlist()?;
                emit.text(&text)?;
            }
            Kind::Dash(stage) => {
                stage.finish_tracks()?;
                for (name, bytes) in stage.take_files() {
                    emit.side(&name, &bytes)?;
                }
                let text = stage.render()?;
                emit.text(&text)?;
            }
            _ => {}
        }
        Ok(())
    }
}

/// Drain every output a stage has queued.
fn drain_stage<S>(stage: &mut S, out: &mut Vec<MuxOutput>)
where
    S: for<'a> Stage<In<'a> = MuxInput, Out = MuxOutput, Error = crate::Error>,
{
    while let Some(output) = stage.poll() {
        out.push(output);
    }
}

// ---------------------------------------------------------------------------
// DASH: one Representation per track, cut at the anchor's shared boundaries
// ---------------------------------------------------------------------------

/// One Representation's buffered samples and pending boundary instants.
struct DashTrack {
    spec: TrackSpec,
    pending: std::collections::VecDeque<Sample>,
    /// `(start, end)` boundary instants in this track's timescale.
    boundaries: std::collections::VecDeque<(u64, u64)>,
    clock: MediaClock,
    elapsed: u64,
    next_segment: u32,
}

/// The DASH writer: a multi-track [`Segmenter`] decides the shared segment
/// boundaries (so every Representation stays aligned), while each track's
/// samples are buffered until its own range is decided, then written as its own
/// `chunk-stream{id}-{n}.m4s` — the shape the MPD's Representations address.
struct DashStream {
    specs: Vec<TrackSpec>,
    opts: Opts,
    segmenter: Segmenter,
    anchor: usize,
    anchor_scale: u64,
    anchor_buf: std::collections::VecDeque<Sample>,
    anchor_clock: MediaClock,
    prev_end: u64,
    tracks: Vec<DashTrack>,
    log: SegmentSummaryLog,
    files: Vec<(String, Vec<u8>)>,
}

impl DashStream {
    fn new(specs: Vec<TrackSpec>, movie_timescale: Timescale, opts: &Opts) -> CliResult<Self> {
        let target = Duration::from_secs(u64::from(opts.segment_duration.max(1)));
        let segmenter = Segmenter::new(specs.clone(), movie_timescale, target)?;
        let anchor = choose_anchor(specs.iter().map(|spec| &spec.config))?;
        let anchor_ts = specs[anchor].timescale;
        let mut files = Vec::with_capacity(specs.len());
        for spec in &specs {
            let init = build_init_segment(core::slice::from_ref(spec), movie_timescale)?;
            files.push((format!("init-stream{}.m4s", spec.track_id.value()), init));
        }
        let tracks = specs
            .iter()
            .map(|spec| DashTrack {
                spec: spec.clone(),
                pending: std::collections::VecDeque::new(),
                boundaries: std::collections::VecDeque::new(),
                clock: MediaClock::new(),
                elapsed: 0,
                next_segment: 0,
            })
            .collect();
        Ok(Self {
            specs,
            opts: opts.clone(),
            segmenter,
            anchor,
            anchor_scale: u64::from(anchor_ts.get()),
            anchor_buf: std::collections::VecDeque::new(),
            anchor_clock: MediaClock::new(),
            prev_end: 0,
            tracks,
            log: SegmentSummaryLog::new(),
            files,
        })
    }

    fn feed(&mut self, input: MuxInput) -> CliResult<()> {
        match input {
            MuxInput::Sample { track, sample } => {
                self.segmenter.feed(
                    MuxInput::Sample {
                        track,
                        sample: sample.clone(),
                    },
                    Timestamp::ZERO,
                )?;
                self.process_segments()?;
                if let Some(state) = self
                    .tracks
                    .iter_mut()
                    .find(|state| state.spec.track_id == track)
                {
                    state.pending.push_back(sample.clone());
                }
                if track == self.specs[self.anchor].track_id {
                    self.anchor_buf.push_back(sample);
                }
                self.drain_tracks(false)
            }
            MuxInput::Discontinuity { .. } => {
                self.segmenter.feed(input, Timestamp::ZERO)?;
                Ok(())
            }
            MuxInput::Flush => {
                self.finish_segmenter()?;
                self.finish_tracks()
            }
            MuxInput::Track(_) => Ok(()),
        }
    }

    fn process_segments(&mut self) -> CliResult<()> {
        while let Some(output) = self.segmenter.poll() {
            if let MuxOutput::Segment(segment) = output {
                self.boundary(&segment)?;
            }
        }
        Ok(())
    }

    /// Translate one finished multi-track segment into per-track boundaries.
    fn boundary(&mut self, segment: &SegmentOut) -> CliResult<()> {
        let anchor_id = self.specs[self.anchor].track_id;
        let count = segment
            .tracks
            .iter()
            .find(|part| part.track == anchor_id)
            .map_or(0, |part| {
                usize::try_from(part.samples).unwrap_or(usize::MAX)
            });
        let mut acc = 0u64;
        for _ in 0..count {
            let sample = self
                .anchor_buf
                .pop_front()
                .ok_or(crate::Error::InvalidInput(
                    "dash: a segment names more anchor samples than were fed",
                ))?;
            acc = acc.saturating_add(self.anchor_clock.tick(&sample));
        }
        let end = self.prev_end.saturating_add(acc);
        for state in &mut self.tracks {
            let scale = u64::from(state.spec.timescale.get());
            let start = self.prev_end.saturating_mul(scale) / self.anchor_scale;
            let end_ticks = end.saturating_mul(scale) / self.anchor_scale;
            state.boundaries.push_back((start, end_ticks));
        }
        self.prev_end = end;
        Ok(())
    }

    fn drain_tracks(&mut self, final_input: bool) -> CliResult<()> {
        let mut produced = Vec::new();
        for index in 0..self.tracks.len() {
            let mut parts = Vec::new();
            {
                let state = &mut self.tracks[index];
                while let Some(&(start, end)) = state.boundaries.front() {
                    let last = final_input && state.boundaries.len() == 1;
                    let Some(count) = plan_consume(state, end, last, final_input) else {
                        break;
                    };
                    let samples: Vec<Sample> = state.pending.drain(0..count).collect();
                    let sequence = state.next_segment.saturating_add(1);
                    let bytes = build_media_segment(
                        sequence,
                        &[FragmentTrackData::new(state.spec.track_id, start, &samples)],
                    )?;
                    let part = TrackPart {
                        track: state.spec.track_id,
                        start,
                        duration: segment_ticks(&bytes)?,
                        timescale: state.spec.timescale.get(),
                        bytes: bytes.len() as u64,
                        samples: u32::try_from(samples.len()).unwrap_or(u32::MAX),
                    };
                    let name =
                        format!("chunk-stream{}-{sequence}.m4s", state.spec.track_id.value());
                    parts.push((
                        part,
                        samples.first().is_some_and(|sample| sample.flags.is_sync),
                        name,
                        bytes,
                    ));
                    state.next_segment = sequence;
                    state.boundaries.pop_front();
                }
            }
            for (part, independent, name, bytes) in parts {
                self.log.push_part(&part, independent)?;
                produced.push((name, bytes));
            }
        }
        self.files.extend(produced);
        Ok(())
    }

    fn finish_segmenter(&mut self) -> CliResult<()> {
        self.segmenter.finish()?;
        self.process_segments()
    }

    fn finish_tracks(&mut self) -> CliResult<()> {
        self.drain_tracks(true)
    }

    fn take_files(&mut self) -> Vec<(String, Vec<u8>)> {
        core::mem::take(&mut self.files)
    }

    fn render(&self) -> CliResult<String> {
        if self.opts.low_latency {
            let segment = Duration::from_secs(u64::from(self.opts.segment_duration.max(1)));
            let mut packager = crate::ll_dash::LlDashPackager::new(
                segment,
                segment / LL_DASH_CHUNKS_PER_SEGMENT,
                LL_DASH_LATENCY_TARGET,
                availability_start_time_now(),
            )?;
            packager.base.dynamic = false;
            packager.base.availability_start_time = None;
            if let Some(url) = &self.opts.utc_timing_url {
                packager =
                    packager.with_utc_timing(crate::ll_dash::UTCTIMING_HTTP_HEAD_2014, url.clone());
            }
            Ok(packager.render(&self.specs, &self.log)?)
        } else {
            if self.opts.utc_timing_url.is_some() {
                return Err(CliError::UtcTimingNeedsLl);
            }
            Ok(DashPackager::default().render(&self.specs, &self.log)?)
        }
    }
}

/// Consume the samples a boundary rule takes for one Representation, or `None`
/// while the track has not delivered enough samples to decide.
fn plan_consume(state: &mut DashTrack, end: u64, last: bool, final_input: bool) -> Option<usize> {
    let mut probe = state.clock;
    let mut elapsed = state.elapsed;
    let mut count = 0usize;
    while count < state.pending.len() && elapsed < end {
        elapsed = elapsed.saturating_add(probe.tick(&state.pending[count]));
        count += 1;
    }
    if final_input && last {
        count = state.pending.len();
    } else {
        if !final_input && elapsed < end {
            return None;
        }
        if count == 0 {
            if let Some(sample) = state.pending.front() {
                elapsed = elapsed.saturating_add(probe.tick(sample));
                count = 1;
            } else if !final_input {
                return None;
            }
        }
    }
    state.clock = probe;
    state.elapsed = elapsed;
    Some(count)
}

// ---------------------------------------------------------------------------
// Input probing and the demux drivers
// ---------------------------------------------------------------------------

/// The movie timescale used when a container declares none.
const DEFAULT_MOVIE_TIMESCALE_TS: Timescale = broadcast_common::timescale!(DEFAULT_MOVIE_TIMESCALE);
/// Bytes read from an input to answer one container probe step.
const PROBE_STEP_BYTES: usize = 4096;

/// Probe `reader`'s leading bytes until the container is decided, returning the
/// bytes consumed (they are replayed when the input cannot seek).
fn probe_reader<R: Read>(reader: &mut R) -> CliResult<(Container, Vec<u8>)> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; PROBE_STEP_BYTES];
    loop {
        match container_probe::probe_with_budget(&buf, container_probe::DEFAULT_BUDGET) {
            container_probe::Probe::Identified { format, .. } => {
                return Ok((container_from_format(format)?, buf));
            }
            container_probe::Probe::Ambiguous { .. } | container_probe::Probe::Unknown => {
                return Err(CliError::UnknownContainer);
            }
            container_probe::Probe::Insufficient { need_at_least, .. } => {
                if buf.len() >= container_probe::DEFAULT_BUDGET {
                    return Err(CliError::UnknownContainer);
                }
                let target = need_at_least
                    .max(buf.len() + 1)
                    .min(container_probe::DEFAULT_BUDGET);
                let mut want = target - buf.len();
                while want > 0 {
                    let step = want.min(chunk.len());
                    let read = reader.read(&mut chunk[..step])?;
                    if read == 0 {
                        return Err(CliError::UnknownContainer);
                    }
                    buf.extend_from_slice(&chunk[..read]);
                    want -= read;
                }
            }
            _ => return Err(CliError::UnknownContainer),
        }
    }
}

/// A reader that yields an already-consumed probe prefix first.
struct PrefixReader<R> {
    prefix: Vec<u8>,
    pos: usize,
    inner: R,
}

impl<R: Read> Read for PrefixReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.pos < self.prefix.len() {
            let available = self.prefix.len() - self.pos;
            let take = available.min(buf.len());
            buf[..take].copy_from_slice(&self.prefix[self.pos..self.pos + take]);
            self.pos += take;
            return Ok(take);
        }
        self.inner.read(buf)
    }
}

/// Restart a seekable input at its beginning.
fn rewind<R: Seek>(reader: &mut R) -> CliResult<()> {
    reader.seek(SeekFrom::Start(0))?;
    Ok(())
}

/// Read `want` bytes at absolute `offset`.
fn read_at<R: Read + Seek>(reader: &mut R, offset: u64, want: usize) -> CliResult<bytes::Bytes> {
    reader.seek(SeekFrom::Start(offset))?;
    let mut buf = vec![0u8; want];
    let mut filled = 0;
    while filled < want {
        let read = reader.read(&mut buf[filled..])?;
        if read == 0 {
            break;
        }
        filled += read;
    }
    buf.truncate(filled);
    Ok(bytes::Bytes::from(buf))
}

/// The event sink a demux driver calls as it produces events.
type EventSink<'a> = dyn FnMut(DemuxEvent, Option<Timescale>) -> CliResult<()> + 'a;

/// Demux a seekable input to EOF, calling `sink` for each event.
fn drive_seekable<R: Read + Seek>(
    reader: &mut R,
    container: Container,
    total_len: Option<u64>,
    sink: &mut EventSink<'_>,
) -> CliResult<()> {
    let mut buf = vec![0u8; READ_CHUNK_BYTES];
    match container {
        Container::MpegTs => {
            let mut demux = StreamingTsDemux::new();
            loop {
                let read = reader.read(&mut buf)?;
                if read == 0 {
                    break;
                }
                demux.feed(&buf[..read])?;
                while let Some(event) = demux.poll_event() {
                    sink(event, Some(crate::rtp::VIDEO_CLOCK_TIMESCALE))?;
                }
            }
            demux.finish()?;
            while let Some(event) = demux.poll_event() {
                sink(event, Some(crate::rtp::VIDEO_CLOCK_TIMESCALE))?;
            }
            Ok(())
        }
        Container::Flv => {
            let mut demux = StreamingFlvDemux::new();
            loop {
                let read = reader.read(&mut buf)?;
                if read == 0 {
                    break;
                }
                demux
                    .feed(&buf[..read])
                    .map_err(|error| crate::Error::InvalidInput(flv_reason(error)))?;
                while let Some(event) = demux.poll_event() {
                    sink(event, Some(crate::flv::FLV_TIMESCALE))?;
                }
            }
            demux.finish();
            while let Some(event) = demux.poll_event() {
                sink(event, Some(crate::flv::FLV_TIMESCALE))?;
            }
            Ok(())
        }
        Container::Mp4 => {
            let mut demux = Mp4Demux::seekable(Mp4DemuxConfig::default().with_total_len(total_len));
            loop {
                while let Some(event) = demux.poll() {
                    sink(event, demux.movie_timescale())?;
                }
                let demand = demux.demand();
                let Some(offset) = demand.offset else {
                    break;
                };
                let bytes = read_at(reader, offset, demand.want_bytes)?;
                demux.feed(Mp4Input::At { offset, bytes }, Timestamp::ZERO)?;
            }
            demux.finish()?;
            while let Some(event) = demux.poll() {
                sink(event, demux.movie_timescale())?;
            }
            Ok(())
        }
        Container::MpegPs => {
            let mut demux = StreamingPsDemux::new();
            loop {
                let read = reader.read(&mut buf)?;
                if read == 0 {
                    break;
                }
                demux.feed(&buf[..read], Timestamp::ZERO)?;
                while let Some(event) = demux.poll() {
                    sink(event, Some(crate::rtp::VIDEO_CLOCK_TIMESCALE))?;
                }
            }
            demux.finish()?;
            while let Some(event) = demux.poll() {
                sink(event, Some(crate::rtp::VIDEO_CLOCK_TIMESCALE))?;
            }
            Ok(())
        }
        Container::WebM => Err(CliError::UnsupportedContainer(container.name())),
    }
}

/// Demux a forward-only input to EOF, calling `sink` for each event.
fn drive_forward<R: Read>(
    reader: &mut R,
    container: Container,
    sink: &mut EventSink<'_>,
) -> CliResult<()> {
    let mut buf = vec![0u8; READ_CHUNK_BYTES];
    match container {
        Container::MpegTs => {
            let mut demux = StreamingTsDemux::new();
            loop {
                let read = reader.read(&mut buf)?;
                if read == 0 {
                    break;
                }
                demux.feed(&buf[..read])?;
                while let Some(event) = demux.poll_event() {
                    sink(event, Some(crate::rtp::VIDEO_CLOCK_TIMESCALE))?;
                }
            }
            demux.finish()?;
            while let Some(event) = demux.poll_event() {
                sink(event, Some(crate::rtp::VIDEO_CLOCK_TIMESCALE))?;
            }
            Ok(())
        }
        Container::Flv => {
            let mut demux = StreamingFlvDemux::new();
            loop {
                let read = reader.read(&mut buf)?;
                if read == 0 {
                    break;
                }
                demux
                    .feed(&buf[..read])
                    .map_err(|error| crate::Error::InvalidInput(flv_reason(error)))?;
                while let Some(event) = demux.poll_event() {
                    sink(event, Some(crate::flv::FLV_TIMESCALE))?;
                }
            }
            demux.finish();
            while let Some(event) = demux.poll_event() {
                sink(event, Some(crate::flv::FLV_TIMESCALE))?;
            }
            Ok(())
        }
        Container::Mp4 => {
            let mut demux = Mp4Demux::sequential(Mp4DemuxConfig::default());
            loop {
                let read = reader.read(&mut buf)?;
                if read == 0 {
                    break;
                }
                demux.feed(Mp4Input::Next(&buf[..read]), Timestamp::ZERO)?;
                while let Some(event) = demux.poll() {
                    sink(event, demux.movie_timescale())?;
                }
            }
            demux.feed(Mp4Input::Eof, Timestamp::ZERO)?;
            demux.finish()?;
            while let Some(event) = demux.poll() {
                sink(event, demux.movie_timescale())?;
            }
            Ok(())
        }
        Container::MpegPs => {
            let mut demux = StreamingPsDemux::new();
            loop {
                let read = reader.read(&mut buf)?;
                if read == 0 {
                    break;
                }
                demux.feed(&buf[..read], Timestamp::ZERO)?;
                while let Some(event) = demux.poll() {
                    sink(event, Some(crate::rtp::VIDEO_CLOCK_TIMESCALE))?;
                }
            }
            demux.finish()?;
            while let Some(event) = demux.poll() {
                sink(event, Some(crate::rtp::VIDEO_CLOCK_TIMESCALE))?;
            }
            Ok(())
        }
        Container::WebM => Err(CliError::UnsupportedContainer(container.name())),
    }
}

/// Map an FLV demux failure to a stable static reason (the library `Error`
/// variant takes `&'static str`).
fn flv_reason(_error: crate::flv::FlvError) -> &'static str {
    "FLV demux failed"
}

/// Collect the track set and movie timescale a demux announces.
#[derive(Default)]
struct SpecCollector {
    specs: Vec<TrackSpec>,
    movie: Option<Timescale>,
}

impl SpecCollector {
    fn movie(&mut self, movie: Option<Timescale>) {
        if self.movie.is_none() {
            self.movie = movie;
        }
    }

    fn event(&mut self, event: &DemuxEvent) {
        match event {
            DemuxEvent::TrackAdded(spec) | DemuxEvent::TrackUpdated(spec) => {
                if let Some(existing) = self
                    .specs
                    .iter_mut()
                    .find(|existing| existing.track_id == spec.track_id)
                {
                    *existing = spec.clone();
                } else {
                    self.specs.push(spec.clone());
                }
            }
            _ => {}
        }
    }
}

/// The movie timescale a container's muxers build their `moov` with.
fn movie_timescale(container: Container, declared: Option<Timescale>) -> Timescale {
    match container {
        Container::Mp4 => declared.unwrap_or(DEFAULT_MOVIE_TIMESCALE_TS),
        Container::MpegTs => crate::rtp::VIDEO_CLOCK_TIMESCALE,
        Container::Flv => crate::flv::FLV_TIMESCALE,
        Container::MpegPs => crate::rtp::VIDEO_CLOCK_TIMESCALE,
        Container::WebM => DEFAULT_MOVIE_TIMESCALE_TS,
    }
}

/// `true` when the format writes ISOBMFF, so opaque `Data`/`Subtitle` tracks
/// that the builder cannot place must be dropped first.
fn is_bmff_format(format: OutputFormat) -> bool {
    matches!(
        format,
        OutputFormat::Cmaf | OutputFormat::Hls | OutputFormat::Dash | OutputFormat::Progressive
    )
}

/// Apply `--tracks` and the ISOBMFF track filter to an announced track set.
fn apply_specs(
    specs: Vec<TrackSpec>,
    opts: &Opts,
    format: OutputFormat,
) -> CliResult<Vec<TrackSpec>> {
    let mut specs = specs;
    if !opts.tracks.is_empty() {
        specs.retain(|spec| opts.tracks.contains(&spec.track_id));
        if specs.is_empty() {
            return Err(CliError::NoTracksSelected);
        }
    }
    if is_bmff_format(format) {
        let keeps = |spec: &TrackSpec| -> bool {
            match &spec.config {
                // A data track crosses into ISOBMFF only when the caller maps
                // it to an `mp4:` identity; its own identity may be another
                // container's (the long-standing "drop opaque data from
                // CMAF" behaviour).
                CodecConfig::Data { .. } => matches!(
                    opts.data_identity.get(spec.track_id),
                    Some(DataIdentity::Mp4 { .. })
                ),
                _ => spec.config.is_muxable_in_bmff(),
            }
        };
        let dropped: Vec<TrackId> = specs
            .iter()
            .filter(|spec| !keeps(spec))
            .map(|spec| spec.track_id)
            .collect();
        if !dropped.is_empty() {
            eprintln!(
                "warning: dropping track(s) {dropped:?} from CMAF/fMP4 output \
                 (no ISOBMFF carriage in this crate for their codec; map one with \
                 --data-identity <track>=mp4:<handler>:<sample-entry-hex>)"
            );
            specs.retain(keeps);
            if specs.is_empty() {
                return Err(crate::Error::InvalidInput(
                    "select_tracks_by: predicate kept no track",
                )
                .into());
            }
        }
    }
    Ok(specs)
}

/// Apply the caller's [`DataIdentityMap`] to every announcement through the
/// one [`DataIdentityStage`], returning the track specs the muxer must use.
fn apply_identity(specs: &[TrackSpec], map: &DataIdentityMap) -> CliResult<Vec<TrackSpec>> {
    if map.is_empty() {
        return Ok(specs.to_vec());
    }
    let mut stage = DataIdentityStage::new(map.clone());
    let mut out = Vec::with_capacity(specs.len());
    for spec in specs {
        stage.feed(MuxInput::Track(Box::new(spec.clone())), Timestamp::ZERO)?;
        match stage.poll() {
            Some(MuxInput::Track(spec)) => out.push(*spec),
            _ => {
                return Err(crate::Error::InvalidInput(
                    "data identity stage produced no track announcement",
                )
                .into());
            }
        }
    }
    Ok(out)
}

/// Parse a `--data-identity <track>=<spec>` argument.
fn parse_data_identity_argument(argument: &str) -> CliResult<(TrackId, DataIdentity)> {
    let bad = || CliError::BadDataIdentity(argument.to_string());
    let (track_text, rest) = argument.split_once('=').ok_or_else(bad)?;
    let track: u32 = track_text.trim().parse().map_err(|_| bad())?;
    let identity = parse_data_identity(rest).ok_or_else(bad)?;
    Ok((TrackId::new(track), identity))
}

/// Parse the `<spec>` half of a `--data-identity` argument.
fn parse_data_identity(spec: &str) -> Option<DataIdentity> {
    let (family, tail) = spec.split_once(':')?;
    match family {
        "ts" => {
            let (stream_type, descriptors) = match tail.split_once(':') {
                Some((stream_type, hex)) => (
                    stream_type.parse::<u8>().ok()?,
                    broadcast_common::hex::hex_decode(hex)?,
                ),
                None => (tail.parse::<u8>().ok()?, Vec::new()),
            };
            Some(DataIdentity::Ts {
                stream_type,
                descriptors,
                carriage: DataCarriage::Pes,
            })
        }
        "mp4" => {
            let (handler_text, entry_hex) = tail.split_once(':')?;
            let handler_bytes = handler_text.as_bytes();
            if handler_bytes.len() != isobmff::box_types::BOX_TYPE_SIZE {
                return None;
            }
            let mut handler = [0u8; isobmff::box_types::BOX_TYPE_SIZE];
            handler.copy_from_slice(handler_bytes);
            let sample_entry = hex_bytes(entry_hex)?;
            if sample_entry.is_empty() {
                return None;
            }
            Some(DataIdentity::Mp4 {
                handler,
                sample_entry,
            })
        }
        "matroska" => {
            let (codec_id, codec_private) = match tail.split_once(':') {
                Some((codec_id, hex)) => (codec_id, hex_bytes(hex)?),
                None => (tail, bytes::Bytes::new()),
            };
            if codec_id.is_empty() {
                return None;
            }
            Some(DataIdentity::Matroska {
                codec_id: codec_id.to_string(),
                codec_private,
            })
        }
        _ => None,
    }
}

/// Decode a hex string's bytes, or `None` when it is not hex.
fn hex_bytes(text: &str) -> Option<bytes::Bytes> {
    broadcast_common::hex::hex_decode(text).map(bytes::Bytes::from)
}

/// Read an unstreamable container whole, bounded by the caller's cap.
fn read_whole<R: Read>(reader: &mut R, container: Container, opts: &Opts) -> CliResult<Media> {
    let mut buf = Vec::new();
    match opts.max_whole_input_bytes {
        Some(limit) => {
            let mut limited = reader.take(limit.saturating_add(1));
            limited.read_to_end(&mut buf)?;
            if buf.len() as u64 > limit {
                return Err(CliError::WholeInputTooLarge { limit });
            }
        }
        None => {
            reader.read_to_end(&mut buf)?;
        }
    }
    let media = match container {
        Container::WebM => WebmDemux::new().unpackage(&buf)?,
        _ => {
            return Err(CliError::UnsupportedContainer(container.name()));
        }
    };
    Ok(media)
}

// ---------------------------------------------------------------------------
// The run drivers
// ---------------------------------------------------------------------------

/// Demux, interleave and mux `input` (container autodetected) into `opts.format`.
/// Pure function over bytes: no filesystem access, so it is the unit under test.
pub fn run_bytes(input: &[u8], opts: &Opts) -> CliResult<Output> {
    let container = detect_container(input)?;
    let mut cursor = std::io::Cursor::new(input);
    let mut emit = MemoryEmit::default();
    run_seekable(
        &mut cursor,
        container,
        u64::try_from(input.len()).ok(),
        opts,
        &mut emit,
        no_keys(),
    )?;
    Ok(emit.output())
}

/// Run over a seekable input.
fn run_seekable<R: Read + Seek>(
    reader: &mut R,
    container: Container,
    total_len: Option<u64>,
    opts: &Opts,
    emit: &mut dyn Emit,
    keys: DecryptKeys,
) -> CliResult<()> {
    if container == Container::WebM {
        #[cfg(feature = "cenc")]
        if keys.is_some() {
            return Err(crate::Error::InvalidInput(
                "--decrypt only applies to CENC-protected MP4/CMAF input",
            )
            .into());
        }
        let media = read_whole(reader, container, opts)?;
        return run_media_path(&media, opts, emit);
    }
    if opts.format == OutputFormat::Progressive {
        if !emit.seekable() {
            return Err(CliError::ProgressiveNeedsSeekableOutput);
        }
        return run_progressive(reader, container, total_len, opts, emit, keys);
    }
    let (specs, movie) = discover_seekable(reader, container, total_len, opts)?;
    rewind(reader)?;
    let mut stream = CliStream::start(specs, movie, opts, emit, None, keys)?;
    drive_seekable(reader, container, total_len, &mut |event, _| {
        stream.event(event, emit)
    })?;
    stream.finish(emit)
}

/// Run over a forward-only input.
fn run_forward<R: Read>(
    reader: &mut R,
    container: Container,
    opts: &Opts,
    emit: &mut dyn Emit,
    keys: DecryptKeys,
) -> CliResult<()> {
    if container == Container::WebM {
        let media = read_whole(reader, container, opts)?;
        return run_media_path(&media, opts, emit);
    }
    if opts.format == OutputFormat::Progressive {
        return Err(crate::Error::ProgressiveNeedsReplayableInput.into());
    }
    let mut collector = SpecCollector::default();
    let mut pending: Vec<DemuxEvent> = Vec::new();
    let mut pending_bytes = 0u64;
    let mut stream: Option<CliStream> = None;
    let mut keys = keys;
    drive_forward(reader, container, &mut |event, movie| {
        collector.movie(movie);
        if let Some(stream) = stream.as_mut() {
            return stream.event(event, emit);
        }
        collector.event(&event);
        if let DemuxEvent::Sample { sample, .. } = &event {
            pending_bytes = pending_bytes.saturating_add(sample.data.len() as u64);
        }
        let trigger = match container {
            Container::Flv | Container::MpegPs => pending_bytes >= FLV_TRACK_SETTLE_BYTES,
            _ => matches!(event, DemuxEvent::TracksResolved { .. }),
        };
        pending.push(event);
        if trigger {
            let specs = apply_specs(collector.specs.clone(), opts, opts.format)?;
            let movie = movie_timescale(container, collector.movie);
            let mut started = CliStream::start(specs, movie, opts, emit, None, keys.take())?;
            for event in pending.drain(..) {
                if !is_track_event(&event) {
                    started.event(event, emit)?;
                }
            }
            stream = Some(started);
        }
        Ok(())
    })?;
    match stream {
        Some(mut stream) => stream.finish(emit),
        None => {
            let specs = apply_specs(collector.specs.clone(), opts, opts.format)?;
            let movie = movie_timescale(container, collector.movie);
            let mut started = CliStream::start(specs, movie, opts, emit, None, keys.take())?;
            for event in pending {
                if !is_track_event(&event) {
                    started.event(event, emit)?;
                }
            }
            started.finish(emit)
        }
    }
}

fn is_track_event(event: &DemuxEvent) -> bool {
    matches!(
        event,
        DemuxEvent::TrackAdded(_) | DemuxEvent::TrackUpdated(_)
    )
}

/// Read the announced track set and movie timescale from a seekable input.
fn discover_seekable<R: Read + Seek>(
    reader: &mut R,
    container: Container,
    total_len: Option<u64>,
    opts: &Opts,
) -> CliResult<(Vec<TrackSpec>, Timescale)> {
    let mut collector = SpecCollector::default();
    drive_seekable(reader, container, total_len, &mut |event, movie| {
        collector.movie(movie);
        collector.event(&event);
        Ok(())
    })?;
    let specs = apply_specs(collector.specs.clone(), opts, opts.format)?;
    let movie = movie_timescale(container, collector.movie);
    Ok((specs, movie))
}

/// Two passes over a seekable input: a descriptor-only plan pass, then the
/// progressive writer at the offsets that plan fixed.
fn run_progressive<R: Read + Seek>(
    reader: &mut R,
    container: Container,
    total_len: Option<u64>,
    opts: &Opts,
    emit: &mut dyn Emit,
    keys: DecryptKeys,
) -> CliResult<()> {
    let (plan, specs, movie) = progressive_plan(reader, container, total_len, opts)?;
    rewind(reader)?;
    let mut stream = CliStream::start(specs, movie, opts, emit, Some(plan), keys)?;
    drive_seekable(reader, container, total_len, &mut |event, _| {
        stream.event(event, emit)
    })?;
    stream.finish(emit)
}

/// The plan pass: record every sample's descriptor (no coded bytes held) and
/// the announced track set.
fn progressive_plan<R: Read + Seek>(
    reader: &mut R,
    container: Container,
    total_len: Option<u64>,
    opts: &Opts,
) -> CliResult<(isobmff::plan::OwnedPlan, Vec<TrackSpec>, Timescale)> {
    let mut collector = SpecCollector::default();
    let mut buffered: Vec<DemuxEvent> = Vec::new();
    let mut buffered_bytes = 0u64;
    let mut kept: Vec<TrackId> = Vec::new();
    let mut plan_pass: Option<PlanPass> = None;
    let mut failure: Option<CliError> = None;
    drive_seekable(reader, container, total_len, &mut |event, movie| {
        collector.movie(movie);
        if let Some(pass) = plan_pass.as_mut() {
            if let DemuxEvent::Sample { track_id, sample } = event
                && kept.contains(&track_id)
                && let Err(error) = pass.feed(
                    MuxInput::Sample {
                        track: track_id,
                        sample,
                    },
                    Timestamp::ZERO,
                )
                && failure.is_none()
            {
                failure = Some(error.into());
            }
            return Ok(());
        }
        collector.event(&event);
        if let DemuxEvent::Sample { sample, .. } = &event {
            buffered_bytes = buffered_bytes.saturating_add(sample.data.len() as u64);
        }
        let trigger = match container {
            Container::Flv | Container::MpegPs => buffered_bytes >= FLV_TRACK_SETTLE_BYTES,
            _ => matches!(event, DemuxEvent::TracksResolved { .. }),
        };
        buffered.push(event);
        if trigger {
            let specs = apply_specs(collector.specs.clone(), opts, opts.format)?;
            let movie = movie_timescale(container, collector.movie);
            let mut pass = PlanPass::new(movie);
            for spec in &specs {
                pass.feed(MuxInput::Track(Box::new(spec.clone())), Timestamp::ZERO)?;
            }
            kept = specs.iter().map(|spec| spec.track_id).collect();
            for event in buffered.drain(..) {
                if let DemuxEvent::Sample { track_id, sample } = event
                    && kept.contains(&track_id)
                {
                    pass.feed(
                        MuxInput::Sample {
                            track: track_id,
                            sample,
                        },
                        Timestamp::ZERO,
                    )?;
                }
            }
            plan_pass = Some(pass);
        }
        Ok(())
    })?;
    if plan_pass.is_none() {
        let specs = apply_specs(collector.specs.clone(), opts, opts.format)?;
        let movie = movie_timescale(container, collector.movie);
        let mut pass = PlanPass::new(movie);
        for spec in &specs {
            pass.feed(MuxInput::Track(Box::new(spec.clone())), Timestamp::ZERO)?;
        }
        kept = specs.iter().map(|spec| spec.track_id).collect();
        for event in buffered.drain(..) {
            if let DemuxEvent::Sample { track_id, sample } = event
                && kept.contains(&track_id)
            {
                pass.feed(
                    MuxInput::Sample {
                        track: track_id,
                        sample,
                    },
                    Timestamp::ZERO,
                )?;
            }
        }
        plan_pass = Some(pass);
    }
    if let Some(error) = failure {
        return Err(error);
    }
    let mut plan_pass = plan_pass.ok_or(crate::Error::ProgressiveNeedsReplayableInput)?;
    plan_pass.finish()?;
    let plan = plan_pass.poll().ok_or(crate::Error::InvalidInput(
        "progressive: the plan pass produced no plan",
    ))?;
    let specs = apply_specs(collector.specs.clone(), opts, opts.format)?;
    let movie = movie_timescale(container, collector.movie);
    Ok((plan, specs, movie))
}

/// Mux an already-demuxed [`Media`] (a container this CLI cannot stream yet).
fn run_media_path(media: &Media, opts: &Opts, emit: &mut dyn Emit) -> CliResult<()> {
    let specs = apply_specs(
        media
            .tracks
            .iter()
            .map(|track| track.spec.clone())
            .collect(),
        opts,
        opts.format,
    )?;
    let kept: Vec<crate::ir::Track> = media
        .tracks
        .iter()
        .filter(|track| {
            specs
                .iter()
                .any(|spec| spec.track_id == track.spec.track_id)
        })
        .cloned()
        .collect();
    let filtered = Media::new(kept, media.movie_timescale);
    if opts.format == OutputFormat::Progressive && !emit.seekable() {
        return Err(CliError::ProgressiveNeedsSeekableOutput);
    }
    let plan = if opts.format == OutputFormat::Progressive {
        Some(PlanPass::for_media(&filtered)?)
    } else {
        None
    };
    let mut stream = CliStream::start(specs, media.movie_timescale, opts, emit, plan, no_keys())?;
    for track in &filtered.tracks {
        for sample in &track.samples {
            stream.sample(track.spec.track_id, sample.clone(), emit)?;
        }
    }
    stream.finish(emit)
}

// ---------------------------------------------------------------------------
// Manifest rendering and measurement
// ---------------------------------------------------------------------------

/// Render the CMAF-HLS media playlist for `durations`: one `#EXTINF` per
/// segment, each pointing at `seg{i}.m4s` (a media-only `styp`+`moof`+`mdat`
/// file) with an `#EXT-X-MAP` naming the init segment.
///
/// Each `#EXTINF` is the segment's **own** measured duration (read back out of
/// the produced bytes by [`segment_secs`]) and `#EXT-X-TARGETDURATION` is the
/// ceiling of the largest of them (RFC 8216 §4.3.3.1).
///
/// # Errors
/// [`Error::InvalidInput`](crate::Error::InvalidInput) if a duration does not
/// fit [`broadcast_hls::DecimalSeconds`], or if there are no segments.
fn render_cmaf_hls_playlist(durations: &[f64]) -> CliResult<String> {
    use broadcast_hls::{DecimalSeconds, MediaPlaylist, MediaSegment};

    if durations.is_empty() {
        return Err(
            crate::Error::InvalidInput("cannot render an HLS playlist with no segments").into(),
        );
    }

    let segs = durations
        .iter()
        .enumerate()
        .map(|(i, secs)| {
            Ok(MediaSegment {
                uri: format!("seg{i}.m4s"),
                duration: DecimalSeconds::new(*secs).map_err(|_| {
                    crate::Error::InvalidInput("segment duration must be finite and non-negative")
                })?,
                discontinuous: false,
                map: Some(broadcast_hls::MapTag {
                    uri: String::from(HLS_INIT_FILE),
                    byte_range: None,
                    extra_attrs: Vec::new(),
                }),
                ..Default::default()
            })
        })
        .collect::<CliResult<Vec<_>>>()?;

    let max_secs = durations.iter().copied().fold(0.0f64, f64::max).max(1.0);
    let target_duration = u32::try_from(max_secs.ceil() as u64).unwrap_or(u32::MAX);

    let playlist = MediaPlaylist {
        version: HLS_CMAF_VERSION,
        target_duration,
        msn: broadcast_hls::MediaSequence::new(0),
        discontinuity_sequence: 0,
        segments: segs,
        endlist: true,
        ..Default::default()
    };
    playlist
        .to_m3u8()
        .map_err(|error| crate::Error::HlsAttrValue(error).into())
}

/// A media segment's presentation duration in seconds, from the `moof` it
/// carries.
///
/// Each `trun` sample duration is scaled by its **own** track's timescale: a
/// multi-track segment's tracks may run at different rates (90 kHz video and
/// 44.1 kHz audio), so the per-track spans are summed separately and the
/// longest wins — the segment is only as long as its slowest-to-finish track.
///
/// # Errors
/// [`Error::InvalidInput`](crate::Error::InvalidInput) when the segment carries
/// no parseable `moof`/`trun`.
fn segment_secs(segment: &[u8], specs: &[TrackSpec]) -> CliResult<f64> {
    use isobmff::box_types::parse_box;
    use isobmff::movie_fragment::MovieFragmentBox;

    let mut remaining = segment;
    while !remaining.is_empty() {
        let (bx, consumed) = parse_box(remaining)
            .map_err(|_| crate::Error::InvalidInput("media segment box could not be parsed"))?;
        if bx.header.box_type.is(&isobmff::segments::MOOF) {
            let moof = MovieFragmentBox::parse_body(bx.body)
                .map_err(|_| crate::Error::InvalidInput("moof box could not be parsed"))?;
            let mut longest = 0.0f64;
            for traf in &moof.traf {
                let scale = specs
                    .iter()
                    .find(|spec| spec.track_id.value() == traf.tfhd.track_id.value())
                    .map(|spec| f64::from(spec.timescale.get()))
                    .ok_or(crate::Error::InvalidInput(
                        "media segment names a track not present in the input",
                    ))?;
                let mut ticks = 0u64;
                for run in &traf.trun {
                    for sample in &run.samples {
                        ticks += u64::from(sample.sample_duration.unwrap_or(0));
                    }
                }
                let secs = ticks as f64 / scale;
                if secs > longest {
                    longest = secs;
                }
            }
            return Ok(longest);
        }
        if consumed == 0 || consumed > remaining.len() {
            break;
        }
        remaining = &remaining[consumed..];
    }
    Err(
        crate::Error::InvalidInput("media segment carries no moof: cannot derive its duration")
            .into(),
    )
}

/// The largest `trun` sample-duration sum in a media segment, in its track's
/// media timescale — the segment's own decoded span.
///
/// # Errors
/// [`Error::InvalidInput`](crate::Error::InvalidInput) when no `moof`/`trun`
/// can be parsed.
fn segment_ticks(segment: &[u8]) -> CliResult<u64> {
    use isobmff::box_types::parse_box;
    use isobmff::movie_fragment::MovieFragmentBox;

    let mut remaining = segment;
    while !remaining.is_empty() {
        let (bx, consumed) = parse_box(remaining)
            .map_err(|_| crate::Error::InvalidInput("media segment box could not be parsed"))?;
        if bx.header.box_type.is(&isobmff::segments::MOOF) {
            let moof = MovieFragmentBox::parse_body(bx.body)
                .map_err(|_| crate::Error::InvalidInput("moof box could not be parsed"))?;
            let mut longest = 0u64;
            for traf in &moof.traf {
                let mut ticks = 0u64;
                for run in &traf.trun {
                    for sample in &run.samples {
                        ticks += u64::from(sample.sample_duration.unwrap_or(0));
                    }
                }
                if ticks > longest {
                    longest = ticks;
                }
            }
            return Ok(longest);
        }
        if consumed == 0 || consumed > remaining.len() {
            break;
        }
        remaining = &remaining[consumed..];
    }
    Err(
        crate::Error::InvalidInput("media segment carries no moof: cannot derive its duration")
            .into(),
    )
}

// ---------------------------------------------------------------------------
// I/O driver (called by main.rs)
// ---------------------------------------------------------------------------

/// The path that means standard input, or standard output.
const STDIN_INPUT: &str = "-";

/// Resolve the input path from the positional or `-i` flag.
fn input_path(args: &Args) -> &Path {
    args.in_positional
        .as_deref()
        .or(args.input.as_deref())
        .unwrap_or_else(|| Path::new(STDIN_INPUT))
}

/// Resolve the output format from `-f` or the output extension.
fn resolve_format(args: &Args) -> CliResult<OutputFormat> {
    if let Some(format) = args.format {
        return Ok(format.into());
    }
    args.output
        .extension()
        .and_then(|extension| extension.to_str())
        .and_then(OutputFormat::from_extension)
        .ok_or(CliError::UndeterminedFormat)
}

/// Stream the input through the hub and write the output(s) as they arrive.
/// Returns the detected container + chosen format for the caller to report.
pub fn run(args: Args) -> CliResult<(Container, OutputFormat)> {
    let format = resolve_format(&args)?;
    let mut data_identity = DataIdentityMap::new();
    for argument in &args.data_identity {
        let (track, identity) = parse_data_identity_argument(argument)?;
        data_identity.insert(track, identity);
    }
    let opts = Opts {
        format,
        segment_duration: args.segment_duration,
        low_latency: args.ll,
        utc_timing_url: args.utc_timing_url.clone(),
        tracks: args.tracks.iter().copied().map(TrackId::new).collect(),
        max_whole_input_bytes: args.max_whole_input_bytes,
        data_identity,
    };
    let in_path = input_path(&args).to_path_buf();
    let from_stdin = in_path == Path::new(STDIN_INPUT);
    let to_stdout = args.output == Path::new(STDIN_INPUT);
    let manifest = matches!(
        format,
        OutputFormat::Hls | OutputFormat::TsHls | OutputFormat::Dash
    );
    let mut emit: Box<dyn Emit> = if to_stdout {
        if manifest {
            return Err(CliError::ManifestToStdout);
        }
        Box::new(StdoutEmit::new())
    } else {
        Box::new(FileEmit::new(&args.output))
    };

    #[cfg(feature = "cenc")]
    let keys = if args.decrypt {
        Some(decrypt_keys::key_map(&args.keys)?)
    } else {
        None
    };
    #[cfg(not(feature = "cenc"))]
    let keys = no_keys();

    let container;
    if from_stdin {
        if format == OutputFormat::Progressive {
            return Err(crate::Error::ProgressiveNeedsReplayableInput.into());
        }
        let mut stdin = std::io::stdin();
        let (detected, prefix) = probe_reader(&mut stdin)?;
        container = detected;
        #[cfg(feature = "cenc")]
        if keys.is_some() && container != Container::Mp4 {
            return Err(crate::Error::InvalidInput(
                "--decrypt only applies to CENC-protected MP4/CMAF input",
            )
            .into());
        }
        let mut reader = PrefixReader {
            prefix,
            pos: 0,
            inner: stdin,
        };
        run_forward(&mut reader, container, &opts, &mut *emit, keys)?;
    } else {
        let mut file = File::open(&in_path)?;
        let len = file.metadata().ok().map(|metadata| metadata.len());
        let (detected, _prefix) = probe_reader(&mut file)?;
        container = detected;
        #[cfg(feature = "cenc")]
        if keys.is_some() && container != Container::Mp4 {
            return Err(crate::Error::InvalidInput(
                "--decrypt only applies to CENC-protected MP4/CMAF input",
            )
            .into());
        }
        rewind(&mut file)?;
        run_seekable(&mut file, container, len, &opts, &mut *emit, keys)?;
    }
    Ok((container, format))
}

/// Content-key handling for `--decrypt`.
#[cfg(feature = "cenc")]
mod decrypt_keys {
    use super::{CliResult, parse_key};
    use crate::cenc_decrypt::KeyMap;

    /// Parse every `--key <KID>:<key>` argument into a [`KeyMap`].
    pub(super) fn key_map(keys: &[String]) -> CliResult<KeyMap> {
        let mut map = KeyMap::new();
        for spec in keys {
            let (kid, key) = parse_key(spec)?;
            map.insert(kid, key);
        }
        Ok(map)
    }
}

/// The no-`--decrypt` key set.
#[cfg(feature = "cenc")]
fn no_keys() -> DecryptKeys {
    None
}

/// The no-`--decrypt` key set (the `cenc` feature is off, so there is none).
#[cfg(not(feature = "cenc"))]
fn no_keys() -> DecryptKeys {
    None
}

// ---------------------------------------------------------------------------
// CENC decrypt (feature-gated)
// ---------------------------------------------------------------------------

/// Parse a `<32-hex-KID>:<32-hex-key>` string into a `(kid, key)` byte pair.
#[cfg(feature = "cenc")]
fn parse_key(spec: &str) -> CliResult<([u8; KEY_ID_LEN], [u8; KEY_ID_LEN])> {
    let (kid_hex, key_hex) = spec
        .split_once(':')
        .ok_or_else(|| CliError::BadKey(redact_key_spec(spec)))?;
    let kid = parse_hex16(kid_hex).ok_or_else(|| CliError::BadKey(redact_key_spec(spec)))?;
    let key = parse_hex16(key_hex).ok_or_else(|| CliError::BadKey(redact_key_spec(spec)))?;
    Ok((kid, key))
}

/// Parse exactly [`KEY_ID_HEX_LEN`] hex chars into a [`KEY_ID_LEN`]-byte array.
#[cfg(feature = "cenc")]
fn parse_hex16(s: &str) -> Option<[u8; KEY_ID_LEN]> {
    let mut out = [0u8; KEY_ID_LEN];
    broadcast_common::hex::hex_decode_into(s.trim(), &mut out).ok()?;
    Some(out)
}

#[cfg(test)]
mod data_identity_tests {
    use super::{parse_data_identity, parse_data_identity_argument};
    use crate::ir::{DataCarriage, DataIdentity};

    #[test]
    fn every_documented_form_parses() {
        assert_eq!(
            parse_data_identity("ts:6"),
            Some(DataIdentity::Ts {
                stream_type: 6,
                descriptors: Vec::new(),
                carriage: DataCarriage::Pes,
            })
        );
        assert_eq!(
            parse_data_identity("ts:134:0A0B"),
            Some(DataIdentity::Ts {
                stream_type: 134,
                descriptors: vec![0x0A, 0x0B],
                carriage: DataCarriage::Pes,
            })
        );
        assert_eq!(
            parse_data_identity("mp4:meta:0000001467706d64"),
            Some(DataIdentity::Mp4 {
                handler: *b"meta",
                sample_entry: bytes::Bytes::from_static(&[0, 0, 0, 0x14, b'g', b'p', b'm', b'd']),
            })
        );
        assert_eq!(
            parse_data_identity("matroska:S_TEXT/UTF8:4142"),
            Some(DataIdentity::Matroska {
                codec_id: String::from("S_TEXT/UTF8"),
                codec_private: bytes::Bytes::from_static(&[0x41, 0x42]),
            })
        );
    }

    #[test]
    fn malformed_forms_are_none_or_a_typed_error() {
        assert_eq!(parse_data_identity("ts"), None);
        assert_eq!(parse_data_identity("ts:256"), None);
        assert_eq!(parse_data_identity("ts:6:0"), None);
        assert_eq!(parse_data_identity("mp4:met:00"), None);
        assert_eq!(parse_data_identity("mp4:meta:"), None);
        assert_eq!(parse_data_identity("matroska:"), None);
        assert_eq!(parse_data_identity("mkv:x"), None);
        assert!(parse_data_identity_argument("not-a-track=ts:6").is_err());
        assert!(parse_data_identity_argument("4").is_err());
        assert_eq!(
            parse_data_identity_argument("4=ts:6").expect("valid"),
            (
                crate::TrackId::new(4),
                DataIdentity::Ts {
                    stream_type: 6,
                    descriptors: Vec::new(),
                    carriage: DataCarriage::Pes,
                }
            )
        );
    }
}

#[cfg(all(test, feature = "cenc"))]
mod parse_hex16_tests {
    use super::parse_hex16;

    /// 32 BYTES but not 32 hex chars: `"a" + 15 × "é" + "b"`. The old
    /// `&s[i * 2..i * 2 + 2]` slice cuts the first `é` in half and panics.
    #[test]
    fn non_ascii_input_of_the_right_byte_length_is_none_not_a_panic() {
        let s = format!("a{}b", "é".repeat(15));
        assert_eq!(s.len(), 32);
        assert_eq!(parse_hex16(&s), None);
    }

    /// `u8::from_str_radix("+1", 16)` is `Ok(1)`, so the old parser accepted a
    /// sign in place of a hex digit.
    #[test]
    fn a_sign_is_not_a_hex_digit() {
        assert_eq!(parse_hex16(&"+1".repeat(16)), None);
        assert_eq!(parse_hex16(&"-1".repeat(16)), None);
    }

    #[test]
    fn valid_key_still_parses_and_whitespace_is_trimmed() {
        let hex = "0102030405060708090a0b0c0d0e0f10";
        assert_eq!(
            parse_hex16(&format!("  {hex}\n")),
            Some([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16])
        );
        assert_eq!(parse_hex16("0102"), None);
    }
}

#[cfg(all(test, feature = "cenc"))]
mod key_redaction_tests {
    //! W8/W-CSA-4 adversarial review: `CliError::BadKey` must never carry the
    //! key half of a malformed `--key` argument, through any of the three
    //! ways `parse_key` can reject one.
    use super::*;

    const KID_HEX: &str = "0102030405060708090a0b0c0d0e0f10";
    const KEY_HEX: &str = "aabbccddeeff00112233445566778899";

    #[test]
    fn missing_separator_reports_length_only() {
        // No `:` at all — `kid_part` (the whole string) is not 32 hex chars.
        let spec = format!("{KID_HEX}{KEY_HEX}");
        let err = parse_key(&spec).unwrap_err();
        let display = format!("{err}");
        let debug = format!("{err:?}");
        assert!(
            !display.contains(KEY_HEX),
            "Display leaked key hex: {display}"
        );
        assert!(!debug.contains(KEY_HEX), "Debug leaked key hex: {debug}");
        assert!(
            !display.contains(KID_HEX),
            "no separator means no verified KID half either"
        );
    }

    #[test]
    fn malformed_kid_half_reports_length_only() {
        // KID half is present but not valid hex — must not fall back to
        // printing the raw string (which would still carry the key half).
        let spec = format!("not-32-hex-chars:{KEY_HEX}");
        let err = parse_key(&spec).unwrap_err();
        let display = format!("{err}");
        let debug = format!("{err:?}");
        assert!(
            !display.contains(KEY_HEX),
            "Display leaked key hex: {display}"
        );
        assert!(!debug.contains(KEY_HEX), "Debug leaked key hex: {debug}");
    }

    #[test]
    fn malformed_key_half_still_redacts_and_keeps_the_kid() {
        // KID half is valid; only the key half is malformed. The KID is not
        // secret and may appear; the (malformed) key half must not.
        let bad_key_half = "not-32-hex-chars-either";
        let spec = format!("{KID_HEX}:{bad_key_half}");
        let err = parse_key(&spec).unwrap_err();
        let display = format!("{err}");
        let debug = format!("{err:?}");
        assert!(
            !display.contains(bad_key_half),
            "Display leaked the key half: {display}"
        );
        assert!(
            !debug.contains(bad_key_half),
            "Debug leaked the key half: {debug}"
        );
        assert!(
            display.contains(KID_HEX),
            "the KID half is not secret: {display}"
        );
    }

    #[test]
    fn well_formed_key_parses_without_going_through_bad_key() {
        let spec = format!("{KID_HEX}:{KEY_HEX}");
        assert!(parse_key(&spec).is_ok());
    }

    #[test]
    fn unsupported_container_message_has_no_whitespace_run() {
        let msg = format!("{}", CliError::UnsupportedContainer("ogg"));
        assert_eq!(
            msg,
            "unsupported input container ogg: this CLI demuxes MPEG-TS, \
             MP4/CMAF, MPEG-PS, WebM/Matroska and FLV"
        );
    }
}
