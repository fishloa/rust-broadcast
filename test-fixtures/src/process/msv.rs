//! Apple `mediastreamvalidator` invocation and report walk shared by the HLS oracle tests.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use serde_json::Value;

use super::{output_bounded, tool_available};

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
    let report = dir.join("report.json");
    let mut cmd = Command::new("mediastreamvalidator");
    cmd.current_dir(dir)
        .arg("--quiet")
        .arg("-t")
        .arg("5")
        .arg("-O")
        .arg(&report)
        .arg(playlist);
    output_bounded(&mut cmd, deadline).expect("run mediastreamvalidator");
    let text = std::fs::read_to_string(&report).expect("validator report");
    serde_json::from_str(&text).expect("validator report is JSON")
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
