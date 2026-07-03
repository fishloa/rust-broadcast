//! `transmux` binary entry point — the any-to-any media packager CLI (issue
//! #482).
//!
//! Thin `clap` front-end: parse [`Args`](transmux::cli::Args), call
//! [`transmux::cli::run`], report the detected container + chosen format to
//! stderr, and exit non-zero on error. All logic lives in
//! [`transmux::cli`] (built only under the `cli` feature).
//!
//! Follows the workspace CLI standard — see `docs/CLI-STANDARD.md`.

#[cfg(feature = "cli")]
fn main() -> std::process::ExitCode {
    use clap::Parser;
    use transmux::cli::{run, Args};

    let args = Args::parse();
    match run(args) {
        Ok((container, format)) => {
            eprintln!("transmux: {container} → {format}");
            std::process::ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("transmux: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(not(feature = "cli"))]
fn main() {
    // The library is `no_std`; the binary requires the `cli` feature, so this
    // stub only exists to satisfy the `[[bin]]` target when the feature is off.
    eprintln!("transmux: build with `--features cli` to use the command-line packager");
}
