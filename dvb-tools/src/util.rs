//! Shared helpers for `dvb-tools` subcommands.

use std::process::ExitCode;

use mpeg_ts::resync::TsResync;

/// Read a file into memory. On `Err`, prints to stderr and returns `Err`.
pub(crate) fn read_file(path: &str, tool: &str) -> Result<Vec<u8>, ExitCode> {
    match std::fs::read(path) {
        Ok(d) => Ok(d),
        Err(e) => {
            eprintln!("{tool}: {path}: {e}");
            Err(ExitCode::FAILURE)
        }
    }
}

/// Yield every aligned 188-byte packet from `data`, performing TS byte-stream
/// resynchronisation via [`TsResync`].  Supports both 188-byte and 204-byte
/// (RS-coded) streams; the parity bytes are stripped automatically.
///
/// Pulled into a helper so subcommands stay focused on the per-table view
/// rather than the TS framing loop.
pub(crate) fn for_each_packet(
    data: &[u8],
) -> impl Iterator<Item = [u8; mpeg_ts::ts::TS_PACKET_SIZE]> {
    let mut r = TsResync::new();
    r.feed(data).into_iter()
}
