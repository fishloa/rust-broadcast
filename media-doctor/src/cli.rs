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
    /// group when the address is in the IPv4 or IPv6 multicast range).
    #[arg(long = "udp")]
    pub udp: String,

    /// HTTP address to serve Prometheus metrics on (`GET /metrics`).
    #[arg(long = "metrics-addr", default_value = "127.0.0.1:9090")]
    pub metrics_addr: String,

    /// Maximum metrics connections served concurrently. A connection beyond
    /// this is closed immediately at accept, so a flood of idle clients
    /// cannot exhaust the process's tasks or file descriptors.
    #[arg(long = "metrics-max-conns", default_value_t = DEFAULT_METRICS_MAX_CONNS)]
    pub metrics_max_conns: usize,

    /// Total time a metrics connection may take, from accept to response
    /// written, before it is dropped (milliseconds). A *total* deadline, not
    /// a per-read timeout: a peer that dribbles one byte at a time cannot
    /// hold a connection open indefinitely.
    #[arg(long = "metrics-io-timeout-ms", default_value_t = DEFAULT_METRICS_IO_TIMEOUT_MS)]
    pub metrics_io_timeout_ms: u64,

    /// Requested `SO_RCVBUF` for the UDP socket, in bytes (the kernel may cap
    /// it; a warning is printed when it grants less than requested).
    #[arg(long = "udp-rcvbuf")]
    pub udp_rcvbuf: Option<usize>,

    /// Set `SO_REUSEADDR` on the UDP socket (lets several probes share a
    /// multicast port).
    #[arg(long = "udp-reuse-addr")]
    pub udp_reuse_addr: bool,

    /// Interface for the multicast join: an IPv4 address (IPv4 groups) or an
    /// interface index (IPv6 groups).
    #[arg(long = "udp-interface")]
    pub udp_interface: Option<String>,
}

impl WatchArgs {
    /// The UDP socket settings from `--udp-rcvbuf`, `--udp-reuse-addr` and
    /// `--udp-interface`.
    ///
    /// # Errors
    ///
    /// A message naming `--udp-interface` when its value is neither an IPv4
    /// address nor an interface index.
    pub fn udp_config(&self) -> Result<crate::udp::UdpConfig, String> {
        use crate::udp::{MulticastInterface, UdpConfig};
        let interface = match self.udp_interface.as_deref() {
            None => None,
            Some(s) => Some(
                s.parse::<std::net::Ipv4Addr>()
                    .map(MulticastInterface::V4)
                    .or_else(|_| s.parse::<u32>().map(MulticastInterface::V6Index))
                    .map_err(|_| {
                        format!(
                            "invalid --udp-interface {s:?}: expected an IPv4 address or an interface index"
                        )
                    })?,
            ),
        };
        Ok(UdpConfig {
            recv_buffer: self.udp_rcvbuf,
            reuse_addr: self.udp_reuse_addr,
            interface,
        })
    }
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
