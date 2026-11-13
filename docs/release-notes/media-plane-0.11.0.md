# media-plane 0.11.0

_Released 2026-11-14._

Breaking-class release (0.10 -> 0.11): breaking API changes are listed below, and the crate's sibling dependencies move to new caret epochs.

Version: 0.10.0 -> 0.11.0.

## Added

- `Trunk::open_segment`: lowest segment above last closed one with resident parts.

## Fixed

- `SampleCursor`: back-to-back writer re-issues with no data report exactly one `Discontinuity`.

## Also in this release

### Added
- `segmenter::program` (feature `std`): `ProgramSegmenter`, `SegmenterConfig`, `SegmenterEvent`, `SiblingTrunks`, `TsHlsIdrFallback` and `TsHlsIdrStatus`, the per-program segmenter that cuts a `Trunk`'s samples into parts and segments, moved from multimux (#2554). `SegmenterConfig` is `#[non_exhaustive]`: build it from `SegmenterConfig::default()` and the `with_*` setters.
- `segmenter::subtitle` (feature `std`): the teletext-subtitle rendition follower moved from multimux. `teletext_pages` lists a program's subtitle pages (bounded by the caller's `max_pages`), `SubtitleFollower` turns the page's teletext PES data fields into WebVTT parts and segments cut where the leader cut and publishes them into its sibling `Trunk`, with `TeletextPage` and `LeaderPart`. `caption-convert` and `dvb-si` are now optional dependencies behind `std` (issue #2554).
- `segmenter::audio` (feature `std`): the separate-audio-rendition follower moved from multimux. `AudioFollower` cuts one audio track where the video leader cut and publishes into its sibling `Trunk`; the host serves its init segment (`take_init`) and the part target its first sample fixes (`take_part_target_ms`) instead of the follower driving an HLS origin. `resolve_sources` maps rendition selectors to source tracks, `leader_tracks` keeps the primary playlist's tracks, with `LEADER_DURATION_HISTORY` and `LeaderDurations` (issue #2554).
- `publish_queue` (feature `std`): the non-blocking publish queue a segmenter uses, moved from multimux. `PendingPublish` (a part or a finished segment, in stream order), `part_entry`, and `drain_publish_queue`, which hands queued items to a `SegmentWriter` as far as it takes them and force-expires a stalled `StallIngest` pin once the queue has waited `PENDING_PUBLISH_MAX_WAIT` or holds `PENDING_PUBLISH_QUEUE_CAP` segments (issue #2554). `tracing` is now an optional dependency behind `std`.
- `publish_queue` (feature `std`): the non-blocking publish queue a segmenter uses, moved from multimux. `PendingPublish` (a part or a finished segment, in stream order), `part_entry`, and `drain_publish_queue`, which hands queued items to a `SegmentWriter` as far as it takes them and force-expires a stalled `StallIngest` pin once the queue has waited `PENDING_PUBLISH_MAX_WAIT` or holds `PENDING_PUBLISH_QUEUE_CAP` segments (issue #2554). `tracing` is now an optional dependency behind `std`.
- `file_source` (feature `std`): the sans-IO core of linear file playout moved from multimux. `probe_and_demux` identifies a whole file with `container-probe` and demuxes it (`select_demuxer`, `DemuxerKind`, `demux_identified`) into a `ParsedFile`; `ParsedFile::build_playout` merges its tracks into one paced publish order; `FileIngestSession` replays it as `SessionEvent`s, looping on one monotonic per-track timeline and handing out at most `MAX_SAMPLES_PER_DRAIN` due samples per drain (`FILE_TIMED_RING` sizes the ring); `FileReaderError` names every failure (issue #2554). `container-probe` is now an optional dependency behind `std`.
- `ts_session` (feature `std`): the shared MPEG-2 TS ingest session moved from multimux. `TsIngestSession` feeds `transmux::StreamingTsDemux` and translates its events into `SessionEvent`s, `ProgramTracker` (public) tracks which `ProgramId` owns each track and holds pre-resolution samples within `MAX_HELD_SAMPLES`/`MAX_HELD_BYTES`, `SILENT_PID_SETTLE`, `MAX_TS_READ`, and a backlog-drop observer (`with_backlog_drop_observer`) in place of the multimux metric. Feature `test-support` adds `ts_session::test_support` (muxed TS bytes and configs) for a host's tests (issue #2554).
- `ingress::IngestConfig` (`new(handshake)`, `with_trunk`, `with_max_programs`): the handshake bound, the per-program `TrunkConfig` and the per-session program bound an ingest driver is built from. A new type rather than fields on `TrunkConfig`, because the deadline and the program bound belong to a connection, not a ring, and callers vary the three independently (#2514).
- `benches/trunk_parked_readers`: publish cost and wake latency with 1 to 1024 readers parked on `SampleCursor::next` (#2515).
- `Trunk::generation`: a counter bumped by every part, segment, event, track-set, segment-start and time-anchor publish (never a sample), for caching a render keyed by the state it was made from.
- `ServedEgress::is_warm` and `ServedEgress::prepare` (default always warm / no-op): an egress with heavy first-use work says so, so an async host runs `prepare` on a blocking thread instead of on the worker that woke the request.
- `TrunkConfig::waiter_capacity` (default `DEFAULT_WAITER_CAPACITY`, set with `TrunkConfig::with_waiter_capacity`): the bound on concurrent `Trunk::listen` registrations is its own setting instead of `part_capacity`, so a programme is no longer limited to 64 parked blocking-reload requests (#2523).
- `SampleCursor::next` and `Trunk::data_listener`: a sample, event, track-set change or writer hand-off notifies the trunk once, and a consumer awaits it instead of polling; an idle cursor takes no trunk lock (#2524).
- `Trunk::tracks_with_generation` and `Trunk::subscribe_from_backlog_with_tracks`: the track generation and set (and, for the latter, a backlog cursor) read under one lock acquisition.
- `ingress::AdmittedDriver`: the bookkeeping every concurrently-admitting push-ingest loop had hand-written — one `IngestDriver` per admitted session (`accept`/`accept_with`/`register`), each session's clock measured from its own admission rather than the route's start, `feed`/`finish` plus a release-then-reap step that frees the session's `max_sessions` slot, and an idle clock (`touch`/`idle_deadline`) measured only from input the session actually took in. The multiplexing WHIP and RTMP ingest loops now share it instead of each carrying a copy.
- `ingress::ListenDriver::listener`: read access to the listener a driver owns.

### Changed
- The `test-support` feature (the `ts_session::test_support` helpers for a host's tests) is documented in the README feature table (#2582).
- The docs of `SegmentWriter::publish_segment`, `ProgressListener::wait_deadline` and `SlotFreedFuture::wait_deadline` state that they park the calling thread and must not run on an async worker (issue #2533).
- BREAKING: `Trunk::waiter_slot_freed` takes `&Arc<Trunk>` and returns the new `SlotFreedFuture` (a `Future` with `wait_deadline`, replacing the re-exported `EventListener` alias): a woken waiter that answers without taking the slot wakes the next parked one, so one freed slot drains every parked waiter.

### Removed
- BREAKING: the four-positional `IngestDriver::new(session, trunk_config, handshake, max_programs)` and `ListenDriver::new(listener, ..)`; both take `(session_or_listener, IngestConfig)` (#2514).
- BREAKING: `TrunkConfig::new` (five positional `NonZeroUsize`, any reorder compiled). Start from `TrunkConfig::default()` (64 timed, 16 sparse, 8 segment, 64 event, 64 part entries) and set what differs with `with_timed_capacity`, `with_sparse_capacity`, `with_segment_capacity`, `with_event_capacity`, `with_part_capacity` (and the existing `with_waiter_capacity`) (#2514).

### Fixed
- Track-change markers are evicted by the `Timed` ring rather than by their own small log: a marker stays until the oldest retained sample is at or past the next, so a cursor starting anywhere in the retained window reads each sample under its governing marker, never the oldest retained one (audit A11-2). `TRACK_CHANGE_LOG_CAPACITY` is now a 1024-entry hard cap for pathological flapping; a cursor that needed a marker it evicted gets the new `SampleCursorItem::TracksLagged { skipped, generation, tracks }`, which drops the ungoverned samples and carries the current track set.
- A `SampleCursor` that fell more than the track-change log behind is now handed the oldest retained `SampleCursorItem::TracksChanged` at once instead of reaching it only after the samples before its position, so it learns a complete track set rather than missing the evicted changes silently (audit A10-4).
- `SampleCursorItem::TracksChanged { generation, tracks }`: `TrunkWriter::set_tracks` is now an in-band event in the `Timed` stream, so a consumer switches track set exactly between the last sample published under the old set and the first under the new one, instead of racing `Trunk::track_generation` against the ring (audit A9-4).
- `AdmittedDriver::with_session`/`session_parts` now check admission before touching `progress`, so an unknown/reaped id returns `None` without inserting a progress entry.

## Dependencies
- Sibling dependencies move to new caret epochs: `broadcast-common` 11 -> 12, `timed-metadata` 0.9 -> 0.10, `transmux` 0.30 -> 0.31.
- New sibling dependencies: `broadcast-hls` 0.7, `caption-convert` 0.6, `container-probe` 0.4, `dvb-si` 14.

---

Published from tag `media-plane-v0.11.0`.
