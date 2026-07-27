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
