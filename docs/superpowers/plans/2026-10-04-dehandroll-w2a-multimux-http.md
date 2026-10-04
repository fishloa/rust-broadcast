# De-hand-roll W2a-multimux-http — HTTP stack, WHIP/WHEP on axum, SDP Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Delete multimux's hand-rolled HTTP request reader and the WHIP/WHEP signalling listeners; serve every HTTP surface (origin, admin, WHIP, WHEP) through hyper-util's server builder with a Tokio timer, a header-read timeout and a total per-connection deadline, on the axum 0.8 / tower-http 0.7 / reqwest 0.13 stack; parse and build all WHIP/WHEP SDP through `sdp-types` 0.2's typed `Session` API; make WebRTC session tasks own their `MediaTransport` (no `Arc<TokioMutex<_>>` held across `send_to().await`); fix defects 1–3 for **both** the WHIP/WHEP and the RTMP accept pumps.

**Architecture:** One branch `w2/a` in worktree `.worktree/w2-a`, one commit per task. The HTTP bump is mechanical (CHANGELOG-driven). The WHIP/WHEP move to axum is substantive: the two raw `TcpListener` accept loops become `Router`s layered with `ConcurrencyLimit`/`RequestBodyLimit`/`Timeout`/`Cors`, and hand-formatted `HTTP/1.1 201\r\n…` bytes become typed `http::Response` built from `webrtc_runtime`'s `HttpResponse`. Each task keeps the workspace compiling.

**Tech Stack:** Rust 1.95 workspace, `cargo --locked`; axum 0.8.9, tower-http 0.7.1, tower 0.5.3, hyper 1 / hyper-util 0.1.20, reqwest 0.13.5, `http` 1, `headers` 0.4, sdp-types 0.2.0, tokio-util 0.7 (`CancellationToken`, `TaskTracker`), tokio 1 (`task::JoinSet`), `webrtc-runtime` 0.2 (candidate lines come from `MediaTransport::local_candidates()`; NO new `rtc-ice` dependency).

**Spec:** `docs/superpowers/specs/2026-10-03-protocol-runtime-dehandroll-design.md` (§3 HTTP/SDP rows, §4 SP2/SP4/SP6.3, §5 guards, §6 verification, §7 W2, §8 versioning). This plan is **W2a**; **W2b** is `docs/superpowers/plans/2026-10-04-dehandroll-w2b-multimux-runtime.md` (ingest/pull/push/runtime). See "Dependency order".

## Dependency order

- **W2a merges FIRST.** W2b rebases over W2a. W2a depends on **W1-R-low-b** (already on `main` at `2b1de6db`): webrtc-runtime's `http_util`, typed `WhipSession`/`WhepSession` state machines, and `MediaTransport::{poll_timeout, local_candidates}`.
- W2a does **not** depend on W1-R-low-a. Its Task 1 baseline runs the rtsp/rtmp suites *unchanged* (no R-low-a adapters are touched in W2a).
- W2a Task 8 bumps `sdp-types` 0.1→0.2, which breaks `source/sdp.rs` (the `.ok().flatten()` on `get_first_attribute_value`). That single file is adapted in W2a Task 8 so the workspace keeps compiling; the RTSP-source *runtime* migration (to R-low-a's `AsyncRtspClient`) stays in W2b.

## Global Constraints

Copied verbatim from spec §2:

- MSRV 1.95.0; committed `Cargo.lock`, always `--locked`. A dependency add or
  bump may change only the intended lock entries; restore anything else with
  `cargo update -p <pkg> --precise <old>`.
- Every new or bumped crate supports MSRV 1.95: verified for all 34 bumps (max
  1.89) and for the new crates (`tokio-util` 1.71, `socket2` 1.70, `backon`
  1.85, `parking_lot` 1.71, `lru` 1.85, `jiff` 1.70, `hex`, `base64`,
  `arc-swap`, `wait-timeout`).
- No Co-Authored-By or Claude-Session trailers on commits.
- Nothing is tagged or published without the owner's explicit sign-off.
- Epoch purity: if a bumped dependency's types appear in a crate's public API,
  that crate takes a major-class version change. Each wave records this in
  `.delegate/release-versions.txt`.

Owner decisions that apply to this cluster (§1, §4):

- Take **all** dependency bumps: `axum` 0.7→0.8, `tower-http` 0.5→0.7, `reqwest` 0.12→0.13.
- SP2.1: WHIP/WHEP listeners move to `axum::serve` with `with_graceful_shutdown(token)`; `webrtc_http.rs` is deleted; tower layers `ConcurrencyLimitLayer`, `RequestBodyLimitLayer` (64 KiB), `TimeoutLayer` (10 s), tower-http `CorsLayer`; listen addresses stay as configured; chunked request bodies are now accepted.
- SP2.2 (landed W1-R-low-b): the webrtc-runtime WHIP/WHEP state machines already speak `http`/`headers`. W2a consumes them.
- SP4: `sdp-types` 0.2 handles all SDP parse/write. ICE credentials are read at **media level, falling back to session level** (the plan's own OFFER and Chrome/Firefox put `a=ice-ufrag`/`a=ice-pwd` on each `m=` section). Answers build a `Session` and call `Session::write`. ICE candidate lines come from `webrtc_runtime::media::MediaTransport::local_candidates()` (already-marshalled strings, transport.rs:749) — no new `rtc-ice` dependency. fmtp parameter content stays codec logic. Link header is exempt (§9).
- SP6.3: WebRTC session tasks own their `MediaTransport`; no `Arc<tokio::Mutex<MediaTransport>>` across `send_to().await`.

## Review Focus

The five inputs or failure modes most likely to bite users that no task's tests would otherwise cover. Each has a test added to its owning task.

1. **A slow-loris client holds a listener open.** `axum::serve` sets no header-read timeout. Test: a client that sends a request head and stops is dropped at `HEADER_READ_TIMEOUT`, and the listener still serves the next request. Owned by **Task 2** (origin/admin) and **Task 4/5** (WHIP/WHEP).
2. **WHIP never runs its protocol timers while media flows** (defect 1). Test: a session with no datagram still gets `handle_timeout` at `MediaTransport::poll_timeout`, and is *not* reaped by that timer alone. Owned by **Task 7**.
3. **A WHIP/WHEP/RTMP session that never sends a datagram starves new accepts** (defect 2). Test: an accept completes while a read is continuously ready. Owned by **Task 6** (RTMP) and **Task 7** (WHIP/WHEP).
4. **A cancel during shutdown leaks a bound port** (defect 3: the untracked accept pumps in whip, whep and rtmp). Test: dropping/cancelling the listener releases the port within a bounded wait, with a saturated acceptance semaphore so the old code demonstrably blocks. Owned by **Task 6**.
5. **A WHIP/WHEP SDP answer differs in a way interop rejects.** The answer golden is byte-compared across the sdp-types migration; every difference is listed and the real-browser `whip_ingest`/`whep_egress` suites stay green. Owned by **Task 8**.

---

### Task 0: Worktree setup

**Files:** none (environment only).

- [ ] **Step 1: Create the worktree off main**

```bash
cd /Volumes/External/Projects/rust-broadcast
git fetch -q origin
git worktree add -b w2/a .worktree/w2-a origin/main
cd .worktree/w2-a
git -c protocol.file.allow=always submodule update -q --init --reference /Volumes/External/Projects/rust-broadcast/private private
ln -s /Volumes/External/Projects/rust-broadcast/.test-streams .test-streams
ln -s /Volumes/External/Projects/rust-broadcast/multimux/tests/assets/node_modules multimux/tests/assets/node_modules
export CARGO_TARGET_DIR="$PWD/target"
```

- [ ] **Step 2: Baseline — multimux + webrtc-runtime suites pass BEFORE any change**

```bash
timeout 2400 cargo test --locked --all-features -p multimux -p webrtc-runtime 2>&1 | grep -E '^test result|FAILED|panicked' | sort | uniq -c
timeout 1800 cargo test --locked --all-features -p multimux --test whip_ingest --test whep_egress -- --nocapture 2>&1 | grep -E 'test result|FAILED|real result'
```

Expected: only `test result: ok`; `whip_ingest`/`whep_egress` print their real result lines (real browser harness), not skips. Record counts in `.delegate/w2a-report.md` under "baseline".

---

### Task 1: HTTP stack bump (axum 0.8, tower-http 0.7, reqwest 0.13)

**Files:**
- Modify `multimux/Cargo.toml`:
  - `axum = "0.8"`
  - `tower-http = { version = "0.7", default-features = false, features = ["timeout", "limit", "cors"] }` (the `cors` feature is required by Tasks 4/5)
  - `reqwest = { version = "0.13", default-features = false, features = ["rustls", "stream"] }` — **the feature is `rustls`, not `rustls-tls`** (verified: reqwest 0.13.5 `Cargo.toml` `[features]` has `rustls`, `rustls-no-provider`, `stream`, `json`; there is no `rustls-tls`).
  - `[dev-dependencies] reqwest = { version = "0.13", default-features = false, features = ["json"] }` (the `json` feature is unchanged).
- Modify `hls-runtime/Cargo.toml`: `reqwest = { version = "0.13", … features = ["rustls"] }` (its tokio feature currently uses `rustls-tls` — rename to `rustls`).
- Modify the axum path-parameter syntax (see Interfaces) — no other call-site change for tower-http 0.7 (verified `TimeoutLayer`/`RequestBodyLimitLayer`/`CorsLayer` names are unchanged).

**Interfaces:**
- Produces: `multimux` builds against axum 0.8 / tower-http 0.7 / reqwest 0.13; `hls_runtime::client::tokio_client::TokioError` still names `reqwest::Error` (same public shape, new reqwest epoch) — **breaking** for hls-runtime.
- Consumes: nothing new.

- [ ] **Step 1: Capture the origin response-header golden from main BEFORE the bump**

The origin's response headers (CORS + `Cache-Control`) are wire output. Add the test-helper pattern that already exists in `output/smooth.rs:972` (`GOLDEN_BLESS` env var writing the golden when set, else comparing byte-for-byte) to the existing `origin::tests::manifest_and_resource_responses_carry_expected_cache_control_and_cors` test, writing one line per probe (`path<TAB>status<TAB>header:value` sorted by name) for the same paths that test already probes (`/cam1/master.m3u8`, `/cam1/media.m3u8`, `/cam1/init-1-0-1.mp4`, `/cam1/seg-…`, `/metrics`, `/healthz`). The golden is `multimux/tests/golden/origin_response_headers.golden`. Add a `multimux/tests/golden/README.md` entry (commit + regen command), copying the existing smooth entry's shape.

Also golden the WHIP/WHEP 201/204/401/413 headers here (they are the Review-Focus-5 wire output, and `CorsLayer` will change them): extend the existing `whip_ingest`/`whep_egress` header assertions if they do not already pin the exact `Access-Control-*`/`Location`/`ETag` set; if they only assert status, add a header golden `multimux/tests/golden/whip_whep_headers.golden` here. **This is the same commit order W0 used: golden the old output, then migrate.**

```bash
GOLDEN_BLESS=multimux/tests/golden cargo test -p multimux --all-features --locked --lib origin::tests::manifest_and_resource_responses_carry_expected_cache_control_and_cors 2>&1 | grep -E 'test result|FAILED'
git add multimux/tests/golden multimux/src/origin/mod.rs
git commit -m "test(multimux): golden the origin's response headers before the axum 0.8 bump"
```

- [ ] **Step 2: Migrate the axum path-parameter syntax (0.8 changed it to `{...}`)**

The 0.8.0 changelog: *"Upgrade matchit to 0.8, changing the path parameter syntax from `/:single` and `/*many`"*. Change these **production** routes to `{name}`/`{*rest}`:
- `multimux/src/origin/resource.rs:131` `.route("/:file", …)` → `/"{file}"`
- `multimux/src/output/catchup.rs:91` `/vod/:period` → `/vod/{period}`
- `multimux/src/output/catchup.rs:92-95` `/catchup/:file` → `/catchup/{file}`
- `multimux/src/origin/admin.rs:940` `/admin/routes/:name` → `/admin/routes/{name}`

And these **test-server** routes (same rename): `source/dash_pull.rs:1459,1939,2057`, `source/hls_pull.rs:800,1015,1335`, `source/smooth_pull.rs:1569,1595` (`/*path` → `/{*path}`).

- [ ] **Step 3: Bump and update, assert the exact lock diff**

```bash
CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=fallback cargo update -p axum -p tower-http -p reqwest
git diff Cargo.lock | grep -E '^[-+]name' | sort | uniq
```

Expected **exactly** (assert these names, fail on any other): `axum` 0.7.9→0.8.9, `axum-core` 0.4.5→0.5.6, `tower-http` 0.5.2→0.7.1 (plus `tower-http` 0.6.11 appears as *removed* if it was an old edge), `reqwest` 0.12.28→0.13.5, and their transitive moves (`hyper-util`, `http-body-util`, `tower` where the new reqwest/axum require). `reqwest` 0.12 may remain for another crate that pins it — list every lock delta name/version in `.delegate/w2a-report.md`; restore anything unintended with `cargo update -p <pkg> --precise <old>`.

- [ ] **Step 4: Migrate call sites from each crate's CHANGELOG**

```bash
ls ~/.cargo/registry/src/*/axum-0.8.9/CHANGELOG.md ~/.cargo/registry/src/*/reqwest-0.13.5/CHANGELOG.md 2>/dev/null
```

Known changes: axum 0.8 path syntax (Step 2); `Router` now requires `Sync` handlers/services (already true); `axum::serve` is generic over the listener (no change to the call site — W2a Task 2 replaces it anyway); reqwest `rustls-tls`→`rustls` (Step 1). No tower-http 0.7 source change beyond the feature list (verified `TimeoutLayer`/`RequestBodyLimitLayer`/`CorsLayer` exist with the same names).

- [ ] **Step 5: Build + test, goldens byte-identical**

```bash
cargo build --workspace --all-features --all-targets --locked 2>&1 | grep -E '^error' -A6 | head -60
timeout 2400 cargo test --locked --all-features -p multimux -p webrtc-runtime 2>&1 | grep -E '^test result|FAILED|panicked' | sort | uniq -c
git diff --exit-code multimux/tests/golden/ && echo "goldens byte-identical"
```

Expected: no errors; same counts as baseline; goldens unchanged.

- [ ] **Step 6: Commit**

```bash
git add multimux hls-runtime Cargo.lock
git commit -m "chore(deps)!: axum 0.8, tower-http 0.7, reqwest 0.13 (breaking for hls-runtime: reqwest::Error epoch)"
```

---

### Task 2: Serve origin + admin through hyper-util's server builder (header-read timeout, connection cap, total deadline, graceful shutdown)

**Files:**
- Modify `multimux/src/origin/mod.rs` — add `serve_hyper_util` (replacing `axum::serve` at line 1037) and `serve_hyper_util_with_timeout` (configurable, for the test).
- Modify `multimux/src/origin/admin.rs` — replace the two `axum::serve` calls (1100, 1112) with the same helper.
- Modify `multimux/Cargo.toml` — add `hyper = { version = "1", features = ["server", "http1"] }`, `hyper-util = { version = "0.1", features = ["server", "server-auto", "http1", "tokio", "service"] }`.
- New test `multimux/tests/server_timeouts.rs`.

**Interfaces:**
- Produces:
  ```rust
  /// How long a client may take to finish its request header before the
  /// connection is closed (SP2.1). `axum::serve` sets none.
  pub const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(10);
  /// Hard cap on concurrent accepted (not yet served) connections.
  pub const MAX_CONNECTIONS: usize = 1024;
  /// Bounds an entire connection, headers through response (brief item 2).
  pub const TOTAL_CONNECTION_TIMEOUT: Duration = Duration::from_secs(30);

  /// Serve `app` (an axum `Router`) on `listener` through hyper-util's auto
  /// builder: a Tokio timer, `header_read_timeout`, a `MAX_CONNECTIONS`
  /// semaphore, a total per-connection deadline, and graceful shutdown on
  /// `token`. Mirrors axum 0.8.9's own `handle_connection` (serve/mod.rs:354),
  /// which is the authoritative wiring for `ConnectInfo`.
  pub(crate) async fn serve_hyper_util(
      listener: tokio::net::TcpListener,
      app: axum::Router,
      token: tokio_util::sync::CancellationToken,
  ) -> std::io::Result<()>;

  /// [`serve_hyper_util`] with explicit header-read + total timeouts (test-only).
  #[doc(hidden)]
  pub async fn serve_hyper_util_with_timeout(
      listener: tokio::net::TcpListener,
      app: axum::Router,
      token: tokio_util::sync::CancellationToken,
      header_read: Duration,
      total: Duration,
  ) -> std::io::Result<()>;
  ```
- Consumes: `hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer}`, `hyper_util::server::conn::auto::Builder`, `hyper_util::service::TowerToHyperService`, `axum::extract::connect_info::IntoMakeServiceWithConnectInfo`, `axum::serve::IncomingStream`, `std::net::SocketAddr`.

- [ ] **Step 1: Write the failing test**

`multimux/tests/server_timeouts.rs`:

```rust
//! SP2.1: every multimux HTTP listener is served through one hyper-util
//! server with a timer and a header-read timeout, so a client that sends a
//! request head and stops is dropped rather than held open.

use std::sync::Arc;
use std::time::Duration;

use multimux::origin::{AppState, HttpLimits};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

#[tokio::test(start_paused = true)]
async fn a_slow_header_client_is_dropped_at_the_header_read_timeout() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = Arc::new(
        AppState::new(Default::default())
            .with_limits(HttpLimits::default()),
    );
    let token = tokio_util::sync::CancellationToken::new();
    let app = multimux::origin::router(state);
    let serve = tokio::spawn(multimux::origin::serve_hyper_util_with_timeout(
        listener,
        app,
        token.clone(),
        Duration::from_millis(200),
        Duration::from_secs(5),
    ));

    // A slow-loris client: half a request head, then silence.
    let mut slow = TcpStream::connect(addr).await.unwrap();
    slow.write_all(b"POST / HTTP/1.1\r\nHost: x\r\n").await.unwrap();

    // The 200 ms header timeout must close it, not hold it open.
    let mut buf = [0u8; 1];
    let read = tokio::time::timeout(Duration::from_secs(2), slow.read(&mut buf)).await;
    assert!(
        matches!(read, Ok(Ok(0)) | Ok(Err(_))),
        "the slow-header connection must be closed, not held open: {read:?}"
    );

    token.cancel();
    let _ = serve.await;
}

#[tokio::test]
async fn the_listener_still_serves_a_good_request_after_a_slow_client() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = Arc::new(
        AppState::new(Default::default())
            .with_limits(HttpLimits::default()),
    );
    let token = tokio_util::sync::CancellationToken::new();
    let app = multimux::origin::router(state);
    let serve = tokio::spawn(multimux::origin::serve_hyper_util_with_timeout(
        listener,
        app,
        token.clone(),
        Duration::from_millis(200),
        Duration::from_secs(5),
    ));

    // A client that stops mid-head.
    let mut slow = TcpStream::connect(addr).await.unwrap();
    slow.write_all(b"POST / HTTP/1.1\r\n").await.unwrap();
    let mut buf = [0u8; 1];
    let _ = tokio::time::timeout(Duration::from_secs(2), slow.read(&mut buf)).await;

    // The listener still answers a normal request.
    let mut good = TcpStream::connect(addr).await.unwrap();
    good.write_all(b"GET /healthz HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut body = Vec::new();
    tokio::io::AsyncReadExt::read_to_end(&mut good, &mut body).await.unwrap();
    assert!(
        String::from_utf8_lossy(&body).starts_with("HTTP/1.1 200"),
        "healthz must still answer: {}",
        String::from_utf8_lossy(&body)
    );

    token.cancel();
    let _ = serve.await;
}
```

Note: the origin's root ops routes (`/healthz`) are merged inside `router()`; a `default`/empty `streams` state still serves `/healthz` (the `router()` merge at `origin/mod.rs:314` is unconditional). If that is not the case after W2a, the test constructs a one-stream state instead — the assertion is behavioural, not structural.

- [ ] **Step 2: Run the pre-fix code — expect FAIL**

Run with a stub `serve_hyper_util_with_timeout` that just calls `axum::serve(...).await` (the pre-fix behaviour):

```bash
timeout 60 cargo test -p multimux --all-features --locked --test server_timeouts 2>&1 | grep -E 'test result|FAILED|panicked'
```

Expected FAIL: `a_slow_header_client_is_dropped_at_the_header_read_timeout` times out at 2 s (`Err(Elapsed)`) because `axum::serve` sets no header timeout — the write never completes. This is the pre-fix behavioural failure, not a compile error.

- [ ] **Step 3: Implement `serve_hyper_util` (builds the per-connection service itself — `axum::serve::IncomingStream`'s fields are PRIVATE, so it cannot be constructed outside axum; verified axum 0.8.9 `serve/mod.rs:424-430`)**

```rust
use std::{net::SocketAddr, sync::Arc, time::Duration};

use axum::extract::connect_info::ConnectInfo;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use hyper_util::server::conn::auto::Builder;
use hyper_util::service::TowerToHyperService;
use tower::Layer as _;   // for `Extension(..).layer(..)` (round-3 D1 note)

pub const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(10);
pub const MAX_CONNECTIONS: usize = 1024;
pub const TOTAL_CONNECTION_TIMEOUT: Duration = Duration::from_secs(30);

/// Serve `app` on `listener` through hyper-util's auto builder with a Tokio
/// timer, [`HEADER_READ_TIMEOUT`], a [`MAX_CONNECTIONS`] semaphore, a
/// [`TOTAL_CONNECTION_TIMEOUT`] total per-connection deadline, and graceful
/// shutdown on `token`.
///
/// `ConnectInfo<SocketAddr>` (read by `output_auth_gate`) is injected by
/// wrapping the router in `axum::Extension` per connection — the same
/// mechanism axum's own `IntoMakeServiceWithConnectInfo` uses, built here
/// because `axum::serve::IncomingStream` cannot be constructed outside axum
/// (private fields, axum 0.8.9 `serve/mod.rs:424`).
pub(crate) async fn serve_hyper_util(
    listener: tokio::net::TcpListener,
    app: axum::Router,
    token: tokio_util::sync::CancellationToken,
) -> std::io::Result<()> {
    serve_hyper_util_with_timeout(listener, app, token, HEADER_READ_TIMEOUT, TOTAL_CONNECTION_TIMEOUT).await
}

#[doc(hidden)]
pub async fn serve_hyper_util_with_timeout(
    listener: tokio::net::TcpListener,
    app: axum::Router,
    token: tokio_util::sync::CancellationToken,
    header_read: Duration,
    total: Duration,
) -> std::io::Result<()> {
    let conns = Arc::new(tokio::sync::Semaphore::new(MAX_CONNECTIONS));
    let tracked = tokio_util::task::TaskTracker::new();

    loop {
        let permit = tokio::select! {
            () = token.cancelled() => break,
            p = Arc::clone(&conns).acquire_owned() => p.expect("conn semaphore never closed"),
        };
        let (stream, remote_addr) = tokio::select! {
            () = token.cancelled() => break,
            accepted = listener.accept() => accepted?,
        };

        let app = app.clone();
        let child = token.child_token();
        tracked.spawn(async move {
            let _permit = permit;

            // Per-connection service: the router with the peer address in
            // the request extensions — what `ConnectInfo`'s extractor reads.
            // `Router<()>` implements `Service<Request<B>>` with
            // `Error = Infallible` (axum 0.8.9 routing/mod.rs:569).
            let svc = axum::Extension(ConnectInfo(remote_addr)).layer(app);

            let mut builder = Builder::new(TokioExecutor::new());
            builder.http1().timer(TokioTimer::new()).header_read_timeout(Some(header_read));

            let io = TokioIo::new(stream);
            let conn = builder.serve_connection(io, TowerToHyperService::new(svc));
            let mut conn = std::pin::pin!(conn);

            tokio::select! {
                () = child.cancelled() => conn.as_mut().graceful_shutdown(),
                () = tokio::time::sleep(total) => conn.as_mut().graceful_shutdown(),
                _ = conn.as_mut() => {}
            }
        });
    }
    tracked.close();
    // Drain in-flight connections before returning. TaskTracker has NO
    // join_next; its drain API is close() + wait() (verified tokio-util
    // 0.7.19 task_tracker.rs:318/337).
    tracked.wait().await;
    Ok(())
}
```

> **Compile-verified** (scratch crate, hyper 1.11 / hyper-util 0.1.20 / axum 0.8.9 / tokio-util 0.7, with exactly this task's Cargo feature set — `hyper = ["server","http1"]`, `hyper-util = ["server","server-auto","http1","tokio","service"]`; `server-auto` itself pulls `http2`): both `serve_hyper_util_with_timeout` and `serve_hyper_util_service` build, the spawned futures are `Send`, and a call site passing `move |addr| async move { Extension(ConnectInfo(addr)).layer(router) }` type-checks. Two corrections from that run: (1) **`S: Clone` is required** — `TowerToHyperService<S>: hyper::service::Service<R>` needs `S: Clone` (hyper-util 0.1.20 `service/glue.rs:33-36`: `S: tower_service::Service<R> + Clone`; it clones the service per request), and without it rustc reports `S: Clone` unsatisfied at `serve_connection`. (2) **`TokioTimer` is `#[non_exhaustive]` (`rt/tokio.rs:89-91`), so it cannot be named as a value outside hyper-util — `.timer(TokioTimer::new())`.** `S::Future: Send + 'static` and the `Send` bound on `S` are what `TokioExecutor`'s `HttpServerConnExec` needs. `Router`, `Extension<..>.layer(Router)` (a `Route`) and `BudgetLimit<..>` are all `Clone`.

> Verified against sources: `Router<()>: Service<Request<B>>`, `Error = Infallible` (axum 0.8.9 `routing/mod.rs:569`), so `TowerToHyperService::new(svc)` satisfies hyper's `Service<Request<Incoming>, Error: Into<BoxError>>` (Infallible: Into). `axum::Extension(…).layer(router)` is a `tower::Layer` inserting the extension into every request — the same trick `IntoMakeServiceWithConnectInfo`'s `AddExtension` uses (axum `extract/connect_info.rs:141`). `TaskTracker::{close, wait}` exist; `join_next` does not. HTTP/2 slow-loris: the origin's listeners are plain HTTP/1 (no h2c/TLS-ALPN upgrade exists in any `serve*` path), so the auto builder's h2 half is inert — state this in the doc comment.

**The admin media listener (`DynamicMediaService`) is NOT a `Router`; write the second overload in the same file:**

```rust
/// [`serve_hyper_util`] for a caller-supplied per-connection service
/// factory — the admin media listener's `DynamicMediaService` shape.
/// `build` receives only the PEER address: the service it builds is a
/// per-request dispatcher over shared state (`DynamicMediaService` holds an
/// `Arc<RouteRegistry>`, not the stream — admin.rs:956-970), so the stream
/// is never moved into the closure and stays owned by the connection task
/// for the `TokioIo` wrap below. All timeouts/bounds are identical to
/// [`serve_hyper_util`].
pub(crate) async fn serve_hyper_util_service<S, F, Fut>(
    listener: tokio::net::TcpListener,
    build: F,
    token: tokio_util::sync::CancellationToken,
) -> std::io::Result<()>
where
    F: Fn(SocketAddr) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = S> + Send + 'static,
    S: tower::Service<
            hyper::Request<hyper::body::Incoming>,
            Response = axum::response::Response,
            Error = std::convert::Infallible,
        > + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
{
    let conns = Arc::new(tokio::sync::Semaphore::new(MAX_CONNECTIONS));
    let tracked = tokio_util::task::TaskTracker::new();
    // `build` is `Arc`ed: a `&build` local cannot move into a `'static`
    // spawn (the round-3 review's D1 compile bug).
    let build = Arc::new(build);
    loop {
        let permit = tokio::select! {
            () = token.cancelled() => break,
            p = Arc::clone(&conns).acquire_owned() => p.expect("conn semaphore never closed"),
        };
        let (stream, remote_addr) = tokio::select! {
            () = token.cancelled() => break,
            accepted = listener.accept() => accepted?,
        };
        let child = token.child_token();
        let build = Arc::clone(&build);
        tracked.spawn(async move {
            let _permit = permit;
            // Build from the peer alone; the stream stays owned HERE.
            let svc = build(remote_addr).await;
            let mut builder = Builder::new(TokioExecutor::new());
            builder.http1().timer(TokioTimer::new()).header_read_timeout(Some(HEADER_READ_TIMEOUT));
            let io = TokioIo::new(stream);
            let conn = builder.serve_connection(io, TowerToHyperService::new(svc));
            let mut conn = std::pin::pin!(conn);
            tokio::select! {
                () = child.cancelled() => conn.as_mut().graceful_shutdown(),
                () = tokio::time::sleep(TOTAL_CONNECTION_TIMEOUT) => conn.as_mut().graceful_shutdown(),
                _ = conn.as_mut() => {}
            }
        });
    }
    tracked.close();
    tracked.wait().await;
    Ok(())
}
```

> `DynamicMediaService::call` (admin.rs:970) already re-reads the router per request through `current_router()`; the closure passed at the call site closes over `Arc<RouteRegistry>` and builds `DynamicMediaService { registry: … }` per connection. If its `Service` impl needs extra request plumbing (e.g. its own `ConnectInfo`), adapt the closure at the call site — the overload's contract (build-per-connection, Infallible) is what matters.

- [ ] **Step 4: Wire `serve_with_registry_impl` and `serve_with_admin`**

In `origin/mod.rs`, replace `axum::serve(listener, router(state)…).with_graceful_shutdown(shutdown_future).await` (1037–1042) with `serve_hyper_util(listener, router(state), cancel.clone()).await`, keeping the existing `shutdown_future` that calls `cancel.cancel()`. In `admin.rs`: the admin-API listener (1100) becomes `serve_hyper_util(admin_listener, admin_built, admin_cancel)`; the media listener (1112) becomes

```rust
let media_registry = Arc::clone(&registry);
let media_result = crate::origin::serve_hyper_util_service(
    media_listener,
    move |addr| {
        let registry = Arc::clone(&media_registry);
        async move {
            axum::Extension(ConnectInfo(addr)).layer(dynamic_media_service(registry))
        }
    },
    media_cancel,
)
.await;
```

where `dynamic_media_service(registry)` wraps `MediaMakeService`'s current per-request path (`DynamicMediaService` at admin.rs:956) into the per-connection service the overload requires — adapt to its real constructor shape at implementation time, preserving the per-request `current_router()` read.

- [ ] **Step 5: Run — expect PASS**

```bash
timeout 120 cargo test -p multimux --all-features --locked --test server_timeouts 2>&1 | grep -E 'test result|FAILED|panicked'
timeout 900 cargo test -p multimux --all-features --locked --test admin_api 2>&1 | grep -E 'test result|FAILED'
```

- [ ] **Step 6: Revert-check**

Delete `.header_read_timeout(Some(header_read))` (i.e. pass `None`), keep the test at 200 ms, re-run Step 5's first command. Expected FAIL: `a_slow_header_client…` times out. Restore; re-run to PASS. Record in `.delegate/w2a-report.md`.

- [ ] **Step 7: Commit**

```bash
git add multimux
git commit -m "fix(multimux): serve every HTTP listener through hyper-util with a header-read and total-connection deadline"
```

---

### Task 3: origin limit.rs on tower's layers; arc-swap admin router; typed `_HLS_*`; typed Cache-Control

**Files:**
- Modify `multimux/src/origin/limit.rs` — reduce to the 3-budget classification plus ONE `tower::Service` (`BudgetLimit`) that classifies, takes a permit from the matching pool (a `tokio::sync::Semaphore`, the same pool object `tower::limit::GlobalConcurrencyLimitLayer::with_semaphore` holds) bounded by the queue wait, sheds `503`+`Retry-After`, and pins the permit to the response body. The request timeout stays `tower_http::timeout::TimeoutLayer`.
- Modify `multimux/src/origin/mod.rs` — `router()`'s layer stack (314–328); `add_response_headers` (435–477) on `headers` typed values.
- Modify `multimux/src/origin/admin.rs` — `router_slot: RwLock<Router>` → `ArcSwap<Router>`.
- Modify `multimux/Cargo.toml` — `tower = { version = "0.5", default-features = false, features = ["util"] }` (`ServiceExt::ready`/`oneshot` only: the limit/load-shed/timeout layers are no longer layered); `http-body = "1"` (`Frame`/`SizeHint` for `PermitBody`: `axum::body` re-exports only `HttpBody`/`Body`/`Bytes`, verified axum 0.8.9 `body/mod.rs`; `http-body` 1.1.0 is already in `Cargo.lock`, so no new lock version); `headers = "0.4"`; `arc-swap = "1"`.
- New test `multimux/tests/limit_budgets.rs`.

**Interfaces:**
- Produces:
  ```rust
  // limit.rs
  pub enum Budget { Exempt, BlockingReload, Ordinary }   // + label(); main has it PRIVATE (limit.rs:69) -> `pub` + #[doc(hidden)]
  pub fn classify(uri: &axum::http::Uri) -> Budget;
  pub struct BudgetLimitLayer { /* Arc<Semaphore> x2 + queue_timeout */ }
  impl BudgetLimitLayer { pub fn new(ordinary: usize, reload: usize, queue_timeout: Duration) -> Self; }
  impl<S> tower::Layer<S> for BudgetLimitLayer { type Service = BudgetLimit<S>; }
  pub struct BudgetLimit<S> { /* inner + the SAME Arc<Semaphore>s */ }
  impl<S, B> tower::Service<Request<B>> for BudgetLimit<S>
  where S: Service<Request<B>, Response = Response<Body>, Error = Infallible> + Clone + Send + 'static;
  pub struct PermitBody { inner: axum::body::Body, _permit: OwnedSemaphorePermit }
  ```
  The old `GlobalLimit`/`GlobalLimitLayer`/`GlobalLimitService` and `blocking_reload_marker` are **removed**; `origin::HttpLimits` keeps its four fields. **The layer OWNS the pools** (`Router::layer` calls `Layer::layer` once per route; pools created inside `layer()` are one pool per route, i.e. no shared budget — verified: that shape returns `200` where the test below expects `503`).
- Consumes: `tokio::sync::Semaphore::acquire_owned`, `tokio::time::timeout`, `http_body::{Frame, SizeHint}`, `axum::body::{Body, HttpBody}`, `tower::ServiceExt::ready`, `url::form_urlencoded` and `tower_http::timeout::TimeoutLayer::with_status_code` (0.7.1, verified non-deprecated). **Compile-verified** in a scratch crate against axum 0.8.9 / tower 0.5.3 / http-body 1.1.0 (the code below is that file verbatim; its two tests pass, and each mutation named in Step 6 was run and fails).

- [ ] **Step 1: Write the failing tests**

`multimux/tests/limit_budgets.rs`:

```rust
//! SP2.5: three budgets (ops-exempt / blocking-reload / ordinary) enforced by
//! one classify service over shared pools; a `_HLS_*` request is classified
//! by a typed query parse, not a substring scan. The permit lives in the
//! RESPONSE BODY, so a test HOLDS a permit by keeping a response whose body
//! it neither polls nor drops.

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

fn get(path: &str) -> Request<Body> {
    Request::builder().uri(path).body(Body::empty()).unwrap()
}

#[test]
fn a_query_that_merely_contains_the_marker_is_not_a_blocking_reload() {
    use multimux::origin::limit::{Budget, classify};
    let u = |s: &str| s.parse::<axum::http::Uri>().unwrap();
    assert_eq!(classify(&u("/cam1/media.m3u8?_HLS_msnx=1")), Budget::Ordinary);
    assert_eq!(classify(&u("/cam1/media.m3u8?file=_HLS_msn")), Budget::Ordinary);
    assert_eq!(classify(&u("/cam1/media.m3u8?_HLS_msn=5")), Budget::BlockingReload);
    assert_eq!(classify(&u("/cam1/media.m3u8?_HLS_part=2")), Budget::BlockingReload);
    assert_eq!(classify(&u("/metrics")), Budget::Exempt);
}

/// One ordinary permit, held by an unread response body: a second ordinary
/// request — on a DIFFERENT route, proving the pool is shared across routes —
/// waits `queue_timeout`, then is shed `503` + `Retry-After`; an ops route
/// still answers; dropping the held response frees the permit.
///
/// Revert-checks (each run and observed to FAIL): (a) release the permit when
/// `call` returns (`drop(permit); Ok(resp)` instead of `pin_permit`) — the
/// second request is not shed (`200`, not `503`); (b) build the pools inside
/// `Layer::layer` instead of in `BudgetLimitLayer::new` — `Router::layer`
/// then makes one pool per route and the cross-route request gets `200`.
#[tokio::test(start_paused = true)]
async fn an_exhausted_ordinary_budget_sheds_and_ops_routes_still_answer() {
    let app = multimux::origin::limit_budget_test_app(1, Duration::from_millis(50));
    let hold = app.clone().oneshot(get("/cam1/master.m3u8")).await.unwrap();

    let shed = app.clone().oneshot(get("/cam1/media.m3u8")).await.unwrap();
    assert_eq!(shed.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(shed.headers()["retry-after"], "1");

    let ops = app.clone().oneshot(get("/healthz")).await.unwrap();
    assert_eq!(ops.status(), StatusCode::OK, "ops routes are exempt from every budget");

    drop(hold);
    let again = app.clone().oneshot(get("/cam1/media.m3u8")).await.unwrap();
    assert_ne!(again.status(), StatusCode::SERVICE_UNAVAILABLE, "the permit is freed when the body drops");
}
```

`limit_budget_test_app(ordinary, queue) -> axum::Router` (a `#[doc(hidden)] pub fn` in `origin/mod.rs`) is the real `router()` built with `HttpLimits { max_concurrent_requests: ordinary, queue_timeout: queue, ..Default::default() }` over a one-stream state named `cam1` (a real `RouteHandle` with no published program, so `/cam1/master.m3u8` and `/cam1/media.m3u8` exist and answer without media). Which status the held request itself gets (404/503 from the route) is irrelevant: the permit is pinned to the response body whatever its status, so the held response occupies the slot.

- [ ] **Step 2: Run pre-fix — expect FAIL**

First add `limit_budget_test_app` over the CURRENT `router()` (it needs nothing new). Then: `a_query_that_merely_contains_the_marker…` FAILS (the old `.contains()` scan classifies `_HLS_msnx=1` as a blocking reload); `an_exhausted_ordinary_budget_sheds_and_ops_routes_still_answer` FAILS behaviourally: the old `GlobalLimitService` releases its permit when the response FUTURE completes, so after the `hold` request returns the second request is not shed (`200`/`404`, not `503`). Record both.


- [ ] **Step 3: One classify service over shared pools**

Keep `Budget`/`classify` (typed form-parse); delete the hand-rolled `GlobalLimit`/`GlobalLimitLayer`/`GlobalLimitService` and `blocking_reload_marker`; write `origin/limit.rs`'s service as below. **This file compiles** (scratch crate, axum 0.8.9 / tower 0.5.3 / http-body 1.1.0 / tokio 1.53; `use` lines kept), and its permit-lifetime test passes: a second request is shed while a response body is held, freed when it drops.

```rust
use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use axum::body::{Body, Bytes, HttpBody};
use http_body::{Frame, SizeHint};
use axum::http::{Request, Response, Uri};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tower::ServiceExt as _;

/// Main's existing consts, unchanged (limit.rs:58-66).
const OPS_PATHS: [&str; 3] = ["/healthz", "/readyz", "/metrics"];
const RETRY_AFTER: axum::http::HeaderValue = axum::http::HeaderValue::from_static("1");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Budget {
    Exempt,
    BlockingReload,
    Ordinary,
}

impl Budget {
    pub fn label(self) -> &'static str {
        match self {
            Budget::Exempt => "exempt",
            Budget::BlockingReload => "blocking_reload",
            Budget::Ordinary => "ordinary",
        }
    }
}

/// The 3-budget classification (the one thing this module keeps): ops routes
/// are exempt; a request carrying an `_HLS_msn`/`_HLS_part` query KEY (typed
/// form parse, never a substring scan) is a blocking reload; the rest are
/// ordinary.
pub fn classify(uri: &Uri) -> Budget {
    if OPS_PATHS.contains(&uri.path()) {
        return Budget::Exempt;
    }
    let blocking = uri.query().is_some_and(|q| {
        url::form_urlencoded::parse(q.as_bytes()).any(|(k, _)| k == "_HLS_msn" || k == "_HLS_part")
    });
    if blocking { Budget::BlockingReload } else { Budget::Ordinary }
}

/// 503 + `Retry-After` (RFC 9110 §15.6.4 / §10.2.3) + the existing
/// `HTTP_SHED_TOTAL` counter (label `kind`, as on main). The mapping lives
/// here because `BudgetLimit::call` must return `Infallible`.
fn shed_into_response(budget: Budget) -> Response<Body> {
    metrics::counter!(crate::prometheus::HTTP_SHED_TOTAL, "kind" => budget.label()).increment(1);
    let mut resp = axum::response::IntoResponse::into_response(axum::http::StatusCode::SERVICE_UNAVAILABLE);
    resp.headers_mut().insert(axum::http::header::RETRY_AFTER, RETRY_AFTER);
    resp
}

/// The layer OWNS the two pools. `Router::layer` calls `Layer::layer` once
/// per route, so pools created inside `layer()` would be one pool per route
/// (no shared budget at all); the layer holds the `Arc<Semaphore>`s and each
/// produced service clones them — the same shape as tower's
/// `GlobalConcurrencyLimitLayer::with_semaphore`.
#[derive(Clone)]
pub struct BudgetLimitLayer {
    ordinary: Arc<Semaphore>,
    reload: Arc<Semaphore>,
    queue_timeout: Duration,
}

impl BudgetLimitLayer {
    pub fn new(ordinary: usize, reload: usize, queue_timeout: Duration) -> Self {
        Self {
            ordinary: Arc::new(Semaphore::new(ordinary.max(1))),
            reload: Arc::new(Semaphore::new(reload.max(1))),
            queue_timeout,
        }
    }
}

impl<S> tower::Layer<S> for BudgetLimitLayer {
    type Service = BudgetLimit<S>;
    fn layer(&self, inner: S) -> BudgetLimit<S> {
        BudgetLimit {
            inner,
            ordinary: Arc::clone(&self.ordinary),
            reload: Arc::clone(&self.reload),
            queue_timeout: self.queue_timeout,
        }
    }
}

#[derive(Clone)]
pub struct BudgetLimit<S> {
    inner: S,
    ordinary: Arc<Semaphore>,
    reload: Arc<Semaphore>,
    queue_timeout: Duration,
}

/// A response body that owns a pool permit.
pub struct PermitBody {
    inner: Body,
    _permit: OwnedSemaphorePermit,
}

/// Marker extension: this response's body holds a permit.
#[derive(Clone)]
pub struct PermitHeld;

impl PermitBody {
    fn pin_permit(resp: Response<Body>, permit: OwnedSemaphorePermit) -> Response<Body> {
        let (mut parts, body) = resp.into_parts();
        parts.extensions.insert(PermitHeld);
        Response::from_parts(parts, Body::new(PermitBody { inner: body, _permit: permit }))
    }
}

impl HttpBody for PermitBody {
    type Data = Bytes;
    type Error = axum::Error;
    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, axum::Error>>> {
        // `axum::body::Body` is `Unpin`, so `PermitBody` is too.
        Pin::new(&mut self.get_mut().inner).poll_frame(cx)
    }
    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }
    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

impl<S, B> tower::Service<Request<B>> for BudgetLimit<S>
where
    B: Send + 'static,
    S: tower::Service<Request<B>, Response = Response<Body>, Error = Infallible> + Clone + Send + 'static,
    S::Future: Send + 'static,
{
    type Response = Response<Body>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Response<Body>, Infallible>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Infallible>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<B>) -> Self::Future {
        let budget = classify(req.uri());
        let queue_timeout = self.queue_timeout;
        let pool = match budget {
            Budget::Exempt => {
                let mut svc = self.inner.clone();
                return Box::pin(async move { svc.ready().await?.call(req).await });
            }
            Budget::BlockingReload => Arc::clone(&self.reload),
            Budget::Ordinary => Arc::clone(&self.ordinary),
        };
        let mut svc = self.inner.clone();
        Box::pin(async move {
            let permit = match tokio::time::timeout(queue_timeout, pool.acquire_owned()).await {
                Ok(Ok(p)) => p,
                Ok(Err(_)) | Err(_) => return Ok(shed_into_response(budget)),
            };
            let resp = svc.ready().await?.call(req).await?;
            Ok(PermitBody::pin_permit(resp, permit))
        })
    }
}
```

> Three verified details that the first draft had wrong: (1) `S::Error = Infallible` is in the impl bound, so `svc.ready().await?` and `poll_ready` need no `match e {}`; (2) `PermitBody` is NOT generic — `pin_permit` always wraps an `axum::body::Body`, which is `Unpin`, so there is no `B: Unpin` bound and no uninferrable `B`; (3) `PermitHeld` derives `Clone` (`Extensions::insert` needs `Clone + Send + Sync + 'static`). The `GlobalConcurrencyLimitLayer`/`LoadShed` layers are not layered: the acquire timeout IS the shed and the `Arc<Semaphore>` IS tower's pool object. Old behaviour queued up to `queue_timeout` then `503`; new behaviour waits the same `queue_timeout` then sheds — only the permit's lifetime moves to the response body (declared in the CHANGELOG).


Wire it in `origin::router()` (replacing `.layer(limit::GlobalLimitLayer::new(…))` at 324–327) with `BudgetLimitLayer::new(limits.max_concurrent_requests, (limits.max_concurrent_requests / DEFAULT_BLOCKING_RELOAD_DIVISOR).max(1), limits.queue_timeout)` — constructed ONCE and passed to `.layer(..)` (the pools live in the layer value). The root ops routes are merged before the layer, so they pass through `BudgetLimit` and are exempted by `classify`, not by position.

- [ ] **Step 4: Replace the substring scan (done in Step 3's `classify`) and the Cache-Control/CORS consts**

`classify` uses `url::form_urlencoded::parse(q.as_bytes()).any(|(k,_)| k == "_HLS_msn" || k == "_HLS_part")` (typed; `url` is already a multimux dep). In `add_response_headers`, replace `HeaderValue::from_static` with `headers` typed inserts (`AccessControlAllowOrigin::ANY`, `AccessControlAllowMethods`, `AccessControlAllowHeaders` (the explicit `Authorization, Range, Content-Type` list — `*` never covers `Authorization`), `AccessControlExposeHeaders`, `CacheControl`, `Vary`). For `CacheControl`, the value `max-age=31536000, immutable` is produced by `.parse::<CacheControl>()` — assert the rendered bytes equal the golden; if `headers::CacheControl` cannot represent `immutable` (it can — the `CacheDirective::Immutable` exists), the golden is the contract and any byte diff is listed in the CHANGELOG.

- [ ] **Step 5: `arc-swap` admin router**

`origin/admin.rs`: `router_slot: ArcSwap<Router>` (init `ArcSwap::from_pointee(Router::new())`); `rebuild_router` (289) `.store(Arc::new(new_router))`; `current_router` (296) `self.router_slot.load().as_ref().clone()`; the two other `*crate::lock::write(&self.router_slot) = …` sites (465, 667) `.store(Arc::new(…))`. `crate::lock` stays for W2b (SP6.6).

- [ ] **Step 6: Run — expect PASS + revert-check**

```bash
timeout 120 cargo test -p multimux --all-features --locked --test limit_budgets 2>&1 | grep -E 'test result|FAILED'
timeout 600 cargo test -p multimux --all-features --locked --test admin_api --test origin_llhls 2>&1 | grep -E 'test result|FAILED'
```

Revert-checks (run each, record the failure line): (a) restore `classify`'s old `q.contains("_HLS_msn")` scan — `a_query_that_merely_contains_the_marker…` FAILS; (b) in `BudgetLimit::call` replace `Ok(PermitBody::pin_permit(resp, permit))` with `{ drop(permit); Ok(resp) }` — `an_exhausted_ordinary_budget_sheds…` FAILS (`left: 200 / right: 503`... the second request is no longer shed); (c) move the `Arc::new(Semaphore::new(..))` from `BudgetLimitLayer::new` into `Layer::layer` — the same test FAILS (`Router::layer` makes one pool per route; the cross-route request gets `200`). (b) and (c) were run against the compile-verified scratch file and both fail as described. Record all three in `.delegate/w2a-report.md`.

- [ ] **Step 7: Commit**

```bash
git add multimux
git commit -m "refactor(multimux): origin budgets on tower's concurrency/load-shed/timeout; arc-swap admin router; typed _HLS_* and Cache-Control"
```

---

### Task 4: WHIP signalling on the axum router (delete `webrtc_http.rs`); session resource routes

**Files:**
- Modify `multimux/src/source/whip.rs` — `ensure_infra` (249–323) and `handle_whip_connection` (520–695) become an axum `Router` served via `serve_hyper_util`; add `WhipRoute::with_listener` (bind-port-0 harness, moved here from the old W2b Task 4).
- Delete `multimux/src/webrtc_http.rs`; move only `SessionSlot` to a new `multimux/src/webrtc_session.rs`.
- Modify `multimux/src/lib.rs` — `mod webrtc_http;` → `mod webrtc_session;`.
- New test `multimux/tests/whip_http.rs`.

**Interfaces:**
- Produces:
  ```rust
  // source/whip.rs
  pub fn WhipRoute::with_listener(name: impl Into<String>, listener: tokio::net::TcpListener, max_sessions: usize) -> Self;
  pub(crate) fn whip_router(state: Arc<WhipServeState>) -> axum::Router;
  #[doc(hidden)] pub async fn serve_for_test() -> (SocketAddr, WhipRoute, CancellationToken);
  ```
- Consumes: `webrtc_runtime::whip::server::{WhipSession, State, Event}` (**`Event::SdpOffer(Vec<u8>)`, `Event::TrickleIce{..}`, `Event::IceRestart{..}`, `Event::Terminated`** — verified `webrtc-runtime/src/whip/server.rs:31-53`), `webrtc_runtime::whip::server::HttpResponse` (re-export), `headers::{ContentType, ETag, IfMatch, Location, RetryAfter}`, axum `Router`/`Path`/`HeaderMap`/`Bytes`.

- [ ] **Step 1: Write the failing test**

`multimux/tests/whip_http.rs` (real assertions, no comment bodies):

```rust
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const OFFER: &str = "v=0\r\n\
o=- 0 0 IN IP4 127.0.0.1\r\n\
s=-\r\n\
t=0 0\r\n\
m=video 9 UDP/TLS/RTP/SAVPF 96\r\n\
c=IN IP4 0.0.0.0\r\n\
a=ice-ufrag:abcd\r\n\
a=ice-pwd:abcdefghijklmnopqrstuvwx\r\n\
a=fingerprint:sha-256 00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff:\
00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff\r\n\
a=setup:actpass\r\n\
a=mid:0\r\n\
a=rtcp-mux\r\n\
a=rtpmap:96 H264/90000\r\n\
a=fmtp:96 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f\r\n";

async fn raw(addr: std::net::SocketAddr, req: &str) -> String {
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.write_all(req.replace('\n', "\r\n").as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), s.read_to_end(&mut buf)).await;
    String::from_utf8_lossy(&buf).into_owned()
}

fn post_offer(addr: std::net::SocketAddr) -> String {
    let body = OFFER.to_string();
    raw(addr, &format!(
        "POST /whip HTTP/1.1\nHost: x\nContent-Type: application/sdp\n\
         Content-Length: {}\nConnection: close\n\n{body}",
        body.len()
    ))
}

#[tokio::test]
async fn whip_post_is_answered_201_with_a_location_and_typed_content_type() {
    let (addr, _route, _token) = multimux::source::whip::serve_for_test().await;
    let resp = post_offer(addr).await;
    let lower = resp.to_ascii_lowercase();
    assert!(resp.starts_with("HTTP/1.1 201"), "{resp}");
    assert!(lower.contains("content-type: application/sdp"), "{resp}");
    assert!(lower.contains("location:"), "RFC 9725 §4.1 mandates Location: {resp}");
}

#[tokio::test]
async fn whip_options_preflight_is_204_with_cors() {
    let (addr, _route, _token) = multimux::source::whip::serve_for_test().await;
    let resp = raw(addr, "OPTIONS /whip HTTP/1.1\nHost: x\nOrigin: http://example\nAccess-Control-Request-Method: POST\nConnection: close\n\n").await;
    assert!(resp.starts_with("HTTP/1.1 204"), "{resp}");
    assert!(resp.to_ascii_lowercase().contains("access-control-allow-origin:"), "{resp}");
}

#[tokio::test]
async fn a_body_over_64_kib_is_rejected_413_before_reading() {
    let (addr, _route, _token) = multimux::source::whip::serve_for_test().await;
    let resp = raw(addr, "POST /whip HTTP/1.1\nHost: x\nContent-Type: application/sdp\nContent-Length: 131073\nConnection: close\n\n").await;
    assert!(resp.starts_with("HTTP/1.1 413"), "{resp}");
}

#[tokio::test]
async fn a_chunked_request_body_is_accepted() {
    let (addr, _route, _token) = multimux::source::whip::serve_for_test().await;
    let mut s = TcpStream::connect(addr).await.unwrap();
    let body = OFFER;
    s.write_all(format!(
        "POST /whip HTTP/1.1\nHost: x\nContent-Type: application/sdp\n\
         Transfer-Encoding: chunked\nConnection: close\n\n{:x}\r\n{}\r\n0\r\n\r\n",
        body.len(), body
    ).as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), s.read_to_end(&mut buf)).await;
    let text = String::from_utf8_lossy(&buf);
    assert!(text.starts_with("HTTP/1.1 201"), "chunked body must be accepted: {text}");
}
```

- [ ] **Step 2: Run pre-fix — expect FAIL**

Against the pre-fix raw reader, `a_chunked_request_body_is_accepted` FAILS (411 — the raw reader rejects chunked), and `whip_options_preflight`/`whip_post` still pass pre-fix but the chunked + `serve_for_test` (absent) are the failing bits. Record the chunked rejection as the behavioural pre-fix failure.

- [ ] **Step 3: Replace the accept loop with an axum router**

`ensure_infra` binds the listener (via `self.listen`, or the pre-bound one if built with `with_listener`), constructs `whip_router(state)`, spawns `serve_hyper_util(listener, make, cancel)` on a tracked task (Task 6 finishes tracking). The router carries the **session resource** routes, mirroring WHEP:

```rust
fn whip_router(state: Arc<WhipServeState>) -> axum::Router {
    axum::Router::new()
        .route("/whip", post(whip_post).options(whip_options))
        .route("/whip/session", patch(whip_patch).delete(whip_delete))
        .layer(tower::limit::ConcurrencyLimitLayer::new(crate::webrtc_session::MAX_PENDING_HTTP_CONNECTIONS))
        .layer(tower_http::limit::RequestBodyLimitLayer::new(64 * 1024))
        .layer(tower_http::timeout::TimeoutLayer::with_status_code(StatusCode::REQUEST_TIMEOUT, Duration::from_secs(10)))
        .layer(tower_http::cors::CorsLayer::permissive().allow_headers(tower_http::cors::Any).expose_headers(tower_http::cors::Any))
        .with_state(state)
}
```

`whip_post` drives `WhipSession::on_post(body)` → `match Event::SdpOffer(offer) { … }`: parses the offer (`parse_whip_offer`, Task 8), binds the media socket, builds the `MediaTransport`, then `session.accept(answer, etag)` → `HttpResponse` → axum `Response` (a shared `to_axum`). `whip_patch`/`whip_delete` drive `WhipSession::on_patch`/`on_delete`. `whip_options` returns `204` (CorsLayer supplies headers).

- [ ] **Step 4: Run — expect PASS**

```bash
timeout 150 cargo test -p multimux --all-features --locked --features whip,whep --test whip_http 2>&1 | grep -E 'test result|FAILED'
timeout 1800 cargo test -p multimux --all-features --locked --features whip,whep --test whip_ingest -- --nocapture 2>&1 | grep -E 'test result|FAILED|real result'
```

- [ ] **Step 5: Confirm the reader is gone + commit**

```bash
grep -rn 'webrtc_http\|read_http_request\|ReadRequestError' multimux/src multimux/tests | grep -v 'whip_http.rs'
git add multimux && git commit -m "refactor(multimux)!: WHIP on axum (session routes, chunked bodies, CorsLayer); delete the raw HTTP reader"
```

---

### Task 5: WHEP egress on the axum router (output auth as middleware, shared with origin)

**Files:**
- Modify `multimux/src/output/whep.rs` — `run_whep` (1014–1113) + `handle_whep_connection` (457–659) become a `Router`; `check_output_auth` (422–448) becomes an axum middleware shared with `origin::output_auth_gate`; add `WhepRoute::with_listener`.
- Modify `multimux/src/origin/mod.rs` — make `output_auth_gate` (353–421) `pub(crate)` so `whep` calls it rather than duplicating.
- New test `multimux/tests/whep_http.rs`.

**Interfaces:**
- Produces: `pub(crate) fn whep_router(state: Arc<WhepServeState>) -> axum::Router;` `#[doc(hidden)] pub async fn serve_whep_for_test(...) -> (Router, CancellationToken)`.
- Consumes: `webrtc_runtime::whep::server::{WhepSession, Event}` (verified: `Event::SdpOffer/SdpAnswer/TrickleIce/IceRestart/Terminated`), `broadcast_auth::{Verifier, AuthResult, RequestContext}`.

- [ ] **Step 1: Write the failing test**

`multimux/tests/whep_http.rs` — full bodies (the 401+CORS ordering is the biting one: the pre-fix raw listener's hand-built 401 carries no CORS header, so a cross-origin browser player cannot even see the challenge; the new middleware order must keep both):

```rust
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

const OFFER: &str = /* the same full H.264 offer literal as whip_http.rs */;

/// BITING: pre-fix, `check_output_auth` (output/whep.rs:443-447) hand-builds
/// the 401 with only `WWW-Authenticate` — no `Access-Control-Allow-Origin` —
/// so this FAILS on old code. The new order (CorsLayer outside the auth
/// middleware) must produce both headers.
#[tokio::test]
async fn whep_post_without_credentials_is_401_with_challenge_and_cors() {
    let (app, _token) = multimux::output::whep::serve_whep_for_test(
        Some(basic_verifier("user", "pass")),
    )
    .await;
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/whep")
                .header("content-type", "application/sdp")
                .header("origin", "http://player.example")
                .body(Body::from(OFFER))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert!(
        resp.headers().contains_key("www-authenticate"),
        "the Basic/Digest challenge must be present"
    );
    assert_eq!(
        resp.headers()["access-control-allow-origin"], "*",
        "CORS must survive the auth middleware (it is layered outside)"
    );
}

/// CHARACTERISATION (pins existing behaviour across the router move).
#[tokio::test]
async fn whep_post_with_credentials_against_a_live_trunk_is_201_with_location() {
    let (app, _token) = multimux::output::whep::serve_whep_for_test_with_trunk(None).await;
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/whep")
                .header("content-type", "application/sdp")
                .body(Body::from(OFFER))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    assert!(resp.headers().contains_key("location"), "Location must be present");
    assert_eq!(resp.headers()["content-type"], "application/sdp");
}

/// CHARACTERISATION.
#[tokio::test]
async fn whep_options_preflight_is_204_with_cors() {
    let (app, _token) = multimux::output::whep::serve_whep_for_test(None).await;
    let resp = app
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/whep")
                .header("origin", "http://player.example")
                .header("access-control-request-method", "POST")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    assert!(resp.headers().contains_key("access-control-allow-origin"));
}

/// CHARACTERISATION: RequestBodyLimitLayer rejects on Content-Length alone.
#[tokio::test]
async fn a_body_over_64_kib_is_rejected_413() {
    let (app, _token) = multimux::output::whep::serve_whep_for_test(None).await;
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/whep")
                .header("content-type", "application/sdp")
                .header("content-length", "131073")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
}
```

Helpers (`#[doc(hidden)]` on `output/whep.rs`): `serve_whep_for_test(verifier: Option<broadcast_auth::Verifier>) -> (axum::Router, CancellationToken)` builds `whep_router` over a `WhepServeState` with no live trunk (the 401/204/413 paths never reach it); `serve_whep_for_test_with_trunk(verifier)` publishes a real AVC-track `Trunk` first (reuse `tests/whep_egress.rs`'s existing trunk-building helper); `basic_verifier` is the same `Verifier` construction `tests/whep_egress.rs` already uses for its auth'd runs.

- [ ] **Step 2: Run pre-fix — FAIL.** `whep_post_without_credentials_is_401_with_challenge_and_cors` FAILS pre-fix: the pre-fix raw listener's `check_output_auth` builds the 401 with only `WWW-Authenticate` (`output/whep.rs:443-447`), no `Access-Control-Allow-Origin`. The other three are characterisation tests (labelled) pinning behaviour across the router move. Record which failed.

- [ ] **Step 3: Implement.** `run_whep` builds the router (`post(whep_post).options(…)` + `/whep/session` patch/delete), `.layer(from_fn_with_state(state, origin::output_auth_gate))` **inside** the tower layers so a 401 still carries CORS headers (exactly `origin::router`'s ordering), then `serve_hyper_util`. `whep_post` drives `WhepSession::on_post` → `Event::SdpOffer` → `parse_whep_offer` → bind socket → `MediaTransport` → `session.accept(answer, etag)`. `handle_whep_connection`/`check_output_auth` are deleted.

- [ ] **Step 4: Run — PASS** (`whep_http`, `whep_egress` real-browser).

- [ ] **Step 5: Commit**

```bash
git add multimux && git commit -m "refactor(multimux): WHEP on axum with the shared origin output-auth middleware"
```

---

### Task 6: Tracked accept pumps and cancel-aware admission for RTMP + WHIP + WHEP (defects 2, 3)

**Files:**
- Modify `multimux/src/source/rtmp.rs` — `ensure_infra` (289–330: the untracked `tokio::spawn` accept pump and the 20 ms `ACCEPT_POLL_INTERVAL`/`sleep` accept arm at 701) and `read_one` (594–609).
- Modify `multimux/src/source/whip.rs` — `ensure_infra`'s spawn → `TaskTracker`, `acquire_owned` raced against cancel (Task 4 introduced the router; this task tracks it).
- Modify `multimux/src/output/whep.rs` — the accept `acquire_owned` (1052) raced against cancel; the session `VecDeque<JoinHandle>` → `TaskTracker`.
- New test `multimux/tests/accept_lifecycle.rs`.

**Interfaces:**
- Produces: `RtmpInfra`/`WhipInfra`/`WhepRun` hold `(TaskTracker, CancellationToken)`; `Drop` cancels + closes; the RTMP accept arm moves off the fixed 20 ms sleep onto a `tokio::sync::Notify` (fired after each `tx.send`).
- Consumes: `tokio_util::task::TaskTracker`, `tokio_util::sync::CancellationToken`, `tokio::sync::Notify`.

- [ ] **Step 1: Write the failing tests**

`multimux/tests/accept_lifecycle.rs` (real saturation, no `sleep(20)` loops — use a bounded `timeout` on the *condition*):

```rust
//! Defects 2 and 3: the RTMP/WHIP/WHEP accept pumps are tracked tasks, a
//! blocked accept permit is cancelled on shutdown, and accepts are admitted
//! under steady reads (no 20 ms sleep-poll).

use std::time::Duration;
use tokio::net::TcpListener;

async fn wait_for_rebind(addr: std::net::SocketAddr, what: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match TcpListener::bind(addr).await {
            Ok(_) => return,
            Err(_) if tokio::time::Instant::now() < deadline => tokio::task::yield_now().await,
            Err(e) => panic!("{what} port {addr} stayed bound after drop/cancel: {e}"),
        }
    }
}

#[tokio::test]
async fn dropping_a_whip_listener_releases_its_port() {
    for _ in 0..20 {
        let (addr, route, token) = multimux::source::whip::serve_for_test().await;
        drop(route);
        token.cancel();
        wait_for_rebind(addr, "whip").await;
    }
}

#[tokio::test]
async fn cancelling_a_saturated_whep_accept_returns_and_releases_the_port() {
    let (addr, token) = multimux::output::whep::serve_whep_for_test_saturated_accept().await;
    token.cancel();
    wait_for_rebind(addr, "whep").await;
}

#[tokio::test]
async fn an_rtmp_accept_is_admitted_while_a_read_is_continuously_ready() {
    // Defect 2 (RTMP): the pre-fix accept arm is a 20 ms sleep-poll that a
    // steady read load starves. Publisher A connects and we keep its
    // connection continuously readable (byte-at-a-time); publisher B must
    // still be admitted well inside the 20 ms × N window the old code needs.
    let (server_addr, route, token) = multimux::source::rtmp::serve_for_test_with_read_load().await;

    // Publisher A: hold a connection whose read side is always ready.
    let mut a = tokio::net::TcpStream::connect(server_addr).await.unwrap();
    a.write_all(&[0x03]).await.unwrap(); // RTMP C0, never the rest of the handshake
    // A steady trickle keeps the source's read future always-ready.
    let drivel = tokio::spawn(async move {
        let mut i = 0u8;
        loop {
            let _ = a.write_all(&[i]).await;
            i = i.wrapping_add(1);
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    });

    // Publisher B must be admitted (its C0/C1 read by the accept pump)
    // within a bound far tighter than the sleep-starved old loop.
    let mut b = tokio::net::TcpStream::connect(server_addr).await.unwrap();
    b.write_all(&[0x03]).await.unwrap();
    let admitted = route
        .wait_for_sessions(2, Duration::from_millis(500))
        .await
        .expect("the second publisher must be admitted under a steady read");
    assert_eq!(admitted, 2, "two connections must be accepted");
    drivel.abort();
    token.cancel();
}
```

The `drivel` task's `tokio::time::sleep(1 ms)` is a load GENERATOR (it paces the trickle the test needs), not a wait-on-a-condition — W2b-1 Task 10's `tests/harness_guard.rs` allowlists this exact line in this exact test with that reason.

`serve_for_test_with_read_load` is a `#[doc(hidden)]` helper on `RtmpRoute` binding `127.0.0.1:0`, spawning `run_rtmp`, plus `RtmpRoute::wait_for_sessions(n, bound)` polling the `ListenDriver` session count via a `Notify`-driven loop with a bounded deadline (real condition, no fixed sleep). Pre-fix, the sleep-poll accept arm means publisher B is not read within the 500 ms bound whenever A's trickle wins every `select!` wake — the test fails; with the `Notify` admission arm it passes deterministically.

```rust
// helper sketch (in source/rtmp.rs, #[doc(hidden)]):
pub async fn serve_for_test_with_read_load() -> (std::net::SocketAddr, std::sync::Arc<RtmpRouteShared>, CancellationToken);
impl RtmpRouteShared {
    /// Polls the live session count until it reaches `n` (or the bound).
    /// The `Notified` future is CREATED BEFORE the count check (event_listener
    /// idiom): creating it after the check can miss a notification that fires
    /// between the check and the `await` — a lost wakeup that would wait out
    /// the whole bound.
    pub async fn wait_for_sessions(&self, n: usize, bound: Duration) -> std::io::Result<usize> {
        let deadline = tokio::time::Instant::now() + bound;
        loop {
            let notified = self.admit_notify().notified();   // register FIRST
            let count = self.session_count();
            if count >= n { return Ok(count); }
            if tokio::time::Instant::now() >= deadline {
                return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, format!("only {count}/{n} sessions admitted")));
            }
            notified.await;                                    // then wait
        }
    }
}
```

`serve_whep_for_test_saturated_accept` holds `MAX_PENDING_HTTP_CONNECTIONS` accept permits open so the accept loop is genuinely blocked on `acquire_owned` (the only scenario the old blocking `acquire_owned().await` demonstrably fails to cancel). Implement it as a `#[doc(hidden)]` helper with a deterministic semaphore — not a flaky flood.

- [ ] **Step 2: Run pre-fix — expect FAIL**

`cancelling_a_saturated_whep_accept…` FAILS pre-fix (the accept blocks on `acquire_owned`, the token is never consulted; the port stays bound → `wait_for_rebind` panics). `dropping_a_whip_listener…` may pass pre-fix (the untracked task dies with the runtime) — the biting assertion is the saturated-cancel one, not the plain drop. Record both outcomes.

- [ ] **Step 3: Implement**

RTMP `ensure_infra`: wrap the accept pump in `let tracked = TaskTracker::new(); tracked.spawn(async move { … server.accept() … })`, store `(tracked, cancel)` on `RtmpInfra`, `Drop` cancels+closes. Replace the `run_rtmp_with_clock` accept arm's `tokio::time::sleep(ACCEPT_POLL_INTERVAL)` with a `Notify`-driven arm (`accept_notify.notified()` → `driver.poll_accept()` drain), notified inside `handle_whip_connection`/the rtmp admit path after each successful `tx.send`. WHIP/WHEP: race `acquire_owned` against `cancel` (WHEP line 1052) and put the serve task on a `TaskTracker`.

- [ ] **Step 4: Run — PASS** (`accept_lifecycle`, `rtsp_ingest`-adjacent rtmp tests, `whip_ingest`, `whep_egress`).

- [ ] **Step 5: Revert-check**

Restore the untracked `tokio::spawn` accept pump in `rtmp.rs` and the blocking `acquire_owned().await` in `whep.rs`; the saturated-cancel test FAILS (port bound). Restore. Record in `.delegate/w2a-report.md`.

- [ ] **Step 6: Commit**

```bash
git add multimux && git commit -m "fix(multimux): track the RTMP/WHIP/WHEP accept pumps; admit accepts without a sleep-poll; cancel a blocked accept (defects 2, 3)"
```

---

### Task 7: WHIP/WHEP/RTMP sessions own their `MediaTransport`/connection and honour `poll_timeout` (defect 1, SP6.3)

**Files:**
- Modify `multimux/src/source/whip.rs` — `read_one` (1064–1131) and the `run_whip_with_clock` loop (1216–1284).
- Modify `multimux/src/output/whep.rs` — `run_whep_session_with_silence_timeout` (889–1010), `send_sample` (765–836).
- Modify `multimux/src/source/rtmp.rs` — `read_one` uses `RtmpConnection` (already owned, not `Arc<TokioMutex>`); align the timer arm.
- New test `multimux/tests/whip_whep_timers.rs`.

**Interfaces:**
- Produces: the session drive loop holds the `MediaTransport` **by value** in its own task (moved out of the `Arc<TokioMutex<_>>`), and locks it only around the synchronous `handle_datagram`/`encrypt_rtp`/`poll_transmit` drain — **no `Arc<TokioMutex<MediaTransport>>` is held across `send_to().await`** (SP6.3). `read_one` takes `deadline: Option<Instant>` from `poll_timeout()`.
- Consumes: `webrtc_runtime::media::MediaTransport::{poll_timeout, handle_timeout, handle_datagram, poll_transmit, encrypt_rtp}`.

- [ ] **Step 1: Write the failing tests**

`multimux/tests/whip_whep_timers.rs` — real assertions, both biting pre-fix:

```rust
use std::time::Duration;

/// Defect 1: with a quiet media socket, `MediaTransport::handle_timeout`
/// must fire at `poll_timeout` (ICE/DTLS retransmits) WITHOUT the session
/// being reaped. Pre-fix, `handle_timeout` runs only on the read-timeout
/// arm, which also ends the session — so within this window the timer
/// counter never moves and the test fails.
#[tokio::test(start_paused = true)]
async fn whip_handle_timeout_runs_on_the_transport_deadline_and_keeps_the_session() {
    let (addr, route, _token) =
        multimux::source::whip::serve_for_test_with_read_timeout(Duration::from_secs(30)).await;

    // Complete the signalling exchange for real: POST the fixed OFFER and
    // read the 201 (same body as tests/whip_http.rs::post_offer).
    let resp = post_offer(addr).await;
    assert!(resp.starts_with("HTTP/1.1 201"), "signalling must succeed: {resp}");

    // One admitted session, media socket quiet.
    assert_eq!(route.active_sessions(), 1);

    // Advance virtual time past the transport's own deadline (ICE
    // retransmit timers are sub-second; 2 s covers them).
    let before = route.timer_fires();
    tokio::time::advance(Duration::from_secs(2)).await;
    let after = tokio::time::timeout(Duration::from_secs(1), route.wait_timer_fire(before + 1)).await;
    assert!(
        after.is_ok(),
        "handle_timeout must fire at poll_timeout while the session is live"
    );

    // The timer wake is NOT a session end: the session is still admitted.
    assert_eq!(route.active_sessions(), 1, "the timer arm must not reap the session");
}

/// Defect 2 (WHIP): a second publisher must be admitted while the first
/// session's read is continuously ready (the pre-fix 20 ms sleep-poll accept
/// arm is starved by steady reads).
#[tokio::test]
async fn a_second_whip_publisher_is_admitted_while_a_first_reads_continuously() {
    let (addr, route, token) = multimux::source::whip::serve_for_test().await;

    // Publisher A: a session whose socket receives a steady datagram
    // trickle, so its read future is always ready.
    let (_a_addr, a_sessions) = publish_and_stream(addr).await;
    assert_eq!(a_sessions, 1);

    // Publisher B: POSTs the same OFFER and must be answered 201 well
    // inside the window the pre-fix sleep-poll starves.
    let resp = post_offer(addr).await;
    assert!(resp.starts_with("HTTP/1.1 201"), "the second publisher must be admitted: {resp}");
    assert_eq!(route.active_sessions(), 2, "both publishers admitted");
    token.cancel();
}
```

The helpers are `#[doc(hidden)]` items on `WhipRoute` added by this task:
- `serve_for_test_with_read_timeout(read: Duration) -> (SocketAddr, Arc<WhipRouteShared>, CancellationToken)` — binds `127.0.0.1:0`, builds a `WhipRoute` with `IngestTimeouts { read, ..Default::default() }`, spawns `run_whip`.
- `WhipRouteShared::active_sessions() -> usize` — reads the existing `active_sessions` atomic.
- `WhipRouteShared` holds `timer_fires: Arc<AtomicU64>` and `timer_notify: Arc<tokio::sync::Notify>` (per-route, not a global — a global leaks across tests). The timer arm bumps the counter and `notify_waiters()`; `WhipRouteShared::timer_fires() -> u64` reads it and `wait_timer_fire(n: u64)` awaits the Notify (registering the `Notified` BEFORE re-checking the counter — the same lost-wakeup idiom as `wait_for_sessions`) until the counter reaches `n` or the caller's bound elapses. This counter is the honest observable: pre-fix the timer arm does not exist, so `wait_timer_fire` times out and the test FAILS.
- `publish_and_stream(addr)` — opens one WHIP session via `post_offer` and sends a small STUN-looking datagram to that session's media socket every millisecond (a spawned task the test aborts at the end), returning the session count observed via `active_sessions`.

The structural half of SP6.3 (no `Arc<TokioMutex<MediaTransport>>`) is enforced by the Task 9 guard needle, not by a runtime test — it cannot be asserted at runtime.

- [ ] **Step 2: Run pre-fix — FAIL**

`whip_handle_timeout_runs_on_the_transport_deadline_and_keeps_the_session` FAILS: `wait_timer_fire(before + 1)` times out — pre-fix `handle_timeout` never runs while the session is live (only on the 30 s read-timeout arm, which this window never reaches). `a_second_whip_publisher…` FAILS pre-fix whenever the trickle starves the 20 ms accept sleep. Record both.

- [ ] **Step 3: Implement — the session task OWNS its `MediaTransport`; `read_one` hands it back**

The boxed future now returns the transport it borrowed, so `FuturesUnordered` can hold per-read futures while the session keeps ownership between reads (SP6.3):

```rust
/// What one [`read_one`] call observed, plus the transport handed back —
/// the session owns it between reads (SP6.3: no Arc<Mutex<…>> across awaits).
struct ReadResult {
    id: SessionId,
    outcome: ReadOutcome,
    media: MediaTransport,
}

type BoxedRead = Pin<Box<dyn Future<Output = ReadResult> + Send>>;

fn read_one(
    id: SessionId,
    socket: Arc<UdpSocket>,
    mut media: MediaTransport,          // moved in; handed back in ReadResult
    read_timeout: Duration,
) -> BoxedRead {
    Box::pin(async move {
        let mut buf = [0u8; MAX_UDP_DATAGRAM];
        // The transport's own next timer, taken BEFORE the select so a quiet
        // session still gets its ICE/DTLS retransmits (defect 1).
        let deadline = media.poll_timeout();
        let outcome = tokio::select! {
            received = socket.recv_from(&mut buf) => match received {
                Ok((n, peer)) => match media.handle_datagram(Instant::now(), peer, &buf[..n]) {
                    Ok(events) => {
                        flush(&mut media, &socket).await;
                        ReadOutcome::Events(to_wire(events))
                    }
                    Err(e) => {
                        flush(&mut media, &socket).await;
                        tracing::debug!(error = %e, %peer, "whip: datagram rejected, skipping");
                        ReadOutcome::Events(Vec::new())
                    }
                },
                Err(e) => ReadOutcome::TransportError(e.to_string()),
            },
            () = sleep_until_opt(deadline) => {
                media.handle_timeout(Instant::now());
                // Per-route counter (NOT a global: a global would leak
                // across tests). The route's `WhipRouteShared` owns it; the
                // run loop shares the Arc.
                shared.timer_fires.fetch_add(1, Ordering::Relaxed);
                shared.timer_notify.notify_waiters();
                flush(&mut media, &socket).await;
                ReadOutcome::Events(Vec::new())   // a timer wake is not a session end
            }
            () = tokio::time::sleep(read_timeout) => {
                media.handle_timeout(Instant::now());
                flush(&mut media, &socket).await;
                ReadOutcome::TimedOut
            }
        };
        ReadResult { id, outcome, media }
    })
}

/// Drain everything the transport wants to send. `&mut MediaTransport` is
/// held across NO `send_to().await` guard — this IS the owner (SP6.3).
async fn flush(media: &mut MediaTransport, socket: &UdpSocket) {
    while let Some(Datagram { peer, bytes }) = media.poll_transmit() {
        let _ = socket.send_to(&bytes, peer).await;
    }
}

/// `sleep_until` for an optional deadline; `None` = never fires.
fn sleep_until_opt(deadline: Option<Instant>) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    match deadline {
        Some(d) => Box::pin(tokio::time::sleep_until(d)),
        None => Box::pin(std::future::pending()),
    }
}
```

The `run_whip_with_clock` loop changes accordingly: the admit arm now moves the session's owned `MediaTransport` into `read_one` (the session no longer stores `media: Arc<TokioMutex<MediaTransport>>`; `WhipIngestSession` holds it as an `Option<MediaTransport>` the loop takes and gives back):

```rust
AcceptOutcome::Admitted(id) => {
    let session = driver.driver(id).expect("just admitted").session();
    let socket = session.socket_handle();
    let media = session.take_media().expect("a freshly admitted session owns its transport");
    progress.insert(id, DriverProgress::new());
    clocks.admit(id, clock());
    reads.push(read_one(id, socket, media, read_timeout));
}
// …in the reads.next() arm:
Some(result) = reads.next(), if !reads.is_empty() => {
    let ReadResult { id, outcome, media } = result;
    // …feed/reap exactly as before…
    if !reaped {
        // Hand the transport back into the next read.
        reads.push(read_one(id, socket, media, read_timeout));
    }
}
```

`WhipIngestSession`'s `media_handle()` (whip.rs:832) becomes `take_media() -> Option<MediaTransport>` (owned out, `None` while a read holds it), and the `Arc<TokioMutex<MediaTransport>>` field is deleted. `output/whep.rs` gets the same restructure: `AdmittedWhep` owns the `MediaTransport` by value (`run_whep_session` already is one task per session — exactly the SP6.3 shape), `send_sample` takes `&mut MediaTransport` (owned by the session task), and no lock exists to hold across `send_to().await`:

```rust
async fn send_sample(
    socket: &UdpSocket,
    media: &mut MediaTransport,        // owned by the session's own task
    peer: SocketAddr,
    next_seq: &mut u16,
    session: &SessionMedia<'_>,
    sample: &Sample,
) {
    // …build packets as today…
    for pkt in &packets {
        match media.encrypt_rtp(&wire) {
            Ok(protected) => {
                let _ = socket.send_to(&protected, peer).await;  // no lock held
            }
            Err(e) => tracing::warn!(error = %e, "whep: encrypt_rtp failed"),
        }
    }
}
```

- [ ] **Step 4: Run — PASS** (`whip_whep_timers`, `whip_ingest`, `whep_egress`).

- [ ] **Step 5: Revert-check** — delete the `sleep_until_opt(deadline)` arm so `handle_timeout` runs only on the read-timeout arm, and revert `send_sample` to the `Arc<TokioMutex>` form; `whip_handle_timeout_runs_on_the_transport_deadline…` FAILS (`wait_timer_fire` times out — the counter never moves while live). Restore. Record.

- [ ] **Step 6: Commit**

```bash
git add multimux && git commit -m "refactor(multimux)!: WHIP/WHEP session tasks own their MediaTransport; drive timers off poll_timeout (defect 1, SP6.3)"
```

---

### Task 8: WHIP/WHEP SDP through `sdp-types` 0.2's typed `Session`; ICE credentials media-level with session fallback; candidates from `local_candidates()`

**Files:**
- Modify `multimux/src/source/whip.rs` — `parse_whip_offer` (360–458), `build_answer` (475–512), `sdp_attr_anywhere` (465–469).
- Modify `multimux/src/output/whep.rs` — `parse_whep_offer` (224–294), `build_whep_answer` (325–384), `sdp_attr_anywhere` (298–302).
- Modify `multimux/src/source/sdp.rs` — the 0.1→0.2 `get_first_attribute_value`/`get_attribute_values` break (lines 32–37 `.ok().flatten()` → drop the `.ok()`); verify `config.rs:1155` (only `Session::parse`, unchanged — no edit needed, but assert it builds).
- Modify `multimux/Cargo.toml` — `sdp-types = "0.2"` only. (No `rtc-ice` dep: the candidate lines are `MediaTransport::local_candidates()` strings, and webrtc-runtime already does any marshal.)
- New test `multimux/tests/whip_whep_sdp.rs`.

**Interfaces:**
- Produces: `ParsedOffer`/`ParsedWhepOffer` populated from a typed `Session`; ICE ufrag/pwd read at **media level first, session level fallback**; `sdp_attr_anywhere` deleted (both copies). `ParsedOffer` DROPS the verbatim `m_line: String` field (the answer re-renders the m= line from the typed media, so nothing copies offer text) and `codec_lines` becomes typed `(name, Option<value>)` pairs.
- Consumes: `sdp_types::Session::{parse, write, builder}`, `sdp_types::{Media, Attribute, Origin, Time}`, `sdp_types::Media::{get_first_attribute_value, get_attribute_values}` (both return `Option<Option<&str>>` / `impl Iterator<Item = Option<&str>>` — no `.ok()`), `webrtc_runtime::media::{MediaTransport::local_candidates, parse_remote_fingerprint}`.

- [ ] **Step 1: Capture the answer golden before the change**

Golden `multimux/tests/golden/whip_answer.sdp` / `whep_answer.sdp` from the current `build_answer` on the fixed `OFFER`, using the `GOLDEN_BLESS` pattern in `output/smooth.rs:972`. Add a README entry.

```bash
GOLDEN_BLESS=multimux/tests/golden cargo test -p multimux --all-features --locked --features whip,whep --lib source::whip::tests::whip_answer_golden output::whep::tests::whep_answer_golden 2>&1 | grep -E 'test result|FAILED'
git add multimux/tests/golden multimux/tests/golden/README.md && git commit -m "test(multimux): golden the WHIP/WHEP answers before the sdp-types migration"
```

- [ ] **Step 2: Write the typed-parse test (one biting, two characterisation)**

`multimux/tests/whip_whep_sdp.rs` (real code, `OFFER` the full literal from `whip_http.rs`, not a reference):

```rust
use multimux::source::whip::ParsedOffer;   // made pub(crate)-visible via a
                                           // #[doc(hidden)] parse_for_test wrapper this task adds

pub const OFFER: &str = "v=0\r\n\
o=- 0 0 IN IP4 127.0.0.1\r\n\
s=-\r\n\
t=0 0\r\n\
m=video 9 UDP/TLS/RTP/SAVPF 96\r\n\
c=IN IP4 0.0.0.0\r\n\
a=ice-ufrag:abcd\r\n\
a=ice-pwd:abcdefghijklmnopqrstuvwx\r\n\
a=fingerprint:sha-256 00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff:\
00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff\r\n\
a=setup:actpass\r\n\
a=mid:0\r\n\
a=rtcp-mux\r\n\
a=rtpmap:96 H264/90000\r\n\
a=fmtp:96 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f\r\n\
a=candidate:1 1 udp 2130706431 10.0.0.5 54321 typ host\r\n";

/// CHARACTERISATION (passes on old code too): a media-level offer parses.
#[test]
fn a_media_level_offer_parses_ice_credentials_and_candidates() {
    let parsed = ParsedOffer::parse_for_test(OFFER).unwrap();
    assert_eq!(parsed.remote_ufrag, "abcd");
    assert_eq!(parsed.remote_pwd, "abcdefghijklmnopqrstuvwx");
    assert_eq!(parsed.candidates.len(), 1);
    assert!(parsed.candidates[0].starts_with("1 1 udp"));
    assert_eq!(parsed.mid, "0");
}

/// CHARACTERISATION (passes on old code too — the old flat scan is
/// level-blind): session-level ICE credentials still resolve, via the
/// media->session fallback the typed API needs explicitly.
#[test]
fn a_session_level_offer_still_parses_ice_credentials() {
    // OFFER with the media-level `a=ice-ufrag:`/`a=ice-pwd:` lines removed
    // and one session-level pair inserted before `t=0 0` (RFC 8839 §5.4).
    let mut session_level = String::with_capacity(OFFER.len());
    for line in OFFER.lines() {
        if line.starts_with("a=ice-ufrag:") || line.starts_with("a=ice-pwd:") {
            continue; // removed from media level
        }
        if line.starts_with("t=0 0") {
            session_level.push_str("a=ice-ufrag:abcd\r\na=ice-pwd:abcdefghijklmnopqrstuvwx\r\n");
        }
        session_level.push_str(line);
        session_level.push_str("\r\n");
    }
    let parsed = ParsedOffer::parse_for_test(&session_level).unwrap();
    assert_eq!(parsed.remote_ufrag, "abcd");
    assert_eq!(parsed.remote_pwd, "abcdefghijklmnopqrstuvwx");
}

/// BITING (fails when ICE is read at session level only, or when the answer
/// is not built through sdp-types): the typed parser must take its
/// credentials from the media section, and the built answer must be the
/// `Session::write` rendering pinned to the golden.
#[test]
fn a_media_section_with_its_own_ice_credentials_wins_over_a_stale_session_level_pair() {
    // The session level carries a DECOY pair; the media section carries the
    // real one. The typed parser reads media-first, so the real pair wins.
    // The old flat scan also takes the first match — which is the session-
    // level DECOY — so this test FAILS on the old code.
    let mut offer = String::with_capacity(OFFER.len());
    for line in OFFER.lines() {
        if line.starts_with("t=0 0") {
            offer.push_str("a=ice-ufrag:stale\r\na=ice-pwd:stalestalestalestalestale\r\n");
        }
        offer.push_str(line);
        offer.push_str("\r\n");
    }
    let parsed = ParsedOffer::parse_for_test(&offer).unwrap();
    assert_eq!(parsed.remote_ufrag, "abcd", "media-level credentials must win");
    assert_eq!(parsed.remote_pwd, "abcdefghijklmnopqrstuvwx", "media-level credentials must win");
}

#[test]
fn the_built_answer_matches_the_golden() {
    let golden = include_str!("golden/whip_answer.sdp");
    let built = ParsedOffer::build_answer_for_test(OFFER);
    assert_eq!(
        built, golden,
        "answer differs: list each diff in the CHANGELOG and re-bless the golden in the same commit"
    );
}
```

- [ ] **Step 3: Run pre-fix — FAIL (biting test identified)**

Pre-fix (the flat `sdp_attr_anywhere` scan + `format!`-built answer), `a_media_section_with_its_own_ice_credentials_wins_over_a_stale_session_level_pair` FAILS: the old scan takes the FIRST `a=ice-ufrag:` line anywhere, which is the session-level decoy (`stale`), not the media-level real pair (`abcd`) — the new parser reads media-first. The two characterisation tests pass pre-fix (labelled as such); the golden test passes pre-fix and is the migration's diff detector (it fails the moment `Session::write` changes a byte, forcing the diff listing + re-bless). The sdp-types 0.2 bump itself breaks `source/sdp.rs`'s `.ok().flatten()` at compile time — fixed in Step 6. Record the decoy-test failure as the behavioural pre-fix evidence.

- [ ] **Step 4: Rewrite the parsers on the typed API, with the media→session fallback**

```rust
fn ice_attr<'a>(session: &'a sdp_types::Session, media: &'a sdp_types::Media, name: &'a str) -> Option<&'a str> {
    media.get_first_attribute_value(name).flatten()
        .or_else(|| session.get_first_attribute_value(name).flatten())
}

fn parse_whip_offer(offer: &str) -> Result<ParsedOffer> {
    let session = sdp_types::Session::parse(offer.as_bytes())
        .map_err(|e| MultimuxError::Sdp { reason: format!("whip: parse offer: {e}") })?;
    let video: Vec<&sdp_types::Media> = session.medias.iter().filter(|m| m.media == "video").collect();
    if session.medias.len() != video.len() || video.len() != 1 {
        return Err(MultimuxError::Sdp { reason: "…exactly one m=video…".into() });
    }
    let media = video[0];
    let remote_ufrag = ice_attr(&session, media, "ice-ufrag")
        .ok_or_else(|| MultimuxError::Sdp { reason: "whip: offer has no a=ice-ufrag".into() })?
        .to_string();
    let remote_pwd = ice_attr(&session, media, "ice-pwd")
        .ok_or_else(|| MultimuxError::Sdp { reason: "whip: offer has no a=ice-pwd".into() })?
        .to_string();
    let remote_fingerprint = parse_remote_fingerprint(offer)
        .ok_or_else(|| MultimuxError::Sdp { reason: "whip: offer has no a=fingerprint".into() })?;
    let mid = media.get_first_attribute_value("mid").flatten().unwrap_or("0").to_string();
    let candidates: Vec<String> = media
        .get_attribute_values("candidate")
        .flatten()
        .map(str::to_string)
        .collect();
    // H.264 payload type: the first `m=video` fmt whose typed rtpmap names
    // H264 — mirroring the old scan's semantics (skips a paired rtx).
    let mut chosen: Option<(u8, u32)> = None;
    for tok in media.fmt.split_whitespace() {
        let Ok(pt) = tok.parse::<u8>() else { continue };
        // The rtpmap attributes naming THIS pt: value is "<pt> <enc>/<clock>".
        let names_pt = media.get_attribute_values("rtpmap").flatten().any(|v| {
            v.starts_with(&format!("{pt} ")) && v.to_ascii_uppercase().contains("H264")
        });
        if !names_pt {
            continue;
        }
        let clock_rate = media
            .get_first_attribute_value("rtpmap")
            .flatten()
            .and_then(|v| transmux::rtpmap_clock_rate(v))
            .unwrap_or(90_000);
        chosen = Some((pt, clock_rate));
        break;
    }
    let (payload_type, clock_rate) = chosen.ok_or_else(|| MultimuxError::Sdp {
        reason: "whip: m=video has no H.264 (rtpmap naming H264) payload type".into(),
    })?;

    // The codec lines echoed into the answer: this media section's
    // rtpmap/fmtp/rtcp-fb attributes naming `payload_type`, carried as typed
    // (name, value) pairs (the answer re-renders them through its own
    // `Session::write`, so nothing copies offer text verbatim).
    let codec_lines: Vec<(String, Option<String>)> = media
        .attributes
        .iter()
        .filter(|a| matches!(a.attribute.as_str(), "rtpmap" | "fmtp" | "rtcp-fb"))
        .filter(|a| {
            a.value
                .as_deref()
                .and_then(|v| v.split_whitespace().next())
                .and_then(|pt| pt.parse::<u8>().ok())
                == Some(payload_type)
        })
        .map(|a| (a.attribute.clone(), a.value.clone()))
        .collect();

    Ok(ParsedOffer {
        remote_ufrag,
        remote_pwd,
        remote_fingerprint,
        mid,
        candidates,
        payload_type,
        clock_rate,
        codec_lines,
    })
}
```

- [ ] **Step 5: `build_answer` via `Session::write`, candidate lines from `local_candidates()`**

`build_answer` builds a `sdp_types::Session` and `Session::write(&mut Vec<u8>)`. The candidate lines are the strings `MediaTransport::local_candidates()` (transport.rs:749) already returns — decision: no `rtc-ice` dependency is added, because the marshalling already happened when the transport gathered them. Each string becomes `media.add_attribute_with_value("candidate", line)`. If a marshalled string already carries the `candidate:` prefix (compare against the golden's `a=candidate:0 1 udp …` line), strip it in the answer builder — the golden pins the exact wire form. The `a=fingerprint`/`a=setup`/`a=recvonly`/`a=rtcp-mux`/`a=end-of-candidates` are typed attributes.

- [ ] **Step 6: Migrate `source/sdp.rs` for the sdp-types 0.2 break**

```rust
let fmtp = media.get_first_attribute_value("fmtp").flatten();
let rtpmap = media.get_first_attribute_value("rtpmap").flatten();
let control = media.get_first_attribute_value("control").flatten().map(str::to_string);
```
(no `.ok()`; the 0.2 accessors return `Option<Option<&str>>`). `config.rs:1155` needs no edit (only `Session::parse`).

- [ ] **Step 7: Run — PASS (goldens byte-identical or each diff listed), interop green**

```bash
timeout 120 cargo test -p multimux --all-features --locked --features whip,whep --test whip_whep_sdp 2>&1 | grep -E 'test result|FAILED'
git diff --exit-code multimux/tests/golden/ && echo "answer goldens byte-identical"
timeout 1800 cargo test -p multimux --all-features --locked --features whip,whep --test whip_ingest --test whep_egress -- --nocapture 2>&1 | grep -E 'test result|FAILED|real result'
```

If the answer differs from the golden, list every difference in the CHANGELOG (breaking) with an example, re-bless the golden in the same commit, and confirm the browser interop suites stay green.

- [ ] **Step 8: Revert-check** — restore the flat `.lines()` `sdp_attr_anywhere` scan for ICE (drop the media-first `ice_attr`); `a_media_section_with_its_own_ice_credentials_wins_over_a_stale_session_level_pair` FAILS (the flat scan returns the session-level decoy `stale`). Restore.

- [ ] **Step 9: Commit**

```bash
git add multimux && git commit -m "refactor(multimux)!: WHIP/WHEP SDP via sdp-types 0.2 (media-level ICE with session fallback); adapt source/sdp.rs"
```

---

### Task 9: The §5 guard for multimux (last code task)

**Files:**
- New `multimux/tests/no_handroll_guard.rs`.

**Interfaces:** a lexical tripwire scanning `multimux/src/**/*.rs` outside `#[cfg(test)] mod` bodies.

- [ ] **Step 1: Copy the scanner + needles**

Copy `hls-runtime/tests/no_handroll_guard.rs` verbatim. Needles:

```rust
const NEEDLES: &[&str] = &[
    "\"HTTP/1.", "\r\n\r\n", "find(\"://\")",
    "strip_prefix(\"srt://\")", "strip_prefix(\"rtsp://\")", "strip_prefix(\"rtmp://\")",
    "\"a=", "\"m=", "\"v=0",
    "civil_from_days", "days_from_civil",
    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/",
    "thread::sleep",
    "Arc<TokioMutex<MediaTransport>>",   // SP6.3 tripwire
    "Arc<tokio::sync::Mutex<MediaTransport>>",
];
```

The allowlist (each with a reason) covers: `output/dash.rs`/`source/dash_pull.rs`'s `civil_from_days`/`days_from_civil` and the `format!("PT…")` duration builds (cleared by **W2b-1 Task 3** — note "W2b-1 Task 3 removes this"); `push/*`/`redact.rs`/`source/rtsp.rs`'s `find("://")`/`strip_prefix`/`format!("rtmp://…")` (cleared by **W2b-1 Task 2** — note "W2b-1 Task 2 removes this"); `origin/supervisor.rs`/`push/mod.rs`/`source/file_reader.rs`'s `tokio::time::sleep(` backoff/pacing sites (cleared by **W2b-1 Task 9** and **W2b-2 Task 1** — note both). W2a itself leaves **zero** un-allowlisted hits: the W2a-cleared surfaces (webrtc_http deleted, whip/whep on axum, limit classify typed) must be clean. The guard scans `src/` only by design; the test harness (`tests/`) sleeps and reserve-then-rebind loops are guarded by W2b-1 Task 10's `tests/harness_guard.rs` — cross-reference it in this file's module doc.

- [ ] **Step 2: Run — fix or allowlist every hit, then a `src/`-only scan must be green**

```bash
cargo test -p multimux --all-features --locked --test no_handroll_guard 2>&1 | grep -E 'test result|FAILED|src/'
```

- [ ] **Step 3: Bite-check** — append `let _ = "v=0\r\n";` to a non-test fn in `multimux/src/lib.rs`; the guard FAILS naming `src/lib.rs`. Remove it.

- [ ] **Step 4: Commit**

```bash
git add multimux && git commit -m "test(multimux): no-hand-roll tripwire guard (W2b sites allowlisted with removal notes)"
```

---

### Task 10: CHANGELOG, version notes, full gate, hand-off

**Files:** `multimux/CHANGELOG.md`, `hls-runtime/CHANGELOG.md`, `.delegate/w2a-report.md`.

- [ ] **Step 1: CHANGELOG**

`multimux` under `## [Unreleased]`:
- `### Changed (breaking)`: axum 0.8 (path syntax `/{param}`), tower-http 0.7, reqwest 0.13; WHIP/WHEP served from axum (chunked bodies accepted; `webrtc_http` raw reader removed); origin budgets now shed (503) when the pool is full instead of queueing to a timeout; admin media router is an `ArcSwap`; SDP via sdp-types 0.2 (list any answer-byte diff).
- `### Fixed`: slow-loris header-read + total-connection deadline on every listener; WHIP/WHEP protocol timers fire while media flows (defect 1); accepts under steady reads (defect 2); tracked WHIP/WHEP/RTMP accept pumps, blocked accept cancelled (defect 3).
- `### Changed`: Cache-Control/CORS via `headers`/`CorsLayer` (list any header-byte diff).

`hls-runtime`: reqwest 0.12→0.13, `TokioError::Http::source` still `reqwest::Error` (breaking).

- [ ] **Step 2: Full gate**

```bash
/Volumes/External/Projects/rust-broadcast/.delegate/gate-wt.sh "$PWD" > /Volumes/External/Projects/rust-broadcast/.delegate/gate-w2a.log 2>&1
grep -c '^rc=0' /Volumes/External/Projects/rust-broadcast/.delegate/gate-w2a.log
```

Expected: `14` rc=0 steps, no failure block.

- [ ] **Step 3: Write `.delegate/w2a-report.md`** (baseline/after counts, lock deltas, byte diffs, revert-checks, version notes: `multimux` breaking → 0.12.0, `hls-runtime` breaking → 0.8.0).

- [ ] **Step 4: Hand off — do not merge, push, or tag.**

---

## Coverage table (spec §3 + §4 owned by W2a)

| Spec site / item | Where | Task |
|---|---|---|
| §3 HTTP `webrtc_http.rs` (reader, slot cap) | `multimux/src/webrtc_http.rs` | 4 (deleted) |
| §3 HTTP `source/whip.rs`, `output/whep.rs` (accept loops, hand responses/CORS/401/409/201) | whip:249-695, whep:457-659 | 4, 5 |
| §3 HTTP `origin/mod.rs` (Cache-Control/CORS consts) | 435-477 | 3 |
| §3 HTTP `origin/limit.rs` (custom service, `_HLS_*` scan) | limit.rs | 3 |
| §3 HTTP `origin/admin.rs` (`RwLock<Router>` clone) | 236,289,296,465,667 | 3 |
| §3 SDP whip/whep (line scans, answer, candidates) | whip:360-512, whep:224-384 | 8 |
| §3 defect 1 (WHIP `handle_timeout` on read-timeout only) | whip:1118-1127 | 7 |
| §3 defect 2 (RTMP/WHIP accept starvation) | rtmp:178,701; whip:1218 | 6, 7 |
| §3 defect 3 (untracked accept pumps: rtmp, whip, whep) | rtmp:301; whip:276; whep:1041 | 6 |
| §4 SP2.1 (WHIP/WHEP on axum; delete reader; layers incl. CorsLayer) | | 4, 5 |
| §4 SP2.2 (webrtc typed http/headers, consumed) | | 4, 5, 8 |
| §4 SP2.5 (limit tower layers; arc-swap; Query `_HLS_*`; CacheControl) | | 3 |
| §4 SP2.7 (axum 0.8, tower-http 0.7, reqwest 0.13) | | 1 |
| §4 SP2.8 (header goldens, 413/431/timeout/chunked, interop green) | | 1, 2, 4, 5, 8 |
| §4 SP4.1/4.2/4.5 (sdp-types 0.2; candidates via `local_candidates()`; SDP goldens) | | 8 |
| §4 SP6.3 (session tasks own MediaTransport) | whip, whep | 7 |
| §5 guard | | 9 |
| §6 goldens from main; defect tests revert-checked | | 1, 2, 6, 7, 8 |
| §7 W2 HTTP stack (items 1-4) | | 1, 2, 3, 4, 5 |
| §7 W2 SP4 SDP (item 6) | | 8 |
| §7 W2 defects 1-3 (item 8, WHIP/WHEP/RTMP) | | 6, 7 |

Not in W2a (W2b): URL builders, dates/durations, backon, socket2, CancellationToken shutdown, pull scheduler, ingest driver, parking_lot, test-harness port-0 for the *other* test files, `wait-timeout`, `rtsp_ingest` adapter, defect 8 (IPv6), defect 4/5 (push/source), `multimux-cli`.

## Escalations

1. **`serve_hyper_util` exact generic bound.** Resolved without `axum::serve::IncomingStream` (its fields are private, axum 0.8.9 `serve/mod.rs:424`): the per-connection service is `axum::Extension(ConnectInfo(remote_addr)).layer(router)` — `Router<()>: Service<Request<B>, Error = Infallible>` (axum 0.8.9 `routing/mod.rs:569`) — wrapped by `TowerToHyperService` (hyper-util `service/glue.rs:26`). `TaskTracker` has no `join_next`; the drain is `close()` + `wait()` (tokio-util 0.7.19 `task_tracker.rs:318/337`). The admin media listener's `DynamicMediaService` is served through the `serve_hyper_util_service` factory overload.
2. **`limit.rs` budget split via tower layers.** Adopted the classify-service wrapper (the review's escalation-2 fallback) as the primary design; the queue-vs-shed semantic change (pool-full now sheds immediately) is declared in the CHANGELOG.
3. **WHEP/WHIP `acquire_owned` test determinism.** Uses a `serve_whep_for_test_saturated_accept` helper that holds `MAX_PENDING_HTTP_CONNECTIONS` permits open, not a flood.
4. **`Session::write` line order.** Golden compared; every diff listed (semantic equality only with each diff named) + browser interop green.
