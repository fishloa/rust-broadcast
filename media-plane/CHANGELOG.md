# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- New crate: `media-plane`, the media-plane integration layer (plan step 3a-i).
- `ByteStage`: the pre-demux byte-to-byte stage contract, defined as
  `Stage<In<'a> = &'a [u8], Out = Bytes>` rather than a second drive trait
  (per the 2026-07-27 revision of `docs/superpowers/specs/2026-07-26-media-plane-architecture.md`
  §1.1). Validated to compile at MSRV 1.86.
- `no_std` + `alloc` crate skeleton with a `std` feature, mirroring `transmux`.
- `ByteTap` (plan step 3a-ii): a positional, non-blocking observer of bytes
  in the byte layer, yielding `(Bytes, Timestamp)` exactly as received —
  including bytes a demuxer will reject (bad sync byte, TEI set, bad CRC,
  unaligned framing) — via a bounded ring. The producer (`record`) never
  blocks and never grows the ring past its configured capacity; a slow
  consumer instead observes an in-band `TapItem::Lagged { skipped }` from
  `poll`, which cannot be missed the way a side-channel counter could be.
  `TapPoint::{Wire, PostTransform}` is descriptive metadata only. Not a
  `Stage` — it is fed and polled by different callers, not driven as one
  contract.
- `ByteMerge` (plan step 3a-ii): the one bounded multi-input primitive in the
  byte layer — `N` byte sources reduced to one output stream of discrete
  messages (never an undelimited byte soup). `MergePolicy::FirstArrival`
  interleaves every source in arrival order; `MergePolicy::Failover` prefers
  a primary source, switches to a secondary after a configurable silence
  timeout (reset by any primary message, so a single late arrival does not
  cause a spurious switch), and switches back to primary the instant it is
  heard from again. `MergePolicy` is `#[non_exhaustive]`; ST 2022-7 hitless
  switching (`Hitless2022_7`) is deliberately **absent, not stubbed** — it
  needs RTP sequence-number semantics this layer does not have and lands
  with #752. Per-source state and the output queue are both bounded
  independently of call volume (`MergeError::QueueFull` rejects outright
  once the queue is at its cap, rather than growing or evicting silently).
- `Trunk`, `TrunkWriter`, `SampleCursor` (plan step 3b-i, `std`-only): the
  sample path of the `Trunk` — the bounded sample ring, its single writer,
  and the cursor(s) that read it. Two independent retention classes,
  `RetentionClass::Timed` (regular-cadence media, ordinary count-bounded
  eviction) and `RetentionClass::Sparse` (irregular, semantically-critical
  entries such as SCTE-35 cues), each with its **own** capacity so a flood of
  `Timed` publishes can never evict a still-live `Sparse` entry. A `Trunk`
  has exactly one `TrunkWriter` (`Trunk::writer()` returns `None` on every
  call after the first); `TrunkWriter::publish` never blocks or rejects — a
  full ring evicts its oldest entry rather than waiting on a slow reader.
  `Trunk::subscribe()` returns a `SampleCursor` starting from "now"; loss is
  reported in-band via `SampleCursorItem::Lagged` (ordinary, `Timed`) or
  `SampleCursorItem::Degraded` (escalated, `Sparse` — the consumer's derived
  state is now wrong, not merely gapped), following `ByteTap`'s `TapItem`
  precedent so loss can never be skipped past via a side channel. Adds a
  `transmux` (`Sample`) dependency, `std`-gated: `spikes/trunk-bench`
  (spec §3.1) showed writer cost is O(N) in cursor count, so `subscribe()`
  documents — at the call site, not only in the module docs — that a cursor
  is for one distinct consumer of the stream, never one per peer of a
  one-to-many protocol; supported reader count is single-digit by design.
  Payload fan-out is a `Bytes` refcount bump on the production path (no
  `.slice()`/copy anywhere in this module), verified by pointer-identity
  assertion, not content equality.
