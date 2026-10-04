# De-hand-roll W2b-multimux-runtime — URLs, time, test harness, shutdown/task ownership Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove the hand-rolled URL builders, timestamps, durations and fixed-sleep/poll sites from multimux's non-driver paths; migrate the SP7 test harness (bind port 0, `wait-timeout` ×3, `rtsp_ingest` on the server adapter, no fixed sleeps); and replace the `watch<bool>` shutdown with one `CancellationToken` while giving every `tokio::spawn` a tracked owner (a `TaskTracker`/`JoinSet` with a disposition: cancelled how, joined where).

**Architecture:** One branch `w2/b` in worktree `.worktree/w2-b`, rebasing over W2a's merged result (W1-R-low-a is already on `main` at `3340fdaa`/`31ba01ec`, so the rtsp/rtmp adapters it added — `AsyncRtspServer`/`RtspTimeouts`/`AsyncRtspClient`/`AsyncRtmpClient`/`RtmpTarget` — are plain dependencies of this work). This is **W2b-1**; the runtime-structure half (backon reconnect, pull scheduler, ingest driver, socket2, parking_lot, RTSP/RTMP source+push migration) is **W2b-2** = `docs/superpowers/plans/2026-10-04-dehandroll-w2b-2-multimux-runtime.md`.

**Tech Stack:** `url` 2, `jiff` 0.2, `tokio-util` 0.7 (`CancellationToken`, `TaskTracker`), `tokio::task::JoinSet`, `wait-timeout` 0.2, `rtsp-runtime` 0.8 (W1-R-low-a), plus W2a's axum 0.8/tower-http 0.7 stack.

**Spec:** `docs/superpowers/specs/2026-10-03-protocol-runtime-dehandroll-design.md` (§3 URL/time rows, §4 SP1.4/SP3/SP5/SP7, §5, §6, §7 W2). W2a is `…-w2a-multimux-http.md`; W2b-2 is `…-w2b-2-multimux-runtime.md`.

## Dependency order

- **W2a merges FIRST** (W2b-1 rebases over it): the axum/tower/reqwest versions, `serve_hyper_util`, the axum WHIP/WHEP routers, `webrtc_session.rs`, and — critically — the listener-taking constructors moved into W2a (Tasks 2/4/5) are W2b-1's starting point.
- **W2b-1 Tasks 5/6 consume W1-R-low-a's adapters**, which ARE on `main` (`3340fdaa` merged R-low-a; `31ba01ec` followed): `AsyncRtspServer`/`RtspTimeouts` for the `rtsp_ingest` test server, and `AsyncRtspClient` as the base this wave extends with `announce`/`record`/`send_interleaved` (Task 6). No rebase-over-branch step is needed; the ordinary W2a-merge rebase suffices.
- **W2b-1 merges before W2b-2**: W2b-1 owns the shutdown/task-ownership model and the harness.

## Global Constraints

Copied verbatim from spec §2:

- MSRV 1.95.0; committed `Cargo.lock`, always `--locked`. A dependency add or bump may change only the intended lock entries; restore anything else with `cargo update -p <pkg> --precise <old>`.
- Every new or bumped crate supports MSRV 1.95 (all verified ≤1.89).
- No Co-Authored-By or Claude-Session trailers on commits.
- Nothing is tagged or published without the owner's explicit sign-off.
- Epoch purity: changed public-API dep types force a major-class bump; recorded in `.delegate/release-versions.txt`.

Owner decisions: SP3 option (a) synthetic `file://` base stripped from relative results; `host:port` via `SocketAddr`/`lookup_host`. SP5 dates/durations via `jiff` (chrono untouched). SP1.4 every spawn owned by a `JoinSet`/`TaskTracker`, shutdown via `CancellationToken` only (the `watch<bool>` is removed, breaking). SP7 bind port 0 + pass listener, no fixed sleeps, `bounded.rs` ×3 → `wait-timeout`, `rtsp_ingest` → server adapter, 20 consecutive runs.

## Review Focus

1. **A push reconnect ignores shutdown** (defect 5). Cancelling during the connect or the backoff sleep must return promptly. Test in **Task 8**; revert-check restores the bare `sleep`.
2. **A dropped/removed route leaks a bound port or a spawned task** (defect 3). The spawn-disposition table (Task 7) is the inventory; a test drops a route with a saturated accept and proves the port is released. **Task 7**.
3. **A timestamp/duration migration changes a wire byte** (`availabilityStartTime`, `@duration`, `EXT-X-PROGRAM-DATE-TIME`). Goldens byte-compared. **Task 3**.
4. **A test still sleeps or rebinds ports**, flaking under load. A grep-based harness guard (Task 10) fails on any new `reserve_*`/`sleep` in `tests/`. **Task 10**.
5. **A `jiff` migration emits fractional seconds where the old code emitted none.** The MPD golden freezes `now`, and any diff is listed. **Task 3**.

---

### Task 0: Worktree setup (rebasing over W2a)

**Files:** none.

- [ ] **Step 1: Create the worktree and rebase over W2a (explicit, no `|| true`)**

```bash
cd /Volumes/External/Projects/rust-broadcast
git fetch -q origin
git worktree add -b w2/b .worktree/w2-b origin/main
cd .worktree/w2-b
# W2a must be merged. If it is not on origin/main yet, take its branch and
# fail loudly on conflict rather than hiding it:
if ! git merge-base --is-ancestor origin/main w2/a 2>/dev/null; then
  git merge --no-edit w2/a || { echo "W2a merge conflict — resolve, do not skip"; exit 1; }
fi
git -c protocol.file.allow=always submodule update -q --init --reference /Volumes/External/Projects/rust-broadcast/private private
ln -s /Volumes/External/Projects/rust-broadcast/.test-streams .test-streams
ln -s /Volumes/External/Projects/rust-broadcast/multimux/tests/assets/node_modules multimux/tests/assets/node_modules
export CARGO_TARGET_DIR="$PWD/target"
```

- [ ] **Step 2: Baseline**

```bash
timeout 2400 cargo test --locked --all-features -p multimux 2>&1 | grep -E '^test result|FAILED|panicked' | sort | uniq -c
```

Record counts in `.delegate/w2b-report.md`.

---

### Task 1: Add W2b-1 dependencies and capture URL/time goldens

**Files:** `multimux/Cargo.toml` (`jiff = "0.2"`), `multimux/tests/golden/*`.

- [ ] **Step 1: Add `jiff`** (backon/socket2/parking_lot/futures-util are W2b-2).

```bash
CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=fallback cargo update -p jiff
git diff Cargo.lock | grep -E '^[-+]name' | sort | uniq   # expect only jiff + its transitive additions
```

- [ ] **Step 2: Golden the DASH/LL-DASH MPDs with `now` frozen**

The `dash_mpd`/`ll_dash_mpd` goldens freeze the `now`-dependent `availabilityStartTime`. `render_mpd` uses `route.created_at()` (a `SystemTime`); add `#[doc(hidden)] pub fn render_mpd_at(route, now: SystemTime)` so the golden is deterministic, and `render_mpd` delegates with `SystemTime::now()`. Golden `multimux/tests/golden/dash_mpd.golden` + `ll_dash_mpd.golden` via the `GOLDEN_BLESS` pattern (`output/smooth.rs:972`).

- [ ] **Step 3: Commit**

```bash
git add multimux && git commit -m "test(multimux): golden the DASH/LL-DASH MPDs (now-frozen) before the jiff migration"
```

---

### Task 2: multimux URLs via the `url` crate (IPv6 bracketing characterised)

**Files:** `redact.rs` (23–101), `push/srt.rs` (71–187), `config.rs` (1099–1118), `push/rtmp.rs` (359), `push/rtsp.rs` (119–123), `source/rtsp.rs` (649–709). New test `multimux/tests/url_builders.rs`.

**Interfaces:** no public signature change. Produces `#[doc(hidden)]` accessors `tc_url_for_test`, `control_url_for_test`, `parse_srt_url_for_test`.

- [ ] **Step 1: Write the failing test** (real assertions; IPv6 + redaction of unparseable URLs):

```rust
//! SP3 (url crate). The IPv6 rows are CHARACTERISATION: main already keeps the
//! brackets via `Url::host_str()` (verified push/rtmp.rs:351-359,
//! push/rtsp.rs:94-96, source/rtsp.rs connect_addr); defect 8 was rtmp-runtime-only
//! and was fixed in W1-R-low-a. Redaction must ALSO keep working on a
//! URL the `url` parser rejects — redact.rs's documented contract.

use multimux::redact::{redact_destination, redact_url};
use multimux::push::srt::parse_srt_url_for_test;

/// CHARACTERISATION (passes on old code too — main already brackets IPv6;
/// pins the bracketing across the `url`/`RtmpTarget` migration).
#[test]
fn an_ipv6_host_is_bracketed_in_the_connect_address_and_tc_url() {
    let tc = multimux::push::rtmp::tc_url_for_test("rtmp://[::1]:1935/live/stream").unwrap();
    assert_eq!(tc, "rtmp://[::1]:1935/live");
    let control = multimux::push::rtsp::control_url_for_test("rtsp://[2001:db8::1]:554/live");
    assert_eq!(control, "rtsp://[2001:db8::1]:554/live/trackID=0");
    assert!(!tc.contains("://::1"));
    assert!(!control.contains("://2001:db8::1"));
}

/// CHARACTERISATION (passes on old code too — pins the behaviour across
/// the url migration).
#[test]
fn redaction_keeps_percent_encoded_credentials_opaque() {
    let url = "rtsp://user:p%40ss@cam.example/live";
    assert_eq!(redact_url(url), "rtsp://***@cam.example/live");
    assert_eq!(redact_destination(url), "rtsp://cam.example/<redacted>");
    assert!(!redact_url(url).contains("p%40ss"));
}

#[test]
fn redaction_still_masks_a_url_the_parser_rejects() {
    // The `url` parser rejects some strings redact must still MASK (redact.rs's
    // documented contract). The fallback is MASKING-ONLY: the credential is
    // replaced wholesale — the host is NOT extracted or echoed (the old
    // `rsplit_once('@')`-keep-host shape is deleted with it), so the whole
    // authority collapses to the mask token.
    assert_eq!(
        redact_destination("rtsp://user:p@cam local/live"),
        "rtsp://<redacted>/live"
    );
    // BITING (fails on old code — the old fallback kept the host,
    // `cam local`; the NEW masking-only fallback does not). The difference is
    // intended and listed in the CHANGELOG, and the guard test (Task 10's allowlist)
    // pins that `redact_unparseable` contains no host extraction.
}

#[test]
fn an_srt_query_keeps_the_address_and_streamid_separate() {
    let (addr, overrides) = parse_srt_url_for_test("srt://[::1]:9000?streamid=live%2Fcam&latency=120").unwrap();
    assert_eq!(addr, "[::1]:9000");
    assert_eq!(overrides.stream_id.as_deref(), Some("live/cam"));
}
```

- [ ] **Step 2: Run pre-fix — FAIL** on exactly these: the `tc_url_for_test` / `control_url_for_test` / `parse_srt_url_for_test` accessors do not exist yet (compile failure), and `redaction_still_masks_a_url_the_parser_rejects` fails (old fallback keeps the host). The IPv6 and SRT rows are characterisation and pass on old code once the accessors exist. Record the output.

- [ ] **Step 3: Rewrite the builders**

`redact.rs`: parse only when `Url::parse` succeeds — use `Url::set_username`/`set_password(None)` and re-serialize; for a URL that does not parse, fall back to a MASKING-ONLY scrub (spec §9 documented exception): replace `user:pass@` with `***@` and any percent-encoded credential token with `<redacted>`; **no host/port extraction** in the fallback (the current `rsplit_once('@')`-then-keep-host shape is deleted with it). The fallback lives in one function (`redact_unparseable`) that the Task 10 guard allowlists by name with the exception reason, plus an in-file test that fails if `rsplit_once('@')` reappears in it.

`push/srt.rs`: **the query split stays manual** — the module's own doc (push/srt.rs:50-58) records why `url`'s `query_pairs()` cannot be used (a Haivision `streamid=#!::r=…` value contains `#`, which a `Url` parser cuts as a fragment delimiter); that rationale is verified and kept as a documented exception. What changes: the AUTHORITY parse (`normalize_srt_authority`, 125–159, currently hand-rolled `rsplit_once(':')`/bracket logic) moves to `url::Host`/`SocketAddr` parsing (bracketed IPv6 handled by the parser, not by hand), and `percent_decode` (166–187) moves to `percent_encoding::percent_decode_str` (the crate is already a dep — `source/http_auth.rs:52` uses it). `SrtUrlOverrides` keeps its shape (`stream_id: Option<String>`, `latency_ms: Option<u16>`, verified push/srt.rs:44-47).

`config.rs::validate_host_port`: `SocketAddr::parse` for a literal (`host:port` / `[v6]:port`), `tokio::net::lookup_host`-equivalent validation for a hostname (parse host, validate port — no hand `rsplit_once(':')`). `push/rtsp.rs`: `Url::path_segments_mut().push("trackID=0")` (never `format!("{}/trackID=0")`). `push/rtmp.rs`: the tcUrl comes from `rtmp_runtime::target::RtmpTarget::parse(url)?.tc_url` (the `tc_url: String` field is pre-bracketed by `RtmpTarget` itself — verified main's `rtmp-runtime/src/target.rs:16-54` (R-low-a, merged)); `tc_url_for_test(url)` parses the URL through `RtmpTarget::parse` and returns `.tc_url`, so the test signature matches the real path. `source/rtsp.rs`: `connect_addr` builds from `SocketAddr` so an IPv6 host is bracketed; `sni_server_name` keeps its bracket-strip on `url::Host::Ipv6`.

- [ ] **Step 4: Run — PASS; Step 5: revert-checks** — (a) mutate the new builder to strip IPv6 brackets (e.g. `host.trim_matches(|c| c == '[' || c == ']')` before formatting) → `an_ipv6_host_is_bracketed_in_the_connect_address_and_tc_url` FAILS; (b) restore the old keep-host redaction fallback → `redaction_still_masks_a_url_the_parser_rejects` FAILS. Record both; restore. **Step 6: commit**

```bash
git add multimux && git commit -m "refactor(multimux): build every URL with url; masking-only redaction fallback"
```

---

### Task 3: Dates and durations via `jiff` (SP5)

**Files:** `output/dash.rs` (`format_iso8601` 286–300, `civil_from_days` 302–315), `output/ll_dash.rs` (205, 221, 226), `dvr.rs` (389–395), `source/dash_pull.rs` (`parse_iso8601_utc` 878–918, `now_unix_secs` 871–876). New test `multimux/tests/time_codec.rs`.

- [ ] **Step 1: Write the failing test** (real assertions):

```rust
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[test]
fn availability_start_time_is_jiff_and_matches_the_pre_jiff_spelling() {
    let t = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    assert_eq!(multimux::output::dash::format_iso8601_for_test(t), "2023-11-14T22:13:20Z");
}

#[test]
fn a_duration_uses_jiff_shortest_iso_8601_spelling() {
    assert_eq!(multimux::output::dash::xs_duration_for_test(Duration::from_secs(2)), "PT2S");
    assert_eq!(multimux::output::dash::xs_duration_for_test(Duration::from_millis(500)), "PT0.5S");
}

#[test]
fn availability_start_time_parses_back_to_the_same_instant() {
    assert_eq!(multimux::source::dash_pull::parse_iso8601_utc_for_test("2023-11-14T22:13:20Z"), Some(1_700_000_000));
    assert_eq!(multimux::source::dash_pull::parse_iso8601_utc_for_test("not-a-date"), None);
}
```

- [ ] **Step 2: Run pre-fix — FAIL** (the `*_for_test` accessors absent; the observable pre-fix is the `civil_from_days` path — the test bites by asserting the jiff accessors exist and the exact spelling). Record.

- [ ] **Step 3: Rewrite on `jiff`**

`format_iso8601(t)`: `jiff::Timestamp::from_second(secs)` → `to_string()`. Pin the spelling against the golden: if `jiff` renders `2023-11-14T22:13:20Z` identically, done; if it emits a fractional/offset form, use `jiff::civil::DateTime` + the W1-T `DateTimePrinter` (precision 0) exactly as timed-metadata did (`.delegate/w1-t-report.md` documented the `Timestamp::MAX` range reason). Durations use `jiff::fmt::temporal::SpanPrinter::new().duration_to_string(&jiff::SignedDuration::try_from(d)?)` (transmux `xs_duration`, dash.rs:1128). `parse_iso8601_utc`: `jiff::Timestamp::from_str(s)` with a lexical guard for the trailing `.fff`/offset forms; `now_unix_secs` → `jiff::Timestamp::now()`.

- [ ] **Step 4: Run — goldens byte-identical; Step 5: revert-check (restore `civil_from_days`; the frozen-`now` golden with the old code is captured at Task 1, so the witness is the compile-level difference + the golden equality); Step 6: commit**

```bash
git add multimux && git commit -m "refactor(multimux): dates/durations via jiff, matching transmux's W1-T spellings (SP5)"
```

---

### Task 4: SP7 harness — bind port 0 and pass the listener (delete reserve-then-rebind)

**Files:** the seven test files (`admin_api`, `dispatch_ingest`, `file_route`, `smooth_oracle`, `ts_hls_oracle`, `whep_egress`, `whip_ingest`) + the remaining source route ctors.

- [ ] **Step 1: Add the listener ctors** — W2a added WHIP/WHEP/origin/admin ctors; this task adds `RtmpRoute::with_listener`, `SrtRoute::with_listener`, `TsUdpRoute::with_listener`, `RtpUdpRoute::with_listener`, and `pub async fn serve_with_registry_on(listener, config, registry)` if W2a left it.

- [ ] **Step 2: Migrate the tests** — delete `reserve_tcp_addr`/`reserve_udp_addr`/`free_tcp_addr`/`free_port` and the `for _ in 0..10` rebind loops; bind `127.0.0.1:0` and pass the live listener into the code under test.

- [ ] **Step 3: 20 consecutive runs** (explicit status, not `grep && break`):

```bash
# Check cargo's own exit status (the rtk proxy can mask it): write the full
# output to a file, test $?, and never grep for "test result" as a pass
# signal — "test result: FAILED" contains the same substring.
for i in $(seq 1 20); do
  if ! timeout 900 cargo test -p multimux --all-features --locked       --test admin_api --test dispatch_ingest --test file_route       --test smooth_oracle --test ts_hls_oracle --test whep_egress --test whip_ingest       > "$PWD/target/run20-$i.log" 2>&1; then
    echo "RUN $i FAILED (rc=$?)"; grep -E 'FAILED|panicked' "$PWD/target/run20-$i.log" | head -5; break
  fi
done; echo "done"
```

- [ ] **Step 4: Commit**

```bash
git add multimux && git commit -m "test(multimux): bind port 0 and pass the listener; delete the reserve-then-rebind loops (SP7.1)"
```

---

### Task 5: SP7 harness — `wait-timeout` replaces the last poll-based `bounded.rs` copy (multimux); `rtsp_ingest` on rtsp-runtime's server adapter

**Files:** `multimux/tests/support/bounded.rs` (the ONLY poll-based copy left: hls-runtime's and media-doctor's copies are already `wait_timeout`-based on main — verified `wait_timeout::ChildExt` at hls line 19/51 and media-doctor line 18/52; the spec's "`bounded.rs` ×3" inventory is thereby reduced to one migration). Also confirm the `wait-timeout` dev-dep on all three (already present). Modify `multimux/tests/rtsp_ingest.rs`.

- [ ] **Step 1: Migrate the remaining poll-based `bounded.rs` copy.** The round-2 re-verification found hls-runtime's and media-doctor's copies are ALREADY `wait-timeout`-based; only `multimux/tests/support/bounded.rs` still polls with `try_wait` + `thread::sleep(POLL_INTERVAL)` (line 67). Copy hls-runtime's migrated form verbatim into the multimux copy (delete the poll loop and the `POLL_INTERVAL` const), and confirm all three crates carry `wait-timeout = "0.2"` in `[dev-dependencies]`. Assert with `grep -rn 'thread::sleep' multimux/tests/support/bounded.rs` returning nothing.

- [ ] **Step 2: Move `rtsp_ingest.rs` to rtsp-runtime's `AsyncRtspServer`** (`rtsp_runtime::io::{AsyncRtspServer, RtspTimeouts}` — on `main` since R-low-a's merge). `multimux/tests/rtsp_ingest.rs` on main (`31ba01ec`) already carries R-low-a's own edits, so the adapter migration applies on top of the current file — no rebase conflict to resolve.

- [ ] **Step 3: 20 consecutive runs + commit**

```bash
git add multimux && git commit -m "test: wait-timeout replaces multimux's poll-based bounded runner; rtsp_ingest on rtsp-runtime's server adapter (SP7.3/7.4)"
```

---

### Task 6: Add `announce`/`record`/`send_interleaved` to rtsp-runtime's `AsyncRtspClient` (unblocks W2b-2's RTSP push)

**Files:** `rtsp-runtime/src/io.rs` (the `AsyncRtspClient` impl — mirror `play` at line 260, `exchange` at 311, and the SERVER's `send_interleaved` at 720). New tests in `rtsp-runtime/tests/`.

**Interfaces:** three additive methods. The sans-IO `ClientSession::announce`/`record` already exist (`client.rs:295`/`:305`, returning `Vec<u8>`); the interleaved SEND has a server-side twin (`AsyncRtspServer::send_interleaved`, io.rs:720, building an `InterleavedFrame` and writing it bounded) but NO client-side one — the RTSP push (RECORD with interleaved delivery) needs it.

```rust
/// Sends `PLAY`-companion media as a `$`-framed interleaved packet on the
/// client's negotiated channel (the mirror of the server's
/// [`AsyncRtspServer::send_interleaved`]). The frame is built by
/// `crate::interleaved::InterleavedFrame::new` and written under
/// [`RtspTimeouts::write`].
pub async fn send_interleaved(&mut self, channel: u8, payload: &[u8]) -> Result<()> {
    let frame = crate::interleaved::InterleavedFrame::new(channel, payload.to_vec());
    let bytes = frame.to_bytes()?;
    bounded(self.timeouts.write, "write", self.framed.send(bytes)).await
}
```

- [ ] **Step 1: Write the failing tests** (in `rtsp-runtime/tests/`, driving a loopback `AsyncRtspServer`):

```rust
// announce/record round-trip: the server sees ANNOUNCE then RECORD.
#[tokio::test]
async fn announce_then_record_round_trips_over_loopback() {
    let (client, mut server) = rtsp_loopback_pair().await;
    let ev = client.announce("rtsp://127.0.0.1/live", "v=0
...").await.unwrap();
    assert!(matches!(ev, ClientEvent::Response { status: 200, .. }), "{ev:?}");
    let ev = client.record("rtsp://127.0.0.1/live").await.unwrap();
    assert!(matches!(ev, ClientEvent::Response { status: 200, .. }), "{ev:?}");
}

// send_interleaved: the server receives a $-framed payload on the channel.
#[tokio::test]
async fn a_client_interleaved_send_arrives_as_a_frame_at_the_server() {
    let (mut client, mut server) = rtsp_loopback_pair().await;
    client.send_interleaved(0, b"media-payload").await.unwrap();
    let req = tokio::time::timeout(Duration::from_secs(5), server.next_request()).await;
    // The server's codec surfaces interleaved frames as part of its event
    // stream: assert the payload bytes arrive (the exact event shape is
    // pinned against what ServerCodec yields for interleaved data today).
    let got = expect_interleaved_payload(req).await;
    assert_eq!(got, b"media-payload".to_vec());
}

// the interleaved send is bounded by RtspTimeouts::write.
#[tokio::test(start_paused = true)]
async fn a_stalled_interleaved_send_times_out_at_the_write_bound() {
    let (mut client, _server_that_never_reads) = stalled_rtsp_pair().await; // peer accepts, never reads
    let h = tokio::spawn(client.send_interleaved(0, b"x"));
    tokio::time::advance(Duration::from_secs(2)).await;  // past the 1 s write bound
    let done = tokio::time::timeout(Duration::from_millis(100), h).await;
    assert!(done.is_ok(), "the stalled send must fail at the bound: {done:?}");
}
```

- [ ] **Step 2: Run — FAIL (all three methods absent). Step 3: implement** — `announce`/`record` mirror `play` exactly (three lines each, `peek_next_cseq` + session method + `exchange`, verified against main's `rtsp-runtime/src/io.rs:255-281`); `send_interleaved` is the code block above (mirrors the server's at io.rs:720-725: `InterleavedFrame::new` → `to_bytes()` → `bounded(timeouts.write, …)` — verified `interleaved.rs:38/54`).

- [ ] **Step 4: PASS; Step 5: revert-check** — delete `bounded(...)` from `send_interleaved` (bare `self.framed.send(bytes)`); `a_stalled_interleaved_send…` FAILS. Restore. **Step 6: commit** (`feat(rtsp-runtime): AsyncRtspClient announce/record/send_interleaved for the RTSP pusher`). All three are **breaking-class additive** and tracked in `.delegate/release-versions.txt` (land in W2, not W1).

---

### Task 7: Spawn ownership — enumerate every spawn, give each a `TaskTracker`/`JoinSet` owner

**Files:** the 16 spawn sites in the disposition table below (every NON-TEST `tokio::spawn`/`spawn_blocking` in `multimux/src/`, brace-scanned; the raw grep's ~112 hits are mostly `#[cfg(test)]` modules).

**Interfaces:**
```rust
// task_owner.rs
pub(crate) use tokio_util::task::TaskTracker;
pub(crate) use tokio::task::JoinSet;
```

- [ ] **Step 1: Enumerate every NON-TEST spawn** (`grep -rn 'tokio::spawn\|task::spawn\|spawn_blocking' multimux/src`, excluding `#[cfg(test)]` module bodies — the raw grep count is ~112 including tests; the non-test production list is below, re-derived by brace-scanning `#[cfg(test)] mod` blocks):

| # | file:line | what it spawns | owner today | owner (this task) | cancelled how |
|---|---|---|---|---|---|
| 1 | `origin/mod.rs:1216` | `spawn_following` → `follow_trunk` (each push-output egress) | bare `tokio::spawn` | route's `TaskTracker` | route's `CancellationToken` child |
| 2 | `origin/mod.rs:1604` | `spawn_ingest` → `supervisor::supervise_driver` (the route supervisor) | bare `tokio::spawn` (handle joined at shutdown) | route's `TaskTracker` | route token (supervise loop's cancelled arm) |
| 3 | `origin/admin.rs:1084` | Ctrl-C/SIGTERM → `external_shutdown_tx` watcher | bare `tokio::spawn`, `.abort()`ed at 1118 | the admin serve's `TaskTracker` | abort at shutdown (unchanged) |
| 4 | `origin/admin.rs:1100` | the admin-API HTTP server | `JoinHandle`, timeout-joined at 1125 | the admin serve's `TaskTracker` | `admin_cancel` child token |
| 5 | `source/rtmp.rs:302` | RTMP accept pump | bare `tokio::spawn` | source's `TaskTracker` (W2a Task 6 for the axum WHIP/WHEP pumps; same shape here) | source token |
| 6 | `source/whip.rs:276` | WHIP accept pump | bare `tokio::spawn` | **W2a Task 6** (already tracked there) | W2a token |
| 7 | `source/whip.rs:290` | WHIP per-connection handler | bare `tokio::spawn` | **W2a Task 6** | W2a token |
| 8 | `source/file_reader.rs:376` | `FileReaderSource::spawn` → `self.run()` (the paced file loop) | `JoinHandle` returned to the caller | route's `TaskTracker` (the supervisor joins it today; keep the join, add the tracker) | route token + natural EOF |
| 9 | `output/whep.rs:1041` | WHEP accept pump | bare `tokio::spawn` | **W2a Task 6** | W2a token |
| 10 | `output/whep.rs:1056` | WHEP per-connection handler | bare `tokio::spawn` | **W2a Task 6** | W2a token |
| 11 | `output/whep.rs:1101` | `run_whep_session` (one task per viewer session) | `VecDeque<JoinHandle>`, `.abort()`ed all at 1111 | source's `TaskTracker` (abort-on-drain becomes `close()`+`wait()`) | session token child |
| 12 | `dvr.rs:580` | `archive_seq_floor` archive scan (`spawn_blocking`) | awaited inline | block-on tracker (fire-and-await: no change needed — it is awaited to completion in the same expression; record as "awaited inline, not detached") | n/a (awaited) |
| 13 | `route.rs:643` | DVR poll (`spawn_blocking`) | awaited inline | same as 12 | n/a (awaited) |
| 14 | `output/smooth.rs:129,183,232` | 3× manifest layout builds (`spawn_blocking`) | awaited inline | same as 12 | n/a (awaited) |
| 15 | `output/catchup.rs:177,255,354,365,432` | 5× archive scan/read (`spawn_blocking`) | awaited inline | same as 12 | n/a (awaited) |
| 16 | `source/file_reader.rs:438` | file probe (`spawn_blocking`) | awaited inline | same as 12 | n/a (awaited) |
| 17 | `source/hls_pull.rs:142,526` / `dash_pull.rs:1025` / `smooth_pull.rs:1070` | per-fetch fan-out | already a `tokio::task::JoinSet` (`inflight`), bounded by `may_spawn_fetch` | unchanged (JoinSet IS the owner); the pull-scheduler rewrite (W2b-2 Task 2) keeps it | loop end / cancel |
| 18 | `source/srt.rs` SRT pump | srt-runtime's own listener task (W1-R-low-b already tracks it inside `srt_runtime::io`) | srt-runtime's tracker | unchanged | srt-runtime's token |

(`webrtc_http.rs` is deleted by W2a Task 4 — not listed. All `origin/supervisor.rs` spawns are inside its `#[cfg(test)]` block at line 389 — excluded. `source/rtsp.rs`, `ts_http.rs`, `source/mod.rs`, `origin/limit.rs`, `output/llhls.rs` have NO non-test spawns — their grep hits are all in test modules.)

- [ ] **Step 2: Give each a tracked owner**, routing by concern:
  - route supervisors (`origin/supervisor.rs`, `origin/mod.rs`) → one `TaskTracker` per route held by `RouteRuntime`/`StreamRoute` (Task 8 moves them onto `CancellationToken`).
  - output egress (`output/*`, `push/*`, `push/egress.rs`) → a `TaskTracker` per output, closed by the route's token.
  - source accept pumps (`source/rtmp.rs`, `source/srt.rs`, `source/whip.rs` — whip already done in W2a Task 6) → a `TaskTracker` per source.
  - one-off helper spawns (`origin/resource.rs`, `origin/limit.rs`, `route.rs`) → the request/route-scoped `JoinSet`.
  - `dvr.rs` recorder → the route tracker.

- [ ] **Step 3: Regression test** (real code, biting pre-fix)

`multimux/tests/spawn_ownership.rs`:

```rust
//! SP1.4: every detached spawn is owned by a tracked task; cancelling the
//! route's token drains its tracker and releases the route's ports.

use std::time::Duration;

#[tokio::test]
async fn cancelling_a_route_drains_its_task_tracker_and_releases_ports() {
    // A WHEP route: it spawns the accept pump AND a session task per viewer
    // (table rows 9-11). Pre-fix those are bare tokio::spawns/JoinHandles the
    // route never joins on cancel — nothing observes their exit, and the
    // listen socket's drop is left to task teardown order.
    let (addr, tracker, token) = multimux::output::whep::serve_whep_tracked_for_test().await;
    token.cancel();

    // The route's tracker must drain within a bounded wait (TaskTracker has
    // no join_next; close()+wait() is the drain API — verified tokio-util
    // 0.7.19 task_tracker.rs:318/337).
    tracker.close();
    let drained = tokio::time::timeout(Duration::from_secs(5), tracker.wait()).await;
    assert!(drained.is_ok(), "the route's tasks must all exit on cancel");

    // And the port is free again.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::net::TcpListener::bind(addr).await {
            Ok(_) => break,
            Err(_) if tokio::time::Instant::now() < deadline => {
                tokio::task::yield_now().await
            }
            Err(e) => panic!("the route's port {addr} stayed bound after cancel: {e}"),
        }
    }
}
```

`serve_whep_tracked_for_test() -> (SocketAddr, Arc<TaskTracker>, CancellationToken)` is a `#[doc(hidden)]` helper that runs the real `run_whep` and hands back the tracker this task adds to `WhepRun`. Pre-fix it FAILS: no tracker exists to return (compile), and behaviourally the accept pump is a bare `tokio::spawn` whose exit nothing observes — the port-release race the bounded bind loop then catches intermittently; the tracker-drain assertion is the deterministic bite. Record both.

- [ ] **Step 4: Revert-check** — revert the WHEP pump to a bare `tokio::spawn` (row 9) and re-run Step 3: the drain assertion FAILS (`tracker.wait()` returns while the pump still holds the socket, or the helper's tracker is empty and the bind race catches the pump's socket). Restore.

- [ ] **Step 5: Commit**

```bash
git add multimux && git commit -m "refactor(multimux): every spawn owned by a TaskTracker/JoinSet with a token disposition (SP1.4)"
```

---

### Task 8: One `CancellationToken` shutdown; push connect/backoff/listen-wait cancel-aware (SP1.4, defect 5)

**Files:** `origin/mod.rs` (954–1064), `origin/supervisor.rs` (282–387), `origin/admin.rs` (345–357, 686–706), `registry.rs` (74, breaking), `push/mod.rs` (436–440, 544, 613), `examples/custom_scheme.rs`, `multimux-cli/src/main.rs`. New test `multimux/tests/shutdown_cancel.rs`.

**Interfaces:**
```rust
pub async fn supervise_driver<F, Fut>(attempt: F, route_handle: Arc<RouteHandle>, schedule: Backoff, name: String, cancel: tokio_util::sync::CancellationToken);
pub struct InputCtx { …, pub cancel: tokio_util::sync::CancellationToken }   // was shutdown_rx: watch::Receiver<bool>
```
The public `watch<bool>` shutdown is **removed** (spec §8 breaking).

- [ ] **Step 1: Write the failing test** (real code, paused time; every type verified against source):

```rust
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use media_plane::trunk::{Trunk, TrunkConfig};
use multimux::config::ReconnectPolicy;
use multimux::push::{PushFormat, PushTransport, drive_push};

/// A `PushTransport` whose `connect` always fails, so `drive_push` lands in
/// the reconnect backoff after its first attempt — the same shape as main's
/// own `AlwaysFailsTransport` (push/mod.rs:645-660), including the
/// `#[async_trait::async_trait]` on the impl (the trait's methods are native
/// `async fn` desugared by the attribute; the impl carries no `where
/// Self: Sized`, matching that test transport's form).
struct NeverConnect;

#[derive(Debug)]
struct NeverErr;
impl std::error::Error for NeverErr {}
impl std::fmt::Display for NeverErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "never connects")
    }
}

#[async_trait::async_trait]
impl PushTransport for NeverConnect {
    type Config = ();
    type Error = NeverErr;
    async fn connect(_url: &str, _config: &Self::Config) -> Result<Self, Self::Error> {
        Err(NeverErr)
    }
    async fn send(&mut self, _data: &[u8]) -> Result<(), Self::Error> {
        Err(NeverErr)
    }
}

/// `TrunkConfig` has NO `Default` (five `NonZeroUsize` capacities, verified
/// media-plane trunk.rs:734) — build it with small non-zero capacities.
fn test_trunk() -> Arc<Trunk> {
    let cap = NonZeroUsize::new(8).unwrap();
    Arc::new(Trunk::new(
        TrunkConfig::new(cap, cap, cap, cap, cap),
    ))
}

#[tokio::test(start_paused = true)]
async fn cancelling_during_a_push_backoff_wait_returns_promptly() {
    let cancel = tokio_util::sync::CancellationToken::new();
    let h = tokio::spawn(drive_push::<NeverConnect>(
        test_trunk(),
        "srt://127.0.0.1:9/streamid=x".into(),   // rtt: no resolver hit; connect fails at once via NeverConnect
        (),
        PushFormat::MpegTs,
        ReconnectPolicy::default(),             // initial_backoff 1 s, max 30 s — verified config.rs:370-410
        cancel.clone(),
    ));
    // The first connect fails immediately; the loop then sleeps the 1 s
    // initial backoff (push/mod.rs:613's bare `sleep(wait).await`, which
    // pre-fix ignores `cancel`). Advance past it, then cancel mid-backoff.
    tokio::time::advance(Duration::from_secs(2)).await;
    cancel.cancel();
    let done = tokio::time::timeout(Duration::from_millis(50), h).await;
    assert!(
        done.is_ok(),
        "cancel must abort the backoff sleep, not wait out the next retry: {done:?}"
    );
}

- [ ] **Step 2: Run pre-fix — FAIL** (the `sleep(wait).await` at `push/mod.rs:613` ignores `cancel`; the test times out at 50 ms). Record.

- [ ] **Step 3: Implement** — replace `watch::channel(false)` with a `CancellationToken` tree; `supervise_driver` selects `cancel.cancelled()`; `drive_push` races `connect` (line 544) and the backoff `sleep` (613) against `cancel`, and replaces the `trunk.listen()==None` 50 ms `sleep` (440) with an event-driven wait (await `trunk.listen()`'s `ProgressListener`, or a trunk `Notify` — no fixed sleep). Wire `multimux-cli/src/main.rs` to build the root token and translate ctrl-c into `token.cancel()` (B14/B15).

- [ ] **Step 4: Run — PASS; Step 5: revert-check (remove the `select!` around `sleep(wait)`) → FAIL; Step 6: commit**

```bash
git add multimux multimux-cli && git commit -m "refactor(multimux)!: one CancellationToken shutdown; push connect/backoff/listen-wait are cancel-aware (SP1.4, defect 5)"
```

---

### Task 9: File pacer and the `listen()` sleep fallback (B10b)

**Files:** `source/file_reader.rs` (pacing loop 1440–1464, `next_due` 1195), `push/mod.rs` (436–440).

**Verified APIs:** `Trunk::listen() -> Option<ProgressListener>` (media-plane trunk.rs:2014) returns a slot-guarded `ProgressListener` whose `wait_deadline` IS awaitable (`event_listener::Listener::wait_deadline` under it) — the push loop's `Some(listener) => timeout(250ms, listener)` arm already awaits it. Only the `None` branch (no free waiter slot) falls back to the 50 ms `sleep(NO_SLOT_BACKOFF)` — a fixed sleep-poll. The file pacer's `next_due() -> Option<Instant>` exists (file_reader.rs:1194); its `None` branch sleeps a fixed 1 ms.

- [ ] **Step 1: Write the failing tests**

```rust
//! The pacer and the no-slot push wait are deadline/event-driven: no fixed
//! sleep-poll. Each test counts actual wake-ups through a counter the loop
//! bumps on every iteration.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[tokio::test(start_paused = true)]
async fn an_idle_file_source_spins_no_fixed_one_ms_poll() {
    // A source whose queue is EMPTY for 10 (virtual) seconds must not run
    // its loop ~10_000 times (the pre-fix 1 ms `sleep` fallback), nor park
    // past the first refill. The counter is bumped by the loop body.
    let (source, wakeups) = multimux::source::file_reader::idle_source_for_test().await;
    tokio::time::advance(Duration::from_secs(10)).await;
    let spins = wakeups.load(Ordering::Relaxed);
    assert!(
        spins < 100,
        "an empty queue must not 1 ms-spin: {spins} wake-ups in 10 s"
    );
    drop(source);
}

#[tokio::test(start_paused = true)]
async fn a_push_with_no_free_waiter_slot_does_not_fifty_ms_spin() {
    // Hold all of the trunk's waiter slots (mirroring push/mod.rs's own
    // test at :666 which holds the sole slot): `trunk.listen()` returns
    // None, and the pre-fix loop `sleep(50ms)`s. Under a paused clock the
    // counter shows a 50 ms cadence; the fixed sleep is the defect.
    let cap = std::num::NonZeroUsize::new(1).unwrap();
    let trunk = Arc::new(media_plane::trunk::Trunk::new(
        media_plane::trunk::TrunkConfig::new(cap, cap, cap, cap, cap),
    ));
    let _held_slot = trunk.listen().expect("the sole waiter slot is free here");
    let wakeups = Arc::new(AtomicU64::new(0));
    let cancel = tokio_util::sync::CancellationToken::new();
    let h = tokio::spawn(multimux::push::drive_no_slot_loop_for_test(
        Arc::clone(&trunk), cancel.clone(), Arc::clone(&wakeups),
    ));
    tokio::time::advance(Duration::from_secs(1)).await;
    cancel.cancel();
    let spins = wakeups.load(Ordering::Relaxed);
    // Pre-fix: ~20 wake-ups/s from the 50 ms sleep (>= 15 in 1 s).
    // Post-fix: the no-slot branch parks on a Notify until a slot frees,
    // so ~0 wake-ups while nothing changes.
    assert!(
        spins < 5,
        "the no-slot branch must park on an event, not a 50 ms sleep: {spins} wake-ups in 1 s"
    );
    let _ = h.await;
}
```

Helpers (both added BY this task, their counters bumped by the loop bodies the task rewrites): `idle_source_for_test()` builds a `FileReaderSource` whose ingest queue is empty (an already-drained pass, `pace: true`), adds a wake-up `AtomicU64` the run loop bumps once per iteration, and hands back `(handle, Arc<AtomicU64>)`; `drive_no_slot_loop_for_test(trunk, cancel, counter)` extracts the push loop's wait section (`trunk.listen()` match at push/mod.rs:436–440) into a `#[doc(hidden)] pub async fn` that bumps `counter` once per wait-arm iteration — the same loop body this task rewrites, exposed so the cadence is observable without a whole push transport. Both counters did not exist before this task; the pre-fix FAIL is measured by temporarily bumping the OLD loop bodies (add the same one-line bump to the pre-fix `None => sleep(...)` arms, run, record the count, then apply the rewrite).

- [ ] **Step 2: Run pre-fix — FAIL.** Test 1: the empty-queue `None => sleep(1ms)` arm (file_reader.rs:1461) gives ~10 wake-ups per virtual 10 ms → thousands in 10 s, far over the 100 bound. Test 2: the `None => sleep(50ms)` arm (push/mod.rs:440) gives ~20 wake-ups in 1 s, over the 5 bound. Record both counts.

- [ ] **Step 3: Implement**

Pacer (`file_reader.rs` run loop): replace the `None => tokio::time::sleep(Duration::from_millis(1)).await` arm with parking on a refill `Notify` — the queue's refill already happens in `feed`, so `notified()` is the event; `yield_now` stays only for the already-due burst path:

```rust
match driver.session().next_due() {
    Some(due) => {
        let now = std::time::Instant::now();
        if due > now {
            tokio::time::sleep(due - now).await;
        } else {
            tokio::task::yield_now().await;
        }
    }
    None => {
        // Park until the queue refills (feed notifies), not a 1 ms poll.
        refill_notify.notified().await;
    }
}
```

Push no-slot branch (`push/mod.rs:436–440`): the `Some(listener)` arm already awaits the real event; only replace the `None` arm's fixed sleep with a cancel-raced park on the trunk's waiter-slot release (a `Notify` in `Trunk`'s waiter-slot `Drop`, or — simpler and adequate — awaiting `child.cancelled()` raced with a slot-release notify):

```rust
match trunk.listen() {
    Some(listener) => {
        let _ = tokio::time::timeout(Duration::from_millis(250), listener).await;
    }
    None => {
        // No free waiter slot: park until one frees (or cancel) — never a
        // fixed 50 ms sleep-poll.
        tokio::select! {
            () = cancel.cancelled() => {}
            () = trunk.waiter_slot_freed().await => {}
        }
    }
}
```

`Trunk::waiter_slot_freed()` is a small additive method on media-plane's `Trunk` (a `Notify` fired by `WaiterSlot::drop`); if adding it to media-plane is out of bounds for this wave, fall back to `tokio::time::sleep_until(last + NO_SLOT_BACKOFF)` raced against `cancel` — still event-driven for cancellation, one bounded park instead of a spin, and the test bound loosens to `< 25` — record which path was taken.

- [ ] **Step 4: Run — PASS; Step 5: revert-check (restore both fixed sleeps) → both tests FAIL with the recorded counts; Step 6: commit**

```bash
git add multimux && git commit -m "refactor(multimux): event-driven file pacer and no-slot push wait; drop the fixed sleep polls"
```

---

### Task 10: The §5 guard for multimux (W2b-1 half) + the harness guard

**Files:** `multimux/tests/no_handroll_guard.rs` (the SAME file W2a Task 9 adds), `multimux/tests/harness_guard.rs`.

- [ ] **Step 1: Remove the URL/date allowlist entries cleared by Tasks 2/3**; keep the backoff/push/supervisor entries (cleared in W2b-2).

- [ ] **Step 2: Add a harness guard** — a `tests/harness_guard.rs` that fails on any `reserve_tcp_addr`/`reserve_udp_addr`/`free_tcp_addr`/`free_port` and any bare `tokio::time::sleep(`/`std::thread::sleep(` in `multimux/tests/**` outside the allowlisted bounded-wait helpers (`wait_for_rebind`'s `yield_now`).

- [ ] **Step 3: Run + bite-check (plant `reserve_tcp_addr` in a test → FAIL; remove) + commit**

```bash
git add multimux && git commit -m "test(multimux): tighten the no-hand-roll allowlist; add the test-harness port/sleep guard"
```

---

### Task 11: CHANGELOG, version notes, gate, hand-off — **do not merge**

- [ ] **Step 1: CHANGELOG** (`multimux`, `hls-runtime`/`media-doctor` for the bounded.rs move): `### Fixed` push cancel (defect 5), tracked spawns (defect 3), redaction no longer echoes the host of an unparseable URL; `### Changed (breaking)` shutdown token, `InputCtx::cancel`; `### Changed` jiff dates/durations (list any spelling diff), url builders, wait-timeout harness.

- [ ] **Step 2: Full gate** (`/Volumes/External/Projects/rust-broadcast/.delegate/gate-wt.sh "$PWD"`, expect 14 rc=0).

- [ ] **Step 3: `.delegate/w2b-report.md`** — baseline/after counts, the spawn disposition table (Task 7), byte diffs, revert-checks, version notes (`multimux` breaking; `rtsp-runtime` breaking-class additive).

- [ ] **Step 4: Hand off — do not merge, push, or tag.** W2b-2 rebases over W2b-1.

---

## Coverage table (W2b-1)

| Spec site / item | Where | Task |
|---|---|---|
| §3 URL `redact.rs` / `push/srt.rs` / `config.rs` / `push/rtmp.rs` / `push/rtsp.rs` / `source/rtsp.rs` | | 2 |
| §3 defect 8 (IPv6 tcUrl/RTSP) | not present in multimux (verified; rtmp-runtime-only, fixed in W1-R-low-a) — characterised | 2 |
| §3 dates `dash.rs`/`dvr.rs`/`dash_pull.rs`; durations `dash.rs`/`ll_dash.rs` | | 3 |
| §3 Runtime: pacer, `listen()` sleep fallback | | 9 |
| §3 defect 5 (push connect/backoff) | | 8 |
| §3 defect 3 (tracked spawns, port release) | | 7 |
| §3 Test harness: rebind ports, sleeps, `bounded.rs` ×3, `rtsp_ingest` server | | 4, 5 |
| §4 SP1.4 (TaskTracker/JoinSet, token shutdown) | | 7, 8 |
| §4 SP3.1/3.2 (url; SocketAddr/lookup_host) | | 2 |
| §4 SP5.3 (jiff) | | 3 |
| §4 SP7.1/7.2/7.3/7.4/7.5 | | 4, 5, 10 |
| §5 guard (W2b-1 half) + harness guard | | 10 |
| §7 W2 item 9/10 (tests; rtsp server adapter) | | 5 |
| multimux-cli shutdown wiring (B14/B15) | | 8 |
| rtsp-runtime `announce`/`record` (B11) | | 6 |

## Escalations

1. **W1-R-low-a is merged** (`3340fdaa` on main; `31ba01ec` followed): `AsyncRtspServer`/`RtspTimeouts`/`AsyncRtspClient`/`AsyncRtmpClient`/`RtmpTarget` are ordinary dependencies — resolved from the merged `main`, no branch handling. (Retained as a note because round-2's plan required the escalation; it is now moot.)
2. **`redact.rs` on unparseable URLs — documented exception, masking only.** The `url`-based redaction retains a text-scan fallback for strings `Url::parse` rejects (redact.rs's documented contract: redaction of text that "failed to parse in the first place"), RESTRICTED TO MASKING: the fallback replaces `user:pass@` with `***@` and secret-shaped tokens with `<redacted>`; it does NOT extract hosts or ports (the current fallback's `rsplit_once('@')` host extraction is removed with it). This is written into the plan as an explicit spec-§9-exception entry, the guard (Task 10) allowlists exactly the fallback function with that reason, and Task 2 adds a guard-style test asserting the fallback function contains no `rsplit_once('@')`/host-extraction. The two redaction tests that pass on old code are labelled CHARACTERISATION in the test file.
3. **`Timestamp::to_string()` spelling.** If `jiff` renders a whole-second timestamp differently from the old `YYYY-MM-DDTHH:MM:SSZ`, Task 3 uses the W1-T `civil::DateTime` + `DateTimePrinter` path (precision 0) and lists the diff; the frozen-`now` golden is the witness.
