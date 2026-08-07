# hls-runtime 0.2.0

**Release date:** 2026-07-28

Major internal rewrite: the rolling-window `MediaStore` is deleted. The `HlsOrigin` server engine now resolves init/segment/part bytes and renders playlists over the `media_plane::Trunk`'s own rings — the `Trunk` is the single copy of the data, never a second cache. Also adds the `HlsPullClient` (renamed from the old `ll-hls-client` playback client) with `Fmp4Demux`-based output and optional `tokio`+`reqwest` IO adapter.

## What's new

- `HlsOrigin` — sans-IO LL-HLS origin engine resolving init/segment/part bytes from `Trunk` rings.
- `HlsPullClient` — caller-driven playback client with blocking-reload scheduler, part-prefetch, `Fmp4Demux`-based output.
- Optional `tokio`+`reqwest` IO adapter for pull client.

## What changed

- **`MediaStore` deleted** — replaced entirely by the `Trunk` data plane. No second cache of the live window.
- Requires `media-plane` 0.1.

## Migration

Breaking: `MediaStore` is gone. Consumers that built on it must migrate to the `HlsOrigin` + `Trunk` model. See `media-plane` 0.1.0 for the `Trunk` API.
