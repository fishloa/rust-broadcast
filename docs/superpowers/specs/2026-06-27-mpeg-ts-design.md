# mpeg-ts — design

**Status:** approved-pending-review design
**Date:** 2026-06-27
**Crate:** `mpeg-ts` (name free on crates.io)
**Version:** new crate, start **0.1.0** (owned-packet API is new and may refine as zenith adopts it; graduate to 1.0 once validated).
**Posture:** `#![no_std]` + `alloc`; `serde` feature-gated; ISO/IEC 13818-1 cited (already the cited source for these layouts in dvb-si — no new spec exposure).

## Purpose

Surface the canonical, spec-faithful **MPEG-2 Transport Stream framing layer** —
currently buried inside `dvb-si` under its `ts` feature and DVB-branded — as a
standalone crate. It is the foundation the rest of the rust-broadcast family
(ts-fix, the transmux packager, media-doctor) and external consumers (zenith)
build on. There is no canonical bidirectional TS packet/section crate in Rust
today (mpeg2ts-reader is read-only + stale); `mpeg-ts` fills that gap with
parse **and** build + section reassembly + resync + the owned/pipeline
ergonomics real muxers need.

## Scope (decided: A — framing only)

**In `mpeg-ts` v0.1 (generic MPEG-2 Systems framing):**
- TS packet: `TsHeader`, borrowed `TsPacket<'a>`, `AdaptationField` (discontinuity,
  random_access, PCR/OPCR, splice), `Pcr` (27 MHz).
- Generic PSI **section** framing: `Section<'a>`, `SectionReassembler` (TS→section),
  `SectionPacketizer` + `SiMux` (section→TS; the byte-exact inverse).
- `TsResync` sync-byte recovery + `PacketStride`/`ResyncStats`.
- **Owned/pipeline packet API** absorbed from zenith: `TsPacketBuf` (owned
  `[u8; 188]` + pre-parsed fields + `payload()`/`payload_mut()` + builders like
  `serialize_with_payload`). The borrowed `TsPacket<'a>` (zero-copy) and the owned
  `TsPacketBuf` are the crate's two packet representations.

**NOT in v0.1 (explicitly deferred):**
- Generic PSI **tables** (PAT 0x00 / CAT 0x01 / PMT 0x02 / TSDT 0x03) — these are
  ISO/IEC 13818-1 and *conceptually* belong here, but they are woven into dvb-si's
  `AnyTableSection` dispatch + `declare_tables!` macro. Moving them is a large,
  separate refactor → **`mpeg-ts` 0.2**.
- `SiDemux` (PAT-following SI section pump, dispatches `AnyTableSection`) — genuinely
  SI-aware; **stays in dvb-si**, rebuilt on mpeg-ts's primitives.
- Codec bitstream (h264/hevc/nal) and `ci_device` from zenith — separate sub-projects.

## What moves vs stays

| dvb-si file | Lines | Disposition |
|---|---|---|
| `ts.rs` | 1058 | → mpeg-ts (generic) |
| `section.rs` | 533 | → mpeg-ts (generic PSI section) |
| `resync.rs` | 481 | → mpeg-ts (generic) |
| `mux.rs` | 1134 | → mpeg-ts (SectionPacketizer + SiMux) |
| `demux.rs` | 793 | **stays** in dvb-si (SiDemux, SI-aware) — re-pointed onto `mpeg_ts::` primitives |

dvb-si's *internal* coupling to the moved code is tiny (only `collect/` + `tot.rs`
reference `crate::section::Section`), so the extraction is low-risk.

## dvb-si relationship (decided: depend + re-export, non-breaking)

`dvb-si` gains a dependency on `mpeg-ts` and **re-exports the moved types under
their existing paths** so downstream code keeps compiling unchanged:

```rust
// dvb-si lib.rs
#[cfg(feature = "ts")] pub use mpeg_ts as ts_impl; // or finer-grained:
#[cfg(feature = "ts")] pub mod ts      { pub use mpeg_ts::ts::*; }
#[cfg(feature = "ts")] pub mod section { pub use mpeg_ts::section::*; }
#[cfg(feature = "ts")] pub mod resync  { pub use mpeg_ts::resync::*; }
#[cfg(feature = "ts")] pub mod mux     { pub use mpeg_ts::mux::*; }
// demux stays a real module (SiDemux), now built on mpeg_ts primitives.
```

`dvb_si::ts::TsHeader` etc. still resolve (= the mpeg-ts types). **No breaking
change to dvb-si's public API** — same types, same paths, re-exported.

## Release coupling (the real consequence)

dvb-si is one of the **6 lockstep crates** (currently 7.9.0). Giving it a
`mpeg-ts` dependency requires a dvb-si release → a **lockstep `v7.10.0`** (all six:
dvb-common, dvb-si, dvb-t2mi, dvb-bbframe, dvb-conformance, dvb-tools). This
**naturally bundles the deferred `dvb-tools` clap-CLI work (#344)** already sitting
on main. So this epic ships as: `mpeg-ts 0.1.0` (new, independent) + lockstep
`v7.10.0` (dvb-si now depends on mpeg-ts; dvb-tools CLI; others version-parity).

Publish order: `mpeg-ts 0.1.0` first (depends only on dvb-common, live) → then the
lockstep (dvb-si needs mpeg-ts live).

## zenith (consumer, not modified here)

`mpeg-ts` is *shaped so zenith can adopt it* — it absorbs zenith's owned-packet
ergonomics (`OwnedTsPacket` → `TsPacketBuf`, `payload_mut`, builder helpers, PCR
`base*300+ext` which equals dvb-si's `Pcr::as_27mhz`). zenith-pipeline-specific
bits (ISI tagging, parse-stat atomics, BBFrame discontinuity) are **not** core —
zenith keeps those as its own thin layer, or mpeg-ts exposes a generic
`discontinuity` flag. **We do not modify zenith**; its team migrates it to depend
on `mpeg-ts` later. We only take what we need from zenith's `packet.rs`/`pcr.rs` as
reference/seed.

## Module layout (`mpeg-ts/src/`)

```
lib.rs       crate doc (ISO/IEC 13818-1), no_std, alloc, re-exports
packet.rs    TsHeader, TsPacket<'a>, AdaptationField, Pcr, TsPacketBuf (owned)
section.rs   Section<'a>, SectionReassembler (TS→section)
mux.rs       SectionPacketizer, SiMux (section→TS)
resync.rs    TsResync, PacketStride, ResyncStats
error.rs     Error/Result (mpeg-ts's own; dvb-si maps as needed)
```
(`ts.rs` splits into `packet.rs` + the section bits go to `section.rs`/`mux.rs` —
or keep a `ts.rs` mirroring today's layout to minimise churn; decide in the plan.)

## Error handling

`mpeg-ts` gets its own `thiserror` `Error` (BufferTooShort/BadSync/…), mirroring
dvb-common conventions. dvb-si's error type gains a `From<mpeg_ts::Error>` (or maps
at the boundary) so SiDemux/section paths propagate cleanly.

## Testing

- **Move** dvb-si's existing TS-layer unit + round-trip tests with the code (parse↔
  serialize byte-identical — the hard invariant travels with it).
- **Real fixtures:** the extraction must keep passing dvb-si's fixture tests
  (m6-single.ts etc.) — dvb-si's SiDemux (on mpeg-ts) is the integration proof.
- **Owned API:** add `TsPacketBuf` round-trip + `payload_mut` mutation tests
  (seeded from zenith's packet.rs tests).
- `label_coverage` for any public enum (PacketStride etc.); `--no-default-features`
  no_std build gate; ISO/IEC 13818-1 citation in each module `//!`.

## Out of scope (later sub-projects)
PSI tables (mpeg-ts 0.2), the codec crate (h264/hevc/nal from zenith), `ci_device`
(→ dvb-ci-runtime), and the `rust-broadcast` repo rename (this epic is its
trigger, handled separately).
