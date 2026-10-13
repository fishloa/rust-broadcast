//! Shared helpers for `dvb-tools` subcommands.

use std::fs::File;
use std::io::Read;
use std::process::ExitCode;

use mpeg_ts::resync::TsResync;
use mpeg_ts::ts::TS_PACKET_SIZE;

/// Read granularity for streaming a capture: large enough to amortise the
/// syscall, small enough that memory stays flat on a multi-gigabyte capture
/// (r11-W-DT-4 — the old helper read the whole file, then built a second
/// full-size `Vec` of every packet).
pub(crate) const READ_CHUNK: usize = 1 << 20;

/// Open `path`. On `Err`, prints to stderr and returns `Err`.
pub(crate) fn open_file(path: &str, tool: &str) -> Result<File, ExitCode> {
    match File::open(path) {
        Ok(f) => Ok(f),
        Err(e) => {
            eprintln!("{tool}: {path}: {e}");
            Err(ExitCode::FAILURE)
        }
    }
}

/// Streams every aligned 188-byte packet of a capture, performing TS
/// byte-stream resynchronisation via [`TsResync`] over bounded
/// [`READ_CHUNK`]-sized reads. Supports both 188-byte and 204-byte
/// (RS-coded) streams; the parity bytes are stripped automatically.
///
/// An I/O error mid-file ends the iteration; call [`finish`](Self::finish)
/// after the loop to report it (and exit non-zero) rather than silently
/// treating a truncated read as a clean end of file.
///
/// Pulled into a helper so subcommands stay focused on the per-table view
/// rather than the TS framing loop.
pub(crate) struct PacketReader {
    file: File,
    path: String,
    tool: &'static str,
    resync: TsResync,
    chunk: Vec<u8>,
    pending: std::vec::IntoIter<[u8; TS_PACKET_SIZE]>,
    error: Option<std::io::Error>,
    done: bool,
}

impl PacketReader {
    /// Open `path` for streaming. On `Err`, prints to stderr.
    pub(crate) fn open(path: &str, tool: &'static str) -> Result<Self, ExitCode> {
        Ok(Self {
            file: open_file(path, tool)?,
            path: path.to_owned(),
            tool,
            resync: TsResync::new(),
            chunk: vec![0u8; READ_CHUNK],
            pending: Vec::new().into_iter(),
            error: None,
            done: false,
        })
    }

    /// Report a read error that ended the iteration early: prints it and
    /// returns `Err(FAILURE)`. `Ok(())` after a clean end of file.
    pub(crate) fn finish(&mut self) -> Result<(), ExitCode> {
        match self.error.take() {
            Some(e) => {
                eprintln!("{}: {}: {e}", self.tool, self.path);
                Err(ExitCode::FAILURE)
            }
            None => Ok(()),
        }
    }
}

impl Iterator for PacketReader {
    type Item = [u8; TS_PACKET_SIZE];

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(p) = self.pending.next() {
                return Some(p);
            }
            if self.done {
                return None;
            }
            match read_chunk(&mut self.file, &mut self.chunk) {
                Ok(0) => self.done = true,
                Ok(n) => self.pending = self.resync.feed(&self.chunk[..n]).into_iter(),
                Err(e) => {
                    self.error = Some(e);
                    self.done = true;
                }
            }
        }
    }
}

/// One `read`, retried on `Interrupted`.
pub(crate) fn read_chunk(file: &mut File, buf: &mut [u8]) -> std::io::Result<usize> {
    loop {
        match file.read(buf) {
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            other => return other,
        }
    }
}
