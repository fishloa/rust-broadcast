# De-hand-roll W1-R-low-b — srt-runtime, webrtc-runtime, hls-runtime Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** In `srt-runtime`, `webrtc-runtime` and `hls-runtime`: deadline-driven timers (`poll_timeout`) instead of fixed ticks and sleep-polls; tracked tasks that release their UDP port on drop; explicit timeout configs; `Bytes` datagrams with a configurable size; WHIP/WHEP state machines on `http`/`headers` types; SDP fingerprint via `sdp-types` typed attributes; HLS URL resolution with `Url::join` (defect 7), dates with `jiff`, Range with `headers`, retries with `backon`. Carried from W0: `MediaTransport::new` and `StunGather::new` take a caller-supplied `now`. Fixes defect 3 (SRT listener routing pump keeps its UDP port bound after drop) and defect 7 (hls `..`).

**Architecture:** One branch `w1/r-low-b` in worktree `.worktree/w1-r-low-b`. The sans-IO cores keep their APIs except where the spec makes them breaking (webrtc core goes `std` and uses `http` types; hls `TokioClient` config). New timer APIs are additive. Wire/serialised output is pinned by goldens taken from `main` before the first change. This is the second half of the R-low split; `w1-rlow-a` (rtsp, rtmp, dvb-stream) is independent and may run in parallel.

**Tech Stack:** Rust 1.95 workspace, edition 2024, `--locked`. Dependencies: `tokio-util 0.7.19` (features `rt` for `CancellationToken`/`TaskTracker`), `bytes 1`, `url 2.5.8`, `jiff 0.2.37` (MSRV 1.70), `backon 1.6.0` (MSRV 1.85), `headers 0.4.2` + `http 1.5.0`, `sdp-types 0.2.0` (MSRV 1.71), `wait-timeout 0.2.1`, dev-only `axum 0.8.9` (MSRV 1.80). New lock packages: `jiff` (+`jiff-static`? no: static is optional, off), `backon` (+`fastrand`, already locked), `headers`/`headers-core`/`mime`/`httpdate` (and the older `base64 0.22`, `sha1 0.10` that `headers 0.4.2` pins), `sdp-types 0.2.0`, `wait-timeout`, `axum 0.8.9` (+`axum-core 0.5`, other dev deps). Every other lock entry must not move.

**Spec:** `docs/superpowers/specs/2026-10-03-protocol-runtime-dehandroll-design.md` — §3 (webrtc `whep/server.rs`, `whip/server.rs`, `whep/player.rs`, `whip/client.rs`, `media/transport.rs`, hls `client/tokio_client.rs`, `client/action.rs`, `url.rs`, `engine.rs`, SRT listener rows), §4 SP1 (1.2 `poll_timeout`, 1.3 timeouts, 1.4 tasks, 1.5 backon in hls), SP2.2/2.6, SP3 (hls `Url::join` + query, webrtc `stun:`), SP4 (webrtc fingerprint, ICE candidate), SP5 (hls dates), SP6.5, SP7, §5 guards, §7 W1 R-low row, §8 versioning. Defects 3 (srt listener) and 7.

## Global Constraints

Copied from spec §2, plus the owner decisions that apply to this cluster.

- MSRV 1.95.0; committed `Cargo.lock`, always `--locked`. A dependency add or bump may change only the intended lock entries; restore anything else with `cargo update -p <pkg> --precise <old>`.
- Every new or bumped crate supports MSRV 1.95: verified for all 34 bumps (max 1.89) and for the new crates (`tokio-util` 1.71, `socket2` 1.70, `backon` 1.85, `parking_lot` 1.71, `lru` 1.85, `jiff` 1.70, `hex`, `base64`, `arc-swap`, `wait-timeout`).
- No Co-Authored-By or Claude-Session trailers on commits.
- Nothing is tagged or published without the owner's explicit sign-off.
- Epoch purity: if a bumped dependency's types appear in a crate's public API, that crate takes a major-class version change. Each wave records this in `.delegate/release-versions.txt`.

Owner decisions that apply here:

- Q1/Q2: `no_std` may be dropped only in the crates this work touches. Here: **webrtc-runtime core goes `std`** (SP2.2). **srt-runtime and hls-runtime cores stay `no_std`+`alloc`**: CI's `thumbv7em-none-eabi` job builds both (`.github/workflows/ci.yml:145`), nothing in the spec requires dropping it, and every new dependency in their cores (`url`, `jiff`) supports `alloc`-only builds. New IO dependencies sit behind each crate's existing `tokio` feature.
- SP1.2: each timer-bearing core exposes `fn poll_timeout(&self) -> Option<Instant>` (srt connection/listener, webrtc `MediaTransport`, hls `HlsClient`); adapters `select!` over inbound frame, outbound queue, `sleep_until(deadline)` and `cancel.cancelled()`; fixed ticks and sleep-polls are removed (SRT 2 ms tick, hls 10 ms defensive sleep). Deviations forced by upstream APIs are listed under Escalations.
- SP1.3: every adapter takes a config struct with `connect`, `handshake`, `read_idle`, `write` (`Duration`, documented defaults).
- SP1.4: every spawn is owned by a `JoinSet`/`TaskTracker` (or a `JoinHandle` that is aborted on drop); shutdown uses `CancellationToken` only.
- SP1.5: `backon` with jitter in hls-runtime `TokioClient`.
- SP3: URL references with no absolute base use option (a): a synthetic base with that base stripped from the result.
- SP4: fmtp stays codec logic (none in this crate). The Link header (`webrtc-runtime/src/ice.rs`) is a documented exception (§9): its parser/builder stay, with RFC 8288 + RFC 9725 vectors.
- SP5: dates use `jiff`; the existing optional `chrono` features are untouched.
- Take all dependency bumps.

Plan-specific rules:

- Never run two cargo commands concurrently. Another wave may be running in its own worktree with its own `target/`; do not share a target dir.
- When asserting the exit status of a cargo command through the rtk hook, run it as `rtk proxy cargo ...` or read the printed `test result:` lines (memory: "cargo exit codes lie via rtk").
- New public enums must be added to the crate's `tests/label_coverage.rs` SKIP list (with a reason) or get `name()` + `impl_spec_display!`; they must be `#[non_exhaustive]` (`tests/non_exhaustive_coverage.rs`). Prefer `#[non_exhaustive]` structs with `with_*` builders.
- Tests of timers use `#[tokio::test(start_paused = true)]` over `tokio::io::duplex` or pure sans-IO calls; never paused time plus real sockets (auto-advance races real IO). Real-socket tests wait on the real condition under a bounding `timeout`, never a fixed sleep.
- Code blocks were written against the registry sources of the versions in `Cargo.lock` and the crates fetched for this plan (`headers 0.4.2`, `sdp-types 0.2.0`, `backon 1.6.0`, `jiff 0.2.37`, `axum 0.8.9`, `wait-timeout 0.2.1`). They were not compiled when the plan was written; the "expected FAIL / PASS" steps are the compile check. If a signature differs, fix the call site and note it in the report; never weaken a test to make it pass.

## Review Focus

The five inputs most likely to bite users that no existing test covers. Each has a test in the named task.

1. **Dropping an SRT listener while connections are live, and while none are.** The listener handle going away must free the UDP port when nothing else uses it, and must NOT kill connections it already accepted. Owned by Task 5, step 1.
2. **An SRT datagram larger than the configured maximum.** UDP truncates silently; a truncated DATA packet used to be parsed as a short valid packet. It must be dropped and counted, never delivered. Owned by Task 3, step 1.
3. **HLS relative references.** `../seg.m4s`, `./a/../b.m4s`, `//cdn/x`, `?query-only`, a reference containing `://` inside its query, a base with a query and fragment, and a base that is itself relative. Owned by Task 11, step 1.
4. **WHIP/WHEP header strictness.** `If-Match: *`, `If-Match: "*"`, an unquoted ETag, a weak ETag, `Content-Type: application/sdp; charset=utf-8`, a header sent twice, and a non-UTF-8 header value. Owned by Task 9, step 1.
5. **A caller-supplied clock in webrtc.** A `MediaTransport` built at a `now` far in the past must schedule from that `now`, never from the wall clock; and the SDP fingerprint of a bundled offer must come from the media section, with typed-attribute edge cases (`SHA-256` upper case, colon-less digest, session-level only). Owned by Task 7, step 1 and Task 8, step 1.

---

### Task 0: Worktree setup and baseline

**Files:** none (environment only), plus `.delegate/w1-r-low-b-report.md` (created).

- [ ] **Step 1: Create the worktree off main**

```bash
cd /Volumes/External/Projects/rust-broadcast
git fetch -q origin
git worktree add -b w1/r-low-b .worktree/w1-r-low-b origin/main
cd .worktree/w1-r-low-b
git -c protocol.file.allow=always submodule update -q --init --reference /Volumes/External/Projects/rust-broadcast/private private
ln -s /Volumes/External/Projects/rust-broadcast/.test-streams .test-streams
ln -s /Volumes/External/Projects/rust-broadcast/multimux/tests/assets/node_modules multimux/tests/assets/node_modules
rustup target add thumbv7em-none-eabi
git rev-parse HEAD
```

Expected: a commit hash. Record it as `BASE` in `.delegate/w1-r-low-b-report.md` (header `# W1-R-low-b report`, line `BASE=<hash>`). Revert-checks restore files from `$BASE`.

- [ ] **Step 2: Baseline. The three crates must pass BEFORE any change**

```bash
timeout 2400 cargo test --locked --all-features -p srt-runtime -p webrtc-runtime -p hls-runtime 2>&1 | grep -E '^test result|FAILED|panicked|^error' | sort | uniq -c
for c in srt-runtime webrtc-runtime hls-runtime; do cargo build --locked -p $c --no-default-features 2>&1 | tail -1; done
for c in srt-runtime webrtc-runtime hls-runtime; do cargo build --locked -p $c --no-default-features --target thumbv7em-none-eabi 2>&1 | tail -1; done
```

Expected: only `test result: ok` lines (the libsrt interop tests skip when `srt-live-transmit` is absent and print why; run on a machine that has libsrt so they actually execute, and record which ran). Six `Finished` lines (the thumb builds of all three crates pass today: that is the bar Task 16 re-checks). Record passed counts and which libsrt tests ran under "baseline".

- [ ] **Step 3: Record what multimux consumes (the compile-fix surface)**

```bash
grep -rnE 'srt_runtime::|webrtc_runtime::|hls_runtime::' multimux/src multimux/tests fuzz/fuzz_targets | cut -c1-130 > /tmp/w1-rlow-b-consumers.txt; wc -l /tmp/w1-rlow-b-consumers.txt
```

Expected ~75 lines. The signature changes in this plan that reach multimux: `SrtSocket::recv` returns `Bytes` (Task 3: two call sites), `MediaTransport::new` takes `now` (Task 7: five call sites). Everything else (`HlsClient`, `Action`, `HlsOrigin`, `TokioClient::{new,with_config,next_output}`, `webrtc_runtime::whep::{content_type,status}`) keeps its signature.

---

### Task 1: Goldens from main (WHIP/WHEP HTTP output, HLS client output)

**Files:**
- Create: `webrtc-runtime/tests/golden_http.rs`, `webrtc-runtime/tests/golden/whip_whep_http.golden`, `webrtc-runtime/tests/golden/README.md`
- Create: `hls-runtime/tests/golden_client.rs`, `hls-runtime/tests/golden/client_urls.golden`, `hls-runtime/tests/golden/README.md`
- No SRT golden: this plan changes no SRT packet bytes (`git diff $BASE --stat -- srt-runtime/src/packet srt-runtime/src/handshake_sm.rs srt-runtime/src/arq/*.rs` must show only the additive `next_timeout` methods of Task 4; the libsrt interop suites are the oracle for the wire).

**Interfaces:** none new. Each test compares to its golden; `GOLDEN_UPDATE=1` rewrites it. Task 9 re-targets the webrtc test to the typed API and compares to the SAME file; Task 11 compares the hls golden and regenerates it once, for exactly the three reviewed fragment rows described there.

- [ ] **Step 1: Write the webrtc golden test against the CURRENT API**

`webrtc-runtime/tests/golden_http.rs`:

```rust
//! Golden of every HTTP request/response the WHIP/WHEP state machines emit
//! (W1-R-low-b, spec §6). Rendered as `status`/`METHOD url`, then sorted
//! lower-cased `name: value` header lines (the old API's separate `content_type`
//! is rendered as a `content-type` header), then the body length. Task 9 renders
//! the typed `HeaderMap` the same way and must match byte-for-byte except for the
//! differences it lists in the CHANGELOG.

use webrtc_runtime::whep::{player::WhepPlayer, server::WhepSession};
use webrtc_runtime::whip::{client::WhipClient, server::WhipSession};

const URL: &str = "https://origin.example/whip/live";
const SESSION: &str = "https://origin.example/whip/live/s1";

fn render_headers(ct: Option<&str>, headers: &[(String, String)]) -> String {
    let mut lines: Vec<String> = headers
        .iter()
        .map(|(k, v)| format!("{}: {v}", k.to_ascii_lowercase()))
        .collect();
    if let Some(ct) = ct {
        lines.push(format!("content-type: {ct}"));
    }
    lines.sort();
    lines.iter().map(|l| format!("  {l}\n")).collect()
}

macro_rules! resp {
    ($out:expr, $label:expr, $r:expr) => {{
        let r = $r;
        $out.push_str(&format!(
            "{} -> {}\n{}  body={}B\n",
            $label,
            r.status,
            render_headers(r.content_type.map(|c| c), &r.headers),
            r.body.len()
        ));
    }};
}

macro_rules! req {
    ($out:expr, $label:expr, $r:expr) => {{
        let r = $r;
        $out.push_str(&format!(
            "{} -> {} {}\n{}  body={}B\n",
            $label,
            r.method.name(),
            r.url,
            render_headers(r.content_type.map(|c| c.to_string()).as_deref(), &r.headers),
            r.body.len()
        ));
    }};
}

#[test]
fn whip_whep_http_output_is_byte_identical_to_golden() {
    let mut out = String::new();

    // WHIP server
    let mut s = WhipSession::new(SESSION.into());
    resp!(out, "whip accept", s.accept(b"sdp".to_vec(), "etag1".into()));
    resp!(out, "whip ack_trickle", s.ack_trickle());
    resp!(out, "whip ack_restart", s.ack_restart(b"frag".to_vec(), "etag2".into()));
    resp!(out, "whip ack_delete", s.ack_delete());

    // WHEP server
    let mut s = WhepSession::new(SESSION.into());
    resp!(out, "whep accept", s.accept(b"sdp".to_vec(), "e1".into()));
    let mut s = WhepSession::new(SESSION.into());
    resp!(out, "whep counter_offer(None)", s.counter_offer(b"o".to_vec(), None));
    let mut s = WhepSession::new(SESSION.into());
    resp!(out, "whep counter_offer(valid-until)", s.counter_offer(b"o".to_vec(), Some("2030-01-01T00:00:00Z".into())));
    resp!(out, "whep ack_answer", s.ack_answer("e2".into()));
    resp!(out, "whep ack_trickle", s.ack_trickle());
    resp!(out, "whep ack_restart", s.ack_restart(b"frag".to_vec(), "e3".into()));
    resp!(out, "whep no_publisher(Some(5))", WhepSession::no_publisher(Some(5)));
    resp!(out, "whep no_publisher(None)", WhepSession::no_publisher(None));
    resp!(out, "whep ack_delete", s.ack_delete());

    // WHIP client, with and without a bearer token
    for token in [None, Some("tok3n".to_string())] {
        let tag = if token.is_some() { "bearer" } else { "anon" };
        let mut c = WhipClient::new(URL.into(), token);
        req!(out, &format!("whip client {tag} offer"), c.offer(b"o".to_vec()).unwrap());
        c.on_response(webrtc_runtime::whip::client::HttpResponse {
            status: 201,
            content_type: Some("application/sdp".into()),
            location: Some(SESSION.into()),
            etag: Some("etag1".into()),
            body: b"a".to_vec(),
        })
        .unwrap();
        req!(out, &format!("whip client {tag} flush_candidates"), c.flush_candidates(b"f".to_vec()).unwrap());
        c.on_response(webrtc_runtime::whip::client::HttpResponse { status: 204, content_type: None, location: None, etag: None, body: vec![] }).unwrap();
        req!(out, &format!("whip client {tag} ice_restart"), c.ice_restart(b"f".to_vec()).unwrap());
        c.on_response(webrtc_runtime::whip::client::HttpResponse { status: 200, content_type: None, location: None, etag: Some("etag9".into()), body: b"x".to_vec() }).unwrap();
        req!(out, &format!("whip client {tag} terminate"), c.terminate().unwrap());
    }

    // WHEP player
    let mut p = WhepPlayer::new(URL.into(), Some("tok3n".into()));
    req!(out, "whep player offer", p.offer(b"o".to_vec()).unwrap());
    p.on_response(webrtc_runtime::whep::player::HttpResponse {
        status: 201,
        content_type: Some("application/sdp".into()),
        location: Some(SESSION.into()),
        etag: Some("e1".into()),
        body: b"a".to_vec(),
    })
    .unwrap();
    req!(out, "whep player trickle_ice", p.trickle_ice(b"f".to_vec()).unwrap());
    p.on_response(webrtc_runtime::whep::player::HttpResponse { status: 204, content_type: None, location: None, etag: None, body: vec![] }).unwrap();
    req!(out, "whep player terminate", p.terminate().unwrap());

    let path = format!("{}/tests/golden/whip_whep_http.golden", env!("CARGO_MANIFEST_DIR"));
    if std::env::var_os("GOLDEN_UPDATE").is_some() {
        std::fs::write(&path, &out).unwrap();
        return;
    }
    assert_eq!(out, std::fs::read_to_string(&path).expect("golden"), "http golden differs");
}
```

(The macros take the response/request by value and read `r.content_type`; for the WHEP server `no_publisher` is an associated fn, not a method: that is why it is called as `WhepSession::no_publisher(..)`. If the compiler objects to the `r.content_type.map(|c| c)` shape for a `Option<&'static str>`, replace it with `r.content_type` directly; the two macros differ only because client requests carry `Option<&'static str>` and the render takes `Option<&str>`.)

- [ ] **Step 2: Write the hls golden test against the CURRENT API**

`hls-runtime/tests/golden_client.rs`:

```rust
//! Golden of the HLS client's request URLs and actions (W1-R-low-b). Every row
//! here has no `..` segment, so the Task 11 `Url::join` rewrite must reproduce
//! it byte-for-byte except the three `#f` matrix rows Task 11 reviews (the old
//! builder appended a query pair after a fragment); the `..` rows (the defect)
//! are asserted in Task 11's unit tests, not in this golden.

use hls_runtime::client::{Action, BlockingReload, HlsClient};

const LIVE: &str = "#EXTM3U\n#EXT-X-VERSION:6\n#EXT-X-TARGETDURATION:2\n\
#EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES,CAN-SKIP-UNTIL=12.0\n\
#EXT-X-PART-INF:PART-TARGET=0.5\n#EXT-X-MEDIA-SEQUENCE:7\n\
#EXT-X-MAP:URI=\"init.mp4\"\n\
#EXTINF:2.0,\nseg7.m4s\n#EXTINF:2.0,\n/abs/seg8.m4s\n#EXTINF:2.0,\nhttps://cdn.example/seg9.m4s\n\
#EXTINF:2.0,\n//cdn2.example/seg10.m4s\n#EXTINF:2.0,\nsub/seg11.m4s?token=a=b&x=y\n";

#[test]
fn client_actions_and_request_urls_match_golden() {
    let mut out = String::new();
    for base in [
        "http://h.example/live/stream.m3u8",
        "http://h.example:8080/live/stream.m3u8?_HLS_msn=3&k=v",
        "https://h.example/a/b/stream.m3u8#frag",
    ] {
        let mut c = HlsClient::new(base);
        out.push_str(&format!("base {base}\n"));
        out.push_str(&format!("  first {:?}\n", c.poll()));
        c.on_playlist(LIVE.as_bytes()).unwrap();
        while let Some(a) = c.poll() {
            out.push_str(&format!("  {a:?}\n"));
            if let Some(u) = a.playlist_request_url() {
                out.push_str(&format!("    request_url {u}\n"));
            }
        }
    }
    // request URL matrix (blocking x skip), independent of the engine
    for (blocking, skip) in [
        (None, false),
        (Some(BlockingReload { msn: 5, part: None }), false),
        (Some(BlockingReload { msn: 5, part: Some(2) }), true),
        (None, true),
    ] {
        for url in ["http://h/p.m3u8", "http://h/p.m3u8?a=1", "http://h/p.m3u8?a=1&b=2#f"] {
            let a = Action::FetchPlaylist { url: url.into(), blocking, skip };
            out.push_str(&format!("matrix {url} blocking={blocking:?} skip={skip} -> {:?}\n", a.playlist_request_url()));
        }
    }
    let path = format!("{}/tests/golden/client_urls.golden", env!("CARGO_MANIFEST_DIR"));
    if std::env::var_os("GOLDEN_UPDATE").is_some() {
        std::fs::write(&path, &out).unwrap();
        return;
    }
    assert_eq!(out, std::fs::read_to_string(&path).expect("golden"), "client golden differs");
}
```

- [ ] **Step 3: Generate on UNCHANGED main code, verify they compare, commit**

```bash
mkdir -p webrtc-runtime/tests/golden hls-runtime/tests/golden
GOLDEN_UPDATE=1 cargo test --locked -p webrtc-runtime --all-features --test golden_http 2>&1 | grep -E '^test result|FAILED|^error'
GOLDEN_UPDATE=1 cargo test --locked -p hls-runtime --all-features --test golden_client 2>&1 | grep -E '^test result|FAILED|^error'
cargo test --locked -p webrtc-runtime -p hls-runtime --all-features --test golden_http --test golden_client 2>&1 | grep -E '^test result|FAILED|^error'
wc -c webrtc-runtime/tests/golden/*.golden hls-runtime/tests/golden/*.golden
```

Expected: ok everywhere; `whip_whep_http.golden` over 2 KB; `client_urls.golden` over 1.5 KB. Write both `README.md` files (generated at `<BASE>` with the exact command above; compared byte-for-byte by the same test), then:

```bash
git add webrtc-runtime/tests/golden_http.rs webrtc-runtime/tests/golden hls-runtime/tests/golden_client.rs hls-runtime/tests/golden
git commit -m "test(webrtc,hls): byte-for-byte goldens from main for the W1 http and url rewrites"
```

---

### Task 2: Dependencies

**Files:**
- Modify: `srt-runtime/Cargo.toml` lines 22-60
- Modify: `webrtc-runtime/Cargo.toml` lines 14-60
- Modify: `hls-runtime/Cargo.toml` lines 4-110
- Modify: `Cargo.lock`

**Interfaces:** the dependency edges later tasks use. Public-API epoch: `bytes::Bytes` appears in `srt_runtime::io::SrtSocket::recv` (Task 3); `http`/`headers` types appear in webrtc-runtime's whip/whep modules (Task 9); `tokio_util::sync::CancellationToken` appears in `hls_runtime::client::TokioClientConfig` (Task 13). Each of those crates takes a major-class bump, recorded in Task 16.

- [ ] **Step 1: Write the failing check**

```bash
grep -nE 'tokio-util|jiff|backon|headers|^url|wait-timeout|sdp-types|axum' srt-runtime/Cargo.toml webrtc-runtime/Cargo.toml hls-runtime/Cargo.toml
```

Expected: no output.

- [ ] **Step 2: Edit the manifests**

`srt-runtime/Cargo.toml`:

```toml
bytes      = { version = "1", optional = true, default-features = false }
tokio-util = { version = "0.7", optional = true, default-features = false, features = ["rt"] }
...
tokio = ["dep:tokio", "dep:getrandom", "dep:bytes", "dep:tokio-util", "std"]
```

`webrtc-runtime/Cargo.toml` (the core goes std, so these are not optional):

```toml
http        = "1"
headers     = "0.4"
url         = "2"
sdp-types   = "0.2"
...
[dev-dependencies]
axum        = "0.8"
```

(`url` is used by Task 8's `stun:` host formatting, `sdp-types` by Task 8's fingerprint; `axum` by Task 10's example. `sdp-types` and `url` are only needed with the `media` feature, so make them `optional = true` and add `"dep:sdp-types", "dep:url"` to the `media` feature list; `http`/`headers` are unconditional.)

`hls-runtime/Cargo.toml`:

```toml
url          = { version = "2", default-features = false }
jiff         = { version = "0.2", default-features = false, features = ["alloc"] }
backon       = { version = "1", optional = true, default-features = false, features = ["std"] }
headers      = { version = "0.4", optional = true }
tokio-util   = { version = "0.7", optional = true, default-features = false, features = ["rt"] }
...
std   = [..., "url/std", "jiff/std"]
tokio = ["std", "dep:tokio", "dep:reqwest", "dep:broadcast-auth", "dep:backon", "dep:headers", "dep:tokio-util"]
...
[dev-dependencies]
wait-timeout = "0.2"
axum         = "0.8"
tokio        = { version = "1", features = ["net", "io-util", "rt", "macros", "time", "test-util"] }
```

`http 1.5.0` is already in the lock and is re-exported by `headers` (`headers::HeaderMap`), so hls does not list `http` separately.

- [ ] **Step 3: Update the lock; verify only intended entries changed**

```bash
CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=fallback cargo update -p headers -p jiff -p backon -p sdp-types -p wait-timeout -p axum -p tokio-util 2>&1 | tail -4
git diff Cargo.lock | grep -E '^[-+]name|^[-+]version' | paste - - | sort | uniq
```

Expected additions only: `jiff 0.2.x`, `backon 1.6.x`, `headers 0.4.x`, `headers-core 0.3.x`, `mime 0.3.x`, `httpdate 1.x`, `base64 0.22.x` and `sha1 0.10.x` (second versions pulled by `headers`), `sdp-types 0.2.0` (alongside multimux/transmux's 0.1.8), `wait-timeout 0.2.x`, `axum 0.8.x` + `axum-core 0.5.x` (+ whatever it needs: `matchit 0.8`, `serde_path_to_error`, ...), plus the new dependency edges. Any CHANGED existing version: restore it with `cargo update -p <pkg> --precise <old>`, and record the cause. Check `tokio-util` stays 0.7.19: `grep -A1 'name = "tokio-util"' Cargo.lock`.

- [ ] **Step 4: Build and run the unchanged suites**

```bash
cargo build --locked --workspace --all-features 2>&1 | tail -2
cargo test --locked -p srt-runtime -p webrtc-runtime -p hls-runtime --all-features 2>&1 | grep -E '^test result|FAILED|panicked|^error' | sort | uniq -c
for c in srt-runtime webrtc-runtime hls-runtime; do cargo build --locked -p $c --no-default-features --target thumbv7em-none-eabi 2>&1 | tail -1; done
```

Expected: workspace builds; same pass counts as Task 0 plus the new goldens; thumb builds still pass for srt-runtime and hls-runtime (webrtc-runtime is made std in Task 9; until then it still builds). If the hls thumb build fails because of `url` or `jiff`, stop: that is Escalation 4, not something to work around by dropping `no_std`.

- [ ] **Step 5: Commit**

```bash
git add srt-runtime/Cargo.toml webrtc-runtime/Cargo.toml hls-runtime/Cargo.toml Cargo.lock
git commit -m "chore(deps): tokio-util/bytes (srt), http/headers/sdp-types/url (webrtc), url/jiff/backon/headers (hls), wait-timeout/axum dev"
```

---

### Task 3: SRT `IoConfig`, configurable max datagram, `Bytes` datagrams (SP6.5, SP1.3)

Replaces the `MAX_DATAGRAM = 1500` constant and the per-datagram `to_vec()` copies (io.rs 93, 1272-1288, 1895-1935) with a configurable maximum, `BytesMut::split().freeze()` datagrams, and oversize detection. Adds the timeout config struct used by Task 6.

**Files:**
- Modify: `srt-runtime/src/io.rs`: constants (93, 175-246), `RouteEntry` (103-110), `ConnParams.rx` (302-325), `SrtSocket` fields and `send`/`recv` (337-600), `Driver.rx`/`staged`/`deliver`/`ingress`/`release` (661-1096), `spawn_dedicated_socket_forwarder` (1265-1295), `spawn_listener_routing_pump` (1888-1950), `connect_from` (384-540), `SrtListener::bind` (1466-1494); tests in `adapter_tests` at 2255-2860 that use `Vec<u8>` channels and the pointer-identity test at 2673-2695
- Modify: `srt-runtime/src/io.rs` `SocketStats` + `Counters` (262-296): add `rx_oversize`
- Modify: `multimux/src/source/srt.rs` lines 291, 320, 365; `multimux/src/push/srt.rs` line 366
- Test: unit tests in `io.rs` `adapter_tests`; `srt-runtime/tests/io_config.rs` (new)

**Interfaces:**

```rust
// io.rs (feature "tokio")
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct IoConfig {
    /// Largest datagram accepted from the network (bytes). Default 1500. A larger datagram is
    /// dropped and counted in `SocketStats::rx_oversize` (UDP would otherwise truncate it silently).
    pub max_datagram: usize,
    /// Resolve + bind. Default 10 s.              (used from Task 6)
    pub connect: Duration,
    /// Whole caller handshake. Default 5 s (was the HANDSHAKE_TIMEOUT const).   (Task 6)
    pub handshake: Duration,
    /// Longest silence from a connected peer. Default 5 s (was PEER_IDLE_TIMEOUT).   (Task 6)
    pub read_idle: Duration,
    /// Longest a `send_to` may take. Default 5 s.   (Task 6)
    pub write: Duration,
}
impl Default for IoConfig { /* the defaults above */ }
impl IoConfig { pub fn with_max_datagram(self, n: usize) -> Self; with_connect; with_handshake; with_read_idle; with_write }
/// Smallest accepted `max_datagram`: a full SRT header (16 bytes) plus room for a control CIF.
pub const MIN_MAX_DATAGRAM: usize = 64;

impl SrtSocket {
    pub async fn connect<A>(remote: A, config: HandshakeConfig) -> Result<Self>;                   // unchanged, IoConfig::default()
    pub async fn connect_with<A>(remote: A, config: HandshakeConfig, io: IoConfig) -> Result<Self>;
    pub async fn connect_from<A>(local: SocketAddr, remote: A, config: HandshakeConfig) -> Result<Self>;  // unchanged
    pub async fn connect_from_with<A>(local: SocketAddr, remote: A, config: HandshakeConfig, io: IoConfig) -> Result<Self>;
    pub async fn send(&mut self, payload: &[u8]) -> Result<()>;            // unchanged signature (one copy into a Bytes)
    pub async fn send_bytes(&mut self, payload: bytes::Bytes) -> Result<()>;   // NEW, no copy
    pub async fn recv(&mut self) -> Result<Option<bytes::Bytes>>;          // CHANGED: was Option<Vec<u8>>
}
impl SrtListener {
    pub async fn bind<A>(addr: A, config: HandshakeConfig) -> Result<Self>;                        // unchanged
    pub async fn bind_with<A>(addr: A, config: HandshakeConfig, io: IoConfig) -> Result<Self>;
}
pub struct SocketStats { /* existing */ pub rx_oversize: u64 }       // #[non_exhaustive] already
```

`max_datagram` below `MIN_MAX_DATAGRAM` is clamped up to it (documented), not an error: the value is a tuning knob, and a clamp cannot fail a connect.

- [ ] **Step 1: Write the failing tests**

In `io.rs` `adapter_tests`:

```rust
    /// Review-focus 2. UDP truncates a datagram larger than the receive buffer
    /// without telling the reader; a truncated DATA packet then parses as a
    /// shorter, valid packet and its payload is delivered corrupted. The
    /// forwarder must read `max + 1` bytes, drop anything longer than `max`,
    /// and count it.
    #[tokio::test]
    async fn an_oversize_datagram_is_dropped_and_counted_never_truncated() {
        let local = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let stats = Arc::new(Counters::default());
        let (mut rx, _guard) = spawn_dedicated_socket_forwarder(
            Arc::clone(&local),
            peer.local_addr().unwrap(),
            Arc::clone(&stats),
            300,
        );
        let local_addr = local.local_addr().unwrap();
        peer.send_to(&[1u8; 250], local_addr).await.unwrap();
        peer.send_to(&[2u8; 400], local_addr).await.unwrap(); // > 300
        peer.send_to(&[3u8; 300], local_addr).await.unwrap(); // exactly max: accepted
        let a = rx.recv().await.unwrap();
        let b = rx.recv().await.unwrap();
        assert_eq!((a.len(), a[0]), (250, 1));
        assert_eq!((b.len(), b[0]), (300, 3), "the 400-byte datagram must not appear, truncated or not");
        assert_eq!(stats.rx_oversize.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn io_config_clamps_a_tiny_max_datagram() {
        assert_eq!(IoConfig::default().with_max_datagram(1).max_datagram, MIN_MAX_DATAGRAM);
        assert_eq!(IoConfig::default().max_datagram, 1500);
    }
```

Change the existing pointer-identity test to the `Bytes` shape (it must still prove zero copy):

```rust
    #[tokio::test(start_paused = true)]
    async fn delivery_reuses_the_received_datagrams_allocation() {
        let (mut driver, mut rx) = test_driver(8192).await;
        let datagram = bytes::Bytes::from(data_bytes(0, 0, b"zero-copy payload"));
        let allocation = datagram.as_ptr() as usize;
        driver.ingress(datagram).expect("ingress");
        tokio::time::advance(Duration::from_millis(130)).await;
        driver.tick_engines();
        let mut delivered = drain_deliveries(&mut rx);
        assert_eq!(delivered.len(), 1);
        let payload = delivered.remove(0);
        assert_eq!(&payload[..], b"zero-copy payload");
        assert_eq!(payload.as_ptr() as usize, allocation + SRT_HEADER_LEN,
            "the payload must be a view into the received datagram, not a copy");
    }
```

`srt-runtime/tests/io_config.rs`:

```rust
#![cfg(feature = "tokio")]

use std::time::Duration;

use srt_runtime::HandshakeConfig;
use srt_runtime::io::{IoConfig, SrtListener, SrtSocket};

/// A non-default `max_datagram` is honoured end to end, and `recv` hands out `Bytes`.
#[tokio::test]
async fn a_custom_max_datagram_connection_round_trips_a_payload() {
    let io = IoConfig::default().with_max_datagram(9000); // jumbo-frame path
    let mut listener = SrtListener::bind_with("127.0.0.1:0", HandshakeConfig::default(), io).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let accept = tokio::spawn(async move { listener.accept().await.map(|s| (listener, s)) });
    let mut caller = SrtSocket::connect_with(addr, HandshakeConfig::default(), io).await.unwrap();
    let (_listener, mut accepted) = tokio::time::timeout(Duration::from_secs(10), accept).await.unwrap().unwrap().unwrap();
    caller.send(b"hello over a custom datagram size").await.unwrap();
    let got = tokio::time::timeout(Duration::from_secs(10), accepted.recv()).await.unwrap().unwrap().unwrap();
    let got: bytes::Bytes = got;
    assert_eq!(&got[..], b"hello over a custom datagram size");
    assert_eq!(accepted.stats().rx_oversize, 0);
}
```

- [ ] **Step 2: Run, expect FAIL**

```bash
cargo test --locked -p srt-runtime --all-features --lib an_oversize 2>&1 | grep -E '^error|cannot find|test result' | head -5
cargo test --locked -p srt-runtime --all-features --test io_config 2>&1 | grep -E '^error|cannot find|test result' | head -5
```

Expected: compile errors (`IoConfig`, `rx_oversize`, `spawn_dedicated_socket_forwarder` arity).

- [ ] **Step 3: Implement**

Add `bytes = { ..., optional }` use, `use bytes::{Bytes, BytesMut};`, and:

```rust
pub const MIN_MAX_DATAGRAM: usize = 64;
const DEFAULT_MAX_DATAGRAM: usize = 1500;
/// Capacity of the shared receive chunk: datagrams are carved out of it with `split().freeze()`,
/// so one allocation serves many datagrams until the chunk is exhausted.
const RX_CHUNK: usize = 64 * 1024;

impl IoConfig {
    pub fn with_max_datagram(mut self, n: usize) -> Self {
        self.max_datagram = n.max(MIN_MAX_DATAGRAM);
        self
    }
    // with_connect / with_handshake / with_read_idle / with_write: plain setters
}
```

`Counters` gets `rx_oversize: AtomicU64`; `SocketStats` gets `pub rx_oversize: u64` (doc: "datagrams larger than `IoConfig::max_datagram`, dropped rather than truncated"); `SrtSocket::stats` fills it.

A shared receive helper used by the forwarder, the listener pump and the caller handshake loop:

```rust
/// Reads one datagram into `buf` (a rolling chunk), returning it as `Bytes`, or `None` for an
/// oversize datagram (which has been counted and dropped).
async fn recv_datagram(
    udp: &UdpSocket,
    buf: &mut BytesMut,
    max: usize,
    oversize: &AtomicU64,
) -> std::io::Result<Option<(Bytes, std::net::SocketAddr)>> {
    buf.reserve(max + 1); // +1: a datagram of max+1 bytes proves "larger than max"
    let (n, src) = udp.recv_buf_from(buf).await?;
    let datagram = buf.split_to(buf.len()).freeze();   // the n bytes just read; capacity stays in `buf`
    debug_assert_eq!(datagram.len(), n);
    if n > max {
        oversize.fetch_add(1, Ordering::Relaxed);
        return Ok(None);
    }
    Ok(Some((datagram, src)))
}
```

(`tokio::net::UdpSocket::recv_buf_from(&self, buf: &mut B: BufMut)` reads into the spare capacity of `buf`; verified in `tokio-1.53.2/src/net/udp.rs`. `BytesMut::reserve(max + 1)` guarantees at least that much spare capacity and reuses the chunk's allocation when every earlier `Bytes` split from it has been dropped, otherwise allocates a fresh chunk: initialise `BytesMut::with_capacity(RX_CHUNK)` so a fresh chunk is carved ~40 times per allocation.)

Then:

- `spawn_dedicated_socket_forwarder(udp, peer, stats, max_datagram)` returns `(mpsc::Receiver<Bytes>, JoinHandle)`; its loop calls `recv_datagram`, ignores `Ok(None)` (already counted), forwards only `src == peer`, `try_send(bytes)` as before.
- `spawn_listener_routing_pump(udp, routes, unrouted_tx: Sender<(SocketAddr, Bytes)>, unrouted_dropped, max_datagram)`: oversize datagrams count into `unrouted_dropped`; `entry.tx.try_send(bytes)` where `bytes` is the `Bytes` (clone is a refcount bump, so a routed datagram is no longer copied).
- `ConnParams.rx: mpsc::Receiver<Bytes>`, `RouteEntry.tx: mpsc::Sender<Bytes>`, `Driver.rx`, `Driver::ingress(&mut self, datagram: Bytes)`, `StagedDatagram = Bytes`, `staged: BTreeMap<u32, Bytes>`, `deliver: mpsc::Sender<Bytes>`, `SrtSocket.from_driver: mpsc::Receiver<Bytes>`, `SrtSocket::recv -> Result<Option<Bytes>>`.
- `release`: replace `datagram.drain(..SRT_HEADER_LEN.min(datagram.len()))` with `let payload = datagram.slice(SRT_HEADER_LEN.min(datagram.len())..);`.
- `to_driver: mpsc::Sender<Bytes>`; `send(&mut self, payload: &[u8])` does `self.send_bytes(Bytes::copy_from_slice(payload)).await`; `send_bytes` is the old body with `payload`. `Driver::send_one(&mut self, payload: &[u8])` stays (it serialises into the packet it builds).
- `connect_from_with` allocates `let mut chunk = BytesMut::with_capacity(RX_CHUNK);` for the handshake loop and calls `recv_datagram` in place of `socket.recv_from(&mut buf)`; `peer_handshake(&buf[..len])` becomes `peer_handshake(&bytes)` where `bytes` is the datagram that completed the handshake (keep a clone of it in the `Event::Datagram` variant).
- `connect`/`connect_from`/`bind` delegate to the `_with` forms with `IoConfig::default()`; `ConnParams` and `SrtListener` carry `io: IoConfig`.

Update `adapter_tests` helpers: `test_driver*` use `mpsc::Receiver<Bytes>`; `data_bytes(..)` stays `Vec<u8>` and tests wrap with `Bytes::from(..)` at the `ingress` call; `drain_deliveries` returns `Vec<Bytes>`. In the other `tests/*.rs` files `recv()` results compare against byte strings: `assert_eq!(&got[..], b"...")` or `got == &b"..."[..]` (mechanical; `grep -n 'recv()' srt-runtime/tests/*.rs` lists them: io_concurrent_listener, io_handshake, io_loopback, io_loss_recovery, libsrt_interop, libsrt_loss, libsrt_media, pacing_throughput, tsbpd-related ones).

multimux compile fix (call-site adaptation only):

```rust
// multimux/src/source/srt.rs:320 — StreamStatus::Fed keeps Vec<u8> (public type, W2 owns changing it)
    Ok(StreamStatus::Fed(bytes.to_vec()))
// multimux/src/push/srt.rs:366
                    Ok(Ok(Some(data))) if &data[..] == PAYLOAD => {
```

(`driver.feed(&bytes, now)` at srt.rs:319 already takes `&[u8]`; `&Bytes` derefs.)

- [ ] **Step 4: Run, expect PASS**

```bash
cargo test --locked -p srt-runtime --all-features 2>&1 | grep -E '^test result|FAILED|panicked|^error' | sort | uniq -c
cargo build --locked -p multimux --all-features 2>&1 | tail -2
cargo build --locked -p srt-runtime --no-default-features --target thumbv7em-none-eabi 2>&1 | tail -1
```

Expected: all ok (including the libsrt interop suites, which prove the wire is untouched), multimux builds, the thumb build still passes (`bytes` is only under `tokio`).

- [ ] **Step 5: Revert-check (review-focus 2)**

Temporarily change `recv_datagram`'s reserve to `buf.reserve(max)` and the comparison to `n > max + 1000` (the old "truncate silently" behaviour: UDP clips at the buffer size so a 400-byte datagram arrives as 300): run `cargo test --locked -p srt-runtime --all-features --lib an_oversize`. Expected FAIL: the 400-byte datagram appears as a 300-byte item or `rx_oversize` is 0. Record the output, restore.

- [ ] **Step 6: Commit**

```bash
git add srt-runtime multimux/src/source/srt.rs multimux/src/push/srt.rs
git commit -m "feat(srt-runtime)!: IoConfig with configurable max datagram, Bytes datagrams, oversize drop-and-count"
```

---

### Task 4: SRT deadline-driven driver (`poll_timeout`), 2 ms tick removed (SP1.2)

**Files:**
- Modify: `srt-runtime/src/arq/receiver.rs` (add `next_timeout`, after `tick` at 345-370)
- Modify: `srt-runtime/src/tsbpd.rs` (add `next_release_after`, near 566)
- Modify: `srt-runtime/src/io.rs`: `Driver` (add `poll_timeout`), `Driver::run` (795-890), remove `TICK_INTERVAL_MS` (199)
- Test: unit tests in `receiver.rs`, `tsbpd.rs`, and `io.rs` `adapter_tests`

**Interfaces:**

```rust
// arq/receiver.rs (no_std core)
impl Receiver { /// Absolute (since the connection epoch) time of the next periodic Full ACK or, while the
                /// loss list is non-empty, the next periodic NAK. Always `Some`: the 10 ms Full ACK is
                /// unconditional (rule 11).
                pub fn next_timeout(&self) -> Duration; }
// tsbpd.rs (no_std core)
impl TsbpdScheduler { /// Earliest buffered packet play time strictly after `now`, if any. A buffered packet whose
                      /// time has passed but that cannot be released (waiting for a gap with TLPKTDROP off)
                      /// yields no wake-up: it is re-examined on the next event.
                      pub fn next_release_after(&self, now: Duration) -> Option<Duration>; }
// io.rs (Driver is private; its deadline is the SRT connection's `poll_timeout`)
impl Driver { fn poll_timeout(&self) -> Option<Instant>; }
```

The spec's `poll_timeout(&self) -> Option<Instant>` is on the (private) per-connection `Driver`, which is the SRT "connection" core of the adapter; the sans-IO engines it is built from expose the pure `Duration`-based building blocks above so they stay `no_std`. The idle Full ACK every 10 ms is protocol (rule 11), so an idle connection still wakes 100 times a second (down from 500); suppressing it would change wire behaviour and is out of scope.

- [ ] **Step 1: Write the failing tests**

`receiver.rs` test module:

```rust
    #[test]
    fn next_timeout_is_the_full_ack_period_then_tracks_the_nak_interval() {
        let mut r = Receiver::new(1, 0, 8192);
        assert_eq!(r.next_timeout(), FULL_ACK_PERIOD, "fresh receiver: first Full ACK at 10 ms");
        let _ = r.tick(FULL_ACK_PERIOD);
        assert_eq!(r.next_timeout(), FULL_ACK_PERIOD * 2);
        // a gap puts a NAK deadline on the table; it can only make the deadline earlier or equal
        let _ = r.feed_data(5, Duration::from_millis(11)); // 1..=4 now lost
        assert!(r.next_timeout() <= FULL_ACK_PERIOD * 2);
    }
```

`tsbpd.rs` test module:

```rust
    #[test]
    fn next_release_after_is_the_earliest_future_play_time() {
        let mut s = TsbpdScheduler::new(0, 0, DELAY_MS, 0, true, None);
        assert_eq!(s.next_release_after(Duration::ZERO), None, "nothing buffered");
        let _ = s.feed_data(0, 0, Duration::ZERO);
        let t = s.next_release_after(Duration::ZERO).expect("one packet buffered");
        assert_eq!(t, Duration::from_millis(DELAY_MS));
        assert_eq!(s.next_release_after(t), None, "strictly after");
        let _ = s.tick(t);
        assert_eq!(s.next_release_after(Duration::ZERO), None, "released packets leave the buffer");
    }

    #[test]
    fn a_stuck_gap_without_tlpktdrop_produces_no_busy_wakeup() {
        let mut s = TsbpdScheduler::new(0, 0, DELAY_MS, 0, false, None);
        let _ = s.feed_data(2, 0, Duration::ZERO); // 0 and 1 missing, drop disabled
        let late = Duration::from_millis(DELAY_MS * 10);
        let _ = s.tick(late);
        assert_eq!(s.next_release_after(late), None, "past-due but blocked: wait for events, not for time");
    }
```

(`DELAY_MS` is the test module's existing `const DELAY_MS: u64 = 120;` at tsbpd.rs:627.)

`io.rs` `adapter_tests`:

```rust
    #[tokio::test(start_paused = true)]
    async fn an_idle_driver_deadline_is_the_ack_period_not_a_two_ms_tick() {
        let (driver, _rx) = test_driver(8192).await;
        let d = driver.poll_timeout().expect("a connected driver always has a deadline");
        assert_eq!(d - driver.epoch, crate::arq::FULL_ACK_PERIOD);
    }

    #[tokio::test(start_paused = true)]
    async fn the_deadline_includes_the_tsbpd_release_and_the_keepalive() {
        let (mut driver, _rx) = test_driver(8192).await;
        driver.ingress(Bytes::from(data_bytes(0, 0, b"p"))).unwrap();
        // 120 ms play time is later than the 10 ms ACK: ACK still wins
        assert_eq!(driver.poll_timeout().unwrap() - driver.epoch, crate::arq::FULL_ACK_PERIOD);
        tokio::time::advance(Duration::from_millis(10)).await;
        driver.tick_engines();
        let _ = driver.flush_outbound().await;
        // still ACK-bound (next ACK at 20 ms), well before TSBPD (120 ms) and keepalive (1 s after the ACK)
        assert_eq!(driver.poll_timeout().unwrap() - driver.epoch, crate::arq::FULL_ACK_PERIOD * 2);
    }

    /// The old fixed 2 ms ticker ran `tick_engines` ~50 times per 100 ms of idle time; the
    /// deadline-driven loop wakes for the 10 ms ACK cadence only.
    #[tokio::test(start_paused = true)]
    async fn the_run_loop_wakes_at_deadlines_not_on_a_fixed_two_ms_tick() {
        let (mut driver, _rx, _tx) = test_driver_with_ingress(8192).await;
        let ticks = Arc::new(AtomicU64::new(0));
        driver.tick_counter = Some(Arc::clone(&ticks)); // #[cfg(test)] field
        let (_app_tx, app_rx) = mpsc::channel(8);
        let handle = tokio::spawn(driver.run(app_rx));
        // `advance` jumps the clock in ONE step, so walk it 1 ms at a time to let every timer fire in order.
        for _ in 0..100 {
            tokio::time::advance(Duration::from_millis(1)).await;
            tokio::task::yield_now().await;
        }
        let n = ticks.load(Ordering::Relaxed);
        assert!((8..=14).contains(&n), "expected ~10 wake-ups in 100 ms (10 ms ACK cadence), got {n}");
        handle.abort();
    }
```

(`test_driver_with_ingress` returns the live sender, so `rx` stays open and `run` keeps going; `tick_counter: Option<Arc<AtomicU64>>` is a `#[cfg(test)]` field on `Driver`, incremented at the top of `tick_engines`.)

- [ ] **Step 2: Run, expect FAIL**

```bash
cargo test --locked -p srt-runtime --all-features --lib next_timeout next_release_after a_stuck_gap an_idle_driver the_deadline_includes the_run_loop_wakes 2>&1 | grep -E '^error|no method|test result' | head
```

Expected: compile errors (`next_timeout`, `next_release_after`, `poll_timeout`, `tick_counter`).

- [ ] **Step 3: Implement**

`receiver.rs`:

```rust
    pub fn next_timeout(&self) -> Duration {
        let ack = self.last_full_ack_at + FULL_ACK_PERIOD;
        if self.loss_list.is_empty() {
            return ack;
        }
        let nak = self.last_nak_at + nak_interval(self.rtt.rtt(), self.rtt.rtt_var());
        ack.min(nak)
    }
```

`tsbpd.rs`:

```rust
    pub fn next_release_after(&self, now: Duration) -> Option<Duration> {
        let now_us = duration_us(now);
        self.buffer
            .values()
            .copied()
            .filter(|&t| t > now_us)
            .min()
            .map(Duration::from_micros)
    }
```

`io.rs` `Driver`:

```rust
    /// The next instant at which this connection must act without any inbound event.
    fn poll_timeout(&self) -> Option<Instant> {
        let now = self.elapsed();
        let mut at = self.epoch + self.receiver.next_timeout();                 // Full ACK / NAK
        if let Some(t) = self.tsbpd.next_release_after(now) {                   // TSBPD release / TLPKTDROP skip
            at = at.min(self.epoch + t);
        }
        if self.outbound_control.is_empty() && self.outbound_data.is_empty() {  // Keep-Alive (only judged when idle)
            at = at.min(self.last_tx_at + KEEPALIVE_PERIOD);
        }
        at = at.min(self.last_rx_at + PEER_IDLE_TIMEOUT);                       // peer idle (Task 6 makes it configurable)
        if let Some(wait) = self.next_paced_send_wait() {                       // pacing slot
            at = at.min(Instant::now() + wait);
        }
        // Floor: never ask for a wake-up in the past; a 1 ms minimum cannot spin and matches timer granularity.
        Some(at.max(Instant::now() + MIN_WAKE))
    }
```

with `const MIN_WAKE: Duration = Duration::from_millis(1);`. In `run`, delete the `ticker` and its arm; add the deadline arm and tick after every wake:

```rust
        loop {
            let pacing_wait = self.next_paced_send_wait();   // kept only for the flush decision below
            let accept_app_data = app_open && self.send_window_open();
            let wake_at = self.poll_timeout().unwrap_or_else(|| Instant::now() + KEEPALIVE_PERIOD);
            tokio::select! {
                maybe = app_out.recv(), if accept_app_data => { /* unchanged */ }
                maybe = self.rx.recv() => { /* unchanged */ }
                _ = tokio::time::sleep_until(wake_at) => {}
            }
            // Timer-driven engine work runs after EVERY wake-up: it is a pure function of `now`, so
            // an inbound packet also gets its ACK/NAK/TSBPD release without waiting for a tick.
            self.tick_engines();
            if self.flush_outbound().await.is_err() { break; }
            /* peer_idle / peer_shutdown / shutting_down checks unchanged */
        }
```

(The old separate pacing `sleep` arm is subsumed by `poll_timeout`'s pacing term; delete it.) Remove `TICK_INTERVAL_MS` and its doc. Keep `KEEPALIVE_PERIOD`, `PEER_IDLE_TIMEOUT`.

- [ ] **Step 4: Run, expect PASS**

```bash
cargo test --locked -p srt-runtime --all-features 2>&1 | grep -E '^test result|FAILED|panicked|^error' | sort | uniq -c
```

Expected: all ok, including the pre-existing paused-time keep-alive/idle/pacing tests (`an_idle_connection_sends_a_keepalive_after_one_second`, `the_peer_idle_timeout_is_five_silent_seconds`, `data_leaves_one_packet_per_pacing_period`, `ackack_is_not_queued_behind_a_not_yet_due_paced_data_packet`) and every libsrt interop and `pacing_throughput`/`livecc_pacing` test.

- [ ] **Step 5: Revert-check**

Re-add the fixed ticker (`let mut ticker = tokio::time::interval(Duration::from_millis(2)); ... _ = ticker.tick() => {}` and call `tick_engines` only on that arm) and run `cargo test --locked -p srt-runtime --all-features --lib the_run_loop_wakes`. Expected FAIL: `got 50` (or ~50) outside `8..=14`. Restore.

- [ ] **Step 6: Commit**

```bash
git add srt-runtime
git commit -m "feat(srt-runtime): deadline-driven connection driver (Receiver::next_timeout, TsbpdScheduler::next_release_after); 2 ms tick removed"
```

---

### Task 5: SRT listener: tracked background accept task, port released on drop (defect 3)

Handshakes advance in a background task, not inside `accept()`; the routing pump is owned by a `TaskTracker` and ends with a `CancellationToken`; the UDP socket is released when the last of {listener handle, accepted connections} is dropped.

**Files:**
- Modify: `srt-runtime/src/io.rs`: `SrtListener` struct and impl (1326-1885), `spawn_listener_routing_pump` (1888-1950), `ConnParams` (add `life`), `SrtSocket` (add `_life`), `Counters`
- Test: `srt-runtime/src/io.rs` tests that touch `listener.routes` / `listener.accept` (2748-2860), `srt-runtime/tests/io_listener_lifecycle.rs` (new), `srt-runtime/tests/io_concurrent_listener.rs`, `io_handshake.rs` (existing: must pass unchanged except `routes` access)

**Interfaces:**

```rust
// private
struct ListenerLife { cancel: CancellationToken, tasks: TaskTracker }
impl Drop for ListenerLife { fn drop(&mut self) { self.cancel.cancel(); self.tasks.close(); } }
// SrtListener keeps its public surface
impl SrtListener {
    pub async fn accept(&mut self) -> Result<SrtSocket>;               // unchanged signature; now only waits on a channel
    pub fn local_addr(&self) -> Result<SocketAddr>;
    pub fn unrouted_dropped(&self) -> u64;
    /// Connections that finished their handshake but were dropped because `accept` was not keeping up
    /// (the pending-accept queue holds `ACCEPT_BACKLOG` = 64).
    pub fn accept_overflow_dropped(&self) -> u64;                      // NEW
}
```

Design: `SrtListener { life: Arc<ListenerLife>, udp: Arc<UdpSocket>, accepted: mpsc::Receiver<Result<SrtSocket>>, unrouted_dropped: Arc<AtomicU64>, overflow_dropped: Arc<AtomicU64>, routes: RouteTable, local: SocketAddr }`. The old listener state (`config`, `cookie_keys`, `pending`, `outbound_queue`, `recent_accepts`, `unrouted_rx`) moves into a private `ListenerCore` run by one tracked task `listener_task`. The routing pump is a second tracked task. Both are spawned with `life.tasks.spawn(..)` and select on `life.cancel.cancelled()`. Each task holds a `Weak<ListenerLife>`-free clone of the `CancellationToken` only, so there is no ownership cycle. Every accepted `SrtSocket` holds `_life: Option<Arc<ListenerLife>>`; the core holds a `Weak<ListenerLife>` and upgrades it when it hands a connection over (if the upgrade fails, every handle is gone and the task exits). Result: the UDP socket (held by the two tasks and by connection drivers) is closed when the last `Arc<ListenerLife>` drops.

- [ ] **Step 1: Write the failing tests**

`srt-runtime/tests/io_listener_lifecycle.rs`:

```rust
#![cfg(feature = "tokio")]

use std::net::{SocketAddr, UdpSocket};
use std::time::Duration;

use srt_runtime::HandshakeConfig;
use srt_runtime::io::{SrtListener, SrtSocket};

const WAIT: Duration = Duration::from_secs(10);

/// Waits (on the real condition, bounded) until `addr` can be bound again.
async fn port_is_released(addr: SocketAddr) -> bool {
    tokio::time::timeout(WAIT, async {
        loop {
            if UdpSocket::bind(addr).is_ok() {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .is_ok()
}

/// Defect 3: the routing pump held an `Arc<UdpSocket>` in `recv_from` and only noticed a dropped
/// listener when the NEXT datagram arrived, so an idle listener kept its port bound forever.
#[tokio::test]
async fn dropping_an_idle_listener_releases_its_udp_port() {
    let listener = SrtListener::bind("127.0.0.1:0", HandshakeConfig::default()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    assert!(port_is_released(addr).await, "the UDP port stayed bound after the listener was dropped");
}

/// Review-focus 1: dropping the listener HANDLE must not kill connections it already accepted.
#[tokio::test]
async fn an_accepted_connection_outlives_the_listener_handle_and_then_releases_the_port() {
    let mut listener = SrtListener::bind("127.0.0.1:0", HandshakeConfig::default()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let accept = tokio::spawn(async move { listener.accept().await.map(|s| (s, listener)) });
    let mut caller = SrtSocket::connect(addr, HandshakeConfig::default()).await.unwrap();
    let (mut accepted, listener) = tokio::time::timeout(WAIT, accept).await.unwrap().unwrap().unwrap();
    drop(listener);                        // handle gone, connection live
    caller.send(b"still routed").await.unwrap();
    let got = tokio::time::timeout(WAIT, accepted.recv()).await.unwrap().unwrap().unwrap();
    assert_eq!(&got[..], b"still routed", "routing pump must keep serving accepted connections");
    assert!(UdpSocket::bind(addr).is_err(), "port is still in use while a connection is alive");
    drop(accepted);
    drop(caller);
    assert!(port_is_released(addr).await, "port must be released once the last connection is gone");
}

/// Handshakes used to advance only while `accept()` was being polled.
#[tokio::test]
async fn a_caller_connects_even_though_accept_is_not_being_polled() {
    let mut listener = SrtListener::bind("127.0.0.1:0", HandshakeConfig::default()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let caller = tokio::time::timeout(Duration::from_secs(3), SrtSocket::connect(addr, HandshakeConfig::default()))
        .await
        .expect("handshake must complete without anyone calling accept()")
        .unwrap();
    let _accepted = tokio::time::timeout(WAIT, listener.accept()).await.unwrap().unwrap();
    drop(caller);
}
```

In-crate regression for the route cleanup test (existing `dropping_an_accepted_socket_removes_its_route_entry`): its `lock(&listener.routes)` access stays valid because `routes` remains a field of `SrtListener` (the core shares the same `Arc`).

- [ ] **Step 2: Run, expect FAIL**

```bash
cargo test --locked -p srt-runtime --all-features --test io_listener_lifecycle 2>&1 | grep -E '^error|test result|panicked|stayed bound|handshake must complete' | head
```

Expected on the old code: `dropping_an_idle_listener_releases_its_udp_port` FAILS (`stayed bound`), `a_caller_connects_even_though_accept_is_not_being_polled` FAILS (`handshake must complete ...`, the caller waits out its 5 s budget; the 3 s bound fires first), `an_accepted_connection_outlives...` fails at the final `port_is_released`.

- [ ] **Step 3: Implement**

```rust
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

const ACCEPT_BACKLOG: usize = 64;

struct ListenerLife { cancel: CancellationToken, tasks: TaskTracker }
impl Drop for ListenerLife {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.tasks.close();
    }
}

pub struct SrtListener {
    life: Arc<ListenerLife>,
    udp: Arc<UdpSocket>,
    local: std::net::SocketAddr,
    accepted: mpsc::Receiver<Result<SrtSocket>>,
    routes: RouteTable,
    unrouted_dropped: Arc<AtomicU64>,
    overflow_dropped: Arc<AtomicU64>,
}

impl SrtListener {
    pub async fn bind_with<A: tokio::net::ToSocketAddrs>(addr: A, config: HandshakeConfig, io: IoConfig) -> Result<Self> {
        refuse_crypto(&config)?;
        let socket = Arc::new(UdpSocket::bind(addr).await.map_err(|e| io_err("bind", e))?);
        let local = socket.local_addr().map_err(|e| io_err("local_addr", e))?;
        let routes: RouteTable = Arc::new(Mutex::new(std::collections::HashMap::new()));
        let (unrouted_tx, unrouted_rx) = mpsc::channel(UNROUTED_CHANNEL_CAPACITY);
        let (accepted_tx, accepted) = mpsc::channel(ACCEPT_BACKLOG);
        let unrouted_dropped = Arc::new(AtomicU64::new(0));
        let overflow_dropped = Arc::new(AtomicU64::new(0));
        let life = Arc::new(ListenerLife { cancel: CancellationToken::new(), tasks: TaskTracker::new() });

        life.tasks.spawn(routing_pump(
            Arc::clone(&socket), Arc::clone(&routes), unrouted_tx,
            Arc::clone(&unrouted_dropped), io.max_datagram, life.cancel.clone(),
        ));
        let core = ListenerCore {
            udp: Arc::clone(&socket), config, io,
            cookie_keys: CookieKeys::new(Instant::now())?,
            pending: Default::default(), outbound_queue: Default::default(), recent_accepts: Default::default(),
            routes: Arc::clone(&routes), unrouted_rx,
            life: Arc::downgrade(&life), accepted_tx, overflow_dropped: Arc::clone(&overflow_dropped),
            last_pending_tick: Instant::now(),
        };
        life.tasks.spawn(core.run(life.cancel.clone()));
        Ok(SrtListener { life, udp: socket, local, accepted, routes, unrouted_dropped, overflow_dropped })
    }

    pub async fn accept(&mut self) -> Result<SrtSocket> {
        self.accepted.recv().await.unwrap_or(Err(Error::Io {
            kind: std::io::ErrorKind::BrokenPipe,
            context: "listener task ended",
        }))
    }
    pub fn local_addr(&self) -> Result<std::net::SocketAddr> { Ok(self.local) }
    pub fn accept_overflow_dropped(&self) -> u64 { self.overflow_dropped.load(Ordering::Relaxed) }
}
```

`ListenerCore` is the former listener state with the old methods `handle_datagram`, `tick_pending`, `remember_accept`, `drain_completed`, `flush_for_peer`, `flush_all` moved unchanged (they only used `self.*` fields), plus:

```rust
impl ListenerCore {
    /// Next instant a pending handshake needs a tick; `None` when nothing is pending (an idle
    /// listener has no timer at all).
    fn poll_timeout(&self) -> Option<Instant> {
        (!self.pending.is_empty()).then(|| self.last_pending_tick + PENDING_TICK_INTERVAL)
    }

    async fn run(mut self, cancel: CancellationToken) {
        loop {
            while let Some(conn) = self.drain_completed() {
                match self.accepted_tx.try_send(conn) {
                    Ok(()) => {}
                    Err(mpsc::error::TrySendError::Full(_dropped)) => {
                        // dropping the SrtSocket sends the peer a SHUTDOWN; count it
                        self.overflow_dropped.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => return,
                }
            }
            let deadline = self.poll_timeout();
            tokio::select! {
                _ = cancel.cancelled() => return,
                r = self.unrouted_rx.recv() => match r {
                    Some((src, bytes)) => {
                        // cookie rotation no longer rides the tick: do it whenever a datagram arrives
                        self.cookie_keys.rotate_if_due(Instant::now());
                        let _ = self.handle_datagram(src, &bytes);
                        if self.flush_for_peer(src).await.is_err() { return; }
                    }
                    None => return,
                },
                _ = async { match deadline { Some(d) => tokio::time::sleep_until(d).await, None => std::future::pending().await } } => {
                    self.last_pending_tick = Instant::now();
                    self.tick_pending();
                    if self.flush_all().await.is_err() { return; }
                }
            }
        }
    }
}
```

`drain_completed` upgrades the weak life: `let life = self.life.upgrade()?;` (returns `None` when every handle is gone: the loop then ends via `Closed`/cancel on the next iteration) and stores `Some(life)` in the new `ConnParams.life` field so the spawned `SrtSocket` keeps the listener's tasks (and socket) alive. `drain_completed` previously returned `Option<Result<SrtSocket>>`; keep that shape. `routing_pump` is the old `spawn_listener_routing_pump` body as an `async fn` with the loop `tokio::select! { _ = cancel.cancelled() => break, r = recv_datagram(..) => ... }` (the Task 3 helper); it is spawned through `life.tasks`, not `tokio::spawn`. `SrtSocket` gets `_life: Option<Arc<ListenerLife>>` (`None` for `connect`ed sockets); `ConnParams` gets `life: Option<Arc<ListenerLife>>`.

`Debug for SrtListener` keeps `local_addr` and drops the fields that moved; the existing test `listener_debug_never_prints_the_cookie_key` (3451) must move to a `ListenerCore` Debug (the cookie key lives there) or assert on `SrtListener` that no secret is printed: keep `impl Debug for ListenerCore` (the old body) and make `SrtListener`'s Debug print only `local_addr`; adapt that test to build a `ListenerCore` directly.

Tests that referenced `listener.pending`/`cookie_keys` (3476-3560: `the_cookie_key_rotates_every_minute`, `a_handshake_begun_before_a_rotation_still_completes`, `listener_pending_is_capped_under_a_source_flood`, `pending_entries_expire_while_other_sources_keep_sending`, `listener_parse_failure_...`, `listener_feed_failure_...`) operate on the listener state: move them to construct `ListenerCore` through a `#[cfg(test)] fn ListenerCore::for_test(config) -> Self` and drive `handle_datagram`/`tick_pending` directly, unchanged in assertions.

- [ ] **Step 4: Run, expect PASS**

```bash
cargo test --locked -p srt-runtime --all-features 2>&1 | grep -E '^test result|FAILED|panicked|^error' | sort | uniq -c
```

Expected: all ok, including `io_concurrent_listener`, `io_handshake` (duplicate-conclusion/retransmit tests: the background task answers repeats without `accept` being polled) and the libsrt interop suites.

- [ ] **Step 5: Revert-check (defect 3)**

```bash
git stash push -q -- srt-runtime/src/io.rs
git checkout $BASE -- srt-runtime/src/io.rs
cargo test --locked -p srt-runtime --all-features --test io_listener_lifecycle 2>&1 | grep -E 'test result|FAILED|stayed bound|must complete' | head
git checkout HEAD -- srt-runtime/src/io.rs && git stash pop -q
```

Expected (old `io.rs`; the new test file only uses the public API, so it compiles against it): `dropping_an_idle_listener_releases_its_udp_port` FAILS with `the UDP port stayed bound after the listener was dropped` and `a_caller_connects_even_though_accept_is_not_being_polled` FAILS. Record both failing names. (The old file lacks `IoConfig` from Task 3, which `io_config.rs` needs: run only this one test binary.) Then re-run Step 4.

- [ ] **Step 6: Commit**

```bash
git add srt-runtime
git commit -m "fix(srt-runtime)!: listener handshakes in a tracked background task; pump and port released on drop (defect 3)"
```

---

### Task 6: SRT timeouts from `IoConfig` (SP1.3)

**Files:**
- Modify: `srt-runtime/src/io.rs`: `HANDSHAKE_TIMEOUT` (206) and its use in `connect_from_with`; `PEER_IDLE_TIMEOUT` (228), `Driver::peer_idle` (893), `Driver::poll_timeout` (Task 4); `resolve_one`/`UdpSocket::bind` in `connect_from_with` (connect); `flush_outbound` sends (1202-1235); `shutdown`/`send_shutdown` untouched (non-blocking `try_send_to`)
- Test: `io.rs` `adapter_tests`; `srt-runtime/tests/io_config.rs` (append)

**Interfaces:** `IoConfig` fields from Task 3 become live: `connect` bounds `UdpSocket::bind` + `resolve_one`; `handshake` replaces `HANDSHAKE_TIMEOUT`; `read_idle` replaces `PEER_IDLE_TIMEOUT` (and `ConnParams.read_idle`); `write` bounds every `send_to`. A bounded helper:

```rust
/// Runs `fut` for at most `limit`; expiry is `Error::Io { kind: TimedOut, context }`.
async fn bounded<T>(limit: Duration, context: &'static str, fut: impl Future<Output = Result<T>>) -> Result<T> {
    tokio::time::timeout(limit, fut).await.map_err(|_| Error::Io { kind: std::io::ErrorKind::TimedOut, context })?
}
```

Handshake expiry keeps its existing error (`Error::HandshakeTimedOut`).

- [ ] **Step 1: Write the failing tests**

```rust
    #[tokio::test(start_paused = true)]
    async fn bounded_turns_a_stuck_future_into_a_timed_out_io_error() {
        let r: Result<()> = bounded(Duration::from_secs(3), "send", std::future::pending::<Result<()>>()).await;
        assert!(matches!(r, Err(Error::Io { kind: std::io::ErrorKind::TimedOut, context: "send" })), "{r:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn the_configured_read_idle_replaces_the_five_second_constant() {
        let (mut driver, _rx) = test_driver(8192).await;
        driver.read_idle = Duration::from_secs(2);
        assert!(!driver.peer_idle(Instant::now()));
        tokio::time::advance(Duration::from_secs(2)).await;
        assert!(driver.peer_idle(Instant::now()), "idle after the configured 2 s, not 5 s");
        // and the deadline the loop sleeps until follows it
        assert!(driver.poll_timeout().unwrap() <= driver.last_rx_at + Duration::from_secs(2));
    }

    #[tokio::test(start_paused = true)]
    async fn the_handshake_budget_comes_from_the_config() {
        // black hole: bound UDP socket that never answers
        let hole = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let io = IoConfig::default().with_handshake(Duration::from_secs(2));
        let t0 = Instant::now();
        let err = SrtSocket::connect_with(hole.local_addr().unwrap(), HandshakeConfig::default(), io)
            .await
            .expect_err("nobody answers");
        // The default retransmit budget (12 x 250 ms) would end an unanswered connect at ~3 s with
        // stage "caller retransmit budget"; only the configured 2 s deadline yields "caller connect".
        assert!(matches!(err, Error::HandshakeTimedOut { stage: "caller connect" }), "{err:?}");
        assert_eq!(Instant::now() - t0, Duration::from_secs(2), "the configured 2 s deadline");
        drop(hole);
    }
```

(The black-hole test uses a real bound socket plus paused time: the only real IO is the local `send_to`, which completes immediately; nothing waits on the socket, so auto-advance cannot race a datagram that never comes.)

- [ ] **Step 2: Run, expect FAIL**

```bash
cargo test --locked -p srt-runtime --all-features --lib bounded_turns the_configured_read_idle the_handshake_budget 2>&1 | grep -E '^error|no field|cannot find|test result' | head
```

Expected: compile errors (`bounded`, `Driver::read_idle`).

- [ ] **Step 3: Implement**

`ConnParams` and `Driver` get `read_idle: Duration` (from `io.read_idle`); `peer_idle` uses `self.read_idle`; `poll_timeout` uses `self.last_rx_at + self.read_idle`; remove `PEER_IDLE_TIMEOUT` and `HANDSHAKE_TIMEOUT` (the doc comments move onto the `IoConfig` defaults: 5 s / 5 s, libsrt's `SRTO_PEERIDLETIMEO` and the previous connect budget). `connect_from_with`: `let socket = bounded(io.connect, "bind", async { UdpSocket::bind(local_addr).await.map_err(|e| io_err("bind", e)) }).await?;` and `let peer = bounded(io.connect, "resolve", resolve_one(remote_addr)).await?;`; `let deadline = Instant::now() + io.handshake;`. In `flush_outbound`, wrap each send: `bounded(self.write_timeout, "send", async { self.udp.send_to(&bytes, self.peer_addr).await.map(|_| ()).map_err(|e| io_err("send", e)) }).await?` (`write_timeout: Duration` on the driver). The listener core's `flush_for_peer` uses `io.write` the same way.

- [ ] **Step 4: Run, expect PASS**

```bash
cargo test --locked -p srt-runtime --all-features 2>&1 | grep -E '^test result|FAILED|panicked|^error' | sort | uniq -c
```

Expected: all ok (the old `the_peer_idle_timeout_is_five_silent_seconds` still holds with the default 5 s).

- [ ] **Step 5: Revert-check**

Hard-code `Duration::from_secs(5)` in `connect_from_with`'s deadline again and run `cargo test ... --lib the_handshake_budget`: expect FAIL (the error is `caller retransmit budget` at ~3 s, not `caller connect` at 2 s). Restore.

- [ ] **Step 6: Commit**

```bash
git add srt-runtime
git commit -m "feat(srt-runtime): connect/handshake/read_idle/write timeouts from IoConfig; bounded sends"
```

---

### Task 7: webrtc `MediaTransport::new` takes `now`; `poll_timeout`; `local_candidates` (W0 carry, SP1.2)

**Files:**
- Modify: `webrtc-runtime/src/media/transport.rs`: `new` (568-598), `with_certificate` (603-735: the three `Instant::now()` at 622, 644, 708), new methods after `handle_timeout` (816-841), `local_candidates` after `add_remote_candidate` (758-780); in-file tests that call `MediaTransport::new(..)` / `with_certificate(..)` (1492-1500, 1931-1941, 2433-2470 and the other `new(` sites; `grep -n 'MediaTransport::new\|with_certificate(' webrtc-runtime/src webrtc-runtime/tests webrtc-runtime/examples`)
- Modify: `webrtc-runtime/src/media/gather.rs`: add `poll_timeout`
- Modify: `webrtc-runtime/tests/dtls_fingerprint.rs`, `whip_smoke_pcap_stun.rs`, `examples/whip_media_smoke.rs` (call sites)
- Modify (compile fix only): `multimux/src/source/whip.rs` lines 626 and 1434, `multimux/src/output/whep.rs` lines 587, 1412 and 1653

**Interfaces:**

```rust
impl MediaTransport {
    /// `now` is the caller's clock reading at construction; every internal timer (ICE agent, STUN
    /// gather) is scheduled from it. The transport never reads the wall clock.
    pub fn new(config: MediaTransportConfig, now: std::time::Instant) -> Result<Self, Error>;
    /// Earliest instant at which `handle_timeout` has work: the minimum over the ICE agent, every DTLS
    /// association, the STUN gatherer and the retired-key purge. `None` when nothing is scheduled.
    /// Takes `&mut self` (spec says `&self`): the upstream `rtc-ice`/`rtc-stun` `Protocol::poll_timeout`
    /// takes `&mut self`, so the aggregate cannot be `&self`.
    pub fn poll_timeout(&mut self) -> Option<std::time::Instant>;
    /// Local ICE candidates as `a=candidate:` bodies (the `rtc_ice::candidate::Candidate::marshal`
    /// form), for the caller's SDP answer or Trickle-ICE fragment.
    pub fn local_candidates(&self) -> Vec<String>;
}
// gather.rs (pub(super))
impl StunGather { pub(super) fn poll_timeout(&mut self) -> Option<Instant>; }   // StunGather::new already takes `now`
```

multimux edit at each of the five call sites: `MediaTransport::new(cfg)` becomes `MediaTransport::new(cfg, std::time::Instant::now())` (both files already `use std::time::Instant`; use that import).

- [ ] **Step 1: Write the failing tests**

In `transport.rs` tests:

```rust
    /// Review-focus 5 (clock half): `new` used to read the wall clock three times. Built at a `now`
    /// one hour in the FUTURE, every deadline must be at or after that `now`; a wall-clock read would
    /// put the ICE agent's first timer about an hour earlier.
    #[test]
    fn a_transport_schedules_from_the_supplied_now_not_the_wall_clock() {
        let t0 = Instant::now() + Duration::from_secs(3600);
        let mut cfg = test_config(SetupRole::Active);
        cfg.stun_server = Some("127.0.0.1:3478".parse().unwrap());
        let mut mt = MediaTransport::new(cfg, t0).unwrap();
        let d = mt.poll_timeout().expect("ICE connectivity checks and the STUN gatherer have timers");
        assert!(d >= t0, "deadline is {:?} BEFORE the supplied now", t0 - d);
        assert!(d <= t0 + Duration::from_secs(30), "and within a sane retransmission horizon");
    }

    #[test]
    fn poll_timeout_always_moves_forward_so_a_driver_cannot_spin() {
        let t0 = Instant::now();
        let mut cfg = test_config(SetupRole::Active);
        cfg.stun_server = Some("127.0.0.1:3478".parse().unwrap());
        let mut mt = MediaTransport::new(cfg, t0).unwrap();
        let mut last = t0;
        for _ in 0..6 {
            let Some(d) = mt.poll_timeout() else { return };
            assert!(d >= last, "deadline went backwards");
            let _ = mt.handle_timeout(d);
            while mt.poll_transmit().is_some() {}
            let next = mt.poll_timeout();
            assert!(next.is_none_or(|n| n > d), "handle_timeout({d:?}) left a deadline that is not in the future: {next:?}");
            last = d;
        }
    }

    #[test]
    fn local_candidates_lists_the_host_candidate_as_an_attribute_body() {
        let cfg = test_config(SetupRole::Passive);
        let addr = cfg.local_addr;
        let mt = MediaTransport::new(cfg, Instant::now()).unwrap();
        let c = mt.local_candidates();
        assert_eq!(c.len(), 1);
        assert!(c[0].contains(&format!("{} {} typ host", addr.ip(), addr.port())), "{}", c[0]);
        assert!(!c[0].starts_with("candidate:") && !c[0].starts_with("a="));
    }
```

Mechanically add `Instant::now()` as the second argument to every existing `MediaTransport::new(cfg)` and `with_certificate(..)` call in the file's tests, `tests/*.rs` and the example; those are the compile-driven edits.

- [ ] **Step 2: Run, expect FAIL**

```bash
cargo test --locked -p webrtc-runtime --all-features --lib a_transport_schedules poll_timeout_always local_candidates 2>&1 | grep -E '^error|expected 2 arguments|no method|test result' | head
```

Expected: compile errors (`new` takes 1 argument, `poll_timeout`, `local_candidates` missing).

- [ ] **Step 3: Implement**

```rust
    pub fn new(config: MediaTransportConfig, now: Instant) -> Result<Self, Error> {
        let remote_fingerprint_digest = parse_fingerprint_value(&config.remote_fingerprint).map_err(Error::Media)?;
        /* ActPass check, certificate generation unchanged */
        Self::with_certificate(config, remote_fingerprint_digest, certificate, crypto_provider, now)
    }
    fn with_certificate(config, digest, certificate, crypto_provider, now: Instant) -> Result<Self, Error> {
        /* IceAgent::new(now, ...), start_connectivity_checks(now, ...), StunGather::new(now, ...) */
    }

    pub fn poll_timeout(&mut self) -> Option<Instant> {
        fn earliest(a: Option<Instant>, b: Option<Instant>) -> Option<Instant> {
            match (a, b) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            }
        }
        let mut next = Protocol::poll_timeout(&mut self.ice);
        let peers: Vec<SocketAddr> = self.dtls.get_connections_keys().copied().collect();
        for peer in peers {
            next = earliest(next, self.dtls.poll_timeout(&peer));
        }
        if let Some(g) = &mut self.gather {
            next = earliest(next, g.poll_timeout());
        }
        if let Some((_, purge_at)) = &self.retired_srtp_read {
            next = earliest(next, Some(*purge_at));
        }
        next
    }

    pub fn local_candidates(&self) -> Vec<String> {
        self.ice.get_local_candidates().iter().map(|c| c.marshal()).collect()
    }
```

(`rtc-ice-0.21.0/src/agent/mod.rs:751 get_local_candidates`, `rtc-dtls-0.21.0/src/endpoint.rs:288 poll_timeout(&self, remote: &SocketAddr)`, `rtc-stun-0.21.0/src/client.rs:310 poll_timeout`.) `gather.rs`: `pub(super) fn poll_timeout(&mut self) -> Option<Instant> { Protocol::poll_timeout(&mut self.client) }`. Update the `media` module docs: "pass `now` to `new`; schedule `handle_timeout` at `poll_timeout`, never on a fixed tick".

- [ ] **Step 4: Run, expect PASS**

```bash
cargo test --locked -p webrtc-runtime --all-features 2>&1 | grep -E '^test result|FAILED|panicked|^error' | sort | uniq -c
cargo build --locked -p multimux --all-features 2>&1 | tail -2
cargo clippy --locked -p multimux --no-default-features --features whip --all-targets -- -D warnings 2>&1 | tail -2
cargo clippy --locked -p multimux --no-default-features --features whep --all-targets -- -D warnings 2>&1 | tail -2
```

Expected: all ok; multimux builds with `whip` and `whep` alone (the gate runs both).

- [ ] **Step 5: Revert-check**

Replace the `now` argument of `IceAgent::new` and `start_connectivity_checks` with `Instant::now()` again and run `cargo test --locked -p webrtc-runtime --all-features --lib a_transport_schedules`. Expected FAIL: `deadline is ~3600s BEFORE the supplied now`. Restore.

- [ ] **Step 6: Commit**

```bash
git add webrtc-runtime multimux/src/source/whip.rs multimux/src/output/whep.rs
git commit -m "feat(webrtc-runtime)!: MediaTransport::new takes a caller-supplied now; poll_timeout and local_candidates"
```

---

### Task 8: webrtc fingerprint via `sdp-types` typed attributes; host candidates via `rtc-ice`; `stun:` URL via `url` (SP4, SP3)

**Files:**
- Modify: `webrtc-runtime/src/media/transport.rs`: `parse_fingerprint_value` (128-166), `parse_remote_fingerprint` (201-221), `add_server_reflexive_candidate` url (1311), tests (1492-1555 and the loopback helpers 2405-2470)
- Modify: `webrtc-runtime/tests/dtls_fingerprint.rs` (`host_candidate` 86-88 and its callers), `webrtc-runtime/examples/whip_media_smoke.rs` (`answer_candidate_line`)
- Modify: `webrtc-runtime/Cargo.toml` (`media` feature gets `dep:sdp-types`, `dep:url`: done in Task 2)

**Interfaces:**

```rust
pub fn parse_remote_fingerprint(sdp: &str) -> Option<String>;   // signature unchanged; semantics below
fn parse_fingerprint_value(value: &str) -> Result<[u8; FINGERPRINT_LEN], String>;   // private, same contract
fn stun_url(server: SocketAddr) -> String;                      // private
```

Behaviour changes (listed in the CHANGELOG with examples): `parse_remote_fingerprint` now requires a parseable SDP (`Session::parse`) and returns the typed attribute's normalised text (`sha-256` lower-case token, upper-case colon-hex digest); a digest written without colons but with 32 bytes of hex is accepted by the typed parser.

- [ ] **Step 1: Write the failing tests**

In `transport.rs` tests, replace `parse_remote_fingerprint_media_beats_session` and add:

```rust
    const SDP_HEAD: &str = "v=0\r\no=- 1 1 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\n";

    #[test]
    fn parse_remote_fingerprint_media_beats_session() {
        let sdp = format!("{SDP_HEAD}a=fingerprint:sha-256 AA\r\nm=audio 9 RTP/AVP 0\r\na=fingerprint:sha-256 BB\r\n");
        assert_eq!(parse_remote_fingerprint(&sdp).as_deref(), Some("sha-256 BB"));
        let sdp = format!("{SDP_HEAD}a=fingerprint:sha-256 AA \r\nm=audio 9 RTP/AVP 0\r\n");
        assert_eq!(parse_remote_fingerprint(&sdp).as_deref(), Some("sha-256 AA"), "trailing space tolerated");
        assert_eq!(parse_remote_fingerprint(SDP_HEAD), None);
    }

    #[test]
    fn parse_remote_fingerprint_takes_the_first_media_section_of_a_bundle() {
        let sdp = format!(
            "{SDP_HEAD}m=audio 9 RTP/AVP 0\r\na=fingerprint:sha-256 11\r\nm=video 9 RTP/AVP 96\r\na=fingerprint:sha-256 22\r\n"
        );
        assert_eq!(parse_remote_fingerprint(&sdp).as_deref(), Some("sha-256 11"));
    }

    #[test]
    fn parse_remote_fingerprint_normalises_through_the_typed_attribute() {
        let sdp = format!("{SDP_HEAD}a=fingerprint:SHA-256 ab:cd:0f\r\n");
        assert_eq!(parse_remote_fingerprint(&sdp).as_deref(), Some("sha-256 AB:CD:0F"));
    }

    #[test]
    fn something_that_is_not_an_sdp_has_no_fingerprint() {
        assert_eq!(parse_remote_fingerprint("not sdp at all"), None);
        assert_eq!(parse_remote_fingerprint("v=0\r\n"), None, "an SDP without o=/s=/t= is not a session");
    }

    #[test]
    fn a_colonless_32_byte_digest_is_accepted_like_the_typed_attribute_does() {
        let hex = "AB".repeat(32);
        assert_eq!(parse_fingerprint_value(&format!("sha-256 {hex}")).unwrap(), [0xAB; 32]);
        assert!(parse_fingerprint_value(&format!("sha-256 {}", "AB".repeat(31))).is_err(), "31 bytes");
    }

    #[test]
    fn stun_urls_bracket_ipv6_and_keep_the_port() {
        assert_eq!(stun_url("192.0.2.1:3478".parse().unwrap()), "stun:192.0.2.1:3478");
        assert_eq!(stun_url("[2001:db8::1]:3478".parse().unwrap()), "stun:[2001:db8::1]:3478");
    }
```

The existing `new_rejects_malformed_remote_fingerprint` (`"md5 00:11"`, `"sha-256 00:11"`, `""`, `"sha-256"`) and `parse_fingerprint_value_accepts_case_insensitive_sha256` bad-list (`"md5 AB:CD"`, `"sha-256"`, `"sha-256 AB:CD"`, `"sha-256 AB:CD:"`, `"sha-256 ABCD"`, a `ZZ` digest) must pass UNCHANGED against the typed implementation.

Candidate helper in the loopback tests: delete `host_candidate_line` and `host_candidate`; callers use the peer's `local_candidates()[0]` (Task 7), e.g. in `loopback_pair_with_max_remote`: `a.add_remote_candidate(&b.local_candidates()[0])` and the reverse; in `dtls_fingerprint.rs`: `a.add_remote_candidate(&b.local_candidates()[0])`. (`reserve_loopback_addr`/`reserve_udp_addr` stay: they only produce two distinct loopback addresses for sans-IO transports, nothing rebinds them.)

- [ ] **Step 2: Run, expect FAIL**

```bash
cargo test --locked -p webrtc-runtime --all-features --lib parse_remote_fingerprint a_colonless stun_urls 2>&1 | grep -E '^error|cannot find|FAILED|test result' | head
```

Expected: `stun_url` not found (compile error); after stubbing it, `parse_remote_fingerprint_normalises...` and `something_that_is_not_an_sdp...` FAIL on the hand-rolled line scanner (it returns `Some("ab:cd:0f")` / `Some(...)` for a bare `v=0` that carries no fingerprint it would return None, but `"not sdp at all"`... returns None too; the normalisation test is the one that fails).

- [ ] **Step 3: Implement**

```rust
use sdp_types::{Fingerprint, HashFunc, Session};

fn parse_fingerprint_value(value: &str) -> Result<[u8; FINGERPRINT_LEN], String> {
    let fp: Fingerprint = value
        .trim()
        .parse()
        .map_err(|e| format!("remote_fingerprint {value:?}: {e}"))?;
    if fp.hash_func != HashFunc::Sha256 {
        return Err(format!(
            "remote_fingerprint {value:?} must use the {FINGERPRINT_HASH_TOKEN} hash function (RFC 8122 §5, RFC 8827 §6.5)"
        ));
    }
    <[u8; FINGERPRINT_LEN]>::try_from(fp.fingerprint.as_slice()).map_err(|_| {
        format!(
            "remote_fingerprint digest must be exactly {FINGERPRINT_LEN} bytes, got {}",
            fp.fingerprint.len()
        )
    })
}

pub fn parse_remote_fingerprint(sdp: &str) -> Option<String> {
    let session = Session::parse(sdp.as_bytes()).ok()?;
    // Media-level first (a bundled offer's per-m= value is the one a peer commits to),
    // then session-level (RFC 8866 §5.13 allows either).
    let fp = session
        .medias
        .iter()
        .find_map(|m| m.get_first_attribute_typed::<Fingerprint>().and_then(Result::ok))
        .or_else(|| session.get_first_attribute_typed::<Fingerprint>().and_then(Result::ok))?;
    Some(fp.to_string())
}

fn stun_url(server: SocketAddr) -> String {
    let host = match server.ip() {
        std::net::IpAddr::V4(a) => url::Host::<String>::Ipv4(a),
        std::net::IpAddr::V6(a) => url::Host::<String>::Ipv6(a),
    };
    format!("stun:{host}:{}", server.port())
}
```

`add_server_reflexive_candidate` uses `url: Some(stun_url(stun_server))`. Delete the old scanner and the `strip_prefix("a=fingerprint:")` / `starts_with("m=")` lines. The example's `answer_candidate_line` becomes `media.local_candidates().remove(0)`. (`sdp-types-0.2.0/src/lib.rs:610` `Media::get_first_attribute_typed`, `:814` `Session::get_first_attribute_typed`, `attributes.rs:967-1070` `Fingerprint`.)

- [ ] **Step 4: Run, expect PASS**

```bash
cargo test --locked -p webrtc-runtime --all-features 2>&1 | grep -E '^test result|FAILED|panicked|^error' | sort | uniq -c
```

Expected: all ok including `dtls_fingerprint.rs` (the wrong-fingerprint rejection, the `tests/stun_rfc5769_vectors.rs` and `srtp_rfc3711_vectors.rs` oracle suites) and the pre-existing malformed-fingerprint tests.

- [ ] **Step 5: Revert-check**

Put the old line scanner back into `parse_remote_fingerprint` (`git show $BASE:webrtc-runtime/src/media/transport.rs | sed -n '/^pub fn parse_remote_fingerprint/,/^}/p'`) and run the library tests: `parse_remote_fingerprint_normalises_through_the_typed_attribute` FAILS (`left: Some("SHA-256 ab:cd:0f")`). Restore.

- [ ] **Step 6: Commit**

```bash
git add webrtc-runtime
git commit -m "refactor(webrtc-runtime): SDP fingerprint via sdp-types typed attributes; stun URL via url::Host; host candidates via rtc-ice"
```

---

### Task 9: webrtc core goes `std`; WHIP/WHEP state machines on `http` / `headers` types (SP2.2)

**Files:**
- Modify: `webrtc-runtime/src/lib.rs` (drop `#![cfg_attr(not(feature = "std"), no_std)]`, update docs 1-12), `Cargo.toml` (`std` feature becomes `[]`-compatible no-op list: keep the name so `--no-default-features`/`default-features = false` consumers still resolve), `src/error.rs` (new variant)
- Create: `webrtc-runtime/src/http_util.rs` (crate-private)
- Modify: `src/whip/client.rs` (HttpRequest/HttpResponse/Method, 24-120 and all builders), `src/whip/server.rs`, `src/whep/player.rs`, `src/whep/server.rs`
- Modify tests: `tests/whip_round_trip.rs`, `tests/whep_round_trip.rs`, `tests/golden_http.rs` (re-target), `tests/label_coverage.rs` (the per-file `Method` expectation and the expected-set array), `tests/non_exhaustive_coverage.rs` if it lists `Method`; `examples/whip_lifecycle.rs`; fuzz target `fuzz/fuzz_targets/webrtc_ice.rs` is unaffected (ICE only)
- Modify: `.github/workflows/ci.yml` line 145 and the comment above it (remove `webrtc-runtime` from the `thumbv7em` list)

**Interfaces:**

```rust
// whip/client.rs and whep/player.rs (each has its own copy; same shape)
pub use http::Method;                         // replaces the local `Method` enum (BREAKING)
#[derive(Debug, Clone)]
pub struct HttpRequest { pub method: Method, pub url: String, pub headers: http::HeaderMap, pub body: Vec<u8> }
impl HttpRequest {
    pub fn content_type(&self) -> Option<headers::ContentType>;
    pub fn if_match(&self) -> Option<headers::IfMatch>;
}
#[derive(Debug, Clone)]
pub struct HttpResponse { pub status: http::StatusCode, pub headers: http::HeaderMap, pub body: Vec<u8> }
impl HttpResponse {
    pub fn new(status: http::StatusCode) -> Self;
    pub fn with_body(self, body: Vec<u8>) -> Self;
    pub fn with_content_type(self, mime: &str) -> Result<Self, Error>;     // headers::ContentType
    pub fn with_location(self, url: &str) -> Result<Self, Error>;          // headers::Location
    pub fn with_etag(self, opaque: &str) -> Result<Self, Error>;           // headers::ETag, strong, quoted
}
// whip/server.rs and whep/server.rs
pub struct HttpResponse { pub status: http::StatusCode, pub headers: http::HeaderMap, pub body: Vec<u8> }   // + the same helpers
WhipSession::on_patch(&mut self, sdp_fragment: Vec<u8>, headers: &http::HeaderMap) -> Result<Event, Error>   // reads typed If-Match
WhepSession::on_patch(&mut self, sdp_body: Vec<u8>, headers: &http::HeaderMap) -> Result<Event, Error>      // reads typed Content-Type + If-Match
WhepSession::no_publisher(retry_after: Option<std::time::Duration>) -> HttpResponse                          // typed Retry-After
// error.rs
Error::InvalidHeader { header: &'static str }     // a header value that does not parse as its typed form
// unchanged: State, Event, the `content_type`/`status` consts modules (multimux uses `whep::{content_type,status}`),
// WhipClient::{new,offer,flush_candidates,ice_restart,terminate,on_response}, WhipSession::{new,accept,ack_*,on_post,on_delete}, ...
```

Authentication stays a Bearer token string in the constructors; the request carries `Authorization: Bearer <t>` built with `headers::Authorization::bearer`. `Event::TrickleIce { if_match }` and `Event::IceRestart { new_etag }` keep their `String` opaque-tag payloads.

Behaviour differences (CHANGELOG, each with an example): `If-Match` is parsed as an RFC 9110 entity-tag list, so `If-Match: "*"` (quoted) and an unquoted `If-Match: etag1` are no longer accepted as "any" / a match (both are malformed per RFC 9110; `*` unquoted and `"etag1"` quoted are the valid forms) and now yield `Error::InvalidHeader`/`ETagMismatch`; a duplicated `Content-Type` is rejected by the typed decoder; header names are lower-case in `HeaderMap`.

- [ ] **Step 1: Write the failing tests (strictness, review-focus 4) and re-target the golden**

Add `webrtc-runtime/tests/whip_http_headers.rs`:

```rust
use headers::HeaderMapExt;
use http::{HeaderMap, HeaderValue, StatusCode, header};
use webrtc_runtime::Error;
use webrtc_runtime::whep::server::WhepSession;
use webrtc_runtime::whip::server::{Event, WhipSession};

const SESSION: &str = "https://o.example/s/1";

fn established_whip() -> WhipSession {
    let mut s = WhipSession::new(SESSION.into());
    let _ = s.accept(b"a".to_vec(), "etag1".into());
    s
}

fn h(name: header::HeaderName, v: &str) -> HeaderMap {
    let mut m = HeaderMap::new();
    m.insert(name, HeaderValue::from_str(v).unwrap());
    m
}

#[test]
fn if_match_star_means_ice_restart() {
    let mut s = established_whip();
    let ev = s.on_patch(b"f".to_vec(), &h(header::IF_MATCH, "*")).unwrap();
    assert!(matches!(ev, Event::IceRestart { .. }));
}

#[test]
fn if_match_with_the_current_strong_etag_is_trickle_ice() {
    let mut s = established_whip();
    let ev = s.on_patch(b"f".to_vec(), &h(header::IF_MATCH, "\"etag1\"")).unwrap();
    assert!(matches!(ev, Event::TrickleIce { if_match: Some(ref e), .. } if e == "etag1"));
}

#[test]
fn if_match_with_a_stale_etag_is_a_mismatch_carrying_both_values() {
    let mut s = established_whip();
    let err = s.on_patch(b"f".to_vec(), &h(header::IF_MATCH, "\"old\"")).unwrap_err();
    assert!(matches!(err, Error::ETagMismatch { ref expected, ref got } if expected == "etag1" && got == "\"old\""), "{err:?}");
}

#[test]
fn a_weak_etag_never_satisfies_if_match_strong_comparison() {
    let mut s = established_whip();
    assert!(matches!(
        s.on_patch(b"f".to_vec(), &h(header::IF_MATCH, "W/\"etag1\"")),
        Err(Error::ETagMismatch { .. })
    ));
}

#[test]
fn an_unquoted_etag_and_a_quoted_star_are_malformed() {
    let mut s = established_whip();
    assert!(matches!(s.on_patch(b"f".to_vec(), &h(header::IF_MATCH, "etag1")), Err(Error::InvalidHeader { header: "If-Match" })));
    assert!(matches!(s.on_patch(b"f".to_vec(), &h(header::IF_MATCH, "\"*\"")), Err(Error::ETagMismatch { .. })));
}

#[test]
fn a_patch_without_if_match_is_a_plain_trickle() {
    let mut s = established_whip();
    assert!(matches!(s.on_patch(b"f".to_vec(), &HeaderMap::new()).unwrap(), Event::TrickleIce { if_match: None, .. }));
}

#[test]
fn whep_counter_offer_answer_accepts_media_type_parameters_and_rejects_others() {
    let mut s = WhepSession::new(SESSION.into());
    let _ = s.counter_offer(b"o".to_vec(), None);
    let ok = s.on_patch(b"ans".to_vec(), &h(header::CONTENT_TYPE, "application/sdp; charset=utf-8"));
    assert!(ok.is_ok(), "{ok:?}");
    let mut s = WhepSession::new(SESSION.into());
    let _ = s.counter_offer(b"o".to_vec(), None);
    let bad = s.on_patch(b"x".to_vec(), &h(header::CONTENT_TYPE, "text/plain"));
    assert!(bad.is_err());
}

#[test]
fn responses_carry_typed_location_etag_and_content_type() {
    let mut s = WhipSession::new(SESSION.into());
    let r = s.accept(b"a".to_vec(), "etag1".into());
    assert_eq!(r.status, StatusCode::CREATED);
    assert_eq!(r.headers.get(header::LOCATION).unwrap(), SESSION);
    assert_eq!(r.headers.get(header::ETAG).unwrap(), "\"etag1\"");
    assert_eq!(r.headers.typed_get::<headers::ContentType>().unwrap().to_string(), "application/sdp");
}

#[test]
fn no_publisher_sets_retry_after_in_seconds() {
    let r = WhepSession::no_publisher(Some(std::time::Duration::from_secs(5)));
    assert_eq!(r.status, StatusCode::CONFLICT);
    assert_eq!(r.headers.get(header::RETRY_AFTER).unwrap(), "5");
    assert!(WhepSession::no_publisher(None).headers.get(header::RETRY_AFTER).is_none());
}

#[test]
fn a_non_ascii_location_is_an_invalid_header_not_a_panic() {
    let r = webrtc_runtime::whip::client::HttpResponse::new(StatusCode::CREATED).with_location("https://o.example/\u{1F600}");
    assert!(matches!(r, Err(Error::InvalidHeader { header: "Location" })));
}
```

Re-target `tests/golden_http.rs` to the typed API: the two macros become

```rust
fn render(headers: &http::HeaderMap) -> String {
    let mut lines: Vec<String> = headers.iter().map(|(k, v)| format!("{}: {}", k.as_str(), v.to_str().unwrap())).collect();
    lines.sort();
    lines.iter().map(|l| format!("  {l}\n")).collect()
}
// resp!: format!("{} -> {}\n{}  body={}B\n", label, r.status.as_u16(), render(&r.headers), r.body.len())
// req!:  format!("{} -> {} {}\n{}  body={}B\n", label, r.method, r.url, render(&r.headers), r.body.len())
```

and the `on_response(HttpResponse { status: 201, content_type: Some(..), location: .., etag: .., body })` calls become `HttpResponse::new(StatusCode::CREATED).with_content_type("application/sdp").unwrap().with_location(SESSION).unwrap().with_etag("etag1").unwrap().with_body(b"a".to_vec())` (and `HttpResponse::new(StatusCode::NO_CONTENT)`, `HttpResponse::new(StatusCode::OK).with_etag("etag9").unwrap().with_body(..)`). The golden FILE is not regenerated: it must match byte-for-byte except where recorded below.

- [ ] **Step 2: Run, expect FAIL**

```bash
cargo test --locked -p webrtc-runtime --all-features --test whip_http_headers --test golden_http 2>&1 | grep -E '^error|no variant|cannot find|test result' | head
```

Expected: compile errors (`InvalidHeader`, `HttpResponse::new`, `on_patch` arity).

- [ ] **Step 3: Implement**

`lib.rs`: remove the `no_std` attribute; `Cargo.toml`: `std = ["broadcast-common/std", "thiserror/std"]` stays (harmless; the crate is `std` regardless) and `thiserror = { version = "2" }`/`broadcast-common` drop `default-features = false` so the default (`std`) is on in every configuration. `error.rs`: add

```rust
    /// A header value could not be parsed as (or built into) its typed form.
    #[error("invalid {header} header")]
    InvalidHeader {
        /// The header name.
        header: &'static str,
    },
```

`http_util.rs`:

```rust
use headers::{ContentType, ETag, Header, HeaderMapExt, IfMatch, Location};
use http::{HeaderMap, HeaderValue};

use crate::Error;

pub(crate) fn etag(opaque: &str) -> Result<ETag, Error> {
    format!("\"{opaque}\"").parse().map_err(|_| Error::InvalidHeader { header: "ETag" })
}

/// The opaque tag of a typed ETag (quotes and any `W/` prefix removed).
pub(crate) fn etag_opaque(e: &ETag) -> String {
    let mut v: Vec<HeaderValue> = Vec::new();
    e.encode(&mut v);
    v.first()
        .and_then(|h| h.to_str().ok())
        .map(|s| s.trim_start_matches("W/").trim_matches('"').to_string())
        .unwrap_or_default()
}

pub(crate) fn content_type(mime: &str) -> Result<ContentType, Error> {
    mime.parse().map_err(|_| Error::InvalidHeader { header: "Content-Type" })
}

pub(crate) fn location(url: &str) -> Result<Location, Error> {
    let v = HeaderValue::from_str(url).map_err(|_| Error::InvalidHeader { header: "Location" })?;
    Location::decode(&mut std::iter::once(&v)).map_err(|_| Error::InvalidHeader { header: "Location" })
}

pub(crate) fn location_of(h: &HeaderMap) -> Option<String> {
    let l = h.typed_get::<Location>()?;
    let mut v: Vec<HeaderValue> = Vec::new();
    l.encode(&mut v);
    v.first().and_then(|h| h.to_str().ok()).map(str::to_string)
}

pub(crate) fn etag_of(h: &HeaderMap) -> Option<String> {
    h.typed_get::<ETag>().map(|e| etag_opaque(&e))
}

/// True if the request's Content-Type is `expected` by essence (parameters ignored, RFC 9110 §8.3.1).
pub(crate) fn content_type_is(h: &HeaderMap, expected: &str) -> bool {
    h.typed_get::<ContentType>()
        .map(headers::Mime::from)
        .is_some_and(|m| m.essence_str().eq_ignore_ascii_case(expected))
}

/// Shared `If-Match` evaluation for the two servers.
/// `Ok(None)` header absent, `Ok(Some(true))` `*`, `Ok(Some(false))` passes for `current`,
/// `Err(ETagMismatch)` mismatch, `Err(InvalidHeader)` malformed.
pub(crate) enum Precondition { Absent, Any, Passes }
pub(crate) fn check_if_match(h: &HeaderMap, current: &str) -> Result<Precondition, Error> {
    let raw = match h.get(http::header::IF_MATCH) {
        None => return Ok(Precondition::Absent),
        Some(v) => v,
    };
    let parsed = h.typed_try_get::<IfMatch>().map_err(|_| Error::InvalidHeader { header: "If-Match" })?
        .ok_or(Error::InvalidHeader { header: "If-Match" })?;
    if parsed.is_any() {
        return Ok(Precondition::Any);
    }
    if parsed.precondition_passes(&etag(current)?) {
        Ok(Precondition::Passes)
    } else {
        Err(Error::ETagMismatch { expected: current.to_string(), got: raw.to_str().unwrap_or("").to_string() })
    }
}
```

(`headers::Mime` re-export at `headers-0.4.2/src/lib.rs:79`; `HeaderMapExt::typed_try_get` at `map_ext.rs:40`; `IfMatch::{is_any, precondition_passes}` at `common/if_match.rs:54,62`.)

Client/player construction, e.g. in `whip/client.rs`:

```rust
fn build_request(&self, method: Method, url: String, content_type: Option<&'static str>, body: Vec<u8>) -> Result<HttpRequest, Error> {
    let mut headers = HeaderMap::new();
    if let Some(token) = &self.bearer_token {
        headers.typed_insert(headers::Authorization::bearer(token).map_err(|_| Error::InvalidHeader { header: "Authorization" })?);
    }
    if let Some(ct) = content_type {
        headers.typed_insert(http_util::content_type(ct)?);
    }
    Ok(HttpRequest { method, url, headers, body })
}
```

`flush_candidates` adds `headers.typed_insert(IfMatch::from(http_util::etag(&etag)?))`, `ice_restart` adds `headers.typed_insert(IfMatch::any())`. `handle_offer_response` reads `http_util::location_of(&resp.headers)` / `http_util::etag_of(&resp.headers)`; `handle_established_response` dispatches on `resp.status.as_u16()` (`200 | 204` arms become `StatusCode::OK | StatusCode::NO_CONTENT`); `Error::Http { status: resp.status.as_u16() }`. Server `accept` builds `HttpResponse::new(StatusCode::CREATED).with_content_type(content_type::SDP)?...` (builders return `Result`; the server methods keep returning `HttpResponse` by building from known-good literals with `expect`-free fallbacks: use a private infallible constructor `HttpResponse::from_parts(status, headers, body)` and insert pre-validated values: `headers.typed_insert(http_util::content_type("application/sdp").expect("literal"))` is a literal known-valid, so it is allowed to `unwrap_or_else(|_| unreachable!())`-free by storing the two `ContentType` values in `std::sync::LazyLock` statics: `static SDP: LazyLock<ContentType> = LazyLock::new(|| "application/sdp".parse().unwrap());`). Session URL and ETag come from the caller as `String`: `accept(&mut self, sdp_answer, etag: String) -> HttpResponse` keeps its signature; an invalid session URL or etag (non-ASCII) makes `accept` return a `500` `HttpResponse` with no Location, never a panic (add a test for it in `whip_http_headers.rs`: `WhipSession::new("https://o/\u{1F600}").accept(..).status == StatusCode::INTERNAL_SERVER_ERROR`).

`WhipSession::on_patch`:

```rust
        match &self.state {
            State::Established { etag } => match http_util::check_if_match(headers, etag)? {
                Precondition::Any => Ok(Event::IceRestart { sdp_fragment }),
                Precondition::Passes => Ok(Event::TrickleIce { sdp_fragment, if_match: Some(etag.clone()) }),
                Precondition::Absent => Ok(Event::TrickleIce { sdp_fragment, if_match: None }),
            },
            _ => Err(Error::WrongState { operation: "PATCH", state: server_state_name(&self.state) }),
        }
```

`WhepSession::on_patch` replaces `media_type_matches(content_type, X)` with `http_util::content_type_is(headers, X)` and the same `check_if_match`; `no_publisher(retry_after: Option<Duration>)` inserts `headers.typed_insert(headers::RetryAfter::delay(d))`; `counter_offer` builds the 406 `Content-Type` as `http_util::content_type(&format!("application/sdp; valid-until=\"{date}\""))` and `Location`. Delete the local `Method` enums and their `impl_spec_display!`; `pub use http::Method;` in both modules. In `label_coverage.rs` remove `Method` from the per-file expectations and from the expected-set array (the test's own message names the array); `non_exhaustive_coverage.rs` needs no change if it only lists `SKIP`.

Test conversion rules (mechanical; the assertions are unchanged):

| before | after |
|---|---|
| `client::HttpResponse { status: 201, content_type: Some("application/sdp".into()), location: Some(S.into()), etag: Some("e".into()), body: b.to_vec() }` | `client::HttpResponse::new(StatusCode::CREATED).with_content_type("application/sdp").unwrap().with_location(S).unwrap().with_etag("e").unwrap().with_body(b.to_vec())` |
| `client::HttpResponse { status: 204, content_type: None, location: None, etag: None, body: vec![] }` | `client::HttpResponse::new(StatusCode::NO_CONTENT)` |
| `req.method == client::Method::Post` | `req.method == http::Method::POST` |
| `req.content_type == Some("application/sdp")` | `req.content_type().unwrap().to_string() == "application/sdp"` |
| `req.headers.iter().find(\|(k, _)\| k == "If-Match")` ... `.1 == "\"etag1\""` | `req.headers.get(header::IF_MATCH).unwrap() == "\"etag1\""` |
| `resp.headers.iter().find(\|(k, _)\| k == "Location")` | `resp.headers.get(header::LOCATION)` |
| `server.on_patch(frag, Some("\"etag1\""))` | `server.on_patch(frag, &if_match_headers("\"etag1\""))` with a local `fn if_match_headers(v: &str) -> HeaderMap` helper |
| `resp.status == 201` | `resp.status == StatusCode::CREATED` |
| `resp.headers.is_empty()` | `resp.headers.is_empty()` (unchanged) |

`grep -n 'HttpResponse {\|\.content_type\|\.headers\|on_patch(\|Method::' webrtc-runtime/tests/whip_round_trip.rs webrtc-runtime/tests/whep_round_trip.rs webrtc-runtime/examples/whip_lifecycle.rs` lists every site (about 60); convert all, then `cargo test` until green. `examples/whip_lifecycle.rs`: `HttpResponse::new(StatusCode::CREATED).with_content_type(..)...` and print `offer_req.content_type()`.

Where the typed `Content-Type` of a 406 counter-offer re-serialises differently from the old raw string (`application/sdp; valid-until="2030-01-01T00:00:00Z"`), record the exact old/new lines from the golden diff in the report; they are CHANGELOG examples. Any other golden difference is a bug in the conversion: fix the code, do not edit the golden.

CI workflow (manual, never regex): in `.github/workflows/ci.yml` line 145 remove exactly the token ` webrtc-runtime` from the `for c in ...; do` list, and in the comment block above add after the `Deliberately NOT here` list entry: ``# `webrtc-runtime` left the list at W1 (SP2.2): its WHIP/WHEP core is `std` (`http`/`headers`).`` Do it with the Edit tool using the exact old/new strings (` compliance-probe webrtc-runtime ts-fix; do` to ` compliance-probe ts-fix; do`), then verify:

```bash
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml'))" && echo yaml-ok
git diff --stat .github
cargo build --locked -p webrtc-runtime --no-default-features 2>&1 | tail -1
```

Expected: `yaml-ok`, one file changed with at most 3 lines, the host `--no-default-features` build passes. Flag this workflow edit for the orchestrator's review (Escalation 5).

- [ ] **Step 4: Run, expect PASS**

```bash
cargo test --locked -p webrtc-runtime --all-features 2>&1 | grep -E '^test result|FAILED|panicked|^error' | sort | uniq -c
cargo build --locked -p multimux --all-features 2>&1 | tail -2
cargo test --locked -p webrtc-runtime --all-features --doc 2>&1 | grep -E 'test result|FAILED'
```

Expected: all ok (`golden_http` byte-identical except the recorded differences, `whip_http_headers` 10 tests, converted round-trip suites); multimux builds (it only uses `whep::{content_type, status}` and `media`).

- [ ] **Step 5: Revert-check**

Restore the old `on_patch` matching for one run: in `check_if_match` replace the typed parse with `if matches!(raw.to_str(), Ok("*") | Ok("\"*\"")) { return Ok(Precondition::Any) }` and `let client = raw.to_str().unwrap_or("").trim_matches('"'); if client == current {Passes}`. Run `whip_http_headers`: expect `an_unquoted_etag_and_a_quoted_star_are_malformed` and `a_weak_etag_never_satisfies...` FAIL (the old code accepted both). Restore.

- [ ] **Step 6: Commit**

```bash
git add webrtc-runtime .github/workflows/ci.yml
git commit -m "feat(webrtc-runtime)!: WHIP/WHEP state machines on http/headers types; core goes std; webrtc-runtime leaves the thumbv7em CI list"
```

---

### Task 10: SP7 for webrtc-runtime: deadline-driven test pumps, axum + tokio example

**Files:**
- Modify: `webrtc-runtime/tests/dtls_fingerprint.rs` (`pump`, 100-165: `std::thread::sleep(5ms)`)
- Modify: `webrtc-runtime/src/media/transport.rs` tests (loopback `pump`/`std::thread::sleep(2ms)` at ~2545)
- Rewrite: `webrtc-runtime/examples/whip_media_smoke.rs` (hand-rolled HTTP reader, fixed `SIGNALLING_PORT`, 200 ms socket read timeout)

**Interfaces:** none new (uses Task 7's `poll_timeout`).

- [ ] **Step 1: Find the waits (the failing check)**

```bash
grep -nE 'thread::sleep|sleep\(|SIGNALLING_PORT|set_read_timeout|windows\(4\)' webrtc-runtime/tests/*.rs webrtc-runtime/src/media/transport.rs webrtc-runtime/examples/*.rs
```

Expected: `dtls_fingerprint.rs:157`, `transport.rs:~2545`, and the example's reader/port/timeout.

- [ ] **Step 2: Replace the sleeping pumps with virtual-time pumps**

Both pumps share one shape: when a round makes no progress, jump a VIRTUAL clock to the next deadline instead of sleeping.

```rust
fn pump(a: &mut MediaTransport, b: &mut MediaTransport, a_addr: SocketAddr, b_addr: SocketAddr, budget: Duration) -> Pumped {
    let t0 = Instant::now();
    let mut now = t0;
    let mut pumped = Pumped::default();
    loop {
        let mut progressed = false;
        while let Some(d) = a.poll_transmit() { /* deliver to b with `now`, record, progressed = true */ }
        while let Some(d) = b.poll_transmit() { /* deliver to a with `now` */ }
        if done(&pumped) || now - t0 > budget {
            return pumped;
        }
        if !progressed {
            // Nothing in flight: advance the virtual clock to whichever transport is due next.
            let next = [a.poll_timeout(), b.poll_timeout()].into_iter().flatten().min();
            let Some(next) = next else { return pumped };
            now = next.max(now);
            pumped.a_events.extend(a.handle_timeout(now));
            pumped.b_events.extend(b.handle_timeout(now));
        }
    }
}
```

(Keep each file's existing bookkeeping inside the two `while let` loops and the existing completion predicate; only the `thread::sleep` branch and the wall-clock `Instant::now()` passed to `handle_datagram`/`handle_timeout` change to `now`. `Pumped` gets `#[derive(Default)]` if it lacks it.) The `budget` of 3 s becomes 3 s of VIRTUAL time, so the tests finish in milliseconds and are deterministic.

- [ ] **Step 3: Rewrite the example on axum + tokio**

New skeleton (`required-features = ["media"]` stays; `axum` and `tokio` are dev-dependencies, already in the manifest from Task 2 and the existing `tokio = { features = ["full"] }`):

```rust
use std::net::SocketAddr;
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::post;
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::{mpsc, oneshot};
use webrtc_runtime::media::{MAX_REMOTE_CANDIDATES, MediaEvent, MediaTransport, MediaTransportConfig, SetupRole, parse_remote_fingerprint};

struct Offer { sdp: String, answer: oneshot::Sender<String> }

async fn whip(State(tx): State<mpsc::Sender<Offer>>, body: String) -> impl IntoResponse {
    let (answer, rx) = oneshot::channel();
    if tx.send(Offer { sdp: body, answer }).await.is_err() {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    match rx.await {
        Ok(sdp) => (
            StatusCode::CREATED,
            [
                (header::CONTENT_TYPE, "application/sdp"),
                (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
                (header::ACCESS_CONTROL_EXPOSE_HEADERS, "Location"),
                (header::LOCATION, "/whip/1"),
            ],
            sdp,
        )
            .into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn preflight() -> impl IntoResponse {
    (
        StatusCode::NO_CONTENT,
        [
            (header::ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*")),
            (header::ACCESS_CONTROL_ALLOW_METHODS, HeaderValue::from_static("POST, OPTIONS")),
            (header::ACCESS_CONTROL_ALLOW_HEADERS, HeaderValue::from_static("Content-Type")),
        ],
    )
}

#[tokio::main]
async fn main() {
    // Signalling port: first argument, default 8787; `0` picks a free port and prints it.
    let port: u16 = std::env::args().nth(1).and_then(|p| p.parse().ok()).unwrap_or(8787);
    let listener = TcpListener::bind(("127.0.0.1", port)).await.expect("bind whip-lite http port");
    println!("[smoke] WHIP-lite signalling on http://{}/whip", listener.local_addr().unwrap());
    let (tx, mut offers) = mpsc::channel::<Offer>(1);
    let app = Router::new().route("/whip", post(whip).options(preflight)).with_state(tx);
    tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });

    let udp = UdpSocket::bind("127.0.0.1:0").await.expect("bind udp");
    let local_addr = udp.local_addr().unwrap();
    let Offer { sdp: offer_sdp, answer } = offers.recv().await.expect("an offer");
    /* parse ufrag/pwd/mid/candidates with sdp_types::Session::parse(offer_sdp.as_bytes()) typed accessors,
       remote_fingerprint = parse_remote_fingerprint(&offer_sdp).expect("offer has a=fingerprint") */
    let mut media = MediaTransport::new(MediaTransportConfig { /* as before */ }, std::time::Instant::now()).expect("transport");
    /* answer = the offer Session with direction recvonly and the local ice-ufrag/ice-pwd/fingerprint/setup/
       candidate attributes substituted, written with `Session::write` into a Vec<u8>; local candidate is
       `media.local_candidates().remove(0)` */
    let _ = answer.send(answer_sdp);

    // Media loop: socket read, outbound drain, and `poll_timeout` deadline, no fixed tick.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut buf = [0u8; 2048];
    let mut success = false;
    while !success && tokio::time::Instant::now() < deadline {
        while let Some(d) = media.poll_transmit() {
            let _ = udp.send_to(&d.bytes, d.peer).await;
        }
        let wake = media.poll_timeout().map(tokio::time::Instant::from_std).unwrap_or(deadline).min(deadline);
        tokio::select! {
            r = udp.recv_from(&mut buf) => { /* handle_datagram(std::time::Instant::now(), peer, &buf[..n]) and print events as before; success on MediaEvent::Rtp */ }
            _ = tokio::time::sleep_until(wake) => { for e in media.handle_timeout(std::time::Instant::now()) { /* print TimerError */ } }
        }
    }
    /* SUCCESS / TIMED OUT reporting and exit code as before */
}
```

The elided `/* ... */` blocks are the existing example's bodies moved verbatim (event printing, config fields); the SDP is read with `sdp_types::Session::parse` (`Media::get_first_attribute_value("ice-ufrag")`, `mid`, `attributes_typed::<...>`) and the answer is the parsed offer with its `a=` attributes replaced by pushing `sdp_types::Attribute { attribute: "ice-ufrag".into(), value: Some(ufrag) }` entries (fields are public, `sdp-types-0.2.0/src/lib.rs` `Attribute`) and `Session::write(&mut Vec<u8>)` (`writer.rs:180`): no `format!`-built SDP lines remain. Add `sdp-types = "0.2"` to `[dev-dependencies]` (the example runs with `media`, which already depends on it, but examples resolve dev-dependencies).

- [ ] **Step 4: Run, expect PASS, and 20-run stability**

```bash
cargo build --locked -p webrtc-runtime --all-features --examples 2>&1 | tail -2
cargo test --locked -p webrtc-runtime --all-features 2>&1 | grep -E '^test result|FAILED|panicked|^error' | sort | uniq -c
for i in $(seq 1 20); do cargo test --locked -p webrtc-runtime --all-features --test dtls_fingerprint --lib media:: 2>&1 | grep -E '^test result: FAILED|panicked|^error'; done | sort | uniq -c
```

Expected: the example builds; suites ok; the 20-run loop prints nothing (record `20/20`). The example itself needs a browser and is not run in CI (its header says so); run it once manually with a free port to confirm it starts and prints the listen address: `cargo run -p webrtc-runtime --features media --example whip_media_smoke -- 0` then Ctrl-C.

- [ ] **Step 5: Commit**

```bash
git add webrtc-runtime
git commit -m "test(webrtc-runtime): virtual-time pumps driven by poll_timeout; whip_media_smoke on axum + tokio with sdp-types"
```

---

### Task 11: hls `Url::join` and query building (SP3, defect 7)

**Files:**
- Rewrite: `hls-runtime/src/client/url.rs` (115 lines)
- Modify: `hls-runtime/src/client/action.rs` lines 107-129 (`playlist_request_url`)
- Unchanged callers: `engine.rs` lines 438, 739, 755, 774, 783, 1090, 1094, 1124, 1126 and `tokio_client.rs` `note_preload_hint` all call `url::resolve(base, uri) -> String`
- Test: unit tests in `url.rs`; `hls-runtime/tests/golden_client.rs` (unchanged, must still pass)

**Interfaces:**

```rust
pub(crate) fn resolve(base: &str, uri: &str) -> String;                       // signature unchanged
pub(crate) fn append_pair(url: &str, key: &str, value: &str) -> String;       // replaces append_query(url, "k=v")
```

`no_std`+`alloc` stays (the `url` crate is a `default-features = false` dependency of the core). A base that is not an absolute URL is resolved against a synthetic base `hls-relative:///` (SP3 option (a)); the synthetic prefix is stripped from the result, so a relative base yields a relative result. A relative reference that climbs above the root is clamped at the root, like any RFC 3986 resolver.

Behaviour changes (CHANGELOG, each with an example): results are RFC 3986-normalised (`HTTP://H/x` becomes `http://h/x`, `..` segments are removed, spaces are percent-encoded, a default port is dropped); a reference whose QUERY contains `://` is no longer mistaken for an absolute URL.

- [ ] **Step 1: Write the failing tests**

Replace the test module of `url.rs` (keep the five existing resolution tests verbatim, they must still pass) and add:

```rust
    #[test]
    fn dot_dot_segments_are_resolved_defect_7() {
        assert_eq!(resolve("http://h/a/b/p.m3u8", "../x.m4s"), "http://h/a/x.m4s");
        assert_eq!(resolve("http://h/a/b/p.m3u8", "../../x.m4s"), "http://h/x.m4s");
        assert_eq!(resolve("http://h/a/b/p.m3u8", "./c/../d.m4s"), "http://h/a/b/d.m4s");
        assert_eq!(resolve("http://h/a/p.m3u8", "../../../x.m4s"), "http://h/x.m4s", "cannot climb above the root");
        assert_eq!(resolve("http://h/a/p.m3u8", "/z/../y.m4s"), "http://h/y.m4s", "absolute-path references are normalised too");
    }

    /// The old `uri.contains("://")` shortcut returned a relative reference unchanged
    /// when its query happened to contain a URL.
    #[test]
    fn a_scheme_separator_inside_the_query_does_not_make_a_reference_absolute() {
        assert_eq!(
            resolve("http://h/a/p.m3u8", "seg.m4s?redirect=http://x/y"),
            "http://h/a/seg.m4s?redirect=http://x/y"
        );
    }

    #[test]
    fn query_only_and_fragment_references_and_a_base_with_query_and_fragment() {
        assert_eq!(resolve("http://h/a/p.m3u8?x=1#top", "?y=2"), "http://h/a/p.m3u8?y=2");
        assert_eq!(resolve("http://h/a/p.m3u8?x=1#top", "seg.m4s"), "http://h/a/seg.m4s");
        assert_eq!(resolve("http://h:8080/a/p.m3u8", "seg.m4s"), "http://h:8080/a/seg.m4s");
    }

    #[test]
    fn normalisation_is_applied_and_listed() {
        assert_eq!(resolve("http://h/a/p.m3u8", "HTTP://H.Example/X"), "http://h.example/X");
        assert_eq!(resolve("http://h/a/p.m3u8", "my seg.m4s"), "http://h/a/my%20seg.m4s");
        assert_eq!(resolve("http://h:80/a/p.m3u8", "s.m4s"), "http://h/a/s.m4s", "default port dropped");
    }

    #[test]
    fn a_relative_base_resolves_against_a_synthetic_base_that_is_stripped() {
        assert_eq!(resolve("live/stream.m3u8", "seg.m4s"), "live/seg.m4s");
        assert_eq!(resolve("live/stream.m3u8", "../x.m4s"), "x.m4s");
        assert_eq!(resolve("/live/stream.m3u8", "seg.m4s"), "/live/seg.m4s");
        assert_eq!(resolve("live/stream.m3u8", "/abs/x.m4s"), "/abs/x.m4s");
        assert_eq!(resolve("live/stream.m3u8", "http://cdn/x.m4s"), "http://cdn/x.m4s");
    }

    #[test]
    fn append_pair_adds_one_encoded_pair_and_keeps_the_existing_query_verbatim() {
        assert_eq!(append_pair("http://h/p.m3u8", "a", "1"), "http://h/p.m3u8?a=1");
        assert_eq!(append_pair("http://h/p.m3u8?a=1", "b", "2"), "http://h/p.m3u8?a=1&b=2");
        assert_eq!(append_pair("http://h/p.m3u8?a=x%20y&z", "b", "2"), "http://h/p.m3u8?a=x%20y&z&b=2");
        assert_eq!(append_pair("http://h/p.m3u8", "k", "a b&c"), "http://h/p.m3u8?k=a+b%26c");
        assert_eq!(append_pair("p.m3u8", "a", "1"), "p.m3u8?a=1", "relative playlist URL keeps working");
    }
```

- [ ] **Step 2: Run, expect FAIL**

```bash
cargo test --locked -p hls-runtime --all-features --lib client::url 2>&1 | grep -E '^error|cannot find|FAILED|test result' | head
```

Expected: `append_pair` not found (compile error); with a stub, `dot_dot_segments_are_resolved_defect_7` FAILS (`left: "http://h/a/b/../x.m4s"`) and `a_scheme_separator_inside_the_query...` FAILS (`left: "seg.m4s?redirect=http://x/y"`).

- [ ] **Step 3: Implement**

```rust
//! URI resolution and query building for the HLS client core, on the `url` crate
//! (RFC 3986 §5 reference resolution).

use alloc::string::{String, ToString};

use url::Url;

/// Base used when the playlist URL is itself relative; stripped from results.
const SYNTHETIC_BASE: &str = "hls-relative:///";
const SYNTHETIC_ROOT: &str = "hls-relative://";

/// Resolve `uri` (as it appears in a playlist) against `base` (the URL the playlist was fetched from).
pub(crate) fn resolve(base: &str, uri: &str) -> String {
    let (base_url, synthetic) = match Url::parse(base) {
        Ok(u) => (u, false),
        Err(_) => match Url::parse(SYNTHETIC_BASE).and_then(|s| s.join(base)) {
            Ok(u) => (u, true),
            Err(_) => return uri.to_string(),
        },
    };
    match base_url.join(uri) {
        Ok(joined) if synthetic => strip_synthetic(joined.as_str(), base.starts_with('/') || uri.starts_with('/')),
        Ok(joined) => joined.to_string(),
        Err(_) => uri.to_string(),
    }
}

fn strip_synthetic(s: &str, rooted: bool) -> String {
    let rest = s.strip_prefix(SYNTHETIC_ROOT).unwrap_or(s);
    if rooted { rest.to_string() } else { rest.strip_prefix('/').unwrap_or(rest).to_string() }
}

/// Append one `key=value` pair (form-urlencoded) to `url`'s query; the existing query is kept verbatim.
pub(crate) fn append_pair(url: &str, key: &str, value: &str) -> String {
    let (mut u, synthetic) = match Url::parse(url) {
        Ok(u) => (u, false),
        Err(_) => match Url::parse(SYNTHETIC_BASE).and_then(|s| s.join(url)) {
            Ok(u) => (u, true),
            Err(_) => return alloc::format!("{url}{}{key}={value}", if url.contains('?') { '&' } else { '?' }),
        },
    };
    u.query_pairs_mut().append_pair(key, value);
    if synthetic { strip_synthetic(u.as_str(), url.starts_with('/')) } else { u.to_string() }
}
```

`action.rs::playlist_request_url` becomes:

```rust
                let mut u = url.clone();
                if let Some(b) = blocking {
                    u = super::url::append_pair(&u, "_HLS_msn", &b.msn.to_string());
                    if let Some(part) = b.part {
                        u = super::url::append_pair(&u, "_HLS_part", &part.to_string());
                    }
                }
                if *skip {
                    u = super::url::append_pair(&u, "_HLS_skip", "YES");
                }
                Some(u)
```

(`use alloc::string::ToString;` there; `Url::join`, `Url::query_pairs_mut().append_pair` per `url-2.5.8`.)

- [ ] **Step 4: Run, expect PASS; the golden is unchanged**

```bash
cargo test --locked -p hls-runtime --all-features 2>&1 | grep -E '^test result|FAILED|panicked|^error' | sort | uniq -c
cargo build --locked -p hls-runtime --no-default-features --target thumbv7em-none-eabi 2>&1 | tail -1
```

Expected: all ok EXCEPT `golden_client`, which differs in exactly three lines: the `matrix http://h/p.m3u8?a=1&b=2#f` rows that append a pair (`blocking=Some(..) skip=false`, `blocking=Some(..part..) skip=true`, `blocking=None skip=true`). The old string builder appended the pair AFTER the fragment (`...#f&_HLS_msn=5`, a latent defect: the pair becomes part of the fragment and is never sent); `query_pairs_mut` inserts it before the fragment (`...&_HLS_msn=5#f`). Verify with:

```bash
GOLDEN_UPDATE=1 cargo test --locked -p hls-runtime --all-features --test golden_client 2>&1 | grep 'test result'
git diff --stat hls-runtime/tests/golden
git diff hls-runtime/tests/golden/client_urls.golden | grep '^[-+] ' | wc -l
```

Expected: one file, 6 changed lines (3 old, 3 new), and every `base ...` / `request_url` row from the engine unchanged. Any other difference is a normalisation difference to be listed in the report and CHANGELOG with an example, or a bug to fix. Commit the regenerated golden in this task's commit and paste the old/new lines into the report. The thumb build must still pass.

- [ ] **Step 5: Revert-check (defect 7)**

```bash
git show $BASE:hls-runtime/src/client/url.rs > /tmp/old_url.rs
```

Temporarily make `resolve` call the old relative-join branch (`format!("{}{uri}", &base_no_query[..=idx])`) for non-absolute `uri`, run `cargo test --locked -p hls-runtime --all-features --lib client::url`: expect `dot_dot_segments_are_resolved_defect_7` and `a_scheme_separator_inside_the_query...` FAIL. Record the failing assertions and restore.

- [ ] **Step 6: Commit**

```bash
git add hls-runtime
git commit -m "fix(hls-runtime): resolve HLS references with Url::join (.. segments, :// in queries) and build queries with query_pairs_mut"
```

---

### Task 12: hls dates via `jiff` (SP5)

**Files:**
- Modify: `hls-runtime/src/client/engine.rs` lines 80-174 (`program_date_time`, `days_from_civil`, `parse_rfc3339_ms`) and the test at 1615-1660

**Interfaces:** `fn parse_rfc3339_ms(text: &str) -> Option<i64>` keeps its private signature; `days_from_civil` is deleted.

- [ ] **Step 1: Write the failing tests (the existing vector test stays verbatim and must keep passing)**

Append to the engine test module:

```rust
    #[test]
    fn rfc3339_accepts_the_spellings_the_old_parser_accepted() {
        assert_eq!(parse_rfc3339_ms("2026-10-02 10:00:00Z"), Some(1_790_935_200_000), "space separator");
        assert_eq!(parse_rfc3339_ms("2026-10-02t10:00:00z"), Some(1_790_935_200_000), "lower-case t and z");
        assert_eq!(parse_rfc3339_ms("2026-10-02T10:00:00.123456789Z"), Some(1_790_935_200_123), "extra digits truncated");
        assert_eq!(parse_rfc3339_ms("1969-12-31T23:59:59Z"), Some(-1_000), "before the epoch");
    }

    /// A leap second is clamped to :59 (jiff), where the old parser added 60 s to the minute.
    #[test]
    fn a_leap_second_is_clamped_to_the_last_second_of_the_minute() {
        assert_eq!(parse_rfc3339_ms("2016-12-31T23:59:60Z"), Some(1_483_228_799_000));
    }
```

- [ ] **Step 2: Run, expect FAIL**

```bash
cargo test --locked -p hls-runtime --all-features --lib rfc3339 leap_second 2>&1 | grep -E 'FAILED|left|right|test result' | head
```

Expected: `a_leap_second_is_clamped...` FAILS (old: `1483228800000`); the other new test and the original vectors pass on the old code (they pin behaviour that must survive the swap).

- [ ] **Step 3: Implement**

```rust
/// Parse an RFC 3339 / ISO 8601 date-time into Unix milliseconds; `None` for anything that does
/// not parse (never a guess). Delegates to `jiff::Timestamp` (an offset is required).
fn parse_rfc3339_ms(text: &str) -> Option<i64> {
    text.parse::<jiff::Timestamp>().ok().map(|t| t.as_millisecond())
}
```

Delete `days_from_civil` and the old body. (`jiff-0.2.37/src/timestamp.rs:2353` `FromStr for Timestamp`, `as_millisecond`; the default parser accepts `T`/`t`/space, `Z`/`z`, offsets with or without a colon (`fmt/offset.rs:331` `Colon::Optional`) and rejects a missing offset.)

- [ ] **Step 4: Run, expect PASS**

```bash
cargo test --locked -p hls-runtime --all-features 2>&1 | grep -E '^test result|FAILED|panicked|^error' | sort | uniq -c
grep -rnE 'civil_from_days|days_from_civil' hls-runtime/src | head -3
cargo build --locked -p hls-runtime --no-default-features --target thumbv7em-none-eabi 2>&1 | tail -1
```

Expected: all ok (`rfc3339_parses_to_literal_epoch_milliseconds` unchanged: offsets `+02:00` and `-0500`, the whole bad-input list); the grep prints nothing; thumb passes.

- [ ] **Step 5: Revert-check**

Restore the old function from `git show $BASE:hls-runtime/src/client/engine.rs` for one run: `a_leap_second_is_clamped...` FAILS (that is the behaviour difference, listed in the CHANGELOG). Restore.

- [ ] **Step 6: Commit**

```bash
git add hls-runtime
git commit -m "refactor(hls-runtime): EXT-X-PROGRAM-DATE-TIME via jiff::Timestamp; days_from_civil removed"
```

---

### Task 13: hls `TokioClient`: `headers::Range`, `backon`, cancellation, no 10 ms sleep; `HlsClient::next_wait`/`poll_timeout` (SP1.2-1.5, SP2.6)

**Files:**
- Modify: `hls-runtime/src/client/engine.rs`: add `next_wait` and `poll_timeout` after `poll` (line 340)
- Modify: `hls-runtime/src/client/tokio_client.rs`: `TokioError` (57-92), `TokioClientConfig` (94-135), `next_output` (170-260: the `WaitMs` sleep and the `None => sleep(10ms)` arm), `fetch_playlist_resilient` (400-420), `fetch_resource_bounded` (421-445), `build_request` (467-490), `with_config` (140-168); tests at 495-690
- Modify: `hls-runtime/src/client/mod.rs` docs (74 lines)
- Test: unit tests in `engine.rs` and `tokio_client.rs`, `hls-runtime/tests/tokio_client_cancel.rs` (new)

**Interfaces:**

```rust
// engine.rs (no_std core)
impl HlsClient {
    /// The `WaitMs` hint at the front of the action queue, as a `Duration`; `None` when the next
    /// queued action is a fetch (or nothing is queued). The core never reads a clock.
    pub fn next_wait(&self) -> Option<core::time::Duration>;
    /// `now + next_wait()` for callers that schedule on `std::time::Instant` (feature `std`).
    #[cfg(feature = "std")]
    pub fn poll_timeout(&self, now: std::time::Instant) -> Option<std::time::Instant>;
}
// tokio_client.rs
#[derive(Debug, Clone)]
pub struct TokioClientConfig {
    pub request_timeout: Duration,        // existing, 5 s
    pub blocking_timeout: Duration,       // existing, 10 s
    pub max_resource_retries: u32,        // existing, 3 (total attempts)
    pub retry_backoff: Duration,          // existing, 200 ms: now backon's min_delay
    pub max_retry_backoff: Duration,      // existing, 2 s: now backon's max_delay
    pub auth: Option<Credentials>,        // existing
    pub connect_timeout: Duration,        // NEW, 10 s: TCP connect (reqwest `connect_timeout`)
    pub jitter: bool,                     // NEW, true: backon jitter (each delay in [d, 2d))
    pub cancel: tokio_util::sync::CancellationToken,   // NEW: cancelled => `next_output` returns Ok(None)
}
impl TokioClientConfig { pub fn with_auth(self, c: Credentials) -> Self; with_cancel(self, t: CancellationToken) -> Self;
                         with_connect_timeout; with_jitter(self, on: bool) -> Self }
TokioError::Stalled        // NEW #[non_exhaustive] variant: the core had nothing to do and the stream had not ended
```

`reqwest` stays at 0.12 for this crate: `reqwest::Error` is in `TokioError`'s public API, so bumping to 0.13 is a breaking change that must move together with multimux (W2). Escalation 3.

- [ ] **Step 1: Write the failing tests**

`engine.rs`:

```rust
    #[test]
    fn next_wait_reports_the_queued_wait_hint_without_consuming_it() {
        use core::time::Duration;
        let mut c = HlsClient::new("http://h/p.m3u8");
        assert_eq!(c.next_wait(), None, "the first queued action is a fetch");
        let _ = c.poll();
        c.on_playlist(b"#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n#EXTINF:2.0,\nseg0.ts\n").unwrap();
        while c.next_wait().is_none() {
            c.poll().expect("a WaitMs must be queued after a non-blocking live playlist");
        }
        assert_eq!(c.next_wait(), Some(Duration::from_millis(1000)), "half the 2 s target duration");
        assert_eq!(c.next_wait(), Some(Duration::from_millis(1000)), "peeking does not consume");
        let now = std::time::Instant::now();
        assert_eq!(c.poll_timeout(now), Some(now + Duration::from_millis(1000)));
    }
```

`tokio_client.rs` unit tests (private helpers):

```rust
    #[test]
    fn retry_schedule_without_jitter_is_exponential_and_capped() {
        let cfg = TokioClientConfig::default().with_jitter(false);
        let d: Vec<_> = schedule(&cfg, None).take(6).collect();
        assert_eq!(
            d,
            [200, 400, 800, 1600, 2000, 2000].map(Duration::from_millis),
            "min 200 ms, doubling, capped at 2 s"
        );
    }

    #[test]
    fn retry_schedule_jitter_stays_within_one_extra_delay_and_resource_attempts_are_bounded() {
        let cfg = TokioClientConfig::default();
        for (i, got) in schedule(&cfg, None).take(8).enumerate() {
            let base = Duration::from_millis(200u64 << i.min(4)).min(Duration::from_secs(2));
            assert!(got >= base && got < base * 2, "delay {i} = {got:?} outside [{base:?}, {:?})", base * 2);
        }
        assert_eq!(schedule(&cfg, Some(2)).count(), 2, "3 attempts = 2 sleeps");
    }
```

`build_request`'s two existing tests (`build_request_sets_range_header_for_a_normal_byte_range` expecting `bytes=10-29`, and the `u64` overflow rejection) stay unchanged and must pass.

`hls-runtime/tests/tokio_client_cancel.rs`:

```rust
#![cfg(feature = "tokio")]

use std::time::Duration;

use hls_runtime::client::tokio_client::{TokioClient, TokioClientConfig};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

/// A token cancelled BEFORE the first call ends the stream at once: no request is made.
#[tokio::test]
async fn a_pre_cancelled_client_returns_end_of_stream_without_any_io() {
    let cancel = CancellationToken::new();
    cancel.cancel();
    let mut c = TokioClient::with_config("http://127.0.0.1:1/never.m3u8", TokioClientConfig::default().with_cancel(cancel)).unwrap();
    let out = tokio::time::timeout(Duration::from_secs(5), c.next_output()).await.expect("must not block").unwrap();
    assert!(out.is_none());
}

/// A request that would block for ever (the origin accepts and says nothing) must be abandoned
/// as soon as the token is cancelled, not after `request_timeout`.
#[tokio::test]
async fn cancelling_aborts_an_in_flight_request() {
    let hole = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = hole.local_addr().unwrap().port();
    let _keep = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((s, _)) = hole.accept().await {
            held.push(s); // accept, never answer
        }
    });
    let cancel = CancellationToken::new();
    let cfg = TokioClientConfig {
        request_timeout: Duration::from_secs(300),
        blocking_timeout: Duration::from_secs(300),
        ..TokioClientConfig::default().with_cancel(cancel.clone())
    };
    let mut c = TokioClient::with_config(format!("http://127.0.0.1:{port}/p.m3u8"), cfg).unwrap();
    let task = tokio::spawn(async move { c.next_output().await });
    cancel.cancel();
    let out = tokio::time::timeout(Duration::from_secs(10), task).await.expect("cancel must abort the request").unwrap().unwrap();
    assert!(out.is_none());
}
```

(`TokioClientConfig` is a plain struct with public fields, so struct-update syntax works in tests.)

- [ ] **Step 2: Run, expect FAIL**

```bash
cargo test --locked -p hls-runtime --all-features --lib next_wait retry_schedule --test tokio_client_cancel 2>&1 | grep -E '^error|cannot find|no method|FAILED|test result' | head
```

Expected: compile errors (`next_wait`, `schedule`, `with_jitter`, `with_cancel`).

- [ ] **Step 3: Implement**

`engine.rs`:

```rust
    pub fn next_wait(&self) -> Option<core::time::Duration> {
        match self.pending_actions.front() {
            Some(Action::WaitMs(ms)) => Some(core::time::Duration::from_millis(*ms)),
            _ => None,
        }
    }

    #[cfg(feature = "std")]
    pub fn poll_timeout(&self, now: std::time::Instant) -> Option<std::time::Instant> {
        self.next_wait().map(|d| now + d)
    }
```

`tokio_client.rs`:

```rust
use backon::{BackoffBuilder, ExponentialBuilder};
use headers::HeaderMapExt;
use tokio_util::sync::CancellationToken;

/// The retry delays for `config`: `backon` exponential (min = `retry_backoff`, cap = `max_retry_backoff`),
/// optional jitter; `max_sleeps = None` retries for ever (playlist reloads).
fn schedule(config: &TokioClientConfig, max_sleeps: Option<usize>) -> impl Iterator<Item = Duration> + use<> {
    let mut b = ExponentialBuilder::default()
        .with_min_delay(config.retry_backoff)
        .with_max_delay(config.max_retry_backoff);
    if config.jitter {
        b = b.with_jitter();
    }
    b = match max_sleeps {
        Some(n) => b.with_max_times(n),
        None => b.without_max_times(),
    };
    b.build()
}
```

Cancellation helper and use sites:

```rust
impl TokioClient {
    /// Sleeps `d` or until cancelled; `true` = cancelled.
    async fn sleep_or_cancelled(&self, d: Duration) -> bool {
        tokio::select! {
            _ = tokio::time::sleep(d) => false,
            _ = self.config.cancel.cancelled() => true,
        }
    }
}
```

`next_output`: at the top of the loop `if self.config.cancel.is_cancelled() { return Ok(None); }`. Wrap each network await in `fetch_bytes` with `tokio::select! { r = send_and_read => r, _ = self.config.cancel.cancelled() => return Err(TokioError::Cancelled) }`, adding a private `TokioError::Cancelled` that `next_output` maps to `Ok(None)` (add the variant `#[doc(hidden)]`, it never escapes: `next_output` converts it). The two sleeps become:

```rust
                Some(Action::WaitMs(ms)) => {
                    if self.sleep_or_cancelled(Duration::from_millis(ms)).await {
                        return Ok(None);
                    }
                }
                None => return Err(TokioError::Stalled),
```

(`None` means the core queued nothing and had not reached `EndOfStream`: the old code slept 10 ms and looped, hiding a core bug; verify with the existing suites that it is never hit: any test that now fails with `Stalled` is a finding to report, not to paper over with a sleep.) `fetch_playlist_resilient`:

```rust
        let mut delays = schedule(&self.config, None);
        let mut current = url.to_string();
        loop {
            match self.fetch_bytes(&current, None, timeout).await {
                Ok(bytes) => return Ok(bytes),
                Err(TokioError::Cancelled) => return Err(TokioError::Cancelled),
                Err(source) if current != self.playlist_url && !is_retryable(&source) => current.clone_from(&self.playlist_url),
                Err(_) => {
                    let d = delays.next().expect("unbounded schedule");
                    if self.sleep_or_cancelled(d).await {
                        return Err(TokioError::Cancelled);
                    }
                }
            }
        }
```

(`fetch_playlist_resilient` now returns `Result<Vec<u8>, TokioError>` so cancellation propagates.) `fetch_resource_bounded`: `let attempts = self.config.max_resource_retries.max(1) as usize; let mut delays = schedule(&self.config, Some(attempts - 1));` and between attempts `let Some(d) = delays.next() else { break }` then `sleep_or_cancelled`; no sleep after the last attempt. `build_request`:

```rust
    if let Some((offset, length)) = byte_range {
        let end = offset.checked_add(length.saturating_sub(1)).ok_or(TokioError::ByteRangeOverflow { url: url.to_string(), offset, length })?;
        let range = headers::Range::bytes(offset..=end).map_err(|_| TokioError::ByteRangeOverflow { url: url.to_string(), offset, length })?;
        let mut map = headers::HeaderMap::new();
        map.typed_insert(range);
        req = req.headers(map);
    }
```

(`headers-0.4.2/src/common/range.rs:52` `Range::bytes`; `reqwest::RequestBuilder::headers` takes the `http::HeaderMap` that `headers::HeaderMap` re-exports, both from `http 1.5.0`.) `with_config`: `Client::builder().connect_timeout(config.connect_timeout).build()`. Remove the now-unused `retry_backoff * 2` arithmetic and the `Duration::from_millis(10)` sleep. Update `mod.rs` and the module docs: retry/backoff via `backon`, cancellation via `TokioClientConfig::cancel`.

- [ ] **Step 4: Run, expect PASS**

```bash
cargo test --locked -p hls-runtime --all-features 2>&1 | grep -E '^test result|FAILED|panicked|^error|Stalled' | sort | uniq -c
cargo build --locked -p multimux --all-features 2>&1 | tail -2
```

Expected: all ok; `tokio_client_auth`/`tokio_client_restart` (which build `TokioClientConfig { auth: .., ..default() }`) compile unchanged because the struct is not `non_exhaustive`; multimux builds (it uses `TokioClient` only in tests: `multimux/tests/glass_to_glass.rs`, `golden_gate.rs`, via `TokioClient::new`/`next_output`; run `cargo test --locked -p multimux --all-features --test glass_to_glass --test golden_gate` and record the result, they are the end-to-end proof that the reload loop still works without the 10 ms sleep).

- [ ] **Step 5: Revert-check**

(a) `next_wait` returning `None` always: `next_wait_reports...` FAILS. (b) Replace `schedule(&self.config, None)` in `fetch_playlist_resilient` with a fixed `Duration::from_millis(200)` sleep: the jitter/exponential tests still pass (they test `schedule`), so ALSO run `cancelling_aborts_an_in_flight_request` with the `select!` removed from `fetch_bytes`: it FAILS by timing out at 10 s (the 300 s request timeout is not cancelled). Record both, restore.

- [ ] **Step 6: Commit**

```bash
git add hls-runtime
git commit -m "feat(hls-runtime)!: TokioClient on backon + headers::Range + CancellationToken; HlsClient::next_wait/poll_timeout; 10 ms defensive sleep removed"
```

---

### Task 14: SP7 for hls-runtime: `wait-timeout`, axum test origins

**Files:**
- Rewrite: `hls-runtime/tests/support/bounded.rs` (74 lines; used by `tests/bounded_cmd.rs` and `tests/origin_hardening.rs`)
- Rewrite: `hls-runtime/tests/tokio_client_restart.rs` and `hls-runtime/tests/tokio_client_auth.rs` (hand-rolled HTTP/1.1 origins) onto axum

**Interfaces:** `bounded::output_bounded(cmd: &mut Command, deadline: Duration) -> io::Result<Output>` keeps its signature and its behaviour (`tests/bounded_cmd.rs` is the proof and is not edited).

- [ ] **Step 1: Find the hand-rolled pieces (the failing check)**

```bash
grep -nE 'thread::sleep|windows\(4\)|HTTP/1\.1|try_wait|POLL_INTERVAL' hls-runtime/tests/*.rs hls-runtime/tests/support/*.rs
```

Expected: `bounded.rs` (poll loop), both client tests (`windows(4)`, hand-built status line).

- [ ] **Step 2: `bounded.rs` on `wait-timeout`**

```rust
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

/// Run `cmd` to completion within `deadline`, capturing stdout/stderr via temp FILES (a pipe would
/// block until every holder of the write end closes it, even after the tool exited). On overrun the
/// child is killed and an `io::ErrorKind::TimedOut` error naming the program is returned.
pub fn output_bounded(cmd: &mut Command, deadline: Duration) -> io::Result<Output> {
    let (out_path, err_path) = (capture_path("stdout"), capture_path("stderr"));
    let result = run(cmd, deadline, &out_path, &err_path);
    let _ = fs::remove_file(&out_path);
    let _ = fs::remove_file(&err_path);
    result
}

fn run(cmd: &mut Command, deadline: Duration, out_path: &PathBuf, err_path: &PathBuf) -> io::Result<Output> {
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
            format!("{:?} still running after the {deadline:?} hard deadline; killed", cmd.get_program()),
        ));
    };
    Ok(Output { status, stdout: fs::read(out_path)?, stderr: fs::read(err_path)? })
}
```

(`wait-timeout-0.2.1/src/lib.rs:49-70`: `ChildExt::wait_timeout(&mut self, Duration) -> io::Result<Option<ExitStatus>>`.)

- [ ] **Step 3: `tokio_client_restart.rs` on axum**

Replace the spawned accept loop (lines 36-79) with:

```rust
use axum::Router;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;

#[derive(Clone)]
struct Origin {
    log: Arc<Mutex<Vec<String>>>,
    plain_playlists: Arc<AtomicUsize>,
    segment: Arc<Vec<u8>>,
}

async fn handle(State(o): State<Origin>, req: Request) -> axum::response::Response {
    let path = req.uri().path_and_query().map(|p| p.as_str().to_string()).unwrap_or_default();
    o.log.lock().unwrap().push(path.clone());
    if path.contains("_HLS_msn") {
        return StatusCode::BAD_REQUEST.into_response();
    }
    match path.as_str() {
        "/live/index.m3u8" => {
            let n = o.plain_playlists.fetch_add(1, Ordering::SeqCst);
            (StatusCode::OK, if n == 0 { LIVE } else { ENDED }).into_response()
        }
        "/live/index0.ts" => (StatusCode::OK, o.segment.to_vec()).into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}
```

and in the test body `let app = Router::new().fallback(handle).with_state(origin.clone()); tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });` (the listener is bound to port 0 first and passed in: no reserve-then-rebind). The assertions at the bottom are unchanged.

- [ ] **Step 4: `tokio_client_auth.rs` on axum**

Replace `serve` and `has_authorization(head)`: the log becomes `Vec<(String, bool)>` (path, had an `Authorization` header):

```rust
type Log = Arc<Mutex<Vec<(String, bool)>>>;

#[derive(Clone)]
struct Site { routes: Arc<Vec<(String, u16, Vec<u8>)>>, log: Log }

async fn serve_one(State(s): State<Site>, req: Request) -> axum::response::Response {
    let path = req.uri().path().to_string();
    s.log.lock().unwrap().push((path.clone(), req.headers().contains_key(axum::http::header::AUTHORIZATION)));
    match s.routes.iter().find(|(p, _, _)| *p == path) {
        Some((_, status, body)) => (StatusCode::from_u16(*status).unwrap(), body.clone()).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn serve(listener: TcpListener, routes: Vec<(String, u16, Vec<u8>)>, log: Log) {
    let app = Router::new().fallback(serve_one).with_state(Site { routes: Arc::new(routes), log });
    axum::serve(listener, app).await.unwrap();
}
```

and `assert_credentials_stay_on_origin` reads `origin_log.iter().any(|(p, _)| p == "/live/index0.ts")`, `origin_log.iter().all(|(_, auth)| *auth)`, `other_log.iter().all(|(_, auth)| !*auth)`. `run_with` returns `(Vec<(String, bool)>, Vec<(String, bool)>)`.

- [ ] **Step 5: Run, expect PASS, 20-run stability**

```bash
cargo test --locked -p hls-runtime --all-features 2>&1 | grep -E '^test result|FAILED|panicked|^error' | sort | uniq -c
for i in $(seq 1 20); do cargo test --locked -p hls-runtime --all-features --test tokio_client_restart --test tokio_client_auth --test tokio_client_cancel --test bounded_cmd 2>&1 | grep -E '^test result: FAILED|panicked|^error'; done | sort | uniq -c
grep -nE 'windows\(4\)|HTTP/1\.1|POLL_INTERVAL' hls-runtime/tests/*.rs hls-runtime/tests/support/*.rs
```

Expected: ok; the loop prints nothing (`20/20`); the grep prints nothing. `bounded_cmd.rs` still proves the grandchild-holding-the-pipe case.

- [ ] **Step 6: Commit**

```bash
git add hls-runtime
git commit -m "test(hls-runtime): wait-timeout for the bounded runner; axum test origins on OS-assigned ports"
```

---

### Task 15: Guards (spec §5) for srt-runtime, webrtc-runtime, hls-runtime

Last code task: the guard files.

**Files:**
- Create: `srt-runtime/tests/no_handroll_guard.rs`, `webrtc-runtime/tests/no_handroll_guard.rs`, `hls-runtime/tests/no_handroll_guard.rs`

**Interfaces:** none.

- [ ] **Step 1: Write the guard (identical scanner in all three; allowlists differ)**

Use the scanner exactly as in `docs/superpowers/plans/2026-10-03-dehandroll-w1-rlow-a.md` Task 13 (the module doc, `NEEDLES`, `rs_files`, `non_test_source`, `next_item_is_mod`, `block_end`, `brace_delta`, the two tests `no_hand_rolled_protocol_code_in_src` and `the_scanner_itself_bites`): it is reproduced here so this plan is self-contained.

```rust
//! Tripwire against hand-rolled generic protocol code coming back (W1 spec §5).
//!
//! Scans `src/**/*.rs` outside `#[cfg(test)] mod` bodies for: HTTP status-line literals, a CRLFCRLF
//! header terminator, `find("://")`, `strip_prefix("<scheme>://")`, SDP line building (`"a=`, `"m=`,
//! `"v=0`), civil-date helpers, a base64 alphabet literal, and `thread::sleep` / `sleep(` in
//! non-test async code.
//!
//! **This is a lexical tripwire, not a proof.** A renamed helper or a string assembled from pieces
//! evades it. Code review is the real control.

use std::fs;
use std::path::Path;

/// (file suffix, needle, reason).
const ALLOW: &[(&str, &str, &str)] = &[
    // hls-runtime only:
    // ("client/tokio_client.rs", "tokio::time::sleep(",
    //  "backon-scheduled retry delay and WaitMs hint, both raced against the CancellationToken in `sleep_or_cancelled`"),
];

const NEEDLES: &[&str] = &[
    "\"HTTP/1.",
    "\\r\\n\\r\\n",
    "find(\"://\")",
    "strip_prefix(\"http://\")",
    "strip_prefix(\"https://\")",
    "strip_prefix(\"stun:\")",
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

fn non_test_source(src: &str) -> String {
    let lines: Vec<&str> = src.lines().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].trim_start().starts_with("#[cfg(test)]") {
            if let Some(m) = next_item_is_mod(&lines, i) {
                i = block_end(&lines, m) + 1;
                continue;
            }
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
        for (n, line) in non_test_source(src).lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
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
    assert_eq!(body.matches("HTTP/1.").count(), 1, "test-module body removed, non-test line kept");
}
```

Per-crate decisions (made on the first run, each hit fixed in the source or allowlisted with a checkable reason):

- **srt-runtime**: `io.rs` must have no `tokio::time::sleep(` left (Task 4 removed the pacing sleep; the connect loop uses `interval_at`/`sleep_until`). No allowlist expected.
- **webrtc-runtime**: `src/ice.rs` is the documented Link-header exception (§9.1): its string handling does not match any needle, so nothing is allowlisted; if a hit appears there, allowlist that exact line with `"RFC 8288 Link parser: documented exception (§9.1), RFC 8288 + RFC 9725 vectors in tests/"`. `media/transport.rs` must not contain `"a=` outside comments (the fingerprint scanner is gone after Task 8).
- **hls-runtime**: allowlist `("client/tokio_client.rs", "tokio::time::sleep(", "backon-scheduled retry delay and WaitMs hint, both raced against the CancellationToken in sleep_or_cancelled")`: it is the one place an async sleep is legitimate. `server/engine.rs` (the origin) has two `sleep` mentions per the earlier grep: inspect them; if they are doc text they are skipped (comment lines), if code, replace with the `poll`-style deadline the origin already exposes or allowlist with the reason.

- [ ] **Step 2: Run, fix or allowlist, then bite-check**

```bash
cargo test --locked -p srt-runtime -p webrtc-runtime -p hls-runtime --all-features --test no_handroll_guard 2>&1 | grep -E 'test result|FAILED|:[0-9]+: '
```

Expected: green after the per-crate decisions above. Bite-check: temporarily add `let _ = "HTTP/1.1 200";` to a non-test fn in each crate, confirm the guard FAILS naming that file:line, remove it.

- [ ] **Step 3: Commit**

```bash
git add srt-runtime/tests/no_handroll_guard.rs webrtc-runtime/tests/no_handroll_guard.rs hls-runtime/tests/no_handroll_guard.rs
git commit -m "test: no-hand-roll tripwire guards for srt-runtime, webrtc-runtime, hls-runtime"
```

---

### Task 16: Full gate, thumbv7em, CHANGELOG, version notes (do not merge)

**Files:**
- Modify: `srt-runtime/CHANGELOG.md`, `webrtc-runtime/CHANGELOG.md`, `hls-runtime/CHANGELOG.md` (`## [Unreleased]`)
- Modify: `srt-runtime/README.md`, `webrtc-runtime/README.md` (the `no_std` claim), `hls-runtime/README.md`; crate-root `//!` of `webrtc-runtime/src/lib.rs` (done in Task 9), `srt-runtime/src/io.rs` module doc (mentions the 2 ms tick: `grep -n "2 ms\|TICK_INTERVAL\|interval" srt-runtime/src/io.rs | head`)
- Create/extend: `.delegate/w1-r-low-b-report.md`

- [ ] **Step 1: Re-grep the spec §3 sites for this half of the cluster**

```bash
grep -rnE 'TICK_INTERVAL_MS|MAX_DATAGRAM\b|spawn_listener_routing_pump|HANDSHAKE_TIMEOUT|PEER_IDLE_TIMEOUT|days_from_civil|find\("://"\)|append_query|from_millis\(10\)|a=fingerprint:|Instant::now\(\)' srt-runtime/src hls-runtime/src webrtc-runtime/src | grep -v '^[^:]*:[0-9]*:\s*//' | grep -vE 'cfg\(test\)|mod tests' | cut -c1-130
```

Expected: no hit for the first six patterns in `src`; the only `Instant::now()` hits are in the srt adapter (`io.rs`: it is a tokio adapter, which owns the clock) and in `webrtc-runtime` tests. Any `Instant::now()` in `webrtc-runtime/src/media/` non-test code is a miss of Task 7: fix it. Record the output in the report.

- [ ] **Step 2: CHANGELOG `[Unreleased]` entries (breaking ones marked)**

`srt-runtime/CHANGELOG.md`:

```markdown
### Changed
- **BREAKING** `SrtSocket::recv` returns `bytes::Bytes` (was `Vec<u8>`); datagrams are carried as `Bytes` end to end (one allocation shared by many datagrams, payload delivered as a view into the received datagram).
- **BREAKING** `SrtListener` completes handshakes in a tracked background task: `accept()` only waits for a finished connection. Dropping the listener releases its UDP port unless an accepted connection is still alive (defect 3: the routing pump used to keep the port bound after drop).
- The connection driver is deadline-driven (`Receiver::next_timeout`, `TsbpdScheduler::next_release_after`) instead of a fixed 2 ms tick; an idle connection wakes at its 10 ms Full ACK cadence (protocol rule 11).
### Added
- `io::IoConfig { max_datagram, connect, handshake, read_idle, write }` and `SrtSocket::{connect_with, connect_from_with, send_bytes}`, `SrtListener::bind_with`; `SocketStats::rx_oversize`, `SrtListener::accept_overflow_dropped`.
### Fixed
- A datagram larger than the receive buffer was truncated silently and parsed as a shorter valid packet; it is now dropped and counted.
```

`webrtc-runtime/CHANGELOG.md`:

```markdown
### Changed
- **BREAKING** The crate is `std` (WHIP/WHEP use `http`/`headers`). `webrtc-runtime` is no longer built for `thumbv7em-none-eabi`.
- **BREAKING** WHIP/WHEP `HttpRequest`/`HttpResponse` carry `http::HeaderMap` and `http::StatusCode`; `Method` is `http::Method`; `WhipSession::on_patch`/`WhepSession::on_patch` take the request `HeaderMap`; `WhepSession::no_publisher` takes `Option<Duration>`. Stricter: `If-Match: "*"` (quoted) and an unquoted ETag are no longer accepted; a duplicated `Content-Type` is rejected. Examples: <paste the golden/strictness differences recorded in Task 9>.
- **BREAKING** `MediaTransport::new(config, now)` takes the caller's clock reading; nothing in `media/` reads the wall clock.
- `parse_remote_fingerprint` parses the SDP with `sdp-types` and returns the typed attribute's normalised text (`SHA-256 ab:cd` -> `sha-256 AB:CD`); an input that is not a complete SDP now yields `None`.
### Added
- `MediaTransport::{poll_timeout, local_candidates}`; `Error::InvalidHeader`.
```

`hls-runtime/CHANGELOG.md`:

```markdown
### Changed
- **BREAKING** `TokioClientConfig` gains `connect_timeout`, `jitter`, `cancel`; retries use `backon` (exponential, jittered); `Range` is built by `headers::Range`; a cancelled token ends `next_output` with `Ok(None)` and aborts in-flight requests; the 10 ms defensive sleep is removed (`TokioError::Stalled` if the core ever has nothing to do).
- Relative URI resolution uses `Url::join` (RFC 3986): `..` segments now resolve (defect 7), a `://` in a query no longer makes a reference absolute, results are normalised (`HTTP://H/x` -> `http://h/x`, spaces percent-encoded, default port dropped). Query building uses `query_pairs_mut`: an appended `_HLS_*` pair is inserted before a fragment (it used to land inside it and was never sent).
- `EXT-X-PROGRAM-DATE-TIME` parsing uses `jiff`; a leap second is clamped to `:59` (was `+60 s`).
### Added
- `HlsClient::next_wait` and (feature `std`) `HlsClient::poll_timeout`.
```

Update the three READMEs/crate docs: srt (adapter section: deadlines, `IoConfig`, listener lifecycle), webrtc (`std`, typed HTTP), hls (`TokioClientConfig` fields).

- [ ] **Step 3: Version notes in `.delegate/w1-r-low-b-report.md` (the orchestrator updates `.delegate/release-versions.txt`)**

Write exactly:

```
## Version notes (W1-R-low-b)
srt-runtime     BREAKING (recv -> Bytes, listener lifecycle, IoConfig; bytes/tokio-util in the public API) -> 0.x minor bump
webrtc-runtime  BREAKING (std core, http/headers types in the public API, MediaTransport::new(now)) -> 0.x minor bump; leaves the thumbv7em CI list
hls-runtime     BREAKING (TokioClientConfig fields incl. tokio_util::CancellationToken in the public API) -> 0.x minor bump
Epoch purity: public types from http 1.x/headers 0.4 (webrtc), bytes 1 (srt), tokio-util 0.7 CancellationToken (hls) enter public signatures; reqwest 0.12 stays in hls TokioError (bump deferred to W2 with multimux, Escalation 3). No workspace-sibling caret epoch changes.
multimux compile fixes made here: srt Bytes (2 call sites, multimux/src/source/srt.rs:320, push/srt.rs:366), MediaTransport::new(now) (5 sites in source/whip.rs, output/whep.rs). Everything else is W2.
New lock packages: jiff, backon, headers, headers-core, mime, httpdate, base64 0.22 + sha1 0.10 (second versions, pulled by headers 0.4.2), sdp-types 0.2.0, wait-timeout, axum 0.8 + deps (dev only).
Goldens: webrtc whip_whep_http.golden (differences: <paste Task 9>), hls client_urls.golden (differences: <paste Task 11>).
```

- [ ] **Step 4: thumbv7em builds and the full gate**

```bash
for c in srt-runtime hls-runtime; do cargo build --locked -p $c --no-default-features --target thumbv7em-none-eabi 2>&1 | tail -1; done
/Volumes/External/Projects/rust-broadcast/.delegate/gate-wt.sh "$PWD" 2>&1 | tee /tmp/w1-rlow-b-gate.log | grep -E '^== |^rc=|GATE-DONE|FAIL'
```

Expected: both thumb builds `Finished` (webrtc-runtime is deliberately not built there any more); every gate step `rc=0` (14/14 incl. the per-crate no-default-features loop, both multimux feature clippy runs, the dvb-ci-runtime Linux checks, the pinned 1.97.1 clippy, `check-published-dep-consistency.py`), `GATE-DONE`. Any failure: fix and re-run the whole gate.

- [ ] **Step 5: Evidence and hand-off**

Append to the report: failing-then-passing names per defect/behaviour (Tasks 3, 4, 5, 6, 7, 8, 9, 11, 12, 13 revert-checks), the 20/20 stability results (Tasks 10, 14), the libsrt interop tests that actually ran, the gate's 14/14 summary, the golden diffs. Commit:

```bash
git add srt-runtime webrtc-runtime hls-runtime
git commit -m "docs: W1-R-low-b changelogs, README adapter sections, version notes"
git status --short | head
```

**Do not merge.** Do not tag. Do not edit `.delegate/release-versions.txt`. Hand `w1/r-low-b` to the orchestrator for the adversarial review and merge. When merging with `w1/r-low-a`, `Cargo.lock` will conflict (both add packages): take `main`'s lock and re-run `cargo build --locked` after `cargo update -p sdp-types --precise 0.2.0` style commands, then confirm `git diff Cargo.lock` adds only this plan's and plan a's packages.

---

## Coverage table

Every §3 inventory site and SP item assigned to this half of the R-low cluster, and the task that covers it.

| Spec item | Where | Task |
|---|---|---|
| SP1.2 srt connection `poll_timeout`, 2 ms tick removed | srt-runtime/src/io.rs, arq/receiver.rs, tsbpd.rs | 4 |
| SP1.2 srt listener deadline (pending handshakes, no idle timer) | ListenerCore::poll_timeout | 5 |
| SP1.2 webrtc `MediaTransport::poll_timeout` | media/transport.rs | 7 |
| SP1.2 hls `HlsClient` `poll_timeout`, 10 ms defensive sleep removed | engine.rs, tokio_client.rs | 13 |
| W0 carry: `Instant::now()` in `MediaTransport::new` / `StunGather::new` | media/transport.rs, gather.rs | 7 |
| SP1.3 explicit timeouts (srt `IoConfig`; hls `connect_timeout`, request/blocking) | io.rs, tokio_client.rs | 3, 6, 13 |
| SP1.4 tracked tasks / CancellationToken (srt listener, hls TokioClient) | ListenerLife, TokioClientConfig::cancel | 5, 13 |
| SP1.5 backon in hls-runtime | tokio_client.rs | 13 |
| SP6.5 srt listener background accept task, `Bytes`, configurable max datagram, tracked routing pump | io.rs | 3, 5 |
| Defect 3 (srt listener pump keeps port bound) | io.rs | 5 |
| SP2.2 webrtc WHIP/WHEP on `http`/`headers`, core std | whip/*, whep/*, lib.rs | 9 |
| SP2.6 hls Range via `headers::Range` | tokio_client.rs `build_request` | 13 |
| SP3 hls `Url::join` and query; defect 7 | client/url.rs, action.rs | 11 |
| SP3 webrtc `stun:` URL | media/transport.rs | 8 |
| SP3 rtmp tcUrl | not this half | plan a, Task 9 |
| SP4 webrtc fingerprint via sdp-types typed attributes | media/transport.rs | 8 |
| SP4 ICE candidate via rtc-ice | media/transport.rs, tests, example | 7 (`local_candidates`), 8 |
| SP4 sdp-types 0.2 for rtsp-runtime | not this half | plan a, Task 2 |
| SP5 hls dates via jiff | client/engine.rs | 12 |
| SP7 for these crates (sleeps, bounded.rs, hand-rolled origins, webrtc example) | tests, examples | 10, 14 |
| §5 guards | no_handroll_guard.rs x3 | 15 |
| Goldens (§6) | whip_whep_http.golden, client_urls.golden; srt: no wire change | 1 (compare 9, 11) |
| Versioning (§8), CHANGELOG | report | 16 |
| Link header (§9.1 exception) | webrtc-runtime/src/ice.rs unchanged | 9 (note), 15 (guard allowlist if hit) |

Link-header vectors/fuzz (spec §9.1 says "RFC 8288 + RFC 9725 test vectors and a fuzz target added"): `fuzz/fuzz_targets/webrtc_ice.rs` already fuzzes `parse_ice_server_links`/`format_ice_server_links` (verified), and `ice.rs` has unit tests; adding the RFC 8288 §3.5 and RFC 9725 §4.4 example vectors as `webrtc-runtime/tests/ice_link_vectors.rs` is a small additive step owned by W3 (spec §7: guards, cleanup sweep), not by this plan.

## Escalations

1. **`MediaTransport::poll_timeout` is `&mut self`, not `&self`** (spec SP1.2). `rtc-ice 0.21`/`rtc-stun 0.21` implement `sansio::Protocol::poll_timeout(&mut self)` (`sansio-1.0.1/src/lib.rs:707`, `rtc-ice-0.21.0/src/agent/agent_proto.rs:135`), so an aggregate over them cannot be `&self`. The srt `Driver::poll_timeout` is `&self` as specified. Evidence is in the Task 7 interface comment. Owner action: accept the deviation.
2. **HLS `poll_timeout` needs a caller clock.** The hls core is `no_std` and never reads a clock, so `HlsClient::poll_timeout(&self)` (no argument) cannot return an `Instant`. Delivered: `next_wait() -> Option<Duration>` (core) and `poll_timeout(&self, now: Instant) -> Option<Instant>` (feature `std`). The deadline is derived from the queued `WaitMs` hint, which is how the core already expresses its only timer. Owner action: accept, or ask for `HlsClient` to track a `now` (breaking the sans-IO contract).
3. **reqwest 0.12 -> 0.13 for hls-runtime is deferred to W2.** `reqwest::Error` is in `TokioError`'s public API, so the bump is a public-type change that must move together with multimux's own bump (spec §4 SP2.7 lists reqwest with SP2/W2). Until then `headers`'s `http 1.5.0` and reqwest 0.12's `http 1` are the same crate version, which Task 13's `RequestBuilder::headers(headers::HeaderMap)` relies on; the lock check in Task 2 confirms a single `http`.
4. **`url` and `jiff` in the `no_std` hls core.** Both are `default-features = false` + `alloc`, and the thumbv7em build is re-run after Tasks 2, 11, 12 and 16. If either fails to cross-build (icu-based IDNA tables in `url` are the plausible culprit), the fallback is NOT to drop `no_std` silently: stop and ask the owner, because spec Q2 allows dropping `no_std` for hls-runtime but CI currently promises it.
5. **CI workflow edit required** (`.github/workflows/ci.yml:145`): `webrtc-runtime` leaves the `thumbv7em` build list because its core now needs `std`. It is a one-token manual edit with a YAML-parse check (Task 9), not a regex edit (memory: never-regex-edit-yaml-workflows). Flag for the orchestrator's review. `CLAUDE.md` still describes webrtc-runtime's default build as `no_std`-capable: the W3 docs sweep owns that line.
6. **`headers::Location` has no accessor and no public constructor.** It is built through `Header::decode` and read back through `Header::encode` (`http_util::{location, location_of}`), so the typed header is used in both directions, at the cost of two small helpers. Equivalent for `ETag` (`etag`, `etag_opaque`).
7. **`headers 0.4.2` adds second versions of `base64` (0.22) and `sha1` (0.10)** next to the 0.23/0.11 W0 moved the workspace to. No newer `headers` release exists (0.4.2 is the latest on crates.io as of this plan). Cost: two extra small packages in the lock for webrtc-runtime and hls-runtime (feature `tokio`). Owner action: accept, or replace `headers` for the three typed uses (ETag/IfMatch/Range) with hand code (rejected by the spec's own rule).
8. **SRT idle Full ACK cadence stays 10 ms.** The deadline-driven driver removes the 2 ms tick, but the receiver's Full ACK is unconditional every 10 ms (rule 11), so an idle connection still wakes at 100 Hz (was 500 Hz). Going further would change wire behaviour that libsrt interop depends on.
