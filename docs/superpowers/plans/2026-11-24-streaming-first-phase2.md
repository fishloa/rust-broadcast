# Streaming-first Phase 2 (`isobmff` Reader and Writer) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `isobmff` reads and writes whole MP4 files, progressive and fragmented, without ever holding sample bytes or the file: a sans-IO `Reader` that asks for byte ranges and yields sample descriptors, a sans-IO `Writer` that emits byte ranges, every track surfaced whatever its kind, typed metadata, and registration points for private boxes and tracks.

**Architecture:** The sample-table and fragment addressing rules that live in transmux today move down into `isobmff` and are rewritten once, as lazy cursors over the parsed tables (no expanded per-sample vectors, no whole-file slice). The `Reader` and `Writer` are state machines over those cursors. transmux's whole-file demuxers and muxers call the moved rules in the same slice that moves them, so there is one implementation at every commit; their `Unpackage`/`Package` impls go in Phases 3 and 4.

**Tech Stack:** Rust 1.95, `broadcast-common` (`Parse`/`Serialize`, `wire`, `len`), `thiserror`, optional `serde`. `std` feature for the file driver. tokio only in the adapter crate.

**Spec:** `docs/superpowers/specs/2026-11-24-streaming-first-containers-design.md` (§4.2, §4.3, §4.5, §4.7, §4.8, §4.9, §5 Phase 2, §6).

**Tracking issues:** #4057 (programme), #4056 (metadata tracks).

## State of main this plan starts from

- `isobmff` holds all box syntax. Typed sample-table boxes: `stts`, `ctts`, `cslg`, `elst`, `sidx` (`timing.rs`), `stsc`, `stsz`, `stco`, `co64`, `stss`, `stsd`, `hdlr`, the `moov` tree (`init_segment.rs`), `moof`/`traf`/`tfhd`/`tfdt`/`trun` (`movie_fragment.rs`), `senc`/`saiz`/`saio` (`cenc.rs`), `sbgp`/`sgpd` (`sample_groups.rs`). Not typed: `stz2`, `sdtp`, `udta`, `meta`, `ilst`, `keys`, `chpl`, `tfra`/`mfra`, and every QuickTime-only atom. `udta` and `meta` are preserved opaque.
- Progressive demux: `transmux/src/progressive_demux.rs`. `sample_layout`, `chunk_offsets`, `sample_size`, `expand_stts`, `expand_ctts`, `expand_stss` are pure over `isobmff` types; `samples_from_stbl` slices a whole-file `&[u8]`. `elst` is not read.
- Fragmented demux: `transmux/src/media.rs` (`Fmp4Demux`, `apply_trex_defaults`, `absorb_fragment`, `absorb_traf`) over `isobmff::frag_offsets`, whose functions take a whole-file `&[u8]` and a list of `MdatRange`. `sidx` and `mfra` are never used.
- Progressive write: `transmux/src/progressive.rs` (`build_stts`/`build_ctts`/`build_stss`, the `stsc` run builder, `interleave_order`, `plan_mdat_layout`, `needs_co64`, `mdat_payload_offset`, `track_timings`). Its module doc still says one chunk per track; the code interleaves 500 ms chunks.
- Fragmented write: `transmux/src/pipeline.rs` (`build_init_segment`, `build_media_segment`, `build_chunk`) and `transmux/src/fragment_build.rs` (`media_fragment_run`).
- An unknown sample entry makes transmux skip the track (`SkippedTrack`); `SampleEntryVariant::Unknown(OpaqueBox)` already carries its bytes.
- `broadcast_common::stage::Demand` is `#[non_exhaustive]` with `want_bytes` and `saturated`.

## Decisions (made by the orchestrator; binding for every slice)

1. **Module layout.** New public modules in `isobmff`: `limits`, `sample_table` (cursors and table builders), `reader`, `writer`, `plan`, `registry`, `metadata` (`udta`, `meta`, `ilst`, `keys`, chapters, location), `quicktime` (atoms no ISO file carries), `driver` (feature `std`). Existing modules keep their names.
2. **Offsets are `u64`, counts are `u32`,** as on the wire. No `usize` in a public signature that names a file position or a sample count. Every conversion to `usize` for indexing a supplied buffer is checked.
3. **The sample table is the parsed boxes plus a cursor.** `sample_table::TrackCursor` walks `stts`, `ctts`, `stsc`, `stsz`/`stz2`, `stco`/`co64` and `stss` run by run and yields one `SampleRef` at a time. Nothing expands a table to one entry per sample. `expand_stts`, `expand_ctts`, `expand_stss`, `sample_layout`, `chunk_offsets` and `sample_size` in transmux are deleted in the slice that adds the cursor; `samples_from_stbl` and `cenc_decrypt`'s layout use the cursor. One function per job: no wrapper, alias or copy remains.
4. **Limits.** `isobmff::Limits` is `#[non_exhaustive]`, built with `Limits::default()` and `with_*` setters. Two kinds of field, kept apart in the doc:
   - *Hostile-input caps* bound what a count or size read from the file may make the crate allocate (P4): walk depth, `moov` size, samples per track, single sample size, fragment (`moof`) size, one metadata item. They have named default constants taken from design §4.5, because a crate that allocates whatever a file claims is not safe to hand a download.
   - *Caller policy* is never chosen by the library: the progressive writer's reorder buffer for an append-only sink has no default. The append-only constructor takes the cap as an argument; a seekable sink needs none.
5. **Reader shape.** `Reader::new(ReaderConfig) -> Reader`; `ReaderConfig` is `#[non_exhaustive]` with `limits`, `total_len: Option<u64>`, `order: SampleOrder` (`DecodeTime` default, `File`), `source: Source` (`Seekable` default, `ForwardOnly`), `mode: Mode` (`Strict` default, `Permissive`) and an optional `BoxRegistry`. Driving: `next(&mut self) -> Result<Step<'_>>`, `supply(&mut self, offset: u64, bytes: &[u8]) -> Result<()>`. `Step` is `#[non_exhaustive]`: `Need(ByteRange)`, `Tracks(&[TrackInfo])`, `Metadata(&MovieMetadata)`, `Sample(SampleRef)`, `Fragment(FragmentInfo)`, `Warning(Warning)`, `Done`. A `supply` that does not match the outstanding `Need` is a typed error. The reader asks only for structure; a test counter (feature `scan-counters`) proves no byte inside an `mdat` payload is ever requested.
6. **`SampleRef`** is plain data: `track: TrackId`, `index: u32`, `decode_time: u64` (track timescale, from 0 for progressive, from `tfdt` for fragments), `composition_offset: i32`, `duration: u32`, `sync: bool`, `offset: u64`, `size: u32`, `description_index: u32`, and `aux: Option<SampleAux>` (CENC IV and subsamples from `senc`). Fragment samples borrow nothing from the `moof` buffer.
7. **Edit lists are surfaced, not applied.** `TrackInfo` carries the parsed `elst` entries. The reader reports decode order and decode times exactly as transmux does today, so Phase 3 changes no golden.
8. **Every track is surfaced.** `TrackInfo` has `track_id`, `handler: [u8; 4]`, `timescale`, `duration`, the `tkhd` fields, `references` (`tref`), `edits`, and `entries: Vec<SampleEntryInfo>` where each has `fourcc`, `body` (the raw sample-entry box bytes) and `typed: Option<SampleEntryVariant>`; a registered private entry comes back through the registry. No track is dropped for an unknown handler or fourcc. The first `stsd` entry is no longer the only one read: `description_index` on each sample selects it.
9. **Seeking.** `seek_to(track, decode_time)` lands on the sync sample at or before the time, using `stss` (progressive), `sidx` or `mfra`/`tfra` (fragmented, when present; otherwise a typed `NoIndex` error). `tfra`, `mfra`, `mfro` become typed boxes with `Parse`/`Serialize`.
10. **Plan.** `plan::SamplePlan` is per track: the `TrackInfo` plus the table boxes. It is what `Reader::plan()` returns for a progressive file (no media read) and what `Writer::progressive` consumes. `plan::PlanBuilder` builds one from pushed descriptors (run-length coded as it goes) for callers that have samples and no index. The table builders (`build_stts`, `build_ctts`, `build_stss`, the `stsc` run builder) move from `transmux/src/progressive.rs` into `sample_table` and are called by both `PlanBuilder` and, until Phase 4, `ProgressiveMux`.
11. **Writer shape.** `Writer::fragmented(tracks, limits)`: `init() -> Vec<u8>`, `push(track, SampleMeta, &[u8])`, `flush() -> Result<Fragment>` (one `moof`+`mdat`, capped by `Limits`). `Writer::progressive(plan, Layout)` with `Layout { faststart: bool, sink: Sink }` where `Sink` is `Seekable` or `AppendOnly { reorder_bytes: u64 }`: `header() -> Write`, `push(track, index, &[u8]) -> Result<Writes>`, `finish() -> Result<Writes>`; each `Write` is `{ offset: u64, bytes }`. A pushed sample whose size differs from its plan entry is `Error::PlanMismatch`. There is no single-pass progressive constructor.
12. **Byte identity.** `ProgressiveMux` and `CmafMux` are re-expressed over the `Writer` in the writer slices and every existing golden stays byte-identical: same `ftyp`, same 500 ms interleave, same `stco`/`co64` and `mdat` header choice, same `elst`. The layout rules (`interleave_order`, `plan_mdat_layout`, `needs_co64`, `mdat_payload_offset`) move into `isobmff::writer`; `track_timings` stays in transmux until Phase 4 because it reads the IR. The stale module doc in `progressive.rs` is corrected when it is touched.
13. **Metadata is typed both ways** (P10): `udta` text atoms including `©xyz` (ISO 6709 string kept verbatim, with a parsed latitude/longitude/altitude accessor), 3GPP `loci`, `meta` with `hdlr`, `ilst` in both forms (iTunes four-character atoms with `data` type and locale; `mdta` with a `keys` table), freeform `----` (`mean`/`name`/`data`), `trkn`/`disk` pairs, cover art (`covr`, bounded by `Limits`), and Nero `chpl`. An atom the tables do not know is an opaque item, written back unchanged. Each typed atom cites `private/specs-verified/apple/isobmff/quicktime-file-format-online.md`, `private/specs-verified/3gpp/isobmff/ts-126-244-v19.md` or the ISO transcription; an atom with no source in `private/` stays opaque.
14. **QuickTime is read only.** `quicktime` types parse and are kept so a `.mov` reads completely: version 1 and 2 sound sample descriptions, `wave`, `wide`, `tapt`/`clef`/`prof`/`enof`, `gmhd`/`gmin`, the `tmcd` sample entry and timecode sample, chapter tracks (`tref` `chap`), and a file with a `qt  ` brand or no `ftyp`. They implement `Parse` only and are listed in the `Serialize`-symmetry guard's table with this reason. Writing a track read from a `.mov` either produces its ISO form or fails with `Error::QuickTimeOnly { atom }`.
15. **Permissive mode** tolerates exactly the faults listed in design §4.7 and reports each as `Step::Warning` with a typed `Warning` variant. It never changes a `Limits` check.
16. **`BoxRegistry`** is modelled on `dvb_si::descriptors::DescriptorRegistry`: `register::<T>()` for a type implementing a new `BoxDef` trait (`FOURCC` or `UUID`, optional parent scope) plus `Parse`/`Serialize`; the reader, the box tree and the writer consult it; an unregistered box is `OpaqueBox`.
17. **broadcast-common** gains, additively: `Demand::at(offset: u64, want_bytes: usize)` with a public `offset: Option<u64>` field, and module `capped` (feature `std`): `read_capped(reader, limit, what)` and `read_file_capped(path, limit, what)` returning a typed `TooLarge { what, limit }` without allocating past the limit. No version number changes; both go under `[Unreleased]`.
18. **Drivers.** `isobmff::driver::FileReader<R: Read + Seek>` and `ForwardReader<R: Read>`: an iterator of steps that answers every `Need` itself and a `read_sample(&SampleRef, &mut Vec<u8>)`. A new crate `isobmff-tokio` holds the `AsyncRead + AsyncSeek` form. `broadcast-client` gets the HTTP range form as sans-IO actions (`Action::HttpRange`), coalescing adjacent `Need`s so a remote `moov` costs a bounded number of requests. None offers a read-everything call.
19. **Fixtures** (all in `private/fixtures/mp4/`, PROVENANCE there): `camera/gopro-*.mp4` (five tracks: `avc1`, `mp4a`, `tmcd`, `gpmd`, `fdsc`), `tags/afconvert-aac.m4a`, `tags/lavf-tagged.m4a`, `tags/mp4box-chapters-nero.m4a`, `quicktime/lavf-aac-tmcd.mov`, `quicktime/lavf-pcm24.mov`. Tests load them through `test_fixtures`. An Apple-camera `.mov` with `mebx` metadata tracks and `mdta` keys is wanted and not yet committed; the slice that needs it says so in its PR rather than faking one.

## Global Constraints

- No release, no version number change. Breaking changes go under each crate's CHANGELOG `[Unreleased]`.
- `isobmff` depends on `broadcast-common`, `thiserror` and optional `serde` only. `no_std` + `alloc` except `driver`.
- No sample bytes and no whole file held anywhere in new code. A test that needs a whole small fixture reads it in the test, not through a library call.
- One function per job. Code that moves is deleted where it was, in the same change; no re-export, alias or wrapper.
- Every golden and fixture round-trip in the workspace stays byte-identical.
- No numeric literal other than 0 and 1 outside a constant definition; no `#[allow]`/`#[expect]`; no `.unwrap()`/`.expect()`/`panic!` in non-test code; zero warnings; no comments that narrate code; every public enum gets `name()` + `impl_spec_display!` or a `label` SKIP row; every length written through `fit_*`.
- No spec text in a public file. Cite the private path and the clause or page title.
- Every count or size read from input is checked against `Limits` before anything is allocated from it.
- Guard tables (`workspace-guards/tests/guards/*_data.rs`, `broadcast-common/tests/it/ordering_guard.rs`) are updated in the change that moves or adds the file they name.
- Gate for every slice, run on the lane's own tree before it reports: the six CI gates, per-feature builds of `isobmff`, `tools/check-bare-features.sh`, `cargo test -p workspace-guards`, `python3 tools/check-citations.py`, `python3 tools/check-doc-links.py`, `python3 -m pytest tools/tests -q`, `cargo build -p media-doctor --examples --locked`, side lockfiles (`demo`, `tools/differential-sweep`) resolving under `--locked`, fuzz targets building.

## Review Focus

- A progressive file whose `moov` follows a multi-gigabyte `mdat`: the reader must reach it by header skips alone; over a forward-only source that is a typed error naming the cause. Test in slice 2.
- `stsc` with a `sample_description_index` other than 1, and a `stsd` with two entries: each sample must name its entry. Test in slice 2 with a constructed table and with any fixture that has one.
- A fragment whose `trun` data offsets point into a *later* `mdat`, or an `mdat` with `largesize` or size 0 (to end of file): addressing without a whole-file slice must agree with today's resolver. Differential test in slice 3.
- An append-only progressive write where the input is all video then all audio: the reorder buffer must fail at the caller's cap with a typed error, never grow. Test in slice 7.
- An `ilst` whose item claims a 4 GiB `data` atom, and a `keys` table claiming 2^32 entries: typed error, no allocation. Tests in slice 4.

---

## Slices

Two tracks. The reader track is slices 1, 2, 3, 4, 5 in order. The writer track is slices 6, 7, starting once slice 1 is merged. Slices 8 and 9 follow both. Two lanes never edit the same file: the reader track owns `isobmff/src/{sample_table,reader,metadata,quicktime}` and transmux's demux files; the writer track owns `isobmff/src/{writer,plan}` and transmux's mux files.

### Slice 1: `Limits` and the sample-table cursor

**Problem:** the rules that turn sample tables into per-sample positions live in transmux, expand every table to one entry per sample, and index with `usize` against a whole-file length.

**Files:** create `isobmff/src/limits.rs`, `isobmff/src/sample_table.rs`, `isobmff/tests/it/sample_table.rs`; add typed `stz2` and `sdtp` to `isobmff/src/init_segment.rs` (`StblChild` variants); modify `transmux/src/progressive_demux.rs`, `transmux/src/cenc_decrypt.rs`, `transmux/src/progressive.rs`.

**Produces:**
- `Limits` with `DEFAULT_*` constants and `with_*` setters (Decision 4).
- `sample_table::SampleRef` (Decision 6) and `TrackCursor::new(stbl: &SampleTableBox, track: TrackId, limits: &Limits, file_len: Option<u64>) -> Result<TrackCursor<'_>>`, an `Iterator<Item = Result<SampleRef>>` with `len()`, `seek_sync(decode_time: u64) -> Result<()>` and `peek_decode_time()`.
- `sample_table::{build_stts, build_ctts, build_stss, build_stsc}` moved from `progressive.rs`.

**Gate:**
- Differential: for every MP4 under `fixtures/mp4/` and `private/fixtures/mp4/` with a `stbl` that has samples, the cursor's `(offset, size, decode_time, composition_offset, duration, sync)` sequence equals what `sample_layout` + `expand_*` gave on the parent commit (the test is written against the old functions first and committed, then switched when they are deleted).
- Hostile: `stsz` claiming more samples than `Limits`, `stsc` with non-ascending `first_chunk`, chunk offset past `file_len`, `stts` runs summing to fewer or more samples than `stsz`: each a typed error; `test_alloc::ThreadCapped` proves no allocation proportional to the claimed count.
- `rtk proxy git grep -n "expand_stts\|expand_ctts\|expand_stss\|fn sample_layout\|fn chunk_offsets"` over the tree returns nothing.
- Every transmux golden unchanged.

### Slice 2: `Reader`, progressive files

**Problem:** nothing can read an MP4 without the whole file in memory.

**Files:** create `isobmff/src/reader.rs` (and `reader/` submodules as needed), `isobmff/src/plan.rs` (the read half), `isobmff/tests/it/reader_progressive.rs`, `fuzz/fuzz_targets/isobmff_reader.rs`; add `big_media::write_big_mp4` to `test-harness`.

**Produces:** `ReaderConfig`, `Reader`, `Step`, `ByteRange`, `TrackInfo`, `SampleEntryInfo`, `SampleOrder`, `Source`, `Mode`, `Reader::plan()`, `Reader::seek_to` over `stss` (Decisions 5 to 10). New `Error` variants: `MoovAfterMdatForwardOnly`, `SupplyMismatch { expected: ByteRange, got: ByteRange }`, `LimitExceeded { what, limit }`, `NoIndex`.

**Gate:**
- Every progressive fixture, driven through a test driver that serves `Need`s from the file: the `Sample` steps in `File` order equal slice 1's cursor output per track; in `DecodeTime` order they are non-decreasing in decode time across tracks (compared on a common timescale), ties in track order.
- Differential against `ProgressiveDemux`: same tracks (for those transmux accepts), and the bytes at each `SampleRef` equal the corresponding `Sample.data` before transmux's NAL normalisation.
- `private/fixtures/mp4/camera/gopro-*.mp4`: five tracks reported, `gpmd`, `tmcd` and `fdsc` among them with their raw sample-entry bodies; sample counts per track equal `ffprobe -count_packets` values recorded in the test as constants with the command in a comment.
- `scan-counters`: on `write_big_mp4` output (4 GiB sparse `mdat`, real tables, `moov` last and `moov` first), bytes requested are less than `moov` size plus a named header allowance; `test_alloc::ProcessPeak` stays under a named budget far below the file size.
- `moov` last over `Source::ForwardOnly` is `MoovAfterMdatForwardOnly`; `moov` first over `ForwardOnly` never issues a backward `Need`.
- Hostile sweep: every limit in `Limits` has a test that exceeds it by one.
- Fuzz target builds and runs its smoke budget with the fixtures as corpus.

### Slice 3: `Reader`, fragmented files

**Problem:** fragment addressing takes a whole-file slice; `sidx` and `mfra` are unused.

**Files:** modify `isobmff/src/frag_offsets.rs` (the resolver takes the `moof`, its offset and the following `mdat` header ranges; no file slice; `sample_ranges*` keep one implementation), `isobmff/src/movie_fragment.rs` (`apply_trex_defaults` moves here from `transmux/src/media.rs`), `isobmff/src/timing.rs` (typed `tfra`/`mfra`/`mfro`), `isobmff/src/reader.rs`, `transmux/src/media.rs`, `transmux/src/cenc_decrypt.rs`; create `isobmff/tests/it/reader_fragmented.rs`, `fuzz/fuzz_targets/isobmff_reader_fragmented.rs`.

**Produces:** `Step::Fragment(FragmentInfo { sequence_number, moof: ByteRange, tracks })`, then that fragment's `Sample` steps with `aux` from `senc` (IV size from `tenc`/`seig`), `seek_to` over `sidx` and `tfra`. One `moof` held at a time, capped by `Limits`.

**Gate:**
- Differential against `Fmp4Demux` on every fragmented fixture (including `cenc-layouts/*`, `llcmaf/ll-chunked.mp4`, `fixtures/dash/**`, `fixtures/hls/cmaf-fmp4`): same per-sample ranges, times, flags and CENC entries.
- A generated 4 GiB fragmented file reads with peak memory under a named budget and no `mdat` payload byte requested.
- `seek_to` on `fixtures/dash/ondemand-sidx/*` lands on the fragment `sidx` names; a file with no index returns `NoIndex`.
- transmux goldens unchanged; `rtk proxy git grep -n "fn apply_trex_defaults" -- transmux` returns nothing.

### Slice 4: metadata (#4056)

**Problem:** `udta` and `meta` are opaque; tags, location and chapters cannot be read or written.

**Files:** create `isobmff/src/metadata.rs` (+ submodules `udta`, `ilst`, `keys`, `chapters`, `location`), `isobmff/tests/it/metadata.rs`, `fuzz/fuzz_targets/isobmff_metadata.rs`, `isobmff/examples/read_tags.rs`, `isobmff/examples/write_tags.rs`; modify `isobmff/src/init_segment.rs` (`MovieBox`/`TrackBox` expose typed `udta`/`meta`), `isobmff/src/reader.rs` (`Step::Metadata`).

**Produces:** Decision 13's types with `Parse` and `Serialize`; `MovieMetadata { movie: Metadata, tracks: Vec<(TrackId, Metadata)> }`.

**Gate:**
- `tags/lavf-tagged.m4a`: title, artist, album, track 3 of 12, disc 1 of 2, date, genre, comment and encoder read as typed values equal to constants in the test.
- `tags/afconvert-aac.m4a`: the freeform `iTunSMPB` item reads with its `mean`, `name` and value.
- `tags/mp4box-chapters-nero.m4a`: two chapters at 0 and 1.5 s, first titled `Intro`.
- `quicktime/lavf-aac-tmcd.mov`: the `udta` title reads.
- Byte-exact round trip of the `udta` and `meta` boxes of every fixture above and of the three GoPro files (whose vendor atoms stay opaque items).
- Mutation check recorded in the PR: flipping the `trkn` total or a `keys` index in the fixture bytes changes the parsed value.
- Review Focus hostile tests.
- Both examples registered in `tools/run-examples.sh`.

### Slice 5: QuickTime input and permissive mode

**Problem:** a camera `.mov` carries atoms the ISO parser rejects or drops, and real files carry tolerable faults that fail the whole read.

**Files:** create `isobmff/src/quicktime.rs` (+ submodules), `isobmff/tests/it/quicktime.rs`, `isobmff/tests/it/permissive.rs`; modify `isobmff/src/init_segment.rs` (sound sample description versions 1 and 2 on the audio entries; `gmhd` as a media-information header), `isobmff/src/reader.rs` (`Mode`, `Step::Warning`), the `Serialize`-symmetry guard table.

**Produces:** Decision 14 and 15 types; `Warning` (`#[non_exhaustive]`, one variant per tolerated fault, each carrying the box type and offset).

**Gate:**
- `quicktime/lavf-aac-tmcd.mov`: three tracks; the audio entry is a version 1 description with a `wave` atom whose `esds` yields the same AudioSpecificConfig bytes as `ffprobe -show_entries stream=extradata`; the `tmcd` entry's flags, timescale, frame duration and frame count are typed and the single timecode sample decodes to 01:00:00:00.
- `quicktime/lavf-pcm24.mov`: `in24` entry with its version and the `wave` children typed or opaque, sample count equal to the recorded constant.
- GoPro files: `tmcd` track and `gmhd` typed.
- One test per tolerated fault, each built by damaging a real fixture in the test: strict returns the typed error, permissive returns the `Warning` and the same samples as the undamaged file up to the fault.
- A `Limits` violation fails identically in both modes.
- If an Apple-camera `.mov` is in `private/fixtures/mp4/quicktime/` by then: its `mebx` tracks are reported with their key tables and the `mdta` location key reads. If not, the PR says this is outstanding.

### Slice 6: `Writer`, fragmented

**Problem:** fragment building is private to transmux and bound to its IR.

**Files:** create `isobmff/src/writer.rs` (+ `writer/fragmented.rs`), `isobmff/tests/it/writer_fragmented.rs`, `fuzz/fuzz_targets/isobmff_writer_roundtrip.rs`; modify `transmux/src/fragment_build.rs`, `transmux/src/pipeline.rs`, `transmux/src/media.rs` (`CmafMux`), and the fragment writers that call `build_chunk` (`ll_dash.rs`, `ll_hls.rs`, `segmenter.rs`, `smooth_fragment.rs`, `cli.rs`).

**Produces:** `Writer::fragmented`, `TrackSetup { track_id, handler, timescale, entry: SampleEntryInfo, .. }`, `SampleMeta { duration, composition_offset, sync, aux }`, `Fragment { bytes }`, `Error::FragmentTooLarge`. `media_fragment_run` and the box-level body of `build_chunk` move into the writer; transmux keeps only the IR-to-`SampleMeta` mapping and its `emsg`/`styp` choices.

**Gate:**
- Every fragmented golden in the workspace byte-identical.
- Read a fragmented fixture with the `Reader`, push every sample into the `Writer` with the same fragment boundaries, compare bytes: identical for every fixture whose `moof` child order `child_order_for_rewrite` already reproduces; the rest listed in the test with the differing box named.
- A `gpmd` track (raw sample entry, handler `meta`) from a GoPro file writes into a fragmented file that `ffprobe` lists with five streams and that the `Reader` reads back sample for sample.
- A fragment exceeding `Limits` is `FragmentTooLarge` before the buffer grows past the cap.

### Slice 7: `Writer`, progressive from a plan

**Problem:** the progressive layout is computed from a whole `Media`.

**Files:** create `isobmff/src/writer/progressive.rs`, the write half of `isobmff/src/plan.rs`, `isobmff/tests/it/writer_progressive.rs`; modify `transmux/src/progressive.rs`.

**Produces:** `PlanBuilder`, `Writer::progressive`, `Layout`, `Sink`, `Write`, `Error::{PlanMismatch, ReorderBufferFull { cap }, NoPlan}` (Decisions 10 to 12). `ProgressiveMux::package` builds a plan from its `Media` and drives the writer.

**Gate:**
- Every progressive golden byte-identical, fast start and `mdat`-first.
- `Reader::plan()` of a progressive fixture fed to `Writer::progressive` with the same layout, samples pushed in file order, reproduces `moov` byte for byte on fixtures written by transmux; on third-party fixtures the sample tables are equal box for box and the test names any other difference.
- Seekable sink with samples pushed in reverse order produces the same file.
- Append-only sink: an all-video-then-all-audio push order succeeds when `reorder_bytes` covers the skew and fails with `ReorderBufferFull` one byte below it; `ThreadCapped` proves the buffer never exceeds the cap.
- A `co64` and 64-bit `mdat` layout from a generated plan above 4 GiB, written to a sparse file, reads back through the `Reader`.
- A pushed sample one byte shorter than planned is `PlanMismatch`.

### Slice 8: `BoxRegistry`, the outside-crate example, the whole-file round trip

**Problem:** a third party cannot add a box or a track kind, and nothing proves the reader and writer are inverse on real files.

**Files:** create `isobmff/src/registry.rs`, `isobmff/tests/it/registry.rs`, `isobmff/tests/it/whole_file_round_trip.rs`, and a `publish = false` workspace member `examples/isobmff-private-format` (its own crate, depending only on `isobmff` and `broadcast-common`) that defines one private box and one private metadata sample entry and round-trips a file carrying both; modify `isobmff/src/reader.rs`, `isobmff/src/writer.rs`, `isobmff/README.md` (five-line open-and-iterate example, extension section, migration notes from `mp4` and `mp4parse`), guard tables for the new member.

**Gate:**
- The example crate compiles without any `isobmff` change and its test reads back the typed box and typed entry it wrote.
- Whole-file round trip (read every box and track, write through the `Writer`, compare bytes, strict mode) over `tags/*`, `camera/gopro-*` and every transmux-written progressive and fragmented fixture. A file that cannot be byte-identical (QuickTime-only atoms, a third-party interleave) is listed with the exact reason and is instead compared track by track, sample by sample, and box by box for everything the writer does emit.
- Registering over a box type the crate already types is an error unless override is requested.

### Slice 9: drivers, `Demand` offset, capped read

**Problem:** the sans-IO core needs five lines to use, and the HTTP and tokio users need their adapters.

**Files:** create `isobmff/src/driver.rs`, `isobmff/examples/list_samples.rs`, new crate `isobmff-tokio`, `broadcast-common/src/capped.rs`; modify `broadcast-common/src/stage.rs`, `broadcast-client` (range driver over its HTTP actions), README files, CLAUDE.md crate list, release-lane workflow for the new crate (unused until a release is approved), guard tables.

**Gate:**
- `list_samples` on a GoPro fixture prints the five tracks and their sample counts; it is registered in `tools/run-examples.sh`.
- `FileReader` over the 4 GiB generated files: peak memory under the named budget.
- `ForwardReader` over a pipe (a `Read` that panics on any seek) reads fragmented and `moov`-first files and returns the typed error for `moov`-last.
- `isobmff-tokio`: the same three cases under `tokio::test`.
- broadcast-client: against a local range-serving test server, opening a `moov`-last fixture costs at most a named number of range requests, asserted; a server without range support is a typed error.
- `read_capped` at the limit succeeds, one byte over is `TooLarge`, and `ThreadCapped` proves the over-limit case allocates no more than the limit plus a named slack.

## Order and landing

Slice 1, then slice 2 and slice 6 in parallel, then 3 beside 7, then 4, 5, then 8, then 9. Each slice is one PR, audited on the orchestrator's own gate run and merged before the next slice on its track starts. #4056 closes with slice 8 (read in slices 2 and 4, write in 6 and 7, round trip in 8).
