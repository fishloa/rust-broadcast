# transmux 0.18.0 — 2026-07-21

**Breaking: British-spelling rename**, plus new LL-HLS client-facing playlist
parsing and bounded-memory hardening for the streaming reassembly paths
(issue #663, the multimux-hub epic).

## Breaking changes

- `rtp::RtpPacketizer` → `rtp::RtpPacketiser`.
- `rtp::RtpDepacketizer` → `rtp::RtpDepacketiser`.
- `rtp_stream::RtpStreamDepacketizer` → `rtp_stream::RtpStreamDepacketiser`.
- Free functions `rtp::packetize_klv`/`depacketize_klv` →
  `packetise_klv`/`depacketise_klv`.
- All pure renames — behaviour-preserving, no functional change. RFC 6184's
  SDP `fmtp` `mode`-selection attribute key is an external wire-protocol
  string and was deliberately **left** spelled exactly as the RFC defines
  it (not renamed).
- `hls::MediaSegment`/`PartSpec`/`LowLatencyConfig`/`RenditionReport`/
  `SkipInfo` all gained new fields (see Added below) — breaking for any
  external struct-literal construction; all five now derive `Default`, so
  add `..Default::default()` at existing call sites.

## Migration

| Old (0.17.x) | New (0.18.0) |
|---|---|
| `transmux::rtp::RtpPacketizer` | `transmux::rtp::RtpPacketiser` |
| `transmux::rtp::RtpDepacketizer` | `transmux::rtp::RtpDepacketiser` |
| `transmux::rtp_stream::RtpStreamDepacketizer` | `transmux::rtp_stream::RtpStreamDepacketiser` |
| `transmux::rtp::packetize_klv` | `transmux::rtp::packetise_klv` |
| `transmux::rtp::depacketize_klv` | `transmux::rtp::depacketise_klv` |

A find-and-replace of `Packetizer`→`Packetiser`, `Depacketizer`→
`Depacketiser`, `packetize`→`packetise`, `depacketize`→`depacketise` in your
own call sites is sufficient — the wire format and behaviour are unchanged.

## Added

- **`hls::MediaPlaylist::parse` + `hls::MasterPlaylist::parse`** (issue
  #717 slice 1): the symmetric inverse of the existing `to_m3u8()`
  renderers — parse an m3u8 string back into the same structs, so an
  LL-HLS client (e.g. the new `ll-hls-runtime` crate) can reuse the
  origin's wire model instead of growing a second one. Covers the full
  Media Playlist tag set plus the LL-HLS client-relevant tags
  (`#EXT-X-PART-INF`, `#EXT-X-SERVER-CONTROL`, `#EXT-X-PART`,
  `#EXT-X-PRELOAD-HINT`, `#EXT-X-RENDITION-REPORT`, `#EXT-X-SKIP`) and the
  Multivariant side (`#EXT-X-STREAM-INF`/`#EXT-X-I-FRAME-STREAM-INF`).
  Unrecognized tags are preserved verbatim into `extra_tags` (never an
  error). New types: `ByteRange`, `MapTag`, `PreloadHintType`,
  `RenditionReport`, `SkipInfo`. `#EXT-X-MEDIA` (Multivariant alternate
  renditions) remains a documented, unmodeled gap on both playlist sides.
- **`hls::OpenSegment::with_map`** — the in-progress LL-HLS segment builder
  gained a `map: Option<MapTag>` field so a muxer assembling an open
  segment can carry forward the Media Initialization Section in effect for
  it, mirroring `MediaSegment::map` on the parse side.
- **Fix, found while building the `ll-hls-runtime` client (issue #717
  slice 5): `LowLatencyConfig::can_block_reload`.** The
  `CAN-BLOCK-RELOAD` attribute's actual `YES`/`NO` value was previously
  discarded by `parse` (only the tag's presence set `low_latency`), so a
  client couldn't distinguish a genuine `CAN-BLOCK-RELOAD=NO` origin from
  one that supports blocking reload. `parse` now reads the real value
  (defaulting to `false` per RFC 8216bis when absent); `to_m3u8()` renders
  the actual value instead of always emitting `YES`.

## Security — bounded reassembly buffers (issue #663 P5.2, audit-ingest #4)

Two streaming-reassembly buffers previously grew without bound against
malformed or hostile input:

- `rtp_stream::RtpStreamDepacketiser`'s per-track in-progress access-unit
  buffer is now capped at `MAX_AU_BUFFER_BYTES` (4 MiB); on overflow the
  partial AU is dropped and `push` returns the new recoverable
  `Error::BufferCapExceeded`.
- `ts_demux::StreamingTsDemux`'s per-PID PES buffer is now capped at
  `MAX_PES_BUFFER_BYTES` (4 MiB); on overflow the partial PES is dropped
  and a `DemuxEvent::Discontinuity` is raised for the PID.

Neither cap changes behaviour for any well-formed stream (a real access
unit/PES is orders of magnitude smaller).

## Compatibility

MSRV unchanged (1.86). `no_std` + `alloc` posture unchanged.
