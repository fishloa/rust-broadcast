# Rust Audit Report: rust-broadcast workspace

Audited tree: `main` @ `29038fb0` (the 2026-10-05 de-hand-roll release), 57 crates.
Read-only: no source file was modified, nothing was built (no `target/` writes). This report is the only file created.

## Method and limits (read this first)

Evidence is a mix of **lexical scans** (Python over `src/**/*.rs`, with `#[cfg(test)]` modules and `feature = "test-hooks"` items stripped by brace matching) and **targeted reads** of the flagged sites. Counts are upper bounds; the test-stripper misses some shapes (for example `multimux/src/origin/supervisor.rs` reports 14 hits that are test code). Not done: `cargo clippy`/`miri`/`loom` runs, profiling (so allocation and `.clone()` hot-path remarks are heuristics, not measurements), a line-by-line read of async cancellation safety (the earlier per-wave reviews covered multimux, rtsp/rtmp/srt/webrtc/hls; this audit only sampled), and `bindings/`, `demo/`, `fuzz/` (only scanned for `unsafe`). Anything marked *(unverified)* was not confirmed in source.

## Executive summary

**Health score: 8.5 / 10.** The codebase is unusually disciplined: only 3 `unsafe` sites in production code (all documented), edition 2024 and one MSRV everywhere, structured `thiserror` errors with no `anyhow` in libraries, no guard held across `.await` (heuristic), no `unsafe impl Send/Sync`, route names validated before they reach file paths, and 71 of 79 direct dependencies already at latest (the other 8 are patch-level behind).

Top 3 risks:

1. **`unsafe` policy is not met and not enforced.** The requirement is *no unsafe*. There are 2 production FFI `ioctl` calls, 1 hand-rolled volatile zeroizer, 8 copies of a test `unsafe impl GlobalAlloc`, 3 more test-only `unsafe` uses, and one real double-close hazard in a test. Only 19 of 57 crates carry `#![forbid(unsafe_code)]`, and there is no `[workspace.lints]`, so nothing stops new `unsafe` landing.
2. **Implicit-invariant panics in code that parses untrusted input.** 76 `first_chunk::<N>().unwrap()` / `try_into().unwrap()` sites in `dvb-si` and 29 panicking sites in `transmux` are each currently guarded by an earlier length check, but the guard is a convention, not a type. A refactor error becomes a remote panic on hostile broadcast input.
3. **No central dependency or lint policy.** 0 `[workspace.dependencies]` across 57 crates, so versions are copied per crate; the lock holds duplicate generations of `tower-http`, `sha2`/`digest`/`md-5`, `rand`/`getrandom`, `hashbrown` (x3), `syn`, `indexmap` and `webpki-roots`.

## 1. Memory safety and `unsafe`

> **Policy update (owner, later in the session): `unsafe` is allowed, but must be handled carefully.** The earlier "no unsafe" reading is superseded. Read the U1–U7 tables below as *rigor findings*, not policy violations. Required bar for every remaining `unsafe`: (1) smallest possible block, (2) a `// SAFETY:` comment stating each invariant and why it holds, (3) wrapped in a safe function whose signature makes misuse impossible, (4) isolated in one small module, with the rest of the crate `#![deny(unsafe_code)]` plus a scoped `#[allow(unsafe_code)]` on that module, (5) a test that exercises the invariant, (6) no `unsafe` where a safe, vetted crate API exists. Under this bar: U1/U2 stay (quarantine them, add a ioctl-struct layout test, no owner exception needed); U3 should still go (a vetted `zeroize` crate beats hand-rolled volatile); **U4 is a real bug and must be fixed regardless** (owning wrapper over a borrowed fd); U5/U7 are optional cleanups (U7: dedupe the 8 copies into one audited helper). Workspace lint: use `unsafe_code = "deny"` with per-module allows, not `forbid`.

**Production code: 3 sites, all with `// SAFETY:` comments, no UB found.**

| # | Location | What | Verdict |
|---|---|---|---|
| U1 | `dvb-ci-runtime/src/linux.rs:192` | `libc::ioctl(fd, CA_RESET)` | Sound (no argument, valid open fd). **Violates "no unsafe"; irreducible.** |
| U2 | `dvb-ci-runtime/src/linux.rs:209` | `libc::ioctl(fd, CA_GET_SLOT_INFO, &mut CaSlotInfo)` | Sound: `CaSlotInfo` is `#[repr(C)] { i32, i32, u32 }`, matching the kernel `ca_slot_info`, and outlives the call. **Violates policy; irreducible.** |
| U3 | `dvb-csa/src/zeroize.rs:21` | `ptr::write_volatile(slot, T::default())` plus `compiler_fence` | Sound, but it **re-implements the `zeroize` crate, which is already in the dependency tree** (`srt-runtime/Cargo.toml` depends on it). Easy removal. |

Why U1/U2 cannot be made safe by changing crate: I checked `rustix` 1.1.5 in the local registry. `rustix::ioctl::ioctl` is declared `pub unsafe fn` and `Ioctl` is `pub unsafe trait`, so an ioctl-based CA device driver always needs one `unsafe` somewhere. The owner has to choose: (a) one quarantined exception (a single `ioctl` module with `#[allow(unsafe_code)]`, crate otherwise `forbid`), or (b) drop the Linux CA-device back-end. I did not find or verify a third-party crate exposing a *safe* DVB-CA API *(unverified: I did not search crates.io beyond rustix)*.

**Test, bench and example code (also counted, per the requirement):**

| # | Location | Issue | Safe replacement |
|---|---|---|---|
| U4 | `multimux/tests/udp_bind.rs:32` | `socket2::Socket::from_raw_fd(self.as_raw_fd())` builds an **owning** socket from a borrowed fd, then relies on `mem::forget`. If the next `.expect("SO_RCVBUF readable")` panics, the socket drops and **closes an fd it does not own** (double-close; a later fd reuse can then hit the wrong file). | `socket2::SockRef::from(&udp)` (safe, borrowing; exists in socket2) |
| U5 | `dvb-ci-runtime/src/linux.rs:353, 629` (inside the test module) | `libc::mkfifo`, `libc::poll` | `rustix::fs::mkfifoat` (exists, safe) and `rustix::event::poll` (already imported at `linux.rs:20`) |
| U6 | `dvb-csa/src/zeroize.rs:71` | `ptr::drop_in_place` in a test | Disappears with U3 (`zeroize::Zeroizing`) |
| U7 | 8 test files: `dvb-ci-runtime/tests/pump_alloc.rs`, `dvb-ci/tests/hostile_alloc.rs`, `dvb-si/tests/decompression_size_bounds.rs`, `ssai-runtime/tests/per_viewer_allocations.rs`, `transmux/tests/{alloc_counts_sweep,alloc_measurement,cenc_senc_alloc_bound,hostile_input_bounds}.rs` | The same counting `unsafe impl GlobalAlloc` copied 8 times (5 to 7 `unsafe` lexical hits each). | One shared, unpublished test-support crate wrapping a vetted allocation-counting crate (the `unsafe` then lives in the dependency, not our code), like the `test-bounded` crate added in this release. |

**Enforcement gap.** `#![forbid(unsafe_code)]` is present in 19 of 57 crates. There is no `[workspace.lints]`. Once U1 to U7 are resolved, add `[workspace.lints.rust] unsafe_code = "forbid"` and inherit it in every crate (`[lints] workspace = true`); `forbid` also covers each crate's tests and benches.

No `unsafe impl Send/Sync`, no raw-pointer dereference, no uninitialised memory (`MaybeUninit`/`mem::uninitialized`) found in production code.

## 2. Ownership, borrowing, concurrency

- **Clones / allocation.** `.clone()` density is highest in `multimux` (291 sites, 5.1 per kLOC) and `rtsp-runtime` (30, 6.3 per kLOC); `transmux` has 173 `to_vec()/to_owned()` sites. I did not sample whether these are `Arc::clone`, small `String`s, or per-sample payload copies *(unverified: heuristic only)*. Positive: `media-plane` carries samples as `bytes::Bytes` (`trunk.rs:873,1233`), so fan-out is a cheap refcount clone. Recommendation: profile `transmux` demux on a real capture before changing anything.
- **Locks.** No guard held across `.await` found; no `Arc<Mutex<_>>` outside `multimux` (8) and `srt-runtime` (1). `multimux` still uses `std::sync::Mutex` in 28 places (WHIP/WHEP): `source/whip.rs:1314` does `.lock().expect("media transport mutex")`, so a poisoned lock cascades into a panic on a session task. The rest of the crate moved to `parking_lot` in this release; finish the job or recover with `PoisonError::into_inner`. (Low.)
- **Blocking in async.** One candidate: `multimux/src/source/udp.rs:183` (`bind_udp`) uses `std::net::UdpSocket` inside an `async fn`; it is non-blocking syscalls only, so benign. `dvb-ci-runtime/src/linux.rs` sleeps `RESET_SETTLE` (3 s) in a synchronous method; fine if called on a blocking thread, which should be stated in its doc *(unverified: not traced to callers)*.
- **Cancellation safety.** Sampled only; earlier review rounds fixed three cancel-safety defects (supervisor abandon-on-cancel, RTMP write path, HLS pacing). Residual risk: unreviewed `select!` arms added later; the lexical spawn-disposition guard in `multimux` helps.

## 3. Error handling and resilience

- **Good:** structured `thiserror` error types throughout; no `anyhow` in any library crate; `Box<dyn Error>` only in binaries (`media-doctor` 7, `ts-fix` 1) and 3 places in libraries (`multimux` 2, `hls-runtime` 1).
- **Panicking sites in production code: ~243** by the scan (129 `expect`, 92 `unwrap`, 20 `unreachable!`, 2 `panic!`), after excluding test modules and `test-hooks` seams. Most are guarded or infallible-by-construction. The ones worth changing:

| # | Location | Finding | Severity |
|---|---|---|---|
| P1 | `rtmp-runtime/src/chunk.rs:1427` | `ChunkWriter::write` **panics on a caller precondition** (`valid chunk_stream_id 2..=65599`) in a public API. Return `Err` (the project's own #1129 rule for lengths). | Medium |
| P2 | `multimux/src/origin/admin.rs:153,160,177,441,442,631` | `expect("armed")` x6 and `expect("... called twice")`: a runtime state flag standing in for a type. Use a type-state (`Pending` -> `Armed`) so the invalid state is unrepresentable. | Medium |
| P3 | `dvb-si/src/tables/*.rs` (76 sites; e.g. `bat.rs:123`, `ait.rs:328`, `compatibility.rs:277`, `demux.rs:449`) | `u16::from_be_bytes(*bytes[..].first_chunk::<2>().unwrap())` after `check_section_length`. Guarded today, but the guard is far from the use and the project parses untrusted broadcast data. Introduce one checked reader (`be_u16(bytes, off) -> Result`) and delete the unwraps. | Medium (robustness) |
| P4 | `transmux/src/init_segment.rs:742-745, 910-913` | `bytes[a..b].try_into().unwrap()` guarded by `bytes.len() < need` at `:730`. Same pattern as P3. | Low |
| P5 | `hls-runtime/src/server/engine.rs:1261,1367,1386,1448,1453,1455` | `expect` on `Duration::from_secs_f64` / finite-float invariants; use `Duration::try_from_secs_f64` and propagate. | Low |
| P6 | `multimux/src/source/whip.rs:1314-1316` | std-mutex poison `expect` plus a take-once `expect("... already taken")`. See section 2. | Low |

- **Lost error context.** `map_err(|_| ...)` appears 56x in `transmux`, 24x in `srt-runtime`, 20x in `st377-1`. Most are `TryFromIntError` -> a typed error carrying a field name (fine); I did not review each *(unverified)*. Ignored send/write results: `transmux` 3, `multimux` 1 (channel-closed `let _ = tx.send(..)` pattern, usually intended).

## 4. Type safety and domain modelling

- **Primitive obsession.** `Pid` and `ProgramId` newtypes exist, but 31 public signatures still take bare integers: `mpeg-ts/src/mux.rs` (7 x `pid: u16`), `dvb-stream` (3), `dvb-t2mi/src/pump.rs:363`, `transmux/src/ir/media.rs:31`, `transmux` `track_id: u32` (12 sites, e.g. `ll_dash.rs:307`), `media-plane/src/trunk.rs:2180`, `rtmp-runtime/src/chunk.rs:807` (`csid: u32`), and `media-doctor/src/report.rs:52` uses **`pid: u32` where everything else uses `u16`** (a width inconsistency a newtype would remove). Recommend `Pid`, `TrackId` and `Timescale` newtypes at the public boundaries.
- **Exhaustiveness.** `#[non_exhaustive]` and label-coverage drift guards are enforced per crate; wildcard arms on foreign `#[non_exhaustive]` enums (rtc-ice/dtls/stun) are now deliberate and fail safe (reviewed this release).

## 5. Dependencies and project hygiene

- Edition **2024** on all 58 manifests (54 explicit, 4 inherited); `rust-version` = workspace 1.95.0 for 56 crates, absent on 2 (likely `demo`/`fuzz`).
- **No `[workspace.dependencies]`** (57 crates, 79 distinct external deps) and **no `[workspace.lints]`**; clippy strictness exists only as CI flags (`-D warnings`).
- **Outdated:** 8 of 79 direct deps behind latest, all patch-level: `hyper` 1.11.0 -> 1.12.0, `hyper-util`, `http-body-util`, `jiff`, `serde_json`, `tokio-util`, `wasm-bindgen`, `zeroize`. None major. (Latest checked against the crates.io sparse index; MSRV compatibility of the newer versions was not checked.)
- **Duplicate generations in `Cargo.lock` (482 packages):** `hashbrown` x3; x2 each: `tower-http`, `webpki-roots`, `sha2`/`digest`/`md-5`/`crypto-common` (RustCrypto 0.10 vs 0.11), `rand`/`rand_core`/`rand_chacha`/`getrandom`, `syn`, `indexmap`, `cpufeatures`. The `tower-http` pair is the one most likely to be avoidable by aligning versions.
- **Heaviest direct dependency counts:** `multimux` 45, `rust-broadcast-fuzz` 40, `media-doctor` 22, `transmux` 22. Feature isolation looks deliberate (optional `libc`/`rustix` in `dvb-ci-runtime`, `test-hooks`/`test-seams` features), but I did not audit each `default-features` setting.

## 6. Security-relevant positives (so they are not "fixed" by accident)

- `multimux/src/dvr.rs:606-640` `validate_route_dir_name` rejects empty names, `.`/`..`/`..` substrings, `/`, `\`, NUL, over-long names, applied at the point a name becomes a path, with a unit test: no path traversal through route names.
- Hostile-length parsers are covered by allocation-bound tests (`dvb-ci`, `dvb-si` decompression, `transmux`); every one of them uses the `GlobalAlloc` copy in U7, so replacing that must preserve the bound checks.

## Prioritised action plan

**Quick wins (hours each)**
1. U4: replace the owning `from_raw_fd` with `SockRef::from(&udp)` in `multimux/tests/udp_bind.rs`.
2. U3/U6: delete `dvb-csa/src/zeroize.rs`; use `zeroize::Zeroizing` / `Zeroize` (already in the tree).
3. U5: `rustix::fs::mkfifoat` and `rustix::event::poll` in the `dvb-ci-runtime` test module.
4. P1: make `ChunkWriter::write` return an error on an invalid chunk-stream id.
5. Add `[workspace.lints.rust] unsafe_code = "deny"` now (forbid after step 7) and inherit it in all 57 crates.
6. Align `tower-http`; take the 8 patch updates.

**Medium (days)**
7. U7: one shared test-support crate for allocation counting, backed by a vetted crate; delete the 8 copies.
8. P3/P4: introduce one checked big-endian reader and remove the `first_chunk().unwrap()` / `try_into().unwrap()` pattern in `dvb-si` and `transmux`.
9. P2: type-state for `multimux`'s `PendingRuntime` (`Pending`/`Armed`/`Installed`).
10. Introduce `[workspace.dependencies]`; consolidate duplicate generations.

**Major / needs an owner decision**
11. U1/U2: the Linux CA `ioctl` calls cannot be made safe by any crate swap. Decide: quarantine one documented `unsafe` module (policy exception), or remove the device back-end.
12. `Pid`/`TrackId`/`Timescale` newtypes across public APIs (breaking: schedule with the next major-class release).
