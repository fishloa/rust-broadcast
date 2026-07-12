//! CLI subcommands and entry-point (feature `cli`).

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
}

/// Top-level CLI.
#[derive(clap::Parser, Debug)]
#[command(
    name = "media-doctor",
    version,
    about = "DVB/MPEG-TS diagnostics harness"
)]
pub enum Cli {
    /// Run diagnostic checks against a Transport Stream.
    Check(CheckArgs),
    /// Continuously ingest a live UDP MPEG-TS feed, serving Prometheus metrics.
    Watch(WatchArgs),
}
