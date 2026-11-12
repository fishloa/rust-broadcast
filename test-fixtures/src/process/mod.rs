//! Deadline-bounded external-tool runner for tests (release audit: the full
//! workspace run hung 89 minutes in `mediastreamvalidator_oracle`).
//!
//! `Command::output()` captures through pipes and waits until EVERY holder of
//! the write end closes it. A tool that leaves a grandchild holding stdout
//! (`mediastreamvalidator` does) therefore blocks `output()` forever even
//! after the tool itself exited (defunct). This runner redirects stdout and
//! stderr to temp FILES (no pipe to hold open), waits for the child with
//! `wait_timeout` against a hard deadline (no poll loop, no sleeping), and
//! kills the child on overrun with a clear error.
//!
//! This module also holds the other process helpers the external-tool tests
//! shared as hand-copies: [`KillOnDrop`], [`tool_available`], [`free_udp_port`]
//! and [`free_tcp_port`].
//!
//! This is the ONE copy: `hls-runtime`, `multimux` and `media-doctor` used to
//! carry byte-identical copies, then a separate `test-bounded` crate held it
//! (de-hand-roll W3, spec §7 SP7.3); it now lives here, in the unpublished
//! `test-fixtures` crate, which takes no workspace-sibling deps, so a path-only
//! dev-dependency on it never appears in a published manifest.

use std::fs::{self, File};
use std::io;
use std::net::{TcpListener, UdpSocket};
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

use wait_timeout::ChildExt;

pub mod msv;
pub mod xsd;

static CALL_COUNTER: AtomicU64 = AtomicU64::new(0);

fn capture_path(kind: &str) -> PathBuf {
    let id = CALL_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("bounded-cmd-{}-{id}.{kind}", std::process::id()))
}

/// Run `cmd` to completion within `deadline`, capturing stdout/stderr via
/// temp files. On overrun the child is killed and an
/// [`io::ErrorKind::TimedOut`] error naming the program is returned.
pub fn output_bounded(cmd: &mut Command, deadline: Duration) -> io::Result<Output> {
    let out_path = capture_path("stdout");
    let err_path = capture_path("stderr");
    let result = run(cmd, deadline, &out_path, &err_path);
    let _ = fs::remove_file(&out_path);
    let _ = fs::remove_file(&err_path);
    result
}

fn run(
    cmd: &mut Command,
    deadline: Duration,
    out_path: &PathBuf,
    err_path: &PathBuf,
) -> io::Result<Output> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::from(File::create(out_path)?))
        .stderr(Stdio::from(File::create(err_path)?))
        .spawn()?;
    let Some(status) = child.wait_timeout(deadline)? else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!(
                "{:?} still running after the {deadline:?} hard deadline; killed",
                cmd.get_program()
            ),
        ));
    };
    Ok(Output {
        status,
        stdout: fs::read(out_path)?,
        stderr: fs::read(err_path)?,
    })
}

/// Kills and reaps the wrapped child on drop, so a panic, an early return or a
/// timeout dropping the future never leaks a real external process.
pub struct KillOnDrop(pub Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Whether `tool arg` runs and exits successfully (`ffprobe -version`,
/// `mediastreamvalidator --version`).
#[must_use]
pub fn tool_available(tool: &str, arg: &str) -> bool {
    Command::new(tool)
        .arg(arg)
        .output()
        .is_ok_and(|o| o.status.success())
}

/// First port of the range [`free_udp_port`] hands out; below every OS
/// ephemeral range (Linux 32768.., macOS 49152..), so a test's own
/// `bind("127.0.0.1:0")` can never be given a port reserved here.
const UDP_RANGE_START: u16 = 20_000;
const UDP_RANGE_LEN: u16 = 12_000;
const UDP_PID_STRIDE: u32 = 211;

static UDP_NEXT: AtomicU32 = AtomicU32::new(0);

/// A UDP port that was free on loopback a moment ago, never handed out twice
/// within this process and never inside the OS ephemeral range, so neither a
/// sibling test calling this nor one binding port 0 can be given it before the
/// caller's external process binds it.
///
/// # Panics
/// If no port of the range can be bound.
#[must_use]
pub fn free_udp_port() -> u16 {
    let span = u32::from(UDP_RANGE_LEN);
    let seed = std::process::id().wrapping_mul(UDP_PID_STRIDE) % span;
    for _ in 0..span {
        let step = UDP_NEXT.fetch_add(1, Ordering::Relaxed);
        let port = UDP_RANGE_START + u16::try_from((seed + step) % span).expect("below span");
        if UdpSocket::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
    panic!("no free UDP port in {UDP_RANGE_START}..+{UDP_RANGE_LEN}");
}

/// A TCP port that was free on loopback a moment ago.
///
/// # Panics
/// If no loopback listener can be bound.
#[must_use]
pub fn free_tcp_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind a free TCP port")
        .local_addr()
        .expect("local addr")
        .port()
}
