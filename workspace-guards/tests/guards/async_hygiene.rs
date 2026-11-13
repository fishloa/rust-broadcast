//! Tripwire guard (issue #2515, items 3 and 4): the tokio adapters of the runtime
//! crates make no blocking call, and no crate that a peer can push into holds an
//! unbounded queue.
//!
//! Blocking needles are scanned in the runtime crates' non-test source (their
//! adapters run on the caller's reactor); an unbounded channel is banned in every
//! crate of [`UNBOUNDED_CRATES`], because a peer that outpaces the reader would
//! grow it without limit.
//!
//! **This is a lexical tripwire, not a proof.** A renamed wrapper evades it; code
//! review is the real control.

use workspace_guards::src_files;

const BLOCKING_NEEDLES: &[&str] = &[
    "std::fs::",
    "File::open(",
    "thread::sleep(",
    ".block_on(",
    ".blocking_send(",
    ".blocking_recv(",
    ".blocking_lock(",
    "std::net::TcpStream",
    "std::net::TcpListener",
    "std::net::UdpSocket",
    "reqwest::blocking",
];

const UNBOUNDED_NEEDLES: &[&str] = &[
    "unbounded_channel(",
    "UnboundedSender",
    "UnboundedReceiver",
    "mpsc::unbounded",
];

/// Runtime crates whose adapters run on the caller's reactor.
const BLOCKING_CRATES: &[&str] = &[
    "broadcast-client",
    "dash-runtime",
    "hls-runtime",
    "rtmp-runtime",
    "rtsp-runtime",
    "smooth-runtime",
    "srt-runtime",
    "webrtc-runtime",
];

const UNBOUNDED_CRATES: &[&str] = &[
    "broadcast-client",
    "dash-runtime",
    "dvb-stream",
    "hls-runtime",
    "media-plane",
    "multimux",
    "rtmp-runtime",
    "rtsp-runtime",
    "smooth-runtime",
    "srt-runtime",
    "webrtc-runtime",
];

fn scan(crate_name: &str, files: &[(String, String)], needles: &[&str]) -> Vec<String> {
    let mut violations = Vec::new();
    for (rel, text) in files {
        for (no, line) in workspace_guards::scan::non_test_source_blanked(text)
            .lines()
            .enumerate()
            .map(|(i, l)| (i + 1, l))
            .filter(|(_, l)| !l.trim_start().starts_with("//"))
        {
            if let Some(needle) = needles.iter().find(|n| line.contains(**n)) {
                violations.push(format!("{crate_name}: {rel}:{no}: `{needle}`"));
            }
        }
    }
    violations
}

#[test]
fn no_runtime_adapter_makes_a_blocking_call() {
    let mut violations = Vec::new();
    for name in BLOCKING_CRATES {
        let files = src_files(name);
        assert!(!files.is_empty(), "{name}: no source files under src/");
        violations.extend(scan(name, &files, BLOCKING_NEEDLES));
    }
    assert!(
        violations.is_empty(),
        "blocking call in a runtime adapter (use the async equivalent or spawn_blocking in the host):\n{}",
        violations.join("\n")
    );
}

#[test]
fn no_peer_fed_queue_is_unbounded() {
    let mut violations = Vec::new();
    for name in UNBOUNDED_CRATES {
        let files = src_files(name);
        assert!(!files.is_empty(), "{name}: no source files under src/");
        violations.extend(scan(name, &files, UNBOUNDED_NEEDLES));
    }
    assert!(
        violations.is_empty(),
        "unbounded channel (bound it so a fast peer cannot grow it without limit):\n{}",
        violations.join("\n")
    );
}

#[test]
fn table_names_are_workspace_members() {
    let all: Vec<&str> = BLOCKING_CRATES
        .iter()
        .chain(UNBOUNDED_CRATES)
        .copied()
        .collect();
    crate::common::assert_names_are_members("async_hygiene", &all);
}

#[test]
fn the_guard_reports_a_blocking_call_and_an_unbounded_channel() {
    const SOURCE: &str = "async fn f() {\n    let b = std::fs::read(\"x\");\n    let (t, r) = tokio::sync::mpsc::unbounded_channel();\n}\n";
    let files = vec![("src/lib.rs".to_string(), SOURCE.to_string())];
    assert_eq!(scan("t", &files, BLOCKING_NEEDLES).len(), 1);
    assert_eq!(scan("t", &files, UNBOUNDED_NEEDLES).len(), 1);
    let test_only = format!("#[cfg(test)]\nmod tests {{\n{SOURCE}}}\n");
    let files = vec![("src/lib.rs".to_string(), test_only)];
    assert!(scan("t", &files, BLOCKING_NEEDLES).is_empty());
}
