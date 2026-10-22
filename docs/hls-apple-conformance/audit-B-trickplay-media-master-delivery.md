# Audit B — Apple HLS Authoring Spec sections 6, 8, 9, 10 (origin-facing core)

Read-only code audit of origin/main @ `90a30b58e` (hls-runtime `HlsOrigin` + multimux). Spec: `private/specs-md/apple_hls_authoring_specification_for_apple_devices.md`. Condensed from the auditor's report (85 rows: 14 MET-VERIFIED, 3 MET-UNVERIFIED, 6 PARTIAL, 24 GAP, 38 NOT-APPLICABLE). `E` = `hls-runtime/src/server/engine.rs`, `BH` = `broadcast-hls/src/lib.rs`.

## Headline facts

1. **Gzip (10.1, MUST)** not implemented: tower-http built with `timeout, limit, cors` only. Issue #1424.
2. **Master is a single `EXT-X-STREAM-INF`** (no `EXT-X-MEDIA`, no steering, no independent-segments).
3. **I-frame playlists (6.x)**: engine renders them (P2a) — master advertisement (P2b) and multimux route (P3) were in flight at audit time. fMP4 only.
4. **Latent defect**: `compute_iframe_len` measures stored segment bytes but `inject_segment_events` splices `emsg` after `styp` on the served path (and `parse_segment_filename` never matches real instance-named segments, so injection is currently dead). Issue #1422.
5. **Missing live-playlist tags**: `EXT-X-PROGRAM-DATE-TIME` (8.4 MUST), `EXT-X-INDEPENDENT-SEGMENTS` (9.12 MUST), `EXT-X-DISCONTINUITY-SEQUENCE` only when > 0 (8.17 MUST). Issue #1423.
6. **Content types**: `.m4s` served `video/mp4`, recommended `video/iso.segment` (10.4d).

## Gaps by severity (MUST first)

| # | Item | Gap | Issue |
|---|---|---|---|
| 1 | 10.1 | no gzip playlists | #1424 |
| 2 | 8.4/8.5 | no PROGRAM-DATE-TIME | #1423 |
| 3 | 8.17 | DISCONTINUITY-SEQUENCE omitted when 0 | #1423 |
| 4 | 9.11/9.12 | INDEPENDENT-SEGMENTS never emitted | #1423 |
| 5 | 6.1/6.9/9.3/9.4 | I-frame variants advertised + BANDWIDTH/CODECS/RESOLUTION | #1413 (P2b merged, P3 PR #1430) |
| 6 | 6.10 | emsg injection vs I-frame byte range | #1422 |
| 7 | 9.6/9.7/8.9 | separate audio `EXT-X-MEDIA` | #1428 |
| 8 | 8.14 | frame-rate change not a discontinuity | #1425 |
| 9 | 8.12 | default window ~32 s vs 15 min (tvOS 120) | — |
| 10 | 10.4d/10.2 | `.m4s` MIME, failover | #1426 |

Also: fallback master before first measurement has no `CODECS` and a fixed 5 Mb/s `BANDWIDTH` (9.1, 9.13; #1425).

## Not assessable by reading

6.4/6.6/6.7 (encoder choices for multi-bitrate sources); 8.1 (segmenter duration accuracy); 8.24 (`Date`, added by hyper); the macOS-only `mediastreamvalidator` oracle covers the `transmux`/`media-doctor` packaging path, **not** `HlsOrigin` — first run on the live origin: #1429.
