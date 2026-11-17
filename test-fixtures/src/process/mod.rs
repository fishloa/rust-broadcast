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
use std::net::{SocketAddr, TcpListener, UdpSocket};
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

/// Like [`tool_available`], but also accepts a tool that prints its version and
/// exits non-zero (e.g. `srt-live-transmit -version`).
#[must_use]
pub fn tool_available_lenient(tool: &str, arg: &str) -> bool {
    Command::new(tool)
        .arg(arg)
        .output()
        .is_ok_and(|o| o.status.success() || !o.stdout.is_empty() || !o.stderr.is_empty())
}

/// Set (to any value) to make every [`require_tool`] gate a hard failure on a
/// host without the oracle, instead of a loud skip. CI jobs that install the
/// tools set it, so a green run there means the oracle actually ran.
pub const REQUIRE_EXTERNAL_TOOLS: &str = "REQUIRE_EXTERNAL_TOOLS";

/// The one gate for tests whose coverage comes from an external oracle binary
/// (`ffprobe`, `MP4Box`, `mp4dump`, `ffmpeg`, …).
///
/// Returns `true` when `tool arg` runs, so the caller proceeds. When it is
/// absent the result is `false` and the caller returns, printing why — UNLESS
/// `env_name` or [`REQUIRE_EXTERNAL_TOOLS`] is set, in which case this panics:
/// an oracle test that passes without its oracle is not coverage, and these are
/// the tests meant to catch a misreading of a spec shared by our code and our
/// own checker.
///
/// # Panics
/// If the tool is absent and a require switch is set.
#[must_use]
pub fn require_tool(tool: &str, arg: &str, env_name: &str) -> bool {
    let present = tool_available(tool, arg);
    require_tool_probed(present, tool, env_name);
    present
}

/// [`require_tool`] for a tool whose version probe exits non-zero (see
/// [`tool_available_lenient`]).
///
/// # Panics
/// If the tool is absent and a require switch is set.
#[must_use]
pub fn require_tool_lenient(tool: &str, arg: &str, env_name: &str) -> bool {
    let present = tool_available_lenient(tool, arg);
    require_tool_probed(present, tool, env_name);
    present
}

/// The gate itself: returns when `present`, panics when the tool is absent and
/// `env_name` or [`REQUIRE_EXTERNAL_TOOLS`] is set, and otherwise prints why the
/// caller is about to skip.
///
/// Takes the caller's own presence probe so a test that probes through
/// [`output_bounded`] keeps its hang guard.
///
/// # Panics
/// If `present` is false and a require switch is set.
pub fn require_tool_probed(present: bool, tool: &str, env_name: &str) {
    gate(
        present,
        tool,
        env_name,
        std::env::var_os(env_name).is_some(),
        std::env::var_os(REQUIRE_EXTERNAL_TOOLS).is_some(),
    );
}

/// [`require_tool_probed`] with both switches passed in, so every arm is
/// testable without mutating process state.
fn gate(present: bool, tool: &str, env_name: &str, site_required: bool, globally_required: bool) {
    if present {
        return;
    }
    if site_required || globally_required {
        let switch = if site_required {
            env_name
        } else {
            REQUIRE_EXTERNAL_TOOLS
        };
        panic!("external oracle `{tool}` is not on PATH and {switch} is set");
    }
    let switch = if env_name == REQUIRE_EXTERNAL_TOOLS {
        REQUIRE_EXTERNAL_TOOLS.to_string()
    } else {
        format!("{env_name}=1 or {REQUIRE_EXTERNAL_TOOLS}=1")
    };
    eprintln!(
        "SKIP: `{tool}` is not on PATH, so this oracle did NOT run (set {switch} to fail instead)"
    );
}

/// Locate `bin` on `PATH`, returning its full path.
#[must_use]
pub fn which(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(bin))
        .find(|p| p.is_file())
}

/// Wait until `addr` accepts a bare TCP connection.
pub async fn wait_for_port(addr: SocketAddr) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("nothing ever accepted a connection on {addr} within the hang guard");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
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

#[cfg(test)]
mod tests {
    use super::{REQUIRE_EXTERNAL_TOOLS, gate};

    const SITE: &str = "SOME_TOOL_REQUIRE";

    #[test]
    fn a_present_tool_is_never_a_failure() {
        gate(true, "ffprobe", SITE, false, false);
        gate(true, "ffprobe", SITE, true, false);
        gate(true, "ffprobe", SITE, false, true);
    }

    #[test]
    fn an_absent_tool_returns_an_unrequired_skip() {
        gate(false, "ffprobe", SITE, false, false);
    }

    #[test]
    #[should_panic(
        expected = "external oracle `ffprobe` is not on PATH and SOME_TOOL_REQUIRE is set"
    )]
    fn the_site_switch_turns_an_absent_tool_into_a_failure() {
        gate(false, "ffprobe", SITE, true, false);
    }

    #[test]
    #[should_panic(
        expected = "external oracle `MP4Box` is not on PATH and REQUIRE_EXTERNAL_TOOLS is set"
    )]
    fn the_global_switch_turns_an_absent_tool_into_a_failure() {
        gate(false, "MP4Box", SITE, false, true);
    }

    #[test]
    fn the_global_switch_is_named_require_external_tools() {
        assert_eq!(REQUIRE_EXTERNAL_TOOLS, "REQUIRE_EXTERNAL_TOOLS");
    }
}
