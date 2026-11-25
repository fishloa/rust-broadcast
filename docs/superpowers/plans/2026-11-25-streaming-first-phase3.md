# Streaming-first Phase 3 (streaming MP4 demux in transmux and its consumers) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** MP4, progressive and fragmented, is demuxed in transmux by one `Stage` over `isobmff::Reader` that emits `DemuxEvent`s and never holds the file; every consumer feeds that stage; the two whole-input MP4 demuxers are deleted.

**Architecture:** `transmux::Mp4Demux` wraps the sans-IO reader. It has two input shapes on the one `Stage` trait: offset-tagged buffers answered against `Stage::demand()` (a seekable file), and plain sequential bytes (a pipe, or an init segment followed by media segments). It keeps transmux's own conditioning (sample entry to `CodecConfig`, legacy-codec refinement, NAL length normalisation, display tagging) and reports skipped tracks and damaged fragments as events. Consumers that today concatenate init and segment and re-demux keep one stage per representation instead.

**Tech Stack:** Rust 1.95, `isobmff` (reader, drivers), `broadcast-common` (`Stage`, `Demand::at`), `bytes`.

**Spec:** `docs/superpowers/specs/2026-11-24-streaming-first-containers-design.md` (§4.4, §5 Phase 3, §6). **Depends on** Phase 2 slices 1 to 5 and the reader fix lane (fragment samples emitted as soon as their `mdat` is located; permissive mode).

**Tracking issue:** #4057.

## State of main this plan starts from

- `transmux::media::Fmp4Demux` (`Unpackage` over a whole `&[u8]`, stateless, no `Stage`) and `transmux::progressive_demux::ProgressiveDemux` (`Unpackage`, plus a `Stage` impl that accumulates the whole input up to `max_bytes`). Both build a `Media`.
- Conditioning they share, in `transmux/src/media.rs`: `track_spec_from_trak`, `codec_config_from_entry`, `overlay_sample_entry_geometry`, `tag_fmp4_inband_display`, `refine_legacy_config(config, samples)`, `normalise_track_nal_lengths(spec, samples)`, `skipped_track`. A failed `moof` becomes `InputDegradation::MalformedUnit`; an unmappable track becomes a `SkippedTrack`. Neither demuxer reads `elst` or encryption boxes.
- `Track::start_decode_time`: first fragment's `tfdt` (fragmented) or 0 (progressive).
- `cenc_decrypt.rs` has its own third MP4 walk (`CencDecryptor::from_fmp4`, `demux_protected`). It stays until Phase 4.
- Non-test call sites of the two demuxers: transmux `cli.rs:582`, `repackage.rs:364`; hls-runtime `client/engine.rs` (`demux_and_emit`: init ++ segment), `server/engine.rs` (`set_init_from`); dash-runtime `client/demux.rs` (`init_info`, `demux_segment`), `ingest/session.rs` (two); smooth-runtime `client/stream.rs`, `server/engine.rs`; media-plane `file_source.rs` (`demux_identified`, behind `probe_and_demux`); multimux `source/file_reader.rs` (`read_capped_whole`, capped by `max_file_bytes`); media-doctor `container_codec.rs` (after `fs::read` in the binary); `demo/src/mp4.rs`; `test-harness/src/dash/loopback.rs` (`oracle`); fuzz target `transmux_progressive_demux`. Test call sites: several hundred, mostly `transmux/tests`.
- Streaming TS and FLV demuxers already implement `Stage<In = &[u8], Out = DemuxEvent>`; `media_plane::file_stream::StreamingFileSession` drives them through `FileDemand { Bytes, Rewind, Wait, Done }` / `FileInput { Bytes, Eof }`.
- `test-harness` has no helper that collects `DemuxEvent`s into per-track vectors.

## Decisions (made by the orchestrator; binding for every slice)

1. **One stage.** `transmux::mp4_demux::Mp4Demux`, `impl Stage` with `In<'a> = Mp4Input<'a>`, `Out = DemuxEvent`, `Error = transmux::Error`. `Mp4Input` is `#[non_exhaustive]`: `At { offset: u64, bytes: Bytes }` (answering a `Demand::at`), `Next(&'a [u8])` (sequential), `Eof`. Constructed by `Mp4Demux::seekable(Mp4DemuxConfig)` or `Mp4Demux::sequential(Mp4DemuxConfig)`; feeding the wrong shape is a typed error.
2. **Seekable mode.** `demand()` returns `Demand::at(offset, len)` for the reader's structure needs and then for sample bytes. Sample requests are one range per call; neighbouring samples of the order being emitted are coalesced into one range up to `Mp4DemuxConfig::read_window` bytes. `read_window` is caller policy: `None` (the default) means one request per sample; the library picks no size. Each `Sample.data` is cut from the fed `Bytes` with `slice`, never copied.
3. **Sequential mode.** The reader runs `Source::ForwardOnly` with `SampleOrder::File`. The stage holds at most one fragment (`moof` plus the `mdat` payload its runs address), capped by `Limits::max_fragment_bytes`, or for a `moov`-first progressive input the samples between the current position and the next needed offset: nothing more. `moov` after `mdat` is the reader's typed refusal. An init segment followed by any number of media segments is one sequential input: the stage keeps the `moov` it parsed and continues across `styp`/`moof` boundaries. `Mp4Input::SegmentEnd` (over the reader's `end_of_unit`) lets a segment consumer drop a damaged partial segment and continue with the next one under the same init.
4. **Events.** Per track, `TrackAdded` is emitted when its first sample has been conditioned (so `refine_legacy_config` has what it needs; the hold is one sample per track), then `TracksResolved` once every announced track has either been added or abandoned, or at the first sample of a second fragment, whichever comes first. A track whose sample entry transmux cannot map is `TrackAbandoned` with the fourcc in the reason (today's `SkippedTrack`). A malformed fragment in a fragmented input is `InputDegraded { kind: MalformedUnit }` and the stage continues with the next fragment (the reader's permissive mode supplies this); every other reader warning maps to `InputDegraded` with a kind that names it. Samples carry `dts`, `pts = dts + composition offset`, duration and the sync flag exactly as the old demuxers computed them.
5. **`start_decode_time`** is not an event. A consumer that needs it reads the first sample's `dts`. The collector in Decision 8 fills `Track::start_decode_time` the way each old demuxer did (first `tfdt`, or 0 for progressive) so goldens do not move.
6. **Conditioning moves, it is not rewritten.** `track_spec_from_trak` takes the reader's `TrackInfo`/typed sample entry; `refine_legacy_config` takes the first sample; `normalise_track_nal_lengths` becomes per-sample (`normalise_nal_lengths(spec, &mut Sample)`), with the spec rewrite done once before `TrackAdded`. One implementation of each; the old signatures are deleted with the old demuxers.
7. **Metadata tracks stay out of the IR in this phase.** A track with no transmux `CodecConfig` is abandoned as today. `CodecConfig::Data` with an MP4 identity is Phase 4 (design §4.9). The reader still surfaces those tracks to anyone using `isobmff` directly.
8. **Tests and the CLI still need a `Media` until Phase 4.** `test-harness` gains `collect::{collect_events, media_from_mp4, media_from_events}`: drive any demux stage over a small fixture and build per-track vectors or a `Media`. Inside transmux, `cli.rs`, `repackage.rs` and `cenc_decrypt.rs` use a `pub(crate)` collector over `Mp4Demux` with the same shape. No public whole-input MP4 demux entry point remains after slice 6.
9. **Segment consumers keep a stage per representation.** hls-runtime, dash-runtime and smooth-runtime feed the init once and then each media segment's bytes, and read events; they stop concatenating init and segment. A discontinuity or a new init replaces the stage. Their public outputs are unchanged.
10. **File playout.** `media_plane::file_stream` gains MP4: `FileDemand::BytesAt { offset, len }` and `FileInput::At { offset, bytes }` beside the sequential forms (both enums are `#[non_exhaustive]`); `StreamingFileSession` wraps `Mp4Demux::seekable`. multimux's `FileReader` answers `BytesAt` with a seek and a bounded read. `FileReaderConfig::max_file_bytes` stops applying to MP4 (its doc says so; the field goes in Phase 6). Looping re-creates the stage.
11. **Order of deletion.** `Fmp4Demux`, `ProgressiveDemux`, `demux_progressive`, `samples_from_stbl`, the `ProgressiveDemux` `Stage` impl and the `transmux_progressive_demux` fuzz target are deleted in slice 6, after every consumer has moved. Between slices old and new coexist on main; nothing is released.

## Global Constraints

- No release, no version number change. Breaking changes go under each crate's CHANGELOG `[Unreleased]`.
- Every golden and fixture round-trip in the workspace stays byte-identical.
- No whole file, and no more than one bounded unit, held in new code. No `fs::read`/`read_to_end` on a media path that this phase touches.
- No library-chosen policy number: read windows, pacing and buffer sizes are caller arguments with "unset" as the default. Hostile-input caps come from `isobmff::Limits`.
- One function per job; moved code is deleted where it was in the same change; no alias, wrapper or re-export.
- multimux and the runtimes stay thin: no box or sample-table logic outside `isobmff` and `transmux` (the `one_box_home` guard enforces the first half).
- No numeric literal other than 0 and 1 outside a constant definition; no `#[allow]`/`#[expect]`; no `.unwrap()`/`.expect()`/`panic!` in non-test code; zero warnings; no comments that narrate code; public enums get `name()` + `impl_spec_display!` or a `label` SKIP row.
- Named private fixtures hard-fail when the private checkout is present; tests load fixtures through `test_fixtures`.
- A lane that adds or changes a fuzz target runs it for 120 s and pastes the final line.
- Gate for every slice: the six CI gates on every crate touched, per-feature builds, `tools/check-bare-features.sh`, `cargo test -p workspace-guards --test guards`, `python3 tools/check-citations.py`, `python3 tools/check-doc-links.py`, `python3 -m pytest tools/tests -q`, `cargo build -p media-doctor --examples --locked`, side lockfiles under `--locked`, fuzz `cargo check --locked`.

## Review Focus

- An fMP4 whose first fragment's `tfdt` is not zero, and one whose tracks start at different `tfdt`s: sample `dts` values and the collector's `start_decode_time` must equal today's. Differential in slice 1.
- A segment consumer fed a media segment whose `moof` is damaged: the next segment must still demux under the same init, and the consumer must report the damage as it does today (`FragmentDamaged`). Tests in slices 2 to 4.
- A progressive file whose audio and video are not interleaved (all of one, then all of the other), seekable mode, decode-time order: correct order with memory bounded by `read_window` plus one sample, not by the distance between the tracks' data. Test in slice 1.
- An init segment change mid-stream (new `moov` on the same sequential input): the stage must not mix the old tracks' state with the new. Decide by test in slice 1: a second `moov` is a typed error telling the caller to start a new stage.
- A looped MP4 file in multimux: the second pass must produce the same samples as the first, with no growth in memory across loops. Test in slice 5.

---

## Slices

### Slice 1: `Mp4Demux`

**Files:** create `transmux/src/mp4_demux.rs`, `transmux/tests/it/mp4_demux.rs`, `transmux/tests/it/mp4_demux_differential.rs`, `transmux/tests/mp4_demux_big.rs`, `fuzz/fuzz_targets/transmux_mp4_demux.rs`, `test-harness/src/collect.rs`; modify `transmux/src/media.rs` (conditioning helpers take the reader's types and a single sample; the old demuxers call the same helpers), `transmux/src/lib.rs`, `transmux/src/error.rs`.

**Produces:** Decisions 1 to 8 (`Mp4Demux`, `Mp4DemuxConfig`, `Mp4Input`, the `test-harness` collector).

**Gate:**
- Differential, both modes: for every MP4 fixture the old demuxers accept (walk `fixtures/` and `private/fixtures/mp4`; a named minimum count), `media_from_mp4` over `Mp4Demux` equals the old demuxer's `Media`: track specs, per-sample bytes, `dts`, `pts`, duration, sync, display tags, `start_decode_time`, the skipped tracks (as abandoned), the degraded list. Sequential mode is checked on every fragmented fixture and every `moov`-first progressive one, fed in chunks of 1 byte, 7 bytes and 64 KiB.
- Init plus N segments fed as one sequential input equals demuxing `init ++ segment` N times (fixtures under `fixtures/dash`, `fixtures/hls/cmaf-fmp4`).
- `transmux/tests/mp4_demux_big.rs`: an 8 GiB generated MP4 (`test_harness::big_media::write_big_mp4`), progressive `moov`-first, `moov`-last and fragmented, demuxed in seekable mode with `read_window` unset and with a 1 MiB window: `test_alloc::ProcessPeak` under a named budget far below the file size; fragmented also in sequential mode from a `Read`-only source.
- Review Focus tests owned by this slice.
- Fuzz 120 s.

### Slice 2: hls-runtime

**Files:** `hls-runtime/src/client/engine.rs` (`demux_and_emit`), `hls-runtime/src/server/engine.rs` (`set_init_from`), their tests.

One `Mp4Demux::sequential` per init slot; the init is fed when stored, each segment or part body when fetched. `progressive_movie` refusal kept. Outputs (`Output::Samples`, `FragmentDamaged`, `TrackSkipped`) byte-for-byte as before.

**Gate:** every existing hls-runtime test passes unchanged; a new test proves no buffer of `init.len() + segment.len()` is allocated per segment (`ThreadCapped` with a budget below that sum for a large generated segment); damaged-`moof` segment followed by a good one; multimux's HLS-pull suite and the `mediastreamvalidator` oracle tests pass.

### Slice 3: dash-runtime

**Files:** `dash-runtime/src/client/demux.rs` (`init_info`, `demux_segment`), `dash-runtime/src/ingest/session.rs`, tests; `test-harness/src/dash/loopback.rs` (`oracle`).

Same pattern per representation. `init_info` reads track specs from the stage's events after feeding the init alone.

**Gate:** every existing dash-runtime and broadcast-client DASH test passes unchanged (including the fault matrix and, where available, the livesim2 end-to-end); the allocation test as in slice 2; `sidx`-addressed on-demand content (`fixtures/dash/ondemand-sidx`) still plays.

### Slice 4: smooth-runtime

**Files:** `smooth-runtime/src/client/stream.rs`, `smooth-runtime/src/server/engine.rs`, tests.

The synthetic init is fed once per stream; each fragment after it. `tfxd` rebasing unchanged.

**Gate:** every existing smooth-runtime, broadcast-client Smooth and multimux Smooth-pull test passes unchanged; allocation test.

### Slice 5: file playout and tools

**Files:** `media-plane/src/file_stream.rs`, `media-plane/src/file_source.rs` (`select_demuxer`, `demux_identified`, `probe_and_demux` lose their MP4 arms), `multimux/src/source/file_reader.rs`, `media-doctor/src/container_codec.rs` and `src/bin/media-doctor.rs` (the container check streams from the file through the stage; no `fs::read` on that path), `demo/src/mp4.rs`, READMEs and `CLAUDE.md`.

**Gate:**
- multimux file playout of a generated 8 GiB MP4 (progressive and fragmented), end to end to an LL-HLS output, under `test_alloc` with a 64 MiB cap: the design's Phase 3 gate. Also with looping, two passes, same sample digests, no growth.
- `FileTooLarge` is no longer returned for an MP4 above `max_file_bytes`; it still is for PS and WebM.
- media-doctor on the same generated file under the same cap; its findings on every MP4 fixture equal today's (recorded before the change).
- Existing multimux file-reader tests pass unchanged.

### Slice 6: delete and migrate tests

**Files:** every test and example that names `Fmp4Demux` or `ProgressiveDemux` (transmux `tests/`, in-crate test modules, hls/dash/smooth/broadcast-client/media-plane/multimux/dvr-runtime/playout-runtime/container-probe tests), `transmux/src/cli.rs`, `repackage.rs`, `cenc_decrypt.rs`, `stitch.rs`, `webm_demux.rs` tests, the fuzz target, `workspace-guards` (a new `deleted_names` row or the nearest existing family: the two type names may not reappear), `CLAUDE.md`, READMEs, `bindings/transmux-py` docs.

Tests move onto `test_harness::collect`; transmux's own non-test users move onto the `pub(crate)` collector. Then the old types, functions and fuzz target are deleted.

**Gate:** `rtk proxy git grep -n -E "Fmp4Demux|ProgressiveDemux|demux_progressive|samples_from_stbl"` prints nothing outside CHANGELOGs and `docs/history`; transmux's test pass count before equals after (tests are moved, not dropped: state the two numbers); every golden byte-identical; the guard fails when one of the names is reintroduced (shown by a temporary edit, reverted).

## Order and landing

Slice 1, then slices 2, 3, 4 and 5 in parallel (disjoint crates; slice 5 owns media-plane, multimux, media-doctor, demo), then slice 6. Each slice is one PR, audited on the orchestrator's own gate run.
