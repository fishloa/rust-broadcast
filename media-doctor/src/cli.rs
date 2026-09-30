//! CLI subcommands and entry-point (feature `cli`).

/// `media-doctor check-hls` — run HLS playlist validation.
#[derive(clap::Parser, Debug)]
pub struct CheckHlsArgs {
    /// Input HLS playlist file (.m3u8).
    #[arg(short = 'i', long = "input")]
    pub input: String,

    /// Emit JSON report on stdout instead of human text.
    #[arg(long = "json")]
    pub json: bool,
}

/// `media-doctor check-dash` — run DASH MPD validation.
#[derive(clap::Parser, Debug)]
pub struct CheckDashArgs {
    /// Input DASH MPD file (.mpd).
    #[arg(short = 'i', long = "input")]
    pub input: String,

    /// Emit JSON report on stdout instead of human text.
    #[arg(long = "json")]
    pub json: bool,
}

/// `media-doctor check` — run diagnostics against a TS file.
#[derive(clap::Parser, Debug)]
pub struct CheckArgs {
    /// Input Transport Stream file.
    #[arg(short = 'i', long = "input")]
    pub input: String,

    /// Emit JSON report on stdout instead of human text.
    #[arg(long = "json")]
    pub json: bool,
}

/// `media-doctor watch` — continuously ingest a live UDP MPEG-TS feed and
/// expose Prometheus metrics (issue #665). **UDP only** in this release: SRT
/// ingest (`srt-runtime`) is a follow-up, not yet implemented.
#[derive(clap::Parser, Debug)]
pub struct WatchArgs {
    /// UDP address to listen on for raw MPEG-TS, e.g. `0.0.0.0:5000` for
    /// unicast or `239.1.1.1:5000` for multicast (auto-joins the multicast
    /// group when the address is in the IPv4 multicast range).
    #[arg(long = "udp")]
    pub udp: String,

    /// HTTP address to serve Prometheus metrics on (`GET /metrics`).
    #[arg(long = "metrics-addr", default_value = "127.0.0.1:9090")]
    pub metrics_addr: String,

    /// Maximum metrics connections served concurrently. A connection beyond
    /// this is answered `503` and closed immediately, so a flood of idle
    /// clients cannot exhaust the process's threads.
    #[arg(long = "metrics-max-conns", default_value_t = DEFAULT_METRICS_MAX_CONNS)]
    pub metrics_max_conns: usize,

    /// Total time a metrics connection may take, from accept to response
    /// written, before it is dropped (milliseconds). A *total* deadline, not
    /// a per-read timeout: a peer that dribbles one byte at a time cannot
    /// hold a connection open indefinitely.
    #[arg(long = "metrics-io-timeout-ms", default_value_t = DEFAULT_METRICS_IO_TIMEOUT_MS)]
    pub metrics_io_timeout_ms: u64,
}

/// Default concurrent-connection cap for the metrics endpoint.
pub const DEFAULT_METRICS_MAX_CONNS: usize = 32;

/// Default total per-connection deadline for the metrics endpoint, in
/// milliseconds.
pub const DEFAULT_METRICS_IO_TIMEOUT_MS: u64 = 5_000;

/// Top-level CLI.
#[derive(clap::Parser, Debug)]
#[command(
    name = "media-doctor",
    version,
    about = "DVB/MPEG-TS diagnostics harness"
)]
#[non_exhaustive]
pub enum Cli {
    /// Run diagnostic checks against a Transport Stream.
    Check(CheckArgs),
    /// Validate an HLS playlist (.m3u8).
    CheckHls(CheckHlsArgs),
    /// Validate a DASH MPD (.mpd).
    CheckDash(CheckDashArgs),
    /// Continuously ingest a live UDP MPEG-TS feed, serving Prometheus metrics.
    Watch(WatchArgs),
}
