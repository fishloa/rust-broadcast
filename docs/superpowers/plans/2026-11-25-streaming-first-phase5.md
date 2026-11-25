# Streaming-first Phase 5 (MPEG-PS and Matroska stream) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** MPEG-PS and Matroska/WebM inputs stream like every other container, Matroska syntax lives in its own crate, and no file input is read whole anywhere.

**Architecture:** `mpeg-ps` gains a streaming front end and transmux a `StreamingPsDemux` stage over it. A new crate `broadcast-matroska` owns EBML and Matroska element syntax, a sans-IO reader (forward-only or seekable, Cues-driven seeking) and a writer; transmux's WebM demuxer and Matroska muxer become thin stages over it. The whole-input caps for PS and WebM are deleted.

**Tech Stack:** Rust 1.95, `broadcast-common` (`Stage`, `Parse`/`Serialize`), `mpeg-ps`, `mpeg-pes`, `bytes`.

**Spec:** `docs/superpowers/specs/2026-11-24-streaming-first-containers-design.md` (§3 crate table, §5 Phase 5, §6).

**Tracking issue:** #4057.

## State of main this plan starts from

- Every muxer, transform, protection step and manifest writer in transmux is a stage (`mux_stage::{MuxInput, MuxOutput}`); the CLI streams TS, MP4 and FLV inputs to every output.
- Whole-input demuxers left: `transmux::ps_demux::PsDemux` (`Unpackage`, 1750 lines) and `transmux::webm_demux::WebmDemux` (`Unpackage`, 2184 lines). The CLI reads PS and WebM whole under `--max-whole-input-bytes`; multimux and media-plane read them whole under `FileReaderConfig::max_file_bytes`.
- Matroska syntax is hand-written twice inside transmux: read in `webm_demux.rs`, written in `mkv_mux.rs` (`MkvMuxStage`, 1831 lines). The `Segment` size is written as EBML unknown size; `Info.Duration` is patched with `MuxOutput::BytesAt`.
- Spec material: `private/specs/rfc8794_ebml.txt`, `private/specs/rfc9559_matroska.txt`, `private/specs-verified/mpeg/transmux/webm/` (EBML/Matroska and VP8/Vorbis-in-WebM transcriptions), `private/specs-verified/mpeg/mpeg-ps/`. Fixtures: `fixtures/mkv/*` (six files, three codecs families), `fixtures/ps/*`.

## Decisions (made by the orchestrator; binding for every slice)

1. **PS streams on `mpeg-ps`.** `mpeg-ps` gains a sans-IO pack/PES scanner (feed bytes, poll packs, system headers, program stream maps and PES packets; resync on a lost start code; holds at most one PES, capped by a caller limit whose hostile-input default lives with the crate's other limits). transmux's `StreamingPsDemux` is a `Stage<In<'a> = &'a [u8], Out = DemuxEvent>` over it, reusing the PES/access-unit code the TS demuxer already uses (no second copy). `PsDemux` and its `Unpackage` impl are deleted.
2. **One home for Matroska syntax: `broadcast-matroska`.** EBML (RFC 8794: element IDs, VINT sizes including unknown size, the EBML header) and Matroska elements (RFC 9559: Segment, SeekHead, Info, Tracks and TrackEntry with Video/Audio/ContentEncodings, Cluster, SimpleBlock, BlockGroup, Cues, Chapters, Tags, Attachments, Void, CRC-32) as typed structures with symmetric `Parse`/`Serialize` and round-trip tests. Unknown elements round-trip as raw id plus payload. `no_std` + `alloc`, depends only on `broadcast-common` + `thiserror`. A new workspace-guards family `one_ebml_home` (or a row in the existing `one_box_home` shape) bans EBML element-ID literals outside the crate.
3. **Reader and writer are sans-IO.** `broadcast_matroska::reader::Reader` mirrors `isobmff::reader`: `step()`/`supply(offset, bytes)`, `Step::{Need, Tracks, Block, Cues, Tags, Warning, Done}`, `Source::{Seekable, ForwardOnly}`, `seek_to(time)` through Cues (or a bounded Cluster scan without them), `Limits` for hostile input. It holds at most one Cluster header plus one block. `writer::Writer` writes header, tracks, clusters as blocks arrive, Cues at the end, with the unknown-size Segment and the `BytesAt` duration patch `MkvMuxStage` uses today.
4. **transmux stages over the crate.** `MatroskaDemux` (`seekable` / `sequential`, the same shape as `Mp4Demux`) replaces `WebmDemux`; `MkvMuxStage` is rewritten over the writer with byte-identical goldens. Both files lose their own EBML code.
5. **Caps deleted for PS and WebM.** The CLI's `--max-whole-input-bytes` and `CliError::WholeInputTooLarge` are deleted; multimux and media-plane stream PS and WebM files through their streaming file session; `FileReaderConfig::max_file_bytes` then applies to nothing (the field itself is deleted in Phase 6).

## Global Constraints

- No release, no version number change; breaking changes under each crate's CHANGELOG `[Unreleased]`. `broadcast-matroska` is a new crate at 0.1.0, unpublished until its release lane runs.
- Every golden and fixture round-trip stays byte-identical; a golden that must change needs the orchestrator's permission with a per-line reason.
- No whole input and no more than one bounded unit held per stage; no library-chosen policy numbers (caller supplies; hostile-input caps keep defaults in a `Limits` type).
- One function per job; converted code is deleted in the same change; no wrapper, alias or re-export.
- Run the FULL test suite of every crate touched and of every consumer (broadcast-client included when a type it uses changes); report counts before and after.
- An existing assertion may change only to the equivalent assertion on the new API; never raise a budget or re-pin a golden; list every changed assertion verbatim.
- `no_std` + `alloc` builds after the last edit (`alloc::` imports explicit); every example touched is run; every new or changed fuzz target runs 120 s for real.
- No numeric literal other than 0 and 1 outside a constant definition; no `#[allow]`/`#[expect]`; no `.unwrap()`/`.expect()`/`panic!` in non-test code; zero warnings; no narrating comments; new public enums get `name()` + `impl_spec_display!` or a `label` SKIP row; every layout cites the private transcription path and clause; no spec text in public files.
- A new crate clears `docs/CRATE-ACCEPTANCE.md` (round-trip invariant, real-fixture tests, fuzz, at least two examples, guards table rows, release-docs surfaces) and gets its release workflow (`release-broadcast-matroska.yml`, tag `broadcast-matroska-v*`) without being published.
- Baselines only via `git show origin/main:<path>` or a worktree under `.worktrees/`, never `/tmp`.

## Review Focus

- A Matroska file with an unknown-size Segment and unknown-size Clusters (live WebM): the reader must find Cluster boundaries by element ID, not by size. Test in slice 3.
- A PS stream that loses sync mid-pack (truncated capture): the scanner resyncs on the next pack start code and reports the gap, without dropping the following pack. Test in slice 1.
- EBML lacing (Xiph, EBML and fixed-size) in one SimpleBlock: each laced frame becomes one sample with the right timestamp. Test in slice 3.
- Seeking a seekable Matroska input with Cues to a time between two cue points lands on the preceding keyframe; without Cues the bounded scan gives the same answer. Test in slice 3.
- A 4 GiB generated Matroska and a 4 GiB generated PS through the CLI to TS under a small `ThreadCapped` budget. Tests in slices 1 and 4.

---

## Slices

### Slice 1: MPEG-PS streams

**Files:** `mpeg-ps/src/` (new scanner module), `transmux/src/ps_demux.rs`, the PES/access-unit code it shares with `ts_demux.rs`, `transmux/src/cli.rs` (PS input arm), `media-plane/src/file_stream*` and `file_source*`, multimux `source/file*`, `test-harness` (PS generator, collector), `fuzz/`, tests.

Decision 1 and the PS half of Decision 5.

**Gate:** every PS golden and fixture demuxes to the same samples as `PsDemux` did (differential test against `origin/main`'s output recorded before the deletion: per-track sample count, timestamps and a bytes hash for every `fixtures/ps/*` file); the resync Review Focus test; a 4 GiB generated PS → TS through the CLI under a fixed small budget (soak); the multimux PS file playout test streams with no cap; fuzz target for the scanner runs 120 s.

### Slice 2: `broadcast-matroska` syntax

**Files:** new crate `broadcast-matroska/` (workspace member, guards tables, `docs/CRATES.md`, README, CHANGELOG, release workflow, fuzz targets, two examples), nothing in transmux yet.

Decision 2.

**Gate:** every element type round-trips (parse → serialize → byte-identical, serialize → parse → equal); each `fixtures/mkv/*` file walks to the end and re-serializes byte-identically element by element; the element-ID constants are pinned to a drift test loading the private transcription at runtime (skips loudly without `private/`); fuzz targets (element walker, typed parse) run 120 s; CRATE-ACCEPTANCE checklist stated line by line.

### Slice 3: Matroska reader and writer

**Files:** `broadcast-matroska/src/{reader,writer,limits}.rs`, a blocking driver under feature `std` (as `isobmff::driver`), examples, fuzz, tests.

Decision 3.

**Gate:** the reader yields, for every `fixtures/mkv/*` file, the same tracks and samples as `WebmDemux` on `origin/main` (differential, recorded before any deletion); forward-only and seekable sources agree; the three Review Focus tests (unknown sizes, lacing, Cues seek); the writer reproduces `MkvMuxStage`'s current output byte-identically for the committed goldens; reader fuzz target runs 120 s; peak memory reading a generated 4 GiB file is bounded by one Cluster header plus one block (soak).

### Slice 4: transmux over the crate; caps deleted

**Files:** `transmux/src/webm_demux.rs` → `matroska_demux.rs`, `transmux/src/mkv_mux.rs`, `transmux/src/cli.rs`, `media-plane` and multimux file sources, `test-harness`, workspace-guards `one_ebml_home` rows, docs, `CLAUDE.md`.

Decisions 4 and 5.

**Gate:** every Matroska/WebM golden byte-identical (demux and mux); no EBML literal left in transmux (guard); the CLI converts a generated 4 GiB Matroska to TS under a fixed small budget (soak); `--max-whole-input-bytes` gone and `rtk proxy git grep -n -E "fs::read\(|read_to_end" -- transmux/src media-plane/src multimux/src` shows no media read; multimux WebM file playout streams.

## Order and landing

Slices 1 and 2 in parallel (disjoint crates). Slice 3 after 2. Slice 4 after 1 and 3 (both touch `cli.rs` and the file sources). Each slice is one PR, audited on the orchestrator's own gate run.
