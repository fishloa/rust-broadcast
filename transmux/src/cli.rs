//! `transmux` command-line packager — the any-to-any hub as a CLI (issue #482).
//!
//! Wires the existing demux spokes ([`TsDemux`], [`Fmp4Demux`], [`PsDemux`],
//! [`WebmDemux`], [`FlvDemux`]) through the neutral [`Media`] IR into the
//! existing mux spokes ([`CmafMux`], [`HlsPackager`], [`TsHlsPackager`],
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
use crate::media::{CmafMux, Fmp4Demux, HlsPackager, Media};
use crate::progressive::ProgressiveMux;
use crate::ps_demux::PsDemux;
use crate::ts_demux::TsDemux;
use crate::ts_hls::TsHlsPackager;
use crate::ts_mux::TsMux;
use crate::webm_demux::WebmDemux;

// ---------------------------------------------------------------------------
// Container signatures (no magic numbers — every byte pattern is named).
// ---------------------------------------------------------------------------

/// MPEG-2 TS sync byte (ISO/IEC 13818-1 §2.4.3.2): every 188-byte packet starts
/// with `0x47`.
const TS_SYNC_BYTE: u8 = 0x47;
/// MPEG-2 TS packet length in bytes (ISO/IEC 13818-1 §2.4.3.2).
const TS_PACKET_LEN: usize = 188;
/// MPEG Program Stream `pack_start_code` (ISO/IEC 13818-1 §2.5.3.3): the 4-byte
/// prefix `00 00 01 BA` opening a pack header.
const PS_PACK_START_CODE: [u8; 4] = [0x00, 0x00, 0x01, 0xBA];
/// EBML header magic (RFC 8794 §4 / Matroska): a WebM/MKV file begins with the
/// EBML element ID `1A 45 DF A3`.
const EBML_MAGIC: [u8; 4] = [0x1A, 0x45, 0xDF, 0xA3];
/// FLV file signature (Adobe Flash Video §E.2): the first three bytes are
/// `"FLV"`.
const FLV_SIGNATURE: [u8; 3] = *b"FLV";
/// ISO base media file `size`+`fourcc` box header length: the 4-byte fourcc that
/// identifies the container lives at byte offset 4 (ISO/IEC 14496-12 §4.2).
const BOX_FOURCC_OFFSET: usize = 4;
/// ISO-BMFF top-level box fourccs that identify an MP4/CMAF stream at the head of
/// a file (ISO/IEC 14496-12 §4.3 `ftyp` / §8.16.2 `styp` / §8.2.1 `moov` /
/// §8.8.4 `moof`).
const ISOBMFF_LEADING_FOURCCS: [[u8; 4]; 4] = [*b"ftyp", *b"styp", *b"moov", *b"moof"];

/// Default LL-DASH target latency in milliseconds (`Latency@target`), used when
/// `--ll` selects the DASH low-latency profile.
const LL_DASH_LATENCY_TARGET_MS: u32 = 3000;
/// Placeholder `MPD@availabilityStartTime` for `--ll` DASH output (the CLI does
/// not synthesise a real wall-clock time; a downstream origin would set it).
const LL_DASH_AVAILABILITY_START: &str = "1970-01-01T00:00:00Z";

// ---------------------------------------------------------------------------
// Detected container + output format
// ---------------------------------------------------------------------------

/// The input container as recognised by [`detect_container`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
pub enum OutputFormat {
    /// CMAF/fMP4 single init+media segment → [`CmafMux`].
    Cmaf,
    /// CMAF-HLS media playlist (`.m3u8`) → [`HlsPackager`].
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
#[derive(Debug, clap::Parser)]
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

/// clap `ValueEnum` mirror of [`OutputFormat`] (kebab-case flag values).
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
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
pub enum CliError {
    /// Reading the input or writing the output failed.
    Io(std::io::Error),
    /// A demux or mux spoke rejected the media.
    Transmux(crate::Error),
    /// The input container could not be recognised from its leading bytes.
    UnknownContainer,
    /// No `-f/--format` was given and none could be inferred from the output
    /// path extension.
    UndeterminedFormat,
    /// The requested track-ID selection left no tracks.
    NoTracksSelected,
    /// A `--key` argument was malformed.
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
            CliError::UndeterminedFormat => write!(
                f,
                "output format not given and not inferable from the output extension; \
                 pass -f/--format"
            ),
            CliError::NoTracksSelected => {
                write!(f, "the --tracks selection matched no tracks in the input")
            }
            CliError::BadKey(s) => {
                write!(f, "invalid --key {s:?}: expected <32-hex-KID>:<32-hex-key>")
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

/// Recognise the input container from its leading bytes.
///
/// Detection order and signatures (all named consts above):
/// - FLV: `"FLV"` at offset 0.
/// - WebM/Matroska: EBML magic `1A 45 DF A3` at offset 0.
/// - MPEG-PS: `pack_start_code` `00 00 01 BA` at offset 0.
/// - MP4/CMAF: an ISO-BMFF box fourcc (`ftyp`/`styp`/`moov`/`moof`) at offset 4.
/// - MPEG-TS: sync byte `0x47` at offset 0 **and** at offset 188.
///
/// Returns [`CliError::UnknownContainer`] if nothing matches.
pub fn detect_container(data: &[u8]) -> CliResult<Container> {
    if data.len() >= FLV_SIGNATURE.len() && data[..FLV_SIGNATURE.len()] == FLV_SIGNATURE {
        return Ok(Container::Flv);
    }
    if data.len() >= EBML_MAGIC.len() && data[..EBML_MAGIC.len()] == EBML_MAGIC {
        return Ok(Container::WebM);
    }
    if data.len() >= PS_PACK_START_CODE.len()
        && data[..PS_PACK_START_CODE.len()] == PS_PACK_START_CODE
    {
        return Ok(Container::MpegPs);
    }
    if data.len() >= BOX_FOURCC_OFFSET + 4 {
        let fourcc = &data[BOX_FOURCC_OFFSET..BOX_FOURCC_OFFSET + 4];
        if ISOBMFF_LEADING_FOURCCS.iter().any(|f| f == fourcc) {
            return Ok(Container::Mp4);
        }
    }
    // TS: sync at 0 and one packet later (guards against a stray 0x47).
    if data.first() == Some(&TS_SYNC_BYTE) && data.get(TS_PACKET_LEN) == Some(&TS_SYNC_BYTE) {
        return Ok(Container::MpegTs);
    }
    Err(CliError::UnknownContainer)
}

// ---------------------------------------------------------------------------
// Core: bytes → Media → bytes/text
// ---------------------------------------------------------------------------

/// The packaged output of a run: raw bytes for binary formats, or (for HLS/DASH)
/// a text manifest plus the referenced media segments.
#[derive(Debug)]
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
    /// Track-ID selection; empty = all tracks.
    pub tracks: Vec<u32>,
}

impl Default for Opts {
    fn default() -> Self {
        Self {
            format: OutputFormat::Cmaf,
            segment_duration: 6,
            low_latency: false,
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

/// Dispatch to the correct mux spoke for `opts.format`.
fn package(media: &Media, opts: &Opts) -> CliResult<Output> {
    match opts.format {
        OutputFormat::Cmaf => Ok(Output::Bytes(CmafMux::new(1).package(media)?)),
        OutputFormat::Progressive => Ok(Output::Bytes(ProgressiveMux::new(true).package(media)?)),
        OutputFormat::Ts => Ok(Output::Bytes(TsMux::new().package(media)?)),
        OutputFormat::Hls => {
            // CMAF-HLS: a media playlist plus the init+media CMAF segment it maps
            // each track to. HlsPackager emits `{prefix}{track_id}.m4s` URIs; we
            // supply one CMAF artifact per playlist so referenced segments exist.
            let text = HlsPackager::default().package(media)?;
            let cmaf = CmafMux::new(1).package(media)?;
            // The default HlsPackager names segments `seg{track_id}.m4s`; emit the
            // combined CMAF under each referenced name so the playlist resolves.
            let segments = media
                .tracks
                .iter()
                .map(|t| (format!("seg{}.m4s", t.spec.track_id), cmaf.clone()))
                .collect();
            Ok(Output::Manifest { text, segments })
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
            let text = if opts.low_latency {
                // LL-DASH: chunk = half the segment; a placeholder wall-clock
                // availabilityStartTime (the CLI does not synthesise real UTC).
                let seg = opts.segment_duration.max(1) as f64;
                crate::ll_dash::LlDashPackager::new(
                    seg,
                    seg / 2.0,
                    LL_DASH_LATENCY_TARGET_MS,
                    LL_DASH_AVAILABILITY_START,
                )?
                .package(media)?
            } else {
                DashPackager::default().package(media)?
            };
            // DASH SegmentTemplate references init/chunk files per representation;
            // emit one CMAF artifact per track under both the init and first-chunk
            // names the default templates produce.
            let cmaf = CmafMux::new(1).package(media)?;
            let mut segments = Vec::new();
            for t in &media.tracks {
                let id = t.spec.track_id;
                segments.push((format!("init-stream{id}.m4s"), cmaf.clone()));
                segments.push((format!("chunk-stream{id}-1.m4s"), cmaf.clone()));
            }
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
            if let Some(parent) = out_path.parent() {
                if !parent.as_os_str().is_empty() {
                    fs::create_dir_all(parent)?;
                }
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
    // clean container.
    Ok(CmafMux::new(1).package(&media)?)
}

/// Parse a `<32-hex-KID>:<32-hex-key>` string into a `(kid, key)` byte pair.
#[cfg(feature = "cenc")]
fn parse_key(spec: &str) -> CliResult<([u8; 16], [u8; 16])> {
    let (kid_hex, key_hex) = spec
        .split_once(':')
        .ok_or_else(|| CliError::BadKey(spec.to_string()))?;
    let kid = parse_hex16(kid_hex).ok_or_else(|| CliError::BadKey(spec.to_string()))?;
    let key = parse_hex16(key_hex).ok_or_else(|| CliError::BadKey(spec.to_string()))?;
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
