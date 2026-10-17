//! Differential sweep harness: one source file, built against the last
//! published wave (`--features baseline`) and against HEAD (`--features head`),
//! printing one deterministic JSON line per `(fixture, probe)`.
//!
//! ```text
//! sweep --root <repo-root-with-fixtures> [--bin-dir <dir-with-ts-fix-and-dvb-tools>]
//!       [--only <probe-prefix>] [--timeout <secs>] [--max-mb <n>]
//! ```
//!
//! Every probe is guarded by `catch_unwind` and a per-file timeout: a panic or
//! a timeout is itself a result (`{"panic": ".."}` / `{"timeout": true}`).
//! Only API signatures that differ between the trees are gated with
//! `cfg(feature = "baseline")` / `cfg(feature = "head")`; see README.md for the
//! list of adapted call sites.

#![allow(clippy::too_many_lines)]

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

mod probes;

pub fn sha(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// The raw 13-bit value of a PID, whether the tree models it as a bare `u16`
/// or as `mpeg_ts::pid::Pid` (and whichever formatting traits that type has),
/// so the two trees print the same text for the same PID.
pub trait PidRaw {
    fn raw(&self) -> u16;
}

impl PidRaw for u16 {
    fn raw(&self) -> u16 {
        *self
    }
}

impl PidRaw for mpeg_ts::pid::Pid {
    fn raw(&self) -> u16 {
        self.value()
    }
}

pub fn hx<T: PidRaw>(v: T) -> String {
    format!("{:04X}", v.raw())
}

pub fn hx_opt<T: PidRaw>(v: Option<T>) -> String {
    v.map_or_else(|| "-".to_string(), hx)
}

/// First identifier-ish word of a `Debug` rendering (`Avc { .. }` -> `Avc`).
pub fn variant_name(dbg: &str) -> String {
    dbg.chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect()
}

pub struct Fixture {
    pub rel: String,
    pub ext: String,
    pub data: Vec<u8>,
}

impl Fixture {
    pub fn is_ts_ext(&self) -> bool {
        matches!(self.ext.as_str(), "ts" | "m2ts" | "mpg" | "tsv")
    }
}

/// 188-byte TS packets of `data`, stride/offset auto-detected (188, 192 with a
/// 4-byte prefix, 204). `None` when no lattice with five consecutive syncs.
pub fn ts_packets(data: &[u8]) -> Option<Vec<&[u8]>> {
    for stride in [188usize, 192, 204] {
        let pre = if stride == 192 { 4 } else { 0 };
        for start in 0..stride.min(data.len()) {
            if (0..5).all(|k| data.get(start + pre + k * stride) == Some(&0x47)) {
                let mut out = Vec::new();
                let mut p = start;
                while p + pre + 188 <= data.len() {
                    out.push(&data[p + pre..p + pre + 188]);
                    p += stride;
                }
                return Some(out);
            }
        }
    }
    None
}

type ProbeFn = fn(&Fixture, &Cfg) -> Option<Value>;

pub struct Cfg {
    pub bin_dir: Option<PathBuf>,
}

const PROBES: &[(&str, ProbeFn)] = &[
    ("transmux.stream", probes::transmux_stream),
    ("transmux.batch", probes::transmux_batch),
    ("multimux.ts_ingest", probes::multimux_ts_ingest),
    ("transmux.run", probes::transmux_run),
    ("dvb-si.sections", probes::dvb_si_sections),
    ("dvb-si.section_bin", probes::dvb_si_section_bin),
    ("dvb-t2mi.pump", probes::dvb_t2mi),
    ("mpeg-ts.packets", probes::mpeg_ts),
    ("scte35.sections", probes::scte35),
    ("st291.parse", probes::st291),
    ("media-doctor.findings", probes::media_doctor),
    ("hls.playlist", probes::hls),
    ("dash.mpd", probes::dash),
    ("cli.ts-fix", probes::cli_ts_fix),
    ("cli.dvb-tools", probes::cli_dvb_tools),
];

const EXTS: &[&str] = &[
    "ts", "m2ts", "mpg", "mp4", "m4s", "mkv", "webm", "flv", "ps", "mxf", "bin", "m3u8", "mpd",
];

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() {
            walk(&p, out);
        } else if ft.is_file() {
            out.push(p);
        }
    }
}

fn run_guarded(
    f: ProbeFn,
    fx: std::sync::Arc<Fixture>,
    cfg: std::sync::Arc<Cfg>,
    timeout: Duration,
) -> Option<Value> {
    let (tx, rx) = mpsc::channel();
    let spawn = std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(move || {
            let r = catch_unwind(AssertUnwindSafe(|| f(&fx, &cfg)));
            let v = match r {
                Ok(v) => v,
                Err(p) => {
                    let msg = p
                        .downcast_ref::<String>()
                        .cloned()
                        .or_else(|| p.downcast_ref::<&str>().map(|s| (*s).to_string()))
                        .unwrap_or_else(|| "<non-string panic>".into());
                    Some(json!({ "panic": msg }))
                }
            };
            let _ = tx.send(v);
        });
    if spawn.is_err() {
        return Some(json!({ "panic": "thread spawn failed" }));
    }
    match rx.recv_timeout(timeout) {
        Ok(v) => v,
        Err(_) => Some(json!({ "timeout": true })),
    }
}

fn main() {
    let mut root: Option<PathBuf> = None;
    let mut only: Option<String> = None;
    let mut timeout = 300u64;
    let mut max_mb = 100u64;
    let mut cfg = Cfg { bin_dir: None };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--root" => root = args.next().map(PathBuf::from),
            "--only" => only = args.next(),
            "--timeout" => timeout = args.next().and_then(|s| s.parse().ok()).unwrap_or(300),
            "--max-mb" => max_mb = args.next().and_then(|s| s.parse().ok()).unwrap_or(100),
            "--bin-dir" => cfg.bin_dir = args.next().map(PathBuf::from),
            other => {
                eprintln!("unknown argument {other}");
                std::process::exit(2);
            }
        }
    }
    let root = root.unwrap_or_else(|| {
        eprintln!("--root <repo root> is required");
        std::process::exit(2);
    });
    let cfg = std::sync::Arc::new(cfg);

    let mut files = Vec::new();
    walk(&root.join("fixtures"), &mut files);
    walk(&root.join("private").join("fixtures"), &mut files);

    let mut nfix = 0u64;
    let mut nprobe = 0u64;
    for path in files {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !EXTS.contains(&ext.as_str()) {
            continue;
        }
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        if meta.len() > max_mb * 1_000_000 {
            continue;
        }
        let Ok(data) = std::fs::read(&path) else {
            continue;
        };
        let rel = path
            .strip_prefix(&root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let fx = std::sync::Arc::new(Fixture { rel, ext, data });
        nfix += 1;
        for (name, f) in PROBES {
            if only.as_deref().is_some_and(|o| !name.starts_with(o)) {
                continue;
            }
            let Some(result) =
                run_guarded(*f, fx.clone(), cfg.clone(), Duration::from_secs(timeout))
            else {
                continue;
            };
            nprobe += 1;
            let mut line = Map::new();
            line.insert("fixture".into(), json!(fx.rel));
            line.insert("probe".into(), json!(name));
            line.insert("result".into(), result);
            println!("{}", Value::Object(line));
        }
    }
    eprintln!("sweep: {nfix} fixtures, {nprobe} (fixture, probe) results");
}

/// Small helper: increment a counter in a `BTreeMap<String, u64>`.
pub fn bump(m: &mut BTreeMap<String, u64>, key: impl Into<String>) {
    *m.entry(key.into()).or_insert(0) += 1;
}
