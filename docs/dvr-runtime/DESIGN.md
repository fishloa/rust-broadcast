# dvr-runtime: sans-IO DVR archive, EIT programme tracker, PushSession (design)

Status: owner-approved decisions folded in 2026-11-21; final owner read pending (issue #3687). Shipped in
`dvr-runtime` 0.1.0: section 4 is the shipped API (its later amendments are listed at the end of section 4). Line
numbers are `origin/main` at 5ed0c087d. RFC citations
are section numbers only; spec text lives in `private/specs-verified/ietf/specs/ietf_draft_pantos_hls_rfc8216bis.txt`
(cited as "8216bis") and is not quoted here.

Owner decisions applied:
1. New sans-IO crate `dvr-runtime` owns the archive format, the writer's roll/retention state machine and catch-up
   read planning; the `std::fs` adapter is behind feature `fs`.
2. Name `dvr-runtime`.
3. The EIT present/following tracker moves to `dvb-si::epg`.
4. `drive_push`'s decision core becomes `media_plane::PushSession`.
5. No DVR deployments exist: no migration, no compatibility path, no legacy reader. Any file that is not in the new
   format is a typed parse error.
6. The per-period index is a standard HLS media playlist with `EXT-X-BYTERANGE` into the period's single media file,
   written and parsed only through `broadcast-hls`.

## 0. Decisions at a glance

| # | Question | Decision | Why |
|---|---|---|---|
| D1 | Core shape | `ArchiveWriter` returns an ordered `Plan` of storage ops per input; the host executes it and calls `complete(PlanOutcome)`; state is committed only on success | the "failed roll leaves the recorder unchanged" and "partial write resyncs the offset" fixes (#1083 D1/W10) become a property of the type, and tests need no disk |
| D2 | Async boundary | dvr-runtime has no `async`, no tokio, no thread, no clock read. The `fs` adapter is blocking and says so. multimux keeps every `spawn_blocking` (route.rs:1537, admin.rs:1617, output/catchup.rs:348) and the scan semaphore | the runtime pattern used across the workspace; `DvrError::ArchiveScan(JoinError)` (dvr.rs:237) leaves the library |
| D3 | Clocks | period-age roll on caller-supplied monotonic `broadcast_common::Timestamp`; wall time (`EXT-X-PROGRAM-DATE-TIME`) is a caller-supplied `jiff::Timestamp` per segment | API-CONVENTIONS: engines never read a clock; replaces `SystemTime::elapsed` (dvr.rs:1162) |
| D4 | Index format | `pN.m3u8`: an HLS media playlist, `PLAYLIST-TYPE:EVENT`, appended one segment record at a time, closed with `EXT-X-ENDLIST` (section 3) | a standard; any HLS tool can read an archive period; crash safety is a rule on line boundaries |
| D5 | Exact timing | one vendor tag per segment, `#EXT-X-RUSTBROADCAST-TIMING:SEQ=..,DTS=..,DUR=..,TIMESCALE=..` (3.4) | `#EXTINF` is a rounded decimal; catch-up needs the exact Trunk timeline values |
| D6 | Programme identity | `EXT-X-DATERANGE` with a reverse-DNS `CLASS`; `pN.event.json` is dropped | every field it held fits in DATERANGE attributes (3.5) |
| D7 | Init changes | an fMP4 init change rolls the period (unchanged behaviour, as `roll_to_init` today); each period holds exactly one init, written first in the media file, referenced by one `EXT-X-MAP` with `BYTERANGE` | owner decision 2026-11-21: keeps every raw pN.m4s playable as a plain file and one init per period |
| D8 | Read cache | owned by an `FsArchive` value per route, not a process-global `static` (catchup.rs:301) | dropping the route's `FsArchive` releases it; `drop_route_cache` (catchup.rs:342) and its admin call (admin.rs:1091) are deleted |
| D9 | Catch-up HLS output | `render_playlist` (catchup.rs:589-664) stays in multimux; dvr-runtime emits format-neutral `CatchupSegment`s | HTTP serving is multimux's |
| D10 | Observability | no `tracing`/`metrics` in dvr-runtime; typed `WriterEvent`s and counters; multimux logs and increments `DVR_*` | same split as the other runtimes |
| D11 | EIT tracker | `dvb_si::epg::PresentEventTracker`, payload `EpgEvent` | one decoder of EIT title/start/duration |
| D12 | Push core | `media_plane::push_session::PushSession` owns the connected egress and returns a `PushDecision`; send errors classified through a new `PushErrorKind` bound on the session's methods only | additive; `PushEgress` unchanged |

## 1. Goal and non-goals

Goal: move the DVR archive (format, writer state machine, read planning) out of multimux into a sans-IO crate that
is testable without a disk or a runtime and is the one owner of the on-disk format; move the EIT programme-boundary
logic to where EIT decoding lives; make the push loop's decisions unit-testable without sockets. multimux stays a
thin adapter.

Non-goals:
- No migration, compatibility shim or legacy reader for archives written by multimux 0.20.
- No change to `SegmentEgress`, `SegmentSink`, `RetentionDriver`, `PushEgress` or `ArchiveOverrun` semantics.
- No change to `ReconnectEngine` timing, URL redaction or log text in multimux.
- No HTTP and no catch-up playlist rendering in dvr-runtime.
- TS data-file recovery stays unimplemented, as today (dvr.rs:1594-1599).

## 2. What moves where

### 2.1 multimux/src/dvr.rs (3927 lines; code 1-1909, tests 1910-3927)

| Item (origin/main) | Destination |
|---|---|
| module doc 1-182 | rewritten for the new format in the `dvr-runtime` crate root and `format` docs; operator `overrun` text stays on `DvrConfig` |
| `DEFAULT_PERIOD_DURATION_SECS` 214 | multimux (serde default) |
| `DvrError` 219-238 | replaced: `dvr_runtime::ArchiveError` / `fs::FsError`; multimux `Error::Dvr` wraps `ArchiveError`; `ArchiveScan(JoinError)` becomes multimux `Error::ArchiveScan` |
| `DvrConfig` 244-281, default fn 283, builders 321-369 | stays in multimux (serde, `ConfigField`) |
| `ArchiveOverrunSerde` 292-319 | stays in multimux |
| `DvrConfig::validate` 373-414 | rules move to `dvr_runtime::ArchiveConfig::validate` (typed `ConfigError`); multimux maps each variant to its `ConfigField` |
| `DvrConfig::retention_active` 424-426 | stays; delegates to `RetentionLimits::is_active` |
| `EitProgramme` 442-460, `from_event` 465-502 | detection and decode: `dvb_si::epg::PresentEvent`; on disk: `EXT-X-DATERANGE` built by `dvr_runtime::format::programme_date_range` |
| `IndexEntry` 512-543, `timestamp_ns` 546, `duration_ns` 560 | deleted; replaced by the playlist record (3.3) and `dvr_runtime::read::ArchivedSegment` |
| `InitGeneration` 578-581 | `dvr_runtime::InitGeneration` |
| `PeriodRecord` 584-589 | `dvr_runtime::writer` (private) |
| `DvrRecorder` struct 601-707 | split: `dvr_runtime::ArchiveWriter` (state) + `dvr_runtime::fs::FsRecorder` (cursor, trunk, files) |
| `archive_seq_floor` 720-733 | multimux keeps the async fn; the scan becomes `FsArchive::seq_floor()` |
| `floor_from_scan` 737-744 | stays in multimux (`spawn_blocking`) |
| `validate_route_dir_name` 751-793 | `dvr_runtime::ArchiveName::new` |
| `DvrRecorder::new` 799-856 | `FsRecorder::new` + `ArchiveWriter::new` |
| `with_seq_offset` 868 | `ArchiveConfig::with_seq_offset` |
| `seed_from_disk` 883-938 | `ArchiveWriter::seed(&[SeedFile])` (decision) + `fs` listing (I/O) |
| `overrun_policy` 941, `current_programme` 947 | `ArchiveWriter::overrun()`, `ArchiveWriter::programme()` |
| `feed_si` 960-998 | carry buffer + SiDemux: `dvb_si::epg::PresentEventTracker::feed_ts`; glue: `FsRecorder::feed_si` |
| `handle_si_event` 1002-1054 | detection to dvb-si; roll and DATERANGE decision to `ArchiveWriter::on_programme` |
| `write_programme_sidecar` 1061-1082 | deleted (DATERANGE in the next segment record) |
| `needs_poll` 1097-1099 | deleted (#3686: no caller) |
| `needs_poll_for_init` 1104-1124 | `ArchiveWriter::needs_poll` |
| `poll_and_persist` 1126-1135 | deleted (only tests call it; they use the `inits` form) |
| `poll_and_persist_inits` 1143-1195, `roll_to_init` 1197-1206 | `ArchiveWriter::on_inits` / `on_tick` / `on_item` + `FsRecorder::poll`; an init change rolls the period (D7, unchanged) |
| `start_period` 1211-1291 | `ArchiveWriter` roll planning |
| `append_segment` 1295-1435 | `ArchiveWriter::on_item` (`Segment` arm) |
| `append_index_entry` 1452-1506, `flush_index` 1517-1551 | deleted; replaced by `StoreOp::CreatePlaylist` / `AppendPlaylist` |
| `rebuild_index` 1569-1649 | `dvr_runtime::recover::scan_fmp4_data(data) -> Vec<RecoveredBox>` (pure; finds the period's single init, then its `moof`+`mdat` spans) + `FsArchive::recover_spans` |
| path helpers 1652-1669 | `dvr_runtime::layout` |
| `per_period_byte_budget` 1688-1693, `enforce_retention` 1695-1725, `evict_oldest_period` 1727-1771 | `ArchiveWriter` retention (eviction is a plan of `Remove` ops) |
| `handle_terminated` 1803-1858 | `ArchiveWriter::on_item(Terminated)` returns `Step::Rearm` / `Step::Stopped`; `FsRecorder` re-pins |
| `impl SegmentEgress for DvrRecorder` 1861-1908 | `impl SegmentEgress for dvr_runtime::fs::FsRecorder` |
| tests 1910-3927 | section 8.1 |

### 2.2 multimux catchup.rs, deleted in PR 5 (1571 lines; code 1-665, tests 666-1571)

| Item | Destination |
|---|---|
| `ArchivedSegment` 49-58 | `dvr_runtime::read::ArchivedSegment` (pub; `ByteSpan` replaces `byte_offset`/`byte_len`; adds `init: Option<ByteSpan>`) |
| `CatchupSegment` 66-74, `InitRef` 83-88 | `dvr_runtime::read::{CatchupSegment<G>, InitRef<G>}`, generic over the live init generation id so `hls_runtime::server::Generation` stays out; `InitRef::Archived { period, span }` |
| `archive_dir` 96-98 | `dvr_runtime::layout::route_dir(root, &ArchiveName)` |
| `list_period_nums` 105-122 | `layout::classify(name) -> Option<ArchiveFile>` (pure) + `FsArchive::periods` |
| `read_period_segments` 142-232, cache 234-308, `index_cache` 334, `prune_index_cache` 368 | `dvr_runtime::fs::FsArchive` (owned cache; same bounds `INDEX_CACHE_MAX_ENTRIES`/`_SEGMENTS` and `(mtime, len, inode)` stamp) |
| `parse_index` 316-328 | deleted; replaced by `read::parse_period_playlist` (3.6) |
| `drop_route_cache` 342-348, `cached_index_entries` 353 | deleted (D8); test observable becomes `FsArchive::cached_periods()` |
| `scan_archive` 381-392 | `FsArchive::scan() -> ArchiveScan` |
| `find_archived_segment` 399-406 | `FsArchive::find(SegmentSeq)` |
| `read_archived_bytes` 421-468 | bound check `read::checked_range(span, file_len)` (pure) + `FsArchive::read` |
| `ReadArchivedError` 472-488 | `dvr_runtime::read::ReadError` (thiserror) |
| `merge_segments` 498-547 | `dvr_runtime::read::merge` |
| `apply_window` 553-578 | `dvr_runtime::read::window` (`Option<Duration>`) |
| `MIN_TARGET_DURATION_SECS`, `DEFAULT_MEDIA_SEQUENCE`, `render_playlist` 580-664 | stay in multimux (moved into `output/catchup.rs`; `catchup.rs` is deleted) |

### 2.3 multimux/src/output/catchup.rs (1609 lines)

Stays. Changes: reads through `FsArchive` inside `run_scan` (340-353, unchanged); `init_uri_of` (116) takes
`InitRef<Generation>` and names an archived init by period (`catchup/init-p{N}.mp4`, unchanged); `catchup_init` (472) reads the period's
`EXT-X-MAP` byte range.

### 2.4 multimux/src/push/mod.rs (970 lines)

| Item | Destination |
|---|---|
| `SendMediaError` 82, `PushTransport` 119, `ReconnectEngine` 291-408 | stay; `SendMediaError` gains `impl media_plane::PushErrorKind` |
| `PushMetrics` 411-441 | stays (atomics), filled from `PushSession::counters()`; `bytes_sent` stays transport-side |
| `log_partial_selection` 444-458 | stays; fed by `PushDecision::Renegotiated` / `Connected { carried, proposed }` |
| `drive_push` 470-735: track-change block (~535-590), loss counting and send (~591-640), give-up (~643), negotiate after connect (~655-690), flush outcome (~695-718) | decisions to `media_plane::PushSession`; `tokio::select!`, the 250 ms listen cap, `PUSH_FLUSH_TIMEOUT`, connect/setup awaits, backoff sleep and redaction stay |

### 2.5 Other crates

- media-plane: new `push_session` module only (section 6).
- dvb-si: new tracker in `epg` (section 5).
- broadcast-hls: prerequisite additions (section 3.8).

## 3. The on-disk format (owned by dvr-runtime, archive format version 1)

### 3.1 Layout

```
<archive_root>/<archive_name>/
  pN.m4s | pN.ts       period media file (N = period number, decimal u32, no padding)
  pN.m3u8              period playlist (the index)
  .pN.m3u8.tmp         transient, during playlist creation only
  .lock                the recorder's single-writer lock; never an archive file
```

Kept from today: one media file per period; period numbers increase for the life of the directory; a new run never
appends to a period an earlier run wrote (`seed`, as dvr.rs:883); retention by whole period; on eviction the playlist
is removed before the media file, so a racing reader never lists a period whose bytes are gone (dvr.rs:1738-1762);
the `ArchiveName` rules of dvr.rs:751-793; byte rolls on `retention_bytes` (dvr.rs:1341-1357); time rolls; EIT rolls.

Changes and their reasons:

| # | Change | Reason |
|---|---|---|
| F1 | index is `pN.m3u8`, an HLS media playlist with `EXT-X-BYTERANGE` | owner decision; standard and tool-readable. The `.m3u8` suffix is what 8216bis §4 requires for a playlist identified by path |
| F2 | the playlist header carries an archive marker tag with `VERSION=1` (3.4) | an explicit format version; a playlist without it is `PlaylistError::NotAnArchive` |
| F3 | exact Trunk timing and the archive sequence number per segment in a vendor tag (3.4) | `#EXTINF` is rounded; catch-up needs exact `start`/`duration` and must verify the sequence |
| F4 | append-only text records, closed by appending `#EXT-X-ENDLIST`; torn-line rule (3.6) | no seek-and-overwrite (dvr.rs:1452-1506), no whole-file rewrite |
| F5 | fMP4 init change rolls the period (unchanged, D7) | one init per period, first in the media file, one `EXT-X-MAP`; a raw period file stays playable |
| F6 | `pN.event.json` dropped; programme identity is an `EXT-X-DATERANGE` | everything it held is expressible (3.5) |
| F7 | media and playlist files opened `create_new` at roll | today `OpenOptions::append(true)` (dvr.rs:1221) would silently continue an existing file |
| F8 | the period also rolls on a sequence gap, on a segment longer than the header's target duration, and when the playlist would exceed `broadcast_hls::limits::MAX_PLAYLIST_BYTES` | HLS numbers segments implicitly from `EXT-X-MEDIA-SEQUENCE`; `#EXT-X-TARGETDURATION` is in the header and is never rewritten; the reader must parse every period with default limits |
| F9 | the playlist is created at the period's first segment, not at roll | `EXT-X-MEDIA-SEQUENCE` is that segment's archive sequence number. A period with an init and no segment has no playlist and is not listed, as today |

### 3.2 Media file

TS: concatenated `SegmentEntry::bytes`. fMP4: the period's one init, then its segments. Every segment and the init are referenced by a byte range in the
playlist. A raw period file stays playable as a file in both containers (TS by concatenation, fMP4 as init plus
fragments).

### 3.3 Playlist `pN.m3u8`

Produced entirely by `broadcast-hls` rendering a `MediaPlaylist` (3.8 adds the incremental renderers). Example
(fMP4, one init, one programme boundary, one discontinuous segment):

```
#EXTM3U
#EXT-X-VERSION:6
#EXT-X-TARGETDURATION:3
#EXT-X-MEDIA-SEQUENCE:120
#EXT-X-PLAYLIST-TYPE:EVENT
#EXT-X-RUSTBROADCAST-ARCHIVE:VERSION=1,CONTAINER=FMP4,PERIOD=4
#EXT-X-DATERANGE:ID="programme-1537-30807",CLASS="io.github.fishloa.rust-broadcast.programme",START-DATE="2024-05-18T18:00:02.000Z",PLANNED-DURATION=3600,END-ON-NEXT=YES,X-DVB-SERVICE-ID=1537,X-DVB-EVENT-ID=30807,X-SCHEDULED-START="2024-05-18T18:00:00Z",X-TITLE="50' inside"
#EXT-X-RUSTBROADCAST-TIMING:SEQ=120,DTS=240000000000,DUR=2000000000,TIMESCALE=1000000000
#EXT-X-MAP:URI="p4.m4s",BYTERANGE="812@0"
#EXT-X-PROGRAM-DATE-TIME:2024-05-18T18:00:02.000Z
#EXTINF:2,
#EXT-X-BYTERANGE:48211@812
p4.m4s
#EXT-X-RUSTBROADCAST-TIMING:SEQ=121,DTS=242000000000,DUR=2000000000,TIMESCALE=1000000000
#EXT-X-DISCONTINUITY
#EXT-X-PROGRAM-DATE-TIME:2024-05-18T18:00:04.000Z
#EXTINF:2,
#EXT-X-BYTERANGE:51002@49023
p4.m4s
#EXT-X-ENDLIST
```

The exact tag order within a record is whatever `MediaPlaylist::to_m3u8` emits for that `MediaSegment` (pre-tags,
discontinuity, map, PDT, `EXTINF`, `BYTERANGE`, URI); the example shows the intent, the golden (3.7) pins the bytes.

Header (written once, at the period's first segment, atomically: temp, `sync_all`, rename, directory `sync_all`):
- `#EXT-X-VERSION`: `broadcast-hls`'s computed version (BYTERANGE needs at least 4; MAP in a non-I-frame playlist
  pushes it higher).
- `#EXT-X-TARGETDURATION`: `ArchiveConfig::target_duration` rounded up, or the first segment's rounded duration if
  larger. A later segment whose rounded duration exceeds it rolls the period first (F8).
- `#EXT-X-MEDIA-SEQUENCE`: the first segment's archive sequence number.
- `#EXT-X-PLAYLIST-TYPE:EVENT`: segments are only appended; retention removes whole files, never playlist lines.
- the archive marker tag (3.4).

Segment record (one append of the lines `render_segment` produces, then `sync_data`):
- `#EXT-X-RUSTBROADCAST-TIMING` (required by the reader).
- `#EXT-X-DATERANGE` when a programme starts or first becomes known (3.5).
- `#EXT-X-DISCONTINUITY` when `SegmentEntry::meta.discontinuous`, and for the first segment of a new run after `seed`.
- `#EXT-X-MAP:URI="pN.m4s",BYTERANGE="<len>@<offset>"` (fMP4 only; on the period's first segment,
  rendered by broadcast-hls).
- `#EXT-X-PROGRAM-DATE-TIME` on every segment when the host supplies wall time. A `DATERANGE` requires a PDT in the
  playlist (8216bis §4.4.5.1), so the writer refuses `programme_service` without wall time (`ConfigError`).
- `#EXTINF:<seconds>,` from `DecimalSeconds::from_duration`.
- `#EXT-X-BYTERANGE:<len>@<offset>`, offset always explicit.
- URI `pN.m4s` / `pN.ts` (relative, the period's own media file).

Close: when the period rolls, `#EXT-X-ENDLIST` is appended (one append, `sync_data`). A period left without it (a
crash, or the open period) reads as an open `EVENT` playlist.

Write order per segment (one plan): `AppendData{expect_len}` (the init first, for the period's first segment), `SyncData`,
`AppendPlaylist{expect_len}` (or `CreatePlaylist` for the period's first segment), retention `Remove`s. A crash after
the media bytes and before the record leaves unreferenced tail bytes, which nothing reads because the period is never
reopened. `expect_len`: the store refuses with `StoreFailure::LengthMismatch{actual}` if the file is not that long and
truncates back to `expect_len` after a partial write; the writer resyncs from `actual` (replaces dvr.rs:1268-1294).

### 3.4 Vendor tags

Names: `#EXT-X-RUSTBROADCAST-ARCHIVE` (playlist level) and `#EXT-X-RUSTBROADCAST-TIMING` (per segment).
Justification: 8216bis §4.1 makes every line that begins with `#EXT` a tag and §6.3.1 requires clients to ignore
tags they do not recognise, so standard players skip both. 8216bis defines no registry or namespace for vendor tags;
following its reverse-DNS guidance for `X-` attributes (§4.4.5.1), the name carries a full vendor token
(`RUSTBROADCAST`) rather than a short prefix such as `RB`, which is more likely to collide with another vendor or a
future standard tag. The `EXT-X-` prefix matches every other tag in the file. Values use the standard attribute-list
syntax (§4.2) with `decimal-integer` and `enumerated-string` values, so `broadcast_hls::parse_attribute_list_strict`
reads them.

`#EXT-X-RUSTBROADCAST-ARCHIVE:VERSION=1,CONTAINER=FMP4|TS,PERIOD=<N>`: `VERSION` other than `1` is
`PlaylistError::UnsupportedVersion`; `PERIOD` must equal the `N` of the file name.

`#EXT-X-RUSTBROADCAST-TIMING:SEQ=<s>,DTS=<t>,DUR=<d>,TIMESCALE=<ts>`:
- `SEQ`: archive sequence number (Trunk sequence + `seq_offset`); the reader requires `SEQ == EXT-X-MEDIA-SEQUENCE +
  index` (F8 guarantees it) and fails with `PlaylistError::SequenceMismatch` otherwise.
- `DTS`: `SegmentEntry::timeline_position` in `TIMESCALE` ticks.
- `DUR`: `SegmentEntry::duration` in `TIMESCALE` ticks; a duration that does not fit `u64` ticks is refused at write.
- `TIMESCALE`: `1000000000` in version 1 (the Trunk timeline is nanoseconds, so this is exact); non-zero required.
A segment without exactly one timing tag is `PlaylistError::MissingTiming { index }`.

### 3.5 Programme boundaries

`EXT-X-DATERANGE`, built by `dvr_runtime::format::programme_date_range(&PresentEvent, start: ProgramDateTime)` and
rendered by broadcast-hls:
- `ID="programme-<service_id>-<event_id>"` plus `-<n>` if the same event reappears in one playlist (IDs are unique
  per playlist with distinct attributes).
- `CLASS="io.github.fishloa.rust-broadcast.programme"`, `END-ON-NEXT=YES` (needs `CLASS`): a programme ends where the
  next one starts.
- `START-DATE`: PDT of the segment the record is attached to (the observed boundary).
- `PLANNED-DURATION`: the EIT duration in seconds, when decodable.
- `X-DVB-SERVICE-ID`, `X-DVB-EVENT-ID`: decimal.
- `X-SCHEDULED-START`: the EIT start time, quoted RFC 3339, when decodable.
- `X-TITLE`: quoted-string when the title contains no `"`, CR, LF or other control character (8216bis §4.1);
  otherwise `X-TITLE-UTF8=0x<hex>` (a hexadecimal-sequence, lossless). Exactly one of the two, or neither.
When a programme transition rolls the period (unchanged behaviour), the DATERANGE goes on the new period's first
segment; any other roll repeats the current programme's DATERANGE on the new period's first segment.

### 3.6 Reader and the torn-line rule

`read::parse_period_playlist(period: PeriodNum, container: ArchiveContainer, bytes: &[u8]) -> Result<PeriodPlaylist,
PlaylistError>`:
1. Drop trailing NUL bytes (a crash can leave a zero-filled tail).
2. Cut after the last `\n`. Bytes after it are a torn line; reported as `PeriodPlaylist::torn_bytes`.
3. Parse with `broadcast_hls::MediaPlaylist::parse_with_options` (3.8) with the timing tag declared segment-scoped.
4. A trailing incomplete record (tags whose URI line never landed) appears as broadcast-hls's `open_segment` or as a
   `DanglingSegmentTags` diagnostic (3.8); it is discarded and counted in `torn_records`.
5. Validate: archive marker present and matching; every segment has its URI equal to the period's media file name,
   an explicit `BYTERANGE` offset, exactly one timing tag with `SEQ` continuous; spans ascending and non-overlapping;
   fMP4 segments carry a `MAP` with a byte range; no `GAP`, no `EXT-X-KEY`, no parts. Any failure is a typed
   `PlaylistError` and makes that one period unreadable (`ArchiveScan::unreadable`); other periods are unaffected.
6. A span past the media file's real length is checked at read time (`ReadError::RangeBeyondFile`, as
   catchup.rs:441-454).

A non-archive file named `pN.m3u8` fails at step 3 or 5 with a typed error. There is no other legacy handling.

### 3.7 Format tests

- `format::tests::period_playlist_golden`: the writer's ops for a fixed input (fMP4, one init, one
  discontinuity, one DATERANGE with a hex title) concatenate to the committed new golden `p4.m3u8` under the dvr-runtime golden fixtures topic
  byte-for-byte; `parse_period_playlist` reproduces every input value; `MediaPlaylist::parse(golden).to_m3u8()` equals
  the golden (header + appended records == one full render).
- new integration test `writer_reader_round_trip::fs_recorder_output_reads_back_exactly`: `FsRecorder` writes several periods;
  `FsArchive::scan` returns each segment's seq/start/duration/discontinuity/init span, and `FsArchive::read` returns
  the exact `SegmentEntry::bytes`.
- `torn_tail_every_byte`: truncate the golden at every byte and append NULs; the reader returns exactly the complete
  records before the cut and never errors.
- `a_playlist_without_the_archive_marker_is_not_an_archive`.
- new integration test `hls_oracle::archive_playlists_pass_mediastreamvalidator`: a closed TS period and a closed fMP4 period
  recorded from real segments (8.3) validate with Apple `mediastreamvalidator` through `test_fixtures::process::msv`;
  zero errors required; the vendor tags may only appear as unknown-tag warnings, which the test lists explicitly.
  Skips loudly when the tool is absent (macOS-only, as `media-doctor`'s oracle test).
- Fuzz target `dvr_playlist` over `parse_period_playlist`.

### 3.8 Prerequisite: broadcast-hls additions

broadcast-hls 0.8.1 already round-trips `EXT-X-BYTERANGE` (`MediaSegment::byte_range`), `EXT-X-MAP` with
`BYTERANGE` (`MapTag`), `EXT-X-PROGRAM-DATE-TIME` (`ProgramDateTime`), `EXT-X-DISCONTINUITY`, `EXT-X-ENDLIST`,
`EXT-X-PLAYLIST-TYPE`. It lacks:

| # | Addition | Why | Class |
|---|---|---|---|
| H1 | `DateRange` (typed `EXT-X-DATERANGE`: `id`, `class`, `start: ProgramDateTime`, `end`, `duration`, `planned_duration`, `end_on_next`, `cue`, `client_attrs: Vec<(ClientAttrName, AttrValue)>`) with `to_tag_line()` / `parse_tag_line()`; `ClientAttrName` validates the `X-` prefix and AttributeName characters | today a DATERANGE is only a verbatim `pre_tags` string; dvr-runtime must not build tag text | additive |
| H2 | `CustomTag { name: CustomTagName, attrs: Vec<(String, AttrValue)> }` with `to_tag_line()` / `parse_tag_line()`; `CustomTagName` requires `EXT-` and tag-name characters | the vendor tags, built and read by broadcast-hls | additive |
| H3 | `ParseOptions { segment_scoped_tags: Vec<CustomTagName> }` (`#[non_exhaustive]`, `Default`) and `MediaPlaylist::parse_with_options(input, &Limits, &ParseOptions)` | today an unknown tag goes to playlist-level `extra_tags` (lib.rs:2369-2373, `is_segment_defining_tag` lib.rs:3598) and loses its position; `Limits` is `Copy` so it cannot take a `Vec` without breaking | additive |
| H4 | `MediaPlaylist::render_header()`, `render_segment(i)`, `render_endlist()`; invariant `header + Σ render_segment(i) + endlist == to_m3u8()` pinned by a property test | append-only writing without hand-built text | additive |
| H5 | `ParseDiagnosticKind::DanglingSegmentTags` (the enum is `#[non_exhaustive]`): an `#EXTINF`/`#EXT-X-BYTERANGE` with no URI at end of input is reported, not silently dropped | step 4 of the torn-line rule needs to see it | additive |

All five are additive. Follow-up (separate issue, not blocking): `timed-metadata::DateRange` (timed-metadata
`src/daterange.rs:61`) duplicates H1 and should move onto it.

## 4. dvr-runtime public API sketch

### 4.1 Crate facts

- `std` only (no `no_std` claim): the writer reuses `media_plane::{SegmentEntry, SegmentCursorItem, ArchiveOverrun}`
  and `media_plane::ids::SegmentSeq`, which are `std`-gated. The core does no I/O.
- Feature `fs` (off by default): `fs::{FsRecorder, FsArchive, FsError}` over `std::fs`; blocking.
- Dependencies (the release wave's versions; all five siblings move an epoch in it):

```toml
broadcast-common = { path = "../broadcast-common", version = "14.0.0", features = ["std"] }
broadcast-hls    = { path = "../broadcast-hls",    version = "0.9.0",  features = ["std"] }   # H1-H5
media-plane      = { path = "../media-plane",      version = "0.14.0", default-features = false, features = ["std"] }
dvb-si           = { path = "../dvb-si",           version = "16.0.0", default-features = false, features = ["std", "jiff", "ts"] }
transmux         = { path = "../transmux",         version = "0.33.0", default-features = false, features = ["std"] }  # parse_box for recover
bytes            = "1"
jiff             = "1"
thiserror        = "2"
[dev-dependencies]
test-fixtures = { path = "../test-fixtures" }
```

  No tokio, tracing, metrics, parking_lot, serde or serde_json (no JSON anywhere in the format).
- Version 0.1.0, independently versioned, unpublished until its release lane (`release-dvr-runtime.yml`, tag
  `dvr-runtime-v*`) runs.

### 4.2 Types

```rust
#[non_exhaustive] pub struct ArchiveName(String);          // ArchiveName::new(&str) -> Result<Self, ConfigError>
pub const MAX_ARCHIVE_NAME_LEN: usize = 255;
#[non_exhaustive] pub enum ArchiveContainer { Fmp4, Ts }  // name(): "FMP4" | "TS" (the marker tag token); impl_spec_display!; ext()
#[non_exhaustive] pub struct PeriodNum(u32);              // checked next()

#[non_exhaustive]
pub struct ArchiveConfig {
    pub container: ArchiveContainer,
    pub target_duration: Duration,               // the route segmenter's target
    pub period_duration: Option<Duration>,       // None: no time roll (today's 0)
    pub retention: RetentionLimits,
    pub overrun: ArchiveOverrun,
    pub seq_offset: u64,
    pub programme_service: Option<u16>,          // today's dvb_service_id; requires wall time
}
impl ArchiveConfig {
    pub fn new(container: ArchiveContainer, target_duration: Duration, retention: RetentionLimits) -> Self;
    pub fn with_period_duration(self, d: Option<Duration>) -> Self;  // + with_overrun, with_seq_offset, with_programme_service
    pub fn validate(&self) -> Result<(), ConfigError>;
}
#[non_exhaustive]
pub struct RetentionLimits { pub periods: Option<NonZeroUsize>, pub bytes: Option<NonZeroU64> }
impl RetentionLimits { pub fn is_active(&self) -> bool; pub fn per_period_byte_budget(&self) -> Option<NonZeroU64>; }

pub struct InitGeneration { pub first_seq: SegmentSeq, pub init: Bytes }

#[non_exhaustive] pub enum RollReason { First, Duration, ByteBudget, Programme, SequenceGap, TargetDuration, PlaylistFull, InitChanged }
#[non_exhaustive] pub enum WriterEvent {
    PeriodOpened { period: PeriodNum, reason: RollReason },
    PeriodClosed { period: PeriodNum },
    Evicted { period: PeriodNum },
    EvictionFailed { period: PeriodNum },
    Gap { skipped: u64 },
    Rearmed { skipped: u64 },
    Terminated,
    SkippedBeforeInit { seq: SegmentSeq },
}
pub struct WriterCounters { pub gaps: u64, pub segments: u64, pub bytes: u64 }
```

`ArchiveContainer` and `RollReason` get `name()` + `impl_spec_display!` and a `label` table line; `WriterEvent`,
`StoreOp`, `Step`, `PushDecision` and the error enums go on the documented SKIP list.

### 4.3 Sans-IO writer

```rust
pub struct ArchiveWriter { /* closed periods, open period, offsets, init generation, programme, pending plan */ }

impl ArchiveWriter {
    pub fn new(config: ArchiveConfig) -> Result<Self, ConfigError>;
    pub fn seed(&mut self, existing: &[SeedFile]) -> Result<(), ArchiveError>;      // once, before the first plan
    pub fn note_stale_temp_playlists(&mut self, periods: &[PeriodNum]) -> Result<(), ArchiveError>;
    pub fn needs_poll(&self, latest_closed: Option<SegmentSeq>, latest_init: Option<&[u8]>) -> bool;
    pub fn on_inits(&mut self, inits: &[InitGeneration]) -> Result<(), ArchiveError>;   // queues; no I/O by itself
    pub fn on_tick(&mut self, now: Timestamp) -> Result<Step, ArchiveError>;               // time roll
    pub fn on_item(&mut self, now: Timestamp, wall: Option<jiff::Timestamp>, item: &SegmentCursorItem)
        -> Result<Step, ArchiveError>;
    pub fn on_programme(&mut self, change: PresentEventChange) -> Result<Step, ArchiveError>;
    pub fn set_latest_closed(&mut self, latest: Option<SegmentSeq>);                    // for the re-arm gap size
    pub fn complete(&mut self, plan: PlanId, outcome: PlanOutcome) -> Result<(), ArchiveError>;
    pub fn last_input_consumed(&self) -> bool;      // false: the last input needs retrying
    pub fn poll_event(&mut self) -> Option<WriterEvent>;
    pub fn overrun(&self) -> ArchiveOverrun;
    pub fn programme(&self) -> Option<&PresentEvent>;
    pub fn counters(&self) -> WriterCounters;
}

#[non_exhaustive] pub enum Step { Idle, Execute(Plan), Rearm { skipped: u64 }, Stopped }
pub struct Plan { pub id: PlanId, pub ops: Vec<StoreOp> }
#[non_exhaustive] pub enum StoreOp {
    CreateDir,
    CreateData { period: PeriodNum },                                   // create_new
    AppendData { period: PeriodNum, expect_len: u64, bytes: Bytes },    // init generation or segment
    SyncData { period: PeriodNum },
    CreatePlaylist { period: PeriodNum, text: String },                 // atomic; header + first record, from broadcast-hls
    AppendPlaylist { period: PeriodNum, expect_len: u64, text: String },// one record, or #EXT-X-ENDLIST
    Remove { file: ArchiveFile },                                       // NotFound counts as done
}
#[non_exhaustive] pub enum PlanOutcome { Done, Failed { at_op: usize, failure: StoreFailure } }
#[non_exhaustive] pub enum StoreFailure {
    LengthMismatch { actual: u64 },
    Io(std::io::ErrorKind),
    NotDurable(std::io::ErrorKind),              // effect in place, durability failed: counts as done
}
```

Every `text` is produced by broadcast-hls (`render_header`/`render_segment`/`render_endlist`). Contract: while a plan
is outstanding every `on_*` returns `ArchiveError::PlanPending`; `Done` commits; `Failed` commits nothing except a
`LengthMismatch` resync; a failed `Remove` keeps the period in the ledger and emits `EvictionFailed`.

`on_item` decisions (today's behaviour plus F8):

| Item | Condition | Step / state |
|---|---|---|
| any | writer stopped | `Idle` |
| `Segment` | fMP4, no init yet | `Idle` + `SkippedBeforeInit` |
| `Segment` | seq + offset > u32, or duration ticks > u64 | `Err` before any op |
| `Segment` | `programme_service` set and `wall` is `None` | `Err(ArchiveError::WallClockRequired)` |
| `Segment` | no open period | roll (`First`) |
| `Segment` | seq is not previous + 1 | close + roll (`SequenceGap`) |
| `Segment` | rounded duration > header target | close + roll (`TargetDuration`) |
| `Segment` | byte budget set, period has a segment, offset + len > budget | close + roll (`ByteBudget`) |
| `Segment` | record would push the playlist past `MAX_PLAYLIST_BYTES` | close + roll (`PlaylistFull`) |
| `Segment` | fMP4, init generation for this seq differs from the open period's init | close + roll (`InitChanged`): new media file, init first, then the segment |
| `Segment` | otherwise | `AppendData`, `SyncData`, `AppendPlaylist`/`CreatePlaylist`, retention `Remove`s |
| `Gap{n}` / `Lagged{n}` | | `gaps += n`, `Gap` event (the next segment then rolls on `SequenceGap`) |
| `Terminated` | overrun `StallIngest` | `Rearm{skipped}` (distance from the last appended seq to `latest_closed`, `checked_distance_from`) |
| `Terminated` | `Terminate` or other | `Stopped`, `Terminated` event |

`on_tick`: roll (`Duration`) when the open period's age on the caller's monotonic clock reaches `period_duration`.
`on_programme`: `Baseline` attaches a DATERANGE to the next record; `Transition` closes + rolls (`Programme`) if a
period is open, else only attaches. Retention after each committed append: evict oldest closed while
`closed + open bytes > bytes`; then while `closed periods > periods`. Budget `bytes / (periods + 1)`, min 1
(dvr.rs:1688-1693).

### 4.4 Sans-IO reader

```rust
pub mod read {
    pub struct ArchivedSegment { pub seq: SegmentSeq, pub start: Timestamp, pub duration: Duration,
                                 pub discontinuous: bool, pub period: PeriodNum, pub span: ByteSpan,
                                 pub init: Option<ByteSpan>, pub wall: Option<jiff::Timestamp> }
    pub struct ArchivedProgramme { pub period: PeriodNum, pub first_seq: SegmentSeq, pub service_id: u16,
                                   pub event_id: u16, pub title: Option<String>,
                                   pub scheduled_start: Option<jiff::Timestamp>, pub planned: Option<Duration> }
    pub struct PeriodPlaylist { pub segments: Vec<ArchivedSegment>, pub programmes: Vec<ArchivedProgramme>,
                                pub closed: bool, pub torn_bytes: usize, pub torn_records: usize }
    pub fn parse_period_playlist(period: PeriodNum, container: ArchiveContainer, bytes: &[u8])
        -> Result<PeriodPlaylist, PlaylistError>;
    pub enum InitRef<G> { Archived { period: PeriodNum, span: ByteSpan }, Live(G) }
    pub struct LiveSegment<G> { pub seq: SegmentSeq, pub start: Timestamp, pub duration: Duration,
                                pub discontinuous: bool, pub init: G }
    pub struct CatchupSegment<G> { pub seq: SegmentSeq, pub start: Timestamp, pub duration: Duration,
                                   pub discontinuous: bool, pub init: Option<InitRef<G>> }
    pub fn merge<G: Copy>(archived: &[ArchivedSegment], live: &[LiveSegment<G>]) -> Vec<CatchupSegment<G>>;
    pub fn window<G: Copy>(segments: &[CatchupSegment<G>], window: Option<Duration>) -> Vec<CatchupSegment<G>>;
    pub fn checked_range(span: ByteSpan, file_len: u64) -> Result<ByteSpan, ReadError>;
    pub struct ArchiveScan { pub segments: Vec<ArchivedSegment>, pub unreadable: Vec<(PeriodNum, PlaylistError)>,
                             pub conflicts: Vec<SeqConflict> }
    pub struct SeqConflict { pub seq: SegmentSeq, pub kept: PeriodNum, pub dropped: PeriodNum }
}
pub mod layout {
    pub enum ArchiveFile { Media(PeriodNum), Playlist(PeriodNum) }
    pub struct SeedFile { /* ArchiveFile + the bytes it occupies, for retention */ }
    pub fn classify(name: &str, container: ArchiveContainer) -> Option<ArchiveFile>;
    pub fn file_name(file: ArchiveFile, container: ArchiveContainer) -> String;
    pub const LOCK_FILE_NAME: &str;
}
pub mod recover {
    pub enum RecoveredBox { Init(ByteSpan), Fragment(ByteSpan) }
    pub fn scan_fmp4_data(data: &[u8]) -> Result<Vec<RecoveredBox>, RecoverError>;
}
```

`ArchiveScan::unreadable` replaces "log and skip" (catchup.rs:179-199); multimux logs and counts it. A sequence
number two periods claim is reported in `conflicts` and the earlier period's record is kept, so a run that restarted
at too low an offset is visible instead of silently hiding segments. `ArchivedProgramme` gives catch-up a
per-programme listing for free (not wired to HTTP in this work). `merge` marks every sequence jump discontinuous,
inside the archive as well as at the archive-to-live boundary.

### 4.5 `fs` feature

```rust
pub struct FsRecorder { /* ArchiveWriter, SegmentCursor, Arc<Trunk>, dir, lock file, Option<PresentEventTracker> */ }
impl FsRecorder {
    pub fn new(root: &Path, name: ArchiveName, config: ArchiveConfig, trunk: &Arc<Trunk>) -> Result<Self, FsError>;
    pub fn with_cursor(root: &Path, name: ArchiveName, config: ArchiveConfig, trunk: &Arc<Trunk>,
                       cursor: &mut Option<SegmentCursor>) -> Result<Self, FsError>;
    pub fn succeed(self, config: ArchiveConfig, trunk: &Arc<Trunk>,
                   cursor: &mut Option<SegmentCursor>) -> Result<Self, FsError>;
    pub fn needs_poll(&self, latest_closed: Option<SegmentSeq>, latest_init: Option<&[u8]>) -> bool;
    pub fn poll(&mut self, now: Timestamp, wall: impl Fn(&SegmentEntry) -> Option<jiff::Timestamp>,
                inits: &[InitGeneration]) -> Result<(), FsError>;           // blocking
    pub fn feed_si(&mut self, ts: &[u8]) -> Result<(), FsError>;             // blocking (may roll)
    pub fn on_programme(&mut self, change: PresentEventChange) -> Result<(), FsError>;
    pub fn poll_event(&mut self) -> Option<WriterEvent>;
    pub fn writer(&self) -> &ArchiveWriter;
}

pub struct FsArchive { /* dir, container, owned LRU cache keyed by (mtime, len, inode) */ }
impl FsArchive {
    pub fn open(root: &Path, name: &ArchiveName, container: ArchiveContainer) -> Self;
    pub fn scan(&mut self) -> Result<ArchiveScan, ReadError>;
    pub fn find(&mut self, seq: SegmentSeq) -> Result<Option<ArchivedSegment>, ReadError>;
    pub fn read(&self, segment: &ArchivedSegment) -> Result<Bytes, ReadError>;
    pub fn read_init(&self, period: PeriodNum, span: ByteSpan) -> Result<Bytes, ReadError>;
    pub fn seq_floor(&mut self) -> Result<Option<SegmentSeq>, ReadError>;
    pub fn recover_spans(&self, period: PeriodNum) -> Result<Vec<RecoveredBox>, ReadError>;
    pub fn cached_periods(&self) -> usize;
}
```

The directory is guarded by an exclusive advisory lock on `.lock` (`File::try_lock`), taken by `new` and held until
the recorder drops; `succeed` moves it to the successor run without releasing it, and `with_cursor` keeps the host's
cursor when the lock refuses the create. `poll` is the only driver: `FsRecorder` deliberately does not implement
`media_plane::egress::SegmentEgress`, because one cursor item carries no monotonic time, wall time or init
generation, so a period could never roll on `period_duration` and an fMP4 archive would drop every segment as
arriving before its init.

A listing or a period playlist that exists but cannot be read is a typed `ReadError`, never an empty archive or a
zero floor: a restarted route that could not list its archive must not number over it. `seq_floor` is `None` for an
archive with no segment, and fails when a period playlist could not be read. Period playlists are read at most
`MAX_PLAYLIST_BYTES + 1` bytes.

multimux holds one `FsArchive` per route in a `parking_lot::Mutex` on `RouteHandle` and locks it inside `run_scan`'s
blocking closure. `wall` is how multimux supplies PDT (its existing `ProgramDateTimeSource`).

### 4.6 Errors

```rust
#[derive(Debug, thiserror::Error)] #[non_exhaustive]
pub enum ConfigError { EmptyName, TraversalName, NameTooLong { len: usize, max: usize }, SeparatorInName,
                       NoRetention, UnboundedPeriod, ZeroTargetDuration }
pub enum ArchiveError { Config(#[from] ConfigError), PlanPending, UnknownPlan { id: PlanId },
                        SeqOverflow { trunk_seq: SegmentSeq, offset: u64 }, DurationOverflow { seq: SegmentSeq },
                        PeriodNumbersExhausted, EmptySegment { seq: SegmentSeq }, WallClockRequired,
                        Render(#[from] broadcast_hls::Error) }
pub enum PlaylistError { Hls(#[from] broadcast_hls::Error), NotAnArchive, UnsupportedVersion { found: u64 },
                         ContainerMismatch, PeriodMismatch { tag: u32, file: u32 },
                         MissingTiming { index: usize }, MalformedTiming { index: usize },
                         SequenceMismatch { index: usize }, ForeignUri { index: usize },
                         MissingOffset { index: usize }, Overlap { index: usize }, MissingMap { index: usize },
                         Unsupported { index: usize, tag: UnsupportedTag }, RecordAfterEndlist, DuplicateEndlist,
                         NotFound, Io { kind: std::io::ErrorKind } }
pub enum ReadError { Gone { period: PeriodNum }, RangeBeyondFile { span: ByteSpan, file_len: u64 },
                     Playlist(#[from] PlaylistError), Recover(#[from] RecoverError), Io { kind: std::io::ErrorKind } }
// fs feature:
pub enum FsError { Archive(#[from] ArchiveError), Io { op: StoreOpKind, path: PathBuf, #[source] source: std::io::Error },
                   Locked { path: PathBuf } }
```

No `String` reasons (today's `DvrError::Config { reason: String }` at dvr.rs:222-227 goes).

### 4.7 Amendments made while shipping 0.1.0

| # | Amendment | Reason |
|---|---|---|
| A1 | `WriterEvent::{NotDurable, StaleTempPlaylistRemoved}`, `StoreFailure::NotDurable` and `ArchiveError::StaleTempPlaylist` | a playlist renamed into place whose directory sync failed is in place: counting it as failed would archive the segment twice, and a `.pN.m3u8.tmp` left by a crash is removed and reported before its period number is reused |
| A2 | `ArchiveWriter::last_input_consumed` | a store failure can be repaired in place (a `LengthMismatch` truncation), so the recorder learns from the writer whether the input still needs retrying instead of guessing |
| A3 | `FsRecorder::{with_cursor, succeed}` and `on_programme` | a host that replaces a recorder (a reconnect) must not miss a segment closed in between, and must hand the directory lock over without a release window; a programme change detected before the recorder existed applies the same way as a fed EIT section |
| A4 | `FsRecorder` is not a `SegmentEgress` | see 4.5: one cursor item carries no clock and no init generation, so the adapter could never time-roll and would drop every fMP4 segment |
| A5 | `ArchiveScan::conflicts` / `SeqConflict` | a run that restarted below `seq_floor` collided with archived sequences; the collision is reported instead of hiding segments |
| A6 | `PlaylistError::{MalformedTiming, RecordAfterEndlist, DuplicateEndlist, NotFound, Io}`, `ReadError::Recover`, `FsError::Locked` | naming each failure the reader can meet: a timing tag that does not decode, RFC 8216bis §4.3.3.4's endlist rule, a playlist evicted between listing and reading, a recovery scan that refused the file, and a second recorder refused the directory |
| A7 | the `.lock` file (3.1) | one writer per archive directory; the kernel releases it on exit, so there is no stale-lock state |

## 5. dvb-si EIT programme tracker (`dvb_si::epg`, feature `jiff`; `feed_ts` also needs `ts`)

```rust
#[non_exhaustive] pub struct PresentEvent { pub service_id: u16, pub event: EpgEvent }
#[non_exhaustive] pub enum PresentEventChange {
    Baseline(PresentEvent),
    Transition { previous_event_id: u16, current: PresentEvent },
}
pub struct PresentEventTracker { /* service_id, last event_id, SiDemux on the EIT PID only, carry buffer, bounded queue */ }
impl PresentEventTracker {
    pub fn new(service_id: u16) -> Self;
    pub fn on_section(&mut self, section: &EitSection<'_>) -> Option<PresentEventChange>;
    #[cfg(feature = "ts")] pub fn feed_ts(&mut self, ts: &[u8]);     // any alignment; carry-over as dvr.rs:964-995
    pub fn poll_change(&mut self) -> Option<PresentEventChange>;
    pub fn changes_dropped(&self) -> u64;                            // PRESENT_CHANGE_QUEUE_CAP = 16
    pub fn current(&self) -> Option<&PresentEvent>;
}
```

Rules (from dvr.rs:1002-1054): only `EitKind::PresentFollowingActual` with matching `service_id`; the present event is
the one with `running_status == Running`, not the first in the loop; the same `event_id` again yields nothing
(#3677); SiDemux's version gate suppresses repeats. `EpgEvent` gains a crate-private constructor from
`tables::eit::EitEvent` sharing `extract_short_event` and friends with `event_to_epg` (epg.rs:749). Additive: dvb-si
minor. Both new types are data-carrying: `label` SKIP list.

## 6. media-plane PushSession (`media_plane::push_session`, feature `std`)

```rust
pub trait PushErrorKind { fn kind(&self) -> PushErrorClass; }
#[non_exhaustive] pub enum PushErrorClass { DropItem, Reconnect }    // name() + impl_spec_display!

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)] #[non_exhaustive]
pub struct PushCounters { pub samples_sent: u64, pub samples_dropped: u64, pub connects: u32 }

pub struct PushSession<E> { /* connected: Option<E>, pending: Option<(E, u64, usize, usize)>, negotiated: Option<u64>, counters */ }

impl<E: PushEgress> PushSession<E> where E::Error: PushErrorKind {
    pub fn new() -> Self;
    pub fn on_item(&mut self, item: &SampleCursorItem) -> PushDecision<E>;
    pub fn on_connected(&mut self, egress: E, generation: u64, tracks: &[TrackSpec]) -> PushDecision<E>;
    pub fn pending_setup_mut(&mut self) -> Option<&mut E>;          // multimux awaits transport setup on it
    pub fn on_setup(&mut self, ok: bool) -> PushDecision<E>;
    pub fn egress_mut(&mut self) -> Option<&mut E>;                 // for the async flush
    pub fn on_flush(&mut self, outcome: FlushResult) -> PushDecision<E>;
    pub fn on_give_up(&mut self) -> PushDecision<E>;
    pub fn is_connected(&self) -> bool;
    pub fn counters(&self) -> PushCounters;
}
#[non_exhaustive] pub enum FlushResult { Flushed, Failed, TimedOut, Cancelled }
#[non_exhaustive] pub enum PushDecision<E: PushEgress> {
    Continue,
    Renegotiated { carried: usize, proposed: usize },
    RenegotiateRefused { reason: &'static str },
    MuxDropped(E::Error),
    NeedsSetup,
    Connected { carried: usize, proposed: usize },
    Reconnect { egress: E, cause: ReconnectCause<E::Error> },   // egress handed back so the caller closes it
    Backoff { egress: E, cause: Option<E::Error> },          // the failed egress, handed back
    Redundant { egress: E },                                 // on_connected while connected or pending
    Stop { egress: Option<E> },
}
#[non_exhaustive] pub enum ReconnectCause<Er> { Send(Er), Renegotiate(Er), Flush, FlushTimeout, Setup }
```

Decision table (today's `drive_push`, push/mod.rs:535-718):

| Input | State | Egress call / result | Counters | Decision |
|---|---|---|---|---|
| `TracksLagged{skipped}` | any | | dropped += skipped | then as `TracksChanged` |
| `TracksChanged{g}` | not connected, or `negotiated >= g` | | | `Continue` |
|  | connected, `negotiated` None or `< g` | `renegotiate` → `Accepted(sel)` | negotiated = g | `Renegotiated{carried, proposed}` |
|  |  | `Refused{reason}` | negotiated = g | `RenegotiateRefused{reason}` |
|  |  | `Error(e)` | negotiated = None | `Reconnect{Renegotiate(e)}` |
| `Lagged{n}` / `Degraded{n}` | any | then as the rows below | dropped += n | |
| `Timed` / `Sparse` | not connected | | dropped += 1 | `Continue` |
| other item | not connected | | | `Continue` |
| non-track item | connected | `send` → `Ok` | `Timed`/`Sparse`: sent += 1 | `Continue` |
|  | connected | `Err(e)`, `DropItem` | dropped += 1 | `MuxDropped(e)` |
|  | connected | `Err(e)`, `Reconnect` | negotiated = None | `Reconnect{Send(e)}` |
| `on_connected(egress, g, tracks)` | not connected | `negotiate` → `Accepted` | | `NeedsSetup` (egress held pending) |
|  |  | `Error(e)` / other | handed back | `Backoff{egress, Some(e)}` / `Backoff{egress, None}` |
| `on_connected(egress, ..)` | connected or pending | handed back | | `Redundant{egress}` |
| `on_setup(true)` | pending | becomes connected | connects += 1, negotiated = g | `Connected{carried, proposed}` |
| `on_setup(false)` | pending | handed back | | `Reconnect{Setup}` |
| `on_flush(Flushed)` | connected | | | `Continue` |
| `on_flush(Failed)` / `(TimedOut)` | connected | handed back | negotiated = None | `Reconnect{Flush}` / `Reconnect{FlushTimeout}` |
| `on_flush(Cancelled)` | any | handed back | | `Stop` |
| `on_give_up()` | any | handed back | | `Stop` (the caller has drained the cursor) |

multimux keeps the cancel `select!`s, `trunk.listen()` with the 250 ms cap and `waiter_slot_freed`, `T::connect`,
`setup().await`, `flush_transmit_bounded(PUSH_FLUSH_TIMEOUT)`, `ReconnectEngine` (`on_connect` on `Connected`,
`on_disconnect` on `Reconnect`/`Backoff`), `log_partial_selection`, redaction, and `PushMetrics` (stored from
`counters()` plus transport `bytes_sent`). `SendMediaError` implements `PushErrorKind` (`Mux` → `DropItem`,
`Transport` → `Reconnect`).

## 7. multimux after the move; version impact

multimux keeps: `dvr::DvrConfig`, `ArchiveOverrunSerde`, `DEFAULT_PERIOD_DURATION_SECS`, the `ConfigField` mapping
(`EmptyName|TraversalName|NameTooLong|SeparatorInName` → `RouteName`, `NoRetention` → `Retention`,
`UnboundedPeriod` → `PeriodDurationSecs`; the empty `archive_root` check stays in multimux → `ArchiveRoot`),
`DvrConfig::to_archive_config(container, target_duration, seq_offset) -> Result<ArchiveConfig, Error>`, route wiring
(route.rs:1047, 1511-1612, 1616-1632, 2982), the async `archive_seq_floor`, the catch-up HTTP output and
`render_playlist`, the `DVR_*` metrics driven from `WriterEvent`s, `drive_push`'s async shell, admin.

Re-exports: none. `multimux::dvr` keeps only the config types; a consumer that needs archive types depends on
dvr-runtime. Removed from multimux's public API: `DvrRecorder`, `IndexEntry`, `EitProgramme`, `InitGeneration`,
`DvrError` (replaced by `Error::Dvr(dvr_runtime::ArchiveError)`).

| Crate | Change | Semver class | Version |
|---|---|---|---|
| broadcast-hls | H1-H5, additive | 0.x additive | 0.9.0 |
| dvb-si | additive tracker | minor (lockstep) | 16.0.0, with dvb-t2mi, dvb-bbframe, dvb-conformance, dvb-tools |
| media-plane | additive `push_session`, `PushErrorKind`, `PushErrorClass` | 0.x additive | 0.14.0 |
| dvr-runtime | new | first release | 0.1.0 |
| multimux | public items removed; archive format changed; catch-up init URL changed; new dep dvr-runtime 0.1 | 0.x breaking | 0.21.0 |
| multimux-cli | its `multimux` caret epoch moves 0.20 → 0.21 | major-class (0.x: minor) | 0.19.0 |

No caret bucket spans two epochs: the jiff 0.2 → 1 wave moves `broadcast-common` (14.0.0), `broadcast-hls`,
`dvb-si`, `transmux` and `media-plane` together, and dvr-runtime builds against that wave.
Publish order: broadcast-common, broadcast-hls, dvb-si lockstep `v16.0.0`, transmux, media-plane; then dvr-runtime;
then multimux; then multimux-cli.

## 8. Test plan

### 8.1 Where the existing tests go

dvr.rs `mod tests` (1910-3927, 33 tests):

| Tests | New home |
|---|---|
| `a_later_recorder_leaves_earlier_periods_untouched` 1991, `retention_covers_periods_left_by_earlier_runs` 2044 | `dvr-runtime/tests/it/seed.rs` (fs) + a pure `seed` unit |
| `archive_floor_is_the_highest_archived_sequence` 2063 | dvr-runtime (`FsArchive::seq_floor`) |
| `a_failed_scan_is_an_error_not_a_zero_floor` 2090 | stays in multimux (JoinError path) |
| `a_sequence_beyond_u32_is_refused_without_writing` 2098 | writer unit (no plan) |
| `index_entries_are_appended_in_place_and_stay_valid_json` 2122, `a_failed_index_append_falls_back_to_a_whole_rewrite` 2161 | replaced: `records_append_and_parse_back`, `a_failed_playlist_append_truncates_and_resyncs` |
| `rebuild_index_*` 2217, 2248, 2279, 2569; `index_rebuild_works` 2879 | `dvr_runtime::recover` units (pure, on bytes), plus a case with trailing bytes after the last `mdat` |
| `dvr_recorder_rejects_an_unsafe_route_dir_name` 2326 | `ArchiveName` units |
| `append_after_a_stale_offset_stays_consistent` 2360 | writer unit via `LengthMismatch` + fs it |
| `one_oversized_segment_does_not_roll_an_empty_period` 2415, `byte_budget_keeps_the_configured_number_of_finished_periods` 2475, `byte_retention_counts_the_open_period` 2609, `a_byte_capped_route_with_no_time_roll_never_exceeds_the_period_budget` 2664, `period_rollover_and_retention` 2995 | writer units (plans asserted, no disk) + one fs it each for files on disk |
| `eviction_removes_the_index_before_the_period_file` 2525 | writer unit (playlist `Remove` before media `Remove`) |
| `retention_removes_the_programme_sidecar_with_its_period` 3888 | deleted with the sidecar (F6) |
| `period_file_is_independently_playable_fmp4` 2731 | rewritten: the period playlist plays (demux through its byte ranges with transmux) |
| `index_offsets_are_exact` 2790 | fs it on the playlist's byte ranges |
| `init_change_rolls_the_file` 2926 | kept, ported to dvr-runtime: an init change closes the period with `EXT-X-ENDLIST` and opens a new period whose media file starts with the new init |
| `recording_does_not_perturb_live_serving` 3054 | dvr-runtime it (Trunk only) |
| `ingest_pipeline_records_and_demux_from_disk` 3107, `real_dvbt_capture_records_and_replays_from_disk_only` 3252 | stay in multimux (they drive `advance_route`) |
| `hard_cap_rolls_on_clock_even_with_eit_configured_and_never_fed` 3375 | writer unit with simulated `now` |
| `eit_transition_rolls_period_using_real_fixture_following_event` 3448 | detection half to dvb-si; roll + DATERANGE half to dvr-runtime (8.3) |
| `feed_si_reads_split_at_any_size_recover_the_same_programme` 3618 | dvb-si tracker test |
| `a_failing_eit_section_is_dropped_not_refed_forever` 3652 | dvr-runtime fs it; the `si_errors_total` metric half stays in multimux |
| `stall_pin_force_expiry_rearms_and_records_a_gap_instead_of_stopping_forever` 3770 | writer unit (`Step::Rearm`) + fs it (re-pin) |
| `a_failed_period_roll_leaves_the_next_period_starting_with_the_init` 3846 | writer unit (`Failed` commits nothing) |

catchup.rs `mod tests` (666-1571, 25 tests): `merge_segments_*` 719-798 and `apply_window_*` 812-892 to
`dvr_runtime::read` units; `scan_archive_recovers_real_recorder_output_byte_exact` 938,
`find_archived_segment_locates_the_right_period_and_range` 996, `read_archived_bytes_*` 1051/1294,
`period_index_cache_*` 1172/1237/1307, `each_archive_directory_has_its_own_index_cache` 1452,
`the_index_cache_is_bounded_*` 1496/1544 to dvr-runtime fs tests; `parse_index_recovers_the_complete_entries_of_a_torn_sidecar`
1401 is replaced by `torn_tail_every_byte` (3.7); `dropping_a_route_cache_releases_its_indices` 1478 is deleted with
the global cache (an `FsArchive` drop test replaces it); `render_playlist_*` 1098-1379 stay in multimux.
output/catchup.rs tests (599-1609) and `tests/it/catchup_e2e.rs` stay in multimux, re-pointed at the new format and
init URL; `two_runs_with_different_inits_validate_with_mediastreamvalidator` (1390) keeps validating the served
catch-up playlist.

push: `ReconnectEngine` tests (856-960) and `drive_push_yields_when_no_listener_slot_is_free` (804) stay. New
the new `media_plane::push_session` module's unit tests cover every decision-table row with a scripted `PushEgress` mock.

### 8.2 Audit-23 findings (#3666-#3686)

| Issue | Lands in |
|---|---|
| #3666 validate, defaults, labels | `ArchiveConfig::validate`, `RetentionLimits::is_active`, `ArchiveWriter::overrun` in dvr-runtime; `DvrConfig` default 10800, `with_dvb_service_id`, `ArchiveOverrunSerde::name`, `Route::validate_dvr` in multimux |
| #3667 config boundaries | `ArchiveName` length boundary (dvr.rs:764) in dvr-runtime; config.rs limits in multimux |
| #3668, #3669, #3670, #3671 | multimux |
| #3672 labels/Debug | `ReadArchivedError::fmt` becomes `ReadError` Display tests in dvr-runtime; `SignedUrlKeySpec`, `TsHlsIdrFallback`, `ProgramDateTimeSource`, `Phase` in multimux |
| #3673 | multimux |
| #3674 gap accounting (Gap, Lagged, Stall `(Some, Some)`) | dvr-runtime writer units |
| #3675 roll and retention boundaries | dvr-runtime writer units at the exact limits, plus one fs it asserting files removed |
| #3676 append and rebuild with several segments | dvr-runtime: playlist append for the first vs later records; `recover::scan_fmp4_data` with three pairs and a `moof` followed by `free` |
| #3677 repeated present event id | dvb-si tracker unit + dvr-runtime `on_programme` (no roll, no second DATERANGE) |
| #3678 `seed_from_disk` swallows non-NotFound | dvr-runtime fs it (`chmod 000`, skipped as root) |
| #3679, #3680 | multimux |
| #3681 drive_push decisions and metrics | decision rows: media-plane `PushSession` units; `PushMetrics` read-back and flush failure end to end: multimux new integration test `push_drive` |
| #3682, #3683, #3684 | multimux |
| #3685 cache bounds and pruning | dvr-runtime `FsArchive` (exact `MAX_ENTRIES` boundary, two dirs) |
| #3686 dead API | `needs_poll` deleted in the dvr-runtime move; `TsHlsIdrFallback::falls_back` in multimux |

### 8.3 Real-fixture tests

- dvr-runtime integration test `real_dvbt::dvb_capture_records_and_reads_back`: feed the public
  `fixtures/ts/tnt-5w-12732v-isi6-10s.ts` (via `test_fixtures::path`) through `media_plane::ts_session` into a `Trunk`
  and an `FsRecorder` (TS container). Assert: every `pN.m3u8` parses with `broadcast_hls::MediaPlaylist::parse` and
  with `read::parse_period_playlist`; `FsArchive::read` of every segment equals the bytes the Trunk closed,
  byte-exact; re-demuxing the archived bytes finds the same 5 services as the live path. No skip branch.
- The same recording, closed, is one of the two inputs to `archive_playlists_pass_mediastreamvalidator` (3.7); the
  fMP4 input comes from `fixtures/ts/h264-aac.ts` through the fMP4 segmenter with a forced track change, so the
  recording rolls to a second period; each period playlist carries exactly one `EXT-X-MAP` and validates on its own.
- new dvb-si integration test `epg_present_tracker`: `fixtures/ts/tnt-5w-12732v-isi6-10s.ts` through `feed_ts`: one `Baseline`
  for service `0x0601`, event `0x7857`, title from the capture; the captured following event `0x7858` re-signalled
  with `running_status = Running` and version + 1 (as dvr.rs:3448) gives exactly one `Transition`; the same bytes
  again give none. Expected values cross-checked with `dvb-tools epg --json`.
- dvr-runtime: that transition through `FsRecorder::feed_si` closes `p0.m3u8` with `#EXT-X-ENDLIST` and starts
  `p1.m3u8` with a DATERANGE whose `X-DVB-EVENT-ID` is `30808` (`0x7858`) and whose title matches the capture.

## 9. Migration sequencing (each PR leaves main green)

1. **broadcast-hls H1-H5** (additive) with the render-split property test and DATERANGE/custom-tag round trips.
   Nothing else changes.
2. **dvb-si tracker** (additive) + tests (8.3, #3677 tracker half).
3. **media-plane PushSession** (additive) + unit tests.
4. **dvr-runtime crate** (depends on 1 and 2 by path): format, writer, reader, `fs`, recover, golden, oracle test,
   fuzz target `dvr_playlist`, 2 examples (`record_ts_file`, `list_programmes`), README/CHANGELOG, workspace-guards
   table lines (`non_exhaustive`, `label`, `truncation`, `panic_idioms`, `serialize_mask`, handroll shape 3 with the
   `"#EXT` needle so no m3u8 text is built outside broadcast-hls), CI lane. multimux untouched.
5. **multimux DVR/catch-up onto dvr-runtime**: delete `dvr.rs` except config, delete `catchup.rs` (move
   `render_playlist`), route wiring, `FsArchive` on `RouteHandle`, error and metric mapping, init URL, the multimux
   halves of #3666/#3667/#3672. CHANGELOG: archive format replaced; older archives are not readable.
6. **multimux `drive_push` onto PushSession** + new integration test `push_drive` (#3681).
7. **Release**: broadcast-common, broadcast-hls, dvb-si lockstep 16.0.0, transmux, media-plane; then dvr-runtime
   0.1.0; then multimux 0.21.0 and multimux-cli 0.19.0, per RELEASE-AUDIT.

PRs 1, 2 and 3 are independent and can land in any order; 4 needs 1 and 2; 5 needs 4; 6 needs 3.

## 10. Owner decisions (2026-11-21)

1. `std` only for 0.1.0, no `no_std` claim; the core reuses media-plane's `std`-gated input types.
2. broadcast-hls (H1-H5) and media-plane (`PushSession`) ship in the jiff 1 wave: broadcast-hls 0.9.0,
   media-plane 0.14.0.
3. An fMP4 init change rolls the period, as multimux does today (`roll_to_init`); each period has one init and one
   `EXT-X-MAP`, and every raw `pN.m4s` stays playable as a plain file. This reverses the earlier draft of D7/F5.
4. A sequence gap keeps rolling the period (F8 as written).
