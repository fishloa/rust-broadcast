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
write resynchronises its offset from the length the store reports. The whole
roll/retention state machine is testable without a disk.

A period rolls on: the first segment; a sequence gap; a segment longer than
the playlist's target duration; the per-period byte budget; a playlist that
would exceed the HLS size bound; an fMP4 init change; the configured period
duration on the caller's monotonic clock; and a programme transition.

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

```rust
use std::time::Duration;
use dvr_runtime::{ArchiveConfig, ArchiveContainer, ArchiveName, RetentionLimits};

let retention = RetentionLimits::new()
    .with_periods(std::num::NonZeroUsize::new(8).unwrap());
let config = ArchiveConfig::new(ArchiveContainer::Ts, Duration::from_secs(2), retention)
    .with_period_duration(Some(Duration::from_secs(3600)));
let _name = ArchiveName::new("cam-1").unwrap();
```

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
