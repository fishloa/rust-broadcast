# dvr-runtime

[![crates.io](https://img.shields.io/crates/v/dvr-runtime.svg)](https://crates.io/crates/dvr-runtime)
[![docs.rs](https://docs.rs/dvr-runtime/badge.svg)](https://docs.rs/dvr-runtime)
[![license](https://img.shields.io/crates/l/dvr-runtime.svg)](#license)

Sans-IO DVR archive for a live origin: the on-disk period format, the writer's
roll/retention state machine, and catch-up read planning. No I/O, no async, no
clock reads in the core; the `fs` feature adds the blocking `std::fs` adapter.

## The format

One directory per archive, one period per media file:

```text
<archive_root>/<archive_name>/
  pN.m4s | pN.ts   period media file
  pN.m3u8          period playlist (the index)
```

The playlist is a standard HLS Media Playlist (RFC 8216 / RFC 8216bis):
`PLAYLIST-TYPE:EVENT`, one segment record appended at a time, closed with
`#EXT-X-ENDLIST`, every segment an `EXT-X-BYTERANGE` into the period's single
media file. The fMP4 init is the media file's head, so a raw `pN.m4s`/`pN.ts`
is playable on its own. Two vendor tags carry the archive's own facts:

| Tag | Level | Attributes |
|---|---|---|
| `#EXT-X-RUSTBROADCAST-ARCHIVE` | playlist | `VERSION=1,CONTAINER=FMP4\|TS,PERIOD=<N>` |
| `#EXT-X-RUSTBROADCAST-TIMING` | segment | `SEQ=<s>,DTS=<t>,DUR=<d>,TIMESCALE=<ts>` |

RFC 8216bis §4.1 makes every `#EXT` line a tag and §6.3.1 requires clients to
ignore unknown tags, so a standard player skips both. Programme identity is an
`EXT-X-DATERANGE` (§4.4.5.1) with `CLASS="io.github.fishloa.rust-broadcast.programme"`
and the `X-DVB-*` attributes.

Every line is produced and read by [`broadcast-hls`]; this crate never
assembles tag text by hand.

## The writer is a plan, not a file

`ArchiveWriter` turns each decision into an ordered `Plan` of `StoreOp`s. The
host executes it and calls `complete(PlanOutcome)`; only a `Done` commits
state. A failed roll therefore leaves the recorder unchanged, and a partial
write resynchronises its offset from the length the store reports, and
`last_input_consumed` tells the host whether the failed input still needs
retrying or was repaired in place. The whole roll/retention state machine is
testable without a disk.

A period rolls on: the first segment; a sequence gap; a segment longer than
the playlist's target duration; the per-period byte budget; a playlist that
would exceed the HLS size bound; an fMP4 init change; the configured period
duration on the caller's monotonic clock; and a programme transition. A
period opened by any of those rolls repeats the current programme's
`EXT-X-DATERANGE` on its first segment, so the programme is findable on every
period it covers.

## The reader plans catch-up

`read::parse_period_playlist` reads one period with the torn-line rule and
typed validation, and `read::merge` combines the archive with the live
window's still-unarchived tail into one ascending sequence, marking every
sequence jump discontinuous; `read::window` cuts that to the trailing DVR
window and `read::checked_range` verifies a recorded byte range against the
media file. `read::ArchiveScan` is one pass over every period: its
`unreadable` periods, and its `SeqConflict`s where a restarted run claimed an
archived sequence (the earlier period's record wins). `recover` rescans an
fMP4 period's init and fragment spans when a playlist is lost.

Periods that cannot be read are a typed error, never an empty archive: a
route that cannot list its archive or cannot read a period playlist must not
number itself from `0` over it.

## The `fs` adapter

`fs::FsRecorder` executes the writer's plans on `std::fs` and is the only
driver of the writer: `poll` passes monotonic time, the wall clock and the
fMP4 init generations, which one `media_plane` cursor item does not carry. It
holds an exclusive `.lock` on its directory, taken by `new`, moved to the next
run by `succeed` and kept by `with_cursor` when the lock still refuses the
create, so a host that reconnects misses no segment; `on_programme` applies a
programme change the host detected before the recorder existed.
`fs::FsArchive` is the read half with its own bounded parsed-playlist cache.

## Features

| Feature | Adds |
|---|---|
| `fs` | `fs::FsRecorder` (executes plans on `std::fs`) and `fs::FsArchive` (the read half plus a bounded per-archive parsed-playlist cache). Blocking. |

The core is `std` but performs no I/O. No tokio, tracing, metrics, serde or
JSON, and no clock reads: every instant is an argument.

## Quick start

```toml
[dependencies]
dvr-runtime = { version = "0.1", features = ["fs"] }
```

The core has no I/O and no clock: the host supplies the instant, the wall
clock and the cursor items its `media_plane::Trunk` publishes, and executes
the plan.

```rust
use std::time::Duration;
use dvr_runtime::{
    ArchiveConfig, ArchiveContainer, ArchiveName, ArchiveWriter, PlanOutcome, RetentionLimits, Step,
};
use media_plane::trunk::SegmentCursorItem;

let retention = RetentionLimits::new()
    .with_periods(std::num::NonZeroUsize::new(8).unwrap());
let config = ArchiveConfig::new(ArchiveContainer::Ts, Duration::from_secs(2), retention)
    .with_period_duration(Some(Duration::from_secs(3600)));
let _name = ArchiveName::new("cam-1").unwrap();
let mut writer = ArchiveWriter::new(config).unwrap();

// The host loop: hand over one cursor item, execute the plan it returns, report.
loop {
    let Some(item) = cursor.poll() else { break };
    let step = writer.on_item(now, wall, &item).unwrap();
    let Step::Execute(plan) = step else { continue };
    for op in &plan.ops {
        store.execute(op)?;                  // create_new data file, append, sync, ...
    }
    let outcome = store.outcome(&plan);      // Done, or Failed { at_op, failure }
    writer.complete(plan.id, outcome).unwrap();
    if !writer.last_input_consumed() {
        // keep `item` and retry it, or record a `SegmentCursorItem::Gap { skipped: 1 }`
    }
}
```

With the `fs` feature the loop is `FsRecorder::poll(now, wall, &inits)`, which
builds and executes the plan for you.

## Examples

```text
cargo run -p dvr-runtime --example record_ts_file --all-features -- fixtures/ts/h264-aac.ts
cargo run -p dvr-runtime --example list_programmes --all-features -- fixtures/ts/tnt-5w-12732v-isi6-10s.ts
```

## Spec

- HLS Media Playlist: RFC 8216 / RFC 8216bis §4.1, §4.2, §4.4.4, §4.4.5,
  §6.3.1 (transcription in the private repo at
  `private/specs-verified/ietf/specs/ietf_draft_pantos_hls_rfc8216bis.txt`).
- Programme boundaries: ETSI EN 300 468 §5.2.4 (EIT present/following),
  decoded by `dvb-si`.

## License

MIT OR Apache-2.0.
