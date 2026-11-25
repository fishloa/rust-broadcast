# Streaming-first Phase 4 (incremental muxers and protection) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** every muxer, packager, protection step and transform in transmux is a `Stage` fed `(TrackId, Sample)`; no output path needs a whole `Media`; the transmux CLI streams any input to any output; metadata tracks cross containers.

**Architecture:** Samples carry their own protection data; track specs carry the scheme. Fragment muxers wrap `isobmff::writer::FragmentedWriter`; the progressive muxer is fed a sample plan and then samples; manifest packagers render from a running segment summary, never from samples. Transforms that need a whole-input fact take it from a plan, or hold a bounded start window. Each converted type loses its `Package`/`Encrypt`/`Decrypt` impl in the same slice; the traits themselves go in Phase 6.

**Tech Stack:** Rust 1.95, `broadcast-common` (`Stage`, `Demand`), `isobmff` (writers, plan), `bytes`.

**Spec:** `docs/superpowers/specs/2026-11-24-streaming-first-containers-design.md` (§4.3, §4.4, §4.9, §5 Phase 4, §6).

**Tracking issue:** #4057.

## State of main this plan starts from

- Demux: every container has a streaming demux `Stage` emitting `DemuxEvent` (`Mp4Demux`, `StreamingTsDemux`, `StreamingFlvDemux`, `RtpStreamDepacketiser`); PS and WebM still demux whole (Phase 5).
- Already streaming muxers (`Stage<In = (TrackId, Sample)>`): `Segmenter` (CMAF segments), `StreamingTsHlsSegmenter`, `LlHlsSegmenter`, `ll_dash::LlSegmenter`. `Stitcher`, `JumpDetector`, `LiveJoin`, `IdrWait` are already per sample.
- `Package` over a whole `&Media` (`broadcast-common/src/mux.rs`): `CmafMux`, `HlsPackager`, `TsMux`, `StreamingTsMux` (streaming-shaped but `Package`), `FlvMux`, `RtmpMux`, `MkvMux`, `RtpPacketiser`, `TsHlsPackager`, `LlDashPackager`, `DashPackager` (reads per-track total duration and bytes for bandwidth), `SmoothPackager` (its `package_track_fragment` is already per fragment), `ProgressiveMux` (builds a plan from all samples, then `isobmff::writer::progressive`).
- Whole-`Media` transforms: `rebase_to_zero`, `apply_offset`, `insert_discontinuity_gap`, `derive_iframe_track`/`append_iframe_track`, `concat`, `splice_insert`, `snap_to_preceding_sync`, `Media::{select_tracks, select_tracks_by, trim}`. Production use is only the CLI and examples.
- Protection: `CencEncryptor` (`Encrypt` over `&mut Media`, plans IVs and subsamples over every sample, writes `Track.encryption`), `CencDecryptor` (holds the whole file, its own MP4 walk `demux_protected`, `Decrypt` over `&mut Media`), `cenc_segment::protect_init_segment(init, track_id, &TrackEncryption)`. `Track.encryption.samples` is a per-track list parallel to `Track.samples`; `ir::Sample` has no protection field. `isobmff::writer::{SampleAux, SampleMeta.aux}` already write `senc`/`saiz`/`saio` per sample. SAMPLE-AES functions in `sample_aes.rs` are per NAL/frame already.
- `CodecConfig::Data { stream_type, descriptors, carriage }`: TS identity only; refused by every non-TS muxer (`UnmuxableDataTrack` in `pipeline.rs`, `UnsupportedCodec { codec: "Data" }` in DASH/stsd, MKV, RTP; FLV filters it out).
- Production `.package(` callers: transmux `cli.rs` (whole file read with `fs::read`, whole outputs written with `fs::write`), `rtmp.rs` (internal), multimux `push/mod.rs` (`StreamingTsMux`/`TsMux` per drained batch), `push/egress.rs` (a one-sample `Media` per sample), `push/rtmp.rs` (headers `Media`, `flv_frame_payloads`), `output/dash.rs` (trial package and MPD from empty-sample tracks), smooth-runtime server (`package_track_fragment`), test-harness `big_media` (`StreamingTsMux`), fuzz `broadcast_client` (`RtpPacketiser`).
- #953: 6.66 GB peak RSS for a 2 GB CENC fMP4 to TS. No benchmark exists in the repo.

## Decisions (made by the orchestrator; binding for every slice)

1. **Protection rides on the sample.** `ir::Sample` gains `protection: Option<Arc<SampleProtection>>` (IV, subsample map: the same shape as `isobmff::writer::SampleAux`, converted by `From` both ways; no second definition of the fields). `TrackSpec` gains `protection: Option<TrackProtection>` (scheme, `tenc` defaults, pattern, constant IV, key id). `Track.encryption` and `TrackEncryption.samples` are deleted in slice 1; every producer and consumer moves. `Mp4Demux` fills both from `senc`/`sinf`.
2. **One muxer shape.** A muxer is `Stage<In<'a> = MuxInput<'a>, Out = MuxOutput>`. `MuxInput` is `#[non_exhaustive]`: `Track(TrackSpec)` (announce or update), `Sample { track: TrackId, sample: Sample }`, `Discontinuity { track: Option<TrackId> }`, `Flush` (close the current unit now). `MuxOutput` is `#[non_exhaustive]`: `Bytes(Bytes)` (append), `BytesAt { offset: u64, bytes: Bytes }` (patch, progressive only), `Init(Bytes)`, `Segment(SegmentOut)` (bytes plus a `SegmentSummary`: track, sequence, start, duration, bytes, independent), `Text(String)` (playlist or manifest). Converted muxers implement exactly this; segmenters that already are stages keep their own `Out` types only if a mapping into `MuxOutput` exists as `From` (state it per type) and move to `MuxOutput` where that is a rename.
3. **A muxer holds at most one output unit** (a fragment, a TS PES, an FLV tag, a Matroska cluster, an RTP packet group), capped by a `Limits`-style field whose limit is the caller's: hostile-input caps keep the defaults in `isobmff::Limits`; caller policy (interleave window, fragment duration, cluster duration, target segment duration) has no library default and is a constructor argument.
4. **Manifests from summaries.** `DashPackager`, `LlDashPackager`, `SmoothPackager`, `HlsPackager` become renderers over a `SegmentSummaryLog` (per track: run-length coded `(start, duration, bytes, independent)` entries, capped by a caller-supplied count; bandwidth from summed bytes and durations). They take no samples. VOD renders at `finish()`, live per window. multimux `output/dash.rs` stops building empty-sample `Media`.
5. **Progressive needs a plan.** `ProgressiveMux` becomes `ProgressiveMuxStage::new(plan: OwnedPlan, layout)`; the plan comes from `isobmff::plan::PlanBuilder` fed a descriptor-only pass (`PlanPass` stage: a demux stage driven over a replayable input whose samples are reduced to descriptors after transmux's conditioning, so sizes match what the muxer will be fed). The CLI performs the two passes over a seekable input. A non-replayable input (stdin) asking for progressive output is `Error::ProgressiveNeedsReplayableInput` naming fragmented MP4 as the alternative.
6. **Transforms.** `rebase_to_zero` becomes a stage with two constructors: `with_origin(origin)` (exact, from a plan) and `start_window(window: Duration, max_bytes: u64)` (holds output until every announced track produced a sample or the window closes; both values are caller arguments). `apply_offset`, gap insertion, trick-play derivation are per-sample stages with at most one sample of lookahead. `concat`/`splice_insert` become a `SpliceStage` switching source at a splice point, snapping to the preceding sync sample with a plan, or holding at most one GOP (caller cap) without one. The whole-`Media` functions are deleted when their stage lands.
7. **Data tracks cross containers.** `CodecConfig::Data` gains an `identity: DataIdentity` with `#[non_exhaustive]` variants `Ts { stream_type, descriptors, carriage }`, `Mp4 { handler, sample_entry: Bytes }` (raw sample-entry box bytes, the form `isobmff::writer::SampleEntry::Raw` takes), `Matroska { codec_id, codec_private }`. `Mp4Demux` produces `Mp4` identities instead of abandoning a track with an unknown sample entry; muxers accept a `Data` track whose identity matches their container. Carrying a data track to another family needs the caller to supply the target identity (`DataIdentityMap`); without it the muxer refuses with a typed error naming the track. `TrackSpec.es_info_descriptors` stops duplicating `Data.descriptors`.
8. **Payload plug-ins.** `transmux::payload::PayloadRegistry` maps a data-track identity to an object-safe decoder rendering one sample as structured values; the CLI and media-doctor take plug-ins as cargo features (one per plug-in crate). `motion-imagery-metadata` registers ST 0601 under the existing `motion-imagery-metadata` feature.
9. **The CLI streams.** No `fs::read` of media; inputs are driven through the matching demux stage (seekable file or stdin), outputs written as `MuxOutput` arrives (`BytesAt` needs a seekable output file). Every input to every output; progressive output uses the two-pass plan (Decision 5).
10. **Benchmark (#953).** `transmux/tests/bench_953.rs` (soak-gated): generate a 2 GiB CENC fragmented MP4 with `test_harness::big_media` (encrypted with a test key), run the CLI path fMP4 → decrypt → TS, record peak RSS (`test_alloc::ProcessPeak`) and assert below 128 MiB; the PR body records the number.

## Global Constraints

- No release, no version number change; breaking changes under each crate's CHANGELOG `[Unreleased]`.
- Every golden and fixture round-trip stays byte-identical (the muxers are re-expressed, not redesigned; a golden that must change needs the orchestrator's explicit permission and a per-line explanation, as in Phase 3).
- No whole input and no more than one bounded unit held per stage; no library-chosen policy numbers (Decision 3).
- One function per job; converted code is deleted where it was in the same change; no wrapper, alias or re-export.
- Run the FULL test suite (`cargo test -p <crate> --all-features --locked`, every binary) of every crate touched and of every consumer crate named in the slice; report counts before and after.
- Public types stay `Send + Sync` where they were; no fixed `sleep` in tests (multimux guard); no `String`/`Vec` without `alloc::` imports in `no_std` crates; run `--no-default-features` builds after the last edit.
- An existing assertion may change only to the equivalent assertion on the new API; list every changed assertion verbatim before and after in the report.
- Every new or changed fuzz target runs 120 s for real; paste the final line.
- No numeric literal other than 0 and 1 outside a constant definition; no `#[allow]`/`#[expect]`; no `.unwrap()`/`.expect()`/`panic!` in non-test code; zero warnings; no narrating comments; new public enums get `name()` + `impl_spec_display!` or a `label` SKIP row.
- Gate per slice: the six CI gates on every crate touched, per-feature builds, `tools/check-bare-features.sh`, `cargo test -p workspace-guards --test guards`, citations, doc links, `pytest tools/tests`, `cargo build -p media-doctor --examples --locked`, side lockfiles, fuzz `cargo check`.

## Review Focus

- A CENC track whose samples arrive with protection data but whose `TrackSpec` was announced before the `tenc` was known (a `TrackUpdated` after the first samples): the encrypting or decrypting stage must not emit a sample with the wrong key or IV size. Test in slice 6.
- A DASH VOD packager fed one long track and one short track: bandwidth and durations from summaries must equal today's MPD byte for byte. Golden in slice 2.
- Progressive output from a TS input whose AAC is ADTS and whose H.264 is Annex B: plan sizes must equal post-conditioning sizes, or the writer reports `PlanMismatch`. Test in slice 4.
- A data track (GoPro `gpmd`) carried MP4 → TS → MP4 with an identity map: sample bytes identical after the round trip, and the MP4 sample entry restored byte-identically. Test in slice 8.
- A live TS push whose track set changes mid-stream (a PMT update): the TS muxer stage must emit a new PMT and continue, not refuse. Test in slice 3.

---

## Slices

### Slice 1: protection on the sample; data-track identities

**Files:** `transmux/src/ir/{sample,track,codec}.rs`, `transmux/src/mp4_demux.rs`, `transmux/src/cenc_encrypt.rs`, `transmux/src/cenc_decrypt.rs`, `transmux/src/cenc_segment.rs`, `transmux/src/pipeline.rs`, every producer/consumer of `Track.encryption` and `CodecConfig::Data` (ts_demux, ts_mux, pipeline/stsd, dash, mkv_mux, rtp, flv), tests.

Decisions 1 and 7 (IR only; muxers keep refusing `Mp4`/`Matroska` identities until their slice). `Mp4Demux` stops abandoning tracks with unknown sample entries: they arrive as `Data { identity: Mp4 { .. } }`.

**Gate:** every golden byte-identical; CENC fixtures (`fixtures/mp4/cenc-layouts/*`, `cenc*.mp4`) demux with the same per-sample IVs and subsamples as the Phase 3 differential recorded; the GoPro tracks `tmcd`/`gpmd`/`fdsc` now arrive as data tracks with their raw entries; `rtk proxy git grep -n "TrackEncryption.samples\|\.encryption\b" -- transmux/src` shows no use of the deleted field.

### Slice 2: fragment muxers and manifests

**Files:** `transmux/src/media.rs` (`CmafMux`, `HlsPackager`), `transmux/src/segmenter.rs`, `ll_dash.rs`, `ll_dash/mpd.rs`, `dash.rs`, `smooth.rs`, `pipeline.rs`, `repackage.rs`, multimux `output/dash.rs`, smooth-runtime server, tests.

`CmafMux` → `CmafMuxStage` over `FragmentedWriter` (Decision 2); `Segmenter`/`LlSegmenter`/`LlHlsSegmenter` emit `MuxOutput`; manifests from `SegmentSummaryLog` (Decision 4); `Repackager::run` drives stages; `Package` impls of these types deleted. Data tracks with an `Mp4` identity are muxed (raw sample entry).

**Gate:** every CMAF/HLS/DASH/LL-DASH/Smooth golden and every multimux golden byte-identical; MPD and Smooth manifest goldens byte-identical from summaries; a long VOD (generated, 10 000 segments) renders its MPD with memory independent of sample count (`ThreadCapped`); the GoPro file → CMAF keeps all five tracks.

### Slice 3: TS muxer and TS-HLS

**Files:** `transmux/src/ts_mux.rs`, `ts_hls.rs`, multimux `push/{mod,egress,srt,rtsp}.rs`, `test-harness/src/big_media.rs`, tests.

One `TsMuxStage` replaces `TsMux` and `StreamingTsMux` (PSI interval, continuity, PCR as today; track announce/update mid-stream emits a new PMT version); `TsHlsPackager` deleted in favour of `StreamingTsHlsSegmenter` (which emits `MuxOutput`). multimux push paths feed the stage per sample instead of building per-batch or per-sample `Media`.

**Gate:** TS and TS-HLS goldens byte-identical; multimux push suites (SRT, RTSP, RTMP egress) pass; a PMT-update test (Review Focus); peak memory of a 1 GiB generated TS remux under a fixed small budget.

### Slice 4: progressive MP4 from a plan

**Files:** `transmux/src/progressive.rs`, new `transmux/src/plan_pass.rs`, `isobmff` only if a plan API gap is found (report it), tests.

Decision 5. `ProgressiveMux` and its `Package` impl deleted; `track_timings` keeps its edit-list rules, now computed from the plan.

**Gate:** progressive goldens byte-identical (fast start and `mdat`-first); TS → progressive MP4 with ADTS/Annex B (Review Focus); stdin → progressive refused with the typed error; a generated 4 GiB input → progressive output with peak memory bounded by the plan plus one interleave window.

### Slice 5: FLV, RTMP, Matroska, RTP muxers

**Files:** `transmux/src/{flv,rtmp,mkv_mux,rtp}.rs`, multimux `push/rtmp.rs`, `fuzz/fuzz_targets/broadcast_client.rs`, tests.

Each becomes a stage (one tag / one cluster / one packet group held); `flv_frame_payloads(&Media)` and `locate_av_tracks(&Media)` become per-sample helpers; `Package` impls deleted. Matroska muxes `Data` tracks with a `Matroska` identity.

**Gate:** FLV/RTMP/MKV/RTP goldens byte-identical; multimux RTMP push suite passes; the fuzz target builds and runs 120 s.

### Slice 6: protection as stages

**Files:** `transmux/src/cenc_encrypt.rs`, `cenc_decrypt.rs`, `cenc_segment.rs`, `sample_aes.rs`, `transmux/src/cli.rs` (decrypt path), `demo/src/mp4.rs`, examples, tests.

`CencEncryptStage` and `CencDecryptStage` (per sample, `cenc` and `cbcs`, subsample planning per sample, IV generation per track in sequence; CTR range disjointness checked incrementally); `SampleAesStage` for HLS SAMPLE-AES. `CencDecryptor` (whole file, `demux_protected`) is deleted: decryption reads protection from `Mp4Demux`'s samples. `protect_init_segment` takes `TrackProtection`. `Encrypt`/`Decrypt` impls deleted.

**Gate:** every CENC golden and the encrypt→decrypt round trips byte-identical; the late-`tenc` Review Focus test; the ffmpeg/mp4decrypt oracle tests that exist pass; fuzz targets for both stages run 120 s.

### Slice 7: transforms as stages

**Files:** `transmux/src/{rebase,trickplay,splice,repackage,media}.rs`, `transmux/examples/ssai_ad_stitch/`, tests.

Decision 6. The whole-`Media` functions are deleted; `Media::select_tracks*`/`trim` become stage options or test helpers in `test_harness::collect`.

**Gate:** every transform test re-expressed over the stages with the same assertions; splice and trick-play goldens byte-identical; the SSAI example produces identical output.

### Slice 8: the CLI streams; data tracks across containers; plug-ins; #953

**Files:** `transmux/src/cli.rs`, `transmux/src/payload.rs`, `media-doctor` (plug-in features), `tools/differential-sweep` if it calls `cli::run_bytes`, `transmux/tests/bench_953.rs`, `test-harness/src/big_media.rs` (CENC generator), docs.

Decisions 7 (cross-family with `DataIdentityMap`, CLI flag `--data-identity <track>=<spec>`), 8, 9, 10.

**Gate:** every CLI golden byte-identical; every input × output combination the CLI supports has a test over a fixture; stdin → fragmented works and stdin → progressive is refused; GoPro `gpmd` MP4 → TS → MP4 round trip (Review Focus); `media-doctor --plugin motion-imagery-metadata` prints decoded ST 0601 values for the KLV fixture; #953 benchmark under 128 MiB with the measured number in the PR.

## Order and landing

Slice 1 first (IR change every other slice uses). Then slices 2, 3, 5 and 7 in parallel (disjoint files; slice 2 owns `media.rs`/`pipeline.rs`, slice 3 owns `ts_*`, slice 5 owns `flv`/`rtmp`/`mkv`/`rtp`, slice 7 owns `rebase`/`trickplay`/`splice`). Slice 4 after slice 2 (shares `pipeline.rs`). Slice 6 after slice 2. Slice 8 last. Each slice is one PR, audited on the orchestrator's own gate run.
