//! `media-doctor` — media diagnostics harness for DVB / MPEG-2 Transport Streams.
//!
//! An extensible lint-style analysis framework: individual [`Diagnostic`]s each
//! check one rule against a TS byte-stream, producing [`Finding`]s that feed into
//! a [`Report`]. A trivial built-in [`SyncByteCheck`] proves the
//! harness.
//!
//! # Feature flags
//!
//! | Feature | Default | Description |
//! |---|---|---|
//! | `std`   | yes     | `std::error::Error` impls |
//! | `serde` | yes     | JSON report output via `serde` / `serde_json` |
//! | `cli`   | yes     | `clap`-based CLI binary |
//!
//! # Quick start (library)
//!
//! ```rust
//! use media_doctor::{Diagnostic, Report, SyncByteCheck};
//!
//! let mut report = Report::new();
//! let diag = SyncByteCheck;
//! diag.run(&[0x47, 0x00, 0x00, 0x10], &mut report);
//! assert!(report.findings().is_empty());
//! ```

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]

extern crate alloc;

mod container_codec;
mod diagnostics;
mod playlist;
mod report;

pub use container_codec::check_container_codec;
pub use diagnostics::cc_anomaly::CcAnomalyCheck;
pub use diagnostics::codec_signalling::CodecSignallingCheck;
pub use diagnostics::fps_cadence::FpsCadenceCheck;
pub use diagnostics::interlace::InterlaceCheck;
pub use diagnostics::param_sets::ParamSetsCheck;
pub use diagnostics::pat_pmt_version::PatPmtVersionCheck;
pub use diagnostics::pcr_check::PcrCheck;
pub use diagnostics::pts_check::PtsCheck;
pub use diagnostics::scte35_check::Scte35Check;
pub use diagnostics::sync_byte::SyncByteCheck;
pub use playlist::check_playlist;
pub use report::{Finding, Location, Report, Severity};

/// A pluggable diagnostic check that examines a Transport Stream byte buffer.
///
/// Implementors receive the full TS byte slice (contiguous 188-byte packets, no
/// framing gaps — `ts.len()` is a multiple of 188) and push any findings into
/// `report`.
pub trait Diagnostic {
    /// Check a TS byte buffer, appending findings to `report`.
    fn run(&self, ts: &[u8], report: &mut Report);
}

/// Run every registered [`Diagnostic`] against a TS buffer.
///
/// This is the simplest harness runner: it feeds the buffer through each
/// diagnostic in order. A streaming version will follow in a later story.
pub fn run_all(ts: &[u8], diagnostics: &[&dyn Diagnostic], report: &mut Report) {
    for diag in diagnostics {
        diag.run(ts, report);
    }
}

#[cfg(feature = "cli")]
pub mod cli;
