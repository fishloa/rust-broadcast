# mpeg-pes v0.1.1 — 2026-06-27

## Rename from `dvb-pes`

`mpeg-pes` 0.1.1 is the renamed successor to `dvb-pes` 0.1.1. PES (Packetised
Elementary Stream, ISO/IEC 13818-1 §2.4.3) is a generic MPEG-2 Systems construct,
not DVB-specific — the new name reflects that. The code is identical; only the
crate name changed.

### Migration

In `Cargo.toml`, replace `dvb-pes = "0.1"` with `mpeg-pes = "0.1"`; in `use`
statements replace `dvb_pes::` with `mpeg_pes::`. All types and paths are
identical.

The deprecated `dvb-pes` shim (v0.1.2) re-exports `mpeg-pes` 0.1 for backwards
compatibility and will not receive new features. The workspace consumers
`mpeg-ps` (0.1.2) and `dvb-subtitle` (0.1.1) now depend on `mpeg-pes` directly.

## What's in v0.1.1

All functionality from `dvb-pes` 0.1.1: PES depacketisation + PTS/DTS extraction
(ISO/IEC 13818-1 §2.4.3.6/§2.4.3.7), `#![no_std]` + `alloc`, depends only on
`dvb-common`.
