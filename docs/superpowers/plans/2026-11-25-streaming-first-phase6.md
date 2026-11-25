# Streaming-first Phase 6 (delete and lock) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** nothing in the workspace can read a media input whole: the whole-input types and traits are deleted, every tool streams, a guard keeps it that way, and the wire-format contract moves to a stable `broadcast-core`.

**Architecture:** The remaining whole-input demuxers and the `Media`/`Track` IR are deleted; tests collect stage output through `test-harness`. Small bounded documents go through one capped-read helper. A lexical guard (`bounded_input`) bans whole-file reads and the deleted names. `broadcast-core` takes the wire-format contract out of `broadcast-common`.

**Tech Stack:** Rust 1.95, `broadcast-common`, `workspace-guards`, `test-harness`.

**Spec:** `docs/superpowers/specs/2026-11-24-streaming-first-containers-design.md` (§4.6, §5 Phase 6, §6).

**Tracking issue:** #4057.

## State of main this plan starts from

- Every container has a streaming demux stage (`StreamingTsDemux`, `Mp4Demux`, `StreamingFlvDemux`, `StreamingPsDemux`, `MatroskaDemux`, `RtpStreamDepacketiser`) and every output a muxer stage; the transmux CLI, multimux and media-plane stream every file input.
- Still present: the whole-input demuxers `TsDemux`, `FlvDemux`, `RtmpDemux`, `RtpDepacketiser` (each `Unpackage`); `transmux::{Media, Track}`; `mux_stage::drive_media`, `Repackage::run_media` and the `Media`-shaped helpers; the four traits in `broadcast_common::mux` (`Unpackage`, `Package`, `Encrypt`, `Decrypt`; only `Unpackage` still has production impls); `FileReaderConfig::max_file_bytes` (applies to nothing).
- `Unpackage` users outside transmux: broadcast-client (8 files), media-plane (4), multimux (3), media-doctor (1), dash-runtime (1), rtp-packet (1), `bindings/transmux-py` (outside the workspace: consumes a published transmux).
- `fs::read` / `read_to_end` / `read_to_string` appear in the `src/` or `examples/` of about 34 crates, mostly an example or CLI reading one file.
- `st377-1` (`Package` partitions), `dvb-csa`, `webrtc-runtime`, `srt-runtime` use the words Package/Encrypt/Decrypt for unrelated things.

## Decisions (made by the orchestrator; binding for every slice)

1. **One capped read.** `broadcast_common::bounded` (feature `std`): `read_bounded(reader, limit: Limit) -> Result<Vec<u8>, TooLarge>` and `read_file_bounded(path, limit)`, where `Limit` is a named newtype the caller constructs (no default, no "unlimited"); it never allocates past the limit and returns a typed error carrying the limit's name and value. Every small-document read in `src/` and `examples/` (config, playlist, SDP, key file, one wire structure in an example) goes through it. Async callers use `spawn_blocking`. There is no async twin.
2. **Media never goes through it.** A tool or example that takes a media file streams it through the matching stage or scanner in bounded chunks (`dvb-tools`, `ts-fix`, `media-doctor`, `compliance-probe`, the `tools/` binaries, every crate example that reads a capture).
3. **The whole-input demuxers and the IR containers are deleted.** `TsDemux`, `FlvDemux`, `RtmpDemux`, `RtpDepacketiser` and their `Unpackage` impls go; callers move to the streaming stage. `transmux::{Media, Track}`, `drive_media`, `Repackage::run_media` and every `Media`-shaped helper go. Tests that want "all samples of this fixture" use `test_harness::collect` (which owns its own collected-track type; it is `publish = false` and never a non-dev dependency). `SkippedTrack`, `PcrSample` and the other event payloads stay as `DemuxEvent` data.
4. **The four mux traits are deleted** from `broadcast_common::mux` (the module goes). Unrelated uses of the same words in other crates are untouched; the guard matches whole paths.
5. **`FileReaderConfig::max_file_bytes`** and its builder are deleted.
6. **transmux dependency diet.** `dvb-si`, `rtmp-runtime`, `cc-data`, `broadcast-hls`, `rtp-packet`, `mpeg-ps`, `broadcast-matroska` become optional features of transmux where the core IR and MP4/TS paths do not need them; `default` keeps today's behaviour. Each feature builds alone.
7. **The guard.** `workspace-guards` family `bounded_input`: bans `fs::read`, `fs::read_to_string`, `read_to_end`, `read_to_string`, their `tokio::fs` forms, and the whole paths `broadcast_common::mux::{Unpackage, Package, Encrypt, Decrypt}`, `transmux::Media`, `transmux::Track` in the `src/` and `examples/` of every workspace member. The one allowlisted site is the body of `broadcast_common::bounded`. No per-crate exemptions for media.
8. **`broadcast-core` 1.0.** A new crate holding exactly the wire-format contract (`Parse`/`Serialize`, `len`, the shared error kinds, `timescale`, `impl_spec_display!`, `CencScheme`, and whatever §4.1 of the spec lists), no dependencies, features `std` and `serde` only. Items move out of `broadcast-common` (deleted there, not re-exported) and every wire-format crate imports them from `broadcast-core`. A `workspace-guards` check pins its public API listing (may only grow). No crate is published by this plan: versions do not change until the owner calls a release.
9. **`bindings/transmux-py`** consumes a published transmux and cannot move before a release: it is left untouched and listed in the release notes' follow-ups.

## Global Constraints

- No release, no version number change; breaking changes under each crate's CHANGELOG `[Unreleased]`.
- Every golden and fixture round-trip stays byte-identical. A test moved off `Media` keeps its assertions; list every changed assertion verbatim.
- Never raise a budget, re-pin a golden, weaken or delete a test (a test of a deleted whole-input API is re-expressed on the stage; if its property no longer exists, say so and stop on that point).
- One function per job; deleted code is deleted, not deprecated, aliased or re-exported.
- Full suites of every crate touched and every consumer; counts before and after.
- `no_std` + `alloc` builds after the last edit; every example touched is run and registered in `tools/run-examples.sh` when it takes arguments; fuzz targets touched run 120 s.
- No numeric literal other than 0 and 1 outside a constant; no `#[allow]`/`#[expect]`; no `.unwrap()`/`.expect()`/`panic!` in non-test code; zero warnings; no narrating comments.
- Baselines only via `git show origin/main:<path>` or a worktree under `.worktrees/`; no unbounded helper scripts.

## Review Focus

- A tool given a 4 GiB capture (`dvb-tools dump`, `ts-fix`, `media-doctor check`): peak memory independent of the file. Soak test per tool in slice 4.
- A config or playlist larger than its named limit: a typed too-large error naming the limit, never an OOM or a silent truncation. Test in slice 1.
- A test that compared two whole `Media` values for equality: the collected form must compare the same fields (specs, sample bytes, timing, flags, protection), or the test silently weakens. Checked in slice 3 by a collector equality test that fails when any field is dropped.
- `transmux` built with `--no-default-features` plus each single feature after the dependency diet: every combination compiles and the default build's public API is unchanged. Slice 6.
- A crate that used `broadcast_common::Parse` through a glob import: after the `broadcast-core` move it must import explicitly; no glob re-export hides the move. Slice 8.

---

## Slices

### Slice 1: the capped-read helper and its adoption for documents

**Files:** `broadcast-common/src/bounded.rs`, every `src/` and `examples/` site that reads a small document (config, playlist, MPD, SDP, key, one wire structure): replace with the helper and a named limit.

**Gate:** helper unit tests (under, at and over the limit; never allocates past it, measured with `test-alloc`); every converted site builds and its tests pass; a list of the sites NOT converted because they read media (input to slices 2 and 4).

### Slice 2: delete the whole-input demuxers

**Files:** `transmux/src/{ts_demux,flv,rtmp,rtp}.rs`, their callers in broadcast-client, media-plane, multimux, media-doctor, dash-runtime, rtp-packet, `test-harness/src/collect.rs`, transmux tests.

Decision 3, first half: `TsDemux`, `FlvDemux`, `RtmpDemux`, `RtpDepacketiser` and their `Unpackage` impls.

**Gate:** every golden byte-identical; a recorded baseline per fixture from the deleted demuxers (as Phase 5 did) reproduced by the streaming stages at three chunk sizes; consumers' full suites.

### Slice 3: delete `Media`, `Track` and the mux traits

**Files:** `transmux/src/ir/media.rs`, `mux_stage.rs` (`drive_media`), `repackage.rs` (`run_media`), `broadcast-common/src/mux.rs`, `test-harness/src/collect.rs` (owns the collected-track type), every transmux test and example, consumers.

Decisions 3 (second half) and 4.

**Gate:** every golden byte-identical; the collector equality test of Review Focus; `rtk proxy git grep -n -w -E "Unpackage|drive_media|run_media" -- '*.rs'` prints only test-harness; transmux test count unchanged or higher.

### Slice 4: every tool and example streams

**Files:** `dvb-tools`, `ts-fix`, `media-doctor`, `compliance-probe`, `tools/` binaries, crate examples that read a capture.

Decision 2.

**Gate:** each tool's goldens unchanged; a soak per tool on a generated 4 GiB TS under a small `ThreadCapped` budget; every example runs.

### Slice 5: `max_file_bytes` deleted

**Files:** multimux `FileReaderConfig`, media-plane, docs.

Decision 5. Small; may ride with slice 2 or 4.

### Slice 6: transmux dependency diet

**Files:** `transmux/Cargo.toml`, `cfg` gates in `transmux/src`, consumers' feature lists, `tools/check-bare-features.sh` data.

Decision 6.

**Gate:** Review Focus (every single-feature build); default build's public API listing unchanged; consumers build.

### Slice 7: the `bounded_input` guard

**Files:** `workspace-guards/tests/guards/bounded_input*.rs`, any stragglers it finds.

Decision 7.

**Gate:** the guard passes with only the helper's body allowlisted; a deliberately planted `fs::read` in a scratch copy fails it (prove the guard bites, then remove the plant).

### Slice 8: `broadcast-core`

**Files:** new crate `broadcast-core`, `broadcast-common` (items removed), every wire-format crate's imports, guards tables, docs.

Decision 8.

**Gate:** the six CI gates across the workspace; the public-API pin test; no `pub use broadcast_core` in broadcast-common; `cargo metadata` shows every wire-format crate depending on `broadcast-core`.

## Order and landing

Slice 1 first. Then 2, 4 and 6 in parallel (2 owns the demuxers and their callers, 4 the tools, 6 the manifests); 5 rides with 2. Slice 3 after 2 and 4. Slice 7 after 3. Slice 8 last (it touches every crate; nothing else runs beside it). Each slice is one PR (slice 3 and slice 8 may be split by crate cluster), audited on the orchestrator's own gate run.
