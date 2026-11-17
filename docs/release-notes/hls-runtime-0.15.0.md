# hls-runtime 0.15.0

_Released 2026-11-17._

### Fixed
- Client: every segment, part, map, key and variant URI is resolved through `broadcast_common::url_ref::resolve_http`, so a reference to `file:`/`ftp:`/`data:` or any other non-web scheme is refused (as `UnresolvableReference`) instead of requested, and an `https` stream refuses an `http` key/map/segment URI rather than downgrading the fetch (audit 20 finding 42, #2853).
- Client: Part/Segment bodies held until their init segment is ready are capped at `MAX_PENDING_DEMUX_BYTES` (64 MiB), dropping the oldest with `Error::PendingDemuxOverflow`, and `prune` drops them below the window. A playlist whose `EXT-X-MAP` never becomes ready while media keeps arriving can no longer grow the queue for the life of the pull (audit 20 finding 3, #2814).
- Client: `prune_inits` drops a non-`Ready` init slot that neither a segment nor the current playlist's maps refer to, and the slot map is capped at `MAX_INIT_SLOTS` (64), so a playlist naming a new failing `EXT-X-MAP` on every reload can no longer grow the map without bound or slow the per-map scan (audit 20 finding 4, #2815).

### Added
- `client::Error::PlaylistTooLarge`: a playlist over a `broadcast_hls::limits::Limits` bound is a distinct, non-retryable error rather than a generic `PlaylistParse`, so a caller never re-fetches the same over-limit response (audit 20 finding 41, #2852).
- `HlsClientConfig::limits` (a `broadcast_hls::limits::Limits`, default 4 MiB playlist, 1 000 000 segments, 64 MiB resource body) and `HlsClientConfig::with_limits`; `on_playlist` parses through `parse_with` and `on_resource` refuses a body over `max_segment_bytes` with `Error::ResourceTooLarge` (audit 20 finding 41, #2852).
- `client::Error::UnresolvableReferences { count }`: the `next_error` queue is capped at 256 reports and coalesces the excess into one counted error, so a playlist naming many bad references cannot grow the queue without bound when the host does not drain it (audit 20 finding 33, #2844).
- `client::Error::UnsupportedDefine`: a playlist declaring `EXT-X-DEFINE` variables (RFC 8216bis §4.4.2.3) is refused rather than fetched with `{$name}` left literally in the URI, which produced wrong requests and 404 storms (audit 20 finding 19, #2830).

### Changed
- Test-only: `MAX_CACHED_RENDERS`, `FUTURE_PART_SEGMENT_BOUND`, `ABUSE_MSN_FUTURE_BOUND` (Media and I-frame playlists) and `MAX_RELOAD_WAIT_MS` each have an over-limit test that fails if the bound is removed (audit 20 finding 52, #2863).
- BREAKING: `server::master_playlist_m3u8` returns `Result<String, broadcast_hls::Error>` and renders through `broadcast_hls::MasterPlaylist`, so a caller-supplied media-playlist name carrying a newline or `"` is refused instead of injecting playlist lines (audit 20 finding 31, #2842). Migration: `?`/`expect` the result; the rendered text is unchanged for a valid name.
- Client: a fragment's track specs are compared with `TrackSpec`'s `PartialEq` instead of formatting and comparing two `Debug` strings per fragment, so the per-fragment cost no longer scales with the codec-config size (audit 20 finding 50, #2861).

### Fixed
- Client: an `EXT-X-MEDIA-SEQUENCE` that overflows `u64` is rejected before any client state is mutated, so one hostile reload can no longer move the high-water mark to `u64::MAX` and drop every key, wedging the session for every later playlist (audit 20 finding 18, #2829).

---

Published from tag `hls-runtime-v0.15.0`.
