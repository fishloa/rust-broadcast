//! Shared test helper: run an external tool with a hard deadline.
//!
//! `Command::output()` reads stdout/stderr through pipes and waits for EOF on
//! both. Apple's `mediastreamvalidator` can leave a grandchild holding those
//! pipes open after it exits, so `output()` then blocks forever (a sibling
//! crate's test hung 89 minutes on a defunct validator child). This helper
//! redirects both streams to temp FILES, polls `try_wait` against a deadline,
//! kills the child on overrun and fails the test loudly.

use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// `mediastreamvalidator`'s own `-t` timeout, in seconds.
pub const MSV_TOOL_TIMEOUT_SECS: u64 = 30;
/// Margin added on top of the tool's own timeout before we kill it.
pub const MSV_DEADLINE_MARGIN: Duration = Duration::from_secs(30);
/// Hard deadline for a `mediastreamvalidator` run.
pub const MSV_DEADLINE: Duration =
    Duration::from_secs(MSV_TOOL_TIMEOUT_SECS + MSV_DEADLINE_MARGIN.as_secs());
/// Deadline for quick probes such as `--help` / `--version`.
pub const PROBE_DEADLINE: Duration = Duration::from_secs(15);
/// Poll interval while waiting on the child.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Captured result of [`run_bounded`].
pub struct Bounded {
    pub status: ExitStatus,
    pub stdout: String,
    pub stderr: String,
}

/// Run `cmd` to completion or `deadline`, whichever is first. Returns `Err` only when the
/// process cannot be spawned; panics (failing the test) when the deadline is
/// exceeded, after killing the child.
pub fn run_bounded(mut cmd: Command, deadline: Duration, label: &str) -> std::io::Result<Bounded> {
    let dir = std::env::temp_dir().join(format!(
        "transmux-bounded-{}-{}",
        std::process::id(),
        label.replace(|c: char| !c.is_ascii_alphanumeric(), "_")
    ));
    std::fs::create_dir_all(&dir).expect("bounded scratch dir");
    let out_path = dir.join("stdout");
    let err_path = dir.join("stderr");
    let open = |p: &Path| std::fs::File::create(p).expect("bounded output file");
    let child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::from(open(&out_path)))
        .stderr(Stdio::from(open(&err_path)))
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(e);
        }
    };
    let start = Instant::now();
    let status = loop {
        match child.try_wait().expect("try_wait") {
            Some(s) => break s,
            None if start.elapsed() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                panic!(
                    "{label} exceeded its {deadline:?} hard deadline and was killed \
                     (a hung external tool must fail the test, not hang the suite)"
                );
            }
            None => std::thread::sleep(POLL_INTERVAL),
        }
    };
    let read =
        |p: &Path| String::from_utf8_lossy(&std::fs::read(p).unwrap_or_default()).into_owned();
    let result = Bounded {
        status,
        stdout: read(&out_path),
        stderr: read(&err_path),
    };
    let _ = std::fs::remove_dir_all(&dir);
    Ok(result)
}
