# Streaming-first containers: bounded memory, one MP4 crate

Status: approved by owner 2026-11-24.
Tracking: #4057 (whole-file inputs), #4056 (MP4 metadata tracks), #953 (closed without a fix; same defect).

## 1. Why

**A big file can kill the process.** `multimux`'s linear-playout `FileReader` (`multimux/src/source/file_reader.rs`) reads the whole media file into memory by design, then demuxes it into a fully materialised `Media`. Even a TS file, whose demuxer already streams, is drained into the IR. A 4 GB file costs at least 8 GB in a server process. `dvr-runtime`'s `recover_spans` reads a whole period file, and the transmux CLI and `media-doctor` read whole inputs. #953 measured 6.66 GB peak RSS for a 2 GB CENC fMP4 to TS (Bento4 `mp4decrypt`: 25 MB; Shaka Packager: 163 MB).

**The philosophy was wrong.** This is a library for streams and segments. Holding an unbounded input in memory in order to parse it is wrong, without exception (owner, 2026-11-24).

**MP4 parsing has the wrong home.** The MP4 box code is mature (owner, 2026-11-24), but it lives inside `transmux`. To parse MP4 today, a consumer must depend on transmux and its 13 required dependencies (dvb-si, rtmp-runtime, cc-data, broadcast-hls, mpeg-ps, rtp-packet, …). Inside the workspace, seven crates walk MP4 boxes by hand (container-probe, dash-runtime, dvr-runtime, hls-runtime, media-plane, media-doctor, test-harness), and `mp4-emsg` is a separate MP4 box crate. Outside it, telemetry-parser maintains a fork of Mozilla's stalled `mp4parse` because nothing better exists. A lean, seekable, maintained MP4 crate fixes our duplication and becomes something other projects would want.

**The aim is to be the default crates for this work** (owner, 2026-11-24). The widely used MP4 crates are abandoned: `mp4` (about 3.9M recent downloads) and Mozilla's `mp4parse` (about 2.2M) both last released in 2023. A maintained crate that reads and writes, seeks, bounds memory and is fuzzed can take that place, but only if outside projects can depend on it without churn (§4.6), it covers what those users do (§4.7), and it is easy to start with (§4.8).

**Drone/camera telemetry** (DJI `djmd`, GoPro `gpmd`, Google `camm`, `tx3g` captions, `©xyz`) is a useful second goal. It needs the same reader to keep metadata tracks instead of dropping them.

## 2. Principles (binding)

- **P1. No unbounded input in memory.** Memory is bounded by configuration, never by input size. Only a *bounded unit* may be held whole: a TS packet, a PES packet or PSI section, an fMP4 fragment (`moof`+`mdat`), an HLS/DASH/Smooth segment, an EBML cluster, or a container index (`moov`, `sidx`, Matroska `Cues`). Each has a configured cap. A file is never a bounded unit.
- **P2. Indexes cost per sample, not per byte.** A progressive MP4's sample table is held (about 16 bytes per sample: two hours of 4K at 60 fps is about 430k samples, about 7 MB). Sample bytes are read only when that sample is being emitted.
- **P3. sans-IO.** Readers and writers never open files or sockets. A seekable reader *asks* for byte ranges; a writer *emits* byte ranges, at an offset when it must patch. Callers do the I/O: a file, tokio, or an HTTP range request. This matches every runtime in the workspace and keeps the core `no_std`+`alloc`.
- **P4. Every length or count read from input is checked against a limit before allocation.** Crafted files (an index claiming 2^32 samples, a 4 GB sample in a 1 KB file) fail with a typed error, never an allocation.
- **P5. One crate per purpose.** One MP4 box implementation in the workspace. Container syntax lives in crates below transmux that depend only on broadcast-common, following broadcast-hls, broadcast-dash and broadcast-smooth.
- **P6. The whole-input API is deleted, not wrapped** (owner chose option B). `Unpackage`/`Package`/`Encrypt`/`Decrypt`, the `Media` and `Track` types, and every `&[u8]`-whole-file demux entry point go. `Media` does not survive as a carrier for one segment either: a type that can hold a segment can hold a file, and no guard can tell the two apart. A stage that must hold a bounded unit buffers it itself, under its own limit (§4.4). A `workspace-guards` family keeps the deleted names gone.
- **P7. Output is byte-identical.** Every existing golden and fixture round-trip produces identical bytes at every phase. The rewrite changes how bytes are reached, not what is produced.
- **P9. A foundation that does not break.** A crate meant for outside adoption depends only on a small frozen core, never on anything that majors for unrelated reasons (§4.6).
- **P8. A fact about the whole input comes from a plan, never from held samples.** Timeline origin, total duration, sample sizes and counts are read from a *sample plan* (§4.4): sample descriptors without sample bytes. A seekable container gives one from its index. A replayable forward-only input gives one from a descriptor-only pass. An input that can give neither gets a bounded start window or a typed refusal, never a silent different answer.
- **P10. Whatever is read can be written.** Every structure, track kind and metadata item the crates read has a write path of the same rank, with a round-trip test (parse, serialize, byte-identical). A read-only capability is a defect unless §4.9 lists it with its reason.
- **P11. Open to private formats.** A third party can add a box, a sample entry, a track kind or a payload format from its own crate, for reading and for writing, without editing ours. What nobody has registered is carried opaque and written back byte-identical; it is never dropped and never an error.

## 3. Goals, non-goals, anti-goals

**Goals**
- Every server path (multimux file playout, DVR recovery, every pull source) runs in configured memory independent of input size.
- Every file-shaped demux path (progressive MP4, fragmented MP4, CENC-protected MP4, MPEG-PS, WebM/Matroska) streams or seeks.
- Every mux path consumes samples incrementally.
- A standalone `isobmff` crate: complete box syntax, sans-IO seekable reader and writer, depends only on broadcast-common, good enough for external projects to adopt.
- MP4 metadata tracks and clip location are surfaced, not dropped (#4056).
- transmux's required dependencies shrink to what its core needs; everything else becomes a feature.
- A frozen `broadcast-core` at 1.0 under `isobmff` and the other wire-format crates, so a consumer of one of them never takes a breaking release caused by an unrelated module (§4.6).
- `isobmff` covers the common outside uses of an MP4 crate: iTunes-style tags, QuickTime `.mov` input, and real-world malformed files in a permissive mode (§4.7).
- First-class streaming adapters for files, tokio and HTTP ranges, so the easy path is also the bounded one (§4.8).
- Extension points for private boxes, tracks and payloads, and read/write symmetry throughout (§4.9).

**Non-goals**
- Decoding codec payloads (still opaque samples).
- Decoding DJI `djmd` or other camera telemetry payloads (a separate owner decision, §10).
- Moving codec *bitstream* parsers (SPS/PPS, AAC AudioSpecificConfig, AC-3, DTS, VVC, AVS3, Dolby Vision, HDR10+) out of transmux. They are container-independent and shared with TS; a future `codec` crate is noted in §10, not done here.
- Performance work beyond bounded memory and a sane number of reads.
- HEIF/AVIF still images (`iloc`/`iinf` item storage). A large area with its own users; noted in §11, not done here.
- A single-pass progressive MP4 from an input that cannot be replayed. That file would need a second layout (64-bit `mdat` placeholder, arrival-order chunks, no fast start) that no golden covers. Fragmented MP4 is the format for that job. The writer refuses with a typed error (§4.3).

**Anti-goals (a failure even if it "works")**
- A "streaming" API that buffers the whole input internally.
- A size cap as the fix. Phase 0's cap is a stop-gap with a deletion phase, nothing more.
- A second MP4 parser anywhere in the workspace, including a re-export shim kept "for compatibility".
- Any change in output bytes.

## 4. Target architecture

### 4.1 Crate map

| Crate | Owns | Depends on |
|---|---|---|
| `broadcast-core` (new, Milestone 2) | The frozen wire-format contract: `Parse`/`Serialize`, the checked length helpers (`len`), the bit reader/writer (`bits`), the shared error kinds, `impl_spec_display!`, `reserved`, `timescale`, `CencScheme`. `no_std`+`alloc`, no dependencies. Released at 1.0 and held there (§4.6). | — |
| `isobmff` (new) | All ISO/IEC 14496-12 box syntax, 14496-15 codec configuration boxes (`avcC`, `hvcC`, `vvcC`, `av1C`, `vpcC`, `dac3`, `dec3`, `dac4`, `dOps`, `dfLa`, `ddts`/`udts`, `mhaC`, `esds`), 14496-30 subtitle entries, 23001-7 CENC boxes (`tenc`, `senc`, `saiz`, `saio`, `pssh`, `sinf`, `schm`), `emsg` (absorbed from `mp4-emsg`, which is deleted), `udta`/`meta`/`©xyz`, iTunes-style `ilst` metadata, QuickTime-specific atoms on read (§4.7), sample groups. The sans-IO `Reader` and `Writer` (§4.2, §4.3). `Limits`. The `std` adapter (§4.8). | broadcast-common in Milestone 1; `broadcast-core` only from Milestone 2 |
| `broadcast-matroska` (new, Phase 5) | EBML + Matroska/WebM syntax, sans-IO reader (Cues-driven seeking) and writer. Absorbs `transmux/src/webm_demux.rs` and `mkv_mux.rs` syntax. | `broadcast-core` |
| `mpeg-ts`, `mpeg-pes`, `mpeg-ps`, `rtmp-runtime` (FLV) | Unchanged ownership. PS gains a streaming front end (Phase 5). | as today |
| `transmux` | The neutral IR (`TrackSpec`, `Sample`, `CodecConfig`, `DemuxEvent`, the sample plan), codec bitstream glue, mapping between container syntax and IR, the streaming demux and mux stages, pipelines, CLI. No `Media`, no `Track`. | broadcast-common, isobmff, mpeg-ts, mpeg-pes required; the rest behind features |
| `broadcast-common` | Everything not in `broadcast-core`: `Stage`, `clock33`, `klv`, `ber`, `xml`, `time`, `net`, `backoff` and the rest. It depends on `broadcast-core` and does not re-export it. `Stage` stays and `Demand` gains an optional absolute offset (§4.4). A `std`-gated capped-read helper (§6). The `mux` traits (`Unpackage`, `Package`, `Encrypt`, `Decrypt`) are deleted. | — |

The seven hand-rolled box walkers move onto `isobmff` (Phase 1).

**What `isobmff` does not own.** Box syntax stops at the box. `avcC`/`hvcC`/`vvcC` carry their parameter sets as typed NAL-unit wrappers with opaque payloads (the wrappers move with the boxes); decoding an SPS or PPS stays in transmux. Modules that today mix a box with a bitstream parser (`ac3`, `dts`, `av1`, `vp9`, `opus`, `flac`, `mpegh`, `dovi`) are split at the box boundary: the box moves, the bitstream parser stays. Codec strings derived from a decoded bitstream stay in transmux. `isobmff` has its own `TrackId` (the 14496-12 `track_ID`) and its own error type built on broadcast-common's shared error kinds; transmux keeps the container-independent IR `TrackId` and converts. PlayReady/Widevine payload handling (`drm.rs`) stays in transmux; only `pssh` box syntax moves.

### 4.2 `isobmff::Reader` (sans-IO, seekable)

A state machine the caller drives. It reads *structure* only: box headers, `moov`, `moof`, `sidx`, `mfra`. Sample bytes never pass through it.

- `Reader::new(limits, total_len: Option<u64>)`.
- `next() -> Step`, where `Step` is one of:
  - `Need { offset, len }`: the caller supplies those bytes with `supply(offset, &[u8])`. Top-level boxes are walked by reading headers only; `mdat` is skipped by its declared size.
  - `Tracks(...)`: every track, including those transmux cannot map, with handler type, sample-entry fourcc, the raw sample-entry body and the parsed typed entry when known (#4056).
  - `Metadata(...)`: `udta`/`meta` items, including the ISO 6709 location (`©xyz`).
  - `Sample { track, decode_time, composition_offset, duration, sync, offset, size, aux }`: a sample *descriptor*. `offset` and `size` say where the bytes are; the caller fetches them itself when it is ready to emit the sample. `aux` carries CENC per-sample data (IV, subsamples) when present.
  - `Fragment { ... }` for fragmented files, emitted per `moof`; `sidx` and `mfra` are used to seek when present.
  - `Done`.
- `seek_to(time)`: jump using `stss`/`sidx`/`tfra`.
- `plan()`: for a progressive file, the whole sample plan (§4.4) straight from the index, with no media read.

**Sample order.** The caller picks one of two orders. *Decode-time order* merges the tracks by decode time, ties broken by track order; this is the default, because every muxer downstream needs interleaved input and a file with all video then all audio must still play. *File order* walks chunks as they lie on disk; it is for scans that do not care about interleave (media-doctor, recovery). On a well-interleaved file the two coincide.

**Seeking is asked for, not assumed.** A `Need` at or ahead of the current position can be met by a forward-only source that discards the gap. Only a `Need` behind the current position requires a seekable source. So a fragmented file, and a progressive file with `moov` first, both read from a pipe. A progressive file with `moov` after `mdat` over a forward-only source is a typed error naming the cause.

Progressive files: the reader finds `moov` wherever it is. When `moov` sits after `mdat`, the header walk reaches it by skipping `mdat` by size; no media bytes are read. Fragmented files: one fragment at a time; memory is one `moof`.

A thin `std` adapter (feature `std`) drives a `Reader` over `Read + Seek` for callers that just have a file. A tokio adapter lives with the tokio users (multimux); an HTTP range adapter lives in broadcast-client.

### 4.3 `isobmff::Writer` (sans-IO)

- **Fragmented / CMAF**: emits the init segment, then one fragment per `flush()` from pushed samples. Memory: one fragment, capped by `Limits`.
- **Progressive**: always written from a sample plan (§4.4). The plan fixes the whole layout before the first byte: chunk interleave, `stco` or `co64`, a 32-bit or 64-bit `mdat` header, `elst` against the earliest track, and the exact `moov`. This is the same information today's `ProgressiveMux` reads from a whole `Media`, so the layout and the bytes are identical, with `moov` first (fast start) or last as configured. Memory: the plan (P2). At this layer the plan is a box-level sample table (sizes, times, sync flags per track); transmux builds it from its IR plan.
  - The writer then takes each sample's bytes and emits `Write { offset, bytes }`. With a seekable sink, samples may arrive in any order; each lands at its planned offset. With an append-only sink, the writer holds samples that arrive ahead of their turn in a reorder buffer capped by `Limits`; overflow is a typed error.
  - Every pushed sample is checked against its plan entry (size and timing). A mismatch means the input changed between passes and is a typed error.
- **No plan, no progressive file.** An input that is neither seekable nor replayable cannot produce a plan. The writer refuses it with a typed error that names fragmented MP4 as the alternative. There is no single-pass progressive mode.

### 4.4 The streaming core in transmux

**One shape.** Every demuxer, muxer, segmenter, packager and protection step is a `Stage`. There is no second, whole-unit API beside it.

- **Demux**: every demuxer is a `Stage` emitting `DemuxEvent` (it already exists for TS and FLV: `TrackAdded` / `TrackUpdated` / `TrackRemoved` / `Sample` / `ClockReference` / `InputDegraded` and the rest). Forward-only byte streams take `&[u8]`.
- **Seekable demux on the push-based `Stage`**: a seekable demuxer's input is an offset-tagged buffer, `(offset, Bytes)`, and it states where it wants its next bytes through `Stage::demand()`. `Demand` gains an optional absolute offset beside `want_bytes` (`Demand` is `#[non_exhaustive]`, so this is additive). One driver loop serves both families: read `demand()`, seek or skip forward if an offset is given, read, `feed`. The stage checks the offset of what it is fed; the wrong range is a typed error. The MP4 stage wraps `isobmff::Reader`, turning the reader's `Need` into `Demand` for structure bytes and each sample descriptor into a `Demand` for the sample's own range.
- **No sample copies**: the driver reads a requested range into an owned buffer and feeds it as `Bytes`. The stage coalesces neighbouring samples into one request, capped by a per-track read-ahead limit, and cuts each `Sample.data` out with `Bytes::slice`. A sample's bytes are read once and never copied on the way to the muxer.
- **Mux**: every muxer takes `(TrackId, Sample)` through `feed`, with `flush()`/`finish()`, and emits output units: append bytes, bytes at an offset, a finished segment, or a playlist/manifest text. The segmenters (`segmenter`, `ts_hls`, `ll_hls`, `ll_dash`) already have this shape. `CmafMux`, `TsMux`/`StreamingTsMux`, `ProgressiveMux`, `DashPackager`, `SmoothPackager`, `HlsPackager`, `RtpPacketiser`, `RtmpMux`, `MkvMux` and `FlvMux` move to it and lose their `Package` impls.
- **Bounded units are buffered inside the stage that needs them.** A fragment muxer must hold one fragment because `trun` precedes `mdat`; it holds it itself, counts it against `Limits`, and fails with a typed error at the cap. Callers that today wrap one segment in a `Media` and call `package()` feed the samples and call `flush()` instead. Callers that today `unpackage()` one fetched segment feed the segment bytes to the demux stage and read events.
- **Manifests work from summaries.** `DashPackager` and `SmoothPackager` never needed samples; they need each track's spec, segment durations and byte counts. The segmenter keeps that running summary (one entry per segment, run-length coded, capped) and the packager renders from it: at `finish()` for VOD, per window for live. `SegmentTimeline` addressing reads the same summary.
- **Per-sample protection data rides on the sample.** Today `Track.encryption` holds a per-track list of per-sample entries. With `Track` gone, the scheme and `tenc` defaults live on `TrackSpec` and each sample carries its own IV and subsample map. Encrypt/decrypt are per-sample stages (CENC `cenc`/`cbcs`, SAMPLE-AES), not whole-`Media` passes.
- **Input-wide lists become events.** `Media.pcr`, `Media.skipped`, `Media.degraded` and `Media.degraded_at` were collections of things the streaming core already reports in-band (`ClockReference`, `TrackAbandoned`, `InputDegraded` with its record). They stay events. Nothing in the library accumulates them, because a damaged multi-GB input would make that list unbounded. A consumer that wants totals counts them. The movie timescale is carried by the track-resolution event and taken by a muxer as configuration.

**The sample plan (P8).** A plan is, per track, the `TrackSpec` plus one descriptor per sample: decode time, composition offset, duration, size, sync flag. No bytes. About 16 bytes per sample.
- A progressive MP4 or a Matroska file with `Cues` yields a plan from its index.
- A replayable forward-only input (a TS, FLV or PS file) yields one from a descriptor-only pass: the same demux stage runs and the driver keeps each sample's descriptor and drops its bytes. The descriptor is taken after transmux's own conditioning (Annex B to length-prefixed, parameter-set handling), so sizes match what the muxer will be given.
- Consumers of the plan: the progressive writer (§4.3), `rebase_to_zero` (the origin is the earliest first sample across tracks), splice and trick-play when they must name an exact sync point ahead of time.

**Transforms.**
- `apply_offset`, discontinuity-gap insertion and trick-play derivation are local: per-sample stages with at most one sample of lookahead (an I-frame's duration is the distance to the next one; the last is settled at `finish()`).
- `rebase_to_zero` needs the origin. With a plan it is exact. On a forward-only input with no plan, the stage holds output until every announced track has produced a sample or the start window (`Limits`) closes, and takes the earliest first sample seen. This generalises what the live path does with its first batch.
- `concat` and `splice_insert` become a stage that switches source at a splice point. Snapping to the preceding sync sample is exact with a plan; without one the stage holds back at most one GOP, capped, as `ssai-runtime` conditions against real boundaries today.
- Goldens are produced from fixture files, which are seekable or replayable, so every golden goes through a plan and keeps its bytes.

**Tests and tooling** that want "all samples of this small fixture" collect `DemuxEvent`s into per-track `Vec`s with a helper in the `publish = false` `test-harness` crate, taken as a path-only dev-dependency. It is never available to non-test code.

### 4.5 Limits and defaults (builder may tune with evidence)

| Limit | Default | Reason |
|---|---|---|
| max box header walk depth | 16 | deepest real nesting is about 8 |
| max `moov` size | 256 MB | about 16M samples; far above any real file |
| max samples per track | 50M | 24 h at 600 Hz audio |
| max single sample size | 64 MB | well above an 8K intra frame |
| max fragment (`moof`+`mdat`) held | 64 MB | above any real CMAF fragment |
| max segment held (HLS/DASH/Smooth pull) | 64 MB | as today's per-source caps |
| max samples in a plan | as max samples per track, per track | the plan is an index (P2) |
| per-track read-ahead (seekable demux) | 4 MB | coalesces sample reads; bounds what one shared buffer pins |
| progressive reorder buffer (append-only sink) | 64 MB | far above the skew of a sanely muxed input |
| start window (origin without a plan) | 10 s of media or 16 MB | above real mux skew at stream start |
| max segments in a manifest summary | 1M | a week of 0.5 s segments |

Every limit is a field of the crate's `Limits` struct, overridable by the caller; no literal appears outside it.

### 4.6 `broadcast-core`: a foundation that does not break

broadcast-common is at major 14 and holds about 35 modules, from the `Parse`/`Serialize` traits to an XML reader, a backoff policy and HTTP dates. Any break in any module is a major, and under epoch purity every crate above it then takes a major-class release. An outside project that adopts `isobmff` would get a breaking bump because an unrelated module changed. No project builds on that.

The MP4 box code uses five things from broadcast-common: `len`, the error kinds, `timescale`, `impl_spec_display!` and `CencScheme`, plus the two traits. That set is small and settled.

- `broadcast-core` holds exactly the wire-format contract listed in §4.1. It has no dependencies and no optional features beyond `std` and `serde`.
- It is released at 1.0 and stays there. An addition is a minor. A break needs an owner decision recorded in this repo and is expected never to happen; a `workspace-guards` check pins its public API (the committed `cargo public-api` listing may only grow).
- Every wire-format crate (`isobmff`, `broadcast-matroska`, `mpeg-ts`, `mpeg-pes`, `mpeg-ps`, `dvb-si` and the rest) depends on `broadcast-core`. Only crates that need the volatile helpers also depend on broadcast-common.
- broadcast-common keeps its name and everything else. Items that moved are deleted from it, not re-exported, and every crate's imports change in the Milestone 2 re-release, which is breaking for all of them already.
- It lands in Milestone 2 because that is the one planned workspace-wide break. `isobmff` 0.1 (Milestone 1) depends on broadcast-common; `isobmff` 0.2 (Milestone 2) depends only on `broadcast-core`. Outreach to outside projects (§11) starts from 0.2.
- `isobmff` then has its own path to 1.0: after the reader, writer and §4.7 coverage have been used by transmux and at least one outside adopter for one release cycle without an API change.

### 4.7 Coverage outside users need

These shape the `isobmff` API, so they are in scope here rather than added later.

- **iTunes-style metadata** (`moov/udta/meta/ilst`): typed read and write of the common atoms (title, artist, album, track and disc numbers, date, genre, cover art, encoder), freeform `----` items, and chapter lists (Nero `chpl` and QuickTime chapter tracks), read and write. Tag reading is the most common reason a project takes an MP4 crate. Cover art is a bounded unit under `Limits`.
- **QuickTime `.mov`**: the atoms camera and editing files carry that ISO files do not: `wave` and version 1/2 sound sample descriptions, `wide`, `mdta`-keyed metadata (`keys`/`ilst`), `tapt`/`clef`, `gmhd` and `tmcd` timecode tracks, a QuickTime-brand `ftyp` or none at all. Read and write (P10): each atom round-trips, and a written `.mov` is checked with `ffprobe` and, on macOS, AVFoundation's own reader.
- **Permissive mode**: `Limits`-style configuration picks strict (today's behaviour: any violation is an error) or permissive. Permissive tolerates the known real-world faults and reports each as a typed warning event instead of failing: an unknown sample entry (kept as an opaque track, which #4056 needs anyway), a box that overruns its parent by trailing bytes, a `wave` atom without `esds`, a zero `mvhd` timescale, a truncated final `mdat`. It never guesses a field value and never relaxes a `Limits` bound. Strict stays the default inside the workspace; byte-identical round-trips are a strict-mode property.
- Each of these gets real fixtures first (camera originals, an iTunes-tagged `.m4a`, a QuickTime `.mov`), in `private/fixtures/` with PROVENANCE, and the spec transcription where one exists (QuickTime File Format is Apple-published; the `ilst` atom set is documented by Apple and by long-standing convention, so each atom is cited to its source or left opaque).

### 4.8 The easy path is the bounded path

sans-IO is right for the core and too much ceremony for a first use. The crates ship drivers so that five lines open a file and iterate samples, and those five lines still stream.

- `isobmff` feature `std`: a driver over `Read + Seek` (and over `Read` alone for forward-readable files), giving an iterator of steps and a `read_sample` that fills a caller buffer.
- A tokio driver over `AsyncRead + AsyncSeek`, in a small adapter crate so `isobmff` itself never depends on tokio.
- An HTTP range driver in broadcast-client, so a remote MP4 can be inspected without downloading it.
- transmux gets the matching one-call drivers for "this file to that output".
- Every driver is covered by the bounded-memory tests of §6. None of them offers a "read it all" call.
- Each crate's README opens with the five-line example, and `isobmff` carries migration notes for users of `mp4` and `mp4parse`.

### 4.9 Extension points and write symmetry

The workspace already has this pattern twice: `dvb_si::DescriptorRegistry` lets a caller register a private descriptor type, and multimux's `SchemeRegistry` lets a third-party crate add an input or output scheme. Containers get the same treatment at three levels.

**1. Private boxes.** `isobmff::BoxRegistry`, modelled on `DescriptorRegistry`: a caller registers a type that implements `Parse` and `Serialize` for a box type (fourcc, or `uuid` plus its 16-byte extended type), optionally scoped to a parent box. The reader and the box tree hand it back typed; the writer serializes it. An unregistered box is an `OpaqueBox` with its bytes, written back as it was. `broadcast-matroska` has the same registry keyed by EBML element ID.

**2. Private tracks.** A track is defined by its handler type and sample entry.
- Read: every track is surfaced (§4.2), whatever its handler or sample-entry fourcc. A registered sample-entry type comes back typed; an unknown one comes back with its fourcc and raw body. Its samples flow like any others.
- Write: the writer accepts a track described by a caller-supplied handler type and sample entry (a registered type or raw fourcc plus body) and muxes its samples, progressive and fragmented. So a DJI `djmd`, GoPro `gpmd`, Google `camm`, ST 0601 KLV or a company's own metadata track can be written into an MP4, not only read from one.
- In transmux's IR this is `CodecConfig::Data`, which exists today for unknown TS streams. It gains the MP4 and Matroska identities (handler, sample-entry fourcc and body; Matroska `CodecID` and `CodecPrivate`) beside the TS one (`stream_type`, descriptors), and it becomes muxable in every container that can carry an arbitrary track: TS (private `stream_type` and registration descriptor, already carried), MP4 and Matroska. The mux paths stop rejecting it. Carrying a data track between container families needs the caller to say what identity it takes in the target; without that the muxer refuses with a typed error naming the track, never drops it.

**3. Payload formats.** A sample's bytes are the plug-in author's own wire format. A payload format is a type implementing `broadcast_core::{Parse, Serialize}` over one sample; nothing else is required to decode or encode it. `motion-imagery-metadata` does this for ST 0601; a `djmd` or `gpmd` crate, ours or anyone's, does the same. Because the traits live in the frozen `broadcast-core` (§4.6), a plug-in crate written against 1.0 keeps working.
- For tools that must name and print a payload they were not compiled against by type (`media-doctor`, the CLI's dump, the browser inspector), a `PayloadRegistry` maps a track identity to an object-safe decoder that renders a sample as structured values. The CLIs take plug-ins as cargo features, each feature registering one crate.
- The normalised telemetry type (position, attitude, gimbal, footprint, time) stays the separate decision of §10: it is one consumer of this extension point, not part of it.

**Symmetry (P10), stated per area:**

| Area | Read | Write |
|---|---|---|
| ISO BMFF boxes, codec configuration, CENC, sample groups | yes | yes (today) |
| Progressive and fragmented files | yes | yes |
| Metadata and private tracks (#4056) | yes | yes |
| `udta`/`meta`, `©xyz`, iTunes tags, chapters | yes | yes |
| QuickTime-specific atoms | yes | yes |
| Private boxes via `BoxRegistry` | yes | yes |
| Matroska/WebM, TS, PS, FLV data tracks | yes | yes, where the container can carry them |
| Permissive-mode repairs | yes | no: a tolerated fault is reported, and the writer emits the corrected, conformant form. Reproducing a malformed file is not a goal. |
| ST 0601 KLV values | yes | yes, as far as the `misb` crate encodes; anything it cannot encode is written from the raw item kept on read |

The round-trip test for a file is: read it, write every track and every box back through the `Writer`, and compare bytes (strict mode). The Phase 2 gate runs it over the camera and tagged fixtures of §4.7.

## 5. Phases

Each phase lands on main green, with byte-identical goldens, before the next starts. Phases 1 and 2 can overlap once Phase 1's crate exists. Phase 5 depends only on Phase 2's patterns and can run beside Phase 4.

**Delete as you convert.** When a demuxer or muxer gains its stage form, its `Unpackage`/`Package` impl is deleted in the same PR series and its callers move with it. Old and new entry points may coexist on main between PRs of one phase. No release ships both for the same container and direction. Phase 6 is then the last sweep, not a big bang.

**Phase 0 – stop the crash (days).**
- multimux `FileReader`: TS and FLV files stream from disk through the existing `StreamingTsDemux` / `StreamingFlvDemux`, paced sample by sample, never into `Media`.
- The whole-file entry point is in `media-plane` (`file_source::probe_and_demux(&[u8])`), not only in multimux. Phase 0 gives `media-plane` a streaming file session for TS and FLV; probing reads a bounded prefix through `container-probe`'s read-more loop.
- For MP4, PS and WebM files, `FileReader` keeps refusing inputs above `FileReaderConfig::max_file_bytes` with the existing `FileTooLarge` error. That field already exists (default 1 GiB) and is reused: no new field, no breaking release. From Phase 0 it no longer applies to TS and FLV, which stream uncapped; its doc says so. The cap stops applying to MP4 in Phase 3 and to PS and WebM in Phase 5, and the field and its builder are deleted in Phase 6 (a breaking change, taken in the second milestone).
- `dvr-runtime::recover_spans` scans the period file box by box from disk.
- The peak-memory test harness used by every later phase (§6) lands here.
- Ships as patch/minor releases of multimux, media-plane and dvr-runtime.

**Phase 1 – extract `isobmff` (mature code moves, does not change).**
- "Does not change" means wire bytes and behaviour. Signatures change where the move forces it: the new crate has its own error type and `TrackId`, and types move to a new path. Nothing else is redesigned in this phase.
- The split follows §4.1: box syntax moves; SPS/PPS and other bitstream decoding stays; mixed modules are cut at the box boundary. `TrackEncryption`'s box-level parts (`tenc`, `senc` entries) are `isobmff` types; transmux maps them to its IR.
- Move the box syntax, codec configuration boxes, CENC boxes, subtitle entries and sample groups out of transmux; absorb `mp4-emsg` (its history moves to `docs/history/mp4-emsg/`, as `rtcp-packet` did).
- transmux and every consumer import from `isobmff`; no re-export shim.
- Move the seven hand-rolled box walkers onto it.
- Gate: byte-identical goldens; every existing box test passes in its new home; CRATE-ACCEPTANCE for the new crate.

**Phase 2 – the sans-IO `Reader` and `Writer`.**
- §4.2 and §4.3, including metadata tracks and `©xyz` (#4056) and `Limits`.
- broadcast-common minor: `Demand`'s optional offset and the capped-read helper (§6). Both additive.
- Fuzz targets for the reader (progressive and fragmented) and writer round-trip.
- The §4.7 coverage (tags, `.mov`, permissive mode) and the §4.8 drivers, each behind its real fixtures.
- The §4.9 extension points: `BoxRegistry`, typed or opaque sample entries on read and write, and a worked third-party example crate under `examples/` that adds one private box and one private metadata track from outside `isobmff`.
- Gate addition: whole-file round-trip (read, write back, byte-identical) over every §4.7 fixture, including a file with a metadata track.
- Gate: peak-memory tests over generated multi-GB files; hostile-count/size tests; a remote-fixture test over an HTTP range adapter that bounds the number of requests.

**Phase 3 – streaming MP4 demux in transmux and its consumers.**
- MP4 (progressive and fragmented) demux becomes a `Stage` over the `Reader` (§4.4). `Fmp4Demux` and `ProgressiveDemux` lose `Unpackage`; `ProgressiveDemux`'s accumulate-everything `Stage` impl is deleted.
- Every caller that demuxes one fetched fMP4 segment (hls-runtime, dash-runtime, smooth-runtime, broadcast-client, multimux pulls) feeds the stage and reads events.
- multimux file playout streams MP4 (the cap no longer applies to MP4). media-doctor streams from file.
- The transmux CLI moves in Phase 4, with its muxers. Converting it here would need an interim cap for outputs whose muxer is not yet incremental.
- Gate: multimux file playout of a generated 8 GB MP4 under a 64 MB cap, end to end.

**Phase 4 – incremental muxers and protection.**
- Every muxer and packager listed in §4.4 becomes a stage and loses `Package`; its callers in all consumer crates move in the same series, split by muxer and consumer cluster.
- The sample plan, the planned progressive writer (fast start and `mdat`-first), and the transforms of §4.4.
- CENC encrypt/decrypt and SAMPLE-AES as per-sample stages; per-sample protection data moves onto `Sample`.
- `CodecConfig::Data` gains its MP4 and Matroska identities and becomes muxable (§4.9): a metadata track is carried TS to MP4 and MP4 to TS when the caller names the target identity. `PayloadRegistry` and the CLI plug-in features.
- The transmux CLI streams every input to every output. A progressive MP4 output from a non-replayable input (stdin) is refused with the typed error of §4.3.
- Gate: the #953 benchmark re-run (2 GB CENC fMP4 to TS), peak RSS recorded in the PR and kept below 128 MB.

**Phase 5 – remaining file containers.**
- MPEG-PS streaming demux on `mpeg-ps`.
- `broadcast-matroska` with Cues-driven seeking; WebM demux and MKV mux move onto it.
- Phase 0's PS and WebM caps deleted.

**Phase 6 – delete and lock.**
- Delete what is left: the four `mux` traits in broadcast-common, `Media`, `Track`, the whole-`Media` transform functions, `FileReaderConfig::max_file_bytes`, and any remaining whole-input entry point. Most callers already moved in Phases 3 to 5; this phase moves the residue across the 16 consuming crates and their tests (onto the `test-harness` collector).
- Every binary and tool that takes a media file streams it: `dvb-tools`, `ts-fix`, `media-doctor`, `compliance-probe`, the `tools/` binaries.
- transmux dependency diet: dvb-si, rtmp-runtime, cc-data, broadcast-hls, rtp-packet, mpeg-ps and mp4-emsg's successor become features where they are not core.
- New `workspace-guards` family `bounded_input` (§6).
- `broadcast-core` 1.0 is split out of broadcast-common (§4.6) and every wire-format crate moves onto it, in the same workspace-wide re-release.

**Phase 7 – follow-ons (separate decisions, §10).**
- Telemetry normalisation in `motion-imagery-metadata`.
- Offer `isobmff` to telemetry-parser in place of `mp4parse-patched`, with the PRs already scoped (move `argh` out of library dependencies; per-format features; a sample-fed decoder entry point).

## 6. Verification

Every phase, run by Claude on its own gate run, never on a delegate's word:
- **Byte identity**: every golden and fixture round-trip in the workspace, unchanged.
- **Bounded memory**: `test_alloc::ThreadCapped` tests that demux and remux generated multi-GB inputs (sparse `mdat`, real index tables; built at test time, never committed) for every container and every mux output, with a budget far below the input size (64 MB). Each one fails on today's main.
- **Hostile input**: extend `transmux/tests/hostile_count_sweep.rs` and `hostile_input_bounds.rs` to the new readers; every claimed count/size above `Limits` is a typed error with no allocation.
- **Fuzz**: every new reader and writer gets a target; the 10 s PR smoke and the nightly 60 s budget cover them.
- **End to end**: multimux file playout of a generated 8 GB file under the cap (Phase 3); the #953 benchmark (Phase 4).
- **Guard**: `bounded_input` (Phase 6). It bans `fs::read`, `fs::read_to_string`, `read_to_end`, `read_to_string`, their `tokio::fs` forms, and the deleted trait and type names (`Unpackage`, `Package`, `Encrypt`, `Decrypt`, `transmux::Media`, `transmux::Track`, matched as whole paths) in the `src/` and `examples/` of every member. Media input has no exception: it streams. A small bounded document (JSON config, `.m3u8`, SDP, a key file, one wire structure in an example) goes through the one capped-read helper in broadcast-common (`std` feature): a reader form and a path form, each taking a named limit and returning a typed too-large error without allocating past the limit. Async callers run it under `spawn_blocking`; there is no async twin. The helper's own body is the guard's single allowlisted site. Examples are in scope because they are what users copy. The guard is lexical: it cannot see an HTTP body collected by a client library, so per-source segment caps and review remain the control there.
- The 6-gate suite, zero warnings, no magic numbers outside `#[cfg(test)]`.

## 7. Releases

- Phase 0 ships at once as multimux/media-plane/dvr-runtime patch or minor releases.
- **Milestone 1 (Phases 1–3)**: `isobmff` 0.1 (new crate), broadcast-common minor (additive), transmux major-class (MP4 types move crates, whole-input MP4 demux deleted), consumers bumped under epoch purity. `mp4-emsg` gets no further release; its README points to `isobmff`, and every crate that used it moves in this wave so no graph holds both.
- **Milestone 2 (Phases 4–6)**: `broadcast-core` 1.0 (new, published first of all), broadcast-common major (mux traits deleted, core items moved out), transmux major-class (`Media`, `Track` and every `Package` impl deleted), every workspace crate re-released on the new broadcast-common, `broadcast-matroska` 0.1. broadcast-common is published first and goes live before anything else, so no graph ever holds two of it. Any other pending broadcast-common break rides this major rather than causing another.
- Two breaking waves of transmux, not more: each one re-releases its 16 consumers.
- **Python bindings** (outside the workspace, they build against published crates, so they follow each milestone after it is live):
  - `transmux-py`: `demux_ts(bytes) -> dict` is removed, not kept. It is replaced by a `Demuxer` class with `feed(bytes)` returning the events so far and `finish()`, the shape `dvb-si-py`'s `Demux` already has, plus a generator that reads a Python file object in chunks and yields track and sample events. A breaking release on PyPI after Milestone 2. It is not frozen on an old transmux: a frozen binding keeps publishing the whole-input philosophy and pins users to a dead line.
  - `dvb-si-py`: no API change. It re-pins to the new dvb-si line after Milestone 2 and ships a patch.
- Every release follows RELEASE-DOCS and RELEASE-AUDIT. No publish without owner OK.

## 8. Risks

- **Size**: transmux is 121k lines; ~15k are MP4 box code. Phase 1 is a move, but it touches every import path. Done crate-cluster by crate-cluster.
- **Workspace-wide type change**: deleting `Media`/`Track`/`Package` outright, bounded-unit callers included, touches every segmenter, packager and 16 crates. This is several times the work of keeping `Media` for segments and is accepted (§10). It is contained by "delete as you convert" (§5): one muxer and one consumer cluster per lane, never one lane for the type.
- **broadcast-common major**: deleting four traits re-releases the whole workspace. It happens once, in Milestone 2.
- **Two-pass cost**: a plan from a forward-only file demuxes it twice. Only outputs that need a whole-input fact pay it (progressive MP4, exact rebase or splice). Pass one keeps no sample bytes.
- **Input changes between passes**: caught by the per-sample plan check (§4.3), a typed error rather than a corrupt file.
- **Interleave tie-breaks**: streaming muxers must merge tracks with the same key today's whole-`Media` muxers use, or bytes drift. The demux order (decode time, then track order) is fixed in §4.2 and byte identity is the gate.
- **Conflicts**: the running magic-number lanes edit transmux. Phase 1 starts after they land.
- **Seek count**: decode-time order over a file that is not interleaved alternates between tracks. Reads are coalesced per track up to the read-ahead limit, and the HTTP range test bounds request counts on both an interleaved and a non-interleaved fixture.
- **Golden drift**: a move that silently changes bytes. Byte identity is the gate at every phase.
- **Dev-dependency cycles** when tests move with the box code: cross-crate tests live in the highest crate.

## 9. Left to the builder

- Exact type and method names, module layout inside `isobmff`, and how the `Step` enum is split.
- Whether the `std` adapter is a function or a type.
- The names of the offset-tagged input type, the `Demand` offset field and its constructor, the plan types, and the capped-read helper.
- Whether the low-level per-fragment box builder (`build_media_segment` over borrowed sample slices) stays public or becomes private to the fragment muxer.
- The internal representation of the sample table (P2's memory bound is the requirement).
- Which consumer cluster migrates first within Phase 6.
- Tuning the §4.5 defaults with evidence.

## 10. Decisions recorded

Owner (2026-11-24):
- Whole-input API deleted, not wrapped (option B).
- `isobmff` owns codec configuration boxes too.
- The box code is mature; Phase 1 moves it rather than rewriting it.

Architect, on the builder's review questions (2026-11-24):
- **Q1, byte identity against a streaming progressive MP4.** The progressive writer always works from a sample plan, which fixes the same layout today's muxer computes from a whole `Media`, so bytes are identical and fast start works from any replayable input, TS included. The proposed single-pass mode for non-replayable input is rejected: it is a second layout with no golden, and fragmented MP4 already serves that case; the writer refuses with a typed error.
- **Q2, the fate of `Media`.** `Media` and `Track` are deleted entirely, bounded units included. A type that can carry a segment can carry a file and no guard can tell them apart, so keeping it would leave the whole-input API one helper away; a stage that needs a bounded unit holds it itself under `Limits`, which is also where the cap can be enforced. The larger scope is accepted and contained by deleting each impl as its stage lands.
- **Q3, whole-timeline facts.** They come from the sample plan (P8): the index for seekable inputs, a descriptor-only pass for replayable ones, a bounded start window or one-GOP hold otherwise. `Media.pcr`/`skipped`/`degraded` stay the in-band events they already are; the proposal to report them at `finish()` is rejected because accumulating them is itself unbounded.
- **Q4, guard width.** Media input is never whole-read, in `src/` or `examples/` of any member. Small bounded documents go through one capped-read helper in broadcast-common, called under `spawn_blocking` from async code, and its body is the guard's only allowlisted site.
- **Q5, Python bindings.** `transmux-py` is redesigned as a feed/iterate demuxer and `demux_ts(bytes)` is removed in a breaking PyPI release after Milestone 2; freezing it would keep the wrong philosophy published. `dvb-si-py` only re-pins.
- **Q6, what "unchanged" means in Phase 1.** Bytes and behaviour, not signatures. `isobmff` gets its own error type and `TrackId`; parameter sets stay opaque typed NAL wrappers with bitstream decoding left in transmux; mixed box/bitstream modules are split at the box; box-level encryption data moves and transmux maps it; no shims or re-exports.
- **Q7, Phase 0 cap.** Reuse `FileReaderConfig::max_file_bytes` for MP4/PS/WebM only, TS and FLV stream uncapped, no new field and no breaking release. The field is deleted in Phase 6.

Architect, further changes made in this revision:
- `isobmff::Reader` never handles sample bytes; it yields descriptors and the caller fetches ranges. This removes a copy and keeps `bytes` out of `isobmff`.
- A seekable demux fits the push-based `Stage` through an offset-tagged input and an optional offset on `Demand`, so one driver loop serves streams and files, and forward skips work on pipes.
- Decode-time order is the reader's default, so a non-interleaved file still feeds muxers correctly.
- The transmux CLI converts in Phase 4, not Phase 3, to avoid an interim cap.
- Phase 0 covers `media-plane`'s whole-file entry point, which the first draft missed.
- Per-sample protection data moves from `Track` onto `Sample`.

Separate owner decisions, not blocking this design:
- Crate names `isobmff` and `broadcast-matroska` (both free on crates.io) are assumed throughout; a name is permanent once published, so it is confirmed with the Milestone 1 release sign-off.
- Telemetry: normalised type in `motion-imagery-metadata`; DJI `djmd` decoding (no published spec); telemetry-parser outreach (§5 Phase 7).
- A future `codec` crate for the container-independent bitstream parsers.

### Owner approvals (2026-11-24)

- The design as written, including every decision above.
- Progressive MP4 from non-replayable input is refused with a typed error pointing to fragmented MP4. Single-pass progressive output can be added later as its own tested feature.
- `examples/` are under the `bounded_input` guard.
- The capped-read helper lives in broadcast-common behind `std`.
- The broadcast-common major and the workspace re-release at Milestone 2.
- Crate names `isobmff` and `broadcast-matroska`.
- People must be able to plug in special or private parsers (the drone metadata case), and writing is supported as much as reading: P10, P11 and §4.9.
- The aim is to be the default crates for this work. Folded in: a frozen `broadcast-core` 1.0 (§4.6), tags, `.mov` input and permissive mode in `isobmff` (§4.7), streaming drivers (§4.8), and the adoption programme (§11). The name `broadcast-core` is free on crates.io and is confirmed at the Milestone 2 sign-off.
- The §4.5 defaults are starting estimates, confirmed by measurement in Phase 2.

## 11. Adoption programme (starts when Milestone 1 ships)

Not design work, but what turns good crates into the default ones. Tracked as issues, run alongside Phases 4 to 6.

- **Public proof.** Differential runs against ffmpeg, Bento4 and Shaka Packager over public corpora (ffmpeg FATE samples, the GPAC test suite), with results published in the repo. OSS-Fuzz enrolment for `isobmff`, `mpeg-ts` and `dvb-si`. Published benchmarks: peak memory and throughput against Bento4 and Shaka on the #953 input.
- **Tools people install.** Prebuilt binaries and a Homebrew formula for `media-doctor` and the `transmux` CLI, built by CI on tag. An `mp4dump`-style box inspector subcommand. A browser (WASM) inspector on the Pages site: drop a file, see its boxes, nothing uploaded.
- **Project signals.** A CONTRIBUTING guide that says which tests run without the private fixtures (the public tier must be enough for an outside PR). A public roadmap. A docs site with a cookbook. Whether the repo moves to a GitHub organisation is an owner decision.
- **Adopters.** Upstream PRs once `isobmff` 0.2 is live: telemetry-parser/Gyroflow (replace `mp4parse-patched`), Rerun (`re_mp4` exists because `mp4` died), `retina`. One well-known user is worth more than any README.
- **Later coverage.** HEIF/AVIF items. A `codec` crate for the container-independent bitstream parsers. Single-pass progressive output as its own tested feature.
