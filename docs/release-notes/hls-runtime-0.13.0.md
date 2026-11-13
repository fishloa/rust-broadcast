# hls-runtime 0.13.0

_Released 2026-11-14._

Breaking-class release (0.12 -> 0.13): breaking API changes are listed below, and the crate's sibling dependencies move to new caret epochs.

Version: 0.12.0 -> 0.13.0.

## Breaking changes and migration

- Behaviour: a Media Playlist request (primary, audio, subtitle and I-frame) without `_HLS_msn`/`_HLS_part` made before the playlist holds a closed segment is answered `EgressResponse::Await` until the first segment closes, then with the playlist, and `NotFound` once the caller's `AwaitPolicy` expires (an adapter's `503`, RFC 8216bis section 6.2.5.2). Players that ignore Partial Segments can now open a cold stream. To keep the old immediate answer:

```rust
// after
let origin = HlsOrigin::builder(trunk)
    .hold_cold_playlist(false)
    // ...
    .build()?;
```

## Added

- `HlsOriginBuilder::discontinuity_sequence(u64)` and `HlsOrigin::discontinuity_sequence_after()`: an origin that replaces another starts its `EXT-X-DISCONTINUITY-SEQUENCE` where the previous one ended.
- `HlsOriginBuilder::hold_cold_playlist(bool)` and `HOLD_COLD_PLAYLIST_DEFAULT` (`true`).

## Fixed

Client:
- A failed omitted-offset `EXT-X-BYTERANGE` segment or part is re-requested with its own range; a restart clears the stored ranges.
- A discontinuity revealed when a segment closes after all its parts were delivered is signalled late instead of dropped.
- A part whose fetch failed is re-requested when its segment closes without listing parts, and the segment is not marked delivered until every requested part is.
- A variant switch emits one `Discontinuity`, not two.
- A skipped segment is skipped once; a playlist reload no longer rewinds the byte-range cursor of later omitted-offset segments.

Origin:
- A blocking reload for an existing part of the just-closed segment is answered instead of parked.
- A segment keeps its `EXT-X-PROGRAM-DATE-TIME` when the playlist window holds more segments than the trunk's `event_capacity`.
- The open segment is found when the segmenter's first segment is numbered above 1.
- A bare `_HLS_msn` reload (media and I-frame playlists) is satisfied from the rendered window, so a playlist answered as satisfied lists that segment.

## Also in this release

### Changed
- The client's RFC 3339 tests exercise `broadcast_hls::ProgramDateTime::parse`, the parser the client actually uses, instead of a test-only `jiff::Timestamp` copy that rejected the zone-less and whitespace-padded values the production parser accepts (the test-only parser is gone) (#2591).
- BREAKING: `HlsClient::new(url)` takes a `client::HlsClientConfig` (`HlsClientConfig::new(url).with_variant_policy(..).with_playlist_base(..)`), and `HlsClient::set_variant_policy` is removed in favour of the config's `variant_policy` (`set_playlist_base` stays: a redirect changes the base mid-stream) (#2514).
- BREAKING: every `HlsOriginBuilder` setter is named `with_*` (`with_target_duration_secs`, `with_window_segments`, `with_container`, `with_low_latency`, `with_hold_cold_playlist`, `with_media_sequence_offset`, `with_discontinuity_sequence`, `with_instance`, `with_program_date_time_arrival`, `with_declared_bandwidth_bps`, `with_audio_renditions`, `with_audio_sibling_origins`, `with_subtitle_renditions`, `with_subtitle_sibling_origins`, `with_playlist_uri`, `with_rendition_siblings`, `with_closed_captions_none`, `with_closed_caption_overrides`), matching `TrunkConfig`, `IngestConfig` and the other engine configs (#2514).
- URL-reference resolution now uses the shared guarded resolver in `broadcast-common::url_ref` (control characters rejected before joining) (issue #2458).

### Removed
- BREAKING: `server::Container` moved to `transmux::Container` (same variants and methods, `segment_extension` now public); `HlsOriginBuilder` and the origin take and report that type (issue #2554).
- BREAKING: `client::tokio_client` (`TokioClient`, `TokioClientConfig`, `TokioClientStats`, `TokioError`) and the `tokio` client feature, plus its tests and its now-unused `tokio`, `reqwest`, `broadcast-auth`, `backon`, `headers` and `tokio-util` dependencies. It carried its own copies of the retry, digest and body-cap policy, reachable only from tests; the IO belongs with the crate that drives the engine. Drive the sans-IO `HlsClient` core instead (`poll` out, `on_playlist`/`on_resource` in), as `multimux`'s HLS pull and `multimux/tests/golden_gate.rs`/`glass_to_glass.rs` do.
- `bcp47_language` (moved to `broadcast-common::lang::bcp47_language`).

### Added
- `HlsOrigin` caches each rendered media and I-frame playlist per state stamp (the trunk's publish counter, the origin's setter epoch, the drained head, and the same for rendition siblings): concurrent requests share one render instead of each re-rendering (#2526). `HlsOrigin::render_epoch` exposes the setter epoch.
- `HlsOrigin` implements `ServedEgress::is_warm`/`prepare`: the I-frame and bitrate byte scans of a newly published segment run in `prepare`, which a host calls on a blocking thread, so no async worker waits on the drain gate.
- `BLOCKING_RELOAD_TARGET_DURATIONS`, `BLOCKING_RELOAD_TIMEOUT` and `playlist_reload_timeout` moved here from `multimux` (RFC 8216bis §6.2.5.2).
- `HlsOrigin::set_init_from`: `set_init` for a caller that names the first segment cut against the new init, for segments closed but not yet published.

## Dependencies
- Sibling dependencies move to new caret epochs: `broadcast-common` 11 -> 12, `broadcast-hls` 0.6 -> 0.7, `container-probe` 0.3 -> 0.4, `media-plane` 0.10 -> 0.11, `timed-metadata` 0.9 -> 0.10, `transmux` 0.30 -> 0.31.
- New sibling dependencies: `mpeg-ts` 0.8.

---

Published from tag `hls-runtime-v0.13.0`.
