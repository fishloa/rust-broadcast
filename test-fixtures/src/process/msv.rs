//! Apple `mediastreamvalidator` invocation and report walk shared by the HLS oracle tests.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use serde_json::Value;

use super::tool_available;

/// `(JSON path, errorRequirementLevel, errorComment)` of one validator message; a rendition
/// whose parse failed reports level 1.
pub type Finding = (String, i64, String);

const LEVEL_MUST: i64 = 1;

/// Whether `mediastreamvalidator` is on `PATH`.
#[must_use]
pub fn available() -> bool {
    tool_available("mediastreamvalidator", "--version")
}

/// Validates `playlist` (relative to `dir`, where its resources were written) and returns the
/// JSON report written beside it.
///
/// # Panics
/// If the validator cannot run within `deadline` or writes no readable report.
#[must_use]
pub fn run_report(dir: &Path, playlist: &str, deadline: Duration) -> Value {
    MsvRun::new(dir, playlist).deadline(deadline).run()
}

/// Every message in `report` with its JSON path.
#[must_use]
pub fn findings(report: &Value) -> Vec<Finding> {
    let mut out = Vec::new();
    collect(report, "$", &mut out);
    out
}

fn collect(value: &Value, path: &str, out: &mut Vec<Finding>) {
    match value {
        Value::Object(map) => {
            let parse_failed = map
                .get("parseFailed")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if let Some(Value::Array(messages)) = map.get("messages") {
                for m in messages {
                    let level = m
                        .get("errorRequirementLevel")
                        .and_then(Value::as_i64)
                        .unwrap_or(0);
                    out.push((
                        path.to_string(),
                        if parse_failed { LEVEL_MUST } else { level },
                        m.get("errorComment")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                    ));
                }
            }
            for (k, v) in map {
                if k != "messages" {
                    collect(v, &format!("{path}/{k}"), out);
                }
            }
        }
        Value::Array(items) => {
            for (i, v) in items.iter().enumerate() {
                collect(v, &format!("{path}[{i}]"), out);
            }
        }
        _ => {}
    }
}

/// The distinct `errorComment` strings anywhere in `report`, sorted.
#[must_use]
pub fn error_comments(report: &Value) -> Vec<String> {
    let mut comments = Vec::new();
    walk_comments(report, &mut comments);
    comments.sort();
    comments.dedup();
    comments
}

fn walk_comments(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(m) => {
            if let Some(c) = m.get("errorComment").and_then(Value::as_str) {
                out.push(c.to_string());
            }
            m.values().for_each(|c| walk_comments(c, out));
        }
        Value::Array(a) => a.iter().for_each(|c| walk_comments(c, out)),
        _ => {}
    }
}

/// Maximum number of attempts `run_report` (and `first_report`) will make
/// before giving up on a flaky validator run.
pub const MAX_ATTEMPTS: u32 = 3;

/// Diagnostics captured from one validator attempt.
#[derive(Debug, Clone)]
pub struct MsvAttempt {
    pub status: Option<std::process::ExitStatus>,
    pub stdout: String,
    pub stderr: String,
}

/// Retry a flaky validator run up to `attempts` times. `run(attempt)` returns
/// `Some(text)` once a non-empty report is produced; otherwise `None`. On
/// total failure every attempt's diagnostic string is returned.
pub fn first_report<F>(attempts: u32, mut run: F) -> Result<String, Vec<String>>
where
    F: FnMut(u32) -> Option<String>,
{
    let mut diagnostics = Vec::new();
    for attempt in 0..attempts {
        match run(attempt) {
            Some(text) => return Ok(text),
            None => diagnostics.push(format!("attempt {} produced no JSON", attempt)),
        }
    }
    Err(diagnostics)
}

/// Builder for a `mediastreamvalidator` invocation with retry behaviour.
pub struct MsvRun<'a> {
    dir: &'a std::path::Path,
    entry: &'a str,
    parse_only: bool,
    timeout_secs: u32,
    extra: Vec<String>,
    deadline: Option<std::time::Duration>,
}

impl<'a> MsvRun<'a> {
    #[must_use]
    pub fn new(dir: &'a std::path::Path, entry: &'a str) -> Self {
        Self {
            dir,
            entry,
            parse_only: false,
            timeout_secs: 5,
            extra: Vec::new(),
            deadline: None,
        }
    }

    #[must_use]
    pub fn parse_only(mut self, v: bool) -> Self {
        self.parse_only = v;
        self
    }

    #[must_use]
    pub fn timeout_secs(mut self, v: u32) -> Self {
        self.timeout_secs = v;
        self
    }

    #[must_use]
    pub fn extra(mut self, args: &[&str]) -> Self {
        self.extra = args.iter().map(|s| (*s).to_string()).collect();
        self
    }

    #[must_use]
    pub fn arg(mut self, a: &str) -> Self {
        self.extra.push(a.to_string());
        self
    }

    #[must_use]
    pub fn deadline(mut self, d: std::time::Duration) -> Self {
        self.deadline = Some(d);
        self
    }

    pub fn run(&self) -> serde_json::Value {
        use std::fs;
        use std::sync::atomic::{AtomicU64, Ordering};
        static CALL_COUNTER: AtomicU64 = AtomicU64::new(0);
        let deadline = self
            .deadline
            .unwrap_or_else(|| std::time::Duration::from_secs(u64::from(self.timeout_secs) + 30));
        let mut attempts_diag: Vec<MsvAttempt> = Vec::new();
        let run = |attempt: u32| -> Option<String> {
            let call_id = CALL_COUNTER.fetch_add(1, Ordering::Relaxed);
            let out_json = std::env::temp_dir().join(format!(
                "mediastreamvalidator-oracle-{}-{}-{}.json",
                std::process::id(),
                call_id,
                attempt
            ));
            let mut cmd = Command::new("mediastreamvalidator");
            cmd.current_dir(self.dir);
            if self.parse_only {
                cmd.arg("--parse-playlist-only");
            }
            for a in &self.extra {
                cmd.arg(a);
            }
            cmd.arg("--quiet")
                .arg("-t")
                .arg(self.timeout_secs.to_string())
                .arg("-O")
                .arg(&out_json)
                .arg(self.entry);
            let out = match super::output_bounded(&mut cmd, deadline) {
                Ok(o) => o,
                Err(e) => {
                    attempts_diag.push(MsvAttempt {
                        status: None,
                        stdout: String::new(),
                        stderr: e.to_string(),
                    });
                    return None;
                }
            };
            let text = match fs::read_to_string(&out_json) {
                Ok(t) if !t.trim().is_empty() => t,
                _ => {
                    attempts_diag.push(MsvAttempt {
                        status: Some(out.status),
                        stdout: String::from_utf8_lossy(&out.stdout).to_string(),
                        stderr: String::from_utf8_lossy(&out.stderr).to_string(),
                    });
                    let _ = fs::remove_file(&out_json);
                    return None;
                }
            };
            let _ = fs::remove_file(&out_json);
            Some(text)
        };
        let text = first_report(MAX_ATTEMPTS, run).unwrap_or_else(|_| {
            let mut msg = String::from("mediastreamvalidator produced no JSON");
            for d in &attempts_diag {
                msg.push_str(&format!(
                    "\nstatus={:?} stdout={} stderr={}",
                    d.status, d.stdout, d.stderr
                ));
            }
            panic!("{}", msg);
        });
        serde_json::from_str(&text).expect("validator report is JSON")
    }
}

// keep run_report signature

#[cfg(test)]
mod first_report_tests {
    use super::*;
    #[test]
    fn yields_on_first() {
        let r = first_report(3, |_| Some("json".to_string()));
        assert_eq!(r.unwrap(), "json");
    }
    #[test]
    fn yields_on_third() {
        let mut n = 0u32;
        let r = first_report(3, |_| {
            n += 1;
            if n == 3 { Some("ok".to_string()) } else { None }
        });
        assert_eq!(r.unwrap(), "ok");
    }
    #[test]
    fn all_none_three_diags() {
        let r = first_report(3, |_| None);
        assert_eq!(r.unwrap_err().len(), 3);
    }
    #[test]
    fn never_more_than_attempts() {
        let mut calls = 0u32;
        let _ = first_report(3, |_| {
            calls += 1;
            None
        });
        assert_eq!(calls, 3);
    }
}
