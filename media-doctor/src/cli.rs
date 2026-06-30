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
}
