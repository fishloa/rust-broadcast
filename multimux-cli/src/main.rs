//! CLI for the `multimux` live RTSP -> LL-HLS just-in-time repackaging HTTP
//! origin.
//!
//! Either point it at a JSON config file describing one or more routes, or
//! use the single-route quick start (`--rtsp` + `--name`) for a single
//! source. See `multimux`'s README for the served endpoint table and v1 scope.
//!
//! # Example
//!
//! ```bash
//! multimux --rtsp rtsp://cam.local/stream --name cam1
//! multimux --config routes.json
//! ```

use std::path::PathBuf;

use clap::Parser;
use multimux::config::{Config, InputSpec, Route};
use multimux::output::OutputKind;
use multimux::{MultimuxError, Result};

#[derive(Parser)]
#[command(
    name = "multimux",
    version,
    about = "Live RTSP -> LL-HLS just-in-time repackaging HTTP origin",
    long_about = "Pulls one or more live RTSP sources and serves each as LL-HLS \
                  (RFC 8216bis) from an in-process HTTP origin.\n\
                  Either point it at a JSON config file (--config) describing one or \
                  more routes, or use the single-route quick start (--rtsp + --name)."
)]
struct Cli {
    /// JSON config file describing routes + segmentation/window/bind parameters.
    #[arg(long, value_name = "FILE", conflicts_with_all = ["rtsp", "name"])]
    config: Option<PathBuf>,

    /// Single-route quick start: RTSP source URL to pull (requires --name).
    #[arg(long, value_name = "URL", requires = "name")]
    rtsp: Option<String>,

    /// Single-route quick start: served stream name, i.e. the URL path
    /// segment (requires --rtsp).
    #[arg(long, value_name = "NAME", requires = "rtsp")]
    name: Option<String>,

    /// `host:port` the HTTP origin binds.
    #[arg(long, value_name = "ADDR", default_value_t = Config::default().bind)]
    bind: String,

    /// Target full-segment duration, in seconds.
    #[arg(long, value_name = "SECS", default_value_t = Config::default().target_duration_secs)]
    target_duration: f64,

    /// LL-HLS part target, in milliseconds.
    #[arg(long, value_name = "MS", default_value_t = Config::default().part_target_ms)]
    part_ms: u32,

    /// Rolling window depth: full segments retained in RAM.
    #[arg(long, value_name = "N", default_value_t = Config::default().window_segments)]
    window: usize,

    /// Single-route quick start: which delivery protocol(s) to serve the
    /// ingested stream as, comma-separated (`llhls`, `dash`) — issue #663 P4:
    /// one ingest, many outputs. Ignored when `--config` is used (a config
    /// file sets `outputs` per route; see `multimux::config::Route::outputs`).
    #[arg(
        long,
        value_name = "LIST",
        value_delimiter = ',',
        default_value = "llhls",
        conflicts_with = "dash"
    )]
    outputs: Vec<String>,

    /// Single-route quick start shorthand for `--outputs llhls,dash` (serve
    /// LL-HLS *and* DASH from the same ingest).
    #[arg(long)]
    dash: bool,
}

/// Parse `--outputs`'s comma-separated tokens into [`OutputKind`]s (or
/// resolve the `--dash` shorthand to `[llhls, dash]`) — the CLI's own
/// mapping of the wire tokens `multimux::config`'s serde `OutputKind` already
/// accepts in a JSON config's `outputs` array, kept in sync with those exact
/// token spellings (`"llhls"`/`"dash"`) rather than re-deriving them.
fn parse_outputs(cli: &Cli) -> Result<Vec<OutputKind>> {
    if cli.dash {
        return Ok(vec![OutputKind::LlHls, OutputKind::Dash]);
    }
    cli.outputs
        .iter()
        .map(|s| match s.trim() {
            "llhls" => Ok(OutputKind::LlHls),
            "dash" => Ok(OutputKind::Dash),
            other => Err(MultimuxError::ConfigInvalid {
                field: "outputs",
                reason: format!("unknown output kind {other:?} (expected llhls or dash)"),
            }),
        })
        .collect()
}

/// Build a [`Config`] from the parsed CLI: `--config <FILE>` if given,
/// otherwise the single-route quick start built from `--rtsp`/`--name` plus
/// the bind/timing/window flags.
fn build_config(cli: Cli) -> Result<Config> {
    if let Some(path) = cli.config {
        return Config::from_json_file(&path);
    }
    // Computed before any field of `cli` is moved out below (a shared
    // reference to the whole struct is only valid pre-move).
    let outputs = parse_outputs(&cli)?;
    let rtsp_url = cli.rtsp.ok_or_else(|| MultimuxError::ConfigInvalid {
        field: "rtsp",
        reason: "either --config <FILE> or --rtsp <URL> --name <NAME> is required".into(),
    })?;
    // clap's `requires = "name"` on `--rtsp` guarantees `cli.name` is present
    // whenever `cli.rtsp` is.
    let name = cli
        .name
        .expect("clap requires --name whenever --rtsp is given");

    let config = Config {
        bind: cli.bind,
        target_duration_secs: cli.target_duration,
        part_target_ms: cli.part_ms,
        window_segments: cli.window,
        routes: vec![Route {
            name,
            input: InputSpec::Rtsp { url: rtsp_url },
            outputs,
        }],
    };
    config.validate()?;
    Ok(config)
}

/// Initializes the process-wide `tracing` subscriber: human-readable output
/// on stderr, filtered by `RUST_LOG` (`EnvFilter` syntax, e.g.
/// `RUST_LOG=multimux=debug`), defaulting to `info` when unset. Only the
/// binary does this — the `multimux` library only ever emits `tracing`
/// events, never installs a subscriber itself, so it composes into whatever
/// host process embeds it.
fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();
}

#[tokio::main]
async fn main() {
    init_tracing();
    if let Err(e) = run().await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();
    let config = build_config(cli)?;
    multimux::origin::serve(config).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn quick_start_flags_build_a_single_route_config() {
        let cli = Cli::parse_from([
            "multimux",
            "--rtsp",
            "rtsp://cam.local/stream",
            "--name",
            "cam1",
        ]);
        let cfg = build_config(cli).unwrap();
        assert_eq!(cfg.routes.len(), 1);
        assert_eq!(cfg.routes[0].name, "cam1");
        match &cfg.routes[0].input {
            InputSpec::Rtsp { url } => assert_eq!(url, "rtsp://cam.local/stream"),
            other => panic!("expected InputSpec::Rtsp, got {other:?}"),
        }
    }

    /// Quick start with no `--outputs`/`--dash` defaults to LL-HLS only —
    /// matches `Route::outputs`'s own default, so an existing invocation's
    /// behaviour is unchanged (issue #663 P4).
    #[test]
    fn quick_start_defaults_to_llhls_only() {
        let cli = Cli::parse_from([
            "multimux",
            "--rtsp",
            "rtsp://cam.local/stream",
            "--name",
            "cam1",
        ]);
        let cfg = build_config(cli).unwrap();
        assert_eq!(cfg.routes[0].outputs, vec![OutputKind::LlHls]);
    }

    /// `--dash` is the shorthand for "both outputs from the same ingest".
    #[test]
    fn dash_flag_selects_llhls_and_dash() {
        let cli = Cli::parse_from([
            "multimux",
            "--rtsp",
            "rtsp://cam.local/stream",
            "--name",
            "cam1",
            "--dash",
        ]);
        let cfg = build_config(cli).unwrap();
        assert_eq!(
            cfg.routes[0].outputs,
            vec![OutputKind::LlHls, OutputKind::Dash]
        );
    }

    /// `--outputs llhls,dash` is the explicit spelling of the same thing.
    #[test]
    fn outputs_flag_parses_comma_separated_list() {
        let cli = Cli::parse_from([
            "multimux",
            "--rtsp",
            "rtsp://cam.local/stream",
            "--name",
            "cam1",
            "--outputs",
            "llhls,dash",
        ]);
        let cfg = build_config(cli).unwrap();
        assert_eq!(
            cfg.routes[0].outputs,
            vec![OutputKind::LlHls, OutputKind::Dash]
        );
    }

    /// An unknown `--outputs` token is a config error, not a silent no-op.
    #[test]
    fn outputs_flag_rejects_unknown_token() {
        let cli = Cli::parse_from([
            "multimux",
            "--rtsp",
            "rtsp://cam.local/stream",
            "--name",
            "cam1",
            "--outputs",
            "lldash",
        ]);
        assert!(build_config(cli).is_err());
    }

    #[test]
    fn cli_definition_is_valid() {
        // Guards against a malformed clap derive (conflicts/requires wiring).
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }
}
