# hls-runtime 0.13.0

_Released 2026-11-10._

Breaking-class release (0.12 -> 0.13): a Media Playlist request before the first closed segment is now held (cold-playlist hold); sibling epochs move.

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

## Dependencies

- New dependency on `mpeg-ts` 0.7 (the TS sync byte is read from `mpeg_ts::ts::TS_SYNC_BYTE` instead of a local copy).
- Sibling epochs move: `transmux` 0.30 -> 0.31, `broadcast-hls` 0.6 -> 0.7, `timed-metadata` 0.9 -> 0.10, `media-plane` 0.10 -> 0.11, `broadcast-auth` 0.5 -> 0.6. Patch moves: `broadcast-common` 11.0.1, `container-probe` 0.3.1, `mpeg-ts` 0.7.1.

---

Published from tag `hls-runtime-v0.13.0`.
