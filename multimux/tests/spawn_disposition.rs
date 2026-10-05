//! SP1.4 spawn-disposition guard (W2b-1 Task 7).
//!
//! Every non-test `tokio::spawn` / `spawn_blocking` / `thread::spawn` / `…
//! .spawn(` in `src/**` must appear in the line-pinned [`DISPOSITION`] table
//! with its owner and cancellation path. A NEW untracked spawn fails this test,
//! so "every spawn has a disposition" cannot silently rot.
//!
//! The scanner strips `#[cfg(test)] mod` bodies using the same whole-file
//! string/raw-string/comment-aware brace walker as `no_handroll_guard.rs`, so
//! a spawn inside a test module is not counted and a raw string cannot desync
//! the walker.
//!
//! **This is a lexical tripwire, not a proof.** Code review is the real control.

use std::fs;
use std::path::Path;

/// `(file suffix, 1-based line, owner + cancellation-path reason)`.
const DISPOSITION: &[(&str, u32, &str)] = &[
    (
        "source/file_reader.rs",
        407,
        "FileReader::spawn -> SpawnedReader (owns a CancellationToken; Drop cancels)",
    ),
    (
        "source/file_reader.rs",
        483,
        "spawn_blocking for a blocking file read; awaited inline (owned by the caller)",
    ),
    (
        "route.rs",
        643,
        "spawn_blocking DVR persist; awaited inline",
    ),
    ("dvr.rs", 595, "spawn_blocking DVR write; awaited inline"),
    (
        "source/pull.rs",
        117,
        "PullScheduler's `inflight` JoinSet (drained on drop); the shared engine for the three pull sources",
    ),
    ("source/rtmp.rs", 325, "RTMP accept pump TaskTracker"),
    (
        "source/rtmp.rs",
        744,
        "RTMP run task; owned by the route supervisor handle",
    ),
    (
        "source/rtmp.rs",
        753,
        "RTMP accept pump; owned by the route supervisor handle",
    ),
    (
        "source/whip.rs",
        349,
        "WHIP signalling server TaskTracker (WhipInfra)",
    ),
    (
        "source/whip.rs",
        554,
        "WHIP run task; owned by the route supervisor handle",
    ),
    (
        "source/whip.rs",
        569,
        "WHIP accept pump; owned by the route supervisor handle",
    ),
    (
        "output/whep.rs",
        916,
        "WHEP saturated-accept test harness server; cancelled by the returned token",
    ),
    (
        "output/whep.rs",
        966,
        "serve_whep_run_for_test -> run_whep_with_tracker; cancelled by the returned token",
    ),
    (
        "output/whep.rs",
        1508,
        "WHEP signalling server; JOINED before run_whep returns (port-release ordering)",
    ),
    (
        "output/whep.rs",
        1529,
        "WHEP viewer session on the run's session TaskTracker (close()+wait() on cancel)",
    ),
    (
        "origin/mod.rs",
        620,
        "per-connection task on the listener's TaskTracker (serve_hyper_util reaps finished tasks)",
    ),
    (
        "origin/mod.rs",
        721,
        "per-connection task on the listener's TaskTracker (serve_hyper_util reaps finished tasks)",
    ),
    (
        "output/catchup.rs",
        177,
        "spawn_blocking archive scan; awaited inline",
    ),
    (
        "output/catchup.rs",
        255,
        "spawn_blocking archive scan; awaited inline",
    ),
    (
        "output/catchup.rs",
        354,
        "spawn_blocking archived-segment lookup; awaited inline",
    ),
    (
        "output/catchup.rs",
        365,
        "spawn_blocking archived-segment read; awaited inline",
    ),
    (
        "output/catchup.rs",
        432,
        "spawn_blocking archived-segment read; awaited inline",
    ),
    (
        "output/smooth.rs",
        129,
        "spawn_blocking Smooth layout build; awaited inline",
    ),
    (
        "output/smooth.rs",
        183,
        "spawn_blocking Smooth layout locate; awaited inline",
    ),
    (
        "output/smooth.rs",
        232,
        "spawn_blocking Smooth fragment build; awaited inline",
    ),
    (
        "origin/mod.rs",
        1578,
        "Ctrl-C/SIGTERM shutdown watcher; aborted/awaited at process shutdown",
    ),
    (
        "origin/mod.rs",
        1754,
        "spawn_following -> follow_trunk; the returned JoinHandle is owned by the caller and dropped on route teardown (async drop cancels)",
    ),
    (
        "origin/mod.rs",
        2214,
        "supervise_driver; spawned under the route cancellation token",
    ),
    (
        "origin/admin.rs",
        1071,
        "external shutdown-signal watcher; aborted by hand at admin teardown",
    ),
    (
        "origin/admin.rs",
        1083,
        "shutdown-watch task translating the signal channel into the token; aborted by hand at teardown",
    ),
    (
        "origin/admin.rs",
        1089,
        "admin listener server task; joined/aborted at admin teardown",
    ),
];

fn rs_files(dir: &Path, out: &mut Vec<(String, String)>) {
    for entry in fs::read_dir(dir).expect("read src dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rs_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push((
                path.display().to_string(),
                fs::read_to_string(&path).expect("read source"),
            ));
        }
    }
}

/// The surviving (non-test-module) lines of `src` WITH their 1-based original
/// line numbers, so a hit reports the real line. Mirrors
/// `no_handroll_guard.rs`'s walker (string / raw-string / char / comment aware
/// across lines).
fn surviving_lines(src: &str) -> Vec<(usize, String)> {
    let b = src.as_bytes();
    let n = b.len();

    // Mark every byte inside a `#[cfg(test)] mod` span as removed.
    let mut removed = vec![false; n];
    let mut i = 0usize;
    while i < n {
        if let Some((attr_start, mod_kw, body_open)) = match_cfg_test_mod(b, i) {
            let end = match body_open {
                None => find_semicolon(b, mod_kw).unwrap_or(mod_kw),
                Some(open) => match_close_brace(b, open).unwrap_or(n - 1),
            };
            for r in removed.iter_mut().take(end + 1).skip(attr_start) {
                *r = true;
            }
            i = end + 1;
            continue;
        }
        i += 1;
    }

    // Emit each original line whose FIRST byte survived.
    let mut out = Vec::new();
    let mut line_start = 0usize;
    let mut original_line = 1usize;
    for (idx, &byte) in b.iter().enumerate() {
        let is_break = byte == b'\n';
        if is_break || idx == n - 1 {
            let end = if is_break { idx } else { n };
            if line_start < n && !removed[line_start] {
                out.push((
                    original_line,
                    String::from_utf8_lossy(&b[line_start..end]).into_owned(),
                ));
            }
            original_line += 1;
            line_start = idx + 1;
        }
    }
    out
}

#[allow(clippy::type_complexity)]
fn match_cfg_test_mod(b: &[u8], from: usize) -> Option<(usize, usize, Option<usize>)> {
    let n = b.len();
    let mut j = from;
    while j < n && (b[j] == b' ' || b[j] == b'\t') {
        j += 1;
    }
    if j + 12 > n || &b[j..j + 12] != b"#[cfg(test)]" {
        return None;
    }
    let attr_start = j;
    let mut k = j + 12;
    loop {
        while k < n && matches!(b[k], b' ' | b'\t' | b'\r' | b'\n') {
            k += 1;
        }
        if k < n && b[k] == b'#' && k + 1 < n && b[k + 1] == b'[' {
            let close = matching_bracket(b, k + 1)?;
            k = close + 1;
            continue;
        }
        break;
    }
    if k + 4 <= n && &b[k..k + 4] == b"pub " {
        k += 4;
        while k < n && matches!(b[k], b' ' | b'\t') {
            k += 1;
        }
    }
    if k + 4 > n || &b[k..k + 4] != b"mod " {
        return None;
    }
    let mod_kw = k;
    let mut m = k + 4;
    while m < n && (b[m] == b'_' || (b[m] as char).is_alphanumeric()) {
        m += 1;
    }
    while m < n && b[m] != b'{' && b[m] != b';' {
        m += 1;
    }
    if m >= n {
        return None;
    }
    if b[m] == b';' {
        return Some((attr_start, mod_kw, None));
    }
    Some((attr_start, mod_kw, Some(m)))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ScanState {
    Normal,
    BlockComment(u32),
}

fn matching_bracket(b: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = open;
    let mut state = ScanState::Normal;
    while i < b.len() {
        let (ns, consumed) = advance_state(b, i, &mut state);
        if state == ScanState::Normal {
            match b[i] {
                b'[' => depth += 1,
                b']' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                _ => {}
            }
        }
        state = ns;
        i += 1 + consumed;
    }
    None
}

fn match_close_brace(b: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = open;
    let mut state = ScanState::Normal;
    let mut seen_open = false;
    while i < b.len() {
        let (ns, consumed) = advance_state(b, i, &mut state);
        if state == ScanState::Normal {
            match b[i] {
                b'{' => {
                    depth += 1;
                    seen_open = true;
                }
                b'}' => {
                    depth -= 1;
                    if seen_open && depth == 0 {
                        return Some(i);
                    }
                }
                _ => {}
            }
        }
        state = ns;
        i += 1 + consumed;
    }
    None
}

fn advance_state(b: &[u8], i: usize, state: &mut ScanState) -> (ScanState, usize) {
    if let ScanState::BlockComment(depth) = *state {
        if i + 1 < b.len() && b[i] == b'*' && b[i + 1] == b'/' {
            return (
                if depth <= 1 {
                    ScanState::Normal
                } else {
                    ScanState::BlockComment(depth - 1)
                },
                1,
            );
        }
        if i + 1 < b.len() && b[i] == b'/' && b[i + 1] == b'*' {
            return (ScanState::BlockComment(depth + 1), 1);
        }
        return (ScanState::BlockComment(depth), 0);
    }
    if i + 1 < b.len() && b[i] == b'/' && b[i + 1] == b'/' {
        let mut k = i;
        while k < b.len() && b[k] != b'\n' {
            k += 1;
        }
        return (ScanState::Normal, k.saturating_sub(i).saturating_sub(1));
    }
    if i + 1 < b.len() && b[i] == b'/' && b[i + 1] == b'*' {
        return (ScanState::BlockComment(1), 1);
    }
    if let Some(end) = raw_string_span(b, i) {
        return (ScanState::Normal, end.saturating_sub(i));
    }
    if b[i] == b'"' {
        let mut k = i + 1;
        while k < b.len() {
            if b[k] == b'\\' {
                k += 2;
                continue;
            }
            if b[k] == b'"' {
                break;
            }
            k += 1;
        }
        return (ScanState::Normal, k.saturating_sub(i));
    }
    if b[i] == b'\'' {
        let mut k = i + 1;
        if k < b.len() && b[k] == b'\\' {
            k += 1;
        }
        if k < b.len() {
            k += 1;
        }
        if k < b.len() && b[k] == b'\'' {
            return (ScanState::Normal, k - i);
        }
    }
    (ScanState::Normal, 0)
}

fn raw_string_span(b: &[u8], i: usize) -> Option<usize> {
    let mut k = i;
    if k < b.len() && b[k] == b'b' {
        k += 1;
    }
    if k >= b.len() || b[k] != b'r' {
        return None;
    }
    k += 1;
    let hash_start = k;
    while k < b.len() && b[k] == b'#' {
        k += 1;
    }
    let hashes = k - hash_start;
    if k >= b.len() || b[k] != b'"' {
        return None;
    }
    k += 1;
    while k < b.len() {
        if b[k] == b'"' {
            let mut h = 0;
            while h < hashes && k + 1 + h < b.len() && b[k + 1 + h] == b'#' {
                h += 1;
            }
            if h == hashes {
                return Some(k + hashes);
            }
        }
        k += 1;
    }
    Some(b.len().saturating_sub(1))
}

fn find_semicolon(b: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i < b.len() {
        if b[i] == b';' {
            return Some(i);
        }
        i += 1;
    }
    None
}

const SPAWN_NEEDLES: &[&str] = &[
    "tokio::spawn(",
    "tokio::task::spawn_blocking(",
    "std::thread::spawn(",
    "thread::spawn(",
    // A JoinSet/TaskTracker `.spawn(` (an owned-group spawn).
    ".spawn(",
];

#[test]
fn every_non_test_spawn_has_a_pinned_disposition() {
    let mut files = Vec::new();
    rs_files(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    let mut found: Vec<(String, usize)> = Vec::new();
    for (path, src) in &files {
        for (line_no, text) in surviving_lines(src) {
            if text.trim_start().starts_with("//") {
                continue;
            }
            if SPAWN_NEEDLES.iter().any(|needle| text.contains(needle)) {
                found.push((path.clone(), line_no));
            }
        }
    }

    let mut unexplained = Vec::new();
    for (path, line) in &found {
        let ok = DISPOSITION
            .iter()
            .any(|(f, l, _)| Path::new(path).ends_with(f) && *l as usize == *line);
        if !ok {
            unexplained.push(format!("{path}:{line}"));
        }
    }
    assert!(
        unexplained.is_empty(),
        "untracked spawn sites (add a DISPOSITION entry with owner + cancel path):\n{}",
        unexplained.join("\n")
    );
    // Every pinned entry must still name a real spawn, so the table cannot rot
    // past a removed/renamed spawn.
    let mut stale = Vec::new();
    for (f, l, _) in DISPOSITION {
        let hit = found
            .iter()
            .any(|(path, line)| Path::new(path).ends_with(f) && *l as usize == *line);
        if !hit {
            stale.push(format!("{f}:{l}"));
        }
    }
    assert!(
        stale.is_empty(),
        "DISPOSITION entries no longer match a real spawn (update the table):\n{}",
        stale.join("\n")
    );
}
