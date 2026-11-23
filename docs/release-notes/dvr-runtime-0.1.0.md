# dvr-runtime 0.1.0

_Released 2026-11-22._

First release. `dvr-runtime` is the one owner of the live-origin DVR archive format: it defines what a recorded archive looks like on disk, the sans-IO writer that produces it, the reader that plans a catch-up request over it, and an optional blocking `std::fs` adapter. `multimux` 0.21 records and serves catch-up through this crate; before that, both halves lived inside `multimux`. The design is in [docs/dvr-runtime/DESIGN.md](../dvr-runtime/DESIGN.md).

## What it is

- **`ArchiveWriter`** is sans-IO. The host feeds it one cursor item at a time; it returns an ordered `Plan` of `StoreOp`s and commits its own state only when the host reports `complete(PlanOutcome::Done)`. A failed store operation therefore changes nothing: the same plan is retried, and no partial period is ever adopted.
- **Roll rules** are explicit and each has its own roll reason: first segment, sequence gap, target-duration overflow, byte budget, playlist size, init change, period duration and programme transition.
- **`read`** parses one period playlist under the torn-line rule with typed validation, and plans catch-up over the archive plus the live tail (`merge`, `window`, `checked_range`, `ArchiveScan`, `SeqConflict`). **`recover`** rescans an fMP4 period file for its init and fragment spans.

## The on-disk format

One directory per archive, one *period* per media file, one HLS Media Playlist index per period:

```
<archive>/
  p0.m3u8   #EXTM3U, EXT-X-VERSION, PLAYLIST-TYPE:EVENT, EXT-X-TARGETDURATION,
            #EXT-X-RUSTBROADCAST-ARCHIVE:VERSION=1,CONTAINER=FMP4,PERIOD=0
            #EXT-X-MAP:<the period's one fMP4 init, as a byterange>
            #EXT-X-DATERANGE:<programme identity, repeated on the first segment of every period>
            #EXTINF..., #EXT-X-RUSTBROADCAST-TIMING:..., #EXT-X-BYTERANGE:... <uri>
            #EXT-X-ENDLIST   (only once the period is closed)
  p0.m4s    # the period's single media file
```

- Every line is rendered and parsed by `broadcast-hls`; the crate never writes playlist text by hand.
- `EXT-X-RUSTBROADCAST-TIMING` carries each segment's exact Trunk timing, so a catch-up reader does not have to re-parse media to find a boundary.
- `EXT-X-DATERANGE` carries the programme identity, and a period opened by *any* roll repeats the current programme on its first segment, so a byte-capped or untimed route keeps programme identity on every period of that programme.
- The playlist marker names the container and the period number, so a reader can reject a file it does not understand instead of guessing.

## Crash-safety guarantees

- A playlist is created at the first segment and appended one record at a time; a record is appended only after its media bytes are on disk, and a playlist renamed into place whose directory sync failed is reported as `NotDurable` rather than silently counted as durable.
- A crash leaves at worst a torn last line, which the reader detects and ignores by rule (never a silently truncated record), and a temp playlist, which is reported as `StaleTempPlaylist` before its period number is reused.
- `seed` numbers past every archive file, and an `AlreadyExists` create consumes the number instead of retrying forever, so a crash never makes the next run reuse a period.
- A media file left without a playlist enters the retention ledger with its size, so it cannot pin the archive below its byte budget.
- One `.lock` file per archive directory keeps a single writer: a second recorder is refused with `FsError::Locked`, and `FsRecorder::succeed` hands the lock from a predecessor to its successor without releasing it in between.
- An archive directory that exists but cannot be read is a typed `ReadError`, never an empty archive and never a zero sequence floor, so a restarted route cannot number over segments it could not list.

## Breaking changes

None (first release). Sibling floors: `broadcast-common` 14, `broadcast-hls` 0.9, `dvb-si` 16, `media-plane` 0.14, `transmux` 0.33, with `mpeg-ts` 0.10 as a dev-dependency. Those are the versions that carry the APIs this crate uses (typed `DateRange`/`CustomTag`/`ParseOptions`/renderers, `epg::PresentEventTracker`, jiff 1 types).

## Known gaps

- TS recovery (`recover` is fMP4-only); a recovered fragment begins at its `styp`/`emsg`/`prft` run before the `moof`.
- `FsRecorder::poll` is the only driver: the crate no longer ships the incomplete `SegmentEgress` adapter, because that trait hands over one cursor item with no monotonic time, wall time or init generations.

## Pre-release audit

Issues fixed before this first release by the 2026-11-22 pre-release audit: #3977/#4005 (dependency floors), #3978 (programme DATERANGE on every roll), #3979 (unreadable archive is an error), #3980 (incomplete `SegmentEgress` dropped), #3981/#4006 (one dated CHANGELOG section), #3982 (crate-root commands), #3983 (oracle covers DATERANGE/PDT), #3984 (fuzz seeds and the TS branch), #3985 (this note), #4007 (stated floors), #4023 (DESIGN's jiff version), #4028/#4029/#4030/#4031/#4032/#4033/#4034/#4035/#4036 (the LOW findings: tag tokens, key index, recovery spans, checked lengths, playlist read cap, OS error source, lock op kind, discontinuity marking, terminated-run counts), #4037 (DESIGN matches the code), #4038 (workspace crate index), #4039 (README coverage and quick start), #4040 (test scratch dirs).

---

Published from tag `dvr-runtime-v0.1.0`.
