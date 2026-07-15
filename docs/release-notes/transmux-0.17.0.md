# transmux 0.17.0 — 2026-07-15

Minor, purely additive. Adds a **streaming, timing- and config-aware RTP
depayloader** and **SDP-fmtp → codec-config** helpers (issue #700) — the
upstream library work that lets a live RTSP origin (issue #663, `multimux`) turn
an incoming RTP feed into correctly-timed CMAF, with no media/transport-spec
logic in the app itself. RTP is now a first-class streaming ingest spoke
alongside the existing batch `RtpDepacketizer`.

## Added (#700)

- **`RtpStreamDepacketizer`** — stateful counterpart to the batch
  `RtpDepacketizer`. Fed RTP packets incrementally via `push(track_id, &[u8])`,
  it emits fully-timed `Sample`s and `flush(track_id)` drains the tail:
  - real per-AU `duration` from RTP-timestamp deltas (32-bit wire timestamp
    unwrapped to a monotonic `u64`; IR timescale = RTP clock rate),
  - `is_sync` from IDR detection (H.264) / always-true (AAC),
  - carries the real `CodecConfig` supplied at construction,
  - `track_specs()` yields the `TrackSpec`s for building the init segment.
  - `RtpStreamTrack` (the per-track config) is `#[non_exhaustive]`; construct
    via `RtpStreamTrack::new(track_id, kind, config, clock_rate)`.
- **`rtp_sdp` helpers** (re-exported at the crate root):
  - `avc_config_from_sprop(&str) -> AVCConfigurationBox` — RFC 6184 §8.1
    `sprop-parameter-sets` (base64 SPS/PPS) → `avcC`.
  - `aac_config_from_fmtp(&str) -> CodecConfig` — RFC 3640 §4.1 `config`
    (hex `AudioSpecificConfig`) → `CodecConfig::Aac` (esds), recovering sample
    rate + channel count.

Internally, the existing FU-A/STAP-A/AAC reassembly is refactored to a
timing-preserving core (`reassemble_video`/`reassemble_audio` → `ReassembledAu`)
shared by both the batch and streaming paths; the batch `RtpDepacketizer` output
is byte-identical.

## Correctness

Proven on a **real broadcast fixture** (`h264_aac.ts`): demux → RTP packetize →
SDP-derived config → streaming depayload → the recovered samples match the
original in codec config (SPS/PPS byte-for-byte), keyframe count, and total
duration (within one frame), and build init + media segments + a part that pass
the fMP4/CMAF conformance validator. The gate was mutation-verified to bite
(forcing `duration = 0`, `is_sync = false`, or a corrupted SPS each fails a
distinct assertion).

## v1 limits (documented)

- Low-delay H.264 only: `composition_offset = 0`; B-frame DTS reconstruction is
  future work.
- One AAC access unit per packet (multi-AU RFC 3640 aggregation would give
  non-final AUs `duration = 0`).
- Packets must be fed in arrival order (matching the batch reassembly contract).
- Cross-track A/V alignment via RTCP Sender Report NTP/RTP correlation is out of
  scope.

## Compatibility

Additive — no breaking changes. `media-doctor`'s transmux dependency floor moves
to `0.17` (it does not use the new API yet; the bump keeps the workspace
consistent).
