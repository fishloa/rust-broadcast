//! Tripwire: a forwarding wrapper must forward EVERY defaulted trait method.
//!
//! `RecordingCaDevice` once omitted `CaDevice::reset_settle()`; the trait
//! default (`Duration::ZERO`) silently applied and `Driver::init` wrote the
//! first TPDU right after `CA_RESET` (EINVAL on a CXD2099). Two layers:
//!
//! 1. behavioural: a device overriding every defaulted method with a
//!    distinctive value, wrapped in each wrapper type;
//! 2. lexical: every `fn name(..) {` default body in the `CaDevice` /
//!    `CiDataDevice` trait source must appear as `fn name(` in every wrapper
//!    impl (a generic `impl<..> Trait for X<..>`, or any impl whose body
//!    delegates through `self.inner.`) anywhere under `src/`. Adding a new
//!    defaulted method without forwarding it fails here. Lexical, not a
//!    proof: code review remains the real control.

use dvb_ci_runtime::device::{CaDevice, RecordingCaDevice, SlotInfo};
use std::io;
use std::time::Duration;

const DISTINCT_SETTLE: Duration = Duration::from_millis(4321);

struct Distinct;

impl CaDevice for Distinct {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        Ok(0)
    }
    fn write(&mut self, _: &[u8]) -> io::Result<()> {
        Ok(())
    }
    fn reset(&mut self) -> io::Result<()> {
        Ok(())
    }
    fn reset_settle(&self) -> Duration {
        DISTINCT_SETTLE
    }
    fn slot_info(&mut self) -> io::Result<SlotInfo> {
        Ok(SlotInfo::default())
    }
    fn poll(&mut self, _: Duration) -> io::Result<bool> {
        Ok(false)
    }
}

#[test]
fn recording_wrapper_forwards_every_defaulted_method() {
    assert_ne!(DISTINCT_SETTLE, Duration::ZERO);
    let w = RecordingCaDevice::new(Distinct);
    assert_eq!(w.reset_settle(), DISTINCT_SETTLE);
}

// ---- lexical guard -------------------------------------------------------

/// `(trait, wrapper-impl header substring, method, reason)` intentional
/// non-forwards. Empty: every wrapper forwards everything.
const ALLOW: &[(&str, &str, &str, &str)] = &[];

fn rs_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// `(offset of the opening brace, text of the balanced block)` for the first
/// `{` at/after `from`.
fn block(src: &str, from: usize) -> (usize, &str) {
    let open = from + src[from..].find('{').unwrap();
    let mut depth = 0usize;
    for (i, c) in src[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return (open, &src[open..open + i + 1]);
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced braces");
}

/// Names of methods in `pub trait <name>` that have a default body.
fn defaulted(src: &str, name: &str) -> Vec<String> {
    let at = src
        .find(&format!("pub trait {name} {{"))
        .unwrap_or_else(|| panic!("trait {name} not found"));
    let (_, body) = block(src, at);
    let mut out = Vec::new();
    // skip the trait's own opening brace
    let mut rest = &body[1..];
    while let Some(i) = rest.find("fn ") {
        let after = &rest[i + 3..];
        let end = after.find('(').unwrap();
        let fname = after[..end].trim().to_string();
        // the signature ends at `;` (required) or `{` (default body)
        let sig_end = after.find([';', '{']).unwrap();
        if after.as_bytes()[sig_end] == b'{' {
            out.push(fname);
            let (o, b) = block(after, sig_end);
            rest = &after[o + b.len()..];
        } else {
            rest = &after[sig_end + 1..];
        }
    }
    out
}

#[test]
fn every_wrapper_impl_forwards_every_defaulted_method() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let traits = [
        (
            "CaDevice",
            std::fs::read_to_string(root.join("device.rs")).unwrap(),
        ),
        (
            "CiDataDevice",
            std::fs::read_to_string(root.join("dataplane.rs")).unwrap(),
        ),
    ];
    let mut files = Vec::new();
    rs_files(&root, &mut files);

    // The guard must see the method that caused the bug, or it is vacuous.
    assert!(defaulted(&traits[0].1, "CaDevice").contains(&"reset_settle".to_string()));

    let mut wrappers = 0;
    let mut failures = Vec::new();
    for (tr, tsrc) in &traits {
        let defaults = defaulted(tsrc, tr);
        for f in &files {
            let src = std::fs::read_to_string(f).unwrap();
            let needle = format!("{tr} for ");
            let mut from = 0;
            while let Some(i) = src[from..].find(&needle) {
                let at = from + i;
                from = at + needle.len();
                // Only real impl headers: `impl ... Trait for` on one line.
                let line_start = src[..at].rfind('\n').map_or(0, |n| n + 1);
                let line = src[line_start..at].trim_start();
                if !line.starts_with("impl") {
                    continue;
                }
                let (open, body) = block(&src, at);
                let header = &src[line_start..open];
                let is_wrapper = line.starts_with("impl<") || body.contains("self.inner.");
                if !is_wrapper {
                    continue;
                }
                wrappers += 1;
                for m in &defaults {
                    let allowed = ALLOW
                        .iter()
                        .any(|(t, h, mm, _)| t == tr && header.contains(h) && mm == m);
                    if !allowed && !body.contains(&format!("fn {m}(")) {
                        failures.push(format!(
                            "{}: `{}` does not forward {tr}::{m}",
                            f.display(),
                            header.trim()
                        ));
                    }
                }
            }
        }
    }
    assert!(
        wrappers >= 2,
        "guard found only {wrappers} wrapper impls: scan broken"
    );
    assert!(
        failures.is_empty(),
        "unforwarded defaults:\n{}",
        failures.join("\n")
    );
}
