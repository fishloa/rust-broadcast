---
status: DRAFT
---
# hls-runtime 0.13.0

_Released 2026-11-10._

Breaking-class release (0.12 -> 0.13): a Media Playlist request before the first closed segment is now held (cold-playlist hold); sibling epochs move.

Version: 0.12.0 -> 0.13.0.

## Breaking changes and migration

- Behaviour: a Media Playlist request (primary, audio, subtitle, I-frame) without `_HLS_msn`/`_HLS_part` made before the playlist holds a closed segment is answered `EgressResponse::Await` until the first segment closes, then with the playlist, and `NotFound` once the caller's `AwaitPolicy` expires. Clients ignoring Partial Segments cannot open a playlist with no `EXTINF`. `hold_cold_playlist(false)` restores the previous immediate answer.

## Added

- `HlsOriginBuilder::discontinuity_sequence(u64)` and `HlsOrigin::discontinuity_sequence_after()`.
- `HlsOriginBuilder::hold_cold_playlist(bool)` and `HOLD_COLD_PLAYLIST_DEFAULT` (`true`).

## Fixed

- TS sync byte read from `mpeg_ts::ts::TS_SYNC_BYTE`.
- Client fetch/retry, discontinuity, variant switch, skipped segment fixes.
- Origin blocking reload, program date time, open segment, bare `_HLS_msn` fixes.

---

Published from tag `hls-runtime-v0.13.0`.
