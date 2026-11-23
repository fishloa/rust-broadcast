# multimux 0.21.0

_Released 2026-11-22._

Minor for a 0.x crate, breaking. The DVR archive moves out of this crate into `dvr-runtime`, which becomes the one owner of the on-disk format; the public wall clock becomes `jiff::Timestamp`; the remaining `std::sync::Mutex` slots become `parking_lot`; and a push output's decisions now come from `media-plane`'s `push_session`. `multimux-cli` 0.19.0 follows.

## Breaking changes

- **The DVR archive format is replaced by `dvr-runtime`'s** (#3929, part of #3687). Each period is a media file (`pN.m4s`/`pN.ts`) plus an HLS media playlist `pN.m3u8` that indexes it with `EXT-X-BYTERANGE`, carries each segment's exact timing in `#EXT-X-RUSTBROADCAST-TIMING`, names the period's one fMP4 init with `EXT-X-MAP` and marks EIT programme boundaries with `EXT-X-DATERANGE`. The `pN.idx` JSON sidecar and the `pN.event.json` programme sidecar are gone. **An archive written by multimux 0.20 or earlier is not readable and there is no migration**: a route over such an archive scans it as empty, so start new routes on an empty directory (or archive the old tree aside). A period also rolls on a sequence gap, on a segment longer than the playlist's target duration, and before the playlist would exceed the HLS size limit.
- Removed from the public API: `dvr::{DvrRecorder, DvrError, IndexEntry, EitProgramme, InitGeneration}` (use `dvr_runtime::InitGeneration`) and `DvrRecorder::needs_poll`/`needs_poll_for_init`/`poll_and_persist`/`poll_and_persist_inits`/`feed_si`/`with_seq_offset`/`rebuild_index`/`current_programme`/`overrun_policy`. `dvr` keeps only `DvrConfig` and `ArchiveOverrunSerde`; a consumer that needs archive types depends on `dvr-runtime`.
- `MultimuxError::Dvr(DvrError)` is replaced by `MultimuxError::ArchiveScan(JoinError)` (the archive scan behind a route's sequence floor did not run to completion); `DvrConfig::validate` returns `MultimuxError::ConfigInvalid` naming `ArchiveRoot`, `Retention` or `PeriodDurationSecs`, with `dvr_runtime::ArchiveConfig::validate`'s rules. New `DvrConfig::to_archive_config`.
- `RouteHandle::with_archive_floor` takes a `broadcast_hls::MediaSequence`, not a bare `u32`; a value past the `u32` sequence domain clamps to `u32::MAX` rather than wrapping (#3991).
- Wall-clock instants in the public API are `jiff::Timestamp`, not `std::time::SystemTime`: `RouteHandle::presentation_origin`/`created_at`, `output::dash::render_mpd_at`/`render_ll_dash_mpd_at`, `output::ll_dash::render_ll_dash_mpd_at` and the `format_iso8601_for_test` seam. Rendered `availabilityStartTime`/`publishTime` are byte-identical. Migration: pass `jiff::Timestamp::now()` where a `SystemTime` was read.
- `config::TsHlsIdrFallback::falls_back` is removed; nothing consulted it (#3686).
- Builds against `broadcast-common` 14, `broadcast-auth` 0.9, `broadcast-client` 0.8, `broadcast-dash` 0.3, `broadcast-hls` 0.9, `broadcast-smooth` 0.3, `container-probe` 0.6, `dash-runtime` 0.7, `dvb-conformance` 16, `dvb-si` 16, `dvb-stream` 0.11, `hls-runtime` 0.16, `media-doctor` 0.18, `media-plane` 0.14, `mpeg-ts` 0.10, `rtmp-runtime` 0.12, `rtp-packet` 0.9, `rtsp-runtime` 0.14, `scte35-splice` 8, `smooth-runtime` 0.4, `srt-runtime` 0.12, `timed-metadata` 0.12, `transmux` 0.33 and `webrtc-runtime` 0.10. A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2); a consumer must resolve a single copy of each.

## Added

- Catch-up routes take an optional `program` query parameter naming a program by its `program_number` (`catchup.m3u8?program=1538`, `vod/p{N}.m3u8?program=1538`), reading `<route>@<program_number>`; absent (or `0`) reads `<route>/` as before, and `?program=<dvb_service_id>` is an alias. Every URI in a named program's playlists carries the parameter. A program with archived segments but no live program is served from the archive alone; an unknown `program_number` answers 404 (#3950).

## Changed

- Recording runs through `dvr_runtime::fs::FsRecorder` on the blocking pool: the route's monotonic clock for period age, an arrival-anchored wall clock for every segment's `EXT-X-PROGRAM-DATE-TIME`, and `dvb_si::epg::PresentEventTracker` for programme boundaries. Writer events are logged and drive the pin/rearm, SI-error, predecessor-flush and gap counters; a store failure keeps the segment and retries it on the next poll. A recorder refused by another lock holder backs off rather than retrying on every ingest tick.
- Catch-up reads through one `dvr_runtime::fs::FsArchive` per route; the process-global index cache is gone. Unreadable periods and two periods claiming one sequence number are logged per request. `catchup/init-p{N}.mp4` serves the range the period playlist's `EXT-X-MAP` names.
- The catch-up live tail maps each segment to the init generation it was closed under (it used to name generation 1 for every segment).
- The remaining `std::sync::Mutex` slots (RTMP/WHIP accept queues, pre-bound sockets, `PreboundBinds`, the ingest golden recorder's log) are `parking_lot::Mutex`, so `PreboundBinds::{tcp,udp}` and `Recorder::new`/`Recorded` take `parking_lot` locks and there is no poison handling (#3971).
- `push::drive_push`'s decisions are `media_plane::push_session::PushSession`'s: negotiation, loss accounting, drop-versus-reconnect classification, flush outcome and give-up come from the sans-IO core. A connection whose negotiation fails or is refused is closed before the backoff, and a redundant second connection is closed too (#3930, #3987).
- `PushMetrics::bytes_sent` reports the payload bytes written over the transport (it was never incremented), and `samples_dropped` also counts cursor-lag and disconnected samples (#3930).
- `DvrConfig::default()` now has `period_duration_secs: 10800`, the same default serde applies (#3666).
- Internal: error types derive `thiserror::Error` (messages unchanged) (#3974, #4020).

## Fixed

- A multi-program (MPTS) route records every program: each has its own archive with its own lock, sequence floor, retention and EIT tracker, so the second program is no longer refused by the first one's lock. Set `dvb_service_id` on an MPTS input to choose which service the default catch-up URL serves; every other program records into `<archive_root>/<route>@<program_number>/`. An archive root that exists but cannot be listed fails the route's floor scan instead of reading as empty.

## Pre-release audit

Issues fixed in this crate by the 2026-11-22 pre-release audit: #3991 (`with_archive_floor` keeps the sequence type), #4018 (duplicate headings), #4020 (`thiserror`).

---

Published from tag `multimux-v0.21.0`.
