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

use std::fs::{self, File};
use std::io;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use wait_timeout::ChildExt;

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
