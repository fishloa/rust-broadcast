# Changelog

All notable changes to this crate are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this crate uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-11-22

First release: the DVR on-disk format, the sans-IO roll/retention writer, the
catch-up read planner and the blocking `fs` adapter.

### Added

- The archive format: one HLS Media Playlist per period
  (`pN.m3u8`), one media file per period (`pN.m4s` / `pN.ts`), the
  `EXT-X-RUSTBROADCAST-ARCHIVE` playlist marker and the per-segment
  `EXT-X-RUSTBROADCAST-TIMING` tag. Segments are `EXT-X-BYTERANGE` into the
  period's single media file; one fMP4 init per period, at the file's head.
- `ArchiveWriter`: a sans-IO roll/retention state machine returning an ordered
  `Plan` of `StoreOp`s, committing state only on `complete(PlanOutcome::Done)`.
  Rolls on first segment, sequence gap, target-duration overflow, byte budget,
  playlist size, init change, period duration and programme transition. Every
  length it converts is checked with `u64::try_from` and its playlist-size
  total saturates, so no over-range media size can wrap (#4031).
- A period opened by any of those rolls repeats the current programme's
  `EXT-X-DATERANGE` on its first segment, so a byte-capped or untimed route
  keeps the programme identity on every period of that programme, and the
  first segment after lost ring entries is marked discontinuous (#3978). A
  stall-terminated run that recorded nothing counts no skipped segment, so
  its gap and rearm counts describe what it missed and not the Trunk's whole
  history (#4036).
  `read::merge` marks every sequence jump discontinuous, inside the archive as
  well as at the archive-to-live boundary (#4035).
- `read::parse_period_playlist` with the torn-line rule, typed validation and
  programme decoding; `read::merge`/`read::window`/`read::checked_range` for
  catch-up planning; `read::ArchiveScan` and `read::SeqConflict` for one scan
  of every period. A tag the archive refuses is named by its spec token
  (`EXT-X-GAP`, `EXT-X-KEY`, `EXT-X-PART`) (#4028), and a key at the index of
  the first segment it covers rather than always at `0` (#4029).
- `recover::scan_fmp4_data`: init and `moof`+`mdat` span recovery from an fMP4
  period file. A fragment begins at the `styp`/`emsg`/`prft` run before its
  `moof`, so a recorded CMAF segment's span comes back whole; an LL-HLS
  segment split into several `moof` runs is still one fragment per `moof`
  (#4030).
- Feature `fs`: `FsRecorder` (blocking plan execution, retry on a partial
  write, `poll` as the only driver) and `FsArchive` (scan/find/read/recover
  plus a bounded per-archive parsed-playlist cache). `FsRecorder` is not a
  `media_plane::egress::SegmentEgress`: that trait hands over one cursor item
  with no monotonic time, wall time or init generations, so a period could
  never roll on `period_duration` and an fMP4 archive would drop every
  segment as arriving before its init (#3980).
- `FsArchive::scan`/`find` report a directory or period playlist that exists
  but cannot be read as a typed `ReadError` instead of an empty archive, and
  `seq_floor` returns `Option<SegmentSeq>` and fails on an unreadable period,
  so a restarted route never numbers over an archive it could not list
  (#3979). Period playlists are read at most `MAX_PLAYLIST_BYTES + 1` bytes
  (#4032). A failed store operation surfaces the OS error it saw as the
  `FsError::Io` source, with its errno intact (#4033), and a lock-file open or
  lock failure names the new `StoreOpKind::Lock` instead of `List` (#4034).
- `FsRecorder::with_cursor`: create a recorder from a pinning cursor the host
  took earlier, keeping the cursor when the create is refused (for example
  `FsError::Locked` while the previous recorder of the archive still holds
  its directory), so a host replacing a recorder misses no segment closed in
  between (multimux reconnects, #3929).
- `FsRecorder::succeed`: the recorder of a new run on the same archive, taking
  over the previous recorder's directory lock without releasing it, so a
  process the host spawns at that moment cannot make the new run's create
  fail with `FsError::Locked`.
- `FsRecorder::on_programme`: apply a programme change the host detected
  before the recorder existed, with the same retry as `feed_si`.
- `StoreFailure::NotDurable`, `WriterEvent::NotDurable`,
  `WriterEvent::StaleTempPlaylistRemoved` and `ArchiveError::StaleTempPlaylist`:
  a playlist renamed into place whose directory sync failed counts as
  created, so its segment is never archived twice, and a playlist left behind
  by a crash is noted before its period number is reused.
- Crash recovery: `seed` numbers past every archive file, an `AlreadyExists`
  create consumes the number instead of retrying it forever, a media file
  left without a playlist enters the retention ledger with its size, and a
  failed `sync_data` is a retried store failure.

### Changed

- Builds against `jiff` 1.0: the recording and read-side wall-clock times are
  jiff 1.x `Timestamp`s.
- Requires broadcast-hls 0.9, dvb-si 16, broadcast-common 14, media-plane 0.14
  and transmux 0.33 (#4007).

### Documentation

- The crate-root examples are the commands that run, with the `fs` feature
  and a real input file (#3982).
- The independent `mediastreamvalidator` oracle also reads a closed
  programme-tracking period, so `EXT-X-PROGRAM-DATE-TIME` and the
  `EXT-X-DATERANGE` line (including the UTF-8 title) are checked by Apple's
  validator and not only by this crate's own parser (#3983).
- The `dvr_playlist` fuzz target takes its container and period from the
  first input byte, so the TS branch is reachable, and `fuzz/seed-corpus.py`
  seeds it with the fMP4 golden and the same period written as TS (#3984).
