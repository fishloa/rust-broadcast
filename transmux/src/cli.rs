//! `transmux` command-line packager — the any-to-any hub as a CLI (issue #482).
//!
//! Wires the existing demux spokes ([`TsDemux`], [`Fmp4Demux`], [`PsDemux`],
//! [`WebmDemux`], [`FlvDemux`]) through the neutral [`Media`] IR into the
//! existing mux spokes ([`CmafMux`], [`TsHlsPackager`],
//! [`DashPackager`], [`ProgressiveMux`], [`TsMux`]).
//!
//! It writes **no** new demux/mux logic — it is a front-end that autodetects the
//! input container, runs it through the hub, and writes the chosen output.
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

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use broadcast_common::{Package, Unpackage};

use crate::dash::DashPackager;
use crate::flv::FlvDemux;
use crate::media::{CmafMux, Fmp4Demux, Media};
use crate::progressive::ProgressiveMux;
use crate::ps_demux::PsDemux;
use crate::segmenter::{MediaClock, Segmenter};
use crate::ts_demux::TsDemux;
use crate::ts_hls::TsHlsPackager;
use crate::ts_mux::TsMux;
use crate::webm_demux::WebmDemux;

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

/// Default LL-DASH target latency in milliseconds (`Latency@target`), used when
/// `--ll` selects the DASH low-latency profile.
const LL_DASH_LATENCY_TARGET_MS: u32 = 3000;
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
    use std::time::{SystemTime, UNIX_EPOCH};

    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Days since the Unix epoch → civil date (Howard Hinnant's algorithm).
    let days = secs / SECS_PER_DAY;
    let rem = secs % SECS_PER_DAY;
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let (y, mo, d) = civil_from_days(days as i64);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

/// Seconds in a day (the clock does not need leap seconds here).
const SECS_PER_DAY: u64 = 86_400;

/// Convert days since 1970-01-01 to a `(year, month, day)` civil date.
/// Howard Hinnant's `civil_from_days` (public domain, exact for all i64 days).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

// ---------------------------------------------------------------------------
// Detected container + output format
// ---------------------------------------------------------------------------

/// The input container as recognised by [`detect_container`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Container {
    /// MPEG-2 Transport Stream → [`TsDemux`].
    MpegTs,
    /// ISO-BMFF fragmented MP4 / CMAF → [`Fmp4Demux`].
    Mp4,
    /// MPEG Program Stream → [`PsDemux`].
    MpegPs,
    /// WebM / Matroska (EBML) → [`WebmDemux`].
    WebM,
    /// FLV (Flash Video) → [`FlvDemux`].
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
    /// CMAF/fMP4 single init+media segment → [`CmafMux`].
    Cmaf,
    /// CMAF-HLS media playlist (`.m3u8`) → CMAF segments via [`Segmenter`].
    Hls,
    /// Classic HLS with MPEG-2 TS media segments → [`TsHlsPackager`].
    TsHls,
    /// DASH MPD manifest → [`DashPackager`].
    Dash,
    /// MPEG-2 Transport Stream → [`TsMux`].
    Ts,
    /// Progressive single-file MP4 → [`ProgressiveMux`].
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
    /// May be given positionally or with `-i/--input`.
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
            .field("tracks", &self.tracks);
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
#[cfg_attr(not(feature = "cenc"), allow(dead_code))]
fn redact_key_spec(spec: &str) -> String {
    let kid_part = spec.split_once(':').map(|(k, _)| k).unwrap_or(spec);
    let is_kid_hex = kid_part.len() == 32 && kid_part.bytes().all(|b| b.is_ascii_hexdigit());
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
#[derive(Debug)]
#[non_exhaustive]
pub enum CliError {
    /// Reading the input or writing the output failed.
    Io(std::io::Error),
    /// A demux or mux spoke rejected the media.
    Transmux(crate::Error),
    /// The input container could not be recognised from its leading bytes.
    UnknownContainer,
    /// The prober identified a container this CLI has no demuxer for.
    UnsupportedContainer(&'static str),
    /// `--utc-timing-url` was given without `--ll`. `MPD/UTCTiming` is modelled
    /// only on the low-latency packager, so the flag would otherwise be dropped
    /// silently.
    UtcTimingNeedsLl,
    /// No `-f/--format` was given and none could be inferred from the output
    /// path extension.
    UndeterminedFormat,
    /// The requested track-ID selection left no tracks.
    NoTracksSelected,
    /// A `--key` argument was malformed. Carries `redact_key_spec`'s
    /// output, never the raw argument — the key half must never reach an
    /// error message, `Display`, or (this enum derives `Debug`) a `dbg!`.
    BadKey(String),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::Io(e) => write!(f, "i/o error: {e}"),
            CliError::Transmux(e) => write!(f, "transmux error: {e}"),
            CliError::UnknownContainer => write!(
                f,
                "unknown input container: leading bytes match no supported format \
                 (MPEG-TS, MP4/CMAF, MPEG-PS, WebM, FLV)"
            ),
            CliError::UnsupportedContainer(name) => write!(
                f,
                "unsupported input container {name}: this CLI demuxes MPEG-TS,                  MP4/CMAF, MPEG-PS, WebM/Matroska and FLV"
            ),
            CliError::UtcTimingNeedsLl => write!(
                f,
                "--utc-timing-url needs --ll (MPD/UTCTiming is emitted only for the low-latency DASH profile)"
            ),
            CliError::UndeterminedFormat => write!(
                f,
                "output format not given and not inferable from the output extension; \
                 pass -f/--format"
            ),
            CliError::NoTracksSelected => {
                write!(f, "the --tracks selection matched no tracks in the input")
            }
            CliError::BadKey(s) => {
                write!(f, "invalid --key ({s}): expected <32-hex-KID>:<32-hex-key>")
            }
        }
    }
}

impl std::error::Error for CliError {}

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
    use container_probe::Format;

    let format = match container_probe::probe(data) {
        container_probe::Probe::Identified { format, .. } => format,
        // A tie or an inconclusive read is not a decision: the caller (or a
        // user) must supply the format explicitly.
        _ => return Err(CliError::UnknownContainer),
    };

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
// Core: bytes → Media → bytes/text
// ---------------------------------------------------------------------------

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
    pub tracks: Vec<u32>,
}

impl Default for Opts {
    fn default() -> Self {
        Self {
            format: OutputFormat::Cmaf,
            segment_duration: 6,
            low_latency: false,
            utc_timing_url: None,
            tracks: Vec::new(),
        }
    }
}

/// Demux `input` (container autodetected) into the hub IR, apply track
/// selection, then package it into `opts.format`. Pure function over bytes: no
/// filesystem access, so it is the unit under test.
pub fn run_bytes(input: &[u8], opts: &Opts) -> CliResult<Output> {
    let container = detect_container(input)?;
    let mut media = demux(container, input)?;
    if !opts.tracks.is_empty() {
        media
            .tracks
            .retain(|t| opts.tracks.contains(&t.spec.track_id));
        if media.tracks.is_empty() {
            return Err(CliError::NoTracksSelected);
        }
    }
    package(&media, opts)
}

/// Dispatch to the correct demux spoke for `container`.
fn demux(container: Container, input: &[u8]) -> CliResult<Media> {
    let media = match container {
        Container::MpegTs => TsDemux::new().unpackage(input)?,
        Container::Mp4 => Fmp4Demux::new().unpackage(input)?,
        Container::MpegPs => PsDemux::new().unpackage(input)?,
        Container::WebM => WebmDemux::new().unpackage(input)?,
        // FlvDemux has its own error type; map it into the library Error.
        Container::Flv => FlvDemux::new()
            .unpackage(input)
            .map_err(|e| crate::Error::InvalidInput(flv_reason(e)))?,
    };
    Ok(media)
}

/// Map an FLV demux failure to a stable static reason (the library `Error`
/// variant takes `&'static str`).
fn flv_reason(_e: crate::flv::FlvError) -> &'static str {
    "FLV demux failed"
}

/// Drop tracks a fMP4/CMAF-based mux entry point cannot carry (opaque
/// `CodecConfig::Data` / `CodecConfig::Subtitle` — see
/// `CodecConfig::is_muxable_in_bmff`), warning on stderr which ones — so the
/// CLI does not fail on ordinary real-world input: a real DVB multiplex
/// routinely carries DVB subtitles/teletext/ANC/SCTE-35 as opaque `Data`
/// tracks (issue: "the CLI must not fail on normal input", media plane
/// step-2 fix wave 1). Every fMP4/CMAF-based output format
/// (Cmaf/Progressive/Hls/Dash) filters through this before muxing; Ts/TsHls
/// carry `Data` tracks verbatim and are left unfiltered.
///
/// # Errors
/// [`Error::InvalidInput`](crate::Error::InvalidInput) if filtering would
/// leave no track at all.
fn filter_for_bmff_mux(media: &Media) -> CliResult<Media> {
    let dropped: Vec<u32> = media
        .tracks
        .iter()
        .filter(|t| !t.spec.config.is_muxable_in_bmff())
        .map(|t| t.spec.track_id)
        .collect();
    if dropped.is_empty() {
        return Ok(media.clone());
    }
    eprintln!(
        "warning: dropping track(s) {dropped:?} from CMAF/fMP4 output \
         (no ISOBMFF carriage in this crate for their codec)"
    );
    Ok(media.select_tracks_by(|t| t.spec.config.is_muxable_in_bmff())?)
}

/// Split `media` into one init segment plus its `styp`+`moof`+`mdat` media
/// segments, cutting on the anchor track's keyframes at roughly
/// `target_secs` seconds each ([`Segmenter`]).
///
/// The media segments carry **no** `moov` — exactly what ISO/IEC 23009-1
/// §6.3.4.2 and CMAF require of a media segment (audit r05-W29): the old CLI
/// handed every Representation the whole `CmafMux` artifact (`ftyp`+`moov`+
/// `moof`+`mdat`) under both the init and the media name.
///
/// # Errors
/// Propagates [`Segmenter`]'s errors (no anchor-capable track, all samples
/// un-timed, …).
fn segment_media(media: &Media, target_secs: u32) -> CliResult<(Vec<u8>, Vec<Vec<u8>>)> {
    // At least one whole second: a sub-second target would cut a segment per
    // keyframe at best, and the CLI flag is documented in whole seconds.
    let target = f64::from(target_secs.max(1));
    let specs: Vec<crate::pipeline::TrackSpec> =
        media.tracks.iter().map(|t| t.spec.clone()).collect();
    let mut seg = Segmenter::new(specs, media.movie_timescale, target)?;
    let init = seg.init_segment()?;

    // Feed every track's samples in decode order, merged by decode time so the
    // segmenter sees a single interleaved stream.
    //
    // The merge key is the sample's decode time **in the track's own
    // timescale**, so samples of tracks with different timescales (90 kHz video
    // vs 44.1 kHz audio) must be compared by cross-multiplication, not by their
    // raw tick values: 44 100 audio ticks and 90 000 video ticks are both "one
    // second", so sorting the raw numbers put an entire audio stream *before*
    // the video it belongs with. The old raw-tick sort did exactly that, and
    // every segment after the first lost its audio (audit fix wave 2, item 1).
    let origin = crate::media::TimelineOrigin::of(media.tracks.iter());
    let mut merged: Vec<(OrderedKey, usize, usize)> = Vec::new();
    for (ti, t) in media.tracks.iter().enumerate() {
        let scale = t.spec.timescale.max(1);
        for si in 0..t.samples.len() {
            let sample = &t.samples[si];
            // Section-carried samples have no dts; they belong to the segment
            // their neighbours are in, so they sort just after their own
            // track's last timed sample (`OrderedKey::Untimed`).
            let key = match sample.dts {
                Some(dts) => OrderedKey::Timed {
                    // Relative to the common origin, in ticks, kept as a
                    // rational `(ticks, timescale)` so two tracks' keys compare
                    // exactly.
                    ticks: dts.saturating_sub(origin.in_timescale(scale)),
                    timescale: scale,
                },
                None => OrderedKey::Untimed,
            };
            merged.push((key, ti, si));
        }
    }
    merged.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));

    for (_, ti, si) in merged {
        let track_id = media.tracks[ti].spec.track_id;
        seg.push(track_id, media.tracks[ti].samples[si].clone())?;
    }
    seg.flush()?;
    Ok((init, seg.take_ready()))
}

/// A media segment plus its measured presentation duration, in that order.
type MeasuredSegment = (Vec<u8>, f64);

/// One track's DASH segmentation: `(sample range, tfdt)` per segment.
type TrackSegments = Vec<(core::ops::Range<usize>, u64)>;

/// Segment `media` **and** measure what came out: the init segment, the media
/// segments, and each segment's presentation duration in seconds.
///
/// The duration is read from the produced bytes ([`segment_secs`]) rather than
/// recomputed from the input, so a playlist's `#EXTINF` and the file it names
/// can never disagree — the earlier "presentation span divided evenly over the
/// segment count" gave every `#EXTINF` the same value (1.52 s) while the real
/// segments were ~2.0 s and ~1.04 s, and the mismatched sum broke
/// `#EXT-X-TARGETDURATION` with it (audit fix wave 2, item 2).
///
/// # Errors
/// Propagates [`segment_media`]'s errors, and fails on a segment whose duration
/// cannot be parsed (never a fabricated zero).
fn segment_media_measured(
    media: &Media,
    target_secs: u32,
) -> CliResult<(Vec<u8>, Vec<MeasuredSegment>)> {
    let (init, segments) = segment_media(media, target_secs)?;
    let mut out = Vec::with_capacity(segments.len());
    for seg in segments {
        let secs = segment_secs(&seg, media)?;
        out.push((seg, secs));
    }
    Ok((init, out))
}

/// Per-track segmentation of `media`, aligned to one shared set of boundaries.
///
/// The boundaries come from running the **whole** `media` through
/// [`segment_media`] once, so the crate's own cut rule (an anchor-track sync
/// sample once the buffered duration has reached the target) decides them, and
/// every track is then split at exactly those instants. Each produced segment's
/// `traf` records how many samples of each track it carries, which is what this
/// reads back — the DASH Representations therefore stay aligned
/// (`segmentAlignment="true"`, ISO/IEC 23009-1 §5.3.7.2) instead of each track
/// cutting at its own times (audit fix wave 2, item 5).
///
/// # Errors
/// Propagates [`segment_media`]'s errors, and
/// [`Error::InvalidInput`](crate::Error::InvalidInput) for a segment whose
/// fragment boxes cannot be parsed.
fn segment_track_ranges(media: &Media, target_secs: u32) -> CliResult<Vec<TrackSegments>> {
    let (_init, segments) = segment_media(media, target_secs)?;

    // The boundary instants, taken from the **anchor** track's cuts: the
    // segmenter places one `traf` per track in each segment, and the anchor's
    // `tfdt` sequence is the shared timeline every other track must follow.
    let configs: Vec<&crate::pipeline::CodecConfig> =
        media.tracks.iter().map(|t| &t.spec.config).collect();
    let anchor_idx = crate::segmenter::choose_anchor(configs.into_iter())?;
    let anchor_id = media.tracks[anchor_idx].spec.track_id;

    // Boundary instants in the anchor's own timescale, derived from the
    // anchor's sample durations (the same rule the segmenter accumulates with),
    // by counting how many of its samples each produced segment carries.
    let mut boundaries_anchor: Vec<u64> = Vec::with_capacity(segments.len());
    let mut acc = 0u64;
    let mut clock = MediaClock::new();
    let mut cursor = 0usize;
    for seg in &segments {
        let present = segment_track_fragments(seg)?;
        let count = present
            .iter()
            .find(|(id, _, _)| *id == anchor_id)
            .map(|(_, c, _)| *c as usize)
            .unwrap_or(0);
        for s in media.tracks[anchor_idx]
            .samples
            .iter()
            .skip(cursor)
            .take(count)
        {
            acc += clock.tick(s);
        }
        cursor += count;
        boundaries_anchor.push(acc);
    }
    let anchor_scale = u64::from(media.tracks[anchor_idx].spec.timescale.max(1));

    // Per track: split at those same instants. A sample belongs to the segment
    // whose half-open start-time interval contains it, so a segment's samples
    // are the run beginning before its end boundary; `tfdt` is the boundary
    // itself, converted into this track's timescale — the same instant as the
    // anchor's, which is what `segmentAlignment="true"` means
    // (ISO/IEC 23009-1 §5.3.7.2).
    let mut per_track: Vec<Vec<(core::ops::Range<usize>, u64)>> =
        Vec::with_capacity(media.tracks.len());
    for t in media.tracks.iter() {
        let scale = u64::from(t.spec.timescale.max(1));
        let mut ranges = Vec::with_capacity(boundaries_anchor.len());
        let mut start_idx = 0usize;
        let mut clock = MediaClock::new();
        let mut elapsed = 0u64;
        // `boundaries_anchor` holds each segment's cumulative *end*; a segment's
        // `tfdt` is the previous end (0 for the first), i.e. the instant the
        // segment starts — the value that must agree across Representations.
        let mut prev_end_anchor = 0u64;
        for &end_anchor in &boundaries_anchor {
            // The segment's start instant, in this track's timescale (exact
            // when the anchor's timescale divides it; otherwise rounded down,
            // which keeps the declared start no later than the real one).
            let start_ticks = prev_end_anchor.saturating_mul(scale) / anchor_scale;
            let end_ticks = end_anchor.saturating_mul(scale) / anchor_scale;
            let mut i = start_idx;
            while i < t.samples.len() && elapsed < end_ticks {
                elapsed += clock.tick(&t.samples[i]);
                i += 1;
            }
            // Always progress: an empty track range would make the segment
            // unframeable, and a track that has run out simply contributes its
            // remaining samples to the last segment it appears in.
            if i == start_idx && i < t.samples.len() {
                elapsed += clock.tick(&t.samples[i]);
                i += 1;
            }
            ranges.push((start_idx..i, start_ticks));
            start_idx = i;
            prev_end_anchor = end_anchor;
        }
        if start_idx < t.samples.len() {
            // Anything past the last boundary joins the final segment (the
            // anchor's span is defined by its own samples, which can end a
            // fraction of a frame before a slower track's).
            if let Some(last) = ranges.last_mut() {
                last.0.end = t.samples.len();
            }
        }
        per_track.push(ranges);
    }
    Ok(per_track)
}

/// `(track_id, sample_count, tfdt)` for every `traf` in a media segment's
/// `moof`.
///
/// # Errors
/// [`Error::InvalidInput`](crate::Error::InvalidInput) when the segment's boxes
/// cannot be parsed.
fn segment_track_fragments(segment: &[u8]) -> CliResult<Vec<(u32, u32, u64)>> {
    use crate::box_types::parse_box;
    use crate::movie_fragment::MovieFragmentBox;

    let mut remaining = segment;
    while !remaining.is_empty() {
        let (bx, consumed) = parse_box(remaining)
            .map_err(|_| crate::Error::InvalidInput("media segment box could not be parsed"))?;
        if bx.header.box_type.is(b"moof") {
            let moof = MovieFragmentBox::parse_body(bx.body)
                .map_err(|_| crate::Error::InvalidInput("moof box could not be parsed"))?;
            let mut out = Vec::with_capacity(moof.traf.len());
            for traf in &moof.traf {
                let count: u32 = traf
                    .trun
                    .iter()
                    .map(|r| u32::try_from(r.samples.len()).unwrap_or(u32::MAX))
                    .fold(0u32, u32::saturating_add);
                let tfdt = traf
                    .tfdt
                    .as_ref()
                    .map(|t| t.base_media_decode_time())
                    .unwrap_or(0);
                out.push((traf.tfhd.track_id, count, tfdt));
            }
            return Ok(out);
        }
        if consumed == 0 || consumed > remaining.len() {
            break;
        }
        remaining = &remaining[consumed..];
    }
    Err(
        crate::Error::InvalidInput("media segment carries no moof: cannot derive its track layout")
            .into(),
    )
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
/// no parseable `moof`/`trun` (a zero duration would be a lie, and
/// [`segment_ticks`]'s old `unwrap_or(0)` silently produced one).
fn segment_secs(segment: &[u8], media: &Media) -> CliResult<f64> {
    use crate::box_types::parse_box;
    use crate::movie_fragment::MovieFragmentBox;

    let mut remaining = segment;
    while !remaining.is_empty() {
        let (bx, consumed) = parse_box(remaining)
            .map_err(|_| crate::Error::InvalidInput("media segment box could not be parsed"))?;
        if bx.header.box_type.is(b"moof") {
            let moof = MovieFragmentBox::parse_body(bx.body)
                .map_err(|_| crate::Error::InvalidInput("moof box could not be parsed"))?;
            let mut longest = 0.0f64;
            for traf in &moof.traf {
                let scale = media
                    .tracks
                    .iter()
                    .find(|t| t.spec.track_id == traf.tfhd.track_id)
                    .map(|t| f64::from(t.spec.timescale.max(1)))
                    .ok_or(crate::Error::InvalidInput(
                        "media segment names a track not present in the input",
                    ))?;
                let mut ticks = 0u64;
                for run in &traf.trun {
                    for s in &run.samples {
                        ticks += u64::from(s.sample_duration.unwrap_or(0));
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

/// An interleaving key for one sample: its decode time relative to a common
/// origin, as an exact rational (`ticks` over `timescale`) so two tracks with
/// different timescales order correctly (90 000/90 000 == 44 100/44 100).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OrderedKey {
    /// A sample with a decode time.
    Timed {
        /// Decode ticks relative to the common origin, in `timescale`.
        ticks: i64,
        /// That track's media timescale (`mdhd.timescale`, ISO/IEC 14496-12
        /// §8.4.2), always `>= 1`.
        timescale: u32,
    },
    /// A sample with no decode time (a section-carried sample): it follows
    /// every timed sample.
    Untimed,
}

impl Ord for OrderedKey {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        use core::cmp::Ordering;
        match (self, other) {
            (
                OrderedKey::Timed {
                    ticks: a,
                    timescale: at,
                },
                OrderedKey::Timed {
                    ticks: b,
                    timescale: bt,
                },
            ) => {
                // Exact cross-multiplication as `i128`: two `u32` timescales and
                // two `i64` times fit with room to spare.
                (*a as i128 * *bt as i128).cmp(&(*b as i128 * *at as i128))
            }
            (OrderedKey::Untimed, OrderedKey::Untimed) => Ordering::Equal,
            (OrderedKey::Untimed, OrderedKey::Timed { .. }) => Ordering::Greater,
            (OrderedKey::Timed { .. }, OrderedKey::Untimed) => Ordering::Less,
        }
    }
}

impl PartialOrd for OrderedKey {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// The largest `trun` sample-duration sum in a media segment, in its track's
/// media timescale — the segment's own decoded span.
///
/// Read from the produced bytes (the authoritative values) rather than
/// recomputed, so a manifest's declared duration and the file it addresses can
/// never disagree.
///
/// # Errors
/// [`Error::InvalidInput`](crate::Error::InvalidInput) when no `moof`/`trun`
/// can be parsed. The caller must not fall back to zero: a zero
/// `SegmentTemplate@duration` is a lie that makes the MPD unplayable (audit fix
/// wave 2, item 5).
fn segment_ticks(segment: &[u8]) -> CliResult<u64> {
    use crate::box_types::parse_box;
    use crate::movie_fragment::MovieFragmentBox;

    // Top-level walk: the first `moof` (a media segment is `[styp] moof mdat`).
    let mut remaining = segment;
    while !remaining.is_empty() {
        let (bx, consumed) = parse_box(remaining)
            .map_err(|_| crate::Error::InvalidInput("media segment box could not be parsed"))?;
        if bx.header.box_type.is(b"moof") {
            let moof = MovieFragmentBox::parse_body(bx.body)
                .map_err(|_| crate::Error::InvalidInput("moof box could not be parsed"))?;
            let mut longest = 0u64;
            for traf in &moof.traf {
                let mut ticks = 0u64;
                for run in &traf.trun {
                    for s in &run.samples {
                        ticks += u64::from(s.sample_duration.unwrap_or(0));
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

/// Render the CMAF-HLS media playlist for `segments`: one `#EXTINF` per segment,
/// each pointing at `seg{i}.m4s` (a media-only `styp`+`moof`+`mdat` file) with
/// an `#EXT-X-MAP` naming the init segment.
///
/// Each `#EXTINF` is the segment's **own** measured duration —
/// `segment_media_measured` reads it back out of the bytes — and
/// `#EXT-X-TARGETDURATION` is the ceiling of the largest of them (RFC 8216
/// §4.3.3.1: "an integer ... equal to or greater than the EXTINF duration of
/// each Media Segment"). Dividing the presentation span evenly by the segment
/// count instead gave 1.52 s for segments that really ran ~2.0 s and ~1.04 s.
///
/// # Errors
/// [`Error::InvalidInput`](crate::Error::InvalidInput) if a duration does not
/// fit [`broadcast_hls::DecimalSeconds`], or if there are no segments.
fn render_cmaf_hls_playlist(segments: &[(Vec<u8>, f64)]) -> CliResult<String> {
    use broadcast_hls::{DecimalSeconds, MediaPlaylist, MediaSegment};

    if segments.is_empty() {
        return Err(
            crate::Error::InvalidInput("cannot render an HLS playlist with no segments").into(),
        );
    }

    let segs = segments
        .iter()
        .enumerate()
        .map(|(i, (_, secs))| {
            Ok(MediaSegment {
                uri: format!("seg{i}.m4s"),
                duration: DecimalSeconds::new(*secs).map_err(|_| {
                    crate::Error::InvalidInput("segment duration must be finite and non-negative")
                })?,
                discontinuous: false,
                // Apple's HLS authoring spec requires an #EXT-X-MAP on every
                // fMP4 segment; the init file is written alongside them.
                map: Some(broadcast_hls::MapTag {
                    uri: String::from(HLS_INIT_FILE),
                    byte_range: None,
                    extra_attrs: Vec::new(),
                }),
                ..Default::default()
            })
        })
        .collect::<CliResult<Vec<_>>>()?;

    // RFC 8216 §4.3.3.1: EXT-X-TARGETDURATION must be an integer at least the
    // largest EXTINF. `ceil` of the max, floored at 1 s.
    let max_secs = segments
        .iter()
        .map(|(_, s)| *s)
        .fold(0.0f64, f64::max)
        .max(1.0);
    let target_duration = u32::try_from(max_secs.ceil() as u64).unwrap_or(u32::MAX);

    let playlist = MediaPlaylist {
        version: HLS_CMAF_VERSION,
        target_duration,
        media_sequence: 0,
        discontinuity_sequence: 0,
        segments: segs,
        endlist: true,
        ..Default::default()
    };
    playlist
        .to_m3u8()
        .map_err(|e| crate::Error::HlsAttrValue(e).into())
}

/// Dispatch to the correct mux spoke for `opts.format`.
fn package(media: &Media, opts: &Opts) -> CliResult<Output> {
    match opts.format {
        OutputFormat::Cmaf => {
            let media = filter_for_bmff_mux(media)?;
            Ok(Output::Bytes(CmafMux::new(1).package(&media)?))
        }
        OutputFormat::Progressive => {
            let media = filter_for_bmff_mux(media)?;
            Ok(Output::Bytes(ProgressiveMux::new(true).package(&media)?))
        }
        OutputFormat::Ts => Ok(Output::Bytes(TsMux::new().package(media)?)),
        OutputFormat::Hls => {
            // CMAF-HLS, segmented at `--segment-duration` (audit r05-W29): each
            // segment is a self-initializing CMAF artifact (init + one media
            // segment) — the shape Apple's authoring spec wants — and the
            // playlist carries one `#EXTINF` per segment, so a player advances
            // through the whole presentation rather than replaying one blob.
            let media = filter_for_bmff_mux(media)?;
            let (init, measured) = segment_media_measured(&media, opts.segment_duration)?;
            let text = render_cmaf_hls_playlist(&measured)?;

            // The init file the playlist's `#EXT-X-MAP` names, plus the media
            // segments **as media only** (`styp`+`moof`+`mdat`). A segment must
            // not repeat `ftyp`/`moov`: the `#EXT-X-MAP` already supplies the
            // Media Initialization Section, and a segment carrying its own copy
            // makes Apple's validator fail ("Error injecting segment data") and
            // MP4Box report a duplicate `ftyp` (audit fix wave 2, item 2).
            let mut files: Vec<(String, Vec<u8>)> =
                alloc::vec![(String::from(HLS_INIT_FILE), init)];
            files.extend(
                measured
                    .iter()
                    .enumerate()
                    .map(|(i, (seg, _))| (format!("seg{i}.m4s"), seg.clone())),
            );
            Ok(Output::Manifest {
                text,
                segments: files,
            })
        }
        OutputFormat::TsHls => {
            let out = TsHlsPackager::new(opts.segment_duration).package(media)?;
            let segments = out
                .segments
                .into_iter()
                .enumerate()
                .map(|(i, bytes)| (format!("seg{i}.ts"), bytes))
                .collect();
            Ok(Output::Manifest {
                text: out.playlist,
                segments,
            })
        }
        OutputFormat::Dash => {
            let media = filter_for_bmff_mux(media)?;

            // Segment every Representation first, single-track (a
            // Representation's Segments must carry only its own track —
            // ISO/IEC 23009-1 §5.3.9.1, issue #614), recording each track's
            // per-segment durations so the MPD's `SegmentTemplate@duration`
            // matches the files actually written (audit r05-W29).
            //
            // `Segmenter` emits `styp`+`moof`+`mdat` media segments — no `moov`,
            // which ISO/IEC 23009-1 §6.3.4.2 forbids inside a media segment and
            // which the old whole-file `CmafMux` output put in every one.
            let mut segments: Vec<(String, Vec<u8>)> = Vec::new();
            let mut track_segments: Vec<crate::dash::TrackSegments> = Vec::new();

            // One shared set of boundaries for every Representation, so the
            // `segmentAlignment="true"` the MPD declares is true
            // (ISO/IEC 23009-1 §5.3.7.2). Segmented per track instead, video cut
            // at 2.0 s and audio at 2.02 s (audit fix wave 2, item 5).
            let layout = segment_track_ranges(&media, opts.segment_duration)?;

            for (ti, t) in media.tracks.iter().enumerate() {
                let id = t.spec.track_id;
                // A Representation carries exactly one track, so its init
                // segment is that track's own `moov` (ISO/IEC 23009-1 §5.3.9.1).
                segments.push((
                    format!("init-stream{id}.m4s"),
                    crate::pipeline::build_init_segment(
                        core::slice::from_ref(&t.spec),
                        media.movie_timescale,
                    )?,
                ));

                let mut durations = Vec::with_capacity(layout[ti].len());
                for (seg_no, (range, tfdt)) in layout[ti].iter().enumerate() {
                    let samples = &t.samples[range.clone()];
                    let bytes = crate::build_media_segment(
                        u32::try_from(seg_no + 1).unwrap_or(u32::MAX),
                        &[crate::FragmentTrackData::new(id, *tfdt, samples)],
                    )?;
                    // The real duration, never a fabricated zero.
                    durations.push(segment_ticks(&bytes)?);
                    segments.push((format!("chunk-stream{id}-{}.m4s", seg_no + 1), bytes));
                }
                track_segments.push(crate::dash::TrackSegments {
                    track_id: id,
                    durations,
                });
            }

            // The CLI writes **static files**, so every MPD it emits describes
            // a `static` presentation even under `--ll`: the LL-DASH
            // availability signalling (`availabilityTimeOffset`,
            // `availabilityTimeComplete=false`) is what makes the chunks
            // requestable early, and a `dynamic` MPD over files that will never
            // grow only computes a live edge a player then can't resolve. The
            // library's `LlDashPackager` still defaults to `dynamic` for a live
            // origin; the CLI overrides it.
            //
            // `UTCTiming` is emitted only when the caller names a source
            // (`--utc-timing-url`): a `static` MPD needs none, and an offline
            // artefact must not bake in a third-party time server.
            let text = if opts.low_latency {
                // LL-DASH: chunk = half the segment.
                let seg = opts.segment_duration.max(1) as f64;
                let mut pkg = crate::ll_dash::LlDashPackager::new(
                    seg,
                    seg / 2.0,
                    LL_DASH_LATENCY_TARGET_MS,
                    availability_start_time_now(),
                )?;
                pkg.base.dynamic = false;
                pkg.base.availability_start_time = None;
                pkg.base.segments = track_segments;
                if let Some(url) = &opts.utc_timing_url {
                    pkg =
                        pkg.with_utc_timing(crate::ll_dash::UTCTIMING_HTTP_HEAD_2014, url.clone());
                }
                pkg.package(&media)?
            } else {
                if opts.utc_timing_url.is_some() {
                    // `UTCTiming` is not modelled on the plain whole-segment
                    // packager; say so instead of silently dropping it.
                    return Err(CliError::UtcTimingNeedsLl);
                }
                DashPackager {
                    segments: track_segments,
                    ..DashPackager::default()
                }
                .package(&media)?
            };
            Ok(Output::Manifest { text, segments })
        }
    }
}

// ---------------------------------------------------------------------------
// I/O driver (called by main.rs)
// ---------------------------------------------------------------------------

/// Resolve the input path from the positional or `-i` flag.
fn input_path(args: &Args) -> &Path {
    // clap guarantees exactly one is present (required_unless_present +
    // conflicts_with).
    args.in_positional
        .as_deref()
        .or(args.input.as_deref())
        .expect("clap requires one of <IN> or --input")
}

/// Resolve the output format from `-f` or the output extension.
fn resolve_format(args: &Args) -> CliResult<OutputFormat> {
    if let Some(f) = args.format {
        return Ok(f.into());
    }
    args.output
        .extension()
        .and_then(|e| e.to_str())
        .and_then(OutputFormat::from_extension)
        .ok_or(CliError::UndeterminedFormat)
}

/// Read the input, run the hub, and write the output(s) to disk. Returns the
/// detected container + chosen format for the caller to report.
pub fn run(args: Args) -> CliResult<(Container, OutputFormat)> {
    let in_path = input_path(&args).to_path_buf();
    let format = resolve_format(&args)?;
    let input = fs::read(&in_path)?;
    let container = detect_container(&input)?;

    let opts = Opts {
        format,
        segment_duration: args.segment_duration,
        low_latency: args.ll,
        utc_timing_url: args.utc_timing_url.clone(),
        tracks: args.tracks.clone(),
    };

    #[cfg(feature = "cenc")]
    let media_bytes;
    #[cfg(feature = "cenc")]
    let input_ref: &[u8] = if args.decrypt {
        media_bytes = decrypt_input(&input, container, &args.keys)?;
        &media_bytes
    } else {
        &input
    };
    #[cfg(not(feature = "cenc"))]
    let input_ref: &[u8] = &input;

    let out = run_bytes(input_ref, &opts)?;
    write_output(&args.output, out)?;
    Ok((container, format))
}

/// Write the packaged output. For a manifest, the text goes to `out_path` and
/// each segment is written alongside it (same parent directory).
fn write_output(out_path: &Path, out: Output) -> CliResult<()> {
    match out {
        Output::Bytes(b) => {
            fs::write(out_path, b)?;
        }
        Output::Manifest { text, segments } => {
            if let Some(parent) = out_path.parent()
                && !parent.as_os_str().is_empty()
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(out_path, text)?;
            let dir = out_path.parent().unwrap_or_else(|| Path::new("."));
            for (name, bytes) in segments {
                fs::write(dir.join(name), bytes)?;
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// CENC decrypt (feature-gated)
// ---------------------------------------------------------------------------

/// Decrypt CENC-protected fMP4 input into a fresh unprotected fMP4 the demuxers
/// can consume. The `keys` are `<KID-hex>:<key-hex>` pairs.
#[cfg(feature = "cenc")]
fn decrypt_input(input: &[u8], container: Container, keys: &[String]) -> CliResult<Vec<u8>> {
    use broadcast_common::Decrypt;

    if container != Container::Mp4 {
        return Err(CliError::Transmux(crate::Error::InvalidInput(
            "--decrypt only applies to CENC-protected MP4/CMAF input",
        )));
    }
    let mut key_map = crate::cenc_decrypt::KeyMap::new();
    for spec in keys {
        let (kid, key) = parse_key(spec)?;
        key_map.insert(kid, key);
    }
    let decryptor = crate::cenc_decrypt::CencDecryptor::from_fmp4(input)?;
    let mut media = decryptor.demux()?;
    decryptor.decrypt(&mut media, &key_map)?;
    // Re-package the now-cleartext samples as fMP4 so autodetect + demux run on a
    // clean container. Filter first (media plane step-2 fix wave 1): CmafMux
    // now rejects a non-carriable track (e.g. an encrypted subtitle track)
    // instead of silently dropping it.
    let media = filter_for_bmff_mux(&media)?;
    Ok(CmafMux::new(1).package(&media)?)
}

/// Parse a `<32-hex-KID>:<32-hex-key>` string into a `(kid, key)` byte pair.
#[cfg(feature = "cenc")]
fn parse_key(spec: &str) -> CliResult<([u8; 16], [u8; 16])> {
    let (kid_hex, key_hex) = spec
        .split_once(':')
        .ok_or_else(|| CliError::BadKey(redact_key_spec(spec)))?;
    let kid = parse_hex16(kid_hex).ok_or_else(|| CliError::BadKey(redact_key_spec(spec)))?;
    let key = parse_hex16(key_hex).ok_or_else(|| CliError::BadKey(redact_key_spec(spec)))?;
    Ok((kid, key))
}

/// Parse exactly 32 hex chars into a 16-byte array.
#[cfg(feature = "cenc")]
fn parse_hex16(s: &str) -> Option<[u8; 16]> {
    let s = s.trim();
    if s.len() != 32 {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
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
}
