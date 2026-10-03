# De-hand-roll W1-R-low-a — rtsp-runtime, rtmp-runtime, dvb-stream Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace every hand-rolled framing loop, header scan, URL build and unbounded socket wait in `rtsp-runtime`, `rtmp-runtime` and `dvb-stream` with `tokio_util::codec` adapters over the existing sans-IO cores, typed RTSP headers, the `url` crate and `socket2`. Add the new `rtmp-runtime` client adapter. Fix defects 4 (rtsp/rtmp: no timeouts) and 8 (rtmp tcUrl on IPv6), plus one extra defect found while reading the code (the RTSP `stale=` scan misses `stale` as the first challenge parameter).

**Architecture:** One branch `w1/r-low-a` in worktree `.worktree/w1-r-low-a`. Each sans-IO core keeps its public API. Each socket adapter becomes a `tokio_util::codec::Framed` whose `Decoder`/`Encoder` delegate to the core, plus a config struct with explicit `connect` / `handshake` / `read_idle` / `write` timeouts. Timer-bearing cores gain `poll_timeout` / `handle_timeout` (RTSP keepalive). Wire output is pinned by goldens taken from `main` before the first change. This is the first half of the R-low split; `w1-rlow-b` (srt, webrtc, hls) is independent and may run in parallel.

**Tech Stack:** Rust 1.95 workspace, edition 2024, `--locked`. New dependencies: `tokio-util 0.7.19` (features `codec`, `net`, `rt`), `futures-util 0.3` (feature `sink`), `bytes 1`, `url 2` (rtmp), `socket2 0.6` (dvb-stream), `http-auth 0.1` (rtsp, already in the lock), `sdp-types 0.2` (rtsp). All already in `Cargo.lock` except `sdp-types 0.2.0` (new package, MSRV 1.71).

**Spec:** `docs/superpowers/specs/2026-10-03-protocol-runtime-dehandroll-design.md` — §3 (RTSP, rtmp tcUrl, dvb-stream rows), §4 SP1.1–SP1.4, SP1.6 (socket2 in dvb-stream), SP3 (rtmp tcUrl), SP4 (sdp-types 0.2 via rtsp-runtime), SP6.1 (new rtmp client adapter), SP7, §5 guards, §7 W1 R-low row, §8 versioning. Defects 4 (rtsp, rtmp) and 8 (rtmp).

## Global Constraints

Copied from spec §2, plus the owner decisions that apply to this cluster.

- MSRV 1.95.0; committed `Cargo.lock`, always `--locked`. A dependency add or bump may change only the intended lock entries; restore anything else with `cargo update -p <pkg> --precise <old>`.
- Every new or bumped crate supports MSRV 1.95: verified for all 34 bumps (max 1.89) and for the new crates (`tokio-util` 1.71, `socket2` 1.70, `backon` 1.85, `parking_lot` 1.71, `lru` 1.85, `jiff` 1.70, `hex`, `base64`, `arc-swap`, `wait-timeout`).
- No Co-Authored-By or Claude-Session trailers on commits.
- Nothing is tagged or published without the owner's explicit sign-off.
- Epoch purity: if a bumped dependency's types appear in a crate's public API, that crate takes a major-class version change. Each wave records this in `.delegate/release-versions.txt`.

Owner decisions that apply here:

- Q1/Q2: `no_std` may be dropped only in the crates this work touches; none of the three crates in this plan is `no_std`, so nothing is dropped here.
- SP1.1: each stream adapter is a `tokio_util::codec::Framed` whose `Decoder`/`Encoder` delegate to the existing sans-IO core. The cores keep their APIs. rtmp's `pending_write` cancel-safety code is deleted.
- SP1.3: every adapter takes a config struct with `connect`, `handshake`, `read_idle` and `write`, all `Duration` with documented defaults. No awaited IO is unbounded.
- SP1.4: every spawn is owned by a `JoinSet` or `TaskTracker`; shutdown uses `CancellationToken` only.
- SP1.6: `socket2` is called directly at each bind site (dvb-stream here); there is no shared wrapper crate.
- SP3 option (a) for URLs with no absolute base: not needed in this plan (rtmp URLs are always absolute).
- SP4: `sdp-types` 0.2 lands in `rtsp-runtime` here; `multimux` follows in W2.
- Take all dependency bumps.

Plan-specific rules:

- Never run two cargo commands concurrently (lock contention). Another wave (W1-T, W1-P, W1-R-low-b) may be running in its own worktree with its own `target/`; do not share a target dir.
- When asserting exit status of a cargo command through the rtk hook, run it as `rtk proxy cargo ...` or read the printed `test result:` lines; loop/variable arguments make the hooked exit code unreliable (memory: "cargo exit codes lie via rtk").
- New public enums must be added to the crate's `tests/label_coverage.rs` SKIP list (with a reason) or get `name()` + `Display`; they must be `#[non_exhaustive]` (`tests/non_exhaustive_coverage.rs`). This plan prefers `pub(crate)` types and `#[non_exhaustive]` structs with `with_*` builders so no new public enum is added.
- Every `Instant` / sleep in a new test is paused time (`#[tokio::test(start_paused = true)]`) over `tokio::io::duplex`, never real sockets plus paused time (auto-advance races real IO).
- Code blocks below were written against the registry sources of the versions in `Cargo.lock` (and `tokio-util 0.7.19`, `rtsp-types 0.1.3`, `http-auth 0.1.10`, `sdp-types 0.2.0`, `url 2.5.8`, `socket2 0.6.5`). They were not compiled when the plan was written; the TDD "expected FAIL / PASS" steps are the compile check. If a signature differs, fix the call site and note it in the report; never change a test's assertion to make it pass.

## Review Focus

The five inputs most likely to bite users that no existing test covers. Each has a test in the named task.

1. **RTSP messages split at every byte boundary, and bodies that never finish.** A request or response arriving one byte at a time, a `Content-Length` larger than the buffer cap, and a header block that never terminates. These were the quadratic-reparse and slow-loris shapes. Owned by Task 5 (client codec, step 1) and Task 6 (server codec, step 1).
2. **Digest `stale` and `Session` header spelling.** `WWW-Authenticate: Digest stale=true, realm="a,b", nonce="x"` (stale first, comma inside a quoted realm), `Session: abc; timeout=30` / `Session: abc;timeout = 30` / `Session: "weird"`. Owned by Task 3, step 1.
3. **Cancelled adapter calls losing a reply.** `RtmpConnection::next_events` dropped mid-flush (a `select!` loser) must still deliver every reply byte exactly once. Owned by Task 8, step 1.
4. **RTMP URL shapes.** `rtmp://[::1]:1935/live/key`, `rtmp://user:pw@host/live/key?token=a/b`, no port, trailing slash, stream key containing `/`. Owned by Task 9, step 1.
5. **TS framing edges in dvb-stream.** A corrupt sync byte mid-stream, a datagram carrying 10 packets plus 5 stray bytes, a datagram of pure junk (must not stall or grow the buffer), and a reader that returns one byte per read. Owned by Task 10, step 1 and Task 11, step 1.

---

### Task 0: Worktree setup and baseline

**Files:** none (environment only), plus `.delegate/w1-r-low-a-report.md` (created).

- [ ] **Step 1: Create the worktree off main**

```bash
cd /Volumes/External/Projects/rust-broadcast
git fetch -q origin
git worktree add -b w1/r-low-a .worktree/w1-r-low-a origin/main
cd .worktree/w1-r-low-a
git -c protocol.file.allow=always submodule update -q --init --reference /Volumes/External/Projects/rust-broadcast/private private
ln -s /Volumes/External/Projects/rust-broadcast/.test-streams .test-streams
ln -s /Volumes/External/Projects/rust-broadcast/multimux/tests/assets/node_modules multimux/tests/assets/node_modules
git rev-parse HEAD
```

Expected: a commit hash. Record it as `BASE` in `.delegate/w1-r-low-a-report.md` (create the file, header `# W1-R-low-a report`, line `BASE=<hash>`). Every revert-check below restores files from `$BASE`.

- [ ] **Step 2: Baseline. The three crates must pass BEFORE any change**

```bash
timeout 1800 cargo test --locked --all-features -p rtsp-runtime -p rtmp-runtime -p dvb-stream 2>&1 | grep -E '^test result|FAILED|panicked|^error' | sort | uniq -c
```

Expected: only `test result: ok` lines (the ffmpeg-dependent tests skip themselves when `ffmpeg` is absent). Record the per-binary passed counts in the report under "baseline". Also run the three crates' no-default-features builds and record that they pass:

```bash
for c in rtsp-runtime rtmp-runtime dvb-stream; do cargo build --locked -p $c --no-default-features 2>&1 | tail -1; done
```

Expected: three `Finished` lines.

- [ ] **Step 3: Record what multimux consumes from these crates (the compile-fix surface)**

```bash
grep -rnE 'rtsp_runtime::|rtmp_runtime::|dvb_stream::' multimux/src multimux/tests fuzz/fuzz_targets | cut -c1-140 > /tmp/w1-rlow-a-consumers.txt; wc -l /tmp/w1-rlow-a-consumers.txt
```

Expected: ~25 lines, all of them sans-IO core types (`ClientSession`, `ServerSession`, `Transport`, `TransportSpec`, `InterleavedFrame`, `Credentials`, `rtmp_runtime::{amf0,chunk,client,server,io::{AsyncRtmpServer,RtmpConnection}}`) plus `rtsp_runtime::io::default_tls_client_config` and the `RTSP_DEFAULT_PORT` consts. `dvb_stream` is not used by multimux. No task below changes any of those signatures, so no multimux edit is expected; Task 14 re-checks with `cargo build -p multimux --all-features --locked`.

---

### Task 1: Goldens from main (RTSP transcript, RTMP client transcript, dvb-stream events)

Goldens are generated from `main` BEFORE any behaviour change, committed, and compared byte-for-byte by later tasks.

**Files:**
- Create: `rtsp-runtime/tests/golden_wire.rs`
- Create: `rtsp-runtime/tests/golden/session_transcript.golden`, `rtsp-runtime/tests/golden/transport_parsed.golden`, `rtsp-runtime/tests/golden/transport_header.golden`, `rtsp-runtime/tests/golden/README.md`
- Create: `rtmp-runtime/tests/golden_wire.rs`, `rtmp-runtime/tests/golden/client_publish.golden`, `rtmp-runtime/tests/golden/README.md`
- Create: `dvb-stream/tests/golden_events.rs`, `dvb-stream/tests/golden/m6_single_events.golden`, `dvb-stream/tests/golden/README.md`

**Interfaces:** none new. The tests use only the existing public API. Each test compares to its golden file; with `GOLDEN_UPDATE=1` it rewrites the file instead. Later tasks never set `GOLDEN_UPDATE` except Task 4, which regenerates exactly one file (`transport_header.golden`) after a reviewed diff.

- [ ] **Step 1: Write the RTSP golden test**

`rtsp-runtime/tests/golden_wire.rs`:

```rust
//! Byte-for-byte goldens for the RTSP wire output (W1-R-low-a, spec §6).
//!
//! `session_transcript.golden` is a full client<->server exchange, both ends the
//! sans-IO cores, with a fixed User-Agent and a deterministic session id.
//! `transport_parsed.golden` is the `Debug` of each parsed `Transport` (semantic
//! pin, must never change). `transport_header.golden` is the serialized header
//! text (byte pin; Task 4 may change parameter ORDER only, and says so in the
//! CHANGELOG). Regenerate only with `GOLDEN_UPDATE=1`.

use rtsp_runtime::{ClientSession, ServerSession, Transport, TransportSpec};

const URI: &str = "rtsp://example.test/stream";

fn golden(name: &str, actual: &str) {
    let path = format!("{}/tests/golden/{name}", env!("CARGO_MANIFEST_DIR"));
    if std::env::var_os("GOLDEN_UPDATE").is_some() {
        std::fs::write(&path, actual).expect("write golden");
        return;
    }
    let want = std::fs::read_to_string(&path).expect("read golden (run once with GOLDEN_UPDATE=1 on main)");
    assert_eq!(actual, want, "golden {name} differs");
}

fn exchange(label: &str, req: Vec<u8>, c: &mut ClientSession, s: &mut ServerSession, out: &mut String) {
    out.push_str(&format!("--- {label} request ---\n{}", String::from_utf8_lossy(&req)));
    let (resp, _) = s.handle_request(&req).expect("server handles request");
    out.push_str(&format!("--- {label} response ---\n{}", String::from_utf8_lossy(&resp)));
    c.handle_data(&resp).expect("client handles response");
}

#[test]
fn session_transcript_is_byte_identical() {
    let mut c = ClientSession::new().with_user_agent("golden-ua");
    let mut s = ServerSession::new(|| 0).with_session_seed(0xABCD).with_session_timeout(60);
    let mut out = String::new();
    let r = c.options(URI).unwrap();
    exchange("OPTIONS", r, &mut c, &mut s, &mut out);
    let r = c.describe(URI).unwrap();
    // DESCRIBE has no SDP route in the bare ServerSession: record the request only.
    out.push_str(&format!("--- DESCRIBE request ---\n{}", String::from_utf8_lossy(&r)));
    let t = Transport::single(TransportSpec::rtp_avp_tcp_interleaved(0, 1));
    let r = c.setup(URI, &t).unwrap();
    exchange("SETUP", r, &mut c, &mut s, &mut out);
    let r = c.play(URI).unwrap();
    exchange("PLAY", r, &mut c, &mut s, &mut out);
    let r = c.get_parameter(URI, b"").unwrap();
    out.push_str(&format!("--- GET_PARAMETER request ---\n{}", String::from_utf8_lossy(&r)));
    let r = c.pause(URI).unwrap();
    exchange("PAUSE", r, &mut c, &mut s, &mut out);
    let r = c.teardown(URI).unwrap();
    exchange("TEARDOWN", r, &mut c, &mut s, &mut out);
    golden("session_transcript.golden", &out);
}

const TRANSPORT_CASES: &[&str] = &[
    "RTP/AVP/TCP;interleaved=0-1",
    "RTP/AVP;unicast;client_port=8000-8001",
    "RTP/AVP;unicast;client_port=8000-8001;server_port=9000-9001;ssrc=DEADBEEF",
    "RTP/AVP/UDP;multicast;destination=224.2.0.1;port=3456-3457;ttl=16",
    "RTP/AVP;unicast;mode=\"RECORD\";append;interleaved=2-3",
    "RTP/AVP;multicast;layers=2;ttl=5",
    "RTP/AVP/TCP;unicast;interleaved=4,RTP/AVP;unicast;client_port=5000-5001",
    "RTP/AVP;unicast;source=10.0.0.1;destination=10.0.0.2",
    "RTP/AVP;unicast;interleaved=6",
];

#[test]
fn transport_parse_and_header_goldens() {
    let (mut parsed, mut header) = (String::new(), String::new());
    for case in TRANSPORT_CASES {
        let t = Transport::parse(case).unwrap_or_else(|e| panic!("{case}: {e}"));
        parsed.push_str(&format!("{case}\n  {t:?}\n"));
        header.push_str(&format!("{case}\n  {}\n", t.to_header_value()));
    }
    golden("transport_parsed.golden", &parsed);
    golden("transport_header.golden", &header);
}
```

- [ ] **Step 2: Write the RTMP golden test**

`rtmp-runtime/tests/golden_wire.rs`:

```rust
//! Byte-for-byte golden of the RTMP publish client's wire output (W1-R-low-a).
//! Client and server are both the sans-IO cores, pumped in memory. The
//! handshake random fill is deterministic (`default_random_fill`), so the whole
//! transcript is stable. Regenerate only with `GOLDEN_UPDATE=1` on `main`.

use rtmp_runtime::amf0::Amf0Value;
use rtmp_runtime::client::{ClientConfig, ClientSession};
use rtmp_runtime::server::ServerSession;

fn hex(label: &str, bytes: &[u8], out: &mut String) {
    out.push_str(&format!("{label} ({} bytes)\n", bytes.len()));
    for line in bytes.chunks(32) {
        out.push_str("  ");
        for b in line {
            out.push_str(&format!("{b:02x}"));
        }
        out.push('\n');
    }
}

#[test]
fn client_publish_transcript_is_byte_identical() {
    let mut cfg = ClientConfig::default();
    cfg.app = "live".into();
    cfg.stream_key = "key".into();
    cfg.tc_url = Some("rtmp://example.test:1935/live".into());
    let mut c = ClientSession::new(cfg);
    let mut s = ServerSession::with_defaults();
    let mut out = String::new();

    let mut to_server = c.start();
    hex("C0+C1", &to_server, &mut out);
    for round in 0..8 {
        let (reply, _) = s.handle_data(&to_server).expect("server");
        hex(&format!("server reply {round}"), &reply, &mut out);
        let (next, _) = c.handle_data(&reply).expect("client");
        hex(&format!("client output {round}"), &next, &mut out);
        if c.is_publishing() {
            break;
        }
        to_server = next;
    }
    assert!(c.is_publishing(), "client must reach Publishing");
    let a = c.send_audio(40, &[0xAF, 0x01, 0x11, 0x22]).unwrap();
    hex("audio", &a, &mut out);
    let v = c.send_video(40, &[0x17, 0x01, 0, 0, 0, 0x33, 0x44]).unwrap();
    hex("video", &v, &mut out);
    let m = c
        .send_metadata(&[("width".to_string(), Amf0Value::Number(1280.0))])
        .unwrap();
    hex("metadata", &m, &mut out);

    let path = format!("{}/tests/golden/client_publish.golden", env!("CARGO_MANIFEST_DIR"));
    if std::env::var_os("GOLDEN_UPDATE").is_some() {
        std::fs::write(&path, &out).unwrap();
        return;
    }
    assert_eq!(out, std::fs::read_to_string(&path).expect("golden"), "client_publish.golden differs");
}
```

- [ ] **Step 3: Write the dvb-stream golden test**

`dvb-stream/tests/golden_events.rs`:

```rust
//! Golden of `SectionStream` over a real capture (W1-R-low-a): the full event
//! sequence (pid, table_id, version, section length, first 8 bytes) plus the
//! demux and resync counters. Regenerate only with `GOLDEN_UPDATE=1` on `main`.

use std::pin::Pin;

use dvb_stream::SectionStream;
use futures_core::Stream;

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/ts/m6-single.ts");

#[tokio::test]
async fn section_stream_events_are_byte_identical_to_golden() {
    let data = std::fs::read(FIXTURE).expect("m6-single.ts");
    let mut stream = SectionStream::new(std::io::Cursor::new(data));
    let mut out = String::new();
    while let Some(ev) = std::future::poll_fn(|cx| Pin::new(&mut stream).poll_next(cx)).await {
        out.push_str(&format!("{ev:?}\n"));
    }
    out.push_str(&format!("stats={:?}\nresync={:?}\n", stream.stats(), stream.resync_stats()));
    let path = format!("{}/tests/golden/m6_single_events.golden", env!("CARGO_MANIFEST_DIR"));
    if std::env::var_os("GOLDEN_UPDATE").is_some() {
        std::fs::write(&path, &out).unwrap();
        return;
    }
    assert_eq!(out, std::fs::read_to_string(&path).expect("golden"), "events differ");
}
```

- [ ] **Step 4: Generate the goldens on UNCHANGED main code, then verify they compare**

```bash
mkdir -p rtsp-runtime/tests/golden rtmp-runtime/tests/golden dvb-stream/tests/golden
GOLDEN_UPDATE=1 cargo test --locked -p rtsp-runtime --all-features --test golden_wire 2>&1 | grep -E '^test result|FAILED|error'
GOLDEN_UPDATE=1 cargo test --locked -p rtmp-runtime --all-features --test golden_wire 2>&1 | grep -E '^test result|FAILED|error'
GOLDEN_UPDATE=1 cargo test --locked -p dvb-stream --all-features --test golden_events 2>&1 | grep -E '^test result|FAILED|error'
# now WITHOUT the env var: must pass byte-for-byte
cargo test --locked -p rtsp-runtime -p rtmp-runtime -p dvb-stream --all-features --test golden_wire --test golden_events 2>&1 | grep -E '^test result|FAILED|error'
wc -c rtsp-runtime/tests/golden/*.golden rtmp-runtime/tests/golden/*.golden dvb-stream/tests/golden/*.golden
```

Expected: every `test result: ok`; every golden file non-empty (the session transcript is a few KB; `m6_single_events.golden` has one line per section). If the dvb-stream golden is under 500 bytes the fixture produced no events: stop and investigate (the existing `differential.rs` asserts the same fixture is non-empty).

- [ ] **Step 4b: Move the fixed UDP port off 43991 NOW (SP7, early)**

`dvb-stream/tests/error_and_datagram.rs` binds the fixed `TEST_PORT = 43_991`; other W1 branches run tests on the same machine in parallel. Interim fix on the OLD API (Task 11 replaces it with `UdpSectionStream::from_socket`): replace `const TEST_PORT` by a port picked from the OS: `let probe = std::net::UdpSocket::bind("0.0.0.0:0").unwrap(); let test_port = probe.local_addr().unwrap().port(); drop(probe);` and use `test_port` in both the bind and the `send_to`. Run `cargo test --locked -p dvb-stream --all-features --test error_and_datagram` and expect ok (the test still skips cleanly without multicast). Include the file in this task's commit.

- [ ] **Step 5: Write the three READMEs, commit**

Each `README.md` (same shape, three copies):

```markdown
Goldens for W1-R-low-a. Generated from `main` at commit <BASE> with:

    GOLDEN_UPDATE=1 cargo test --locked -p <crate> --all-features --test <golden test>

Compared byte-for-byte by the same test without `GOLDEN_UPDATE`. Only Task 4
(`transport_header.golden`, parameter order) may regenerate one, after a reviewed diff.
```

```bash
git add rtsp-runtime/tests/golden_wire.rs rtsp-runtime/tests/golden rtmp-runtime/tests/golden_wire.rs rtmp-runtime/tests/golden dvb-stream/tests/golden_events.rs dvb-stream/tests/golden dvb-stream/tests/error_and_datagram.rs
git commit -m "test(rtsp,rtmp,dvb-stream): byte-for-byte goldens from main for the W1 adapter rewrite"
```

---

### Task 2: Dependencies and the sdp-types 0.2 bump (SP4)

**Files:**
- Modify: `rtsp-runtime/Cargo.toml` lines 16-37 (`[dependencies]`) and 47 (`[dev-dependencies]` `sdp-types`)
- Modify: `rtsp-runtime/tests/integration.rs` line 346 (the only `sdp_types` use in the crate)
- Modify: `rtmp-runtime/Cargo.toml` lines 8-21
- Modify: `dvb-stream/Cargo.toml` lines 14-32
- Modify: `Cargo.lock`

**Interfaces:**
- Consumes: nothing.
- Produces: the dependency edges later tasks use (`tokio_util::codec::{Decoder,Encoder,Framed,FramedRead}`, `tokio_util::udp::UdpFramed`, `futures_util::{SinkExt,StreamExt}`, `bytes::{Bytes,BytesMut,Buf}`, `url::Url`, `socket2`, `http_auth::parse_challenges`).
- `sdp-types` moves to a dev-dependency only (SP4 bump to 0.2 stays satisfied); it is not in any `rtsp-runtime` public signature (verified: `grep -n "sdp_types" rtsp-runtime/src` shows only a doc line in `lib.rs`), so the bump is not an epoch change for rtsp-runtime. The crate's own `[dependencies]` entry is unused by `src`; it stays (spec SP4 lists the bump) and moves to 0.2.

- [ ] **Step 1: Write the failing check (the dependency is declared but at the old version)**

```bash
grep -n 'sdp-types' rtsp-runtime/Cargo.toml
```

Expected: two lines, both `"0.1"`.

- [ ] **Step 2: Edit the manifests**

`rtsp-runtime/Cargo.toml` `[dependencies]` — REMOVE the `sdp-types  = "0.1"` line (no `src` line uses it; review finding A4: a dead dependency is caught by no gate) and add (after `getrandom`):

```toml
# Typed RTSP/auth header handling replaces hand-rolled scans (W1 SP1/SP2): the
# stale= scan uses `parse_challenges`, Session/Transport go through rtsp-types.
http-auth     = { version = "0.1", default-features = false }
# Framed adapters (SP1.1). `codec` = Decoder/Encoder/Framed.
tokio-util    = { version = "0.7", optional = true, default-features = false, features = ["codec"] }
futures-util  = { version = "0.3", optional = true, default-features = false, features = ["sink", "std"] }
bytes         = { version = "1", optional = true }
```

and change the `tokio` feature line to:

```toml
tokio = ["dep:tokio", "dep:getrandom", "dep:tokio-util", "dep:futures-util", "dep:bytes"]
```

`[dev-dependencies]`: `sdp-types = "0.2"` (the only user is `tests/integration.rs`), plus `tokio = { version = "1", features = ["full", "test-util"] }`.

`rtmp-runtime/Cargo.toml`: replace the tokio line and feature:

```toml
tokio = { version = "1", optional = true, features = ["net", "io-util", "rt", "macros", "sync", "time"] }
tokio-util   = { version = "0.7", optional = true, default-features = false, features = ["codec"] }
futures-util = { version = "0.3", optional = true, default-features = false, features = ["sink", "std"] }
bytes        = { version = "1", optional = true }
url          = { version = "2", optional = true }
...
tokio = ["dep:tokio", "dep:tokio-util", "dep:futures-util", "dep:bytes", "dep:url"]
```

and under `[dev-dependencies]` add `tokio = { version = "1", features = ["full", "test-util"] }`.

`dvb-stream/Cargo.toml`: `tokio` gets `"time"`, add:

```toml
tokio-util   = { version = "0.7", default-features = false, features = ["codec", "net"] }
bytes        = "1"
socket2      = { version = "0.6", optional = true, features = ["all"] }
...
udp = ["dep:socket2"]
```

- [ ] **Step 3: Fix the one sdp-types call site**

`rtsp-runtime/tests/integration.rs` line 346 calls `sdp_types::Session::parse(&sdp)`; 0.2.0 keeps `Session::parse(data: &[u8]) -> Result<Session, ParserError>` (`sdp-types-0.2.0/src/parser.rs:436`). Run the file; if `sdp` is a `String`, the call is already `&sdp` coerced from `&String`? It is not (`&String` does not coerce to `&[u8]`): change to `Session::parse(sdp.as_bytes())` if the compiler complains, nothing else.

- [ ] **Step 4: Update the lock and verify only the intended entries changed**

```bash
CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=fallback cargo update -p sdp-types --precise 0.2.0 2>&1 | tail -3
git diff Cargo.lock | grep -E '^[-+]name|^[-+]version' | paste - - | sort | uniq
```

Expected: `sdp-types` 0.1.8 -> 0.2.0 only if no other workspace member still needs 0.1 (multimux and transmux still declare `0.1`, so BOTH `sdp-types 0.1.8` and `0.2.0` stay in the lock; that is expected until W2/W1-T). New lock packages allowed: `sdp-types 0.2.0` and whatever it adds (`fallible-iterator`, `hex` are already present or tiny). Any other changed package: restore with `cargo update -p <pkg> --precise <old>` and record it in the report.

- [ ] **Step 5: Build and run the unchanged suites**

```bash
cargo build --locked --workspace --all-features 2>&1 | tail -2
cargo test --locked -p rtsp-runtime -p rtmp-runtime -p dvb-stream --all-features 2>&1 | grep -E '^test result|FAILED|panicked|^error' | sort | uniq -c
```

Expected: workspace builds; the same pass counts as Task 0 plus the three new golden tests.

- [ ] **Step 6: Commit**

```bash
git add rtsp-runtime/Cargo.toml rtsp-runtime/tests/integration.rs rtmp-runtime/Cargo.toml dvb-stream/Cargo.toml Cargo.lock
git commit -m "chore(deps): tokio-util/futures-util/url/socket2 for the W1 adapters; rtsp-runtime to sdp-types 0.2"
```

---

### Task 3: RTSP `Session`/`stale` through typed parsers (defect: `stale` scan)

Replaces `parse_session` (client.rs 556-564), `challenge_is_stale` (client.rs 543-554) and the server's `Session` id split (server.rs 266). Fixes a real bug: `challenge_is_stale` splits the whole challenge on `,`, so `Digest stale=true, ...` yields the first segment `Digest stale=true`, whose name is `Digest stale`, so `stale=true` as the first parameter is missed; a comma inside a quoted `realm` also mis-splits.

**Files:**
- Modify: `rtsp-runtime/src/client.rs`: remove `challenge_is_stale` and `parse_session` (lines 539-564); callers at lines 396 (Session capture) and 463 (`stale`); tests at 812-818.
- Modify: `rtsp-runtime/src/server.rs` line 266 (`session_header_matches`).
- Create: `rtsp-runtime/src/headers_util.rs` (crate-private).
- Modify: `rtsp-runtime/src/lib.rs` (add `mod headers_util;`).

**Interfaces** (all `pub(crate)`):

```rust
// rtsp-runtime/src/headers_util.rs
/// (session id, optional timeout seconds) from a `Session` header value.
pub(crate) fn parse_session(value: &str) -> Option<(String, Option<u64>)>;
/// True if any `Digest` challenge in a `WWW-Authenticate` value carries `stale=true`.
pub(crate) fn challenge_is_stale(challenge: &str) -> bool;
/// The session id of a `Session` header value (id only, timeout ignored).
pub(crate) fn session_id_of(value: &str) -> Option<String>;
```

- [ ] **Step 1: Write the failing tests**

Create `rtsp-runtime/src/headers_util.rs` with only the test module first (the functions do not exist yet, so this fails to compile):

```rust
//! Typed header helpers over `rtsp_types::headers::Session` and
//! `http_auth::parse_challenges`; replaces hand-rolled `split(',')` scans.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_is_found_as_the_first_parameter_after_the_scheme() {
        // Old scan: first segment is "Digest stale=true" -> name "Digest stale" -> missed.
        assert!(challenge_is_stale(r#"Digest stale=true, realm="cam", nonce="n""#));
    }

    #[test]
    fn stale_is_not_confused_by_a_comma_inside_a_quoted_realm() {
        assert!(!challenge_is_stale(r#"Digest realm="a, stale=true", nonce="n""#));
        assert!(challenge_is_stale(r#"Digest realm="a,b", nonce="n", stale="TRUE""#));
    }

    #[test]
    fn stale_matches_the_existing_cases() {
        assert!(challenge_is_stale(r#"Digest realm="cam",nonce="x",stale="True""#));
        assert!(challenge_is_stale(r#"Digest realm="cam",STALE=true"#));
        assert!(!challenge_is_stale(r#"Digest realm="cam",nonce="x""#));
        assert!(!challenge_is_stale(r#"Digest realm="cam",stale=false"#));
        // A Basic challenge next to a Digest one: only Digest's params count.
        assert!(!challenge_is_stale(r#"Basic realm="x", stale=true"#));
    }

    #[test]
    fn session_header_forms() {
        assert_eq!(parse_session("abc"), Some(("abc".into(), None)));
        assert_eq!(parse_session("abc;timeout=30"), Some(("abc".into(), Some(30))));
        assert_eq!(parse_session("abc ; timeout=30"), Some(("abc".into(), Some(30))));
        assert_eq!(parse_session("0000000000abcdef;timeout=60"), Some(("0000000000abcdef".into(), Some(60))));
        assert_eq!(parse_session(""), None);
        assert_eq!(session_id_of("abc;timeout=5").as_deref(), Some("abc"));
    }
}
```

- [ ] **Step 2: Run, expect FAIL (compile error)**

```bash
cargo test --locked -p rtsp-runtime --all-features --lib headers_util 2>&1 | grep -E '^error|cannot find function|test result'
```

Expected: `cannot find function `challenge_is_stale`` / `parse_session` (the module is also not yet declared: add `mod headers_util;` to `lib.rs` first so the error is the missing functions).

- [ ] **Step 3: Implement**

Above the test module in `headers_util.rs`:

```rust
use rtsp_types::headers::{self, Session};
use rtsp_types::{Response, StatusCode, Version};

/// `Session` header value -> (id, timeout), through the typed `rtsp_types` header.
/// (`rtsp_types::headers::Headers::new` is crate-private, so the value is parsed
/// by lodging it in a throw-away `Response`, exactly as rtsp-types' own tests do.)
pub(crate) fn parse_session(value: &str) -> Option<(String, Option<u64>)> {
    let resp = Response::builder(Version::V1_0, StatusCode::Ok)
        .header(headers::SESSION, value)
        .empty();
    let s = resp.typed_header::<Session>().ok()??;
    if s.0.is_empty() {
        return None;
    }
    Some((s.0.clone(), s.1))
}

pub(crate) fn session_id_of(value: &str) -> Option<String>;
```

- [ ] **Step 1: Write the failing tests**

Create `rtsp-runtime/src/headers_util.rs` with only the test module first (the functions do not exist yet, so this fails to compile):

```rust
//! Typed header helpers over `rtsp_types::headers::Session` and
//! `http_auth::parse_challenges`; replaces hand-rolled `split(',')` scans.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_is_found_as_the_first_parameter_after_the_scheme() {
        // Old scan: first segment is "Digest stale=true" -> name "Digest stale" -> missed.
        assert!(challenge_is_stale(r#"Digest stale=true, realm="cam", nonce="n""#));
    }

    #[test]
    fn stale_is_not_confused_by_a_comma_inside_a_quoted_realm() {
        assert!(!challenge_is_stale(r#"Digest realm="a, stale=true", nonce="n""#));
        assert!(challenge_is_stale(r#"Digest realm="a,b", nonce="n", stale="TRUE""#));
    }

    #[test]
    fn stale_matches_the_existing_cases() {
        assert!(challenge_is_stale(r#"Digest realm="cam",nonce="x",stale="True""#));
        assert!(challenge_is_stale(r#"Digest realm="cam",STALE=true"#));
        assert!(!challenge_is_stale(r#"Digest realm="cam",nonce="x""#));
        assert!(!challenge_is_stale(r#"Digest realm="cam",stale=false"#));
        // A Basic challenge next to a Digest one: only Digest's params count.
        assert!(!challenge_is_stale(r#"Basic realm="x", stale=true"#));
    }

    #[test]
    fn session_header_forms() {
        assert_eq!(parse_session("abc"), Some(("abc".into(), None)));
        assert_eq!(parse_session("abc;timeout=30"), Some(("abc".into(), Some(30))));
        assert_eq!(parse_session("abc ; timeout=30"), Some(("abc".into(), Some(30))));
        assert_eq!(parse_session("0000000000abcdef;timeout=60"), Some(("0000000000abcdef".into(), Some(60))));
        assert_eq!(parse_session(""), None);
        assert_eq!(session_id_of("abc;timeout=5").as_deref(), Some("abc"));
    }
}
```

- [ ] **Step 2: Run, expect FAIL (compile error)**

```bash
cargo test --locked -p rtsp-runtime --all-features --lib headers_util 2>&1 | grep -E '^error|cannot find function|test result'
```

Expected: `cannot find function `challenge_is_stale`` / `parse_session` (the module is also not yet declared: add `mod headers_util;` to `lib.rs` first so the error is the missing functions).

- [ ] **Step 3: Implement**

Above the test module in `headers_util.rs`:

```rust
use rtsp_types::headers::{self, HeaderName, Headers, Session, TypedHeader};

/// `Session` header value -> (id, timeout), through the typed `rtsp_types` header.
pub(crate) fn parse_session(value: &str) -> Option<(String, Option<u64>)> {
    let mut h = Headers::new();
    h.insert(headers::SESSION, value.to_string());
    let s = Session::from_headers(&h).ok()??;
    if s.0.is_empty() {
        return None;
    }
    Some((s.0.clone(), s.1))
}

pub(crate) fn session_id_of(value: &str) -> Option<String> {
    parse_session(value).map(|(id, _)| id)
}

/// `stale=true` in any Digest challenge (RFC 7616 §3.3): parsed with the
/// RFC 7235 challenge grammar, so quoted commas and a leading `stale` are right.
pub(crate) fn challenge_is_stale(challenge: &str) -> bool {
    let Ok(challenges) = http_auth::parse_challenges(challenge) else {
        return false;
    };
    challenges
        .iter()
        .filter(|c| c.scheme.eq_ignore_ascii_case("digest"))
        .flat_map(|c| c.params.iter())
        .any(|(name, value)| {
            name.eq_ignore_ascii_case("stale") && value.to_unescaped().eq_ignore_ascii_case("true")
        })
}
```

(`Response::builder(Version, StatusCode)`, `ResponseBuilder::header(name, value)`, `.empty()` and `Response::typed_header::<H>()` are in `rtsp-types-0.1.3/src/message.rs` lines 630, 900, 916, 832.) Then in `client.rs` delete the two old fns and their unit tests (lines 539-564 and 812-818, which `headers_util::tests::stale_matches_the_existing_cases` now supersedes) and call `crate::headers_util::{challenge_is_stale, parse_session}`: at line 396 use `if let Some((id, timeout)) = header_value(response.header(&headers::SESSION)).and_then(crate::headers_util::parse_session)`; in `server.rs::session_header_matches` replace `value.split(';').next()...trim()` with `crate::headers_util::session_id_of(value)` and compare with `ids_equal`.

- [ ] **Step 4: Run, expect PASS; the existing client/server unit tests and goldens still pass**

```bash
cargo test --locked -p rtsp-runtime --all-features 2>&1 | grep -E '^test result|FAILED|panicked|^error' | sort | uniq -c
```

Expected: all ok, including `golden_wire` (the transcript is unchanged) and the existing `stale_quoted_rechallenge_refreshes_nonce_after_a_prior_retry`.

- [ ] **Step 5: Revert-check (defect evidence)**

```bash
git show $BASE:rtsp-runtime/src/client.rs | sed -n '/^fn challenge_is_stale/,/^}/p' > /tmp/old_stale.rs
cat /tmp/old_stale.rs | head -3
```

Temporarily replace the body of `challenge_is_stale` in `headers_util.rs` with the old `challenge.split(',').any(|param| ...)` body from `/tmp/old_stale.rs`, run:

```bash
cargo test --locked -p rtsp-runtime --all-features --lib headers_util 2>&1 | grep -E 'test result|FAILED|stale_is_found'
```

Expected: `FAILED` for `stale_is_found_as_the_first_parameter_after_the_scheme` and `stale_is_not_confused_by_a_comma_inside_a_quoted_realm` (record the two failing test names in the report). Restore with `git checkout -- rtsp-runtime/src/headers_util.rs` and re-run Step 4.

- [ ] **Step 6: Commit**

```bash
git add rtsp-runtime/src
git commit -m "fix(rtsp-runtime): parse Session and WWW-Authenticate stale with typed parsers; stale as first parameter was missed"
```

---

### Task 4: `Transport` header through `rtsp_types::headers::Transports`

The public `Transport`/`TransportSpec`/`LowerTransport`/`Delivery` types and their `parse`/`to_header_value` signatures stay; only the hand-rolled `parse_spec`, `parse_scalar`, `parse_u8_range`, `parse_u16_range`, `split_range` and the string builder (transport.rs 127-421) are replaced by conversions to and from `rtsp_types::headers::{Transports, Transport as RtpT, RtpTransport, RtpTransportParameters}`.

**Files:**
- Modify: `rtsp-runtime/src/transport.rs` lines 126-210 (`parse_spec`), 213-256 (`to_header_value`), 286-305 (`Transport::parse/to_header_value`), 312-346 (helpers); keep the type definitions (lines 17-123) and the tests (348-421).
- Modify: `rtsp-runtime/tests/golden/transport_header.golden` (regenerated once, after review).

**Interfaces:** unchanged public signatures:

```rust
impl Transport { pub fn parse(value: &str) -> Result<Self>; pub fn to_header_value(&self) -> String; }
impl TransportSpec { pub fn to_header_value(&self) -> String; }
```

Behaviour contract: `transport_parsed.golden` (the `Debug` of every parsed case) must stay byte-identical. `transport_header.golden` may change, and only in parameter ORDER and only for the cases below; every difference goes into the CHANGELOG with an example.

- [ ] **Step 1: Write the failing tests (semantic pin plus the order table)**

Append to `rtsp-runtime/src/transport.rs`'s test module:

```rust
    // `rtsp_types` emits parameters in its own fixed order. The order is not
    // semantically significant (RFC 2326 §12.39 grammar is an unordered list).
    #[test]
    fn serialisation_order_is_the_typed_headers_order() {
        let spec = TransportSpec {
            lower_transport: Some(LowerTransport::Udp),
            delivery: Some(Delivery::Multicast),
            destination: Some("224.2.0.1".into()),
            port: Some((3456, 3457)),
            ttl: Some(16),
            ..Default::default()
        };
        assert_eq!(
            spec.to_header_value(),
            "RTP/AVP/UDP;multicast;ttl=16;port=3456-3457;destination=224.2.0.1"
        );
    }

    #[test]
    fn single_channel_interleaved_round_trips_as_a_pair() {
        let t = Transport::parse("RTP/AVP;unicast;interleaved=6").unwrap();
        assert_eq!(t.first().unwrap().interleaved, Some((6, 6)));
    }

    #[test]
    fn layers_survive_via_the_others_map() {
        let t = Transport::parse("RTP/AVP;multicast;layers=2;ttl=5").unwrap();
        assert_eq!(t.first().unwrap().layers, Some(2));
        assert!(t.to_header_value().contains("layers=2"));
    }

    /// RFC 2326 §12.39 tokens and parameter names are case-insensitive; the old parser accepted
    /// any case, the typed rtsp-types parser is exact-case, so the value is case-normalised first.
    #[test]
    fn mixed_case_tokens_and_parameter_names_parse_like_the_old_parser() {
        let t = Transport::parse("RTP/AVP/TCP;Interleaved=0-1").unwrap();
        assert_eq!(t.first().unwrap().interleaved, Some((0, 1)));
        let t = Transport::parse("rtp/avp/udp;UNICAST;Client_Port=8000-8001;SSRC=DEADBEEF").unwrap();
        let s = t.first().unwrap();
        assert_eq!(s.lower_transport, Some(LowerTransport::Udp));
        assert_eq!(s.delivery, Some(Delivery::Unicast));
        assert_eq!(s.client_port, Some((8000, 8001)));
        assert_eq!(s.ssrc, Some(0xDEAD_BEEF));
        // values are NOT touched: a destination keeps its case
        let t = Transport::parse("RTP/AVP;multicast;Destination=Cam.Example").unwrap();
        assert_eq!(t.first().unwrap().destination.as_deref(), Some("Cam.Example"));
    }

    #[test]
    fn layers_round_trip_through_the_others_map() {
        let t = Transport::parse("RTP/AVP;multicast;layers=3;ttl=5").unwrap();
        let again = Transport::parse(&t.to_header_value()).unwrap();
        assert_eq!(t, again);
        assert_eq!(again.first().unwrap().layers, Some(3));
    }

    /// Duplicated parameters: last one wins, as in the old parser (pinned so the typed parser's
    /// "each parameter appears once" assumption cannot silently change it).
    #[test]
    fn a_duplicated_parameter_keeps_the_last_value_like_before() {
        let t = Transport::parse("RTP/AVP;unicast;client_port=1000-1001;client_port=2000-2001").unwrap();
        assert_eq!(t.first().unwrap().client_port, Some((2000, 2001)));
    }

    #[test]
    fn non_avp_profile_and_unknown_lower_transport_are_rejected() {
        assert!(Transport::parse("RTP/SAVP").is_err());
        assert!(Transport::parse("RTP/AVP/SCTP").is_err());
    }
```

- [ ] **Step 2: Run, expect FAIL**

```bash
cargo test --locked -p rtsp-runtime --all-features --lib transport 2>&1 | grep -E 'test result|FAILED|panicked'
```

Expected: `serialisation_order_is_the_typed_headers_order` FAILS (old order puts `destination` first and `port` before `ttl`... the old output is `RTP/AVP/UDP;multicast;destination=224.2.0.1;ttl=16;port=3456-3457`) and `layers_survive...` may pass already. Record the failing names.

- [ ] **Step 3: Implement the conversions**

First the case normaliser (review finding A2; `rtsp-types-0.1.3/src/headers/transport.rs:96-108,327-336,525-560` match `RTP`/`AVP`/`TCP`/`UDP` and parameter names exact-case, the old `parse_spec` used `eq_ignore_ascii_case`). It runs on the raw header value before the typed parse, upper-casing the transport triple of each comma-separated spec and lower-casing parameter NAMES only (never values):

```rust
fn normalise_case(value: &str) -> String {
    value
        .split(',')
        .map(|spec| {
            let mut parts = spec.split(';');
            let head = parts.next().unwrap_or("").trim();
            let head = head.split('/').map(|t| t.trim().to_ascii_uppercase()).collect::<Vec<_>>().join("/");
            let rest = parts.map(|p| match p.split_once('=') {
                Some((k, v)) => format!("{}={}", k.trim().to_ascii_lowercase(), v),
                None => p.trim().to_ascii_lowercase(),
            });
            std::iter::once(head).chain(rest).collect::<Vec<_>>().join(";")
        })
        .collect::<Vec<_>>()
        .join(",")
}
```

`Transport::parse` hands `normalise_case(value)` to the typed parser. (If a duplicated-parameter case is rejected rather than last-wins by the typed parser, check the OLD behaviour first with `git show $BASE:rtsp-runtime/src/transport.rs`: it assigned each occurrence, so last wins; make the conversion take the last by re-scanning, or record an escalation with the rtsp-types line `transport.rs:512` FIXME as evidence.)

Replace the bodies with:

```rust
use rtsp_types::headers::{
    self, RtpLowerTransport, RtpProfile, RtpTransport, RtpTransportParameters, Transport as RtpT,
    Transports,
};
use rtsp_types::{Response, StatusCode, Version};

impl Transport {
    /// Parses a full `Transport` header value (comma-separated specs).
    pub fn parse(value: &str) -> Result<Self> {
        let resp = Response::builder(Version::V1_0, StatusCode::Ok)
            .header(headers::TRANSPORT, value)
            .empty();
        let typed = resp
            .typed_header::<Transports>()
            .map_err(|_| Error::TransportParse(format!("malformed Transport header {value:?}")))?
            .ok_or_else(|| Error::TransportParse("no transport-specs".into()))?;
        if typed.is_empty() {
            return Err(Error::TransportParse("no transport-specs".into()));
        }
        let specs = typed
            .iter()
            .map(TransportSpec::from_typed)
            .collect::<Result<Vec<_>>>()?;
        Ok(Transport { specs })
    }

    /// Serializes to a `Transport` header value.
    pub fn to_header_value(&self) -> String {
        let typed: Transports = self.specs.iter().map(TransportSpec::to_typed).collect::<Vec<_>>().into();
        let resp = Response::builder(Version::V1_0, StatusCode::Ok)
            .typed_header(&typed)
            .empty();
        resp.header(&headers::TRANSPORT)
            .map(|v| v.as_str().to_string())
            .unwrap_or_default()
    }
}

impl TransportSpec {
    fn from_typed(t: &RtpT) -> Result<Self> {
        let rtp = match t {
            RtpT::Rtp(rtp) => rtp,
            // `XYZ/AVP` and any non-RTP protocol: the typed parser files it as `Other`.
            RtpT::Other(o) => {
                return Err(Error::TransportParse(format!(
                    "unsupported transport protocol {:?} (only RTP)",
                    o.spec
                )));
            }
        };
        if rtp.profile != RtpProfile::Avp {
            return Err(Error::TransportParse(format!("unsupported profile {} (only AVP)", rtp.profile)));
        }
        let lower_transport = match &rtp.lower_transport {
            None => None,
            Some(RtpLowerTransport::Tcp) => Some(LowerTransport::Tcp),
            Some(RtpLowerTransport::Udp) => Some(LowerTransport::Udp),
            Some(other) => return Err(Error::TransportParse(format!("unknown lower-transport {other}"))),
        };
        let p = &rtp.params;
        let pair8 = |v: Option<(u8, Option<u8>)>| v.map(|(lo, hi)| (lo, hi.unwrap_or(lo)));
        let pair16 = |v: Option<(u16, Option<u16>)>| v.map(|(lo, hi)| (lo, hi.unwrap_or(lo)));
        let layers = match p.others.get("layers") {
            Some(Some(v)) => Some(v.trim().parse::<u32>().map_err(|e| Error::TransportParse(format!("bad layers {v:?}: {e}")))?),
            _ => None,
        };
        let mode = p.mode.first().map(|m| m.to_string());
        Ok(TransportSpec {
            lower_transport,
            delivery: match (p.unicast, p.multicast) {
                (_, true) => Some(Delivery::Multicast),
                (true, false) => Some(Delivery::Unicast),
                _ => None,
            },
            interleaved: pair8(p.interleaved),
            client_port: pair16(p.client_port),
            server_port: pair16(p.server_port),
            port: pair16(p.port),
            ttl: p.ttl,
            layers,
            ssrc: p.ssrc.first().copied(),
            destination: p.destination.as_ref().map(|s| s.trim_matches('"').to_string()),
            source: p.source.as_ref().map(|s| s.trim_matches('"').to_string()),
            mode,
            append: p.append,
        })
    }

    fn to_typed(&self) -> RtpT {
        let mut params = RtpTransportParameters::default();
        match self.delivery {
            Some(Delivery::Unicast) => params.unicast = true,
            Some(Delivery::Multicast) => params.multicast = true,
            None => {}
        }
        params.interleaved = self.interleaved.map(|(lo, hi)| (lo, Some(hi)));
        params.client_port = self.client_port.map(|(lo, hi)| (lo, Some(hi)));
        params.server_port = self.server_port.map(|(lo, hi)| (lo, Some(hi)));
        params.port = self.port.map(|(lo, hi)| (lo, Some(hi)));
        params.ttl = self.ttl;
        params.ssrc = self.ssrc.into_iter().collect();
        params.destination = self.destination.clone();
        params.source = self.source.clone();
        params.append = self.append;
        if let Some(m) = &self.mode {
            params.mode = vec![rtsp_types::headers::transport::TransportMode::from(m.as_str())];
        }
        if let Some(l) = self.layers {
            params.others.insert("layers".into(), Some(l.to_string()));
        }
        RtpT::Rtp(RtpTransport {
            profile: RtpProfile::Avp,
            lower_transport: self.lower_transport.map(|l| match l {
                LowerTransport::Tcp => RtpLowerTransport::Tcp,
                LowerTransport::Udp => RtpLowerTransport::Udp,
            }),
            params,
        })
    }

    /// Serializes this spec to its `Transport` header textual form (no comma).
    pub fn to_header_value(&self) -> String {
        Transport::single(self.clone()).to_header_value()
    }
}
```

Delete the now-dead helpers `parse_spec`, `parse_scalar`, `parse_u8_range`, `parse_u16_range`, `split_range`, and the `PROTO_RTP`/`PROFILE_AVP` consts if unused. Before relying on the `others` serialisation, read `rtsp-types-0.1.3/src/headers/transport.rs` lines 720-790 (the tail of `insert_into`): it must write `others` as `;name=value`; if it does not, `layers_survive_via_the_others_map` fails and `layers` is written by appending `;layers=N` to the typed output inside `TransportSpec::to_header_value` instead (record which in the report). Quoted `mode`: `TransportMode::to_string` for `"RECORD"` gives `RECORD`, and the typed serialiser re-adds the quotes (`mode="RECORD"`), same as before.

- [ ] **Step 4: Run; the Debug golden must be identical, the header golden shows only order changes**

```bash
cargo test --locked -p rtsp-runtime --all-features 2>&1 | grep -E '^test result|FAILED|panicked' | sort | uniq -c
```

Expected: `transport_parsed.golden` test... `transport_parse_and_header_goldens` FAILS on `transport_header.golden` only if the order changed (it asserts parsed first, so a parsed-golden diff fails first with `golden transport_parsed.golden differs`; that must NOT happen). If the parsed golden differs, fix the conversion, not the golden. Then review the header diff:

```bash
GOLDEN_UPDATE=1 cargo test --locked -p rtsp-runtime --all-features --test golden_wire transport_parse_and_header_goldens 2>&1 | grep -E 'test result'
git diff --stat rtsp-runtime/tests/golden
git diff rtsp-runtime/tests/golden/transport_header.golden
git diff --exit-code rtsp-runtime/tests/golden/transport_parsed.golden rtsp-runtime/tests/golden/session_transcript.golden
```

Expected: the last command exits 0 (the semantic golden and the SETUP/PLAY transcript are unchanged... the transcript contains `Transport: RTP/AVP/TCP;unicast;interleaved=0-1` which has no reordered parameters, so it must be byte-identical). In `transport_header.golden` only lines with 2+ non-trivial parameters may differ; paste each changed line (old vs new) into the report; they become the CHANGELOG example list in Task 14.

- [ ] **Step 5: Revert-check**

Restore the old implementation for one run: `git checkout $BASE -- rtsp-runtime/src/transport.rs`, then `cargo test --locked -p rtsp-runtime --all-features --lib transport`. Expected: `serialisation_order_is_the_typed_headers_order` FAILS against the old code (that is the evidence the order test bites). `git checkout HEAD -- rtsp-runtime/src/transport.rs` to restore.

- [ ] **Step 6: Commit**

```bash
git add rtsp-runtime/src/transport.rs rtsp-runtime/tests/golden/transport_header.golden
git commit -m "refactor(rtsp-runtime): Transport header parse/build via rtsp_types typed Transports (parameter order changes only)"
```

---

### Task 5: RTSP client adapter on `Framed`, with timeouts (defect 4, client side)

Replaces the hand-rolled `read_buf` / `read_once` / `fill_from_socket` loop (io.rs 66-345) with `Framed<S, ClientCodec>`; every awaited IO is bounded by `RtspTimeouts`. Also bounds the sans-IO core's own header phase (the client-side quadratic reparse).

**Files:**
- Create: `rtsp-runtime/src/codec.rs` (feature `tokio`)
- Modify: `rtsp-runtime/src/io.rs` lines 41-345 (client), 62-70 (`io_err`), and the TLS client constructors 347-405
- Modify: `rtsp-runtime/src/error.rs` (add `Timeout`, `From<std::io::Error>`)
- Modify: `rtsp-runtime/src/client.rs` lines 311-354 (`handle_data`: header-phase cap)
- Modify: `rtsp-runtime/src/lib.rs` (`mod codec;`, export `RtspTimeouts`)
- Test: `rtsp-runtime/tests/io_timeouts.rs` (new), unit tests in `codec.rs` and `client.rs`

**Interfaces:**

```rust
// io.rs (feature "tokio")
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct RtspTimeouts {
    /// TCP connect. Default 10 s.
    pub connect: Duration,
    /// TLS handshake (rtsps) / first complete request on a server connection. Default 10 s.
    pub handshake: Duration,
    /// Longest wait for the next complete frame (a response, a request, an interleaved frame).
    /// It bounds the whole frame, not each byte, so a peer dripping one byte at a time still
    /// times out. Default 30 s.
    pub read_idle: Duration,
    /// Longest wait for a write to be accepted by the socket. Default 10 s.
    pub write: Duration,
}
impl Default for RtspTimeouts { /* the defaults above */ }
impl RtspTimeouts { pub fn with_connect(self, d: Duration) -> Self; pub fn with_handshake(self, d: Duration) -> Self;
                    pub fn with_read_idle(self, d: Duration) -> Self; pub fn with_write(self, d: Duration) -> Self; }

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncRtspClient<S> {
    pub fn with_stream(stream: S, session: ClientSession) -> Self;               // default timeouts
    pub fn with_stream_timeouts(stream: S, session: ClientSession, timeouts: RtspTimeouts) -> Self;
    pub fn timeouts(&self) -> RtspTimeouts;
}
impl AsyncRtspClient<TcpStream> {
    pub async fn connect<A: ToSocketAddrs>(addr: A) -> Result<Self>;            // unchanged
    pub async fn connect_with<A: ToSocketAddrs>(addr: A, session: ClientSession) -> Result<Self>; // unchanged
    pub async fn connect_with_timeouts<A: ToSocketAddrs>(addr: A, session: ClientSession, timeouts: RtspTimeouts) -> Result<Self>;
}
// error.rs
Error::Timeout { what: &'static str }   // #[error("timed out waiting for {what}")]; what in {"connect","handshake","read","write"}
impl From<std::io::Error> for Error      // -> Error::Io(e.to_string())
// codec.rs (crate-private)
pub(crate) struct ClientCodec { pub(crate) session: ClientSession, /* queue */ }
pub(crate) async fn bounded<T>(limit: Duration, what: &'static str,
    fut: impl Future<Output = Result<T>>) -> Result<T>;
```

All existing `AsyncRtspClient` methods (`options`, `describe`, `setup`, `play`, `pause`, `teardown`, `get_parameter`, `recv_interleaved`, `state`, `session_id`, `session`) keep their signatures. multimux uses only `rtsp_runtime::io::default_tls_client_config` from this module, which is untouched.

- [ ] **Step 1: Write the failing tests**

`rtsp-runtime/tests/io_timeouts.rs`:

```rust
//! Defect 4: no awaited IO in the RTSP adapters is unbounded. All time is paused
//! virtual time over in-memory `duplex` pipes (never real sockets + paused time).
#![cfg(feature = "tokio")]

use std::time::Duration;

use rtsp_runtime::{AsyncRtspClient, ClientSession, Error, RtspTimeouts};
use tokio::io::{AsyncReadExt, AsyncWriteExt, duplex};

const URI: &str = "rtsp://h/s";

fn short() -> RtspTimeouts {
    RtspTimeouts::default()
        .with_read_idle(Duration::from_secs(5))
        .with_write(Duration::from_secs(5))
}

#[tokio::test(start_paused = true)]
async fn a_server_that_never_answers_times_out_the_request() {
    let (client_io, _server_io) = duplex(4096); // peer kept alive, never replies
    let mut c = AsyncRtspClient::with_stream_timeouts(client_io, ClientSession::new(), short());
    let err = c.options(URI).await.expect_err("must time out");
    assert!(matches!(err, Error::Timeout { what: "read" }), "got {err:?}");
}

#[tokio::test(start_paused = true)]
async fn a_peer_that_stops_reading_times_out_the_write() {
    let (client_io, _server_io) = duplex(8); // 8-byte pipe, never drained
    let mut c = AsyncRtspClient::with_stream_timeouts(client_io, ClientSession::new(), short());
    let err = c.options(URI).await.expect_err("must time out");
    assert!(matches!(err, Error::Timeout { what: "write" }), "got {err:?}");
}

#[tokio::test(start_paused = true)]
async fn a_response_dripped_one_byte_at_a_time_still_times_out_as_a_whole() {
    let (client_io, mut server_io) = duplex(4096);
    // Server: swallow the request, then send a response byte every 2 s: never finishes in 5 s.
    tokio::spawn(async move {
        let mut buf = [0u8; 512];
        let _ = server_io.read(&mut buf).await;
        for b in b"RTSP/1.0 200 OK\r\nCSeq: 1\r\n\r\n" {
            tokio::time::sleep(Duration::from_secs(2)).await;
            if server_io.write_all(&[*b]).await.is_err() {
                break;
            }
        }
    });
    let mut c = AsyncRtspClient::with_stream_timeouts(client_io, ClientSession::new(), short());
    let err = c.options(URI).await.expect_err("frame deadline, not per-byte idle");
    assert!(matches!(err, Error::Timeout { what: "read" }), "got {err:?}");
}

```

The `bounded` helper's own test lives in `codec.rs`'s `#[cfg(test)]` module (no `#[doc(hidden)]` public item, review finding):

```rust
    #[tokio::test(start_paused = true)]
    async fn a_connect_that_never_completes_is_bounded() {
        let r: Result<()> = bounded(Duration::from_secs(3), "connect", std::future::pending::<Result<()>>()).await;
        assert!(matches!(r, Err(Error::Timeout { what: "connect" })));
    }
```

Add to `rtsp-runtime/src/codec.rs` (new file, tests first; the types do not exist yet):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const RESPONSE: &[u8] = b"RTSP/1.0 200 OK\r\nCSeq: 1\r\nPublic: OPTIONS, DESCRIBE\r\n\r\n";

    #[test]
    fn client_codec_reassembles_a_response_split_at_every_byte_boundary() {
        for split in 1..RESPONSE.len() {
            let mut c = ClientSession::new();
            let _ = c.options("rtsp://h/s").unwrap();
            let mut codec = ClientCodec::new(c);
            let mut buf = BytesMut::new();
            buf.extend_from_slice(&RESPONSE[..split]);
            assert!(codec.decode(&mut buf).unwrap().is_none(), "split {split}: premature event");
            buf.extend_from_slice(&RESPONSE[split..]);
            let ev = codec.decode(&mut buf).unwrap();
            assert!(matches!(ev, Some(ClientEvent::Response { cseq: 1, .. })), "split {split}: {ev:?}");
        }
    }

    #[test]
    fn client_codec_one_byte_at_a_time() {
        let mut c = ClientSession::new();
        let _ = c.options("rtsp://h/s").unwrap();
        let mut codec = ClientCodec::new(c);
        let mut buf = BytesMut::new();
        let mut got = None;
        for b in RESPONSE {
            buf.extend_from_slice(&[*b]);
            if let Some(ev) = codec.decode(&mut buf).unwrap() {
                got = Some(ev);
            }
        }
        assert!(matches!(got, Some(ClientEvent::Response { cseq: 1, .. })));
    }
}
```

and in `client.rs`'s test module:

```rust
    #[test]
    fn an_unterminated_header_over_the_head_cap_is_rejected() {
        let mut c = ClientSession::new();
        let mut junk = b"RTSP/1.0 200 OK\r\nX-Pad: ".to_vec();
        junk.extend(std::iter::repeat_n(b'a', 70 * 1024)); // no terminator, ever
        let err = c.handle_data(&junk).expect_err("header phase is capped at 64 KiB");
        assert!(matches!(err, Error::MessageParse(_)), "{err:?}");
    }
```

- [ ] **Step 2: Run, expect FAIL**

```bash
cargo test --locked -p rtsp-runtime --all-features --test io_timeouts 2>&1 | grep -E '^error|cannot find|no variant|test result' | head
```

Expected: compile errors (`RtspTimeouts` not found, `Error::Timeout` not found, `bounded`). That is the FAIL.

- [ ] **Step 3: Implement**

`error.rs` additions:

```rust
    #[error("timed out waiting for {what}")]
    Timeout {
        /// Which bounded wait expired: `"connect"`, `"handshake"`, `"read"` or `"write"`.
        what: &'static str,
    },
...
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e.to_string())
    }
}
```

`client.rs::handle_data`: after the existing `MAX_INBOUND_BYTES` check, in the message arm change

```rust
                Err(rtsp_types::ParseError::Incomplete(needed)) => {
                    // `None` = still inside the header block (no length known yet).
                    if needed.is_none() && self.inbound.len() > crate::limits::MAX_HEAD_BYTES {
                        return Err(Error::MessageParse(format!(
                            "header block over {} bytes without a terminator",
                            crate::limits::MAX_HEAD_BYTES
                        )));
                    }
                    break;
                }
```

Because `codec.rs` is feature-gated and the core is not, put the two limits in a tiny always-compiled `rtsp-runtime/src/limits.rs` (`pub(crate) const MAX_HEAD_BYTES: usize = 64 * 1024; pub(crate) const MAX_MESSAGE_BYTES: usize = 2 * 1024 * 1024;`) with `mod limits;` in `lib.rs`; the core and `codec.rs` both use `crate::limits::*`. (The core's `MAX_INBOUND_BYTES` of 1 MiB at client.rs:44 stays.)

`codec.rs` (client half; the server half is Task 6):

```rust
//! `tokio_util::codec` adapters over the sans-IO cores (W1 SP1.1). The cores keep
//! their APIs; a codec only moves bytes between a `BytesMut` and a core.

use std::collections::VecDeque;
use std::future::Future;
use std::time::Duration;

use bytes::BytesMut;
use tokio_util::codec::{Decoder, Encoder};

use crate::client::{ClientEvent, ClientSession};
use crate::error::{Error, Result};

/// Runs `fut` for at most `limit`; expiry is `Error::Timeout { what }`.
pub(crate) async fn bounded<T>(
    limit: Duration,
    what: &'static str,
    fut: impl Future<Output = Result<T>>,
) -> Result<T> {
    tokio::time::timeout(limit, fut)
        .await
        .map_err(|_| Error::Timeout { what })?
}

/// Client-side codec: bytes in -> [`ClientEvent`]s out via
/// [`ClientSession::handle_data`]; requests (already serialized by the session)
/// are written verbatim.
pub(crate) struct ClientCodec {
    pub(crate) session: ClientSession,
    queue: VecDeque<ClientEvent>,
}

impl ClientCodec {
    pub(crate) fn new(session: ClientSession) -> Self {
        Self { session, queue: VecDeque::new() }
    }
}

impl Decoder for ClientCodec {
    type Item = ClientEvent;
    type Error = Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<ClientEvent>> {
        if let Some(ev) = self.queue.pop_front() {
            return Ok(Some(ev));
        }
        if src.is_empty() {
            return Ok(None);
        }
        // The session buffers partial messages itself; hand it everything.
        let chunk = src.split();
        self.queue.extend(self.session.handle_data(&chunk)?);
        Ok(self.queue.pop_front())
    }
}

impl Encoder<Vec<u8>> for ClientCodec {
    type Error = Error;
    fn encode(&mut self, item: Vec<u8>, dst: &mut BytesMut) -> Result<()> {
        dst.extend_from_slice(&item);
        Ok(())
    }
}
```

`io.rs`: replace the client section. Imports become

```rust
use std::collections::VecDeque;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpStream, ToSocketAddrs};
use tokio_util::codec::Framed;

use crate::codec::{ClientCodec, bounded};
```

and the client:

```rust
pub struct AsyncRtspClient<S> {
    framed: Framed<S, ClientCodec>,
    /// Interleaved media surfaced while awaiting a response; capped (MAX_PENDING_MEDIA_FRAMES).
    pending_media: VecDeque<ClientEvent>,
    timeouts: RtspTimeouts,
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncRtspClient<S> {
    pub fn with_stream(stream: S, session: ClientSession) -> Self {
        Self::with_stream_timeouts(stream, session, RtspTimeouts::default())
    }
    pub fn with_stream_timeouts(stream: S, session: ClientSession, timeouts: RtspTimeouts) -> Self {
        Self { framed: Framed::new(stream, ClientCodec::new(session)), pending_media: VecDeque::new(), timeouts }
    }
    pub fn timeouts(&self) -> RtspTimeouts { self.timeouts }
    pub fn state(&self) -> crate::SessionState { self.framed.codec().session.state() }
    pub fn session_id(&self) -> Option<&str> { self.framed.codec().session.session_id() }
    pub fn session(&self) -> &ClientSession { &self.framed.codec().session }

    async fn write(&mut self, bytes: Vec<u8>) -> Result<()> {
        bounded(self.timeouts.write, "write", self.framed.send(bytes)).await
    }

    /// The next decoded event; `Ok(None)` = clean EOF. One deadline covers the WHOLE
    /// frame (see [`RtspTimeouts::read_idle`]).
    async fn next_event(&mut self) -> Result<Option<ClientEvent>> {
        match tokio::time::timeout(self.timeouts.read_idle, self.framed.next()).await {
            Err(_) => Err(Error::Timeout { what: "read" }),
            Ok(None) => Ok(None),
            Ok(Some(r)) => r.map(Some),
        }
    }

    pub async fn options(&mut self, uri: &str) -> Result<ClientEvent> {
        let cseq = self.framed.codec().session.peek_next_cseq();
        let bytes = self.framed.codec_mut().session.options(uri)?;
        self.exchange(cseq, bytes).await
    }
    // describe / setup / play / pause / teardown / get_parameter: identical shape,
    // `self.session.X(..)` becomes `self.framed.codec_mut().session.X(..)`.

    async fn exchange(&mut self, cseq: u32, request: Vec<u8>) -> Result<ClientEvent> {
        let mut cseq = cseq;
        self.write(request).await?;
        loop {
            let Some(event) = self.next_event().await? else {
                return Err(Error::Io("peer closed connection before response".into()));
            };
            match event {
                ClientEvent::Response { cseq: rcseq, .. } if rcseq == cseq => return Ok(event),
                ClientEvent::Response { .. } => {} // stray (abandoned earlier exchange)
                ClientEvent::AuthRetry { cseq: retry_cseq, ref request, .. } => {
                    let retry = request.clone();
                    self.write(retry).await?;
                    cseq = retry_cseq;
                }
                ClientEvent::MediaData { .. } => {
                    if self.pending_media.len() >= MAX_PENDING_MEDIA_FRAMES {
                        self.pending_media.pop_front();
                    }
                    self.pending_media.push_back(event);
                }
            }
        }
    }

    pub async fn recv_interleaved(&mut self) -> Result<Option<ClientEvent>> {
        loop {
            if let Some(event) = self.pending_media.pop_front() {
                return Ok(Some(event));
            }
            match self.next_event().await? {
                None => return Ok(None),
                Some(ev @ ClientEvent::MediaData { .. }) => return Ok(Some(ev)),
                Some(_) => {} // control responses are applied by the session; not surfaced here
            }
        }
    }
}
```

(The old `exchange` collected all events decoded from one read before returning, so a `Response` followed by coalesced `MediaData` in one TCP segment buffered the media. With one event per `next_event` call the codec's queue keeps the remaining events for the next call, so the media is NOT dropped: the response returns first, the media stays in `ClientCodec::queue` and comes out of the next `next_event`. `pending_media` therefore only fills with media seen BEFORE the response; the existing `pending_media_queue_is_capped_while_exchange_waits` test still holds.)

Connect constructors:

```rust
impl AsyncRtspClient<TcpStream> {
    pub async fn connect<A: ToSocketAddrs>(addr: A) -> Result<Self> {
        Self::connect_with_timeouts(addr, ClientSession::new(), RtspTimeouts::default()).await
    }
    pub async fn connect_with<A: ToSocketAddrs>(addr: A, session: ClientSession) -> Result<Self> {
        Self::connect_with_timeouts(addr, session, RtspTimeouts::default()).await
    }
    pub async fn connect_with_timeouts<A: ToSocketAddrs>(
        addr: A, session: ClientSession, timeouts: RtspTimeouts,
    ) -> Result<Self> {
        let stream = bounded(timeouts.connect, "connect", async {
            TcpStream::connect(addr).await.map_err(|e| io_err("connect", e))
        })
        .await?;
        Ok(Self::with_stream_timeouts(stream, session, timeouts))
    }
}
```

TLS: `connect_tls_with` calls `connect_tls_with_timeouts(addr, server_name, config, session, RtspTimeouts::default())`, which does `bounded(t.connect, "connect", TcpStream::connect)` then `bounded(t.handshake, "handshake", connector.connect(dns, tcp))`. Add `RtspTimeouts` (struct, `Default`, `with_*`) to `io.rs`; `pub use io::{..., RtspTimeouts}` in `lib.rs`.

- [ ] **Step 4: Run, expect PASS**

```bash
cargo test --locked -p rtsp-runtime --all-features 2>&1 | grep -E '^test result|FAILED|panicked|^error' | sort | uniq -c
```

Expected: all `ok`, including `io_timeouts` (4), the two codec tests, `an_unterminated_header_over_the_head_cap_is_rejected`, every pre-existing `io_loopback` test and the `golden_wire` goldens (unchanged: the sans-IO output is the same).

- [ ] **Step 5: Revert-check (defect 4)**

Temporarily reintroduce the old "no deadline" behaviour in `io.rs`: in `next_event` replace the `tokio::time::timeout(...)` with `Ok(self.framed.next().await)` shaped to the same return type, and in `write` replace `bounded(...)` with `self.framed.send(bytes).await`. Then:

```bash
timeout 120 cargo test --locked -p rtsp-runtime --all-features --test io_timeouts a_server_that_never a_peer_that_stops a_response_dripped 2>&1 | tail -5; echo "exit=$?"
```

Expected: the run does not finish (killed by `timeout`, exit 124 from the harness) or the three tests fail: with no deadline the awaits never return. Record this in the report ("old behaviour: call never completes on its own"). Restore both edits (`git checkout -- rtsp-runtime/src/io.rs` is safe here because Step 6 has not committed yet only if you have not staged the file; otherwise undo the two edits by hand) and re-run Step 4.

- [ ] **Step 6: Commit**

```bash
git add rtsp-runtime
git commit -m "feat(rtsp-runtime)!: client adapter on tokio_util Framed with connect/handshake/read/write timeouts; cap the header phase"
```

---

### Task 6: RTSP server adapter on `Framed`, with timeouts (defect 4, server side)

**Files:**
- Modify: `rtsp-runtime/src/codec.rs` (add `ServerCodec`, `ServerFrame`)
- Modify: `rtsp-runtime/src/io.rs` lines 407-614 (server struct, `next_request`, `send_interleaved`, helpers `has_header_end`, `complete_request_len`, `HEADER_END_LF_CRLF_LEN`, `MAX_SERVER_READ_BUFFER`)
- Test: `rtsp-runtime/tests/io_timeouts.rs` (append), `codec.rs` unit tests

**Interfaces:**

```rust
impl<S: AsyncRead + AsyncWrite + Unpin> AsyncRtspServer<S> {
    pub fn with_stream(stream: S, session: ServerSession) -> Self;                 // default timeouts
    pub fn with_stream_timeouts(stream: S, session: ServerSession, timeouts: RtspTimeouts) -> Self;
    pub fn timeouts(&self) -> RtspTimeouts;
    pub async fn next_request(&mut self) -> Result<Option<Vec<ServerEvent>>>;      // unchanged signature
    pub async fn send_interleaved(&mut self, channel: u8, payload: &[u8]) -> Result<()>;
    pub fn stream_mut(&mut self) -> &mut S;                                         // kept
}
impl AsyncRtspServer<TcpStream> {
    pub fn accept(stream: TcpStream) -> Self;                                        // unchanged
    pub fn accept_with(stream: TcpStream, session: ServerSession) -> Self;           // unchanged
    pub fn accept_with_timeouts(stream: TcpStream, session: ServerSession, timeouts: RtspTimeouts) -> Self;
}
// tls: accept_tls (unchanged signature, default timeouts) + accept_tls_with_timeouts(.., timeouts) bounding the handshake by `timeouts.handshake`.
// codec.rs (crate-private)
pub(crate) struct ServerCodec { skip_until: usize }
pub(crate) enum ServerFrame { Request(Vec<u8>), Media { channel: u8, data: Vec<u8> } }
```

`stream_mut` stays (an io_loopback test writes raw bytes through it); implement it as `self.framed.get_mut()`.

Deadline semantics: the FIRST `next_request` on a connection waits at most `timeouts.handshake` for a complete request (a connection that never sends a request); every later call waits at most `timeouts.read_idle`. A frame (request or interleaved) must complete inside that single deadline, so a client dripping bytes cannot hold the connection forever (slow-loris).

- [ ] **Step 1: Write the failing tests**

Append to `rtsp-runtime/tests/io_timeouts.rs`:

```rust
use rtsp_runtime::{AsyncRtspServer, ServerSession};

fn server_over(io: tokio::io::DuplexStream, t: RtspTimeouts) -> AsyncRtspServer<tokio::io::DuplexStream> {
    AsyncRtspServer::with_stream_timeouts(io, ServerSession::new(|| 7).with_session_seed(1), t)
}

#[tokio::test(start_paused = true)]
async fn an_idle_connection_that_never_sends_a_request_times_out_at_handshake() {
    let (_client_io, server_io) = duplex(4096);
    let mut s = server_over(server_io, RtspTimeouts::default().with_handshake(Duration::from_secs(4)));
    let err = s.next_request().await.expect_err("handshake deadline");
    assert!(matches!(err, Error::Timeout { what: "read" }), "got {err:?}");
}

#[tokio::test(start_paused = true)]
async fn slow_loris_request_dripped_a_byte_every_few_seconds_times_out() {
    let (mut client_io, server_io) = duplex(4096);
    tokio::spawn(async move {
        for b in b"OPTIONS rtsp://h/s RTSP/1.0\r\nCSeq: 1\r\n\r\n" {
            tokio::time::sleep(Duration::from_secs(3)).await; // never idle for 10 s...
            if client_io.write_all(&[*b]).await.is_err() {
                return;
            }
        }
    });
    let mut s = server_over(server_io, RtspTimeouts::default().with_handshake(Duration::from_secs(10)));
    let err = s.next_request().await.expect_err("...but the whole request must finish in 10 s");
    assert!(matches!(err, Error::Timeout { what: "read" }), "got {err:?}");
}

#[tokio::test(start_paused = true)]
async fn a_header_block_that_never_terminates_is_rejected_at_the_head_cap() {
    let (mut client_io, server_io) = duplex(256 * 1024);
    let mut s = server_over(server_io, RtspTimeouts::default());
    tokio::spawn(async move {
        let _ = client_io.write_all(b"OPTIONS rtsp://h/s RTSP/1.0\r\nX-Pad: ").await;
        let junk = vec![b'a'; 80 * 1024];
        let _ = client_io.write_all(&junk).await;
        std::future::pending::<()>().await; // keep the pipe open: the cap, not EOF, must trip
    });
    let err = s.next_request().await.expect_err("64 KiB header cap");
    assert!(matches!(err, Error::MessageParse(_)), "got {err:?}");
}

#[tokio::test(start_paused = true)]
async fn a_content_length_larger_than_the_message_cap_is_rejected_up_front() {
    let (mut client_io, server_io) = duplex(4096);
    let mut s = server_over(server_io, RtspTimeouts::default());
    tokio::spawn(async move {
        let _ = client_io
            .write_all(b"ANNOUNCE rtsp://h/s RTSP/1.0\r\nCSeq: 1\r\nContent-Length: 9999999\r\n\r\n")
            .await;
        std::future::pending::<()>().await;
    });
    let err = s.next_request().await.expect_err("body larger than the 2 MiB cap");
    assert!(matches!(err, Error::MessageParse(_)), "got {err:?}");
}
```

`codec.rs` unit tests (server half):

```rust
    #[test]
    fn server_codec_request_split_at_every_boundary_and_pipelined_frames() {
        let req: &[u8] = b"OPTIONS rtsp://h/s RTSP/1.0\r\nCSeq: 1\r\n\r\n";
        for split in 1..req.len() {
            let mut codec = ServerCodec::default();
            let mut buf = BytesMut::from(&req[..split]);
            assert!(codec.decode(&mut buf).unwrap().is_none(), "split {split}");
            buf.extend_from_slice(&req[split..]);
            assert!(matches!(codec.decode(&mut buf).unwrap(), Some(ServerFrame::Request(r)) if r == req));
        }
        // request, then an interleaved frame, back to back in one buffer
        let mut codec = ServerCodec::default();
        let mut buf = BytesMut::new();
        buf.extend_from_slice(req);
        buf.extend_from_slice(&[0x24, 3, 0, 2, 0xAA, 0xBB]);
        assert!(matches!(codec.decode(&mut buf).unwrap(), Some(ServerFrame::Request(_))));
        assert!(matches!(
            codec.decode(&mut buf).unwrap(),
            Some(ServerFrame::Media { channel: 3, ref data }) if data == &[0xAA, 0xBB]
        ));
        assert!(codec.decode(&mut buf).unwrap().is_none());
    }

    #[test]
    fn server_codec_waits_for_a_body_without_reparsing_every_byte() {
        let head = b"ANNOUNCE rtsp://h/s RTSP/1.0\r\nCSeq: 1\r\nContent-Length: 20\r\n\r\n";
        let mut codec = ServerCodec::default();
        let mut buf = BytesMut::from(&head[..]);
        assert!(codec.decode(&mut buf).unwrap().is_none());
        assert_eq!(codec.skip_until, head.len() + 20, "knows how many bytes it still needs");
        buf.extend_from_slice(&[b'x'; 20]);
        assert!(matches!(codec.decode(&mut buf).unwrap(), Some(ServerFrame::Request(r)) if r.len() == head.len() + 20));
    }
```

- [ ] **Step 2: Run, expect FAIL**

```bash
cargo test --locked -p rtsp-runtime --all-features --test io_timeouts --lib 2>&1 | grep -E '^error|cannot find|test result' | head
```

Expected: compile errors for `ServerCodec`, `with_stream_timeouts` on the server.

- [ ] **Step 3: Implement**

`codec.rs` server half:

```rust
use bytes::Buf;
use rtsp_types::Message;

use crate::interleaved::{InterleavedFrame, MAGIC};
use crate::limits::{MAX_HEAD_BYTES, MAX_MESSAGE_BYTES};

/// One server-side frame: a complete request, or an interleaved `$` block (§10.12).
pub(crate) enum ServerFrame {
    Request(Vec<u8>),
    Media { channel: u8, data: Vec<u8> },
}

/// Frames a request stream. Uses `rtsp_types::Message::parse` as the only
/// parser: it reports how many more bytes a body needs (`Incomplete(Some(n))`),
/// so a body is not re-parsed until it can possibly be complete; a header block
/// that has no known length yet is capped at [`MAX_HEAD_BYTES`]. No header
/// terminator is searched for by hand.
#[derive(Default)]
pub(crate) struct ServerCodec {
    /// Buffer length below which a re-parse cannot succeed yet.
    skip_until: usize,
}

impl Decoder for ServerCodec {
    type Item = ServerFrame;
    type Error = Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<ServerFrame>> {
        if src.is_empty() {
            return Ok(None);
        }
        if src[0] == MAGIC {
            return match InterleavedFrame::parse(src)? {
                Some((frame, used)) => {
                    src.advance(used);
                    Ok(Some(ServerFrame::Media { channel: frame.channel, data: frame.payload }))
                }
                None => Ok(None),
            };
        }
        if src.len() < self.skip_until {
            return Ok(None);
        }
        match Message::<Vec<u8>>::parse(src) {
            Ok((_, used)) => {
                self.skip_until = 0;
                Ok(Some(ServerFrame::Request(src.split_to(used).to_vec())))
            }
            Err(rtsp_types::ParseError::Incomplete(Some(more))) => {
                let want = src.len().saturating_add(more.get());
                if want > MAX_MESSAGE_BYTES {
                    return Err(Error::MessageParse(format!(
                        "request of at least {want} bytes exceeds the {MAX_MESSAGE_BYTES}-byte maximum"
                    )));
                }
                self.skip_until = want;
                Ok(None)
            }
            Err(rtsp_types::ParseError::Incomplete(None)) => {
                if src.len() > MAX_HEAD_BYTES {
                    return Err(Error::MessageParse(format!(
                        "header block over {MAX_HEAD_BYTES} bytes without a terminator"
                    )));
                }
                Ok(None)
            }
            Err(rtsp_types::ParseError::Error) => Err(Error::MessageParse("malformed RTSP request".into())),
        }
    }
}

impl Encoder<Vec<u8>> for ServerCodec {
    type Error = Error;
    fn encode(&mut self, item: Vec<u8>, dst: &mut BytesMut) -> Result<()> {
        dst.extend_from_slice(&item);
        Ok(())
    }
}
```

`limits.rs`: `pub(crate) const MAX_HEAD_BYTES: usize = 64 * 1024; pub(crate) const MAX_MESSAGE_BYTES: usize = 2 * 1024 * 1024;`.

`io.rs` server:

```rust
pub struct AsyncRtspServer<S> {
    framed: Framed<S, ServerCodec>,
    session: ServerSession,
    timeouts: RtspTimeouts,
    first_request_seen: bool,
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncRtspServer<S> {
    pub fn with_stream(stream: S, session: ServerSession) -> Self {
        Self::with_stream_timeouts(stream, session, RtspTimeouts::default())
    }
    pub fn with_stream_timeouts(stream: S, session: ServerSession, timeouts: RtspTimeouts) -> Self {
        Self { framed: Framed::new(stream, ServerCodec::default()), session, timeouts, first_request_seen: false }
    }
    pub fn timeouts(&self) -> RtspTimeouts { self.timeouts }
    pub fn state(&self) -> crate::SessionState { self.session.state() }
    pub fn session_id(&self) -> Option<&str> { self.session.session_id() }
    pub fn stream_mut(&mut self) -> &mut S { self.framed.get_mut() }

    pub async fn next_request(&mut self) -> Result<Option<Vec<ServerEvent>>> {
        let limit = if self.first_request_seen { self.timeouts.read_idle } else { self.timeouts.handshake };
        let frame = match tokio::time::timeout(limit, self.framed.next()).await {
            Err(_) => return Err(Error::Timeout { what: "read" }),
            Ok(None) => return Ok(None),
            Ok(Some(frame)) => frame?,
        };
        self.first_request_seen = true;
        match frame {
            ServerFrame::Media { channel, data } => Ok(Some(vec![ServerEvent::MediaData { channel, data }])),
            ServerFrame::Request(bytes) => {
                let (response, events) = self.session.handle_request(&bytes)?;
                bounded(self.timeouts.write, "write", self.framed.send(response)).await?;
                Ok(Some(events))
            }
        }
    }

    pub async fn send_interleaved(&mut self, channel: u8, payload: &[u8]) -> Result<()> {
        let bytes = crate::interleaved::InterleavedFrame::new(channel, payload.to_vec()).to_bytes()?;
        bounded(self.timeouts.write, "write", self.framed.send(bytes)).await
    }
}
```

`Ok(None)` on clean EOF with a partial request buffered: `FramedRead` yields `Err` via `decode_eof` ("bytes remaining on stream") which `Error::from(io::Error)` maps to `Error::Io`; the old code returned `Error::Io("peer closed connection mid-request")`: same variant, different text.

`accept_with` -> `accept_with_timeouts(stream, session, RtspTimeouts::default())`. TLS: `accept_tls_with_timeouts` wraps `acceptor.accept(stream)` in `bounded(timeouts.handshake, "handshake", ...)`. Delete `has_header_end`, `complete_request_len`, `HEADER_END_LF_CRLF_LEN`, `MAX_SERVER_READ_BUFFER`, `READ_CHUNK` and the `Message`/`MAGIC` imports from `io.rs`.

- [ ] **Step 4: Run, expect PASS**

```bash
cargo test --locked -p rtsp-runtime --all-features 2>&1 | grep -E '^test result|FAILED|panicked|^error' | sort | uniq -c
```

Expected: all ok. The pre-existing `io_loopback::...oversized_request_buffer...` test (it streams 2 MiB+ of header junk and expects the server to reject) still passes, now at the 64 KiB head cap.

- [ ] **Step 5: Revert-check**

Temporarily set `limit` in `next_request` to `Duration::from_secs(u32::MAX as u64)` (no effective deadline) and `MAX_HEAD_BYTES` in `limits.rs` to `usize::MAX` (no head cap). Run:

```bash
timeout 120 cargo test --locked -p rtsp-runtime --all-features --test io_timeouts 2>&1 | grep -E 'test result|FAILED|slow_loris|never_sends|never_terminates'; echo "exit=$?"
```

Expected: `slow_loris_request_dripped_a_byte_every_few_seconds_times_out`, `an_idle_connection_that_never_sends_a_request_times_out_at_handshake` fail or hang until `timeout` kills the run, and `a_header_block_that_never_terminates_is_rejected_at_the_head_cap` fails (it waits for ever instead of erroring at 64 KiB). Record the names. Restore both constants by hand and re-run Step 4.

- [ ] **Step 6: Commit**

```bash
git add rtsp-runtime
git commit -m "feat(rtsp-runtime)!: server adapter on tokio_util Framed; per-frame deadline defeats slow-loris; header and body caps"
```

---

### Task 7: RTSP keepalive deadline (`poll_timeout` / `handle_timeout`)

**Files:**
- Modify: `rtsp-runtime/src/client.rs` (struct fields ~93-105, `new` ~117-130, `build_request_with_body` ~236-266, new methods)
- Modify: `rtsp-runtime/src/io.rs` (`recv_interleaved`, `exchange`: call `mark_activity`)
- Test: `rtsp-runtime/src/client.rs` unit tests; `rtsp-runtime/tests/io_timeouts.rs` (append)

**Interfaces:**

```rust
impl ClientSession {
    /// Record that a request was written or a response read at `now`.
    pub fn mark_activity(&mut self, now: std::time::Instant);
    /// Next keepalive deadline: `last_activity + timeout/2` once a Session id exists
    /// (timeout = the SETUP response's `timeout=`, else the RFC 2326 §12.37 default of 60 s).
    /// `None` before SETUP, after TEARDOWN, or if no activity was ever recorded.
    pub fn poll_timeout(&self) -> Option<std::time::Instant>;
    /// If `now >= poll_timeout()`, the bytes of a `GET_PARAMETER` keepalive on the last
    /// request URI (and activity is re-armed at `now`); `Ok(None)` otherwise.
    pub fn handle_timeout(&mut self, now: std::time::Instant) -> Result<Option<Vec<u8>>>;
}
```

Spec says `poll_timeout(&self) -> Option<Instant>`: exactly that signature. The core never reads a clock; `now` is always caller-supplied.

- [ ] **Step 1: Write the failing tests**

In the `client.rs` test module:

```rust
    use std::time::{Duration, Instant};

    fn established(timeout: Option<u64>) -> ClientSession {
        let mut c = ClientSession::new();
        let _ = c.setup("rtsp://h/s", &Transport::single(TransportSpec::rtp_avp_tcp_interleaved(0, 1))).unwrap();
        let sess = match timeout { Some(t) => format!("abc;timeout={t}"), None => "abc".into() };
        let resp = format!(
            "RTSP/1.0 200 OK\r\nCSeq: 1\r\nSession: {sess}\r\nTransport: RTP/AVP/TCP;unicast;interleaved=0-1\r\n\r\n"
        );
        c.handle_data(resp.as_bytes()).unwrap();
        c
    }

    #[test]
    fn keepalive_is_due_at_half_the_session_timeout() {
        let t0 = Instant::now();
        let mut c = established(Some(20));
        assert_eq!(c.poll_timeout(), None, "no activity recorded yet");
        c.mark_activity(t0);
        assert_eq!(c.poll_timeout(), Some(t0 + Duration::from_secs(10)));
        assert!(c.handle_timeout(t0 + Duration::from_secs(9)).unwrap().is_none());
        let req = c.handle_timeout(t0 + Duration::from_secs(10)).unwrap().expect("keepalive due");
        let text = String::from_utf8(req).unwrap();
        assert!(text.starts_with("GET_PARAMETER rtsp://h/s RTSP/1.0"), "{text}");
        assert!(text.contains("Session: abc"), "{text}");
        // re-armed from the moment it was sent
        assert_eq!(c.poll_timeout(), Some(t0 + Duration::from_secs(20)));
    }

    #[test]
    fn keepalive_uses_the_rfc_default_timeout_when_none_was_declared() {
        let t0 = Instant::now();
        let mut c = established(None);
        c.mark_activity(t0);
        assert_eq!(c.poll_timeout(), Some(t0 + Duration::from_secs(30))); // 60 s / 2
    }

    #[test]
    fn no_keepalive_before_setup_or_after_teardown() {
        let t0 = Instant::now();
        let mut c = ClientSession::new();
        c.mark_activity(t0);
        assert_eq!(c.poll_timeout(), None);
        assert!(c.handle_timeout(t0 + Duration::from_secs(3600)).unwrap().is_none());
    }
```

`rtsp-runtime/tests/io_timeouts.rs` (adapter, paused time, in-memory server using the real `AsyncRtspServer`):

```rust
#[tokio::test(start_paused = true)]
async fn recv_interleaved_sends_a_keepalive_before_the_session_expires() {
    use rtsp_runtime::{Transport, TransportSpec, ClientEvent};
    let (client_io, server_io) = duplex(16 * 1024);
    let (seen_tx, mut seen_rx) = tokio::sync::mpsc::unbounded_channel::<tokio::time::Instant>();
    let server = tokio::spawn(async move {
        let mut s = AsyncRtspServer::with_stream_timeouts(
            server_io,
            ServerSession::new(|| 9).with_session_seed(9).with_session_timeout(20),
            RtspTimeouts::default().with_read_idle(Duration::from_secs(120)),
        );
        let mut n = 0;
        while let Ok(Some(events)) = s.next_request().await {
            n += 1;
            if n == 3 {
                // SETUP, PLAY, then the keepalive GET_PARAMETER arrived.
                seen_tx.send(tokio::time::Instant::now()).unwrap();
                s.send_interleaved(0, b"frame").await.unwrap();
            }
            let _ = events;
        }
    });
    let t0 = tokio::time::Instant::now();
    let mut c = AsyncRtspClient::with_stream_timeouts(
        client_io,
        ClientSession::new(),
        RtspTimeouts::default().with_read_idle(Duration::from_secs(120)),
    );
    c.setup(URI, &Transport::single(TransportSpec::rtp_avp_tcp_interleaved(0, 1))).await.unwrap();
    c.play(URI).await.unwrap();
    let ev = c.recv_interleaved().await.unwrap().expect("media after keepalive");
    assert!(matches!(ev, ClientEvent::MediaData { channel: 0, .. }));
    let seen = seen_rx.recv().await.unwrap();
    let after = seen - t0;
    assert!(after >= Duration::from_secs(10) && after < Duration::from_secs(11), "keepalive at {after:?}");
    drop(c);
    let _ = server.await;
}
```

- [ ] **Step 2: Run, expect FAIL**

```bash
cargo test --locked -p rtsp-runtime --all-features keepalive recv_interleaved_sends 2>&1 | grep -E '^error|no method|test result' | head
```

Expected: `no method named mark_activity` (compile error).

- [ ] **Step 3: Implement**

`ClientSession` gets two fields `last_activity: Option<Instant>` and `last_uri: Option<String>` (both `None` in `new`), and:

```rust
const DEFAULT_SESSION_TIMEOUT_SECS: u64 = 60; // RFC 2326 §12.37
const KEEPALIVE_FRACTION_DEN: u32 = 2;

pub fn mark_activity(&mut self, now: Instant) {
    self.last_activity = Some(now);
}

pub fn poll_timeout(&self) -> Option<Instant> {
    self.session_id.as_ref()?;
    let timeout = Duration::from_secs(self.session_timeout.unwrap_or(DEFAULT_SESSION_TIMEOUT_SECS));
    Some(self.last_activity? + timeout / KEEPALIVE_FRACTION_DEN)
}

pub fn handle_timeout(&mut self, now: Instant) -> Result<Option<Vec<u8>>> {
    match self.poll_timeout() {
        Some(deadline) if now >= deadline => {}
        _ => return Ok(None),
    }
    let Some(uri) = self.last_uri.clone() else { return Ok(None) };
    let bytes = self.get_parameter(&uri, b"")?;
    self.last_activity = Some(now);
    Ok(Some(bytes))
}
```

`build_request_with_body` sets `self.last_uri = Some(uri.to_string())` after a successful build. TEARDOWN already clears `session_id`/`session_timeout` (client.rs ~410), which makes `poll_timeout` `None`.

Adapter (`io.rs`): `fn now() -> std::time::Instant { tokio::time::Instant::now().into_std() }` (virtual-time aware). `write` calls `self.framed.codec_mut().session.mark_activity(now())` after a successful send; `next_event` marks activity on every decoded event. `recv_interleaved` becomes:

```rust
    pub async fn recv_interleaved(&mut self) -> Result<Option<ClientEvent>> {
        enum Woke { Frame(Result<Option<ClientEvent>>), Keepalive }
        loop {
            if let Some(event) = self.pending_media.pop_front() {
                return Ok(Some(event));
            }
            let deadline = self.framed.codec().session.poll_timeout().map(tokio::time::Instant::from_std);
            let woke = {
                let next = self.next_event();
                tokio::pin!(next);
                match deadline {
                    Some(d) => tokio::select! {
                        r = &mut next => Woke::Frame(r),
                        _ = tokio::time::sleep_until(d) => Woke::Keepalive,
                    },
                    None => Woke::Frame(next.await),
                }
            };
            match woke {
                Woke::Frame(r) => match r? {
                    None => return Ok(None),
                    Some(ev @ ClientEvent::MediaData { .. }) => return Ok(Some(ev)),
                    Some(_) => {}
                },
                Woke::Keepalive => {
                    if let Some(req) = self.framed.codec_mut().session.handle_timeout(now())? {
                        self.write(req).await?;
                    }
                }
            }
        }
    }
```

`next_event` is cancel-safe (`Framed::next` is), so dropping it on the keepalive arm loses nothing.

- [ ] **Step 4: Run, expect PASS**

```bash
cargo test --locked -p rtsp-runtime --all-features 2>&1 | grep -E '^test result|FAILED|panicked|^error' | sort | uniq -c
```

Expected: all ok.

- [ ] **Step 5: Revert-check**

Temporarily make `poll_timeout` return `None` unconditionally (the old "no keepalive scheduling" behaviour) and run:

```bash
timeout 120 cargo test --locked -p rtsp-runtime --all-features keepalive recv_interleaved_sends 2>&1 | grep -E 'test result|FAILED|keepalive'
```

Expected: `keepalive_is_due_at_half_the_session_timeout`, `keepalive_uses_the_rfc_default_timeout_when_none_was_declared` FAIL, and `recv_interleaved_sends_a_keepalive_before_the_session_expires` fails at its 120 s read-idle limit. Record the names, undo the edit by hand, re-run Step 4.

- [ ] **Step 6: Commit**

```bash
git add rtsp-runtime
git commit -m "feat(rtsp-runtime): ClientSession::poll_timeout/handle_timeout keepalive deadline, driven by the adapter's select"
```

---

### Task 8: RTMP server adapter on `Framed`, with timeouts; delete `pending_write` (defect 4, rtmp side)

**Files:**
- Create: `rtmp-runtime/src/codec.rs` (feature `tokio`)
- Modify: `rtmp-runtime/src/io.rs` lines 1-217 (everything above `mod tests`); keep and adapt the two tests at 219-330
- Modify: `rtmp-runtime/src/lib.rs` (`#[cfg(feature = "tokio")] mod codec;`, doc text at lines 18-27 and 57-61 that says "no equivalent client adapter")
- Test: `rtmp-runtime/tests/io_timeouts.rs` (new)

**Interfaces:**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct RtmpTimeouts {
    /// DNS + TCP connect (client). Default 10 s.
    pub connect: Duration,
    /// From connection start until the `connect` command is accepted (`ServerEvent::Connected` /
    /// `ClientEvent::Connected`). Default 10 s.
    pub handshake: Duration,
    /// Longest wait for the next batch of events (one deadline per wait, not per byte). Default 30 s.
    pub read_idle: Duration,
    /// Longest wait for the socket to accept pending writes. Default 10 s.
    pub write: Duration,
}
impl Default for RtmpTimeouts { .. }
impl RtmpTimeouts { pub fn with_connect(self, d: Duration) -> Self; with_handshake; with_read_idle; with_write }

impl AsyncRtmpServer {                       // existing bind/accept/local_addr unchanged
    pub fn with_timeouts(self, timeouts: RtmpTimeouts) -> Self;
}
pub struct RtmpConnection<S = tokio::net::TcpStream> { /* Framed<S, ServerCodec>, ready, closed, handshake_deadline, timeouts */ }
impl RtmpConnection<TcpStream> { pub fn peer_addr(&self) -> io::Result<SocketAddr>; }
impl<S: AsyncRead + AsyncWrite + Unpin> RtmpConnection<S> {
    pub fn from_stream(stream: S, session: ServerSession, timeouts: RtmpTimeouts) -> Self;
    /// Same contract as before: `Ok(None)` once finished; an `RtmpError` is mapped to `InvalidData`
    /// and closes the connection. New: a deadline expiry is `io::ErrorKind::TimedOut` and closes it.
    /// Cancel-safe: dropping the future at any await point loses neither a reply byte nor an event.
    pub async fn next_events(&mut self) -> io::Result<Option<Vec<ServerEvent>>>;
}
```

`RtmpConnection` stays nameable as a bare type (the default type parameter keeps `multimux/src/source/rtmp.rs`'s `RtmpConnection` imports compiling); `next_events`, `peer_addr`, `AsyncRtmpServer::{bind,accept,local_addr}` keep their signatures, so no multimux edit.

- [ ] **Step 1: Write the failing tests**

`rtmp-runtime/tests/io_timeouts.rs`:

```rust
//! Defect 4 (rtmp): no awaited IO is unbounded; cancel-safety of `next_events`
//! (the reason `pending_write` existed). Paused virtual time over `duplex` pipes.
#![cfg(feature = "tokio")]

use std::io::ErrorKind;
use std::time::Duration;

use rtmp_runtime::client::{ClientConfig, ClientSession};
use rtmp_runtime::io::{RtmpConnection, RtmpTimeouts};
use rtmp_runtime::server::ServerSession;
use tokio::io::{AsyncReadExt, AsyncWriteExt, duplex};

fn conn(io: tokio::io::DuplexStream, t: RtmpTimeouts) -> RtmpConnection<tokio::io::DuplexStream> {
    RtmpConnection::from_stream(io, ServerSession::with_defaults(), t)
}

#[tokio::test(start_paused = true)]
async fn an_idle_peer_times_out_at_read_idle() {
    let (_client, server) = duplex(4096);
    let mut c = conn(server, RtmpTimeouts::default().with_handshake(Duration::from_secs(300)).with_read_idle(Duration::from_secs(5)));
    let t0 = tokio::time::Instant::now();
    let err = c.next_events().await.expect_err("must time out");
    assert_eq!(err.kind(), ErrorKind::TimedOut, "{err}");
    assert_eq!(t0.elapsed(), Duration::from_secs(5));
    assert!(c.next_events().await.unwrap().is_none(), "connection is closed after a timeout");
}

#[tokio::test(start_paused = true)]
async fn a_peer_that_never_completes_connect_times_out_at_handshake() {
    let (mut client, server) = duplex(4096);
    // Client sends C0+C1 and then goes quiet: handshake bytes arrive, `connect` never does.
    let mut cs = ClientSession::new(ClientConfig::default());
    client.write_all(&cs.start()).await.unwrap();
    let mut c = conn(server, RtmpTimeouts::default().with_handshake(Duration::from_secs(3)).with_read_idle(Duration::from_secs(60)));
    let t0 = tokio::time::Instant::now();
    let first = c.next_events().await.unwrap().expect("handshake batch");
    assert!(first.is_empty());
    let err = c.next_events().await.expect_err("handshake deadline");
    assert_eq!(err.kind(), ErrorKind::TimedOut, "{err}");
    assert_eq!(t0.elapsed(), Duration::from_secs(3));
    drop(client);
}

#[tokio::test(start_paused = true)]
async fn a_peer_that_stops_reading_times_out_the_write() {
    let (mut client, server) = duplex(2048); // reply (3073 B) cannot fit
    let mut cs = ClientSession::new(ClientConfig::default());
    client.write_all(&cs.start()).await.unwrap(); // 1537 B fits
    let mut c = conn(server, RtmpTimeouts::default().with_write(Duration::from_secs(4)));
    let err = c.next_events().await.expect_err("write deadline");
    assert_eq!(err.kind(), ErrorKind::TimedOut, "{err}");
}

/// The reason `pending_write` used to exist, now covered by `Framed`'s own buffer:
/// cancel `next_events` while its reply is only partly written, then call it again.
/// The peer must receive the reply exactly once, byte-identical to an uncancelled run.
#[tokio::test]
async fn a_cancelled_next_events_neither_loses_nor_duplicates_the_reply() {
    let c0c1 = ClientSession::new(ClientConfig::default()).start();
    let expected = {
        let mut s = ServerSession::with_defaults();
        s.handle_data(&c0c1).unwrap().0
    };
    assert_eq!(expected.len(), 3073);

    let (mut client, server) = duplex(2048);
    client.write_all(&c0c1).await.unwrap();
    let mut c = conn(server, RtmpTimeouts::default());

    // First call: decodes C0+C1, starts writing the 3073-byte reply into a 2048-byte pipe, blocks.
    // `biased` + an always-ready arm cancels the call right after its first suspension.
    tokio::select! {
        biased;
        r = c.next_events() => panic!("must not finish: {r:?}"),
        _ = std::future::ready(()) => {}
    }

    // Drain the client side concurrently while the second call finishes the flush.
    let reader = tokio::spawn(async move {
        let mut got = vec![0u8; 3073];
        client.read_exact(&mut got).await.unwrap();
        // The server side is dropped below, so EOF follows: anything else that arrives is a duplicate.
        let mut rest = Vec::new();
        client.read_to_end(&mut rest).await.unwrap();
        (got, rest)
    });
    let events = c.next_events().await.unwrap().expect("the events of the cancelled call are not lost");
    assert!(events.is_empty());
    drop(c); // closes the server side: the reader sees EOF right after the reply
    let (got, rest) = reader.await.unwrap();
    assert_eq!(got, expected, "reply must be byte-identical and complete");
    assert!(rest.is_empty(), "reply must not be duplicated: {} extra bytes", rest.len());
}
```

(No wall-clock wait anywhere: the negative assertion is "EOF arrives with nothing after the 3073 bytes".)

- [ ] **Step 2: Run, expect FAIL**

```bash
cargo test --locked -p rtmp-runtime --all-features --test io_timeouts 2>&1 | grep -E '^error|cannot find|test result' | head
```

Expected: compile errors (`RtmpTimeouts`, `RtmpConnection::from_stream`).

- [ ] **Step 3: Implement**

`codec.rs`:

```rust
//! `tokio_util::codec` adapters over the sans-IO RTMP cores (W1 SP1.1).

use std::io;

use bytes::BytesMut;
use tokio_util::codec::{Decoder, Encoder};

use crate::server::{ServerEvent, ServerSession};

pub(crate) fn io_err(e: crate::RtmpError) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e)
}

/// Feeds every inbound chunk to the session. The session's reply bytes are NOT
/// returned as items: they are parked in `reply` and moved into the framed
/// write buffer by the adapter in the same synchronous step that receives the
/// events, so there is no await point at which a reply can be dropped.
pub(crate) struct ServerCodec {
    session: ServerSession,
    reply: Vec<u8>,
}

impl ServerCodec {
    pub(crate) fn new(session: ServerSession) -> Self {
        Self { session, reply: Vec::new() }
    }
    pub(crate) fn take_reply(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.reply)
    }
}

impl Decoder for ServerCodec {
    type Item = Vec<ServerEvent>;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> io::Result<Option<Vec<ServerEvent>>> {
        if src.is_empty() {
            return Ok(None);
        }
        let chunk = src.split();
        let (reply, events) = self.session.handle_data(&chunk).map_err(io_err)?;
        self.reply.extend_from_slice(&reply);
        Ok(Some(events))
    }
}

impl Encoder<Vec<u8>> for ServerCodec {
    type Error = io::Error;
    fn encode(&mut self, item: Vec<u8>, dst: &mut BytesMut) -> io::Result<()> {
        dst.extend_from_slice(&item);
        Ok(())
    }
}
```

`io.rs` (replace lines 1-217):

```rust
use std::io;
use std::net::SocketAddr;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream, ToSocketAddrs};
use tokio::time::Instant;
use tokio_util::codec::Framed;

use crate::codec::{ServerCodec, io_err};
use crate::server::{ServerConfig, ServerEvent, ServerSession};

// RtmpTimeouts struct + Default (10 s / 10 s / 30 s / 10 s) + with_* builders, as in the Interfaces block.

fn timed_out(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, format!("RTMP {what} timed out"))
}

#[derive(Debug)]
pub struct AsyncRtmpServer { listener: TcpListener, config: ServerConfig, timeouts: RtmpTimeouts }

impl AsyncRtmpServer {
    pub async fn bind<A: ToSocketAddrs>(addr: A, config: ServerConfig) -> io::Result<Self> {
        Ok(Self { listener: TcpListener::bind(addr).await?, config, timeouts: RtmpTimeouts::default() })
    }
    pub fn with_timeouts(mut self, timeouts: RtmpTimeouts) -> Self { self.timeouts = timeouts; self }
    pub async fn accept(&self) -> io::Result<RtmpConnection> {
        let (stream, _peer) = self.listener.accept().await?;
        Ok(RtmpConnection::from_stream(stream, ServerSession::new(self.config.clone()), self.timeouts))
    }
    pub fn local_addr(&self) -> io::Result<SocketAddr> { self.listener.local_addr() }
}

pub struct RtmpConnection<S = TcpStream> {
    framed: Framed<S, ServerCodec>,
    /// Events decoded but not yet handed to the caller. Set synchronously together with the
    /// reply bytes, so a cancelled flush leaves both in place for the next call.
    ready: Option<Vec<ServerEvent>>,
    closed: bool,
    handshake_deadline: Option<Instant>,
    timeouts: RtmpTimeouts,
}

impl RtmpConnection<TcpStream> {
    pub fn peer_addr(&self) -> io::Result<SocketAddr> { self.framed.get_ref().peer_addr() }
}

impl<S: AsyncRead + AsyncWrite + Unpin> RtmpConnection<S> {
    pub fn from_stream(stream: S, session: ServerSession, timeouts: RtmpTimeouts) -> Self {
        Self {
            framed: Framed::new(stream, ServerCodec::new(session)),
            ready: None,
            closed: false,
            handshake_deadline: Some(Instant::now() + timeouts.handshake),
            timeouts,
        }
    }

    pub async fn next_events(&mut self) -> io::Result<Option<Vec<ServerEvent>>> {
        if self.closed {
            return Ok(None);
        }
        loop {
            // 1. Drain whatever a cancelled earlier call left in the write buffer.
            match tokio::time::timeout(self.timeouts.write, self.framed.flush()).await {
                Err(_) => { self.closed = true; return Err(timed_out("write")); }
                Ok(Err(e)) => { self.closed = true; return Err(e); }
                Ok(Ok(())) => {}
            }
            // 2. Hand out events whose reply is now fully written.
            if let Some(events) = self.ready.take() {
                if events.iter().any(|e| matches!(e, ServerEvent::Connected { .. })) {
                    self.handshake_deadline = None;
                }
                if events.iter().any(|e| matches!(e, ServerEvent::Eof)) {
                    self.closed = true;
                }
                return Ok(Some(events));
            }
            // 3. Wait for the next batch under ONE deadline (handshake or read-idle, whichever is nearer).
            let idle = Instant::now() + self.timeouts.read_idle;
            let (deadline, what) = match self.handshake_deadline {
                Some(h) if h < idle => (h, "handshake"),
                _ => (idle, "read"),
            };
            match tokio::time::timeout_at(deadline, self.framed.next()).await {
                Err(_) => { self.closed = true; return Err(timed_out(what)); }
                Ok(None) => { self.closed = true; return Ok(None); }
                Ok(Some(Err(e))) => { self.closed = true; return Err(e); }
                Ok(Some(Ok(events))) => {
                    // Synchronous from here to the next await: reply and events move together.
                    let reply = self.framed.codec_mut().take_reply();
                    self.framed.write_buffer_mut().extend_from_slice(&reply);
                    self.ready = Some(events);
                }
            }
        }
    }
}
```

`Framed::get_ref` exists (`tokio-util-0.7.19/src/codec/framed.rs`; if the compiler objects, use `get_mut`-free `framed.get_ref()`; it is at the same `impl` block as `get_mut` line 184). Delete `READ_CHUNK`, `flush_pending`, the `pending_write` field and its test `pending_write_survives_and_is_flushed_on_next_call` (superseded by `a_cancelled_next_events_neither_loses_nor_duplicates_the_reply`); the loopback replay test at io.rs:219-290 stays but its `RtmpConnection::new(stream, session)` call in the deleted test becomes `from_stream`. Update `lib.rs` module docs (the "no equivalent client adapter" lines are corrected in Task 9).

- [ ] **Step 4: Run, expect PASS**

```bash
cargo test --locked -p rtmp-runtime --all-features 2>&1 | grep -E '^test result|FAILED|panicked|^error' | sort | uniq -c
cargo build --locked -p multimux --all-features 2>&1 | tail -2
```

Expected: all ok (including `ingest_fixture`, the loopback replay, the golden); `multimux` builds without edits.

- [ ] **Step 5: Revert-check**

Temporarily make step 1 skip the flush (delete the `self.framed.flush()` match) and move `self.ready` hand-out before it; the cancellation test then loses the reply tail: run `cargo test --locked -p rtmp-runtime --all-features --test io_timeouts a_cancelled` and expect FAIL (`reply must be byte-identical and complete` or the reader blocks, `timeout 60`). Separately set `read_idle` use to `Duration::MAX / 4` and expect `an_idle_peer_times_out_at_read_idle` to fail/hang. Record, restore by hand, re-run Step 4.

- [ ] **Step 6: Commit**

```bash
git add rtmp-runtime
git commit -m "feat(rtmp-runtime)!: server adapter on tokio_util Framed; connect/handshake/read/write timeouts; pending_write removed"
```

---

### Task 9: New RTMP client adapter with `url`-built tcUrl (SP6.1, SP3, defect 8)

**Files:**
- Create: `rtmp-runtime/src/target.rs` (feature `tokio`): `RtmpTarget`, `RtmpUrlError`
- Modify: `rtmp-runtime/src/codec.rs` (add `ClientCodec`)
- Modify: `rtmp-runtime/src/io.rs` (add `AsyncRtmpClient`)
- Modify: `rtmp-runtime/src/lib.rs` (`pub mod target;`, module docs lines 18-27 / 57-61, new example in docs)
- Create: `rtmp-runtime/examples/publish_url.rs`? Not needed: the existing `examples/capture_publish.rs` stays; the crate rule "examples per crate" is already met.
- Test: `rtmp-runtime/tests/client_adapter.rs` (new), unit tests in `target.rs`

**Interfaces:**

```rust
// target.rs (feature "tokio")
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RtmpTarget {
    pub host: url::Host<String>,   // Domain / Ipv4 / Ipv6, never a pre-formatted string
    pub port: u16,                 // 1935 when the URL has none
    pub app: String,
    pub stream_key: String,        // remaining path + `?query`, raw (not percent-decoded)
    pub tc_url: String,            // `rtmp://host[:port]/app`, IPv6 bracketed, userinfo/query/fragment removed
}
#[derive(Debug, thiserror::Error)] #[non_exhaustive]
pub enum RtmpUrlError { #[error("invalid URL: {0}")] Parse(String), #[error("unsupported scheme {0:?} (only rtmp)")] Scheme(String),
                        #[error("URL has no host")] NoHost, #[error("URL has no app segment")] NoApp, #[error("URL has no stream key")] NoStreamKey }
impl RtmpTarget {
    /// `rtmp://host[:port]/app/stream-key[?query]`
    pub fn parse(url: &str) -> Result<Self, RtmpUrlError>;
    /// Host and port from `base` (any path/query is ignored), `app` and `stream_key` given separately.
    pub fn from_parts(base: &str, app: &str, stream_key: &str) -> Result<Self, RtmpUrlError>;
    /// Resolved socket addresses (DNS only for `Host::Domain`).
    pub async fn resolve(&self) -> std::io::Result<Vec<std::net::SocketAddr>>;
}

// codec.rs
pub(crate) struct ClientCodec { session: ClientSession, reply: Vec<u8> }   // Item = Vec<ClientEvent>

// io.rs
pub struct AsyncRtmpClient<S = TcpStream> { framed: Framed<S, ClientCodec>, ready, closed, timeouts, handshake_deadline }
impl AsyncRtmpClient<TcpStream> {
    /// DNS + TCP connect bounded by `timeouts.connect`; then `publish` is NOT started.
    pub async fn connect(target: &RtmpTarget, timeouts: RtmpTimeouts) -> io::Result<Self>;
}
impl<S: AsyncRead + AsyncWrite + Unpin> AsyncRtmpClient<S> {
    pub fn from_stream(stream: S, target: &RtmpTarget, timeouts: RtmpTimeouts) -> Self;
    /// Runs handshake -> connect -> createStream -> publish; returns once `ClientEvent::Publishing`.
    /// Bounded by `timeouts.handshake` as a whole. `ClientEvent::Error` -> `io::ErrorKind::ConnectionRefused`.
    pub async fn publish(&mut self) -> io::Result<()>;
    pub async fn send_audio(&mut self, timestamp: u32, data: &[u8]) -> io::Result<()>;
    pub async fn send_video(&mut self, timestamp: u32, data: &[u8]) -> io::Result<()>;
    pub async fn send_metadata(&mut self, metadata: &[(String, Amf0Value)]) -> io::Result<()>;
    /// Next batch of server events (acks, errors, close); `Ok(None)` at EOF. Same deadline rules as the server.
    pub async fn next_events(&mut self) -> io::Result<Option<Vec<ClientEvent>>>;
}
```

`send_*` queue the message into the framed write buffer synchronously and then flush under `timeouts.write`; a cancel during the flush leaves the message queued (it is sent by the next call), never half-framed.

This is the adapter `multimux/src/push/rtmp.rs` migrates to in W2 (SP6.1). Nothing in multimux changes now.

- [ ] **Step 1: Write the failing tests**

`target.rs` unit tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_host_default_port() {
        let t = RtmpTarget::parse("rtmp://example.test/live/key").unwrap();
        assert_eq!((t.port, t.app.as_str(), t.stream_key.as_str()), (1935, "live", "key"));
        assert_eq!(t.tc_url, "rtmp://example.test/live");
    }

    /// Defect 8: an IPv6 host must stay bracketed in tcUrl; building it from a bare
    /// `IpAddr` + port (`format!("rtmp://{ip}:{port}/{app}")`) yields `rtmp://::1:1935/live`.
    #[test]
    fn ipv6_host_is_bracketed_in_tc_url() {
        let t = RtmpTarget::parse("rtmp://[::1]:1935/live/key").unwrap();
        assert_eq!(t.host, url::Host::Ipv6("::1".parse().unwrap()));
        assert_eq!(t.tc_url, "rtmp://[::1]:1935/live");
        let t = RtmpTarget::parse("rtmp://[2001:db8::7]/live/key").unwrap();
        assert_eq!(t.tc_url, "rtmp://[2001:db8::7]/live");
        assert_eq!(t.port, 1935);
    }

    #[test]
    fn userinfo_query_and_fragment_never_reach_tc_url() {
        let t = RtmpTarget::parse("rtmp://user:s3cret@host:1936/live/key?token=a/b#frag").unwrap();
        assert_eq!(t.tc_url, "rtmp://host:1936/live");
        assert_eq!(t.stream_key, "key?token=a/b");
        assert!(!t.tc_url.contains("s3cret"));
    }

    #[test]
    fn stream_key_keeps_extra_path_segments_and_trailing_slash_is_tolerated() {
        assert_eq!(RtmpTarget::parse("rtmp://h/live/a/b/c").unwrap().stream_key, "a/b/c");
        assert_eq!(RtmpTarget::parse("rtmp://h/live/key/").unwrap().stream_key, "key/");
    }

    #[test]
    fn missing_pieces_and_wrong_scheme_are_errors() {
        assert!(matches!(RtmpTarget::parse("rtmp://h/"), Err(RtmpUrlError::NoApp)));
        assert!(matches!(RtmpTarget::parse("rtmp://h/live"), Err(RtmpUrlError::NoStreamKey)));
        assert!(matches!(RtmpTarget::parse("http://h/live/k"), Err(RtmpUrlError::Scheme(_))));
        assert!(matches!(RtmpTarget::parse("not a url"), Err(RtmpUrlError::Parse(_))));
    }

    #[test]
    fn from_parts_takes_host_and_port_from_base_only() {
        let t = RtmpTarget::from_parts("rtmp://[::1]:1940/ignored/path", "live", "k").unwrap();
        assert_eq!(t.tc_url, "rtmp://[::1]:1940/live");
        assert_eq!((t.app.as_str(), t.stream_key.as_str(), t.port), ("live", "k", 1940));
    }

    #[tokio::test]
    async fn resolve_uses_the_literal_address_without_dns() {
        let t = RtmpTarget::parse("rtmp://[::1]:1935/live/key").unwrap();
        assert_eq!(t.resolve().await.unwrap(), vec!["[::1]:1935".parse().unwrap()]);
        let t = RtmpTarget::parse("rtmp://127.0.0.1:1/live/key").unwrap();
        assert_eq!(t.resolve().await.unwrap(), vec!["127.0.0.1:1".parse().unwrap()]);
    }
}
```

`rtmp-runtime/tests/client_adapter.rs` (publish against the real server adapter over loopback TCP, over IPv6 when available, plus black-hole and rejection cases):

```rust
//! New client adapter (SP6.1) against `AsyncRtmpServer` over real loopback sockets,
//! and the bounded failure modes over `duplex` + paused time.
#![cfg(feature = "tokio")]

use std::io::ErrorKind;
use std::time::Duration;

use rtmp_runtime::client::ClientEvent;
use rtmp_runtime::io::{AsyncRtmpClient, AsyncRtmpServer, RtmpTimeouts};
use rtmp_runtime::server::{ServerConfig, ServerEvent};
use rtmp_runtime::target::RtmpTarget;

async fn publish_roundtrip(bind: &str, url_host: &str) {
    let server = match AsyncRtmpServer::bind(bind, ServerConfig::default()).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("skipping {bind}: {e}");
            return;
        }
    };
    let port = server.local_addr().unwrap().port();
    let accepted = tokio::spawn(async move {
        let mut conn = server.accept().await.unwrap();
        let mut seen = Vec::new();
        while let Some(batch) = conn.next_events().await.unwrap() {
            seen.extend(batch);
            if seen.iter().any(|e| matches!(e, ServerEvent::Media { .. })) {
                break;
            }
        }
        seen
    });
    let target = RtmpTarget::parse(&format!("rtmp://{url_host}:{port}/live/testkey")).unwrap();
    let mut c = AsyncRtmpClient::connect(&target, RtmpTimeouts::default()).await.unwrap();
    c.publish().await.unwrap();
    c.send_video(0, &[0x17, 0x01, 0, 0, 0, 0xDE, 0xAD]).await.unwrap();
    let seen = accepted.await.unwrap();
    assert!(seen.iter().any(|e| matches!(e, ServerEvent::Connected { app } if app == "live")));
    assert!(seen.iter().any(|e| matches!(e, ServerEvent::Publish { stream_key, .. } if stream_key == "testkey")));
    assert!(seen.iter().any(|e| matches!(e, ServerEvent::Media { .. })));
}

#[tokio::test]
async fn publishes_over_ipv4_loopback() {
    publish_roundtrip("127.0.0.1:0", "127.0.0.1").await;
}

/// Defect 8 end to end: the bracketed IPv6 URL both resolves and connects.
#[tokio::test]
async fn publishes_over_ipv6_loopback() {
    publish_roundtrip("[::1]:0", "[::1]").await;
}

#[tokio::test(start_paused = true)]
async fn publish_to_a_peer_that_never_answers_times_out_at_handshake() {
    let (client_io, _server_io) = tokio::io::duplex(8192);
    let target = RtmpTarget::parse("rtmp://h/live/k").unwrap();
    let mut c = AsyncRtmpClient::from_stream(client_io, &target, RtmpTimeouts::default().with_handshake(Duration::from_secs(4)));
    let t0 = tokio::time::Instant::now();
    let err = c.publish().await.expect_err("handshake deadline");
    assert_eq!(err.kind(), ErrorKind::TimedOut, "{err}");
    assert_eq!(t0.elapsed(), Duration::from_secs(4));
}

#[tokio::test]
async fn a_server_that_rejects_publish_surfaces_connection_refused() {
    let server = AsyncRtmpServer::bind("127.0.0.1:0", ServerConfig::default().with_expected_stream_key(Some("right".into())))
        .await
        .unwrap();
    let port = server.local_addr().unwrap().port();
    tokio::spawn(async move {
        let mut conn = server.accept().await.unwrap();
        while let Ok(Some(_)) = conn.next_events().await {}
    });
    let target = RtmpTarget::parse(&format!("rtmp://127.0.0.1:{port}/live/wrong")).unwrap();
    let mut c = AsyncRtmpClient::connect(&target, RtmpTimeouts::default()).await.unwrap();
    let err = c.publish().await.expect_err("BadName");
    assert_eq!(err.kind(), ErrorKind::ConnectionRefused, "{err}");
    let _ = ClientEvent::Closed; // keep the import used if the compiler prunes it
}
```

- [ ] **Step 2: Run, expect FAIL**

```bash
cargo test --locked -p rtmp-runtime --all-features --lib target --test client_adapter 2>&1 | grep -E '^error|cannot find|unresolved|test result' | head
```

Expected: compile errors (`target` module, `AsyncRtmpClient` not found).

- [ ] **Step 3: Implement `target.rs`**

```rust
//! `rtmp://host[:port]/app/stream-key` -> typed target, built with the `url` crate.
//! Replaces `format!("rtmp://{host}:{port}/{app}")`, which produces `rtmp://::1:1935/live`
//! for an IPv6 address (defect 8) and leaks userinfo/query into `tcUrl`.

use std::net::{IpAddr, SocketAddr};

use url::{Host, Url};

pub const RTMP_DEFAULT_PORT: u16 = 1935;
const SCHEME: &str = "rtmp";

// RtmpTarget / RtmpUrlError as in the Interfaces block.

impl RtmpTarget {
    pub fn parse(url: &str) -> Result<Self, RtmpUrlError> {
        let base = parse_base(url)?;
        let mut segs = base.path_segments().ok_or(RtmpUrlError::NoApp)?;
        let app = segs.next().filter(|s| !s.is_empty()).ok_or(RtmpUrlError::NoApp)?.to_string();
        let rest: Vec<&str> = segs.collect();
        let mut key = rest.join("/");
        if key.is_empty() {
            return Err(RtmpUrlError::NoStreamKey);
        }
        if let Some(q) = base.query() {
            key.push('?');
            key.push_str(q);
        }
        Self::build(&base, &app, &key)
    }

    pub fn from_parts(base: &str, app: &str, stream_key: &str) -> Result<Self, RtmpUrlError> {
        Self::build(&parse_base(base)?, app, stream_key)
    }

    fn build(base: &Url, app: &str, stream_key: &str) -> Result<Self, RtmpUrlError> {
        let host = base.host().ok_or(RtmpUrlError::NoHost)?.to_owned();
        let mut tc = base.clone();
        tc.set_path(&format!("/{app}"));
        tc.set_query(None);
        tc.set_fragment(None);
        let _ = tc.set_username("");
        let _ = tc.set_password(None);
        Ok(RtmpTarget {
            host,
            port: base.port().unwrap_or(RTMP_DEFAULT_PORT),
            app: app.to_string(),
            stream_key: stream_key.to_string(),
            tc_url: tc.to_string(),
        })
    }

    pub async fn resolve(&self) -> std::io::Result<Vec<SocketAddr>> {
        match &self.host {
            Host::Ipv4(a) => Ok(vec![SocketAddr::new(IpAddr::V4(*a), self.port)]),
            Host::Ipv6(a) => Ok(vec![SocketAddr::new(IpAddr::V6(*a), self.port)]),
            Host::Domain(d) => Ok(tokio::net::lookup_host((d.as_str(), self.port)).await?.collect()),
        }
    }
}

fn parse_base(url: &str) -> Result<Url, RtmpUrlError> {
    let u = Url::parse(url).map_err(|e| RtmpUrlError::Parse(e.to_string()))?;
    if u.scheme() != SCHEME {
        return Err(RtmpUrlError::Scheme(u.scheme().to_string()));
    }
    Ok(u)
}
```

(`Url::host()`, `Host::to_owned() -> Host<String>`, `Url::path_segments()`, `set_username`/`set_password` (return `Result<(), ()>`), `set_path` are in `url-2.5.8/src/lib.rs`; `Url::to_string()` of a non-special `rtmp://[::1]:1935/live` keeps the brackets and the port because `rtmp` is not a special scheme so no default port is elided: verified by the unit test, which is the compile-and-behaviour check. If `Url` drops `:1935` or rewrites the path, fix `build` and record it.)

`codec.rs` client half and `io.rs` client:

```rust
pub(crate) struct ClientCodec { session: ClientSession, reply: Vec<u8> }
// new(config) / session_mut() / take_reply(): same shape as ServerCodec;
// Decoder: Item = Vec<ClientEvent>, decode = session.handle_data(&chunk) -> (reply, events).
// Encoder<Vec<u8>>: dst.extend_from_slice.

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncRtmpClient<S> {
    pub fn from_stream(stream: S, target: &RtmpTarget, timeouts: RtmpTimeouts) -> Self {
        let mut cfg = ClientConfig::default();
        cfg.app = target.app.clone();
        cfg.stream_key = target.stream_key.clone();
        cfg.tc_url = Some(target.tc_url.clone());
        let mut codec = ClientCodec::new(ClientSession::new(cfg));
        let c0c1 = codec.session_mut().start();
        let mut framed = Framed::new(stream, codec);
        framed.write_buffer_mut().extend_from_slice(&c0c1); // flushed by the first `publish` call
        Self { framed, ready: None, closed: false, timeouts, handshake_deadline: Some(Instant::now() + timeouts.handshake) }
    }

    pub async fn publish(&mut self) -> io::Result<()> {
        let deadline = self.handshake_deadline.unwrap_or_else(|| Instant::now() + self.timeouts.handshake);
        let run = async {
            loop {
                let Some(events) = self.next_events_until(None).await? else {
                    return Err(io::Error::new(ErrorKind::UnexpectedEof, "peer closed during publish"));
                };
                for e in &events {
                    match e {
                        ClientEvent::Publishing => return Ok(()),
                        ClientEvent::Error { code, description } => {
                            return Err(io::Error::new(ErrorKind::ConnectionRefused, format!("{code}: {description}")));
                        }
                        _ => {}
                    }
                }
            }
        };
        match tokio::time::timeout_at(deadline, run).await {
            Err(_) => { self.closed = true; Err(timed_out("handshake")) }
            Ok(r) => { if r.is_ok() { self.handshake_deadline = None; } r }
        }
    }
    // next_events: identical loop to RtmpConnection::next_events (flush -> hand out `ready` -> wait one
    // deadline), with ClientEvent::Closed ending the connection; `next_events_until(None)` is that loop,
    // shared so `publish` can reuse it under its own outer deadline.
    // send_audio/send_video/send_metadata:
    //     let bytes = self.framed.codec_mut().session_mut().send_video(ts, data).map_err(io_err)?;
    //     self.framed.write_buffer_mut().extend_from_slice(&bytes);
    //     bounded flush under timeouts.write (TimedOut on expiry).
}

impl AsyncRtmpClient<TcpStream> {
    pub async fn connect(target: &RtmpTarget, timeouts: RtmpTimeouts) -> io::Result<Self> {
        let stream = tokio::time::timeout(timeouts.connect, async {
            let addrs = target.resolve().await?;
            TcpStream::connect(&addrs[..]).await
        })
        .await
        .map_err(|_| timed_out("connect"))??;
        Ok(Self::from_stream(stream, target, timeouts))
    }
}
```

The "next_events" loop body is the same one as Task 8's; factor it into a private generic helper `pump<S, C>(...)` only if the duplication exceeds ~25 lines after writing; otherwise duplicate and note it (one reviewer decision, not a design change). `ClientEvent::Closed` sets `closed`. `lib.rs` docs: replace "It has no tokio socket adapter of its own" and "no equivalent client adapter" with the `AsyncRtmpClient` description.

- [ ] **Step 4: Run, expect PASS**

```bash
cargo test --locked -p rtmp-runtime --all-features 2>&1 | grep -E '^test result|FAILED|panicked|^error' | sort | uniq -c
```

Expected: all ok. If the host has no IPv6 loopback, `publishes_over_ipv6_loopback` prints `skipping [::1]:0` and passes vacuously; the unit tests in `target.rs` carry the defect-8 assertion regardless.

- [ ] **Step 5: Revert-check (defect 8)**

In `RtmpTarget::build` temporarily replace the `Url`-based `tc_url` with `format!("rtmp://{}:{}/{app}", ip_or_host, port)` using the `IpAddr` `Display` (unbracketed). Run `cargo test --locked -p rtmp-runtime --all-features --lib target`. Expected: `ipv6_host_is_bracketed_in_tc_url` FAILS with `left: "rtmp://::1:1935/live"`, and `userinfo_query_and_fragment_never_reach_tc_url` fails if the naive build copied the userinfo. Record the failing output, restore, re-run Step 4.

- [ ] **Step 6: Commit**

```bash
git add rtmp-runtime
git commit -m "feat(rtmp-runtime): AsyncRtmpClient adapter and url-built RtmpTarget (tcUrl, IPv6 brackets, userinfo stripped)"
```

---

### Task 10: dvb-stream `TsFramer` becomes a `tokio_util` TS `Decoder`

**Files:**
- Create: `dvb-stream/src/ts_codec.rs`
- Delete: `dvb-stream/src/framer.rs`
- Modify: `dvb-stream/src/section_stream.rs` (rewrite the `SectionStream` half, lines 1-127; the UDP half moves in Task 11)
- Modify: `dvb-stream/src/t2mi_stream.rs` (lines 1-102)
- Modify: `dvb-stream/src/lib.rs` (`mod ts_codec; pub use ts_codec::TsDecoder;`, doc text lines 1-37)
- Test: `dvb-stream/tests/ts_codec.rs` (new); the existing `differential.rs`, `t2mi_differential.rs`, `resync_observability.rs`, `error_and_datagram.rs` (reader parts) and `golden_events.rs` must pass UNCHANGED

**Interfaces:**

```rust
// ts_codec.rs
#[derive(Debug, Clone)]
pub struct TsDecoder { /* synced, stats, datagram */ }
impl TsDecoder {
    pub fn new() -> Self;                      // byte-stream mode (file, TCP)
    pub fn datagram() -> Self;                 // each buffer handed in is one independent datagram (UDP)
    pub fn resync_stats(&self) -> ResyncStats;
}
impl Default for TsDecoder { fn default() -> Self { Self::new() } }
impl Decoder for TsDecoder { type Item = bytes::Bytes /* exactly one 188-byte packet */; type Error = std::io::Error; }
// section_stream.rs / t2mi_stream.rs: unchanged public surface for the reader case
impl<R: AsyncRead + Unpin> SectionStream<R> { pub fn new(reader: R) -> Self; with_builder; with_demux; stats; resync_stats; take_io_error }
impl<R: AsyncRead + Unpin> T2miEventStream<R> { pub fn new(reader: R, pid: u16) -> Self; with_pump; stats; resync_stats; take_io_error }
// shared, crate-private: SectionPump<S>/T2miPump<S> drive any `Stream<Item = io::Result<Bytes>>` of aligned packets
```

`SectionStream<R>` / `T2miEventStream<R>` keep `R` = the reader type, so every existing call site and the `drain_section_stream<R: AsyncRead + Unpin>(stream: &mut SectionStream<R>)` test helper compile unchanged. Internally they wrap `FramedRead<R, TsDecoder>`.

- [ ] **Step 1: Write the failing tests**

`dvb-stream/tests/ts_codec.rs`:

```rust
//! `TsDecoder` framing edges (review-focus item 5), driven through `FramedRead`.

use bytes::{Bytes, BytesMut};
use dvb_stream::TsDecoder;
use tokio_util::codec::Decoder;

fn pkt(tag: u8) -> [u8; 188] {
    let mut p = [0xFFu8; 188];
    p[0] = 0x47;
    p[1] = tag;
    p
}

fn drain(d: &mut TsDecoder, buf: &mut BytesMut) -> Vec<Bytes> {
    let mut out = Vec::new();
    while let Some(p) = d.decode(buf).unwrap() {
        out.push(p);
    }
    out
}

#[test]
fn one_byte_at_a_time_yields_every_packet_exactly_once() {
    let mut d = TsDecoder::new();
    let mut buf = BytesMut::new();
    let mut got = Vec::new();
    for tag in 1..=3u8 {
        for b in pkt(tag) {
            buf.extend_from_slice(&[b]);
            got.extend(drain(&mut d, &mut buf));
        }
    }
    assert_eq!(got.len(), 3);
    assert_eq!(got.iter().map(|p| p[1]).collect::<Vec<_>>(), vec![1, 2, 3]);
    assert_eq!(d.resync_stats().resyncs, 1);
    assert_eq!(d.resync_stats().bytes_discarded, 0);
}

#[test]
fn a_corrupt_sync_byte_mid_stream_counts_one_desync_and_recovers() {
    let mut d = TsDecoder::new();
    let mut buf = BytesMut::new();
    buf.extend_from_slice(&pkt(1));
    let mut bad = pkt(2);
    bad[0] = 0x00;
    buf.extend_from_slice(&bad);
    buf.extend_from_slice(&pkt(3));
    let got = drain(&mut d, &mut buf);
    assert_eq!(got.len(), 1, "only the packet before the corruption survives (old behaviour: rest of the buffer is dropped)");
    assert_eq!(d.resync_stats().desyncs, 1);
    assert_eq!(d.resync_stats().bytes_discarded, 188 * 2);
    // the stream recovers on the next aligned data
    buf.extend_from_slice(&pkt(4));
    buf.extend_from_slice(&pkt(5));
    let got = drain(&mut d, &mut buf);
    assert_eq!(got.iter().map(|p| p[1]).collect::<Vec<_>>(), vec![4, 5]);
    assert_eq!(d.resync_stats().resyncs, 2);
}

#[test]
fn datagram_mode_resyncs_per_datagram_and_drops_the_stray_tail() {
    let mut d = TsDecoder::datagram();
    let mut buf = BytesMut::new();
    for tag in 0..10u8 {
        buf.extend_from_slice(&pkt(tag));
    }
    buf.extend_from_slice(&[0x47, 1, 2, 3, 4]); // 5 stray bytes starting like a sync byte
    let mut got = Vec::new();
    while let Some(p) = d.decode_eof(&mut buf).unwrap() {
        got.push(p);
    }
    assert_eq!(got.len(), 10, "all 10 packets of a >7x188 datagram");
    assert!(buf.is_empty(), "the stray tail is consumed, never stitched onto the next datagram");
    assert_eq!(d.resync_stats().bytes_discarded, 5);
    // next datagram starts clean even though the previous tail looked like a sync byte
    buf.extend_from_slice(&pkt(77));
    assert_eq!(d.decode_eof(&mut buf).unwrap().unwrap()[1], 77);
}

#[test]
fn a_pure_junk_datagram_does_not_stall_or_grow_the_buffer() {
    let mut d = TsDecoder::datagram();
    let mut buf = BytesMut::from(&[0x00u8; 1316][..]);
    assert!(d.decode_eof(&mut buf).unwrap().is_none());
    assert!(buf.is_empty());
    assert_eq!(d.resync_stats().bytes_discarded, 1316);
}

#[test]
fn a_partial_packet_at_eof_in_stream_mode_is_a_clean_end_not_an_error() {
    let mut d = TsDecoder::new();
    let mut buf = BytesMut::new();
    buf.extend_from_slice(&pkt(1));
    buf.extend_from_slice(&pkt(2)[..100]);
    assert_eq!(drain(&mut d, &mut buf).len(), 1);
    assert!(d.decode_eof(&mut buf).unwrap().is_none(), "old behaviour: trailing partial packet ignored");
}
```

(`dvb-stream/Cargo.toml` already has `bytes` from Task 2; add `bytes` and `tokio-util` are normal deps so the test target sees them.)

- [ ] **Step 2: Run, expect FAIL**

```bash
cargo test --locked -p dvb-stream --all-features --test ts_codec 2>&1 | grep -E '^error|unresolved|cannot find|test result' | head
```

Expected: `unresolved import dvb_stream::TsDecoder`.

- [ ] **Step 3: Implement**

`ts_codec.rs`:

```rust
//! 188-byte TS packet framing as a `tokio_util::codec::Decoder` (W1 SP1.1).
//! Replaces `framer::TsFramer`'s hand-rolled read buffer: `FramedRead` owns the
//! buffer and the read loop; this type owns only the resync/alignment state.

use std::io;

use bytes::{Buf, Bytes, BytesMut};
use tokio_util::codec::Decoder;

use crate::ResyncStats;
use crate::resync::{TS_PACKET_SIZE, TS_SYNC_BYTE, resync};

#[derive(Debug, Clone, Default)]
pub struct TsDecoder {
    synced: bool,
    stats: ResyncStats,
    datagram: bool,
}

impl TsDecoder {
    #[must_use]
    pub fn new() -> Self { Self::default() }

    /// Each buffer handed to `decode_eof` is one independent datagram: resync from scratch
    /// every time and never carry a partial tail over (#1036 / W-DS-2).
    #[must_use]
    pub fn datagram() -> Self { Self { datagram: true, ..Self::default() } }

    #[must_use]
    pub fn resync_stats(&self) -> ResyncStats { self.stats }
}

impl Decoder for TsDecoder {
    type Item = Bytes;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> io::Result<Option<Bytes>> {
        if !self.synced {
            if src.is_empty() {
                return Ok(None);
            }
            match resync(src) {
                Some(off) => {
                    src.advance(off);
                    self.synced = true;
                    self.stats.resyncs += 1;
                    self.stats.bytes_discarded += off as u64;
                }
                None => {
                    // no sync byte at all: discard the chunk (counted), exactly as before
                    self.stats.bytes_discarded += src.len() as u64;
                    src.clear();
                    return Ok(None);
                }
            }
        }
        if src.len() < TS_PACKET_SIZE {
            return Ok(None);
        }
        if src[0] != TS_SYNC_BYTE {
            // Mid-stream desync: drop everything buffered (the old per-chunk behaviour) and re-resync.
            self.stats.desyncs += 1;
            self.stats.bytes_discarded += src.len() as u64;
            src.clear();
            self.synced = false;
            return Ok(None);
        }
        Ok(Some(src.split_to(TS_PACKET_SIZE).freeze()))
    }

    fn decode_eof(&mut self, src: &mut BytesMut) -> io::Result<Option<Bytes>> {
        if let Some(p) = self.decode(src)? {
            return Ok(Some(p));
        }
        // End of input (stream mode) or end of datagram (datagram mode): a trailing partial packet
        // is dropped. Stream mode ignores it silently (clean EOF); datagram mode counts it and
        // forgets the sync state so the next datagram resyncs independently.
        if self.datagram {
            self.stats.bytes_discarded += src.len() as u64;
            self.synced = false;
        }
        src.clear();
        Ok(None)
    }
}
```

`section_stream.rs` (reader half):

```rust
use bytes::Bytes;
use tokio_util::codec::FramedRead;

use crate::ts_codec::TsDecoder;

/// Shared by every `SectionStream` flavour: drains a stream of aligned packets into the demux.
pub(crate) struct SectionPump<S> {
    pub(crate) frames: S,
    demux: SiDemux,
    queue: VecDeque<SectionEvent>,
    io_error: Option<std::io::Error>,
    done: bool,
}

impl<S: Stream<Item = std::io::Result<Bytes>> + Unpin> SectionPump<S> {
    pub(crate) fn new(frames: S, demux: SiDemux) -> Self {
        Self { frames, demux, queue: VecDeque::new(), io_error: None, done: false }
    }
    pub(crate) fn poll_next(&mut self, cx: &mut Context<'_>) -> Poll<Option<SectionEvent>> {
        loop {
            if let Some(ev) = self.queue.pop_front() {
                return Poll::Ready(Some(ev));
            }
            if self.done {
                return Poll::Ready(None);
            }
            match Pin::new(&mut self.frames).poll_next(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(None) => self.done = true,
                Poll::Ready(Some(Err(e))) => { self.io_error = Some(e); self.done = true; }
                Poll::Ready(Some(Ok(pkt))) => self.queue.extend(self.demux.feed(&pkt)),
            }
        }
    }
    pub(crate) fn stats(&self) -> dvb_si::demux::Stats { self.demux.stats() }
    pub(crate) fn take_io_error(&mut self) -> Option<std::io::Error> { self.io_error.take() }
}

pub struct SectionStream<R> {
    pump: SectionPump<FramedRead<R, TsDecoder>>,
}

impl<R: AsyncRead + Unpin> SectionStream<R> {
    pub fn new(reader: R) -> Self { Self::with_demux(reader, SiDemux::builder().build()) }
    pub fn with_builder(reader: R, builder: SiDemuxBuilder) -> Self { Self::with_demux(reader, builder.build()) }
    pub fn with_demux(reader: R, demux: SiDemux) -> Self {
        Self { pump: SectionPump::new(FramedRead::new(reader, TsDecoder::new()), demux) }
    }
    pub fn stats(&self) -> dvb_si::demux::Stats { self.pump.stats() }
    pub fn resync_stats(&self) -> ResyncStats { self.pump.frames.decoder().resync_stats() }
    pub fn take_io_error(&mut self) -> Option<std::io::Error> { self.pump.take_io_error() }
}

impl<R: AsyncRead + Unpin> Stream for SectionStream<R> {
    type Item = SectionEvent;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<SectionEvent>> {
        self.get_mut().pump.poll_next(cx)
    }
}
```

`T2miEventStream` mirrors this with a `T2miEventPump<S>` over `T2miPump::feed_ts`. Delete `framer.rs`, `UdpReader` (Task 11 provides the UDP replacement; until then keep the UDP constructors compiling by temporarily leaving `UdpReader` and its `bind_multicast` impls in place and re-pointing them to `SectionStream::new(UdpReader { socket })` WITHOUT `set_datagram_framed` marked `// replaced in Task 11`; the UDP tests stay green because Task 11 lands next).

- [ ] **Step 4: Run, expect PASS; goldens and differential tests unchanged**

```bash
cargo test --locked -p dvb-stream --all-features 2>&1 | grep -E '^test result|FAILED|panicked|^error' | sort | uniq -c
```

Expected: all ok. `section_stream_mid_stream_corruption_detected` still sees `bytes_discarded == 564` and `desyncs == 1`; `golden_events` is byte-identical; `section_stream_slow_reader_corruption_recovers` (18-byte reads) still gets `resyncs == 2`.

- [ ] **Step 5: Revert-check**

There is no defect in this task; the equivalent evidence is that the new tests bite. Temporarily delete the `src.clear()` in the `None =>` junk arm of `decode` and run `cargo test --locked -p dvb-stream --all-features --test ts_codec a_pure_junk`: expect FAIL (`buf.is_empty()` assertion). Restore.

- [ ] **Step 6: Commit**

```bash
git add dvb-stream
git commit -m "refactor(dvb-stream)!: TsFramer replaced by a tokio_util TS Decoder (FramedRead); framing behaviour pinned by goldens"
```

---

### Task 11: dvb-stream UDP on `UdpFramed` with `socket2` binds (SP1.1, SP1.6)

**Files:**
- Modify: `dvb-stream/src/section_stream.rs` (UDP half, lines 128-190), `dvb-stream/src/t2mi_stream.rs` (UDP half, lines 104-133)
- Create: `dvb-stream/src/udp.rs` (feature `udp`): `MulticastConfig`, `UdpPackets`
- Modify: `dvb-stream/src/lib.rs` (`#[cfg(feature = "udp")] pub mod udp;`, re-exports, doc table)
- Modify: `dvb-stream/tests/error_and_datagram.rs` lines ~95-175 (the fixed-port `TEST_PORT = 43_991` multicast test)
- Test: `dvb-stream/tests/udp_stream.rs` (new)

**Interfaces** (feature `udp`):

```rust
// udp.rs
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct MulticastConfig {
    pub bind_addr: std::net::SocketAddrV4,   // port to listen on (usually 0.0.0.0:PORT)
    pub group: std::net::Ipv4Addr,           // group to join
    pub interface: std::net::Ipv4Addr,       // interface for the join; default = bind_addr.ip() (the old behaviour)
    pub recv_buffer_size: Option<usize>,     // SO_RCVBUF; default None = OS default
    pub reuse_address: bool,                 // SO_REUSEADDR (+ SO_REUSEPORT on unix); default true
}
impl MulticastConfig {
    pub fn new(bind_addr: SocketAddrV4, group: Ipv4Addr) -> Self;
    pub fn with_interface(self, interface: Ipv4Addr) -> Self;
    pub fn with_recv_buffer_size(self, bytes: usize) -> Self;
    pub fn with_reuse_address(self, on: bool) -> Self;
    /// socket2 bind + join, returned non-blocking and ready for tokio.
    pub fn bind(&self) -> std::io::Result<std::net::UdpSocket>;
}
pub(crate) struct UdpPackets(UdpFramed<TsDecoder, tokio::net::UdpSocket>);   // Stream<Item = io::Result<Bytes>>

// section_stream.rs
pub struct UdpSectionStream { /* SectionPump<UdpPackets> */ }
impl UdpSectionStream {
    pub fn from_socket(socket: tokio::net::UdpSocket) -> Self;                                    // already bound: tests bind port 0
    pub fn from_socket_with_demux(socket: tokio::net::UdpSocket, demux: SiDemux) -> Self;
    pub async fn bind_multicast(bind_addr: SocketAddrV4, group: Ipv4Addr) -> io::Result<Self>;      // same signature as before
    pub async fn bind(config: &MulticastConfig) -> io::Result<Self>;
    pub fn stats(&self) -> dvb_si::demux::Stats; pub fn resync_stats(&self) -> ResyncStats; pub fn take_io_error(&mut self) -> Option<io::Error>;
}
impl Stream for UdpSectionStream { type Item = SectionEvent; }
// t2mi_stream.rs: UdpT2miStream with the same shape plus `pid: u16` (`from_socket(socket, pid)`, `bind_multicast(bind_addr, group, pid)`, `bind(&config, pid)`).
```

Breaking, recorded in the changelog: `SectionStream<UdpReader>` / `T2miEventStream<UdpReader>` and `section_stream::UdpReader` are removed; their `bind_multicast` constructors move to `UdpSectionStream` / `UdpT2miStream` with unchanged signatures.

- [ ] **Step 1: Write the failing tests**

`dvb-stream/tests/udp_stream.rs` (port 0 everywhere; no fixed port, no sleeps):

```rust
#![cfg(feature = "udp")]

use std::net::{Ipv4Addr, SocketAddrV4};
use std::pin::Pin;
use std::time::Duration;

use dvb_stream::UdpSectionStream;
use dvb_stream::udp::MulticastConfig;
use futures_core::Stream;

fn pkt(cc: u8) -> [u8; 188] {
    let mut p = [0xFFu8; 188];
    p[0] = 0x47;
    p[1] = 0x1F; // null PID 0x1FFF stuffing: counted by the demux, no section event
    p[2] = 0xFF;
    p[3] = 0x10 | (cc & 0x0F);
    p
}

async fn pump_until_packets(stream: &mut UdpSectionStream, want: u64) {
    tokio::time::timeout(Duration::from_secs(20), async {
        while stream.stats().packets < want {
            let _ = std::future::poll_fn(|cx| match Pin::new(&mut *stream).poll_next(cx) {
                std::task::Poll::Ready(ev) => std::task::Poll::Ready(ev),
                std::task::Poll::Pending => std::task::Poll::Pending,
            })
            .await;
        }
    })
    .await
    .expect("packets must arrive");
}

/// 10 packets (1880 B, larger than 7x188) plus a 5-byte tail in ONE datagram, over plain
/// unicast loopback on an ephemeral port (`from_socket`), so it needs no multicast support.
#[tokio::test]
async fn an_oversized_datagram_is_fully_seen_and_its_stray_tail_is_dropped() {
    let rx = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let addr = rx.local_addr().unwrap();
    let mut stream = UdpSectionStream::from_socket(rx);
    let tx = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let mut datagram = Vec::new();
    for cc in 0..10u8 {
        datagram.extend_from_slice(&pkt(cc));
    }
    datagram.extend_from_slice(&[0x47, 1, 2, 3, 4]);
    tx.send_to(&datagram, addr).await.unwrap();
    pump_until_packets(&mut stream, 10).await;
    assert_eq!(stream.stats().packets, 10);
    assert_eq!(stream.resync_stats().bytes_discarded, 5);
}

/// A junk-only datagram between two good ones does not poison either neighbour.
#[tokio::test]
async fn a_junk_datagram_between_good_ones_is_isolated() {
    let rx = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let addr = rx.local_addr().unwrap();
    let mut stream = UdpSectionStream::from_socket(rx);
    let tx = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    tx.send_to(&pkt(0), addr).await.unwrap();
    tx.send_to(&[0u8; 700], addr).await.unwrap();
    tx.send_to(&pkt(1), addr).await.unwrap();
    pump_until_packets(&mut stream, 2).await;
    assert_eq!(stream.stats().packets, 2);
}

#[test]
fn multicast_config_applies_the_socket_options() {
    let cfg = MulticastConfig::new(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0), Ipv4Addr::new(239, 255, 42, 1))
        .with_recv_buffer_size(256 * 1024);
    // Join may be refused in a sandbox with no multicast route: that is an environment limit,
    // not a failure of the option wiring, so only the Ok path asserts.
    match cfg.bind() {
        Ok(sock) => {
            let s = socket2::SockRef::from(&sock);
            assert!(s.recv_buffer_size().unwrap() >= 256 * 1024 / 2, "SO_RCVBUF applied (kernels may double/clamp)");
            assert!(s.reuse_address().unwrap(), "SO_REUSEADDR applied by default");
            assert!(sock.local_addr().unwrap().port() != 0);
        }
        Err(e) => eprintln!("skipping multicast_config_applies_the_socket_options: {e}"),
    }
}
```

(`socket2` is an optional dep enabled by `udp`; the test target needs it too: it is a normal dependency of the crate, so `socket2::SockRef` resolves in tests.)

Replace the fixed-port test in `tests/error_and_datagram.rs` (the `TEST_PORT = 43_991` multicast one) with a call into `UdpSectionStream::from_socket` over an ephemeral unicast socket, and delete its `tokio::time::sleep(20ms)` nudge loop in favour of `pump_until_packets`-style polling (SP7); keep one multicast test that skips cleanly when the join fails, using `MulticastConfig` with `bind_addr` port 0.

- [ ] **Step 2: Run, expect FAIL**

```bash
cargo test --locked -p dvb-stream --all-features --test udp_stream 2>&1 | grep -E '^error|unresolved|cannot find|test result' | head
```

Expected: `UdpSectionStream` / `udp::MulticastConfig` unresolved.

- [ ] **Step 3: Implement**

`udp.rs`:

```rust
//! UDP/multicast input: `socket2` for the bind and join (SO_REUSEADDR, SO_RCVBUF, multicast
//! interface), `tokio_util::udp::UdpFramed` + [`TsDecoder::datagram`] for framing.

use std::io;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket as StdUdp};
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures_core::Stream;
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use tokio_util::udp::UdpFramed;

use crate::ts_codec::TsDecoder;

// MulticastConfig struct + new/with_* as in the Interfaces block (interface defaults to *bind_addr.ip()).

impl MulticastConfig {
    pub fn bind(&self) -> io::Result<StdUdp> {
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
        if self.reuse_address {
            socket.set_reuse_address(true)?;
            #[cfg(all(unix, not(any(target_os = "solaris", target_os = "illumos", target_os = "cygwin", target_os = "nuttx", target_os = "wasi"))))]
            socket.set_reuse_port(true)?;
        }
        if let Some(n) = self.recv_buffer_size {
            socket.set_recv_buffer_size(n)?;
        }
        socket.bind(&SockAddr::from(self.bind_addr))?;
        socket.join_multicast_v4(&self.group, &self.interface)?;
        socket.set_nonblocking(true)?;
        Ok(socket.into())
    }
}

/// `UdpFramed` yields `(packet, source)`; the streams only want the packet.
pub(crate) struct UdpPackets(UdpFramed<TsDecoder, tokio::net::UdpSocket>);

impl UdpPackets {
    pub(crate) fn new(socket: tokio::net::UdpSocket) -> Self {
        Self(UdpFramed::new(socket, TsDecoder::datagram()))
    }
    pub(crate) fn resync_stats(&self) -> crate::ResyncStats { self.0.codec().resync_stats() }
}

impl Stream for UdpPackets {
    type Item = io::Result<Bytes>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.0).poll_next(cx).map(|o| o.map(|r| r.map(|(pkt, _src)| pkt)))
    }
}
```

`UdpSectionStream::bind_multicast(bind_addr, group)` = `Self::bind(&MulticastConfig::new(bind_addr, group))`; `bind(config)` = `let std = config.bind()?; Ok(Self::from_socket(tokio::net::UdpSocket::from_std(std)?))`. `socket2::Socket -> std::net::UdpSocket` is `From` (verified in `socket2-0.6.5/src/socket.rs`: `impl From<Socket> for std::net::UdpSocket`). The `udp` feature in `Cargo.toml` is `udp = ["dep:socket2"]` (Task 2). Remove the temporary `UdpReader` and `// replaced in Task 11` shims from Task 10.

- [ ] **Step 4: Run, expect PASS**

```bash
cargo test --locked -p dvb-stream --all-features 2>&1 | grep -E '^test result|FAILED|panicked|^error' | sort | uniq -c
cargo build --locked -p dvb-stream --no-default-features 2>&1 | tail -1
cargo clippy --locked -p dvb-stream --all-features --all-targets -- -D warnings 2>&1 | tail -2
```

Expected: all ok; the no-default-features build has no `udp` module and no socket2.

- [ ] **Step 5: Revert-check**

The old behaviours were (a) a fixed-port test and (b) `bind` + `join_multicast_v4` on a plain tokio socket with no options. Evidence for (b): temporarily make `MulticastConfig::bind` skip `set_recv_buffer_size`; `multicast_config_applies_the_socket_options` must FAIL when the environment allows the join (if it skips, record "skipped: no multicast route" and instead assert the call order by running the `with_recv_buffer_size(256 KiB)` check against a plain `UdpSocket::bind(("0.0.0.0", 0))`, whose default `SO_RCVBUF` is not 256 KiB on macOS/Linux, to show the assertion discriminates). Restore.

- [ ] **Step 6: Commit**

```bash
git add dvb-stream
git commit -m "feat(dvb-stream)!: UDP input on tokio_util UdpFramed with socket2 binds (SO_RCVBUF/REUSEADDR/interface); UdpReader removed"
```

---

### Task 12: SP7 test-harness fixes in these three crates

**Files:**
- Modify: `rtsp-runtime/tests/io_loopback.rs` line 113 (the `sleep(100ms)` that keeps the connection open)
- Modify: `rtmp-runtime/tests/ffmpeg_publish.rs` line 66 (`std::thread::sleep(Duration::from_secs(40))`)
- Verify (already done in Tasks 5-11): rtsp/rtmp timeout tests use paused time; dvb-stream udp tests bind port 0 and have no sleeps

**Interfaces:** none.

- [ ] **Step 1: Find the remaining waits (the failing check)**

```bash
grep -rnE 'sleep\(|thread::sleep|bind\("127.0.0.1:[0-9]+|TEST_PORT|:\s*43_' rtsp-runtime/tests rtmp-runtime/tests dvb-stream/tests rtsp-runtime/src rtmp-runtime/src dvb-stream/src | grep -v 'start_paused\|// '
```

Expected before the edit: the `io_loopback.rs:113` sleep and `ffmpeg_publish.rs:66`. Anything else listed is a finding: fix it the same way.

- [ ] **Step 2: Fix `io_loopback.rs`**

The server task sleeps 100 ms "until the client has read both frames". Replace the sleep with an explicit rendezvous: a `tokio::sync::oneshot` the client fires after reading frame 2, and have the server `await` it:

```rust
let (done_tx, done_rx) = tokio::sync::oneshot::channel::<()>();
let server = tokio::spawn(async move {
    // ... accept, SETUP, PLAY, send both frames ...
    srv.stream_mut().flush().await.unwrap();
    let _ = done_rx.await; // keep the connection open until the client has both frames
});
// client, after asserting f2:
let _ = done_tx.send(());
server.await.unwrap();
```

- [ ] **Step 3: Fix `ffmpeg_publish.rs`**

Lines 62-70 spawn a thread that sleeps 40 s and then runs `kill <pid>` (a fixed wait, and a PID that may have been reused by then). Delete that `std::thread::spawn` block and bound the real condition instead. Replace `drive.await;` / `let _ = ffmpeg.wait();` (lines 95-96) with:

```rust
    // Watchdog on the real condition: a wedged publish fails the test instead of hanging it.
    let outcome = tokio::time::timeout(Duration::from_secs(40), drive).await;
    if outcome.is_err() {
        let _ = ffmpeg.kill();
    }
    let _ = ffmpeg.wait();
    assert!(outcome.is_ok(), "publish wedged: no end-of-stream within 40 s");
```

(`drive` borrows `audio`/`video`/`published`; the timeout drops it before the asserts read them.) The test still skips when `ffmpeg` is not on `PATH`; `pid` and the `std::thread` import become unused: remove them.

- [ ] **Step 4: 20-run stability for every touched/added test**

```bash
for i in $(seq 1 20); do cargo test --locked -p rtsp-runtime -p rtmp-runtime -p dvb-stream --all-features 2>&1 | grep -E '^test result: FAILED|panicked|^error' ; done | sort | uniq -c
```

Expected: no output (20 consecutive clean runs). Record `20/20` in the report with the test-binary list.

- [ ] **Step 5: Commit**

```bash
git add rtsp-runtime/tests rtmp-runtime/tests
git commit -m "test(rtsp,rtmp): replace fixed sleeps with rendezvous/bounded waits"
```

---

### Task 13: Guards (spec §5) for rtsp-runtime, rtmp-runtime, dvb-stream

The guard test files go in the last code task. Each is a lexical tripwire (the same pattern as `transmux/tests/no_dom_guard.rs`): it scans `src/**/*.rs` outside `#[cfg(test)] mod` bodies and fails on the banned patterns unless allowlisted with a reason. The module doc states it is a tripwire and that review is the real control.

**Files:**
- Create: `rtsp-runtime/tests/no_handroll_guard.rs`, `rtmp-runtime/tests/no_handroll_guard.rs`, `dvb-stream/tests/no_handroll_guard.rs` (identical scanner, per-crate allowlists)

**Interfaces:** none.

- [ ] **Step 1: Write the guard (rtsp-runtime shown; the other two differ only in `ALLOW`)**

```rust
//! Tripwire against hand-rolled generic protocol code coming back (W1 spec §5).
//!
//! Scans `src/**/*.rs` outside `#[cfg(test)] mod` bodies for: HTTP/RTSP status-line literals,
//! a CRLFCRLF header terminator, `find("://")`, `strip_prefix("<scheme>://")`, SDP line building
//! (`"a=`, `"m=`, `"v=0`), civil-date helpers, a base64 alphabet literal, and
//! `thread::sleep` / `sleep(` in non-test async code.
//!
//! **This is a lexical tripwire, not a proof.** A renamed helper or a string assembled from
//! pieces evades it. Code review is the real control.

use std::fs;
use std::path::Path;

/// (file suffix, needle, reason). Empty means no exemptions.
const ALLOW: &[(&str, &str, &str)] = &[
    // rtsp-runtime: the RTSP wire literals the SANS-IO CORE legitimately owns are built by
    // rtsp_types, not by string; nothing is allowlisted.
];

const NEEDLES: &[&str] = &[
    "\"HTTP/1.",
    "\"RTSP/1.0 ",
    "\\r\\n\\r\\n",
    "find(\"://\")",
    "strip_prefix(\"rtsp://\")",
    "strip_prefix(\"rtmp://\")",
    "strip_prefix(\"http://\")",
    "\"a=",
    "\"m=",
    "\"v=0",
    "civil_from_days",
    "days_from_civil",
    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/",
    "thread::sleep",
    "tokio::time::sleep(",
];

fn rs_files(dir: &Path, out: &mut Vec<(String, String)>) {
    for entry in fs::read_dir(dir).expect("read src dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rs_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push((path.display().to_string(), fs::read_to_string(&path).expect("read source")));
        }
    }
}

/// Source with every `#[cfg(test)] mod name { ... }` body removed (brace-matched, string/char/comment aware).
fn non_test_source(src: &str) -> String {
    let lines: Vec<&str> = src.lines().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].trim_start().starts_with("#[cfg(test)]") && next_item_is_mod(&lines, i).is_some() {
            i = block_end(&lines, next_item_is_mod(&lines, i).unwrap()) + 1;
            continue;
        }
        out.push_str(lines[i]);
        out.push('\n');
        i += 1;
    }
    out
}

fn next_item_is_mod(lines: &[&str], attr: usize) -> Option<usize> {
    let mut j = attr + 1;
    while j < lines.len() && lines[j].trim_start().starts_with("#[") {
        j += 1;
    }
    let t = lines.get(j)?.trim_start();
    let t = t.strip_prefix("pub ").unwrap_or(t);
    t.starts_with("mod ").then_some(j)
}

/// Index of the line holding the closing brace of the block opened on `from`.
fn block_end(lines: &[&str], from: usize) -> usize {
    let (mut depth, mut seen_open) = (0i64, false);
    for (k, line) in lines.iter().enumerate().skip(from) {
        let (d, opened) = brace_delta(line);
        depth += d;
        seen_open |= opened;
        if seen_open && depth <= 0 {
            return k;
        }
    }
    lines.len() - 1
}

fn brace_delta(line: &str) -> (i64, bool) {
    let (mut delta, mut seen) = (0i64, false);
    let mut chars = line.chars().peekable();
    let mut in_str = false;
    while let Some(c) = chars.next() {
        if in_str {
            match c {
                '\\' => { chars.next(); }
                '"' => in_str = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '/' if chars.peek() == Some(&'/') => break,
            '\'' => {
                let rest: String = chars.clone().take(3).collect();
                if let Some(end) = rest.find('\'') {
                    for _ in 0..=end { chars.next(); }
                }
            }
            '{' => { delta += 1; seen = true; }
            '}' => delta -= 1,
            _ => {}
        }
    }
    (delta, seen)
}

#[test]
fn no_hand_rolled_protocol_code_in_src() {
    let mut files = Vec::new();
    rs_files(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
    let mut hits = Vec::new();
    for (path, src) in &files {
        let body = non_test_source(src);
        for (n, line) in body.lines().enumerate() {
            let t = line.trim_start();
            if t.starts_with("//") {
                continue; // doc/comment text may quote the patterns
            }
            for needle in NEEDLES {
                if line.contains(needle) && !ALLOW.iter().any(|(f, nd, _)| path.ends_with(f) && nd == needle) {
                    hits.push(format!("{path}:{}: {needle}", n + 1));
                }
            }
        }
    }
    assert!(hits.is_empty(), "hand-rolled protocol code found (allowlist with a reason, or use the crate):\n{}", hits.join("\n"));
}

#[test]
fn the_scanner_itself_bites() {
    let src = "fn real() { let x = \"HTTP/1.1 200\"; }\n#[cfg(test)]\nmod tests {\n    fn t() { let y = \"HTTP/1.1 200\"; }\n}\n";
    let body = non_test_source(src);
    assert!(body.contains("HTTP/1."), "non-test line kept");
    assert_eq!(body.matches("HTTP/1.").count(), 1, "test-module body removed");
}
```

Per-crate allowlists, decided after the first run: rtsp-runtime's `io.rs` has no literals left after Tasks 5-6 (`"RTSP/1.0 ` appears only in `#[cfg(test)]`); `client.rs`/`server.rs` build messages through `rtsp_types` builders. rtmp-runtime and dvb-stream have none expected. `tokio::time::sleep(` is intentionally in `NEEDLES`: `sleep_until(deadline)` (the `poll_timeout`-driven waits) does not contain `sleep(`, so only a fixed sleep trips it.

- [ ] **Step 2: Run, expect FAIL where hand-rolling remains, then fix or allowlist with a reason**

```bash
cargo test --locked -p rtsp-runtime -p rtmp-runtime -p dvb-stream --all-features --test no_handroll_guard 2>&1 | grep -E 'test result|FAILED|:[0-9]+: '
```

Expected on the first run: either green or a short hit list. Each hit is fixed in the source (preferred) or added to that crate's `ALLOW` with a one-line reason a reviewer can check. Do not widen a needle.

- [ ] **Step 3: Bite check**

Temporarily add `let _ = "HTTP/1.1 200";` to a non-test fn in each crate's `src`, confirm `no_hand_rolled_protocol_code_in_src` FAILS naming that file:line, remove it.

- [ ] **Step 4: Commit**

```bash
git add rtsp-runtime/tests/no_handroll_guard.rs rtmp-runtime/tests/no_handroll_guard.rs dvb-stream/tests/no_handroll_guard.rs
git commit -m "test: no-hand-roll tripwire guards for rtsp-runtime, rtmp-runtime, dvb-stream"
```

---

### Task 14: Full gate, CHANGELOG, version notes (do not merge)

**Files:**
- Modify: `rtsp-runtime/CHANGELOG.md`, `rtmp-runtime/CHANGELOG.md`, `dvb-stream/CHANGELOG.md` (`## [Unreleased]`)
- Modify: `rtsp-runtime/README.md`, `rtmp-runtime/README.md`, `dvb-stream/README.md` (adapter sections, new API names)
- Modify: `docs/` pages of rtsp-runtime that mention the old read loop (`grep -rn "read_buf\|fill_from_socket" rtsp-runtime/docs rtsp-runtime/README.md`)
- Create: `.delegate/w1-r-low-a-report.md` additions (version notes, evidence)

- [ ] **Step 1: Re-grep the spec §3 sites for this cluster**

```bash
grep -rnE 'has_header_end|complete_request_len|fill_from_socket|pending_write|TsFramer|UdpReader|rtmp://\{|find\("://"\)' rtsp-runtime/src rtmp-runtime/src dvb-stream/src multimux/src/push/rtmp.rs | cut -c1-120
```

Expected: no hit in `rtsp-runtime/src`, `rtmp-runtime/src`, `dvb-stream/src`. The `rtmp://{host}:{port}` hit in `multimux/src/push/rtmp.rs` is W2's (SP6.1 migration to `AsyncRtmpClient`); record it in the report as "handed to W2".

- [ ] **Step 2: CHANGELOG `[Unreleased]` entries (breaking ones marked)**

`rtsp-runtime/CHANGELOG.md`:

```markdown
### Changed
- **BREAKING** The tokio adapter is a `tokio_util::codec::Framed` over the sans-IO core. New `RtspTimeouts` (connect / handshake / read_idle / write; defaults 10 s / 10 s / 30 s / 10 s) and `*_with_timeouts` constructors; `AsyncRtspClient::with_stream` / `AsyncRtspServer::with_stream` keep their signatures and use the defaults. New `Error::Timeout { what }`. `read_idle` bounds the whole frame, so a peer dripping bytes times out.
- **BREAKING** (quadratic re-parse is BOUNDED, not removed: the core still re-parses its inbound buffer per chunk, now at most 64 KiB of head) Header blocks are capped at 64 KiB and request bodies at 2 MiB (was a single 2 MiB buffer cap); the client core rejects an unterminated header over 64 KiB.
- `Transport` header parse/build uses `rtsp_types::headers::Transports`. Parameter ORDER of the serialised header changed (semantically equivalent); parsed values are identical. Examples: <paste the changed lines from Task 4 step 4: `RTP/AVP/UDP;multicast;destination=224.2.0.1;port=3456-3457;ttl=16` -> `RTP/AVP/UDP;multicast;ttl=16;port=3456-3457;destination=224.2.0.1`, ...>.
- `sdp-types` 0.1 -> 0.2 (not in the public API).

### Added
- `ClientSession::{mark_activity, poll_timeout, handle_timeout}`: keepalive deadline (half the Session timeout, default 60 s) driven by the adapter.

### Fixed
- `WWW-Authenticate` `stale=true` was missed when `stale` was the first parameter or the realm contained a comma.
- Slow-loris and never-terminated requests no longer hold a server connection or grow memory (defect 4).
```

`rtmp-runtime/CHANGELOG.md`: BREAKING `RtmpConnection<S = TcpStream>` + `from_stream`, `RtmpTimeouts`, `AsyncRtmpServer::with_timeouts`, `next_events` deadline errors are `io::ErrorKind::TimedOut`, `pending_write` removed (cancel-safety now by `Framed`'s buffer); Added: `AsyncRtmpClient`, `RtmpTarget`/`RtmpUrlError` (`tcUrl` built with `url`, IPv6 bracketed, userinfo/query stripped: defect 8); Fixed: defect 4.

`dvb-stream/CHANGELOG.md`: BREAKING `UdpReader` removed, `SectionStream<UdpReader>`/`T2miEventStream<UdpReader>` replaced by `UdpSectionStream`/`UdpT2miStream` (same `bind_multicast` signatures), `TsFramer` replaced by public `TsDecoder`; Added: `MulticastConfig` (socket2: `SO_RCVBUF`, `SO_REUSEADDR`/`SO_REUSEPORT`, multicast interface), `from_socket` constructors; Changed: `tokio-util`, `bytes`, `socket2` dependencies.

Update each README's adapter section; `rtmp-runtime/README.md` and `lib.rs` docs now say a client adapter exists.

- [ ] **Step 3: Version notes in `.delegate/w1-r-low-a-report.md` (the orchestrator updates `.delegate/release-versions.txt`)**

Write exactly:

```
## Version notes (W1-R-low-a)
rtsp-runtime  BREAKING (adapter API: Framed + RtspTimeouts; header caps) -> 0.x minor bump (0.7.0 -> 0.8.0 relative to the last published 0.7.x)
rtmp-runtime  BREAKING (RtmpConnection<S>, timeouts, new client adapter + url/tokio-util deps) -> 0.x minor bump
dvb-stream    BREAKING (UDP constructors, TsFramer -> TsDecoder, bytes/tokio-util in public API) -> 0.x minor bump
Epoch purity: tokio-util/bytes types appear in the public API of dvb-stream (TsDecoder: Decoder<Item = Bytes>) and in none of rtsp/rtmp's (codecs are pub(crate)); sdp-types 0.2 is not in any rtsp-runtime signature. No workspace-sibling caret epoch changes in this plan.
multimux: no source change needed for these three crates (verified: cargo build -p multimux --all-features --locked); the rtmp push client migration is W2 (SP6.1).
New lock packages: sdp-types 0.2.0 (+ transitive), nothing else beyond edges (tokio-util, futures-util, bytes, url, socket2, http-auth already locked).
```

- [ ] **Step 4a: Rebase onto main and regenerate the lock (merge order is T, RA, P, RB)**

Do this once every earlier branch has merged (RA consumes nothing from T but still rebases):

```bash
git fetch -q origin && git rebase origin/main
git checkout origin/main -- Cargo.lock
CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=fallback cargo update -p sdp-types --precise 0.2.0
cargo build --workspace --all-features --locked 2>&1 | tail -2
git diff origin/main -- Cargo.lock | grep -E '^[-+]name|^[-+]version' | paste - - | sort | uniq
```

Never hand-merge `Cargo.lock`. If `--locked` complains that a package is missing, run the plan's own `cargo update -p ...` (Task 2 lists them) and re-check that only this plan's packages differ from `origin/main`. Re-run Tasks 1's golden tests and the three crates' suites after the rebase.

- [ ] **Step 4b: Confirm the CI thumbv7em cross-builds still build**

None of this plan's crates is in CI's `thumbv7em-none-eabi` list, so it must stay green untouched. Read the exact list and command from `.github/workflows/ci.yml` (the `for c in ...; do cargo build -p "$c" --no-default-features --target thumbv7em-none-eabi --locked` step) and run it:

```bash
rustup target add thumbv7em-none-eabi
LIST=$(python3 - <<'PY'
import re
t = open('.github/workflows/ci.yml').read()
print(re.search(r'for c in ([^;]+); do\s*\n\s*echo "::group::\$c \(no_std\)"', t).group(1))
PY
)
for c in $LIST; do cargo build -p "$c" --no-default-features --target thumbv7em-none-eabi --locked 2>&1 | tail -1 | grep -q Finished || echo "FAIL $c"; done
```

Expected: no `FAIL` line (the python only READS the list; YAML is never rewritten by script).

- [ ] **Step 4: Run the full gate on the worktree**

```bash
/Volumes/External/Projects/rust-broadcast/.delegate/gate-wt.sh "$PWD" 2>&1 | tee /tmp/w1-rlow-a-gate.log | grep -E '^== |^rc=|GATE-DONE|FAIL'
```

The script takes the shared gate lock itself (the orchestrator adds it); just call it. Expected: every `rc=0` (14 steps including the per-crate no-default-features loop), `GATE-DONE`. Also run `tools/check-published-dep-consistency.py` (step 14 of the gate does). Any failure: fix, re-run the whole gate; do not report partial results.

- [ ] **Step 5: Final evidence and hand-off**

Append to the report: the failing-then-passing names per defect (Tasks 3, 5, 6, 7, 8, 9 revert-checks), the 20/20 stability result, the gate's 14/14 summary, and the golden diff summary (`git diff --stat $BASE -- '*.golden'` shows only `transport_header.golden` changed). Commit docs, changelogs and report:

```bash
git add rtsp-runtime rtmp-runtime dvb-stream
git commit -m "docs: W1-R-low-a changelogs, README adapter sections, version notes"
git status --short | head
```

**Do not merge.** Do not push a tag. Do not edit `.delegate/release-versions.txt`. Hand the branch `w1/r-low-a` to the orchestrator for the adversarial review and merge.

---

## Coverage table

Every §3 inventory site and SP item assigned to this half of the R-low cluster, and the task that covers it.

| Spec item | Where | Task |
|---|---|---|
| RTSP `io.rs` `has_header_end`, read loops | rtsp-runtime/src/io.rs | 5, 6 |
| RTSP `io.rs` no timeouts (defect 4) | rtsp-runtime/src/io.rs | 5, 6 |
| RTSP `transport.rs` Transport parse/build | rtsp-runtime/src/transport.rs | 4 |
| RTSP `client.rs` `stale=` scan, Session parse | rtsp-runtime/src/client.rs | 3 |
| RTSP `server.rs` Session id | rtsp-runtime/src/server.rs | 3 |
| SP1.1 rtsp Framed adapters | client + server | 5, 6 |
| SP1.2 rtsp `ClientSession` keepalive `poll_timeout` | client.rs, io.rs | 7 |
| SP1.3 explicit timeout config (rtsp, rtmp) | RtspTimeouts, RtmpTimeouts | 5, 6, 8, 9 |
| SP4 sdp-types 0.2 via rtsp-runtime | Cargo.toml, tests/integration.rs | 2 |
| rtmp-runtime `io.rs` framing, no timeouts (defect 4), `pending_write` deleted | rtmp-runtime/src/io.rs | 8 |
| SP6.1 new rtmp client adapter | AsyncRtmpClient | 9 |
| SP3 rtmp tcUrl, defect 8 (rtmp) | RtmpTarget | 9 |
| dvb-stream `TsFramer` -> `Decoder` | dvb-stream/src/framer.rs | 10 |
| dvb-stream `UdpReader` -> `UdpFramed` | dvb-stream/src/section_stream.rs | 11 |
| SP1.6 socket2 in dvb-stream | dvb-stream/src/udp.rs | 11 |
| SP1.4 tracked tasks / CancellationToken for these crates | none needed: all three crates spawn nothing (rtsp/rtmp adapters are per-connection futures; dvb-stream states "no internal tasks") | n/a, see Escalations 1 |
| SP7 for these crates (sleeps, fixed port) | rtsp io_loopback, rtmp ffmpeg_publish, dvb-stream error_and_datagram | 11, 12 |
| §5 guards | no_handroll_guard.rs x3 | 13 |
| Goldens (§6) | rtsp transcript, rtmp client transcript, dvb-stream events | 1 (compare in 3-11, 14) |
| Versioning (§8) | report | 14 |

Items of the R-low row owned by the sibling plan `2026-10-03-dehandroll-w1-rlow-b.md`: srt-runtime, webrtc-runtime, hls-runtime, SP2.2/2.6, SP3 (hls, webrtc stun), SP4 (webrtc), SP5 (hls), SP6.5, defect 7, and the `Instant::now()` carry from W0.

## Escalations

0. **Owner decisions applied (review 2026-10-03):** Transport parameter-order change accepted (`layers` kept through `others`, round-trip test in Task 4); case-insensitive Transport tokens/parameter names kept by normalising case before the typed parse (test with `RTP/AVP/TCP;Interleaved=0-1`); no sleeps in tests; fixed port 43991 moved to an OS-assigned port in Task 1. **No fuzz targets added** for `TsDecoder`, the RTSP/RTMP codecs and `headers_util` (review G7): the codecs delegate to cores that already have fuzz targets (`rtmp_server_session`, `rtmp_chunk_amf0`), `TsDecoder` and the RTSP codecs have none; a follow-up story should add `ts_decoder` and `rtsp_codec` targets.

1. **SP1.4 for the three crates in this plan is vacuous, not skipped.** `rtsp-runtime`, `rtmp-runtime` and `dvb-stream` own no spawned task: every adapter is a per-connection future driven by the caller, and `dvb-stream`'s docs state "no internal tasks". Nothing here leaks a `JoinHandle` or a bound port, so there is no `TaskTracker`/`CancellationToken` to add. The tracked-task and cancellation work for this cluster is in the sibling plan (srt listener, hls `TokioClient`). Evidence: `grep -rn "tokio::spawn\|task::spawn\|JoinHandle" rtsp-runtime/src rtmp-runtime/src dvb-stream/src` is empty after Task 11 (it is empty on `main` too; the matches are all in `#[cfg(test)]`). Owner action: none unless they want an explicit cancellation parameter on the long-lived `next_events`/`recv_interleaved` futures; callers cancel by dropping or `select!`, which Task 8's cancel-safety test pins.
2. **`Transport` header serialisation order changes** (Task 4). `rtsp-types` has no `layers` field and serialises parameters in its own fixed order, so the byte output of `Transport::to_header_value` differs for multi-parameter specs (semantically identical; `layers` is carried through the typed `others` map). Spec §1 criterion 3 allows this only if each difference is listed in the CHANGELOG with an example, which Task 14 does. If the owner requires byte-identical headers, the alternative is to keep the old hand-written serialiser (a §9 exception) and use the typed header for parsing only.
