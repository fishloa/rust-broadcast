# transmux 0.11.0 — 2026-07-04

Completes broadcast-audio and arbitrary-stream carriage on the MPEG-2 TS side of
the **any-to-any container hub**, and reworks the TS demuxer into a single
event-driven streaming core. `transmux` still parses only codec *config* headers
— it never en/decodes; coded samples stay opaque. `no_std` + `alloc`.
Independently versioned.

## Streaming demux core

- **`StreamingTsDemux`** (#555) — a new incremental MPEG-2 TS demux core. `feed()`
  accepts bytes of any size or alignment (down to one byte at a time;
  resynchronises via `mpeg_ts::resync::TsResync`), `poll_event()` drains
  `DemuxEvent`s (`TrackAdded`/`TrackUpdated`/`Sample`/`Pcr`/`Discontinuity`) as
  they become known, and `finish()` flushes trailing partial access units. The
  whole-buffer `TsDemux` is now a thin batch wrapper over it — one code path, no
  separate implementation. Memory is bounded independent of stream length (incl.
  a FIFO-capped pre-PMT buffer for a full-multiplex feed with never-claimed PIDs).

## Per-sample timing & audio splitting

- **Per-sample `SourceTiming`** (#556) — every video/audio `Sample` carries the
  33-bit-unwrapped 90 kHz PES-clock PTS/DTS of the frame it came from (duration
  sums can't reproduce PES timing: the 44.1 kHz cadence never divides the 90 kHz
  clock). AC-3/E-AC-3 PES access units are split into individual syncframes
  (dependent-substream-aware) so real per-frame durations and exact PES-boundary
  timestamps survive into the IR.
- **Opaque data tracks + PCR timeline** (#557) — PMT `stream_type` `0x06`/`0x15`
  surface as opaque `CodecConfig::Data` tracks (descriptors preserved); the full
  27 MHz PCR timeline is collected into `Media::pcr`.

## DTS Transport-Stream spoke (#560)

- **TS → IR**: `stream_type` `0x82`/`0x85`/`0x8A` now resolves a `CodecConfig::Dts`
  track (previously dropped). `dts::DtsCoreFrameInfo::from_es` parses the DTS
  Coherent Acoustics **core substream** frame header (ETSI TS 102 114 §5.3/§5.4,
  sync `0x7FFE8001` — rate/channels/samples-per-frame), `into_ddts()` builds a
  core-only `ddts` box (§E.2.2.3, Tables E-2/E-3), and each PES AU is split into
  core frames with interpolated per-frame `SourceTiming`.
- **IR → TS**: `EsKind::Dts` (`stream_type` `0x82`) emits DTS back to TS.
- (DTS fMP4↔IR and DASH `CODECS=` already shipped; this completes the spoke.)
- MPEG-H TS carriage remains deferred (fixture-blocked; the MPEG-H fMP4 path
  already ships).

## Lossless carriage of any TS elementary stream (#576)

- TS→IR→TS is no longer limited to a hardcoded `stream_type` allowlist — **every**
  PMT elementary stream is carried. `CodecConfig::Data` gains a
  `carriage: DataCarriage` (`Pes`/`Sections`): section-carried streams (`0x05`
  private_sections, `0x0A`–`0x0D`/`0x14` DSM-CC, `0x86` SCTE-35) are reassembled
  via `SectionReassembler` (one section = one `Sample`), everything else via PES.
- **IR → TS**: `EsKind::Data` re-emits the preserved `stream_type` (PES via
  `private_stream_1` `0xBD`, sections via `SectionPacketizer`), and the PMT now
  writes each ES's preserved `ES_info` descriptors so carried streams stay
  identifiable. **Classic TS-HLS (`TsHlsPackager`) carries all of it for free** —
  every `.ts` segment's PMT lists every stream + descriptors.
- **fMP4/CMAF** has no box for these opaque streams, so the fMP4 mux now **skips**
  `Data` tracks gracefully (was a hard `UnsupportedCodec` error) — a TS→fMP4 of a
  mixed multiplex succeeds with its carriable A/V tracks.
- Fixed a latent `ts_mux` packet-interleave bug: the sort keyed on the
  33-bit-wrapped DTS, which could reorder a track's own packets past the wrap
  (reachable once opaque Data tracks with untrusted recovered durations flow
  through). The interleave key is now a separate non-wrapping monotonic value.

## Compatibility

Two source-breaking changes (0.x minor bump per Cargo SemVer): `CodecConfig::Data`
gained a `carriage` field and `Sample` gained `source_timing` — external
match/construct sites must update (use the `Sample::from_*` builders +
`with_source_timing`). The new `DataCarriage` enum is `#[non_exhaustive]`.
Requires broadcast-common ≥ 8.4. MSRV 1.86 (edition 2024).
